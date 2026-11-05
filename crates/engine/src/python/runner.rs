//! Running due cells, one at a time: submit the next, take its result, write it, plan again. A cell
//! only runs in a worker that has run every cell before it, since they share variables: a fresh
//! worker runs those first. A result writes no undo step.

use std::sync::Arc;
use std::time::Duration;

use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_model::PyOutput;
use gridcraft_python::diagnostics::{Diagnostic, for_outcome};
use gridcraft_python::protocol::OUTPUT_CUT;
use gridcraft_python::{Client, Outcome, Output, RunResult, Transport};
use web_time::Instant;

use super::inputs::{Problem, Reader, code_hash, literals};
use super::plan::{CellState, Cx, DocRun, Source};
use super::{PythonCapability, PythonRun, PythonStats};
use crate::DocState;

/// Runs kept in a workbook's log.
const LOG_LIMIT: usize = 200;
/// Bytes of printed output a log entry keeps.
const LOG_OUTPUT_BYTES: usize = 64 * 1024;
/// Cells a poll settles without the worker (a bad reference, for one).
const LOCAL_PER_POLL: usize = 32;
/// How many workers one go may lose before the cells still waiting are left stale: every lost
/// worker makes the cells before it run again.
const MAX_REPLAYS: u32 = 3;
/// How many times a cell tries again when its worker doesn't start. The launcher refuses when a
/// descriptor it inherited is open, and a thread starting another process at that moment can leak
/// one to it; that passes on the next try, and what doesn't pass is answered a moment later.
pub(super) const START_RETRIES: u32 = 3;

/// The request a worker is running.
pub(super) struct Inflight {
    pub id: u64,
    /// The cell's index in the run order. A plan keeps it valid: when the list changes at or before
    /// it, the result is discarded.
    pub index: usize,
    /// The code and inputs it runs with.
    pub fp: u64,
    pub started: Instant,
    /// What it reads or what runs before it changed since it started: the cell runs again.
    pub discard: bool,
}

#[allow(clippy::disallowed_methods)] // `web_time`'s clock, which works on wasm
fn now() -> Instant {
    Instant::now()
}

/// What starts workers.
pub(super) struct Starter<'a> {
    pub capability: &'a PythonCapability,
    pub interpreter: Option<&'a gridcraft_python::describe::Interpreter>,
}

impl Starter<'_> {
    fn client(&self) -> Option<Client> {
        let limits = self.capability.limits.clone();
        if let Some(factory) = &self.capability.transport {
            return Some(Client::new(factory(), limits));
        }
        self.interpreter.and_then(|i| confined(i, self.capability)).map(|t| Client::new(t, limits))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn confined(interpreter: &gridcraft_python::describe::Interpreter, capability: &PythonCapability) -> Option<Box<dyn Transport>> {
    Some(Box::new(gridcraft_python::process::ProcessTransport::confined(interpreter, capability.launcher.clone())))
}

#[cfg(target_arch = "wasm32")]
fn confined(_: &gridcraft_python::describe::Interpreter, _: &PythonCapability) -> Option<Box<dyn Transport>> {
    None
}

/// What one step of the queue did.
enum Step {
    /// A request went to the worker.
    Submitted,
    /// The cell settled without the worker, and the workbook changed.
    Wrote,
    /// Nothing to do for this cell.
    Skipped,
    /// The list ended.
    End,
    /// No request can go out now.
    Stop,
}

/// What a finished request means for the session.
#[derive(Default)]
pub(super) struct Finish {
    pub wrote: bool,
    /// The worker couldn't start: Python isn't ready after all.
    pub env_problem: Option<Diagnostic>,
}

/// What to write to a cell.
pub(super) enum Write {
    Result(PyOutput),
    Error(CellError),
}

/// A value's type as Python would name it, for the card.
fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Empty => "NoneType",
        Value::Number(_) => "float",
        Value::Text(_) => "str",
        Value::Bool(_) => "bool",
        Value::Error(_) => "error",
        Value::Array(_) => "list",
    }
}

fn preview(v: &Value) -> String {
    match v {
        Value::Empty => "None".into(),
        Value::Number(n) => n.to_string(),
        Value::Text(t) => t.chars().take(255).collect(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Error(e) => e.as_str().into(),
        Value::Array(a) => format!("{} rows, {} columns", a.rows, a.cols),
    }
}

/// A result as a cell keeps it. A Python object shows its type's name, and its preview stays for
/// the card.
fn kept(output: Output) -> PyOutput {
    match output {
        Output::Value(value) => PyOutput { type_name: type_name(&value).into(), preview: preview(&value), value, convertible: true },
        Output::Object { type_name, preview } => PyOutput { value: Value::text(type_name.as_str()), type_name, preview, convertible: false },
    }
}

/// The diagnostic for a result that holds an error the person should hear about.
fn result_diagnostic(value: &Value) -> Option<Diagnostic> {
    match value {
        Value::Error(CellError::Calc) => Some(Diagnostic::new("result.empty", "", None)),
        Value::Error(CellError::Num) => Some(Diagnostic::new("result.number", "", None)),
        _ => None,
    }
}

/// The name a `python.name` error is about.
fn undefined_name(outcome: &Outcome) -> Option<&str> {
    match outcome {
        Outcome::PythonError { error, .. } if error.kind.as_deref() == Some("python.name") => error.subject.as_deref(),
        _ => None,
    }
}

/// Printed output cut for the log, and whether anything was cut, here or by the worker.
fn cut_output(mut stdout: String) -> (String, bool) {
    let mut cut = stdout.ends_with(OUTPUT_CUT);
    if stdout.len() > LOG_OUTPUT_BYTES {
        let mut end = LOG_OUTPUT_BYTES;
        while !stdout.is_char_boundary(end) {
            end -= 1;
        }
        stdout.truncate(end);
        cut = true;
    }
    (stdout, cut)
}

/// The names `code` assigns, by a plain look at it: `name =`, and the names `def`, `class`, `import`,
/// `as` and `for` bind. Strings and comments are skipped.
pub(super) fn assigned_names(code: &str) -> Vec<&str> {
    let bytes = code.as_bytes();
    let is_name = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
    let mut out = Vec::new();
    let mut i = 0;
    let mut previous: &str = "";
    while let Some(&b) = bytes.get(i) {
        match b {
            b'#' => i += bytes.get(i..).and_then(|rest| rest.iter().position(|c| *c == b'\n')).unwrap_or(bytes.len() - i),
            b'"' | b'\'' => i = after_string(bytes, i),
            _ if is_name(b) && !b.is_ascii_digit() => {
                let end = i + bytes.get(i..).map_or(0, |rest| rest.iter().take_while(|c| is_name(**c)).count());
                let name = code.get(i..end).unwrap_or_default();
                let attribute = i > 0 && bytes.get(i - 1) == Some(&b'.');
                if !attribute && (matches!(previous, "def" | "class" | "import" | "as" | "for") || is_assignment(bytes, end)) {
                    out.push(name);
                }
                previous = name;
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

/// Whether a name ending at `end` is followed by `=` that isn't `==`.
fn is_assignment(bytes: &[u8], end: usize) -> bool {
    let at = end + bytes.get(end..).map_or(0, |rest| rest.iter().take_while(|c| **c == b' ' || **c == b'\t').count());
    bytes.get(at) == Some(&b'=') && bytes.get(at + 1) != Some(&b'=')
}

/// Just past the string starting at `at`, triple-quoted or not.
fn after_string(bytes: &[u8], at: usize) -> usize {
    let Some(&q) = bytes.get(at) else { return at.saturating_add(1) };
    let triple = bytes.get(at + 1) == Some(&q) && bytes.get(at + 2) == Some(&q);
    let mut i = if triple { at + 3 } else { at + 1 };
    while let Some(&c) = bytes.get(i) {
        match c {
            b'\\' => i += 2,
            c if c == q && !triple => return i + 1,
            c if c == q && bytes.get(i + 1) == Some(&q) && bytes.get(i + 2) == Some(&q) => return i + 3,
            b'\n' if !triple => return i,
            _ => i += 1,
        }
    }
    i
}

/// Writes a result or an error to a cell, through the same path as any edit: calculation lays out
/// the spill and recalculates what reads it. It makes no undo step. Returns whether anything changed.
pub(super) fn write_cell(d: &mut DocState, sheet: usize, at: CellRef, write: Write) -> bool {
    let unchanged =
        d.wb.sheet(sheet)
            .and_then(|s| s.cell(at))
            .and_then(|c| c.py.as_deref())
            .is_some_and(|py| !py.busy && !py.stale && matches!(&write, Write::Result(out) if py.output.as_ref() == Some(out)));
    if unchanged {
        return false;
    }
    let done = crate::cmd::commit(d, |cx| {
        let Some(cell) = cx.sheet_mut(sheet)?.cells.get_mut(at) else { return Ok(()) };
        let Some(py) = cell.py.as_mut() else { return Ok(()) };
        let py = Arc::make_mut(py);
        py.busy = false;
        py.stale = false;
        match write {
            Write::Result(out) => py.output = Some(out),
            Write::Error(e) => {
                py.output = None;
                cell.value = Value::Error(e);
            }
        }
        cx.touch(sheet, at);
        Ok(())
    });
    if let Err(e) = done {
        log::warn!("Python cells: {e}");
    }
    d.revision = d.revision.saturating_add(1);
    true
}

impl DocRun {
    /// Sends the next cell due to the worker, if none is running. Returns whether the workbook
    /// changed on the way (cells settled without the worker).
    pub fn pump(&mut self, d: &mut DocState, starter: &Starter, cx: Cx, stats: &mut PythonStats) -> bool {
        if self.inflight.is_some() {
            return false;
        }
        let mut wrote = false;
        let mut local = LOCAL_PER_POLL;
        while self.due > 0 && local > 0 {
            match self.step(d, starter, cx, stats) {
                Step::Submitted | Step::Stop => break,
                Step::Wrote => {
                    wrote = true;
                    local -= 1;
                }
                Step::Skipped => {}
                Step::End => self.back_to_first_queued(),
            }
        }
        if self.due == 0 {
            self.passes = 0;
            self.replays = 0;
        }
        wrote
    }

    fn back_to_first_queued(&mut self) {
        self.recount();
        self.cursor = self.entries.iter().position(|e| e.state == CellState::Queued).unwrap_or(0);
    }

    fn step(&mut self, d: &mut DocState, starter: &Starter, cx: Cx, stats: &mut PythonStats) -> Step {
        let index = self.cursor;
        let Some(e) = self.entries.get(index) else { return Step::End };
        if e.state != CellState::Queued && e.in_worker {
            self.cursor += 1;
            return Step::Skipped;
        }
        let (sheet, uid, at) = (e.sheet, e.uid, e.at);
        let wb = d.wb.clone();
        let Some(py) = wb.sheet(sheet).and_then(|s| s.cell(at)).and_then(|c| c.py.clone()) else {
            self.cursor += 1;
            return Step::Skipped;
        };
        let lits = literals(&wb, sheet, at, &py.code);
        let mut reader = Reader::new(&wb, stats);
        let fp = reader.fingerprint(code_hash(&py.code, py.object), &lits);
        if let Some((_, error)) = self.refused.get(&(uid, at)).copied().filter(|(before, _)| *before == fp) {
            return self.settle_refused(d, index, fp, error);
        }
        let inputs = reader.inputs(&lits);
        drop(reader);
        match inputs {
            Err(problem) => self.settle_problem(d, index, fp, problem, cx, stats),
            Ok(inputs) => {
                if self.client.is_none() {
                    self.client = starter.client();
                    stats.clients = stats.clients.saturating_add(u64::from(self.client.is_some()));
                }
                let Some(client) = self.client.as_mut() else { return Step::Stop };
                match client.submit_cell(&py.code, inputs, py.object) {
                    Ok(id) => {
                        self.inflight = Some(Inflight { id, index, fp, started: now(), discard: false });
                        Step::Submitted
                    }
                    Err(_) => {
                        client.stop();
                        Step::Stop
                    }
                }
            }
        }
    }

    /// A cell that stopped the worker with these same code and inputs stays as it was.
    fn settle_refused(&mut self, d: &mut DocState, index: usize, fp: u64, error: CellError) -> Step {
        let wrote = self.entries.get(index).is_some_and(|e| e.state == CellState::Queued);
        self.settled(index, fp, false);
        if wrote && let Some(e) = self.entries.get(index) {
            let (sheet, at) = (e.sheet, e.at);
            write_cell(d, sheet, at, Write::Error(error));
            self.seen = Some(d.wb.clone());
            return Step::Wrote;
        }
        Step::Skipped
    }

    /// A cell whose inputs can't be given to it doesn't run.
    fn settle_problem(&mut self, d: &mut DocState, index: usize, fp: u64, problem: Problem, cx: Cx, stats: &mut PythonStats) -> Step {
        let (diag, error) = match problem {
            Problem::TooLarge { cells } => (
                Diagnostic::new("xl.tooLarge", format!("{cells} cells; the limit is {}", gridcraft_python::protocol::MAX_ARRAY_CELLS), None),
                CellError::Python,
            ),
            Problem::Bad { reference, why } => (Diagnostic::new("xl.badReference", format!("{reference}: {why}"), None), CellError::Ref),
        };
        self.settled(index, fp, true);
        let Some(e) = self.entries.get_mut(index) else { return Step::Skipped };
        let (sheet, at) = (e.sheet, e.at);
        e.diag = Some(diag.clone());
        self.log_run(PythonRun::new(d, sheet, at, Duration::ZERO, String::new(), false, Some(diag.kind.id)));
        write_cell(d, sheet, at, Write::Error(error));
        self.plan(d, cx, Source::Result(index), stats);
        Step::Wrote
    }

    /// The cell at `index` needs nothing more for now.
    fn settled(&mut self, index: usize, fp: u64, in_worker: bool) {
        if let Some(e) = self.entries.get_mut(index) {
            if e.state == CellState::Queued {
                self.due = self.due.saturating_sub(1);
            }
            e.state = CellState::Clean;
            e.unsettled = false;
            e.fp = fp;
            e.ran = Some(fp);
            e.in_worker = in_worker;
            e.shown = (false, false);
        }
        self.cursor = index.saturating_add(1);
    }

    /// Waits up to `wait` for the running request, and returns its result when it has one. It also
    /// notices a worker that went away while idle: its variables went with it.
    pub fn poll_worker(&mut self, wait: Duration) -> Option<RunResult> {
        let client = self.client.as_mut()?;
        if self.inflight.is_some() {
            return client.poll(wait);
        }
        let alive = client.alive();
        let idle = client.poll(Duration::ZERO);
        if alive && !client.alive() {
            self.worker_gone();
        }
        idle
    }

    /// The worker is gone, and the variables with it.
    pub fn worker_gone(&mut self) {
        for e in &mut self.entries {
            e.in_worker = false;
        }
        self.cursor = 0;
    }

    fn log_run(&mut self, run: PythonRun) {
        self.log.push_back(run);
        while self.log.len() > LOG_LIMIT {
            self.log.pop_front();
        }
    }

    /// Writes what a finished request gave.
    pub fn finish(&mut self, d: &mut DocState, result: RunResult, cx: Cx, stats: &mut PythonStats) -> Finish {
        let mut out = Finish::default();
        let Some(flight) = self.inflight.take_if(|f| f.id == result.id) else { return out };
        let outcome = result.outcome;
        if matches!(outcome, Outcome::FailedToStart(_)) && !flight.discard && self.start_tries < START_RETRIES {
            self.start_tries += 1;
            return out;
        }
        self.start_tries = 0;
        let diag = for_outcome(&outcome);
        let refused = diag.as_ref().is_some_and(|d| d.kind.id == "blocked.sandboxCheck");
        let lost = outcome.lost_worker();
        let error = if refused { Some(CellError::Blocked) } else { outcome.cell_error() };
        if matches!(outcome, Outcome::FailedToStart(_)) {
            out.env_problem = diag.clone();
        } else if lost {
            self.note_kill(&flight, error.unwrap_or(CellError::Python));
        }
        let name_order = undefined_name(&outcome).and_then(|name| self.defined_later(&d.wb, name, flight.index, diag.as_ref()));
        let (write, stdout) = match outcome {
            Outcome::Ok { output, stdout } => (Write::Result(kept(output)), stdout),
            Outcome::PythonError { stdout, .. } => (Write::Error(error.unwrap_or(CellError::Python)), stdout),
            _ => (Write::Error(error.unwrap_or(CellError::Python)), String::new()),
        };
        let (stdout, cut) = cut_output(stdout);
        let cell_diag = name_order.or(diag).or_else(|| match &write {
            Write::Result(o) => result_diagnostic(&o.value),
            Write::Error(_) => None,
        });
        let cell_diag = cell_diag.or_else(|| cut.then(|| Diagnostic::new("limit.output", "", None)));
        self.record(d, &flight, stdout, cut, cell_diag.as_ref());
        if flight.discard {
            if lost {
                self.worker_gone();
            }
            return out;
        }
        self.settled(flight.index, flight.fp, !lost);
        if lost {
            self.worker_gone();
        }
        let Some(e) = self.entries.get_mut(flight.index) else { return out };
        let (sheet, at) = (e.sheet, e.at);
        e.diag = cell_diag;
        out.wrote = write_cell(d, sheet, at, write);
        out.wrote |= self.plan(d, cx, Source::Result(flight.index), stats);
        self.give_up_after_losses();
        out.wrote |= self.sync_flags(d);
        out
    }

    fn record(&mut self, d: &DocState, flight: &Inflight, stdout: String, cut: bool, diag: Option<&Diagnostic>) {
        let Some(e) = self.entries.get(flight.index) else { return };
        let run = PythonRun::new(d, e.sheet, e.at, flight.started.elapsed(), stdout, cut, diag.map(|x| x.kind.id));
        self.log_run(run);
    }

    /// The cell stopped the worker: it doesn't run again until something about it changes, and the
    /// cells still due find a fresh worker.
    fn note_kill(&mut self, flight: &Inflight, error: CellError) {
        if let Some(e) = self.entries.get(flight.index) {
            self.refused.insert((e.uid, e.at), (flight.fp, error));
        }
        self.replays = self.replays.saturating_add(1);
    }

    fn give_up_after_losses(&mut self) {
        if self.replays > MAX_REPLAYS {
            self.demote_queued();
        }
    }

    /// `python.nameOrder`: a later cell assigns the name this cell didn't find.
    fn defined_later(&mut self, wb: &gridcraft_model::Workbook, name: &str, index: usize, diag: Option<&Diagnostic>) -> Option<Diagnostic> {
        let table = self.assigns.get_or_insert_with(|| {
            let mut table: std::collections::HashMap<String, Vec<u32>> = std::collections::HashMap::new();
            for (i, e) in self.entries.iter().enumerate() {
                let Some(py) = wb.sheet(e.sheet).and_then(|s| s.cell(e.at)).and_then(|c| c.py.as_deref()) else { continue };
                for assigned in assigned_names(&py.code) {
                    table.entry(assigned.to_string()).or_default().push(u32::try_from(i).unwrap_or(u32::MAX));
                }
            }
            table
        });
        let later = table.get(name)?.iter().find(|i| **i as usize > index)?;
        let e = self.entries.get(*later as usize)?;
        let sheet = wb.sheet(e.sheet)?;
        let place = format!("{}!{}", gridcraft_core::quote_sheet(&sheet.name), e.at.a1());
        Some(Diagnostic::new("python.nameOrder", format!("{name} is assigned in {place}"), diag.and_then(|d| d.line)))
    }

    /// Stops the running cell and leaves the ones waiting as stale. Returns whether anything was
    /// running or waiting.
    pub fn stop(&mut self, d: &mut DocState) -> bool {
        let flight = self.inflight.take();
        if flight.is_none() && self.due == 0 {
            return false;
        }
        if flight.is_some()
            && let Some(client) = self.client.as_mut()
        {
            client.stop();
        }
        if let Some(f) = flight.as_ref().filter(|f| !f.discard) {
            self.stop_running(d, f);
        }
        if flight.is_some() {
            self.worker_gone();
        }
        self.demote_queued();
        self.sync_flags(d);
        self.seen = Some(d.wb.clone());
        true
    }

    fn stop_running(&mut self, d: &mut DocState, flight: &Inflight) {
        const STOPPED: CellError = CellError::Python;
        self.note_kill(flight, STOPPED);
        let diag = Diagnostic::new("state.stopped", "", None);
        self.record(d, flight, String::new(), false, Some(&diag));
        self.settled(flight.index, flight.fp, false);
        if let Some(e) = self.entries.get_mut(flight.index) {
            let (sheet, at) = (e.sheet, e.at);
            e.diag = Some(diag);
            write_cell(d, sheet, at, Write::Error(STOPPED));
        }
    }

    /// Asks for every cell to run, in a fresh worker when `fresh`, whatever the calculation mode.
    pub fn run_everything(&mut self, d: &mut DocState, fresh: bool) -> usize {
        if fresh {
            if let Some(client) = self.client.as_mut() {
                client.stop();
            }
            self.inflight = None;
            self.worker_gone();
        }
        self.refused.clear();
        for e in &mut self.entries {
            e.ran = None;
            e.state = CellState::Queued;
            e.unsettled = false;
        }
        self.cursor = 0;
        if let Some(f) = self.inflight.as_mut() {
            f.discard = true;
        }
        self.passes = 0;
        self.replays = 0;
        self.recount();
        self.sync_flags(d);
        self.due
    }

    /// Asks for the cell at `index` to run. Under manual calculation the cells after it are marked
    /// stale, as they read what it leaves in the variables.
    pub fn run_one(&mut self, d: &mut DocState, index: usize, cx: Cx) -> usize {
        let Some(e) = self.entries.get(index) else { return 0 };
        self.refused.remove(&(e.uid, e.at));
        for (i, e) in self.entries.iter_mut().enumerate().skip(index) {
            e.ran = None;
            e.unsettled = false;
            if i == index || !cx.manual {
                e.state = CellState::Queued;
            } else if e.state != CellState::Queued {
                e.state = CellState::Stale;
            }
        }
        self.cursor = self.cursor.min(index);
        if let Some(f) = self.inflight.as_mut().filter(|f| f.index >= index) {
            f.discard = true;
        }
        self.recount();
        self.sync_flags(d);
        self.due
    }
}
