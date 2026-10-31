//! The prompt box: attachments, multi-line input, Think toggle, model picker
//! and the send / stop button.

use egui::{Align2, Color32, CornerRadius, Key, Modifiers, Sense, Stroke, Vec2, pos2, vec2};

use super::icons::{self, Icon};
use super::sidebar::menu_item;
use super::theme::{self, Palette};
use super::widgets;
use crate::app::{FreeBotApp, SettingsTab};
use crate::store::{AttachmentData, ModelRef};

impl FreeBotApp {
    pub fn composer_ui(&mut self, ui: &mut egui::Ui, hint: &str) {
        let p = Palette::of(ui);
        let streaming = self.current.as_deref().is_some_and(|id| self.is_streaming(id));
        let edit_id = egui::Id::unique("composer-input");

        // Enter sends; Shift+Enter inserts a newline. Intercept before the TextEdit sees it.
        let has_focus = ui.ctx().memory(|m| m.has_focus(edit_id));
        let mut send = false;
        if has_focus && self.settings.enter_to_send && self.search.is_none() && self.settings_ui.is_none() {
            // `consume_key` ignores Shift, so check it explicitly to keep Shift+Enter as a newline.
            send = ui.input_mut(|i| !i.modifiers.shift && i.consume_key(Modifiers::NONE, Key::Enter));
        }
        if has_focus && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
            send = true;
        }

        let frame = egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, if has_focus { p.muted.gamma_multiply(0.7) } else { p.border }))
            .corner_radius(CornerRadius::same(28))
            .inner_margin(egui::Margin { left: 14, right: 12, top: 12, bottom: 10 })
            .shadow(egui::Shadow {
                offset: [0, 4],
                blur: 18,
                spread: 0,
                color: Color32::from_black_alpha(if ui.visuals().dark_mode { 60 } else { 18 }),
            });

        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !self.composer.attachments.is_empty() {
                self.attachment_strip(ui);
                ui.add_space(6.0);
            }

            let out = egui::ScrollArea::vertical().id_salt("composer-scroll").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.composer.text)
                        .id(edit_id)
                        .hint_text(egui::RichText::new(hint).color(p.faint))
                        .font(theme::font(16.0))
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::symmetric(4, 6))
                        .desired_rows(1)
                        .desired_width(f32::INFINITY)
                        .lock_focus(true),
                )
            });
            if std::mem::take(&mut self.composer.request_focus) {
                out.inner.request_focus();
            }
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if widgets::round_outline_button(ui, Icon::Paperclip, "Attach images or text files", 34.0).clicked() {
                    self.pick_attachments();
                }
                ui.add_space(2.0);
                let think = self.composer.think;
                if widgets::pill_toggle(ui, Icon::Bulb, "Think", think, "Ask the model to reason step by step before answering").clicked() {
                    self.composer.think = !think;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let can_send = !self.composer.text.trim().is_empty() || !self.composer.attachments.is_empty();
                    if streaming {
                        if send_button(ui, Icon::Stop, true, "Stop").clicked()
                            && let Some(id) = self.current.clone()
                        {
                            self.stop(&id);
                        }
                    } else if send_button(ui, Icon::ArrowUp, can_send, "Send").clicked() && can_send {
                        send = true;
                    }
                    ui.add_space(4.0);
                    self.model_picker(ui);
                });
            });
        });

        if send && !streaming {
            self.send_composer();
            self.composer.request_focus = true;
        }
    }

    fn attachment_strip(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let mut remove = None;
        let atts = self.composer.attachments.clone();
        ui.horizontal_wrapped(|ui| {
            for a in &atts {
                let size = vec2(if a.is_image() { 64.0 } else { 190.0 }, 64.0);
                let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter().rect(rect, CornerRadius::same(12), p.surface_hover, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                match &a.data {
                    AttachmentData::Image { .. } => {
                        if let Some(tex) = self.texture_for(ui.ctx(), a) {
                            let img = egui::Image::new(&tex).fit_to_exact_size(rect.size()).corner_radius(CornerRadius::same(12));
                            img.paint_at(ui, rect);
                        }
                    }
                    AttachmentData::Text { content } => {
                        let icon_rect = egui::Rect::from_min_size(rect.min + vec2(10.0, 14.0), Vec2::splat(36.0));
                        ui.painter().rect_filled(icon_rect, CornerRadius::same(8), p.selected);
                        icons::paint(ui.painter(), icon_rect.shrink(7.0), Icon::File, p.text);
                        let font = theme::font(13.0);
                        let name = widgets::truncate_to_width(ui, &a.name, &font, rect.width() - 64.0);
                        ui.painter().text(pos2(icon_rect.right() + 8.0, rect.top() + 24.0), Align2::LEFT_CENTER, name, font, p.text);
                        ui.painter().text(
                            pos2(icon_rect.right() + 8.0, rect.top() + 42.0),
                            Align2::LEFT_CENTER,
                            format!("{} lines", content.lines().count()),
                            theme::font(12.0),
                            p.muted,
                        );
                    }
                }
                if resp.hovered() || ui.rect_contains_pointer(rect) {
                    let x = egui::Rect::from_center_size(rect.right_top() + vec2(-10.0, 10.0), Vec2::splat(20.0));
                    let xr = ui.interact(x, ui.scope_id().with(("rm-att", &a.id)), Sense::click());
                    ui.painter().circle_filled(x.center(), 10.0, p.accent);
                    icons::paint(ui.painter(), x.shrink(5.0), Icon::Close, p.on_accent);
                    if xr.on_hover_text("Remove").clicked() {
                        remove = Some(a.id.clone());
                    }
                }
                resp.on_hover_text(&a.name);
            }
        });
        if let Some(id) = remove {
            self.composer.attachments.retain(|a| a.id != id);
        }
    }

    fn model_picker(&mut self, ui: &mut egui::Ui) {
        let current = self.settings.effective_model();
        let label = current.as_ref().map(|m| m.model.clone()).unwrap_or_else(|| "Add a model".into());
        let resp = widgets::pill_button(ui, &label, None, true, false).on_hover_text("Choose a model");
        if current.is_none() && resp.clicked() {
            self.open_settings(SettingsTab::Providers);
            return;
        }
        let models = self.settings.all_models();
        let multi_provider = self.settings.providers.iter().filter(|p| !p.models.is_empty()).count() > 1;
        let mut picked: Option<ModelRef> = None;
        let mut manage = false;
        egui::Popup::menu(&resp).width(300.0).align(egui::RectAlign::TOP_END).show(|ui| {
            let p = Palette::of(ui);
            ui.set_min_width(280.0);
            ui.add_space(2.0);
            ui.label(egui::RichText::new("Model").font(theme::font(12.5)).color(p.faint));
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                let mut last_provider = String::new();
                for (m, provider_name) in &models {
                    if multi_provider && *provider_name != last_provider {
                        last_provider = provider_name.clone();
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(provider_name).font(theme::font(12.0)).color(p.muted));
                    }
                    let selected = current.as_ref() == Some(m);
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    if r.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(8), p.hover);
                    }
                    let font = theme::font(14.0);
                    let name = widgets::truncate_to_width(ui, &m.model, &font, rect.width() - 50.0);
                    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, name, font, p.text);
                    if selected {
                        icons::paint(
                            ui.painter(),
                            egui::Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(18.0)),
                            Icon::Check,
                            p.text,
                        );
                    }
                    if r.clicked() {
                        picked = Some(m.clone());
                    }
                }
            });
            ui.separator();
            if menu_item(ui, Icon::Gear, "Manage providers and models", false).clicked() {
                manage = true;
            }
        });
        if let Some(m) = picked {
            self.settings.selected_model = Some(m);
            self.mark_settings_dirty();
        }
        if manage {
            self.open_settings(SettingsTab::Providers);
        }
    }
}

fn send_button(ui: &mut egui::Ui, icon: Icon, enabled: bool, tooltip: &str) -> egui::Response {
    let p = Palette::of(ui);
    let size = 36.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if enabled { if resp.hovered() { p.accent.gamma_multiply(0.85) } else { p.accent } } else { p.selected };
    let fg = if enabled { p.on_accent } else { p.faint };
    ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    icons::paint(ui.painter(), rect.shrink(9.0), icon, fg);
    resp.on_hover_text(tooltip)
}
