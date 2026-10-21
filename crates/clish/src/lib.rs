//! Data-driven IOS-style CLI engine.
//!
//! Commands are registered as patterns against modes; this crate knows nothing about
//! devices. The embedder maps `Action::Handler` ids to device operations.

mod grammar;
pub mod msg;
mod parse;
mod session;

pub use grammar::{Action, Command, Grammar, Mode, ModeId, ParamType, Tok};
pub use parse::{complete, options, parse, Args, ErrorKind, ParseError, Parsed, Suggestion, Value};
pub use session::{render_error, render_help, Exec, Session};
