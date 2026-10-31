//! Small reusable widgets.

use egui::{Align2, Color32, CornerRadius, Response, Sense, Stroke, Ui, Vec2, vec2};

use super::icons::{self, Icon};
use super::theme::{self, Palette};

/// A square, borderless icon button with a hover background.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    icon_button_sized(ui, icon, tooltip, 32.0, None)
}

pub fn icon_button_sized(ui: &mut Ui, icon: Icon, tooltip: &str, size: f32, color: Option<Color32>) -> Response {
    let p = Palette::of(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() || resp.is_pointer_button_down_on() {
            ui.painter().rect_filled(rect, CornerRadius::same((size * 0.3) as u8), p.hover);
        }
        let c = color.unwrap_or(if resp.hovered() { p.text } else { p.muted });
        icons::paint(ui.painter(), rect.shrink(size * 0.22), icon, c);
    }
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) }
}

/// A round button with an outline, used in the composer.
pub fn round_outline_button(ui: &mut Ui, icon: Icon, tooltip: &str, size: f32) -> Response {
    let p = Palette::of(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let fill = if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
    ui.painter().circle(rect.center(), size / 2.0 - 0.5, fill, Stroke::new(1.0, p.border));
    icons::paint(ui.painter(), rect.shrink(size * 0.2), icon, p.text);
    resp.on_hover_text(tooltip)
}

/// A pill-shaped toggle such as "Think".
pub fn pill_toggle(ui: &mut Ui, icon: Icon, label: &str, on: bool, tooltip: &str) -> Response {
    let p = Palette::of(ui);
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::font(14.0), p.text);
    let h = 34.0;
    let w = 14.0 + 18.0 + 6.0 + galley.size().x + 14.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let (fill, stroke, fg) = if on {
        (p.selected, Stroke::new(1.0, p.muted), p.text)
    } else if resp.hovered() {
        (p.hover, Stroke::new(1.0, p.border), p.text)
    } else {
        (Color32::TRANSPARENT, Stroke::new(1.0, p.border), p.muted)
    };
    ui.painter().rect(rect, CornerRadius::same((h / 2.0) as u8), fill, stroke, egui::StrokeKind::Inside);
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 14.0 + 9.0, rect.center().y), Vec2::splat(18.0));
    icons::paint(ui.painter(), icon_rect, icon, fg);
    ui.painter().text(egui::pos2(icon_rect.right() + 6.0, rect.center().y), Align2::LEFT_CENTER, label, theme::font(14.0), fg);
    resp.on_hover_text(tooltip)
}

/// A full-width sidebar row with an icon, label and optional shortcut hint.
pub fn nav_row(ui: &mut Ui, icon: Icon, label: &str, hint: &str) -> Response {
    let p = Palette::of(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(10), p.hover);
    }
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(20.0));
    icons::paint(ui.painter(), icon_rect, icon, p.text);
    ui.painter().text(egui::pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, label, theme::font(14.5), p.text);
    if !hint.is_empty() && resp.hovered() {
        ui.painter().text(egui::pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, hint, theme::font(12.0), p.faint);
    }
    resp
}

/// A pill button with text and an optional trailing chevron (model picker, chips).
pub fn pill_button(ui: &mut Ui, label: &str, icon: Option<Icon>, chevron: bool, outlined: bool) -> Response {
    let p = Palette::of(ui);
    let font = theme::font(14.0);
    let max_label = 220.0;
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), p.text);
    let label_w = galley.size().x.min(max_label);
    let h = 34.0;
    let mut w = 14.0 + label_w + 14.0;
    if icon.is_some() {
        w += 24.0;
    }
    if chevron {
        w += 18.0;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let fill = if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
    let stroke = if outlined { Stroke::new(1.0, p.border) } else { Stroke::NONE };
    ui.painter().rect(rect, CornerRadius::same((h / 2.0) as u8), fill, stroke, egui::StrokeKind::Inside);
    let mut x = rect.left() + 14.0;
    if let Some(icon) = icon {
        icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(x + 9.0, rect.center().y), Vec2::splat(18.0)), icon, p.muted);
        x += 24.0;
    }
    let text_rect = egui::Rect::from_min_size(egui::pos2(x, rect.top()), vec2(label_w, h));
    let truncated = truncate_to_width(ui, label, &font, label_w);
    ui.painter().with_clip_rect(text_rect).text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, truncated, font, p.text);
    if chevron {
        icons::paint(
            ui.painter(),
            egui::Rect::from_center_size(egui::pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(16.0)),
            Icon::ChevronDown,
            p.muted,
        );
    }
    resp
}

/// Shorten `text` with an ellipsis so it fits in `width`.
pub fn truncate_to_width(ui: &Ui, text: &str, font: &egui::FontId, width: f32) -> String {
    let measure = |s: &str| ui.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x);
    if measure(text) <= width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if measure(&s) <= width { lo = mid } else { hi = mid - 1 }
    }
    chars[..lo].iter().collect::<String>().trim_end().to_string() + "…"
}

/// Primary (filled) button.
pub fn primary_button(ui: &mut Ui, label: &str) -> Response {
    let p = Palette::of(ui);
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(p.on_accent).font(theme::font_bold(14.0)))
            .fill(p.accent)
            .corner_radius(CornerRadius::same(18))
            .min_size(vec2(0.0, 34.0)),
    )
}

/// Secondary (outlined) button.
pub fn secondary_button(ui: &mut Ui, label: &str) -> Response {
    let p = Palette::of(ui);
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(p.text).font(theme::font(14.0)))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(18))
            .min_size(vec2(0.0, 34.0)),
    )
}

pub fn danger_button(ui: &mut Ui, label: &str) -> Response {
    let p = Palette::of(ui);
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).font(theme::font_bold(14.0)))
            .fill(p.danger)
            .corner_radius(CornerRadius::same(18))
            .min_size(vec2(0.0, 34.0)),
    )
}

/// An on/off switch.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let p = Palette::of(ui);
    let size = vec2(38.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let bg = if *on { p.accent } else { p.border };
    ui.painter().rect_filled(rect, CornerRadius::same(11), bg);
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    let knob = if *on { p.on_accent } else { p.bg };
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 8.0, knob);
    resp
}

/// A labelled settings row with a control on the right.
pub fn setting_row(ui: &mut Ui, title: &str, subtitle: &str, control: impl FnOnce(&mut Ui)) {
    let p = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() - 200.0);
            ui.label(egui::RichText::new(title).font(theme::font(14.5)).color(p.text));
            if !subtitle.is_empty() {
                ui.label(egui::RichText::new(subtitle).font(theme::font(12.5)).color(p.muted));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(6.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, p.border));
    ui.add_space(10.0);
}

/// Frame for a text input field.
pub fn input_frame(ui: &Ui) -> egui::Frame {
    let p = Palette::of(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(10, 2))
}

pub fn text_field(ui: &mut Ui, value: &mut String, hint: &str, password: bool) -> Response {
    input_frame(ui)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text(hint)
                    .password(password)
                    .align(egui::Align2::LEFT_CENTER)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::ZERO)
                    .desired_width(f32::INFINITY),
            )
        })
        .inner
}
