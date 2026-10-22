//! The topology canvas: devices and cables drawn from a lab, with pan, zoom and click-to-open.
//!
//! Transform maths is plain functions (unit tested); drawing uses the egui painter. Shapes
//! differ per device kind and every node carries a text label, so nothing depends on colour.

use egui::{
    Align2, CornerRadius, FontId, Pos2, Rect, Role, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
};
use engine::lab::LabKind;

/// What a click on the canvas does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Click opens a console, drag moves a device, drag the background pans.
    Select,
    /// Click empty space to add a device of this kind.
    Place(LabKind),
    /// Click two devices to cable them.
    Link,
    /// Click a device or a cable to remove it.
    Delete,
}

impl Tool {
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => "click a device to open it · drag to move · drag background to pan",
            Tool::Place(_) => "click empty space to place a device · Esc to stop",
            Tool::Link => "click two devices to cable them · Esc to cancel",
            Tool::Delete => "click a device or a cable to remove it · Esc to stop",
        }
    }
}

/// Everything the canvas draws and how it reacts, for one frame.
#[derive(Debug, Clone, Copy)]
pub struct Scene<'a> {
    pub nodes: &'a [Node],
    pub links: &'a [(usize, usize)],
    pub notes: &'a [(String, [i32; 2])],
    pub tool: Tool,
    /// First device of a cable being drawn.
    pub link_from: Option<usize>,
    pub flights: &'a [FlightDraw<'a>],
}

/// A frame in flight, to draw as a dot between two nodes.
#[derive(Debug, Clone, Copy)]
pub struct FlightDraw<'a> {
    pub from: usize,
    pub to: usize,
    /// 0..=1 along the cable.
    pub t: f32,
    pub label: &'a str,
}

/// Something the user did on the canvas, for the app to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasEvent {
    Open(usize),
    Moved(usize, [i32; 2]),
    Place([i32; 2]),
    LinkEnd(usize),
    DeleteNode(usize),
    DeleteLink(usize),
}

/// Node size in logical units, before zoom.
const NODE: Vec2 = Vec2::new(64.0, 44.0);
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;

/// Pan and zoom. Screen = origin + pan + world * zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub pan: Vec2,
    pub zoom: f32,
    /// False until the first frame has fitted the lab to the available space.
    pub fitted: bool,
}

impl Default for View {
    fn default() -> Self {
        View {
            pan: Vec2::ZERO,
            zoom: 1.0,
            fitted: false,
        }
    }
}

impl View {
    pub fn to_screen(self, origin: Pos2, p: [i32; 2]) -> Pos2 {
        origin + self.pan + Vec2::new(p[0] as f32, p[1] as f32) * self.zoom
    }

    /// Inverse of [`View::to_screen`], in fractional world units.
    pub fn to_world(self, origin: Pos2, screen: Pos2) -> Vec2 {
        (screen - origin - self.pan) / self.zoom
    }

    /// Scale by `factor` keeping the world point under `anchor` (a screen position) fixed.
    pub fn zoom_about(&mut self, origin: Pos2, anchor: Pos2, factor: f32) {
        let z = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let f = z / self.zoom;
        // anchor - origin - pan scales by f around the anchor.
        let a = anchor - origin;
        self.pan = a - (a - self.pan) * f;
        self.zoom = z;
    }

    /// Fit `points` (plus node size and margin) into a viewport of `size`, centred.
    pub fn fit(points: &[[i32; 2]], size: Vec2) -> View {
        if points.is_empty() || size.x < 1.0 || size.y < 1.0 {
            return View {
                fitted: true,
                ..View::default()
            };
        }
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in points {
            let v = Vec2::new(p[0] as f32, p[1] as f32);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        let margin = 24.0;
        let extent = (hi - lo) + NODE + Vec2::splat(2.0 * margin);
        let zoom = (size.x / extent.x)
            .min(size.y / extent.y)
            .clamp(MIN_ZOOM, 1.5);
        let content_centre = (lo + hi) * 0.5;
        let pan = size * 0.5 - content_centre * zoom;
        View {
            pan,
            zoom,
            fitted: true,
        }
    }
}

fn node_rect(view: &View, origin: Pos2, pos: [i32; 2]) -> Rect {
    Rect::from_center_size(view.to_screen(origin, pos), NODE * view.zoom)
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub kind: LabKind,
    pub pos: [i32; 2],
    pub selected: bool,
    /// Whether clicking opens a terminal (switches have no console yet).
    pub has_console: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Canvas {
    pub view: View,
    /// While dragging a node: pointer offset from the node's centre, in world units.
    grab: Option<Vec2>,
    /// Screen-space centres from the last frame, for tests and keyboard hints.
    pub last_centres: Vec<Pos2>,
}

impl Canvas {
    /// Draw and handle input. Returns what the user did, in order.
    pub fn show(&mut self, ui: &mut Ui, scene: &Scene) -> Vec<CanvasEvent> {
        let Scene {
            nodes,
            links,
            notes,
            tool,
            link_from,
            flights,
        } = *scene;
        let mut events = Vec::new();
        let (rect, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let origin = rect.min;
        if !self.view.fitted {
            let pts: Vec<[i32; 2]> = nodes.iter().map(|n| n.pos).collect();
            self.view = View::fit(&pts, rect.size());
        }
        // Pan by dragging the background; double-click refits.
        if bg.dragged() {
            self.view.pan += bg.drag_delta();
        }
        if bg.double_clicked() {
            self.view.fitted = false;
        }
        // Wheel / pinch zoom about the pointer.
        if let Some(ptr) = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| rect.contains(*p))
        {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch * (scroll * 0.0015).exp();
            if (factor - 1.0).abs() > 1e-4 {
                self.view.zoom_about(origin, ptr, factor);
            }
        }

        let painter = ui.painter_at(rect);
        let v = ui.visuals().clone();
        painter.rect_filled(rect, CornerRadius::ZERO, v.extreme_bg_color);

        self.last_centres = nodes
            .iter()
            .map(|n| self.view.to_screen(origin, n.pos))
            .collect();
        let link_stroke = Stroke::new(
            (2.0 * self.view.zoom).clamp(1.0, 4.0),
            v.widgets.noninteractive.fg_stroke.color,
        );
        for &(a, b) in links {
            painter.line_segment([self.last_centres[a], self.last_centres[b]], link_stroke);
        }
        // Rubber band from the first device of a cable being drawn.
        if let (Some(a), Some(ptr)) = (link_from, ui.input(|i| i.pointer.hover_pos())) {
            if let Some(&from) = self.last_centres.get(a) {
                painter.line_segment([from, ptr], Stroke::new(2.0, v.selection.stroke.color));
            }
        }
        // Background clicks: place a device, or remove the cable under the pointer.
        if bg.clicked() {
            if let Some(p) = bg.interact_pointer_pos() {
                match tool {
                    Tool::Place(_) => {
                        let w = self.view.to_world(origin, p);
                        events.push(CanvasEvent::Place([w.x.round() as i32, w.y.round() as i32]));
                    }
                    Tool::Delete => {
                        let hit = links.iter().position(|&(a, b)| {
                            dist_to_segment(p, self.last_centres[a], self.last_centres[b]) < 7.0
                        });
                        if let Some(k) = hit {
                            events.push(CanvasEvent::DeleteLink(k));
                        }
                    }
                    _ => {}
                }
            }
        }
        for (text, pos) in notes {
            painter.text(
                self.view.to_screen(origin, *pos),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional((13.0 * self.view.zoom).clamp(9.0, 22.0)),
                v.weak_text_color(),
            );
        }

        for (i, n) in nodes.iter().enumerate() {
            let r = node_rect(&self.view, origin, n.pos);
            let id = ui.make_persistent_id(("node", i));
            let resp = ui.interact(r, id, Sense::click_and_drag());
            let role = if n.has_console {
                "Open terminal"
            } else {
                "No console yet"
            };
            resp.widget_info(|| {
                WidgetInfo::labeled(
                    Role::Button,
                    true,
                    format!("{}, {}. {role}", n.name, kind_name(n.kind)),
                )
            });
            let resp = resp.on_hover_text(format!("{} ({})", n.name, kind_name(n.kind)));
            match tool {
                Tool::Select => {
                    if resp.clicked() {
                        events.push(CanvasEvent::Open(i));
                    }
                    let ptr = resp.interact_pointer_pos();
                    if resp.drag_started() {
                        // Measure from where the button went down, not from where egui
                        // noticed a drag, or the node would lag the cursor by the drag threshold.
                        let press = ui.input(|i| i.pointer.press_origin()).or(ptr);
                        self.grab = press.map(|p| {
                            self.view.to_world(origin, p)
                                - Vec2::new(n.pos[0] as f32, n.pos[1] as f32)
                        });
                    }
                    if resp.dragged() {
                        if let (Some(p), Some(g)) = (ptr, self.grab) {
                            let w = self.view.to_world(origin, p) - g;
                            let pos = [w.x.round() as i32, w.y.round() as i32];
                            if pos != n.pos {
                                events.push(CanvasEvent::Moved(i, pos));
                            }
                        }
                    }
                    if resp.drag_stopped() {
                        self.grab = None;
                    }
                }
                Tool::Link if resp.clicked() => events.push(CanvasEvent::LinkEnd(i)),
                Tool::Delete if resp.clicked() => events.push(CanvasEvent::DeleteNode(i)),
                _ => {}
            }
            draw_node(
                &painter,
                ui,
                r,
                n,
                resp.hovered() || resp.has_focus(),
                self.view.zoom,
            );
        }
        // Frames in flight: a dot with a caption, so meaning never rests on colour alone.
        for f in flights {
            let (Some(&a), Some(&b)) = (self.last_centres.get(f.from), self.last_centres.get(f.to))
            else {
                continue;
            };
            let at = a + (b - a) * f.t;
            let radius = (6.0 * self.view.zoom).clamp(4.0, 10.0);
            painter.circle_filled(at, radius, v.selection.bg_fill);
            painter.circle_stroke(at, radius, Stroke::new(1.5, v.selection.stroke.color));
            painter.text(
                at + Vec2::new(0.0, -radius - 3.0),
                Align2::CENTER_BOTTOM,
                f.label,
                FontId::proportional((11.0 * self.view.zoom).clamp(9.0, 18.0)),
                v.text_color(),
            );
        }
        events
    }
}

/// Distance from `p` to the segment `a`-`b`, in screen pixels.
fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 == 0.0 {
        0.0
    } else {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    };
    (p - (a + ab * t)).length()
}

fn kind_name(k: LabKind) -> &'static str {
    match k {
        LabKind::Router => "router",
        LabKind::Switch => "switch",
        LabKind::Host => "PC",
    }
}

fn draw_node(painter: &egui::Painter, ui: &Ui, r: Rect, n: &Node, emphasised: bool, zoom: f32) {
    let v = ui.visuals();
    let fill = if n.has_console {
        v.widgets.inactive.bg_fill
    } else {
        v.faint_bg_color
    };
    let ink = if n.has_console {
        v.text_color()
    } else {
        v.weak_text_color()
    };
    let outline = if n.selected {
        Stroke::new(3.0, v.selection.stroke.color)
    } else if emphasised {
        Stroke::new(2.0, v.widgets.hovered.fg_stroke.color)
    } else {
        Stroke::new(1.5, ink)
    };
    // Shape carries the device kind: circle router, wide box switch, box with stand for a PC.
    match n.kind {
        LabKind::Router => {
            let radius = r.height() * 0.5;
            painter.circle_filled(r.center(), radius, fill);
            painter.circle_stroke(r.center(), radius, outline);
        }
        LabKind::Switch => {
            painter.rect_filled(r, CornerRadius::same(3), fill);
            painter.rect_stroke(r, CornerRadius::same(3), outline, StrokeKind::Inside);
        }
        LabKind::Host => {
            let screen = Rect::from_min_max(
                r.min + Vec2::new(r.width() * 0.12, 0.0),
                r.max - Vec2::new(r.width() * 0.12, r.height() * 0.22),
            );
            painter.rect_filled(screen, CornerRadius::same(4), fill);
            painter.rect_stroke(screen, CornerRadius::same(4), outline, StrokeKind::Inside);
            let stand = Rect::from_center_size(
                Pos2::new(r.center().x, r.max.y - r.height() * 0.08),
                Vec2::new(r.width() * 0.4, r.height() * 0.1),
            );
            painter.rect_filled(stand, CornerRadius::same(2), ink);
        }
    }
    let glyph = match n.kind {
        LabKind::Router => "R",
        LabKind::Switch => "S",
        LabKind::Host => "PC",
    };
    let size = (13.0 * zoom).clamp(8.0, 26.0);
    let glyph_at = if n.kind == LabKind::Host {
        r.center() - Vec2::new(0.0, r.height() * 0.1)
    } else {
        r.center()
    };
    painter.text(
        glyph_at,
        Align2::CENTER_CENTER,
        glyph,
        FontId::monospace(size),
        ink,
    );
    painter.text(
        Pos2::new(r.center().x, r.max.y + 3.0 * zoom),
        Align2::CENTER_TOP,
        &n.name,
        FontId::proportional((12.0 * zoom).clamp(9.0, 22.0)),
        v.text_color(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_puts_every_node_inside_the_viewport() {
        let pts = [[60, 130], [250, 130], [450, 130], [640, 130]];
        let size = Vec2::new(800.0, 240.0);
        let v = View::fit(&pts, size);
        for p in pts {
            let s = v.to_screen(Pos2::ZERO, p);
            let r = Rect::from_center_size(s, NODE * v.zoom);
            assert!(
                Rect::from_min_size(Pos2::ZERO, size).contains_rect(r),
                "{p:?} -> {r:?}"
            );
        }
    }

    #[test]
    fn fit_centres_the_content() {
        let v = View::fit(&[[0, 0], [100, 0]], Vec2::new(400.0, 200.0));
        let mid = v.to_screen(Pos2::ZERO, [50, 0]);
        assert!(
            (mid.x - 200.0).abs() < 0.01 && (mid.y - 100.0).abs() < 0.01,
            "{mid:?}"
        );
    }

    #[test]
    fn fit_handles_degenerate_input() {
        assert!(View::fit(&[], Vec2::new(100.0, 100.0)).fitted);
        assert!(View::fit(&[[5, 5]], Vec2::new(0.0, 0.0)).fitted);
        let one = View::fit(&[[5, 5]], Vec2::new(300.0, 200.0));
        assert!(one.zoom >= MIN_ZOOM && one.zoom <= MAX_ZOOM);
    }

    #[test]
    fn zoom_keeps_the_world_point_under_the_pointer_fixed() {
        let mut v = View {
            pan: Vec2::new(30.0, -10.0),
            zoom: 1.0,
            fitted: true,
        };
        let origin = Pos2::new(10.0, 20.0);
        let world = [123, 45];
        // Pointer is exactly over this world point.
        let anchor = v.to_screen(origin, world);
        v.zoom_about(origin, anchor, 2.0);
        assert_eq!(v.zoom, 2.0);
        let after = v.to_screen(origin, world);
        assert!((after - anchor).length() < 1e-3, "{after:?} vs {anchor:?}");
    }

    #[test]
    fn to_world_inverts_to_screen() {
        let v = View {
            pan: Vec2::new(17.0, -9.0),
            zoom: 1.75,
            fitted: true,
        };
        let origin = Pos2::new(40.0, 25.0);
        let w = v.to_world(origin, v.to_screen(origin, [123, -45]));
        assert!((w - Vec2::new(123.0, -45.0)).length() < 1e-3, "{w:?}");
    }

    #[test]
    fn segment_distance_clamps_to_the_ends() {
        let (a, b) = (Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0));
        assert_eq!(dist_to_segment(Pos2::new(5.0, 3.0), a, b), 3.0);
        assert_eq!(dist_to_segment(Pos2::new(-4.0, 3.0), a, b), 5.0);
        assert_eq!(dist_to_segment(Pos2::new(1.0, 1.0), a, a), 2f32.sqrt());
    }

    #[test]
    fn zoom_is_clamped() {
        let mut v = View::default();
        v.zoom_about(Pos2::ZERO, Pos2::ZERO, 1e6);
        assert_eq!(v.zoom, MAX_ZOOM);
        v.zoom_about(Pos2::ZERO, Pos2::ZERO, 1e-6);
        assert_eq!(v.zoom, MIN_ZOOM);
    }
}
