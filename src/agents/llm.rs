//! One model call of an agent turn: a streaming `/chat/completions` request
//! with tools. Supports native function calling and a prompt-based fallback
//! (`<tool_call>{...}</tool_call>`) for endpoints and models without it.

use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::api::{self, StreamEvent, ThinkSplitter};

pub struct LlmRequest {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub messages: Vec<Value>,
    /// OpenAI tool definitions (`{"type":"function","function":{...}}`).
    pub tools: Vec<Value>,
    pub native_tools: bool,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Default)]
pub struct LlmStep {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<RawToolCall>,
}

pub enum Delta {
    Content(String),
    Reasoning(String),
}

#[derive(Debug)]
pub enum LlmError {
    /// The endpoint rejected the `tools` parameter.
    ToolsUnsupported(String),
    Cancelled,
    Other(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::ToolsUnsupported(e) | LlmError::Other(e) => write!(f, "{e}"),
            LlmError::Cancelled => write!(f, "Stopped."),
        }
    }
}

pub fn build_body(req: &LlmRequest) -> Value {
    let mut body = json!({ "model": req.model, "messages": req.messages, "stream": true });
    if req.native_tools && !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.clone());
        body["tool_choice"] = json!("auto");
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(m) = req.max_tokens {
        body["max_tokens"] = json!(m);
    }
    body
}

/// Accumulates streamed `tool_calls` deltas, which arrive in pieces keyed by index.
#[derive(Default)]
pub struct ToolCallAccumulator {
    calls: Vec<RawToolCall>,
}

impl ToolCallAccumulator {
    pub fn push(&mut self, deltas: &Value) {
        let Some(arr) = deltas.as_array() else { return };
        for (pos, d) in arr.iter().enumerate() {
            let idx = d.get("index").and_then(Value::as_u64).map(|i| i as usize).unwrap_or(pos);
            while self.calls.len() <= idx {
                self.calls.push(RawToolCall::default());
            }
            let c = &mut self.calls[idx];
            if let Some(id) = d.get("id").and_then(Value::as_str)
                && !id.is_empty()
            {
                c.id = id.to_string();
            }
            if let Some(f) = d.get("function") {
                if let Some(n) = f.get("name").and_then(Value::as_str) {
                    c.name.push_str(n);
                }
                match f.get("arguments") {
                    Some(Value::String(a)) => c.arguments.push_str(a),
                    Some(v @ Value::Object(_)) => c.arguments.push_str(&v.to_string()),
                    _ => {}
                }
            }
        }
    }

    pub fn finish(self) -> Vec<RawToolCall> {
        self.calls
            .into_iter()
            .filter(|c| !c.name.is_empty())
            .enumerate()
            .map(|(i, mut c)| {
                if c.id.is_empty() {
                    c.id = format!("call_{}_{}", i, crate::store::new_id().split('-').next().unwrap_or("x"));
                }
                if c.arguments.trim().is_empty() {
                    c.arguments = "{}".into();
                }
                c
            })
            .collect()
    }
}

/// Hides `<tool_call>` blocks from the visible stream in prompt mode.
#[derive(Default)]
pub struct ToolTagFilter {
    hidden: bool,
    pending: String,
}

impl ToolTagFilter {
    pub fn push(&mut self, chunk: &str) -> String {
        if self.hidden {
            return String::new();
        }
        self.pending.push_str(chunk);
        if let Some(pos) = self.pending.find("<tool_call") {
            self.hidden = true;
            let out = self.pending[..pos].to_string();
            self.pending.clear();
            return out;
        }
        let tag = "<tool_call";
        let keep = (1..tag.len())
            .rev()
            .find(|&n| {
                self.pending.len() >= n
                    && self.pending.is_char_boundary(self.pending.len() - n)
                    && tag.starts_with(&self.pending[self.pending.len() - n..])
            })
            .unwrap_or(0);
        let cut = self.pending.len() - keep;
        let out = self.pending[..cut].to_string();
        self.pending.drain(..cut);
        out
    }

    pub fn finish(&mut self) -> String {
        if self.hidden { String::new() } else { std::mem::take(&mut self.pending) }
    }
}

/// Pull `<tool_call>{"name":..,"arguments":{..}}</tool_call>` blocks out of text.
/// Returns the text without the blocks and the calls found.
pub fn parse_prompt_tool_calls(text: &str) -> (String, Vec<RawToolCall>) {
    let mut out = String::new();
    let mut calls = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<tool_call>") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "<tool_call>".len()..];
        let (inner, next) = match after.find("</tool_call>") {
            Some(end) => (&after[..end], &after[end + "</tool_call>".len()..]),
            None => (after, ""),
        };
        let inner = inner.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
        if let Ok(v) = serde_json::from_str::<Value>(inner) {
            let name = v.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let args = match v.get("arguments").or_else(|| v.get("parameters")) {
                Some(Value::String(s)) => s.clone(),
                Some(a) => a.to_string(),
                None => "{}".into(),
            };
            if !name.is_empty() {
                calls.push(RawToolCall { id: format!("call_p{}", calls.len()), name, arguments: args });
            }
        }
        rest = next;
    }
    out.push_str(rest);
    (out.trim().to_string(), calls)
}

/// Describe tools in the system prompt for models without native function calling.
pub fn prompt_tool_instructions(tools: &[Value]) -> String {
    let mut s = String::from(
        "\n\n## Calling tools\nYou can call tools. To call one, reply with a block like this and nothing after it:\n<tool_call>{\"name\": \"tool_name\", \"arguments\": {\"arg\": \"value\"}}</tool_call>\nYou may put several blocks in one reply. Tool results come back in the next message. When you are finished, reply normally without any block.\n\nAvailable tools:\n",
    );
    for t in tools {
        if let Some(f) = t.get("function") {
            s.push_str(&format!(
                "- {}: {} Parameters: {}\n",
                f.get("name").and_then(Value::as_str).unwrap_or(""),
                f.get("description").and_then(Value::as_str).unwrap_or(""),
                f.get("parameters").map(|p| p.to_string()).unwrap_or_default()
            ));
        }
    }
    s
}

/// Run one streaming model call. Blocks the calling thread.
pub fn run_step(req: &LlmRequest, cancel: &AtomicBool, on_delta: &mut dyn FnMut(Delta)) -> Result<LlmStep, LlmError> {
    let body = build_body(req);
    let resp = api::authed(api::client(None).post(api::endpoint(&req.base_url, "chat/completions")), &req.api_key)
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .map_err(|e| LlmError::Other(format!("Could not reach the provider: {}", api::root_cause(&e))))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        let lower = text.to_lowercase();
        if req.native_tools && (400..500).contains(&status.as_u16()) && (lower.contains("tool") || lower.contains("function")) {
            return Err(LlmError::ToolsUnsupported(api::error_message(status.as_u16(), &text)));
        }
        return Err(LlmError::Other(api::error_message(status.as_u16(), &text)));
    }
    let is_sse =
        resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("event-stream"));

    let mut step = LlmStep::default();
    let mut acc = ToolCallAccumulator::default();
    let mut splitter = ThinkSplitter::default();
    let mut filter = ToolTagFilter::default();
    let mut raw_content = String::new();
    let prompt_mode = !req.native_tools;

    let mut handle = |v: &Value, step: &mut LlmStep, raw: &mut String, on: &mut dyn FnMut(Delta)| -> Result<(), LlmError> {
        if let Some(err) = v.get("error") {
            let msg = err.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| err.to_string());
            return Err(LlmError::Other(msg));
        }
        let Some(choice) = v.pointer("/choices/0") else { return Ok(()) };
        let Some(delta) = choice.get("delta").or_else(|| choice.get("message")) else { return Ok(()) };
        for key in ["reasoning_content", "reasoning"] {
            if let Some(r) = delta.get(key).and_then(Value::as_str)
                && !r.is_empty()
            {
                step.reasoning.push_str(r);
                on(Delta::Reasoning(r.to_string()));
            }
        }
        if let Some(c) = delta.get("content").and_then(Value::as_str) {
            let mut evs = Vec::new();
            splitter.push(c, &mut evs);
            for e in evs {
                match e {
                    StreamEvent::Reasoning(r) => {
                        step.reasoning.push_str(&r);
                        on(Delta::Reasoning(r));
                    }
                    StreamEvent::Content(t) => {
                        raw.push_str(&t);
                        let visible = if prompt_mode { filter.push(&t) } else { t };
                        if !visible.is_empty() {
                            on(Delta::Content(visible));
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(tc) = delta.get("tool_calls") {
            acc.push(tc);
        }
        Ok(())
    };

    if is_sse {
        for line in BufReader::new(resp).lines() {
            if cancel.load(Ordering::Relaxed) {
                return Err(LlmError::Cancelled);
            }
            let line = line.map_err(|e| LlmError::Other(format!("Connection lost: {e}")))?;
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data == "[DONE]" {
                break;
            }
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                handle(&v, &mut step, &mut raw_content, on_delta)?;
            }
        }
    } else {
        let mut text = String::new();
        let mut resp = resp;
        resp.read_to_string(&mut text).map_err(|e| LlmError::Other(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).map_err(|_| LlmError::Other("The provider sent an unreadable response.".into()))?;
        handle(&v, &mut step, &mut raw_content, on_delta)?;
    }
    let mut tail = Vec::new();
    splitter.finish(&mut tail);
    for e in tail {
        if let StreamEvent::Content(t) = e {
            raw_content.push_str(&t);
            let visible = if prompt_mode { filter.push(&t) } else { t };
            if !visible.is_empty() {
                on_delta(Delta::Content(visible));
            }
        }
    }
    if prompt_mode {
        let rest = filter.finish();
        if !rest.is_empty() {
            on_delta(Delta::Content(rest));
        }
        let (text, calls) = parse_prompt_tool_calls(&raw_content);
        step.content = text;
        step.tool_calls = calls;
    } else {
        step.content = raw_content.trim().to_string();
        step.tool_calls = acc.finish();
    }
    Ok(step)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_streamed_tool_calls() {
        let mut acc = ToolCallAccumulator::default();
        acc.push(&json!([{"index":0,"id":"c1","function":{"name":"read_","arguments":"{\"pa"}}]));
        acc.push(&json!([{"index":0,"function":{"name":"file","arguments":"th\":\"a\"}"}}]));
        acc.push(&json!([{"index":1,"id":"c2","function":{"name":"list_agents","arguments":""}}]));
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, "{\"path\":\"a\"}");
        assert_eq!(calls[1].arguments, "{}");
    }

    #[test]
    fn parses_prompt_tool_calls() {
        let (text, calls) = parse_prompt_tool_calls(
            "Let me check.\n<tool_call>{\"name\":\"read_file\",\"arguments\":{\"path\":\"x.txt\"}}</tool_call>\n<tool_call>{\"name\":\"list_agents\"}</tool_call>",
        );
        assert_eq!(text, "Let me check.");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments, "{\"path\":\"x.txt\"}");
    }

    #[test]
    fn filter_hides_tool_tags() {
        let mut f = ToolTagFilter::default();
        let mut out = f.push("Hello <tool");
        out.push_str(&f.push("_call>{\"name\":1}"));
        out.push_str(&f.finish());
        assert_eq!(out, "Hello ");
    }

    #[test]
    fn body_includes_tools_only_natively() {
        let mut req = LlmRequest {
            base_url: String::new(),
            api_key: String::new(),
            model: "m".into(),
            messages: vec![],
            tools: vec![json!({"type":"function","function":{"name":"x"}})],
            native_tools: true,
            temperature: None,
            max_tokens: None,
        };
        assert!(build_body(&req).get("tools").is_some());
        req.native_tools = false;
        assert!(build_body(&req).get("tools").is_none());
    }
}
