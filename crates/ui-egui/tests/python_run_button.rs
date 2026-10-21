//! The formula bar's Python button: Run on a cell that hasn't run (committing code being edited
//! first), Stop while it runs or waits, Re-run once it has a result or an error, and Run
//! disabled while Python is turned off for the workbook. Not shown on normal cells.

use egui::Event;
use egui_kittest::kittest::Queryable;
use gridcraft_engine::core::{CellError, CellRef, Value};
use gridcraft_engine::{Session, Trust};
use gridcraft_python::client::Launcher;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::formula_bar::{BarLayout, RunButton, run_button_state};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness_with(launcher: Launcher, dark: bool) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(launcher);
    let mut app = SheetApp::new(s, Default::default());
    app.ui.dark = dark;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(16);
    h
}

fn harness() -> Harness {
    harness_with(Launcher::InProcess, false)
}

/// This test binary started again as a worker process, so Stop has a real process to kill.
fn process_launcher() -> Launcher {
    let program = std::env::current_exe().unwrap();
    let args = ["worker_process_entry", "--exact", "--nocapture", "--test-threads=1"].map(String::from).to_vec();
    Launcher::Process { program, args, env: vec![("GRIDCRAFT_UI_PYTHON_WORKER".into(), "1".into())] }
}

#[test]
fn worker_process_entry() {
    if std::env::var_os("GRIDCRAFT_UI_PYTHON_WORKER").is_some() {
        gridcraft_python::worker::serve(std::io::BufReader::new(std::io::stdin()), std::io::stdout(), gridcraft_python::worker::OnClose::ExitProcess);
        std::process::exit(0);
    }
}

fn wait_python(h: &mut Harness) {
    for _ in 0..1500 {
        h.run_steps(1);
        if !h.state().session.python_busy() {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("Python never finished");
}

fn select(h: &mut Harness, cell: &str) {
    h.state_mut().run("selection.set", json!({"cell": cell})).unwrap();
    h.run_steps(3);
}

fn value(h: &Harness, cell: &str) -> Value {
    h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::parse(cell).unwrap())
}

fn runs_of(h: &mut Harness, cell: &str) -> u64 {
    let log = h.state_mut().run("python.diagnostics", json!({})).unwrap();
    log["runs"].as_array().unwrap().iter().filter(|r| r["cell"] == cell).count() as u64
}

fn state(h: &Harness) -> Option<RunButton> {
    run_button_state(h.state())
}

const LABELS: [&str; 3] = ["Run Python cell", "Stop Python", "Re-run Python cell"];

#[test]
fn normal_cells_have_no_button_and_python_cells_rerun() {
    let mut h = harness();
    h.state_mut().run("cell.set", json!({"cell": "B1", "input": "5"})).unwrap();
    select(&mut h, "B1");
    assert_eq!(state(&h), None);
    for l in LABELS {
        assert!(h.query_by_label(l).is_none(), "{l}");
    }
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "21 * 2"})).unwrap();
    wait_python(&mut h);
    select(&mut h, "A1");
    assert_eq!(state(&h), Some(RunButton::Rerun));
    let before = runs_of(&mut h, "A1");
    h.get_by_label("Re-run Python cell").click();
    h.run_steps(2);
    wait_python(&mut h);
    assert_eq!(runs_of(&mut h, "A1"), before + 1, "it ran again");
    assert_eq!(value(&h, "A1"), Value::Number(42.0));
    // An error is a result too: Re-run.
    h.state_mut().run("python.set", json!({"cell": "A2", "code": "1/0"})).unwrap();
    wait_python(&mut h);
    select(&mut h, "A2");
    assert_eq!(state(&h), Some(RunButton::Rerun));
}

/// A new cell being written: Run commits the code (as Ctrl+Enter does) and runs it.
#[test]
fn run_commits_the_code_being_edited() {
    let mut h = harness();
    select(&mut h, "C3");
    h.state_mut().begin_python_edit();
    h.run_steps(3);
    assert_eq!(state(&h), Some(RunButton::Run { enabled: true }));
    h.input_mut().events.push(Event::Text("6 * 7".into()));
    h.run_steps(3);
    assert!(h.state().editor.as_ref().is_some_and(|e| e.python && e.text == "6 * 7"));
    h.get_by_label("Run Python cell").click();
    h.run_steps(2);
    wait_python(&mut h);
    assert!(h.state().editor.is_none(), "committed");
    assert_eq!(value(&h, "C3"), Value::Number(42.0));
    assert_eq!(state(&h), Some(RunButton::Rerun));
    // Editing it again: the same code shows Re-run, changed code Run; Run commits and runs.
    h.state_mut().begin_edit(None, true);
    h.run_steps(3);
    assert!(h.state().editor.as_ref().is_some_and(|e| e.python));
    assert_eq!(state(&h), Some(RunButton::Rerun));
    let before = runs_of(&mut h, "C3");
    h.get_by_label("Re-run Python cell").click();
    h.run_steps(2);
    wait_python(&mut h);
    assert!(h.state().editor.is_none());
    assert_eq!(runs_of(&mut h, "C3"), before + 1, "unchanged code runs once");
}

/// A cell that loops forever: Stop while it runs, then Re-run.
#[test]
fn stop_while_a_cell_runs() {
    let mut h = harness_with(process_launcher(), false);
    h.state_mut().session.prefs.python_timeout_seconds = 60.0;
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "while True:\n    pass"})).unwrap();
    h.state_mut().run("python.set", json!({"cell": "A2", "code": "2"})).unwrap();
    select(&mut h, "A2");
    assert_eq!(state(&h), Some(RunButton::Stop), "A2 waits on A1");
    select(&mut h, "A1");
    for _ in 0..20 {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(state(&h), Some(RunButton::Stop));
    assert_eq!(value(&h, "A1"), Value::Error(CellError::Busy));
    h.get_by_label("Stop Python").click();
    h.run_steps(3);
    assert!(!h.state().session.python_busy(), "stopped");
    assert_eq!(value(&h, "A1"), Value::Error(CellError::Python));
    assert_eq!(state(&h), Some(RunButton::Rerun));
}

/// Python turned off for a workbook from a file: Run, disabled, pointing to Enable Python.
#[test]
fn run_is_disabled_while_python_is_turned_off() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "1 + 1"})).unwrap();
    wait_python(&mut h);
    h.state_mut().session.active_mut().unwrap().trust = Trust::FromFile;
    select(&mut h, "A1");
    let s = state(&h).unwrap();
    assert_eq!(s, RunButton::Run { enabled: false });
    assert!(s.tooltip().contains("Enable Python"));
    let before = runs_of(&mut h, "A1");
    h.get_by_label("Run Python cell").click();
    h.run_steps(3);
    assert!(!h.state().session.python_busy());
    assert_eq!(runs_of(&mut h, "A1"), before, "nothing ran");
    assert!(h.state().message.is_none());
}

/// The bar keeps its layout: the field moves only by the PY badge and the button.
#[test]
fn the_bar_layout_stays_put() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "1"})).unwrap();
    wait_python(&mut h);
    select(&mut h, "B1");
    let normal = BarLayout::get(&h.ctx);
    select(&mut h, "A1");
    let python = BarLayout::get(&h.ctx);
    assert_eq!(normal.name_box, python.name_box);
    let shift = python.field.left() - normal.field.left();
    assert!(shift > 0.0 && shift < 80.0, "{shift}");
    assert_eq!(normal.field.right(), python.field.right());
    assert_eq!(normal.height, python.height);
}

/// `GRIDCRAFT_RUN_BUTTON_SNAPSHOTS=/dir` saves the bar on a normal cell and in each state, in
/// both themes, for review.
#[test]
fn snapshots() {
    let Ok(dir) = std::env::var("GRIDCRAFT_RUN_BUTTON_SNAPSHOTS") else { return };
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let mut h = harness_with(process_launcher(), dark);
        h.state_mut().session.prefs.python_timeout_seconds = 60.0;
        h.state_mut().run("cell.set", json!({"cell": "B1", "input": "5"})).unwrap();
        h.state_mut().run("python.set", json!({"cell": "A1", "code": "sum([1, 2, 3])"})).unwrap();
        wait_python(&mut h);
        let save = |h: &mut Harness, name: &str| {
            h.run_steps(3);
            h.render().unwrap().save(format!("{dir}/run-button-{name}-{theme}.png")).unwrap();
        };
        select(&mut h, "B1");
        save(&mut h, "normal");
        select(&mut h, "A1");
        save(&mut h, "rerun");
        select(&mut h, "A3");
        h.state_mut().begin_python_edit();
        h.input_mut().events.push(Event::Text("x = 1".into()));
        save(&mut h, "run");
        h.state_mut().cancel_edit();
        h.state_mut().run("python.set", json!({"cell": "A2", "code": "while True:\n    pass"})).unwrap();
        select(&mut h, "A2");
        save(&mut h, "stop");
        h.state_mut().run("python.stop", json!({})).unwrap();
        h.state_mut().session.active_mut().unwrap().trust = Trust::FromFile;
        select(&mut h, "A1");
        save(&mut h, "disabled");
    }
}
