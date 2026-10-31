//! OpenAI-compatible chat client: streaming chat completions, model listing
//! and title generation. Each request runs on its own thread and reports back
//! through a channel, waking the UI with `request_repaint`.

use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde_json::{Value, json};

use crate::store::{AttachmentData, Message, Provider, Role};

/// Something that can be woken up when new data arrives (the egui context in the app).
pub trait Waker: Send + Sync + 'static {
    fn wake(&self);
}

impl Waker for egui::Context {
    fn wake(&self) {
        self.request_repaint();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Content(String),
    Reasoning(String),
    Done,
    Error(String),
}

pub struct StreamHandle {
    pub rx: Receiver<StreamEvent>,
    cancel: Arc<AtomicBool>,
}

impl StreamHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub system_prompt: Option<String>,
    pub messages: Vec<Value>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub think: bool,
}

pub fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}/{}", base_url.trim().trim_end_matches('/'), path.trim_start_matches('/'))
}

fn client(timeout: Option<Duration>) -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(timeout)
        .user_agent(concat!("FreeBot/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("failed to build HTTP client")
}

fn authed(rb: reqwest::blocking::RequestBuilder, api_key: &str) -> reqwest::blocking::RequestBuilder {
    if api_key.trim().is_empty() { rb } else { rb.bearer_auth(api_key.trim()) }
}

/// Convert stored messages into the OpenAI `messages` array (without the system prompt).
pub fn build_messages(history: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in history {
        let v = m.current();
        match m.role {
            Role::Assistant => {
                if v.error.is_some() && v.content.is_empty() {
                    continue;
                }
                out.push(json!({ "role": "assistant", "content": v.content }));
            }
            Role::User => {
                let mut text = v.content.clone();
                let mut images = Vec::new();
                for a in &m.attachments {
                    match &a.data {
                        AttachmentData::Text { content } => {
                            text.push_str(&format!("\n\n<file name=\"{}\">\n{}\n</file>", a.name, content));
                        }
                        AttachmentData::Image { mime, base64 } => {
                            images.push(json!({
                                "type": "image_url",
                                "image_url": { "url": format!("data:{mime};base64,{base64}") }
                            }));
                        }
                    }
                }
                if images.is_empty() {
                    out.push(json!({ "role": "user", "content": text }));
                } else {
                    let mut parts = vec![json!({ "type": "text", "text": text })];
                    parts.extend(images);
                    out.push(json!({ "role": "user", "content": parts }));
                }
            }
        }
    }
    out
}

pub fn build_body(req: &ChatRequest, with_reasoning_param: bool) -> Value {
    let mut messages = Vec::new();
    if let Some(sys) = req.system_prompt.as_ref().filter(|s| !s.trim().is_empty()) {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    messages.extend(req.messages.iter().cloned());
    let mut body = json!({ "model": req.model, "messages": messages, "stream": true });
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(m) = req.max_tokens {
        body["max_tokens"] = json!(m);
    }
    if req.think && with_reasoning_param {
        body["reasoning_effort"] = json!("high");
    }
    body
}

/// Pull a readable message out of an error response body.
pub fn error_message(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("error"))
                .or_else(|| v.get("message"))
                .or_else(|| v.get("detail"))
                .map(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string()))
        })
        .unwrap_or_else(|| body.trim().chars().take(400).collect());
    let hint = match status {
        401 | 403 => " Check the API key in Settings → Providers.",
        404 => " Check the base URL and model name in Settings → Providers.",
        429 => " The provider is rate limiting requests. Try again shortly.",
        _ => "",
    };
    if detail.is_empty() { format!("Request failed with HTTP {status}.{hint}") } else { format!("HTTP {status}: {detail}{hint}") }
}

/// Splits `<think>…</think>` blocks that some local models emit inline
/// into the reasoning channel.
#[derive(Default)]
pub struct ThinkSplitter {
    in_think: bool,
    pending: String,
}

impl ThinkSplitter {
    pub fn push(&mut self, chunk: &str, out: &mut Vec<StreamEvent>) {
        self.pending.push_str(chunk);
        loop {
            let tag = if self.in_think { "</think>" } else { "<think>" };
            if let Some(pos) = self.pending.find(tag) {
                let before: String = self.pending[..pos].to_string();
                self.emit(before, out);
                self.pending.drain(..pos + tag.len());
                self.in_think = !self.in_think;
                continue;
            }
            // Keep a possible partial tag at the end for the next chunk.
            let keep = partial_suffix(&self.pending, tag);
            let cut = self.pending.len() - keep;
            let text: String = self.pending[..cut].to_string();
            self.pending.drain(..cut);
            self.emit(text, out);
            break;
        }
    }

    pub fn finish(&mut self, out: &mut Vec<StreamEvent>) {
        let rest = std::mem::take(&mut self.pending);
        self.emit(rest, out);
    }

    fn emit(&self, text: String, out: &mut Vec<StreamEvent>) {
        if text.is_empty() {
            return;
        }
        out.push(if self.in_think { StreamEvent::Reasoning(text) } else { StreamEvent::Content(text) });
    }
}

fn partial_suffix(s: &str, tag: &str) -> usize {
    (1..tag.len()).rev().find(|&n| s.len() >= n && s.is_char_boundary(s.len() - n) && tag.starts_with(&s[s.len() - n..])).unwrap_or(0)
}

/// Parse one SSE `data:` payload into events. Returns `true` when the stream is finished.
pub fn parse_sse_data(data: &str, splitter: &mut ThinkSplitter, out: &mut Vec<StreamEvent>) -> bool {
    let data = data.trim();
    if data == "[DONE]" {
        return true;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else { return false };
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| err.to_string());
        out.push(StreamEvent::Error(msg));
        return true;
    }
    if let Some(choice) = v.pointer("/choices/0") {
        let delta = choice.get("delta").or_else(|| choice.get("message"));
        if let Some(delta) = delta {
            for key in ["reasoning_content", "reasoning"] {
                if let Some(r) = delta.get(key).and_then(Value::as_str)
                    && !r.is_empty()
                {
                    out.push(StreamEvent::Reasoning(r.to_string()));
                }
            }
            if let Some(c) = delta.get("content").and_then(Value::as_str) {
                splitter.push(c, out);
            }
        }
    }
    false
}

/// Start a streaming chat completion on a background thread.
pub fn stream_chat(req: ChatRequest, waker: Arc<dyn Waker>) -> StreamHandle {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        let send = |e: StreamEvent| {
            let _ = tx.send(e);
            waker.wake();
        };
        match run_stream(&req, &c2, &tx, &*waker, req.think) {
            Ok(()) => send(StreamEvent::Done),
            Err(RetryOr::RetryWithoutReasoning) => match run_stream(&req, &c2, &tx, &*waker, false) {
                Ok(()) => send(StreamEvent::Done),
                Err(RetryOr::Fail(e)) => send(StreamEvent::Error(e)),
                Err(RetryOr::RetryWithoutReasoning) => send(StreamEvent::Error("The provider rejected the request.".into())),
            },
            Err(RetryOr::Fail(e)) => send(StreamEvent::Error(e)),
        }
    });
    StreamHandle { rx, cancel }
}

enum RetryOr {
    RetryWithoutReasoning,
    Fail(String),
}

fn run_stream(
    req: &ChatRequest,
    cancel: &AtomicBool,
    tx: &Sender<StreamEvent>,
    waker: &dyn Waker,
    with_reasoning: bool,
) -> Result<(), RetryOr> {
    let body = build_body(req, with_reasoning);
    let resp = authed(client(None).post(endpoint(&req.base_url, "chat/completions")), &req.api_key)
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .map_err(|e| RetryOr::Fail(format!("Could not reach the provider: {}", root_cause(&e))))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        if with_reasoning && status.as_u16() == 400 && text.to_lowercase().contains("reasoning") {
            return Err(RetryOr::RetryWithoutReasoning);
        }
        return Err(RetryOr::Fail(error_message(status.as_u16(), &text)));
    }

    let is_sse =
        resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("event-stream"));

    let mut splitter = ThinkSplitter::default();
    let mut events = Vec::new();
    let flush = |events: &mut Vec<StreamEvent>| {
        if events.is_empty() {
            return false;
        }
        let mut failed = false;
        for e in events.drain(..) {
            failed |= matches!(e, StreamEvent::Error(_));
            let _ = tx.send(e);
        }
        waker.wake();
        failed
    };

    if !is_sse {
        // Some servers ignore `stream: true` and answer with one JSON body.
        let mut text = String::new();
        let mut resp = resp;
        resp.read_to_string(&mut text).map_err(|e| RetryOr::Fail(e.to_string()))?;
        parse_sse_data(&text, &mut splitter, &mut events);
        splitter.finish(&mut events);
        flush(&mut events);
        return Ok(());
    }

    let reader = BufReader::new(resp);
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line.map_err(|e| RetryOr::Fail(format!("Connection lost: {e}")))?;
        let Some(data) = line.strip_prefix("data:") else { continue };
        let done = parse_sse_data(data, &mut splitter, &mut events);
        if flush(&mut events) {
            // An error event was already delivered; stop without a Done.
            return Ok(());
        }
        if done {
            break;
        }
    }
    splitter.finish(&mut events);
    flush(&mut events);
    Ok(())
}

fn root_cause(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg = s.to_string();
        src = s.source();
    }
    msg
}

/// List the models a provider offers (`GET /models`).
pub fn fetch_models(provider: &Provider) -> Result<Vec<String>, String> {
    let resp = authed(client(Some(Duration::from_secs(30))).get(endpoint(&provider.base_url, "models")), &provider.api_key)
        .send()
        .map_err(|e| format!("Could not reach the provider: {}", root_cause(&e)))?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(error_message(status.as_u16(), &text));
    }
    parse_models(&text)
}

pub fn parse_models(text: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("Unexpected response: {e}"))?;
    let list = v.get("data").or_else(|| v.get("models")).and_then(Value::as_array).ok_or("Unexpected response: no model list")?;
    let mut ids: Vec<String> =
        list.iter().filter_map(|m| m.get("id").or_else(|| m.get("name")).and_then(Value::as_str).map(str::to_string)).collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Ask the model for a short conversation title. Blocking; call from a thread.
pub fn generate_title(base_url: &str, api_key: &str, model: &str, prompt: &str, answer: &str) -> Option<String> {
    let clip = |s: &str| s.chars().take(1500).collect::<String>();
    let body = json!({
        "model": model,
        "stream": false,
        "messages": [
            { "role": "system", "content": "You write very short titles for chat conversations. Reply with only the title: 2 to 6 words, no quotes, no trailing punctuation." },
            { "role": "user", "content": format!("User: {}\n\nAssistant: {}\n\nTitle:", clip(prompt), clip(answer)) }
        ]
    });
    let resp =
        authed(client(Some(Duration::from_secs(60))).post(endpoint(base_url, "chat/completions")), api_key).json(&body).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().ok()?;
    let raw = v.pointer("/choices/0/message/content")?.as_str()?;
    clean_title(raw)
}

pub fn clean_title(raw: &str) -> Option<String> {
    // Drop any reasoning block, then keep the first non-empty line.
    let raw = match raw.rfind("</think>") {
        Some(i) => &raw[i + "</think>".len()..],
        None => raw,
    };
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches("Title:").trim();
    let t: String = line
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '*' || c == '#' || c == '.' || c.is_whitespace())
        .chars()
        .take(60)
        .collect();
    if t.is_empty() { None } else { Some(t) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Attachment, Message};

    fn collect(chunks: &[&str]) -> Vec<StreamEvent> {
        let mut s = ThinkSplitter::default();
        let mut out = Vec::new();
        for c in chunks {
            s.push(c, &mut out);
        }
        s.finish(&mut out);
        // Merge adjacent events of the same kind for easy comparison.
        let mut merged: Vec<StreamEvent> = Vec::new();
        for e in out {
            match (merged.last_mut(), e) {
                (Some(StreamEvent::Content(a)), StreamEvent::Content(b)) => a.push_str(&b),
                (Some(StreamEvent::Reasoning(a)), StreamEvent::Reasoning(b)) => a.push_str(&b),
                (_, e) => merged.push(e),
            }
        }
        merged
    }

    #[test]
    fn think_tags_split_across_chunks() {
        let ev = collect(&["<thi", "nk>pondering", " more</th", "ink>The answer", " is 4."]);
        assert_eq!(ev, vec![StreamEvent::Reasoning("pondering more".into()), StreamEvent::Content("The answer is 4.".into())]);
    }

    #[test]
    fn plain_content_passes_through() {
        assert_eq!(collect(&["a < b", " and c <t"]), vec![StreamEvent::Content("a < b and c <t".into())]);
    }

    #[test]
    fn sse_parsing() {
        let mut s = ThinkSplitter::default();
        let mut out = Vec::new();
        assert!(!parse_sse_data(r#" {"choices":[{"delta":{"reasoning_content":"hmm"}}]}"#, &mut s, &mut out));
        assert!(!parse_sse_data(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#, &mut s, &mut out));
        assert!(!parse_sse_data(r#"{"choices":[{"delta":{}, "finish_reason":"stop"}]}"#, &mut s, &mut out));
        assert!(parse_sse_data(" [DONE]", &mut s, &mut out));
        assert_eq!(out, vec![StreamEvent::Reasoning("hmm".into()), StreamEvent::Content("Hi".into())]);

        let mut out = Vec::new();
        assert!(parse_sse_data(r#"{"error":{"message":"bad key"}}"#, &mut s, &mut out));
        assert_eq!(out, vec![StreamEvent::Error("bad key".into())]);
    }

    #[test]
    fn body_includes_system_prompt_and_images() {
        let mut user = Message::user("What is this?".into(), vec![], true);
        user.attachments.push(Attachment {
            id: "1".into(),
            name: "a.png".into(),
            data: AttachmentData::Image { mime: "image/png".into(), base64: "AAAA".into() },
        });
        user.attachments.push(Attachment {
            id: "2".into(),
            name: "notes.txt".into(),
            data: AttachmentData::Text { content: "hello".into() },
        });
        let req = ChatRequest {
            base_url: "http://x/v1/".into(),
            api_key: String::new(),
            model: "m".into(),
            system_prompt: Some("Be brief".into()),
            messages: build_messages(&[user]),
            temperature: Some(0.5),
            max_tokens: None,
            think: true,
        };
        let body = build_body(&req, true);
        assert_eq!(body["messages"][0]["role"], "system");
        let parts = body["messages"][1]["content"].as_array().unwrap();
        assert!(parts[0]["text"].as_str().unwrap().contains("<file name=\"notes.txt\">"));
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(body["reasoning_effort"], "high");
        assert!(build_body(&req, false).get("reasoning_effort").is_none());
        assert_eq!(endpoint(&req.base_url, "chat/completions"), "http://x/v1/chat/completions");
    }

    #[test]
    fn model_lists() {
        let v = parse_models(r#"{"data":[{"id":"b"},{"id":"a"},{"id":"a"}]}"#).unwrap();
        assert_eq!(v, vec!["a", "b"]);
        assert!(parse_models("{}").is_err());
    }

    #[test]
    fn errors_and_titles() {
        assert!(error_message(401, r#"{"error":{"message":"Invalid key"}}"#).contains("Invalid key"));
        assert!(error_message(401, "").contains("API key"));
        assert_eq!(clean_title("\"Rust GUI Options.\"\n"), Some("Rust GUI Options".into()));
        assert_eq!(clean_title("<think>x</think>\nTitle: Hello"), Some("Hello".into()));
        assert_eq!(clean_title("  \n"), None);
    }
}
