//! The lab workspace around the canvas: the top bar, the component bar along the bottom that
//! devices and cables are dragged out of, the pop-up window a device opens in, and the status
//! line. A child of `app` so it can reach the app's fields.

use super::*;
use crate::canvas::{draw_cable_icon, draw_device_icon, kind_colour, kind_name, PaletteItem};
use crate::chrome::{device_chip, tool_button};
use crate::icons::Icon;
use egui::{Align2, CornerRadius, FontId, Id, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

/// A group of models in the component bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Routers,
    Switches,
    Pcs,
    Cables,
}

impl Category {
    const ALL: [Category; 4] = [
        Category::Routers,
        Category::Switches,
        Category::Pcs,
        Category::Cables,
    ];

    fn label(self) -> &'static str {
        match self {
            Category::Routers => "Routers",
            Category::Switches => "Switches",
            Category::Pcs => "PCs",
            Category::Cables => "Cables",
        }
    }

    fn models(self) -> &'static [Model] {
        match self {
            Category::Routers => ROUTERS,
            Category::Switches => SWITCHES,
            Category::Pcs => PCS,
            Category::Cables => CABLES,
        }
    }
}

/// One thing in the component bar.
pub struct Model {
    pub label: &'static str,
    pub detail: &'static str,
    pub item: PaletteItem,
}

const fn device(
    kind: LabKind,
    template: &'static str,
    label: &'static str,
    detail: &'static str,
) -> Model {
    Model {
        label,
        detail,
        item: PaletteItem::Device { kind, template },
    }
}

const ROUTERS: &[Model] = &[
    device(LabKind::Router, "router-2g", "Router", "2 Gigabit ports"),
    device(LabKind::Router, "router-3g", "Router", "3 Gigabit ports"),
    device(LabKind::Router, "router-4g", "Router", "4 Gigabit ports"),
];
const SWITCHES: &[Model] = &[
    device(LabKind::Switch, "switch-l2-8", "Switch", "8 Fast Ethernet"),
    device(
        LabKind::Switch,
        "switch-l2-24",
        "Switch",
        "24 Fast Ethernet",
    ),
    device(
        LabKind::Switch,
        "switch-l2-48",
        "Switch",
        "48 Fast Ethernet",
    ),
];
const PCS: &[Model] = &[device(LabKind::Host, "host-pc", "PC", "1 Ethernet port")];
const CABLES: &[Model] = &[Model {
    label: "Cable",
    detail: "picks free ports",
    item: PaletteItem::Cable,
}];

/// Which tab the device window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTab {
    Console,
    Settings,
}

const MODEL_CARD: Vec2 = Vec2::new(104.0, 82.0);

impl App {
    // ---- the device window ----

    /// The device whose window is open, if one is.
    pub fn device_window(&self) -> Option<&str> {
        self.window_open
            .then(|| self.tabs.get(self.active).map(|t| t.name.as_str()))
            .flatten()
    }

    pub fn close_device_window(&mut self) {
        self.window_open = false;
    }

    /// Show the Console or Settings tab of the device window.
    pub fn set_device_tab(&mut self, tab: DeviceTab) {
        if tab == DeviceTab::Console && self.device_tab != DeviceTab::Console {
            self.focus_pending = true;
        }
        self.device_tab = tab;
    }

    pub fn device_tab(&self) -> DeviceTab {
        self.device_tab
    }

    /// Open the window of the device on tab `tab`, placed beside the device rather than over it.
    pub(super) fn open_window(&mut self, tab: usize) {
        let was = self.window_open && self.active == tab;
        self.set_active(tab);
        self.window_open = true;
        self.focus_pending = true;
        if was {
            return;
        }
        self.window_model = self.model_text(self.tabs.get(tab).map(|t| t.node));
        let node = self.tabs.get(tab).map(|t| t.node);
        self.window_place = Some(window_spot(
            self.canvas_rect,
            &self.canvas.last_centres,
            node,
        ));
    }

    fn device_window_ui(&mut self, ctx: &egui::Context) {
        if !self.window_open {
            return;
        }
        let Some(t) = self.tabs.get(self.active) else {
            self.window_open = false;
            return;
        };
        let Some(node) = self.nodes.get(t.node) else {
            self.window_open = false;
            return;
        };
        let (name, kind) = (t.name.clone(), node.kind);
        let mut open = true;
        let mut window = egui::Window::new(RichText::new(&name).strong().size(15.0))
            .id(Id::unique("device-window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(window_size(self.canvas_rect));
        if let Some(pos) = self.window_place.take() {
            window = window.current_pos(pos);
        }
        let focus = std::mem::take(&mut self.focus_pending);
        window.show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(40.0, 30.0), Sense::hover());
                draw_device_icon(ui.painter(), r, kind, kind_colour(kind), Stroke::NONE);
                ui.label(RichText::new(&self.window_model).size(15.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (tab, icon, label) in [
                        (DeviceTab::Settings, Icon::Settings, "Settings"),
                        (DeviceTab::Console, Icon::Console, "Console"),
                    ] {
                        if tool_button(ui, icon, label, self.device_tab == tab, true).clicked() {
                            self.set_device_tab(tab);
                        }
                    }
                });
            });
            ui.separator();
            match self.device_tab {
                DeviceTab::Console => {
                    let active = self.active;
                    let Some(t) = self.tabs.get_mut(active) else {
                        return;
                    };
                    let id = ui.make_persistent_id(("terminal", t.device));
                    let prompt = t.prompt.clone();
                    let actions = widget::show(ui, id, &mut t.model, &prompt, focus);
                    // While a question is on screen, keystrokes answer it rather than run commands.
                    if self.confirm.is_none() {
                        for a in actions {
                            self.apply(active, a);
                        }
                    }
                }
                DeviceTab::Settings => self.settings_panel(ui),
            }
        });
        if !open {
            self.window_open = false;
        }
    }

    /// "Router, 3 ports" for the device at node index `node`.
    fn model_text(&self, node: Option<usize>) -> String {
        let Some(n) = node.and_then(|i| self.nodes.get(i)) else {
            return String::new();
        };
        let template = labfile::load(&self.lab_text)
            .ok()
            .and_then(|l| l.lab.devices.get(node?).map(|d| d.template.clone()));
        let kind = kind_name(n.kind);
        let mut kind = kind.to_string();
        if let Some(k) = kind.get_mut(0..1) {
            k.make_ascii_uppercase();
        }
        match template.as_deref().and_then(port_count) {
            Some(1) => format!("{kind}, 1 port"),
            Some(p) => format!("{kind}, {p} ports"),
            None => kind,
        }
    }

    // ---- top bar ----

    pub(super) fn top_bar(&mut self, ui: &mut Ui) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if tool_button(ui, Icon::Home, "Home", false, true)
                    .on_hover_text("Back to the home screen (the lab stays open)")
                    .clicked()
                {
                    self.show_home();
                }
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_max_width(220.0);
                    ui.add(
                        egui::Label::new(RichText::new(&self.title).size(17.0).strong()).truncate(),
                    );
                    if self.is_dirty() {
                        ui.label(
                            RichText::new("● Unsaved changes").color(ui.visuals().warn_fg_color),
                        );
                    } else {
                        ui.weak("No unsaved changes");
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    ui.add(
                        egui::TextEdit::singleline(&mut self.file_path)
                            .desired_width(180.0)
                            .hint_text("lab.toml"),
                    )
                    .on_hover_text("File used by Save and Open");
                });
                ui.separator();
                if tool_button(ui, Icon::Open, "Open", false, true)
                    .on_hover_text("Open a lab file (Ctrl+O)")
                    .clicked()
                {
                    self.request_open();
                }
                if tool_button(ui, Icon::Save, "Save", false, true)
                    .on_hover_text("Save the lab (Ctrl+S)")
                    .clicked()
                {
                    self.save_action();
                }
                ui.separator();
                let undo = self.history.undo_label().map(|l| format!("Undo {l}"));
                let redo = self.history.redo_label().map(|l| format!("Redo {l}"));
                if tool_button(ui, Icon::Undo, "Undo", false, self.can_undo())
                    .on_hover_text(undo.unwrap_or_else(|| "Nothing to undo".into()))
                    .clicked()
                {
                    self.undo();
                }
                if tool_button(ui, Icon::Redo, "Redo", false, self.can_redo())
                    .on_hover_text(redo.unwrap_or_else(|| "Nothing to redo".into()))
                    .clicked()
                {
                    self.redo();
                }
                ui.separator();
                if tool_button(ui, Icon::Select, "Select", self.tool == Tool::Select, true)
                    .on_hover_text("Click a device to open it, drag to move it")
                    .clicked()
                {
                    self.set_tool(Tool::Select);
                }
                if tool_button(ui, Icon::Delete, "Delete", self.tool == Tool::Delete, true)
                    .on_hover_text("Click a device or a cable to remove it")
                    .clicked()
                {
                    let t = if self.tool == Tool::Delete {
                        Tool::Select
                    } else {
                        Tool::Delete
                    };
                    self.set_tool(t);
                }
                ui.separator();
                self.device_strip(ui);
            });
            ui.add_space(2.0);
        });
    }

    /// A chip per device (or one picker for a big lab); a chip opens the device's window.
    fn device_strip(&mut self, ui: &mut Ui) {
        if self.tabs.len() > TAB_LIMIT {
            ui.vertical(|ui| {
                ui.add_space(12.0);
                ui.horizontal(|ui| self.device_picker(ui));
            });
            return;
        }
        egui::ScrollArea::horizontal()
            .id_salt("device-chips")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let mut chosen = None;
                        for i in 0..self.tabs.len() {
                            let kind = self
                                .nodes
                                .get(self.tabs[i].node)
                                .map_or(LabKind::Host, |n| n.kind);
                            let on = self.window_open && self.active == i;
                            let r = device_chip(ui, kind, &self.tabs[i].name, on).on_hover_text(
                                format!("Open {} (Alt+{})", self.tabs[i].name, i + 1),
                            );
                            if r.clicked() {
                                chosen = Some(i);
                            }
                        }
                        if let Some(i) = chosen {
                            self.select(i);
                        }
                    });
                });
            });
    }

    // ---- the component bar ----

    pub(super) fn component_bar(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("components").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                // Categories, then the models in the chosen one.
                for c in Category::ALL {
                    if category_tile(ui, c, self.palette == c).clicked() {
                        self.palette = c;
                    }
                }
                ui.separator();
                self.model_rects.clear();
                for m in self.palette.models() {
                    self.model_card(ui, m);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if tool_button(ui, Icon::Simulation, "Simulation", self.sim_open, true)
                        .on_hover_text("Step through frames one hop at a time and inspect them")
                        .clicked()
                    {
                        self.set_simulation(true);
                    }
                    if tool_button(ui, Icon::Realtime, "Live", !self.sim_open, true)
                        .on_hover_text("The lab runs by itself")
                        .clicked()
                    {
                        self.set_simulation(false);
                    }
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_max_width(260.0);
                        ui.add(egui::Label::new(RichText::new(self.tool_hint()).weak()).wrap());
                    });
                });
            });
            ui.add_space(4.0);
        });
    }

    fn tool_hint(&self) -> &'static str {
        match self.tool {
            Tool::Select => {
                "Drag a model onto the canvas, or click it and then click the canvas. Click a device to open it."
            }
            Tool::Place(_) => "Click the canvas to place another; Esc to stop.",
            Tool::Link if self.link_from.is_some() => {
                "Now click the device at the other end; Esc to cancel."
            }
            Tool::Link => "Click a device, then the one to cable it to; Esc to stop.",
            Tool::Delete => "Click a device or a cable to remove it; Esc to stop.",
        }
    }

    /// Show the simulation panel (or hide it and go back to letting the lab run).
    pub fn set_simulation(&mut self, on: bool) {
        if on != self.sim_open {
            // The canvas changes width: fit the lab to it again.
            self.canvas.view.fitted = false;
        }
        self.sim_open = on;
        if !on {
            self.anim.playing = false;
        }
    }

    pub fn simulation_mode(&self) -> bool {
        self.sim_open
    }

    /// Open a category of the component bar: `"routers"`, `"switches"`, `"pcs"` or `"cables"`.
    pub fn show_components(&mut self, category: &str) {
        if let Some(c) = Category::ALL
            .into_iter()
            .find(|c| c.label().eq_ignore_ascii_case(category))
        {
            self.palette = c;
        }
    }

    /// Screen centre of a model's card in the component bar as of the last frame: a template
    /// (`router-2g`) or `"cable"` (for tests).
    pub fn component_centre(&self, model: &str) -> Option<Pos2> {
        self.model_rects
            .iter()
            .find(|(item, _)| match item {
                PaletteItem::Device { template, .. } => *template == model,
                PaletteItem::Cable => model == "cable",
            })
            .map(|(_, r)| r.center())
    }

    /// Arm the tool for a model in the component bar (what clicking it does).
    pub fn pick_model(&mut self, item: PaletteItem) {
        match item {
            PaletteItem::Device { kind, template } => {
                self.set_tool(Tool::Place(kind));
                self.place_template = Some(template);
            }
            PaletteItem::Cable => self.set_tool(Tool::Link),
        }
    }

    /// Something dragged out of the component bar was dropped on the canvas.
    pub(super) fn dropped(&mut self, item: PaletteItem, pos: [i32; 2], over: Option<usize>) {
        match item {
            PaletteItem::Device { template, .. } => self.add_model(template, pos),
            PaletteItem::Cable => match over {
                Some(i) => {
                    self.set_tool(Tool::Link);
                    self.link_from = Some(i);
                    self.notice = Some(format!(
                        "cable from {}: click the device at the other end",
                        self.nodes[i].name
                    ));
                }
                None => {
                    self.set_tool(Tool::Link);
                    self.notice = Some("drop the cable on a device to start it there".into());
                }
            },
        }
    }

    fn model_card(&mut self, ui: &mut Ui, m: &Model) {
        let id = Id::unique(("model", m.label, m.detail));
        let (rect, resp) = ui.allocate_exact_size(MODEL_CARD, Sense::click_and_drag());
        self.model_rects.push((m.item, rect));
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                true,
                format!("{}, {}", m.label, m.detail),
            )
        });
        let armed = match m.item {
            PaletteItem::Device { template, .. } => {
                matches!(self.tool, Tool::Place(_)) && self.place_template == Some(template)
            }
            PaletteItem::Cable => self.tool == Tool::Link,
        };
        let v = ui.visuals().clone();
        let fill = if armed {
            v.selection.bg_fill
        } else if resp.hovered() {
            v.widgets.hovered.weak_bg_fill
        } else {
            v.faint_bg_color
        };
        let p = ui.painter();
        p.rect(
            rect,
            CornerRadius::same(8),
            fill,
            Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
            StrokeKind::Inside,
        );
        let icon = Rect::from_center_size(
            rect.center_top() + Vec2::new(0.0, 22.0),
            Vec2::new(44.0, 30.0),
        );
        paint_item(p, icon, m.item, v.text_color());
        p.text(
            rect.center_bottom() - Vec2::new(0.0, 20.0),
            Align2::CENTER_BOTTOM,
            m.label,
            FontId::proportional(13.0),
            v.strong_text_color(),
        );
        p.text(
            rect.center_bottom() - Vec2::new(0.0, 5.0),
            Align2::CENTER_BOTTOM,
            m.detail,
            FontId::proportional(10.5),
            v.weak_text_color(),
        );
        let resp = resp.on_hover_text("Drag onto the canvas, or click and then click the canvas");
        if resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), m.item);
        }
        if resp.dragged() {
            // A copy of the symbol follows the pointer while it is dragged.
            if let Some(at) = ui.ctx().pointer_interact_pos() {
                let layer = egui::LayerId::new(egui::Order::Tooltip, id);
                let ghost = ui.ctx().layer_painter(layer);
                paint_item(
                    &ghost,
                    Rect::from_center_size(at, Vec2::new(64.0, 44.0)),
                    m.item,
                    v.text_color(),
                );
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if resp.clicked() {
            if armed {
                self.set_tool(Tool::Select);
            } else {
                self.pick_model(m.item);
            }
        }
    }

    // ---- status line ----

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            let now = self.sim_now;
            ui.horizontal(|ui| {
                match &self.notice {
                    Some(n) => ui.label(n),
                    None => ui
                        .weak("Scroll to zoom, drag the background to pan, double-click it to fit"),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("frame {:.1} ms", self.frame_ms));
                    if let Some(ms) = self.ready_ms {
                        ui.weak(format!("ready in {ms:.0} ms"));
                    }
                    ui.label(format!("time {:.3} s", now as f64 / 1e9));
                });
            });
        });
    }

    // ---- the canvas ----

    pub(super) fn canvas_ui(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                self.canvas_rect = ui.available_rect_before_wrap();
                let active_node = (self.window_open)
                    .then(|| self.tabs.get(self.active).map(|t| t.node))
                    .flatten();
                let nodes: Vec<Node> = self
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| Node {
                        name: n.name.clone(),
                        kind: n.kind,
                        pos: n.pos,
                        selected: Some(i) == active_node || self.link_from == Some(i),
                        has_console: n.tab.is_some(),
                    })
                    .collect();
                let flights: Vec<FlightDraw> = self
                    .anim
                    .flights
                    .iter()
                    .filter_map(|f| {
                        Some(FlightDraw {
                            from: f.from,
                            to: f.to,
                            t: f.progress(self.wall)?,
                            label: &f.label,
                        })
                    })
                    .collect();
                let events = self.canvas.show(
                    ui,
                    &Scene {
                        nodes: &nodes,
                        links: &self.links,
                        notes: &self.notes,
                        tool: self.tool,
                        link_from: self.link_from,
                        flights: &flights,
                    },
                );
                self.handle_canvas(events);
            });
        let ctx = ui.ctx().clone();
        self.device_window_ui(&ctx);
    }
}

/// Ports a template has (`router-3g` 3, `switch-l2-24` 24, `host-pc` 1).
fn port_count(template: &str) -> Option<u16> {
    if template == "host-pc" {
        return Some(1);
    }
    let digits: String = template
        .rsplit('-')
        .next()?
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Where to open the device window: the corner of the canvas where it covers the fewest devices,
/// never the one it was opened for if that can be helped, and otherwise away from that device.
fn window_spot(area: Rect, centres: &[Pos2], node: Option<usize>) -> Pos2 {
    let size = window_size(area);
    let margin = 12.0;
    let xs = [
        area.min.x + margin,
        (area.max.x - size.x - margin).max(area.min.x),
    ];
    let ys = [
        area.min.y + margin,
        (area.max.y - size.y - margin).max(area.min.y),
    ];
    let own = node.and_then(|i| centres.get(i)).copied();
    let mut best = (usize::MAX, f32::MIN, Pos2::new(xs[1], ys[0]));
    for &y in &ys {
        for &x in &xs {
            let r = Rect::from_min_size(Pos2::new(x, y), size).expand(30.0);
            let covered = centres.iter().filter(|c| r.contains(**c)).count();
            let own_covered = own.is_some_and(|c| r.contains(c));
            let score = covered + if own_covered { 1000 } else { 0 };
            // Ties go to the spot farthest from the device, then to the top.
            let far = own.map_or(0.0, |c| r.center().distance(c)) - (y - area.min.y) * 0.01;
            if score < best.0 || (score == best.0 && far > best.1) {
                best = (score, far, Pos2::new(x, y));
            }
        }
    }
    best.2
}

/// The device window's first size: most of the canvas, but no bigger than a console needs.
fn window_size(area: Rect) -> Vec2 {
    Vec2::new(
        (area.width() * 0.45).clamp(340.0, 600.0),
        (area.height() - 24.0).clamp(240.0, 420.0),
    )
}

fn paint_item(p: &egui::Painter, r: Rect, item: PaletteItem, ink: egui::Color32) {
    match item {
        PaletteItem::Device { kind, .. } => {
            draw_device_icon(p, r, kind, kind_colour(kind), Stroke::NONE);
        }
        PaletteItem::Cable => draw_cable_icon(p, r, ink),
    }
}

/// A category in the component bar: a representative symbol over its name.
fn category_tile(ui: &mut Ui, c: Category, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(84.0, MODEL_CARD.y), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, c.label()));
    let v = ui.visuals().clone();
    let fill = if selected {
        v.selection.bg_fill
    } else if resp.hovered() {
        v.widgets.hovered.weak_bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), fill);
    let icon = Rect::from_center_size(
        rect.center_top() + Vec2::new(0.0, 26.0),
        Vec2::new(40.0, 28.0),
    );
    let item = match c {
        Category::Routers => PaletteItem::Device {
            kind: LabKind::Router,
            template: "",
        },
        Category::Switches => PaletteItem::Device {
            kind: LabKind::Switch,
            template: "",
        },
        Category::Pcs => PaletteItem::Device {
            kind: LabKind::Host,
            template: "",
        },
        Category::Cables => PaletteItem::Cable,
    };
    paint_item(p, icon, item, v.text_color());
    let ink = if selected {
        v.selection.stroke.color
    } else {
        v.text_color()
    };
    p.text(
        rect.center_bottom() - Vec2::new(0.0, 10.0),
        Align2::CENTER_BOTTOM,
        c.label(),
        FontId::proportional(12.0),
        ink,
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::{port_count, window_spot};
    use egui::{Pos2, Rect, Vec2};

    #[test]
    fn the_window_opens_where_it_covers_the_fewest_devices() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0));
        // Devices along the top and one on the left: the window goes bottom right.
        let centres = [
            Pos2::new(100.0, 80.0),
            Pos2::new(600.0, 80.0),
            Pos2::new(1100.0, 80.0),
            Pos2::new(100.0, 600.0),
        ];
        let at = window_spot(area, &centres, Some(0));
        assert!(at.x > 500.0 && at.y > 200.0, "{at:?}");
        // With nothing in the way it opens on the far side from the device.
        let at = window_spot(area, &[Pos2::new(1100.0, 350.0)], Some(0));
        assert!(at.x < 100.0, "{at:?}");
    }

    #[test]
    fn port_counts_come_from_the_template_name() {
        assert_eq!(port_count("router-3g"), Some(3));
        assert_eq!(port_count("switch-l2-24"), Some(24));
        assert_eq!(port_count("host-pc"), Some(1));
    }
}
