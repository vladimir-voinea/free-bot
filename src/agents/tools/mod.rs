//! The tools agents can call. Each tool has a JSON schema, a risk level that
//! decides whether the user must approve it, and an implementation.

mod browser_tools;
mod files;
mod knowledge;
mod team;
mod web;

use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use super::engine::{Fleet, FleetState, Wake};
use super::model::Agent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// Talking to teammates, memory, skills: never needs approval.
    Internal,
    /// Reads or acts only inside the agent's own computer.
    Safe,
    /// Can change things outside the agent's sandbox (shell, connector writes, new agents).
    Risky,
}

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub params: Value,
    pub risk: Risk,
    /// Shown while it runs ("Reading a file").
    pub status: String,
}

impl ToolDef {
    pub fn new(name: &str, description: &str, params: Value, risk: Risk, status: &str) -> Self {
        Self { name: name.into(), description: description.into(), params, risk, status: status.into() }
    }

    pub fn to_openai(&self) -> Value {
        json!({"type": "function", "function": {"name": self.name, "description": self.description, "parameters": self.params}})
    }
}

pub struct ToolCtx<'a> {
    pub fleet: &'a Fleet,
    pub agent: &'a Agent,
    pub thread: &'a str,
    pub wake: &'a Wake,
    pub cancel: &'a AtomicBool,
    pub call_id: &'a str,
}

#[derive(Debug, Default)]
pub struct ToolOutput {
    pub text: String,
    pub error: bool,
    /// A thread to link from the tool card.
    pub link: Option<String>,
    /// Stop the turn after this call (the agent is waiting for the user).
    pub end_turn: bool,
}

impl ToolOutput {
    pub fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Default::default() }
    }
    pub fn err(text: impl Into<String>) -> Self {
        Self { text: text.into(), error: true, ..Default::default() }
    }
}

/// Builds a JSON schema object from `(name, type, description, required)` tuples.
pub fn schema(props: &[(&str, &str, &str, bool)]) -> Value {
    let mut p = serde_json::Map::new();
    let mut req = Vec::new();
    for (name, ty, desc, required) in props {
        p.insert((*name).into(), json!({"type": ty, "description": desc}));
        if *required {
            req.push(json!(name));
        }
    }
    json!({"type": "object", "properties": p, "required": req})
}

pub fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

pub fn definitions(fleet: &Fleet, st: &FleetState, agent: &Agent) -> Vec<ToolDef> {
    let mut defs = Vec::new();
    let t = &agent.tools;
    if t.team {
        defs.extend(team::definitions(st.agents.len() > 1));
    }
    if t.memory {
        defs.extend(knowledge::memory_definitions(st.config.knowledge_dir.is_some()));
    }
    if t.skills {
        defs.extend(knowledge::skill_definitions());
    }
    if t.routines {
        defs.extend(knowledge::routine_definitions());
    }
    if t.files {
        defs.extend(files::file_definitions());
    }
    if t.terminal {
        defs.extend(files::terminal_definitions());
    }
    if t.web {
        defs.extend(web::definitions());
    }
    if t.browser {
        defs.extend(browser_tools::definitions());
    }
    defs.extend(fleet.mcp().tool_defs(agent, &st.config.connectors));
    defs
}

pub fn execute(ctx: &ToolCtx, name: &str, args: &Value) -> ToolOutput {
    if let Some(out) = team::execute(ctx, name, args) {
        return out;
    }
    if let Some(out) = knowledge::execute(ctx, name, args) {
        return out;
    }
    if let Some(out) = files::execute(ctx, name, args) {
        return out;
    }
    if let Some(out) = web::execute(ctx, name, args) {
        return out;
    }
    if let Some(out) = browser_tools::execute(ctx, name, args) {
        return out;
    }
    if name.starts_with("mcp__") {
        return match ctx.fleet.mcp().call(name, args.clone()) {
            Ok(text) => ToolOutput::ok(text),
            Err(e) => ToolOutput::err(e),
        };
    }
    ToolOutput::err(format!("Unknown tool \"{name}\"."))
}

/// A one-line, human description of a call, for tool cards and approval prompts.
pub fn describe_call(name: &str, args: &Value) -> String {
    let s = |k: &str| str_arg(args, k).map(|v| v.chars().take(90).collect::<String>()).unwrap_or_default();
    match name {
        "list_agents" => "Looked at the team".into(),
        "send_message" => format!("Message to {}", s("to")),
        "delegate_task" => format!("Handed a task to {}", s("to")),
        "post_to_channel" => format!("Posted in #{}", s("channel").trim_start_matches('#')),
        "ask_user" => "Asked you a question".into(),
        "update_task" => format!("Marked a task {}", s("status")),
        "create_agent" => format!("Create a new bot: {}", s("name")),
        "remember" => format!("Remembered: {}", s("fact")),
        "forget" => "Forgot a memory".into(),
        "search_memory" => format!("Searched memory for \"{}\"", s("query")),
        "search_knowledge" => format!("Searched the knowledge folder for \"{}\"", s("query")),
        "use_skill" => format!("Loaded the skill \"{}\"", s("name")),
        "save_skill" => format!("Saved the skill \"{}\"", s("name")),
        "create_routine" => format!("Created the routine \"{}\" ({})", s("name"), s("schedule")),
        "list_files" => format!("Listed files in {}", if s("path").is_empty() { "the workspace".into() } else { s("path") }),
        "read_file" => format!("Read {}", s("path")),
        "write_file" => format!("Wrote {}", s("path")),
        "run_command" => format!("Run `{}`", s("command")),
        "web_search" => format!("Searched the web for \"{}\"", s("query")),
        "web_fetch" => format!("Opened {}", s("url")),
        "browser_open" => format!("Opened {} in the browser", s("url")),
        "browser_snapshot" => "Read the browser page".into(),
        "browser_click" => format!("Clicked element {}", args.get("ref").map(|v| v.to_string()).unwrap_or_default()),
        "browser_type" => format!("Typed into element {}", args.get("ref").map(|v| v.to_string()).unwrap_or_default()),
        "browser_press" => format!("Pressed {}", s("key")),
        "browser_scroll" => format!("Scrolled {}", s("direction")),
        "browser_back" => "Went back in the browser".into(),
        n if n.starts_with("mcp__") => {
            let mut parts = n.splitn(3, "__");
            parts.next();
            let server = parts.next().unwrap_or("");
            let tool = parts.next().unwrap_or("");
            format!("Used {tool} on {server}")
        }
        n => format!("Used {n}"),
    }
}
