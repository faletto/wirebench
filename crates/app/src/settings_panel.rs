//! Drawing the device settings panel (a child of `app` so it can reach the app's fields). What it
//! sends is built by `settings.rs`; the engine runs it through the device's own CLI.

use super::*;
use crate::settings::{
    host_lines, interface_lines, port_lines, route_lines, spanning_lines, vlan_lines,
};
use engine::panel::DeviceState;

impl App {
    /// The device whose tab is open, if any.
    fn panel_device(&self) -> Option<u32> {
        self.tabs.get(self.active).map(|t| t.device)
    }

    /// Read the device's state again if the panel is showing another device or an old one.
    fn panel_refresh(&mut self) {
        let Some(device) = self.panel_device() else {
            self.panel.device = None;
            self.panel.state = None;
            return;
        };
        if self.panel.device == Some(device) && !self.panel.stale {
            return;
        }
        let changed_device = self.panel.device != Some(device);
        if let Some(EngineEvent::State { state, .. }) =
            self.engine.send(EngineCmd::DeviceState { device }).pop()
        {
            if changed_device || self.panel.state.as_ref() != Some(&state) {
                self.panel.state = Some(state);
                self.panel.load_drafts();
            }
        }
        if changed_device {
            self.panel.said.clear();
        }
        self.panel.device = Some(device);
        self.panel.stale = false;
    }

    /// Send CLI lines to the active device, as if typed in configuration mode. Returns what the
    /// CLI said about any it refused. The lab counts as edited and the panel reads the device again.
    pub fn configure_active(&mut self, lines: &[&str]) -> Vec<String> {
        let Some(device) = self.panel_device() else {
            return vec!["no device is open".to_string()];
        };
        let lines: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
        self.panel_send(device, lines);
        self.panel.said.clone()
    }

    fn panel_send(&mut self, device: u32, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        // The engine forgets what could be stepped back over; so must the log of marks.
        self.sim.marks.clear();
        let said = match self
            .engine
            .send(EngineCmd::Configure { device, lines })
            .pop()
        {
            Some(EngineEvent::Configured { said, .. }) => said,
            Some(EngineEvent::Error { message }) => vec![message],
            _ => vec![],
        };
        self.panel.stale = true;
        self.changes.mark_edited(self.wall);
        self.panel_refresh();
        self.panel.said = said;
        // The device's prompt or hostname may have changed.
        self.refresh_clock();
    }

    /// The panel's state for the active device (reads it first), for tests.
    pub fn active_state(&mut self) -> Option<DeviceState> {
        self.panel_refresh();
        self.panel.state.clone()
    }

    fn panel_try(&mut self, device: u32, lines: Result<Vec<String>, String>) {
        match lines {
            Ok(l) => self.panel_send(device, l),
            Err(e) => self.panel.said = vec![e],
        }
    }

    pub(super) fn settings_panel(&mut self, ui: &mut Ui) {
        self.panel_refresh();
        ui.heading("Settings");
        let (Some(device), Some(state)) = (self.panel.device, self.panel.state.clone()) else {
            ui.weak("Open a device to see its settings.");
            return;
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                match state {
                    DeviceState::Host(h) => self.host_settings(ui, device, &h),
                    DeviceState::Router(r) => self.router_settings(ui, device, &r),
                    DeviceState::Switch(s) => self.switch_settings(ui, device, &s),
                }
                if !self.panel.said.is_empty() {
                    ui.separator();
                    for line in &self.panel.said.clone() {
                        ui.colored_label(ui.visuals().warn_fg_color, line);
                    }
                    if ui.small_button("Dismiss").clicked() {
                        self.panel.said.clear();
                    }
                }
            });
    }

    fn host_settings(&mut self, ui: &mut Ui, device: u32, h: &engine::panel::HostState) {
        ui.strong(&h.name);
        let mut dhcp = h.dhcp;
        ui.horizontal(|ui| {
            ui.label("Address");
            ui.add(
                egui::TextEdit::singleline(&mut self.panel.host_addr)
                    .hint_text("10.0.0.5/24")
                    .desired_width(130.0),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Gateway");
            ui.add(
                egui::TextEdit::singleline(&mut self.panel.host_gw)
                    .hint_text("10.0.0.1")
                    .desired_width(130.0),
            );
        });
        if ui.button("Apply").clicked() {
            let lines = host_lines(&self.panel.host_addr, &self.panel.host_gw, false);
            self.panel_try(device, lines);
        }
        if ui
            .checkbox(&mut dhcp, "Get the address from DHCP")
            .changed()
            && dhcp
        {
            self.panel_send(device, vec!["ipconfig dhcp".into()]);
        }
    }

    fn router_settings(&mut self, ui: &mut Ui, device: u32, r: &engine::panel::RouterState) {
        ui.strong(&r.name);
        ui.label("Interfaces");
        for i in &r.interfaces {
            ui.horizontal(|ui| {
                ui.label(&i.cli);
                let up = if i.up {
                    "up"
                } else if i.shutdown {
                    "shut"
                } else {
                    "down"
                };
                ui.weak(up);
            });
            ui.horizontal(|ui| {
                let text = self.panel.if_addr.entry(i.cli.clone()).or_default();
                ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text("no address")
                        .desired_width(120.0),
                );
                let mut shut = i.shutdown;
                let set = ui.small_button("Set").clicked();
                let toggled = ui.checkbox(&mut shut, "shut").changed();
                if set || toggled {
                    let addr = self.panel.if_addr.get(&i.cli).cloned().unwrap_or_default();
                    let lines = interface_lines(&i.cli, &addr, shut);
                    self.panel_try(device, lines);
                }
            });
        }
        ui.separator();
        ui.label("Static routes");
        for route in &r.routes {
            ui.horizontal(|ui| {
                ui.label(format!("{} via {}", route.net, route.via));
                if ui.small_button("x").clicked() {
                    let lines = route_lines(&route.net, &route.via, false);
                    self.panel_try(device, lines);
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.panel.route_net)
                    .hint_text("10.0.2.0/24")
                    .desired_width(100.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.panel.route_via)
                    .hint_text("next hop")
                    .desired_width(80.0),
            );
            if ui.small_button("Add").clicked() {
                let lines = route_lines(&self.panel.route_net, &self.panel.route_via, true);
                if lines.is_ok() {
                    self.panel.route_net.clear();
                    self.panel.route_via.clear();
                }
                self.panel_try(device, lines);
            }
        });
    }

    fn switch_settings(&mut self, ui: &mut Ui, device: u32, s: &engine::panel::SwitchState) {
        ui.strong(&s.name);
        ui.horizontal(|ui| {
            ui.label("Spanning tree");
            let now = s.spanning_tree.clone().unwrap_or_else(|| "off".into());
            let mut pick = now.clone();
            egui::ComboBox::from_id_salt("stp_mode")
                .selected_text(&now)
                .show_ui(ui, |ui| {
                    for m in ["off", "pvst", "rapid-pvst"] {
                        ui.selectable_value(&mut pick, m.to_string(), m);
                    }
                });
            if pick != now {
                self.panel_send(device, spanning_lines(&pick));
            }
        });
        ui.separator();
        ui.label("VLANs");
        for (id, name) in &s.vlans {
            ui.horizontal(|ui| {
                ui.label(format!("{id}  {name}"));
                if *id != 1 && ui.small_button("x").clicked() {
                    self.panel_send(device, vlan_lines(*id, "", false));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.panel.vlan_id)
                    .hint_text("id")
                    .desired_width(40.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.panel.vlan_name)
                    .hint_text("name")
                    .desired_width(90.0),
            );
            if ui.small_button("Add").clicked() {
                match self.panel.vlan_id.trim().parse::<u16>() {
                    Ok(id) if (1..=4094).contains(&id) => {
                        let lines = vlan_lines(id, &self.panel.vlan_name, true);
                        self.panel.vlan_id.clear();
                        self.panel.vlan_name.clear();
                        self.panel_send(device, lines);
                    }
                    _ => self.panel.said = vec!["VLAN id must be 1 to 4094".into()],
                }
            }
        });
        ui.separator();
        ui.label("Ports");
        for p in &s.ports {
            let mut mode = p.mode.clone();
            let mut vlan = p.access_vlan;
            let mut shut = p.shutdown;
            let mut fast = p.portfast;
            ui.horizontal(|ui| {
                ui.label(&p.name);
                egui::ComboBox::from_id_salt(("mode", &p.cli))
                    .width(110.0)
                    .selected_text(if p.trunking && p.mode.starts_with("dynamic") {
                        format!("{} (trunk)", p.mode)
                    } else {
                        p.mode.clone()
                    })
                    .show_ui(ui, |ui| {
                        for m in ["access", "trunk", "dynamic auto", "dynamic desirable"] {
                            ui.selectable_value(&mut mode, m.to_string(), m);
                        }
                    });
                ui.add(
                    egui::DragValue::new(&mut vlan)
                        .range(1..=4094)
                        .prefix("vlan "),
                );
                ui.checkbox(&mut shut, "shut");
                ui.checkbox(&mut fast, "fast");
            });
            if mode != p.mode || vlan != p.access_vlan || shut != p.shutdown || fast != p.portfast {
                self.panel_send(device, port_lines(&p.cli, &mode, vlan, shut, fast));
            }
        }
    }
}
