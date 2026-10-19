//! Deterministic discrete-event kernel.
//!
//! No wall clock, no thread-local randomness, no hash-map iteration in anything that
//! affects ordering. Same inputs give a byte-identical trace on every target.

mod rng;
mod time;
mod trace;
mod world;

pub use rng::DeviceRng;
pub use time::SimTime;
pub use trace::{Trace, TraceKind};
pub use world::{DeviceId, Event, Kernel, LinkConfig, LinkId, PortId, TimerId};
