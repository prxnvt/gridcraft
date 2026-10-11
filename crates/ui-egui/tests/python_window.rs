//! The Python editor window: opened from the formula bar's expand button (shown only for Python)
//! and `ui.pythonEditor`; it edits the same code as the formula bar, commits with Ctrl+Enter,
//! closes with Esc or its X keeping the edit, runs, stops and re-runs from its toolbar, shows the
//! last run's output and error (whose line a click goes to), follows the selection, and stays open
//! when another dialog opens.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellError, CellRef, Value};
use gridcraft_python::client::Launcher;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::formula_bar::BarLayout;
use gridcraft_ui_egui::python_window::{self, PyWindow};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness_with(launcher: Launcher, dark: bool) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(launcher);
    let mut app = SheetApp::new(s, Default::default());
    app.ui.dark = dark;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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

const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };

fn key_with(h: &mut Harness, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
    }
    h.run_steps(2);
}

fn text(h: &mut Harness, t: &str) {
    h.input_mut().events.push(Event::Text(t.into()));
    h.run_steps(2);
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

fn code(h: &Harness, cell: &str) -> Option<String> {
    let sh = h.state().session.active().unwrap().wb.active().unwrap().clone();
    sh.cell(CellRef::parse(cell).unwrap()).and_then(|c| c.py.as_ref()).map(|p| p.code.clone())
}

fn editor_text(h: &Harness) -> Option<String> {
    h.state().editor.as_ref().filter(|e| e.python).map(|e| e.text.clone())
}

fn window(h: &Harness) -> Option<PyWindow> {
    h.state().py_window
}

fn at(cell: &str) -> CellRef {
    CellRef::parse(cell).unwrap()
}

fn focused_in_window(h: &Harness) -> bool {
    h.ctx.memory(|m| m.focused()) == Some(python_window::editor_id())
}

fn runs_of(h: &mut Harness, cell: &str) -> u64 {
    let log = h.state_mut().run("python.diagnostics", json!({})).unwrap();
    log["runs"].as_array().unwrap().iter().filter(|r| r["cell"] == cell).map(|r| r["count"].as_u64().unwrap()).sum()
}

const EXPAND: &str = "Open in Python editor";

#[test]
fn the_expand_button_shows_only_for_python_and_opens_the_window() {
    let mut h = harness();
    h.state_mut().run("cell.set", json!({"cell": "B1", "input": "5"})).unwrap();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "21 * 2"})).unwrap();
    wait_python(&mut h);
    select(&mut h, "B1");
    assert!(h.query_by_label(EXPAND).is_none(), "not on a normal cell");
    let normal = BarLayout::get(&h.ctx);
    select(&mut h, "A1");
    let python = BarLayout::get(&h.ctx);
    assert_eq!(normal.field.right() - python.field.right(), 24.0, "the field makes room for the button");
    assert!(window(&h).is_none());
    h.get_by_label(EXPAND).click();
    h.run_steps(3);
    assert_eq!(window(&h), Some(PyWindow { sheet: 0, cell: at("A1") }));
    assert_eq!(editor_text(&h).as_deref(), Some("21 * 2"), "the cell's code is being edited");
    assert_eq!(h.state().editor.as_ref().map(|e| e.caret), Some(0), "from the top of the code");
    assert!(focused_in_window(&h));
    assert!(h.query_by_label_contains("Python: Sheet1!A1").is_some(), "the title names the cell");
    // A new Python cell being written (Insert Python) has the button too.
    h.state_mut().run("ui.pythonEditor", json!({"close": true})).unwrap();
    h.state_mut().cancel_edit();
    select(&mut h, "C3");
    h.state_mut().begin_python_edit();
    h.run_steps(3);
    assert!(h.query_by_label(EXPAND).is_some());
}

#[test]
fn typing_in_the_window_shows_in_the_formula_bar_and_ctrl_enter_commits() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "1"})).unwrap();
    wait_python(&mut h);
    let r = h.state_mut().run("ui.pythonEditor", json!({"cell": "A1"})).unwrap();
    assert_eq!(r["open"], true);
    assert_eq!(r["editing"], true);
    assert_eq!(r["cell"], "A1");
    h.run_steps(3);
    assert!(focused_in_window(&h));
    key_with(&mut h, Key::End, Modifiers::NONE);
    text(&mut h, " + 41");
    assert_eq!(editor_text(&h).as_deref(), Some("1 + 41"));
    assert_eq!(python_window::state(h.state())["dirty"], true);
    assert!(h.query_by_label_contains("\u{2022}").is_some(), "the title marks uncommitted changes");
    // The formula bar shows the code read-only (and doesn't take the keys).
    assert!(h.query_all_by_label("1 + 41").count() >= 1, "the bar shows the code");
    key_with(&mut h, Key::Enter, Modifiers::NONE);
    text(&mut h, "x = 2");
    assert_eq!(editor_text(&h).as_deref(), Some("1 + 41\nx = 2"), "Enter is a new line, not a commit");
    key_with(&mut h, Key::Enter, CTRL);
    assert!(h.state().editor.is_none(), "committed");
    assert_eq!(code(&h, "A1").as_deref(), Some("1 + 41\nx = 2"));
    assert!(window(&h).is_some(), "the window stays open");
    wait_python(&mut h);
    // Undo brings the old code back, as for the formula bar.
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(code(&h, "A1").as_deref(), Some("1"));
}

#[test]
fn esc_and_the_x_close_the_window_and_keep_the_edit() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "a = 1"})).unwrap();
    wait_python(&mut h);
    h.state_mut().run("ui.pythonEditor", json!({"cell": "A1"})).unwrap();
    h.run_steps(3);
    key_with(&mut h, Key::End, Modifiers::NONE);
    text(&mut h, "0");
    key_with(&mut h, Key::Escape, Modifiers::NONE);
    assert!(window(&h).is_none(), "Esc closes the window");
    assert_eq!(editor_text(&h).as_deref(), Some("a = 10"), "and keeps the edit");
    h.run_steps(3);
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(egui::Id::new("gridcraft.formula_bar")), "the formula bar has it now");
    // Reopened, the edit continues in the window; the X closes it, keeping the edit too.
    h.get_by_label(EXPAND).click();
    h.run_steps(3);
    assert!(window(&h).is_some());
    assert!(focused_in_window(&h));
    text(&mut h, "0");
    h.get_by_label("Close window").click();
    h.run_steps(3);
    assert!(window(&h).is_none());
    assert_eq!(editor_text(&h).as_deref(), Some("a = 100"));
    assert_eq!(code(&h, "A1").as_deref(), Some("a = 1"), "nothing committed");
    // The command closes it as well.
    h.state_mut().run("ui.pythonEditor", json!({})).unwrap();
    h.run_steps(2);
    let r = h.state_mut().run("ui.pythonEditor", json!({"close": true})).unwrap();
    assert_eq!(r["open"], false);
    // Another cell can't be opened over an edit.
    assert!(h.state_mut().run("ui.pythonEditor", json!({"cell": "B2"})).is_err());
}

#[test]
fn the_toolbar_runs_stops_and_reruns() {
    let mut h = harness_with(process_launcher(), false);
    h.state_mut().session.prefs.python_timeout_seconds = 60.0;
    select(&mut h, "A1");
    h.state_mut().run("ui.pythonEditor", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Select a Python cell").is_some(), "a normal cell");
    h.get_by_label("Insert Python Here").click();
    h.run_steps(3);
    assert!(editor_text(&h).is_some(), "a new Python cell is being written");
    text(&mut h, "6 * 7");
    h.get_by_label("Run").click();
    h.run_steps(2);
    wait_python(&mut h);
    assert!(h.state().editor.is_none(), "Run committed the code");
    assert_eq!(value(&h, "A1"), Value::Number(42.0));
    let before = runs_of(&mut h, "A1");
    h.get_by_label("Re-run").click();
    h.run_steps(2);
    wait_python(&mut h);
    assert_eq!(runs_of(&mut h, "A1"), before + 1);
    // A cell that never ends: Stop.
    h.state_mut().run("python.set", json!({"cell": "A3", "code": "while True:\n    pass"})).unwrap();
    select(&mut h, "A3");
    for _ in 0..20 {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(window(&h).map(|w| w.cell), Some(at("A3")), "the window follows the selection");
    assert!(h.query_by_label("Running\u{2026}").is_some());
    // The window's Stop (the status bar has one too).
    let window_rect = h.ctx.memory(|m| m.area_rect(python_window::window_id())).unwrap();
    h.get_all_by_label("Stop").find(|n| window_rect.contains(n.rect().center())).unwrap().click();
    h.run_steps(3);
    assert!(!h.state().session.python_busy(), "stopped");
    assert_eq!(value(&h, "A3"), Value::Error(CellError::Python));
    assert!(h.query_by_label("Re-run").is_some());
}

#[test]
fn the_output_strip_shows_printed_output_and_the_error_line() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "print(\"hello there\")\nx = 1\ny = x / 0"})).unwrap();
    wait_python(&mut h);
    select(&mut h, "A1");
    h.state_mut().run("ui.pythonEditor", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("hello there").is_some(), "printed output");
    let link = h.get_by_label_contains("Line 3:");
    link.click();
    h.run_steps(3);
    let ed = h.state().editor.clone().unwrap();
    assert_eq!(ed.caret, "print(\"hello there\")\nx = 1\n".chars().count(), "the caret is at the start of line 3");
    assert!(focused_in_window(&h));
    // Committed or not, a click on the line goes there.
    h.state_mut().cancel_edit();
    h.run_steps(3);
    h.get_by_label_contains("Line 3:").click();
    h.run_steps(3);
    assert_eq!(h.state().editor.as_ref().map(|e| e.caret), Some(27));
    // A result shows with its value.
    h.state_mut().cancel_edit();
    h.state_mut().run("python.set", json!({"cell": "B1", "code": "print(5)\n40 + 2"})).unwrap();
    wait_python(&mut h);
    select(&mut h, "B1");
    assert_eq!(window(&h).map(|w| w.cell), Some(at("B1")));
    assert!(h.query_by_label("42").is_some(), "the result");
    assert!(h.query_by_label("OK").is_some(), "the status");
}

#[test]
fn the_window_follows_the_selection_when_nothing_is_edited() {
    let mut h = harness();
    for (cell, code) in [("A1", "1"), ("A2", "2")] {
        h.state_mut().run("python.set", json!({"cell": cell, "code": code})).unwrap();
    }
    wait_python(&mut h);
    select(&mut h, "A1");
    h.state_mut().run("ui.pythonEditor", json!({})).unwrap();
    h.run_steps(3);
    // Editing A1: selecting elsewhere doesn't move the window off the edit.
    select(&mut h, "A2");
    assert_eq!(window(&h).map(|w| w.cell), Some(at("A1")));
    h.state_mut().cancel_edit();
    h.run_steps(3);
    assert_eq!(window(&h).map(|w| w.cell), Some(at("A2")));
    assert!(h.query_by_label_contains("Python: Sheet1!A2").is_some());
    select(&mut h, "C5");
    assert!(h.query_by_label("Select a Python cell").is_some());
    assert!(h.query_by_label("Insert Python Here").is_some());
    select(&mut h, "A1");
    assert!(h.query_by_label_contains("Python: Sheet1!A1").is_some());
    assert!(h.state().editor.is_none(), "following doesn't start an edit");
}

#[test]
fn another_dialog_leaves_the_window_open() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "1"})).unwrap();
    wait_python(&mut h);
    h.state_mut().run("ui.pythonEditor", json!({"cell": "A1"})).unwrap();
    h.run_steps(3);
    // Installs go to a CPython's environment: one that doesn't exist (nothing installs).
    let env = std::env::temp_dir().join(format!("gridcraft-ui-pywindow-{}", std::process::id()));
    let s = &mut h.state_mut().session;
    s.prefs.python_environment = env.display().to_string();
    let found = gridcraft_python::interp::Interpreter {
        program: "/nonexistent/gc/python3".into(),
        version: "3.14.0".into(),
        externally_managed: true,
        source: "PATH".into(),
        ..Default::default()
    };
    s.set_python_detection(gridcraft_python::interp::Detection { found: Some(found), rejected: vec![] });
    s.set_python_uv(false);
    h.state_mut().open_dialog("pythonInstall", json!({"packages": ["numpy"]}));
    h.run_steps(3);
    assert!(h.state().dialog.is_some());
    assert!(window(&h).is_some());
    h.state_mut().dialog = None;
    h.run_steps(3);
    assert!(window(&h).is_some());
    assert_eq!(editor_text(&h).as_deref(), Some("1"));
}

/// `GRIDCRAFT_PY_WINDOW_SNAPSHOTS=/dir` saves the window (editing, with output and an error, and
/// on a normal cell) in both themes, for review.
#[test]
fn snapshots() {
    let Ok(dir) = std::env::var("GRIDCRAFT_PY_WINDOW_SNAPSHOTS") else { return };
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let mut h = harness_with(Launcher::InProcess, dark);
        h.state_mut().run("range.setValues", json!({"range": "A1:B4", "values": [["Item", "Qty"], ["Pens", 3], ["Pads", 5], ["Ink", 2]]})).unwrap();
        let code = "rows = xl(\"A1:B4\", headers=True)\ntotal = 0\nfor r in rows:\n    total = total + r[1]\nprint(\"rows:\", len(rows))\nprint(\"total:\", total)\ntotal / 0";
        h.state_mut().run("python.set", json!({"cell": "D2", "code": code})).unwrap();
        h.state_mut().run("python.set", json!({"cell": "D3", "code": "print(\"ok\")\nsum([1, 2, 3])"})).unwrap();
        wait_python(&mut h);
        let save = |h: &mut Harness, name: &str| {
            h.run_steps(4);
            h.render().unwrap().save(format!("{dir}/py-window-{name}-{theme}.png")).unwrap();
        };
        h.state_mut().run("ui.pythonEditor", json!({"cell": "D2"})).unwrap();
        save(&mut h, "error");
        h.state_mut().cancel_edit();
        select(&mut h, "D3");
        save(&mut h, "result");
        select(&mut h, "F8");
        save(&mut h, "normal");
    }
}
