//! Python cells that give pictures, on the fake worker: what a cell keeps, its return type, and what
//! commands give agents. `apps/gridcraft-cli/tests/it/python_cells.rs` runs real matplotlib.

use serde_json::json;

use gridcraft_core::{CellError, Value};

use super::python_run::{c, picture_bytes, py, runs, session, settle};
use crate::Session;

fn value(s: &Session, cell: &str) -> Value {
    s.doc().unwrap().wb.active().unwrap().value(c(cell))
}

fn python(s: &Session, cell: &str) -> gridcraft_model::PyCode {
    s.doc().unwrap().wb.active().unwrap().cell(c(cell)).unwrap().py.as_deref().unwrap().clone()
}

fn in_cell(s: &Session, cell: &str) -> bool {
    s.doc().unwrap().wb.active().unwrap().picture_at(c(cell)).is_some()
}

#[test]
fn a_picture_with_no_choice_is_a_python_object_named_image() {
    let (mut s, _h) = session();
    py(&mut s, "A1", "picture 40 30");
    settle(&mut s);
    assert_eq!(value(&s, "A1"), Value::text("Image"));
    let py1 = python(&s, "A1");
    assert!(py1.shows_object() && !py1.object, "in effect, not chosen");
    assert!(!in_cell(&s, "A1"), "a plot isn't drawn inside a small cell");
    let image = py1.image().unwrap();
    assert_eq!((image.width, image.height, image.density), (40, 30, 1));
    assert_eq!(image.picture.mime, "image/png");
    assert!(py1.to_formula().ends_with(",1)"), "the file says Python Object: {}", py1.to_formula());
    assert_eq!(s.execute("cell.get", json!({"cell": "A1"})).unwrap()["python"]["output"], "object");
}

#[test]
fn a_cell_with_no_choice_goes_back_to_spilling_when_it_stops_giving_a_picture() {
    let (mut s, _h) = session();
    py(&mut s, "A1", "picture 40 30");
    settle(&mut s);
    py(&mut s, "A1", "spill 2 1");
    settle(&mut s);
    assert!(!python(&s, "A1").shows_object());
    assert_eq!(value(&s, "A2"), Value::Number(20.0));
}

#[test]
fn choosing_excel_value_draws_it_in_the_cell_without_running_and_undo_takes_it_back() {
    let (mut s, h) = session();
    py(&mut s, "A1", "picture 40 30");
    settle(&mut s);
    let ran = runs(&h).len();
    let r = s.execute("python.output", json!({"cell": "A1", "mode": "value"})).unwrap();
    assert_eq!(r["output"], "value");
    settle(&mut s);
    assert_eq!(runs(&h).len(), ran, "the picture is already there");
    assert!(in_cell(&s, "A1"));
    assert!(python(&s, "A1").to_formula().ends_with(",0)"));
    s.execute("edit.undo", json!({})).unwrap();
    settle(&mut s);
    assert!(!in_cell(&s, "A1"));
    assert!(python(&s, "A1").shows_object());
    s.execute("edit.redo", json!({})).unwrap();
    settle(&mut s);
    assert!(in_cell(&s, "A1"));
}

#[test]
fn the_shortcut_with_no_mode_turns_a_picture_into_an_excel_value_first() {
    let (mut s, _h) = session();
    py(&mut s, "A1", "picture 40 30");
    settle(&mut s);
    assert_eq!(s.execute("python.output", json!({"cell": "A1"})).unwrap()["output"], "value");
    settle(&mut s);
    assert!(in_cell(&s, "A1"));
    assert_eq!(s.execute("python.output", json!({"cell": "A1"})).unwrap()["output"], "object");
    settle(&mut s);
    assert!(!in_cell(&s, "A1"));
    assert_eq!(value(&s, "A1"), Value::text("Image"));
    assert!(python(&s, "A1").object);
}

#[test]
fn excel_value_chosen_first_draws_in_the_cell_when_it_arrives() {
    let (mut s, _h) = session();
    s.execute("python.set", json!({"cell": "A1", "code": "picture 40 30", "output": "value"})).unwrap();
    settle(&mut s);
    assert!(in_cell(&s, "A1"));
    assert_eq!(value(&s, "A1"), Value::text("Image"), "what a formula that reads it sees");
}

#[test]
fn a_new_result_replaces_the_picture_and_a_number_takes_it_away() {
    let (mut s, _h) = session();
    s.execute("python.set", json!({"cell": "A1", "code": "picture 40 30", "output": "value"})).unwrap();
    settle(&mut s);
    py(&mut s, "A1", "picture 8 6");
    settle(&mut s);
    assert_eq!(python(&s, "A1").image().unwrap().width, 8);
    assert!(in_cell(&s, "A1"));
    py(&mut s, "A1", "ret 5");
    settle(&mut s);
    assert!(!in_cell(&s, "A1"));
    assert_eq!(value(&s, "A1"), Value::Number(5.0));
}

#[test]
fn a_failure_leaves_no_picture_in_the_cell() {
    let (mut s, _h) = session();
    s.execute("python.set", json!({"cell": "A1", "code": "picture 40 30", "output": "value"})).unwrap();
    settle(&mut s);
    py(&mut s, "A1", "fail");
    settle(&mut s);
    assert!(!in_cell(&s, "A1"));
    assert_eq!(value(&s, "A1"), Value::Error(CellError::Python));
}

#[test]
fn convert_to_normal_cell_keeps_the_picture_it_shows() {
    let (mut s, _h) = session();
    s.execute("python.set", json!({"cell": "A1", "code": "picture 40 30", "output": "value"})).unwrap();
    settle(&mut s);
    s.execute("python.clear", json!({"cell": "A1"})).unwrap();
    let wb = &s.doc().unwrap().wb;
    let sh = wb.active().unwrap();
    assert!(sh.cell(c("A1")).unwrap().py.is_none());
    assert!(sh.cell_pictures.contains_key(&c("A1")));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(python(&s, "A1").image().is_some());
    assert!(!s.doc().unwrap().wb.active().unwrap().cell_pictures.contains_key(&c("A1")));
}

#[test]
fn python_image_and_save_image_give_the_bytes() {
    let (mut s, _h) = session();
    py(&mut s, "A1", "picture 40 30 2");
    py(&mut s, "A2", "ret 1");
    settle(&mut s);
    let r = s.execute("python.image", json!({"cell": "A1"})).unwrap();
    assert_eq!((&r["cell"], &r["mime"], &r["width"], &r["height"], &r["density"]), (&json!("A1"), &json!("image/png"), &json!(20), &json!(15), &json!(2)));
    let bytes = crate::io::base64_decode(r["base64"].as_str().unwrap()).unwrap();
    assert_eq!(bytes, picture_bytes(40, 30));
    assert!(s.execute("python.image", json!({"cell": "A2"})).unwrap_err().to_string().contains("no picture"));
    assert!(s.execute("python.image", json!({"cell": "C9"})).unwrap_err().to_string().contains("isn't a Python cell"));
    assert!(s.execute("python.saveImage", json!({"cell": "A1"})).is_err(), "a path is needed, and no dialog opens");
    let dir = std::env::temp_dir().join(format!("gridcraft-py-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plot");
    let saved = s.execute("python.saveImage", json!({"cell": "A1", "path": path.to_str().unwrap()})).unwrap();
    let target = saved["path"].as_str().unwrap().to_string();
    assert!(target.ends_with("plot.png"));
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
    let named = dir.join("named.PNG");
    let saved = s.execute("python.saveImage", json!({"cell": "A1", "path": named.to_str().unwrap()})).unwrap();
    assert!(saved["path"].as_str().unwrap().ends_with("named.PNG"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_workbook_as_json_keeps_the_picture() {
    let (mut s, _h) = session();
    py(&mut s, "A1", "picture 40 30 2");
    settle(&mut s);
    let saved = s.execute("file.saveBytes", json!({"format": "json"})).unwrap();
    let bytes = crate::io::base64_decode(saved["base64"].as_str().unwrap()).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let text = doc.to_string();
    assert!(text.contains("\"density\":2"), "{}", &text[..text.len().min(400)]);
    let back: gridcraft_model::Workbook = serde_json::from_slice(&bytes).unwrap();
    let py1 = back.sheets[0].cell(c("A1")).unwrap().py.as_deref().unwrap().clone();
    assert_eq!(py1.image().unwrap().picture.data, picture_bytes(40, 30));
}

#[test]
fn a_cell_that_changes_an_earlier_figure_runs_again_from_that_cell() {
    let (mut s, h) = session();
    set(&mut s, "C1", "1");
    py(&mut s, "A1", "picture 40 30");
    py(&mut s, "A2", "ret 2");
    // A3 reads C1, and says it changed the figure of A1 (sheet 0, row 0, column 0).
    py(&mut s, "A3", "touch 0 0 0 xl(\"C1\")");
    settle(&mut s);
    let before = runs(&h).len();
    set(&mut s, "C1", "2");
    settle(&mut s);
    assert_eq!(
        runs(&h).split_off(before),
        ["picture 40 30", "ret 2", "touch 0 0 0 xl(\"C1\")"],
        "from the figure's cell, not from the one whose input changed"
    );
    // A cell after them all is no reason to go back.
    py(&mut s, "A4", "ret 4");
    settle(&mut s);
    let before = runs(&h).len();
    py(&mut s, "A4", "ret 5");
    settle(&mut s);
    assert_eq!(runs(&h).split_off(before), ["ret 5"]);
}

fn set(s: &mut Session, cell: &str, input: &str) {
    s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
}
