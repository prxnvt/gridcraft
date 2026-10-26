//! Python in cells: the protocol between GridCraft and the worker that runs the user's own Python.
//!
//! - [`protocol`] is the JSON-lines protocol between them, with a cap on every field.
//! - [`run`] holds the limits, the outcomes of a request and the pipe to a worker.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod protocol;
pub mod run;

pub use protocol::{Input, Output, PyError, SelfTest};
pub use run::{Busy, Limits, Outcome, RunResult, Transport};

/// Whether this build can run Python at all (the web build can't).
pub const AVAILABLE: bool = cfg!(not(target_arch = "wasm32"));
