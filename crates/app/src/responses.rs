//! What becomes of each answer from the engine. The app never waits for one: it posts a command
//! with a note of what it was for ([`Awaiting`]) and, when the answer arrives (at the start of the
//! next frame, or at once with an engine in this thread), [`App::pump`] hands it to
//! [`App::on_response`], which finds the note and does what the person was waiting for.

use super::*;

/// What an unanswered command was for.
#[derive(Debug, Clone)]
pub(super) enum Awaiting {
    /// A console is being opened for this device (its tab shows nothing until it is).
    OpenTerminal { device: u32 },
    /// A line was typed into this device's console.
    Exec { device: u32 },
    /// `?` was pressed in this device's console.
    Help { device: u32 },
    /// Tab was pressed in this device's console.
    Complete { device: u32 },
    /// Only the side effects matter (the lab text, the clock).
    Nothing,
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
        for r in self.engine.poll() {
            if let Some(what) = self.pending.remove(&r.id) {
                self.on_response(what, r.events);
            }
        }
    }

    fn on_response(&mut self, what: Awaiting, events: Vec<EngineEvent>) {
        absorb(&mut self.lab_text, &events);
        self.note_clock(&events);
        let mut main = events
            .into_iter()
            .find(|e| !matches!(e, EngineEvent::LabText { .. } | EngineEvent::Time { .. }));
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
            Awaiting::Nothing => {}
        }
    }

    /// The tab showing `device`'s console, if there is one.
    fn tab_for(&mut self, device: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.device == device)
    }
}
