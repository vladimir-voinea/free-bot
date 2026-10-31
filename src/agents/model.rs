//! Data model of the agent layer: agents, threads (direct chats, group
//! channels and agent-to-agent chats), memory, skills, tasks, routines and
//! connectors.

use std::path::PathBuf;

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Timelike, Utc, Weekday};
use serde::{Deserialize, Serialize};

use crate::store::{ModelRef, new_id};

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Circle,
    Square,
    Hexagon,
    Diamond,
    Star,
}

impl Shape {
    pub const ALL: [Shape; 5] = [Shape::Circle, Shape::Square, Shape::Hexagon, Shape::Diamond, Shape::Star];
}

pub const AGENT_COLORS: [[u8; 3]; 8] = [
    [0x4f, 0x8c, 0xff],
    [0x9b, 0x6b, 0xff],
    [0xff, 0x6b, 0x9b],
    [0xff, 0x9a, 0x3c],
    [0xf2, 0xc9, 0x4c],
    [0x3c, 0xc8, 0x8f],
    [0x2b, 0xbf, 0xd6],
    [0x8a, 0x94, 0xa6],
];

/// When an agent must stop and ask before using a tool.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ApprovalMode {
    /// Ask before risky actions (shell commands, creating agents, connector writes).
    #[default]
    Risky,
    /// Ask before every action that touches the outside world.
    Everything,
    /// Never ask.
    Never,
}

/// How tool calls are sent to the model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ToolMode {
    /// Native function calling, falling back to the prompt protocol if the endpoint rejects `tools`.
    #[default]
    Auto,
    Native,
    /// Tools described in the prompt and called with `<tool_call>` tags, for models without function calling.
    Prompt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ToolSet {
    pub team: bool,
    pub files: bool,
    pub terminal: bool,
    pub web: bool,
    pub browser: bool,
    pub memory: bool,
    pub skills: bool,
    pub routines: bool,
}

impl Default for ToolSet {
    fn default() -> Self {
        Self { team: true, files: true, terminal: true, web: true, browser: true, memory: true, skills: true, routines: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// Short job title such as "Chief of staff".
    pub title: String,
    /// What this agent does. Other agents read it to decide what to hand over.
    pub description: String,
    /// Extra standing instructions.
    pub instructions: String,
    pub color: [u8; 3],
    pub shape: Shape,
    /// `None` uses the default model from settings.
    pub model: Option<ModelRef>,
    pub temperature: Option<f32>,
    pub tools: ToolSet,
    /// Connector ids this agent may use. `None` means all connectors.
    pub connectors: Option<Vec<String>>,
    pub approval: ApprovalMode,
    /// Tools the user chose to always allow for this agent.
    pub always_allow: Vec<String>,
    pub tool_mode: ToolMode,
    /// Working folder for files and commands. `None` uses the agent's own folder in the data directory.
    pub workspace: Option<PathBuf>,
    /// Follow up on stalled tasks on its own.
    pub proactive: bool,
    pub created: DateTime<Utc>,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New bot".into(),
            title: String::new(),
            description: String::new(),
            instructions: String::new(),
            color: AGENT_COLORS[0],
            shape: Shape::Circle,
            model: None,
            temperature: None,
            tools: ToolSet::default(),
            connectors: None,
            approval: ApprovalMode::Risky,
            always_allow: Vec::new(),
            tool_mode: ToolMode::Auto,
            workspace: None,
            proactive: true,
            created: Utc::now(),
        }
    }
}

impl Agent {
    pub fn label(&self) -> String {
        if self.title.trim().is_empty() { self.name.clone() } else { format!("{} ({})", self.name, self.title.trim()) }
    }
}

/// Ready-made agents offered when creating one.
pub struct AgentTemplate {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub instructions: &'static str,
    pub color: usize,
    pub shape: Shape,
}

pub const AGENT_TEMPLATES: &[AgentTemplate] = &[
    AgentTemplate {
        name: "Atlas",
        title: "Chief of staff",
        description: "Runs the team. Takes any request, breaks it into tasks, checks whether another bot owns each part and delegates first, follows up until everything is done, and reports back with a short summary.",
        instructions: "Before doing work yourself, check the team list. If a teammate's description matches, delegate. Do it yourself only when nobody fits. Keep the user's time free: bring them decisions, not status updates.",
        color: 0,
        shape: Shape::Hexagon,
    },
    AgentTemplate {
        name: "Nova",
        title: "Researcher",
        description: "Researches any topic on the web, reads sources, compares options and writes concise briefs with links.",
        instructions: "Search, open the best sources, and cite them. Prefer primary sources. End with a short recommendation.",
        color: 1,
        shape: Shape::Circle,
    },
    AgentTemplate {
        name: "Forge",
        title: "Engineer",
        description: "Writes and runs code, scripts and shell commands in its workspace, fixes bugs and automates repetitive work.",
        instructions: "Work in your workspace folder. Run code to verify it before saying it works. Explain what you changed.",
        color: 5,
        shape: Shape::Square,
    },
    AgentTemplate {
        name: "Quill",
        title: "Writer",
        description: "Drafts emails, posts, documents and replies in the user's voice, and edits text for clarity.",
        instructions: "Match the user's tone. Keep drafts short. Save longer documents to files in your workspace.",
        color: 2,
        shape: Shape::Diamond,
    },
    AgentTemplate {
        name: "Echo",
        title: "Inbox assistant",
        description: "Triages messages and requests from connected tools, drafts replies and flags what needs the user.",
        instructions: "Sort into needs-you, drafted, and handled. Never send anything on the user's behalf without approval.",
        color: 3,
        shape: Shape::Star,
    },
];

// ---------------------------------------------------------------------------
// Threads and messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type", content = "id")]
pub enum Author {
    User,
    Agent(String),
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    AwaitingApproval,
    Denied,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON arguments as sent by the model.
    pub arguments: String,
    pub result: Option<String>,
    pub status: ToolStatus,
    /// A thread this call opened or used, such as the chat with a teammate.
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadMessage {
    pub id: String,
    pub author: Author,
    pub content: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub model: String,
    /// Still being written by a running agent.
    #[serde(skip)]
    pub streaming: bool,
}

impl ThreadMessage {
    pub fn new(author: Author, content: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            author,
            content: content.into(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            created: Utc::now(),
            error: None,
            model: String::new(),
            streaming: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum ThreadKind {
    /// The user's direct chat with one agent.
    Dm { agent: String },
    /// A group channel with the user and several agents.
    Channel {
        name: String,
        members: Vec<String>,
        /// The agent that answers messages that mention nobody.
        lead: Option<String>,
    },
    /// Two agents talking to each other.
    AgentDm { a: String, b: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Thread {
    pub id: String,
    pub kind: ThreadKind,
    pub messages: Vec<ThreadMessage>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    #[serde(default)]
    pub unread: bool,
    /// An agent is waiting for the user here.
    #[serde(default)]
    pub needs_you: bool,
}

impl Thread {
    pub fn new(kind: ThreadKind) -> Self {
        let now = Utc::now();
        Self { id: new_id(), kind, messages: Vec::new(), created: now, updated: now, unread: false, needs_you: false }
    }

    pub fn has_agent(&self, agent: &str) -> bool {
        match &self.kind {
            ThreadKind::Dm { agent: a } => a == agent,
            ThreadKind::Channel { members, .. } => members.iter().any(|m| m == agent),
            ThreadKind::AgentDm { a, b } => a == agent || b == agent,
        }
    }

    pub fn is_dm_with(&self, agent: &str) -> bool {
        matches!(&self.kind, ThreadKind::Dm { agent: a } if a == agent)
    }

    pub fn is_between(&self, x: &str, y: &str) -> bool {
        matches!(&self.kind, ThreadKind::AgentDm { a, b } if (a == x && b == y) || (a == y && b == x))
    }

    pub fn channel_name(&self) -> Option<&str> {
        match &self.kind {
            ThreadKind::Channel { name, .. } => Some(name),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Memory, skills, tasks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub id: String,
    /// The agent that owns this memory. `None` means it is shared by the whole team.
    pub agent: Option<String>,
    pub text: String,
    pub created: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Step-by-step instructions the agent follows.
    pub instructions: String,
    pub author: Option<String>,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Open,
    Blocked,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub from: Author,
    pub to: String,
    pub thread: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub note: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    /// Last time someone was nudged about this task.
    #[serde(default)]
    pub nudged: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Routines
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Trigger {
    Every {
        minutes: u32,
    },
    Hourly {
        minute: u32,
    },
    Daily {
        hour: u32,
        minute: u32,
    },
    Weekly {
        weekday: u32,
        hour: u32,
        minute: u32,
    },
    Monthly {
        day: u32,
        hour: u32,
        minute: u32,
    },
    /// Runs when anything in the folder or file changes.
    FileChange {
        path: PathBuf,
    },
    /// Runs when `POST /hook/<token>` arrives on the local webhook port.
    Webhook {
        token: String,
    },
}

impl Trigger {
    pub fn is_schedule(&self) -> bool {
        !matches!(self, Trigger::FileChange { .. } | Trigger::Webhook { .. })
    }

    pub fn describe(&self) -> String {
        let hm = |h: u32, m: u32| format!("{h:02}:{m:02}");
        match self {
            Trigger::Every { minutes } if *minutes % 60 == 0 && *minutes >= 60 => format!("Every {} h", minutes / 60),
            Trigger::Every { minutes } => format!("Every {minutes} min"),
            Trigger::Hourly { minute } => format!("Hourly at :{minute:02}"),
            Trigger::Daily { hour, minute } => format!("Daily at {}", hm(*hour, *minute)),
            Trigger::Weekly { weekday, hour, minute } => format!("{}s at {}", weekday_name(*weekday), hm(*hour, *minute)),
            Trigger::Monthly { day, hour, minute } => format!("Monthly on day {day} at {}", hm(*hour, *minute)),
            Trigger::FileChange { path } => format!("When {} changes", path.display()),
            Trigger::Webhook { .. } => "On webhook".into(),
        }
    }

    /// Parse a schedule written in plain words, as agents and users write it:
    /// "every 30 minutes", "hourly at :15", "daily at 9:00", "weekly on monday at 09:30",
    /// "monthly on day 1 at 8:00".
    pub fn parse(s: &str) -> Option<Trigger> {
        let s = s.trim().to_lowercase();
        let words: Vec<&str> = s.split_whitespace().collect();
        let time_after_at = |s: &str| -> Option<(u32, u32)> {
            let t = s.split(" at ").nth(1)?.trim().trim_start_matches(':');
            parse_time(t)
        };
        if words.first() == Some(&"every") {
            let n: u32 = words.get(1).and_then(|w| w.parse().ok()).unwrap_or(1);
            let unit = words.iter().skip(1).find(|w| w.parse::<u32>().is_err()).copied().unwrap_or("minutes");
            let minutes = if unit.starts_with('h') {
                n * 60
            } else if unit.starts_with("day") {
                n * 60 * 24
            } else {
                n
            };
            return (minutes > 0).then_some(Trigger::Every { minutes });
        }
        if s.starts_with("hourly") {
            let minute = s.split(':').nth(1).and_then(|m| m.trim().parse().ok()).unwrap_or(0);
            return (minute < 60).then_some(Trigger::Hourly { minute });
        }
        if s.starts_with("daily") || s.starts_with("every day") {
            let (hour, minute) = time_after_at(&s).unwrap_or((9, 0));
            return Some(Trigger::Daily { hour, minute });
        }
        if s.starts_with("monthly") {
            let day = s
                .split("day")
                .nth(1)
                .and_then(|r| r.split_whitespace().next())
                .and_then(|d| d.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok())
                .unwrap_or(1u32)
                .clamp(1, 28);
            let (hour, minute) = time_after_at(&s).unwrap_or((9, 0));
            return Some(Trigger::Monthly { day, hour, minute });
        }
        if s.starts_with("weekly") || weekday_from(&s).is_some() && s.contains(" at ") {
            let weekday = weekday_from(&s).unwrap_or(0);
            let (hour, minute) = time_after_at(&s).unwrap_or((9, 0));
            return Some(Trigger::Weekly { weekday, hour, minute });
        }
        None
    }

    /// The next time a schedule fires strictly after `after`.
    pub fn next_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        let at = |date: chrono::NaiveDate, h: u32, m: u32| -> Option<DateTime<Local>> {
            let t = NaiveTime::from_hms_opt(h.min(23), m.min(59), 0)?;
            Local.from_local_datetime(&date.and_time(t)).earliest()
        };
        match self {
            Trigger::Every { minutes } => Some(after + Duration::minutes((*minutes).max(1) as i64)),
            Trigger::Hourly { minute } => {
                let base = after.with_minute(*minute % 60)?.with_second(0)?.with_nanosecond(0)?;
                Some(if base > after { base } else { base + Duration::hours(1) })
            }
            Trigger::Daily { hour, minute } => {
                let today = at(after.date_naive(), *hour, *minute)?;
                if today > after { Some(today) } else { at(after.date_naive() + Duration::days(1), *hour, *minute) }
            }
            Trigger::Weekly { weekday, hour, minute } => {
                for d in 0..8 {
                    let date = after.date_naive() + Duration::days(d);
                    if date.weekday().num_days_from_monday() == *weekday % 7
                        && let Some(t) = at(date, *hour, *minute)
                        && t > after
                    {
                        return Some(t);
                    }
                }
                None
            }
            Trigger::Monthly { day, hour, minute } => {
                let mut y = after.year();
                let mut mo = after.month();
                for _ in 0..3 {
                    if let Some(date) = chrono::NaiveDate::from_ymd_opt(y, mo, (*day).clamp(1, 28))
                        && let Some(t) = at(date, *hour, *minute)
                        && t > after
                    {
                        return Some(t);
                    }
                    mo += 1;
                    if mo > 12 {
                        mo = 1;
                        y += 1;
                    }
                }
                None
            }
            Trigger::FileChange { .. } | Trigger::Webhook { .. } => None,
        }
    }
}

fn parse_time(t: &str) -> Option<(u32, u32)> {
    let t = t.trim();
    let (num, pm) = if let Some(x) = t.strip_suffix("pm") {
        (x.trim(), Some(true))
    } else if let Some(x) = t.strip_suffix("am") {
        (x.trim(), Some(false))
    } else {
        (t.split_whitespace().next().unwrap_or(t), None)
    };
    let mut parts = num.split(':');
    let mut h: u32 = parts.next()?.trim().parse().ok()?;
    let m: u32 = parts.next().map(|m| m.trim().parse().unwrap_or(0)).unwrap_or(0);
    match pm {
        Some(true) if h < 12 => h += 12,
        Some(false) if h == 12 => h = 0,
        _ => {}
    }
    (h < 24 && m < 60).then_some((h, m))
}

pub fn weekday_name(d: u32) -> &'static str {
    ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][(d % 7) as usize]
}

fn weekday_from(s: &str) -> Option<u32> {
    let days = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];
    days.iter().find(|d| s.contains(&weekday_name(d.num_days_from_monday()).to_lowercase()[..3])).map(|d| d.num_days_from_monday())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub agent: String,
    /// Where results are posted. `None` means the agent's direct chat.
    pub thread: Option<String>,
    pub instructions: String,
    pub trigger: Trigger,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_status: Option<String>,
    pub created: DateTime<Utc>,
}

impl Routine {
    pub fn new(name: String, agent: String, instructions: String, trigger: Trigger) -> Self {
        Self {
            id: new_id(),
            name,
            agent,
            thread: None,
            instructions,
            trigger,
            enabled: true,
            last_run: None,
            last_status: None,
            created: Utc::now(),
        }
    }

    pub fn next_run(&self) -> Option<DateTime<Local>> {
        if !self.enabled {
            return None;
        }
        let base = self.last_run.unwrap_or(self.created).with_timezone(&Local);
        self.trigger.next_after(base)
    }
}

// ---------------------------------------------------------------------------
// Connectors (MCP servers) and fleet settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Transport {
    /// A local MCP server started as a child process speaking JSON-RPC over stdio.
    Stdio { command: String, args: Vec<String>, env: Vec<(String, String)> },
    /// A remote MCP server over Streamable HTTP.
    Http { url: String, headers: Vec<(String, String)> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Connector {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub transport: Transport,
    /// Tools that run without asking even when they are not marked read-only.
    #[serde(default)]
    pub trusted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FleetConfig {
    /// A folder of notes and documents every agent can search.
    pub knowledge_dir: Option<PathBuf>,
    /// Most handoffs in one chain of agent messages, to stop endless back-and-forth.
    pub max_hops: u32,
    /// Most model calls in one agent turn.
    pub max_steps: u32,
    pub webhook_port: u16,
    pub webhooks_enabled: bool,
    pub notifications: bool,
    pub connectors: Vec<Connector>,
    /// Chrome, Chromium or Edge executable. Empty means auto-detect.
    pub browser_path: String,
    /// Minutes before an open task with no updates gets a follow-up.
    pub stall_minutes: u32,
    pub onboarded: bool,
}

impl Default for FleetConfig {
    fn default() -> Self {
        Self {
            knowledge_dir: None,
            max_hops: 8,
            max_steps: 24,
            webhook_port: 8765,
            webhooks_enabled: false,
            notifications: true,
            connectors: Vec::new(),
            browser_path: String::new(),
            stall_minutes: 30,
            onboarded: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_schedules() {
        assert_eq!(Trigger::parse("every 30 minutes"), Some(Trigger::Every { minutes: 30 }));
        assert_eq!(Trigger::parse("Every 2 hours"), Some(Trigger::Every { minutes: 120 }));
        assert_eq!(Trigger::parse("hourly at :15"), Some(Trigger::Hourly { minute: 15 }));
        assert_eq!(Trigger::parse("daily at 9:30"), Some(Trigger::Daily { hour: 9, minute: 30 }));
        assert_eq!(Trigger::parse("daily at 5pm"), Some(Trigger::Daily { hour: 17, minute: 0 }));
        assert_eq!(Trigger::parse("weekly on friday at 16:00"), Some(Trigger::Weekly { weekday: 4, hour: 16, minute: 0 }));
        assert_eq!(Trigger::parse("monthly on day 3 at 08:00"), Some(Trigger::Monthly { day: 3, hour: 8, minute: 0 }));
        assert_eq!(Trigger::parse("whenever"), None);
    }

    #[test]
    fn next_runs() {
        let base = Local.with_ymd_and_hms(2026, 3, 4, 10, 20, 0).unwrap(); // a Wednesday
        assert_eq!(Trigger::Every { minutes: 15 }.next_after(base).unwrap(), base + Duration::minutes(15));
        assert_eq!(Trigger::Hourly { minute: 5 }.next_after(base).unwrap().hour(), 11);
        assert_eq!(Trigger::Hourly { minute: 45 }.next_after(base).unwrap().minute(), 45);
        let d = Trigger::Daily { hour: 9, minute: 0 }.next_after(base).unwrap();
        assert_eq!((d.day(), d.hour()), (5, 9));
        let w = Trigger::Weekly { weekday: 4, hour: 9, minute: 0 }.next_after(base).unwrap();
        assert_eq!(w.weekday(), Weekday::Fri);
        let m = Trigger::Monthly { day: 1, hour: 9, minute: 0 }.next_after(base).unwrap();
        assert_eq!((m.month(), m.day()), (4, 1));
    }
}
