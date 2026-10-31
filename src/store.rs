//! Data model and on-disk persistence.
//!
//! Settings live in `settings.json` and every conversation is its own JSON file
//! under `conversations/`. Both sit in the platform data directory, or in the
//! directory named by the `FREEBOT_DATA_DIR` environment variable.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// Base URL of an OpenAI-compatible API, for example `https://api.openai.com/v1`.
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<String>,
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "My provider".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            models: Vec::new(),
        }
    }
}

/// Ready-made starting points shown in the "Add provider" menu.
pub const PROVIDER_PRESETS: &[(&str, &str, &[&str])] = &[
    ("OpenAI", "https://api.openai.com/v1", &["gpt-4o-mini", "gpt-4o"]),
    ("OpenRouter", "https://openrouter.ai/api/v1", &["openai/gpt-4o-mini"]),
    ("Groq", "https://api.groq.com/openai/v1", &["llama-3.3-70b-versatile"]),
    ("Mistral", "https://api.mistral.ai/v1", &["mistral-small-latest"]),
    ("DeepSeek", "https://api.deepseek.com/v1", &["deepseek-chat", "deepseek-reasoner"]),
    ("Together", "https://api.together.xyz/v1", &[]),
    ("Ollama (local)", "http://localhost:11434/v1", &["llama3.2"]),
    ("LM Studio (local)", "http://localhost:1234/v1", &[]),
    ("Custom", "", &[]),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelRef {
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ThemePref {
    #[default]
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub providers: Vec<Provider>,
    pub selected_model: Option<ModelRef>,
    pub theme: ThemePref,
    /// Name used in the home screen greeting. Empty means no name.
    pub display_name: String,
    /// Extra instructions sent as the system prompt of every chat.
    pub custom_instructions: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub enter_to_send: bool,
    pub auto_title: bool,
    pub show_model_name: bool,
    pub sidebar_open: bool,
    pub ui_scale: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            selected_model: None,
            theme: ThemePref::Dark,
            display_name: String::new(),
            custom_instructions: String::new(),
            temperature: None,
            max_tokens: None,
            enter_to_send: true,
            auto_title: true,
            show_model_name: true,
            sidebar_open: true,
            ui_scale: 1.0,
        }
    }
}

impl Settings {
    pub fn provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// The selected model if it still exists, otherwise the first available model.
    pub fn effective_model(&self) -> Option<ModelRef> {
        if let Some(sel) = &self.selected_model
            && let Some(p) = self.provider(&sel.provider_id)
            && !sel.model.is_empty()
            && (p.models.is_empty() || p.models.contains(&sel.model))
        {
            return Some(sel.clone());
        }
        self.providers.iter().find_map(|p| p.models.first().map(|m| ModelRef { provider_id: p.id.clone(), model: m.clone() }))
    }

    pub fn all_models(&self) -> Vec<(ModelRef, String)> {
        let mut out = Vec::new();
        for p in &self.providers {
            for m in &p.models {
                out.push((ModelRef { provider_id: p.id.clone(), model: m.clone() }, p.name.clone()));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Conversations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AttachmentData {
    /// Base64-encoded image bytes.
    Image { mime: String, base64: String },
    /// A text file, inlined into the prompt.
    Text { content: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub data: AttachmentData,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        matches!(self.data, AttachmentData::Image { .. })
    }
}

/// One answer to a prompt. Regenerating an answer adds another variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Variant {
    pub content: String,
    pub reasoning: String,
    pub model: String,
    /// Seconds spent in the reasoning phase, when the model reasoned.
    pub thinking_secs: Option<f32>,
    pub error: Option<String>,
    /// `Some(true)` for thumbs up, `Some(false)` for thumbs down.
    pub feedback: Option<bool>,
    pub created: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub variants: Vec<Variant>,
    pub active: usize,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Whether "Think" was on when this prompt was sent.
    #[serde(default)]
    pub think: bool,
}

impl Message {
    pub fn user(text: String, attachments: Vec<Attachment>, think: bool) -> Self {
        Self {
            id: new_id(),
            role: Role::User,
            variants: vec![Variant { content: text, created: Some(Utc::now()), ..Default::default() }],
            active: 0,
            attachments,
            think,
        }
    }

    pub fn assistant(model: String) -> Self {
        Self {
            id: new_id(),
            role: Role::Assistant,
            variants: vec![Variant { model, created: Some(Utc::now()), ..Default::default() }],
            active: 0,
            attachments: Vec::new(),
            think: false,
        }
    }

    pub fn current(&self) -> &Variant {
        &self.variants[self.active.min(self.variants.len() - 1)]
    }

    pub fn current_mut(&mut self) -> &mut Variant {
        let i = self.active.min(self.variants.len() - 1);
        &mut self.variants[i]
    }

    pub fn text(&self) -> &str {
        &self.current().content
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    #[serde(default)]
    pub pinned: bool,
    pub messages: Vec<Message>,
    /// Private chats are never written to disk and never shown in history.
    #[serde(skip)]
    pub private: bool,
    /// The title was set by the user or the model and should not be replaced.
    #[serde(default)]
    pub titled: bool,
}

impl Conversation {
    pub fn new(private: bool) -> Self {
        let now = Utc::now();
        Self {
            id: new_id(),
            title: "New chat".into(),
            created: now,
            updated: now,
            pinned: false,
            messages: Vec::new(),
            private,
            titled: false,
        }
    }

    /// The whole conversation as Markdown, for export and sharing.
    pub fn to_markdown(&self) -> String {
        let mut s = format!("# {}\n\n", self.title);
        for m in &self.messages {
            let who = match m.role {
                Role::User => "You",
                Role::Assistant => "Free Bot",
            };
            s.push_str(&format!("**{who}:**\n\n{}\n\n", m.text().trim()));
            for a in &m.attachments {
                s.push_str(&format!("_Attachment: {}_\n\n", a.name));
            }
        }
        s
    }
}

/// A short title made from the first prompt, used until a better one exists.
pub fn fallback_title(prompt: &str) -> String {
    let line = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("New chat").trim();
    let mut out: String = line.chars().take(48).collect();
    if line.chars().count() > 48 {
        out = out.trim_end().to_string();
        out.push('…');
    }
    out
}

/// Sidebar history groups, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DateGroup {
    Pinned,
    Today,
    Yesterday,
    Previous7,
    Previous30,
    Older,
}

impl DateGroup {
    pub fn label(self) -> &'static str {
        match self {
            DateGroup::Pinned => "Pinned",
            DateGroup::Today => "Today",
            DateGroup::Yesterday => "Yesterday",
            DateGroup::Previous7 => "Previous 7 days",
            DateGroup::Previous30 => "Previous 30 days",
            DateGroup::Older => "Older",
        }
    }

    pub fn of(updated: DateTime<Utc>, pinned: bool, today: NaiveDate) -> Self {
        if pinned {
            return DateGroup::Pinned;
        }
        let day = updated.with_timezone(&Local).date_naive();
        let days = (today - day).num_days();
        match days {
            i64::MIN..=0 => DateGroup::Today,
            1 => DateGroup::Yesterday,
            2..=7 => DateGroup::Previous7,
            8..=30 => DateGroup::Previous30,
            _ => DateGroup::Older,
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open_default() -> Self {
        let root = std::env::var_os("FREEBOT_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| directories::ProjectDirs::from("dev", "FreeBot", "Free Bot").map(|d| d.data_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from(".free-bot"));
        Self::open(root)
    }

    pub fn open(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let _ = fs::create_dir_all(root.join("conversations"));
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    fn conv_path(&self, id: &str) -> PathBuf {
        // Ids are UUIDs we generate, but never trust them as path components.
        let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        self.root.join("conversations").join(format!("{safe}.json"))
    }

    pub fn load_settings(&self) -> Settings {
        fs::read(self.settings_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> std::io::Result<()> {
        // The file holds API keys, so keep it private to the user.
        write_atomic(&self.settings_path(), &serde_json::to_vec_pretty(s)?, true)
    }

    pub fn load_conversations(&self) -> Vec<Conversation> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(self.root.join("conversations")) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "json")
                    && let Some(c) = fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Conversation>(&b).ok())
                {
                    out.push(c);
                }
            }
        }
        out.sort_by_key(|c| std::cmp::Reverse(c.updated));
        out
    }

    pub fn save_conversation(&self, c: &Conversation) -> std::io::Result<()> {
        if c.private || c.messages.is_empty() {
            return Ok(());
        }
        write_atomic(&self.conv_path(&c.id), &serde_json::to_vec(c)?, false)
    }

    pub fn delete_conversation(&self, id: &str) {
        let _ = fs::remove_file(self.conv_path(id));
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_and_conversations_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path());
        let mut s = Settings::default();
        s.providers.push(Provider { models: vec!["m1".into()], ..Default::default() });
        store.save_settings(&s).unwrap();
        assert_eq!(store.load_settings(), s);

        let mut c = Conversation::new(false);
        c.messages.push(Message::user("hello".into(), vec![], false));
        store.save_conversation(&c).unwrap();
        let loaded = store.load_conversations();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].messages[0].text(), "hello");

        store.delete_conversation(&c.id);
        assert!(store.load_conversations().is_empty());
    }

    #[test]
    fn private_chats_are_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path());
        let mut c = Conversation::new(true);
        c.messages.push(Message::user("secret".into(), vec![], false));
        store.save_conversation(&c).unwrap();
        assert!(store.load_conversations().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn settings_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path());
        store.save_settings(&Settings::default()).unwrap();
        let mode = fs::metadata(dir.path().join("settings.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn effective_model_falls_back() {
        let mut s = Settings::default();
        assert!(s.effective_model().is_none());
        let p = Provider { models: vec!["a".into(), "b".into()], ..Default::default() };
        let pid = p.id.clone();
        s.providers.push(p);
        assert_eq!(s.effective_model().unwrap().model, "a");
        s.selected_model = Some(ModelRef { provider_id: pid.clone(), model: "b".into() });
        assert_eq!(s.effective_model().unwrap().model, "b");
        s.selected_model = Some(ModelRef { provider_id: pid, model: "gone".into() });
        assert_eq!(s.effective_model().unwrap().model, "a");
    }

    #[test]
    fn date_groups() {
        let today = Local::now().date_naive();
        let now = Utc::now();
        assert_eq!(DateGroup::of(now, false, today), DateGroup::Today);
        assert_eq!(DateGroup::of(now, true, today), DateGroup::Pinned);
        assert_eq!(DateGroup::of(now - chrono::Duration::days(1), false, today), DateGroup::Yesterday);
        assert_eq!(DateGroup::of(now - chrono::Duration::days(5), false, today), DateGroup::Previous7);
        assert_eq!(DateGroup::of(now - chrono::Duration::days(20), false, today), DateGroup::Previous30);
        assert_eq!(DateGroup::of(now - chrono::Duration::days(90), false, today), DateGroup::Older);
    }

    #[test]
    fn fallback_titles() {
        assert_eq!(fallback_title("\n  Hello there \nsecond"), "Hello there");
        let long = "a".repeat(80);
        assert!(fallback_title(&long).ends_with('…'));
        assert_eq!(fallback_title("   "), "New chat");
    }
}
