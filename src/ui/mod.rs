//! Everything that draws. The window is a sidebar on the left and the main
//! area on the right (home screen or an open chat), with modals on top.

pub mod agents;
pub mod chat;
pub mod composer;
pub mod icons;
pub mod modals;
pub mod settings;
pub mod sidebar;
pub mod theme;
pub mod widgets;

use std::time::Duration;

use egui::{Align2, CornerRadius, Frame, Key, KeyboardShortcut, Modifiers, Stroke, vec2};

use crate::app::{FreeBotApp, SettingsTab};
use theme::Palette;

const SC_NEW_CHAT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::J);
const SC_NEW_CHAT_ALT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O);
const SC_SEARCH: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::K);
const SC_SIDEBAR: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::B);
const SC_SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
const SC_PRIVATE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::P);

pub fn shortcut_text(ctx: &egui::Context, sc: &KeyboardShortcut) -> String {
    ctx.format_shortcut(sc)
}

impl eframe::App for FreeBotApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sync_settings(&ctx);
        self.pump();
        self.pump_fleet(&ctx);
        self.handle_shortcuts(&ctx);
        self.handle_dropped_files(&ctx);
        self.hovered_msg = self.hovered_msg_next.take();

        let p = Palette::of(ui);
        let width = if self.settings.sidebar_open { theme::SIDEBAR_WIDTH } else { theme::RAIL_WIDTH };
        egui::Panel::left("sidebar")
            .exact_size(width)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.sidebar).stroke(Stroke::new(1.0, p.border)).inner_margin(egui::Margin::symmetric(10, 12)))
            .show(ui, |ui| self.sidebar_ui(ui));

        egui::CentralPanel::default().frame(Frame::new().fill(p.bg)).show(ui, |ui| self.main_ui(ui));

        self.modals_ui(&ctx);
        self.agent_modals(&ctx);
        self.toasts_ui(&ctx);
        self.drop_overlay(&ctx);

        if !self.streams.is_empty() || !self.fleet.lock().runs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        // Routines and follow-ups are checked about once a second.
        ctx.request_repaint_after(Duration::from_millis(1000));
        self.screenshot_step(&ctx);
    }

    fn on_exit(&mut self) {
        self.save_settings();
        self.fleet.stop_all();
        self.fleet.flush();
    }
}

impl FreeBotApp {
    /// Keep the engine in step with settings, run due routines, and show notices.
    fn pump_fleet(&mut self, ctx: &egui::Context) {
        self.fleet.set_settings(&self.settings);
        self.fleet.tick();
        self.agent_ui.window_focused = ctx.input(|i| i.focused);
        let (notices, notify) = {
            let mut st = self.fleet.lock();
            (std::mem::take(&mut st.notices), st.config.notifications)
        };
        for n in notices {
            let viewing = matches!(&self.page, Some(agents::Page::Thread(t)) if Some(t) == n.thread.as_ref());
            if self.agent_ui.window_focused {
                if !viewing {
                    self.toast(format!("{}: {}", n.title, n.body));
                }
            } else if notify {
                std::thread::spawn(move || {
                    let _ = notify_rust::Notification::new().summary(&n.title).body(&n.body).appname("Free Bot").show();
                });
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (new_chat, search, sidebar, settings, private, esc) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&SC_NEW_CHAT) || i.consume_shortcut(&SC_NEW_CHAT_ALT),
                i.consume_shortcut(&SC_SEARCH),
                i.consume_shortcut(&SC_SIDEBAR),
                i.consume_shortcut(&SC_SETTINGS),
                i.consume_shortcut(&SC_PRIVATE),
                i.key_pressed(Key::Escape),
            )
        });
        if new_chat {
            self.new_chat();
        }
        if search {
            if self.search.is_some() { self.search = None } else { self.open_search() }
        }
        if sidebar {
            self.settings.sidebar_open = !self.settings.sidebar_open;
            self.mark_settings_dirty();
        }
        if settings {
            self.open_settings(SettingsTab::General);
        }
        if private {
            self.toggle_private();
        }
        // Escape stops a running answer when nothing else is open.
        if esc
            && self.search.is_none()
            && self.settings_ui.is_none()
            && self.confirm.is_none()
            && self.editing.is_none()
            && let Some(id) = self.current.clone()
        {
            self.stop(&id);
        }
    }

    pub fn toggle_private(&mut self) {
        if self.current.is_none() {
            self.private_mode = !self.private_mode;
        } else {
            // Start a fresh chat in private mode, like the home toggle.
            self.new_chat();
            self.private_mode = true;
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            let path = f.path().to_path_buf();
            if path.is_file() {
                self.attach_path(&path);
            } else {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
                match f.bytes() {
                    Ok(bytes) => self.attach_bytes(&name, &bytes),
                    Err(e) => self.toast_error(format!("Could not read dropped file: {e}")),
                }
            }
        }
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
        painter.rect_filled(screen, 0.0, p.overlay);
        let card = egui::Rect::from_center_size(screen.center(), vec2(360.0, 160.0));
        painter.rect(card, CornerRadius::same(20), p.surface, Stroke::new(2.0, p.muted), egui::StrokeKind::Inside);
        icons::paint(
            &painter,
            egui::Rect::from_center_size(card.center() - vec2(0.0, 22.0), egui::Vec2::splat(36.0)),
            icons::Icon::Paperclip,
            p.text,
        );
        painter.text(card.center() + vec2(0.0, 30.0), Align2::CENTER_CENTER, "Drop files to attach", theme::font_bold(17.0), p.text);
    }

    fn toasts_ui(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        egui::Area::new(egui::Id::unique("toasts"))
            .order(egui::Order::Tooltip)
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .interactable(false)
            .show(ctx, |ui| {
                for t in &self.toasts {
                    Frame::new()
                        .fill(if t.error { p.danger } else { p.accent })
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(egui::Margin::symmetric(16, 10))
                        .show(ui, |ui| {
                            ui.set_max_width(520.0);
                            let fg = if t.error { egui::Color32::WHITE } else { p.on_accent };
                            ui.label(egui::RichText::new(&t.text).color(fg).font(theme::font(14.0)));
                        });
                    ui.add_space(6.0);
                }
            });
    }

    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some(plan) = &mut self.screenshot else { return };
        plan.frames += 1;
        ctx.request_repaint();
        if plan.frames == 3
            && let Some(prompt) = plan.autosend.take()
        {
            if let Some(agents::Page::Thread(t)) = self.page.clone() {
                self.fleet.user_post(&t, &prompt);
            } else {
                self.composer.text = prompt;
                self.composer.think = self.screenshot.as_ref().is_some_and(|p| p.view.contains("think"));
                self.send_composer();
            }
            if let Some(plan) = &mut self.screenshot {
                plan.started = std::time::Instant::now();
            }
            return;
        }
        if plan.frames > 12 && plan.started.elapsed() >= plan.delay && !plan.requested {
            plan.requested = true;
            let path = plan.path.clone();
            let ctx2 = ctx.clone();
            ctx.request_screenshot(move |img| {
                let [w, h] = img.size;
                let mut buf = Vec::with_capacity(w * h * 4);
                for px in &img.pixels {
                    buf.extend_from_slice(&px.to_array());
                }
                if let Some(rgba) = image::RgbaImage::from_raw(w as u32, h as u32, buf)
                    && let Err(e) = rgba.save(&path)
                {
                    eprintln!("screenshot failed: {e}");
                }
                ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
            });
        }
    }
}
