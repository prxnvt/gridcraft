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
}

/// Something that happened while polling.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A run finished.
    Done(RunOutcome),
    /// The initialization code raised an error (the worker still runs cells).
    InitError(PyError),
}

pub const TIMEOUT_MESSAGE: &str = "Python took longer than the time limit";
pub const CRASH_MESSAGE: &str = "Python stopped unexpectedly";

pub fn memory_message(limits: &Limits) -> String {
    format!("Python used more than {} MB", limits.memory_bytes / (1024 * 1024))
}
