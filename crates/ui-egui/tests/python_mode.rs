//! Python mode in the formula bar: entering it (`=PY` and Tab or Enter, Ctrl+Alt+Shift+P),
//! editing code (Enter, Tab, Ctrl+Enter), leaving it (the PY badge, the shortcut, Backspace in an
//! empty editor) and undo, the 5-line bar, F2 pointing with `xl()`, background runs and Stop.

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellError, CellRef, Value};
use gridcraft_python::client::Launcher;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::formula_bar::{BarLayout, MAX_LINES};
use gridcraft_ui_egui::grid::Geo;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness_with(launcher: Launcher) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(launcher);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(16);
    h
}

fn harness() -> Harness {
    harness_with(Launcher::InProcess)
}

const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };
const PY_SHORTCUT: Modifiers = Modifiers { alt: true, ctrl: true, shift: true, mac_cmd: false, command: true };

fn key_with(h: &mut Harness, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
    }
    h.run_steps(2);
}

fn key(h: &mut Harness, k: Key) {
    key_with(h, k, Modifiers::NONE);
}

fn text(h: &mut Harness, t: &str) {
    h.input_mut().events.push(Event::Text(t.into()));
    h.run_steps(2);
}

/// Types `t` into the grid: the first character starts the edit, the rest goes into the editor.
fn type_in_grid(h: &mut Harness, t: &str) {
    let mut chars = t.chars();
    if let Some(c) = chars.next() {
        text(h, &c.to_string());
    }
    let rest: String = chars.collect();
    if !rest.is_empty() {
        text(h, &rest);
    }
}

fn cell_center(h: &Harness, cell: &str) -> Pos2 {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    let geo = Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll);
    geo.cell_rect(sheet, CellRef::parse(cell).unwrap()).center()
}

fn click_at(h: &mut Harness, pos: Pos2) {
    h.input_mut().events.extend([
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(2);
}

fn drag(h: &mut Harness, from: Pos2, to: Pos2) {
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.input_mut().events.push(Event::PointerMoved(from));
    h.run_steps(1);
    h.input_mut().events.push(button(from, true));
    h.run_steps(1);
    for k in 1..=8 {
        let t = k as f32 / 8.0;
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * t));
        h.run_steps(1);
    }
    h.input_mut().events.push(button(to, false));
    h.run_steps(3);
}

fn value(h: &Harness, cell: &str) -> Value {
    h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::parse(cell).unwrap())
}

fn code(h: &Harness, cell: &str) -> Option<String> {
    let sh = h.state().session.active().unwrap().wb.active().unwrap().clone();
    sh.cell(CellRef::parse(cell).unwrap()).and_then(|c| c.py.as_ref()).map(|p| p.code.clone())
}

fn python_mode(h: &Harness) -> bool {
    h.state().editor.as_ref().is_some_and(|e| e.python)
}

fn editor_text(h: &Harness) -> String {
    h.state().editor.as_ref().map(|e| e.text.clone()).unwrap_or_default()
}

/// Steps frames until no Python runs (the in-process worker answers on its own thread).
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

fn select(h: &mut Harness, cell: &str) {
    h.state_mut().run("selection.set", json!({"cell": cell})).unwrap();
    h.run_steps(2);
}

#[test]
fn eq_py_then_tab_enters_python_mode() {
    let mut h = harness();
    type_in_grid(&mut h, "=PY");
    assert_eq!(editor_text(&h), "=PY");
    key(&mut h, Key::Tab);
    assert!(python_mode(&h), "=PY and Tab enter Python mode");
    let ed = h.state().editor.clone().unwrap();
    assert!(ed.from_formula_bar && ed.text.is_empty() && ed.cell == CellRef::new(0, 0), "{ed:?}");
    assert!(h.get_by_label("PY").rect().is_positive(), "the PY badge shows");
}

#[test]
fn eq_py_through_autocomplete_and_tab_enters_python_mode() {
    let mut h = harness();
    type_in_grid(&mut h, "=p");
    let ac = h.state().editor.as_ref().map(|e| e.autocomplete.clone()).unwrap_or_default();
    assert!(!ac.iter().any(|n| n == "PY") || ac.len() > 1, "PY is offered among the P functions or typed in full: {ac:?}");
    text(&mut h, "y");
    let ac = h.state().editor.as_ref().map(|e| e.autocomplete.clone()).unwrap_or_default();
    assert!(ac.is_empty() || ac == ["PY"], "{ac:?}");
    key(&mut h, Key::Tab);
    assert!(python_mode(&h));
}

#[test]
fn eq_py_then_enter_enters_python_mode() {
    let mut h = harness();
    type_in_grid(&mut h, "=py");
    key(&mut h, Key::Enter);
    assert!(python_mode(&h), "a bare =PY committed with Enter enters Python mode");
    assert_eq!(code(&h, "A1"), None, "nothing is committed yet");
    assert_eq!(h.state().session.active().unwrap().selection.active, CellRef::new(0, 0), "the selection stays");
}

#[test]
fn enter_and_tab_edit_code_and_ctrl_enter_commits_through_python_set() {
    let mut h = harness();
    key_with(&mut h, Key::P, PY_SHORTCUT);
    assert!(python_mode(&h), "Ctrl+Alt+Shift+P starts a Python cell");
    text(&mut h, "def f(x):");
    key(&mut h, Key::Enter);
    assert_eq!(editor_text(&h), "def f(x):\n    ", "Enter adds a line, indented after a colon");
    text(&mut h, "return x * 2");
    key(&mut h, Key::Enter);
    key_with(&mut h, Key::Tab, Modifiers::SHIFT);
    assert_eq!(editor_text(&h), "def f(x):\n    return x * 2\n", "Shift+Tab dedents");
    key(&mut h, Key::Tab);
    assert_eq!(editor_text(&h), "def f(x):\n    return x * 2\n    ", "Tab indents four spaces");
    key_with(&mut h, Key::Tab, Modifiers::SHIFT);
    text(&mut h, "f(21)");
    // F4 and function AutoComplete are off.
    key(&mut h, Key::F4);
    assert_eq!(editor_text(&h), "def f(x):\n    return x * 2\nf(21)");
    assert!(h.state().editor.as_ref().is_some_and(|e| e.autocomplete.is_empty()));
    assert!(python_mode(&h), "Enter and Tab don't commit");
    key_with(&mut h, Key::Enter, CTRL);
    assert!(h.state().editor.is_none(), "Ctrl+Enter commits: {:?}", h.state().message);
    assert_eq!(code(&h, "A1").as_deref(), Some("def f(x):\n    return x * 2\nf(21)"));
    wait_python(&mut h);
    assert_eq!(value(&h, "A1"), Value::Number(42.0));
    assert_eq!(h.state().session.active().unwrap().selection.active, CellRef::new(0, 0), "the cell stays selected");
    // Editing it again (F2) opens the code in the formula bar, in Python mode.
    key(&mut h, Key::F2);
    assert!(python_mode(&h));
    assert!(h.state().editor.as_ref().is_some_and(|e| e.from_formula_bar));
    assert_eq!(editor_text(&h), "def f(x):\n    return x * 2\nf(21)");
    key(&mut h, Key::Escape);
    assert!(h.state().editor.is_none(), "Esc cancels");
    assert_eq!(value(&h, "A1"), Value::Number(42.0));
}

#[test]
fn esc_cancels_a_new_python_cell() {
    let mut h = harness();
    key_with(&mut h, Key::P, PY_SHORTCUT);
    text(&mut h, "1 + 1");
    key(&mut h, Key::Escape);
    assert!(h.state().editor.is_none());
    assert_eq!(code(&h, "A1"), None);
    assert_eq!(value(&h, "A1"), Value::Empty);
}

/// Makes A1 a Python cell with `code`, run.
fn python_cell(h: &mut Harness, code: &str) {
    h.state_mut().run("python.set", json!({"cell": "A1", "code": code})).unwrap();
    wait_python(h);
}

#[test]
fn leaving_python_mode_three_ways_then_undo_brings_the_code_back() {
    for way in ["badge", "shortcut", "backspace"] {
        let mut h = harness();
        python_cell(&mut h, "6 * 7");
        assert_eq!(value(&h, "A1"), Value::Number(42.0));
        match way {
            "badge" => {
                // Clicking the badge on a selected Python cell starts the edit and leaves at once.
                h.get_by_label("PY").click();
                h.run_steps(2);
            }
            "shortcut" => {
                key(&mut h, Key::F2);
                assert!(python_mode(&h));
                key_with(&mut h, Key::P, PY_SHORTCUT);
            }
            _ => {
                key(&mut h, Key::F2);
                // Empty the editor, then Backspace once more.
                for _ in 0..5 {
                    key(&mut h, Key::Backspace);
                }
                assert!(python_mode(&h), "Backspace edits code while there is some");
                assert_eq!(editor_text(&h), "");
                key(&mut h, Key::Backspace);
            }
        }
        assert!(h.state().editor.is_some(), "{way}");
        assert!(!python_mode(&h), "{way} leaves Python mode");
        assert_eq!(editor_text(&h), "", "{way} clears the input");
        assert!(h.state().editor.as_ref().is_some_and(|e| e.py_left));
        key(&mut h, Key::Enter);
        assert!(h.state().editor.is_none(), "{way}: {:?}", h.state().message);
        assert_eq!(code(&h, "A1"), None, "{way}: the cell is a normal cell");
        assert_eq!(value(&h, "A1"), Value::Empty);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        wait_python(&mut h);
        assert_eq!(code(&h, "A1").as_deref(), Some("6 * 7"), "{way}: undo brings the code back");
        assert_eq!(value(&h, "A1"), Value::Number(42.0));
    }
}

#[test]
fn leaving_then_esc_keeps_the_python_cell() {
    let mut h = harness();
    python_cell(&mut h, "1");
    key(&mut h, Key::F2);
    key_with(&mut h, Key::P, PY_SHORTCUT);
    assert!(!python_mode(&h));
    key(&mut h, Key::Escape);
    assert_eq!(code(&h, "A1").as_deref(), Some("1"));
}

#[test]
fn convert_to_normal_cell_from_the_cell_menu() {
    let mut h = harness();
    python_cell(&mut h, "'hi'");
    let at = cell_center(&h, "A1");
    // Press and release in separate frames, as a real right-click is.
    h.input_mut().events.extend([
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(1);
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(h.state().grid.context_menu.is_some());
    h.get_by_label("Convert to Normal Cell").click();
    h.run_steps(3);
    assert_eq!(code(&h, "A1"), None);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(code(&h, "A1").as_deref(), Some("'hi'"));
}

#[test]
fn the_bar_shows_five_lines_of_code_then_scrolls() {
    let mut h = harness();
    let code8: String = (1..=8).map(|i| format!("x{i} = {i}")).collect::<Vec<_>>().join("\n");
    python_cell(&mut h, &code8);
    h.run_steps(16);
    let viewing = BarLayout::get(&h.ctx);
    assert_eq!(viewing.lines, 8, "a selected Python cell shows its code");
    assert_eq!(viewing.visible_lines, MAX_LINES, "up to five lines, then it scrolls");
    key(&mut h, Key::F2);
    h.run_steps(16);
    let editing = BarLayout::get(&h.ctx);
    assert_eq!(editing.lines, 8);
    assert_eq!(editing.visible_lines, MAX_LINES);
    let caret = editing.caret.expect("caret");
    assert!(editing.field.expand(1.0).contains(caret.center()), "the caret at the end stays in view: {caret:?} in {:?}", editing.field);
    assert!((editing.name_box.top() - viewing.name_box.top()).abs() < 0.5, "the Name Box stays on the first line");
}

#[test]
fn f2_points_at_cells_and_inserts_xl_calls() {
    let mut h = harness();
    key_with(&mut h, Key::P, PY_SHORTCUT);
    text(&mut h, "df = ");
    // Without F2, clicking a cell commits (as it does for a formula) — so turn pointing on first.
    key(&mut h, Key::F2);
    assert!(h.state().editor.as_ref().is_some_and(|e| e.py_point));
    let (b2, c6) = (cell_center(&h, "B2"), cell_center(&h, "C6"));
    drag(&mut h, b2, c6);
    assert_eq!(editor_text(&h), "df = xl(\"B2:C6\")", "dragging inserts one xl() call");
    assert!(python_mode(&h));
    let d1 = cell_center(&h, "D1");
    click_at(&mut h, d1);
    assert_eq!(editor_text(&h), "df = xl(\"D1\")", "a click replaces the call just inserted");
    // F2 again: pointing off; xl() references are still outlined in their colours.
    key(&mut h, Key::F2);
    assert!(h.state().editor.as_ref().is_some_and(|e| !e.py_point));
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

#[test]
fn a_slow_cell_runs_in_the_background_and_esc_stops_it() {
    let mut h = harness_with(process_launcher());
    h.state_mut().session.prefs.python_timeout_seconds = 60.0;
    // python.set answers at once; the run goes on in the background.
    let r = h.state_mut().run("python.set", json!({"cell": "A1", "code": "while True:\n    pass"})).unwrap();
    assert_eq!(r["value"], json!({"error": "#BUSY!"}));
    for _ in 0..30 {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(h.state().session.python_busy(), "it keeps running while frames go on");
    assert_eq!(value(&h, "A1"), Value::Error(CellError::Busy));
    assert!(h.get_by_label("Stop").rect().is_positive(), "the status bar offers Stop");
    key(&mut h, Key::Escape);
    assert!(!h.state().session.python_busy(), "Esc stops it");
    assert_eq!(value(&h, "A1"), Value::Error(CellError::Python));
    let log = h.state_mut().run("python.diagnostics", json!({})).unwrap();
    assert_eq!(log["runs"].as_array().and_then(|r| r.last()).map(|r| r["error"].clone()), Some(json!("Stopped")));
    // Nothing reruns by itself.
    for _ in 0..20 {
        h.run_steps(1);
    }
    assert!(!h.state().session.python_busy());
    // The status bar Stop button does the same.
    h.state_mut().run("python.set", json!({"cell": "A2", "code": "while True:\n    pass"})).unwrap();
    h.run_steps(4);
    assert!(h.state().session.python_busy());
    h.get_by_label("Stop").click();
    h.run_steps(2);
    assert!(!h.state().session.python_busy());
    assert_eq!(value(&h, "A2"), Value::Error(CellError::Python));
}

#[test]
fn stale_python_cells_and_object_cards() {
    let mut h = harness();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "{2, 1} | {3}", "output": "object"})).unwrap();
    wait_python(&mut h);
    assert_eq!(value(&h, "A1"), Value::text("set"), "an object shows its type");
    // Ctrl+Shift+F5 opens the preview card.
    select(&mut h, "A1");
    key_with(&mut h, Key::F5, Modifiers { shift: true, ..CTRL });
    assert_eq!(h.state().grid.py_card, Some((0, CellRef::new(0, 0))));
    let preview = h.state_mut().run("cell.get", json!({"cell": "A1"})).unwrap()["python"]["preview"].as_str().unwrap_or_default().to_string();
    assert!(preview.starts_with('{') && preview.contains('3'), "{preview}");
    assert!(h.get_by_label(&preview).rect().is_positive(), "the card shows the repr");
    key(&mut h, Key::Escape);
    assert_eq!(h.state().grid.py_card, None);
    // Clicking the PY tag opens it too.
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    let geo = Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll);
    let tag = gridcraft_ui_egui::grid::py_tag_rect(geo.cell_rect(sheet, CellRef::new(0, 0)), geo.z, gridcraft_engine::model::VAlign::Bottom).unwrap();
    click_at(&mut h, tag.center());
    assert_eq!(h.state().grid.py_card, Some((0, CellRef::new(0, 0))));
    let d8 = cell_center(&h, "D8");
    click_at(&mut h, d8);
    assert_eq!(h.state().grid.py_card, None, "a click elsewhere closes it");
    // Manual calculation leaves a changed cell stale until F9.
    h.state_mut().run("formulas.calculationOptions", json!({"mode": "manual"})).unwrap();
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "5", "output": "value"})).unwrap();
    h.run_steps(2);
    let stale = |h: &Harness| {
        h.state().session.active().unwrap().wb.active().unwrap().cell(CellRef::new(0, 0)).and_then(|c| c.py.clone()).is_some_and(|p| p.stale)
    };
    assert!(stale(&h));
    key(&mut h, Key::F9);
    wait_python(&mut h);
    assert!(!stale(&h));
    assert_eq!(value(&h, "A1"), Value::Number(5.0));
}
