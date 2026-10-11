//! The Python Diagnostics pane, the Initialization dialog and the Enable Python bar.

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
