//! The worker protocol: one JSON object per line.
//!
//! The parent writes [`ToWorker`] lines to the worker's standard input; the worker writes
//! [`FromWorker`] lines to its standard output, each after [`FRAME_MARK`] so the parent can tell
//! frames from anything else that reaches the stream. Cell values travel as
//! [`gridcraft_core::Value`].

use gridcraft_core::Value;
use serde::{Deserialize, Serialize};

/// Starts every frame the worker writes. Text before it on a line is not part of the frame.
pub const FRAME_MARK: &str = "\u{1e}gcpy ";

/// What one Python run may use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Limits {
    /// Time one cell may run, in milliseconds.
    pub timeout_ms: u64,
    /// Resident memory the worker may use, in bytes; the parent kills it above that.
    pub memory_bytes: u64,
    /// Python call depth.
    pub recursion: usize,
    /// Bytes of printed output kept per cell (the rest is cut off).
    pub stdout_bytes: usize,
    /// Cells a result may convert to.
    pub output_cells: usize,
    /// `xl()` calls per cell.
    pub xl_calls: usize,
    /// Time spent importing modules that weren't loaded yet that doesn't count against
    /// `timeout_ms`, at most, in milliseconds (a package's first import after an install can
    /// take many seconds).
    pub import_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            timeout_ms: 5_000,
            memory_bytes: 512 * 1024 * 1024,
            recursion: 200,
            stdout_bytes: 1024 * 1024,
            output_cells: 1_000_000,
            xl_calls: 10_000,
            import_ms: 60_000,
        }
    }
}

/// Parent to worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToWorker {
    /// Start (or restart) the interpreter with these limits, run `init_code` first, then answer
    /// [`FromWorker::Ready`].
    #[serde(rename_all = "camelCase")]
    Init { limits: Limits, init_code: String },
    /// Run one cell's code in the shared namespace; answer [`FromWorker::Done`] with the same id.
    Run { id: u64, code: String },
    /// The answer to [`FromWorker::Xl`].
    XlValue { id: u64, reply: XlReply },
    /// Throw the namespace away and start again (the initialization code runs again); answer
    /// [`FromWorker::Ready`].
    Reset,
}

/// The cells an `xl()` call asked for, or why they can't be read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum XlReply {
    /// A single cell's value, or an array for a range.
    Value(Value),
    /// The reference doesn't name cells (the message becomes a Python exception).
    Error(String),
}

/// Worker to parent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FromWorker {
    /// The interpreter is ready. `init_error` says how the initialization code failed, if it did.
    #[serde(rename_all = "camelCase")]
    Ready { init_error: Option<PyError> },
    /// Run `id` reads cells: answer with [`ToWorker::XlValue`].
    Xl { id: u64, reference: String, headers: bool },
    /// Run `id` has spent `ms` milliseconds so far importing modules (capped at
    /// [`Limits::import_ms`]); `module` is the one being imported now, if any. The parent extends
    /// the run's deadline by `ms`. Sent while a slow import runs (CPython only).
    Extend {
        id: u64,
        ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        module: Option<String>,
    },
    /// Run `id` finished.
    Done(Done),
}

/// How a run ended.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Done {
    pub id: u64,
    /// The value of the last expression, when the code ran without an error.
    pub output: Option<Output>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<PyError>,
    /// The run's duration in milliseconds, measured in the worker.
    pub elapsed_ms: f64,
}

/// A Python result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Output {
    /// The result as cells: a scalar, or an array to spill. For a result with no cell form (a
    /// function, a set, a class instance) it is the preview as text.
    pub value: Value,
    /// Python's name for the result's type (`int`, `list`, `dict`…).
    pub type_name: String,
    /// The result's `repr`, cut to [`PREVIEW_CHARS`].
    pub preview: String,
    /// Whether `value` is the result converted to cells (false: only the preview is).
    pub convertible: bool,
    /// What an object-mode cell shows: the type with its size (`DataFrame 6×4`, `list 12`,
    /// `dict 4 keys`, `str 5 lines`), or just the type name.
    pub label: String,
    /// The result is a table (a DataFrame, Series, 1-D or 2-D ndarray, list, list of lists, or
    /// dict), so the app can show and export it without running the code again.
    pub tabular: bool,
    /// The table's first row is a header row (a DataFrame's columns, a dict of columns' keys).
    pub headers: bool,
    /// The table, when it isn't `value` itself (a dict of equal-length lists: one column per
    /// key, under its key), capped like `value`.
    pub table: Option<Value>,
}

/// Characters of a result's `repr` kept as its preview.
pub const PREVIEW_CHARS: usize = 255;

/// A Python error: the line it happened on (in the cell's code) and its message.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PyError {
    pub line: Option<u32>,
    /// `ExceptionType: message`.
    pub message: String,
    /// The worker's time limit stopped the code.
    pub timeout: bool,
    /// The worker can't be trusted to run more code (its time limit interrupted an import,
    /// which can leave a package half loaded): the parent starts a fresh one.
    pub restart: bool,
    /// The module being imported when the time limit stopped the code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importing: Option<String>,
}

/// One frame as a line (with [`FRAME_MARK`] and a newline).
pub fn frame_line<T: Serialize>(msg: &T) -> String {
    match serde_json::to_string(msg) {
        Ok(json) => format!("{FRAME_MARK}{json}\n"),
        // Serializing these types can't fail (no maps with non-string keys, no NaN: `Value`
        // never holds one); an empty frame is ignored by the reader.
        Err(_) => format!("{FRAME_MARK}{{}}\n"),
    }
}

/// The frame in a line the worker wrote, if there is one.
pub fn parse_from_worker(line: &str) -> Option<FromWorker> {
    let at = line.find(FRAME_MARK)?;
    serde_json::from_str(line.get(at + FRAME_MARK.len()..)?.trim_end()).ok()
}

/// A line the parent wrote.
pub fn parse_to_worker(line: &str) -> Option<ToWorker> {
    serde_json::from_str(line.trim()).ok()
}

/// A line for the parent to write (no mark: the worker's input carries nothing else).
pub fn to_worker_line(msg: &ToWorker) -> String {
    match serde_json::to_string(msg) {
        Ok(json) => format!("{json}\n"),
        Err(_) => "{}\n".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_core::{Array, CellError};

    #[test]
    fn frames_round_trip_and_ignore_other_text() {
        let done = FromWorker::Done(Done {
            id: 7,
            output: Some(Output {
                value: Array::new(1, 2, vec![Value::Number(1.5), Value::Error(CellError::Div0)]).map(Value::from).unwrap_or_default(),
                type_name: "list".into(),
                preview: "[1.5, '#DIV/0!']".into(),
                convertible: true,
                label: "list 2".into(),
                tabular: true,
                ..Output::default()
            }),
            stdout: "hi\n".into(),
            ..Done::default()
        });
        let line = frame_line(&done);
        assert_eq!(parse_from_worker(&line), Some(done.clone()));
        // Text a test harness prints before the frame on the same line is skipped.
        assert_eq!(parse_from_worker(&format!("test worker ... {line}")), Some(done));
        assert_eq!(parse_from_worker("running 1 test"), None);
        assert_eq!(parse_from_worker(&format!("{FRAME_MARK}{{not json")), None);
        let run = ToWorker::Run { id: 3, code: "x = 1\n\"quoted\"".into() };
        assert_eq!(parse_to_worker(&to_worker_line(&run)), Some(run));
        let reply = ToWorker::XlValue { id: 3, reply: XlReply::Error("no such sheet".into()) };
        assert_eq!(parse_to_worker(&to_worker_line(&reply)), Some(reply));
        assert_eq!(parse_to_worker("garbage"), None);
        // The shim's heartbeat while it imports, with and without a module.
        let extend = FromWorker::Extend { id: 9, ms: 1_250, module: Some("sklearn".into()) };
        assert_eq!(parse_from_worker(&frame_line(&extend)), Some(extend));
        assert_eq!(
            parse_from_worker(&format!("{FRAME_MARK}{{\"type\":\"extend\",\"id\":2,\"ms\":40}}")),
            Some(FromWorker::Extend { id: 2, ms: 40, module: None })
        );
        let timeout = PyError { timeout: true, restart: true, importing: Some("sklearn".into()), ..PyError::default() };
        let line = frame_line(&FromWorker::Done(Done { id: 3, error: Some(timeout.clone()), ..Done::default() }));
        assert!(line.contains("\"importing\":\"sklearn\""));
        assert_eq!(parse_from_worker(&line), Some(FromWorker::Done(Done { id: 3, error: Some(timeout), ..Done::default() })));
    }
}
