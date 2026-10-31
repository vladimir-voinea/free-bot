//! Full pages: Activity (approvals, running bots, tasks, recent actions),
//! Routines, Knowledge (skills, team memory, knowledge folder), the first-run
//! home card, and the Bots and Connectors settings tabs.

use egui::{Align2, CornerRadius, Sense, Stroke, Vec2, pos2, vec2};

use super::super::icons::{self, Icon};
use super::super::theme::{self, Palette};
use super::super::widgets;
use super::{Page, avatar, short_time};
use crate::agents::engine::Decision;
use crate::agents::mcp::McpStatus;
use crate::agents::model::*;
use crate::agents::tools::describe_call;
use crate::app::FreeBotApp;

const PAGE_WIDTH: f32 = 860.0;

fn page_header(ui: &mut egui::Ui, icon: Icon, title: &str, subtitle: &str) {
    let p = Palette::of(ui);
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        widgets::icon(ui, icon, 28.0, p.text);
        ui.label(egui::RichText::new(title).font(theme::font_bold(24.0)).color(p.text));
    });
    ui.label(egui::RichText::new(subtitle).font(theme::font(14.0)).color(p.muted));
    ui.add_space(14.0);
}

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let p = Palette::of(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn section(ui: &mut egui::Ui, title: &str) {
    let p = Palette::of(ui);
    ui.add_space(14.0);
    ui.label(egui::RichText::new(title).font(theme::font_bold(15.0)).color(p.text));
    ui.add_space(6.0);
}

fn empty(ui: &mut egui::Ui, text: &str) {
    let p = Palette::of(ui);
    ui.label(egui::RichText::new(text).font(theme::font(13.5)).color(p.faint));
}

struct PendingApproval {
    call_id: String,
    agent: Option<Agent>,
    thread: String,
    label: String,
}

impl FreeBotApp {
    pub fn agent_page_ui(&mut self, ui: &mut egui::Ui, page: &Page) {
        let p = Palette::of(ui);
        egui::Frame::new().fill(p.bg).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt(("page", format!("{page:?}"))).auto_shrink([false, false]).show(ui, |ui| {
                super::super::chat::centered_column(ui, PAGE_WIDTH, |ui| match page {
                    Page::Activity => self.activity_page(ui),
                    Page::Routines => self.routines_page(ui),
                    Page::Knowledge => self.knowledge_page(ui),
                    Page::Thread(_) => {}
                });
                ui.add_space(30.0);
            });
        });
    }

    fn thread_label(st: &crate::agents::engine::FleetState, id: &str) -> String {
        match st.thread(id).map(|t| &t.kind) {
            Some(ThreadKind::Dm { agent }) => format!("Chat with {}", st.agent(agent).map(|a| a.name.as_str()).unwrap_or("?")),
            Some(ThreadKind::Channel { name, .. }) => format!("#{name}"),
            Some(ThreadKind::AgentDm { a, b }) => {
                let n = |x: &str| st.agent(x).map(|a| a.name.clone()).unwrap_or_default();
                format!("{} & {}", n(a), n(b))
            }
            None => "Deleted conversation".into(),
        }
    }

    fn activity_page(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        page_header(ui, Icon::Pulse, "Activity", "What your bots are doing, and what needs you.");
        let (approvals, needs, running, tasks, recent) = {
            let st = self.fleet.lock();
            let mut approvals = Vec::new();
            let mut recent = Vec::new();
            for t in &st.threads {
                for m in &t.messages {
                    let agent = match &m.author {
                        Author::Agent(id) => st.agent(id).cloned(),
                        _ => None,
                    };
                    for c in &m.tool_calls {
                        let args: serde_json::Value = serde_json::from_str(&c.arguments).unwrap_or_default();
                        let label = describe_call(&c.name, &args);
                        if c.status == ToolStatus::AwaitingApproval {
                            approvals.push(PendingApproval {
                                call_id: c.id.clone(),
                                agent: agent.clone(),
                                thread: t.id.clone(),
                                label: label.clone(),
                            });
                        }
                        recent.push((m.created, agent.clone(), label, c.status, t.id.clone()));
                    }
                }
            }
            recent.sort_by_key(|r| std::cmp::Reverse(r.0));
            recent.truncate(30);
            let needs: Vec<(String, String)> =
                st.threads.iter().filter(|t| t.needs_you).map(|t| (t.id.clone(), Self::thread_label(&st, &t.id))).collect();
            let running: Vec<(Option<Agent>, String, String, String, u64)> = st
                .runs
                .iter()
                .map(|((a, t), r)| {
                    (st.agent(a).cloned(), t.clone(), Self::thread_label(&st, t), r.status.clone(), r.started.elapsed().as_secs())
                })
                .collect();
            let mut tasks: Vec<(Task, String, String)> = st
                .tasks
                .iter()
                .map(|t| {
                    let from = match &t.from {
                        Author::Agent(id) => st.agent(id).map(|a| a.name.clone()).unwrap_or_default(),
                        _ => "You".into(),
                    };
                    let to = st.agent(&t.to).map(|a| a.name.clone()).unwrap_or_default();
                    (t.clone(), from, to)
                })
                .collect();
            tasks.sort_by_key(|(t, _, _)| (t.status == TaskStatus::Done, std::cmp::Reverse(t.updated)));
            tasks.truncate(40);
            (approvals, needs, running, tasks, recent)
        };

        section(ui, "Needs you");
        if approvals.is_empty() && needs.is_empty() {
            empty(ui, "Nothing right now.");
        }
        for a in approvals {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(ag) = &a.agent {
                        avatar(ui, ag, 26.0);
                    }
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width() - 290.0);
                        ui.label(egui::RichText::new(&a.label).font(theme::font_bold(14.0)).color(p.text));
                        ui.label(
                            egui::RichText::new(format!(
                                "{} wants to do this",
                                a.agent.as_ref().map(|x| x.name.as_str()).unwrap_or("A bot")
                            ))
                            .font(theme::font(12.5))
                            .color(p.muted),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::secondary_button(ui, "Open").clicked() {
                            self.open_thread(&a.thread);
                        }
                        if widgets::secondary_button(ui, "Deny").clicked() {
                            self.fleet.decide(&a.call_id, Decision::Deny);
                        }
                        if widgets::primary_button(ui, "Approve").clicked() {
                            self.fleet.decide(&a.call_id, Decision::Approve);
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
        for (tid, label) in needs {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::icon(ui, Icon::Bell, 20.0, p.text);
                    ui.label(egui::RichText::new(format!("A question is waiting in {label}")).font(theme::font(14.0)).color(p.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::secondary_button(ui, "Answer").clicked() {
                            self.open_thread(&tid);
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }

        section(ui, "Working now");
        if running.is_empty() {
            empty(ui, "All bots are idle.");
        }
        for (agent, tid, label, status, secs) in running {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(a) = &agent {
                        avatar(ui, a, 26.0);
                    }
                    ui.label(
                        egui::RichText::new(format!("{} · {status}", agent.as_ref().map(|a| a.name.as_str()).unwrap_or("?")))
                            .font(theme::font_bold(14.0))
                            .color(p.text),
                    );
                    ui.label(egui::RichText::new(format!("in {label} · {secs}s")).font(theme::font(13.0)).color(p.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::secondary_button(ui, "Stop").clicked() {
                            self.fleet.stop_thread(&tid);
                        }
                        if widgets::secondary_button(ui, "Open").clicked() {
                            self.open_thread(&tid);
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }

        section(ui, "Tasks between bots");
        if tasks.is_empty() {
            empty(ui, "When bots hand work to each other, the tasks show up here.");
        }
        for (t, from, to) in tasks {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(8), p.hover);
            }
            let (label, color) = match t.status {
                TaskStatus::Open => ("Open", p.text),
                TaskStatus::Blocked => ("Blocked", p.danger),
                TaskStatus::Done => ("Done", p.faint),
            };
            ui.painter().text(pos2(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, theme::font_bold(12.5), color);
            let title = widgets::truncate_to_width(ui, &t.title, &theme::font(13.5), rect.width() - 330.0);
            ui.painter().text(pos2(rect.left() + 76.0, rect.center().y), Align2::LEFT_CENTER, title, theme::font(13.5), p.text);
            ui.painter().text(
                pos2(rect.right() - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                format!("{from} → {to} · {}", short_time(t.updated)),
                theme::font(12.5),
                p.muted,
            );
            if resp.on_hover_text(if t.note.is_empty() { "Open the conversation" } else { &t.note }).clicked() {
                self.open_thread(&t.thread);
            }
        }

        section(ui, "Recent actions");
        if recent.is_empty() {
            empty(ui, "No actions yet.");
        }
        for (when, agent, label, status, tid) in recent {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(8), p.hover);
            }
            if let Some(a) = &agent {
                super::paint_avatar(
                    ui.painter(),
                    egui::Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), Vec2::splat(18.0)),
                    a,
                );
            }
            let mark = match status {
                ToolStatus::Done => "",
                ToolStatus::Error => " · failed",
                ToolStatus::Denied => " · denied",
                ToolStatus::Running => " · running",
                ToolStatus::AwaitingApproval => " · waiting for you",
            };
            let text = widgets::truncate_to_width(ui, &format!("{label}{mark}"), &theme::font(13.0), rect.width() - 120.0);
            ui.painter().text(pos2(rect.left() + 32.0, rect.center().y), Align2::LEFT_CENTER, text, theme::font(13.0), p.text);
            ui.painter().text(
                pos2(rect.right() - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                short_time(when),
                theme::font(12.0),
                p.faint,
            );
            if resp.clicked() {
                self.open_thread(&tid);
            }
        }
    }

    fn routines_page(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        page_header(ui, Icon::Clock, "Routines", "Recurring work your bots do on their own, on a schedule or when something happens.");
        let (routines, agents) = {
            let st = self.fleet.lock();
            (st.routines.clone(), st.agents.clone())
        };
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, "New routine").clicked() {
                if agents.is_empty() {
                    self.toast("Create a bot first.");
                } else {
                    self.routine_editor_for(None);
                }
            }
            ui.label(
                egui::RichText::new("Bots can also create routines when you ask them in chat.").font(theme::font(13.0)).color(p.muted),
            );
        });
        ui.add_space(12.0);
        if routines.is_empty() {
            empty(ui, "No routines yet. Try: \"Every weekday at 9:00, summarize what changed in my projects.\"");
        }
        let mut action: Option<(usize, String)> = None;
        for r in &routines {
            let agent = agents.iter().find(|a| a.id == r.agent);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(a) = agent {
                        avatar(ui, a, 30.0);
                    }
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width() - 260.0);
                        ui.label(egui::RichText::new(&r.name).font(theme::font_bold(15.0)).color(p.text));
                        let next = r.next_run().map(|t| format!(" · next {}", t.format("%a %b %-d %H:%M"))).unwrap_or_default();
                        let last = r.last_run.map(|t| format!(" · last {}", short_time(t))).unwrap_or_default();
                        ui.label(
                            egui::RichText::new(format!(
                                "{} · {}{next}{last}",
                                agent.map(|a| a.name.as_str()).unwrap_or("?"),
                                r.trigger.describe()
                            ))
                            .font(theme::font(12.5))
                            .color(p.muted),
                        );
                        let first = r.instructions.lines().next().unwrap_or("");
                        ui.label(egui::RichText::new(first).font(theme::font(13.0)).color(p.text));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button_sized(ui, Icon::Trash, "Delete", 30.0, None).clicked() {
                            action = Some((3, r.id.clone()));
                        }
                        if widgets::icon_button_sized(ui, Icon::Pencil, "Edit", 30.0, None).clicked() {
                            action = Some((2, r.id.clone()));
                        }
                        if widgets::icon_button_sized(ui, Icon::Play, "Run now", 30.0, None).clicked() {
                            action = Some((1, r.id.clone()));
                        }
                        let mut on = r.enabled;
                        if widgets::switch(ui, &mut on).on_hover_text("Enabled").changed() {
                            action = Some((if on { 4 } else { 5 }, r.id.clone()));
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
        if let Some((kind, id)) = action {
            match kind {
                1 => {
                    self.fleet.run_routine(&id, None);
                    self.toast("Routine started.");
                }
                2 => {
                    let r = routines.iter().find(|r| r.id == id).cloned();
                    self.routine_editor_for(r);
                }
                3 => {
                    let mut st = self.fleet.lock();
                    st.routines.retain(|r| r.id != id);
                    st.dirty.routines = true;
                }
                _ => {
                    let mut st = self.fleet.lock();
                    if let Some(r) = st.routines.iter_mut().find(|r| r.id == id) {
                        r.enabled = kind == 4;
                        if r.enabled {
                            // Do not fire for every run missed while it was off.
                            r.last_run = Some(chrono::Utc::now());
                        }
                    }
                    st.dirty.routines = true;
                }
            }
        }
    }

    fn knowledge_page(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        page_header(ui, Icon::Book, "Knowledge", "Skills every bot can use, what the team remembers, and the shared knowledge folder.");
        let (skills, mems, agents, kdir) = {
            let st = self.fleet.lock();
            (st.skills.clone(), st.memories.clone(), st.agents.clone(), st.config.knowledge_dir.clone())
        };

        section(ui, "Skills");
        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, "New skill").clicked() {
                self.agent_ui.skill_editor = Some(Skill {
                    id: crate::store::new_id(),
                    name: String::new(),
                    description: String::new(),
                    instructions: String::new(),
                    author: None,
                    updated: chrono::Utc::now(),
                });
            }
            ui.label(
                egui::RichText::new("Bots save skills when they learn a procedure, or when you teach a task in their browser.")
                    .font(theme::font(12.5))
                    .color(p.muted),
            );
        });
        ui.add_space(6.0);
        if skills.is_empty() {
            empty(ui, "No skills yet.");
        }
        let mut delete_skill = None;
        for s in &skills {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width() - 80.0);
                        ui.label(egui::RichText::new(&s.name).font(theme::font_bold(14.5)).color(p.text));
                        let by = s
                            .author
                            .as_ref()
                            .and_then(|a| agents.iter().find(|x| &x.id == a))
                            .map(|a| format!(" · by {}", a.name))
                            .unwrap_or_default();
                        ui.label(egui::RichText::new(format!("{}{by}", s.description)).font(theme::font(12.5)).color(p.muted));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button_sized(ui, Icon::Trash, "Delete", 28.0, None).clicked() {
                            delete_skill = Some(s.id.clone());
                        }
                        if widgets::icon_button_sized(ui, Icon::Pencil, "Edit", 28.0, None).clicked() {
                            self.agent_ui.skill_editor = Some(s.clone());
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
        if let Some(id) = delete_skill {
            let mut st = self.fleet.lock();
            st.skills.retain(|s| s.id != id);
            st.dirty.skills = true;
        }

        section(ui, "Team memory");
        let shared: Vec<&Memory> = mems.iter().filter(|m| m.agent.is_none()).collect();
        if shared.is_empty() {
            empty(ui, "Facts shared with every bot appear here. Bot-specific memories are in each bot's Memory panel.");
        }
        let mut forget = None;
        for m in shared {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("• {}", m.text)).font(theme::font(13.5)).color(p.text));
                if widgets::icon_button_sized(ui, Icon::Close, "Forget", 22.0, None).clicked() {
                    forget = Some(m.id.clone());
                }
            });
        }
        if let Some(id) = forget {
            let mut st = self.fleet.lock();
            st.memories.retain(|m| m.id != id);
            st.dirty.memories = true;
        }

        section(ui, "Knowledge folder");
        ui.add(egui::Label::new(egui::RichText::new("A folder of notes and documents (Markdown, text, CSV, JSON, HTML). Every bot can search it and read files from it. It is never modified.").font(theme::font(13.0)).color(p.muted)).wrap());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let text = kdir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "Not set".into());
            ui.label(egui::RichText::new(text).monospace().color(p.text));
            if widgets::secondary_button(ui, "Choose folder").clicked()
                && let Some(d) = rfd::FileDialog::new().pick_folder()
            {
                let mut cfg = self.fleet.lock().config.clone();
                cfg.knowledge_dir = Some(d);
                self.fleet.save_config(cfg);
            }
            if kdir.is_some() && widgets::secondary_button(ui, "Remove").clicked() {
                let mut cfg = self.fleet.lock().config.clone();
                cfg.knowledge_dir = None;
                self.fleet.save_config(cfg);
            }
        });
    }

    /// The card on the home screen that introduces bots.
    pub fn bots_home_card(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let n = self.fleet.lock().agents.len();
        if n > 0 {
            return;
        }
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 18.0, p.selected);
                icons::paint(ui.painter(), r.shrink(8.0), Icon::Bot, p.text);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 150.0).max(120.0));
                    ui.label(egui::RichText::new("Build your team of bots").font(theme::font_bold(15.0)).color(p.text));
                    ui.add(egui::Label::new(egui::RichText::new("Bots work on their own with a browser, files and a terminal, hand tasks to each other, and come back when they need you.").font(theme::font(13.0)).color(p.muted)).wrap());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, "Create a bot").clicked() {
                        self.new_agent_editor();
                    }
                });
            });
        });
        ui.add_space(14.0);
    }

    pub fn bots_settings_tab(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let mut cfg = self.fleet.lock().config.clone();
        let before = cfg.clone();
        widgets::setting_row(ui, "Notifications", "Desktop notifications when a bot needs you or replies", |ui| {
            widgets::switch(ui, &mut cfg.notifications);
        });
        widgets::setting_row(ui, "Steps per turn", "Most model calls a bot makes before it stops and waits", |ui| {
            ui.add(egui::DragValue::new(&mut cfg.max_steps).range(1..=200));
        });
        widgets::setting_row(ui, "Handoffs per chain", "Most bot-to-bot handoffs in a row, to stop endless back-and-forth", |ui| {
            ui.add(egui::DragValue::new(&mut cfg.max_hops).range(1..=50));
        });
        widgets::setting_row(ui, "Follow up after", "Minutes before a stalled task gets a nudge from proactive bots", |ui| {
            ui.add(egui::DragValue::new(&mut cfg.stall_minutes).range(1..=10_080).suffix(" min"));
        });
        widgets::setting_row(ui, "Webhooks", &format!("Let routines run on POST to http://127.0.0.1:{}/hook/…", cfg.webhook_port), |ui| {
            widgets::switch(ui, &mut cfg.webhooks_enabled);
            ui.add(egui::DragValue::new(&mut cfg.webhook_port).range(1024..=65535));
        });
        ui.label(egui::RichText::new("Browser").font(theme::font_bold(14.0)).color(p.text));
        let detected =
            crate::agents::browser::find_browser(&cfg.browser_path).map(|p| p.display().to_string()).unwrap_or_else(|| "Not found".into());
        ui.label(
            egui::RichText::new(format!("Chrome, Chromium or Edge, used headless. In use: {detected}"))
                .font(theme::font(12.5))
                .color(p.muted),
        );
        widgets::text_field(ui, &mut cfg.browser_path, "Auto-detect (or a path to the browser)", false);
        ui.add_space(10.0);
        ui.label(egui::RichText::new("Background").font(theme::font_bold(14.0)).color(p.text));
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Bots, routines and webhooks run while this window is open. To keep them working with the window closed, run \"free-bot --background\" (for example at login). Opening the app takes over from it.",
                )
                .font(theme::font(12.5))
                .color(p.muted),
            )
            .wrap(),
        );
        ui.add_space(10.0);
        ui.label(egui::RichText::new("Data").font(theme::font_bold(14.0)).color(p.text));
        let root = self.fleet.store().root().display().to_string();
        ui.label(
            egui::RichText::new(format!("Bots, chats, memory and workspaces are stored in {root}")).font(theme::font(12.5)).color(p.muted),
        );
        if cfg != before {
            self.fleet.browser().set_path(&cfg.browser_path);
            self.fleet.save_config(cfg);
        }
    }

    pub fn connectors_settings_tab(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        ui.add(egui::Label::new(egui::RichText::new("Connect tools and services through MCP servers, such as GitHub, Slack, Notion, Google Drive, databases or your file system. Each connection is shared by every bot. Tools marked read-only run without asking; others ask first unless you trust the connector.").font(theme::font(13.0)).color(p.muted)).wrap());
        ui.add_space(10.0);
        if widgets::primary_button(ui, "Add connector").clicked() {
            self.connector_editor_for(None);
        }
        ui.add_space(10.0);
        let connectors = self.fleet.lock().config.connectors.clone();
        if connectors.is_empty() {
            empty(ui, "No connectors yet.");
        }
        let mut action: Option<(u8, String)> = None;
        for c in &connectors {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::icon(ui, Icon::Plug, 22.0, p.text);
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width() - 160.0);
                        ui.label(egui::RichText::new(&c.name).font(theme::font_bold(14.5)).color(p.text));
                        let status = if !c.enabled {
                            ("Off".to_string(), p.faint)
                        } else {
                            match self.fleet.mcp().status(&c.id) {
                                Some(McpStatus::Ready(tools)) => (format!("Connected · {} tools", tools.len()), p.muted),
                                Some(McpStatus::Failed(e)) => (format!("Failed: {e}"), p.danger),
                                Some(McpStatus::Starting) | None => ("Connecting…".to_string(), p.muted),
                            }
                        };
                        ui.add(egui::Label::new(egui::RichText::new(status.0).font(theme::font(12.5)).color(status.1)).wrap());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button_sized(ui, Icon::Trash, "Remove", 28.0, None).clicked() {
                            action = Some((0, c.id.clone()));
                        }
                        if widgets::icon_button_sized(ui, Icon::Pencil, "Edit", 28.0, None).clicked() {
                            action = Some((1, c.id.clone()));
                        }
                        if widgets::icon_button_sized(ui, Icon::Refresh, "Reconnect", 28.0, None).clicked() {
                            action = Some((2, c.id.clone()));
                        }
                        let mut on = c.enabled;
                        if widgets::switch(ui, &mut on).changed() {
                            action = Some((if on { 3 } else { 4 }, c.id.clone()));
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
        if let Some((kind, id)) = action {
            let mut cfg = self.fleet.lock().config.clone();
            match kind {
                0 => cfg.connectors.retain(|c| c.id != id),
                1 => {
                    let c = cfg.connectors.iter().find(|c| c.id == id).cloned();
                    self.connector_editor_for(c);
                    return;
                }
                2 => {
                    if let Some(c) = cfg.connectors.iter().find(|c| c.id == id) {
                        self.fleet.mcp().restart(c);
                    }
                    return;
                }
                _ => {
                    if let Some(c) = cfg.connectors.iter_mut().find(|c| c.id == id) {
                        c.enabled = kind == 3;
                    }
                }
            }
            self.fleet.save_config(cfg);
        }
    }
}
