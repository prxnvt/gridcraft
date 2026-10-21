//! The worker: one Monty interpreter with one namespace shared by every cell, driven by the
//! protocol on a line reader and writer (standard input and output in the worker process, channels
//! in tests).
//!
//! Printed output goes to per-run buffers that come back in [`Done`]; protocol frames are the only
//! thing written to the output stream.

use std::borrow::Cow;
use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

use gridcraft_core::{Array, CellError, Value};
use monty::{MontyRepl, ReplProgress};
use monty_types::{
    CompileOptions, ExcType, ExtFunctionResult, MontyException, MontyObject, NameLookupResult, ObjectRef, PrintWriter, PrintWriterCallback,
    ResourceLimits, ResourceTracker,
};

use crate::protocol::{Done, FromWorker, Limits, Output, PREVIEW_CHARS, PyError, ToWorker, XlReply, frame_line, parse_to_worker};

/// What the worker does when its input ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnClose {
    /// Return from [`serve`] when the loop next reads.
    Return,
    /// End the process at once, even while code runs (the worker process: its parent is gone).
    ExitProcess,
}

/// Serves the protocol until the input ends. Lines are read on a separate thread, so with
/// [`OnClose::ExitProcess`] a closed input ends the process even while Python code is running.
pub fn serve<R: BufRead + Send + 'static, W: Write>(input: R, output: W, on_close: OnClose) {
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::Builder::new().name("python-worker-input".into()).spawn(move || {
        let mut input = input;
        loop {
            let mut line = String::new();
            match input.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
        if on_close == OnClose::ExitProcess {
            std::process::exit(0);
        }
    });
    if reader.is_err() {
        return;
    }
    let mut worker = Worker { repl: None, limits: Limits::default(), init_code: String::new(), out: output, input: rx };
    while let Ok(line) = worker.input.recv() {
        let Some(msg) = parse_to_worker(&line) else { continue };
        match msg {
            ToWorker::Init { limits, init_code } => {
                worker.limits = limits;
                worker.init_code = init_code;
                worker.restart();
            }
            ToWorker::Reset => worker.restart(),
            ToWorker::Run { id, code } => {
                let done = worker.run(id, &code);
                worker.send(&FromWorker::Done(done));
            }
            // A reply nobody waits for (its run already ended).
            ToWorker::XlValue { .. } => {}
        }
    }
}

struct Worker<W: Write> {
    repl: Option<MontyRepl>,
    limits: Limits,
    init_code: String,
    out: W,
    input: mpsc::Receiver<String>,
}

/// Collects printed output up to a size, then notes that the rest was cut.
struct Capture {
    stdout: String,
    stderr: String,
    max: usize,
    cut: bool,
}

impl Capture {
    fn new(max: usize) -> Capture {
        Capture { stdout: String::new(), stderr: String::new(), max, cut: false }
    }
    fn push(&mut self, stderr: bool, text: &str) {
        let used = self.stdout.len().saturating_add(self.stderr.len());
        let room = self.max.saturating_sub(used);
        let buf = if stderr { &mut self.stderr } else { &mut self.stdout };
        if text.len() <= room {
            buf.push_str(text);
        } else {
            let mut end = room;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            buf.push_str(text.get(..end).unwrap_or_default());
            if !self.cut {
                self.cut = true;
                buf.push_str("\n[output cut off]\n");
            }
        }
    }
}

impl PrintWriterCallback for Capture {
    fn stdout_write(&mut self, output: Cow<'_, str>) -> Result<(), MontyException> {
        self.push(false, &output);
        Ok(())
    }
    fn stdout_push(&mut self, end: char) -> Result<(), MontyException> {
        self.push(false, end.encode_utf8(&mut [0; 4]));
        Ok(())
    }
    fn stderr_write(&mut self, output: Cow<'_, str>) -> Result<(), MontyException> {
        self.push(true, &output);
        Ok(())
    }
    fn stderr_push(&mut self, end: char) -> Result<(), MontyException> {
        self.push(true, end.encode_utf8(&mut [0; 4]));
        Ok(())
    }
}

/// Monotonic milliseconds (the worker only runs natively, where the clock works).
#[allow(clippy::disallowed_methods)]
fn now() -> std::time::Instant {
    std::time::Instant::now()
}

impl<W: Write> Worker<W> {
    fn send(&mut self, msg: &FromWorker) {
        // The parent reads frames until the stream closes; if it is gone there is nobody to tell.
        let _ = self.out.write_all(frame_line(msg).as_bytes());
        let _ = self.out.flush();
    }

    fn new_repl(&self) -> MontyRepl {
        let limits = ResourceLimits {
            max_feed_duration: Some(Duration::from_millis(self.limits.timeout_ms.max(1))),
            max_recursion_depth: self.limits.recursion.clamp(1, 100_000),
            max_suspensions: self.limits.xl_calls.max(1),
            ..ResourceLimits::default()
        };
        MontyRepl::new("cell.py", ResourceTracker::new(limits), CompileOptions::default())
    }

    /// A fresh interpreter with the initialization code run in it.
    fn restart(&mut self) {
        self.repl = Some(self.new_repl());
        let init_error = if self.init_code.trim().is_empty() {
            None
        } else {
            let code = self.init_code.clone();
            self.run(0, &code).error
        };
        self.send(&FromWorker::Ready { init_error });
    }

    fn run(&mut self, id: u64, code: &str) -> Done {
        if let Some(error) = refusal(code) {
            return Done { id, error: Some(error), ..Done::default() };
        }
        let started = now();
        let repl = match self.repl.take() {
            Some(r) => r,
            None => self.new_repl(),
        };
        let mut capture = Capture::new(self.limits.stdout_bytes);
        let (repl, result) = self.drive(id, repl, code, &mut capture);
        self.repl = Some(repl);
        let mut done =
            Done { id, stdout: capture.stdout, stderr: capture.stderr, elapsed_ms: started.elapsed().as_secs_f64() * 1000.0, ..Done::default() };
        match result {
            Ok(value) => match to_output(&value.as_ref(), self.limits.output_cells) {
                Ok(o) => done.output = Some(o),
                Err(message) => done.error = Some(PyError { line: None, message, timeout: false, restart: false, importing: None }),
            },
            Err(e) => done.error = Some(e),
        }
        done
    }

    /// Runs `code` to the end, answering its suspensions: `xl()` asks the parent; file, network
    /// and other host access is refused.
    fn drive(&mut self, id: u64, repl: MontyRepl, code: &str, capture: &mut Capture) -> (MontyRepl, Result<MontyObject, PyError>) {
        let inputs = vec![("xl".to_string(), MontyObject::function("xl", Some(XL_DOC.to_string())))];
        let mut progress = repl.feed_start(code, inputs, PrintWriter::Callback(capture));
        loop {
            progress = match progress {
                Ok(ReplProgress::Complete { repl, value }) => return (repl, Ok(value)),
                Err(e) => {
                    let e = *e;
                    return (e.repl, Err(py_error(&e.error)));
                }
                Ok(ReplProgress::FunctionCall(call)) => {
                    let result = if call.function_name == "xl" && call.object_id.is_none() {
                        let reference = call.args.arg(0).and_then(|a| a.as_str().map(str::to_string));
                        let headers = call.args.kwarg("headers").or_else(|| call.args.arg(1)).is_some_and(|h| h.is_truthy());
                        match reference {
                            Some(reference) => self.ask_xl(id, reference, headers),
                            None => ExtFunctionResult::Error(MontyException::new(
                                ExcType::TypeError,
                                Some("xl() takes a reference as text, such as xl(\"A1:B5\")".into()),
                            )),
                        }
                    } else {
                        ExtFunctionResult::NotFound(call.function_name.clone())
                    };
                    call.resume(result, PrintWriter::Callback(capture))
                }
                Ok(ReplProgress::OsCall(call)) => {
                    let refused = MontyException::new(ExcType::PermissionError, Some(NO_HOST_ACCESS.into()));
                    call.resume(ExtFunctionResult::Error(refused), PrintWriter::Callback(capture))
                }
                Ok(ReplProgress::NameLookup(lookup)) => lookup.resume(NameLookupResult::Undefined, PrintWriter::Callback(capture)),
                Ok(ReplProgress::ResolveFutures(state)) => {
                    let refused = MontyException::new(ExcType::RuntimeError, Some("Python cells can't wait for asynchronous work".into()));
                    state.abort(refused, PrintWriter::Callback(capture))
                }
            };
        }
    }

    /// Asks the parent for the cells `reference` names and waits for the answer.
    fn ask_xl(&mut self, id: u64, reference: String, headers: bool) -> ExtFunctionResult {
        self.send(&FromWorker::Xl { id, reference, headers });
        loop {
            let Ok(line) = self.input.recv() else {
                // The parent went away; the run can't continue.
                return ExtFunctionResult::Error(MontyException::new(ExcType::RuntimeError, Some("GridCraft stopped answering".into())));
            };
            match parse_to_worker(&line) {
                Some(ToWorker::XlValue { id: got, reply }) if got == id => {
                    return match reply {
                        // The built-in Python gets dates and times as their serial numbers: its
                        // results can't be read back as dates.
                        XlReply::Value(v) | XlReply::Dated { value: v, .. } => ExtFunctionResult::Return(xl_object(&v, headers)),
                        XlReply::Error(msg) => ExtFunctionResult::Error(MontyException::new(ExcType::ValueError, Some(msg))),
                    };
                }
                // Nothing else is sent while a run waits for cells.
                _ => continue,
            }
        }
    }
}

/// Why the built-in Python won't run `code` at all: it imports a module Monty doesn't have, or
/// uses something Monty's compiler reports as unsupported. Nothing of such a cell runs.
fn refusal(code: &str) -> Option<PyError> {
    if let Some((module, line)) = crate::imports::missing_module(code) {
        return Some(PyError { line: Some(line), message: crate::imports::needs_cpython(&module), timeout: false, restart: false, importing: None });
    }
    // Compiled on its own: other errors (syntax, names) come from the run itself, as usual.
    match monty::MontyRun::new(code.to_string(), "cell.py", vec!["xl".into()], CompileOptions::default()) {
        Err(e) if e.exc_type() == ExcType::NotImplementedError => {
            let line = e.traceback().last().map(|f| f.start.line);
            Some(PyError {
                line,
                message: crate::imports::unsupported(e.message().unwrap_or("unsupported code")),
                timeout: false,
                restart: false,
                importing: None,
            })
        }
        _ => None,
    }
}

const XL_DOC: &str = "xl(reference, headers=False): the value of a cell, or a range as a list of rows (a list of dicts with headers=True).";
const NO_HOST_ACCESS: &str = "Python cells have no access to files, the network or the system";

fn py_error(e: &MontyException) -> PyError {
    // The innermost frame in the cell's own code (`cell.py`, or the REPL's per-snippet names).
    let line = e.traceback().iter().rev().find(|f| f.filename.contains("cell")).or_else(|| e.traceback().last()).map(|f| f.start.line);
    let timeout = e.exc_type() == ExcType::TimeoutError && e.message().is_some_and(|m| m.contains("time limit exceeded"));
    PyError { line, message: e.summary(), timeout, restart: false, importing: None }
}

/// A Python value for cells `xl()` read: a scalar for one cell, a list of rows for a range, or a
/// list of dicts keyed by the first row with `headers`.
fn xl_object(v: &Value, headers: bool) -> MontyObject {
    match v {
        Value::Array(a) => {
            let row = |r: usize| (0..a.cols).map(move |c| scalar_object(a.get(r, c).unwrap_or(&Value::Empty)));
            if headers && a.rows >= 1 {
                let keys: Vec<MontyObject> = (0..a.cols).map(|c| MontyObject::string(a.get(0, c).map(Value::display).unwrap_or_default())).collect();
                MontyObject::list((1..a.rows).map(|r| MontyObject::dict(keys.iter().cloned().zip(row(r)))))
            } else {
                MontyObject::list((0..a.rows).map(|r| MontyObject::list(row(r))))
            }
        }
        v => scalar_object(v),
    }
}

fn scalar_object(v: &Value) -> MontyObject {
    match v {
        Value::Empty => MontyObject::none(),
        // Whole numbers come through as int, so `range(xl("A1"))` works.
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 => MontyObject::int(*n as i64),
        Value::Number(n) => MontyObject::float(*n),
        Value::Text(t) => MontyObject::string(t.to_string()),
        Value::Bool(b) => MontyObject::bool(*b),
        Value::Error(e) => MontyObject::string(e.as_str()),
        Value::Array(a) => a.data.first().map(scalar_object).unwrap_or_else(MontyObject::none),
    }
}

/// Converts a Python result to cells: None, bool, int, float and str to a scalar; a list to a
/// column; a list of lists to a table; a dict to a key/value table. Anything else is an object
/// shown by its preview.
pub fn to_output(o: &ObjectRef<'_>, max_cells: usize) -> Result<Output, String> {
    let type_name = o.type_name().to_string();
    let preview = cut(&o.py_repr(), PREVIEW_CHARS);
    let (value, convertible) = match scalar(o) {
        Some(v) => (v, true),
        None => match table(o, max_cells)? {
            Some(v) => (v, true),
            None => (Value::text(preview.as_str()), false),
        },
    };
    Ok(Output { value, type_name, preview, convertible, dates: Vec::new() })
}

fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}...", s.get(..at).unwrap_or_default()),
        None => s.to_string(),
    }
}

fn scalar(o: &ObjectRef<'_>) -> Option<Value> {
    match o.type_name() {
        "NoneType" => Some(Value::Empty),
        "bool" => o.as_bool().map(Value::Bool),
        // A float out of range (or a huge int) is #NUM!, as Excel never stores those.
        "int" => Some(o.as_float().map(Value::number).unwrap_or(Value::Error(CellError::Num))),
        "float" => Some(o.as_float().map(Value::number).unwrap_or(Value::Error(CellError::Num))),
        "str" => o.as_str().map(Value::text),
        _ => None,
    }
}

/// A cell inside a table: a scalar, or the repr of anything else.
fn element(o: &ObjectRef<'_>) -> Value {
    scalar(o).unwrap_or_else(|| Value::text(cut(&o.py_repr(), PREVIEW_CHARS)))
}

fn table(o: &ObjectRef<'_>, max_cells: usize) -> Result<Option<Value>, String> {
    let too_big = || format!("The result has more than {max_cells} cells");
    let sequence = |o: &ObjectRef<'_>| matches!(o.type_name(), "list" | "tuple");
    let array = |rows: usize, cols: usize, data: Vec<Value>| -> Value {
        Array::new(rows, cols, data).map(Value::from).unwrap_or(Value::Error(CellError::Calc))
    };
    if sequence(o) {
        let items = o.items().unwrap_or_default();
        if items.is_empty() {
            return Ok(Some(Value::Error(CellError::Calc)));
        }
        if items.iter().all(sequence) {
            let rows: Vec<Vec<ObjectRef<'_>>> = items.iter().map(|r| r.items().unwrap_or_default()).collect();
            let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
            if rows.len().checked_mul(cols).is_none_or(|n| n > max_cells) {
                return Err(too_big());
            }
            let mut data = Vec::with_capacity(rows.len() * cols);
            for r in &rows {
                for c in 0..cols {
                    // Short rows are padded with #N/A, as Excel pads arrays.
                    data.push(r.get(c).map(element).unwrap_or(Value::Error(CellError::NA)));
                }
            }
            return Ok(Some(array(rows.len(), cols, data)));
        }
        if items.len() > max_cells {
            return Err(too_big());
        }
        return Ok(Some(array(items.len(), 1, items.iter().map(element).collect())));
    }
    if o.type_name() == "dict" {
        let pairs = o.pairs().unwrap_or_default();
        if pairs.is_empty() {
            return Ok(Some(Value::Error(CellError::Calc)));
        }
        if pairs.len().saturating_mul(2) > max_cells {
            return Err(too_big());
        }
        let data = pairs.iter().flat_map(|(k, v)| [element(k), element(v)]).collect();
        return Ok(Some(array(pairs.len(), 2, data)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(code: &str) -> Result<Output, PyError> {
        let (tx_in, rx_in) = mpsc::channel::<String>();
        let mut w = Worker { repl: None, limits: Limits::default(), init_code: String::new(), out: Vec::new(), input: rx_in };
        drop(tx_in);
        let done = w.run(1, code);
        match done.error {
            Some(e) => Err(e),
            None => Ok(done.output.unwrap_or_default()),
        }
    }

    fn value(code: &str) -> Value {
        out(code).map(|o| o.value).unwrap_or_else(|e| panic!("{code}: {e:?}"))
    }

    #[test]
    fn conversions() {
        assert_eq!(value("None"), Value::Empty);
        assert_eq!(value("True"), Value::Bool(true));
        assert_eq!(value("1 + 2"), Value::Number(3.0));
        assert_eq!(value("2.5"), Value::Number(2.5));
        assert_eq!(value("float('nan')"), Value::Error(CellError::Num));
        assert_eq!(value("float('inf')"), Value::Error(CellError::Num));
        assert_eq!(value("10**400"), Value::Error(CellError::Num));
        assert_eq!(value("'hi'"), Value::text("hi"));
        assert_eq!(value("[1, 'a']"), Array::new(2, 1, vec![Value::Number(1.0), Value::text("a")]).map(Value::from).unwrap());
        assert_eq!(
            value("[[1, 2], (3,)]"),
            Array::new(2, 2, vec![Value::Number(1.0), Value::Number(2.0), Value::Number(3.0), Value::Error(CellError::NA)]).map(Value::from).unwrap()
        );
        assert_eq!(value("{'a': 1, 'b': None}"), Array::new(2, 2, vec!["a".into(), 1.0.into(), "b".into(), Value::Empty]).map(Value::from).unwrap());
        assert_eq!(value("[]"), Value::Error(CellError::Calc));
        let o = out("{1, 2}").unwrap();
        assert!(!o.convertible);
        assert_eq!((o.type_name.as_str(), o.preview.as_str()), ("set", "{1, 2}"));
        let o = out("[[1]]").unwrap();
        assert_eq!(o.type_name, "list");
        assert!(o.convertible);
        let o = out("'x' * 1000").unwrap();
        assert_eq!(o.preview.chars().count(), PREVIEW_CHARS + 3);
    }

    #[test]
    fn errors_carry_their_line_and_host_access_is_refused() {
        let e = out("x = 1\ny = 2\n1 / 0").unwrap_err();
        assert_eq!(e.line, Some(3));
        assert!(e.message.starts_with("ZeroDivisionError"), "{e:?}");
        let e = out("def f(:\n  pass").unwrap_err();
        assert!(e.message.starts_with("SyntaxError"), "{e:?}");
        let e = out("open('/etc/hosts').read()").unwrap_err();
        assert!(e.message.contains("PermissionError"), "{e:?}");
        let e = out("import os\nos.getenv('HOME')").unwrap_err();
        assert!(e.message.contains("PermissionError") || e.message.contains("Error"), "{e:?}");
        let e = out("def f(n): return f(n + 1)\nf(0)").unwrap_err();
        assert!(e.message.starts_with("RecursionError"), "{e:?}");
        let e = out("[[0] * 2000 for _ in range(1000)]").unwrap_err();
        assert!(e.message.contains("more than 1000000 cells"), "{e:?}");
    }

    #[test]
    fn output_is_cut_off_at_the_limit() {
        let mut c = Capture::new(10);
        c.push(false, "0123456");
        c.push(true, "789abc");
        c.push(false, "more");
        assert_eq!(c.stdout, "0123456");
        assert_eq!(c.stderr, "789\n[output cut off]\n");
    }

    #[test]
    fn xl_values_become_python_values() {
        let a = Array::new(2, 2, vec!["k".into(), "v".into(), 1.0.into(), Value::Error(CellError::Div0)]).map(Value::from).unwrap();
        assert_eq!(xl_object(&a, false).py_repr(), "[['k', 'v'], [1, '#DIV/0!']]");
        assert_eq!(xl_object(&a, true).py_repr(), "[{'k': 1, 'v': '#DIV/0!'}]");
        assert_eq!(xl_object(&Value::Number(2.5), false).py_repr(), "2.5");
        assert_eq!(xl_object(&Value::Empty, false).py_repr(), "None");
    }

    #[test]
    fn dates_reach_the_built_in_python_as_serial_numbers() {
        let (tx_in, rx_in) = mpsc::channel::<String>();
        let mut w = Worker { repl: None, limits: Limits::default(), init_code: String::new(), out: Vec::new(), input: rx_in };
        let dates = vec![crate::DateCell { at: 0, kind: crate::DateKind::DateTime }];
        let reply = ToWorker::XlValue { id: 1, reply: XlReply::Dated { value: Value::Number(45292.5), dates } };
        tx_in.send(crate::protocol::to_worker_line(&reply)).unwrap();
        let done = w.run(1, "xl('A1') + 1");
        assert_eq!(done.output.map(|o| (o.value, o.dates)), Some((Value::Number(45293.5), Vec::new())));
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    fn run(code: &str) -> Done {
        let (_tx, rx) = mpsc::channel::<String>();
        let mut w = Worker { repl: None, limits: Limits::default(), init_code: String::new(), out: Vec::new(), input: rx };
        w.run(1, code)
    }

    #[test]
    fn code_the_built_in_python_cant_run_is_refused_whole() {
        for (code, module, line) in [
            ("print('ran')\nimport pandas as pd\npd.DataFrame()", "pandas", 2),
            ("import statistics", "statistics", 1),
            ("from numpy import array", "numpy", 1),
        ] {
            let d = run(code);
            let e = d.error.unwrap_or_else(|| panic!("{code}"));
            assert_eq!(
                e.message,
                format!(
                    "This cell needs CPython: `{module}` isn't available in the built-in Python. Install Python 3.11 or newer and set it in Python Options."
                )
            );
            assert_eq!(e.line, Some(line));
            assert_eq!(d.stdout, "", "nothing of the cell ran");
        }
        let d = run("print('ran')\nmatch 1:\n    case 1: pass");
        assert_eq!(
            d.error.map(|e| e.message),
            Some("This cell needs CPython: the built-in Python doesn't support pattern matching (match statements). Install Python 3.11 or newer and set it in Python Options.".into())
        );
        assert_eq!(d.stdout, "");
        let d = run("from math import *");
        assert!(d.error.is_some_and(|e| e.message.contains("doesn't support wildcard imports")));
        // What the built-in Python has still runs, and ordinary errors are unchanged.
        assert!(run("import math, json\nfrom collections import Counter\nmath.sqrt(4)").error.is_none());
        assert!(run("undefined_name").error.is_some_and(|e| e.message.starts_with("NameError")));
        assert!(run("def f(:").error.is_some_and(|e| e.message.starts_with("SyntaxError")));
    }
}
