//! A thread: a direct chat with a bot, a group channel, or a chat between two
//! bots. Messages are grouped by author, tool calls show as compact cards,
//! and approvals appear inline.

use egui::{Align, Align2, CornerRadius, Layout, Sense, Stroke, Vec2, pos2, vec2};
use egui_commonmark::CommonMarkViewer;

use super::super::icons::{self, Icon};
use super::super::theme::{self, Palette};
use super::super::widgets;
use super::{PanelTab, avatar, paint_avatar, short_time};
use crate::agents::engine::Decision;
use crate::agents::model::{Agent, Author, Thread, ThreadKind, ThreadMessage, ToolCall, ToolStatus};
use crate::agents::tools::describe_call;
use crate::app::FreeBotApp;

const THREAD_WIDTH: f32 = 820.0;

struct Snapshot {
    thread: Thread,
    agents: Vec<Agent>,
    /// (agent id, status) for agents working in this thread.
    working: Vec<(String, String, u64)>,
    user: String,
}

impl Snapshot {
    fn agent(&self, id: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| a.id == id)
    }
}

impl FreeBotApp {
    pub fn thread_ui(&mut self, ui: &mut egui::Ui, thread_id: &str) {
        let snap = {
            let st = self.fleet.lock();
            let Some(t) = st.thread(thread_id) else {
                drop(st);
                self.page = None;
                return;
            };
            let working = st
                .runs
                .iter()
                .filter(|((_, t), _)| t == thread_id)
                .map(|((a, _), r)| (a.clone(), r.status.clone(), r.started.elapsed().as_secs()))
                .collect();
            Snapshot { thread: t.clone(), agents: st.agents.clone(), working, user: st.settings.display_name.trim().to_string() }
        };
        let dm_agent = match &snap.thread.kind {
            ThreadKind::Dm { agent } => snap.agent(agent).cloned(),
            _ => None,
        };

        self.thread_top_bar(ui, &snap, dm_agent.as_ref());

        let p = Palette::of(ui);
        if let (Some(tab), Some(a)) = (self.agent_ui.panel, dm_agent.as_ref()) {
            egui::Panel::right("agent-panel")
                .exact_size(400.0)
                .resizable(false)
                .show_separator_line(true)
                .frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin::symmetric(14, 12)))
                .show(ui, |ui| self.agent_panel_ui(ui, a, tab));
        }

        let read_only = matches!(snap.thread.kind, ThreadKind::AgentDm { .. });
        egui::Panel::bottom(("thread-composer", thread_id))
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 16, right: 16, top: 6, bottom: 10 }))
            .show_separator_line(false)
            .resizable(false)
            .show(ui, |ui| {
                super::super::chat::centered_column(ui, THREAD_WIDTH, |ui| {
                    if read_only {
                        ui.vertical_centered(|ui| {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new("Bots talk to each other here. You can read along; message them in their own chats.")
                                    .font(theme::font(13.0))
                                    .color(p.muted),
                            );
                            ui.add_space(6.0);
                        });
                    } else {
                        self.thread_composer(ui, &snap);
                    }
                });
            });

        let scroll = std::mem::take(&mut self.agent_ui.scroll_bottom);
        egui::ScrollArea::vertical().id_salt(("thread", thread_id)).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.add_space(10.0);
            super::super::chat::centered_column(ui, THREAD_WIDTH, |ui| {
                if snap.thread.messages.is_empty() {
                    self.thread_intro(ui, &snap, dm_agent.as_ref());
                }
                let mut prev: Option<&ThreadMessage> = None;
                for m in &snap.thread.messages {
                    let grouped = prev.is_some_and(|pm| {
                        pm.author == m.author && m.author != Author::System && (m.created - pm.created).num_seconds() < 300
                    });
                    self.message_ui(ui, &snap, m, grouped);
                    prev = Some(m);
                }
                for (aid, status, secs) in &snap.working {
                    let streaming_visible =
                        snap.thread.messages.last().is_some_and(|m| m.streaming && m.author == Author::Agent(aid.clone()));
                    if !streaming_visible {
                        working_line(ui, snap.agent(aid), status, *secs);
                    }
                }
                ui.add_space(20.0);
            });
            if scroll {
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }
        });
    }

    fn thread_top_bar(&mut self, ui: &mut egui::Ui, snap: &Snapshot, dm_agent: Option<&Agent>) {
        let p = Palette::of(ui);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
        let mut bar =
            ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 10.0))).layout(Layout::left_to_right(Align::Center)));
        let ui = &mut bar;
        match &snap.thread.kind {
            ThreadKind::Dm { .. } => {
                if let Some(a) = dm_agent {
                    if avatar(ui, a, 30.0).on_hover_text("Profile").clicked() {
                        self.agent_ui.panel = Some(PanelTab::Profile);
                    }
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(&a.name).font(theme::font_bold(15.5)).color(p.text));
                    if !a.title.is_empty() {
                        ui.label(egui::RichText::new(&a.title).font(theme::font(13.5)).color(p.muted));
                    }
                }
            }
            ThreadKind::Channel { name, members, .. } => {
                widgets::icon(ui, Icon::Hash, 22.0, p.muted);
                ui.label(egui::RichText::new(name).font(theme::font_bold(15.5)).color(p.text));
                ui.add_space(8.0);
                for m in members.iter().take(8) {
                    if let Some(a) = snap.agent(m) {
                        avatar(ui, a, 20.0).on_hover_text(a.label());
                    }
                }
            }
            ThreadKind::AgentDm { a, b } => {
                for id in [a, b] {
                    if let Some(ag) = snap.agent(id) {
                        avatar(ui, ag, 24.0);
                    }
                }
                let n = |id: &str| snap.agent(id).map(|x| x.name.clone()).unwrap_or_default();
                ui.label(egui::RichText::new(format!("{} & {}", n(a), n(b))).font(theme::font_bold(15.5)).color(p.text));
            }
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(a) = dm_agent {
                let open = self.agent_ui.panel;
                let tab_btn = |ui: &mut egui::Ui, icon: Icon, tip: &str, tab: PanelTab| -> bool {
                    let on = open == Some(tab);
                    widgets::icon_button_sized(ui, icon, tip, 32.0, on.then_some(p.text)).clicked()
                };
                if tab_btn(ui, Icon::Brain, "Memory", PanelTab::Memory) {
                    self.agent_ui.panel = if open == Some(PanelTab::Memory) { None } else { Some(PanelTab::Memory) };
                }
                if tab_btn(ui, Icon::Terminal, "Computer: browser, files, terminal", PanelTab::Computer) {
                    self.agent_ui.panel = if open == Some(PanelTab::Computer) { None } else { Some(PanelTab::Computer) };
                }
                if tab_btn(ui, Icon::Gear, "Profile and settings", PanelTab::Profile) {
                    self.agent_ui.panel = if open == Some(PanelTab::Profile) { None } else { Some(PanelTab::Profile) };
                }
                let _ = a;
            } else if matches!(snap.thread.kind, ThreadKind::Channel { .. })
                && widgets::icon_button(ui, Icon::Users, "Edit channel").clicked()
            {
                self.edit_channel(&snap.thread.id);
            }
            if !snap.working.is_empty() {
                ui.add_space(6.0);
                if widgets::secondary_button(ui, "Stop").on_hover_text("Stop every bot working here").clicked() {
                    self.fleet.stop_thread(&snap.thread.id);
                }
            }
        });
    }

    fn thread_intro(&mut self, ui: &mut egui::Ui, snap: &Snapshot, dm_agent: Option<&Agent>) {
        let p = Palette::of(ui);
        ui.add_space(40.0);
        ui.vertical_centered(|ui| match (&snap.thread.kind, dm_agent) {
            (ThreadKind::Dm { .. }, Some(a)) => {
                avatar(ui, a, 64.0);
                ui.add_space(10.0);
                ui.label(egui::RichText::new(&a.name).font(theme::font_bold(22.0)).color(p.text));
                if !a.title.is_empty() {
                    ui.label(egui::RichText::new(&a.title).font(theme::font(15.0)).color(p.muted));
                }
                ui.add_space(8.0);
                ui.set_max_width(520.0);
                ui.add(egui::Label::new(egui::RichText::new(&a.description).font(theme::font(14.0)).color(p.muted)).wrap());
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("Give it a job and leave. It works with its own browser, files and terminal, and comes back when it needs you.")
                        .font(theme::font(13.0))
                        .color(p.faint),
                );
            }
            (ThreadKind::Channel { name, .. }, _) => {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Hash, p.muted);
                ui.label(egui::RichText::new(format!("#{name}")).font(theme::font_bold(22.0)).color(p.text));
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Messages that mention nobody go to the lead bot. Mention @Name to talk to a specific bot. Bots hand work to each other here.")
                        .font(theme::font(13.5))
                        .color(p.muted),
                );
            }
            _ => {
                ui.label(egui::RichText::new("No messages yet.").font(theme::font(14.0)).color(p.muted));
            }
        });
        ui.add_space(30.0);
    }

    fn message_ui(&mut self, ui: &mut egui::Ui, snap: &Snapshot, m: &ThreadMessage, grouped: bool) {
        let p = Palette::of(ui);
        if m.author == Author::System {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(640.0);
                        let first = m.content.lines().next().unwrap_or("");
                        let expanded = self.agent_ui.expanded_calls.contains(&m.id);
                        let text = if expanded { m.content.as_str() } else { first };
                        let resp = ui.add(
                            egui::Label::new(egui::RichText::new(text).font(theme::font(13.0)).color(p.muted)).wrap().sense(Sense::click()),
                        );
                        if m.content.lines().count() > 1 && resp.on_hover_text("Show details").clicked() {
                            if expanded {
                                self.agent_ui.expanded_calls.remove(&m.id);
                            } else {
                                self.agent_ui.expanded_calls.insert(m.id.clone());
                            }
                        }
                    });
            });
            ui.add_space(4.0);
            return;
        }

        let indent = 44.0;
        if !grouped {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                match &m.author {
                    Author::Agent(id) => match snap.agent(id) {
                        Some(a) => {
                            avatar(ui, a, 34.0);
                        }
                        None => {
                            ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                        }
                    },
                    _ => {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 17.0, p.selected);
                        let initial = snap.user.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "Y".into());
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, initial, theme::font_bold(15.0), p.text);
                    }
                }
                ui.add_space(4.0);
                let (name, title) = match &m.author {
                    Author::Agent(id) => {
                        snap.agent(id).map(|a| (a.name.clone(), a.title.clone())).unwrap_or(("Removed bot".into(), String::new()))
                    }
                    _ => (if snap.user.is_empty() { "You".into() } else { snap.user.clone() }, String::new()),
                };
                ui.label(egui::RichText::new(name).font(theme::font_bold(14.5)).color(p.text));
                if !title.is_empty() {
                    ui.label(egui::RichText::new(title).font(theme::font(12.5)).color(p.faint));
                }
                ui.label(egui::RichText::new(short_time(m.created)).font(theme::font(12.0)).color(p.faint));
            });
        } else {
            ui.add_space(2.0);
        }

        egui::Frame::new().inner_margin(egui::Margin { left: indent as i8, right: 0, top: if grouped { 0 } else { -6 }, bottom: 0 }).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                if !m.reasoning.trim().is_empty() {
                    let open = self.agent_ui.expanded_reasoning.contains(&m.id);
                    let label = if m.streaming && m.content.is_empty() { "Thinking…" } else { "Thought process" };
                    let resp = small_toggle(ui, Icon::Bulb, label, open);
                    if resp.clicked() {
                        if open {
                            self.agent_ui.expanded_reasoning.remove(&m.id);
                        } else {
                            self.agent_ui.expanded_reasoning.insert(m.id.clone());
                        }
                    }
                    if open {
                        ui.add(egui::Label::new(egui::RichText::new(m.reasoning.trim()).font(theme::font(13.0)).color(p.muted)).wrap());
                    }
                }
                if !m.content.trim().is_empty() {
                    let viewer = CommonMarkViewer::new()
                        .syntax_theme_dark("base16-ocean.dark")
                        .syntax_theme_light("InspiredGitHub")
                        .indentation_spaces(2);
                    ui.scope(|ui| {
                        ui.style_mut().override_text_style = Some(egui::TextStyle::Body);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        viewer.show(ui, &mut self.md_cache, &m.content);
                    });
                } else if m.streaming && m.tool_calls.is_empty() && m.reasoning.is_empty() {
                    typing(ui);
                }
                for c in &m.tool_calls {
                    self.tool_card(ui, c);
                }
                if let Some(err) = &m.error {
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(p.danger.gamma_multiply(0.12))
                        .stroke(Stroke::new(1.0, p.danger.gamma_multiply(0.6)))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(egui::Label::new(egui::RichText::new(err).font(theme::font(13.5)).color(p.text)).wrap());
                        });
                }
            },
        );
    }

    fn tool_card(&mut self, ui: &mut egui::Ui, c: &ToolCall) {
        let p = Palette::of(ui);
        let args: serde_json::Value = serde_json::from_str(&c.arguments).unwrap_or_default();
        let label = describe_call(&c.name, &args);
        let icon = tool_icon(&c.name);
        ui.add_space(4.0);

        if c.status == ToolStatus::AwaitingApproval {
            let mut decision = None;
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.5, p.text.gamma_multiply(0.5)))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        widgets::icon(ui, Icon::Shield, 20.0, p.text);
                        ui.label(egui::RichText::new("Approval needed").font(theme::font_bold(14.0)).color(p.text));
                    });
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(egui::RichText::new(&label).font(theme::font(14.0)).color(p.text)).wrap());
                    let details = pretty_args(&args);
                    if !details.is_empty() {
                        ui.add_space(4.0);
                        code_box(ui, &details, 8);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if widgets::primary_button(ui, "Approve").clicked() {
                            decision = Some(Decision::Approve);
                        }
                        if widgets::secondary_button(ui, "Always allow")
                            .on_hover_text("Never ask again for this tool and this bot")
                            .clicked()
                        {
                            decision = Some(Decision::AlwaysAllow);
                        }
                        if widgets::secondary_button(ui, "Deny").clicked() {
                            decision = Some(Decision::Deny);
                        }
                    });
                });
            if let Some(d) = decision {
                self.fleet.decide(&c.id, d);
            }
            return;
        }

        let expanded = self.agent_ui.expanded_calls.contains(&c.id);
        let font = theme::font(13.5);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(620.0), 30.0), Sense::click());
        let bg = if resp.hovered() { p.surface_hover } else { p.surface };
        ui.painter().rect(rect, CornerRadius::same(9), bg, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        icons::paint(
            ui.painter(),
            egui::Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(16.0)),
            icon,
            p.muted,
        );
        let text = widgets::truncate_to_width(ui, &label, &font, rect.width() - 90.0);
        ui.painter().text(pos2(rect.left() + 32.0, rect.center().y), Align2::LEFT_CENTER, text, font, p.text);
        let sc = pos2(rect.right() - 36.0, rect.center().y);
        match c.status {
            ToolStatus::Running => {
                let t = ui.input(|i| i.time) as f32;
                let a0 = t * 6.0;
                let pts: Vec<egui::Pos2> = (0..14)
                    .map(|i| {
                        let a = a0 + i as f32 * 0.32;
                        pos2(sc.x + 6.0 * a.cos(), sc.y + 6.0 * a.sin())
                    })
                    .collect();
                ui.painter().add(egui::Shape::line(pts, Stroke::new(1.8, p.muted)));
            }
            ToolStatus::Done => icons::paint(ui.painter(), egui::Rect::from_center_size(sc, Vec2::splat(15.0)), Icon::Check, p.muted),
            ToolStatus::Error => icons::paint(ui.painter(), egui::Rect::from_center_size(sc, Vec2::splat(14.0)), Icon::Close, p.danger),
            ToolStatus::Denied => {
                ui.painter().text(sc, Align2::CENTER_CENTER, "Denied", theme::font(11.5), p.danger);
            }
            ToolStatus::AwaitingApproval => {}
        }
        let chev = if expanded { Icon::ChevronDown } else { Icon::ChevronRight };
        icons::paint(
            ui.painter(),
            egui::Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), Vec2::splat(13.0)),
            chev,
            p.faint,
        );
        if resp.clicked() {
            if expanded {
                self.agent_ui.expanded_calls.remove(&c.id);
            } else {
                self.agent_ui.expanded_calls.insert(c.id.clone());
            }
        }
        if let Some(link) = &c.link
            && ui
                .add(
                    egui::Label::new(egui::RichText::new("Open conversation →").font(theme::font(12.5)).color(p.muted))
                        .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
        {
            let l = link.clone();
            self.open_thread(&l);
        }
        if expanded {
            ui.add_space(4.0);
            let details = pretty_args(&args);
            if !details.is_empty() {
                ui.label(egui::RichText::new("Input").font(theme::font(12.0)).color(p.faint));
                code_box(ui, &details, 12);
            }
            if let Some(r) = &c.result {
                ui.label(egui::RichText::new("Result").font(theme::font(12.0)).color(p.faint));
                code_box(ui, r, 24);
            }
        }
    }

    fn thread_composer(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        let p = Palette::of(ui);
        let tid = snap.thread.id.clone();
        let mut text = self.agent_ui.drafts.remove(&tid).unwrap_or_default();
        let hint = match &snap.thread.kind {
            ThreadKind::Dm { agent } => format!("Message {}", snap.agent(agent).map(|a| a.name.as_str()).unwrap_or("bot")),
            ThreadKind::Channel { name, .. } => format!("Message #{name} · @mention a bot"),
            _ => String::new(),
        };
        let members: Vec<Agent> = match &snap.thread.kind {
            ThreadKind::Channel { members, .. } => members.iter().filter_map(|m| snap.agent(m).cloned()).collect(),
            _ => Vec::new(),
        };
        // @mention suggestions for the word being typed.
        let partial = text.rsplit(|c: char| c.is_whitespace()).next().unwrap_or("").to_string();
        let suggestions: Vec<Agent> = if partial.starts_with('@') && !members.is_empty() {
            let q = partial[1..].to_lowercase();
            members.iter().filter(|a| a.name.to_lowercase().starts_with(&q) && a.name.to_lowercase() != q).cloned().collect()
        } else {
            Vec::new()
        };
        let mut pick: Option<String> = None;
        if !suggestions.is_empty() {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    for a in &suggestions {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(7), p.hover);
                        }
                        paint_avatar(
                            ui.painter(),
                            egui::Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(20.0)),
                            a,
                        );
                        ui.painter().text(
                            pos2(rect.left() + 32.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            a.label(),
                            theme::font(13.5),
                            p.text,
                        );
                        if resp.clicked() {
                            pick = Some(a.name.clone());
                        }
                    }
                });
            ui.add_space(4.0);
        }

        let busy = !snap.working.is_empty();
        let mut send = false;
        egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(22))
            .inner_margin(egui::Margin { left: 16, right: 8, top: 8, bottom: 8 })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 44.0;
                    let id = egui::Id::unique(("thread-input", &tid));
                    // Tab completes the first suggestion.
                    if !suggestions.is_empty() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)) {
                        pick = Some(suggestions[0].name.clone());
                    }
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                    let has_focus = ui.memory(|m| m.has_focus(id));
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .id(id)
                            .hint_text(hint)
                            .font(theme::font(15.0))
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::symmetric(0, 6))
                            .desired_rows(1)
                            .desired_width(w),
                    );
                    if std::mem::take(&mut self.agent_ui.focus_composer) {
                        r.request_focus();
                    }
                    if has_focus && enter && self.settings.enter_to_send {
                        // The TextEdit already inserted the newline; drop it and send.
                        if text.ends_with('\n') {
                            text.pop();
                        }
                        send = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::BOTTOM), |ui| {
                        let can = !text.trim().is_empty();
                        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
                        let fill = if can { p.accent } else { p.border };
                        ui.painter().circle_filled(rect.center(), 17.0, fill);
                        icons::paint(ui.painter(), rect.shrink(8.0), Icon::ArrowUp, if can { p.on_accent } else { p.muted });
                        if resp.on_hover_text(if busy { "Send (bots are working; they will see it next)" } else { "Send" }).clicked() && can
                        {
                            send = true;
                        }
                    });
                });
            });
        if let Some(name) = pick {
            let cut = text.len() - partial.len();
            text.truncate(cut);
            text.push('@');
            text.push_str(&name);
            text.push(' ');
            self.agent_ui.focus_composer = true;
        }
        if send && !text.trim().is_empty() {
            let responders = self.fleet.user_post(&tid, &text);
            if responders.is_empty() && matches!(snap.thread.kind, ThreadKind::Channel { .. }) {
                self.toast("Nobody in this channel to answer. Add bots in channel settings.");
            }
            text.clear();
            self.agent_ui.scroll_bottom = true;
            self.agent_ui.focus_composer = true;
        }
        self.agent_ui.drafts.insert(tid, text);
    }
}

fn tool_icon(name: &str) -> Icon {
    match name {
        n if n.starts_with("browser_") => Icon::Globe,
        "web_search" | "web_fetch" => Icon::Globe,
        "run_command" => Icon::Terminal,
        "list_files" | "read_file" | "write_file" => Icon::Folder,
        "remember" | "forget" | "search_memory" | "search_knowledge" => Icon::Brain,
        "use_skill" | "save_skill" => Icon::Book,
        "create_routine" => Icon::Clock,
        "ask_user" => Icon::Bell,
        "send_message" | "delegate_task" | "post_to_channel" | "list_agents" | "update_task" | "create_agent" => Icon::Users,
        n if n.starts_with("mcp__") => Icon::Plug,
        _ => Icon::Bot,
    }
}

fn pretty_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(m) if m.is_empty() => String::new(),
        serde_json::Value::Object(m) => m
            .iter()
            .map(|(k, v)| match v {
                serde_json::Value::String(s) => format!("{k}: {s}"),
                other => format!("{k}: {other}"),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn code_box(ui: &mut egui::Ui, text: &str, max_lines: usize) {
    let p = Palette::of(ui);
    let shown: String = {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > max_lines {
            format!("{}\n… {} more lines", lines[..max_lines].join("\n"), lines.len() - max_lines)
        } else {
            text.to_string()
        }
    };
    let shown: String = shown.chars().take(4000).collect();
    egui::Frame::new().fill(p.code_bg).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width().min(620.0));
        ui.add(egui::Label::new(egui::RichText::new(shown).monospace().size(12.5).color(p.text)).wrap().selectable(true));
    });
}

fn small_toggle(ui: &mut egui::Ui, icon: Icon, label: &str, open: bool) -> egui::Response {
    let p = Palette::of(ui);
    let font = theme::font(12.5);
    let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), p.text).size().x + 44.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(12), p.hover);
    }
    icons::paint(ui.painter(), egui::Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), Vec2::splat(14.0)), icon, p.muted);
    ui.painter().text(pos2(rect.left() + 24.0, rect.center().y), Align2::LEFT_CENTER, label, font, p.muted);
    let chev = if open { Icon::ChevronDown } else { Icon::ChevronRight };
    icons::paint(ui.painter(), egui::Rect::from_center_size(pos2(rect.right() - 10.0, rect.center().y), Vec2::splat(11.0)), chev, p.faint);
    resp
}

fn typing(ui: &mut egui::Ui) {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    for i in 0..3 {
        let ph = (t * 5.0 - i as f32 * 0.7).sin() * 0.5 + 0.5;
        ui.painter().circle_filled(
            pos2(rect.left() + 6.0 + i as f32 * 10.0, rect.center().y - ph * 2.5),
            3.2,
            p.text.gamma_multiply(0.35 + 0.65 * ph),
        );
    }
}

fn working_line(ui: &mut egui::Ui, a: Option<&Agent>, status: &str, secs: u64) {
    let p = Palette::of(ui);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if let Some(a) = a {
            avatar(ui, a, 22.0);
        }
        let t = ui.input(|i| i.time) as f32;
        let c = p.muted.gamma_multiply(0.6 + 0.4 * (t * 3.0).sin().abs());
        let name = a.map(|a| a.name.as_str()).unwrap_or("A bot");
        ui.label(egui::RichText::new(format!("{name} · {status} · {secs}s")).font(theme::font(13.0)).color(c));
    });
}
