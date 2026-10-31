//! The main area: top bar, home screen and the message list of an open chat.

use egui::{Align, Align2, CornerRadius, Layout, Sense, Stroke, Vec2, pos2, vec2};
use egui_commonmark::CommonMarkViewer;

use super::icons::{self, Icon};
use super::sidebar::menu_item;
use super::theme::{self, CONTENT_WIDTH, Palette};
use super::widgets;
use crate::app::{FreeBotApp, SettingsTab};
use crate::store::{AttachmentData, Message, Role};

const SUGGESTIONS: &[(&str, &str)] = &[
    ("Explain", "Explain how large language models work, in simple terms."),
    ("Write code", "Write a Rust function that checks whether a string is a palindrome, with tests."),
    ("Brainstorm", "Brainstorm 10 creative names for a cozy neighborhood coffee shop."),
    ("Summarize", "Summarize the key ideas of stoic philosophy in five bullet points."),
    ("Plan", "Plan a relaxed 3-day weekend trip to Lisbon with food recommendations."),
];

/// Lay out a centered column of at most `max_width`.
pub fn centered_column<R>(ui: &mut egui::Ui, max_width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let avail = ui.available_width();
    let w = avail.min(max_width);
    let margin = ((avail - w) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(margin);
        ui.vertical(|ui| {
            ui.set_width(w);
            add(ui)
        })
        .inner
    })
    .inner
}

impl FreeBotApp {
    pub fn main_ui(&mut self, ui: &mut egui::Ui) {
        self.top_bar(ui);
        match self.current.clone().filter(|id| self.conv(id).is_some()) {
            None => self.home_ui(ui),
            Some(id) => self.chat_ui(ui, &id),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
        let mut bar =
            ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 10.0))).layout(Layout::left_to_right(Align::Center)));
        let ui = &mut bar;

        if let Some(conv) = self.current_conv() {
            let title = conv.title.clone();
            let private = conv.private;
            if private {
                private_badge(ui);
                ui.add_space(6.0);
            }
            let font = theme::font(15.0);
            let t = widgets::truncate_to_width(ui, &title, &font, (ui.available_width() - 160.0).max(80.0));
            ui.label(egui::RichText::new(t).font(font).color(p.text));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| match self.current.clone() {
            None => {
                let on = self.private_mode;
                let resp =
                    widgets::pill_toggle(ui, Icon::Ghost, "Private", on, "Private chats are not saved and disappear when you leave them");
                if resp.clicked() {
                    self.private_mode = !on;
                }
            }
            Some(id) => {
                if widgets::icon_button(ui, Icon::Compose, "New chat").clicked() {
                    self.new_chat();
                }
                let resp = widgets::icon_button(ui, Icon::Share, "Share");
                let mut action = None;
                egui::Popup::menu(&resp).width(220.0).show(|ui| {
                    ui.set_min_width(210.0);
                    if menu_item(ui, Icon::Copy, "Copy as Markdown", false).clicked() {
                        action = Some(0);
                    }
                    if menu_item(ui, Icon::Download, "Save as Markdown file", false).clicked() {
                        action = Some(1);
                    }
                });
                match action {
                    Some(0) => {
                        if let Some(c) = self.conv(&id) {
                            ui.ctx().copy_text(c.to_markdown());
                            self.toast("Copied chat to clipboard");
                        }
                    }
                    Some(1) => self.export_conv_markdown(&id),
                    _ => {}
                }
            }
        });
    }

    fn home_ui(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let configured = self.settings.effective_model().is_some();
        let block_h = if configured { 330.0 } else { 420.0 };
        let top_space = ((ui.available_height() - block_h) / 2.0 - 40.0).max(12.0);
        ui.add_space(top_space);

        centered_column(ui, CONTENT_WIDTH, |ui| {
            // Brand lockup.
            ui.vertical_centered(|ui| {
                let galley = ui.painter().layout_no_wrap("Free Bot".into(), theme::font_bold(40.0), p.text);
                let total = 52.0 + 14.0 + galley.size().x;
                let (rect, _) = ui.allocate_exact_size(vec2(total, 60.0), Sense::hover());
                let logo = egui::Rect::from_min_size(pos2(rect.left(), rect.center().y - 26.0), Vec2::splat(52.0));
                icons::paint_logo(ui.painter(), logo, p.text, p.bg);
                ui.painter().galley(pos2(logo.right() + 14.0, rect.center().y - galley.size().y / 2.0), galley, p.text);
                ui.add_space(6.0);
                if self.private_mode {
                    ui.label(egui::RichText::new("Private chat · not saved to history").font(theme::font(15.0)).color(p.muted));
                } else {
                    ui.label(egui::RichText::new(greeting(&self.settings.display_name)).font(theme::font(17.0)).color(p.muted));
                }
            });
            ui.add_space(26.0);

            if !configured {
                if setup_card(ui) {
                    self.open_settings(SettingsTab::Providers);
                }
                ui.add_space(16.0);
            }

            self.composer_ui(ui, "What do you want to know?");
            ui.add_space(14.0);

            // Suggestion chips.
            ui.horizontal_wrapped(|ui| {
                let chips_w: f32 = SUGGESTIONS.len() as f32 * 118.0;
                ui.add_space(((ui.available_width() - chips_w) / 2.0).max(0.0));
                for (label, prompt) in SUGGESTIONS {
                    let icon = match *label {
                        "Explain" => Icon::Bulb,
                        "Write code" => Icon::File,
                        "Brainstorm" => Icon::Chat,
                        "Summarize" => Icon::Copy,
                        _ => Icon::Compose,
                    };
                    if widgets::pill_button(ui, label, Some(icon), false, true).clicked() {
                        self.composer.text = prompt.to_string();
                        self.composer.request_focus = true;
                    }
                }
            });
        });
    }

    fn chat_ui(&mut self, ui: &mut egui::Ui, conv_id: &str) {
        let p = Palette::of(ui);
        // Composer docked at the bottom.
        egui::Panel::bottom("composer-dock")
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 16, right: 16, top: 6, bottom: 8 }))
            .show_separator_line(false)
            .resizable(false)
            .show(ui, |ui| {
                centered_column(ui, CONTENT_WIDTH, |ui| {
                    self.composer_ui(ui, "Ask anything");
                    ui.add_space(6.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("Answers come from the model you choose and can be wrong. Check important info.")
                                .font(theme::font(11.5))
                                .color(p.faint),
                        );
                    });
                });
            });

        let scroll_to_bottom = std::mem::take(&mut self.scroll_to_bottom);
        egui::ScrollArea::vertical().id_salt(("chat", conv_id)).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.add_space(8.0);
            centered_column(ui, CONTENT_WIDTH, |ui| {
                let n = self.conv(conv_id).map(|c| c.messages.len()).unwrap_or(0);
                for idx in 0..n {
                    let Some(msg) = self.conv(conv_id).and_then(|c| c.messages.get(idx)).cloned() else { break };
                    let is_last = idx + 1 == n;
                    let top = ui.cursor().top();
                    match msg.role {
                        Role::User => self.user_message(ui, conv_id, idx, &msg),
                        Role::Assistant => self.assistant_message(ui, conv_id, idx, &msg, is_last),
                    }
                    let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
                    if ui.rect_contains_pointer(rect) {
                        self.hovered_msg_next = Some(msg.id.clone());
                    }
                }
                ui.add_space(24.0);
            });
            if scroll_to_bottom {
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }
        });
    }

    fn user_message(&mut self, ui: &mut egui::Ui, conv_id: &str, idx: usize, msg: &Message) {
        let p = Palette::of(ui);
        let hovered = self.hovered_msg.as_deref() == Some(msg.id.as_str());
        ui.add_space(12.0);

        if let Some((eid, text)) = &mut self.editing
            && *eid == msg.id
        {
            let mut action = 0;
            egui::Frame::new().fill(p.bubble).corner_radius(CornerRadius::same(20)).inner_margin(egui::Margin::symmetric(16, 12)).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .font(theme::font(15.5))
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::ZERO)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::primary_button(ui, "Send").clicked() {
                            action = 2;
                        }
                        if widgets::secondary_button(ui, "Cancel").clicked() {
                            action = 1;
                        }
                    });
                },
            );
            match action {
                1 => self.editing = None,
                2 => {
                    let text = text.clone();
                    self.submit_edit(conv_id, idx, text);
                }
                _ => {}
            }
            return;
        }

        // Attachments above the bubble, right-aligned.
        if !msg.attachments.is_empty() {
            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                for a in msg.attachments.iter().rev() {
                    match &a.data {
                        AttachmentData::Image { .. } => {
                            if let Some(tex) = self.texture_for(ui.ctx(), a) {
                                let size = tex.size_vec2();
                                let scale = (180.0 / size.y).min(240.0 / size.x).min(1.0);
                                ui.add(egui::Image::new(&tex).fit_to_exact_size(size * scale).corner_radius(CornerRadius::same(14)))
                                    .on_hover_text(&a.name);
                            }
                        }
                        AttachmentData::Text { .. } => {
                            file_chip(ui, &a.name);
                        }
                    }
                }
            });
            ui.add_space(6.0);
        }

        let text = msg.text().to_string();
        if !text.is_empty() {
            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                let max_w = (ui.available_width() * 0.78).max(200.0);
                egui::Frame::new()
                    .fill(p.bubble)
                    .corner_radius(CornerRadius { nw: 22, ne: 6, sw: 22, se: 22 })
                    .inner_margin(egui::Margin::symmetric(18, 11))
                    .show(ui, |ui| {
                        // Shrink the bubble to the text, up to `max_w`.
                        let natural = text
                            .lines()
                            .map(|l| ui.fonts_mut(|f| f.layout_no_wrap(l.to_string(), theme::font(15.5), p.text).size().x))
                            .fold(0.0f32, f32::max);
                        ui.set_width(natural.min(max_w) + 1.0);
                        ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(&text).font(theme::font(15.5)).color(p.text)).wrap().selectable(true),
                            );
                        });
                    });
            });
        }

        // Hover actions.
        ui.add_space(2.0);
        let streaming = self.is_streaming(conv_id);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.set_height(30.0);
            if hovered {
                if widgets::icon_button_sized(ui, Icon::Copy, "Copy", 28.0, None).clicked() {
                    ui.ctx().copy_text(text.clone());
                    self.toast("Copied");
                }
                if !streaming && widgets::icon_button_sized(ui, Icon::Pencil, "Edit", 28.0, None).clicked() {
                    self.editing = Some((msg.id.clone(), text.clone()));
                }
            }
        });
    }

    fn assistant_message(&mut self, ui: &mut egui::Ui, conv_id: &str, idx: usize, msg: &Message, is_last: bool) {
        let p = Palette::of(ui);
        let v = msg.current().clone();
        let streaming_this = is_last && self.streams.get(conv_id).is_some_and(|s| s.msg_id == msg.id);
        let hovered = self.hovered_msg.as_deref() == Some(msg.id.as_str());
        ui.add_space(8.0);

        // Reasoning ("Thinking…" / "Thought for 6s").
        let thinking_now = streaming_this && v.content.is_empty() && (!v.reasoning.is_empty() || msg_think(self, conv_id, idx));
        if !v.reasoning.is_empty() || thinking_now {
            let expanded = self.expanded_reasoning.contains(&msg.id);
            let label = if streaming_this && v.thinking_secs.is_none() {
                let secs = self.streams.get(conv_id).map(|s| s.started.elapsed().as_secs()).unwrap_or(0);
                format!("Thinking… {secs}s")
            } else {
                match v.thinking_secs {
                    Some(s) if s >= 1.0 => format!("Thought for {}s", s.round() as u64),
                    _ => "Thought for a moment".into(),
                }
            };
            let resp = thinking_header(ui, &label, expanded, streaming_this && v.thinking_secs.is_none());
            if resp.clicked() {
                if expanded {
                    self.expanded_reasoning.remove(&msg.id);
                } else {
                    self.expanded_reasoning.insert(msg.id.clone());
                }
            }
            if expanded && !v.reasoning.is_empty() {
                ui.add_space(4.0);
                let top = ui.cursor().top();
                egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 0, top: 2, bottom: 2 }).show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(v.reasoning.trim()).font(theme::font(14.0)).color(p.muted)).wrap());
                });
                let x = ui.max_rect().left() + 3.0;
                ui.painter().vline(x, top..=ui.cursor().top(), Stroke::new(2.0, p.border));
            }
            ui.add_space(6.0);
        }

        if !v.content.is_empty() {
            let viewer =
                CommonMarkViewer::new().syntax_theme_dark("base16-ocean.dark").syntax_theme_light("InspiredGitHub").indentation_spaces(2);
            ui.scope(|ui| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Body);
                ui.spacing_mut().item_spacing.y = 8.0;
                viewer.show(ui, &mut self.md_cache, &v.content);
            });
        } else if streaming_this && !thinking_now {
            typing_dots(ui);
        }

        if let Some(err) = &v.error {
            ui.add_space(6.0);
            let mut retry = false;
            egui::Frame::new()
                .fill(p.danger.gamma_multiply(0.12))
                .stroke(Stroke::new(1.0, p.danger.gamma_multiply(0.6)))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(err).color(p.text).font(theme::font(14.0))).wrap());
                    });
                    if is_last {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if widgets::secondary_button(ui, "Retry").clicked() {
                                retry = true;
                            }
                            if (err.contains("API key") || err.contains("base URL"))
                                && widgets::secondary_button(ui, "Open settings").clicked()
                            {
                                self.open_settings(SettingsTab::Providers);
                            }
                        });
                    }
                });
            if retry {
                self.regenerate(conv_id, idx);
            }
        }

        // Action row. Always visible on the latest answer, on hover otherwise.
        if !streaming_this && !v.content.is_empty() {
            ui.add_space(2.0);
            let show = is_last || hovered;
            ui.horizontal(|ui| {
                ui.set_height(30.0);
                ui.spacing_mut().item_spacing.x = 2.0;
                if !show {
                    return;
                }
                if msg.variants.len() > 1 {
                    if widgets::icon_button_sized(ui, Icon::ChevronLeft, "Previous answer", 26.0, None).clicked() {
                        self.switch_variant(conv_id, &msg.id, -1);
                    }
                    ui.label(
                        egui::RichText::new(format!("{}/{}", msg.active + 1, msg.variants.len())).font(theme::font(13.0)).color(p.muted),
                    );
                    if widgets::icon_button_sized(ui, Icon::ChevronRight, "Next answer", 26.0, None).clicked() {
                        self.switch_variant(conv_id, &msg.id, 1);
                    }
                    ui.add_space(4.0);
                }
                if widgets::icon_button_sized(ui, Icon::Refresh, "Regenerate", 30.0, None).clicked() {
                    self.regenerate(conv_id, idx);
                }
                if widgets::icon_button_sized(ui, Icon::Copy, "Copy", 30.0, None).clicked() {
                    ui.ctx().copy_text(v.content.clone());
                    self.toast("Copied");
                }
                let up = v.feedback == Some(true);
                let down = v.feedback == Some(false);
                if widgets::icon_button_sized(ui, Icon::ThumbUp, "Good response", 30.0, up.then_some(p.text)).clicked() {
                    self.set_feedback(conv_id, &msg.id, true);
                }
                if widgets::icon_button_sized(ui, Icon::ThumbDown, "Bad response", 30.0, down.then_some(p.text)).clicked() {
                    self.set_feedback(conv_id, &msg.id, false);
                }
                if self.settings.show_model_name && !v.model.is_empty() {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(&v.model).font(theme::font(12.5)).color(p.faint));
                }
            });
        }
        ui.add_space(6.0);
    }
}

fn msg_think(app: &FreeBotApp, conv_id: &str, idx: usize) -> bool {
    app.conv(conv_id).and_then(|c| c.messages[..idx].iter().rev().find(|m| m.role == Role::User)).is_some_and(|m| m.think)
}

fn greeting(name: &str) -> String {
    use chrono::Timelike;
    let h = chrono::Local::now().hour();
    let part = match h {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    };
    let name = name.trim();
    if name.is_empty() { format!("{part}. How can I help you today?") } else { format!("{part}, {name}. How can I help you today?") }
}

fn private_badge(ui: &mut egui::Ui) {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(84.0, 26.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(13), p.surface_hover, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(16.0)),
        Icon::Ghost,
        p.text,
    );
    ui.painter().text(pos2(rect.left() + 28.0, rect.center().y), Align2::LEFT_CENTER, "Private", theme::font(13.0), p.text);
}

fn file_chip(ui: &mut egui::Ui, name: &str) {
    let p = Palette::of(ui);
    let font = theme::font(13.5);
    let w = (ui.painter().layout_no_wrap(name.to_string(), font.clone(), p.text).size().x + 48.0).min(260.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(12), p.surface, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(18.0)),
        Icon::File,
        p.muted,
    );
    let t = widgets::truncate_to_width(ui, name, &font, w - 44.0);
    ui.painter().text(pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, t, font, p.text);
}

fn thinking_header(ui: &mut egui::Ui, label: &str, expanded: bool, live: bool) -> egui::Response {
    let p = Palette::of(ui);
    let font = theme::font(14.0);
    let gw = ui.painter().layout_no_wrap(label.to_string(), font.clone(), p.text).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(gw + 58.0, 32.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(16), p.hover);
    }
    let color = if live {
        let t = ui.input(|i| i.time) as f32;
        p.muted.gamma_multiply(0.55 + 0.45 * (t * 3.0).sin().abs())
    } else {
        p.muted
    };
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(18.0)),
        Icon::Bulb,
        color,
    );
    ui.painter().text(pos2(rect.left() + 32.0, rect.center().y), Align2::LEFT_CENTER, label, font, color);
    let chev = if expanded { Icon::ChevronDown } else { Icon::ChevronRight };
    icons::paint(ui.painter(), egui::Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), Vec2::splat(14.0)), chev, p.muted);
    resp
}

fn typing_dots(ui: &mut egui::Ui) {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(48.0, 24.0), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    for i in 0..3 {
        let phase = (t * 5.0 - i as f32 * 0.7).sin() * 0.5 + 0.5;
        ui.painter().circle_filled(
            pos2(rect.left() + 8.0 + i as f32 * 12.0, rect.center().y - phase * 3.0),
            4.0,
            p.text.gamma_multiply(0.35 + 0.65 * phase),
        );
    }
}

fn setup_card(ui: &mut egui::Ui) -> bool {
    let p = Palette::of(ui);
    let mut clicked = false;
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(18))
        .inner_margin(egui::Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 18.0, p.selected);
                icons::paint(ui.painter(), r.shrink(9.0), Icon::Key, p.text);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 140.0).max(120.0));
                    ui.label(egui::RichText::new("Connect your AI provider").font(theme::font_bold(15.0)).color(p.text));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                "Free Bot works with any OpenAI-compatible API: OpenAI, OpenRouter, Groq, Ollama, LM Studio and more.",
                            )
                            .font(theme::font(13.0))
                            .color(p.muted),
                        )
                        .wrap(),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::primary_button(ui, "Add provider").clicked() {
                        clicked = true;
                    }
                });
            });
        });
    clicked
}
