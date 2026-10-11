//! Python results you can use: the preview card's table with Copy as CSV, Copy as Markdown,
//! Save as .csv and Spill Here, and the output toggle (Ctrl+Alt+Shift+M, the PY badge menu,
//! Insert Python's menu and the cell menu).

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_python::client::Launcher;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

const OUTPUT_SHORTCUT: Modifiers = Modifiers { alt: true, ctrl: true, shift: true, mac_cmd: false, command: true };

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

fn object(h: &Harness, cell: &str) -> bool {
    let d = h.state().session.active().unwrap();
    d.wb.active().unwrap().cell(CellRef::parse(cell).unwrap()).and_then(|c| c.py.as_ref()).is_some_and(|p| p.object)
}

fn copied(h: &Harness) -> Vec<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

const TABLE: &str = "{'region': ['North', 'South, East'], 'note': ['a|b', 'say \"hi\"']}";

#[test]
fn the_card_shows_a_table_and_takes_it_out() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "B2", "code": TABLE, "output": "object"})).unwrap();
    wait_python(&mut h);
    assert_eq!(value(&h, "B2"), Value::text("dict 2 keys"));
    h.state_mut().grid.py_card = Some((0, CellRef::new(1, 1)));
    h.run_steps(3);
    for label in ["dict 2 keys", "region", "note", "North", "South, East", "a|b"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    h.get_by_label("Copy as CSV").click();
    h.run_steps(1);
    let csv = "region,note\r\nNorth,a|b\r\n\"South, East\",\"say \"\"hi\"\"\"\r\n";
    assert!(copied(&h).iter().any(|t| t == csv), "{:?}", copied(&h));
    h.run_steps(1);
    h.get_by_label("Copy as Markdown").click();
    h.run_steps(1);
    let md = "| region | note |\n| --- | --- |\n| North | a\\|b |\n| South, East | say \"hi\" |\n";
    assert!(copied(&h).iter().any(|t| t == md), "{:?}", copied(&h));
    h.run_steps(1);
    // Save as .csv (the UI command; the button asks the host's save dialog for the path).
    let path = std::env::temp_dir().join(format!("gridcraft-py-card-{}.csv", std::process::id()));
    let r = h.state_mut().run("python.saveCsv", json!({"cell": "B2", "path": path.display().to_string()})).unwrap();
    assert_eq!(r["rows"], 3);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), csv);
    let _ = std::fs::remove_file(&path);
    assert!(h.state_mut().run("python.saveCsv", json!({"cell": "Z9"})).is_err(), "not a Python cell");
    // Spill Here switches the cell to values.
    h.get_by_label("Spill Here").click();
    h.run_steps(2);
    wait_python(&mut h);
    assert!(!object(&h, "B2"));
    assert_eq!(value(&h, "B2"), Value::text("region"));
    assert_eq!(h.state().grid.py_card, None);
}

#[test]
fn a_non_table_object_keeps_its_repr() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "{1, 2, 3}", "output": "object"})).unwrap();
    wait_python(&mut h);
    assert_eq!(value(&h, "A1"), Value::text("set 3"));
    h.state_mut().grid.py_card = Some((0, CellRef::new(0, 0)));
    h.run_steps(3);
    assert_eq!(h.query_all_by_label("{1, 2, 3}").count(), 2, "the code in the formula bar, and the repr in the card");
    assert!(h.query_by_label("Copy as CSV").is_none());
}

#[test]
fn the_output_toggles_from_the_shortcut_and_the_menus() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "[1, 2, 3]"})).unwrap();
    wait_python(&mut h);
    assert!(!object(&h, "A1"));
    // Ctrl+Alt+Shift+M.
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key: Key::M, physical_key: None, pressed, repeat: false, modifiers: OUTPUT_SHORTCUT });
    }
    h.run_steps(2);
    wait_python(&mut h);
    assert!(object(&h, "A1"), "the shortcut switched it to a Python object");
    assert_eq!(value(&h, "A1"), Value::text("list 3"));
    // The PY badge's menu.
    h.get_by_label("Python Output (Ctrl+Alt+Shift+M)").click();
    h.run_steps(2);
    h.get_by_label("   Excel Values").click();
    h.run_steps(2);
    wait_python(&mut h);
    assert!(!object(&h, "A1"));
    assert_eq!(value(&h, "A3"), Value::Number(3.0), "values spill again");
    // Insert Python's menu on the Formulas tab.
    h.state_mut().run("ui.ribbonTab", json!({"tab": "Formulas"})).unwrap();
    h.run_steps(3);
    h.get_by_label("Insert Python options").click();
    h.run_steps(2);
    h.get_by_label("   Output as Python Object").click();
    h.run_steps(2);
    wait_python(&mut h);
    assert!(object(&h, "A1"));
    // The cell's menu.
    h.state_mut().grid.context_menu = Some(egui::pos2(300.0, 300.0));
    h.run_steps(2);
    h.get_by_label("   Output as Excel Values").click();
    h.run_steps(2);
    wait_python(&mut h);
    assert!(!object(&h, "A1"));
}

#[test]
fn multi_line_text_choice_round_trips_through_the_command() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "'one\\ntwo'"})).unwrap();
    wait_python(&mut h);
    assert_eq!(value(&h, "A1"), Value::text("one\ntwo"));
    h.state_mut().run("python.output", json!({"cell": "A1", "lines": "rows"})).unwrap();
    wait_python(&mut h);
    assert_eq!((value(&h, "A1"), value(&h, "A2")), (Value::text("one"), Value::text("two")));
}
