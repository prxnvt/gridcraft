//! Formulas › Python: the Insert Python split button and its menu, Reset and its menu (with
//! Stop), Diagnostics, and their keytips.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_python::client::Launcher;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h.state_mut().run("ui.ribbonTab", json!({"tab": "Formulas"})).unwrap();
    h.run_steps(4);
    h
}

fn wait_python(h: &mut Harness) {
    for _ in 0..1500 {
        h.run_steps(1);
        if !h.state().session.python_busy() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("Python never finished");
}

fn click(h: &mut Harness, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn runs(h: &mut Harness) -> usize {
    h.state_mut().run("python.diagnostics", json!({})).unwrap()["runs"].as_array().map_or(0, Vec::len)
}

#[test]
fn insert_python_starts_a_python_cell() {
    let mut h = harness();
    click(&mut h, "Insert Python");
    assert!(h.state().editor.as_ref().is_some_and(|e| e.python), "the main part starts Python mode");
    h.state_mut().cancel_edit();
    click(&mut h, "Insert Python options");
    click(&mut h, "Python Cell");
    assert!(h.state().editor.as_ref().is_some_and(|e| e.python && e.py_point), "pointing is on from the start, as in formulas");
    h.state_mut().cancel_edit();
    click(&mut h, "Insert Python options");
    click(&mut h, "Insert xl() Reference");
    assert!(h.state().editor.as_ref().is_some_and(|e| e.python && e.py_point), "Insert xl() Reference turns pointing on");
    h.state_mut().cancel_edit();
    click(&mut h, "Insert Python options");
    click(&mut h, "Python Options…");
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("pythonOptions"));
    h.state_mut().dialog = None;
    h.run_steps(2);
    click(&mut h, "Insert Python options");
    click(&mut h, "Python in GridCraft");
    assert!(h.state().message.as_ref().is_some_and(|(t, m)| t == "Python in GridCraft" && m.contains("xl(")));
}

#[test]
fn reset_reruns_every_cell_and_its_menu_clears_diagnostics() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "n = 2\nn"})).unwrap();
    wait_python(&mut h);
    assert_eq!(runs(&mut h), 1);
    click(&mut h, "Reset");
    wait_python(&mut h);
    assert_eq!(runs(&mut h), 2, "Reset runs the cell again");
    click(&mut h, "Reset options");
    // Stop is offered, but only while Python runs.
    assert!(h.get_by_label("Stop (Esc)").accesskit_node().is_disabled(), "nothing runs now");
    click(&mut h, "Reset and Clear Diagnostics");
    wait_python(&mut h);
    assert_eq!(runs(&mut h), 1, "the log starts over with the rerun");
    let v = h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::new(0, 0));
    assert_eq!(v, Value::Number(2.0));
}

#[test]
fn diagnostics_opens_the_pane() {
    let mut h = harness();
    click(&mut h, "Diagnostics");
    assert_eq!(h.state().grid.pane.as_deref(), Some("pythonDiagnostics"));
}

#[test]
fn python_keytips_open_their_menus() {
    let mut h = harness();
    for k in [Key::F10, Key::M, Key::Y] {
        for pressed in [true, false] {
            h.input_mut().events.push(Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
        }
        h.run_steps(2);
    }
    h.run_steps(2);
    assert!(h.query_by_label("Insert xl() Reference").is_some(), "MY opens the Insert Python menu");
}

#[test]
fn a_narrow_window_folds_calculation_then_python_into_one_button_each() {
    let at = |width: f32| {
        let mut s = Session::new();
        s.new_workbook();
        let mut app = SheetApp::new(s, Default::default());
        app.ui.ribbon_tab = "Formulas".into();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 700.0)).build_ui_state(
            |ui, app: &mut SheetApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(6);
        h
    };
    let wide = at(1700.0);
    assert!(wide.query_by_label("Calculate Now").is_some() && wide.query_by_label("Reset").is_some(), "nothing folds when it fits");
    let mid = at(1366.0);
    assert!(mid.query_by_label("Calculate Now").is_none(), "Calculation folds first");
    assert!(mid.query_by_label("Reset").is_some());
    // With the Function Library's small buttons (M14.10), the folded tab fits 1280 px.
    let mut narrow = at(1280.0);
    assert!(narrow.query_by_label("Reset").is_none(), "then Python");
    let o = gridcraft_ui_egui::ribbon::RibbonOverflow::get(&narrow.ctx);
    assert!(!o.overflows(), "folded, the tab fits: {} in {}", o.content, o.visible);
    for _ in 0..10 {
        narrow.run_steps(1);
        assert!(narrow.query_by_label("Reset").is_none(), "the fold doesn't flip between frames");
    }
    narrow.get_by_label("Python").click();
    narrow.run_steps(3);
    narrow.get_by_label("Insert Python").click();
    narrow.run_steps(3);
    assert!(narrow.state().editor.as_ref().is_some_and(|e| e.python));
}
