//! The M14.12 gate: Monty, with no host access, keeps a shared namespace across cells, calls a
//! host `xl()`, converts values and captures `print`. The worker protocol replaces this module.

use std::io::Read;

/// Reads standard input to its end.
pub(crate) fn drain_stdin() -> i32 {
    let mut sink = Vec::new();
    match std::io::stdin().read_to_end(&mut sink) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use monty::{MontyRepl, ReplProgress};
    use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceLimits, ResourceTracker};

    /// Runs one cell, answering `xl()` calls; returns the repr of the value or the error summary,
    /// and what it printed.
    fn run(repl: MontyRepl, code: &str) -> (MontyRepl, Result<String, String>, String) {
        let mut out = String::new();
        let inputs = vec![("xl".to_string(), MontyObject::function("xl", None))];
        let mut progress = repl.feed_start(code, inputs, PrintWriter::CollectString(&mut out, Some(1 << 20)));
        loop {
            match progress {
                Ok(ReplProgress::FunctionCall(call)) => {
                    let range = call.args.arg(0).and_then(|a| a.as_str().map(str::to_string)).unwrap_or_default();
                    let v = if range.contains(':') {
                        let row = |a, b| MontyObject::list([a, b]);
                        MontyObject::list([row(MontyObject::int(1), MontyObject::float(2.5)), row(MontyObject::string("a"), MontyObject::none())])
                    } else {
                        MontyObject::float(42.0)
                    };
                    progress = call.resume(v, PrintWriter::CollectString(&mut out, Some(1 << 20)));
                }
                Ok(ReplProgress::Complete { repl, value }) => return (repl, Ok(value.py_repr()), out),
                Ok(other) => return (other.into_repl(), Err("host access".into()), out),
                Err(e) => return (e.repl, Err(e.error.summary()), out),
            }
        }
    }

    #[test]
    fn sandboxed_interpreter_shares_a_namespace_calls_xl_converts_and_captures_print() {
        let limits = ResourceLimits { max_feed_duration: Some(Duration::from_secs(2)), max_recursion_depth: 200, ..ResourceLimits::default() };
        let repl = MontyRepl::new("cell.py", ResourceTracker::new(limits), CompileOptions::default());
        let (repl, r, out) = run(repl, "x = 1\nprint('hi', xl('A1'))\nxl('A1:B2')");
        assert_eq!(r, Ok("[[1, 2.5], ['a', None]]".into()));
        assert_eq!(out, "hi 42.0\n");
        let (repl, r, _) = run(repl, "x + 1");
        assert_eq!(r, Ok("2".into()), "names persist from one cell to the next");
        let (repl, r, _) = run(repl, "import math\nmath.sqrt(16)");
        assert_eq!(r, Ok("4.0".into()));
        let (repl, r, _) = run(repl, "open('/etc/hosts').read()");
        assert_eq!(r, Err("host access".into()), "file access suspends to the host, which refuses it");
        let (repl, r, _) = run(repl, "def f(n): return f(n + 1)\nf(0)");
        assert!(r.as_ref().is_err_and(|e| e.starts_with("RecursionError")), "{r:?}");
        let (_, r, _) = run(repl, "while True: pass");
        assert!(r.as_ref().is_err_and(|e| e.starts_with("TimeoutError")), "{r:?}");
    }
}
