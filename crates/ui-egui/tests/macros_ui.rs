//! Automate › Macros in the window: recording with the status bar's Stop, the Macros dialog,
//! macro shortcuts and the prompt when a macro's output would land on data.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(tab: &str) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.ribbon_tab = tab.into();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
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

fn save(h: &mut Harness, m: serde_json::Value) {
    h.state_mut().run("macro.save", json!({"macro": m})).unwrap();
    h.run_steps(2);
}

#[test]
fn record_from_the_ribbon_and_stop_from_the_status_bar() {
    let mut h = harness("Automate");
    h.get_by_label("Record Macro").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("recordMacro"), "Record Macro asks for a name first");
    assert!(!h.state().session.is_recording());
    h.run_steps(3);
    h.get_by_label("   OK   ").click();
    h.run_steps(2);
    assert!(h.state().session.is_recording());
    // While recording the toggle can't change and the button says Stop Recording.
    assert!(h.query_by_label("Stop Recording").is_some());
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "7"})).unwrap();
    h.run_steps(2);
    assert!(h.query_by_label_contains("Recording · 1 step").is_some(), "the status bar counts the steps");
    h.get_by_label("Stop").click();
    h.run_steps(2);
    assert!(!h.state().session.is_recording());
    let wb = &h.state().session.active().unwrap().wb;
    assert_eq!(wb.macros.len(), 1);
    let name = wb.macros[0].name.clone();
    assert_eq!(h.state().macros.recent, [name]);
    assert!(h.query_by_label_contains("Recording ·").is_none());
}

#[test]
fn use_relative_references_toggles_how_the_next_recording_starts() {
    let mut h = harness("Automate");
    assert!(h.state().macros.relative, "relative is the default");
    h.get_by_label("Use Relative References").click();
    h.run_steps(2);
    assert!(!h.state().macros.relative);
    h.get_by_label("Record Macro").click();
    h.run_steps(5);
    h.get_by_label("   OK   ").click();
    h.run_steps(2);
    assert!(h.state().session.macro_recording.as_ref().is_some_and(|r| !r.relative));
}

#[test]
fn the_macros_dialog_runs_the_selected_macro() {
    let mut h = harness("Automate");
    save(
        &mut h,
        json!({"name": "Fill", "output": {"onConflict": "overwrite"}, "steps": [{"command": "cell.set", "params": {"cell": "C3", "input": "done"}}]}),
    );
    save(&mut h, json!({"name": "Other", "steps": []}));
    h.get_by_label("Macros…").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("macros"));
    h.get_by_label("Fill").click();
    h.run_steps(2);
    h.get_by_label("Run").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!(value(&h, "C3"), Value::Text("done".into()));
    // One undo step undoes the whole run.
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(value(&h, "C3"), Value::Empty);
}

#[test]
fn options_set_a_shortcut_that_runs_the_macro() {
    let mut h = harness("Home");
    save(
        &mut h,
        json!({"name": "Stamp", "output": {"onConflict": "overwrite"}, "steps": [{"command": "cell.set", "params": {"cell": "B2", "input": "stamped"}}]}),
    );
    assert!(gridcraft_ui_egui::macro_ui::save_options(h.state_mut(), "Stamp", Some('k'), "Stamps B2"));
    let m = h.state().session.active().unwrap().wb.macros[0].clone();
    assert_eq!((m.shortcut.as_deref(), m.description.as_str()), (Some("Ctrl+Shift+K"), "Stamps B2"));
    let mods = Modifiers::COMMAND | Modifiers::SHIFT;
    h.input_mut().events.push(Event::Key { key: Key::K, physical_key: None, pressed: true, repeat: false, modifiers: mods });
    h.input_mut().events.push(Event::Key { key: Key::K, physical_key: None, pressed: false, repeat: false, modifiers: mods });
    h.run_steps(3);
    assert_eq!(value(&h, "B2"), Value::Text("stamped".into()));
}

#[test]
fn a_macro_whose_output_lands_on_data_asks_and_can_shift_it_down() {
    let mut h = harness("Automate");
    h.state_mut().run("cell.set", json!({"cell": "B1", "input": "old"})).unwrap();
    save(&mut h, json!({"name": "Put", "steps": [{"command": "cell.set", "params": {"cell": "B1", "input": "new"}}]}));
    gridcraft_ui_egui::macro_ui::run_macro(h.state_mut(), "Put");
    h.run_steps(2);
    let q = h.state().confirm.clone().expect("a question");
    assert_eq!(q.choices.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["Overwrite", "Shift Down", "Shift Right"]);
    assert!(q.text.contains("B1"), "{}", q.text);
    assert!(!q.text.contains("onConflict"), "the hint for programs is not shown: {}", q.text);
    h.get_by_label("Shift Down").click();
    h.run_steps(3);
    assert!(h.state().confirm.is_none());
    assert_eq!(value(&h, "B1"), Value::Text("new".into()));
    assert_eq!(value(&h, "B2"), Value::Text("old".into()));
    assert_eq!(h.state().macros.recent, ["Put"]);
}

#[test]
fn the_record_macro_dialog_names_the_macro_and_checks_name_and_shortcut() {
    let mut h = harness("Automate");
    save(&mut h, json!({"name": "Taken", "shortcut": "Ctrl+Shift+K", "steps": []}));
    h.get_by_label("Record Macro").click();
    h.run_steps(2);
    let set = |h: &mut Harness, k: &str, v: &str| {
        h.state_mut().dialog.as_mut().unwrap().values.insert(k.into(), json!(v));
        h.run_steps(2);
    };
    assert_eq!(h.state().dialog.as_ref().unwrap().values["name"], json!("Macro1"), "the next free name is suggested");
    set(&mut h, "name", "taken");
    assert!(h.query_by_label_contains("already exists").is_some());
    set(&mut h, "name", "Two\nlines");
    assert!(h.query_by_label_contains("line breaks").is_some());
    set(&mut h, "name", "Tidy Up");
    set(&mut h, "shortcut", "K");
    assert!(h.query_by_label_contains("already runs macro “Taken”").is_some());
    set(&mut h, "shortcut", "S");
    assert!(h.query_by_label_contains("Save As").is_some());
    // OK stays off while something is wrong.
    h.get_by_label("   OK   ").click();
    h.run_steps(2);
    assert!(!h.state().session.is_recording());
    set(&mut h, "shortcut", "J");
    set(&mut h, "description", "Tidies the block");
    assert!(h.query_by_label_contains("Choose another").is_none());
    h.get_by_label("   OK   ").click();
    h.run_steps(2);
    assert!(h.state().session.is_recording());
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "7"})).unwrap();
    h.get_by_label("Stop").click();
    h.run_steps(2);
    let m = h.state().session.active().unwrap().wb.macros.iter().find(|m| m.name == "Tidy Up").cloned().expect("saved under its name");
    assert_eq!(m.shortcut.as_deref(), Some("Ctrl+Shift+J"));
    assert_eq!(m.description, "Tidies the block");
}
