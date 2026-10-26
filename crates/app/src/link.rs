//! The one place the app talks to the engine. Every command and answer crosses here as postcard
//! bytes, as it must when the engine runs in a Web Worker; today the engine is in the same
//! thread and the call returns at once.

use engine::lab::BuiltLab;
use engine::protocol::{decode, encode, Engine, EngineCmd, EngineEvent, LabView};
use std::cell::RefCell;

pub struct EngineLink {
    engine: RefCell<Engine>,
}

impl std::fmt::Debug for EngineLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EngineLink")
    }
}

impl EngineLink {
    /// An engine running `built`, and how its lab looks.
    pub fn from_built(built: BuiltLab) -> (EngineLink, LabView) {
        let (engine, view) = Engine::from_built(built);
        (
            EngineLink {
                engine: RefCell::new(engine),
            },
            view,
        )
    }

    /// A stand-in while the real link is moved into a new `App`; never sent anything.
    pub fn detached() -> EngineLink {
        EngineLink {
            engine: RefCell::new(Engine::new(engine::World::new(kernel::Trace::new()))),
        }
    }

    pub fn send(&self, cmd: EngineCmd) -> Vec<EngineEvent> {
        let reply = self.engine.borrow_mut().handle_bytes(&encode(&cmd));
        decode(&reply).expect("the engine's answers decode")
    }

    /// Ask the engine to load a lab (lab file text); the new lab's view, or why not.
    pub fn load(&self, lab: &str) -> Result<LabView, String> {
        match self
            .send(EngineCmd::Load {
                lab: lab.to_string(),
            })
            .into_iter()
            .next()
        {
            Some(EngineEvent::Lab { view }) => Ok(view),
            Some(EngineEvent::Error { message }) => Err(message),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    /// The lab as it stands, as lab file text.
    pub fn snapshot(&self) -> String {
        match self.send(EngineCmd::Snapshot).into_iter().next() {
            Some(EngineEvent::LabText { text }) => text,
            other => panic!("the engine has no lab to snapshot: {other:?}"),
        }
    }
}
