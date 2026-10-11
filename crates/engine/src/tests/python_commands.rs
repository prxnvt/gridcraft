//! The python.* commands.

use gridcraft_core::{CellError, Value};
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

fn py(s: &mut Session, cell: &str, code: &str) -> serde_json::Value {
    s.execute("python.set", json!({"cell": cell, "code": code})).unwrap()
}

#[test]
fn set_runs_the_code_and_clear_undoes_python() {
    let mut s = py_session();
    s.execute("cell.set", json!({"cell": "A1", "input": "20"})).unwrap();
    let r = py(&mut s, "B1", "xl('A1') + 1");
    assert_eq!(r["value"], 21.0);
    assert_eq!(v(&s, "B1"), Value::Number(21.0));
    assert_eq!(s.execute("cell.get", json!({"cell": "B1"})).unwrap()["input"], "xl('A1') + 1");
    // Setting the same code again doesn't run it.
    let runs = s.doc().unwrap().py_log.len();
    py(&mut s, "B1", "xl('A1') + 1");
    assert_eq!(s.doc().unwrap().py_log.len(), runs);
    s.execute("python.clear", json!({"cell": "B1"})).unwrap();
    assert_eq!(v(&s, "B1"), Value::Empty);
    assert!(s.execute("cell.get", json!({"cell": "B1"})).unwrap()["python"].is_null());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.execute("cell.get", json!({"cell": "B1"})).unwrap()["python"]["code"], "xl('A1') + 1");
    assert!(s.execute("python.clear", json!({"cell": "C9"})).is_err(), "not a Python cell");
    // Insert makes an empty Python cell (in the active cell by default).
    s.execute("selection.set", json!({"cell": "D4"})).unwrap();
    s.execute("python.insert", json!({})).unwrap();
    assert_eq!(s.execute("cell.get", json!({"cell": "D4"})).unwrap()["python"]["code"], "");
}

#[test]
fn output_mode_switches_between_object_and_values() {
    let mut s = py_session();
    py(&mut s, "A1", "[10, 20]");
    assert_eq!(v(&s, "A2"), Value::Number(20.0));
    s.execute("python.output", json!({"cell": "A1", "mode": "object"})).unwrap();
    assert_eq!(v(&s, "A1"), Value::text("list"));
    assert_eq!(v(&s, "A2"), Value::Empty, "an object doesn't spill");
    assert_eq!(s.execute("cell.get", json!({"cell": "A1"})).unwrap()["python"]["output"], "object");
    s.execute("python.output", json!({"cell": "A1", "mode": "value"})).unwrap();
    assert_eq!(v(&s, "A2"), Value::Number(20.0));
    assert!(s.execute("python.output", json!({"cell": "A1", "mode": "sideways"})).is_err());
    let r = s.execute("python.set", json!({"cell": "B1", "code": "3", "output": "object"})).unwrap();
    assert_eq!(r["value"], 3.0, "scalars show as values in object mode too");
}

#[test]
fn code_is_capped_and_protected_cells_refuse_it() {
    let mut s = py_session();
    let long = "1".repeat(gridcraft_model::python::MAX_CODE_CHARS + 1);
    assert!(s.execute("python.set", json!({"cell": "A1", "code": long})).is_err());
    assert!(s.execute("python.set", json!({"cell": "A1"})).is_err(), "code is required");
    assert!(s.execute("python.set", json!({"cell": "not a cell", "code": "1"})).is_err());
    s.execute("review.protectSheet", json!({})).unwrap();
    let e = s.execute("python.set", json!({"cell": "A1", "code": "1"})).unwrap_err();
    assert!(e.to_string().contains("protected"), "{e}");
}

#[test]
fn reset_and_diagnostics() {
    let mut s = py_session();
    s.execute("home.insertSheet", json!({"name": "Other"})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    py(&mut s, "A1", "total = 5\ntotal");
    py(&mut s, "A2", "total * 2");
    s.execute("python.set", json!({"cell": "Other!A1", "code": "1/0"})).unwrap();
    let all = s.execute("python.diagnostics", json!({})).unwrap();
    assert!(all["runs"].as_array().unwrap().len() >= 3);
    let errors = s.execute("python.diagnostics", json!({"errorsOnly": true})).unwrap();
    let errors = errors["runs"].as_array().unwrap();
    assert!(errors.iter().all(|r| r["status"] != "ok"));
    assert!(errors.iter().any(|r| r["sheet"] == "Other" && r["cell"] == "A1" && r["line"] == 1));
    let other = s.execute("python.diagnostics", json!({"sheet": "Other"})).unwrap();
    assert!(other["runs"].as_array().unwrap().iter().all(|r| r["sheet"] == "Other"));
    s.execute("python.reset", json!({"clearDiagnostics": true})).unwrap();
    let after = s.execute("python.diagnostics", json!({})).unwrap();
    let cells: Vec<String> =
        after["runs"].as_array().unwrap().iter().map(|r| format!("{}!{}", r["sheet"].as_str().unwrap(), r["cell"].as_str().unwrap())).collect();
    assert_eq!(cells, ["Sheet1!A1", "Sheet1!A2", "Other!A1"], "reset runs every cell in order, after clearing the log");
    assert_eq!(v(&s, "A2"), Value::Number(10.0));
}

#[test]
fn initialization_code_runs_first_and_again_on_change() {
    let mut s = py_session();
    s.execute("python.initialization", json!({"code": "rate = 2"})).unwrap();
    py(&mut s, "A1", "rate * 21");
    assert_eq!(v(&s, "A1"), Value::Number(42.0));
    assert_eq!(s.execute("python.initialization", json!({})).unwrap()["code"], "rate = 2");
    s.execute("python.initialization", json!({"code": "rate = 3"})).unwrap();
    assert_eq!(v(&s, "A1"), Value::Number(63.0), "changing it reruns the cells");
    // An error in it is reported, and cells still run.
    s.execute("python.initialization", json!({"code": "rate = 1/0"})).unwrap();
    assert_eq!(v(&s, "A1"), Value::Error(CellError::Python), "`rate` is undefined");
    let runs = s.execute("python.diagnostics", json!({})).unwrap();
    assert!(runs["runs"].as_array().unwrap().iter().any(|r| r["error"].as_str().is_some_and(|e| e.starts_with("Initialization: ZeroDivisionError"))));
    s.execute("python.initialization", json!({"code": ""})).unwrap();
    assert_eq!(s.execute("python.initialization", json!({})).unwrap()["code"], "");
}

#[test]
fn files_need_python_enabled() {
    let mut s = py_session();
    s.doc_mut().unwrap().trust = Trust::FromFile;
    py(&mut s, "A1", "6 * 7");
    assert_eq!(v(&s, "A1"), Value::Error(CellError::Blocked));
    s.execute("python.reset", json!({})).unwrap();
    assert_eq!(v(&s, "A1"), Value::Error(CellError::Blocked), "reset doesn't enable Python");
    let r = s.execute("python.enable", json!({})).unwrap();
    assert_eq!(r["trust"], "trusted");
    assert_eq!(v(&s, "A1"), Value::Number(42.0));
    py(&mut s, "A2", "1 + 1");
    assert_eq!(v(&s, "A2"), Value::Number(2.0));
}

#[test]
fn without_a_worker_python_cells_are_blocked() {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::Unavailable("no Python here".into()));
    py(&mut s, "A1", "1");
    assert_eq!(v(&s, "A1"), Value::Error(CellError::Blocked));
    let runs = s.execute("python.diagnostics", json!({})).unwrap();
    assert_eq!(runs["runs"][0]["status"], "blocked");
    assert_eq!(runs["runs"][0]["error"], "no Python here");
}

#[test]
fn the_time_limit_is_a_setting() {
    let mut s = py_session();
    assert_eq!(s.execute("python.options", json!({})).unwrap()["timeoutSeconds"], 5.0);
    s.execute("python.options", json!({"timeoutSeconds": 0.2})).unwrap();
    assert!(s.execute("python.options", json!({"timeoutSeconds": 0})).is_err());
    assert!(s.execute("python.options", json!({"timeoutSeconds": f64::MAX})).is_err());
    py(&mut s, "A1", "while True: pass");
    assert_eq!(v(&s, "A1"), Value::Error(CellError::Timeout));
}

#[test]
fn table_headers_refuse_python() {
    let mut s = py_session();
    s.execute("range.setValues", json!({"range": "A1", "values": [["Name", "Score"], ["a", 1]]})).unwrap();
    s.execute("insert.table", json!({"range": "A1:B2", "headers": true})).unwrap();
    assert!(s.execute("python.set", json!({"cell": "B1", "code": "1"})).is_err());
    assert!(s.execute("python.insert", json!({"cell": "B1"})).is_err());
    assert_eq!(v(&s, "B1"), Value::text("Score"), "the header keeps its name");
    py(&mut s, "B2", "40 + 2");
    assert_eq!(v(&s, "B2"), Value::Number(42.0), "body cells take Python");
}

#[test]
fn options_and_diagnostics_report_the_python_that_runs_cells() {
    use gridcraft_python::interp::{Detection, Interpreter, Rejected};
    let mut s = py_session();
    // An explicit launcher (tests) never searches.
    let o = s.execute("python.options", json!({})).unwrap();
    assert_eq!(o["backend"]["kind"], "custom");
    assert_eq!(o["interpreter"], "");
    let rejected = vec![Rejected { path: "/usr/bin/python3".into(), reason: "it is Python 3.9.6; Python cells need 3.11 or newer".into() }];
    s.set_python_detection(Detection { found: None, rejected: rejected.clone() });
    let d = s.execute("python.diagnostics", json!({})).unwrap();
    assert_eq!(d["backend"]["kind"], "builtin");
    assert_eq!(d["backend"]["summary"], "Running on the built-in Python (Monty): install Python 3.11+ for packages");
    assert_eq!(d["backend"]["rejected"][0]["reason"], "it is Python 3.9.6; Python cells need 3.11 or newer");
    let found = Interpreter {
        program: "/opt/homebrew/bin/python3".into(),
        args: vec![],
        version: "3.14.0".into(),
        pandas: Some("2.2.3".into()),
        numpy: None,
        externally_managed: true,
        source: "PATH".into(),
    };
    s.set_python_detection(Detection { found: Some(found), rejected });
    let b = &s.execute("python.options", json!({})).unwrap()["backend"];
    assert_eq!(b["kind"], "cpython");
    assert_eq!(b["summary"], "Running on CPython 3.14 (/opt/homebrew/bin/python3), pandas 2.2.3");
    assert_eq!((b["version"].as_str(), b["externallyManaged"].as_bool()), (Some("3.14.0"), Some(true)));
    // The path is a preference; bad values are refused.
    let o = s.execute("python.options", json!({"interpreter": "  /nowhere/python  "})).unwrap();
    assert_eq!(o["interpreter"], "/nowhere/python");
    assert_eq!(s.prefs.python_interpreter, "/nowhere/python");
    assert!(s.execute("python.options", json!({"interpreter": 3})).is_err());
    assert!(s.execute("python.options", json!({"interpreter": "a\u{0}b"})).is_err());
    assert!(s.execute("python.options", json!({"interpreter": "x".repeat(5_000)})).is_err());
    let o = s.execute("python.options", json!({"interpreter": "", "detect": true})).unwrap();
    assert_eq!(o["interpreter"], "");
}
