//! Agents, channels and agent-to-agent chats in the sidebar.

use egui::{Align2, CornerRadius, Sense, Vec2, pos2, vec2};

use super::super::icons::{self, Icon};
use super::super::sidebar::menu_item;
use super::super::theme::{self, Palette};
use super::super::widgets;
use super::{AgentEditor, ChannelEditor, Page, PanelTab, paint_avatar};
use crate::agents::model::{AGENT_COLORS, Agent, Shape, ThreadKind};
use crate::app::FreeBotApp;

struct Row {
    thread: String,
    label: String,
    agent: Option<Agent>,
    icon: Option<Icon>,
    busy: bool,
    unread: bool,
    needs_you: bool,
    approvals: usize,
}

impl FreeBotApp {
    pub fn open_thread(&mut self, id: &str) {
        self.page = Some(Page::Thread(id.to_string()));
        self.agent_ui.scroll_bottom = true;
        self.agent_ui.focus_composer = true;
        let mut st = self.fleet.lock();
        st.viewing = Some(id.to_string());
        if let Some(t) = st.thread_mut(id)
            && t.unread
        {
            t.unread = false;
            st.mark_thread(id);
        }
    }

    pub fn open_page(&mut self, page: Page) {
        if let Page::Thread(id) = &page {
            let id = id.clone();
            self.open_thread(&id);
            return;
        }
        self.page = Some(page);
        self.fleet.lock().viewing = None;
    }

    pub fn new_agent_editor(&mut self) {
        let n = self.fleet.lock().agents.len();
        let agent = Agent {
            name: String::new(),
            color: AGENT_COLORS[n % AGENT_COLORS.len()],
            shape: Shape::ALL[n % Shape::ALL.len()],
            ..Default::default()
        };
        self.agent_ui.editor = Some(AgentEditor { agent, is_new: true, workspace_text: String::new() });
    }

    pub fn edit_agent(&mut self, id: &str) {
        if let Some(a) = self.fleet.lock().agent(id).cloned() {
            let ws = a.workspace.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            self.agent_ui.editor = Some(AgentEditor { agent: a, is_new: false, workspace_text: ws });
        }
    }

    pub fn new_channel_editor(&mut self) {
        let members: Vec<String> = self.fleet.lock().agents.iter().map(|a| a.id.clone()).collect();
        let lead = members.first().cloned();
        self.agent_ui.channel_editor = Some(ChannelEditor { id: None, name: String::new(), members, lead });
    }

    pub fn agents_sidebar(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let (agent_rows, channel_rows, between_rows) = {
            let st = self.fleet.lock();
            let approvals_in = |tid: &str| {
                st.thread(tid)
                    .map(|t| {
                        t.messages
                            .iter()
                            .flat_map(|m| &m.tool_calls)
                            .filter(|c| c.status == crate::agents::model::ToolStatus::AwaitingApproval)
                            .count()
                    })
                    .unwrap_or(0)
            };
            let mut agents = Vec::new();
            for a in &st.agents {
                let t = st.threads.iter().find(|t| t.is_dm_with(&a.id));
                let thread = t.map(|t| t.id.clone()).unwrap_or_default();
                // An agent row lights up for approvals anywhere it works.
                let approvals: usize = st.threads.iter().filter(|t| t.has_agent(&a.id)).map(|t| approvals_in(&t.id)).sum();
                agents.push(Row {
                    label: a.name.clone(),
                    agent: Some(a.clone()),
                    icon: None,
                    busy: st.agent_busy(&a.id),
                    unread: t.is_some_and(|t| t.unread),
                    needs_you: t.is_some_and(|t| t.needs_you),
                    approvals,
                    thread,
                });
            }
            let mut channels = Vec::new();
            let mut between = Vec::new();
            for t in &st.threads {
                match &t.kind {
                    ThreadKind::Channel { name, .. } => channels.push(Row {
                        thread: t.id.clone(),
                        label: name.clone(),
                        agent: None,
                        icon: Some(Icon::Hash),
                        busy: st.thread_busy(&t.id),
                        unread: t.unread,
                        needs_you: t.needs_you,
                        approvals: approvals_in(&t.id),
                    }),
                    ThreadKind::AgentDm { a, b } => {
                        let n = |id: &str| st.agent(id).map(|x| x.name.clone()).unwrap_or_default();
                        between.push(Row {
                            thread: t.id.clone(),
                            label: format!("{} & {}", n(a), n(b)),
                            agent: None,
                            icon: Some(Icon::Swap),
                            busy: st.thread_busy(&t.id),
                            unread: t.unread,
                            needs_you: false,
                            approvals: approvals_in(&t.id),
                        })
                    }
                    ThreadKind::Dm { .. } => {}
                }
            }
            between.sort_by_key(|r| r.label.clone());
            (agents, channels, between)
        };

        // Agents.
        if section_header(ui, "Bots", Some("New bot")) {
            self.new_agent_editor();
        }
        if agent_rows.is_empty() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(10), p.hover);
            }
            icons::paint(
                ui.painter(),
                egui::Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(18.0)),
                Icon::Plus,
                p.muted,
            );
            ui.painter().text(
                pos2(rect.left() + 40.0, rect.center().y),
                Align2::LEFT_CENTER,
                "Create your first bot",
                theme::font(14.0),
                p.muted,
            );
            if resp.clicked() {
                self.new_agent_editor();
            }
        }
        for r in agent_rows {
            self.thread_row(ui, r);
        }

        // Channels.
        ui.add_space(8.0);
        if section_header(ui, "Channels", Some("New channel")) {
            self.new_channel_editor();
        }
        if channel_rows.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Group chats with several bots.").font(theme::font(12.5)).color(p.faint));
            });
        }
        for r in channel_rows {
            self.thread_row(ui, r);
        }

        // Agent-to-agent chats.
        if !between_rows.is_empty() {
            ui.add_space(8.0);
            let open = self.agent_ui.between_open;
            let attention = between_rows.iter().any(|r| r.unread || r.approvals > 0 || r.busy);
            let label = format!("Between bots ({})", between_rows.len());
            if collapsible_header(ui, &label, open, attention) {
                self.agent_ui.between_open = !open;
            }
            if open {
                for r in between_rows {
                    self.thread_row(ui, r);
                }
            }
        }
        ui.add_space(8.0);
    }

    fn thread_row(&mut self, ui: &mut egui::Ui, r: Row) {
        let p = Palette::of(ui);
        let selected = self.page.as_ref() == Some(&Page::Thread(r.thread.clone()));
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        let menu_id = ui.scope_id().with(("thread-menu", &r.thread));
        let menu_open = egui::Popup::is_id_open(ui.ctx(), menu_id);
        let hovered = resp.hovered() || menu_open;
        if selected {
            ui.painter().rect_filled(rect, CornerRadius::same(10), p.selected);
        } else if hovered {
            ui.painter().rect_filled(rect, CornerRadius::same(10), p.hover);
        }
        let icon_rect = egui::Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(22.0));
        match (&r.agent, r.icon) {
            (Some(a), _) => paint_avatar(ui.painter(), icon_rect, a),
            (None, Some(icon)) => icons::paint(ui.painter(), icon_rect.shrink(2.0), icon, p.muted),
            _ => {}
        }
        let bold = r.unread || r.needs_you;
        let font = if bold { theme::font_bold(14.0) } else { theme::font(14.0) };
        let right = if hovered { 36.0 } else { 28.0 };
        let text = widgets::truncate_to_width(ui, &r.label, &font, rect.width() - 40.0 - right);
        ui.painter().text(pos2(rect.left() + 38.0, rect.center().y), Align2::LEFT_CENTER, text, font, p.text);

        // Status on the right: approvals, needs you, working, unread.
        let badge_c = pos2(rect.right() - 14.0, rect.center().y);
        if !hovered {
            if r.approvals > 0 || r.needs_you {
                let n = if r.approvals > 0 { r.approvals.to_string() } else { "!".into() };
                ui.painter().circle_filled(badge_c, 8.0, p.danger);
                ui.painter().text(badge_c, Align2::CENTER_CENTER, n, theme::font_bold(10.5), egui::Color32::WHITE);
            } else if r.busy {
                let t = ui.input(|i| i.time) as f32;
                for i in 0..3 {
                    let ph = (t * 5.0 - i as f32 * 0.8).sin() * 0.5 + 0.5;
                    ui.painter().circle_filled(
                        pos2(badge_c.x - 8.0 + i as f32 * 6.0, badge_c.y),
                        2.2,
                        p.text.gamma_multiply(0.3 + 0.7 * ph),
                    );
                }
            } else if r.unread {
                ui.painter().circle_filled(badge_c, 4.0, p.text);
            }
        }
        if resp.clicked() {
            self.open_thread(&r.thread);
        }
        if hovered {
            let btn = egui::Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(26.0));
            let bresp = ui.interact(btn, menu_id.with("btn"), Sense::click());
            if bresp.hovered() {
                ui.painter().rect_filled(btn, CornerRadius::same(7), p.surface_hover);
            }
            icons::paint(ui.painter(), btn.shrink(5.0), Icon::Dots, p.text);
            let thread = r.thread.clone();
            let agent = r.agent.clone();
            let is_channel = r.icon == Some(Icon::Hash);
            egui::Popup::menu(&bresp).id(menu_id).width(190.0).show(|ui| {
                ui.set_min_width(180.0);
                if let Some(a) = &agent {
                    if menu_item(ui, Icon::Pencil, "Edit bot", false).clicked() {
                        self.edit_agent(&a.id);
                    }
                    if menu_item(ui, Icon::Terminal, "Open computer", false).clicked() {
                        self.open_thread(&thread);
                        self.agent_ui.panel = Some(PanelTab::Computer);
                    }
                    if menu_item(ui, Icon::Refresh, "Clear chat", false).clicked() {
                        self.fleet.clear_thread(&thread);
                    }
                    ui.separator();
                    if menu_item(ui, Icon::Trash, "Delete bot", true).clicked() {
                        self.agent_ui.confirm_delete_agent = Some(a.id.clone());
                    }
                } else {
                    if is_channel && menu_item(ui, Icon::Pencil, "Edit channel", false).clicked() {
                        self.edit_channel(&thread);
                    }
                    if menu_item(ui, Icon::Refresh, "Clear messages", false).clicked() {
                        self.fleet.clear_thread(&thread);
                    }
                    ui.separator();
                    if menu_item(ui, Icon::Trash, "Delete", true).clicked() {
                        self.agent_ui.confirm_delete_thread = Some(thread.clone());
                    }
                }
            });
        }
    }

    pub fn edit_channel(&mut self, id: &str) {
        let st = self.fleet.lock();
        if let Some(t) = st.thread(id)
            && let ThreadKind::Channel { name, members, lead } = &t.kind
        {
            let ed = ChannelEditor { id: Some(id.to_string()), name: name.clone(), members: members.clone(), lead: lead.clone() };
            drop(st);
            self.agent_ui.channel_editor = Some(ed);
        }
    }
}

/// A small section title with an optional "+" button. Returns true when "+" is clicked.
fn section_header(ui: &mut egui::Ui, label: &str, add_tooltip: Option<&str>) -> bool {
    let p = Palette::of(ui);
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.set_height(26.0);
        ui.add_space(10.0);
        ui.label(egui::RichText::new(label).font(theme::font(12.5)).color(p.faint));
        if let Some(tip) = add_tooltip {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                if widgets::icon_button_sized(ui, Icon::Plus, tip, 24.0, None).clicked() {
                    clicked = true;
                }
            });
        }
    });
    clicked
}

fn collapsible_header(ui: &mut egui::Ui, label: &str, open: bool, attention: bool) -> bool {
    let p = Palette::of(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let color = if resp.hovered() { p.text } else { p.faint };
    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, label, theme::font(12.5), color);
    let chev = if open { Icon::ChevronDown } else { Icon::ChevronRight };
    icons::paint(ui.painter(), egui::Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), Vec2::splat(14.0)), chev, color);
    if attention && !open {
        ui.painter().circle_filled(pos2(rect.right() - 30.0, rect.center().y), 3.5, p.text);
    }
    resp.clicked()
}
