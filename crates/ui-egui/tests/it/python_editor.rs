//! The Python editor window: its routes in and out, editing and applying the code through
//! `python.set`, the questions before unsaved code would be lost, Apply refused for a cell that
//! changed, closing and quitting refused while there is unsaved code, and code hidden on a
//! protected sheet kept out of the window.

use egui::{Event, Key, Modifiers, Pos2, Rect, vec2};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::python_window::{self, APPLY, CANCEL, CLOSE, DISCARD, EDIT_IN_EDITOR, Pending, PyWindow};
use serde_json::json;

use crate::common::Harness;

fn blank() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

/// B2 holds two lines of code.
fn with_code() -> Session {
    let mut s = blank();
    s.execute("python.set", json!({"cell": "B2", "code": "x = 1\nx + 1"})).unwrap();
    s
}

fn harness(session: Session) -> Harness {
    sized(session, 1200.0, 800.0)
}

fn sized(session: Session, w: f32, h: f32) -> Harness {
    crate::common::harness_with(crate::common::builder().with_size(vec2(w, h)), crate::common::app(session), 4)
}

fn press(key: Key, modifiers: Modifiers) -> [Event; 2] {
    [
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
    ]
}

fn send(h: &mut Harness, events: impl IntoIterator<Item = Event>) {
    h.input_mut().events.extend(events);
    h.run_steps(3);
}

fn key(h: &mut Harness, key: Key, modifiers: Modifiers) {
    send(h, press(key, modifiers));
}

fn type_text(h: &mut Harness, text: &str) {
    send(h, [Event::Text(text.into())]);
}

fn win(h: &Harness) -> Option<PyWindow> {
    h.state().py_window.clone()
}

fn text(h: &Harness) -> String {
    win(h).map(|w| w.text).unwrap_or_default()
}

fn code(h: &Harness, cell: &str) -> Option<String> {
    let d = h.state().session.active()?;
    d.wb.active()?.cell(CellRef::parse(cell)?)?.py.as_ref().map(|p| p.code.clone())
}

fn ui(h: &mut Harness, p: serde_json::Value) -> Result<serde_json::Value, String> {
    let r = h.state_mut().run("ui.pythonEditor", p);
    h.run_steps(3);
    r
}

/// Puts the window's caret at the end of the code.
fn to_end(h: &mut Harness) {
    if let Some(w) = h.state_mut().py_window.as_mut() {
        w.caret = Some(w.text.chars().count());
    }
    h.run_steps(2);
}

fn select(h: &mut Harness, cell: &str) {
    h.state_mut().session.execute("selection.set", json!({"cell": cell})).unwrap();
    h.run_steps(2);
}

/// Clicks the widget labelled `label` inside `area`.
fn click_in(h: &mut Harness, area: Rect, label: &str) {
    let node = h.query_all_by_label(label).find(|n| area.contains(n.rect().center()));
    let Some(node) = node else { panic!("no {label:?} in {area:?}") };
    node.click();
    h.run_steps(3);
}

/// Clicks a button of the editor window.
fn click(h: &mut Harness, label: &str) {
    let area = win(h).and_then(|w| w.rect).expect("the window is on screen");
    click_in(h, area, label);
}

#[test]
fn the_window_opens_and_closes_by_each_route() {
    let mut h = harness(with_code());
    // The control channel, without a dialog.
    let r = ui(&mut h, json!({"on": true, "cell": "B2"})).unwrap();
    assert_eq!((r["open"].as_bool(), r["cell"].as_str()), (Some(true), Some("B2")));
    assert_eq!(text(&h), "x = 1\nx + 1");
    assert!(h.state().dialog.is_none() && h.state().message.is_none());
    assert!(h.query_all_by_label("Python Editor").count() >= 1, "the window shows");
    h.check_accessibility();
    assert_eq!(ui(&mut h, json!({"on": false})).unwrap()["open"], false);
    assert!(win(&h).is_none());
    // The formula bar's button, on a selected Python cell; Esc closes the window.
    select(&mut h, "B2");
    h.get_by_label(EDIT_IN_EDITOR).click();
    h.run_steps(3);
    assert_eq!(win(&h).map(|w| w.cell), Some(CellRef::new(1, 1)));
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(win(&h).is_none(), "Esc closes an editor without changes");
    // The cell menu's entry; the Close button.
    h.state_mut().grid.context_menu = Some(Pos2::new(300.0, 300.0));
    h.run_steps(3);
    let menu = h.ctx.memory(|m| m.area_rect(egui::Id::unique("cell_context"))).expect("the cell menu");
    click_in(&mut h, menu, EDIT_IN_EDITOR);
    assert_eq!(win(&h).map(|w| w.cell), Some(CellRef::new(1, 1)));
    assert!(h.state().grid.context_menu.is_none(), "the menu closed");
    click(&mut h, CLOSE);
    assert!(win(&h).is_none());
    // No button and no menu entry on an ordinary cell.
    select(&mut h, "D4");
    assert!(h.query_by_label(EDIT_IN_EDITOR).is_none());
    // Toggling from the control channel: without `on` it opens, then closes.
    assert_eq!(ui(&mut h, json!({})).unwrap()["open"], true);
    assert_eq!(win(&h).map(|w| w.cell), Some(CellRef::new(3, 3)), "on the active cell");
    assert_eq!(ui(&mut h, json!({})).unwrap()["open"], false);
}

#[test]
fn enter_tab_and_ctrl_enter_or_apply_commit_through_python_set() {
    let mut h = harness(blank());
    select(&mut h, "C3");
    ui(&mut h, json!({"on": true})).unwrap();
    type_text(&mut h, "for i in range(3):");
    key(&mut h, Key::Enter, Modifiers::NONE);
    type_text(&mut h, "print(i)");
    key(&mut h, Key::Enter, Modifiers::NONE);
    key(&mut h, Key::Tab, Modifiers::SHIFT);
    type_text(&mut h, "i");
    assert_eq!(text(&h), "for i in range(3):\n    print(i)\ni", "Enter keeps the indentation, Shift+Tab takes it back");
    key(&mut h, Key::Enter, Modifiers::COMMAND);
    assert_eq!(code(&h, "C3").as_deref(), Some("for i in range(3):\n    print(i)\ni"), "Ctrl+Enter applied it");
    let w = win(&h).expect("the window stays open");
    assert!(!w.dirty());
    // Apply, after more typing.
    type_text(&mut h, " * 2");
    assert!(win(&h).is_some_and(|w| w.dirty()));
    assert!(h.query_by_label("Unsaved changes").is_some());
    click(&mut h, APPLY);
    assert_eq!(code(&h, "C3").as_deref(), Some("for i in range(3):\n    print(i)\ni * 2"));
    assert!(win(&h).is_some_and(|w| !w.dirty()));
    // An undo changes the cell: the window, with no changes of its own, shows it.
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(text(&h), "for i in range(3):\n    print(i)\ni");
    // Each apply is one undo step, through python.set.
    let journal: Vec<String> = h.state().session.journal.iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(journal.iter().filter(|id| *id == "python.set").count(), 2, "{journal:?}");
}

#[test]
fn esc_with_changes_asks_first() {
    let mut h = harness(with_code());
    ui(&mut h, json!({"on": true, "cell": "B2"})).unwrap();
    to_end(&mut h);
    type_text(&mut h, "\ny = 2");
    key(&mut h, Key::Escape, Modifiers::NONE);
    let w = win(&h).expect("still open");
    assert_eq!(w.pending, Some(Pending::Close), "it asks");
    assert!(h.query_by_label("The code for Sheet1!B2 has unsaved changes.").is_some());
    // Typing goes nowhere while it asks.
    type_text(&mut h, "zzz");
    assert_eq!(text(&h), "x = 1\nx + 1\ny = 2");
    assert_eq!(code(&h, "D1"), None);
    // Cancel (and Esc) go back to the code.
    click(&mut h, CANCEL);
    assert_eq!(win(&h).and_then(|w| w.pending), None);
    type_text(&mut h, "3");
    assert_eq!(text(&h), "x = 1\nx + 1\ny = 23", "the keyboard is back in the code");
    click(&mut h, CLOSE);
    assert_eq!(win(&h).and_then(|w| w.pending), Some(Pending::Close), "Close asks too");
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(win(&h).and_then(|w| w.pending), None, "Esc answers Cancel");
    // Discard closes and changes nothing.
    key(&mut h, Key::Escape, Modifiers::NONE);
    click(&mut h, DISCARD);
    assert!(win(&h).is_none());
    assert_eq!(code(&h, "B2").as_deref(), Some("x = 1\nx + 1"));
    // Apply puts the code in, then closes.
    ui(&mut h, json!({"on": true, "cell": "B2"})).unwrap();
    type_text(&mut h, "# top\n");
    key(&mut h, Key::Escape, Modifiers::NONE);
    click(&mut h, APPLY);
    assert!(win(&h).is_none());
    assert_eq!(code(&h, "B2").as_deref(), Some("# top\nx = 1\nx + 1"));
}

#[test]
fn another_cell_with_unsaved_changes_asks_first() {
    let mut h = harness(with_code());
    ui(&mut h, json!({"on": true, "cell": "A1"})).unwrap();
    type_text(&mut h, "a = 1");
    // The cell menu on B2 asks; Cancel keeps A1's code.
    select(&mut h, "B2");
    python_window::open_from_menu(h.state_mut());
    h.run_steps(3);
    let w = win(&h).unwrap();
    assert_eq!((w.cell, w.text.as_str()), (CellRef::new(0, 0), "a = 1"));
    assert!(matches!(w.pending, Some(Pending::Open { .. })), "it asks");
    click(&mut h, CANCEL);
    assert_eq!(win(&h).map(|w| (w.cell, w.text)), Some((CellRef::new(0, 0), "a = 1".to_string())));
    // Discard opens B2 and leaves A1 as it was.
    python_window::open_from_menu(h.state_mut());
    h.run_steps(3);
    click(&mut h, DISCARD);
    assert_eq!(win(&h).map(|w| (w.cell, w.text)), Some((CellRef::new(1, 1), "x = 1\nx + 1".to_string())));
    assert_eq!(code(&h, "A1"), None);
    // Apply puts B2's changes in, then opens on the formula bar's cell with its code.
    type_text(&mut h, "# b2\n");
    select(&mut h, "C3");
    h.state_mut().toggle_python_mode();
    h.run_steps(3);
    type_text(&mut h, "c = 3");
    h.get_by_label(EDIT_IN_EDITOR).click();
    h.run_steps(3);
    assert!(h.state().editor.is_some(), "the formula bar keeps its edit while the window asks");
    click(&mut h, APPLY);
    assert_eq!(code(&h, "B2").as_deref(), Some("# b2\nx = 1\nx + 1"));
    assert_eq!(win(&h).map(|w| (w.cell, w.text)), Some((CellRef::new(2, 2), "c = 3".to_string())));
    assert!(h.state().editor.is_none(), "and its edit moved into the window");
    // The control channel never asks: it refuses, unless forced.
    let e = ui(&mut h, json!({"on": true, "cell": "D4"})).unwrap_err();
    assert!(e.contains("unsaved changes to Sheet1!C3"), "{e}");
    assert!(win(&h).is_some_and(|w| w.cell == CellRef::new(2, 2) && w.pending.is_none()));
    assert!(h.state().dialog.is_none() && h.state().message.is_none());
    assert_eq!(ui(&mut h, json!({"on": true, "cell": "Sheet1!D4", "force": true})).unwrap()["cell"], "D4");
    assert_eq!(code(&h, "C3"), None, "force discarded the code");
    // The same cell again keeps the window's code.
    type_text(&mut h, "d");
    ui(&mut h, json!({"on": true, "cell": "D4"})).unwrap();
    assert_eq!(text(&h), "d");
}

fn control(h: &mut Harness, method: &str, p: serde_json::Value) -> serde_json::Value {
    let ctx = h.ctx.clone();
    match gridcraft_ui_egui::control::handle(h.state_mut(), &ctx, method, &p) {
        gridcraft_ui_egui::control::Outcome::Done(v) => v,
        gridcraft_ui_egui::control::Outcome::Screenshot { .. } => json!(null),
    }
}

/// What `cell` holds on the active sheet, as the formula bar shows it.
fn input(h: &Harness, cell: &str) -> String {
    let d = h.state().session.active().unwrap();
    d.wb.active().unwrap().input_text(CellRef::parse(cell).unwrap())
}

/// The refusal shown when the window's cell changed under it.
fn stale_message(cell: &str) -> String {
    python_window::STALE.replace("{cell}", cell)
}

fn notice(h: &Harness) -> Option<String> {
    python_window::state(h.state())["notice"].as_str().map(str::to_string)
}

#[test]
fn a_row_inserted_above_the_cell_refuses_apply_rather_than_overwrite_what_moved_in() {
    let mut s = with_code();
    s.execute("cell.set", json!({"cell": "B1", "input": "keep"})).unwrap();
    let mut h = harness(s);
    ui(&mut h, json!({"on": true, "cell": "B2"})).unwrap();
    to_end(&mut h);
    type_text(&mut h, " + 2");
    // B2's code moves to B3, and B1's text to B2.
    h.state_mut().run("home.insertRows", json!({"rows": "1:1"})).unwrap();
    h.run_steps(3);
    assert_eq!((input(&h, "B2"), code(&h, "B3").as_deref()), ("keep".to_string(), Some("x = 1\nx + 1")));
    key(&mut h, Key::Enter, Modifiers::COMMAND);
    assert_eq!(input(&h, "B2"), "keep", "the text that moved to B2 is not replaced");
    assert_eq!(code(&h, "B2"), None);
    assert_eq!(code(&h, "B3").as_deref(), Some("x = 1\nx + 1"), "nor the code that moved away");
    let w = win(&h).expect("the window stays open");
    assert_eq!(w.text, "x = 1\nx + 1 + 2", "the code is kept");
    let message = stale_message("Sheet1!B2");
    assert_eq!(notice(&h).as_deref(), Some(message.as_str()));
    assert!(h.query_by_label(&message).is_some(), "shown in the window");
    assert!(h.state().message.is_none(), "not in a message box over it");
    // Apply from the question about unsaved changes refuses the same way.
    key(&mut h, Key::Escape, Modifiers::NONE);
    click(&mut h, APPLY);
    assert!(win(&h).is_some_and(|w| w.text == "x = 1\nx + 1 + 2" && w.pending.is_none()));
    assert_eq!(input(&h, "B2"), "keep");
    // The control channel doesn't take the address for the same cell: it refuses to replace the
    // unsaved code, and with `force` opens afresh on what B2 holds now.
    let e = ui(&mut h, json!({"on": true, "cell": "B2"})).unwrap_err();
    assert!(e.contains("unsaved changes"), "{e}");
    assert!(win(&h).is_some_and(|w| w.text == "x = 1\nx + 1 + 2"));
    ui(&mut h, json!({"on": true, "cell": "B2", "force": true})).unwrap();
    type_text(&mut h, "y = 3");
    key(&mut h, Key::Enter, Modifiers::COMMAND);
    assert_eq!(code(&h, "B2").as_deref(), Some("y = 3"), "opened again on the cell, Apply works");
}

#[test]
fn closing_the_workbook_or_quitting_with_unsaved_code_is_refused() {
    let mut h = harness(blank());
    ui(&mut h, json!({"on": true})).unwrap();
    type_text(&mut h, "a = 1");
    // File › Close is refused, programmatically and from the interface, which shows why.
    assert!(h.state_mut().run("file.close", json!({})).unwrap_err().contains("Python editor"));
    h.state_mut().run_or_alert("file.close", json!({}));
    h.run_steps(3);
    assert!(h.state().message.as_ref().is_some_and(|(_, m)| m.contains("Python editor")));
    assert_eq!(h.state().session.documents().len(), 1, "the workbook stays open");
    // The app window's close button and Quit stop, with a message.
    h.state_mut().message = None;
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    h.run_steps(2);
    assert!(h.state().message.as_ref().is_some_and(|(_, m)| m.contains("unsaved changes to Sheet1!A1")));
    h.state_mut().message = None;
    let ctx = h.ctx.clone();
    h.state_mut().request_quit(&ctx);
    assert!(h.state().message.is_some() && !h.state().allow_close);
    h.state_mut().message = None;
    // app.quit over the control channel is refused too.
    let r = control(&mut h, "app.quit", json!({}));
    assert!(r["error"].as_str().unwrap_or_default().contains("Python editor has unsaved changes to Sheet1!A1"), "{r}");
    assert!(!h.state().allow_close);
    assert!(win(&h).is_some_and(|w| w.text == "a = 1" && w.pending.is_none()), "the code stays");
    // Once the code is discarded, the quit goes through.
    ui(&mut h, json!({"on": false, "force": true})).unwrap();
    h.state_mut().request_quit(&ctx);
    assert!(h.state().allow_close);
}

#[test]
fn apply_is_refused_once_the_sheet_moved_or_was_renamed() {
    let mut s = blank();
    s.execute("home.insertSheet", json!({"name": "Data"})).unwrap();
    s.execute("python.set", json!({"cell": "Data!B2", "code": "d = 1"})).unwrap();
    let mut h = harness(s);
    let code_on = |h: &Harness, sheet: &str| {
        let d = h.state().session.active().unwrap();
        let sh = &d.wb.sheets[d.wb.sheet_index(sheet).unwrap()];
        sh.cell(CellRef::new(1, 1)).and_then(|c| c.py.as_ref().map(|p| p.code.clone()))
    };
    ui(&mut h, json!({"on": true, "cell": "Data!B2"})).unwrap();
    to_end(&mut h);
    type_text(&mut h, " + 1");
    // The sheet moves, and its cell changes elsewhere: Apply writes nothing.
    let at = h.state().session.active().and_then(|d| d.wb.sheet_index("Data")).unwrap();
    h.state_mut().run("sheet.move", json!({"sheet": "Data", "to": 1 - at})).unwrap();
    h.state_mut().run("python.set", json!({"cell": "Data!B2", "code": "z = 9"})).unwrap();
    h.run_steps(3);
    key(&mut h, Key::Enter, Modifiers::COMMAND);
    assert_eq!(code_on(&h, "Data").as_deref(), Some("z = 9"), "the cell's new code is not replaced");
    assert_eq!(notice(&h), Some(stale_message("Data!B2")));
    assert_eq!(text(&h), "d = 1 + 1", "the code is kept");
    // Renamed: refused too. Without unsaved code, the window closes with the old name.
    h.state_mut().run("sheet.rename", json!({"sheet": "Data", "name": "Inputs"})).unwrap();
    h.run_steps(3);
    click(&mut h, APPLY);
    assert_eq!(code_on(&h, "Inputs").as_deref(), Some("z = 9"));
    ui(&mut h, json!({"on": true, "cell": "Inputs!B2", "force": true})).unwrap();
    h.state_mut().run("sheet.rename", json!({"sheet": "Inputs", "name": "Data"})).unwrap();
    h.run_steps(3);
    assert!(win(&h).is_none());
}

#[test]
fn a_workbook_closed_some_other_way_leaves_the_typed_code_on_the_clipboard() {
    let mut h = harness(blank());
    h.state_mut().run("file.new", json!({})).unwrap();
    ui(&mut h, json!({"on": true})).unwrap();
    type_text(&mut h, "a = 1");
    // As when opening a file replaces an untouched Book1: no command asks first.
    let i = h.state().session.active_index();
    h.state_mut().session.close_document(i);
    h.step();
    let copied = h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "a = 1"));
    h.run_steps(2);
    assert!(copied, "the typed code is on the clipboard");
    assert!(win(&h).is_none());
    assert_eq!(h.state().toast.as_ref().map(|t| t.0.as_str()), Some(python_window::COPIED));
}

/// B2 holds code, formatted Hidden (and unlocked); the sheet is protected when `protected`.
fn hidden_code(protected: bool) -> Harness {
    let mut s = with_code();
    s.execute("home.formatCells", json!({"range": "B2", "style": {"protection": {"locked": false, "hidden": true}}})).unwrap();
    if protected {
        s.execute("review.protectSheet", json!({})).unwrap();
    }
    s.execute("selection.set", json!({"cell": "B2"})).unwrap();
    harness(s)
}

#[test]
fn the_control_channel_refuses_hidden_code_on_a_protected_sheet() {
    let mut h = hidden_code(true);
    let e = ui(&mut h, json!({"on": true, "cell": "B2"})).unwrap_err();
    assert!(e.contains("B2 are hidden because the sheet is protected"), "{e}");
    assert!(win(&h).is_none());
    // Without `cell` (the active cell) and without `on` too.
    assert!(ui(&mut h, json!({})).is_err());
    assert!(win(&h).is_none());
    assert!(h.state().dialog.is_none() && h.state().message.is_none(), "no dialog");
}

#[test]
fn the_formula_bar_offers_no_editor_button_on_hidden_code() {
    let mut h = hidden_code(true);
    assert!(h.query_by_label(EDIT_IN_EDITOR).is_none(), "no button");
    python_window::open_from_bar(h.state_mut());
    h.run_steps(3);
    assert!(win(&h).is_none(), "the route opens nothing");
    // Without protection the button is there.
    let h = hidden_code(false);
    assert!(h.query_by_label(EDIT_IN_EDITOR).is_some());
}

#[test]
fn the_cell_menu_offers_no_editor_entry_on_hidden_code() {
    let mut h = hidden_code(true);
    h.state_mut().grid.context_menu = Some(Pos2::new(300.0, 300.0));
    h.run_steps(3);
    let menu = h.ctx.memory(|m| m.area_rect(egui::Id::unique("cell_context"))).expect("the cell menu");
    assert!(h.query_all_by_label(EDIT_IN_EDITOR).all(|n| !menu.contains(n.rect().center())), "no entry in the menu");
    python_window::open_from_menu(h.state_mut());
    h.run_steps(3);
    assert!(win(&h).is_none(), "the route opens nothing");
}

#[test]
fn protecting_the_sheet_closes_the_window_on_hidden_code_and_keeps_typed_code_on_the_clipboard() {
    // Without changes, the window just closes.
    let mut h = hidden_code(false);
    ui(&mut h, json!({"on": true, "cell": "B2"})).unwrap();
    assert_eq!(text(&h), "x = 1\nx + 1");
    h.state_mut().session.execute("review.protectSheet", json!({})).unwrap();
    h.run_steps(3);
    assert!(win(&h).is_none(), "the code is out of view");
    assert!(h.query_by_label("x = 1\nx + 1").is_none());
    assert!(h.state().toast.is_none(), "nothing typed, nothing copied");
    // With typed code, it closes too, and the typed code goes to the clipboard.
    let mut h = hidden_code(false);
    ui(&mut h, json!({"on": true, "cell": "B2"})).unwrap();
    to_end(&mut h);
    type_text(&mut h, " # typed");
    assert!(win(&h).is_some_and(|w| w.dirty()));
    h.state_mut().session.execute("review.protectSheet", json!({})).unwrap();
    h.step();
    let copied = h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "x = 1\nx + 1 # typed"));
    h.run_steps(2);
    assert!(win(&h).is_none(), "the code is out of view");
    assert!(copied, "the typed code is on the clipboard");
    assert_eq!(h.state().toast.as_ref().map(|t| t.0.as_str()), Some(python_window::COPIED));
    assert_eq!(code(&h, "B2").as_deref(), Some("x = 1\nx + 1"), "nothing was applied");
}

#[test]
fn a_window_kept_on_a_moved_or_renamed_sheet_closes_when_its_code_becomes_hidden() {
    for change in ["move", "rename"] {
        let mut s = blank();
        s.execute("home.insertSheet", json!({"name": "Data"})).unwrap();
        s.execute("python.set", json!({"cell": "Data!B2", "code": "d = 1"})).unwrap();
        s.execute("home.formatCells", json!({"range": "Data!B2", "style": {"protection": {"locked": false, "hidden": true}}})).unwrap();
        let mut h = harness(s);
        ui(&mut h, json!({"on": true, "cell": "Data!B2"})).unwrap();
        to_end(&mut h);
        type_text(&mut h, " + 1");
        // With unsaved code the window stays when its sheet moves or is renamed.
        if change == "move" {
            let at = h.state().session.active().and_then(|d| d.wb.sheet_index("Data")).unwrap();
            h.state_mut().run("sheet.move", json!({"sheet": "Data", "to": 1 - at})).unwrap();
        } else {
            h.state_mut().run("sheet.rename", json!({"sheet": "Data", "name": "Inputs"})).unwrap();
        }
        h.run_steps(3);
        assert!(win(&h).is_some_and(|w| w.text == "d = 1 + 1"), "{change}: the window stays");
        let active = h.state().session.active().map(|d| d.wb.active().unwrap().name.clone());
        assert_ne!(active.as_deref(), Some("Sheet1"), "{change}: the cell's sheet is active");
        h.state_mut().session.execute("review.protectSheet", json!({})).unwrap();
        h.step();
        let copied = h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "d = 1 + 1"));
        h.run_steps(2);
        assert!(win(&h).is_none(), "{change}: the code is out of view");
        assert!(copied, "{change}: the typed code is on the clipboard");
    }
}
