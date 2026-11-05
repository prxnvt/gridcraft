//! Turning Python on and understanding it: the preference and what the session gets from it, the
//! Enable bar, the status bar item, the interpreter list and the words all of them share. The
//! Options page is `backstage/python.rs`, the panes are `python_ui/panes.rs`, the ribbon tab is
//! `ribbon/python.rs`.
//!
//! # Who can turn Python on
//!
//! Only a person, with the widgets here: "Enable Python in cells", the interpreter, the limits, and
//! the workbook's switch. None of them is a command. Everything that writes them goes through
//! [`write_settings`] or [`set_workbook`], and both refuse while [`locked`]: a synthetic event from
//! the control channel (`ui.click`, `ui.drag`, `ui.key`, `ui.text`) is queued or was just applied.
//! The widgets are drawn disabled for those frames too, so such a click never reaches them.
//!
//! This keeps out what talks to the running app: commands, agents, MCP, the control channel, macros
//! and files. Its limit: someone who can edit the preferences file on disk, or who operates the real
//! mouse and keyboard or the system's accessibility interface, acts as the person does, and nothing
//! here can tell the difference.

use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::time::Duration;

use gridcraft_engine::python::{self, Diagnostic, Limits, MAX_TIMEOUT, TransportFactory};
use gridcraft_engine::{PythonCapability, PythonEnvironment, PythonState};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::SheetApp;
use crate::i18n::Language;
use crate::icons::Icon;

/// How often the app wakes while Python works, so results appear without input. An idle app stays
/// idle: nothing asks for a repaint then.
const BUSY_REPAINT: Duration = Duration::from_millis(100);

/// Frames the switches stay refused after the last synthetic input.
const LOCK_FRAMES: u64 = 2;

/// What the time limit field takes, in seconds. The upper end is what the client accepts.
pub const TIME_LIMIT_SECS: RangeInclusive<u64> = 1..=MAX_TIMEOUT.as_secs();

/// What the memory limit field takes, in MiB: below this a Python doesn't start with common
/// packages, and above it the limit stops meaning anything on any machine.
pub const MEMORY_LIMIT_MB: RangeInclusive<u64> = 128..=262_144;

const MIB: u64 = 1024 * 1024;

/// The person's Python settings, kept with the other preferences (`UiState::python`). Python is off
/// until a person turns it on: an unreadable value reads as the default, so off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PythonPrefs {
    /// Enable Python in cells.
    pub enabled: bool,
    /// The interpreter chosen by path; without one, Automatic.
    pub interpreter: Option<String>,
    pub time_limit_secs: u64,
    pub memory_limit_mb: u64,
}

impl Default for PythonPrefs {
    fn default() -> Self {
        let limits = Limits::default();
        PythonPrefs { enabled: false, interpreter: None, time_limit_secs: limits.timeout.as_secs(), memory_limit_mb: limits.memory_bytes / MIB }
    }
}

impl PythonPrefs {
    /// The limits a worker gets, within what the fields take, whatever a saved file says.
    pub fn limits(&self) -> Limits {
        let time = self.time_limit_secs.clamp(*TIME_LIMIT_SECS.start(), *TIME_LIMIT_SECS.end());
        let memory = self.memory_limit_mb.clamp(*MEMORY_LIMIT_MB.start(), *MEMORY_LIMIT_MB.end());
        Limits { timeout: Duration::from_secs(time), memory_bytes: memory.saturating_mul(MIB), ..Limits::default() }
    }
}

/// Reads saved settings leniently: anything malformed reads as the default, so Python is off and the
/// rest of the preferences survive.
pub fn lenient_prefs<'de, D: serde::Deserializer<'de>>(d: D) -> Result<PythonPrefs, D::Error> {
    let v = Json::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// The interface's Python state. What the person chose is in `UiState::python`.
#[derive(Default)]
pub struct PythonUi {
    /// What the session was last given.
    given: Option<PythonPrefs>,
    /// Whether this system can confine a worker, asked once.
    sandbox: Option<Result<String, String>>,
    /// The frame the switches stay refused until, after synthetic input.
    locked_until: Option<u64>,
    /// Workers from this factory instead of a real interpreter (tests).
    #[doc(hidden)]
    pub transport: Option<TransportFactory>,
    /// This report instead of a check of Python (tests).
    #[doc(hidden)]
    pub report: Option<PythonEnvironment>,
}

impl PythonUi {
    /// For tests: this answer about the sandbox, on any system.
    #[doc(hidden)]
    pub fn set_sandbox(&mut self, answer: Result<String, String>) {
        self.sandbox = Some(answer);
    }

    /// For tests: gives the session a new capability at the next frame.
    #[doc(hidden)]
    pub fn give_again(&mut self) {
        self.given = None;
    }

    /// Why this system can't confine a worker, if it can't.
    pub fn no_sandbox(&self) -> Option<&str> {
        self.sandbox.as_ref().and_then(|s| s.as_ref().err()).map(String::as_str)
    }
}

/// What the app does for Python every frame, from `SheetApp::logic`: gives the session what the
/// person chose when that changed, and picks up what is ready. Nothing here waits, so a frame never
/// blocks on Python.
pub fn tick(app: &mut SheetApp, ctx: &egui::Context) {
    if app.python.sandbox.is_none() {
        app.python.sandbox = Some(python::sandbox_support());
    }
    if app.synthetic_pending || !app.synthetic.is_empty() {
        app.python.locked_until = Some(ctx.cumulative_frame_nr().saturating_add(LOCK_FRAMES));
    }
    sync(app);
    if app.session.python_poll(Duration::ZERO) {
        ctx.request_repaint();
    }
    if app.session.python_busy() {
        ctx.request_repaint_after(BUSY_REPAINT);
    }
}

/// Gives the session what the person chose, when that changed since it was last given.
fn sync(app: &mut SheetApp) {
    if app.python.given.as_ref() == Some(&app.ui.python) {
        return;
    }
    let prefs = app.ui.python.clone();
    let capability = PythonCapability {
        enabled: prefs.enabled,
        interpreter: prefs.interpreter.as_ref().map(PathBuf::from),
        launcher: None,
        limits: prefs.limits(),
        transport: app.python.transport.clone(),
        report: app.python.report.clone(),
    };
    app.session.set_python_capability(capability);
    app.python.given = Some(prefs);
}

/// Whether synthetic input is about: the switches refuse it, whatever it is aimed at.
pub fn locked(app: &SheetApp, ctx: &egui::Context) -> bool {
    app.synthetic_pending || !app.synthetic.is_empty() || app.python.locked_until.is_some_and(|until| ctx.cumulative_frame_nr() <= until)
}

/// The state to show: on a system with no sandbox Python is blocked, whatever the setting says.
pub fn state(app: &SheetApp) -> PythonState {
    if app.python.no_sandbox().is_some() { PythonState::Blocked } else { app.session.python_state() }
}

/// The state's name, as a translation key.
pub fn state_label(state: PythonState) -> &'static str {
    match state {
        PythonState::Off => "Python|Off",
        PythonState::NotSetUp => "Python|Not Set Up",
        PythonState::Ready => "Python|Ready",
        PythonState::Running => "Python|Running",
        PythonState::Blocked => "Python|Blocked",
    }
}

/// The state's icon. Each has its own shape as well as its own colour.
pub fn state_icon(state: PythonState) -> Icon {
    match state {
        PythonState::Off => Icon::PythonOff,
        PythonState::NotSetUp => Icon::PythonNotSetUp,
        PythonState::Ready => Icon::PythonReady,
        PythonState::Running => Icon::PythonRunning,
        PythonState::Blocked => Icon::PythonBlocked,
    }
}

/// Why the state is what it is, or what is worth saying although it is fine.
pub fn reason(app: &SheetApp) -> Option<Diagnostic> {
    if let Some(why) = app.python.no_sandbox() {
        return Some(Diagnostic::new("blocked.platform", why, None));
    }
    let report = app.session.python_report().diagnostic.clone();
    let urgent = report.as_ref().is_some_and(|d| d.kind.id != "packages.noPandas");
    if !urgent && !app.session.python_workbook_enabled() && app.session.python_has_cells() {
        return Some(Diagnostic::new("off.workbook", "", None));
    }
    report
}

/// Whether Python is being checked rather than running a cell.
pub fn checking(app: &SheetApp) -> bool {
    state(app) == PythonState::Running && app.session.python_running().is_none() && app.session.python_report().interpreter.is_none()
}

/// A diagnostic in the interface's language, with its specifics. The specifics are data from the
/// system or from Python, and are shown as they came, on one line and cut.
pub fn sentence(lang: Language, d: &Diagnostic) -> String {
    let message = lang.tr(d.kind.message);
    let detail: String = d.detail.lines().next().unwrap_or_default().chars().take(200).collect();
    if detail.is_empty() { message.to_string() } else { format!("{message} ({detail})") }
}

/// Why cells can't be asked to run now, in the interface's language. A worker that didn't start can
/// be tried again, so that isn't a reason.
pub fn run_blocked(app: &SheetApp) -> Option<String> {
    let lang = app.ui.language;
    match reason(app) {
        Some(d) if d.kind.id == "connect.failed" || d.kind.id == "packages.noPandas" => {}
        Some(d) => return Some(sentence(lang, &d)),
        None => {}
    }
    (!app.session.python_workbook_enabled()).then(|| lang.tr(python_off_workbook()).to_string())
}

fn python_off_workbook() -> &'static str {
    python::CATALOG.iter().find(|k| k.id == "off.workbook").map_or("", |k| k.message)
}

/// Why this system can't run Python, in the interface's language.
pub fn system_blocked(app: &SheetApp) -> Option<String> {
    app.python.no_sandbox().map(|why| sentence(app.ui.language, &Diagnostic::new("blocked.platform", why, None)))
}
