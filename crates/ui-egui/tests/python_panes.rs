//! The Python Diagnostics pane, the Initialization and Python Options dialogs, the Enable Python
//! bar and the status bar note of which Python runs cells.

use egui_kittest::kittest::Queryable;
use gridcraft_engine::core::{CellError, CellRef, Value};
use gridcraft_engine::{Session, Trust};
use gridcraft_python::client::Launcher;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 860.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h
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

fn value(h: &Harness, cell: &str) -> Value {
    h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::parse(cell).unwrap())
}

fn set(h: &mut Harness, cell: &str, code: &str) {
    h.state_mut().run("python.set", json!({"cell": cell, "code": code})).unwrap();
    wait_python(h);
}

#[test]
fn the_diagnostics_pane_lists_runs_filters_and_selects_cells() {
    let mut h = harness();
    set(&mut h, "A1", "print('hello')\n1");
    set(&mut h, "B3", "x = 1\n1/0");
    h.state_mut().run("sheet.add", json!({})).ok();
    h.run_steps(2);
    h.state_mut().run("ui.ribbonTab", json!({"tab": "Formulas"})).unwrap();
    h.state_mut().grid.pane = Some(gridcraft_ui_egui::python_ui::PANE.into());
    h.run_steps(3);
    assert!(h.query_by_label("Python Diagnostics").is_some());
    assert!(h.query_by_label("Line 2: ZeroDivisionError: division by zero").is_some(), "the error with its line");
    assert!(h.query_by_label("hello").is_some(), "printed output");
    // Errors only hides the run that worked.
    h.get_by_label("Errors only").click();
    h.run_steps(2);
    assert!(h.query_by_label("hello").is_none());
    assert!(h.query_by_label("Line 2: ZeroDivisionError: division by zero").is_some());
    // Clicking the cell selects it, on its sheet.
    let sheet = h.state().session.active().unwrap().wb.sheets[0].name.clone();
    h.get_by_label(&format!("{sheet}!B3")).click();
    h.run_steps(2);
    let d = h.state().session.active().unwrap();
    assert_eq!(d.wb.active_sheet, 0);
    assert_eq!(d.selection.active, CellRef::new(2, 1));
    // This sheet: runs elsewhere drop out.
    h.state_mut().run("sheet.activate", json!({"sheet": 1})).ok();
    h.run_steps(2);
    if h.state().session.active().unwrap().wb.active_sheet == 1 {
        h.get_by_label("This sheet").click();
        h.run_steps(2);
        assert!(h.query_by_label("No Python runs yet.").is_some());
        h.get_by_label("This sheet").click();
        h.run_steps(2);
    }
    h.get_by_label("Clear").click();
    h.run_steps(2);
    assert!(h.state().session.active().unwrap().py_log.is_empty());
    assert!(h.query_by_label("No Python runs yet.").is_some());
}

#[test]
fn an_error_cell_offers_show_diagnostics() {
    let mut h = harness();
    set(&mut h, "A1", "1/0");
    assert_eq!(value(&h, "A1"), Value::Error(CellError::Python));
    h.state_mut().grid.py_card = Some((0, CellRef::new(0, 0)));
    h.run_steps(2);
    assert!(h.query_by_label("Line 1: ZeroDivisionError: division by zero").is_some(), "the card shows the error");
    h.get_by_label("Show Diagnostics").click();
    h.run_steps(2);
    assert_eq!(h.state().grid.pane.as_deref(), Some(gridcraft_ui_egui::python_ui::PANE));
    assert_eq!(h.state().grid.py_card, None);
}

#[test]
fn the_initialization_dialog_sets_code_every_cell_uses() {
    let mut h = harness();
    set(&mut h, "A1", "rate * 2");
    assert_eq!(value(&h, "A1"), Value::Error(CellError::Python), "`rate` isn't defined yet");
    h.state_mut().open_dialog("pythonInitialization", json!({}));
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.clone()).as_deref(), Some("pythonInitialization"));
    if let Some(d) = h.state_mut().dialog.as_mut() {
        d.values.insert("code".into(), json!("rate = 21"));
    }
    h.run_steps(1);
    h.get_by_label("   OK   ").click();
    h.run_steps(2);
    assert!(h.state().dialog.is_none(), "{:?}", h.state().message);
    wait_python(&mut h);
    assert_eq!(value(&h, "A1"), Value::Number(42.0));
    // It opens again with the saved code.
    h.state_mut().open_dialog("pythonInitialization", json!({}));
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().and_then(|d| d.values.get("code").cloned()), Some(json!("rate = 21")));
}

#[test]
fn a_workbook_from_a_file_shows_the_enable_bar_until_enabled() {
    let mut h = harness();
    h.run_steps(2);
    assert!(h.query_by_label("Enable Python").is_none(), "a new workbook needs no bar");
    set(&mut h, "A1", "6 * 7");
    // As if it had been opened from a file: Python is off, and saved values stay.
    h.state_mut().session.active_mut().unwrap().trust = Trust::FromFile;
    h.run_steps(2);
    assert!(h.query_by_label("Python cells are turned off for this workbook.").is_some());
    h.state_mut().run("python.set", json!({"cell": "A2", "code": "1"})).unwrap();
    h.run_steps(2);
    assert_eq!(value(&h, "A2"), Value::Error(CellError::Blocked), "nothing runs until enabled");
    h.get_by_label("Enable Python").click();
    wait_python(&mut h);
    assert_eq!(h.state().session.active().unwrap().trust, Trust::Trusted);
    assert!(h.query_by_label("Enable Python").is_none(), "the bar goes away");
    assert_eq!(value(&h, "A1"), Value::Number(42.0));
    assert_eq!(value(&h, "A2"), Value::Number(1.0));
}

#[test]
fn the_enable_bar_can_be_dismissed() {
    let mut h = harness();
    set(&mut h, "A1", "1");
    h.state_mut().session.active_mut().unwrap().trust = Trust::FromFile;
    h.run_steps(2);
    assert!(h.query_by_label("Enable Python").is_some());
    h.get_by_label("Dismiss this message").click();
    h.run_steps(2);
    assert!(h.query_by_label("Enable Python").is_none());
    assert_eq!(h.state().session.active().unwrap().trust, Trust::FromFile, "dismissing doesn't enable");
}

fn cpython() -> gridcraft_python::interp::Detection {
    gridcraft_python::interp::Detection {
        found: Some(gridcraft_python::interp::Interpreter {
            program: "/opt/homebrew/bin/python3".into(),
            args: vec![],
            version: "3.14.0".into(),
            pandas: Some("2.2.3".into()),
            numpy: Some("2.1.0".into()),
            externally_managed: true,
            source: "PATH".into(),
            ..Default::default()
        }),
        rejected: vec![gridcraft_python::interp::Rejected {
            path: "/usr/bin/python3".into(),
            reason: "it is Python 3.9.6; Python cells need 3.11 or newer".into(),
        }],
    }
}

fn builtin() -> gridcraft_python::interp::Detection {
    gridcraft_python::interp::Detection { found: None, rejected: vec![] }
}

#[test]
fn the_diagnostics_header_says_which_python_runs_cells() {
    let mut h = harness();
    set(&mut h, "A1", "1");
    h.state_mut().session.set_python_detection(builtin());
    h.state_mut().grid.pane = Some(gridcraft_ui_egui::python_ui::PANE.into());
    h.run_steps(3);
    assert!(h.query_by_label("Running on the built-in Python (Monty): install Python 3.11+ for packages").is_some());
    h.state_mut().session.set_python_detection(cpython());
    h.run_steps(3);
    assert!(h.query_by_label("Running on CPython 3.14 (/opt/homebrew/bin/python3), pandas 2.2.3, numpy 2.1.0").is_some());
    // The header's Options button opens Python Options.
    h.get_by_label("Options…").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("pythonOptions"));
}

#[test]
fn the_enable_bar_text_depends_on_the_python_that_runs_cells() {
    let mut h = harness();
    set(&mut h, "A1", "1");
    h.state_mut().session.active_mut().unwrap().trust = Trust::FromFile;
    h.state_mut().session.set_python_detection(builtin());
    h.run_steps(3);
    assert!(h.query_by_label("Python runs in a sandbox with no file or network access.").is_some());
    h.state_mut().session.set_python_detection(cpython());
    h.run_steps(3);
    assert!(
        h.query_by_label(
            "Python cells run with your Python at /opt/homebrew/bin/python3, with full access to your files and network. Enable only for workbooks you trust."
        )
        .is_some()
    );
    assert!(h.query_by_label("Python runs in a sandbox with no file or network access.").is_none());
    assert!(h.query_by_label("Enable Python").is_some());
}

#[test]
fn the_status_bar_says_which_python_runs_cells() {
    let mut h = harness();
    h.run_steps(2);
    assert!(h.query_by_label("Py: built-in").is_none(), "a workbook without Python shows nothing");
    set(&mut h, "A1", "1");
    assert!(h.query_by_label("Py: built-in").is_some(), "an explicit launcher is the built-in Python");
    h.state_mut().session.set_python_detection(cpython());
    h.run_steps(3);
    h.get_by_label("Py: CPython").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("pythonOptions"), "clicking it opens Python Options");
}

#[test]
fn python_options_shows_the_backend_and_sets_the_interpreter_and_time_limit() {
    let mut h = harness();
    h.state_mut().session.set_python_detection(cpython());
    h.state_mut().run("ui.ribbonTab", json!({"tab": "Formulas"})).unwrap();
    h.run_steps(3);
    h.state_mut().open_dialog("pythonOptions", json!({}));
    h.run_steps(3);
    for label in [
        "Running on CPython 3.14 (/opt/homebrew/bin/python3), pandas 2.2.3, numpy 2.1.0",
        "3.14.0",
        "/opt/homebrew/bin/python3",
        "2.2.3",
        "2.1.0",
        "Not used (1)",
    ] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert!(h.query_by_label_contains("managed by your system").is_some(), "the PEP 668 note");
    // The rejected candidates, with their reasons.
    h.get_by_label("Not used (1)").click();
    h.run_steps(3);
    assert!(h.query_by_label("/usr/bin/python3: it is Python 3.9.6; Python cells need 3.11 or newer").is_some());
    // OK sets the time limit and the interpreter (which starts a new search).
    if let Some(d) = h.state_mut().dialog.as_mut() {
        d.values.insert("timeoutSeconds".into(), json!(2.5));
        d.values.insert("interpreter".into(), json!("/nowhere/python3"));
    }
    h.run_steps(2);
    h.get_by_label("   OK   ").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().session.prefs.python_timeout_seconds, 2.5);
    assert_eq!(h.state().session.prefs.python_interpreter, "/nowhere/python3");
    // The built-in Python's wording, and Detect Again.
    h.state_mut().session.set_python_detection(builtin());
    h.state_mut().open_dialog("pythonOptions", json!({}));
    h.run_steps(3);
    assert!(h.query_by_label("Running on the built-in Python (Monty): install Python 3.11+ for packages").is_some());
    assert!(h.query_by_label_contains("sandbox with no file or network access").is_some());
    assert!(h.query_by_label("Detect Again").is_some());
}

#[test]
fn diagnostics_rows_collapse_repeats_and_the_header_lists_packages() {
    let mut h = harness();
    set(&mut h, "A1", "1/0");
    for _ in 0..2 {
        h.state_mut().run("python.reset", json!({})).unwrap();
        wait_python(&mut h);
    }
    h.state_mut().run("python.packages", json!({"packages": ["pandas", "polars"]})).unwrap();
    let mut found = cpython();
    if let Some(i) = found.found.as_mut() {
        i.installed = vec![("pandas".into(), "2.2.3".into())];
    }
    h.state_mut().session.set_python_detection(found);
    h.state_mut().grid.pane = Some(gridcraft_ui_egui::python_ui::PANE.into());
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().py_log.len(), 1, "one row for the three runs");
    assert!(h.query_by_label("\u{d7}3").is_some(), "with its count");
    assert!(h.query_by_label("Running on CPython 3.14 (/opt/homebrew/bin/python3), pandas 2.2.3, numpy 2.1.0").is_some());
    assert!(h.query_by_label("pandas 2.2.3").is_some());
    assert!(h.query_by_label("polars (missing)").is_some());
    assert!(h.query_by_label("Install polars…").is_some(), "missing packages can be installed");
    assert_eq!(gridcraft_ui_egui::python_ui::ago(1_500.0), "just now");
    assert_eq!(gridcraft_ui_egui::python_ui::ago(42_000.0), "42 s ago");
    assert_eq!(gridcraft_ui_egui::python_ui::ago(3.0 * 60_000.0), "3 min ago");
    assert_eq!(gridcraft_ui_egui::python_ui::ago(f64::NAN), "just now");
}
