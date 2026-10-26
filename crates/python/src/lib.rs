//! Python in cells: the worker that runs the user's own Python, and the protocol to it.
//!
//! - [`interp`] finds an interpreter (Python 3.11 or newer).
//! - `shim.py` is what that interpreter runs: it reads requests, keeps the globals the cells
//!   share, gives each cell its `xl()` inputs and converts the result.
//! - [`protocol`] is the JSON-lines protocol between them, with a cap on every field.
//! - `process::ProcessTransport` starts the interpreter (not in the web build).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod interp;
pub mod protocol;
pub mod run;

#[cfg(not(target_arch = "wasm32"))]
pub mod process;

pub use protocol::{Input, Output, PyError, SelfTest};
pub use run::{Busy, Limits, Outcome, RunResult, Transport};

/// Whether this build can run Python at all (the web build can't).
pub const AVAILABLE: bool = cfg!(not(target_arch = "wasm32"));
