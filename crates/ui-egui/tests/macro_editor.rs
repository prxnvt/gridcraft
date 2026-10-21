//! The macro step editor: editing, Add Step, breakpoints, dragging steps, and the debugger
//! (step, continue, run to the end as one undo step, stop with keep or undo).

use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::{Step, StepKind};
use gridcraft_ui_egui::{Services, SheetApp};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    harness_with(Default::default())
}

fn harness_with(services: Services) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, services);
    app.ui.ribbon_tab = "Automate".into();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn value(h: &Harness, a: &str) -> Value {
    h.state().session.active().and_then(|d| d.wb.active().map(|s| s.value(CellRef::parse(a).unwrap_or_default()))).unwrap_or_default()
}

/// Three steps that write A1, A2, A3.
fn three(h: &mut Harness) {
    let steps: Vec<_> = ["A1", "A2", "A3"].iter().map(|c| json!({"command": "cell.set", "params": {"cell": c, "input": c}})).collect();
    h.state_mut().run("macro.save", json!({"macro": {"name": "Three", "output": {"onConflict": "overwrite"}, "steps": steps}})).unwrap();
    h.state_mut().open_dialog("macroEditor", json!({"macro": "Three"}));
    h.run_steps(3);
}

fn draft_cells(h: &Harness) -> Vec<String> {
    let ed = h.state().macros.editor.as_ref().unwrap();
    ed.draft
        .steps
        .iter()
        .map(|s| match &s.kind {
            StepKind::Command { params, .. } => params["cell"].as_str().unwrap_or("").to_string(),
            _ => String::new(),
        })
        .collect()
}

fn click(h: &mut Harness, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

#[test]
fn new_macro_add_step_through_the_picker_and_save() {
    let mut h = harness();
    click(&mut h, "New Macro");
    assert!(h.state().macros.editor.is_some());
    click(&mut h, "Add Step");
    h.input_mut().events.push(Event::Text("bold".into()));
    h.run_steps(2);
    h.input_mut().events.push(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let ed = h.state().macros.editor.clone().unwrap();
    assert!(matches!(&ed.draft.steps[..], [s] if matches!(&s.kind, StepKind::Command { id, .. } if id == "home.bold")));
    assert_eq!(ed.selected.as_deref(), Some("1"));
    click(&mut h, "Save");
    let wb = &h.state().session.active().unwrap().wb;
    assert_eq!(wb.macros.len(), 1);
    assert_eq!(wb.macros[0].name, ed.draft.name);
    assert!(!h.state().macros.editor.as_ref().unwrap().dirty);
}

#[test]
fn the_gutter_toggles_breakpoints() {
    let mut h = harness();
    three(&mut h);
    click(&mut h, "Breakpoint 2");
    assert!(h.state().macros.editor.as_ref().unwrap().draft.steps[1].breakpoint);
    click(&mut h, "Breakpoint 2");
    assert!(!h.state().macros.editor.as_ref().unwrap().draft.steps[1].breakpoint);
}

#[test]
fn dragging_a_step_reorders_the_list() {
    let mut h = harness();
    three(&mut h);
    let from = h.get_by_label("Step 1: Set A1 to A1").rect().center();
    let to = h.get_by_label("Step 3: Set A3 to A3").rect().center() + egui::vec2(0.0, 8.0);
    h.input_mut().events.push(Event::PointerMoved(from));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    for k in 1..=6 {
        let p = from + (to - from) * (k as f32 / 6.0);
        h.input_mut().events.push(Event::PointerMoved(p));
        h.run_steps(1);
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(draft_cells(&h), ["A2", "A3", "A1"]);
    let ed = h.state().macros.editor.as_ref().unwrap();
    assert!(ed.dirty);
    assert_eq!(ed.selected.as_deref(), Some("3"), "the moved step stays selected");
}

#[test]
fn step_continue_and_finish_as_one_undo_step() {
    let mut h = harness();
    three(&mut h);
    click(&mut h, "Breakpoint 3");
    let undo_before = h.state().session.active().unwrap().undo.len();
    // Continue starts the run (saving the breakpoint first) and stops before step 3.
    click(&mut h, "Continue");
    let ed = h.state().macros.editor.clone().unwrap();
    assert_eq!(ed.debug.as_ref().and_then(|d| d["current"]["step"].as_str()), Some("3"));
    assert_eq!(ed.selected.as_deref(), Some("3"), "the current step is selected");
    assert!(h.query_by_label_contains("Paused before step 3").is_some());
    assert_eq!((value(&h, "A1"), value(&h, "A3")), (Value::Text("A1".into()), Value::Empty));
    assert!(h.state().grid.macro_overlay.as_ref().is_some_and(|o| !o.target.is_empty()), "the target is outlined");
    // Clearing the breakpoint while paused, then Step runs step 3 and the run finishes.
    click(&mut h, "Breakpoint 3");
    click(&mut h, "Step");
    assert!(h.state().session.macro_debug.is_none());
    assert_eq!(value(&h, "A3"), Value::Text("A3".into()));
    let ed = h.state().macros.editor.clone().unwrap();
    assert!(ed.outcome.as_deref().is_some_and(|o| o.starts_with("Finished")), "{:?}", ed.outcome);
    let doc = h.state().session.active().unwrap();
    // The breakpoint save is one entry, the run another.
    assert_eq!(doc.undo.last().map(|e| e.label.as_str()), Some("Run Macro: Three"));
    assert!(doc.undo.len() <= undo_before + 2);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!((value(&h, "A1"), value(&h, "A3")), (Value::Empty, Value::Empty));
}

#[test]
fn run_to_here_then_stop_asks_whether_to_keep_the_changes() {
    for keep in [false, true] {
        let mut h = harness();
        three(&mut h);
        click(&mut h, "Step 3: Set A3 to A3");
        click(&mut h, "Run to Here");
        assert_eq!(
            h.state().macros.editor.as_ref().unwrap().debug.as_ref().and_then(|d| d["current"]["step"].as_str()),
            Some("3"),
            "{:?} {:?}",
            h.state().message,
            h.state().macros.editor.as_ref().unwrap().outcome
        );
        click(&mut h, "Stop");
        assert!(h.state().macros.editor.as_ref().unwrap().ask_stop);
        click(&mut h, if keep { "Keep Changes" } else { "Undo Changes" });
        assert!(h.state().session.macro_debug.is_none());
        let want = if keep { Value::Text("A2".into()) } else { Value::Empty };
        assert_eq!(value(&h, "A2"), want, "keep = {keep}");
        assert_eq!(value(&h, "A3"), Value::Empty);
    }
}

#[test]
fn step_into_from_the_macros_dialog_pauses_before_the_first_step() {
    let mut h = harness();
    three(&mut h);
    h.state_mut().dialog = None;
    h.state_mut().macros.editor = None;
    h.state_mut().open_dialog("macros", json!({"name": "Three"}));
    h.run_steps(2);
    click(&mut h, "Step Into");
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("macroEditor"));
    assert!(h.state().session.macro_debug.is_some());
    assert!(h.query_by_label_contains("Debugging “Three”").is_some(), "the status bar says so");
}

#[test]
fn a_recorded_macro_opens_in_the_editor() {
    let mut h = harness();
    click(&mut h, "Record Macro");
    // The Record Macro dialog comes first.
    h.run_steps(2);
    click(&mut h, "OK");
    assert!(h.state().session.is_recording());
    h.state_mut().run("cell.set", json!({"cell": "B2", "input": "5"})).unwrap();
    h.state_mut().run("home.bold", json!({})).unwrap();
    h.run_steps(2);
    click(&mut h, "Stop");
    let name = h.state().session.active().unwrap().wb.macros[0].name.clone();
    h.state_mut().open_dialog("macroEditor", json!({"macro": name}));
    h.run_steps(3);
    let ed = h.state().macros.editor.clone().unwrap();
    assert!(!ed.draft.steps.is_empty());
    assert!(!ed.dirty);
}

#[test]
fn run_from_the_editor_asks_before_writing_over_data() {
    let mut h = harness();
    h.state_mut().run("cell.set", json!({"cell": "A2", "input": "keep me"})).unwrap();
    let steps = vec![json!({"command": "cell.set", "params": {"cell": "A2", "input": "new"}})];
    h.state_mut().run("macro.save", json!({"macro": {"name": "Over", "steps": steps}})).unwrap();
    h.state_mut().open_dialog("macroEditor", json!({"macro": "Over", "check": true}));
    h.run_steps(3);
    // Check marks the cell it would write over.
    let o = h.state().grid.macro_overlay.clone().unwrap();
    assert_eq!(o.conflicts.iter().map(|(_, r)| r.a1()).collect::<Vec<_>>(), ["A2"]);
    click(&mut h, "Run");
    assert!(h.state().macros.question.as_ref().is_some_and(|q| q.choices.len() == 3));
    click(&mut h, "Overwrite");
    assert_eq!(value(&h, "A2"), Value::Text("new".into()));
}

/// Gives the open editor's draft a step, as editing it does.
fn edit_draft(h: &mut Harness) {
    let ed = h.state_mut().macros.editor.as_mut().unwrap();
    ed.draft.steps.push(Step::command("cell.set", json!({"cell": "A1", "input": "7"})));
    ed.dirty = true;
    h.run_steps(2);
}

fn dialog(h: &Harness) -> Option<&str> {
    h.state().dialog.as_ref().map(|d| d.name())
}

/// Clicks a button of the Save Changes? prompt (the formula bar has a Cancel of its own).
fn answer(h: &mut Harness, label: &str) {
    h.query_all_by_label(label).last().unwrap().click();
    h.run_steps(3);
}

#[test]
fn closing_the_workbook_asks_about_an_unsaved_draft_and_cancel_brings_the_editor_back() {
    let mut h = harness();
    h.state_mut().open_dialog("macroEditor", json!({}));
    h.run_steps(3);
    edit_draft(&mut h);
    assert!(!h.state().session.active().unwrap().is_dirty());
    let r = h.state_mut().run("file.close", json!({})).unwrap();
    assert_eq!(r["closed"], false);
    h.run_steps(3);
    assert_eq!(dialog(&h), Some("saveChanges"));
    answer(&mut h, "Cancel");
    assert_eq!(dialog(&h), Some("macroEditor"));
    assert_eq!(h.state().macros.editor.as_ref().map(|e| e.draft.steps.len()), Some(1));
    // Don't Save closes the workbook and the editor, discarding the draft.
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(3);
    answer(&mut h, "Don't Save");
    assert!(h.state().macros.editor.is_none());
    assert_eq!(dialog(&h), None);
    assert!(h.state().session.active().unwrap().wb.macros.is_empty());
}

#[test]
fn save_in_the_prompt_saves_the_draft_into_the_workbook_file() {
    let path = std::env::temp_dir().join(format!("gridcraft-macro-draft-{}.xlsx", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let target = path.to_string_lossy().to_string();
    let mut h = harness_with(Services { pick_save: Some(Box::new(move |_| Some(target.clone()))), ..Default::default() });
    h.state_mut().open_dialog("macroEditor", json!({}));
    h.run_steps(3);
    edit_draft(&mut h);
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(dialog(&h), Some("saveChanges"));
    answer(&mut h, "Save");
    // The workbook was saved and closed, and the editor with it.
    assert!(h.state().macros.editor.is_none());
    assert!(h.state().session.documents().iter().all(|d| d.path.is_none()));
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    let wb = &s.active().unwrap().wb;
    assert_eq!(wb.macros.len(), 1);
    assert_eq!(wb.macros[0].steps.len(), 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_untouched_new_macro_does_not_make_closing_ask() {
    let mut h = harness();
    h.state_mut().open_dialog("macroEditor", json!({}));
    h.run_steps(3);
    assert!(h.state().macros.editor.as_ref().unwrap().dirty);
    let r = h.state_mut().run("file.close", json!({})).unwrap();
    assert_eq!(r["closed"], true);
    h.run_steps(3);
    assert_eq!(dialog(&h), None);
    assert!(h.state().macros.editor.is_none());
}
