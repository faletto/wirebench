//! The little window that opens beside a device on the canvas: either its right-click menu (port
//! status, open the console or settings, delete) or, while cabling, the list of ports to plug the
//! cable into. Port status comes from the engine, so the window fills in when the answer arrives.

use super::*;
use egui::{Color32, Id, Order, Rect, RichText};
use engine::lab::{PortInfo, PortStatus};

/// What the window is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PopupKind {
    /// Right-click menu.
    Menu,
    /// Choosing which port a cable goes into.
    PickPort,
}

#[derive(Debug, Clone)]
pub(super) struct Popup {
    pub node: usize,
    pub at: Pos2,
    pub kind: PopupKind,
    /// None until the engine has answered.
    pub ports: Option<Vec<PortInfo>>,
}

/// What a click in the window asked for.
enum Action {
    Port(u16),
    Console,
    Settings,
    Delete,
}

fn status_text(s: PortStatus) -> &'static str {
    match s {
        PortStatus::Disconnected => "disconnected",
        PortStatus::Up => "up",
        PortStatus::Down => "down",
    }
}

fn status_colour(s: PortStatus, weak: Color32) -> Color32 {
    match s {
        PortStatus::Disconnected => weak,
        PortStatus::Up => Color32::from_rgb(46, 160, 67),
        PortStatus::Down => Color32::from_rgb(207, 70, 60),
    }
}

impl App {
    /// Open the window for `node` at `at` and ask the engine how its ports stand.
    pub(super) fn open_popup(&mut self, node: usize, at: Pos2, kind: PopupKind) {
        if node >= self.nodes.len() {
            return;
        }
        self.popup = Some(Popup {
            node,
            at,
            kind,
            ports: None,
        });
        self.popup_rows.clear();
        self.post(EngineCmd::Ports { device: node }, Awaiting::Ports { node });
        self.pump();
    }

    /// Where a window for `node` goes: just right of the device.
    pub(super) fn popup_spot(&self, node: usize) -> Pos2 {
        self.canvas
            .last_centres
            .get(node)
            .map_or(self.canvas_rect.center(), |c| *c + egui::vec2(40.0, -10.0))
    }

    /// The engine's answer about `node`'s ports.
    pub(super) fn ports_arrived(&mut self, node: usize, ports: Vec<PortInfo>) {
        let Some(p) = self.popup.as_mut().filter(|p| p.node == node) else {
            return;
        };
        if p.kind == PopupKind::PickPort
            && !ports.iter().any(|q| q.status == PortStatus::Disconnected)
        {
            let name = self.nodes[node].name.clone();
            self.popup = None;
            self.notice = Some(format!("{name} has no free port"));
            return;
        }
        p.ports = Some(ports);
    }

    pub fn popup_open(&self) -> bool {
        self.popup.is_some()
    }

    /// Port names and status in the open window, once the engine has said (for tests).
    pub fn popup_ports(&self) -> Option<Vec<(String, PortStatus)>> {
        let p = self.popup.as_ref()?;
        Some(
            p.ports
                .as_ref()?
                .iter()
                .map(|q| (q.name.clone(), q.status))
                .collect(),
        )
    }

    /// Where a row or button of the open window was drawn last frame (for tests).
    pub fn popup_row_centre(&self, starts_with: &str) -> Option<Pos2> {
        self.popup_rows
            .iter()
            .find(|(l, _)| l.starts_with(starts_with))
            .map(|(_, r)| r.center())
    }

    /// Right-click a device (what the canvas does on a secondary click).
    pub fn open_menu(&mut self, device: &str) {
        if let Some(i) = self.nodes.iter().position(|n| n.name == device) {
            let at = self.popup_spot(i);
            self.open_popup(i, at, PopupKind::Menu);
        }
    }

    /// Plug the cable being drawn into `port` of `node`: the first press of this starts a cable,
    /// the second ends it.
    pub fn pick_port(&mut self, node: usize, port: u16) {
        self.popup = None;
        if node >= self.nodes.len() {
            return;
        }
        match self.link_from.take() {
            None => {
                self.link_from = Some((node, port));
                self.notice = Some(format!(
                    "cable from {}: click the device at the other end",
                    self.nodes[node].name
                ));
            }
            Some((a, pa)) => self.connect_ports(a, pa, node, port),
        }
    }

    /// Cable port `pa` of device `a` to port `pb` of device `b`.
    pub fn connect_ports(&mut self, a: usize, pa: u16, b: usize, pb: u16) {
        if a >= self.nodes.len() || b >= self.nodes.len() {
            return;
        }
        let before = self.snapshot_lab();
        self.post(
            EngineCmd::AddLinkPorts { a, pa, b, pb },
            Awaiting::Edit {
                what: EditKind::AddLink { a, b },
                before,
            },
        );
        self.pump();
    }

    fn open_from_menu(&mut self, node: usize, tab: DeviceTab) {
        match self.nodes.get(node).and_then(|n| n.tab) {
            Some(t) => {
                self.notice = None;
                self.select(t);
                self.device_tab = tab;
            }
            None => {
                if let Some(n) = self.nodes.get(node) {
                    self.notice = Some(format!("{} has no console yet", n.name));
                }
            }
        }
    }

    pub(super) fn popup_ui(&mut self, ctx: &egui::Context) {
        let Some(popup) = self.popup.clone() else {
            return;
        };
        let Some(node) = self.nodes.get(popup.node) else {
            self.popup = None;
            return;
        };
        let (name, kind, has_tab) = (node.name.clone(), node.kind, node.tab.is_some());
        let mut action = None;
        let mut rows: Vec<(String, Rect)> = Vec::new();
        let area = egui::Area::new(Id::unique("device_popup"))
            .order(Order::Foreground)
            .fixed_pos(popup.at)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(190.0);
                    let weak = ui.visuals().weak_text_color();
                    match popup.kind {
                        PopupKind::Menu => {
                            ui.horizontal(|ui| {
                                ui.strong(&name);
                                ui.label(RichText::new(crate::canvas::kind_name(kind)).weak());
                            });
                        }
                        PopupKind::PickPort => {
                            ui.strong(format!("Plug the cable into {name}"));
                        }
                    }
                    ui.separator();
                    match &popup.ports {
                        None => {
                            ui.label(RichText::new("reading ports…").weak());
                        }
                        Some(ports) => {
                            egui::ScrollArea::vertical()
                                .max_height(240.0)
                                .auto_shrink([true, true])
                                .show(ui, |ui| {
                                    for (i, p) in ports.iter().enumerate() {
                                        let peer = p
                                            .peer
                                            .as_ref()
                                            .map(|d| format!(" to {d}"))
                                            .unwrap_or_default();
                                        let text =
                                            format!("{}: {}{peer}", p.name, status_text(p.status));
                                        let colour = status_colour(p.status, weak);
                                        match popup.kind {
                                            PopupKind::Menu => {
                                                let r =
                                                    ui.label(RichText::new(&text).color(colour));
                                                rows.push((text, r.rect));
                                            }
                                            PopupKind::PickPort => {
                                                let free = p.status == PortStatus::Disconnected;
                                                let label = if free {
                                                    format!("{}: free", p.name)
                                                } else {
                                                    text.clone()
                                                };
                                                let r =
                                                    ui.add_enabled(
                                                        free,
                                                        egui::Button::new(
                                                            RichText::new(&label).color(if free {
                                                                ui.visuals().text_color()
                                                            } else {
                                                                colour
                                                            }),
                                                        )
                                                        .min_size(egui::vec2(
                                                            ui.available_width(),
                                                            0.0,
                                                        )),
                                                    );
                                                rows.push((label, r.rect));
                                                if r.clicked() {
                                                    action = Some(Action::Port(i as u16));
                                                }
                                            }
                                        }
                                    }
                                });
                        }
                    }
                    if popup.kind == PopupKind::Menu {
                        ui.separator();
                        for (label, act) in [
                            ("Open console", Action::Console),
                            ("Open settings", Action::Settings),
                            ("Delete", Action::Delete),
                        ] {
                            let enabled = has_tab || matches!(act, Action::Delete);
                            let r = ui.add_enabled(
                                enabled,
                                egui::Button::new(label)
                                    .min_size(egui::vec2(ui.available_width(), 0.0)),
                            );
                            rows.push((label.to_string(), r.rect));
                            if r.clicked() {
                                action = Some(act);
                            }
                        }
                    }
                });
            });
        self.popup_rows = rows;
        // A press anywhere else dismisses it (the press that began a click on something else
        // still reaches that thing when it is let go).
        let outside = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !area.response.rect.contains(p))
        });
        match action {
            Some(Action::Port(p)) => self.pick_port(popup.node, p),
            Some(Action::Console) => {
                self.popup = None;
                self.open_from_menu(popup.node, DeviceTab::Console);
            }
            Some(Action::Settings) => {
                self.popup = None;
                self.open_from_menu(popup.node, DeviceTab::Settings);
            }
            Some(Action::Delete) => {
                self.popup = None;
                self.delete_device(popup.node);
            }
            None if outside => self.popup = None,
            None => {}
        }
    }
}
