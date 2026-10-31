//! Teamwork: seeing the team, messaging and delegating to other agents,
//! posting in channels, asking the user, tracking tasks, creating agents.

use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::Value;

use super::{Risk, ToolCtx, ToolDef, ToolOutput, schema, str_arg};
use crate::agents::engine::Wake;
use crate::agents::model::{AGENT_COLORS, Agent, Author, Shape, Task, TaskStatus, ThreadKind, ThreadMessage};
use crate::agents::prompt::short_id;
use crate::store::new_id;

const DELEGATE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub fn definitions(has_team: bool) -> Vec<ToolDef> {
    let mut v = vec![
        ToolDef::new(
            "list_agents",
            "List your teammates with their titles, descriptions and whether they are busy.",
            schema(&[]),
            Risk::Internal,
            "Looking at the team",
        ),
        ToolDef::new(
            "post_to_channel",
            "Post a message in a group channel you belong to. Mention members with @Name to hand them work there.",
            schema(&[("channel", "string", "Channel name, for example general", true), ("message", "string", "The message", true)]),
            Risk::Internal,
            "Posting in a channel",
        ),
        ToolDef::new(
            "ask_user",
            "Ask the user a question or for a decision. Your turn ends; you continue when they answer in your direct chat.",
            schema(&[("question", "string", "What you need from them, short and specific", true)]),
            Risk::Internal,
            "Asking you",
        ),
        ToolDef::new(
            "update_task",
            "Update a task you were given or gave someone: mark it done or blocked and add a note.",
            schema(&[
                ("task_id", "string", "The task id shown in Open tasks", true),
                ("status", "string", "open, blocked or done", true),
                ("note", "string", "Short note about the result or what is blocking it", false),
            ]),
            Risk::Internal,
            "Updating a task",
        ),
        ToolDef::new(
            "create_agent",
            "Create a new agent (bot) on the team when no existing teammate fits a recurring kind of work.",
            schema(&[
                ("name", "string", "Short first name", true),
                ("title", "string", "Job title, for example Bookkeeper", true),
                ("description", "string", "What it owns; teammates read this to decide what to hand over", true),
                ("instructions", "string", "Standing instructions", false),
            ]),
            Risk::Risky,
            "Creating a bot",
        ),
    ];
    if has_team {
        v.push(ToolDef::new(
            "delegate_task",
            "Hand a task to a teammate and wait for their result. Use it when their description says they own this kind of work. Give them everything they need in the task text.",
            schema(&[
                ("to", "string", "Teammate name", true),
                ("task", "string", "The task with all needed context and the expected result", true),
            ]),
            Risk::Internal,
            "Waiting for a teammate",
        ));
        v.push(ToolDef::new(
            "send_message",
            "Send a teammate a message without waiting. Their reply comes back to you in this conversation later.",
            schema(&[("to", "string", "Teammate name", true), ("message", "string", "The message", true)]),
            Risk::Internal,
            "Messaging a teammate",
        ));
    }
    v
}

pub fn execute(ctx: &ToolCtx, name: &str, args: &Value) -> Option<ToolOutput> {
    Some(match name {
        "list_agents" => list_agents(ctx),
        "send_message" => send_message(ctx, args),
        "delegate_task" => delegate(ctx, args),
        "post_to_channel" => post_to_channel(ctx, args),
        "ask_user" => ask_user(ctx, args),
        "update_task" => update_task(ctx, args),
        "create_agent" => create_agent(ctx, args),
        _ => return None,
    })
}

fn list_agents(ctx: &ToolCtx) -> ToolOutput {
    let st = ctx.fleet.lock();
    let mut s = String::new();
    for a in &st.agents {
        let me = if a.id == ctx.agent.id { " (you)" } else { "" };
        let busy = if st.agent_busy(&a.id) { "busy" } else { "available" };
        s.push_str(&format!("- {}{me} [{busy}]: {}\n", a.label(), a.description));
    }
    ToolOutput::ok(s)
}

fn resolve_teammate(ctx: &ToolCtx, args: &Value) -> Result<Agent, ToolOutput> {
    let Some(to) = str_arg(args, "to") else { return Err(ToolOutput::err("Say who to send it to with \"to\".")) };
    let st = ctx.fleet.lock();
    match st.find_agent(to) {
        Some(a) if a.id == ctx.agent.id => Err(ToolOutput::err("That is you. Pick a teammate.")),
        Some(a) => Ok(a.clone()),
        None => {
            let names: Vec<String> = st.agents.iter().filter(|a| a.id != ctx.agent.id).map(|a| a.name.clone()).collect();
            Err(ToolOutput::err(format!("No teammate called \"{to}\". The team is: {}.", names.join(", "))))
        }
    }
}

fn send_message(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let target = match resolve_teammate(ctx, args) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Some(text) = str_arg(args, "message") else { return ToolOutput::err("The message is empty.") };
    let (thread, max_hops) = {
        let mut st = ctx.fleet.lock();
        let thread = st.between_thread(&ctx.agent.id, &target.id);
        st.push_message(&thread, ThreadMessage::new(Author::Agent(ctx.agent.id.clone()), text));
        (thread, st.config.max_hops)
    };
    if ctx.wake.hops >= max_hops {
        return ToolOutput {
            text: format!("Message delivered, but {} will not be woken: this chain of handoffs is too long.", target.name),
            link: Some(thread),
            ..Default::default()
        };
    }
    let mut w = Wake::new(&target.id, &thread);
    w.hops = ctx.wake.hops + 1;
    w.reply_to = Some((ctx.agent.id.clone(), ctx.thread.to_string()));
    ctx.fleet.wake(w);
    ToolOutput {
        text: format!("Sent to {}. Their reply will come back to you here.", target.name),
        link: Some(thread),
        ..Default::default()
    }
}

fn delegate(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let target = match resolve_teammate(ctx, args) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Some(task_text) = str_arg(args, "task") else { return ToolOutput::err("Describe the task in \"task\".") };
    if ctx.wake.chain.contains(&target.id) {
        return ToolOutput::err(format!(
            "{} is already waiting on you in this chain of handoffs, so delegating back would deadlock. Do it yourself or reply to them.",
            target.name
        ));
    }
    let (thread, task_id, max_hops) = {
        let mut st = ctx.fleet.lock();
        let thread = st.between_thread(&ctx.agent.id, &target.id);
        let title: String = task_text.lines().next().unwrap_or(task_text).chars().take(80).collect();
        let task = Task {
            id: new_id(),
            title,
            from: Author::Agent(ctx.agent.id.clone()),
            to: target.id.clone(),
            thread: thread.clone(),
            status: TaskStatus::Open,
            note: String::new(),
            created: Utc::now(),
            updated: Utc::now(),
            nudged: None,
        };
        let tid = task.id.clone();
        st.tasks.push(task);
        st.dirty.tasks = true;
        let text = format!("{task_text}\n\n(Task {})", short_id(&tid));
        st.push_message(&thread, ThreadMessage::new(Author::Agent(ctx.agent.id.clone()), text));
        (thread, tid, st.config.max_hops)
    };
    if ctx.wake.hops >= max_hops {
        return ToolOutput::err("This chain of handoffs is too long. Finish the work yourself.");
    }
    let (tx, rx) = mpsc::channel();
    let mut w = Wake::new(&target.id, &thread);
    w.hops = ctx.wake.hops + 1;
    w.chain = ctx.wake.chain.clone();
    w.chain.push(ctx.agent.id.clone());
    w.reply = Some(tx);
    ctx.fleet.wake(w);
    ctx.fleet.set_status(&ctx.agent.id, ctx.thread, format!("Waiting for {}", target.name));

    let started = Instant::now();
    let reply = loop {
        match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(r) => break Ok(r),
            Err(RecvTimeoutError::Timeout) => {
                if ctx.cancel.load(Ordering::Relaxed) {
                    ctx.fleet.stop_thread(&thread);
                    break Err("Stopped.".to_string());
                }
                if started.elapsed() > DELEGATE_TIMEOUT {
                    break Err(format!("{} did not finish within 30 minutes. They may still be working; check back later.", target.name));
                }
            }
            Err(RecvTimeoutError::Disconnected) => break Err(format!("{} stopped without replying.", target.name)),
        }
    };
    let mut st = ctx.fleet.lock();
    if let Some(t) = st.tasks.iter_mut().find(|t| t.id == task_id)
        && t.status == TaskStatus::Open
        && reply.is_ok()
    {
        t.status = TaskStatus::Done;
        t.updated = Utc::now();
        st.dirty.tasks = true;
    }
    drop(st);
    match reply {
        Ok(r) => ToolOutput { text: format!("{} replied:\n\n{r}", target.name), link: Some(thread), ..Default::default() },
        Err(e) => ToolOutput { text: e, error: true, link: Some(thread), ..Default::default() },
    }
}

fn post_to_channel(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let (Some(channel), Some(text)) = (str_arg(args, "channel"), str_arg(args, "message")) else {
        return ToolOutput::err("Give a channel and a message.");
    };
    let (thread, wakes) = {
        let mut st = ctx.fleet.lock();
        let Some(t) = st.find_channel(channel) else {
            return ToolOutput::err(format!("There is no channel called #{}.", channel.trim_start_matches('#')));
        };
        let (id, members) = match &t.kind {
            ThreadKind::Channel { members, .. } => (t.id.clone(), members.clone()),
            _ => unreachable!(),
        };
        if !members.contains(&ctx.agent.id) {
            return ToolOutput::err("You are not a member of that channel.");
        }
        st.push_message(&id, ThreadMessage::new(Author::Agent(ctx.agent.id.clone()), text));
        let max = st.config.max_hops;
        let wakes: Vec<String> = if ctx.wake.hops < max {
            st.mentions(text).into_iter().filter(|m| m != &ctx.agent.id && members.contains(m)).collect()
        } else {
            Vec::new()
        };
        (id, wakes)
    };
    for a in &wakes {
        let mut w = Wake::new(a, &thread);
        w.hops = ctx.wake.hops + 1;
        ctx.fleet.wake(w);
    }
    ToolOutput { text: "Posted.".into(), link: Some(thread), ..Default::default() }
}

fn ask_user(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let Some(q) = str_arg(args, "question") else { return ToolOutput::err("The question is empty.") };
    let mut st = ctx.fleet.lock();
    let in_dm = st.thread(ctx.thread).is_some_and(|t| t.is_dm_with(&ctx.agent.id));
    let dm = st.dm_thread(&ctx.agent.id);
    if !in_dm {
        let origin = match st.thread(ctx.thread).map(|t| t.kind.clone()) {
            Some(ThreadKind::Channel { name, .. }) => format!(" (from #{name})"),
            Some(ThreadKind::AgentDm { a, b }) => {
                let other = if a == ctx.agent.id { b } else { a };
                format!(" (while working with {})", st.agent(&other).map(|a| a.name.clone()).unwrap_or_default())
            }
            _ => String::new(),
        };
        st.push_message(&dm, ThreadMessage::new(Author::Agent(ctx.agent.id.clone()), format!("{q}{origin}")));
    }
    if let Some(t) = st.thread_mut(&dm) {
        t.needs_you = true;
    }
    st.mark_thread(&dm);
    st.notify(format!("{} needs you", ctx.agent.name), q.chars().take(140).collect::<String>(), Some(dm.clone()));
    let text = if in_dm { "Asked. Wait for their answer." } else { "Asked in your direct chat. Wait for their answer." };
    ToolOutput { text: text.into(), link: (!in_dm).then_some(dm), end_turn: true, ..Default::default() }
}

fn update_task(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let Some(tid) = str_arg(args, "task_id") else { return ToolOutput::err("Give the task id.") };
    let status = match str_arg(args, "status").unwrap_or("").to_lowercase().as_str() {
        "done" | "complete" | "completed" => TaskStatus::Done,
        "blocked" => TaskStatus::Blocked,
        "open" => TaskStatus::Open,
        other => return ToolOutput::err(format!("Unknown status \"{other}\". Use open, blocked or done.")),
    };
    let mut st = ctx.fleet.lock();
    let me = Author::Agent(ctx.agent.id.clone());
    let Some(t) = st.tasks.iter_mut().find(|t| (t.id == tid || t.id.starts_with(tid)) && (t.to == ctx.agent.id || t.from == me)) else {
        return ToolOutput::err("No such task among yours.");
    };
    t.status = status;
    t.updated = Utc::now();
    if let Some(n) = str_arg(args, "note") {
        t.note = n.to_string();
    }
    let title = t.title.clone();
    st.dirty.tasks = true;
    ToolOutput::ok(format!("Task \"{title}\" is now {status:?}."))
}

fn create_agent(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let (Some(name), Some(desc)) = (str_arg(args, "name"), str_arg(args, "description")) else {
        return ToolOutput::err("Give at least a name and a description.");
    };
    if ctx.fleet.lock().find_agent(name).is_some_and(|a| a.name.eq_ignore_ascii_case(name)) {
        return ToolOutput::err(format!("There is already a bot called {name}."));
    }
    let n = ctx.fleet.lock().agents.len();
    let agent = Agent {
        name: name.to_string(),
        title: str_arg(args, "title").unwrap_or("").to_string(),
        description: desc.to_string(),
        instructions: str_arg(args, "instructions").unwrap_or("").to_string(),
        color: AGENT_COLORS[n % AGENT_COLORS.len()],
        shape: Shape::ALL[n % Shape::ALL.len()],
        model: ctx.agent.model.clone(),
        ..Default::default()
    };
    let dm = ctx.fleet.create_agent(agent);
    ToolOutput { text: format!("Created {name}. You can now delegate to them."), link: Some(dm), ..Default::default() }
}
