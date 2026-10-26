//! What becomes of each answer from the engine. The app never waits for one: it posts a command
//! with a note of what it was for ([`Awaiting`]) and, when the answer arrives (at the start of the
//! next frame, or at once with an engine in this thread), [`App::pump`] hands it to
//! [`App::on_response`], which finds the note and does what the person was waiting for.

use super::*;

/// What an unanswered command was for.
#[derive(Debug)]
pub(super) enum Awaiting {
    /// A console is being opened for this device (its tab shows nothing until it is).
    OpenTerminal { device: u32 },
    /// A line was typed into this device's console.
    Exec { device: u32 },
    /// `?` was pressed in this device's console.
    Help { device: u32 },
    /// Tab was pressed in this device's console.
    Complete { device: u32 },
    /// A ping was queued in simulation mode.
    Ping,
    /// One simulation step. `kind` says when to stop stepping; `play` that Play asked for it.
    Step {
        animate: bool,
        kind: StepKind,
        play: bool,
    },
    /// The latest step was undone.
    Back,
    /// A structural edit the person made; `before` is the lab as it was, for the undo history.
    Edit {
        what: EditKind,
        before: labfile::LabFile,
    },
    /// The world is being rebuilt from a lab (undo, redo).
    Rebuild { note: String, undo: Undo },
    /// A lab is being opened in place of this one.
    Open { source: String, then: Then },
    /// The panel asked for a device's state.
    PanelState { device: u32 },
    /// Lines were sent from the panel.
    Configured { device: u32 },
    /// Only the side effects matter (the lab text, the clock).
    Nothing,
}

/// Which edit.
#[derive(Debug)]
pub(super) enum EditKind {
    AddDevice { pos: [i32; 2] },
    AddLink { a: usize, b: usize },
    RemoveDevice { name: String },
    RemoveLink,
}

/// What to put back if a rebuild for undo or redo is refused.
#[derive(Debug)]
pub(super) enum Undo {
    Back {
        label: String,
        current: labfile::LabFile,
    },
    Forward {
        label: String,
        current: labfile::LabFile,
    },
}

/// What to do once a lab has been opened.
#[derive(Debug)]
pub(super) enum Then {
    Nothing,
    /// It arrived through the inbox (the browser's file picker).
    Inbox,
    /// It was read from this path (native builds).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Path(String),
    /// It is the autosave being recovered; if it will not load, the offer stands.
    Recovered(crate::autosave::Autosave),
}

/// How far one request to step goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepKind {
    /// Exactly one event.
    One,
    /// On until a frame is delivered (timers in between do not count).
    Hop,
    /// On until the simulation is idle: fast-forward.
    Run,
}

impl App {
    /// Send a command without waiting; `what` says what to do with the answer.
    pub(super) fn post(&mut self, cmd: EngineCmd, what: Awaiting) {
        let id = self.engine.post(cmd);
        self.pending.insert(id, what);
    }

    /// Deal with every answer that has arrived. Called at the start of each frame and after
    /// anything that posts, so an engine that answers at once behaves as if it were called.
    pub fn pump(&mut self) {
        // An answer can post more commands (a stepping run); an engine in this thread answers
        // those at once, so go on until nothing more has arrived.
        loop {
            let answers = self.engine.poll();
            if answers.is_empty() {
                break;
            }
            for r in answers {
                if let Some(what) = self.pending.remove(&r.id) {
                    self.on_response(what, r.events);
                }
            }
        }
    }

    fn on_response(&mut self, what: Awaiting, events: Vec<EngineEvent>) {
        absorb(&mut self.lab_text, &events);
        self.note_clock(&events);
        let mut main = events
            .iter()
            .find(|e| !matches!(e, EngineEvent::LabText { .. } | EngineEvent::Time { .. }))
            .cloned();
        match what {
            Awaiting::OpenTerminal { device } => {
                if let Some(EngineEvent::TerminalOpened {
                    terminal,
                    prompt,
                    greeting,
                }) = main.take()
                {
                    if let Some(t) = self
                        .tabs
                        .iter_mut()
                        .find(|t| t.device == device && t.terminal.is_none())
                    {
                        t.terminal = Some(terminal);
                        t.prompt = prompt;
                        if !greeting.is_empty() {
                            t.model.push_output(&greeting);
                        }
                    }
                }
            }
            Awaiting::Exec { device } => {
                let reopen = match main.take() {
                    Some(EngineEvent::Output { text, prompt, .. }) => {
                        if let Some(t) = self.tab_for(device) {
                            t.model.push_output(&text);
                            t.prompt = prompt;
                        }
                        false
                    }
                    Some(EngineEvent::Closed { .. }) => {
                        if let Some(t) = self.tab_for(device) {
                            t.model.push_output("[session closed; opened a new one]");
                            t.terminal = None;
                        }
                        true
                    }
                    Some(EngineEvent::Error { message }) => {
                        if let Some(t) = self.tab_for(device) {
                            t.model.push_output(&message);
                        }
                        false
                    }
                    _ => false,
                };
                if reopen {
                    self.post(
                        EngineCmd::OpenTerminal { device },
                        Awaiting::OpenTerminal { device },
                    );
                }
                // A command may have changed a config; only a real change makes the lab dirty.
                self.observe_changes();
            }
            Awaiting::Help { device } => {
                if let Some(EngineEvent::Output { text, .. }) = main.take() {
                    if let Some(t) = self.tab_for(device) {
                        t.model.push_output(&text);
                    }
                }
            }
            Awaiting::Complete { device } => {
                if let Some(EngineEvent::Completion { line, .. }) = main.take() {
                    if let Some(t) = self.tab_for(device) {
                        t.model.set_input(&line);
                    }
                }
            }
            Awaiting::Edit { what, before } => self.edit_answered(what, before, main),
            Awaiting::Rebuild { note, undo } => match main {
                Some(EngineEvent::Lab { view }) => self.adopt(view, note),
                other => {
                    let why = match other {
                        Some(EngineEvent::Error { message }) => message,
                        _ => "no answer".to_string(),
                    };
                    self.notice = Some(format!("could not rebuild the lab: {why}"));
                    match undo {
                        Undo::Back { label, current } => self.history.cancel_back(label, current),
                        Undo::Forward { label, current } => {
                            self.history.cancel_forward(label, current)
                        }
                    }
                }
            },
            Awaiting::Open { source, then } => match main {
                Some(EngineEvent::Lab { view }) => {
                    let engine = std::mem::replace(&mut self.engine, EngineLink::detached());
                    let mut fresh = App::assemble(engine, view, self.lab_text.clone());
                    fresh.take_session(self);
                    let problems = fresh.notice.take();
                    fresh.notice = Some(match problems {
                        Some(p) => format!("opened {source} ({p})"),
                        None => format!("opened {source}"),
                    });
                    fresh.open_ok = true;
                    *self = fresh;
                    match then {
                        Then::Nothing => {}
                        Then::Inbox => self.after_open(),
                        Then::Path(path) => {
                            self.file_path = path;
                            self.after_open();
                        }
                        Then::Recovered(_) => {
                            // Exists nowhere but the autosave, so it stays unsaved until saved.
                            let text = self.export_lab();
                            self.changes.mark_recovered(text, self.wall);
                            self.notice = Some("recovered unsaved work; Save to keep it".into());
                        }
                    }
                }
                other => {
                    let why = match other {
                        Some(EngineEvent::Error { message }) => message,
                        _ => "no answer".to_string(),
                    };
                    self.notice = Some(format!("could not open {source}: {why}"));
                    self.open_ok = false;
                    if let Then::Recovered(a) = then {
                        self.recovery = Some(a);
                    }
                }
            },
            Awaiting::PanelState { device } => {
                if let Some(EngineEvent::State { state, .. }) = main {
                    self.panel_state_arrived(device, state);
                }
            }
            Awaiting::Configured { device } => {
                let _ = device;
                self.panel.said = match main {
                    Some(EngineEvent::Configured { said, .. }) => said,
                    Some(EngineEvent::Error { message }) => vec![message],
                    _ => vec![],
                };
            }
            Awaiting::Ping => self.apply_sim(events, false),
            Awaiting::Step {
                animate,
                kind,
                play,
            } => {
                let more = !matches!(main, Some(EngineEvent::Idle { .. }));
                if !more {
                    self.sim.marks.pop();
                }
                let before = self.sim.hops.len();
                self.apply_sim(events, animate);
                let got = self.sim.hops.len() > before;
                self.sim.budget = self.sim.budget.saturating_sub(1);
                let again = more
                    && self.sim.budget > 0
                    && match kind {
                        StepKind::One => false,
                        StepKind::Hop => !got,
                        StepKind::Run => true,
                    };
                if again {
                    self.sim_post_step(animate, kind, play);
                } else {
                    self.sim.in_flight = false;
                    self.sim.outcome = Some(more);
                    if kind == StepKind::Run && more {
                        self.notice =
                            Some("stopped after 100000 events; Run again to continue".into());
                    }
                    if play {
                        if got {
                            self.anim.next_step = self.wall + self.anim.hop_secs();
                        } else {
                            self.anim.playing = false;
                            self.notice = Some("simulation is idle".into());
                        }
                    }
                }
            }
            Awaiting::Back => {
                self.note_clock(&events);
                if matches!(main, Some(EngineEvent::Rewound { .. })) {
                    let keep = self.sim.marks.pop().unwrap_or(0);
                    self.sim.hops.truncate(keep);
                    self.busy = !self.sim.marks.is_empty();
                    self.anim.flights.clear();
                    if self.sim.selected.is_some_and(|i| i >= keep) {
                        self.sim.selected = None;
                        self.sim.field = None;
                    }
                    self.sim.rewound = true;
                    self.refresh_clock();
                }
            }
            Awaiting::Nothing => {}
        }
    }

    /// The answer to a structural edit.
    fn edit_answered(
        &mut self,
        what: EditKind,
        before: labfile::LabFile,
        main: Option<EngineEvent>,
    ) {
        let view = match main {
            Some(EngineEvent::Lab { view }) => view,
            Some(EngineEvent::Error { message }) => {
                self.notice = Some(message);
                return;
            }
            _ => return,
        };
        match what {
            EditKind::AddDevice { pos } => {
                let i = view.devices.len() - 1;
                let d = view.devices[i].clone();
                self.history.record(format!("add {}", d.name), before);
                let (tab, id) = new_tab(&self.engine, &d, i);
                self.tabs.push(tab);
                self.pending
                    .insert(id, Awaiting::OpenTerminal { device: d.device });
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
            EditKind::AddLink { a, b } => {
                self.history.record(
                    format!("cable {} to {}", self.nodes[a].name, self.nodes[b].name),
                    before,
                );
                self.links.push((a, b));
                self.changes.mark_edited(self.wall);
                self.notice = None;
            }
            EditKind::RemoveDevice { name } => {
                self.adopt(view, format!("removed {name}; consoles restarted"));
                self.history.record(format!("remove {name}"), before);
            }
            EditKind::RemoveLink => {
                self.adopt(view, "removed a cable; consoles restarted".into());
                self.history.record("remove a cable", before);
            }
        }
    }

    /// The tab showing `device`'s console, if there is one.
    fn tab_for(&mut self, device: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.device == device)
    }
}
