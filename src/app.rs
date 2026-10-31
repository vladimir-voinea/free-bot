//! Application state and the logic that is not drawing: sending prompts,
//! applying stream events, titles, attachments and persistence.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use base64::Engine;
use chrono::Utc;
use egui_commonmark::CommonMarkCache;

use crate::api::{self, StreamEvent, StreamHandle, Waker};
use crate::store::{self, Attachment, AttachmentData, Conversation, Message, ModelRef, Role, Settings, Store, ThemePref, Variant};
use crate::ui::theme;

pub struct ActiveStream {
    pub handle: StreamHandle,
    pub msg_id: String,
    pub started: Instant,
    pub saw_reasoning: bool,
    pub saw_content: bool,
}

#[derive(Default)]
pub struct Composer {
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub think: bool,
    pub request_focus: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsTab {
    General,
    Providers,
    Personalize,
    Data,
    About,
}

/// Provider id (or `test:<id>`) and the model list or an error.
pub type ModelFetchResult = (String, Result<Vec<String>, String>);

pub struct SettingsState {
    pub tab: SettingsTab,
    pub provider_idx: usize,
    pub new_model: String,
    pub show_key: bool,
    pub fetching: Option<Receiver<ModelFetchResult>>,
    pub fetch_status: Option<Result<String, String>>,
    pub confirm_delete_all: bool,
}

impl SettingsState {
    pub fn new(tab: SettingsTab) -> Self {
        Self {
            tab,
            provider_idx: 0,
            new_model: String::new(),
            show_key: false,
            fetching: None,
            fetch_status: None,
            confirm_delete_all: false,
        }
    }
}

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub selected: usize,
    pub just_opened: bool,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

pub enum Confirm {
    DeleteChat(String),
}

pub struct FreeBotApp {
    pub store: Store,
    pub settings: Settings,
    /// Saved conversations, newest first.
    pub convs: Vec<Conversation>,
    /// The private chat that is open, if any. Never saved.
    pub private_conv: Option<Conversation>,
    /// Id of the open conversation. `None` shows the home screen.
    pub current: Option<String>,
    /// "Private" toggle for the next new chat.
    pub private_mode: bool,
    pub composer: Composer,
    pub streams: HashMap<String, ActiveStream>,
    pub md_cache: CommonMarkCache,
    pub textures: HashMap<String, egui::TextureHandle>,
    pub settings_ui: Option<SettingsState>,
    pub search: Option<SearchState>,
    pub renaming: Option<(String, String)>,
    pub rename_focus: bool,
    pub editing: Option<(String, String)>,
    pub confirm: Option<Confirm>,
    pub expanded_reasoning: HashSet<String>,
    pub hovered_msg: Option<String>,
    pub hovered_msg_next: Option<String>,
    pub scroll_to_bottom: bool,
    pub toasts: Vec<Toast>,
    /// Chats waiting for a generated title.
    titling: HashSet<String>,
    title_tx: Sender<(String, String)>,
    title_rx: Receiver<(String, String)>,
    pub waker: Arc<dyn Waker>,
    applied_theme: Option<ThemePref>,
    applied_scale: f32,
    settings_dirty: Option<Instant>,
    pub screenshot: Option<crate::ScreenshotPlan>,
}

impl FreeBotApp {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<crate::ScreenshotPlan>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply_style(&cc.egui_ctx);
        let store = Store::open_default();
        let settings = store.load_settings();
        let convs = store.load_conversations();
        let (title_tx, title_rx) = mpsc::channel();
        let mut app = Self {
            store,
            settings,
            convs,
            private_conv: None,
            current: None,
            private_mode: false,
            composer: Composer { request_focus: true, ..Default::default() },
            streams: HashMap::new(),
            md_cache: CommonMarkCache::default(),
            textures: HashMap::new(),
            settings_ui: None,
            search: None,
            renaming: None,
            rename_focus: false,
            editing: None,
            confirm: None,
            expanded_reasoning: HashSet::new(),
            hovered_msg: None,
            hovered_msg_next: None,
            scroll_to_bottom: false,
            toasts: Vec::new(),
            titling: HashSet::new(),
            title_tx,
            title_rx,
            waker: Arc::new(cc.egui_ctx.clone()),
            applied_theme: None,
            applied_scale: 0.0,
            settings_dirty: None,
            screenshot,
        };
        if let Some(plan) = &app.screenshot {
            if plan.view.starts_with("chat") {
                app.current = app.convs.first().map(|c| c.id.clone());
            }
            match plan.view.as_str() {
                "settings" => app.settings_ui = Some(SettingsState::new(SettingsTab::Providers)),
                "settings-general" => app.settings_ui = Some(SettingsState::new(SettingsTab::General)),
                "search" => app.open_search(),
                "rail" => app.settings.sidebar_open = false,
                "light" => app.settings.theme = ThemePref::Light,
                "chat-light" => app.settings.theme = ThemePref::Light,
                _ => {}
            }
        }
        app
    }

    // ---- lookup -----------------------------------------------------------

    pub fn conv(&self, id: &str) -> Option<&Conversation> {
        if let Some(p) = self.private_conv.as_ref().filter(|c| c.id == id) {
            return Some(p);
        }
        self.convs.iter().find(|c| c.id == id)
    }

    pub fn conv_mut(&mut self, id: &str) -> Option<&mut Conversation> {
        if self.private_conv.as_ref().is_some_and(|c| c.id == id) {
            return self.private_conv.as_mut();
        }
        self.convs.iter_mut().find(|c| c.id == id)
    }

    pub fn current_conv(&self) -> Option<&Conversation> {
        self.current.as_deref().and_then(|id| self.conv(id))
    }

    pub fn is_streaming(&self, conv_id: &str) -> bool {
        self.streams.contains_key(conv_id)
    }

    pub fn save_conv(&self, id: &str) {
        if let Some(c) = self.conv(id)
            && let Err(e) = self.store.save_conversation(c)
        {
            eprintln!("failed to save conversation: {e}");
        }
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty.get_or_insert_with(Instant::now);
    }

    pub fn save_settings(&mut self) {
        self.settings_dirty = None;
        if let Err(e) = self.store.save_settings(&self.settings) {
            self.toast_error(format!("Could not save settings: {e}"));
        }
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), error: false, until: Instant::now() + Duration::from_secs(3) });
    }

    pub fn toast_error(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), error: true, until: Instant::now() + Duration::from_secs(6) });
    }

    // ---- navigation -------------------------------------------------------

    pub fn new_chat(&mut self) {
        self.leave_private_chat();
        self.current = None;
        self.editing = None;
        self.composer.request_focus = true;
    }

    pub fn open_conv(&mut self, id: String) {
        if self.current.as_deref() != Some(id.as_str()) {
            self.leave_private_chat();
            self.current = Some(id);
            self.editing = None;
            self.scroll_to_bottom = true;
            self.composer.request_focus = true;
        }
    }

    /// Private chats disappear as soon as you navigate away from them.
    fn leave_private_chat(&mut self) {
        if let Some(p) = self.private_conv.take() {
            if let Some(s) = self.streams.remove(&p.id) {
                s.handle.cancel();
            }
            if self.current.as_deref() == Some(p.id.as_str()) {
                self.current = None;
            }
        }
    }

    pub fn open_search(&mut self) {
        self.search = Some(SearchState { just_opened: true, ..Default::default() });
    }

    pub fn open_settings(&mut self, tab: SettingsTab) {
        self.settings_ui = Some(SettingsState::new(tab));
    }

    pub fn delete_conv(&mut self, id: &str) {
        if let Some(s) = self.streams.remove(id) {
            s.handle.cancel();
        }
        self.convs.retain(|c| c.id != id);
        self.store.delete_conversation(id);
        if self.current.as_deref() == Some(id) {
            self.current = None;
        }
    }

    pub fn delete_all_convs(&mut self) {
        for (_, s) in self.streams.drain() {
            s.handle.cancel();
        }
        for c in std::mem::take(&mut self.convs) {
            self.store.delete_conversation(&c.id);
        }
        self.private_conv = None;
        self.current = None;
    }

    pub fn toggle_pin(&mut self, id: &str) {
        if let Some(c) = self.conv_mut(id) {
            c.pinned = !c.pinned;
        }
        self.save_conv(id);
    }

    pub fn start_rename(&mut self, id: &str, title: &str) {
        self.renaming = Some((id.to_string(), title.to_string()));
        self.rename_focus = true;
    }

    pub fn rename_conv(&mut self, id: &str, title: String) {
        let title = title.trim().to_string();
        if title.is_empty() {
            return;
        }
        self.titling.remove(id);
        if let Some(c) = self.conv_mut(id) {
            c.title = title;
            c.titled = true;
        }
        self.save_conv(id);
    }

    // ---- sending ----------------------------------------------------------

    fn model_or_prompt_setup(&mut self) -> Option<ModelRef> {
        match self.settings.effective_model() {
            Some(m) => Some(m),
            None => {
                self.toast_error("Add a provider and a model to start chatting.");
                self.open_settings(SettingsTab::Providers);
                None
            }
        }
    }

    /// Send the composer contents as a new prompt in the current chat (or a new one).
    pub fn send_composer(&mut self) {
        let text = self.composer.text.trim().to_string();
        if text.is_empty() && self.composer.attachments.is_empty() {
            return;
        }
        if self.current.as_deref().is_some_and(|id| self.is_streaming(id)) {
            return;
        }
        let Some(model) = self.model_or_prompt_setup() else { return };

        let conv_id = match self.current.clone() {
            Some(id) if self.conv(&id).is_some() => id,
            _ => {
                let mut c = Conversation::new(self.private_mode);
                c.title = store::fallback_title(if text.is_empty() { "New chat" } else { &text });
                let id = c.id.clone();
                if c.private {
                    self.private_conv = Some(c);
                } else {
                    self.convs.insert(0, c);
                }
                self.current = Some(id.clone());
                id
            }
        };
        let attachments = std::mem::take(&mut self.composer.attachments);
        let think = self.composer.think;
        self.composer.text.clear();
        if let Some(c) = self.conv_mut(&conv_id) {
            c.messages.push(Message::user(text, attachments, think));
            c.messages.push(Message::assistant(model.model.clone()));
            c.updated = Utc::now();
        }
        self.bump_to_top(&conv_id);
        self.start_stream(&conv_id, model, think);
    }

    fn bump_to_top(&mut self, id: &str) {
        if let Some(pos) = self.convs.iter().position(|c| c.id == id) {
            let c = self.convs.remove(pos);
            self.convs.insert(0, c);
        }
    }

    /// Stream an answer into the last message of the conversation, which must be an assistant message.
    fn start_stream(&mut self, conv_id: &str, model: ModelRef, think: bool) {
        let Some(provider) = self.settings.provider(&model.provider_id).cloned() else { return };
        let Some(conv) = self.conv(conv_id) else { return };
        let Some(last) = conv.messages.last() else { return };
        if last.role != Role::Assistant {
            return;
        }
        let msg_id = last.id.clone();
        let history = &conv.messages[..conv.messages.len() - 1];
        let req = api::ChatRequest {
            base_url: provider.base_url.clone(),
            api_key: provider.api_key.clone(),
            model: model.model.clone(),
            system_prompt: Some(self.system_prompt()),
            messages: api::build_messages(history),
            temperature: self.settings.temperature,
            max_tokens: self.settings.max_tokens,
            think,
        };
        let handle = api::stream_chat(req, self.waker.clone());
        self.streams.insert(
            conv_id.to_string(),
            ActiveStream { handle, msg_id, started: Instant::now(), saw_reasoning: false, saw_content: false },
        );
        self.scroll_to_bottom = true;
        self.save_conv(conv_id);
    }

    pub fn system_prompt(&self) -> String {
        let now = chrono::Local::now();
        let mut s = format!(
            "You are Free Bot, a helpful, witty and direct AI assistant. Answer clearly, use Markdown when it helps (headings, lists, tables, fenced code blocks with a language). The current date is {}.",
            now.format("%A, %B %-d, %Y")
        );
        if !self.settings.display_name.trim().is_empty() {
            s.push_str(&format!(" The user's name is {}.", self.settings.display_name.trim()));
        }
        if !self.settings.custom_instructions.trim().is_empty() {
            s.push_str("\n\nFollow these instructions from the user:\n");
            s.push_str(self.settings.custom_instructions.trim());
        }
        s
    }

    pub fn stop(&mut self, conv_id: &str) {
        if let Some(s) = self.streams.remove(conv_id) {
            s.handle.cancel();
            let started = s.started;
            if let Some(c) = self.conv_mut(conv_id)
                && let Some(m) = c.messages.iter_mut().find(|m| m.id == s.msg_id)
            {
                let v = m.current_mut();
                if v.thinking_secs.is_none() && !v.reasoning.is_empty() {
                    v.thinking_secs = Some(started.elapsed().as_secs_f32());
                }
                if v.content.is_empty() && v.reasoning.is_empty() {
                    v.error = Some("Stopped.".into());
                }
            }
            self.save_conv(conv_id);
        }
    }

    /// Generate a new answer for the assistant message at `idx`, keeping the old one as a variant.
    pub fn regenerate(&mut self, conv_id: &str, idx: usize) {
        if self.is_streaming(conv_id) {
            return;
        }
        let Some(model) = self.model_or_prompt_setup() else { return };
        let think = self.conv(conv_id).and_then(|c| c.messages[..idx].iter().rev().find(|m| m.role == Role::User)).is_some_and(|m| m.think);
        let Some(c) = self.conv_mut(conv_id) else { return };
        if idx >= c.messages.len() || c.messages[idx].role != Role::Assistant {
            return;
        }
        c.messages.truncate(idx + 1);
        let m = &mut c.messages[idx];
        let fresh = Variant { model: model.model.clone(), created: Some(Utc::now()), ..Default::default() };
        if m.current().content.is_empty() && m.current().reasoning.is_empty() {
            // A failed attempt is replaced instead of kept as a variant.
            *m.current_mut() = fresh;
        } else {
            m.variants.push(fresh);
            m.active = m.variants.len() - 1;
        }
        c.updated = Utc::now();
        self.start_stream(conv_id, model, think);
    }

    /// Replace the user message at `idx` with `text`, drop everything after it and answer again.
    pub fn submit_edit(&mut self, conv_id: &str, idx: usize, text: String) {
        if self.is_streaming(conv_id) || text.trim().is_empty() {
            return;
        }
        let Some(model) = self.model_or_prompt_setup() else { return };
        let Some(c) = self.conv_mut(conv_id) else { return };
        if idx >= c.messages.len() || c.messages[idx].role != Role::User {
            return;
        }
        c.messages.truncate(idx + 1);
        c.messages[idx].current_mut().content = text.trim().to_string();
        let think = c.messages[idx].think;
        c.messages.push(Message::assistant(model.model.clone()));
        c.updated = Utc::now();
        self.editing = None;
        self.start_stream(conv_id, model, think);
    }

    pub fn set_feedback(&mut self, conv_id: &str, msg_id: &str, value: bool) {
        if let Some(c) = self.conv_mut(conv_id)
            && let Some(m) = c.messages.iter_mut().find(|m| m.id == msg_id)
        {
            let v = m.current_mut();
            v.feedback = if v.feedback == Some(value) { None } else { Some(value) };
        }
        self.save_conv(conv_id);
    }

    pub fn switch_variant(&mut self, conv_id: &str, msg_id: &str, delta: isize) {
        if self.is_streaming(conv_id) {
            return;
        }
        if let Some(c) = self.conv_mut(conv_id)
            && let Some(m) = c.messages.iter_mut().find(|m| m.id == msg_id)
        {
            let n = m.variants.len() as isize;
            m.active = (m.active as isize + delta).clamp(0, n - 1) as usize;
        }
        self.save_conv(conv_id);
    }

    // ---- per-frame processing --------------------------------------------

    /// Apply incoming stream events and finished titles. Called once per frame.
    pub fn pump(&mut self) {
        let ids: Vec<String> = self.streams.keys().cloned().collect();
        for conv_id in ids {
            let mut finished: Option<Option<String>> = None;
            let mut got_any = false;
            while let Some(active) = self.streams.get_mut(&conv_id) {
                let ev = match active.handle.rx.try_recv() {
                    Ok(ev) => ev,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        finished = Some(None);
                        break;
                    }
                };
                got_any = true;
                let msg_id = active.msg_id.clone();
                let started = active.started;
                let first_content = matches!(ev, StreamEvent::Content(_)) && !active.saw_content;
                match &ev {
                    StreamEvent::Reasoning(_) => active.saw_reasoning = true,
                    StreamEvent::Content(_) => active.saw_content = true,
                    _ => {}
                }
                let saw_reasoning = active.saw_reasoning;
                let Some(c) = self.conv_mut(&conv_id) else {
                    finished = Some(None);
                    break;
                };
                let Some(m) = c.messages.iter_mut().find(|m| m.id == msg_id) else {
                    finished = Some(None);
                    break;
                };
                let v = m.current_mut();
                match ev {
                    StreamEvent::Reasoning(r) => v.reasoning.push_str(&r),
                    StreamEvent::Content(t) => {
                        if first_content && saw_reasoning && v.thinking_secs.is_none() {
                            v.thinking_secs = Some(started.elapsed().as_secs_f32());
                        }
                        v.content.push_str(&t);
                    }
                    StreamEvent::Done => {
                        finished = Some(None);
                        break;
                    }
                    StreamEvent::Error(e) => {
                        finished = Some(Some(e));
                        break;
                    }
                }
            }
            if got_any && self.current.as_deref() == Some(conv_id.as_str()) {
                self.scroll_to_bottom = true;
            }
            if let Some(err) = finished {
                self.finish_stream(&conv_id, err);
            }
        }

        while let Ok((conv_id, title)) = self.title_rx.try_recv() {
            if !self.titling.remove(&conv_id) {
                continue; // renamed by the user meanwhile
            }
            if let Some(c) = self.conv_mut(&conv_id) {
                c.title = title;
                c.titled = true;
            }
            self.save_conv(&conv_id);
        }
    }

    fn finish_stream(&mut self, conv_id: &str, err: Option<String>) {
        let Some(active) = self.streams.remove(conv_id) else { return };
        let started = active.started;
        let mut needs_title = None;
        let auto_title = self.settings.auto_title;
        if let Some(c) = self.conv_mut(conv_id) {
            if let Some(m) = c.messages.iter_mut().find(|m| m.id == active.msg_id) {
                let v = m.current_mut();
                if !v.reasoning.is_empty() && v.thinking_secs.is_none() {
                    v.thinking_secs = Some(started.elapsed().as_secs_f32());
                }
                if let Some(e) = err {
                    if !e.is_empty() {
                        v.error = Some(e);
                    }
                } else if v.content.trim().is_empty() && v.reasoning.trim().is_empty() {
                    v.error = Some("The model returned an empty response.".into());
                }
            }
            c.updated = Utc::now();
            if auto_title && !c.titled && c.messages.len() >= 2 {
                let prompt = c.messages[0].text().to_string();
                let answer = c.messages[1].text().to_string();
                if !answer.trim().is_empty() {
                    needs_title = Some((prompt, answer));
                }
            }
        }
        self.save_conv(conv_id);
        if let Some((prompt, answer)) = needs_title {
            self.request_title(conv_id, prompt, answer);
        }
    }

    fn request_title(&mut self, conv_id: &str, prompt: String, answer: String) {
        let Some(model) = self.settings.effective_model() else { return };
        let Some(p) = self.settings.provider(&model.provider_id).cloned() else { return };
        // Ask only once per chat; the fallback title stays if this fails.
        if !self.titling.insert(conv_id.to_string()) {
            return;
        }
        if let Some(c) = self.conv_mut(conv_id) {
            c.titled = true;
        }
        let tx = self.title_tx.clone();
        let waker = self.waker.clone();
        let id = conv_id.to_string();
        std::thread::spawn(move || {
            if let Some(t) = api::generate_title(&p.base_url, &p.api_key, &model.model, &prompt, &answer) {
                let _ = tx.send((id, t));
                waker.wake();
            }
        });
    }

    /// Apply theme and zoom when they change, and flush settings.
    pub fn sync_settings(&mut self, ctx: &egui::Context) {
        if self.applied_theme != Some(self.settings.theme) {
            self.applied_theme = Some(self.settings.theme);
            ctx.set_theme(match self.settings.theme {
                ThemePref::Dark => egui::ThemePreference::Dark,
                ThemePref::Light => egui::ThemePreference::Light,
                ThemePref::System => egui::ThemePreference::System,
            });
        }
        let scale = self.settings.ui_scale.clamp(0.75, 1.75);
        if (self.applied_scale - scale).abs() > f32::EPSILON {
            self.applied_scale = scale;
            ctx.set_zoom_factor(scale);
        }
        // Debounced so typing in a settings field does not write on every keystroke.
        if let Some(since) = self.settings_dirty {
            let wait = Duration::from_millis(700);
            if since.elapsed() >= wait {
                self.save_settings();
            } else {
                ctx.request_repaint_after(wait - since.elapsed());
            }
        }
        self.toasts.retain(|t| t.until > Instant::now());
    }

    // ---- attachments ------------------------------------------------------

    pub fn pick_attachments(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Attach files")
            .add_filter(
                "Images and text",
                &[
                    "png", "jpg", "jpeg", "gif", "webp", "txt", "md", "csv", "json", "rs", "py", "js", "ts", "html", "css", "toml", "yaml",
                    "yml", "xml", "log", "c", "cpp", "h", "java", "go", "sh",
                ],
            )
            .add_filter("All files", &["*"])
            .pick_files();
        for path in files.unwrap_or_default() {
            self.attach_path(&path);
        }
    }

    pub fn attach_path(&mut self, path: &std::path::Path) {
        match load_attachment(path) {
            Ok(a) => self.composer.attachments.push(a),
            Err(e) => self.toast_error(e),
        }
    }

    pub fn attach_bytes(&mut self, name: &str, bytes: &[u8]) {
        match attachment_from_bytes(name, bytes) {
            Ok(a) => self.composer.attachments.push(a),
            Err(e) => self.toast_error(e),
        }
    }

    pub fn texture_for(&mut self, ctx: &egui::Context, a: &Attachment) -> Option<egui::TextureHandle> {
        if let Some(t) = self.textures.get(&a.id) {
            return Some(t.clone());
        }
        let AttachmentData::Image { base64, .. } = &a.data else { return None };
        let bytes = base64::engine::general_purpose::STANDARD.decode(base64).ok()?;
        let img = image::load_from_memory(&bytes).ok()?.thumbnail(256, 256).to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        let tex = ctx.load_texture(format!("att-{}", a.id), color, egui::TextureOptions::LINEAR);
        self.textures.insert(a.id.clone(), tex.clone());
        Some(tex)
    }

    // ---- export -----------------------------------------------------------

    pub fn export_conv_markdown(&mut self, id: &str) {
        let Some(c) = self.conv(id) else { return };
        let md = c.to_markdown();
        let name = sanitize_filename(&c.title);
        if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{name}.md")).add_filter("Markdown", &["md"]).save_file() {
            match std::fs::write(&path, md) {
                Ok(()) => self.toast(format!("Saved {}", path.display())),
                Err(e) => self.toast_error(format!("Could not save: {e}")),
            }
        }
    }

    pub fn export_all_json(&mut self) {
        if let Some(path) = rfd::FileDialog::new().set_file_name("free-bot-chats.json").add_filter("JSON", &["json"]).save_file() {
            let res = serde_json::to_vec_pretty(&self.convs)
                .map_err(|e| e.to_string())
                .and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()));
            match res {
                Ok(()) => self.toast(format!("Exported {} chats", self.convs.len())),
                Err(e) => self.toast_error(format!("Export failed: {e}")),
            }
        }
    }
}

const MAX_TEXT_BYTES: usize = 512 * 1024;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

pub fn load_attachment(path: &std::path::Path) -> Result<Attachment, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read {name}: {e}"))?;
    attachment_from_bytes(&name, &bytes)
}

pub fn attachment_from_bytes(name: &str, bytes: &[u8]) -> Result<Attachment, String> {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    };
    let data = if let Some(mime) = mime {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(format!("{name} is too large (max 20 MB)."));
        }
        AttachmentData::Image { mime: mime.into(), base64: base64::engine::general_purpose::STANDARD.encode(bytes) }
    } else {
        if bytes.len() > MAX_TEXT_BYTES {
            return Err(format!("{name} is too large (max 512 KB for text files)."));
        }
        let content = String::from_utf8(bytes.to_vec()).map_err(|_| format!("{name} is not a text file or a supported image."))?;
        AttachmentData::Text { content }
    };
    Ok(Attachment { id: store::new_id(), name: name.to_string(), data })
}

fn sanitize_filename(s: &str) -> String {
    let out: String = s.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    let out = out.trim().to_string();
    if out.is_empty() { "chat".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_by_type() {
        let a = attachment_from_bytes("notes.md", b"# hi").unwrap();
        assert!(matches!(a.data, AttachmentData::Text { .. }));
        let a = attachment_from_bytes("pic.PNG", &[1, 2, 3]).unwrap();
        assert!(a.is_image());
        assert!(attachment_from_bytes("bin.dat", &[0xff, 0xfe, 0x00]).is_err());
    }

    #[test]
    fn filenames() {
        assert_eq!(sanitize_filename("a/b: c"), "a_b_ c");
        assert_eq!(sanitize_filename("///"), "___");
        assert_eq!(sanitize_filename("  "), "chat");
    }
}
