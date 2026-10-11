//! Python cells under edits: what keeps, moves, rewrites or removes their code.

use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Cell, PyCode};
use serde_json::json;

use super::s;
use crate::Session;

/// Makes `cell` on sheet `sheet` a Python cell with `code` and a cached value.
fn plant(s: &mut Session, sheet: usize, cell: &str, code: &str) {
    let d = s.doc_mut().unwrap();
    let mut wb = (*d.wb).clone();
    let sh = wb.sheet_mut(sheet).unwrap();
    let at = CellRef::parse(cell).unwrap();
    let style = sh.style_id(at);
    sh.set_cell(at, Cell { value: Value::Number(1.0), formula: None, style, py: Some(Arc::new(PyCode::new(code))) });
    d.wb = Arc::new(wb);
}

fn code(s: &Session, sheet: usize, cell: &str) -> Option<String> {
    let d = s.doc().unwrap();
    d.wb.sheet(sheet).unwrap().cell(CellRef::parse(cell).unwrap()).and_then(|c| c.py.as_ref()).map(|p| p.code.clone())
}

#[test]
fn typing_py_formulas_points_to_python_set() {
    let mut s = s();
    let e = s.execute("cell.set", json!({"cell": "A1", "input": "=PY(\"1+1\",0)"})).unwrap_err();
    assert!(e.to_string().contains("python.set"), "{e}");
    assert!(s.execute("cell.set", json!({"cell": "A1", "input": "=py"})).is_err());
    // Other functions starting with PY are not affected.
    s.execute("cell.set", json!({"cell": "A2", "input": "=PI()"})).unwrap();
}

#[test]
fn entering_or_clearing_a_python_cell_removes_its_code() {
    let mut s = s();
    plant(&mut s, 0, "A1", "1 + 1");
    plant(&mut s, 0, "A2", "2 + 2");
    plant(&mut s, 0, "A3", "3 + 3");
    let got = s.execute("cell.get", json!({"cell": "A1"})).unwrap();
    assert_eq!(got["input"], "1 + 1");
    assert_eq!(got["python"]["code"], "1 + 1");
    assert_eq!(got["python"]["output"], "value");
    s.execute("cell.set", json!({"cell": "A1", "input": "hello"})).unwrap();
    assert_eq!(code(&s, 0, "A1"), None);
    s.execute("edit.clearFormats", json!({"range": "A2"})).unwrap();
    assert_eq!(code(&s, 0, "A2").as_deref(), Some("2 + 2"), "clearing formats keeps the code");
    s.execute("edit.clearContents", json!({"range": "A2"})).unwrap();
    assert_eq!(code(&s, 0, "A2"), None);
    s.execute("edit.clearAll", json!({"range": "A3"})).unwrap();
    assert_eq!(code(&s, 0, "A3"), None);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(code(&s, 0, "A3").as_deref(), Some("3 + 3"), "undo brings the code back");
}

#[test]
fn copy_fill_and_sort_carry_the_code_and_paste_values_drops_it() {
    let mut s = s();
    plant(&mut s, 0, "A1", "xl('B1') * 2");
    s.execute("edit.copy", json!({"range": "A1"})).unwrap();
    s.execute("edit.paste", json!({"at": "C1"})).unwrap();
    assert_eq!(code(&s, 0, "C1").as_deref(), Some("xl('B1') * 2"), "the code is copied as it is");
    s.execute("edit.pasteSpecial", json!({"at": "D1", "what": "values"})).unwrap();
    assert_eq!(code(&s, 0, "D1"), None);
    assert_eq!(super::v(&s, "D1"), Value::Number(1.0), "the cached value is pasted");
    s.execute("edit.fillDown", json!({"range": "A1:A3"})).unwrap();
    assert_eq!(code(&s, 0, "A3").as_deref(), Some("xl('B1') * 2"));
    // Sorting moves Python cells with their rows.
    s.execute("range.setValues", json!({"range": "E1", "values": [[3], [1], [2]]})).unwrap();
    plant(&mut s, 0, "F1", "'three'");
    plant(&mut s, 0, "F2", "'one'");
    s.execute("data.sort", json!({"range": "E1:F3", "keys": [{"column": "E"}]})).unwrap();
    assert_eq!(code(&s, 0, "F1").as_deref(), Some("'one'"));
    assert_eq!(code(&s, 0, "F3").as_deref(), Some("'three'"));
}

#[test]
fn xl_references_follow_inserts_moves_and_renames() {
    let mut s = s();
    s.execute("home.insertSheet", json!({})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    let other = s.doc().unwrap().wb.sheet(1).unwrap().name.clone();
    plant(&mut s, 0, "C5", &format!("a = xl(\"A1:A3\")\nb = xl('{other}!B2', headers=True)\nc = xl(dynamic)\n# xl(\"A1\") stays in comments"));
    s.execute("home.insertRows", json!({"rows": "2:2"})).unwrap();
    let c = code(&s, 0, "C6").unwrap();
    assert!(c.starts_with("a = xl(\"A1:A4\")\n"), "{c}");
    assert!(c.contains(&format!("xl('{other}!B2', headers=True)")), "{c}");
    assert!(c.contains("# xl(\"A1\") stays"), "{c}");
    s.execute("sheet.rename", json!({"sheet": 1, "name": "Data"})).unwrap();
    assert!(code(&s, 0, "C6").unwrap().contains("xl('Data!B2', headers=True)"));
    // Cutting the cells a reference names moves the reference with them.
    s.execute("edit.cut", json!({"range": "A1:A4"})).unwrap();
    s.execute("edit.paste", json!({"at": "H1"})).unwrap();
    assert!(code(&s, 0, "C6").unwrap().starts_with("a = xl(\"H1:H4\")"), "{:?}", code(&s, 0, "C6"));
    // Deleting the referenced cells leaves #REF!, which fails when the code runs.
    s.execute("home.deleteColumns", json!({"columns": "H:H"})).unwrap();
    assert!(code(&s, 0, "C6").unwrap().starts_with("a = xl(\"#REF!\")"), "{:?}", code(&s, 0, "C6"));
}

#[test]
fn python_cells_survive_native_json() {
    let mut s = s();
    plant(&mut s, 0, "B2", "x = 1\nx");
    let wb = s.doc().unwrap().wb.clone();
    let json = serde_json::to_string(&*wb).unwrap();
    let back: gridcraft_model::Workbook = serde_json::from_str(&json).unwrap();
    let cell = back.sheet(0).unwrap().cell(CellRef::parse("B2").unwrap()).unwrap();
    assert_eq!(cell.py.as_ref().unwrap().code, "x = 1\nx");
    // Cells written before Python existed have no `py` field.
    let plain: Cell = serde_json::from_value(json!({"value": {"t": "Number", "v": 2.0}, "formula": null, "style": 0})).unwrap();
    assert!(plain.py.is_none());
    assert!(!serde_json::to_string(&plain).unwrap().contains("py"), "no field is written for ordinary cells");
}

#[test]
fn new_errors_parse_and_have_error_type_numbers() {
    use gridcraft_core::CellError;
    for (text, e) in
        [("#PYTHON!", CellError::Python), ("#BUSY!", CellError::Busy), ("#TIMEOUT!", CellError::Timeout), ("#BLOCKED!", CellError::Blocked)]
    {
        assert_eq!(CellError::parse(text), Some(e));
        assert_eq!(e.as_str(), text);
    }
    let mut s = s();
    s.execute("cell.set", json!({"cell": "A1", "input": "=ERROR.TYPE(#BLOCKED!)"})).unwrap();
    assert_eq!(super::v(&s, "A1"), Value::Number(11.0));
    s.execute("cell.set", json!({"cell": "A2", "input": "=ISERROR(#PYTHON!)"})).unwrap();
    assert_eq!(super::v(&s, "A2"), Value::Bool(true));
}
