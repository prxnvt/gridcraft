//! Python in cells.
//!
//! Python code runs in Monty, a Python interpreter written in Rust for sandboxed code, inside a
//! separate worker process: the app binary started again with [`WORKER_FLAG`]. Code such as
//! `'x' * 10**12` makes an interpreter allocate until the process aborts, and a huge integer power
//! never checks its time limit; `catch_unwind` can catch neither. A separate process is the only
//! reliable harness: the app kills it on a timeout or when it uses too much memory, and restarts it
//! after a crash, while the workbook stays safe.
//!
//! Monty has no ambient host access: file, environment and network operations suspend to the host,
//! and the worker refuses them, so Python code can't touch files, the network or other processes.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod interp;
pub mod protocol;
pub mod run;

#[cfg(not(target_arch = "wasm32"))]
pub mod client;

#[cfg(target_arch = "wasm32")]
#[path = "client_web.rs"]
pub mod client;

#[cfg(all(feature = "python", not(target_arch = "wasm32")))]
pub mod worker;

pub use protocol::{Limits, Output, PyError, XlReply};

/// The command-line flag that makes an app binary run as the Python worker. `apps/gridcraft` and
/// `apps/gridcraft-cli` check for it first thing in `main`.
pub const WORKER_FLAG: &str = "--python-worker";

/// Whether this build can run Python (the web build can't: Python cells show `#BLOCKED!`).
pub const AVAILABLE: bool = cfg!(not(target_arch = "wasm32"));

/// Whether the process arguments ask for the worker.
pub fn is_worker_invocation(args: &[String]) -> bool {
    args.iter().any(|a| a == WORKER_FLAG)
}

/// Runs the Python worker on standard input and output until the parent closes its end, and
/// returns the process exit code.
pub fn worker_main() -> i32 {
    #[cfg(all(feature = "python", not(target_arch = "wasm32")))]
    {
        // A large stack: the interpreter's own recursion limit stops Python code long before this
        // runs out, and a deep native recursion only ends this process.
        let served = std::thread::Builder::new().name("python-worker".into()).stack_size(256 * 1024 * 1024).spawn(|| {
            worker::serve(std::io::BufReader::new(std::io::stdin()), std::io::stdout(), worker::OnClose::ExitProcess);
        });
        match served.map(|t| t.join()) {
            Ok(Ok(())) => 0,
            _ => 1,
        }
    }
    #[cfg(not(all(feature = "python", not(target_arch = "wasm32"))))]
    {
        // This binary was built without the interpreter.
        2
    }
}
