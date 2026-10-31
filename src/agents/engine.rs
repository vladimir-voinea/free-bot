//! The fleet engine: owns agents, threads and the rest of the agent state,
//! starts agent turns on background threads, routes messages between agents,
//! holds pending approvals, runs routines and saves everything to disk.
//!
//! The UI and the agent threads share one `Fleet` (an `Arc`). Every access
//! takes the state lock briefly; no lock is held while waiting on the network,
//! a tool or the user.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use chrono::Utc;

use super::model::*;
use super::persist::FleetStore;
use super::{browser::BrowserManager, mcp::McpManager, runtime};
use crate::api::Waker;
use crate::store::{Settings, new_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approve,
    AlwaysAllow,
    Deny,
}

/// A request for an agent to take a turn in a thread.
#[derive(Clone)]
pub struct Wake {
    pub agent: String,
    pub thread: String,
    /// How many agent-to-agent handoffs led here.
    pub hops: u32,
    /// Agents waiting on this one through `delegate_task`, to prevent cycles.
    pub chain: Vec<String>,
    /// Receives the final reply (for `delegate_task`).
    pub reply: Option<Sender<String>>,
    /// Agent and thread to notify with the final reply (for `send_message`).
    pub reply_to: Option<(String, String)>,
}

impl Wake {
    pub fn new(agent: &str, thread: &str) -> Self {
        Self { agent: agent.into(), thread: thread.into(), hops: 0, chain: Vec::new(), reply: None, reply_to: None }
    }
}

pub struct RunInfo {
    pub cancel: Arc<AtomicBool>,
    pub started: Instant,
    /// What the agent is doing right now, shown in the UI ("Running a command").
    pub status: String,
}

pub struct Notice {
    pub title: String,
    pub body: String,
    pub thread: Option<String>,
}

#[derive(Default)]
pub struct Dirty {
    pub agents: bool,
    pub memories: bool,
    pub skills: bool,
    pub tasks: bool,
    pub routines: bool,
    pub config: bool,
    pub threads: HashSet<String>,
}

pub struct FleetState {
    pub agents: Vec<Agent>,
    pub threads: Vec<Thread>,
    pub memories: Vec<Memory>,
    pub skills: Vec<Skill>,
    pub tasks: Vec<Task>,
    pub routines: Vec<Routine>,
    pub config: FleetConfig,
    pub settings: Settings,
    pub runs: HashMap<(String, String), RunInfo>,
    pub queue: Vec<Wake>,
    pub approvals: HashMap<String, Sender<Decision>>,
    pub notices: Vec<Notice>,
    /// The thread open in the UI; it never gets marked unread.
    pub viewing: Option<String>,
    pub dirty: Dirty,
    /// Agents whose browser the user has taken over.
    pub takeover: HashSet<String>,
    /// Webhook calls waiting to fire routines: (token, body).
    pub webhook_hits: Vec<(String, String)>,
    file_marks: HashMap<String, SystemTime>,
}

impl FleetState {
    pub fn agent(&self, id: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| a.id == id)
    }

    pub fn agent_mut(&mut self, id: &str) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    /// Find an agent by id or (case-insensitive) name, as models write it.
    pub fn find_agent(&self, name_or_id: &str) -> Option<&Agent> {
        let q = name_or_id.trim().trim_start_matches('@').to_lowercase();
        self.agents.iter().find(|a| a.id == name_or_id).or_else(|| self.agents.iter().find(|a| a.name.to_lowercase() == q)).or_else(|| {
            self.agents.iter().find(|a| {
                let label = a.label().to_lowercase();
                label == q || q.starts_with(&a.name.to_lowercase())
            })
        })
    }

    pub fn thread(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == id)
    }

    pub fn thread_mut(&mut self, id: &str) -> Option<&mut Thread> {
        self.threads.iter_mut().find(|t| t.id == id)
    }

    pub fn find_channel(&self, name: &str) -> Option<&Thread> {
        let q = name.trim().trim_start_matches('#').to_lowercase();
        self.threads.iter().find(|t| t.channel_name().is_some_and(|n| n.to_lowercase() == q))
    }

    pub fn agent_busy(&self, agent: &str) -> bool {
        self.runs.keys().any(|(a, _)| a == agent)
    }

    pub fn thread_busy(&self, thread: &str) -> bool {
        self.runs.keys().any(|(_, t)| t == thread)
    }

    pub fn pending_approvals(&self) -> usize {
        self.threads
            .iter()
            .flat_map(|t| &t.messages)
            .flat_map(|m| &m.tool_calls)
            .filter(|c| c.status == ToolStatus::AwaitingApproval)
            .count()
    }

    pub fn mark_thread(&mut self, id: &str) {
        self.dirty.threads.insert(id.to_string());
    }

    /// Append a message and mark the thread changed and unread.
    pub fn push_message(&mut self, thread_id: &str, msg: ThreadMessage) -> Option<String> {
        let viewing = self.viewing.as_deref() == Some(thread_id);
        let t = self.thread_mut(thread_id)?;
        let id = msg.id.clone();
        if msg.author != Author::User && !viewing {
            t.unread = true;
        }
        t.messages.push(msg);
        t.updated = Utc::now();
        self.mark_thread(thread_id);
        Some(id)
    }

    pub fn dm_thread(&mut self, agent: &str) -> String {
        if let Some(t) = self.threads.iter().find(|t| t.is_dm_with(agent)) {
            return t.id.clone();
        }
        let t = Thread::new(ThreadKind::Dm { agent: agent.into() });
        let id = t.id.clone();
        self.threads.push(t);
        self.mark_thread(&id);
        id
    }

    pub fn between_thread(&mut self, a: &str, b: &str) -> String {
        if let Some(t) = self.threads.iter().find(|t| t.is_between(a, b)) {
            return t.id.clone();
        }
        let t = Thread::new(ThreadKind::AgentDm { a: a.into(), b: b.into() });
        let id = t.id.clone();
        self.threads.push(t);
        self.mark_thread(&id);
        id
    }

    pub fn notify(&mut self, title: impl Into<String>, body: impl Into<String>, thread: Option<String>) {
        self.notices.push(Notice { title: title.into(), body: body.into(), thread });
    }

    /// Agents mentioned with `@Name` in `text`, longest names first so "@Ava Lee" beats "@Ava".
    pub fn mentions(&self, text: &str) -> Vec<String> {
        let lower = text.to_lowercase();
        let mut agents: Vec<&Agent> = self.agents.iter().collect();
        agents.sort_by_key(|a| std::cmp::Reverse(a.name.len()));
        let mut found = Vec::new();
        let mut taken = lower.clone();
        for a in agents {
            let tag = format!("@{}", a.name.to_lowercase());
            if a.name.is_empty() {
                continue;
            }
            if let Some(pos) = taken.find(&tag) {
                let end = pos + tag.len();
                let boundary = taken[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
                if boundary {
                    found.push(a.id.clone());
                    taken.replace_range(pos..end, &" ".repeat(tag.len()));
                }
            }
        }
        found
    }

    pub fn workspace_dir(&self, store_root: &Path, agent: &Agent) -> PathBuf {
        agent.workspace.clone().unwrap_or_else(|| store_root.join("agents").join(&agent.id).join("workspace"))
    }
}

pub struct Inner {
    pub state: Mutex<FleetState>,
    pub store: FleetStore,
    pub waker: Arc<dyn Waker>,
    pub mcp: McpManager,
    pub browser: Arc<BrowserManager>,
    last_flush: Mutex<Instant>,
    last_tick: Mutex<Instant>,
    webhook_started: AtomicBool,
}

#[derive(Clone)]
pub struct Fleet(pub Arc<Inner>);

impl Fleet {
    pub fn open(data_root: &Path, settings: Settings, waker: Arc<dyn Waker>) -> Self {
        let store = FleetStore::open(data_root);
        let mut threads = store.load_threads();
        // A turn that was cut off by quitting leaves a half-written message behind.
        for t in &mut threads {
            for m in &mut t.messages {
                for c in &mut m.tool_calls {
                    if matches!(c.status, ToolStatus::Running | ToolStatus::AwaitingApproval) {
                        c.status = ToolStatus::Error;
                        c.result.get_or_insert_with(|| "Interrupted when Free Bot closed.".into());
                    }
                }
            }
        }
        let config = store.load_config();
        let state = FleetState {
            agents: store.load_agents(),
            threads,
            memories: store.load_memories(),
            skills: store.load_skills(),
            tasks: store.load_tasks(),
            routines: store.load_routines(),
            config,
            settings,
            runs: HashMap::new(),
            queue: Vec::new(),
            approvals: HashMap::new(),
            notices: Vec::new(),
            viewing: None,
            dirty: Dirty::default(),
            takeover: HashSet::new(),
            webhook_hits: Vec::new(),
            file_marks: HashMap::new(),
        };
        let browser_dir = store.browser_profile_dir();
        let fleet = Fleet(Arc::new(Inner {
            state: Mutex::new(state),
            store,
            waker: waker.clone(),
            mcp: McpManager::new(waker.clone()),
            browser: Arc::new(BrowserManager::new(browser_dir, waker)),
            last_flush: Mutex::new(Instant::now()),
            last_tick: Mutex::new(Instant::now() - Duration::from_secs(60)),
            webhook_started: AtomicBool::new(false),
        }));
        let connectors = fleet.lock().config.connectors.clone();
        fleet.0.mcp.sync(&connectors);
        fleet
    }

    pub fn lock(&self) -> MutexGuard<'_, FleetState> {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn wake_ui(&self) {
        self.0.waker.wake();
    }

    pub fn store(&self) -> &FleetStore {
        &self.0.store
    }

    pub fn mcp(&self) -> &McpManager {
        &self.0.mcp
    }

    pub fn browser(&self) -> &Arc<BrowserManager> {
        &self.0.browser
    }

    pub fn set_settings(&self, s: &Settings) {
        let mut st = self.lock();
        if st.settings != *s {
            st.settings = s.clone();
        }
    }

    pub fn workspace_of(&self, agent: &Agent) -> PathBuf {
        let root = self.0.store.root().to_path_buf();
        let dir = self.lock().workspace_dir(&root, agent);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    // ---- agents and channels ---------------------------------------------

    pub fn create_agent(&self, agent: Agent) -> String {
        let mut st = self.lock();
        let id = agent.id.clone();
        st.agents.push(agent);
        st.dirty.agents = true;
        st.dm_thread(&id)
    }

    pub fn update_agent(&self, agent: Agent) {
        let mut st = self.lock();
        if let Some(a) = st.agent_mut(&agent.id) {
            *a = agent;
            st.dirty.agents = true;
        }
    }

    pub fn delete_agent(&self, id: &str) {
        self.stop_agent(id);
        let mut st = self.lock();
        st.agents.retain(|a| a.id != id);
        let gone: Vec<String> = st
            .threads
            .iter()
            .filter(|t| {
                matches!(&t.kind, ThreadKind::Dm { agent } if agent == id)
                    || matches!(&t.kind, ThreadKind::AgentDm { a, b } if a == id || b == id)
            })
            .map(|t| t.id.clone())
            .collect();
        st.threads.retain(|t| !gone.contains(&t.id));
        for t in &mut st.threads {
            if let ThreadKind::Channel { members, lead, .. } = &mut t.kind {
                members.retain(|m| m != id);
                if lead.as_deref() == Some(id) {
                    *lead = members.first().cloned();
                }
            }
        }
        let channel_ids: Vec<String> = st.threads.iter().filter(|t| t.channel_name().is_some()).map(|t| t.id.clone()).collect();
        for c in channel_ids {
            st.mark_thread(&c);
        }
        st.memories.retain(|m| m.agent.as_deref() != Some(id));
        st.routines.retain(|r| r.agent != id);
        st.tasks.retain(|t| t.to != id);
        st.dirty.agents = true;
        st.dirty.memories = true;
        st.dirty.routines = true;
        st.dirty.tasks = true;
        drop(st);
        for t in gone {
            self.0.store.delete_thread(&t);
        }
    }

    pub fn create_channel(&self, name: &str, members: Vec<String>, lead: Option<String>) -> String {
        let mut st = self.lock();
        let name = sanitize_channel(name);
        let lead = lead.or_else(|| members.first().cloned());
        let t = Thread::new(ThreadKind::Channel { name, members, lead });
        let id = t.id.clone();
        st.threads.push(t);
        st.mark_thread(&id);
        id
    }

    pub fn update_channel(&self, id: &str, name: &str, members: Vec<String>, lead: Option<String>) {
        let mut st = self.lock();
        if let Some(t) = st.thread_mut(id)
            && let ThreadKind::Channel { name: n, members: m, lead: l } = &mut t.kind
        {
            *n = sanitize_channel(name);
            *l = lead.filter(|x| members.contains(x)).or_else(|| members.first().cloned());
            *m = members;
            st.mark_thread(id);
        }
    }

    pub fn delete_thread(&self, id: &str) {
        self.stop_thread(id);
        let mut st = self.lock();
        st.threads.retain(|t| t.id != id);
        st.dirty.threads.remove(id);
        drop(st);
        self.0.store.delete_thread(id);
    }

    pub fn clear_thread(&self, id: &str) {
        self.stop_thread(id);
        let mut st = self.lock();
        if let Some(t) = st.thread_mut(id) {
            t.messages.clear();
            t.needs_you = false;
        }
        st.mark_thread(id);
    }

    // ---- messaging -------------------------------------------------------

    /// The user writes in a thread. Returns the agents that will answer.
    pub fn user_post(&self, thread_id: &str, text: &str) -> Vec<String> {
        let text = text.trim();
        if text.is_empty() {
            return Vec::new();
        }
        let responders = {
            let mut st = self.lock();
            let Some(t) = st.thread(thread_id) else { return Vec::new() };
            let kind = t.kind.clone();
            let responders = match &kind {
                ThreadKind::Dm { agent } => vec![agent.clone()],
                ThreadKind::Channel { members, lead, .. } => {
                    let m: Vec<String> = st.mentions(text).into_iter().filter(|a| members.contains(a)).collect();
                    if !m.is_empty() { m } else { lead.clone().or_else(|| members.first().cloned()).into_iter().collect() }
                }
                ThreadKind::AgentDm { .. } => Vec::new(),
            };
            if let Some(t) = st.thread_mut(thread_id) {
                t.needs_you = false;
            }
            st.push_message(thread_id, ThreadMessage::new(Author::User, text));
            responders
        };
        for a in &responders {
            self.wake(Wake::new(a, thread_id));
        }
        responders
    }

    /// Start an agent turn, or queue it if the agent is already working in that thread.
    pub fn wake(&self, w: Wake) {
        let mut st = self.lock();
        if st.agent(&w.agent).is_none() || st.thread(&w.thread).is_none() {
            if let Some(tx) = w.reply {
                let _ = tx.send("That agent no longer exists.".into());
            }
            return;
        }
        let key = (w.agent.clone(), w.thread.clone());
        if st.runs.contains_key(&key) {
            st.queue.push(w);
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        st.runs.insert(key, RunInfo { cancel: cancel.clone(), started: Instant::now(), status: "Thinking".into() });
        drop(st);
        let fleet = self.clone();
        std::thread::Builder::new()
            .name("agent-turn".into())
            .spawn(move || {
                runtime::run_turn(&fleet, &w, &cancel);
                fleet.finish_turn(w);
            })
            .expect("failed to start agent thread");
        self.wake_ui();
    }

    fn finish_turn(&self, w: Wake) {
        let mut follow_ups: Vec<Wake> = Vec::new();
        {
            let mut st = self.lock();
            st.runs.remove(&(w.agent.clone(), w.thread.clone()));
            let max_hops = st.config.max_hops;
            let agent_name = st.agent(&w.agent).map(|a| a.name.clone()).unwrap_or_default();
            // The agent's last words in this turn.
            let reply = st
                .thread(&w.thread)
                .and_then(|t| t.messages.iter().rev().find(|m| m.author == Author::Agent(w.agent.clone())))
                .map(|m| if m.content.trim().is_empty() { m.error.clone().unwrap_or_default() } else { m.content.clone() })
                .unwrap_or_default();
            let reply = if reply.trim() == "NO_REPLY" { String::new() } else { reply };

            if let Some(tx) = &w.reply {
                let _ = tx.send(if reply.trim().is_empty() { "(They finished without a reply.)".into() } else { reply.clone() });
            }
            if let Some((origin_agent, origin_thread)) = &w.reply_to
                && !reply.trim().is_empty()
                && w.hops < max_hops
            {
                let note = format!("{agent_name} replied to your message:\n\n{reply}");
                st.push_message(origin_thread, ThreadMessage::new(Author::System, note));
                let mut next = Wake::new(origin_agent, origin_thread);
                next.hops = w.hops + 1;
                follow_ups.push(next);
            }
            // In channels, mentions in the reply hand work to other members.
            if let Some(ThreadKind::Channel { members, .. }) = st.thread(&w.thread).map(|t| t.kind.clone())
                && w.hops < max_hops
            {
                for m in st.mentions(&reply) {
                    if m != w.agent && members.contains(&m) {
                        let mut next = Wake::new(&m, &w.thread);
                        next.hops = w.hops + 1;
                        follow_ups.push(next);
                    }
                }
            }
            // A DM reply while the user looks elsewhere is worth a notification.
            if let Some(t) = st.thread(&w.thread)
                && matches!(t.kind, ThreadKind::Dm { .. })
                && st.viewing.as_deref() != Some(w.thread.as_str())
                && !reply.trim().is_empty()
            {
                let body: String = reply.chars().take(140).collect();
                let thread = w.thread.clone();
                st.notify(agent_name.clone(), body, Some(thread));
            }
            // Next queued wake for the same agent and thread.
            if let Some(pos) = st.queue.iter().position(|q| q.agent == w.agent && q.thread == w.thread) {
                follow_ups.insert(0, st.queue.remove(pos));
            }
        }
        for f in follow_ups {
            self.wake(f);
        }
        self.wake_ui();
    }

    pub fn stop_thread(&self, thread: &str) {
        let mut st = self.lock();
        st.queue.retain(|q| q.thread != thread);
        for ((_, t), r) in st.runs.iter() {
            if t == thread {
                r.cancel.store(true, Ordering::Relaxed);
            }
        }
        deny_pending(&mut st, |t| t == thread);
    }

    pub fn stop_agent(&self, agent: &str) {
        let mut st = self.lock();
        st.queue.retain(|q| q.agent != agent);
        for ((a, _), r) in st.runs.iter() {
            if a == agent {
                r.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    pub fn stop_all(&self) {
        let mut st = self.lock();
        st.queue.clear();
        for r in st.runs.values() {
            r.cancel.store(true, Ordering::Relaxed);
        }
        deny_pending(&mut st, |_| true);
    }

    pub fn decide(&self, call_id: &str, decision: Decision) {
        let tx = self.lock().approvals.remove(call_id);
        if let Some(tx) = tx {
            let _ = tx.send(decision);
        }
        self.wake_ui();
    }

    /// Register a pending approval and get the receiver the agent thread waits on.
    pub fn await_approval(&self, call_id: &str) -> mpsc::Receiver<Decision> {
        let (tx, rx) = mpsc::channel();
        self.lock().approvals.insert(call_id.to_string(), tx);
        rx
    }

    pub fn set_status(&self, agent: &str, thread: &str, status: impl Into<String>) {
        if let Some(r) = self.lock().runs.get_mut(&(agent.to_string(), thread.to_string())) {
            r.status = status.into();
        }
        self.wake_ui();
    }

    // ---- routines, check-ins, triggers -------------------------------------

    pub fn run_routine(&self, routine_id: &str, extra: Option<String>) {
        let (agent, thread) = {
            let mut st = self.lock();
            let Some(r) = st.routines.iter().find(|r| r.id == routine_id).cloned() else { return };
            if st.agent(&r.agent).is_none() {
                return;
            }
            let thread = match r.thread.clone().filter(|t| st.thread(t).is_some()) {
                Some(t) => t,
                None => st.dm_thread(&r.agent),
            };
            let mut note = format!("Routine \"{}\" is running ({}).\n\n{}", r.name, r.trigger.describe(), r.instructions);
            if let Some(extra) = extra {
                note.push_str(&format!("\n\nTrigger details:\n{}", super::prompt::truncate(&extra, 4000)));
            }
            st.push_message(&thread, ThreadMessage::new(Author::System, note));
            if let Some(r) = st.routines.iter_mut().find(|x| x.id == routine_id) {
                r.last_run = Some(Utc::now());
                r.last_status = Some("Started".into());
            }
            st.dirty.routines = true;
            (r.agent, thread)
        };
        self.wake(Wake::new(&agent, &thread));
    }

    /// Called often (every frame in the app, every second headless). Cheap when nothing is due.
    pub fn tick(&self) {
        {
            let mut last = self.0.last_tick.lock().unwrap_or_else(|e| e.into_inner());
            if last.elapsed() < Duration::from_secs(1) {
                return;
            }
            *last = Instant::now();
        }
        self.ensure_webhook_server();
        let now = chrono::Local::now();
        let mut due: Vec<(String, Option<String>)> = Vec::new();
        let mut nudges: Vec<Wake> = Vec::new();
        {
            let mut st = self.lock();
            for r in &st.routines {
                if !r.enabled {
                    continue;
                }
                if r.trigger.is_schedule() && r.next_run().is_some_and(|t| t <= now) {
                    due.push((r.id.clone(), None));
                }
            }
            // Webhooks.
            let hits = std::mem::take(&mut st.webhook_hits);
            for (token, body) in hits {
                for r in &st.routines {
                    if r.enabled && matches!(&r.trigger, Trigger::Webhook { token: t } if *t == token) {
                        due.push((r.id.clone(), Some(body.clone())));
                    }
                }
            }
            // File watches, checked every few seconds.
            if now.timestamp() % 5 == 0 {
                let watches: Vec<(String, PathBuf)> = st
                    .routines
                    .iter()
                    .filter(|r| r.enabled)
                    .filter_map(|r| match &r.trigger {
                        Trigger::FileChange { path } => Some((r.id.clone(), path.clone())),
                        _ => None,
                    })
                    .collect();
                for (rid, path) in watches {
                    let mtime = latest_mtime(&path, 2);
                    let prev = st.file_marks.get(&rid).copied();
                    if let Some(m) = mtime {
                        st.file_marks.insert(rid.clone(), m);
                        if prev.is_some_and(|p| m > p) {
                            due.push((rid, Some(format!("Something changed in {}", path.display()))));
                        }
                    }
                }
            }
            // Follow up on stalled tasks for proactive agents.
            let stall = chrono::Duration::minutes(st.config.stall_minutes.max(1) as i64);
            let utc = Utc::now();
            let mut touched = false;
            let mut nudge_list = Vec::new();
            for t in &st.tasks {
                if t.status != TaskStatus::Open || utc - t.updated < stall || t.nudged.is_some_and(|n| utc - n < stall) {
                    continue;
                }
                let Author::Agent(from) = &t.from else { continue };
                let Some(a) = st.agent(from) else { continue };
                if !a.proactive || st.agent_busy(from) || st.agent_busy(&t.to) {
                    continue;
                }
                nudge_list.push((t.id.clone(), from.clone(), t.thread.clone(), t.title.clone()));
            }
            for (tid, from, thread, title) in nudge_list {
                if st.thread(&thread).is_none() {
                    continue;
                }
                let assignee =
                    st.tasks.iter().find(|t| t.id == tid).and_then(|t| st.agent(&t.to)).map(|a| a.name.clone()).unwrap_or_default();
                let note = format!(
                    "Check-in: the task \"{title}\" you gave {assignee} has had no update for a while. Follow up with them, or mark it done or blocked with update_task (task id {}).",
                    super::prompt::short_id(&tid)
                );
                st.push_message(&thread, ThreadMessage::new(Author::System, note));
                if let Some(t) = st.tasks.iter_mut().find(|t| t.id == tid) {
                    t.nudged = Some(utc);
                }
                touched = true;
                let mut w = Wake::new(&from, &thread);
                w.hops = 1;
                nudges.push(w);
            }
            if touched {
                st.dirty.tasks = true;
            }
        }
        for (rid, extra) in due {
            self.run_routine(&rid, extra);
        }
        for w in nudges {
            self.wake(w);
        }
        self.flush_if_due();
    }

    fn ensure_webhook_server(&self) {
        let (enabled, port) = {
            let st = self.lock();
            (st.config.webhooks_enabled && st.routines.iter().any(|r| matches!(r.trigger, Trigger::Webhook { .. })), st.config.webhook_port)
        };
        if !enabled || self.0.webhook_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let fleet = self.clone();
        std::thread::spawn(move || {
            let server = match tiny_http::Server::http(("127.0.0.1", port)) {
                Ok(s) => s,
                Err(e) => {
                    fleet.lock().notify("Webhooks", format!("Could not listen on port {port}: {e}"), None);
                    fleet.0.webhook_started.store(false, Ordering::SeqCst);
                    return;
                }
            };
            for mut req in server.incoming_requests() {
                let url = req.url().to_string();
                let token = url.trim_start_matches("/hook/").split('?').next().unwrap_or("").to_string();
                let mut body = String::new();
                let _ = std::io::Read::read_to_string(&mut std::io::Read::take(req.as_reader(), 64 * 1024), &mut body);
                let known = url.starts_with("/hook/")
                    && fleet.lock().routines.iter().any(|r| matches!(&r.trigger, Trigger::Webhook { token: t } if *t == token));
                if known {
                    fleet.lock().webhook_hits.push((token, body));
                    fleet.wake_ui();
                    let _ = req.respond(tiny_http::Response::from_string("ok"));
                } else {
                    let _ = req.respond(tiny_http::Response::from_string("not found").with_status_code(404));
                }
            }
        });
    }

    // ---- persistence -------------------------------------------------------

    pub fn flush_if_due(&self) {
        let mut last = self.0.last_flush.lock().unwrap_or_else(|e| e.into_inner());
        if last.elapsed() >= Duration::from_millis(1500) {
            *last = Instant::now();
            drop(last);
            self.flush();
        }
    }

    pub fn flush(&self) {
        let mut st = self.lock();
        let dirty = std::mem::take(&mut st.dirty);
        let store = &self.0.store;
        if dirty.agents {
            store.save_agents(&st.agents);
        }
        if dirty.memories {
            store.save_memories(&st.memories);
        }
        if dirty.skills {
            store.save_skills(&st.skills);
        }
        if dirty.tasks {
            store.save_tasks(&st.tasks);
        }
        if dirty.routines {
            store.save_routines(&st.routines);
        }
        if dirty.config {
            store.save_config(&st.config);
        }
        for id in dirty.threads {
            if let Some(t) = st.thread(&id) {
                store.save_thread(t);
            }
        }
    }

    pub fn save_config(&self, cfg: FleetConfig) {
        let connectors = cfg.connectors.clone();
        {
            let mut st = self.lock();
            st.config = cfg;
            st.dirty.config = true;
        }
        self.0.mcp.sync(&connectors);
    }

    pub fn new_routine_token() -> String {
        new_id().replace('-', "")
    }
}

fn deny_pending(st: &mut FleetState, matches: impl Fn(&str) -> bool) {
    let ids: Vec<String> = st
        .threads
        .iter()
        .filter(|t| matches(&t.id))
        .flat_map(|t| &t.messages)
        .flat_map(|m| &m.tool_calls)
        .filter(|c| c.status == ToolStatus::AwaitingApproval)
        .map(|c| c.id.clone())
        .collect();
    for id in ids {
        if let Some(tx) = st.approvals.remove(&id) {
            let _ = tx.send(Decision::Deny);
        }
    }
}

fn sanitize_channel(name: &str) -> String {
    let s: String = name
        .trim()
        .trim_start_matches('#')
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "general".into() } else { s }
}

/// Newest modification time in a file or folder, looking `depth` levels down.
fn latest_mtime(path: &Path, depth: u32) -> Option<SystemTime> {
    let meta = std::fs::metadata(path).ok()?;
    let mut best = meta.modified().ok();
    if meta.is_dir() && depth > 0 {
        for e in std::fs::read_dir(path).ok()?.flatten().take(500) {
            if let Some(m) = latest_mtime(&e.path(), depth - 1) {
                best = Some(best.map_or(m, |b| b.max(m)));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoWake;
    impl Waker for NoWake {
        fn wake(&self) {}
    }

    fn fleet() -> (tempfile::TempDir, Fleet) {
        let dir = tempfile::tempdir().unwrap();
        let f = Fleet::open(dir.path(), Settings::default(), Arc::new(NoWake));
        (dir, f)
    }

    #[test]
    fn mentions_prefer_longest_names() {
        let (_d, f) = fleet();
        let mut st = f.lock();
        st.agents.push(Agent { name: "Ava".into(), ..Default::default() });
        st.agents.push(Agent { name: "Ava Lee".into(), ..Default::default() });
        st.agents.push(Agent { name: "Bo".into(), ..Default::default() });
        let found = st.mentions("hey @ava lee and @Bo, not @Bob");
        let names: Vec<String> = found.iter().map(|id| st.agent(id).unwrap().name.clone()).collect();
        assert_eq!(names, vec!["Ava Lee", "Bo"]);
    }

    #[test]
    fn agents_channels_and_persistence() {
        let (dir, f) = fleet();
        let a = Agent { name: "Atlas".into(), ..Default::default() };
        let b = Agent { name: "Nova".into(), ..Default::default() };
        let (aid, bid) = (a.id.clone(), b.id.clone());
        let dm = f.create_agent(a);
        f.create_agent(b);
        let ch = f.create_channel("#Launch Plan", vec![aid.clone(), bid.clone()], None);
        {
            let mut st = f.lock();
            assert_eq!(st.thread(&ch).unwrap().channel_name(), Some("launch-plan"));
            assert!(st.thread(&dm).unwrap().is_dm_with(&aid));
            let between = st.between_thread(&aid, &bid);
            assert_eq!(st.between_thread(&bid, &aid), between);
            st.memories.push(Memory { id: new_id(), agent: None, text: "likes tea".into(), created: Utc::now() });
            st.dirty.memories = true;
        }
        f.flush();
        let f2 = Fleet::open(dir.path(), Settings::default(), Arc::new(NoWake));
        let st = f2.lock();
        assert_eq!(st.agents.len(), 2);
        assert_eq!(st.threads.len(), 4);
        assert_eq!(st.memories[0].text, "likes tea");
        drop(st);
        f2.delete_agent(&aid);
        let st = f2.lock();
        assert_eq!(st.agents.len(), 1);
        // DM and agent-to-agent chat are gone, the channel stays with one member.
        assert_eq!(st.threads.len(), 2);
    }
}
