//! The command/event protocol between a UI and the engine. On native the UI calls
//! [`Engine::handle`] directly; on the web the same messages cross a Web Worker boundary
//! as postcard bytes ([`Engine::handle_bytes`]). Either way the UI never touches a `World`.

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

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum EngineCmd {
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
    /// Run until no events remain.
    RunToIdle,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum EngineEvent {
    TerminalOpened {
        terminal: u32,
        prompt: String,
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
}

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
        }
    }

    pub fn with_kind(mut self, device: DeviceId, kind: ShellKind) -> Self {
        self.kinds.insert(device.0, kind);
        self
    }

    pub fn handle(&mut self, cmd: EngineCmd) -> Vec<EngineEvent> {
        let err = |m: &str| {
            vec![EngineEvent::Error {
                message: m.to_string(),
            }]
        };
        match cmd {
            EngineCmd::OpenTerminal { device } => {
                let id = DeviceId(device);
                let shell = match self.kinds.get(&device) {
                    Some(ShellKind::Router) => Shell::router(id),
                    Some(ShellKind::Host) => Shell::host(id),
                    Some(ShellKind::Switch) => Shell::switch(id),
                    None => return err("device has no shell"),
                };
                let prompt = shell.prompt(&self.world);
                self.terminals.push(Some(shell));
                vec![EngineEvent::TerminalOpened {
                    terminal: self.terminals.len() as u32 - 1,
                    prompt,
                }]
            }
            EngineCmd::Exec { terminal, line } => {
                let Some(Some(sh)) = self.terminals.get_mut(terminal as usize) else {
                    return err("no such terminal");
                };
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
                self.world
                    .with_device::<netdev::Host, _>(DeviceId(device), |h, c| {
                        h.ping(c, wire::Ipv4Addr(dst))
                    });
                vec![EngineEvent::Time {
                    ns: self.world.kernel.now().0,
                }]
            }
            EngineCmd::Step => vec![match self.world.step() {
                Some(crate::Delivered::Frame {
                    at,
                    device,
                    port,
                    frame,
                }) => EngineEvent::Hop {
                    ns: at.0,
                    device: device.0,
                    port: port.0,
                    summary: wire::summarize(&frame),
                    bytes: frame.len() as u32,
                },
                Some(crate::Delivered::Timer { at, device }) => EngineEvent::Tick {
                    ns: at.0,
                    device: device.0,
                },
                None => EngineEvent::Idle {
                    ns: self.world.kernel.now().0,
                },
            }],
            EngineCmd::RunToIdle => {
                self.world.run_to_idle();
                vec![EngineEvent::Time {
                    ns: self.world.kernel.now().0,
                }]
            }
            EngineCmd::Advance { ns } => {
                let to = SimTime(self.world.kernel.now().0.saturating_add(ns));
                self.world.run_until(to);
                vec![EngineEvent::Time {
                    ns: self.world.kernel.now().0,
                }]
            }
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
