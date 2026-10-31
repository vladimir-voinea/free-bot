//! Connectors: a Model Context Protocol (MCP) client. Each enabled connector
//! is started once and shared by every agent, like plugin connections in the
//! agent apps this follows. Supports local servers over stdio and remote
//! servers over Streamable HTTP.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::model::{Agent, Connector, Transport};
use super::tools::{Risk, ToolDef};
use crate::api::Waker;

const PROTOCOL_VERSION: &str = "2025-06-18";
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub read_only: bool,
}

#[derive(Clone, Debug)]
pub enum McpStatus {
    Starting,
    Ready(Vec<McpTool>),
    Failed(String),
}

trait Rpc: Send + Sync {
    fn request(&self, method: &str, params: Value) -> Result<Value, String>;
    fn notify(&self, method: &str, params: Value);
}

struct Server {
    connector: Connector,
    slug: String,
    status: McpStatus,
    rpc: Option<Arc<dyn Rpc>>,
}

pub struct McpManager {
    servers: Arc<Mutex<HashMap<String, Server>>>,
    waker: Arc<dyn Waker>,
}

impl McpManager {
    pub fn new(waker: Arc<dyn Waker>) -> Self {
        Self { servers: Arc::new(Mutex::new(HashMap::new())), waker }
    }

    /// Start new or changed connectors and stop removed or disabled ones.
    pub fn sync(&self, connectors: &[Connector]) {
        let mut servers = self.servers.lock().unwrap_or_else(|e| e.into_inner());
        servers.retain(|id, s| connectors.iter().any(|c| &c.id == id && c.enabled && c.transport == s.connector.transport));
        for c in connectors.iter().filter(|c| c.enabled) {
            if let Some(s) = servers.get_mut(&c.id) {
                s.connector = c.clone();
                s.slug = slug(&c.name);
                continue;
            }
            servers.insert(c.id.clone(), Server { connector: c.clone(), slug: slug(&c.name), status: McpStatus::Starting, rpc: None });
            self.start(c.clone());
        }
    }

    /// Wait (up to `timeout`) for connectors that are still starting, so an
    /// agent turn right after launch sees their tools.
    pub fn wait_ready(&self, timeout: std::time::Duration) {
        let end = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < end {
            let starting = self.servers.lock().unwrap_or_else(|e| e.into_inner()).values().any(|s| matches!(s.status, McpStatus::Starting));
            if !starting {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    pub fn restart(&self, c: &Connector) {
        self.servers.lock().unwrap_or_else(|e| e.into_inner()).remove(&c.id);
        self.sync(std::slice::from_ref(c));
    }

    fn start(&self, c: Connector) {
        let servers = self.servers.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let result = connect(&c).and_then(|rpc| {
                let tools = list_tools(&*rpc)?;
                Ok((rpc, tools))
            });
            let mut map = servers.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = map.get_mut(&c.id) {
                match result {
                    Ok((rpc, tools)) => {
                        s.rpc = Some(rpc);
                        s.status = McpStatus::Ready(tools);
                    }
                    Err(e) => s.status = McpStatus::Failed(e),
                }
            }
            drop(map);
            waker.wake();
        });
    }

    pub fn status(&self, id: &str) -> Option<McpStatus> {
        self.servers.lock().unwrap_or_else(|e| e.into_inner()).get(id).map(|s| s.status.clone())
    }

    pub fn tool_defs(&self, agent: &Agent, connectors: &[Connector]) -> Vec<ToolDef> {
        let servers = self.servers.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for c in connectors.iter().filter(|c| c.enabled) {
            if agent.connectors.as_ref().is_some_and(|allowed| !allowed.contains(&c.id)) {
                continue;
            }
            let Some(s) = servers.get(&c.id) else { continue };
            let McpStatus::Ready(tools) = &s.status else { continue };
            for t in tools {
                let mut schema = t.schema.clone();
                if !schema.is_object() {
                    schema = json!({"type": "object", "properties": {}});
                }
                let risk = if t.read_only || c.trusted { Risk::Safe } else { Risk::Risky };
                out.push(ToolDef::new(
                    &tool_name(&s.slug, &t.name),
                    &format!("[{}] {}", c.name, t.description),
                    schema,
                    risk,
                    &format!("Using {}", c.name),
                ));
            }
        }
        out
    }

    pub fn call(&self, full_name: &str, args: Value) -> Result<String, String> {
        let (rpc, tool) = {
            let servers = self.servers.lock().unwrap_or_else(|e| e.into_inner());
            let mut found = None;
            for s in servers.values() {
                if let McpStatus::Ready(tools) = &s.status
                    && let Some(t) = tools.iter().find(|t| tool_name(&s.slug, &t.name) == full_name)
                {
                    found = s.rpc.clone().map(|r| (r, t.name.clone()));
                }
            }
            found.ok_or_else(|| "That connector is not connected right now.".to_string())?
        };
        let args = if args.is_object() { args } else { json!({}) };
        let res = rpc.request("tools/call", json!({"name": tool, "arguments": args}))?;
        let text = content_text(&res);
        if res.get("isError").and_then(Value::as_bool).unwrap_or(false) { Err(text) } else { Ok(text) }
    }
}

fn content_text(res: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(items) = res.get("content").and_then(Value::as_array) {
        for it in items {
            match it.get("type").and_then(Value::as_str) {
                Some("text") => parts.push(it.get("text").and_then(Value::as_str).unwrap_or("").to_string()),
                Some("resource") => {
                    let r = it.get("resource").cloned().unwrap_or_default();
                    parts.push(r.get("text").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| r.to_string()));
                }
                Some("image") => parts.push("[image]".into()),
                _ => parts.push(it.to_string()),
            }
        }
    }
    if parts.is_empty()
        && let Some(sc) = res.get("structuredContent")
    {
        parts.push(sc.to_string());
    }
    if parts.is_empty() { "(no output)".into() } else { parts.join("\n") }
}

pub fn slug(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "connector".into() } else { s.chars().take(20).collect() }
}

/// Tool names must match `^[a-zA-Z0-9_-]{1,64}$` for most providers.
pub fn tool_name(server_slug: &str, tool: &str) -> String {
    let t: String = tool.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    let full = format!("mcp__{server_slug}__{t}");
    full.chars().take(64).collect()
}

fn list_tools(rpc: &dyn Rpc) -> Result<Vec<McpTool>, String> {
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let params = match &cursor {
            Some(c) => json!({"cursor": c}),
            None => json!({}),
        };
        let res = rpc.request("tools/list", params)?;
        for t in res.get("tools").and_then(Value::as_array).cloned().unwrap_or_default() {
            tools.push(McpTool {
                name: t.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                description: t.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({"type": "object"})),
                read_only: t.pointer("/annotations/readOnlyHint").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        cursor = res.get("nextCursor").and_then(Value::as_str).map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok(tools)
}

fn connect(c: &Connector) -> Result<Arc<dyn Rpc>, String> {
    let rpc: Arc<dyn Rpc> = match &c.transport {
        Transport::Stdio { command, args, env } => Arc::new(StdioRpc::spawn(command, args, env)?),
        Transport::Http { url, headers } => Arc::new(HttpRpc::new(url, headers)),
    };
    let init = rpc.request(
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "Free Bot", "version": env!("CARGO_PKG_VERSION")}
        }),
    )?;
    if init.get("protocolVersion").is_none() {
        return Err("The server did not answer the MCP handshake.".into());
    }
    rpc.notify("notifications/initialized", json!({}));
    Ok(rpc)
}

// ---- stdio transport --------------------------------------------------------

type Pending = Arc<Mutex<HashMap<u64, Sender<Result<Value, String>>>>>;

struct StdioRpc {
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next: AtomicU64,
}

impl StdioRpc {
    fn spawn(command: &str, args: &[String], env: &[(String, String)]) -> Result<Self, String> {
        let mut cmd = if cfg!(windows) {
            // Lets `npx` and other .cmd launchers work.
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            Command::new(command)
        };
        cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| format!("Could not start \"{command}\": {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let p2 = pending.clone();
        let (reply_tx, reply_rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(method) = msg.get("method").and_then(Value::as_str) {
                    // A request from the server: answer pings, refuse the rest.
                    if let Some(id) = msg.get("id") {
                        let reply = if method == "ping" {
                            json!({"jsonrpc": "2.0", "id": id, "result": {}})
                        } else {
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Not supported"}})
                        };
                        let _ = reply_tx.send(reply.to_string());
                    }
                    continue;
                }
                if let Some(id) = msg.get("id").and_then(Value::as_u64)
                    && let Some(tx) = p2.lock().unwrap_or_else(|e| e.into_inner()).remove(&id)
                {
                    let _ = tx.send(rpc_result(&msg));
                }
            }
            // The process ended: fail everything still waiting.
            for (_, tx) in p2.lock().unwrap_or_else(|e| e.into_inner()).drain() {
                let _ = tx.send(Err("The connector stopped.".into()));
            }
        });
        let stdin = Arc::new(Mutex::new(stdin));
        // Answers to server-initiated requests go out through the same stdin.
        let replies = stdin.clone();
        std::thread::spawn(move || {
            for line in reply_rx {
                let mut w = replies.lock().unwrap_or_else(|e| e.into_inner());
                let _ = writeln!(w, "{line}");
                let _ = w.flush();
            }
        });
        Ok(Self { child: Mutex::new(child), stdin, pending, next: AtomicU64::new(1) })
    }

    fn write(&self, v: &Value) -> Result<(), String> {
        let mut w = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(w, "{v}").and_then(|_| w.flush()).map_err(|e| format!("The connector is not running: {e}"))
    }
}

impl Drop for StdioRpc {
    fn drop(&mut self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Rpc for StdioRpc {
    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        match rx.recv_timeout(CALL_TIMEOUT) {
            Ok(r) => r,
            Err(_) => {
                self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                Err(format!("The connector did not answer {method} in time."))
            }
        }
    }

    fn notify(&self, method: &str, params: Value) {
        let _ = self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }
}

fn rpc_result(msg: &Value) -> Result<Value, String> {
    if let Some(err) = msg.get("error") {
        return Err(err.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| err.to_string()));
    }
    Ok(msg.get("result").cloned().unwrap_or(Value::Null))
}

// ---- Streamable HTTP transport ----------------------------------------------

struct HttpRpc {
    url: String,
    headers: Vec<(String, String)>,
    session: Mutex<Option<String>>,
    next: AtomicU64,
    client: reqwest::blocking::Client,
}

impl HttpRpc {
    fn new(url: &str, headers: &[(String, String)]) -> Self {
        Self {
            url: url.to_string(),
            headers: headers.to_vec(),
            session: Mutex::new(None),
            next: AtomicU64::new(1),
            client: reqwest::blocking::Client::builder().timeout(CALL_TIMEOUT).build().expect("http client"),
        }
    }

    fn post(&self, body: &Value) -> Result<reqwest::blocking::Response, String> {
        let mut rb = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION);
        for (k, v) in &self.headers {
            rb = rb.header(k.as_str(), v.as_str());
        }
        if let Some(s) = self.session.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            rb = rb.header("Mcp-Session-Id", s);
        }
        let resp = rb.json(body).send().map_err(|e| format!("Could not reach the connector: {}", crate::api::root_cause(&e)))?;
        if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.to_string());
        }
        if !resp.status().is_success() {
            let code = resp.status().as_u16();
            let text = resp.text().unwrap_or_default();
            return Err(crate::api::error_message(code, &text));
        }
        Ok(resp)
    }
}

impl Rpc for HttpRpc {
    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let resp = self.post(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let is_sse =
            resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("event-stream"));
        if !is_sse {
            let v: Value = resp.json().map_err(|e| format!("Bad response from the connector: {e}"))?;
            return rpc_result(&v);
        }
        for line in BufReader::new(resp).lines() {
            let line = line.map_err(|e| e.to_string())?;
            let Some(data) = line.strip_prefix("data:") else { continue };
            if let Ok(v) = serde_json::from_str::<Value>(data.trim())
                && v.get("id").and_then(Value::as_u64) == Some(id)
            {
                return rpc_result(&v);
            }
        }
        Err("The connector closed the stream without an answer.".into())
    }

    fn notify(&self, method: &str, params: Value) {
        let _ = self.post(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_valid() {
        assert_eq!(tool_name(&slug("My GitHub!"), "create issue"), "mcp__my_github__create_issue");
        assert!(tool_name("x", &"a".repeat(100)).len() <= 64);
    }

    #[test]
    fn content_to_text() {
        let v = json!({"content": [{"type": "text", "text": "one"}, {"type": "text", "text": "two"}]});
        assert_eq!(content_text(&v), "one\ntwo");
    }
}
