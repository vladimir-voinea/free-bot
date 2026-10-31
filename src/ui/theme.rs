//! Colors, fonts and widget styling.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};

pub const CONTENT_WIDTH: f32 = 760.0;
pub const SIDEBAR_WIDTH: f32 = 264.0;
pub const RAIL_WIDTH: f32 = 60.0;

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn font_bold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

/// Colors used across the app, picked per light/dark mode.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub border: Color32,
    pub bubble: Color32,
    pub text: Color32,
    /// Emphasized text (Markdown bold).
    pub strong: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub code_bg: Color32,
    pub overlay: Color32,
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(0x13, 0x13, 0x14),
            sidebar: Color32::from_rgb(0x19, 0x19, 0x1a),
            surface: Color32::from_rgb(0x20, 0x20, 0x22),
            surface_hover: Color32::from_rgb(0x28, 0x28, 0x2b),
            border: Color32::from_rgb(0x33, 0x33, 0x36),
            bubble: Color32::from_rgb(0x26, 0x26, 0x29),
            text: Color32::from_rgb(0xe3, 0xe3, 0xe6),
            strong: Color32::from_rgb(0xff, 0xff, 0xff),
            muted: Color32::from_rgb(0xa0, 0xa0, 0xa6),
            faint: Color32::from_rgb(0x6c, 0x6c, 0x72),
            hover: Color32::from_rgb(0x26, 0x26, 0x29),
            selected: Color32::from_rgb(0x2c, 0x2c, 0x30),
            accent: Color32::from_rgb(0xf4, 0xf4, 0xf5),
            on_accent: Color32::from_rgb(0x11, 0x12, 0x14),
            danger: Color32::from_rgb(0xf2, 0x6d, 0x6d),
            code_bg: Color32::from_rgb(0x1b, 0x1b, 0x1d),
            overlay: Color32::from_black_alpha(150),
        }
    }

    pub fn light() -> Self {
        Self {
            bg: Color32::from_rgb(0xff, 0xff, 0xff),
            sidebar: Color32::from_rgb(0xf7, 0xf7, 0xf8),
            surface: Color32::from_rgb(0xff, 0xff, 0xff),
            surface_hover: Color32::from_rgb(0xf3, 0xf3, 0xf4),
            border: Color32::from_rgb(0xe2, 0xe2, 0xe5),
            bubble: Color32::from_rgb(0xf0, 0xf0, 0xf2),
            text: Color32::from_rgb(0x22, 0x22, 0x26),
            strong: Color32::from_rgb(0x00, 0x00, 0x00),
            muted: Color32::from_rgb(0x5f, 0x5f, 0x66),
            faint: Color32::from_rgb(0x9a, 0x9a, 0xa1),
            hover: Color32::from_rgb(0xee, 0xee, 0xf0),
            selected: Color32::from_rgb(0xe8, 0xe8, 0xeb),
            accent: Color32::from_rgb(0x14, 0x14, 0x16),
            on_accent: Color32::from_rgb(0xff, 0xff, 0xff),
            danger: Color32::from_rgb(0xd0, 0x36, 0x36),
            code_bg: Color32::from_rgb(0xf6, 0xf6, 0xf7),
            overlay: Color32::from_black_alpha(90),
        }
    }

    pub fn of(ui: &egui::Ui) -> Self {
        Self::for_dark(ui.visuals().dark_mode)
    }

    pub fn for_dark(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.otf"))));
    fonts
        .font_data
        .insert("inter-semibold".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-SemiBold.otf"))));
    fonts
        .font_data
        .insert("jetbrains-mono".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"))));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "jetbrains-mono".into());
    let mut bold = vec!["inter-semibold".to_string()];
    bold.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(semibold(), bold);
    ctx.set_fonts(fonts);
}

pub fn style_visuals(dark: bool) -> Visuals {
    let p = Palette::for_dark(dark);
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.code_bg;
    v.faint_bg_color = p.surface_hover;
    v.code_bg_color = p.code_bg;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(12);
    v.override_text_color = None;
    v.hyperlink_color = if dark { Color32::from_rgb(0x8a, 0xb4, 0xff) } else { Color32::from_rgb(0x1a, 0x5f, 0xd6) };
    v.selection.bg_fill = if dark { Color32::from_rgb(0x3a, 0x4a, 0x6b) } else { Color32::from_rgb(0xc8, 0xd8, 0xf5) };
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.text_cursor.stroke = Stroke::new(2.0, p.text);

    let radius = CornerRadius::same(10);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = radius;
        w.fg_stroke = Stroke::new(1.0, p.text);
    }
    // Markdown **bold** uses the "strong" color, so make it stand out from body text.
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.strong);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.bg_fill = p.surface;
    v.widgets.noninteractive.weak_bg_fill = p.surface;
    v.widgets.inactive.bg_fill = p.surface_hover;
    v.widgets.inactive.weak_bg_fill = p.surface_hover;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_fill = p.selected;
    v.widgets.hovered.weak_bg_fill = p.selected;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.active.bg_fill = p.selected;
    v.widgets.active.weak_bg_fill = p.selected;
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.open.bg_fill = p.selected;
    v.widgets.open.weak_bg_fill = p.selected;
    v
}

pub fn apply_style(ctx: &egui::Context) {
    for dark in [true, false] {
        let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
        ctx.style_mut_of(theme, |s| {
            s.visuals = style_visuals(dark);
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.button_padding = egui::vec2(10.0, 6.0);
            s.spacing.interact_size.y = 30.0;
            s.spacing.scroll.floating = true;
            s.spacing.scroll.bar_width = 6.0;
            s.text_styles.insert(egui::TextStyle::Body, font(15.0));
            s.text_styles.insert(egui::TextStyle::Button, font(14.0));
            s.text_styles.insert(egui::TextStyle::Small, font(12.0));
            s.text_styles.insert(egui::TextStyle::Heading, font_bold(22.0));
            s.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(13.5));
            s.interaction.selectable_labels = false;
        });
    }
}
