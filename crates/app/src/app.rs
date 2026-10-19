//! The M0 spike app: the blank four-device chain, one terminal tab per device, driven through
//! the same `EngineCmd`/`EngineEvent` protocol the web build will use across a Worker.

use crate::term::TermModel;
use crate::widget::{self, TermAction};
use egui::{Key, Ui};
use engine::protocol::{Engine, EngineCmd, EngineEvent, ShellKind};

#[derive(Debug)]
struct Tab {
    name: &'static str,
    device: u32,
    terminal: u32,
    prompt: String,
    model: TermModel,
}

#[derive(Debug)]
pub struct App {
    engine: Engine,
    tabs: Vec<Tab>,
    active: usize,
    /// Set when a tab becomes active so its terminal grabs keyboard focus.
    focus_pending: bool,
    frame_ms: f32,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let c = engine::chain::blank_chain();
        let devices = [
            ("r1", c.r1, ShellKind::Router),
            ("r2", c.r2, ShellKind::Router),
            ("pc1", c.pc1, ShellKind::Host),
            ("pc2", c.pc2, ShellKind::Host),
        ];
        let mut engine = Engine::new(c.world);
        for (_, id, kind) in devices {
            engine = engine.with_kind(id, kind);
        }
        let mut tabs = Vec::new();
        for (name, id, _) in devices {
            let (terminal, prompt) = open_terminal(&mut engine, id.0);
            let mut model = TermModel::default();
            model.push_output(&format!(
                "{name}: type ? for help, Tab to complete. Try `enable`, `configure terminal`."
            ));
            tabs.push(Tab {
                name,
                device: id.0,
                terminal,
                prompt,
                model,
            });
        }
        App {
            engine,
            tabs,
            active: 0,
            focus_pending: true,
            frame_ms: 0.0,
        }
    }

    /// Currently selected tab's prompt, for tests.
    pub fn active_prompt(&self) -> &str {
        &self.tabs[self.active].prompt
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
        self.frame_ms = 0.9 * self.frame_ms + 0.1 * ui.input(|i| i.unstable_dt) * 1000.0;
        for (n, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4]
            .into_iter()
            .enumerate()
        {
            if ui.input(|i| i.modifiers.alt && i.key_pressed(key)) {
                self.select(n);
            }
        }

        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                for i in 0..self.tabs.len() {
                    let label = format!("{} ({})", self.tabs[i].name, i + 1);
                    if ui.selectable_label(self.active == i, label).clicked() {
                        self.select(i);
                    }
                }
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            let now = self.engine.world.kernel.now().0;
            ui.horizontal(|ui| {
                ui.label(format!("virtual time {:.3} ms", now as f64 / 1e6));
                ui.separator();
                ui.label(format!("frame {:.1} ms", self.frame_ms));
                ui.separator();
                ui.weak("Alt+1..4 switch device · Tab complete · ? help · Ctrl+U clear line");
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
