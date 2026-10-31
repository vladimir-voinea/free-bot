//! UI of the agent layer: agents and channels in the sidebar, the thread
//! view with tool cards and approvals, the agent panel (profile, computer,
//! memory), editors, and the Routines, Activity and Knowledge pages.

pub mod editors;
pub mod pages;
pub mod panel;
pub mod sidebar;
pub mod thread;

use std::collections::{HashMap, HashSet};

use egui::{Align2, Color32, Pos2, Rect, Sense, Shape as EShape, Stroke, Vec2, pos2, vec2};

use super::theme;
use crate::agents::model::{Agent, Connector, Routine, Shape, Skill};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Thread(String),
    Routines,
    Activity,
    Knowledge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelTab {
    Profile,
    Computer,
    Memory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputerTab {
    Browser,
    Files,
    Terminal,
}

pub struct AgentEditor {
    pub agent: Agent,
    pub is_new: bool,
    pub workspace_text: String,
}

pub struct ChannelEditor {
    pub id: Option<String>,
    pub name: String,
    pub members: Vec<String>,
    pub lead: Option<String>,
}

pub struct RoutineEditor {
    pub routine: Routine,
    pub is_new: bool,
    /// 0 every N min, 1 hourly, 2 daily, 3 weekly, 4 monthly, 5 file change, 6 webhook.
    pub kind: usize,
    pub minutes: u32,
    pub hour: u32,
    pub minute: u32,
    pub weekday: u32,
    pub day: u32,
    pub path: String,
    pub token: String,
}

pub struct ConnectorEditor {
    pub connector: Connector,
    pub is_new: bool,
    /// For stdio: the whole command line. For HTTP: the URL.
    pub target: String,
    /// `KEY=value` lines (env for stdio, headers as `Name: value` for HTTP).
    pub extra: String,
    pub http: bool,
}

#[derive(Default)]
pub struct AgentUi {
    pub drafts: HashMap<String, String>,
    pub panel: Option<PanelTab>,
    pub computer_tab: Option<ComputerTab>,
    pub editor: Option<AgentEditor>,
    pub channel_editor: Option<ChannelEditor>,
    pub routine_editor: Option<RoutineEditor>,
    pub connector_editor: Option<ConnectorEditor>,
    pub skill_editor: Option<Skill>,
    pub expanded_calls: HashSet<String>,
    pub expanded_reasoning: HashSet<String>,
    pub browser_tex: HashMap<String, (u64, egui::TextureHandle)>,
    pub url_input: String,
    pub files_dir: String,
    pub preview: Option<(String, String)>,
    pub between_open: bool,
    pub scroll_bottom: bool,
    pub focus_composer: bool,
    pub memory_input: String,
    pub memory_shared: bool,
    pub confirm_delete_agent: Option<String>,
    pub confirm_delete_thread: Option<String>,
    pub window_focused: bool,
    pub browser_has_focus: bool,
}

pub fn agent_color(a: &Agent) -> Color32 {
    Color32::from_rgb(a.color[0], a.color[1], a.color[2])
}

fn shape_points(shape: Shape, c: Pos2, r: f32) -> Vec<Pos2> {
    let poly = |n: usize, rot: f32, rr: &dyn Fn(usize) -> f32| -> Vec<Pos2> {
        (0..n)
            .map(|i| {
                let a = rot + i as f32 * std::f32::consts::TAU / n as f32;
                pos2(c.x + rr(i) * a.cos(), c.y + rr(i) * a.sin())
            })
            .collect()
    };
    match shape {
        Shape::Circle => poly(40, 0.0, &|_| r),
        Shape::Square => {
            // Rounded square approximated with a superellipse.
            (0..48)
                .map(|i| {
                    let t = i as f32 * std::f32::consts::TAU / 48.0;
                    let (s, co) = t.sin_cos();
                    let k = 0.5;
                    pos2(c.x + r * 0.95 * co.signum() * co.abs().powf(k), c.y + r * 0.95 * s.signum() * s.abs().powf(k))
                })
                .collect()
        }
        Shape::Hexagon => poly(6, std::f32::consts::FRAC_PI_6, &|_| r),
        Shape::Diamond => poly(4, 0.0, &|_| r * 1.05),
        Shape::Star => poly(10, -std::f32::consts::FRAC_PI_2, &|i| if i % 2 == 0 { r * 1.05 } else { r * 0.62 }),
    }
}

/// Paint an agent avatar: its shape in its color with the first letter.
pub fn paint_avatar(painter: &egui::Painter, rect: Rect, a: &Agent) {
    paint_avatar_raw(painter, rect, a.shape, agent_color(a), &a.name);
}

pub fn paint_avatar_raw(painter: &egui::Painter, rect: Rect, shape: Shape, color: Color32, name: &str) {
    let r = rect.width().min(rect.height()) / 2.0;
    let pts = shape_points(shape, rect.center(), r);
    painter.add(EShape::convex_polygon(pts.clone(), color, Stroke::NONE));
    if shape == Shape::Star {
        // Stars are concave; fill the middle so the letter stays readable.
        painter.circle_filled(rect.center(), r * 0.62, color);
        for i in 0..5 {
            let tri = vec![rect.center(), pts[(2 * i + 9) % 10], pts[2 * i], pts[(2 * i + 1) % 10]];
            painter.add(EShape::convex_polygon(tri, color, Stroke::NONE));
        }
    }
    let lum = 0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32;
    let fg = if lum > 160.0 { Color32::from_rgb(0x15, 0x15, 0x18) } else { Color32::WHITE };
    let initial = name.trim().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
    painter.text(rect.center() + vec2(0.0, 0.5), Align2::CENTER_CENTER, initial, theme::font_bold(r * 0.95), fg);
}

pub fn avatar(ui: &mut egui::Ui, a: &Agent, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    paint_avatar(ui.painter(), rect, a);
    resp
}

/// Relative time such as "2m", "3h", "Mon".
pub fn short_time(t: chrono::DateTime<chrono::Utc>) -> String {
    let local = t.with_timezone(&chrono::Local);
    let now = chrono::Local::now();
    if local.date_naive() == now.date_naive() {
        local.format("%H:%M").to_string()
    } else if (now - local).num_days() < 7 {
        local.format("%a %H:%M").to_string()
    } else {
        local.format("%b %-d").to_string()
    }
}
