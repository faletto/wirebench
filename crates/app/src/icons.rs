//! Toolbar and menu icons, drawn with the painter from lines and simple shapes. Own drawings: no
//! icon font is bundled (it would add to the download) and no vendor artwork is used.

use egui::epaint::{CubicBezierShape, PathShape, PathStroke};
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Home,
    Open,
    Save,
    Undo,
    Redo,
    Select,
    Delete,
    Simulation,
    /// The lab running by itself (a clock).
    Realtime,
    Console,
    Settings,
    New,
    Continue,
    Back,
    Step,
    Play,
    Pause,
    Run,
    Clear,
}

/// Draw `icon` filling the square inside `r`.
pub fn paint(p: &Painter, r: Rect, icon: Icon, colour: Color32) {
    let side = r.width().min(r.height());
    let r = Rect::from_center_size(r.center(), Vec2::splat(side));
    let w = (side * 0.09).clamp(1.2, 3.0);
    let s = Stroke::new(w, colour);
    let at = |x: f32, y: f32| r.min + Vec2::new(x, y) * side;
    match icon {
        Icon::Home => {
            poly(p, &[at(0.12, 0.5), at(0.5, 0.14), at(0.88, 0.5)], s, false);
            poly(
                p,
                &[
                    at(0.24, 0.42),
                    at(0.24, 0.86),
                    at(0.76, 0.86),
                    at(0.76, 0.42),
                ],
                s,
                false,
            );
            p.rect_filled(
                Rect::from_min_max(at(0.42, 0.6), at(0.58, 0.86)),
                CornerRadius::ZERO,
                colour,
            );
        }
        Icon::Open => {
            poly(
                p,
                &[
                    at(0.1, 0.82),
                    at(0.1, 0.22),
                    at(0.38, 0.22),
                    at(0.46, 0.32),
                    at(0.84, 0.32),
                    at(0.84, 0.44),
                ],
                s,
                false,
            );
            poly(
                p,
                &[
                    at(0.1, 0.82),
                    at(0.26, 0.46),
                    at(0.94, 0.46),
                    at(0.78, 0.82),
                ],
                s,
                true,
            );
        }
        Icon::Save => {
            poly(
                p,
                &[
                    at(0.14, 0.14),
                    at(0.72, 0.14),
                    at(0.86, 0.28),
                    at(0.86, 0.86),
                    at(0.14, 0.86),
                ],
                s,
                true,
            );
            p.rect_stroke(
                Rect::from_min_max(at(0.3, 0.14), at(0.66, 0.36)),
                CornerRadius::ZERO,
                s,
                StrokeKind::Middle,
            );
            p.rect_stroke(
                Rect::from_min_max(at(0.28, 0.56), at(0.72, 0.86)),
                CornerRadius::ZERO,
                s,
                StrokeKind::Middle,
            );
        }
        Icon::Undo | Icon::Redo => {
            let flip = icon == Icon::Redo;
            let fx = |x: f32| if flip { 1.0 - x } else { x };
            curve(
                p,
                [
                    at(fx(0.2), 0.42),
                    at(fx(0.55), 0.1),
                    at(fx(1.0), 0.45),
                    at(fx(0.6), 0.84),
                ],
                s,
            );
            head(
                p,
                at(fx(0.2), 0.42),
                Vec2::new(if flip { 1.0 } else { -1.0 }, 0.35),
                side * 0.24,
                colour,
            );
        }
        Icon::Select => {
            let pts = [
                at(0.24, 0.1),
                at(0.24, 0.8),
                at(0.42, 0.63),
                at(0.56, 0.92),
                at(0.66, 0.87),
                at(0.53, 0.59),
                at(0.78, 0.58),
            ];
            p.add(Shape::convex_polygon(
                vec![pts[0], pts[1], pts[2], pts[5], pts[6]],
                colour.gamma_multiply(0.35),
                Stroke::NONE,
            ));
            poly(p, &pts, s, true);
        }
        Icon::Delete => {
            p.line_segment([at(0.14, 0.24), at(0.86, 0.24)], s);
            poly(
                p,
                &[
                    at(0.38, 0.24),
                    at(0.42, 0.12),
                    at(0.58, 0.12),
                    at(0.62, 0.24),
                ],
                s,
                false,
            );
            poly(
                p,
                &[at(0.22, 0.24), at(0.28, 0.9), at(0.72, 0.9), at(0.78, 0.24)],
                s,
                false,
            );
            for x in [0.4, 0.6] {
                p.line_segment([at(x, 0.4), at(x, 0.76)], s);
            }
        }
        Icon::Simulation => {
            // A stopwatch.
            let c = at(0.5, 0.56);
            p.circle_stroke(c, side * 0.34, s);
            p.line_segment([at(0.5, 0.22), at(0.5, 0.1)], s);
            p.line_segment([at(0.4, 0.1), at(0.6, 0.1)], s);
            p.line_segment([c, at(0.66, 0.42)], s);
            p.circle_filled(c, w, colour);
        }
        Icon::Realtime => {
            // A clock.
            let c = at(0.5, 0.5);
            p.circle_stroke(c, side * 0.38, s);
            p.line_segment([c, at(0.5, 0.24)], s);
            p.line_segment([c, at(0.7, 0.58)], s);
        }
        Icon::Console => {
            p.rect_stroke(
                Rect::from_min_max(at(0.08, 0.16), at(0.92, 0.84)),
                CornerRadius::same(2),
                s,
                StrokeKind::Middle,
            );
            poly(p, &[at(0.24, 0.36), at(0.4, 0.5), at(0.24, 0.64)], s, false);
            p.line_segment([at(0.48, 0.66), at(0.72, 0.66)], s);
        }
        Icon::Settings => {
            // Three sliders.
            for (y, knob) in [(0.24, 0.34), (0.5, 0.66), (0.76, 0.46)] {
                p.line_segment([at(0.12, y), at(0.88, y)], s);
                p.circle_filled(at(knob, y), side * 0.09, colour);
            }
        }
        Icon::New => {
            p.line_segment([at(0.5, 0.16), at(0.5, 0.84)], Stroke::new(w * 1.4, colour));
            p.line_segment([at(0.16, 0.5), at(0.84, 0.5)], Stroke::new(w * 1.4, colour));
        }
        Icon::Continue => {
            curve(
                p,
                [
                    at(0.82, 0.5),
                    at(0.82, 0.95),
                    at(0.12, 0.95),
                    at(0.16, 0.42),
                ],
                s,
            );
            curve(
                p,
                [at(0.16, 0.42), at(0.22, 0.08), at(0.74, 0.08), at(0.8, 0.3)],
                s,
            );
            head(p, at(0.84, 0.36), Vec2::new(0.3, 1.0), side * 0.24, colour);
        }
        Icon::Back => {
            p.line_segment([at(0.84, 0.5), at(0.2, 0.5)], s);
            head(p, at(0.14, 0.5), Vec2::new(-1.0, 0.0), side * 0.3, colour);
        }
        Icon::Step => {
            p.add(Shape::convex_polygon(
                vec![at(0.2, 0.2), at(0.66, 0.5), at(0.2, 0.8)],
                colour,
                Stroke::NONE,
            ));
            p.rect_filled(
                Rect::from_min_max(at(0.7, 0.2), at(0.82, 0.8)),
                CornerRadius::ZERO,
                colour,
            );
        }
        Icon::Play => {
            p.add(Shape::convex_polygon(
                vec![at(0.24, 0.16), at(0.84, 0.5), at(0.24, 0.84)],
                colour,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            for x in [0.24, 0.58] {
                p.rect_filled(
                    Rect::from_min_max(at(x, 0.18), at(x + 0.18, 0.82)),
                    CornerRadius::ZERO,
                    colour,
                );
            }
        }
        Icon::Run => {
            for x in [0.1, 0.48] {
                p.add(Shape::convex_polygon(
                    vec![at(x, 0.2), at(x + 0.42, 0.5), at(x, 0.8)],
                    colour,
                    Stroke::NONE,
                ));
            }
        }
        Icon::Clear => {
            p.line_segment([at(0.22, 0.22), at(0.78, 0.78)], s);
            p.line_segment([at(0.78, 0.22), at(0.22, 0.78)], s);
        }
    }
}

fn poly(p: &Painter, pts: &[Pos2], s: Stroke, closed: bool) {
    let shape = if closed {
        PathShape::closed_line(pts.to_vec(), s)
    } else {
        PathShape::line(pts.to_vec(), PathStroke::from(s))
    };
    p.add(shape);
}

fn curve(p: &Painter, pts: [Pos2; 4], s: Stroke) {
    p.add(CubicBezierShape::from_points_stroke(
        pts,
        false,
        Color32::TRANSPARENT,
        s,
    ));
}

/// A filled arrowhead with its tip at `tip`, pointing along `dir`.
fn head(p: &Painter, tip: Pos2, dir: Vec2, len: f32, colour: Color32) {
    let d = dir.normalized();
    let n = Vec2::new(-d.y, d.x);
    let back = tip - d * len;
    p.add(Shape::convex_polygon(
        vec![tip, back + n * len * 0.55, back - n * len * 0.55],
        colour,
        Stroke::NONE,
    ));
}
