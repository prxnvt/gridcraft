//! The Diagnostics log: runs of a cell that end the same way share a row with a count, and the
//! header lists numpy and the workbook's packages, found or missing.

use gridcraft_python::client::Launcher;
use gridcraft_python::interp::{Detection, Interpreter};
use serde_json::json;

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    s
}

fn rows(s: &mut Session) -> Vec<String> {
    let d = s.execute("python.diagnostics", json!({})).unwrap();
    d["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| format!("{}:{}x{}", r["cell"].as_str().unwrap(), r["status"].as_str().unwrap(), r["count"]))
        .collect()
}

#[test]
fn repeated_runs_of_a_cell_collapse_until_the_result_changes() {
    let mut s = session();
    s.execute("python.set", json!({"cell": "A1", "code": "1/0"})).unwrap();
    s.execute("python.set", json!({"cell": "A2", "code": "2"})).unwrap();
    s.execute("python.reset", json!({})).unwrap();
    s.execute("python.reset", json!({})).unwrap();
    assert_eq!(rows(&mut s), ["A1:errorx3", "A2:okx3"], "each cell's three runs share a row");
    let d = s.execute("python.diagnostics", json!({})).unwrap();
    let a1 = &d["runs"][0];
    assert!(a1["timeMs"].as_f64().unwrap() > 0.0 && a1["seq"].as_u64().unwrap() > 1, "{a1}");
    // A different result starts a new row; the same error again after it, another.
    s.execute("python.set", json!({"cell": "A1", "code": "1"})).unwrap();
    s.execute("python.set", json!({"cell": "A1", "code": "1/0"})).unwrap();
    assert_eq!(rows(&mut s), ["A1:errorx3", "A1:okx1", "A1:errorx1", "A2:okx5"], "A2 ran each time with the same result");
    // A different message is a different result.
    s.execute("python.set", json!({"cell": "A1", "code": "undefined_name"})).unwrap();
    assert_eq!(rows(&mut s), ["A1:errorx3", "A1:okx1", "A1:errorx1", "A1:errorx1", "A2:okx6"]);
}

#[test]
fn the_header_lists_numpy_and_the_workbooks_packages() {
    let mut s = session();
    s.execute("python.packages", json!({"packages": ["pandas>=2", "numpy", "polars"]})).unwrap();
    let found = Interpreter {
        program: "/opt/homebrew/bin/python3".into(),
        version: "3.14.0".into(),
        pandas: Some("2.2.3".into()),
        numpy: Some("2.1.0".into()),
        installed: vec![("pandas".into(), "2.2.3".into()), ("NumPy".into(), "2.1.0".into())],
        source: "PATH".into(),
        ..Interpreter::default()
    };
    s.set_python_detection(Detection { found: Some(found), rejected: vec![] });
    let d = s.execute("python.diagnostics", json!({})).unwrap();
    assert_eq!(d["backend"]["summary"], "Running on CPython 3.14 (/opt/homebrew/bin/python3), pandas 2.2.3, numpy 2.1.0");
    assert_eq!(
        d["packages"],
        json!([
            {"requirement": "pandas>=2", "name": "pandas", "found": true, "version": "2.2.3"},
            {"requirement": "numpy", "name": "numpy", "found": true, "version": "2.1.0"},
            {"requirement": "polars", "name": "polars", "found": false, "version": null},
        ])
    );
    // On the built-in Python, every package is missing.
    s.set_python_detection(Detection::default());
    let d = s.execute("python.diagnostics", json!({})).unwrap();
    assert!(d["packages"].as_array().unwrap().iter().all(|p| p["found"] == false));
}
