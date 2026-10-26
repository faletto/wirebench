//! The one place the app talks to the engine. A command goes out as postcard bytes with a number;
//! its answer comes back later, as bytes with the same number, in the order the commands were
//! sent. Where the engine runs (in this thread, or in a Web Worker) is the transport's business.
//!
//! [`EngineLink::send`] posts a command and waits for its answer: it is how code that has not yet
//! been made to cope with late answers still works. Everything new should use
//! [`EngineLink::post`] and [`EngineLink::poll`].

use engine::lab::BuiltLab;
use engine::protocol::{
    decode, encode, Engine, EngineCmd, EngineEvent, LabView, Request, Response,
};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

/// A way of getting request bytes to an engine and response bytes back.
pub trait Transport {
    fn post(&mut self, request: Vec<u8>);
    /// Responses that have arrived since the last call, oldest first.
    fn poll(&mut self) -> Vec<Vec<u8>>;
}

/// The engine in this thread: a request is answered as it is posted.
pub struct Local {
    engine: Engine,
    ready: VecDeque<Vec<u8>>,
}

impl Local {
    pub fn new(engine: Engine) -> Local {
        Local {
            engine,
            ready: VecDeque::new(),
        }
    }
}

impl Transport for Local {
    fn post(&mut self, request: Vec<u8>) {
        let reply = self.engine.serve(&request);
        self.ready.push_back(reply);
    }

    fn poll(&mut self) -> Vec<Vec<u8>> {
        self.ready.drain(..).collect()
    }
}

/// Holds each response back for `polls` calls to `poll` after it was ready: an engine a thread
/// away that takes a while. For tests, and for finding what assumes an answer comes at once.
pub struct Delayed<T: Transport> {
    inner: T,
    polls: usize,
    held: VecDeque<(usize, Vec<u8>)>,
}

impl<T: Transport> Delayed<T> {
    pub fn new(inner: T, polls: usize) -> Self {
        Delayed {
            inner,
            polls,
            held: VecDeque::new(),
        }
    }
}

impl<T: Transport> Transport for Delayed<T> {
    fn post(&mut self, request: Vec<u8>) {
        self.inner.post(request);
    }

    fn poll(&mut self) -> Vec<Vec<u8>> {
        for r in self.inner.poll() {
            self.held.push_back((self.polls, r));
        }
        for h in &mut self.held {
            h.0 = h.0.saturating_sub(1);
        }
        let mut out = vec![];
        // Order is kept: nothing overtakes a response still being held.
        while self.held.front().is_some_and(|h| h.0 == 0) {
            out.push(self.held.pop_front().expect("checked").1);
        }
        out
    }
}

pub struct EngineLink {
    transport: RefCell<Box<dyn Transport>>,
    next_id: Cell<u64>,
    /// Responses read while waiting for a particular one.
    stash: RefCell<VecDeque<Response>>,
}

impl std::fmt::Debug for EngineLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EngineLink")
    }
}

impl EngineLink {
    pub fn new(transport: impl Transport + 'static) -> EngineLink {
        EngineLink {
            transport: RefCell::new(Box::new(transport)),
            next_id: Cell::new(1),
            stash: RefCell::new(VecDeque::new()),
        }
    }

    /// An engine running `built` in this thread, and how its lab looks.
    pub fn from_built(built: BuiltLab) -> (EngineLink, LabView) {
        let (engine, view) = Engine::from_built(built);
        (EngineLink::new(Local::new(engine)), view)
    }

    /// Like [`EngineLink::from_built`] but every answer takes `polls` polls to arrive.
    pub fn from_built_delayed(built: BuiltLab, polls: usize) -> (EngineLink, LabView) {
        let (engine, view) = Engine::from_built(built);
        (
            EngineLink::new(Delayed::new(Local::new(engine), polls)),
            view,
        )
    }

    /// A stand-in while the real link is moved into a new `App`; never sent anything.
    pub fn detached() -> EngineLink {
        EngineLink::new(Local::new(Engine::new(engine::World::new(
            kernel::Trace::new(),
        ))))
    }

    /// Send a command and say which answer is its own.
    pub fn post(&self, cmd: EngineCmd) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.transport
            .borrow_mut()
            .post(encode(&Request { id, cmd }));
        id
    }

    /// Everything that has come back so far, in order.
    pub fn poll(&self) -> Vec<Response> {
        let mut out: Vec<Response> = self.stash.borrow_mut().drain(..).collect();
        for bytes in self.transport.borrow_mut().poll() {
            out.push(decode(&bytes).expect("the engine's answers decode"));
        }
        out
    }

    /// Post a command and wait for its answer (see the module note). Answers to other commands
    /// that arrive meanwhile are kept for the next `poll`.
    pub fn send(&self, cmd: EngineCmd) -> Vec<EngineEvent> {
        let id = self.post(cmd);
        loop {
            let mut mine = None;
            for r in self.poll() {
                if r.id == id {
                    mine = Some(r.events);
                } else {
                    self.stash.borrow_mut().push_back(r);
                }
            }
            if let Some(events) = mine {
                return events;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_with_lab() -> Engine {
        let (e, _) = Engine::from_built(
            engine::lab::load_and_build(
                include_str!("../../../labs/m1_static_routes.lab.toml"),
                kernel::Trace::new(),
            )
            .unwrap(),
        );
        e
    }

    #[test]
    fn local_answers_at_once_and_delayed_after_the_stated_number_of_polls() {
        let local = EngineLink::new(Local::new(engine_with_lab()));
        let id = local.post(EngineCmd::Now);
        assert_eq!(local.poll().iter().map(|r| r.id).collect::<Vec<_>>(), [id]);

        let slow = EngineLink::new(Delayed::new(Local::new(engine_with_lab()), 3));
        let a = slow.post(EngineCmd::Now);
        let b = slow.post(EngineCmd::Now);
        assert!(slow.poll().is_empty());
        assert!(slow.poll().is_empty());
        let got: Vec<u64> = slow.poll().iter().map(|r| r.id).collect();
        assert_eq!(got, [a, b], "in order, together");
    }

    #[test]
    fn send_waits_for_its_own_answer_and_keeps_the_others() {
        let slow = EngineLink::new(Delayed::new(Local::new(engine_with_lab()), 2));
        let earlier = slow.post(EngineCmd::Now);
        let ev = slow.send(EngineCmd::Now);
        assert!(matches!(ev[0], EngineEvent::Time { .. }));
        let rest: Vec<u64> = slow.poll().iter().map(|r| r.id).collect();
        assert_eq!(rest, [earlier], "the earlier answer was not lost");
    }
}
