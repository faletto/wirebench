//! The app: a lab on a topology canvas plus one terminal tab per device that has a console,
//! driven through the same `EngineCmd`/`EngineEvent` protocol the web build will use across
//! a Worker.

use crate::anim::{caption, Anim, SPEEDS};
use crate::autosave::{age_text, now_unix_secs, Autosave, NullStore, Store};
use crate::canvas::{Canvas, CanvasEvent, FlightDraw, Node, Scene, Tool};
use crate::changes::{AutosaveState, Changes};
use crate::files::{self, Inbox, Incoming};
use crate::history::History;
use crate::inspector::Filter;
use crate::link::EngineLink;
use crate::settings::Panel;
use crate::sim::{parse_ipv4, HopRow, Sim};
use crate::term::TermModel;
use crate::widget::{self, TermAction};
use egui::{Key, Modifiers, Pos2, Ui};
use engine::lab::{BuiltLab, LabKind};
use engine::protocol::{DeviceView, EngineCmd, EngineEvent, LabView, ShellKind};

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

/// What the person was about to do when we stopped to ask about unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Open,
    /// Replace the lab with recovered autosave.
    Restore,
    Close,
}

/// Their answer to the unsaved-changes question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Save first, then carry on.
    Save,
    /// Throw the changes away and carry on.
    Discard,
    Cancel,
}

/// Where a device drag is, so one drag is one undo step however many frames it takes.
#[derive(Debug)]
enum Drag {
    None,
    /// The button went down on a device; the lab as it was is held until the device moves.
    Armed(labfile::LabFile),
    /// Already recorded for this drag.
    Recorded,
}

#[path = "settings_panel.rs"]
mod settings_panel;

#[derive(Debug)]
pub struct App {
    engine: EngineLink,
    title: String,
    nodes: Vec<NodeInfo>,
    links: Vec<(usize, usize)>,
    notes: Vec<(String, [i32; 2])>,
    tabs: Vec<Tab>,
    active: usize,
    canvas: Canvas,
    tool: Tool,
    /// First device of a cable being drawn with the Link tool.
    link_from: Option<usize>,
    /// One-line message in the status bar (lab warnings, "no console" hints).
    notice: Option<String>,
    /// Set when a tab becomes active so its terminal grabs keyboard focus.
    focus_pending: bool,
    /// The lab as lab-file text, as the engine last pushed it: what saving, dirty tracking,
    /// autosave and the undo history read, so none of them has to ask and wait.
    lab_text: String,
    sim: Sim,
    panel: Panel,
    anim: Anim,
    changes: Changes,
    history: History,
    drag: Drag,
    store: Box<dyn Store>,
    /// Unsaved work found in the autosave at startup, offered for recovery.
    recovery: Option<Autosave>,
    confirm: Option<Pending>,
    close_confirmed: bool,
    /// Egui context from the latest frame, so actions outside `ui` (a dialog answer, a file
    /// picker) can reach the window.
    ctx: Option<egui::Context>,
    /// Last window title sent, to send only changes.
    last_title: String,
    inbox: Inbox,
    /// Native: the path the Save and Open buttons use.
    file_path: String,
    /// Wall-clock seconds as of the latest frame (egui's clock; works on web too).
    wall: f64,
    /// The wall time at the last frame that advanced simulated time by itself (see `run_live_time`).
    live_at: Option<f64>,
    /// Simulated time and whether upkeep timers are pending, as the engine last said.
    sim_now: u64,
    background: bool,
    /// The person is stepping through something (a queued ping): the live clock must not run it
    /// off ahead of them.
    busy: bool,
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
        let (engine, view) = EngineLink::from_built(built);
        Self::assemble(engine, view)
    }

    /// An app around an engine that already holds the lab `view` describes.
    fn assemble(engine: EngineLink, view: LabView) -> Self {
        let warnings = view.warnings.clone();
        let lab_text = engine.snapshot();
        let mut nodes = Vec::new();
        let mut tabs: Vec<Tab> = Vec::new();
        for d in &view.devices {
            let tab = Some(tabs.len());
            tabs.push(new_tab(&engine, d, nodes.len()));
            nodes.push(NodeInfo {
                name: d.name.clone(),
                kind: lab_kind(d.kind),
                pos: d.pos,
                tab,
                device: d.device,
            });
        }
        let notice = (!warnings.is_empty())
            .then(|| format!("lab: {} (+{} more)", warnings[0], warnings.len() - 1));
        let mut app = App {
            engine,
            title: view.title.clone(),
            nodes,
            links: view.links.iter().map(|l| (l.a, l.b)).collect(),
            notes: view.notes.clone(),
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
            lab_text,
            panel: Panel::default(),
            anim: Anim::default(),
            changes: Changes::default(),
            history: History::default(),
            drag: Drag::None,
            store: Box::new(NullStore),
            recovery: None,
            confirm: None,
            close_confirmed: false,
            ctx: None,
            last_title: String::new(),
            inbox: Inbox::default(),
            file_path: String::new(),
            wall: 0.0,
            live_at: None,
            sim_now: 0,
            background: false,
            busy: false,
            frame_ms: 0.0,
            ready_ms: None,
        };
        app.changes = Changes::clean(app.export_lab());
        app.refresh_clock();
        app
    }

    /// Smoothed time spent building a frame, in milliseconds (for tests).
    pub fn frame_cost_ms(&self) -> f32 {
        self.frame_ms
    }

    /// Number of devices on the canvas.
    /// Simulated time so far, in nanoseconds.
    pub fn sim_time_ns(&self) -> u64 {
        self.sim_now
    }

    pub fn device_count(&self) -> usize {
        self.nodes.len()
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

    /// The lab as the engine has it now, parsed (for the undo history).
    fn snapshot_lab(&self) -> labfile::LabFile {
        labfile::load(&self.lab_text)
            .expect("the engine's snapshots load")
            .lab
    }

    /// Send an edit; the new view of the lab, or the reason it was refused.
    fn edit(&mut self, cmd: EngineCmd) -> Result<LabView, String> {
        let events = self.engine.send(cmd);
        absorb(&mut self.lab_text, &events);
        match events.into_iter().next() {
            Some(EngineEvent::Lab { view }) => Ok(view),
            Some(EngineEvent::Error { message }) => Err(message),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    /// Add a device at a canvas position and open its console.
    pub fn add_device(&mut self, kind: LabKind, pos: [i32; 2]) {
        let before = self.snapshot_lab();
        match self.edit(EngineCmd::AddDevice {
            kind: shell_kind(kind),
            pos,
        }) {
            Ok(view) => {
                let i = view.devices.len() - 1;
                let d = view.devices[i].clone();
                self.history.record(format!("add {}", d.name), before);
                self.tabs.push(new_tab(&self.engine, &d, i));
                self.nodes.push(NodeInfo {
                    name: d.name.clone(),
                    kind: lab_kind(d.kind),
                    pos,
                    tab: Some(self.tabs.len() - 1),
                    device: d.device,
                });
                self.changes.mark_edited(self.wall);
                self.notice = Some(format!("added {}", d.name));
                self.select(self.tabs.len() - 1);
            }
            Err(e) => self.notice = Some(e),
        }
    }

    pub fn move_node(&mut self, node: usize, pos: [i32; 2]) {
        if node < self.nodes.len() {
            match std::mem::replace(&mut self.drag, Drag::None) {
                Drag::Armed(before) => {
                    self.history
                        .record(format!("move {}", self.nodes[node].name), before);
                    self.drag = Drag::Recorded;
                }
                Drag::Recorded => self.drag = Drag::Recorded,
                Drag::None => {
                    let before = self.snapshot_lab();
                    self.history
                        .record(format!("move {}", self.nodes[node].name), before);
                }
            }
            self.nodes[node].pos = pos;
            let _ = self.edit(EngineCmd::MoveDevice { device: node, pos });
            self.changes.mark_edited(self.wall);
        }
    }

    /// Cable two devices on their first free ports.
    pub fn connect(&mut self, a: usize, b: usize) {
        let before = self.snapshot_lab();
        match self.edit(EngineCmd::AddLink { a, b }) {
            Ok(_) => {
                self.history.record(
                    format!("cable {} to {}", self.nodes[a].name, self.nodes[b].name),
                    before,
                );
                self.links.push((a, b));
                self.changes.mark_edited(self.wall);
                self.notice = None;
            }
            Err(e) => self.notice = Some(e),
        }
    }

    /// Remove a device and its cables. The world is rebuilt from the live configs, so
    /// terminals, ARP caches, MAC tables and virtual time restart.
    pub fn delete_device(&mut self, node: usize) {
        let Some(name) = self.nodes.get(node).map(|n| n.name.clone()) else {
            return;
        };
        let before = self.snapshot_lab();
        match self.edit(EngineCmd::RemoveDevice { device: node }) {
            Ok(view) => {
                self.adopt(view, format!("removed {name}; consoles restarted"));
                self.history.record(format!("remove {name}"), before);
            }
            Err(e) => self.notice = Some(e),
        }
    }

    /// Remove a cable (index into the cable list); rebuilds like [`App::delete_device`].
    pub fn delete_link(&mut self, link: usize) {
        let before = self.snapshot_lab();
        match self.edit(EngineCmd::RemoveLink { link }) {
            Ok(view) => {
                self.adopt(view, "removed a cable; consoles restarted".into());
                self.history.record("remove a cable", before);
            }
            Err(e) => self.notice = Some(e),
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Undo the latest edit. Like delete, this rebuilds the world, so consoles, ARP caches and
    /// virtual time restart; configuration typed since the edit is kept.
    pub fn undo(&mut self) {
        let current = self.snapshot_lab();
        let Some((label, lab)) = self.history.step_back(&current) else {
            self.notice = Some("nothing to undo".into());
            return;
        };
        if !self.rebuild(&lab, format!("undid {label}; consoles restarted")) {
            self.history.cancel_back(label, current);
        }
    }

    pub fn redo(&mut self) {
        let current = self.snapshot_lab();
        let Some((label, lab)) = self.history.step_forward(&current) else {
            self.notice = Some("nothing to redo".into());
            return;
        };
        if !self.rebuild(&lab, format!("redid {label}; consoles restarted")) {
            self.history.cancel_forward(label, current);
        }
    }

    /// Replace the world with one built from `lab`. False (and nothing changed) if it won't build.
    fn rebuild(&mut self, lab: &labfile::LabFile, note: String) -> bool {
        match self.edit(EngineCmd::Load {
            lab: labfile::save(lab),
        }) {
            Ok(view) => {
                self.adopt(view, note);
                true
            }
            Err(e) => {
                self.notice = Some(format!("could not rebuild the lab: {e}"));
                false
            }
        }
    }

    /// The engine now holds a different world (a rebuild after a delete or undo): start the
    /// screen over from `view`, keeping what belongs to the person, as the old rebuild did.
    fn adopt(&mut self, view: LabView, note: String) {
        let active = self.tabs.get(self.active).map(|t| t.name.clone());
        let engine = std::mem::replace(&mut self.engine, EngineLink::detached());
        let mut fresh = App::assemble(engine, view);
        fresh.take_session(self);
        fresh.canvas = std::mem::take(&mut self.canvas);
        fresh.changes = std::mem::take(&mut self.changes);
        fresh.history = std::mem::take(&mut self.history);
        fresh.changes.mark_edited(self.wall);
        fresh.notice = Some(note);
        if let Some(i) = active.and_then(|n| fresh.tabs.iter().position(|t| t.name == n)) {
            fresh.active = i;
        }
        *self = fresh;
    }

    /// Carry over what belongs to the person using the app rather than to the lab.
    fn take_session(&mut self, old: &mut App) {
        self.tool = old.tool;
        self.sim.dst = std::mem::take(&mut old.sim.dst);
        self.anim.enabled = old.anim.enabled;
        self.anim.speed = old.anim.speed;
        self.inbox = old.inbox.clone();
        self.store = std::mem::replace(&mut old.store, Box::new(NullStore));
        self.recovery = old.recovery.take();
        self.ctx = old.ctx.take();
        self.close_confirmed = old.close_confirmed;
        self.file_path = std::mem::take(&mut old.file_path);
        self.frame_ms = old.frame_ms;
        self.ready_ms = old.ready_ms;
        self.wall = old.wall;
    }

    // ---- files ----

    /// Set the file name Save and Open use (what the path field edits).
    pub fn set_file_path(&mut self, path: &str) {
        self.file_path = path.to_string();
    }

    /// Where asynchronous file arrivals (the browser's file picker) are queued.
    pub fn inbox(&self) -> Inbox {
        self.inbox.clone()
    }

    /// Replace the lab with the one in `text`. If it cannot be loaded, the current lab stays
    /// and the notice says why. Returns whether the new lab was adopted.
    pub fn load_lab_text(&mut self, text: &str, source: &str) -> bool {
        match self.engine.load(text) {
            Ok(view) => {
                let engine = std::mem::replace(&mut self.engine, EngineLink::detached());
                let mut fresh = App::assemble(engine, view);
                fresh.take_session(self);
                let problems = fresh.notice.take();
                fresh.notice = Some(match problems {
                    Some(p) => format!("opened {source} ({p})"),
                    None => format!("opened {source}"),
                });
                *self = fresh;
                true
            }
            Err(e) => {
                self.notice = Some(format!("could not open {source}: {e}"));
                false
            }
        }
    }

    /// Native: write the lab as it is now to `path`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_to_path(&mut self, path: &str) -> bool {
        match files::write_file(path, &self.export_lab()) {
            Ok(()) => {
                self.file_path = path.to_string();
                self.mark_saved();
                self.notice = Some(format!("saved {path}"));
                true
            }
            Err(e) => {
                self.notice = Some(format!("could not save {path}: {e}"));
                false
            }
        }
    }

    /// Native: replace the lab with the file at `path`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_path(&mut self, path: &str) -> bool {
        match files::read_file(path) {
            Ok(text) => {
                let ok = self.load_lab_text(&text, path);
                if ok {
                    self.file_path = path.to_string();
                    self.after_open();
                }
                ok
            }
            Err(e) => {
                self.notice = Some(format!("could not open {path}: {e}"));
                false
            }
        }
    }

    /// Web: hand the lab to the browser as a download.
    #[cfg(target_arch = "wasm32")]
    fn download_lab(&mut self) {
        let name = files::file_name_for(&self.title);
        self.notice = Some(match files::download(&name, &self.export_lab()) {
            Ok(()) => {
                self.mark_saved();
                format!("downloaded {name}")
            }
            Err(e) => format!("could not download: {e}"),
        });
    }

    fn file_controls(&mut self, ui: &mut Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            ui.add(
                egui::TextEdit::singleline(&mut self.file_path)
                    .desired_width(150.0)
                    .hint_text("lab.toml"),
            )
            .on_hover_text("File used by Save and Open");
            if ui.button("Save").clicked() {
                self.save_action();
            }
            if ui.button("Open").clicked() {
                self.request_open();
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if ui.button("Save").clicked() {
                self.save_action();
            }
            if ui.button("Open…").clicked() {
                self.request_open();
            }
        }
    }

    // ---- unsaved changes, autosave, shortcuts ----

    /// Whether the lab differs from what was last saved or opened.
    pub fn is_dirty(&self) -> bool {
        self.changes.is_dirty()
    }

    /// The question currently on screen, if any.
    pub fn pending_confirm(&self) -> Option<Pending> {
        self.confirm
    }

    /// Window title, with a marker while there are unsaved changes.
    pub fn title_text(&self) -> String {
        let dot = if self.is_dirty() { "● " } else { "" };
        format!("{dot}{} — wirebench", self.title)
    }

    /// Use `store` for autosaves, and offer to recover any unsaved work it holds from a
    /// previous session.
    pub fn set_store(&mut self, store: Box<dyn Store>) {
        let current = self.export_lab();
        self.recovery = store
            .load()
            .filter(|a| a.text != current && labfile::load(&a.text).is_ok());
        self.store = store;
    }

    /// "5 min ago" for the recoverable autosave, if there is one.
    pub fn recovery_age(&self) -> Option<String> {
        self.recovery
            .as_ref()
            .map(|r| age_text(r.unix_secs, now_unix_secs()))
    }

    /// Ask to replace the lab with the recovered work (confirms first if there are
    /// unsaved changes).
    pub fn restore_recovered(&mut self) {
        if self.recovery.is_none() {
            return;
        }
        if self.is_dirty() {
            self.confirm = Some(Pending::Restore);
        } else {
            self.do_restore();
        }
    }

    /// Delete the recoverable autosave.
    pub fn discard_recovered(&mut self) {
        if self.recovery.take().is_some() {
            self.store.clear();
        }
    }

    fn do_restore(&mut self) {
        let Some(a) = self.recovery.take() else {
            return;
        };
        if self.load_lab_text(&a.text, "recovered work") {
            // Exists nowhere but the autosave, so it stays unsaved until saved for real.
            let text = self.export_lab();
            self.changes.mark_recovered(text, self.wall);
            self.notice = Some("recovered unsaved work; Save to keep it".into());
        } else {
            self.recovery = Some(a);
        }
    }

    fn observe_changes(&mut self) {
        let text = self.export_lab();
        self.changes.observe(&text, self.wall);
    }

    /// The lab was just written out: it is the new baseline and the autosave is stale.
    fn mark_saved(&mut self) {
        let text = self.export_lab();
        self.changes.mark_saved(text);
        // Unresolved recovered work is still the only copy; leave it for the banner.
        if self.recovery.is_none() {
            self.store.clear();
        }
    }

    /// A different lab was opened from a file: the autosave of the old one is stale.
    fn after_open(&mut self) {
        if self.recovery.is_none() {
            self.store.clear();
        }
    }

    /// Save with the current file name (native) or as a download (web). Returns success.
    pub fn save_action(&mut self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.file_path.trim().is_empty() {
                self.file_path = files::file_name_for(&self.title);
            }
            let path = self.file_path.clone();
            self.save_to_path(&path)
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.download_lab();
            true
        }
    }

    /// Open a lab, asking first if there are unsaved changes.
    pub fn request_open(&mut self) {
        if self.is_dirty() {
            self.confirm = Some(Pending::Open);
        } else {
            self.do_open();
        }
    }

    fn do_open(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = self.file_path.clone();
            self.open_path(&path);
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(ctx) = self.ctx.clone() {
            if let Err(e) = files::pick(self.inbox.clone(), ctx) {
                self.notice = Some(format!("could not open the file picker: {e}"));
            }
        }
    }

    /// Answer the unsaved-changes question.
    pub fn confirm(&mut self, choice: Choice) {
        let Some(what) = self.confirm.take() else {
            return;
        };
        match choice {
            Choice::Cancel => {}
            Choice::Save => {
                if self.save_action() {
                    self.proceed(what);
                }
            }
            Choice::Discard => {
                if what == Pending::Close && self.recovery.is_none() {
                    self.store.clear();
                }
                self.proceed(what);
            }
        }
    }

    fn proceed(&mut self, what: Pending) {
        match what {
            Pending::Open => self.do_open(),
            Pending::Restore => self.do_restore(),
            Pending::Close => {
                self.close_confirmed = true;
                if let Some(ctx) = &self.ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    /// Write the autosave when it is due, or ask to be woken when it will be.
    fn autosave_tick(&mut self, ctx: &egui::Context) {
        // Never overwrite recovered work that is still waiting for a decision.
        if self.recovery.is_some() {
            return;
        }
        match self.changes.autosave_state(self.wall) {
            AutosaveState::Idle => {}
            AutosaveState::Waiting(secs) => {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(secs));
            }
            AutosaveState::Due => {
                let a = Autosave {
                    text: self.export_lab(),
                    unix_secs: now_unix_secs(),
                };
                if let Err(e) = self.store.save(&a) {
                    self.notice = Some(format!("autosave failed: {e}"));
                }
                // Even on failure: do not retry every frame.
                self.changes.autosaved();
            }
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(what) = self.confirm else {
            return;
        };
        let action = match what {
            Pending::Open => "open another lab",
            Pending::Restore => "replace it with the recovered work",
            Pending::Close => "close wirebench",
        };
        let mut choice = None;
        let resp = egui::Modal::new(egui::Id::unique("unsaved-changes")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label(format!(
                "\"{}\" has changes that are not saved. Save them before you {action}?",
                self.title
            ));
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.button("Discard changes").clicked() {
                    choice = Some(Choice::Discard);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        if resp.should_close() {
            choice = choice.or(Some(Choice::Cancel));
        }
        if let Some(c) = choice {
            self.confirm(c);
        }
    }

    fn recovery_banner(&mut self, ui: &mut Ui) {
        let Some(age) = self.recovery_age() else {
            return;
        };
        egui::Panel::top("recovery").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Unsaved work was recovered");
                ui.label(format!("(autosaved {age})"));
                if ui.button("Restore").clicked() {
                    self.restore_recovered();
                }
                if ui.button("Discard").clicked() {
                    self.discard_recovered();
                }
            });
        });
    }

    /// Apply files that arrived asynchronously (the browser's file picker).
    fn drain_inbox(&mut self) {
        for item in self.inbox.take() {
            match item {
                Incoming::Lab { name, text } => {
                    if self.load_lab_text(&text, &name) {
                        self.after_open();
                    }
                }
                Incoming::Failed(why) => self.notice = Some(why),
            }
        }
    }

    /// The lab as it is now (positions, cabling, live configs) as lab-file text.
    pub fn export_lab(&self) -> String {
        self.lab_text.clone()
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
                CanvasEvent::DragStart => {
                    self.drag = Drag::Armed(self.snapshot_lab());
                }
                CanvasEvent::DragEnd => self.drag = Drag::None,
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
            ui.separator();
            let undo = self.history.undo_label().map(|l| format!("Undo {l}"));
            let redo = self.history.redo_label().map(|l| format!("Redo {l}"));
            let r = ui.add_enabled(self.can_undo(), egui::Button::new("Undo"));
            if r.on_hover_text(undo.unwrap_or_else(|| "nothing to undo".into()))
                .clicked()
            {
                self.undo();
            }
            let r = ui.add_enabled(self.can_redo(), egui::Button::new("Redo"));
            if r.on_hover_text(redo.unwrap_or_else(|| "nothing to redo".into()))
                .clicked()
            {
                self.redo();
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
        self.push_mark();
        self.busy = true;
        let ev = self.engine.send(EngineCmd::PingOnce { device, dst });
        self.apply_sim(ev, false);
    }

    /// Dispatch one event. Returns false once nothing is left to do.
    pub fn sim_step(&mut self) -> bool {
        self.step_inner(false)
    }

    fn step_inner(&mut self, animate: bool) -> bool {
        self.push_mark();
        let ev = self.engine.send(EngineCmd::Step);
        let more = !matches!(ev.first(), Some(EngineEvent::Idle { .. }));
        if !more {
            self.sim.marks.pop();
        }
        self.apply_sim(ev, animate);
        more
    }

    /// Note how long the log is before something the engine can undo. The engine keeps a limited
    /// number of states; so do the marks, or they would drift out of step with it.
    fn push_mark(&mut self) {
        self.sim.marks.push(self.sim.hops.len());
        if self.sim.marks.len() > engine::protocol::HISTORY_LIMIT {
            self.sim.marks.remove(0);
        }
    }

    /// Undo the latest step or queued ping exactly (the world is restored from a copy). Returns
    /// false when there is nothing to go back to.
    pub fn sim_back(&mut self) -> bool {
        self.anim.playing = false;
        let ev = self.engine.send(EngineCmd::StepBack);
        self.note_clock(&ev);
        if !matches!(ev.first(), Some(EngineEvent::Rewound { .. })) {
            return false;
        }
        self.refresh_clock();
        let keep = self.sim.marks.pop().unwrap_or(0);
        self.sim.hops.truncate(keep);
        self.busy = !self.sim.marks.is_empty();
        self.anim.flights.clear();
        if self.sim.selected.is_some_and(|i| i >= keep) {
            self.sim.selected = None;
            self.sim.field = None;
        }
        true
    }

    pub fn sim_can_back(&self) -> bool {
        !self.sim.marks.is_empty()
    }

    /// Open a row of the hop log in the detail view.
    pub fn sim_select(&mut self, row: Option<usize>) {
        self.sim.selected = row.filter(|i| *i < self.sim.hops.len());
        self.sim.field = None;
    }

    pub fn sim_selected(&self) -> Option<usize> {
        self.sim.selected
    }

    pub fn sim_set_filter(&mut self, text: &str) {
        self.sim.filter = text.to_string();
    }

    /// Indices of the hop rows the filter lets through.
    pub fn sim_visible(&self) -> Vec<usize> {
        let f = Filter::parse(&self.sim.filter);
        (0..self.sim.hops.len())
            .filter(|i| f.matches(&self.sim.hops[*i]))
            .collect()
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
        self.note_clock(&events);
        for e in events {
            match e {
                EngineEvent::Hop {
                    ns,
                    device,
                    port,
                    summary,
                    from,
                    frame,
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
                    let layers = wire::dissect(&frame);
                    let protocols = wire::protocols(&layers);
                    self.sim.hops.push(HopRow {
                        ns,
                        device: name,
                        port,
                        summary,
                        frame,
                        layers,
                        protocols,
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
            let back = ui.add_enabled(self.sim_can_back(), egui::Button::new("Back"));
            if back.on_hover_text("Undo the last step").clicked() {
                self.sim_back();
            }
            if ui.button("Clear").clicked() {
                self.sim.hops.clear();
                self.sim.marks.clear();
                self.busy = false;
                self.sim.selected = None;
                self.sim.field = None;
                self.engine.send(EngineCmd::ForgetHistory);
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
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.sim.filter)
                    .hint_text("icmp dev:r1 -arp")
                    .desired_width(f32::INFINITY),
            );
        });
        let visible = self.sim_visible();
        let mut clicked = None;
        let list_height = (ui.available_height() * 0.45).max(80.0);
        egui::ScrollArea::vertical()
            .id_salt("hop_list")
            .max_height(list_height)
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.sim.hops.is_empty() {
                    ui.weak("Queue a ping, then Step to follow each frame.");
                } else if visible.is_empty() {
                    ui.weak("No frame matches the filter.");
                }
                for &i in &visible {
                    let h = &self.sim.hops[i];
                    let line = format!(
                        "{:>9.3} ms  {}:{}  {}",
                        h.ns as f64 / 1e6,
                        h.device,
                        h.port,
                        h.summary
                    );
                    if ui
                        .selectable_label(self.sim.selected == Some(i), line)
                        .clicked()
                    {
                        clicked = Some(i);
                    }
                }
            });
        if let Some(i) = clicked {
            self.sim_select(Some(i));
        }
        ui.separator();
        match self.sim.selected.and_then(|i| self.sim.hops.get(i)) {
            Some(h) => {
                let (frame, layers) = (h.frame.clone(), h.layers.clone());
                ui.label(format!(
                    "{} bytes, delivered to {} port {}",
                    frame.len(),
                    h.device,
                    h.port
                ));
                let mut picked = None;
                egui::ScrollArea::vertical()
                    .id_salt("hop_detail")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (n, layer) in layers.iter().enumerate() {
                            node_tree(ui, layer, &format!("{n}"), &self.sim.field, &mut picked);
                        }
                        ui.add_space(6.0);
                        hex_view(ui, &frame, self.sim.field.as_ref());
                    });
                if let Some(r) = picked {
                    self.sim.field = Some(r);
                }
            }
            None => {
                ui.weak("Select a frame to see its fields and bytes.");
            }
        }
    }

    /// Run one terminal action against the engine and print the result.
    pub fn apply(&mut self, tab: usize, action: TermAction) {
        let is_submit = matches!(action, TermAction::Submit(_));
        let t = &mut self.tabs[tab];
        let terminal = t.terminal;
        match action {
            TermAction::Submit(line) => {
                // What is typed at a password prompt is not echoed.
                if t.prompt == engine::shell::PASSWORD_PROMPT {
                    t.model.push_output(&t.prompt);
                } else {
                    t.model.push_output(&format!("{}{line}", t.prompt));
                }
                self.sim.marks.clear();
                self.panel.stale = true;
                let events = self.engine.send(EngineCmd::Exec { terminal, line });
                absorb(&mut self.lab_text, &events);
                let reply = events
                    .into_iter()
                    .find(|e| !matches!(e, EngineEvent::LabText { .. }));
                // A command may have started spanning tree, which keeps its own time.
                if let Some(EngineEvent::Time { ns, background }) =
                    self.engine.send(EngineCmd::Now).pop()
                {
                    self.sim_now = ns;
                    self.background = background;
                }
                match reply {
                    Some(EngineEvent::Output { text, prompt, .. }) => {
                        t.model.push_output(&text);
                        t.prompt = prompt;
                    }
                    Some(EngineEvent::Closed { .. }) => {
                        t.model.push_output("[session closed; opened a new one]");
                        let (terminal, prompt, greeting) = open_terminal(&self.engine, t.device);
                        if !greeting.is_empty() {
                            t.model.push_output(&greeting);
                        }
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
                    self.engine.send(EngineCmd::Help { terminal, line }).pop()
                {
                    t.model.push_output(&text);
                }
            }
            TermAction::Tab(line) => {
                if let Some(EngineEvent::Completion { line, .. }) =
                    self.engine.send(EngineCmd::Tab { terminal, line }).pop()
                {
                    t.model.set_input(&line);
                }
            }
        }
        // A command may have changed a config; only a real change makes the lab dirty.
        if is_submit {
            self.observe_changes();
        }
    }

    /// Draw the whole UI. Takes only a `Ui` so headless tests can drive it.
    /// Spanning tree and its kind keep time themselves; the rest of the simulation only moves when
    /// something happens. While such timers are pending, let simulated time pass as real time
    /// does, so that waiting at a console works the way it does on a real switch.
    fn run_live_time(&mut self, ui: &Ui) {
        // While a ping is being stepped through, time must not run on ahead of the person stepping.
        if !self.background || self.busy || self.anim.playing {
            self.live_at = None;
            return;
        }
        if let Some(last) = self.live_at.replace(self.wall) {
            // A long gap (a hidden tab) is not made up for.
            let dt = (self.wall - last).clamp(0.0, 0.25);
            let ev = self.engine.send(EngineCmd::Advance {
                ns: (dt * 1e9) as u64,
            });
            self.note_clock(&ev);
            // Time has moved on behind the engine's saved states: they are stale.
            self.sim.marks.clear();
            self.engine.send(EngineCmd::ForgetHistory);
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// Remember the clock from any answer that carries it.
    fn note_clock(&mut self, events: &[EngineEvent]) {
        for e in events {
            match e {
                EngineEvent::Time { ns, background } => {
                    self.sim_now = *ns;
                    self.background = *background;
                }
                // Stepping delivers what was queued; the engine says when nothing is left.
                EngineEvent::Hop { ns, .. } | EngineEvent::Tick { ns, .. } => {
                    self.sim_now = *ns;
                    self.busy = true;
                }
                EngineEvent::Idle { ns } => {
                    self.sim_now = *ns;
                    self.busy = false;
                }
                EngineEvent::Rewound { ns, .. } => self.sim_now = *ns,
                _ => {}
            }
        }
    }

    /// Ask the engine for the time (after a console command, which may have started spanning tree).
    fn refresh_clock(&mut self) {
        let ev = self.engine.send(EngineCmd::Now);
        self.note_clock(&ev);
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let began = crate::clock::since_start_ms();
        if self.ready_ms.is_none() {
            self.ready_ms = crate::clock::since_start_ms();
            if let Some(ms) = self.ready_ms {
                crate::clock::log(&format!("wirebench: first frame after {ms:.0} ms"));
            }
        }
        self.wall = ui.input(|i| i.time);
        self.ctx = Some(ui.ctx().clone());
        self.drain_inbox();
        // Ctrl/Cmd+S saves and Ctrl/Cmd+O opens, taken before any widget can see them (the web
        // build also stops the browser's own Save Page / Open File dialogs).
        if self.confirm.is_none() {
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
                self.save_action();
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O)) {
                self.request_open();
            }
            // Redo is checked first: Cmd+Shift+Z also satisfies Cmd+Z.
            let redo_keys = Modifiers::COMMAND | Modifiers::SHIFT;
            if ui.input_mut(|i| {
                i.consume_key(redo_keys, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y)
            }) {
                self.redo();
            } else if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
                self.undo();
            }
        }
        // Closing the window with unsaved changes asks first.
        if ui.input(|i| i.viewport().close_requested()) && self.is_dirty() && !self.close_confirmed
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Pending::Close);
        }
        self.run_live_time(ui);
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
                if self.is_dirty() {
                    ui.label("● unsaved changes");
                }
                ui.separator();
                self.file_controls(ui);
                ui.separator();
                for i in 0..self.tabs.len() {
                    let label = format!("{} ({})", self.tabs[i].name, i + 1);
                    if ui.selectable_label(self.active == i, label).clicked() {
                        self.select(i);
                    }
                }
            });
        });
        self.recovery_banner(ui);
        egui::Panel::top("canvas")
            .resizable(true)
            .default_size(230.0)
            .show(ui, |ui| {
                self.tool_bar(ui);
                let active_node = self.tabs.get(self.active).map(|t| t.node);
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
        egui::Panel::left("settings")
            .resizable(true)
            .default_size(250.0)
            .show(ui, |ui| self.settings_panel(ui));
        egui::Panel::right("sim")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| self.sim_panel(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            let now = self.sim_now;
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("virtual time {:.3} ms", now as f64 / 1e6));
                ui.separator();
                if let Some(ms) = self.ready_ms {
                    ui.label(format!("ready in {ms:.0} ms"));
                    ui.separator();
                }
                ui.label(format!("frame cost {:.1} ms", self.frame_ms));
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
            let Some(t) = self.tabs.get_mut(active) else {
                ui.weak("No devices. Use + PC, + Switch or + Router, then click the canvas.");
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
        });
        let ctx = ui.ctx().clone();
        self.confirm_dialog(&ctx);
        self.autosave_tick(&ctx);
        // Window and page chrome: a marker in the title, and the browser's leave-page warning.
        let title = self.title_text();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            files::set_page_title(&title);
            self.last_title = title;
        }
        files::set_unsaved(self.is_dirty());
        // Time spent building this frame. egui repaints only on events, so the gap between frames
        // (`unstable_dt`) measures how long the user paused, not how much work a frame takes.
        if let (Some(a), Some(b)) = (began, crate::clock::since_start_ms()) {
            self.frame_ms = 0.9 * self.frame_ms + 0.1 * (b - a) as f32;
        }
    }
}

/// Open the device's first terminal.
fn new_tab(engine: &EngineLink, d: &DeviceView, node: usize) -> Tab {
    let (terminal, prompt, greeting) = open_terminal(engine, d.device);
    let mut model = TermModel::default();
    model.push_output(&format!(
        "{}: type ? for help, Tab to complete. Try `enable`, `configure terminal`.",
        d.name
    ));
    if !greeting.is_empty() {
        model.push_output(&greeting);
    }
    Tab {
        name: d.name.clone(),
        node,
        device: d.device,
        terminal,
        prompt,
        model,
    }
}

fn open_terminal(engine: &EngineLink, device: u32) -> (u32, String, String) {
    match engine.send(EngineCmd::OpenTerminal { device }).remove(0) {
        EngineEvent::TerminalOpened {
            terminal,
            prompt,
            greeting,
        } => (terminal, prompt, greeting),
        other => panic!("unexpected {other:?}"),
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        App::ui(self, ui);
    }
}

/// One layer or field of a dissected frame. Clicking a line picks its bytes for the hex view.
fn node_tree(
    ui: &mut Ui,
    node: &wire::Node,
    path: &str,
    picked_now: &Option<std::ops::Range<usize>>,
    picked: &mut Option<std::ops::Range<usize>>,
) {
    let selected = picked_now.as_ref() == Some(&node.range);
    if node.children.is_empty() {
        if ui.selectable_label(selected, &node.label).clicked() {
            *picked = Some(node.range.clone());
        }
        return;
    }
    let open = egui::CollapsingHeader::new(&node.label)
        .id_salt(path)
        .default_open(true)
        .show(ui, |ui| {
            for (i, c) in node.children.iter().enumerate() {
                node_tree(ui, c, &format!("{path}.{i}"), picked_now, picked);
            }
        });
    if open.header_response.clicked() {
        *picked = Some(node.range.clone());
    }
}

/// The frame in hex, sixteen bytes to a row, with the picked bytes highlighted.
fn hex_view(ui: &mut Ui, frame: &[u8], picked: Option<&std::ops::Range<usize>>) {
    use egui::text::{LayoutJob, TextFormat};
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let plain = TextFormat::simple(font.clone(), ui.visuals().text_color());
    let mut hot = plain.clone();
    hot.background = ui.visuals().selection.bg_fill;
    hot.color = ui.visuals().selection.stroke.color;
    for (row, chunk) in frame.chunks(16).enumerate() {
        let mut job = LayoutJob::default();
        job.append(&format!("{:04x}  ", row * 16), 0.0, plain.clone());
        for (j, b) in chunk.iter().enumerate() {
            let at = row * 16 + j;
            let on = picked.is_some_and(|r| r.contains(&at));
            let gap = if j == 7 { "  " } else { " " };
            job.append(
                &format!("{b:02x}"),
                0.0,
                if on { hot.clone() } else { plain.clone() },
            );
            job.append(gap, 0.0, plain.clone());
        }
        for j in chunk.len()..16 {
            job.append(if j == 7 { "    " } else { "   " }, 0.0, plain.clone());
        }
        job.append(" ", 0.0, plain.clone());
        for (j, b) in chunk.iter().enumerate() {
            let at = row * 16 + j;
            let on = picked.is_some_and(|r| r.contains(&at));
            let ch = if (0x20..0x7f).contains(b) {
                *b as char
            } else {
                '.'
            };
            job.append(
                &ch.to_string(),
                0.0,
                if on { hot.clone() } else { plain.clone() },
            );
        }
        ui.label(job);
    }
}

fn lab_kind(k: ShellKind) -> LabKind {
    match k {
        ShellKind::Router => LabKind::Router,
        ShellKind::Host => LabKind::Host,
        ShellKind::Switch => LabKind::Switch,
    }
}

fn shell_kind(k: LabKind) -> ShellKind {
    match k {
        LabKind::Router => ShellKind::Router,
        LabKind::Host => ShellKind::Host,
        LabKind::Switch => ShellKind::Switch,
    }
}

/// Keep the copy of the lab text current from whatever the engine pushed.
fn absorb(cache: &mut String, events: &[EngineEvent]) {
    for e in events {
        if let EngineEvent::LabText { text } = e {
            cache.clone_from(text);
        }
    }
}
