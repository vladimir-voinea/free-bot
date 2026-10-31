//! Builds what an agent sees: its system prompt (identity, team, where it is,
//! memory, skills, open tasks) and the thread history from its point of view.

use serde_json::{Value, json};

use super::engine::FleetState;
use super::model::{Agent, Author, TaskStatus, Thread, ThreadKind, ThreadMessage, ToolStatus};

const HISTORY_LIMIT: usize = 80;
const TOOL_RESULT_IN_HISTORY: usize = 6000;

pub fn agent_name(st: &FleetState, author: &Author) -> String {
    match author {
        Author::User => user_name(st),
        Author::Agent(id) => st.agents.iter().find(|a| &a.id == id).map(|a| a.name.clone()).unwrap_or_else(|| "A removed bot".into()),
        Author::System => "System".into(),
    }
}

pub fn user_name(st: &FleetState) -> String {
    let n = st.settings.display_name.trim();
    if n.is_empty() { "the user".into() } else { n.to_string() }
}

pub fn system_prompt(st: &FleetState, agent: &Agent, thread: &Thread) -> String {
    let user = user_name(st);
    let now = chrono::Local::now();
    let mut s = format!(
        "You are {name}{title}, an AI agent in Free Bot. You work for {user} as part of a team of AI agents, like a capable colleague.\n\nYour job: {desc}\n",
        name = agent.name,
        title = if agent.title.trim().is_empty() { String::new() } else { format!(", {}", agent.title.trim()) },
        desc = if agent.description.trim().is_empty() { "help with whatever is asked." } else { agent.description.trim() },
    );
    if !agent.instructions.trim().is_empty() {
        s.push_str(&format!("\nStanding instructions:\n{}\n", agent.instructions.trim()));
    }
    if !st.settings.custom_instructions.trim().is_empty() {
        s.push_str(&format!("\nInstructions from {user} for every agent:\n{}\n", st.settings.custom_instructions.trim()));
    }
    s.push_str(&format!("\nCurrent date and time: {}.\n", now.format("%A, %B %-d, %Y %H:%M")));

    // Team.
    let others: Vec<&Agent> = st.agents.iter().filter(|a| a.id != agent.id).collect();
    if others.is_empty() {
        s.push_str("\n## Team\nYou are the only agent right now.\n");
    } else {
        s.push_str("\n## Team\nThese are your teammates. Their descriptions say what they own:\n");
        for a in &others {
            s.push_str(&format!("- {}: {}\n", a.label(), if a.description.is_empty() { "(no description)" } else { &a.description }));
        }
    }
    let channels: Vec<String> = st
        .threads
        .iter()
        .filter_map(|t| match &t.kind {
            ThreadKind::Channel { name, members, .. } if members.contains(&agent.id) => Some(format!("#{name}")),
            _ => None,
        })
        .collect();
    if !channels.is_empty() {
        s.push_str(&format!("Channels you are in: {}.\n", channels.join(", ")));
    }

    // Where the conversation happens.
    s.push_str("\n## This conversation\n");
    match &thread.kind {
        ThreadKind::Dm { .. } => s.push_str(&format!("You are in a direct chat with {user}. Reply to them directly.\n")),
        ThreadKind::Channel { name, members, lead } => {
            let names: Vec<String> = members.iter().filter_map(|m| st.agents.iter().find(|a| &a.id == m)).map(|a| a.name.clone()).collect();
            s.push_str(&format!(
                "You are in the group channel #{name} with {user} and {}. Messages from others are prefixed with the author's name in brackets. Mention a member with @Name to hand them work here. Keep replies short and only answer what is meant for you; if a message is for someone else, stay quiet by replying with exactly NO_REPLY.\n",
                names.join(", ")
            ));
            if lead.as_deref() == Some(agent.id.as_str()) {
                s.push_str("You lead this channel: you answer messages that mention nobody, and you coordinate the others.\n");
            }
        }
        ThreadKind::AgentDm { a, b } => {
            let other = if a == &agent.id { b } else { a };
            s.push_str(&format!(
                "This is a private chat between you and your teammate {}. {user} can read it. Do what is asked, then reply with the result. Your final reply is delivered to them.\n",
                agent_name(st, &Author::Agent(other.clone()))
            ));
        }
    }

    s.push_str(&format!(
        "\n## How to work\n- Finish work end to end with your tools instead of describing what you would do.\n- If a task belongs to a teammate (see their descriptions), hand it over: delegate_task waits for their result, send_message does not wait and their reply comes back to you later.\n- Involve {user} only for decisions, approvals, or information you cannot get. Outside your direct chat with them, use ask_user.\n- Remember lasting facts and preferences with remember. Do not store secrets.\n- Before a procedure you have a skill for, load it with use_skill. After you learn a repeatable procedure or get corrected, save or update a skill with save_skill.\n- Be concise. Use Markdown when it helps.\n"
    ));

    // Memory.
    let mems: Vec<_> = st.memories.iter().filter(|m| m.agent.is_none() || m.agent.as_deref() == Some(&agent.id)).collect();
    if !mems.is_empty() && agent.tools.memory {
        s.push_str("\n## What you remember\n");
        for m in mems.iter().rev().take(40).rev() {
            let scope = if m.agent.is_none() { "team" } else { "you" };
            s.push_str(&format!("- [{}] ({scope}) {}\n", short_id(&m.id), m.text));
        }
    }

    // Skills.
    if agent.tools.skills && !st.skills.is_empty() {
        s.push_str("\n## Skills you can load with use_skill\n");
        for k in &st.skills {
            s.push_str(&format!("- {}: {}\n", k.name, k.description));
        }
    }

    // Tasks.
    let tasks: Vec<_> = st
        .tasks
        .iter()
        .filter(|t| t.status != TaskStatus::Done && (t.to == agent.id || t.from == Author::Agent(agent.id.clone())))
        .collect();
    if !tasks.is_empty() {
        s.push_str("\n## Open tasks\n");
        for t in tasks {
            let dir = if t.to == agent.id {
                format!("assigned to you by {}", agent_name(st, &t.from))
            } else {
                format!("you assigned to {}", agent_name(st, &Author::Agent(t.to.clone())))
            };
            s.push_str(&format!("- [{}] {} ({dir}, {:?})\n", short_id(&t.id), t.title, t.status));
        }
    }
    s
}

pub fn short_id(id: &str) -> &str {
    id.split('-').next().unwrap_or(id)
}

/// The thread from `agent`'s point of view as OpenAI messages.
pub fn history(st: &FleetState, agent: &Agent, thread: &Thread, native_tools: bool) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let msgs: Vec<&ThreadMessage> = thread.messages.iter().filter(|m| !m.streaming).collect();
    let start = msgs.len().saturating_sub(HISTORY_LIMIT);
    let in_dm = matches!(thread.kind, ThreadKind::Dm { .. });
    for m in &msgs[start..] {
        match &m.author {
            Author::Agent(id) if id == &agent.id => {
                let calls: Vec<_> = m.tool_calls.iter().filter(|c| c.status != ToolStatus::Running).collect();
                if native_tools && !calls.is_empty() {
                    let tc: Vec<Value> = calls
                        .iter()
                        .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments}}))
                        .collect();
                    out.push(json!({"role": "assistant", "content": m.content, "tool_calls": tc}));
                    for c in calls {
                        out.push(json!({"role": "tool", "tool_call_id": c.id, "content": result_text(c)}));
                    }
                } else if !calls.is_empty() {
                    let mut text = m.content.clone();
                    for c in &calls {
                        text.push_str(&format!("\n<tool_call>{{\"name\": \"{}\", \"arguments\": {}}}</tool_call>", c.name, c.arguments));
                    }
                    out.push(json!({"role": "assistant", "content": text.trim()}));
                    let results: Vec<String> = calls.iter().map(|c| format!("[Result of {}]\n{}", c.name, result_text(c))).collect();
                    out.push(json!({"role": "user", "content": results.join("\n\n")}));
                } else if !m.content.trim().is_empty() {
                    out.push(json!({"role": "assistant", "content": m.content}));
                }
            }
            author => {
                if m.content.trim().is_empty() {
                    continue;
                }
                let text = if in_dm && *author == Author::User {
                    m.content.clone()
                } else {
                    format!("[{}]: {}", agent_name(st, author), m.content)
                };
                push_user(&mut out, text);
            }
        }
    }
    if !matches!(out.last().and_then(|v| v.get("role")).and_then(Value::as_str), Some("user") | Some("tool")) {
        push_user(&mut out, "[System]: Continue.".into());
    }
    out
}

fn result_text(c: &super::model::ToolCall) -> String {
    let r = match c.status {
        ToolStatus::Denied => "The user denied this action.".to_string(),
        _ => c.result.clone().unwrap_or_else(|| "(no result)".into()),
    };
    truncate(&r, TOOL_RESULT_IN_HISTORY)
}

/// Some chat templates require alternating roles, so consecutive user messages are merged.
fn push_user(out: &mut Vec<Value>, text: String) {
    if let Some(last) = out.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("user")
        && let Some(prev) = last.get("content").and_then(Value::as_str)
    {
        let merged = format!("{prev}\n\n{text}");
        last["content"] = Value::String(merged);
        return;
    }
    out.push(json!({"role": "user", "content": text}));
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n… ({} more characters)", &s[..cut], s.len() - cut)
}
