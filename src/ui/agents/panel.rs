//! The right-hand agent panel: profile and settings, the agent's computer
//! (live browser with take over and teach a task, files, terminal history)
//! and memory.

use std::path::{Path, PathBuf};

use egui::{Align2, CornerRadius, Sense, Stroke, Vec2, pos2, vec2};

use super::super::icons::{self, Icon};
use super::super::theme::{self, Palette};
use super::super::widgets;
use super::thread::code_box;
use super::{ComputerTab, PanelTab, avatar};
use crate::agents::browser::{VIEW_H, VIEW_W};
use crate::agents::model::{Agent, ApprovalMode, Author, Memory, ToolStatus};
use crate::app::FreeBotApp;

impl FreeBotApp {
    pub fn agent_panel_ui(&mut self, ui: &mut egui::Ui, a: &Agent, tab: PanelTab) {
        let p = Palette::of(ui);
        ui.horizontal(|ui| {
            for (t, label) in [(PanelTab::Profile, "Profile"), (PanelTab::Computer, "Computer"), (PanelTab::Memory, "Memory")] {
                let on = tab == t;
                if tab_pill(ui, label, on).clicked() {
                    self.agent_ui.panel = Some(t);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button_sized(ui, Icon::Close, "Close panel", 28.0, None).clicked() {
                    self.agent_ui.panel = None;
                }
            });
        });
        ui.add_space(8.0);
        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, p.border));
        ui.add_space(10.0);
        match tab {
            PanelTab::Profile => {
                egui::ScrollArea::vertical().id_salt("panel-profile").auto_shrink([false, false]).show(ui, |ui| self.profile_tab(ui, a));
            }
            PanelTab::Computer => self.computer_tab(ui, a),
            PanelTab::Memory => {
                egui::ScrollArea::vertical().id_salt("panel-memory").auto_shrink([false, false]).show(ui, |ui| self.memory_tab(ui, a));
            }
        }
    }

    fn profile_tab(&mut self, ui: &mut egui::Ui, a: &Agent) {
        let p = Palette::of(ui);
        ui.vertical_centered(|ui| {
            avatar(ui, a, 64.0);
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&a.name).font(theme::font_bold(19.0)).color(p.text));
            if !a.title.is_empty() {
                ui.label(egui::RichText::new(&a.title).font(theme::font(14.0)).color(p.muted));
            }
        });
        ui.add_space(10.0);
        section(ui, "Description");
        ui.add(
            egui::Label::new(
                egui::RichText::new(if a.description.is_empty() { "No description yet." } else { &a.description })
                    .font(theme::font(13.5))
                    .color(p.text),
            )
            .wrap(),
        );
        if !a.instructions.is_empty() {
            section(ui, "Standing instructions");
            ui.add(egui::Label::new(egui::RichText::new(&a.instructions).font(theme::font(13.5)).color(p.text)).wrap());
        }
        section(ui, "Model");
        let model = match &a.model {
            Some(m) => {
                let prov = self.settings.provider(&m.provider_id).map(|p| p.name.clone()).unwrap_or_default();
                format!("{} · {prov}", m.model)
            }
            None => match self.settings.effective_model() {
                Some(m) => format!("Default ({})", m.model),
                None => "No model set up yet".into(),
            },
        };
        ui.label(egui::RichText::new(model).font(theme::font(13.5)).color(p.text));

        section(ui, "Approvals");
        let mut mode = a.approval;
        ui.horizontal_wrapped(|ui| {
            for (m, label) in [
                (ApprovalMode::Risky, "Ask for risky actions"),
                (ApprovalMode::Everything, "Ask for everything"),
                (ApprovalMode::Never, "Never ask"),
            ] {
                if tab_pill(ui, label, mode == m).clicked() {
                    mode = m;
                }
            }
        });
        if !a.always_allow.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("Always allowed: {}", a.always_allow.join(", "))).font(theme::font(12.5)).color(p.muted));
            if ui.small_button("Reset always-allowed tools").clicked() {
                let mut x = a.clone();
                x.always_allow.clear();
                self.fleet.update_agent(x);
            }
        }
        if mode != a.approval {
            let mut x = a.clone();
            x.approval = mode;
            self.fleet.update_agent(x);
        }

        section(ui, "Tools");
        let t = &a.tools;
        let on: Vec<&str> = [
            (t.team, "Team"),
            (t.browser, "Browser"),
            (t.web, "Web"),
            (t.files, "Files"),
            (t.terminal, "Terminal"),
            (t.memory, "Memory"),
            (t.skills, "Skills"),
            (t.routines, "Routines"),
        ]
        .iter()
        .filter(|(b, _)| *b)
        .map(|(_, n)| *n)
        .collect();
        ui.label(egui::RichText::new(on.join(" · ")).font(theme::font(13.0)).color(p.text));
        let connectors = self
            .fleet
            .lock()
            .config
            .connectors
            .iter()
            .filter(|c| c.enabled && a.connectors.as_ref().is_none_or(|l| l.contains(&c.id)))
            .map(|c| c.name.clone())
            .collect::<Vec<_>>();
        if !connectors.is_empty() {
            ui.label(egui::RichText::new(format!("Connectors: {}", connectors.join(", "))).font(theme::font(13.0)).color(p.muted));
        }

        let mut proactive = a.proactive;
        ui.add_space(8.0);
        widgets::setting_row(ui, "Proactive follow-ups", "Nudges teammates when tasks it handed out stall", |ui| {
            widgets::switch(ui, &mut proactive);
        });
        if proactive != a.proactive {
            let mut x = a.clone();
            x.proactive = proactive;
            self.fleet.update_agent(x);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, "Edit bot").clicked() {
                self.edit_agent(&a.id);
            }
            if widgets::secondary_button(ui, "Routines").clicked() {
                self.open_page(super::Page::Routines);
            }
        });
    }

    fn memory_tab(&mut self, ui: &mut egui::Ui, a: &Agent) {
        let p = Palette::of(ui);
        let mems: Vec<Memory> =
            self.fleet.lock().memories.iter().filter(|m| m.agent.is_none() || m.agent.as_deref() == Some(&a.id)).cloned().collect();
        ui.label(
            egui::RichText::new(format!("What {} remembers. Team memories are shared by every bot.", a.name))
                .font(theme::font(13.0))
                .color(p.muted),
        );
        ui.add_space(8.0);
        let mut remove = None;
        if mems.is_empty() {
            ui.label(
                egui::RichText::new("Nothing yet. Bots save facts and preferences as they learn them.")
                    .font(theme::font(13.0))
                    .color(p.faint),
            );
        }
        for m in mems.iter().rev() {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width() - 34.0);
                            ui.add(egui::Label::new(egui::RichText::new(&m.text).font(theme::font(13.5)).color(p.text)).wrap());
                            ui.label(
                                egui::RichText::new(if m.agent.is_none() { "Team" } else { "Only this bot" })
                                    .font(theme::font(11.5))
                                    .color(p.faint),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button_sized(ui, Icon::Trash, "Forget", 26.0, None).clicked() {
                                remove = Some(m.id.clone());
                            }
                        });
                    });
                });
            ui.add_space(4.0);
        }
        if let Some(id) = remove {
            let mut st = self.fleet.lock();
            st.memories.retain(|m| m.id != id);
            st.dirty.memories = true;
        }
        ui.add_space(8.0);
        let mut text = std::mem::take(&mut self.agent_ui.memory_input);
        let r = widgets::text_field(ui, &mut text, "Add something to remember…", false);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.agent_ui.memory_shared, "Share with the whole team");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let add =
                    widgets::secondary_button(ui, "Add").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if add && !text.trim().is_empty() {
                    let mut st = self.fleet.lock();
                    st.memories.push(Memory {
                        id: crate::store::new_id(),
                        agent: if self.agent_ui.memory_shared { None } else { Some(a.id.clone()) },
                        text: text.trim().to_string(),
                        created: chrono::Utc::now(),
                    });
                    st.dirty.memories = true;
                    text.clear();
                }
            });
        });
        self.agent_ui.memory_input = text;
    }

    fn computer_tab(&mut self, ui: &mut egui::Ui, a: &Agent) {
        let tab = self.agent_ui.computer_tab.unwrap_or(ComputerTab::Browser);
        ui.horizontal(|ui| {
            for (t, icon, label) in [
                (ComputerTab::Browser, Icon::Globe, "Browser"),
                (ComputerTab::Files, Icon::Folder, "Files"),
                (ComputerTab::Terminal, Icon::Terminal, "Terminal"),
            ] {
                if widgets::pill_toggle(ui, icon, label, tab == t, label).clicked() {
                    self.agent_ui.computer_tab = Some(t);
                }
            }
        });
        ui.add_space(10.0);
        match tab {
            ComputerTab::Browser => self.browser_view(ui, a),
            ComputerTab::Files => {
                egui::ScrollArea::vertical().id_salt("panel-files").auto_shrink([false, false]).show(ui, |ui| self.files_view(ui, a));
            }
            ComputerTab::Terminal => {
                egui::ScrollArea::vertical()
                    .id_salt("panel-term")
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| self.terminal_view(ui, a));
            }
        }
    }

    fn browser_view(&mut self, ui: &mut egui::Ui, a: &Agent) {
        let p = Palette::of(ui);
        let browser = self.fleet.browser().clone();
        browser.watch(&a.id);
        let takeover = self.fleet.lock().takeover.contains(&a.id);
        let recording = browser.is_recording(&a.id);

        // Address bar.
        let (url, title, seq, jpeg) = browser.with_frame(&a.id, |f| match f {
            Some(f) => (f.url.clone(), f.title.clone(), f.seq, Some(f.jpeg.clone())),
            None => (String::new(), String::new(), 0, None),
        });
        if (self.agent_ui.url_input.is_empty() || !ui.memory(|m| m.has_focus(egui::Id::unique("browser-url")))) && !url.is_empty() {
            self.agent_ui.url_input = url.clone();
        }
        ui.horizontal(|ui| {
            let mut u = std::mem::take(&mut self.agent_ui.url_input);
            let r = widgets::input_frame(ui).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut u)
                        .id(egui::Id::unique("browser-url"))
                        .hint_text("Type a URL")
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_width(f32::INFINITY),
                )
            });
            if r.inner.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !u.trim().is_empty() {
                browser.user_open(&a.id, u.trim().to_string());
            }
            self.agent_ui.url_input = u;
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let label = if takeover { "Hand back" } else { "Take over" };
            if widgets::pill_toggle(
                ui,
                Icon::Hand,
                label,
                takeover,
                "Control the browser yourself, for example to log in. The bot waits until you hand it back.",
            )
            .clicked()
            {
                let mut st = self.fleet.lock();
                if takeover {
                    st.takeover.remove(&a.id);
                } else {
                    st.takeover.insert(a.id.clone());
                }
            }
            let tlabel = if recording { "Stop teaching" } else { "Teach a task" };
            if widgets::pill_toggle(
                ui,
                Icon::Record,
                tlabel,
                recording,
                "Do the task yourself in this browser. The bot turns your steps into a reusable skill.",
            )
            .clicked()
            {
                if recording {
                    if let Some(steps) = browser.stop_recording(&a.id) {
                        self.fleet.lock().takeover.remove(&a.id);
                        self.finish_teaching(a, steps);
                    }
                } else {
                    browser.start_recording(&a.id);
                    self.fleet.lock().takeover.insert(a.id.clone());
                    if !browser.has_tab(&a.id) {
                        browser.warm_up(&a.id);
                    }
                }
            }
        });
        ui.add_space(6.0);

        let Some(jpeg) = jpeg else {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        widgets::icon(ui, Icon::Globe, 36.0, p.muted);
                        ui.label(egui::RichText::new("The browser opens when the bot needs it.").font(theme::font(13.5)).color(p.muted));
                        ui.add_space(6.0);
                        if widgets::secondary_button(ui, "Open browser now").clicked() {
                            browser.warm_up(&a.id);
                        }
                    });
                });
            return;
        };

        // Upload a new texture when the frame changed.
        let needs = self.agent_ui.browser_tex.get(&a.id).is_none_or(|(s, _)| *s != seq);
        if needs && let Ok(img) = image::load_from_memory(&jpeg) {
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let tex = ui.ctx().load_texture(
                format!("browser-{}", a.id),
                egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                egui::TextureOptions::LINEAR,
            );
            self.agent_ui.browser_tex.insert(a.id.clone(), (seq, tex));
        }
        let Some((_, tex)) = self.agent_ui.browser_tex.get(&a.id).cloned() else { return };
        if !title.is_empty() {
            let t = widgets::truncate_to_width(ui, &title, &theme::font(12.5), ui.available_width());
            ui.label(egui::RichText::new(t).font(theme::font(12.5)).color(p.muted));
        }
        let w = ui.available_width();
        let size = vec2(w, w * VIEW_H as f32 / VIEW_W as f32);
        let (rect, resp) = ui.allocate_exact_size(size, if takeover { Sense::click() } else { Sense::hover() });
        ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        let border = if takeover { Stroke::new(2.0, p.text) } else { Stroke::new(1.0, p.border) };
        ui.painter().rect_stroke(rect, CornerRadius::same(6), border, egui::StrokeKind::Outside);
        if recording {
            ui.painter().circle_filled(rect.left_top() + vec2(14.0, 14.0), 6.0, p.danger);
            ui.painter().text(rect.left_top() + vec2(26.0, 14.0), Align2::LEFT_CENTER, "Recording", theme::font_bold(12.0), p.danger);
        }

        if takeover {
            if resp.clicked() {
                self.agent_ui.browser_has_focus = true;
                if let Some(pos) = resp.interact_pointer_pos() {
                    let x = (pos.x - rect.left()) / rect.width() * VIEW_W as f32;
                    let y = (pos.y - rect.top()) / rect.height() * VIEW_H as f32;
                    browser.user_click(&a.id, x as f64, y as f64);
                }
            }
            if resp.hovered() {
                let dy = ui.input(|i| i.smooth_scroll_delta.y);
                if dy.abs() > 0.5 {
                    browser.user_scroll(&a.id, (-dy * 2.0) as f64);
                }
            }
            if ui.input(|i| i.pointer.any_pressed()) && !resp.hovered() {
                self.agent_ui.browser_has_focus = false;
            }
            if self.agent_ui.browser_has_focus {
                let events = ui.input(|i| i.events.clone());
                for e in events {
                    match e {
                        egui::Event::Text(t) => browser.user_type(&a.id, t),
                        egui::Event::Key { key, pressed: true, .. } => {
                            let k = match key {
                                egui::Key::Enter => Some("Enter"),
                                egui::Key::Backspace => Some("Backspace"),
                                egui::Key::Tab => Some("Tab"),
                                egui::Key::Escape => Some("Escape"),
                                egui::Key::ArrowDown => Some("ArrowDown"),
                                egui::Key::ArrowUp => Some("ArrowUp"),
                                egui::Key::ArrowLeft => Some("ArrowLeft"),
                                egui::Key::ArrowRight => Some("ArrowRight"),
                                _ => None,
                            };
                            if let Some(k) = k {
                                browser.user_key(&a.id, k);
                            }
                        }
                        _ => {}
                    }
                }
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(if self.agent_ui.browser_has_focus {
                    "You are in control. Click and type in the page."
                } else {
                    "Click the page to control it."
                })
                .font(theme::font(12.5))
                .color(p.muted),
            );
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }

    fn finish_teaching(&mut self, a: &Agent, steps: Vec<String>) {
        if steps.is_empty() {
            self.toast("Nothing was recorded.");
            return;
        }
        let mut text = String::from("I just showed you how to do a task in your browser. These are the steps I took:\n\n");
        for (i, s) in steps.iter().enumerate() {
            text.push_str(&format!("{}. {s}\n", i + 1));
        }
        text.push_str("\nTurn this into a reusable skill with save_skill: give it a clear name and description, generalize the specific values into what to look for, and write steps you can follow on your own next time. Then tell me what you saved.");
        let dm = self.fleet.lock().dm_thread(&a.id);
        self.fleet.user_post(&dm, &text);
        self.open_thread(&dm);
    }

    fn files_view(&mut self, ui: &mut egui::Ui, a: &Agent) {
        let p = Palette::of(ui);
        let root = self.fleet.workspace_of(a);
        let rel = self.agent_ui.files_dir.clone();
        let dir = root.join(&rel);
        ui.horizontal(|ui| {
            if !rel.is_empty() && widgets::icon_button_sized(ui, Icon::ChevronLeft, "Up", 26.0, None).clicked() {
                let parent = Path::new(&rel).parent().map(|p| p.display().to_string()).unwrap_or_default();
                self.agent_ui.files_dir = parent;
                self.agent_ui.preview = None;
            }
            let label = if rel.is_empty() { "Workspace".to_string() } else { format!("Workspace/{rel}") };
            ui.label(egui::RichText::new(label).font(theme::font_bold(13.5)).color(p.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button_sized(ui, Icon::Folder, "Open in file manager", 26.0, None).clicked() {
                    open_in_os(&dir);
                }
                if widgets::icon_button_sized(ui, Icon::Refresh, "Refresh", 26.0, None).clicked() {
                    self.agent_ui.preview = None;
                }
            });
        });
        ui.label(egui::RichText::new(root.display().to_string()).font(theme::font(11.5)).color(p.faint));
        ui.add_space(6.0);
        let mut entries: Vec<(bool, String, u64)> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| {
                        let m = e.metadata().ok();
                        (
                            m.as_ref().is_some_and(|m| m.is_dir()),
                            e.file_name().to_string_lossy().to_string(),
                            m.map(|m| m.len()).unwrap_or(0),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
        if entries.is_empty() {
            ui.label(egui::RichText::new("Empty. Files the bot creates show up here.").font(theme::font(13.0)).color(p.faint));
        }
        for (is_dir, name, size) in entries.iter().take(300) {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(6), p.hover);
            }
            icons::paint(
                ui.painter(),
                egui::Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), Vec2::splat(16.0)),
                if *is_dir { Icon::Folder } else { Icon::File },
                p.muted,
            );
            ui.painter().text(pos2(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER, name, theme::font(13.5), p.text);
            if !is_dir {
                ui.painter().text(
                    pos2(rect.right() - 6.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    human_size(*size),
                    theme::font(11.5),
                    p.faint,
                );
            }
            if resp.clicked() {
                let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                if *is_dir {
                    self.agent_ui.files_dir = child;
                    self.agent_ui.preview = None;
                } else {
                    let content = std::fs::read(root.join(&child))
                        .ok()
                        .map(|b| String::from_utf8(b).unwrap_or_else(|_| "(binary file)".into()))
                        .unwrap_or_else(|| "(cannot read)".into());
                    self.agent_ui.preview = Some((child, content.chars().take(50_000).collect()));
                }
            }
        }
        if let Some((name, content)) = self.agent_ui.preview.clone() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&name).font(theme::font_bold(13.0)).color(p.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button_sized(ui, Icon::Close, "Close preview", 24.0, None).clicked() {
                        self.agent_ui.preview = None;
                    }
                    if widgets::icon_button_sized(ui, Icon::Copy, "Copy", 24.0, None).clicked() {
                        ui.ctx().copy_text(content.clone());
                    }
                });
            });
            code_box(ui, &content, 60);
        }
    }

    fn terminal_view(&mut self, ui: &mut egui::Ui, a: &Agent) {
        let p = Palette::of(ui);
        let mut runs: Vec<(chrono::DateTime<chrono::Utc>, String, Option<String>, ToolStatus)> = Vec::new();
        {
            let st = self.fleet.lock();
            for t in &st.threads {
                for m in &t.messages {
                    if m.author != Author::Agent(a.id.clone()) {
                        continue;
                    }
                    for c in &m.tool_calls {
                        if c.name == "run_command" {
                            let cmd = serde_json::from_str::<serde_json::Value>(&c.arguments)
                                .ok()
                                .and_then(|v| v.get("command").and_then(|x| x.as_str()).map(str::to_string))
                                .unwrap_or_default();
                            runs.push((m.created, cmd, c.result.clone(), c.status));
                        }
                    }
                }
            }
        }
        runs.sort_by_key(|r| r.0);
        ui.label(egui::RichText::new("Commands this bot ran in its workspace.").font(theme::font(13.0)).color(p.muted));
        ui.add_space(6.0);
        if runs.is_empty() {
            ui.label(egui::RichText::new("No commands yet.").font(theme::font(13.0)).color(p.faint));
        }
        for (_, cmd, result, status) in runs.iter().rev().take(40).rev() {
            let mut text = format!("$ {cmd}\n");
            match status {
                ToolStatus::Denied => text.push_str("(denied by you)"),
                ToolStatus::AwaitingApproval => text.push_str("(waiting for your approval)"),
                ToolStatus::Running => text.push_str("(running…)"),
                _ => text.push_str(result.as_deref().unwrap_or("")),
            }
            code_box(ui, &text, 30);
            ui.add_space(6.0);
        }
    }
}

fn human_size(b: u64) -> String {
    match b {
        0..1024 => format!("{b} B"),
        1024..1_048_576 => format!("{:.1} KB", b as f64 / 1024.0),
        _ => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

pub fn open_in_os(path: &PathBuf) {
    let _ = std::fs::create_dir_all(path);
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}

fn section(ui: &mut egui::Ui, title: &str) {
    let p = Palette::of(ui);
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).font(theme::font_bold(12.5)).color(p.faint));
    ui.add_space(2.0);
}

pub fn tab_pill(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    let p = Palette::of(ui);
    let font = theme::font(13.0);
    let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), p.text).size().x + 22.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    let fill = if on {
        p.selected
    } else if resp.hovered() {
        p.hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect(rect, CornerRadius::same(14), fill, Stroke::new(1.0, if on { p.muted } else { p.border }), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, if on { p.text } else { p.muted });
    resp
}
