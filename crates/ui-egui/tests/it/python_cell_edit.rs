//! Editing a Python cell's content (F2, the formula bar) edits its code; typing over it replaces
//! it, as typing over a formula does.

use egui::{Event, Key, Modifiers};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("python.set", json!({"cell": "A1", "code": "1 + 1", "output": "object"})).unwrap();
    session.execute("selection.set", json!({"range": "A1"})).unwrap();
    crate::common::harness_with(crate::common::builder().with_size(egui::vec2(1200.0, 800.0)), crate::common::app(session), 4)
}

fn text(h: &mut Harness, text: &str) {
    h.input_mut().events.push(Event::Text(text.into()));
    h.run_steps(2);
}

fn key(h: &mut Harness, key: Key) {
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(2);
}

/// A1's code and return type (true: a Python object), or `None` when it isn't a Python cell.
fn python(h: &Harness) -> Option<(String, bool)> {
    let sh = h.state().session.active().unwrap().wb.active().unwrap();
    sh.cell(CellRef::parse("A1").unwrap()).and_then(|c| c.py.as_deref()).map(|p| (p.code.clone(), p.object))
}

fn select_a1(h: &mut Harness) {
    h.state_mut().session.execute("selection.set", json!({"range": "A1"})).unwrap();
    h.run_steps(2);
}

#[test]
fn f2_edits_a_python_cells_code_and_typing_replaces_it() {
    let mut h = harness();
    let undo_depth = |h: &Harness| h.state().session.active().unwrap().undo.len();
    let before = undo_depth(&h);

    // F2 and Enter without a change writes nothing.
    key(&mut h, Key::F2);
    assert_eq!(h.state().editor.as_ref().map(|e| e.text.as_str()), Some("1 + 1"));
    key(&mut h, Key::Enter);
    assert!(h.state().editor.is_none());
    assert_eq!(undo_depth(&h), before, "an unchanged edit is no undo step");
    assert_eq!(python(&h), Some(("1 + 1".into(), true)));

    // A change is new code for the same Python cell, which keeps its return type.
    select_a1(&mut h);
    key(&mut h, Key::F2);
    text(&mut h, " + 2");
    key(&mut h, Key::Enter);
    assert!(h.state().editor.is_none(), "{:?}", h.state().message);
    assert_eq!(python(&h), Some(("1 + 1 + 2".into(), true)), "the cell stays a Python cell with the new code");

    // Typing over the cell replaces it.
    select_a1(&mut h);
    text(&mut h, "5");
    key(&mut h, Key::Enter);
    assert_eq!(python(&h), None);
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::parse("A1").unwrap()), Value::Number(5.0));
}
