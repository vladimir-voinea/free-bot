//! Free Bot's own vector icon set, drawn with the egui painter on a 24×24 grid.

use egui::epaint::{CubicBezierShape, PathShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Compose,
    Search,
    Sidebar,
    Paperclip,
    ArrowUp,
    Stop,
    Copy,
    Refresh,
    Pencil,
    ThumbUp,
    ThumbDown,
    Gear,
    Trash,
    Pin,
    Ghost,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Plus,
    Check,
    Close,
    Bulb,
    Share,
    Dots,
    File,
    Sun,
    Moon,
    Monitor,
    Key,
    Download,
    Chat,
    Bot,
    Hash,
    Clock,
    Pulse,
    Book,
    Terminal,
    Globe,
    Folder,
    Users,
    Plug,
    Hand,
    Record,
    Shield,
    Play,
    Swap,
    Bell,
    Brain,
}

/// Maps 24×24 icon coordinates into `rect`.
struct Grid {
    origin: Pos2,
    scale: f32,
}

impl Grid {
    fn new(rect: Rect) -> Self {
        let side = rect.width().min(rect.height());
        let origin = rect.center() - vec2(side, side) / 2.0;
        Self { origin, scale: side / 24.0 }
    }
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.scale
    }
    fn s(&self, v: f32) -> f32 {
        v * self.scale
    }
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let g = Grid::new(rect);
    let w = g.s(1.75).max(1.0);
    let st = Stroke::new(w, color);
    let line = |pts: &[(f32, f32)]| {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| g.p(x, y)).collect();
        painter.add(Shape::line(pts, st));
    };
    let closed = |pts: &[(f32, f32)]| {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| g.p(x, y)).collect();
        painter.add(Shape::closed_line(pts, st));
    };
    let rrect = |x: f32, y: f32, w_: f32, h: f32, r: f32| {
        painter.rect_stroke(Rect::from_min_size(g.p(x, y), vec2(g.s(w_), g.s(h))), g.s(r), st, egui::StrokeKind::Middle);
    };
    let circle = |x: f32, y: f32, r: f32| painter.circle_stroke(g.p(x, y), g.s(r), st);
    let dot = |x: f32, y: f32, r: f32| painter.circle_filled(g.p(x, y), g.s(r), color);
    let curve = |a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| {
        painter.add(CubicBezierShape::from_points_stroke(
            [g.p(a.0, a.1), g.p(b.0, b.1), g.p(c.0, c.1), g.p(d.0, d.1)],
            false,
            Color32::TRANSPARENT,
            PathStroke::new(w, color),
        ));
    };
    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| {
        let n = 24;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let t = from + (to - from) * i as f32 / n as f32;
                g.p(cx + r * t.cos(), cy + r * t.sin())
            })
            .collect();
        painter.add(PathShape::line(pts, st));
    };
    use std::f32::consts::PI;

    match icon {
        Icon::Compose => {
            line(&[(11.0, 4.5), (6.5, 4.5), (4.5, 6.5), (4.5, 17.5), (6.5, 19.5), (17.5, 19.5), (19.5, 17.5), (19.5, 13.0)]);
            line(&[(9.5, 14.5), (10.0, 11.5), (17.5, 4.0), (20.0, 6.5), (12.5, 14.0), (9.5, 14.5)]);
        }
        Icon::Search => {
            circle(10.5, 10.5, 6.0);
            line(&[(15.0, 15.0), (20.0, 20.0)]);
        }
        Icon::Sidebar => {
            rrect(3.5, 4.5, 17.0, 15.0, 3.0);
            line(&[(9.5, 4.5), (9.5, 19.5)]);
        }
        Icon::Paperclip => {
            // A classic paperclip, tilted 45 degrees.
            let (sin, cos) = (-PI / 4.0).sin_cos();
            let rot = |x: f32, y: f32| {
                let (dx, dy) = (x - 12.0, y - 12.0);
                g.p(12.0 + dx * cos - dy * sin, 12.0 + dx * sin + dy * cos)
            };
            let mut pts = Vec::new();
            let arc_pts = |cx: f32, cy: f32, r: f32, from: f32, to: f32, pts: &mut Vec<Pos2>| {
                for i in 0..=16 {
                    let t = from + (to - from) * i as f32 / 16.0;
                    pts.push(rot(cx + r * t.cos(), cy + r * t.sin()));
                }
            };
            pts.push(rot(16.5, 6.0));
            arc_pts(12.0, 17.0, 4.5, 0.0, PI, &mut pts);
            arc_pts(10.75, 5.5, 3.25, PI, 2.0 * PI, &mut pts);
            arc_pts(12.0, 16.0, 2.0, 0.0, PI, &mut pts);
            pts.push(rot(10.0, 8.0));
            painter.add(PathShape::line(pts, st));
        }
        Icon::ArrowUp => {
            line(&[(12.0, 19.0), (12.0, 5.5)]);
            line(&[(6.0, 11.0), (12.0, 5.0), (18.0, 11.0)]);
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_min_max(g.p(7.0, 7.0), g.p(17.0, 17.0)), g.s(2.0), color);
        }
        Icon::Copy => {
            rrect(8.5, 8.5, 11.5, 11.5, 2.5);
            line(&[(15.5, 5.5), (15.0, 4.5), (6.5, 4.5), (4.5, 6.5), (4.5, 15.0), (5.5, 15.5)]);
        }
        Icon::Refresh => {
            arc(12.0, 12.0, 7.0, -PI * 0.35, PI * 1.45);
            let tip = (12.0 + 7.0 * (-PI * 0.35f32).cos(), 12.0 + 7.0 * (-PI * 0.35f32).sin());
            line(&[(tip.0 - 4.0, tip.1 - 0.8), tip, (tip.0 + 0.6, tip.1 - 4.2)]);
        }
        Icon::Pencil => {
            closed(&[(4.5, 19.5), (5.2, 15.5), (15.5, 5.2), (18.8, 8.5), (8.5, 18.8)]);
            line(&[(13.5, 7.2), (16.8, 10.5)]);
        }
        Icon::ThumbUp | Icon::ThumbDown => {
            let flip = icon == Icon::ThumbDown;
            let y = |v: f32| if flip { 24.0 - v } else { v };
            rrect(3.5, if flip { 24.0 - 19.5 } else { 10.5 }, 4.0, 9.0, 1.0);
            line(&[
                (7.5, y(11.0)),
                (11.0, y(4.5)),
                (12.5, y(4.5)),
                (13.5, y(6.0)),
                (13.0, y(10.0)),
                (18.5, y(10.0)),
                (20.0, y(11.5)),
                (18.5, y(18.5)),
                (17.0, y(19.5)),
                (7.5, y(19.5)),
            ]);
        }
        Icon::Gear => {
            let c = g.p(12.0, 12.0);
            let teeth = 8;
            let mut pts = Vec::new();
            for i in 0..teeth * 4 {
                let a = (i as f32 / (teeth * 4) as f32) * 2.0 * PI;
                let r = if (i / 2) % 2 == 0 { 8.0 } else { 6.2 };
                pts.push(c + Vec2::angled(a + PI / (teeth as f32 * 4.0)) * g.s(r));
            }
            painter.add(Shape::closed_line(pts, st));
            circle(12.0, 12.0, 2.6);
        }
        Icon::Trash => {
            line(&[(4.5, 7.0), (19.5, 7.0)]);
            line(&[(9.5, 7.0), (9.5, 4.5), (14.5, 4.5), (14.5, 7.0)]);
            line(&[(6.5, 7.0), (7.5, 19.5), (16.5, 19.5), (17.5, 7.0)]);
            line(&[(10.5, 10.5), (10.5, 16.0)]);
            line(&[(13.5, 10.5), (13.5, 16.0)]);
        }
        Icon::Pin => {
            closed(&[(9.0, 4.5), (15.0, 4.5), (14.0, 6.0), (14.5, 11.0), (17.5, 14.0), (6.5, 14.0), (9.5, 11.0), (10.0, 6.0)]);
            line(&[(12.0, 14.0), (12.0, 20.0)]);
        }
        Icon::Ghost => {
            line(&[(5.5, 19.5), (5.5, 11.0)]);
            arc(12.0, 11.0, 6.5, PI, 2.0 * PI);
            line(&[(18.5, 11.0), (18.5, 19.5), (16.3, 17.8), (14.1, 19.5), (12.0, 17.8), (9.9, 19.5), (7.7, 17.8), (5.5, 19.5)]);
            dot(9.6, 11.0, 1.1);
            dot(14.4, 11.0, 1.1);
        }
        Icon::ChevronDown => line(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)]),
        Icon::ChevronLeft => line(&[(14.5, 6.5), (9.0, 12.0), (14.5, 17.5)]),
        Icon::ChevronRight => line(&[(9.5, 6.5), (15.0, 12.0), (9.5, 17.5)]),
        Icon::Plus => {
            line(&[(12.0, 5.0), (12.0, 19.0)]);
            line(&[(5.0, 12.0), (19.0, 12.0)]);
        }
        Icon::Check => line(&[(5.0, 12.5), (10.0, 17.5), (19.0, 7.0)]),
        Icon::Close => {
            line(&[(6.5, 6.5), (17.5, 17.5)]);
            line(&[(17.5, 6.5), (6.5, 17.5)]);
        }
        Icon::Bulb => {
            arc(12.0, 10.0, 5.5, PI * 0.75, PI * 2.25);
            curve((8.1, 13.9), (9.5, 15.2), (9.5, 15.5), (9.5, 16.5));
            curve((15.9, 13.9), (14.5, 15.2), (14.5, 15.5), (14.5, 16.5));
            line(&[(9.5, 16.5), (14.5, 16.5)]);
            line(&[(10.0, 19.5), (14.0, 19.5)]);
        }
        Icon::Share => {
            line(&[(12.0, 14.5), (12.0, 3.5)]);
            line(&[(8.0, 7.5), (12.0, 3.5), (16.0, 7.5)]);
            line(&[(8.5, 10.5), (5.5, 10.5), (5.5, 20.0), (18.5, 20.0), (18.5, 10.5), (15.5, 10.5)]);
        }
        Icon::Download => {
            line(&[(12.0, 4.0), (12.0, 15.0)]);
            line(&[(7.5, 10.5), (12.0, 15.0), (16.5, 10.5)]);
            line(&[(5.0, 16.0), (5.0, 19.5), (19.0, 19.5), (19.0, 16.0)]);
        }
        Icon::Dots => {
            dot(6.0, 12.0, 1.5);
            dot(12.0, 12.0, 1.5);
            dot(18.0, 12.0, 1.5);
        }
        Icon::File => {
            line(&[
                (13.5, 3.5),
                (6.5, 3.5),
                (5.5, 4.5),
                (5.5, 19.5),
                (6.5, 20.5),
                (17.5, 20.5),
                (18.5, 19.5),
                (18.5, 8.5),
                (13.5, 3.5),
                (13.5, 8.5),
                (18.5, 8.5),
            ]);
        }
        Icon::Sun => {
            circle(12.0, 12.0, 4.0);
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                let d = Vec2::angled(a);
                painter.line_segment([g.p(12.0, 12.0) + d * g.s(6.5), g.p(12.0, 12.0) + d * g.s(8.5)], st);
            }
        }
        Icon::Moon => {
            arc(12.0, 12.0, 7.5, -PI * 0.35, PI * 1.35);
            curve((17.0, 6.3), (12.0, 7.0), (11.5, 14.5), (19.2, 14.2));
        }
        Icon::Monitor => {
            rrect(3.5, 4.5, 17.0, 11.5, 2.0);
            line(&[(12.0, 16.0), (12.0, 19.5)]);
            line(&[(8.0, 19.5), (16.0, 19.5)]);
        }
        Icon::Key => {
            circle(8.0, 15.5, 4.0);
            line(&[(10.8, 12.7), (19.0, 4.5)]);
            line(&[(16.0, 7.5), (18.5, 10.0)]);
            line(&[(13.8, 9.7), (15.8, 11.7)]);
        }
        Icon::Chat => {
            paint_bubble(painter, &g, st, None);
        }
        Icon::Bot => {
            rrect(5.0, 8.0, 14.0, 11.0, 3.5);
            line(&[(12.0, 8.0), (12.0, 4.5)]);
            dot(12.0, 4.0, 1.3);
            dot(9.3, 13.5, 1.3);
            dot(14.7, 13.5, 1.3);
            line(&[(3.0, 12.0), (3.0, 15.0)]);
            line(&[(21.0, 12.0), (21.0, 15.0)]);
        }
        Icon::Hash => {
            line(&[(9.5, 4.5), (7.5, 19.5)]);
            line(&[(16.5, 4.5), (14.5, 19.5)]);
            line(&[(5.0, 9.5), (19.5, 9.5)]);
            line(&[(4.5, 14.5), (19.0, 14.5)]);
        }
        Icon::Clock => {
            circle(12.0, 12.0, 8.0);
            line(&[(12.0, 7.5), (12.0, 12.0), (15.0, 14.0)]);
        }
        Icon::Pulse => {
            line(&[(3.0, 12.5), (7.5, 12.5), (10.0, 6.0), (14.0, 18.5), (16.5, 12.5), (21.0, 12.5)]);
        }
        Icon::Book => {
            line(&[(12.0, 6.5), (12.0, 19.5)]);
            closed(&[
                (12.0, 6.5),
                (8.0, 5.0),
                (3.5, 5.0),
                (3.5, 18.0),
                (8.0, 18.0),
                (12.0, 19.5),
                (16.0, 18.0),
                (20.5, 18.0),
                (20.5, 5.0),
                (16.0, 5.0),
            ]);
        }
        Icon::Terminal => {
            rrect(3.5, 4.5, 17.0, 15.0, 2.5);
            line(&[(7.5, 9.5), (10.5, 12.0), (7.5, 14.5)]);
            line(&[(12.5, 15.0), (16.5, 15.0)]);
        }
        Icon::Globe => {
            circle(12.0, 12.0, 8.5);
            line(&[(3.5, 12.0), (20.5, 12.0)]);
            curve((12.0, 3.5), (8.0, 7.0), (8.0, 17.0), (12.0, 20.5));
            curve((12.0, 3.5), (16.0, 7.0), (16.0, 17.0), (12.0, 20.5));
        }
        Icon::Folder => {
            closed(&[(3.5, 6.5), (9.5, 6.5), (11.5, 8.5), (20.5, 8.5), (20.5, 18.5), (3.5, 18.5)]);
        }
        Icon::Users => {
            circle(9.0, 8.5, 3.2);
            arc(9.0, 19.5, 6.0, PI, 2.0 * PI);
            circle(16.5, 9.0, 2.6);
            arc(16.5, 19.0, 4.8, 1.25 * PI, 2.0 * PI);
        }
        Icon::Plug => {
            line(&[(9.0, 3.5), (9.0, 7.5)]);
            line(&[(15.0, 3.5), (15.0, 7.5)]);
            closed(&[(6.0, 7.5), (18.0, 7.5), (18.0, 11.0), (14.5, 15.0), (9.5, 15.0), (6.0, 11.0)]);
            line(&[(12.0, 15.0), (12.0, 20.5)]);
        }
        Icon::Hand => {
            line(&[(8.0, 13.0), (8.0, 6.0)]);
            line(&[(11.0, 12.0), (11.0, 4.5)]);
            line(&[(14.0, 12.0), (14.0, 5.5)]);
            line(&[(17.0, 12.5), (17.0, 8.0)]);
            curve((17.0, 12.5), (17.5, 19.5), (12.0, 21.0), (9.0, 19.0));
            curve((9.0, 19.0), (7.0, 17.5), (5.0, 14.0), (5.0, 12.0));
            line(&[(5.0, 12.0), (8.0, 13.0)]);
        }
        Icon::Record => {
            circle(12.0, 12.0, 8.0);
            dot(12.0, 12.0, 4.0);
        }
        Icon::Shield => {
            closed(&[(12.0, 3.5), (19.5, 6.5), (19.0, 13.0), (12.0, 20.5), (5.0, 13.0), (4.5, 6.5)]);
            line(&[(8.8, 12.0), (11.0, 14.2), (15.5, 9.5)]);
        }
        Icon::Play => {
            closed(&[(8.0, 5.5), (18.5, 12.0), (8.0, 18.5)]);
        }
        Icon::Swap => {
            line(&[(4.5, 9.0), (19.0, 9.0)]);
            line(&[(15.5, 5.5), (19.0, 9.0), (15.5, 12.5)]);
            line(&[(19.5, 15.0), (5.0, 15.0)]);
            line(&[(8.5, 11.5), (5.0, 15.0), (8.5, 18.5)]);
        }
        Icon::Bell => {
            curve((6.0, 16.5), (6.5, 13.0), (6.0, 5.0), (12.0, 5.0));
            curve((12.0, 5.0), (18.0, 5.0), (17.5, 13.0), (18.0, 16.5));
            line(&[(4.5, 16.5), (19.5, 16.5)]);
            line(&[(10.0, 19.5), (14.0, 19.5)]);
        }
        Icon::Brain => {
            circle(9.0, 9.0, 4.0);
            circle(15.0, 9.0, 4.0);
            circle(9.0, 15.0, 4.0);
            circle(15.0, 15.0, 4.0);
        }
    }
}

fn paint_bubble(painter: &Painter, g: &Grid, st: Stroke, fill: Option<Color32>) {
    // Ellipse body with a tail at the lower left, matching the app logo.
    let n = 48;
    let (cx, cy, rx, ry) = (12.0, 11.0, 8.5, 7.0);
    let mut pts: Vec<Pos2> = Vec::new();
    let start = 2.05f32; // angle where the tail joins
    let end = 2.75f32;
    for i in 0..=n {
        let t = end + (2.0 * std::f32::consts::PI - (end - start)) * i as f32 / n as f32;
        pts.push(g.p(cx + rx * t.cos(), cy + ry * t.sin()));
    }
    pts.push(g.p(4.6, 21.0));
    if let Some(fill) = fill {
        painter.add(Shape::Path(PathShape { points: pts.clone(), closed: true, fill, stroke: PathStroke::NONE }));
    }
    if st.width > 0.0 {
        painter.add(Shape::closed_line(pts, st));
    }
}

/// The Free Bot logo mark: a chat bubble with two eyes.
pub fn paint_logo(painter: &Painter, rect: Rect, fg: Color32, bg: Color32) {
    let g = Grid::new(rect);
    paint_bubble(painter, &g, Stroke::NONE, Some(fg));
    for x in [9.3, 14.7] {
        painter.rect_filled(Rect::from_center_size(g.p(x, 11.0), vec2(g.s(2.0), g.s(4.4))), g.s(1.0), bg);
    }
}
