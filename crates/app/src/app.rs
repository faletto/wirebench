//! The app: a lab on a topology canvas plus one terminal tab per device that has a console,
//! driven through the same `EngineCmd`/`EngineEvent` protocol the web build will use across
//! a Worker.

use crate::canvas::{Canvas, Node};
use crate::term::TermModel;
use crate::widget::{self, TermAction};
use egui::{Key, Pos2, Ui};
use engine::lab::{BuiltLab, LabKind};
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
    /// One-line message in the status bar (lab warnings, "no console" hints).
    notice: Option<String>,
    /// Set when a tab becomes active so its terminal grabs keyboard focus.
    focus_pending: bool,
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
        let BuiltLab {
            world,
            title,
            devices,
            links,
            annotations,
            warnings,
        } = built;
        let mut engine = Engine::new(world);
        let mut nodes = Vec::new();
        let mut tabs: Vec<Tab> = Vec::new();
        for d in &devices {
            let shell = match d.kind {
                LabKind::Router => Some(ShellKind::Router),
                LabKind::Host => Some(ShellKind::Host),
                LabKind::Switch => Some(ShellKind::Switch),
            };
            let mut tab = None;
            if let Some(kind) = shell {
                engine = engine.with_kind(d.id, kind);
                let (terminal, prompt) = open_terminal(&mut engine, d.id.0);
                let mut model = TermModel::default();
                model.push_output(&format!(
                    "{}: type ? for help, Tab to complete. Try `enable`, `configure terminal`.",
                    d.name
                ));
                tab = Some(tabs.len());
                tabs.push(Tab {
                    name: d.name.clone(),
                    node: nodes.len(),
                    device: d.id.0,
                    terminal,
                    prompt,
                    model,
                });
            }
            nodes.push(NodeInfo {
                name: d.name.clone(),
                kind: d.kind,
                pos: d.pos,
                tab,
            });
        }
        let notice = (!warnings.is_empty())
            .then(|| format!("lab: {} (+{} more)", warnings[0], warnings.len() - 1));
        App {
            engine,
            title,
            nodes,
            links: links.iter().map(|l| (l.a, l.b)).collect(),
            notes: annotations.into_iter().map(|a| (a.text, a.pos)).collect(),
            tabs,
            active: 0,
            canvas: Canvas::default(),
            notice,
            focus_pending: true,
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
                let active_node = self.tabs[self.active].node;
                let nodes: Vec<Node> = self
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| Node {
                        name: n.name.clone(),
                        kind: n.kind,
                        pos: n.pos,
                        selected: i == active_node,
                        has_console: n.tab.is_some(),
                    })
                    .collect();
                if let Some(i) = self.canvas.show(ui, &nodes, &self.links, &self.notes) {
                    match self.nodes[i].tab {
                        Some(t) => {
                            self.notice = None;
                            self.select(t);
                            // Clicking the already-active device still returns focus to its terminal.
                            self.focus_pending = true;
                        }
                        None => {
                            self.notice = Some(format!("{} has no console yet", self.nodes[i].name))
                        }
                    }
                }
            });
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
