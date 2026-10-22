//! The app: a lab on a topology canvas plus one terminal tab per device that has a console,
//! driven through the same `EngineCmd`/`EngineEvent` protocol the web build will use across
//! a Worker.

use crate::anim::{caption, Anim, SPEEDS};
use crate::canvas::{Canvas, CanvasEvent, FlightDraw, Node, Scene, Tool};
use crate::sim::{parse_ipv4, HopRow, Sim};
use crate::term::TermModel;
use crate::widget::{self, TermAction};
use egui::{Key, Pos2, Ui};
use engine::lab::{BuiltLab, LabDevice, LabKind, Topology};
use engine::protocol::{Engine, EngineCmd, EngineEvent, ShellKind};

/// The lab the app opens with until file loading exists.
const DEFAULT_LAB: &str = include_str!("../../../labs/m1_static_routes.lab.toml");

#[derive(Debug)]
struct Tab {
    name: String,
    /// Index into `App::nodes`.
    node: usize,
    device: u32,
    terminal: u32,
    prompt: String,
    model: TermModel,
}

#[derive(Debug, Clone)]
struct NodeInfo {
    name: String,
    kind: LabKind,
    pos: [i32; 2],
    /// Index into `App::tabs`, if this device has a console.
    tab: Option<usize>,
    /// Engine device id.
    device: u32,
}

#[derive(Debug)]
pub struct App {
    engine: Engine,
    title: String,
    nodes: Vec<NodeInfo>,
    links: Vec<(usize, usize)>,
    notes: Vec<(String, [i32; 2])>,
    tabs: Vec<Tab>,
    active: usize,
    canvas: Canvas,
    topo: Topology,
    tool: Tool,
    /// First device of a cable being drawn with the Link tool.
    link_from: Option<usize>,
    /// One-line message in the status bar (lab warnings, "no console" hints).
    notice: Option<String>,
    /// Set when a tab becomes active so its terminal grabs keyboard focus.
    focus_pending: bool,
    sim: Sim,
    anim: Anim,
    /// Wall-clock seconds as of the latest frame (egui's clock; works on web too).
    wall: f64,
    frame_ms: f32,
    /// Milliseconds from the start of loading to the first drawn frame (see `clock`).
    ready_ms: Option<f64>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self::from_lab_text(DEFAULT_LAB).expect("the bundled lab is valid")
    }

    pub fn from_lab_text(text: &str) -> Result<Self, String> {
        let built = engine::lab::load_and_build(text, kernel::Trace::new())?;
        Ok(Self::from_built(built))
    }

    pub fn from_built(built: BuiltLab) -> Self {
        let (world, topo, warnings) = built.into_parts();
        let mut engine = Engine::new(world);
        let mut nodes = Vec::new();
        let mut tabs: Vec<Tab> = Vec::new();
        for d in &topo.devices {
            let tab = Some(tabs.len());
            tabs.push(new_tab(&mut engine, d, nodes.len()));
            nodes.push(NodeInfo {
                name: d.name.clone(),
                kind: d.kind,
                pos: d.pos,
                tab,
                device: d.id.0,
            });
        }
        let notice = (!warnings.is_empty())
            .then(|| format!("lab: {} (+{} more)", warnings[0], warnings.len() - 1));
        App {
            engine,
            title: topo.title.clone(),
            nodes,
            links: topo.links.iter().map(|l| (l.a, l.b)).collect(),
            notes: topo
                .annotations
                .iter()
                .map(|a| (a.text.clone(), a.pos))
                .collect(),
            topo,
            tool: Tool::Select,
            link_from: None,
            tabs,
            active: 0,
            canvas: Canvas::default(),
            notice,
            focus_pending: true,
            sim: Sim {
                dst: "10.0.2.2".into(),
                ..Sim::default()
            },
            anim: Anim::default(),
            wall: 0.0,
            frame_ms: 0.0,
            ready_ms: None,
        }
    }

    /// Currently selected tab's prompt, for tests.
    pub fn active_prompt(&self) -> &str {
        &self.tabs[self.active].prompt
    }

    /// Name of the device whose terminal is showing.
    pub fn active_device(&self) -> &str {
        &self.tabs[self.active].name
    }

    /// Time from the start of loading to the first frame, once one has been drawn.
    pub fn ready_ms(&self) -> Option<f64> {
        self.ready_ms
    }

    /// Lines shown in the active terminal, oldest first (for tests).
    pub fn active_scrollback(&self) -> &[String] {
        &self.tabs[self.active].model.scrollback
    }

    /// Screen position of a device's centre as of the last frame (for tests).
    pub fn node_centre(&self, name: &str) -> Option<Pos2> {
        let i = self.nodes.iter().position(|n| n.name == name)?;
        self.canvas.last_centres.get(i).copied()
    }

    pub fn status_notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn select(&mut self, tab: usize) {
        if tab < self.tabs.len() && tab != self.active {
            self.active = tab;
            self.focus_pending = true;
        }
    }

    // ---- editing ----

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.link_from = None;
    }

    /// Add a device at a canvas position and open its console.
    pub fn add_device(&mut self, kind: LabKind, pos: [i32; 2]) {
        match self.topo.add_device(&mut self.engine.world, kind, pos) {
            Ok(i) => {
                let d = self.topo.devices[i].clone();
                self.tabs.push(new_tab(&mut self.engine, &d, i));
                self.nodes.push(NodeInfo {
                    name: d.name.clone(),
                    kind: d.kind,
                    pos,
                    tab: Some(self.tabs.len() - 1),
                    device: d.id.0,
                });
                self.notice = Some(format!("added {}", d.name));
                self.select(self.tabs.len() - 1);
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    pub fn move_node(&mut self, node: usize, pos: [i32; 2]) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.pos = pos;
            self.topo.move_device(node, pos);
        }
    }

    /// Cable two devices on their first free ports.
    pub fn connect(&mut self, a: usize, b: usize) {
        match self.topo.add_link(&mut self.engine.world, a, b) {
            Ok(_) => {
                self.links.push((a, b));
                self.notice = None;
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// Remove a device and its cables. The world is rebuilt from the live configs, so
    /// terminals, ARP caches, MAC tables and virtual time restart.
    pub fn delete_device(&mut self, node: usize) {
        let Some(name) = self.nodes.get(node).map(|n| n.name.clone()) else {
            return;
        };
        match self.topo.without_device(&self.engine.world, node) {
            Ok(lab) => self.rebuild(&lab, format!("removed {name}; consoles restarted")),
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// Remove a cable (index into the cable list); rebuilds like [`App::delete_device`].
    pub fn delete_link(&mut self, link: usize) {
        match self.topo.without_link(&self.engine.world, link) {
            Ok(lab) => self.rebuild(&lab, "removed a cable; consoles restarted".into()),
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    fn rebuild(&mut self, lab: &labfile::LabFile, note: String) {
        let active = self.tabs.get(self.active).map(|t| t.name.clone());
        let built = match engine::lab::build(lab, kernel::Trace::new()) {
            Ok(b) => b,
            Err(e) => {
                self.notice = Some(format!("could not rebuild the lab: {e}"));
                return;
            }
        };
        let mut fresh = App::from_built(built);
        // Keep what is about the user's session rather than the lab.
        fresh.canvas = std::mem::take(&mut self.canvas);
        fresh.tool = self.tool;
        fresh.sim.dst = std::mem::take(&mut self.sim.dst);
        fresh.frame_ms = self.frame_ms;
        fresh.ready_ms = self.ready_ms;
        fresh.notice = Some(note);
        if let Some(i) = active.and_then(|n| fresh.tabs.iter().position(|t| t.name == n)) {
            fresh.active = i;
        }
        *self = fresh;
    }

    /// The lab as it is now (positions, cabling, live configs) as lab-file text.
    pub fn export_lab(&self) -> String {
        labfile::save(&self.topo.snapshot(&self.engine.world))
    }

    /// Names of devices in lab order (for tests and keyboard shortcuts).
    pub fn device_names(&self) -> Vec<&str> {
        self.nodes.iter().map(|n| n.name.as_str()).collect()
    }

    /// Cables as pairs of device names, in lab order.
    pub fn cables(&self) -> Vec<(&str, &str)> {
        self.links
            .iter()
            .map(|&(a, b)| (self.nodes[a].name.as_str(), self.nodes[b].name.as_str()))
            .collect()
    }

    fn handle_canvas(&mut self, events: Vec<CanvasEvent>) {
        for ev in events {
            match ev {
                CanvasEvent::Open(i) => match self.nodes[i].tab {
                    Some(t) => {
                        self.notice = None;
                        self.select(t);
                        // Clicking the already-active device still returns focus to its terminal.
                        self.focus_pending = true;
                    }
                    None => {
                        self.notice = Some(format!("{} has no console yet", self.nodes[i].name))
                    }
                },
                CanvasEvent::Moved(i, pos) => self.move_node(i, pos),
                CanvasEvent::Place(pos) => {
                    if let Tool::Place(kind) = self.tool {
                        self.add_device(kind, pos);
                    }
                }
                CanvasEvent::LinkEnd(i) => match self.link_from.take() {
                    None => self.link_from = Some(i),
                    Some(a) => self.connect(a, i),
                },
                // Rebuilds invalidate every index, so nothing after these is applied.
                CanvasEvent::DeleteNode(i) => return self.delete_device(i),
                CanvasEvent::DeleteLink(k) => return self.delete_link(k),
            }
        }
    }

    fn tool_bar(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            for (tool, label) in [
                (Tool::Select, "Select"),
                (Tool::Place(LabKind::Host), "+ PC"),
                (Tool::Place(LabKind::Switch), "+ Switch"),
                (Tool::Place(LabKind::Router), "+ Router"),
                (Tool::Link, "Cable"),
                (Tool::Delete, "Delete"),
            ] {
                if ui.selectable_label(self.tool == tool, label).clicked() {
                    self.set_tool(tool);
                }
            }
            ui.weak(self.tool.hint());
        });
    }

    // ---- simulation mode ----

    /// Logged frame deliveries, oldest first.
    pub fn sim_hops(&self) -> &[HopRow] {
        &self.sim.hops
    }

    /// Queue one echo request from PC `from` without advancing time.
    pub fn sim_queue_ping(&mut self, from: &str, dst: [u8; 4]) {
        let Some(n) = self.nodes.iter().find(|n| n.name == from) else {
            self.notice = Some(format!("no device named {from}"));
            return;
        };
        let device = n.device;
        let ev = self.engine.handle(EngineCmd::PingOnce { device, dst });
        self.apply_sim(ev, false);
    }

    /// Dispatch one event. Returns false once nothing is left to do.
    pub fn sim_step(&mut self) -> bool {
        self.step_inner(false)
    }

    fn step_inner(&mut self, animate: bool) -> bool {
        let ev = self.engine.handle(EngineCmd::Step);
        let more = !matches!(ev.first(), Some(EngineEvent::Idle { .. }));
        self.apply_sim(ev, animate);
        more
    }

    /// Step to the next frame delivery (skipping bare timer events) and animate it.
    /// Returns false when the simulation is idle.
    pub fn sim_step_hop(&mut self) -> bool {
        for _ in 0..10_000 {
            let before = self.sim.hops.len();
            let more = self.step_inner(true);
            if self.sim.hops.len() > before {
                return true;
            }
            if !more {
                return false;
            }
        }
        true
    }

    /// Step to idle, logging every hop but not animating: this is fast-forward. Capped so a
    /// runaway lab cannot hang the UI.
    pub fn sim_run(&mut self) {
        self.anim.playing = false;
        for _ in 0..100_000 {
            if !self.sim_step() {
                return;
            }
        }
        self.notice = Some("stopped after 100000 events; Run again to continue".into());
    }

    // ---- animation ----

    pub fn set_playing(&mut self, playing: bool) {
        self.anim.playing = playing;
        // The first auto-step happens on the next frame.
        self.anim.next_step = self.wall;
    }

    pub fn is_playing(&self) -> bool {
        self.anim.playing
    }

    pub fn set_animation_enabled(&mut self, on: bool) {
        self.anim.enabled = on;
        if !on {
            self.anim.flights.clear();
        }
    }

    /// Index into the speed list (0.5x, 1x, 2x, 4x).
    pub fn set_speed(&mut self, index: usize) {
        self.anim.speed = index.min(SPEEDS.len() - 1);
    }

    /// Frames currently in flight as `(from, to, label)` by device name, for tests.
    pub fn flights(&self) -> Vec<(&str, &str, &str)> {
        self.anim
            .flights
            .iter()
            .map(|f| {
                (
                    self.nodes[f.from].name.as_str(),
                    self.nodes[f.to].name.as_str(),
                    f.label.as_str(),
                )
            })
            .collect()
    }

    fn apply_sim(&mut self, events: Vec<EngineEvent>, animate: bool) {
        for e in events {
            match e {
                EngineEvent::Hop {
                    ns,
                    device,
                    port,
                    summary,
                    from,
                    ..
                } => {
                    if animate && self.anim.enabled {
                        let idx = |d: u32| self.nodes.iter().position(|n| n.device == d);
                        if let (Some(a), Some(b)) = (idx(from), idx(device)) {
                            self.anim.start(a, b, self.wall, caption(&summary));
                        }
                    }
                    let name = self
                        .nodes
                        .iter()
                        .find(|n| n.device == device)
                        .map_or_else(|| format!("#{device}"), |n| n.name.clone());
                    self.sim.hops.push(HopRow {
                        ns,
                        device: name,
                        port,
                        summary,
                    });
                }
                EngineEvent::Error { message } => self.notice = Some(message),
                _ => {}
            }
        }
    }

    fn sim_panel(&mut self, ui: &mut Ui) {
        ui.heading("Simulation");
        let pcs: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| self.nodes[i].kind == LabKind::Host)
            .collect();
        if self.sim.src.is_none_or(|s| !pcs.contains(&s)) {
            self.sim.src = pcs.first().copied();
        }
        let src_name = |s: Option<usize>, nodes: &[NodeInfo]| {
            s.map_or_else(|| "(no PC)".to_string(), |i| nodes[i].name.clone())
        };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("sim_src")
                .selected_text(src_name(self.sim.src, &self.nodes))
                .show_ui(ui, |ui| {
                    for &i in &pcs {
                        ui.selectable_value(&mut self.sim.src, Some(i), &self.nodes[i].name);
                    }
                });
            ui.add(egui::TextEdit::singleline(&mut self.sim.dst).desired_width(96.0));
        });
        let dst = parse_ipv4(&self.sim.dst);
        ui.horizontal_wrapped(|ui| {
            let can_ping = dst.is_some() && self.sim.src.is_some();
            if ui
                .add_enabled(can_ping, egui::Button::new("Queue ping"))
                .clicked()
            {
                if let (Some(d), Some(s)) = (dst, self.sim.src) {
                    let name = self.nodes[s].name.clone();
                    self.sim_queue_ping(&name, d);
                }
            }
            if ui.button("Step").clicked() {
                self.anim.playing = false;
                self.sim_step_hop();
            }
            let play = if self.anim.playing { "Pause" } else { "Play" };
            if ui.button(play).clicked() {
                self.set_playing(!self.anim.playing);
            }
            if ui.button("Run").clicked() {
                self.sim_run();
            }
            if ui.button("Clear").clicked() {
                self.sim.hops.clear();
                self.anim.flights.clear();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Speed");
            egui::ComboBox::from_id_salt("sim_speed")
                .selected_text(format!("{}x", SPEEDS[self.anim.speed]))
                .show_ui(ui, |ui| {
                    for (i, s) in SPEEDS.iter().enumerate() {
                        ui.selectable_value(&mut self.anim.speed, i, format!("{s}x"));
                    }
                });
            let mut on = self.anim.enabled;
            if ui.checkbox(&mut on, "Animate").changed() {
                self.set_animation_enabled(on);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.sim.hops.is_empty() {
                    ui.weak("Queue a ping, then Step to follow each frame.");
                }
                for h in &self.sim.hops {
                    ui.label(format!(
                        "{:>9.3} ms  {}:{}  {}",
                        h.ns as f64 / 1e6,
                        h.device,
                        h.port,
                        h.summary
                    ));
                }
            });
    }

    /// Run one terminal action against the engine and print the result.
    pub fn apply(&mut self, tab: usize, action: TermAction) {
        let t = &mut self.tabs[tab];
        let terminal = t.terminal;
        match action {
            TermAction::Submit(line) => {
                t.model.push_output(&format!("{}{line}", t.prompt));
                match self.engine.handle(EngineCmd::Exec { terminal, line }).pop() {
                    Some(EngineEvent::Output { text, prompt, .. }) => {
                        t.model.push_output(&text);
                        t.prompt = prompt;
                    }
                    Some(EngineEvent::Closed { .. }) => {
                        t.model.push_output("[session closed; opened a new one]");
                        let (terminal, prompt) = open_terminal(&mut self.engine, t.device);
                        t.terminal = terminal;
                        t.prompt = prompt;
                    }
                    Some(EngineEvent::Error { message }) => t.model.push_output(&message),
                    _ => {}
                }
            }
            TermAction::Help(line) => {
                t.model.push_output(&format!("{}{line}?", t.prompt));
                if let Some(EngineEvent::Output { text, .. }) =
                    self.engine.handle(EngineCmd::Help { terminal, line }).pop()
                {
                    t.model.push_output(&text);
                }
            }
            TermAction::Tab(line) => {
                if let Some(EngineEvent::Completion { line, .. }) =
                    self.engine.handle(EngineCmd::Tab { terminal, line }).pop()
                {
                    t.model.set_input(&line);
                }
            }
        }
    }

    /// Draw the whole UI. Takes only a `Ui` so headless tests can drive it.
    pub fn ui(&mut self, ui: &mut Ui) {
        if self.ready_ms.is_none() {
            self.ready_ms = crate::clock::since_start_ms();
            if let Some(ms) = self.ready_ms {
                crate::clock::log(&format!("wirebench: first frame after {ms:.0} ms"));
            }
        }
        self.frame_ms = 0.9 * self.frame_ms + 0.1 * ui.input(|i| i.unstable_dt) * 1000.0;
        self.wall = ui.input(|i| i.time);
        // Playing: one hop per `hop_secs`, then stop when the simulation goes idle.
        if self.anim.playing && self.wall >= self.anim.next_step {
            if self.sim_step_hop() {
                self.anim.next_step = self.wall + self.anim.hop_secs();
            } else {
                self.anim.playing = false;
                self.notice = Some("simulation is idle".into());
            }
        }
        // Ask for frames only while something moves or a step is due: an idle app repaints
        // nothing, which keeps idle CPU near zero.
        if self.anim.retire(self.wall) {
            ui.ctx().request_repaint();
        } else if self.anim.playing {
            let wait = (self.anim.next_step - self.wall).max(0.0);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        let keys = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (n, key) in keys.into_iter().enumerate() {
            if ui.input(|i| i.modifiers.alt && i.key_pressed(key)) {
                self.select(n);
            }
        }

        if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.set_tool(Tool::Select);
        }
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(&self.title);
                ui.separator();
                for i in 0..self.tabs.len() {
                    let label = format!("{} ({})", self.tabs[i].name, i + 1);
                    if ui.selectable_label(self.active == i, label).clicked() {
                        self.select(i);
                    }
                }
            });
        });
        egui::Panel::top("canvas")
            .resizable(true)
            .default_size(230.0)
            .show(ui, |ui| {
                self.tool_bar(ui);
                let active_node = self.tabs[self.active].node;
                let nodes: Vec<Node> = self
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| Node {
                        name: n.name.clone(),
                        kind: n.kind,
                        pos: n.pos,
                        selected: i == active_node || self.link_from == Some(i),
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
        egui::Panel::right("sim")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| self.sim_panel(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            let now = self.engine.world.kernel.now().0;
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("virtual time {:.3} ms", now as f64 / 1e6));
                ui.separator();
                if let Some(ms) = self.ready_ms {
                    ui.label(format!("ready in {ms:.0} ms"));
                    ui.separator();
                }
                ui.label(format!("frame {:.1} ms", self.frame_ms));
                ui.separator();
                match &self.notice {
                    Some(n) => ui.label(n),
                    None => ui.weak("click a device or Alt+1..9 to switch · Tab complete · ? help · scroll to zoom, drag to pan"),
                };
            });
        });
        let active = self.active;
        let focus = std::mem::take(&mut self.focus_pending);
        egui::CentralPanel::default().show(ui, |ui| {
            let t = &mut self.tabs[active];
            let id = ui.make_persistent_id(("terminal", t.device));
            let prompt = t.prompt.clone();
            let actions = widget::show(ui, id, &mut t.model, &prompt, focus);
            for a in actions {
                self.apply(active, a);
            }
        });
    }
}

/// Register the device's console kind with the engine and open its first terminal.
fn new_tab(engine: &mut Engine, d: &LabDevice, node: usize) -> Tab {
    let kind = match d.kind {
        LabKind::Router => ShellKind::Router,
        LabKind::Host => ShellKind::Host,
        LabKind::Switch => ShellKind::Switch,
    };
    engine.set_kind(d.id, kind);
    let (terminal, prompt) = open_terminal(engine, d.id.0);
    let mut model = TermModel::default();
    model.push_output(&format!(
        "{}: type ? for help, Tab to complete. Try `enable`, `configure terminal`.",
        d.name
    ));
    Tab {
        name: d.name.clone(),
        node,
        device: d.id.0,
        terminal,
        prompt,
        model,
    }
}

fn open_terminal(engine: &mut Engine, device: u32) -> (u32, String) {
    match engine.handle(EngineCmd::OpenTerminal { device }).remove(0) {
        EngineEvent::TerminalOpened { terminal, prompt } => (terminal, prompt),
        other => panic!("unexpected {other:?}"),
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        App::ui(self, ui);
    }
}
