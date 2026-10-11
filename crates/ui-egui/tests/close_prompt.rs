//! What a window's close (or Quit) asks about: workbooks with unsaved changes and an uncommitted
//! Python edit; and which windows count as blank, so a document opened from Finder can take
//! their place.

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_python::client::Launcher;
use gridcraft_ui_egui::{Services, SheetApp};
use serde_json::json;

fn app() -> SheetApp {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    SheetApp::new(s, Services::default())
}

#[test]
fn a_new_workbook_is_blank_until_it_changes() {
    let mut a = app();
    assert!(a.is_blank());
    assert!(!a.has_unsaved());
    a.run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    assert!(!a.is_blank());
    assert!(a.has_unsaved());
    assert_eq!(a.unsaved_titles(), vec!["Book1".to_string()]);
}

#[test]
fn a_window_with_no_workbook_is_blank() {
    let a = SheetApp::new(Session::new(), Services::default());
    assert!(a.is_blank());
    assert!(!a.has_unsaved());
}

#[test]
fn a_window_being_typed_in_is_not_blank() {
    let mut a = app();
    a.begin_edit(Some("12".into()), false);
    assert!(!a.is_blank());
}

#[test]
fn an_uncommitted_python_edit_counts_as_unsaved() {
    let mut a = app();
    a.begin_python_edit();
    // Nothing typed yet: same as the (empty) cell.
    assert!(!a.pending_python_edit());
    assert!(!a.has_unsaved());
    if let Some(ed) = a.editor.as_mut() {
        ed.text = "1 + 1".into();
    }
    assert!(a.pending_python_edit());
    assert!(a.has_unsaved());
    assert_eq!(a.unsaved_titles(), vec!["Book1".to_string()]);
    // Discarding (Don't Save) lets the window close.
    a.quit_confirmed = true;
    assert!(!a.has_unsaved());
}

#[test]
fn a_python_edit_that_changes_nothing_is_not_unsaved() {
    let mut a = app();
    a.run("python.set", json!({"cell": "A1", "code": "2 * 3"})).unwrap();
    // (A save waits for running Python cells; nothing steps them here.)
    a.session.finish_python();
    a.run("file.save", json!({"path": std::env::temp_dir().join(format!("gridcraft-close-prompt-{}.xlsx", std::process::id())).to_str().unwrap()}))
        .unwrap();
    assert!(!a.has_unsaved());
    a.begin_python_edit();
    assert_eq!(a.editor.as_ref().map(|e| e.text.as_str()), Some("2 * 3"));
    assert!(!a.pending_python_edit());
    assert!(!a.has_unsaved());
}

#[test]
fn has_document_matches_the_saved_path() {
    let mut a = app();
    let path = std::env::temp_dir().join(format!("gridcraft-close-prompt-has-{}.xlsx", std::process::id()));
    let path = path.to_str().unwrap().to_string();
    a.run("file.save", json!({"path": path})).unwrap();
    assert!(a.has_document(&path));
    assert!(!a.has_document("/nowhere/else.xlsx"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn save_in_the_close_prompt_commits_the_python_edit_first() {
    let path = std::env::temp_dir().join(format!("gridcraft-close-prompt-save-{}.xlsx", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let target = path.to_str().unwrap().to_string();
    let services = Services { pick_save: Some(Box::new(move |_| Some(target.clone()))), ..Default::default() };
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(s, services),
    );
    h.run_steps(4);
    h.state_mut().begin_python_edit();
    if let Some(ed) = h.state_mut().editor.as_mut() {
        ed.text = "40 + 2".into();
    }
    h.state_mut().ask_before_closing();
    h.run_steps(3);
    assert!(h.state().asking_before_closing());
    h.get_by_label("Save").click();
    h.run_steps(3);
    let app = h.state();
    assert!(app.editor.is_none());
    assert!(app.quit_confirmed);
    assert!(!app.asking_before_closing());
    let code = app
        .session
        .active()
        .and_then(|d| d.wb.sheet(0))
        .and_then(|sh| sh.cell(CellRef::new(0, 0)))
        .and_then(|c| c.py.as_ref())
        .map(|p| p.code.clone());
    assert_eq!(code.as_deref(), Some("40 + 2"));
    assert!(path.exists());
    let _ = std::fs::remove_file(&path);
}

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// A window whose Save As picker answers `path`.
fn harness_saving_to(path: &std::path::Path) -> Harness {
    let target = path.to_str().unwrap().to_string();
    let services = Services { pick_save: Some(Box::new(move |_| Some(target.clone()))), ..Default::default() };
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(s, services),
    );
    h.run_steps(4);
    h
}

/// Frames until `done` (Python runs in the background, a step per frame).
fn run_until(h: &mut Harness, done: impl Fn(&SheetApp) -> bool) {
    for _ in 0..2000 {
        if done(h.state()) {
            return;
        }
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("never happened");
}

/// What A1 of the workbook saved at `path` holds.
fn saved_a1(path: &std::path::Path) -> serde_json::Value {
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path.to_str().unwrap()})).unwrap();
    s.execute("cell.get", json!({"cell": "A1"})).unwrap()["value"].clone()
}

#[test]
fn save_in_the_close_prompt_waits_for_the_python_cell_to_finish() {
    let path = std::env::temp_dir().join(format!("gridcraft-close-prompt-wait-{}.xlsx", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut h = harness_saving_to(&path);
    h.state_mut().begin_python_edit();
    if let Some(ed) = h.state_mut().editor.as_mut() {
        ed.text = "sum(range(10)) * 2 - 48".into();
    }
    h.state_mut().ask_before_closing();
    h.run_steps(3);
    h.get_by_label("Save").click();
    h.run_steps(1);
    // The committed cell runs before the file is written; meanwhile the window still counts as
    // asking (so quitting waits) and keeps its unsaved changes.
    assert!(h.state().save_pending() || h.state().quit_confirmed);
    assert!(h.state().asking_before_closing() || h.state().quit_confirmed);
    run_until(&mut h, |a| a.quit_confirmed);
    assert!(!h.state().save_pending());
    assert!(!h.state().asking_before_closing());
    assert_eq!(saved_a1(&path), json!(42.0));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_ordinary_save_waits_for_running_python_cells() {
    let path = std::env::temp_dir().join(format!("gridcraft-save-waits-{}.xlsx", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut h = harness_saving_to(&path);
    h.state_mut().run("python.set", json!({"cell": "A1", "code": "6 * 7"})).unwrap();
    assert!(h.state().session.python_pending_cells() > 0);
    let r = h.state_mut().run("file.save", json!({"path": path.to_str().unwrap()})).unwrap();
    assert_eq!(r["waitingForPython"], true);
    assert!(!path.exists());
    run_until(&mut h, |a| !a.save_pending());
    assert_eq!(saved_a1(&path), json!(42.0));
    assert!(!h.state().has_unsaved());
    let _ = std::fs::remove_file(&path);
}
