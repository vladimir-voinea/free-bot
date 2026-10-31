//! Search palette and confirmation dialogs.

use egui::{Align2, CornerRadius, Key, Sense, Stroke, Vec2, pos2, vec2};

use super::icons::{self, Icon};
use super::theme::{self, Palette};
use super::widgets;
use crate::app::{Confirm, FreeBotApp};
use crate::store::DateGroup;

struct Hit {
    id: Option<String>,
    title: String,
    snippet: String,
    group: &'static str,
}

impl FreeBotApp {
    pub fn modals_ui(&mut self, ctx: &egui::Context) {
        if self.settings_ui.is_some() {
            self.settings_modal(ctx);
        }
        if self.search.is_some() {
            self.search_modal(ctx);
        }
        if self.confirm.is_some() {
            self.confirm_modal(ctx);
        }
    }

    fn modal_frame(ctx: &egui::Context) -> egui::Frame {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(20))
            .inner_margin(egui::Margin::same(0))
            .shadow(egui::Shadow { offset: [0, 12], blur: 40, spread: 0, color: egui::Color32::from_black_alpha(90) })
    }

    fn search_hits(&self, query: &str) -> Vec<Hit> {
        let q = query.trim().to_lowercase();
        let today = chrono::Local::now().date_naive();
        let mut hits = vec![Hit { id: None, title: "New chat".into(), snippet: String::new(), group: "Actions" }];
        for c in &self.convs {
            let group = DateGroup::of(c.updated, c.pinned, today).label();
            if q.is_empty() {
                let snippet = c.messages.iter().rev().find(|m| !m.text().is_empty()).map(|m| m.text().to_string()).unwrap_or_default();
                hits.push(Hit { id: Some(c.id.clone()), title: c.title.clone(), snippet: one_line(&snippet, 0, 90), group });
                continue;
            }
            if c.title.to_lowercase().contains(&q) {
                let snippet = c.messages.first().map(|m| m.text().to_string()).unwrap_or_default();
                hits.push(Hit { id: Some(c.id.clone()), title: c.title.clone(), snippet: one_line(&snippet, 0, 90), group });
                continue;
            }
            for m in &c.messages {
                let text = m.text();
                let lower = text.to_lowercase();
                if let Some(pos) = lower.find(&q) {
                    // `pos` is a byte index into the lowercased text; map it back by char count.
                    let char_pos = lower[..pos].chars().count();
                    hits.push(Hit {
                        id: Some(c.id.clone()),
                        title: c.title.clone(),
                        snippet: one_line(text, char_pos.saturating_sub(30), 90),
                        group,
                    });
                    break;
                }
            }
        }
        if !q.is_empty() && hits.len() > 1 {
            // When searching, matches come first and the action last.
            let action = hits.remove(0);
            hits.push(action);
        }
        hits
    }

    fn search_modal(&mut self, ctx: &egui::Context) {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        let query = self.search.as_ref().map(|s| s.query.clone()).unwrap_or_default();
        let hits = self.search_hits(&query);
        let mut open: Option<Option<String>> = None;

        let modal =
            egui::Modal::new(egui::Id::unique("search-modal")).frame(Self::modal_frame(ctx)).backdrop_color(p.overlay).show(ctx, |ui| {
                ui.set_width(600.0f32.min(ctx.content_rect().width() - 40.0));
                let state = self.search.as_mut().unwrap();
                let (up, down, enter) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(egui::Modifiers::NONE, Key::Enter),
                    )
                });
                if up {
                    state.selected = state.selected.saturating_sub(1);
                }
                if down {
                    state.selected = (state.selected + 1).min(hits.len().saturating_sub(1));
                }
                state.selected = state.selected.min(hits.len().saturating_sub(1));

                egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 14, top: 14, bottom: 12 }).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                        icons::paint(ui.painter(), r, Icon::Search, p.muted);
                        ui.add_space(4.0);
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut state.query)
                                .hint_text("Search chats…")
                                .font(theme::font(16.0))
                                .frame(egui::Frame::NONE)
                                .margin(egui::Margin::ZERO)
                                .desired_width(ui.available_width() - 40.0),
                        );
                        if state.just_opened {
                            state.just_opened = false;
                            resp.request_focus();
                        }
                        if resp.changed() {
                            state.selected = 0;
                        }
                        if widgets::icon_button_sized(ui, Icon::Close, "Close", 30.0, None).clicked() {
                            open = Some(Some("__close__".into()));
                        }
                    });
                });
                let r = ui.available_rect_before_wrap();
                ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, p.border));

                let selected = state.selected;
                // Size the list from last frame's content height; modals do not give
                // scroll areas a stable height on their own.
                let height_id = egui::Id::unique("search-list-height");
                let content_h = ui.ctx().data(|d| d.get_temp::<f32>(height_id)).unwrap_or(120.0);
                let list_h = content_h.clamp(60.0, 420.0);
                let (list_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), list_h), Sense::hover());
                let mut list_ui =
                    ui.new_child(egui::UiBuilder::new().max_rect(list_rect).layout(egui::Layout::top_down(egui::Align::LEFT)));
                let list_out = egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut list_ui, |ui| {
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                        let mut last_group = "";
                        if hits.len() == 1 && !query.trim().is_empty() {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.label(egui::RichText::new("No chats found.").color(p.muted).font(theme::font(14.0)));
                            });
                            ui.add_space(6.0);
                        }
                        for (i, h) in hits.iter().enumerate() {
                            if h.group != last_group {
                                last_group = h.group;
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(10.0);
                                    ui.label(egui::RichText::new(h.group).font(theme::font(12.0)).color(p.faint));
                                });
                            }
                            let height = if h.snippet.is_empty() { 40.0 } else { 56.0 };
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
                            if i == selected || resp.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(10), p.hover);
                            }
                            if i == selected && (up || down) {
                                resp.scroll_to_me(None);
                            }
                            let icon = if h.id.is_none() { Icon::Compose } else { Icon::Chat };
                            icons::paint(
                                ui.painter(),
                                egui::Rect::from_center_size(pos2(rect.left() + 20.0, rect.top() + 20.0), Vec2::splat(18.0)),
                                icon,
                                p.muted,
                            );
                            let font = theme::font(14.5);
                            let t = widgets::truncate_to_width(ui, &h.title, &font, rect.width() - 60.0);
                            ui.painter().text(pos2(rect.left() + 40.0, rect.top() + 20.0), Align2::LEFT_CENTER, t, font, p.text);
                            if !h.snippet.is_empty() {
                                let sf = theme::font(12.5);
                                let s = widgets::truncate_to_width(ui, &h.snippet, &sf, rect.width() - 60.0);
                                ui.painter().text(pos2(rect.left() + 40.0, rect.top() + 39.0), Align2::LEFT_CENTER, s, sf, p.muted);
                            }
                            if resp.clicked() {
                                open = Some(h.id.clone());
                            }
                        }
                    });
                });
                ui.ctx().data_mut(|d| d.insert_temp(height_id, list_out.content_size.y));
                if enter && let Some(h) = hits.get(selected) {
                    open = Some(h.id.clone());
                }
            });

        if modal.should_close() {
            self.search = None;
        }
        match open {
            Some(Some(id)) if id == "__close__" => self.search = None,
            Some(Some(id)) => {
                self.search = None;
                self.open_conv(id);
            }
            Some(None) => {
                self.search = None;
                self.new_chat();
            }
            None => {}
        }
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        let Some(Confirm::DeleteChat(id)) = &self.confirm else { return };
        let id = id.clone();
        let title = self.conv(&id).map(|c| c.title.clone()).unwrap_or_default();
        let mut result = None;
        let modal =
            egui::Modal::new(egui::Id::unique("confirm-modal")).frame(Self::modal_frame(ctx)).backdrop_color(p.overlay).show(ctx, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::same(22)).show(ui, |ui| {
                    ui.set_width(400.0);
                    ui.label(egui::RichText::new("Delete chat?").font(theme::font_bold(18.0)).color(p.text));
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("This will permanently delete \"{title}\"."))
                                .color(p.muted)
                                .font(theme::font(14.0)),
                        )
                        .wrap(),
                    );
                    ui.add_space(18.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::danger_button(ui, "Delete").clicked() {
                            result = Some(true);
                        }
                        if widgets::secondary_button(ui, "Cancel").clicked() {
                            result = Some(false);
                        }
                    });
                });
            });
        if modal.should_close() {
            result = result.or(Some(false));
        }
        match result {
            Some(true) => {
                self.confirm = None;
                self.delete_conv(&id);
                self.toast("Chat deleted");
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }
}

/// A single-line excerpt of `text` starting near char `start`, at most `len` chars.
fn one_line(text: &str, start: usize, len: usize) -> String {
    let plain: String = text.chars().filter(|c| !matches!(c, '*' | '`' | '#' | '_' | '>')).collect();
    let flat: String = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = flat.chars().collect();
    let start = start.min(chars.len());
    let end = (start + len).min(chars.len());
    let mut s: String = chars[start..end].iter().collect();
    if start > 0 {
        s.insert(0, '…');
    }
    if end < chars.len() {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::one_line;

    #[test]
    fn excerpts() {
        assert_eq!(one_line("hello\n  world", 0, 50), "hello world");
        assert_eq!(one_line("abcdef", 2, 2), "…cd…");
        assert_eq!(one_line("ăîș ţâ", 1, 10), "…îș ţâ");
    }
}
