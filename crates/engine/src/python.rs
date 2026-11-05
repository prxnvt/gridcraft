//! Running Python cells.
//!
//! The session holds what the app allows ([`PythonCapability`], set by the app and by nothing a
//! command, a file or the control channel can reach) and, per open workbook, one worker and the state
//! of its cells. Nothing here blocks: a command only marks what is due; [`Session::python_poll`],
//! which the interface calls every frame, takes one finished result, writes it, and sends the next
//! cell. Results make no undo step, and stay out of the journal.
//!
//! - `inputs` resolves a cell's `xl("…")` references and fingerprints what they read.
//! - `plan` keeps the list of Python cells up to date and decides what is due.
//! - `runner` submits cells and writes results.
//! - `environment` works out on a thread of its own whether Python can run here.
//!
//! A due cell, with its kept result, shows `#BUSY!` until the new one arrives; what the result
//! spilled stays in place until then ([`gridcraft_model::PyCode::busy`]).

mod environment;
mod inputs;
mod plan;
mod runner;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gridcraft_core::CellRef;
use gridcraft_model::{CalcMode, Sheet, Workbook};
pub use gridcraft_python::Limits;
pub use gridcraft_python::Transport;
pub use gridcraft_python::describe::{Interpreter, Kind as InterpreterKind};
pub use gridcraft_python::diagnostics::{Action, Diagnostic, Kind, Scope, Severity};
use serde_json::{Value as Json, json};

use environment::Environment;
pub use environment::{PythonEnvironment, PythonState};
use plan::{CellState, Cx, DocRun, Gate, Source};
use runner::{Finish, Starter};

use crate::{DocState, Result, Session};

/// Makes a worker's transport, in place of the real one: a seam for tests.
#[doc(hidden)]
pub type TransportFactory = Arc<dyn Fn() -> Box<dyn Transport> + Send + Sync>;

/// What the app allows Python to do in this session. Only the app sets it
/// ([`Session::set_python_capability`]); there is no command, control method or file content that
/// can turn Python on or choose the interpreter. A session without one (the CLI, MCP) never starts
/// Python: its cells keep their saved values or show `#BLOCKED!`.
#[derive(Clone, Default)]
pub struct PythonCapability {
    /// The person turned Python on.
    pub enabled: bool,
    /// The interpreter the person chose; without one, GridCraft looks for one (uv's first).
    pub interpreter: Option<PathBuf>,
    /// The executable that starts the confined worker, by default this one. It must hand its
    /// arguments to `gridcraft_sandbox::handle_args` first.
    pub launcher: Option<PathBuf>,
    /// What one worker and its requests may use.
    pub limits: Limits,
    /// Workers from this factory instead of a confined interpreter (tests).
    #[doc(hidden)]
    pub transport: Option<TransportFactory>,
}

/// When a workbook's Python cells run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PythonCalculation {
    /// As soon as they are due.
    #[default]
    Automatic,
    /// Due cells are marked stale and keep their value until `python.run` or `python.runAll`.
    Manual,
}

/// One run of a cell, in a workbook's run log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonRun {
    pub sheet: String,
    pub cell: CellRef,
    pub duration: Duration,
    /// What the code printed, at most 64 KiB, kept also when it failed.
    pub stdout: String,
    /// The printed output was longer than that, or than the worker keeps.
    pub stdout_cut: bool,
    /// The id of what is worth saying about the run ([`Kind::id`]).
    pub diagnostic: Option<&'static str>,
}

impl PythonRun {
    fn new(
        d: &DocState,
        sheet: usize,
        cell: CellRef,
        duration: Duration,
        stdout: String,
        stdout_cut: bool,
        diagnostic: Option<&'static str>,
    ) -> PythonRun {
        PythonRun { sheet: d.wb.sheet(sheet).map(|s| s.name.clone()).unwrap_or_default(), cell, duration, stdout, stdout_cut, diagnostic }
    }
}

/// A diagnostic with the place it is about, for `python.diagnostics`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonDiagnosticEntry {
    pub diagnostic: Diagnostic,
    /// The sheet's index and the cell; `None` for what is about the whole workbook or the app.
    pub cell: Option<(usize, CellRef)>,
}

/// How much work the runtime has done, for tests of what it must not do.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PythonStats {
    /// Whole workbooks looked through for Python cells.
    pub full_scans: u64,
    /// Areas read to fingerprint them.
    pub areas_hashed: u64,
    /// Clients created.
    pub clients: u64,
}

static STATE_BUSY: LazyLock<Diagnostic> = LazyLock::new(|| Diagnostic::new("state.busy", "", None));
static STATE_STALE: LazyLock<Diagnostic> = LazyLock::new(|| Diagnostic::new("state.stale", "", None));
static OFF_APP: LazyLock<Diagnostic> = LazyLock::new(|| Diagnostic::new("off.app", "", None));
static OFF_WORKBOOK: LazyLock<Diagnostic> = LazyLock::new(|| Diagnostic::new("off.workbook", "", None));
static OFF_HEADLESS: LazyLock<Diagnostic> = LazyLock::new(|| Diagnostic::new("off.headless", "", None));

/// The Python state of a session.
#[derive(Default)]
pub(crate) struct Runtime {
    capability: Option<PythonCapability>,
    /// A capability was set in this session, so workbooks may hold Python state. Until then Python
    /// costs a command one check of this.
    ever: bool,
    env: Environment,
    docs: HashMap<u64, DocRun>,
    /// Which workbook goes first at the next poll, so that one doesn't starve the others.
    rotor: usize,
    stats: PythonStats,
}

impl Runtime {
    /// The workbook closed: its worker stops with its state.
    pub(crate) fn forget(&mut self, uid: u64) {
        self.docs.remove(&uid);
    }

    fn enabled(&self) -> bool {
        self.capability.as_ref().is_some_and(|c| c.enabled)
    }

    /// The reason Python doesn't run in a workbook, when it is about the app or the workbook.
    fn off_reason(&self, d: &DocState) -> Option<&'static Diagnostic> {
        match &self.capability {
            None => Some(&OFF_HEADLESS),
            Some(c) if !c.enabled => Some(&OFF_APP),
            Some(_) if !self.docs.get(&d.uid).map_or_else(|| d.trust.allows_code(), |r| r.on(d.trust)) => Some(&OFF_WORKBOOK),
            Some(_) => None,
        }
    }
}

fn manual(r: &DocRun, d: &DocState) -> bool {
    r.mode == PythonCalculation::Manual || d.wb.calc.mode == CalcMode::Manual
}

/// The Python cells of a sheet, row by row.
fn python_order_of_sheet(sh: &Sheet) -> Vec<CellRef> {
    sh.cells.iter().filter(|(_, c)| c.py.is_some()).map(|(c, _)| c).collect()
}

/// The workbook's Python cells in the order they run, as in Excel: sheets in tab order, each row by
/// row, left to right. Cells share their variables, so the order decides what each one sees.
pub fn python_order(wb: &Workbook) -> Vec<(usize, CellRef)> {
    wb.sheets.iter().enumerate().flat_map(|(i, sh)| python_order_of_sheet(sh).into_iter().map(move |c| (i, c))).collect()
}

impl Session {
    /// Gives this session what the app allows: `None` for a session that never runs Python. Stops
    /// every worker; cells run again in the new setup. An enabled capability starts the check of
    /// Python in the background ([`Session::python_environment`]). For the app only: no command
    /// reaches this.
    pub fn set_python_capability(&mut self, capability: impl Into<Option<PythonCapability>>) {
        self.python.ever = true;
        for d in &mut self.docs {
            if let Some(r) = self.python.docs.get_mut(&d.uid) {
                r.turn_off(d);
            }
        }
        self.python.capability = capability.into();
        let capability = self.python.capability.clone();
        self.python.env.restart(capability.as_ref());
        self.python_refresh();
    }

    /// What the app allows now.
    pub fn python_capability(&self) -> Option<&PythonCapability> {
        self.python.capability.as_ref()
    }

    /// Turns Python on or off for the active workbook. A workbook made in the app is on when the
    /// capability is; one opened from a file, or holding Python code pasted from one, is off until
    /// the person turns it on here. For the app only: no command reaches this. Turning it on makes
    /// every cell of the workbook due; turning it off stops its worker, and its cells keep what they
    /// show.
    pub fn set_python_workbook_enabled(&mut self, on: bool) {
        let Some(uid) = self.active().map(|d| d.uid) else { return };
        self.python.ever = true;
        self.python.docs.entry(uid).or_default().switch = Some(on);
        self.python_refresh();
    }

    /// Whether Python is on for the active workbook, whatever the capability says.
    pub fn python_workbook_enabled(&self) -> bool {
        self.active().is_some_and(|d| self.python.docs.get(&d.uid).map_or_else(|| d.trust.allows_code(), |r| r.on(d.trust)))
    }

    /// What the interface calls every frame, with no wait: picks up the check of Python, takes at
    /// most one finished result, writes it, recalculates, and sends the next due cell. With a wait
    /// it blocks up to that long for something to happen (for headless callers and tests). Returns
    /// whether anything changed. A session that was never given a capability does nothing.
    pub fn python_poll(&mut self, wait: Duration) -> bool {
        if !self.python.ever {
            return false;
        }
        let mut changed = self.python_take_environment(Duration::ZERO);
        changed |= self.python_refresh();
        if self.python_take_one_result(Duration::ZERO) {
            return true;
        }
        changed |= self.python_pump();
        if changed || wait.is_zero() {
            return changed;
        }
        self.python_wait(wait)
    }

    /// Whether Python is working or about to: a request runs, cells wait for one, or the check of
    /// Python hasn't answered. The interface keeps polling while it is true.
    pub fn python_busy(&self) -> bool {
        self.python.env.checking() || self.python.docs.values().any(|r| r.inflight.is_some() || r.due > 0)
    }

    /// What is wrong, or worth saying, about a Python cell of the active workbook. `None` for a
    /// cell with nothing to say, and for a cell that isn't a Python cell.
    pub fn python_diagnostic(&self, sheet: usize, cell: CellRef) -> Option<&Diagnostic> {
        let d = self.active()?;
        d.wb.sheet(sheet)?.cell(cell)?.py.as_ref()?;
        if let Some(off) = self.python.off_reason(d) {
            return Some(off);
        }
        let r = self.python.docs.get(&d.uid)?;
        let e = r.entries.get(r.index_of(sheet, cell)?)?;
        match e.state {
            CellState::Queued => Some(&STATE_BUSY),
            CellState::Stale if self.python.env.gate() == Gate::No => self.python.env.report.diagnostic.as_ref().or(Some(&STATE_STALE)),
            CellState::Stale => Some(&STATE_STALE),
            CellState::Clean if e.unsettled => e.diag.as_ref().filter(|x| x.kind.severity == Severity::Error).or(Some(&STATE_STALE)),
            CellState::Clean => e.diag.as_ref(),
        }
    }

    /// Everything [`Session::python_diagnostic`] says about the active workbook, in run order, after
    /// what is about the app or the workbook.
    pub fn python_diagnostics(&self) -> Vec<PythonDiagnosticEntry> {
        let Some(d) = self.active() else { return Vec::new() };
        let entry = |diagnostic: &Diagnostic, cell| PythonDiagnosticEntry { diagnostic: diagnostic.clone(), cell };
        let mut out = Vec::new();
        if let Some(off) = self.python.off_reason(d) {
            if !python_order(&d.wb).is_empty() {
                out.push(entry(off, None));
            }
            return out;
        }
        out.extend(self.python.env.report.diagnostic.iter().map(|x| entry(x, None)));
        let Some(r) = self.python.docs.get(&d.uid) else { return out };
        for e in &r.entries {
            if let Some(x) = self.python_diagnostic(e.sheet, e.at).filter(|x| x.kind.scope != Scope::Environment) {
                out.push(entry(x, Some((e.sheet, e.at))));
            }
        }
        out
    }

    /// Can Python cells run here, and in what. The check takes seconds and runs in the background:
    /// until it answers the state is [`PythonState::Running`] with no interpreter. It runs again
    /// when the capability changes and on `python.refreshEnvironment`. The state is also `Running`
    /// while a cell runs. On the web and on systems without a sandbox the answer, `blocked.platform`,
    /// is there at once.
    pub fn python_environment(&self) -> PythonEnvironment {
        let mut report = self.python.env.report.clone();
        if self.python.docs.values().any(|r| r.inflight.is_some()) {
            report.state = PythonState::Running;
        }
        report
    }

    /// The active workbook's last 200 runs, oldest first.
    pub fn python_run_log(&self) -> impl Iterator<Item = &PythonRun> {
        self.active().and_then(|d| self.python.docs.get(&d.uid)).into_iter().flat_map(|r| r.log.iter())
    }

    /// When the active workbook's Python cells run.
    pub fn python_calculation(&self) -> PythonCalculation {
        self.active().and_then(|d| self.python.docs.get(&d.uid)).map_or_else(PythonCalculation::default, |r| r.mode)
    }

    #[doc(hidden)]
    pub fn python_stats(&self) -> PythonStats {
        self.python.stats
    }

    /// The Python cells the runtime tracks in the active workbook, in the order it runs them.
    #[doc(hidden)]
    pub fn python_tracked_cells(&self) -> Vec<(usize, CellRef)> {
        self.active().and_then(|d| self.python.docs.get(&d.uid)).map_or_else(Vec::new, |r| r.entries.iter().map(|e| (e.sheet, e.at)).collect())
    }

    /// After a top-level command: works out what the command made due. A session that was never
    /// given a capability stops at its first check.
    pub(crate) fn python_after_command(&mut self) {
        if self.python.ever {
            self.python_refresh();
        }
    }

    /// Inside a command that reports the cell's value: the same, so the report says `#BUSY!`.
    pub(crate) fn python_after_edit(&mut self) {
        if self.python.ever && self.depth() <= 1 {
            self.python_refresh();
        }
    }

    /// Before content from another workbook arrives in the active one and it takes on that
    /// workbook's trust: Python code from a file turns Python off here until the person turns it on
    /// again, even if they had; content without code leaves Python as it was.
    pub(crate) fn python_note_pasted(&mut self, clip: &crate::Clipboard) {
        if !self.python.ever || clip.trust.allows_code() {
            return;
        }
        let Some(d) = self.docs.get(self.active_index()) else { return };
        let code = clip.wb.sheet(clip.sheet).is_some_and(|sh| sh.cells.iter_range(clip.range).any(|(_, c)| c.py.is_some()));
        let r = self.python.docs.entry(d.uid).or_default();
        let was_on = r.on(d.trust);
        if code {
            r.switch = None;
        } else if was_on {
            r.switch = Some(true);
        }
    }

    /// Plans every workbook that changed. Returns whether a workbook was written to.
    fn python_refresh(&mut self) -> bool {
        let enabled = self.python.enabled();
        let gate = self.python.env.gate();
        let Runtime { docs: runs, stats, .. } = &mut self.python;
        let docs = &mut self.docs;
        runs.retain(|uid, _| docs.iter().any(|d| d.uid == *uid));
        let mut changed = false;
        for d in docs.iter_mut() {
            let r = runs.entry(d.uid).or_default();
            changed |= if enabled && r.on(d.trust) {
                let cx = Cx { gate, manual: manual(r, d) };
                r.plan(d, cx, Source::Edit, stats)
            } else {
                r.stay_off(d)
            };
        }
        changed
    }

    /// Picks up the check of Python, and has every workbook act on the answer.
    fn python_take_environment(&mut self, wait: Duration) -> bool {
        if !self.python.env.take(wait) {
            return false;
        }
        self.python_rederive();
        true
    }

    /// The gate or a mode changed: every workbook marks again what is due.
    fn python_rederive(&mut self) -> bool {
        let gate = self.python.env.gate();
        let enabled = self.python.enabled();
        let Runtime { docs: runs, .. } = &mut self.python;
        let mut changed = false;
        for d in &mut self.docs {
            if let Some(r) = runs.get_mut(&d.uid).filter(|r| enabled && r.tracked) {
                let cx = Cx { gate, manual: manual(r, d) };
                changed |= r.derive(d, cx, Source::Edit, None);
            }
        }
        changed
    }

    /// Takes at most one finished result, from any workbook, starting with the next in turn.
    fn python_take_one_result(&mut self, wait: Duration) -> bool {
        let count = self.docs.len();
        for offset in 0..count {
            let i = self.python.rotor.saturating_add(offset) % count;
            let Some(uid) = self.docs.get(i).map(|d| d.uid) else { continue };
            let Some(result) = self.python.docs.get_mut(&uid).and_then(|r| r.poll_worker(wait)) else { continue };
            self.python.rotor = i.saturating_add(1);
            self.python_finish(i, uid, result);
            self.python_pump();
            return true;
        }
        false
    }

    fn python_finish(&mut self, doc: usize, uid: u64, result: gridcraft_python::RunResult) {
        let gate = self.python.env.gate();
        let Some(d) = self.docs.get_mut(doc) else { return };
        let Runtime { docs: runs, stats, .. } = &mut self.python;
        let Some(r) = runs.get_mut(&uid) else { return };
        let cx = Cx { gate, manual: manual(r, d) };
        let Finish { env_problem, .. } = r.finish(d, result, cx, stats);
        if let Some(problem) = env_problem {
            self.python.env.note_failure(problem);
            self.python_rederive();
        }
    }

    /// Sends the next due cell of every workbook that has none running.
    fn python_pump(&mut self) -> bool {
        let gate = self.python.env.gate();
        if gate != Gate::Go {
            return false;
        }
        let Runtime { docs: runs, stats, capability, env, .. } = &mut self.python;
        let Some(capability) = capability.as_ref() else { return false };
        let starter = Starter { capability, interpreter: env.interpreter() };
        let mut changed = false;
        for d in &mut self.docs {
            if let Some(r) = runs.get_mut(&d.uid).filter(|r| r.tracked) {
                let cx = Cx { gate, manual: manual(r, d) };
                changed |= r.pump(d, &starter, cx, stats);
            }
        }
        changed
    }

    /// Nothing is ready: waits up to `wait` on the first thing that will be.
    fn python_wait(&mut self, wait: Duration) -> bool {
        if self.python.docs.values().any(|r| r.inflight.is_some()) {
            return self.python_take_one_result_waiting(wait);
        }
        if self.python.env.checking() {
            return self.python_take_environment(wait);
        }
        false
    }

    fn python_take_one_result_waiting(&mut self, wait: Duration) -> bool {
        let Some((i, uid)) =
            self.docs.iter().enumerate().find(|(_, d)| self.python.docs.get(&d.uid).is_some_and(|r| r.inflight.is_some())).map(|(i, d)| (i, d.uid))
        else {
            return false;
        };
        let Some(result) = self.python.docs.get_mut(&uid).and_then(|r| r.poll_worker(wait)) else { return false };
        self.python_finish(i, uid, result);
        self.python_pump();
        true
    }

    /// The error a command gives when the active workbook's Python cells can't run now.
    fn python_refuse(&self) -> Result<()> {
        let d = self.doc()?;
        let cannot = |diag: &Diagnostic| {
            let detail = if diag.detail.is_empty() { String::new() } else { format!(" ({})", diag.detail) };
            crate::EngineError::Other(format!("{}{detail}", diag.kind.message))
        };
        if let Some(off) = self.python.off_reason(d) {
            return Err(cannot(off));
        }
        if self.python.env.gate() == Gate::No {
            return Err(self
                .python
                .env
                .report
                .diagnostic
                .as_ref()
                .map_or_else(|| crate::EngineError::Other("Python can't run here.".into()), cannot));
        }
        Ok(())
    }

    /// `python.run`: the cell runs now, whatever the calculation mode.
    pub(crate) fn python_run_cell(&mut self, sheet: usize, at: CellRef) -> Result<usize> {
        self.python_ready_to_run()?;
        let gate = self.python.env.gate();
        let d = self.docs.get_mut(self.active).ok_or(crate::EngineError::NoDocument)?;
        let Some(r) = self.python.docs.get_mut(&d.uid) else { return Ok(0) };
        let Some(index) = r.index_of(sheet, at) else {
            return Err(crate::EngineError::Other(format!("{} isn't a Python cell, so there is nothing to run", at.a1())));
        };
        let cx = Cx { gate, manual: manual(r, d) };
        Ok(r.run_one(d, index, cx))
    }

    /// `python.runAll` and `python.reset`: every cell runs, in a fresh worker when `fresh`.
    pub(crate) fn python_run_all(&mut self, fresh: bool) -> Result<usize> {
        self.python_ready_to_run()?;
        let d = self.docs.get_mut(self.active).ok_or(crate::EngineError::NoDocument)?;
        let Some(r) = self.python.docs.get_mut(&d.uid) else { return Ok(0) };
        Ok(r.run_everything(d, fresh))
    }

    /// Brings the list up to date for a command that asks cells to run. A worker that failed to
    /// start gets another try.
    fn python_ready_to_run(&mut self) -> Result<()> {
        if self.python.ever {
            self.python.env.retry();
            self.python_refresh();
        }
        self.python_refuse()
    }

    /// `python.stop`: stops the running cell and leaves the waiting ones stale.
    pub(crate) fn python_stop(&mut self) -> bool {
        let Some(d) = self.docs.get_mut(self.active) else { return false };
        self.python.docs.get_mut(&d.uid).is_some_and(|r| r.stop(d))
    }

    /// `python.calculation`: sets when the active workbook's cells run. Stays with the open workbook.
    pub(crate) fn python_set_calculation(&mut self, mode: PythonCalculation) {
        let gate = self.python.env.gate();
        let enabled = self.python.enabled();
        let Some(d) = self.docs.get_mut(self.active) else { return };
        let r = self.python.docs.entry(d.uid).or_default();
        if r.mode == mode {
            return;
        }
        r.mode = mode;
        if !enabled || !r.tracked {
            return;
        }
        let cx = Cx { gate, manual: manual(r, d) };
        if cx.manual {
            r.demote_queued();
            r.sync_flags(d);
        } else {
            r.derive(d, cx, Source::Edit, None);
        }
    }

    /// `python.refreshEnvironment`: checks Python again.
    pub(crate) fn python_refresh_environment(&mut self) {
        let capability = self.python.capability.clone();
        self.python.env.restart(capability.as_ref());
        self.python_rederive();
    }
}

/// A diagnostic as JSON, with the catalog's words.
fn diagnostic_json(d: &Diagnostic) -> Json {
    json!({
        "kind": d.kind.id,
        "severity": d.kind.severity.id(),
        "scope": d.kind.scope.id(),
        "message": d.kind.message,
        "action": d.kind.action.id(),
        "detail": d.detail,
        "line": d.line,
    })
}

/// The environment report as JSON (`python.environment`).
pub(crate) fn environment_json(e: &PythonEnvironment) -> Json {
    let path = |p: &std::path::Path| p.display().to_string();
    json!({
        "state": e.state,
        "diagnostic": e.diagnostic.as_ref().map(diagnostic_json),
        "sandbox": e.sandbox,
        "uv": e.uv.as_deref().map(path),
        "interpreter": e.interpreter.as_ref().map(|i| json!({
            "path": path(&i.path),
            "runAs": path(&i.run_as),
            "version": i.version.to_string(),
            "kind": i.kind.label(),
            "siteDirs": i.site_dirs.iter().map(|p| path(p)).collect::<Vec<_>>(),
            "readPaths": i.read_paths.iter().map(|p| path(p)).collect::<Vec<_>>(),
        })),
        "packages": e.packages.iter().map(|(name, version)| json!({"name": name, "version": version})).collect::<Vec<_>>(),
    })
}

/// A diagnostic row as JSON (`python.diagnostics`).
pub(crate) fn diagnostic_entry_json(wb: &Workbook, e: &PythonDiagnosticEntry) -> Json {
    let mut row = diagnostic_json(&e.diagnostic);
    let cell = e.cell.and_then(|(sheet, at)| wb.sheet(sheet).map(|sh| format!("{}!{}", gridcraft_core::quote_sheet(&sh.name), at.a1())));
    if let Some(o) = row.as_object_mut() {
        o.insert("cell".into(), json!(cell));
    }
    row
}

/// A run as JSON (`python.runLog`).
pub(crate) fn run_json(r: &PythonRun) -> Json {
    json!({
        "sheet": r.sheet,
        "cell": r.cell.a1(),
        "ms": r.duration.as_secs_f64() * 1000.0,
        "stdout": r.stdout,
        "stdoutCut": r.stdout_cut,
        "diagnostic": r.diagnostic,
    })
}

#[cfg(test)]
pub(crate) fn assigned_names_for_test(code: &str) -> Vec<&str> {
    runner::assigned_names(code)
}

#[cfg(test)]
pub(crate) const START_RETRIES_FOR_TEST: u32 = runner::START_RETRIES;
