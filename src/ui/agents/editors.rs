//! Dialogs: create or edit a bot, a channel, a routine, a connector or a
//! skill, and confirmations.

use egui::{CornerRadius, Sense, Stroke, Vec2, vec2};

use super::super::theme::{self, Palette};
use super::super::widgets;
use super::{ConnectorEditor, RoutineEditor, paint_avatar_raw};
use crate::agents::engine::Fleet;
use crate::agents::model::*;
use crate::app::FreeBotApp;
use crate::store::ModelRef;

fn frame(ctx: &egui::Context) -> egui::Frame {
    let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(20))
        .inner_margin(egui::Margin::same(22))
        .shadow(egui::Shadow { offset: [0, 12], blur: 40, spread: 0, color: egui::Color32::from_black_alpha(90) })
}

fn label(ui: &mut egui::Ui, text: &str) {
    let p = Palette::of(ui);
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text).font(theme::font_bold(13.0)).color(p.muted));
    ui.add_space(2.0);
}

fn multiline(ui: &mut egui::Ui, value: &mut String, hint: &str, rows: usize) {
    widgets::input_frame(ui).show(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(value)
                .hint_text(hint)
                .font(theme::font(14.0))
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::symmetric(0, 6))
                .desired_rows(rows)
                .desired_width(f32::INFINITY),
        );
    });
}

fn title(ui: &mut egui::Ui, text: &str) {
    let p = Palette::of(ui);
    ui.label(egui::RichText::new(text).font(theme::font_bold(19.0)).color(p.text));
    ui.add_space(4.0);
}

impl FreeBotApp {
    pub fn agent_modals(&mut self, ctx: &egui::Context) {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        if self.agent_ui.editor.is_some() {
            let mut close = false;
            let mut save = false;
            let models = self.settings.all_models();
            let connectors = self.fleet.lock().config.connectors.clone();
            let resp = egui::Modal::new(egui::Id::unique("agent-editor")).frame(frame(ctx)).backdrop_color(p.overlay).show(ctx, |ui| {
                ui.set_width(580.0f32.min(ctx.content_rect().width() - 60.0));
                let ed = self.agent_ui.editor.as_mut().unwrap();
                title(ui, if ed.is_new { "New bot" } else { "Edit bot" });
                egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 200.0).show(ui, |ui| {
                    let a = &mut ed.agent;
                    if ed.is_new {
                        ui.label(egui::RichText::new("Start from a template or describe your own.").font(theme::font(13.5)).color(p.muted));
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            for t in AGENT_TEMPLATES {
                                if widgets::pill_button(ui, &format!("{} · {}", t.name, t.title), None, false, true).clicked() {
                                    a.name = t.name.into();
                                    a.title = t.title.into();
                                    a.description = t.description.into();
                                    a.instructions = t.instructions.into();
                                    a.color = AGENT_COLORS[t.color];
                                    a.shape = t.shape;
                                }
                            }
                        });
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                        paint_avatar_raw(
                            ui.painter(),
                            r,
                            a.shape,
                            egui::Color32::from_rgb(a.color[0], a.color[1], a.color[2]),
                            if a.name.is_empty() { "?" } else { &a.name },
                        );
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                for c in AGENT_COLORS {
                                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                                    ui.painter().circle_filled(r.center(), 10.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                                    if a.color == c {
                                        ui.painter().circle_stroke(r.center(), 12.0, Stroke::new(2.0, p.text));
                                    }
                                    if resp.clicked() {
                                        a.color = c;
                                    }
                                }
                            });
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                for s in Shape::ALL {
                                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                                    if a.shape == s || resp.hovered() {
                                        ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                                    }
                                    paint_avatar_raw(ui.painter(), r.shrink(4.0), s, p.muted, " ");
                                    if resp.clicked() {
                                        a.shape = s;
                                    }
                                }
                            });
                        });
                    });
                    label(ui, "Name");
                    widgets::text_field(ui, &mut a.name, "For example Nova", false);
                    label(ui, "Title");
                    widgets::text_field(ui, &mut a.title, "For example Researcher", false);
                    label(ui, "What it does");
                    ui.label(
                        egui::RichText::new("Other bots read this to decide what to hand over.").font(theme::font(12.5)).color(p.faint),
                    );
                    multiline(ui, &mut a.description, "Researches topics on the web and writes short briefs with sources.", 3);
                    label(ui, "Standing instructions (optional)");
                    multiline(ui, &mut a.instructions, "How you want it to work.", 2);

                    label(ui, "Model");
                    let current = a.model.as_ref().map(|m| m.model.clone()).unwrap_or_else(|| "Default model".into());
                    egui::ComboBox::from_id_salt("agent-model").selected_text(current).width(300.0).show_ui(ui, |ui| {
                        if ui.selectable_label(a.model.is_none(), "Default model").clicked() {
                            a.model = None;
                        }
                        for (m, prov) in &models {
                            let sel = a.model.as_ref() == Some(m);
                            if ui.selectable_label(sel, format!("{} · {prov}", m.model)).clicked() {
                                a.model = Some(ModelRef { provider_id: m.provider_id.clone(), model: m.model.clone() });
                            }
                        }
                    });

                    label(ui, "Tools");
                    ui.horizontal_wrapped(|ui| {
                        let t = &mut a.tools;
                        ui.checkbox(&mut t.team, "Team (message and delegate)");
                        ui.checkbox(&mut t.browser, "Browser");
                        ui.checkbox(&mut t.web, "Web search");
                        ui.checkbox(&mut t.files, "Files");
                        ui.checkbox(&mut t.terminal, "Terminal");
                        ui.checkbox(&mut t.memory, "Memory");
                        ui.checkbox(&mut t.skills, "Skills");
                        ui.checkbox(&mut t.routines, "Routines");
                    });
                    if !connectors.is_empty() {
                        label(ui, "Connectors");
                        let mut all = a.connectors.is_none();
                        if ui.checkbox(&mut all, "All connectors").changed() {
                            a.connectors = if all { None } else { Some(Vec::new()) };
                        }
                        if let Some(list) = &mut a.connectors {
                            ui.horizontal_wrapped(|ui| {
                                for c in &connectors {
                                    let mut on = list.contains(&c.id);
                                    if ui.checkbox(&mut on, &c.name).changed() {
                                        if on {
                                            list.push(c.id.clone());
                                        } else {
                                            list.retain(|x| x != &c.id);
                                        }
                                    }
                                }
                            });
                        }
                    }
                    label(ui, "Approvals");
                    ui.horizontal_wrapped(|ui| {
                        ui.radio_value(&mut a.approval, ApprovalMode::Risky, "Ask before risky actions");
                        ui.radio_value(&mut a.approval, ApprovalMode::Everything, "Ask before every action");
                        ui.radio_value(&mut a.approval, ApprovalMode::Never, "Never ask");
                    });
                    label(ui, "Tool calling");
                    ui.horizontal_wrapped(|ui| {
                        ui.radio_value(&mut a.tool_mode, ToolMode::Auto, "Automatic");
                        ui.radio_value(&mut a.tool_mode, ToolMode::Native, "Native function calling");
                        ui.radio_value(&mut a.tool_mode, ToolMode::Prompt, "Prompt-based (for models without function calling)");
                    });
                    label(ui, "Workspace folder");
                    ui.horizontal(|ui| {
                        ui.set_width(ui.available_width());
                        let w = ui.available_width() - 90.0;
                        ui.allocate_ui(vec2(w, 34.0), |ui| {
                            widgets::text_field(ui, &mut ed.workspace_text, "Default: the bot's own folder", false)
                        });
                        if widgets::secondary_button(ui, "Browse").clicked()
                            && let Some(dir) = rfd::FileDialog::new().pick_folder()
                        {
                            ed.workspace_text = dir.display().to_string();
                        }
                    });
                    ui.checkbox(&mut ed.agent.proactive, "Follow up on stalled tasks on its own");
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ed = self.agent_ui.editor.as_ref().unwrap();
                        let ok = !ed.agent.name.trim().is_empty();
                        if widgets::primary_button(ui, if ed.is_new { "Create bot" } else { "Save" }).clicked() && ok {
                            save = true;
                        }
                        if widgets::secondary_button(ui, "Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            });
            if resp.should_close() {
                close = true;
            }
            if save {
                let mut ed = self.agent_ui.editor.take().unwrap();
                ed.agent.name = ed.agent.name.trim().to_string();
                ed.agent.workspace = Some(ed.workspace_text.trim()).filter(|s| !s.is_empty()).map(std::path::PathBuf::from);
                if ed.is_new {
                    let dm = self.fleet.create_agent(ed.agent);
                    self.open_thread(&dm);
                } else {
                    self.fleet.update_agent(ed.agent);
                }
            } else if close {
                self.agent_ui.editor = None;
            }
        }

        if self.agent_ui.channel_editor.is_some() {
            let agents = self.fleet.lock().agents.clone();
            let mut close = false;
            let mut save = false;
            let resp = egui::Modal::new(egui::Id::unique("channel-editor")).frame(frame(ctx)).backdrop_color(p.overlay).show(ctx, |ui| {
                ui.set_width(460.0);
                let ed = self.agent_ui.channel_editor.as_mut().unwrap();
                title(ui, if ed.id.is_some() { "Edit channel" } else { "New channel" });
                label(ui, "Name");
                widgets::text_field(ui, &mut ed.name, "for example launch-plan", false);
                label(ui, "Bots in this channel");
                if agents.is_empty() {
                    ui.label(egui::RichText::new("Create some bots first.").color(p.muted));
                }
                for a in &agents {
                    let mut on = ed.members.contains(&a.id);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, "").changed() {
                            if on {
                                ed.members.push(a.id.clone());
                            } else {
                                ed.members.retain(|m| m != &a.id);
                            }
                        }
                        super::avatar(ui, a, 22.0);
                        ui.label(egui::RichText::new(a.label()).font(theme::font(14.0)).color(p.text));
                    });
                }
                label(ui, "Lead (answers messages that mention nobody)");
                let lead_name = ed
                    .lead
                    .as_ref()
                    .and_then(|l| agents.iter().find(|a| &a.id == l))
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "None".into());
                egui::ComboBox::from_id_salt("channel-lead").selected_text(lead_name).show_ui(ui, |ui| {
                    for a in agents.iter().filter(|a| ed.members.contains(&a.id)) {
                        if ui.selectable_label(ed.lead.as_deref() == Some(&a.id), &a.name).clicked() {
                            ed.lead = Some(a.id.clone());
                        }
                    }
                });
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, if ed.id.is_some() { "Save" } else { "Create channel" }).clicked()
                        && !ed.members.is_empty()
                    {
                        save = true;
                    }
                    if widgets::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if resp.should_close() {
                close = true;
            }
            if save {
                let ed = self.agent_ui.channel_editor.take().unwrap();
                let name = if ed.name.trim().is_empty() { "general".to_string() } else { ed.name.clone() };
                match ed.id {
                    Some(id) => self.fleet.update_channel(&id, &name, ed.members, ed.lead),
                    None => {
                        let id = self.fleet.create_channel(&name, ed.members, ed.lead);
                        self.open_thread(&id);
                    }
                }
            } else if close {
                self.agent_ui.channel_editor = None;
            }
        }

        if self.agent_ui.routine_editor.is_some() {
            self.routine_modal(ctx);
        }
        if self.agent_ui.connector_editor.is_some() {
            self.connector_modal(ctx);
        }
        if self.agent_ui.skill_editor.is_some() {
            self.skill_modal(ctx);
        }
        self.agent_confirms(ctx);
    }

    fn routine_modal(&mut self, ctx: &egui::Context) {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        let (agents, channels, port, hooks_on) = {
            let st = self.fleet.lock();
            let ch: Vec<(String, String)> =
                st.threads.iter().filter_map(|t| t.channel_name().map(|n| (t.id.clone(), format!("#{n}")))).collect();
            (st.agents.clone(), ch, st.config.webhook_port, st.config.webhooks_enabled)
        };
        let mut close = false;
        let mut save = false;
        let resp = egui::Modal::new(egui::Id::unique("routine-editor")).frame(frame(ctx)).backdrop_color(p.overlay).show(ctx, |ui| {
            ui.set_width(520.0);
            let ed = self.agent_ui.routine_editor.as_mut().unwrap();
            title(ui, if ed.is_new { "New routine" } else { "Edit routine" });
            label(ui, "Name");
            widgets::text_field(ui, &mut ed.routine.name, "Morning inbox sweep", false);
            label(ui, "Bot");
            let an = agents.iter().find(|a| a.id == ed.routine.agent).map(|a| a.name.clone()).unwrap_or_else(|| "Pick a bot".into());
            egui::ComboBox::from_id_salt("routine-agent").selected_text(an).show_ui(ui, |ui| {
                for a in &agents {
                    if ui.selectable_label(ed.routine.agent == a.id, a.label()).clicked() {
                        ed.routine.agent = a.id.clone();
                    }
                }
            });
            label(ui, "When");
            let kinds = ["Every N minutes", "Hourly", "Daily", "Weekly", "Monthly", "When a file or folder changes", "On a webhook"];
            egui::ComboBox::from_id_salt("routine-kind").selected_text(kinds[ed.kind]).show_ui(ui, |ui| {
                for (i, k) in kinds.iter().enumerate() {
                    ui.selectable_value(&mut ed.kind, i, *k);
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| match ed.kind {
                0 => {
                    ui.label("Every");
                    ui.add(egui::DragValue::new(&mut ed.minutes).range(1..=10_080));
                    ui.label("minutes");
                }
                1 => {
                    ui.label("At minute");
                    ui.add(egui::DragValue::new(&mut ed.minute).range(0..=59));
                }
                2 | 3 | 4 => {
                    if ed.kind == 3 {
                        egui::ComboBox::from_id_salt("routine-wd").selected_text(weekday_name(ed.weekday)).show_ui(ui, |ui| {
                            for d in 0..7 {
                                ui.selectable_value(&mut ed.weekday, d, weekday_name(d));
                            }
                        });
                    }
                    if ed.kind == 4 {
                        ui.label("Day");
                        ui.add(egui::DragValue::new(&mut ed.day).range(1..=28));
                    }
                    ui.label("at");
                    ui.add(egui::DragValue::new(&mut ed.hour).range(0..=23));
                    ui.label(":");
                    ui.add(egui::DragValue::new(&mut ed.minute).range(0..=59));
                }
                5 => {
                    ui.allocate_ui(vec2(360.0, 34.0), |ui| widgets::text_field(ui, &mut ed.path, "/path/to/folder", false));
                    if widgets::secondary_button(ui, "Browse").clicked()
                        && let Some(d) = rfd::FileDialog::new().pick_folder()
                    {
                        ed.path = d.display().to_string();
                    }
                }
                _ => {
                    ui.vertical(|ui| {
                        let url = format!("http://127.0.0.1:{port}/hook/{}", ed.token);
                        ui.label(egui::RichText::new(&url).monospace().color(p.text));
                        if ui.small_button("Copy URL").clicked() {
                            ui.ctx().copy_text(url);
                        }
                        if !hooks_on {
                            ui.label(
                                egui::RichText::new("Turn on webhooks in Settings → Bots for this to run.")
                                    .font(theme::font(12.5))
                                    .color(p.danger),
                            );
                        }
                    });
                }
            });
            label(ui, "What to do");
            multiline(
                ui,
                &mut ed.routine.instructions,
                "Check the inbox, draft replies to anything urgent, and give me a three-line summary.",
                4,
            );
            label(ui, "Post results in");
            let current = match &ed.routine.thread {
                Some(t) => {
                    channels.iter().find(|(id, _)| id == t).map(|(_, n)| n.clone()).unwrap_or_else(|| "The bot's direct chat".into())
                }
                None => "The bot's direct chat".into(),
            };
            egui::ComboBox::from_id_salt("routine-thread").selected_text(current).show_ui(ui, |ui| {
                if ui.selectable_label(ed.routine.thread.is_none(), "The bot's direct chat").clicked() {
                    ed.routine.thread = None;
                }
                for (id, n) in &channels {
                    if ui.selectable_label(ed.routine.thread.as_deref() == Some(id), n).clicked() {
                        ed.routine.thread = Some(id.clone());
                    }
                }
            });
            ui.add_space(6.0);
            ui.checkbox(&mut ed.routine.enabled, "Enabled");
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = !ed.routine.name.trim().is_empty() && !ed.routine.agent.is_empty() && !ed.routine.instructions.trim().is_empty();
                if widgets::primary_button(ui, "Save").clicked() && ok {
                    save = true;
                }
                if widgets::secondary_button(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
        if resp.should_close() {
            close = true;
        }
        if save {
            let ed = self.agent_ui.routine_editor.take().unwrap();
            let mut r = ed.routine.clone();
            r.trigger = match ed.kind {
                0 => Trigger::Every { minutes: ed.minutes.max(1) },
                1 => Trigger::Hourly { minute: ed.minute },
                2 => Trigger::Daily { hour: ed.hour, minute: ed.minute },
                3 => Trigger::Weekly { weekday: ed.weekday, hour: ed.hour, minute: ed.minute },
                4 => Trigger::Monthly { day: ed.day, hour: ed.hour, minute: ed.minute },
                5 => Trigger::FileChange { path: ed.path.trim().into() },
                _ => Trigger::Webhook { token: ed.token.clone() },
            };
            if ed.is_new || r.last_run.is_none() {
                // Schedules count from now, not from when the routine was first made.
                r.created = chrono::Utc::now();
            }
            let mut st = self.fleet.lock();
            match st.routines.iter_mut().find(|x| x.id == r.id) {
                Some(x) => *x = r,
                None => st.routines.push(r),
            }
            st.dirty.routines = true;
        } else if close {
            self.agent_ui.routine_editor = None;
        }
    }

    pub fn routine_editor_for(&mut self, r: Option<Routine>) {
        let first_agent = self.fleet.lock().agents.first().map(|a| a.id.clone()).unwrap_or_default();
        let is_new = r.is_none();
        let routine = r.unwrap_or_else(|| Routine::new(String::new(), first_agent, String::new(), Trigger::Daily { hour: 9, minute: 0 }));
        let mut ed = RoutineEditor {
            kind: 2,
            minutes: 60,
            hour: 9,
            minute: 0,
            weekday: 0,
            day: 1,
            path: String::new(),
            token: Fleet::new_routine_token(),
            is_new,
            routine,
        };
        match ed.routine.trigger.clone() {
            Trigger::Every { minutes } => (ed.kind, ed.minutes) = (0, minutes),
            Trigger::Hourly { minute } => (ed.kind, ed.minute) = (1, minute),
            Trigger::Daily { hour, minute } => (ed.kind, ed.hour, ed.minute) = (2, hour, minute),
            Trigger::Weekly { weekday, hour, minute } => (ed.kind, ed.weekday, ed.hour, ed.minute) = (3, weekday, hour, minute),
            Trigger::Monthly { day, hour, minute } => (ed.kind, ed.day, ed.hour, ed.minute) = (4, day, hour, minute),
            Trigger::FileChange { path } => (ed.kind, ed.path) = (5, path.display().to_string()),
            Trigger::Webhook { token } => (ed.kind, ed.token) = (6, token),
        }
        self.agent_ui.routine_editor = Some(ed);
    }

    pub fn connector_editor_for(&mut self, c: Option<Connector>) {
        let is_new = c.is_none();
        let connector = c.unwrap_or_else(|| Connector {
            id: crate::store::new_id(),
            name: String::new(),
            enabled: true,
            transport: Transport::Stdio { command: String::new(), args: Vec::new(), env: Vec::new() },
            trusted: false,
        });
        let (target, extra, http) = match &connector.transport {
            Transport::Stdio { command, args, env } => {
                let mut line = quote(command);
                for a in args {
                    line.push(' ');
                    line.push_str(&quote(a));
                }
                (line, env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n"), false)
            }
            Transport::Http { url, headers } => {
                (url.clone(), headers.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"), true)
            }
        };
        self.agent_ui.connector_editor = Some(ConnectorEditor { connector, is_new, target, extra, http });
    }

    fn connector_modal(&mut self, ctx: &egui::Context) {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        let mut close = false;
        let mut save = false;
        let resp = egui::Modal::new(egui::Id::unique("connector-editor")).frame(frame(ctx)).backdrop_color(p.overlay).show(ctx, |ui| {
            ui.set_width(520.0);
            let ed = self.agent_ui.connector_editor.as_mut().unwrap();
            title(ui, if ed.is_new { "Add connector" } else { "Edit connector" });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Connectors are MCP servers. Connect one once and every bot can use it (you can limit this per bot).",
                    )
                    .font(theme::font(13.0))
                    .color(p.muted),
                )
                .wrap(),
            );
            label(ui, "Name");
            widgets::text_field(ui, &mut ed.connector.name, "GitHub", false);
            label(ui, "Type");
            ui.horizontal(|ui| {
                ui.radio_value(&mut ed.http, false, "Local program (stdio)");
                ui.radio_value(&mut ed.http, true, "Remote server (HTTP)");
            });
            if ed.http {
                label(ui, "URL");
                widgets::text_field(ui, &mut ed.target, "https://example.com/mcp", false);
                label(ui, "Headers, one per line (for example Authorization: Bearer …)");
                multiline(ui, &mut ed.extra, "Authorization: Bearer …", 2);
            } else {
                label(ui, "Command");
                widgets::text_field(ui, &mut ed.target, "npx -y @modelcontextprotocol/server-filesystem /path", false);
                label(ui, "Environment variables, one KEY=value per line");
                multiline(ui, &mut ed.extra, "GITHUB_TOKEN=…", 2);
            }
            ui.add_space(6.0);
            ui.checkbox(&mut ed.connector.trusted, "Trusted: run its tools without asking (read-only tools never ask)");
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::primary_button(ui, "Save").clicked() && !ed.connector.name.trim().is_empty() && !ed.target.trim().is_empty() {
                    save = true;
                }
                if widgets::secondary_button(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
        if resp.should_close() {
            close = true;
        }
        if save {
            let ed = self.agent_ui.connector_editor.take().unwrap();
            let mut c = ed.connector;
            c.name = c.name.trim().to_string();
            c.transport = if ed.http {
                let headers =
                    ed.extra.lines().filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
                Transport::Http { url: ed.target.trim().to_string(), headers }
            } else {
                let mut parts = split_command(&ed.target);
                let command = if parts.is_empty() { String::new() } else { parts.remove(0) };
                let env =
                    ed.extra.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
                Transport::Stdio { command, args: parts, env }
            };
            let mut cfg = self.fleet.lock().config.clone();
            match cfg.connectors.iter_mut().find(|x| x.id == c.id) {
                Some(x) => *x = c.clone(),
                None => cfg.connectors.push(c.clone()),
            }
            self.fleet.save_config(cfg);
            self.fleet.mcp().restart(&c);
        } else if close {
            self.agent_ui.connector_editor = None;
        }
    }

    fn skill_modal(&mut self, ctx: &egui::Context) {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        let mut close = false;
        let mut save = false;
        let resp = egui::Modal::new(egui::Id::unique("skill-editor")).frame(frame(ctx)).backdrop_color(p.overlay).show(ctx, |ui| {
            ui.set_width(560.0);
            let s = self.agent_ui.skill_editor.as_mut().unwrap();
            title(ui, "Skill");
            label(ui, "Name");
            widgets::text_field(ui, &mut s.name, "weekly-report", false);
            label(ui, "When to use it");
            widgets::text_field(ui, &mut s.description, "Every Friday, to send the team summary", false);
            label(ui, "Steps");
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| multiline(ui, &mut s.instructions, "1. …", 10));
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::primary_button(ui, "Save").clicked() && !s.name.trim().is_empty() {
                    save = true;
                }
                if widgets::secondary_button(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
        if resp.should_close() {
            close = true;
        }
        if save {
            let mut s = self.agent_ui.skill_editor.take().unwrap();
            s.updated = chrono::Utc::now();
            let mut st = self.fleet.lock();
            match st.skills.iter_mut().find(|x| x.id == s.id) {
                Some(x) => *x = s,
                None => st.skills.push(s),
            }
            st.dirty.skills = true;
        } else if close {
            self.agent_ui.skill_editor = None;
        }
    }

    fn agent_confirms(&mut self, ctx: &egui::Context) {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        let (title_text, body, which) = if let Some(id) = &self.agent_ui.confirm_delete_agent {
            let name = self.fleet.lock().agent(id).map(|a| a.name.clone()).unwrap_or_default();
            (
                format!("Delete {name}?"),
                "Its chats, memories and routines are deleted too. Files in its workspace stay on disk.".to_string(),
                0,
            )
        } else if self.agent_ui.confirm_delete_thread.is_some() {
            ("Delete this conversation?".to_string(), "All its messages are deleted.".to_string(), 1)
        } else {
            return;
        };
        let mut result = None;
        let resp = egui::Modal::new(egui::Id::unique("agent-confirm")).frame(frame(ctx)).backdrop_color(p.overlay).show(ctx, |ui| {
            ui.set_width(400.0);
            title(ui, &title_text);
            ui.add(egui::Label::new(egui::RichText::new(body).font(theme::font(14.0)).color(p.muted)).wrap());
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::danger_button(ui, "Delete").clicked() {
                    result = Some(true);
                }
                if widgets::secondary_button(ui, "Cancel").clicked() {
                    result = Some(false);
                }
            });
        });
        if resp.should_close() && result.is_none() {
            result = Some(false);
        }
        if let Some(go) = result {
            if which == 0 {
                let id = self.agent_ui.confirm_delete_agent.take().unwrap();
                if go {
                    self.fleet.browser().close_tab(&id);
                    self.fleet.delete_agent(&id);
                    self.page = None;
                }
            } else {
                let id = self.agent_ui.confirm_delete_thread.take().unwrap();
                if go {
                    self.fleet.delete_thread(&id);
                    if self.page == Some(super::Page::Thread(id)) {
                        self.page = None;
                    }
                }
            }
        }
    }
}

fn quote(s: &str) -> String {
    if s.contains(' ') { format!("\"{s}\"") } else { s.to_string() }
}

/// Split a command line on spaces, keeping quoted parts together.
pub fn split_command(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (_, c) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits_commands() {
        assert_eq!(super::split_command("npx -y \"my server\" '/a b'"), vec!["npx", "-y", "my server", "/a b"]);
    }
}
