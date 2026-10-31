//! One agent turn: call the model, stream its reply into the thread, run the
//! tools it asks for (pausing for approval when needed), feed results back,
//! and repeat until it answers without tool calls.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use super::engine::{Decision, Fleet, Wake};
use super::llm::{self, Delta, LlmError, LlmRequest};
use super::model::{ApprovalMode, Author, ThreadMessage, ToolCall, ToolMode, ToolStatus};
use super::prompt;
use super::tools::{self, Risk, ToolCtx};

/// Tool calls the model may make in one step before we stop it.
const MAX_CALLS_PER_STEP: usize = 8;

pub fn run_turn(fleet: &Fleet, w: &Wake, cancel: &AtomicBool) {
    let max_steps = fleet.lock().config.max_steps.max(1);
    let mut native = None;
    for _step in 0..max_steps {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        match one_step(fleet, w, cancel, &mut native) {
            StepOutcome::Continue => continue,
            StepOutcome::Finished => return,
        }
    }
    let mut st = fleet.lock();
    st.push_message(
        &w.thread,
        ThreadMessage::new(Author::System, format!("Stopped after {max_steps} steps. Send a message to let it continue.")),
    );
}

enum StepOutcome {
    Continue,
    Finished,
}

fn one_step(fleet: &Fleet, w: &Wake, cancel: &AtomicBool, native: &mut Option<bool>) -> StepOutcome {
    // Snapshot everything the request needs, then release the lock.
    let (req_base, agent, msg_id, defs) = {
        let mut st = fleet.lock();
        let Some(agent) = st.agent(&w.agent).cloned() else { return StepOutcome::Finished };
        let Some(thread) = st.thread(&w.thread).cloned() else { return StepOutcome::Finished };
        let model =
            agent.model.clone().filter(|m| st.settings.provider(&m.provider_id).is_some()).or_else(|| st.settings.effective_model());
        let Some(model) = model else {
            let mut m = ThreadMessage::new(Author::Agent(agent.id.clone()), "");
            m.error = Some("No model is set up. Add a provider in Settings → Providers, then try again.".into());
            st.push_message(&w.thread, m);
            return StepOutcome::Finished;
        };
        let provider = st.settings.provider(&model.provider_id).cloned().expect("checked above");
        let defs = tools::definitions(fleet, &st, &agent);
        let use_native = native.unwrap_or(agent.tool_mode != ToolMode::Prompt);
        *native = Some(use_native);
        let mut system = prompt::system_prompt(&st, &agent, &thread);
        let tool_json: Vec<_> = defs.iter().map(|d| d.to_openai()).collect();
        if !use_native && !tool_json.is_empty() {
            system.push_str(&llm::prompt_tool_instructions(&tool_json));
        }
        let mut messages = vec![serde_json::json!({"role": "system", "content": system})];
        messages.extend(prompt::history(&st, &agent, &thread, use_native));
        let mut msg = ThreadMessage::new(Author::Agent(agent.id.clone()), "");
        msg.streaming = true;
        msg.model = model.model.clone();
        let msg_id = st.push_message(&w.thread, msg).unwrap_or_default();
        let req = LlmRequest {
            base_url: provider.base_url,
            api_key: provider.api_key,
            model: model.model,
            messages,
            tools: tool_json,
            native_tools: use_native,
            temperature: agent.temperature.or(st.settings.temperature),
            max_tokens: st.settings.max_tokens,
        };
        (req, agent, msg_id, defs)
    };
    fleet.set_status(&w.agent, &w.thread, "Thinking");

    let mut req = req_base;
    let result = loop {
        let mut on_delta = |d: Delta| {
            let mut st = fleet.lock();
            if let Some(m) = find_msg(&mut st, &w.thread, &msg_id) {
                match d {
                    Delta::Content(t) => m.content.push_str(&t),
                    Delta::Reasoning(r) => m.reasoning.push_str(&r),
                }
            }
            drop(st);
            fleet.wake_ui();
        };
        match llm::run_step(&req, cancel, &mut on_delta) {
            Err(LlmError::ToolsUnsupported(_)) if req.native_tools && agent.tool_mode == ToolMode::Auto => {
                // Fall back to describing tools in the prompt.
                *native = Some(false);
                req.native_tools = false;
                if let Some(sys) = req.messages.first_mut() {
                    let text = sys["content"].as_str().unwrap_or_default().to_string() + &llm::prompt_tool_instructions(&req.tools);
                    sys["content"] = serde_json::Value::String(text);
                }
                let st = fleet.lock();
                let agent_now = st.agent(&w.agent).cloned().unwrap_or(agent.clone());
                if let Some(thread) = st.thread(&w.thread) {
                    let hist = prompt::history(&st, &agent_now, thread, false);
                    req.messages.truncate(1);
                    req.messages.extend(hist);
                }
                drop(st);
                let mut st = fleet.lock();
                if let Some(m) = find_msg(&mut st, &w.thread, &msg_id) {
                    m.content.clear();
                    m.reasoning.clear();
                }
                continue;
            }
            other => break other,
        }
    };

    let step = match result {
        Ok(step) => step,
        Err(e) => {
            let mut st = fleet.lock();
            if let Some(m) = find_msg(&mut st, &w.thread, &msg_id) {
                m.streaming = false;
                if !matches!(e, LlmError::Cancelled) || (m.content.is_empty() && m.reasoning.is_empty()) {
                    m.error = Some(e.to_string());
                }
            }
            st.mark_thread(&w.thread);
            return StepOutcome::Finished;
        }
    };

    let mut calls = step.tool_calls;
    calls.truncate(MAX_CALLS_PER_STEP);
    {
        let mut st = fleet.lock();
        let no_reply = step.content.trim() == "NO_REPLY";
        if let Some(m) = find_msg(&mut st, &w.thread, &msg_id) {
            m.content = if no_reply { String::new() } else { step.content.clone() };
            m.streaming = false;
            m.tool_calls = calls
                .iter()
                .map(|c| ToolCall {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    arguments: c.arguments.clone(),
                    result: None,
                    status: ToolStatus::Running,
                    link: None,
                })
                .collect();
        }
        // An empty message (nothing said, nothing done) is noise in the thread.
        if calls.is_empty()
            && let Some(t) = st.thread_mut(&w.thread)
        {
            t.messages.retain(|m| m.id != msg_id || !m.content.trim().is_empty() || !m.reasoning.trim().is_empty() || m.error.is_some());
        }
        st.mark_thread(&w.thread);
    }
    fleet.wake_ui();
    if calls.is_empty() {
        return StepOutcome::Finished;
    }

    let mut end_turn = false;
    for c in &calls {
        if cancel.load(Ordering::Relaxed) {
            set_call(fleet, w, &msg_id, &c.id, ToolStatus::Error, Some("Stopped.".into()), None);
            continue;
        }
        let def = defs.iter().find(|d| d.name == c.name);
        let Some(def) = def else {
            set_call(fleet, w, &msg_id, &c.id, ToolStatus::Error, Some(format!("Unknown tool \"{}\".", c.name)), None);
            continue;
        };
        let args: serde_json::Value = match serde_json::from_str(&c.arguments) {
            Ok(v) => v,
            Err(_) => {
                set_call(fleet, w, &msg_id, &c.id, ToolStatus::Error, Some("The arguments were not valid JSON.".into()), None);
                continue;
            }
        };

        if needs_approval(fleet, &w.agent, def.risk, &def.name) {
            set_call(fleet, w, &msg_id, &c.id, ToolStatus::AwaitingApproval, None, None);
            fleet.set_status(&w.agent, &w.thread, "Waiting for your approval");
            {
                let mut st = fleet.lock();
                let name = st.agent(&w.agent).map(|a| a.name.clone()).unwrap_or_default();
                let thread = w.thread.clone();
                st.notify(format!("{name} needs your approval"), tools::describe_call(&def.name, &args), Some(thread));
            }
            let rx = fleet.await_approval(&c.id);
            let decision = loop {
                match rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(d) => break d,
                    Err(RecvTimeoutError::Timeout) if !cancel.load(Ordering::Relaxed) => continue,
                    _ => break Decision::Deny,
                }
            };
            fleet.lock().approvals.remove(&c.id);
            match decision {
                Decision::Deny => {
                    set_call(fleet, w, &msg_id, &c.id, ToolStatus::Denied, None, None);
                    continue;
                }
                Decision::AlwaysAllow => {
                    let mut st = fleet.lock();
                    if let Some(a) = st.agent_mut(&w.agent)
                        && !a.always_allow.contains(&def.name)
                    {
                        a.always_allow.push(def.name.clone());
                        st.dirty.agents = true;
                    }
                }
                Decision::Approve => {}
            }
            set_call(fleet, w, &msg_id, &c.id, ToolStatus::Running, None, None);
        }

        fleet.set_status(&w.agent, &w.thread, def.status.clone());
        let ctx = ToolCtx { fleet, agent: &agent, thread: &w.thread, wake: w, cancel, call_id: &c.id };
        let out = tools::execute(&ctx, &def.name, &args);
        let status = if out.error { ToolStatus::Error } else { ToolStatus::Done };
        set_call(fleet, w, &msg_id, &c.id, status, Some(prompt::truncate(&out.text, 20_000)), out.link);
        end_turn |= out.end_turn;
    }
    if end_turn || cancel.load(Ordering::Relaxed) { StepOutcome::Finished } else { StepOutcome::Continue }
}

fn needs_approval(fleet: &Fleet, agent: &str, risk: Risk, name: &str) -> bool {
    let st = fleet.lock();
    let Some(a) = st.agent(agent) else { return false };
    if a.always_allow.iter().any(|t| t == name) {
        return false;
    }
    match a.approval {
        ApprovalMode::Never => false,
        ApprovalMode::Risky => risk == Risk::Risky,
        ApprovalMode::Everything => risk != Risk::Internal,
    }
}

fn find_msg<'a>(st: &'a mut super::engine::FleetState, thread: &str, msg: &str) -> Option<&'a mut ThreadMessage> {
    st.thread_mut(thread)?.messages.iter_mut().find(|m| m.id == msg)
}

fn set_call(fleet: &Fleet, w: &Wake, msg_id: &str, call_id: &str, status: ToolStatus, result: Option<String>, link: Option<String>) {
    let mut st = fleet.lock();
    if let Some(m) = find_msg(&mut st, &w.thread, msg_id)
        && let Some(c) = m.tool_calls.iter_mut().find(|c| c.id == call_id)
    {
        c.status = status;
        if result.is_some() {
            c.result = result;
        }
        if link.is_some() {
            c.link = link;
        }
    }
    st.mark_thread(&w.thread);
    drop(st);
    fleet.wake_ui();
}
