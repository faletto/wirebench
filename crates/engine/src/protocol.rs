//! The command/event protocol between a UI and the engine. On native the UI calls
//! [`Engine::handle`] directly; on the web the same messages cross a Web Worker boundary
//! as postcard bytes ([`Engine::handle_bytes`]). Either way the UI never touches a `World`.

use crate::lab::{build, load_and_build, BuiltLab, LabKind, Topology};
use crate::shell::Shell;
use crate::World;
use kernel::{DeviceId, SimTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ShellKind {
    Router,
    Host,
    Switch,
}

/// What the UI needs to know about a lab to draw it: no devices, just names, places and cables.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct LabView {
    pub title: String,
    pub devices: Vec<DeviceView>,
    /// Cables between indices of `devices`, with the 0-based port on each end.
    pub links: Vec<LinkView>,
    pub notes: Vec<(String, [i32; 2])>,
    /// Non-fatal problems found while building the lab.
    pub warnings: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DeviceView {
    pub name: String,
    pub kind: ShellKind,
    pub pos: [i32; 2],
    pub ports: u16,
    /// The id every other message uses for this device.
    pub device: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LinkView {
    pub a: usize,
    pub b: usize,
    pub pa: u16,
    pub pb: u16,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum EngineCmd {
    /// Replace everything with the lab in this text (the lab file format). Consoles close.
    Load {
        lab: String,
    },
    /// Add a device of this kind at this place; it gets the next free name.
    AddDevice {
        kind: ShellKind,
        pos: [i32; 2],
    },
    MoveDevice {
        device: usize,
        pos: [i32; 2],
    },
    /// Cable the first free port on each device (indices into the lab view).
    AddLink {
        a: usize,
        b: usize,
    },
    /// Delete a device and its cables. The world is rebuilt, so consoles close.
    RemoveDevice {
        device: usize,
    },
    RemoveLink {
        link: usize,
    },
    /// The lab as it stands, configurations included, as lab file text.
    Snapshot,
    /// What time it is in the simulation (answers with `Time`).
    Now,
    /// How a device's settings stand now (answers with `DeviceState`).
    DeviceState {
        device: u32,
    },
    /// Type these lines at the device as a person in configuration mode would (a PC: at its
    /// prompt). Answers with `Configured`.
    Configure {
        device: u32,
        lines: Vec<String>,
    },
    OpenTerminal {
        device: u32,
    },
    Exec {
        terminal: u32,
        line: String,
    },
    Help {
        terminal: u32,
        line: String,
    },
    Tab {
        terminal: u32,
        line: String,
    },
    /// Advance virtual time by this many nanoseconds, processing events.
    Advance {
        ns: u64,
    },
    /// Queue one echo request from a PC without advancing time (simulation mode).
    PingOnce {
        device: u32,
        dst: [u8; 4],
    },
    /// Dispatch exactly one simulation event.
    Step,
    /// Undo the latest `Step` or `PingOnce`, exactly: the world goes back to how it was.
    StepBack,
    /// Drop the saved states `StepBack` uses (after the world was changed some other way).
    ForgetHistory,
    /// Run until no events remain.
    RunToIdle,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum EngineEvent {
    /// The lab changed (loaded, edited): this is how it looks now.
    Lab {
        view: LabView,
    },
    State {
        device: u32,
        state: crate::panel::DeviceState,
    },
    /// What the CLI said about the lines of a `Configure`; empty means all went through.
    Configured {
        device: u32,
        said: Vec<String>,
    },
    /// Answer to `Snapshot`.
    LabText {
        text: String,
    },
    TerminalOpened {
        terminal: u32,
        prompt: String,
        /// Text to show before the prompt: the login banner and, if the console needs a
        /// password, the notice before the password prompt. Often empty.
        greeting: String,
    },
    Output {
        terminal: u32,
        text: String,
        prompt: String,
    },
    Completion {
        terminal: u32,
        line: String,
    },
    Closed {
        terminal: u32,
    },
    Time {
        ns: u64,
        /// Periodic upkeep (spanning tree) is scheduled, so time only moves if someone moves it.
        background: bool,
    },
    Error {
        message: String,
    },
    /// A frame arrived at `device`/`port`; `summary` is the inspector's one-line view.
    Hop {
        ns: u64,
        device: u32,
        port: u16,
        summary: String,
        bytes: u32,
        /// The frame itself, for the inspector.
        frame: Vec<u8>,
        /// Sending device and the virtual time its first bit left, for animation.
        from: u32,
        sent_ns: u64,
    },
    /// `StepBack` restored the previous state; `left` more steps can be undone.
    Rewound {
        ns: u64,
        left: u32,
    },
    /// A timer fired (nothing to draw).
    Tick {
        ns: u64,
        device: u32,
    },
    /// `Step` found nothing left to do.
    Idle {
        ns: u64,
    },
}

pub fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    postcard::to_allocvec(v).expect("serialisable")
}

pub fn decode<'a, T: Deserialize<'a>>(b: &'a [u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(b)
}

pub struct Engine {
    pub world: World,
    kinds: BTreeMap<u32, ShellKind>,
    terminals: Vec<Option<Shell>>,
    /// The world as it was before each of the latest steps, oldest first.
    history: Vec<World>,
    /// The lab behind the world, when the engine was made from one.
    topology: Option<Topology>,
}

fn shell_kind(k: LabKind) -> ShellKind {
    match k {
        LabKind::Router => ShellKind::Router,
        LabKind::Host => ShellKind::Host,
        LabKind::Switch => ShellKind::Switch,
    }
}

fn lab_kind(k: ShellKind) -> LabKind {
    match k {
        ShellKind::Router => LabKind::Router,
        ShellKind::Host => LabKind::Host,
        ShellKind::Switch => LabKind::Switch,
    }
}

/// How many steps can be undone. Each is a copy of the world.
pub const HISTORY_LIMIT: usize = 64;

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("terminals", &self.terminals.len())
            .finish()
    }
}

impl Engine {
    pub fn new(world: World) -> Self {
        Engine {
            world,
            kinds: BTreeMap::new(),
            terminals: Vec::new(),
            history: Vec::new(),
            topology: None,
        }
    }

    /// An engine for a built lab, with a console for each device.
    pub fn from_built(built: BuiltLab) -> (Engine, LabView) {
        let mut e = Engine::new(World::new(kernel::Trace::new()));
        let view = e.install(built);
        (e, view)
    }

    /// Replace the world and topology with `built`'s. Consoles close and saved states go.
    fn install(&mut self, built: BuiltLab) -> LabView {
        let (world, topo, warnings) = built.into_parts();
        self.world = world;
        self.kinds.clear();
        for d in &topo.devices {
            self.kinds.insert(d.id.0, shell_kind(d.kind));
        }
        self.terminals.clear();
        self.history.clear();
        let view = Self::view_of(&topo, warnings);
        self.topology = Some(topo);
        view
    }

    fn view_of(topo: &Topology, warnings: Vec<String>) -> LabView {
        LabView {
            title: topo.title.clone(),
            devices: topo
                .devices
                .iter()
                .map(|d| DeviceView {
                    name: d.name.clone(),
                    kind: shell_kind(d.kind),
                    pos: d.pos,
                    ports: d.ports,
                    device: d.id.0,
                })
                .collect(),
            links: topo
                .links
                .iter()
                .map(|l| LinkView {
                    a: l.a,
                    b: l.b,
                    pa: l.pa,
                    pb: l.pb,
                })
                .collect(),
            notes: topo
                .annotations
                .iter()
                .map(|a| (a.text.clone(), a.pos))
                .collect(),
            warnings,
        }
    }

    /// Edits to the lab: run `f` on the topology and world, then report the lab as it is now.
    fn edit(
        &mut self,
        f: impl FnOnce(&mut Topology, &mut World) -> Result<(), String>,
    ) -> Vec<EngineEvent> {
        let Some(topo) = self.topology.as_mut() else {
            return vec![EngineEvent::Error {
                message: "no lab loaded".into(),
            }];
        };
        match f(topo, &mut self.world) {
            Ok(()) => {
                self.history.clear();
                for d in &topo.devices {
                    self.kinds.insert(d.id.0, shell_kind(d.kind));
                }
                vec![EngineEvent::Lab {
                    view: Self::view_of(topo, vec![]),
                }]
            }
            Err(message) => vec![EngineEvent::Error { message }],
        }
    }

    /// Delete by rebuilding from the lab without the thing deleted.
    fn rebuild_without(
        &mut self,
        f: impl FnOnce(&Topology, &World) -> Result<labfile::LabFile, crate::lab::EditError>,
    ) -> Vec<EngineEvent> {
        let Some(topo) = self.topology.as_ref() else {
            return vec![EngineEvent::Error {
                message: "no lab loaded".into(),
            }];
        };
        let err = |m: String| vec![EngineEvent::Error { message: m }];
        let lab = match f(topo, &self.world) {
            Ok(l) => l,
            Err(e) => return err(e.to_string()),
        };
        match build(&lab, kernel::Trace::new()) {
            Ok(b) => vec![EngineEvent::Lab {
                view: self.install(b),
            }],
            Err(e) => err(e.to_string()),
        }
    }

    fn remember(&mut self) {
        match self.world.snapshot() {
            Some(w) => {
                self.history.push(w);
                if self.history.len() > HISTORY_LIMIT {
                    self.history.remove(0);
                }
            }
            None => self.history.clear(),
        }
    }

    pub fn with_kind(mut self, device: DeviceId, kind: ShellKind) -> Self {
        self.set_kind(device, kind);
        self
    }

    /// Declare what console a device gets (needed for devices added after construction).
    pub fn set_kind(&mut self, device: DeviceId, kind: ShellKind) {
        self.kinds.insert(device.0, kind);
    }

    pub fn handle(&mut self, cmd: EngineCmd) -> Vec<EngineEvent> {
        let err = |m: &str| {
            vec![EngineEvent::Error {
                message: m.to_string(),
            }]
        };
        match cmd {
            EngineCmd::Load { lab } => match load_and_build(&lab, kernel::Trace::new()) {
                Ok(b) => vec![EngineEvent::Lab {
                    view: self.install(b),
                }],
                Err(message) => vec![EngineEvent::Error { message }],
            },
            EngineCmd::AddDevice { kind, pos } => self.edit(|t, w| {
                t.add_device(w, lab_kind(kind), pos)
                    .map(drop)
                    .map_err(|e| e.to_string())
            }),
            EngineCmd::MoveDevice { device, pos } => self.edit(|t, _| {
                t.move_device(device, pos);
                Ok(())
            }),
            EngineCmd::AddLink { a, b } => {
                self.edit(|t, w| t.add_link(w, a, b).map(drop).map_err(|e| e.to_string()))
            }
            EngineCmd::RemoveDevice { device } => {
                self.rebuild_without(|t, w| t.without_device(w, device))
            }
            EngineCmd::RemoveLink { link } => self.rebuild_without(|t, w| t.without_link(w, link)),
            EngineCmd::Now => vec![EngineEvent::Time {
                ns: self.world.kernel.now().0,
                background: self.world.kernel.has_background(),
            }],
            EngineCmd::DeviceState { device } => match self.kinds.get(&device) {
                Some(kind) => vec![EngineEvent::State {
                    device,
                    state: crate::panel::state(&self.world, *kind, DeviceId(device)),
                }],
                None => err("no such device"),
            },
            EngineCmd::Configure { device, lines } => match self.kinds.get(&device).copied() {
                Some(kind) => {
                    self.history.clear();
                    let said =
                        crate::panel::configure(&mut self.world, kind, DeviceId(device), &lines);
                    vec![EngineEvent::Configured { device, said }]
                }
                None => err("no such device"),
            },
            EngineCmd::Snapshot => match &self.topology {
                Some(t) => vec![EngineEvent::LabText {
                    text: labfile::save(&t.snapshot(&self.world)),
                }],
                None => err("no lab loaded"),
            },
            EngineCmd::OpenTerminal { device } => {
                let id = DeviceId(device);
                let mut shell = match self.kinds.get(&device) {
                    Some(ShellKind::Router) => Shell::router(id),
                    Some(ShellKind::Host) => Shell::host(id),
                    Some(ShellKind::Switch) => Shell::switch(id),
                    None => return err("device has no shell"),
                };
                let greeting = shell.greeting(&self.world);
                let prompt = shell.prompt(&self.world);
                self.terminals.push(Some(shell));
                vec![EngineEvent::TerminalOpened {
                    terminal: self.terminals.len() as u32 - 1,
                    prompt,
                    greeting,
                }]
            }
            EngineCmd::Exec { terminal, line } => {
                let Some(Some(sh)) = self.terminals.get_mut(terminal as usize) else {
                    return err("no such terminal");
                };
                // A console command changes the world for good; stepping back past it would undo it.
                self.history.clear();
                match sh.exec(&mut self.world, &line) {
                    Some(text) => {
                        let prompt = sh.prompt(&self.world);
                        vec![EngineEvent::Output {
                            terminal,
                            text,
                            prompt,
                        }]
                    }
                    None => {
                        self.terminals[terminal as usize] = None;
                        vec![EngineEvent::Closed { terminal }]
                    }
                }
            }
            EngineCmd::Help { terminal, line } => match self.terminals.get(terminal as usize) {
                Some(Some(sh)) => {
                    vec![EngineEvent::Output {
                        terminal,
                        text: sh.help(&line),
                        prompt: sh.prompt(&self.world),
                    }]
                }
                _ => err("no such terminal"),
            },
            EngineCmd::Tab { terminal, line } => match self.terminals.get(terminal as usize) {
                Some(Some(sh)) => vec![EngineEvent::Completion {
                    terminal,
                    line: sh.tab(&line),
                }],
                _ => err("no such terminal"),
            },
            EngineCmd::PingOnce { device, dst } => {
                if self.kinds.get(&device) != Some(&ShellKind::Host) {
                    return err("only a PC can send a ping");
                }
                self.remember();
                self.world
                    .with_device::<netdev::Host, _>(DeviceId(device), |h, c| {
                        h.ping(c, wire::Ipv4Addr(dst))
                    });
                vec![EngineEvent::Time {
                    ns: self.world.kernel.now().0,
                    background: self.world.kernel.has_background(),
                }]
            }
            EngineCmd::StepBack => match self.history.pop() {
                Some(w) => {
                    self.world = w;
                    vec![EngineEvent::Rewound {
                        ns: self.world.kernel.now().0,
                        left: self.history.len() as u32,
                    }]
                }
                None => err("nothing to step back to"),
            },
            EngineCmd::ForgetHistory => {
                self.history.clear();
                vec![]
            }
            EngineCmd::Step => {
                self.remember();
                let ev = self.step_event();
                if matches!(ev, EngineEvent::Idle { .. }) {
                    self.history.pop();
                }
                vec![ev]
            }
            EngineCmd::RunToIdle => {
                self.history.clear();
                self.world.run_to_idle();
                vec![EngineEvent::Time {
                    ns: self.world.kernel.now().0,
                    background: self.world.kernel.has_background(),
                }]
            }
            EngineCmd::Advance { ns } => {
                self.history.clear();
                let to = SimTime(self.world.kernel.now().0.saturating_add(ns));
                self.world.run_until(to);
                vec![EngineEvent::Time {
                    ns: self.world.kernel.now().0,
                    background: self.world.kernel.has_background(),
                }]
            }
        }
    }

    /// Dispatch one event and describe it.
    fn step_event(&mut self) -> EngineEvent {
        match self.world.step() {
            Some(crate::Delivered::Frame {
                at,
                device,
                port,
                frame,
                from,
                sent,
            }) => EngineEvent::Hop {
                ns: at.0,
                device: device.0,
                port: port.0,
                summary: wire::summarize(&frame),
                bytes: frame.len() as u32,
                frame,
                from: from.0,
                sent_ns: sent.0,
            },
            Some(crate::Delivered::Timer { at, device }) => EngineEvent::Tick {
                ns: at.0,
                device: device.0,
            },
            None => EngineEvent::Idle {
                ns: self.world.kernel.now().0,
            },
        }
    }

    /// Worker-boundary form: postcard in, postcard out.
    pub fn handle_bytes(&mut self, input: &[u8]) -> Vec<u8> {
        match decode::<EngineCmd>(input) {
            Ok(cmd) => encode(&self.handle(cmd)),
            Err(_) => encode(&vec![EngineEvent::Error {
                message: "undecodable command".to_string(),
            }]),
        }
    }
}
