//! Python results you can use: object-mode labels and tables, Spill Here, multi-line text kept
//! in the cell or spilled one line per row (round-tripped through files), CSV and Markdown.

use gridcraft_core::{CellRef, Value};
use gridcraft_python::client::Launcher;
use serde_json::json;

use super::v;
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    s
}

fn py(s: &Session, cell: &str) -> gridcraft_model::PyCode {
    s.doc().unwrap().wb.active().unwrap().cell(CellRef::parse(cell).unwrap()).and_then(|c| c.py.as_deref()).cloned().unwrap()
}

#[test]
fn object_mode_shows_the_size_and_keeps_the_table() {
    let mut s = session();
    s.execute("python.set", json!({"cell": "A1", "code": "{'region': ['N', 'S'], 'units': [3, 4]}", "output": "object"})).unwrap();
    assert_eq!(v(&s, "A1"), Value::text("dict 2 keys"));
    assert_eq!(v(&s, "A2"), Value::Empty, "an object doesn't spill");
    let code = py(&s, "A1");
    let (table, headers) = code.table().unwrap();
    assert!(headers);
    assert_eq!(crate::python::table_text(table), [["region", "units"], ["N", "3"], ["S", "4"]]);
    let got = s.execute("cell.get", json!({"cell": "A1"})).unwrap();
    assert_eq!((got["python"]["label"].as_str(), got["python"]["tabular"].as_bool()), (Some("dict 2 keys"), Some(true)));
    // Spill Here: the same result as values (a dict spills as key/value rows).
    let r = s.execute("python.output", json!({"cell": "A1", "mode": "value"})).unwrap();
    assert_eq!(r["mode"], "value");
    assert_eq!(v(&s, "A1"), Value::text("region"));
    assert_eq!(v(&s, "A2"), Value::text("units"));
    // Labels for the rest of the built-in kinds.
    for (code, label) in [("list(range(12))", "list 12"), ("'a\\nb\\nc\\nd\\ne'", "str 5 lines"), ("{1, 2}", "set 2")] {
        s.execute("python.set", json!({"cell": "C1", "code": code, "output": "object"})).unwrap();
        assert_eq!(v(&s, "C1"), Value::text(label), "{code}");
    }
    s.execute("python.set", json!({"cell": "C1", "code": "'one line'", "output": "object"})).unwrap();
    assert_eq!(v(&s, "C1"), Value::text("one line"), "a one-line string shows itself");
}

#[test]
fn multi_line_text_stays_in_the_cell_or_spills_one_line_per_row() {
    let mut s = session();
    s.execute("python.set", json!({"cell": "B2", "code": "'alpha\\nbeta\\ngamma'"})).unwrap();
    assert_eq!(v(&s, "B2"), Value::text("alpha\nbeta\ngamma"), "kept in the cell by default");
    assert_eq!(v(&s, "B3"), Value::Empty);
    let r = s.execute("python.output", json!({"cell": "B2", "lines": "rows"})).unwrap();
    assert_eq!(r["lines"], "rows");
    assert_eq!((v(&s, "B2"), v(&s, "B3"), v(&s, "B4")), (Value::text("alpha"), Value::text("beta"), Value::text("gamma")));
    assert!(s.execute("python.output", json!({"cell": "B2", "lines": "columns"})).is_err());
    assert!(s.execute("python.output", json!({"cell": "B2"})).is_err(), "mode or lines is needed");
    assert_eq!(s.execute("cell.get", json!({"cell": "B2"})).unwrap()["python"]["lines"], "rows");
    // Both files keep the choice, and the spilled lines show before Python runs again.
    for format in ["xlsx", "json"] {
        let r = s.execute("file.saveBytes", json!({"format": format})).unwrap();
        let mut t = Session::new();
        t.execute("file.open", json!({"name": format!("x.{format}"), "base64": r["base64"]})).unwrap();
        assert!(py(&t, "B2").lines, "{format}");
        assert_eq!(v(&t, "B4"), Value::text("gamma"), "{format}");
        let names: Vec<String> = t.doc().unwrap().wb.names.iter().map(|n| n.name.clone()).collect();
        assert!(!names.iter().any(|n| n.contains("py_lines")), "the list is read into the cells: {names:?}");
    }
    // Back to one cell.
    s.execute("python.output", json!({"cell": "B2", "lines": "cell"})).unwrap();
    assert_eq!(v(&s, "B3"), Value::Empty);
    let r = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    let mut t = Session::new();
    t.execute("file.open", json!({"name": "x.xlsx", "base64": r["base64"]})).unwrap();
    assert!(!py(&t, "B2").lines);
}

#[test]
fn hostile_line_lists_in_files_are_ignored() {
    let mut s = session();
    s.execute("python.set", json!({"cell": "A1", "code": "'a\\nb'"})).unwrap();
    let mut wb = s.doc().unwrap().wb.as_ref().clone();
    crate::cmd::stash::set_json(&mut wb, crate::python::LINES_KEY, &json!([["Nowhere", "A1"], ["Sheet1", "not a cell"], 7, ["Sheet1", "A1"]]))
        .unwrap();
    crate::python::load_line_names(&mut wb);
    let cell = wb.sheets[0].cell(CellRef::new(0, 0)).unwrap();
    assert!(cell.py.as_ref().unwrap().lines);
    let mut wb2 = wb.clone();
    crate::cmd::stash::set_json(&mut wb2, crate::python::LINES_KEY, &json!("garbage")).unwrap();
    crate::python::load_line_names(&mut wb2);
    assert!(crate::cmd::stash::get(&wb2, crate::python::LINES_KEY).is_none());
}

#[test]
fn tables_copy_as_csv_and_markdown_with_escaping() {
    let rows = vec![
        vec!["name".to_string(), "note".to_string()],
        vec!["a,b".into(), "say \"hi\"".into()],
        vec!["pipe|here".into(), "two\nlines".into()],
        vec!["back\\slash".into(), String::new()],
    ];
    assert_eq!(crate::python::to_csv(&rows), "name,note\r\n\"a,b\",\"say \"\"hi\"\"\"\r\npipe|here,\"two\nlines\"\r\nback\\slash,\r\n");
    assert_eq!(
        crate::python::to_markdown(&rows, true),
        "| name | note |\n| --- | --- |\n| a,b | say \"hi\" |\n| pipe\\|here | two<br>lines |\n| back\\\\slash |  |\n"
    );
    assert_eq!(
        crate::python::to_markdown(&rows[1..2], false),
        "| 0 | 1 |\n| --- | --- |\n| a,b | say \"hi\" |\n",
        "numbered columns without a header"
    );
    assert_eq!(crate::python::to_markdown(&[], true), "");
}
