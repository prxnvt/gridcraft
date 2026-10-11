//! Python cells in files: native JSON, and xlsx as `PY("…"&"…",0|1)` with cached values.

use std::io::Read;

use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_python::client::Launcher;
use serde_json::json;

use super::v;
use crate::{Session, Trust};

fn py_session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    s
}

fn py(s: &mut Session, cell: &str, code: &str) {
    s.execute("python.set", json!({"cell": cell, "code": code})).unwrap();
}

fn save(s: &mut Session, format: &str) -> Vec<u8> {
    let r = s.execute("file.saveBytes", json!({"format": format})).unwrap();
    crate::io::base64_decode(r["base64"].as_str().unwrap()).unwrap()
}

fn open(s: &mut Session, name: &str, bytes: &[u8]) {
    s.execute("file.open", json!({"name": name, "base64": crate::io::base64_encode(bytes)})).unwrap();
}

fn sheet_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    z.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml
}

fn code(s: &Session, cell: &str) -> Option<gridcraft_model::PyCode> {
    s.doc().unwrap().wb.active().unwrap().cell(CellRef::parse(cell).unwrap()).and_then(|c| c.py.as_deref()).cloned()
}

/// A workbook with a value, a formula over a Python cell, a spilling table, an object, long code
/// with quotes and line breaks, an error, and initialization code.
fn build() -> (Session, String) {
    let mut s = py_session();
    s.execute("python.initialization", json!({"code": "factor = 2"})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "21"})).unwrap();
    py(&mut s, "B1", "xl('A1') * factor");
    s.execute("cell.set", json!({"cell": "A5", "input": "=B1+1"})).unwrap();
    py(&mut s, "C1", "[[1, 'two'], [3, \"four\"]]");
    s.execute("python.set", json!({"cell": "E1", "code": "[1, 2]", "output": "object"})).unwrap();
    let long = format!("# {}\ntext = \"say \\\"hi\\\"\"\nlen(text)", "x".repeat(6_960));
    assert!(long.chars().count() <= gridcraft_model::python::MAX_CODE_CHARS);
    py(&mut s, "F1", &long);
    py(&mut s, "G1", "1/0");
    assert_eq!(v(&s, "B1"), Value::Number(42.0));
    assert_eq!(v(&s, "D2"), Value::text("four"));
    assert_eq!(v(&s, "F1"), Value::Number(8.0));
    (s, long)
}

#[test]
fn xlsx_stores_py_formulas_with_cached_values() {
    let (mut s, long) = build();
    let bytes = save(&mut s, "xlsx");
    let xml = sheet_xml(&bytes);
    assert!(
        xml.contains("<f>PY(&quot;xl('A1') * factor&quot;,0)</f><v>42</v>") || xml.contains("<f>PY(\"xl('A1') * factor\",0)</f><v>42</v>"),
        "{xml}"
    );
    assert!(!xml.contains("_xlfn.PY"), "plain PY, no prefix");
    assert!(xml.contains(",1)</f>"), "the object cell says 1");
    assert!(xml.contains("t=\"array\" ref=\"C1:D2\""), "the spill is saved like a dynamic array's: {xml}");
    assert!(xml.contains("#VALUE!") && !xml.contains("#PYTHON!"), "Python errors are cached as #VALUE!");

    open(&mut s, "book.xlsx", &bytes);
    let d = s.doc().unwrap();
    assert_eq!(d.trust, Trust::FromFile);
    assert!(d.py_log.is_empty(), "opening a file runs no Python");
    // Cached values show, spills included, and the code is back.
    assert_eq!(v(&s, "B1"), Value::Number(42.0));
    assert_eq!(v(&s, "A5"), Value::Number(43.0));
    assert_eq!(
        (v(&s, "C1"), v(&s, "D1"), v(&s, "C2"), v(&s, "D2")),
        (Value::Number(1.0), Value::text("two"), Value::Number(3.0), Value::text("four"))
    );
    assert_eq!(code(&s, "B1").unwrap().code, "xl('A1') * factor");
    assert!(code(&s, "E1").unwrap().object);
    assert_eq!(code(&s, "F1").unwrap().code, long, "7,000 characters, quotes and line breaks survive");
    assert_eq!(v(&s, "G1"), Value::Error(CellError::Value));
    assert_eq!(s.execute("python.initialization", json!({})).unwrap()["code"], "factor = 2");
    // Edits in the untrusted workbook run nothing.
    s.execute("cell.set", json!({"cell": "A1", "input": "5"})).unwrap();
    assert_eq!(v(&s, "B1"), Value::Number(42.0));
    // Enabling runs everything with the initialization code.
    s.execute("python.enable", json!({})).unwrap();
    assert_eq!(v(&s, "B1"), Value::Number(10.0));
    assert_eq!(v(&s, "D2"), Value::text("four"));
    assert_eq!(v(&s, "G1"), Value::Error(CellError::Python));
}

#[test]
fn native_json_keeps_python_cells_and_their_results() {
    let (mut s, long) = build();
    let bytes = save(&mut s, "json");
    open(&mut s, "book.json", &bytes);
    assert_eq!(s.doc().unwrap().trust, Trust::FromFile);
    assert!(s.doc().unwrap().py_log.is_empty());
    assert_eq!(v(&s, "B1"), Value::Number(42.0));
    assert_eq!(v(&s, "D2"), Value::text("four"));
    assert_eq!(v(&s, "G1"), Value::Error(CellError::Python), "the native format keeps the exact error");
    assert_eq!(code(&s, "F1").unwrap().code, long);
    let e1 = code(&s, "E1").unwrap();
    assert!(e1.object);
    assert_eq!(e1.output.unwrap().type_name, "list");
    assert_eq!(s.execute("python.initialization", json!({})).unwrap()["code"], "factor = 2");
}

#[test]
fn hostile_python_content_in_files_is_capped() {
    let (mut s, _) = build();
    let bytes = save(&mut s, "json");
    // Make the Python cells hostile: code over the cap, and a result whose shape claims far more
    // cells than it holds (the array fields are public, so a crafted file can say anything).
    let mut wb: gridcraft_model::Workbook = serde_json::from_slice(&bytes).unwrap();
    let sh = wb.sheet_mut(0).unwrap();
    let b1 = sh.cells.get_mut(CellRef::parse("B1").unwrap()).unwrap();
    std::sync::Arc::make_mut(b1.py.as_mut().unwrap()).code = "y".repeat(9_000);
    let c1 = sh.cells.get_mut(CellRef::parse("C1").unwrap()).unwrap();
    let out = std::sync::Arc::make_mut(c1.py.as_mut().unwrap()).output.as_mut().unwrap();
    out.value = Value::Array(std::sync::Arc::new(gridcraft_core::Array { rows: 100_000, cols: 100_000, data: vec![Value::Number(1.0)] }));
    let bytes = serde_json::to_vec(&wb).unwrap();
    let (wb, warnings) = crate::io::open_bytes("evil.json", &bytes).unwrap();
    let sh = wb.sheet(0).unwrap();
    let b1 = sh.cell(CellRef::parse("B1").unwrap()).unwrap();
    assert!(b1.py.is_none(), "code over 7,000 characters is dropped");
    assert_eq!(b1.value, Value::Number(42.0), "the cached value stays");
    assert!(warnings.iter().any(|w| w.contains("Python cell")), "{warnings:?}");
    let c1 = sh.cell(CellRef::parse("C1").unwrap()).unwrap();
    assert!(c1.py.as_ref().unwrap().output.is_none(), "a malformed result is dropped");
    // Opening it lays nothing out from the bad result and doesn't hang.
    open(&mut s, "evil.json", &bytes);
    assert_eq!(v(&s, "C1"), Value::Number(1.0));

    // PY formulas from other formats become Python cells; non-literal PY calls stay formulas.
    let mut wb = gridcraft_model::Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    sh.set_cell(CellRef::new(0, 0), gridcraft_model::Cell::formula(gridcraft_model::Formula::new("=PY(\"1+\"&\"1\",1)")));
    sh.set_cell(CellRef::new(1, 0), gridcraft_model::Cell::formula(gridcraft_model::Formula::new("=PY(A1,0)")));
    crate::io::adopt_python(&mut wb);
    let sh = wb.sheet(0).unwrap();
    let a1 = sh.cell(CellRef::new(0, 0)).unwrap();
    assert_eq!(a1.py.as_ref().map(|p| (p.code.as_str(), p.object)), Some(("1+1", true)));
    assert!(a1.formula.is_none());
    assert!(sh.cell(CellRef::new(1, 0)).unwrap().py.is_none());
}

#[test]
fn a_malformed_result_never_spills() {
    let mut wb = gridcraft_model::Workbook::new();
    let bad = gridcraft_core::Array { rows: 4_000_000_000, cols: 4_000_000_000, data: vec![] };
    let py = gridcraft_model::PyCode {
        output: Some(gridcraft_model::PyOutput { value: Value::Array(std::sync::Arc::new(bad)), convertible: true, ..Default::default() }),
        ..gridcraft_model::PyCode::new("x")
    };
    let cell = gridcraft_model::Cell { py: Some(std::sync::Arc::new(py)), ..Default::default() };
    wb.sheet_mut(0).unwrap().set_cell(CellRef::new(0, 0), cell);
    gridcraft_calc::lay_python_spill(&mut wb, 0, CellRef::new(0, 0));
    assert_eq!(wb.sheet(0).unwrap().value(CellRef::new(0, 0)), Value::Error(CellError::Spill));
    assert!(wb.sheet(0).unwrap().spill.is_empty());
}
