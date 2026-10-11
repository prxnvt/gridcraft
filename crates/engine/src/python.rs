//! Running Python cells.
//!
//! Each workbook gets a Python worker process (`gridcraft_python::client::PyWorker`) the first
//! time one of its cells runs; the workers live here, on the session, and are never cloned with a
//! document. After every top-level command that changed a workbook, the cells to run are found:
//! Python cells whose code changed, whose `xl()` references read changed cells, or that read
//! something only known at run time; and, because every cell shares one namespace, every Python
//! cell after the first of those, in row-major order sheet by sheet. Each runs once per refresh.
//!
//! Results are written straight into the workbook, coalesced with the edit that caused them and
//! not undoable (like data tables). Headless callers (CLI, MCP, tests) run to the end inside the
//! command; a host with a frame loop can run them in the background with
//! [`Session::set_python_background`] and [`Session::step_python`], in a job slot of their own so
//! Solver keeps running. Python never runs when a workbook is opened: its cells show their saved
//! values until something changes.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use gridcraft_calc::{Area, Key};
use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_formula::{BinOp, Expr};
use gridcraft_model::{CalcMode, PyOutput, Sheet, Workbook};
use gridcraft_python::client::{Event, Failure, FailureKind, Launcher, PyWorker, RunOutcome};
use gridcraft_python::{Limits, XlReply};
use serde::Serialize;

use crate::{DocState, Prefs, Session};

/// Runs kept in a workbook's diagnostics.
pub const LOG_LIMIT: usize = 500;
/// The hidden-data key of the workbook's initialization code.
pub const INIT_KEY: &str = "py_init";
/// Changed cells on one sheet above which every reference to the sheet counts as changed.
const MANY_CHANGES: usize = 20_000;

/// One Python run, for `python.diagnostics`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PyRunLog {
    /// Increases with every run in the workbook.
    pub seq: u64,
    pub sheet: String,
    /// The cell (`A1`), or empty for the initialization code.
    pub cell: String,
    /// `ok`, `error`, `timeout`, `memory`, `stopped` (the worker stopped), `blocked` or `cycle`.
    pub status: &'static str,
    pub ms: f64,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub line: Option<u32>,
}

/// The Python workers and the work queued for them.
#[derive(Default)]
pub struct PyRuntime {
    /// How workers start; `None` for the app's default.
    launcher: Option<Launcher>,
    /// Run in the background (stepped by the host) rather than inside commands.
    background: bool,
    docs: HashMap<u64, DocPy>,
    /// The running command already refreshed its workbook's Python cells.
    pub(crate) refreshed: bool,
}

#[derive(Default)]
struct DocPy {
    worker: Option<PyWorker>,
    /// The namespace no longer matches the cells (undo, a crash): reset and rerun everything
    /// on the next refresh.
    dirty: bool,
    job: Option<Job>,
    /// Python cell positions per sheet, with the sheet version they were found in.
    positions: HashMap<String, (Arc<Sheet>, BTreeSet<CellRef>)>,
}

/// A refresh in progress.
#[derive(Default)]
struct Job {
    queue: VecDeque<Key>,
    /// Cells rerun after the worker restarted, before the queue continues.
    replay: VecDeque<Key>,
    inflight: Option<Inflight>,
    /// Runs whose results are thrown away (the refresh was restarted while they ran).
    discard: HashSet<u64>,
    /// Cells that ran in this refresh, in order.
    ran: Vec<Key>,
    /// The earlier cells were already rerun once after a worker restart.
    replayed: bool,
}

struct Inflight {
    id: u64,
    key: Key,
    replay: bool,
}

/// A Python cell and what it reads.
struct PyCell {
    key: Key,
    refs: Vec<Area>,
    /// A literal reference whose cells can change without an edit to it (a name, a table).
    volatile: bool,
    /// An `xl()` call whose reference is only known when it runs.
    unknown: bool,
}

/// What changed on one sheet.
enum Changes {
    Cells(Vec<CellRef>),
    All,
}

impl Changes {
    fn hits(&self, area: &Area) -> bool {
        match self {
            Changes::All => true,
            Changes::Cells(cells) => cells.iter().any(|c| area.range.contains(*c)),
        }
    }
}

fn limits(prefs: &Prefs) -> Limits {
    let secs = if prefs.python_timeout_seconds.is_finite() { prefs.python_timeout_seconds.clamp(0.1, 3_600.0) } else { 5.0 };
    Limits { timeout_ms: (secs * 1000.0) as u64, ..Limits::default() }
}

fn init_code(wb: &Workbook) -> String {
    crate::cmd::stash::get(wb, INIT_KEY).unwrap_or_default()
}

fn py_code(wb: &Workbook, (si, c): Key) -> Option<String> {
    wb.sheet(si)?.cell(c)?.py.as_ref().map(|p| p.code.clone())
}

/// The cells `xl()` calls in `code` read, as seen from sheet `si`.
fn references(wb: &Workbook, si: usize, code: &str) -> (Vec<Area>, bool, bool) {
    let (mut refs, mut volatile, mut unknown) = (Vec::new(), false, false);
    for call in gridcraft_model::python::xl_calls(code) {
        match call.reference.and_then(|r| gridcraft_formula::parse(&r).ok()) {
            Some(expr) => {
                let (areas, dynamic) = gridcraft_calc::precedents(wb, si, &expr);
                refs.extend(areas);
                volatile |= dynamic;
            }
            None => unknown = true,
        }
    }
    (refs, volatile, unknown)
}

impl DocPy {
    /// Python cells of `wb` in run order, found incrementally from the last sheet versions seen.
    fn cells(&mut self, wb: &Workbook) -> Vec<PyCell> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (si, sh) in wb.sheets.iter().enumerate() {
            seen.insert(sh.name.clone());
            let positions = match self.positions.remove(&sh.name) {
                Some((old, set)) if Arc::ptr_eq(&old, sh) => set,
                Some((old, mut set)) => {
                    for c in old.cells.diff(&sh.cells) {
                        if sh.cell(c).is_some_and(|x| x.py.is_some()) {
                            set.insert(c);
                        } else {
                            set.remove(&c);
                        }
                    }
                    set
                }
                None => sh.cells.iter().filter(|(_, c)| c.py.is_some()).map(|(c, _)| c).collect(),
            };
            for &c in &positions {
                let Some(py) = sh.cell(c).and_then(|x| x.py.as_ref()) else { continue };
                let (refs, volatile, unknown) = references(wb, si, &py.code);
                out.push(PyCell { key: (si, c), refs, volatile, unknown });
            }
            self.positions.insert(sh.name.clone(), (sh.clone(), positions));
        }
        self.positions.retain(|name, _| seen.contains(name));
        out
    }

    fn worker(&mut self, launcher: &Launcher, limits: &Limits, init: String) -> &mut PyWorker {
        let w = self.worker.get_or_insert_with(|| PyWorker::new(launcher.clone(), limits.clone(), init.clone()));
        w.configure(limits.clone(), init);
        w
    }
}

/// What an edit changed, sheet by sheet (indices into the new workbook), plus the Python cells
/// whose code changed or that were removed.
struct EditChanges {
    sheets: HashMap<usize, Changes>,
    code: HashSet<Key>,
    removed: Vec<Key>,
    /// A sheet holding Python cells disappeared.
    lost_sheet: bool,
}

fn edit_changes(old: &Workbook, new: &Workbook) -> EditChanges {
    let mut ch = EditChanges { sheets: HashMap::new(), code: HashSet::new(), removed: Vec::new(), lost_sheet: false };
    for (si, sh) in new.sheets.iter().enumerate() {
        let Some(before) = old.sheet_index(&sh.name).and_then(|i| old.sheets.get(i)) else {
            ch.sheets.insert(si, Changes::All);
            for (c, cell) in sh.cells.iter() {
                if cell.py.is_some() {
                    ch.code.insert((si, c));
                }
            }
            continue;
        };
        if Arc::ptr_eq(before, sh) {
            continue;
        }
        let mut cells = before.cells.diff(&sh.cells);
        for &c in &cells {
            let (a, b) = (before.cell(c).and_then(|x| x.py.as_deref()), sh.cell(c).and_then(|x| x.py.as_deref()));
            match (a, b) {
                (Some(a), Some(b)) if a.code == b.code && a.object == b.object => {}
                (_, Some(_)) => {
                    ch.code.insert((si, c));
                }
                (Some(_), None) => ch.removed.push((si, c)),
                (None, None) => {}
            }
        }
        if before.spill != sh.spill {
            let keys: BTreeSet<CellRef> = before.spill.keys().chain(sh.spill.keys()).copied().collect();
            cells.extend(keys.into_iter().filter(|c| before.spill.get(c) != sh.spill.get(c)));
        }
        if cells.is_empty() {
            continue;
        }
        ch.sheets.insert(si, if cells.len() > MANY_CHANGES { Changes::All } else { Changes::Cells(cells) });
    }
    ch.lost_sheet = old.sheets.iter().any(|s| new.sheet_index(&s.name).is_none() && s.cells.iter().any(|(_, c)| c.py.is_some()));
    ch
}

/// The first cell (index into `cells`) that must run after an edit, if any.
fn first_affected(cells: &[PyCell], ch: &EditChanges) -> Option<usize> {
    if ch.lost_sheet {
        return (!cells.is_empty()).then_some(0);
    }
    let any = !ch.sheets.is_empty();
    let mut first: Option<usize> = None;
    let mut hit = |i: usize| first = Some(first.map_or(i, |f| f.min(i)));
    for (i, pc) in cells.iter().enumerate() {
        if pc.unknown && any {
            return Some(0);
        }
        if ch.code.contains(&pc.key) || (pc.volatile && any) || pc.refs.iter().any(|a| ch.sheets.get(&a.sheet).is_some_and(|c| c.hits(a))) {
            hit(i);
        }
    }
    // A removed cell may have defined names later cells use: they run again.
    for r in &ch.removed {
        if let Some(i) = cells.iter().position(|pc| pc.key > *r) {
            hit(i);
        }
    }
    first
}

/// What `xl(reference)` reads, seen from `at` on sheet `si`.
fn resolve_xl(wb: &Workbook, si: usize, at: CellRef, reference: &str, max_cells: usize) -> XlReply {
    let text = reference.trim().trim_start_matches('=');
    let Ok(expr) = gridcraft_formula::parse(text) else {
        return XlReply::Error(format!("xl(): `{reference}` is not a cell, range, name or table reference"));
    };
    let is_ref = |e: &Expr| matches!(e, Expr::Ref(_) | Expr::Name(_) | Expr::Struct(_) | Expr::Binary(BinOp::Range | BinOp::Intersect, ..));
    if !is_ref(&expr) {
        let why = if matches!(expr, Expr::Error(CellError::Ref)) {
            "refers to cells that were deleted"
        } else {
            "is not a cell, range, name or table reference"
        };
        return XlReply::Error(format!("xl(): `{reference}` {why}"));
    }
    let mut missing_sheet = None;
    expr.walk(&mut |e| {
        if let Expr::Ref(r) = e
            && let gridcraft_formula::SheetSel::Named(n) = &r.sheet
            && wb.sheet_index(n).is_none()
        {
            missing_sheet = Some(n.clone());
        }
    });
    if let Some(n) = missing_sheet {
        return XlReply::Error(format!("xl(): there is no sheet called `{n}`"));
    }
    if let Expr::Name(n) = &expr
        && wb.names.iter().all(|d| !d.name.eq_ignore_ascii_case(n))
        && wb.sheets.iter().all(|s| s.tables.iter().all(|t| !t.name.eq_ignore_ascii_case(n)))
    {
        return XlReply::Error(format!("xl(): there is no name or table called `{n}`"));
    }
    let v = gridcraft_calc::evaluate_expr(wb, si, at, &expr);
    if let Value::Array(a) = &v
        && a.rows.saturating_mul(a.cols) > max_cells
    {
        return XlReply::Error(format!("xl(): `{reference}` has more than {max_cells} cells"));
    }
    XlReply::Value(v)
}

fn failure_error(kind: FailureKind) -> CellError {
    match kind {
        FailureKind::Timeout => CellError::Timeout,
        FailureKind::Unavailable => CellError::Blocked,
        FailureKind::Error | FailureKind::Memory | FailureKind::Crashed => CellError::Python,
    }
}

fn failure_status(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Error => "error",
        FailureKind::Timeout => "timeout",
        FailureKind::Memory => "memory",
        FailureKind::Crashed => "stopped",
        FailureKind::Unavailable => "blocked",
    }
}

fn push_log(d: &mut DocState, mut entry: PyRunLog) {
    d.py_log_seq = d.py_log_seq.saturating_add(1);
    entry.seq = d.py_log_seq;
    d.py_log.push_back(entry);
    while d.py_log.len() > LOG_LIMIT {
        d.py_log.pop_front();
    }
}

fn log_entry(wb: &Workbook, (si, c): Key, status: &'static str) -> PyRunLog {
    PyRunLog {
        seq: 0,
        sheet: wb.sheet(si).map(|s| s.name.clone()).unwrap_or_default(),
        cell: c.a1(),
        status,
        ms: 0.0,
        stdout: String::new(),
        stderr: String::new(),
        error: None,
        line: None,
    }
}

/// Writes `f` into the Python cell at `key` (`None` output: an error value), lays out its spill
/// and recalculates what depends on it. Returns the positions whose content changed, by sheet.
fn write_cell(d: &mut DocState, key: Key, f: impl FnOnce(&mut gridcraft_model::PyCode, &mut Value)) -> HashMap<usize, Changes> {
    let mut wb = (*d.wb).clone();
    let Some(sh) = wb.sheet_mut(key.0) else { return HashMap::new() };
    let Some(cell) = sh.cells.get_mut(key.1) else { return HashMap::new() };
    let Some(py) = cell.py.as_mut() else { return HashMap::new() };
    f(Arc::make_mut(py), &mut cell.value);
    let changed = gridcraft_calc::lay_python_spill(&mut wb, key.0, key.1);
    d.calc.cells_changed(&mut wb, &changed);
    let old = std::mem::replace(&mut d.wb, Arc::new(wb));
    d.revision += 1;
    let mut out = HashMap::new();
    for (si, sh) in d.wb.sheets.iter().enumerate() {
        match old.sheets.get(si) {
            Some(o) if Arc::ptr_eq(o, sh) => {}
            Some(o) => {
                let cells = o.cells.diff(&sh.cells);
                let mut cells = cells;
                if o.spill != sh.spill {
                    let keys: BTreeSet<CellRef> = o.spill.keys().chain(sh.spill.keys()).copied().collect();
                    cells.extend(keys.into_iter().filter(|c| o.spill.get(c) != sh.spill.get(c)));
                }
                if !cells.is_empty() {
                    out.insert(si, if cells.len() > MANY_CHANGES { Changes::All } else { Changes::Cells(cells) });
                }
            }
            None => {
                out.insert(si, Changes::All);
            }
        }
    }
    out
}

/// Shows `err` in the Python cell at `key` (its result is cleared).
fn write_error(d: &mut DocState, key: Key, err: CellError) -> HashMap<usize, Changes> {
    write_cell(d, key, |py, value| {
        py.output = None;
        py.stale = false;
        *value = Value::Error(err);
    })
}

fn write_outcome(d: &mut DocState, key: Key, o: &RunOutcome) -> HashMap<usize, Changes> {
    let mut entry = log_entry(&d.wb, key, "ok");
    entry.ms = o.elapsed_ms;
    entry.stdout = o.stdout.clone();
    entry.stderr = o.stderr.clone();
    let changed = match &o.result {
        Ok(out) => write_cell(d, key, |py, _| {
            py.output = Some(PyOutput {
                value: out.value.clone(),
                type_name: out.type_name.clone(),
                preview: out.preview.clone(),
                convertible: out.convertible,
            });
            py.stale = false;
        }),
        Err(f) => {
            entry.status = failure_status(f.kind);
            entry.error = Some(f.message.clone());
            entry.line = f.line;
            write_error(d, key, failure_error(f.kind))
        }
    };
    push_log(d, entry);
    changed
}

impl Session {
    /// Sets how this session starts Python workers (tests use [`Launcher::InProcess`]). Running
    /// workers stop.
    pub fn set_python_launcher(&mut self, launcher: Launcher) {
        self.python.launcher = Some(launcher);
        for dp in self.python.docs.values_mut() {
            dp.worker = None;
            dp.job = None;
        }
    }

    /// Runs Python cells in the background, a step per [`Session::step_python`] call, instead of
    /// inside the command that made them run.
    pub fn set_python_background(&mut self, on: bool) {
        self.python.background = on;
    }

    /// Whether Python cells are waiting to run or running.
    pub fn python_busy(&self) -> bool {
        self.python.docs.values().any(|d| d.job.is_some())
    }

    /// Advances background Python work: takes finished results, starts the next cells. Call it
    /// every frame while [`Session::python_busy`]. Returns whether work remains.
    pub fn step_python(&mut self) -> bool {
        let uids: Vec<u64> = self.python.docs.iter().filter(|(_, d)| d.job.is_some()).map(|(u, _)| *u).collect();
        for uid in uids {
            self.drive_python(uid, false);
        }
        self.python_busy()
    }

    /// Runs queued Python work for every workbook to the end.
    pub fn finish_python(&mut self) {
        let uids: Vec<u64> = self.python.docs.iter().filter(|(_, d)| d.job.is_some()).map(|(u, _)| *u).collect();
        for uid in uids {
            self.drive_python(uid, true);
        }
    }

    /// The namespace no longer matches the cells: the next refresh resets it and runs every cell.
    pub(crate) fn python_mark_dirty(&mut self, uid: u64) {
        if let Some(dp) = self.python.docs.get_mut(&uid) {
            dp.dirty = true;
        }
    }

    /// Stops the workbook's worker (closing it, or replacing its document).
    pub(crate) fn python_forget(&mut self, uid: u64) {
        self.python.docs.remove(&uid);
    }

    /// Throws the active workbook's Python namespace away and runs every Python cell again (Reset
    /// Runtime).
    pub fn python_reset(&mut self) -> crate::Result<()> {
        let uid = self.doc()?.uid;
        self.python_rerun_all(uid);
        Ok(())
    }

    /// Stops the active workbook's Python cells: kills its worker and drops the queued run. Every
    /// cell that hadn't finished shows `#PYTHON!` ("Stopped"), and nothing runs again until the
    /// next edit or Reset Runtime (which start from a fresh namespace). Returns how many cells
    /// were stopped.
    pub fn python_stop(&mut self) -> crate::Result<usize> {
        let uid = self.doc()?.uid;
        let Some(dp) = self.python.docs.get_mut(&uid) else { return Ok(0) };
        let Some(job) = dp.job.take() else { return Ok(0) };
        if let Some(w) = dp.worker.as_mut() {
            w.kill();
        }
        // The namespace went with the worker: the next refresh starts over.
        dp.dirty = true;
        let mut pending: Vec<Key> = job.inflight.map(|i| i.key).into_iter().chain(job.replay).chain(job.queue).collect();
        let mut seen = HashSet::new();
        pending.retain(|k| seen.insert(*k));
        let Some(d) = self.docs.iter_mut().find(|d| d.uid == uid) else { return Ok(0) };
        for &key in &pending {
            write_error(d, key, CellError::Python);
            let mut e = log_entry(&d.wb, key, "stopped");
            e.error = Some("Stopped".into());
            push_log(d, e);
        }
        Ok(pending.len())
    }

    /// Throws the namespace away and runs every Python cell of the workbook again.
    pub(crate) fn python_rerun_all(&mut self, uid: u64) {
        let dp = self.python.docs.entry(uid).or_default();
        if let Some(w) = dp.worker.as_mut() {
            w.kill();
        }
        dp.job = None;
        dp.dirty = false;
        self.schedule_python(uid, Some(0), &HashSet::new(), true);
        self.finish_unless_background(uid);
    }

    fn finish_unless_background(&mut self, uid: u64) {
        if !self.python.background {
            self.drive_python(uid, true);
        }
    }

    /// Refreshes the Python cells for a change a command made since `before` (the active
    /// workbook as it was), so the command can report results; the refresh after the command is
    /// then skipped. Only for top-level commands.
    pub(crate) fn python_refresh_now(&mut self, id: &str, before: Arc<Workbook>) -> crate::Result<()> {
        // Inside another command, that command's refresh covers everything it changed.
        if self.depth != 1 {
            return Ok(());
        }
        let uid = self.doc()?.uid;
        self.python_after_command(id, Some((uid, before)));
        self.python.refreshed = true;
        Ok(())
    }

    /// After a top-level command: finds the Python cells the change affects and runs them.
    pub(crate) fn python_after_command(&mut self, id: &str, before: Option<(u64, Arc<Workbook>)>) {
        let Some((uid, old)) = before else { return };
        let Some(d) = self.docs.iter().find(|d| d.uid == uid) else {
            self.python_forget(uid);
            return;
        };
        let calculate = matches!(id, "formulas.calculateNow" | "formulas.calculateSheet");
        if Arc::ptr_eq(&d.wb, &old) && !calculate {
            return;
        }
        let new = d.wb.clone();
        if matches!(id, "edit.undo" | "edit.redo") {
            self.python.docs.entry(uid).or_default().dirty = true;
        }
        let dp = self.python.docs.entry(uid).or_default();
        let cells = dp.cells(&new);
        // (The per-sheet positions stay cached, so a workbook without Python cells costs only a
        // look at the bands each edit touched.)
        if cells.is_empty() {
            return;
        }
        let mut ch = edit_changes(&old, &new);
        // Spills of Python cells that an edit blocked or unblocked are laid out again.
        let reseeded = self.reseed_python_spills(uid, &cells, &ch);
        for (si, c) in reseeded {
            match ch.sheets.entry(si).or_insert_with(|| Changes::Cells(Vec::new())) {
                Changes::Cells(v) => v.extend(c),
                Changes::All => {}
            }
        }
        let mut first = first_affected(&cells, &ch);
        if calculate {
            let stale = cells.iter().position(|pc| self.docs.iter().find(|d| d.uid == uid).is_some_and(|d| is_stale(&d.wb, pc.key)));
            first = match (first, stale) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        let dirty = self.python.docs.get(&uid).is_some_and(|dp| dp.dirty);
        if first.is_none() && !(dirty && calculate) {
            return;
        }
        self.schedule_python(uid, first, &ch.code, calculate);
        self.finish_unless_background(uid);
    }

    /// Lays out again the spills of Python cells that show `#SPILL!` or whose area the edit
    /// touched. Returns the cells that changed.
    fn reseed_python_spills(&mut self, uid: u64, cells: &[PyCell], ch: &EditChanges) -> Vec<(usize, Vec<CellRef>)> {
        let Some(d) = self.docs.iter_mut().find(|d| d.uid == uid) else { return Vec::new() };
        let mut todo = Vec::new();
        for pc in cells {
            let Some(sh) = d.wb.sheet(pc.key.0) else { continue };
            let spill_hit = sh
                .spill_ranges
                .get(&pc.key.1)
                .is_some_and(|r| ch.sheets.get(&pc.key.0).is_some_and(|c| c.hits(&Area { sheet: pc.key.0, range: *r })));
            let blocked = sh.value(pc.key.1) == Value::Error(CellError::Spill);
            if (spill_hit || blocked) && !ch.code.contains(&pc.key) {
                todo.push(pc.key);
            }
        }
        if todo.is_empty() {
            return Vec::new();
        }
        let mut wb = (*d.wb).clone();
        let mut changed = Vec::new();
        for (si, c) in todo {
            changed.extend(gridcraft_calc::lay_python_spill(&mut wb, si, c));
        }
        d.calc.cells_changed(&mut wb, &changed);
        d.wb = Arc::new(wb);
        let mut by_sheet: HashMap<usize, Vec<CellRef>> = HashMap::new();
        for (si, c) in changed {
            by_sheet.entry(si).or_default().push(c);
        }
        by_sheet.into_iter().collect()
    }

    /// Queues the Python cells from `first` (an index in run order) on, or marks them stale or
    /// blocked when they can't run now. `code_changed` are the cells whose code the edit changed.
    fn schedule_python(&mut self, uid: u64, first: Option<usize>, code_changed: &HashSet<Key>, force: bool) {
        let Some(d) = self.docs.iter_mut().find(|d| d.uid == uid) else { return };
        let dp = self.python.docs.entry(uid).or_default();
        let wb = d.wb.clone();
        let cells = dp.cells(&wb);
        let mut first = first.unwrap_or(0);
        if dp.dirty {
            first = 0;
        }
        // A refresh already in progress restarts from the earlier of its next cell and `first`;
        // the run in flight finishes in the worker, but its result is thrown away.
        let mut discard = HashSet::new();
        if let Some(job) = dp.job.take() {
            let pending = job.inflight.as_ref().map(|i| i.key).into_iter().chain(job.replay.iter().copied()).chain(job.queue.iter().copied());
            if let Some(i) = pending.filter_map(|k| cells.iter().position(|pc| pc.key == k)).min() {
                first = first.min(i);
            }
            discard = job.discard;
            discard.extend(job.inflight.map(|i| i.id));
        }
        if dp.dirty {
            if let Some(w) = dp.worker.as_mut() {
                w.reset();
            }
            dp.dirty = false;
        }
        let queue: VecDeque<Key> = cells.iter().skip(first).map(|pc| pc.key).collect();
        if queue.is_empty() {
            return;
        }
        // An untrusted workbook runs nothing: new or edited code shows #BLOCKED!.
        if !d.trust.allows_code() {
            for &key in code_changed {
                write_error(d, key, CellError::Blocked);
                let mut e = log_entry(&d.wb, key, "blocked");
                e.error = Some("Python is turned off for this workbook until you enable it".into());
                push_log(d, e);
            }
            return;
        }
        // Manual calculation leaves them out of date until the workbook is calculated.
        if !force && d.wb.calc.mode != CalcMode::Automatic {
            let mut wb = (*d.wb).clone();
            for (si, c) in &queue {
                if let Some(py) = wb.sheet_mut(*si).and_then(|s| s.cells.get_mut(*c)).and_then(|x| x.py.as_mut()) {
                    Arc::make_mut(py).stale = true;
                }
            }
            d.wb = Arc::new(wb);
            d.revision += 1;
            return;
        }
        // In the background, cells waiting to run show #BUSY!.
        if self.python.background {
            for &key in &queue {
                write_error(d, key, CellError::Busy);
            }
        }
        dp.job = Some(Job { queue, discard, ..Job::default() });
    }

    /// Runs the workbook's queued cells: to the end when `blocking`, else as far as it can without
    /// waiting.
    fn drive_python(&mut self, uid: u64, blocking: bool) {
        let launcher = self.python.launcher.clone().unwrap_or_else(gridcraft_python::client::default_launcher);
        let limits = limits(&self.prefs);
        loop {
            let Some(d) = self.docs.iter_mut().find(|d| d.uid == uid) else {
                self.python.docs.remove(&uid);
                return;
            };
            let Some(dp) = self.python.docs.get_mut(&uid) else { return };
            let Some(job) = dp.job.as_mut() else { return };
            if job.inflight.is_none() {
                let next = job.replay.pop_front().map(|k| (k, true)).or_else(|| job.queue.pop_front().map(|k| (k, false)));
                let Some((key, replay)) = next else {
                    dp.job = None;
                    return;
                };
                let Some(code) = py_code(&d.wb, key) else { continue };
                let init = init_code(&d.wb);
                let worker = dp.worker(&launcher, &limits, init);
                match worker.submit(&code) {
                    Ok(id) => {
                        if let Some(job) = dp.job.as_mut() {
                            job.inflight = Some(Inflight { id, key, replay });
                        }
                    }
                    Err(f) => {
                        let o = RunOutcome { id: 0, result: Err(f), stdout: String::new(), stderr: String::new(), elapsed_ms: 0.0 };
                        finish_run(d, dp, key, replay, &o);
                    }
                }
                continue;
            }
            let Some(key) = job.inflight.as_ref().map(|i| i.key) else { continue };
            let wb = d.wb.clone();
            let max = limits.output_cells;
            let Some(worker) = dp.worker.as_mut() else {
                // The worker went away (a launcher change): the cell runs again.
                if let Some(job) = dp.job.as_mut()
                    && let Some(i) = job.inflight.take()
                {
                    job.queue.push_front(i.key);
                }
                continue;
            };
            let wait = if blocking { Duration::from_millis(10) } else { Duration::ZERO };
            let events = worker.poll(wait, &mut |reference, _headers| resolve_xl(&wb, key.0, key.1, reference, max));
            let mut progressed = false;
            for e in events {
                match e {
                    Event::InitError(err) => {
                        let mut entry = log_entry(&d.wb, key, "error");
                        entry.cell = String::new();
                        entry.sheet = String::new();
                        entry.error = Some(format!("Initialization: {}", err.message));
                        entry.line = err.line;
                        push_log(d, entry);
                    }
                    Event::Done(o) => {
                        let Some(job) = dp.job.as_mut() else { continue };
                        if job.discard.remove(&o.id) {
                            continue;
                        }
                        let Some(inflight) = job.inflight.take_if(|i| i.id == o.id) else { continue };
                        progressed = true;
                        finish_run(d, dp, inflight.key, inflight.replay, &o);
                    }
                }
            }
            if !blocking && !progressed {
                return;
            }
        }
    }
}

/// Whether the Python cell at `key` is out of date (manual calculation).
fn is_stale(wb: &Workbook, (si, c): Key) -> bool {
    wb.sheet(si).and_then(|s| s.cell(c)).and_then(|x| x.py.as_ref()).is_some_and(|p| p.stale)
}

/// Records a finished run: writes its result, checks for a cycle through earlier cells, and after
/// a worker restart reruns the earlier cells once before the queue continues.
fn finish_run(d: &mut DocState, dp: &mut DocPy, key: Key, replay: bool, o: &RunOutcome) {
    let changed = write_outcome(d, key, o);
    let worker_lost = matches!(&o.result, Err(Failure { kind: FailureKind::Timeout | FailureKind::Memory | FailureKind::Crashed, .. }))
        && dp.worker.as_ref().is_some_and(|w| !w.alive());
    let Some(job) = dp.job.as_mut() else { return };
    if replay {
        return;
    }
    // Python fed (through formulas, or directly) a Python cell that already ran: a cycle.
    let wb = d.wb.clone();
    let earlier: Vec<Key> = job
        .ran
        .iter()
        .copied()
        .filter(|k| {
            py_code(&wb, *k).is_some_and(|code| references(&wb, k.0, &code).0.iter().any(|a| changed.get(&a.sheet).is_some_and(|c| c.hits(a))))
        })
        .collect();
    for k in earlier {
        write_error(d, k, CellError::Calc);
        let mut e = log_entry(&d.wb, k, "cycle");
        e.error = Some(format!("This cell reads a result that {} computes after it", key.1.a1()));
        push_log(d, e);
    }
    job.ran.push(key);
    if worker_lost && !job.replayed {
        // The namespace went with the worker: the cells before this one run again first.
        job.replayed = true;
        job.replay = job.ran.iter().copied().filter(|k| *k != key).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xl_references_resolve_against_the_workbook() {
        let mut wb = Workbook::new();
        let sh = wb.sheet_mut(0).unwrap();
        sh.set_value(CellRef::new(0, 0), Value::Number(3.0));
        let max = 1_000_000;
        assert_eq!(resolve_xl(&wb, 0, CellRef::new(5, 5), "A1", max), XlReply::Value(Value::Number(3.0)));
        assert!(matches!(resolve_xl(&wb, 0, CellRef::new(5, 5), "A1:B2", max), XlReply::Value(Value::Array(_))));
        for bad in ["1+1", "#REF!", "nowhere", "Sheet9!A1:", "\"text\""] {
            assert!(matches!(resolve_xl(&wb, 0, CellRef::new(5, 5), bad, max), XlReply::Error(_)), "{bad}");
        }
        assert!(matches!(resolve_xl(&wb, 0, CellRef::new(5, 5), "A1:B2", 3), XlReply::Error(_)));
    }
}
