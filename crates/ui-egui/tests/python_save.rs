//! Saving waits for the Python cells still running, so the file holds their results rather than
//! `#BUSY!`: Save, Save As, the Save of the Save Changes? prompt, AutoSave and the control
//! channel's saves. The window keeps working meanwhile.

use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_python::client::Launcher;
use gridcraft_ui_egui::{ControlRequest, Services, SheetApp};
use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// Code for a cell that takes a moment to run (longer than a frame), then ends with `value`.
fn slow(value: &str) -> String {
    format!("n = 0\nfor i in range(2000000):\n    n += i\n{value}")
}

/// A window whose Save As picker answers `pick` (when given).
fn harness(pick: Option<PathBuf>) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    let services = match pick {
        Some(path) => {
            let target = path.to_string_lossy().into_owned();
            Services { pick_save: Some(Box::new(move |_| Some(target.clone()))), ..Default::default() }
        }
        None => Services::default(),
    };
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(s, services),
    );
    h.run_steps(4);
    h
}

/// A file name for this test in the temporary directory, removed first.
fn temp(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("gridcraft-python-save-{name}-{}.xlsx", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// Frames until `done` (Python runs in the background, a step per frame).
fn run_until(h: &mut Harness, done: impl Fn(&SheetApp) -> bool) {
    for _ in 0..3000 {
        if done(h.state()) {
            return;
        }
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the condition never held");
}

/// What `cell` of the workbook saved at `path` holds.
fn saved(path: &Path, cell: &str) -> Json {
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    s.execute("cell.get", json!({"cell": cell})).unwrap()["value"].clone()
}

fn path_param(path: &Path) -> Json {
    json!(path.to_string_lossy())
}

#[test]
fn save_waits_for_a_running_python_cell() {
    let path = temp("save");
    let mut h = harness(None);
    h.state_mut().run("python.set", json!({"cell": "A1", "code": slow("6 * 7")})).unwrap();
    assert!(h.state().session.python_pending_cells() > 0);
    let r = h.state_mut().run("file.save", json!({"path": path_param(&path)})).unwrap();
    assert_eq!(r["waitingForPython"], true);
    assert!(h.state().save_pending());
    assert!(!path.exists(), "nothing is written while the cell runs");
    run_until(&mut h, |a| !a.save_pending());
    assert_eq!(saved(&path, "A1"), json!(42.0));
    assert!(!h.state().session.active().unwrap().is_dirty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn save_as_and_save_with_a_file_wait_too() {
    let first = temp("save-as-first");
    let second = temp("save-as-second");
    let mut h = harness(Some(second.clone()));
    h.state_mut().run("file.save", json!({"path": path_param(&first)})).unwrap();
    // Save (Cmd+S) on a workbook that has a file.
    h.state_mut().run("python.set", json!({"cell": "A1", "code": slow("6 * 7")})).unwrap();
    let r = h.state_mut().run("file.save", json!({})).unwrap();
    assert_eq!(r["waitingForPython"], true);
    run_until(&mut h, |a| !a.save_pending());
    assert_eq!(saved(&first, "A1"), json!(42.0));
    // Save As: the picker answers at once and the save waits.
    h.state_mut().run("python.set", json!({"cell": "A2", "code": slow("7 * 7")})).unwrap();
    h.state_mut().run("file.saveAs", json!({})).unwrap();
    assert!(h.state().save_pending());
    assert!(!second.exists());
    run_until(&mut h, |a| !a.save_pending());
    assert_eq!(saved(&second, "A2"), json!(49.0));
    let _ = std::fs::remove_file(&first);
    let _ = std::fs::remove_file(&second);
}

#[test]
fn save_in_the_save_changes_prompt_waits_then_closes_the_workbook() {
    let path = temp("prompt");
    let mut h = harness(Some(path.clone()));
    h.state_mut().run("file.new", json!({})).unwrap();
    assert_eq!(h.state().session.documents().len(), 2);
    h.state_mut().run("cell.set", json!({"cell": "B1", "input": "1"})).unwrap();
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("saveChanges"));
    // A Python cell starts just before Save is clicked.
    h.state_mut().run("python.set", json!({"cell": "A1", "code": slow("6 * 7")})).unwrap();
    h.get_by_label("Save").click();
    h.run_steps(1);
    assert!(h.state().save_pending());
    assert!(!path.exists(), "nothing is written while the cell runs");
    assert_eq!(h.state().session.documents().len(), 2, "still open while the cell runs");
    run_until(&mut h, |a| !a.save_pending());
    h.run_steps(1);
    assert_eq!(h.state().session.documents().len(), 1, "closed once saved");
    assert_eq!(saved(&path, "A1"), json!(42.0));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn autosave_waits_for_running_python_cells() {
    let path = temp("autosave");
    let mut h = harness(None);
    h.state_mut().run("file.save", json!({"path": path_param(&path)})).unwrap();
    h.state_mut().grid.autosave = true;
    h.state_mut().grid.last_autosave = 0.0;
    h.state_mut().run("python.set", json!({"cell": "A1", "code": slow("6 * 7")})).unwrap();
    run_until(&mut h, |a| !a.session.active().unwrap().is_dirty());
    assert_eq!(saved(&path, "A1"), json!(42.0));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_control_channel_reply_waits_for_the_save() {
    let path = temp("control");
    let other = temp("control-execute");
    let mut h = harness(None);
    let (tx, rx) = std::sync::mpsc::channel();
    h.state_mut().control_rx = Some(rx);
    h.state_mut().run("python.set", json!({"cell": "A1", "code": slow("6 * 7")})).unwrap();
    let (req, reply) = ControlRequest::new("app.save", json!({"path": path_param(&path)}));
    tx.send(req).unwrap();
    h.run_steps(1);
    assert!(reply.try_recv().is_err(), "the reply waits for the save");
    let inspect = gridcraft_ui_egui::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspect["savePending"], true);
    let answer = (0..3000)
        .find_map(|_| {
            h.run_steps(1);
            std::thread::sleep(std::time::Duration::from_millis(2));
            reply.try_recv().ok()
        })
        .expect("a reply");
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(saved(&path, "A1"), json!(42.0));
    assert_eq!(gridcraft_ui_egui::control::inspect(h.state(), &h.ctx)["savePending"], false);
    // engine.execute with file.save, the same way.
    h.state_mut().run("python.set", json!({"cell": "A2", "code": slow("7 * 7")})).unwrap();
    let (req, reply) = ControlRequest::new("engine.execute", json!({"command": "file.save", "params": {"path": path_param(&other)}}));
    tx.send(req).unwrap();
    h.run_steps(1);
    assert!(reply.try_recv().is_err(), "the reply waits for the save");
    let answer = (0..3000)
        .find_map(|_| {
            h.run_steps(1);
            std::thread::sleep(std::time::Duration::from_millis(2));
            reply.try_recv().ok()
        })
        .expect("a reply");
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(saved(&other, "A2"), json!(49.0));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&other);
}
