//! What a Python run produced, and how workers start: shared by the client and its web stand-in.

use std::path::PathBuf;

use crate::protocol::{Limits, Output, PyError};

/// How to start a worker.
#[derive(Clone, Debug, PartialEq)]
pub enum Launcher {
    /// Run `program` with `args` and extra environment variables (the app binary with
    /// [`crate::WORKER_FLAG`]).
    Process { program: PathBuf, args: Vec<String>, env: Vec<(String, String)> },
    /// A worker thread in this process (tests): no memory watchdog, and a run that never ends
    /// leaves its thread behind when killed.
    InProcess,
    /// Python can't run here; runs fail with this reason.
    Unavailable(String),
}

impl Launcher {
    /// This program started again as the worker.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn current_exe() -> Launcher {
        match std::env::current_exe() {
            Ok(program) => Launcher::Process { program, args: vec![crate::WORKER_FLAG.to_string()], env: vec![] },
            Err(e) => Launcher::Unavailable(format!("GridCraft couldn't find its own program to start Python: {e}")),
        }
    }
}

static DEFAULT_LAUNCHER: std::sync::OnceLock<Launcher> = std::sync::OnceLock::new();

/// Sets how new sessions start Python workers. Apps call it once at start-up (with
/// [`Launcher::current_exe`], as they handle [`crate::WORKER_FLAG`]); later calls are ignored.
pub fn set_default_launcher(launcher: Launcher) {
    let _ = DEFAULT_LAUNCHER.set(launcher);
}

/// How new sessions start Python workers: what the app set, or unavailable.
pub fn default_launcher() -> Launcher {
    DEFAULT_LAUNCHER.get().cloned().unwrap_or_else(|| {
        Launcher::Unavailable(if crate::AVAILABLE {
            "Python isn't set up in this program".into()
        } else {
            "Python isn't available in the web version".into()
        })
    })
}

/// Why a run produced no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The code raised an exception (or didn't compile).
    Error,
    /// It ran longer than the time limit.
    Timeout,
    /// The worker used more memory than allowed and was stopped.
    Memory,
    /// The worker stopped unexpectedly.
    Crashed,
    /// Python can't run in this build or on this machine.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub kind: FailureKind,
    /// The line in the cell's code, for errors.
    pub line: Option<u32>,
    pub message: String,
}

/// What a run produced.
#[derive(Clone, Debug, PartialEq)]
pub struct RunOutcome {
    pub id: u64,
    pub result: Result<Output, Failure>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: f64,
    /// The run was stopped while importing this module (empty: a module whose name isn't known),
    /// which can be a slow first import rather than the code: a run on a fresh worker may work.
    pub importing: Option<String>,
}

/// Something that happened while polling.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A run finished.
    Done(RunOutcome),
    /// The initialization code raised an error (the worker still runs cells).
    InitError(PyError),
}

pub const CRASH_MESSAGE: &str = "Python stopped unexpectedly";
/// A run that recursed until Python's limit, or until the worker's stack ran out.
pub const RECURSION_MESSAGE: &str = "Too much recursion";
/// A cell that stopped the worker the last time it ran, and hasn't changed since.
pub const REFUSED_MESSAGE: &str =
    "Not run: this cell stopped Python the last time it ran. Change its code or the cells it reads, or use Reset Runtime, to run it again.";

/// A run stopped at the time limit.
pub fn timeout_message(limits: &Limits) -> String {
    let secs = limits.timeout_ms as f64 / 1000.0;
    let secs = if secs.fract() == 0.0 { format!("{secs:.0}") } else { format!("{secs}") };
    format!("Stopped after {secs} s (an endless loop?)")
}

/// A run stopped at the time limit while importing `module`, `elapsed_ms` after it started.
pub fn import_timeout_message(elapsed_ms: f64, module: &str) -> String {
    let secs = if elapsed_ms.is_finite() { (elapsed_ms / 1000.0).max(0.0) } else { 0.0 };
    let secs = if secs >= 10.0 { format!("{secs:.0}") } else { format!("{secs:.1}") };
    let what = if module.is_empty() { "a module".to_string() } else { module.to_string() };
    format!("Stopped after {secs} s while importing {what} (the first import after an install can be slow)")
}

/// A worker stopped over its memory cap.
pub fn memory_message(limits: &Limits) -> String {
    format!("Used more than {} MB", limits.memory_bytes / (1024 * 1024))
}

/// Whether a worker's last words say its stack ran out (Rust's handler, CPython's fatal error).
pub fn stack_overflowed(stderr: &str) -> bool {
    stderr.contains("has overflowed its stack") || stderr.contains("Cannot recover from stack overflow") || stderr.contains("Stack overflow")
}
