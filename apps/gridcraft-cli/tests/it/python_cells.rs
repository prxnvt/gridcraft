//! Python cells end to end, through a real `Session`, the real sandbox and a real Python: what the
//! engine's tests with a fake worker can't show. `gridcraft-cli` is the launcher, as the app is in
//! production. On a machine with no Python 3.11 or newer, or that doesn't confine, the tests have
//! nothing to check and pass with a note; on CI (`CI` set) that is a failure, on the systems that
//! confine.
//!
//! The tests that need pandas take the interpreter from `GRIDCRAFT_TEST_PYTHON`, and pass with a
//! note without it.

use std::time::Duration;

use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_engine::python::Limits;
use gridcraft_engine::{PythonCapability, PythonState, Session};
use serde_json::json;

/// How long a test waits for Python, in polls of 50 ms.
const PATIENCE: u32 = 2_400;

/// A session whose Python is ready, or `None` when this machine can't give one.
fn session_with(limits: Limits, interpreter: Option<std::path::PathBuf>) -> Option<Session> {
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        eprintln!("this system doesn't confine programs; skipping");
        return None;
    }
    let mut s = Session::new();
    s.execute("file.new", json!({})).unwrap();
    s.set_python_capability(PythonCapability {
        enabled: true,
        interpreter,
        launcher: Some(env!("CARGO_BIN_EXE_gridcraft-cli").into()),
        limits,
        ..PythonCapability::default()
    });
    let mut left = PATIENCE;
    while s.python_environment().state == PythonState::Running {
        s.python_poll(Duration::from_millis(50));
        left -= 1;
        assert!(left > 0, "the check of Python never answered");
    }
    let env = s.python_environment();
    if env.state != PythonState::Ready {
        let why = env.diagnostic.as_ref().map_or_else(String::new, |d| format!("{}: {} {}", d.kind.id, d.kind.message, d.detail));
        assert!(std::env::var_os("CI").is_none(), "Python isn't ready on CI: {why}");
        eprintln!("Python isn't ready on this machine; skipping: {why}");
        return None;
    }
    Some(s)
}

fn session() -> Option<Session> {
    session_with(Limits::default(), None)
}

/// A session on the interpreter in `GRIDCRAFT_TEST_PYTHON`, which must have pandas.
fn pandas_session() -> Option<Session> {
    let Some(python) = std::env::var_os("GRIDCRAFT_TEST_PYTHON") else {
        eprintln!("GRIDCRAFT_TEST_PYTHON isn't set; skipping the tests that need pandas");
        return None;
    };
    let s = session_with(Limits::default(), Some(python.into()))?;
    assert!(s.python_environment().packages.iter().any(|(name, _)| name.eq_ignore_ascii_case("pandas")), "GRIDCRAFT_TEST_PYTHON has no pandas");
    Some(s)
}

fn settle(s: &mut Session) {
    let mut left = PATIENCE;
    while s.python_busy() {
        s.python_poll(Duration::from_millis(50));
        left -= 1;
        assert!(left > 0, "Python never settled");
    }
}

fn py(s: &mut Session, cell: &str, code: &str) {
    s.execute("python.set", json!({"cell": cell, "code": code})).unwrap();
}

fn set(s: &mut Session, cell: &str, input: &str) {
    s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
}

fn v(s: &Session, cell: &str) -> Value {
    s.doc().unwrap().wb.active().unwrap().value(CellRef::parse(cell).unwrap())
}

fn kind(s: &Session, cell: &str) -> Option<&'static str> {
    s.python_diagnostic(0, CellRef::parse(cell).unwrap()).map(|d| d.kind.id)
}

fn num(n: f64) -> Value {
    Value::Number(n)
}

#[test]
fn the_report_names_the_python_the_sandbox_and_what_is_installed() {
    let Some(mut s) = session() else { return };
    let env = s.python_environment();
    assert_eq!(env.state, PythonState::Ready);
    let python = env.interpreter.as_ref().expect("the report names the interpreter");
    assert!(python.path.is_absolute() && python.version >= gridcraft_python::interp::MIN_VERSION, "{python:?}");
    assert!(!env.sandbox.is_empty());
    assert!(env.diagnostic.is_none() || env.diagnostic.as_ref().is_some_and(|d| d.kind.id == "packages.noPandas"), "{:?}", env.diagnostic);
    let json = s.execute("python.environment", json!({})).unwrap();
    assert_eq!(json["state"], "ready");
    assert!(json["interpreter"]["version"].is_string());
    assert!(json["packages"].is_array());
}

#[test]
fn arithmetic_runs_and_a_formula_reads_the_result() {
    let Some(mut s) = session() else { return };
    py(&mut s, "A1", "21 * 2");
    assert_eq!(v(&s, "A1"), Value::Error(CellError::Busy), "it waits, and nothing blocked");
    set(&mut s, "B1", "=A1+1");
    settle(&mut s);
    assert_eq!(v(&s, "A1"), num(42.0));
    assert_eq!(v(&s, "B1"), num(43.0));
    assert_eq!(kind(&s, "A1"), None);
    py(&mut s, "A2", "'text'");
    py(&mut s, "A3", "1 == 1");
    settle(&mut s);
    assert_eq!(v(&s, "A2"), Value::text("text"));
    assert_eq!(v(&s, "A3"), Value::Bool(true));
}

#[test]
fn xl_reads_a_cell_and_a_range_and_a_change_runs_the_cell_again() {
    let Some(mut s) = session() else { return };
    set(&mut s, "A1", "20");
    set(&mut s, "B1", "1");
    set(&mut s, "B2", "2");
    set(&mut s, "B3", "3");
    py(&mut s, "C1", "xl(\"A1\") + 1");
    // A list of rows without pandas, a DataFrame with it.
    py(&mut s, "C2", "v = xl(\"B1:B3\")\nint(v.iloc[:, 0].sum()) if hasattr(v, 'iloc') else sum(r[0] for r in v)");
    settle(&mut s);
    assert_eq!(v(&s, "C1"), num(21.0));
    assert_eq!(v(&s, "C2"), num(6.0));
    set(&mut s, "A1", "30");
    set(&mut s, "B3", "30");
    settle(&mut s);
    assert_eq!(v(&s, "C1"), num(31.0));
    assert_eq!(v(&s, "C2"), num(33.0));
}

#[test]
fn a_list_of_lists_spills_and_a_cell_in_the_way_blocks_it() {
    let Some(mut s) = session() else { return };
    py(&mut s, "A1", "[[1, 2], [3, 4]]");
    settle(&mut s);
    assert_eq!((v(&s, "A1"), v(&s, "B1"), v(&s, "A2"), v(&s, "B2")), (num(1.0), num(2.0), num(3.0), num(4.0)));
    set(&mut s, "D1", "=SUM(A1:B2)");
    assert_eq!(v(&s, "D1"), num(10.0));
    set(&mut s, "B2", "x");
    assert_eq!(v(&s, "A1"), Value::Error(CellError::Spill));
    assert_eq!(kind(&s, "A1"), Some("result.spill"));
    set(&mut s, "B2", "");
    settle(&mut s);
    assert_eq!(v(&s, "B2"), num(4.0));
}

#[test]
fn opening_a_file_is_refused_by_the_sandbox_and_the_cell_says_so() {
    let Some(mut s) = session() else { return };
    let secret = std::env::temp_dir().canonicalize().unwrap().join(format!("gridcraft-python-cells-{}.txt", std::process::id()));
    std::fs::write(&secret, "secret\n").unwrap();
    py(&mut s, "A2", &format!("print('before')\nopen({:?}).read()", secret.display().to_string()));
    settle(&mut s);
    std::fs::remove_file(&secret).unwrap();
    assert_eq!(v(&s, "A2"), Value::Error(CellError::Python));
    let d = s.python_diagnostic(0, CellRef::parse("A2").unwrap()).unwrap();
    assert_eq!(d.kind.id, "denied.file", "{}", d.detail);
    assert_eq!(s.python_run_log().last().map(|r| r.stdout.as_str()), Some("before\n"), "what it printed is kept");
}

#[test]
fn a_cell_past_the_time_limit_shows_timeout_and_the_rest_goes_on() {
    let limits = Limits { timeout: Duration::from_millis(500), start_timeout: Duration::from_secs(60), ..Limits::default() };
    let Some(mut s) = session_with(limits, None) else { return };
    py(&mut s, "A1", "a = 5\na");
    settle(&mut s);
    assert_eq!(v(&s, "A1"), num(5.0));
    py(&mut s, "A2", "while True:\n    pass");
    py(&mut s, "A3", "a + 1");
    settle(&mut s);
    assert_eq!(v(&s, "A2"), Value::Error(CellError::Timeout));
    assert_eq!(kind(&s, "A2"), Some("limit.time"));
    assert_eq!(v(&s, "A3"), num(6.0), "a fresh worker ran what comes before it, and not the cell that stopped the last one");
}

#[test]
fn cells_share_variables_and_run_in_order() {
    let Some(mut s) = session() else { return };
    py(&mut s, "A1", "base = 10\nbase");
    py(&mut s, "A2", "base * 2");
    py(&mut s, "A3", "total + 1");
    py(&mut s, "A4", "total = 1\ntotal");
    settle(&mut s);
    assert_eq!(v(&s, "A2"), num(20.0));
    // A later cell defines it: the run order is what is wrong, and the cell says where to look.
    assert_eq!(v(&s, "A3"), Value::Error(CellError::Python));
    let d = s.python_diagnostic(0, CellRef::parse("A3").unwrap()).unwrap();
    assert_eq!(d.kind.id, "python.nameOrder");
    assert!(d.detail.contains("A4"), "{}", d.detail);
    // A cell that changes takes the ones after it with it.
    py(&mut s, "A1", "base = 100\nbase");
    settle(&mut s);
    assert_eq!(v(&s, "A2"), num(200.0));
    // An undefined name with no such cell is just that.
    py(&mut s, "A5", "nowhere");
    settle(&mut s);
    assert_eq!(kind(&s, "A5"), Some("python.name"));
}

#[test]
fn stop_and_reset_act_on_the_real_worker() {
    let Some(mut s) = session() else { return };
    py(&mut s, "A1", "a = 1\na");
    settle(&mut s);
    py(&mut s, "A2", "while True:\n    pass");
    // Sent to the worker, and left running.
    s.python_poll(Duration::ZERO);
    assert!(s.python_busy());
    assert_eq!(s.execute("python.stop", json!({})).unwrap()["stopped"], true);
    assert!(!s.python_busy());
    assert_eq!(v(&s, "A2"), Value::Error(CellError::Python));
    assert_eq!(kind(&s, "A2"), Some("state.stopped"));
    // Reset gives fresh variables and runs everything, the stopped cell too; so the cell goes first.
    py(&mut s, "A2", "b = a + 1\nb");
    settle(&mut s);
    assert_eq!(v(&s, "A2"), num(2.0));
    s.execute("python.reset", json!({})).unwrap();
    settle(&mut s);
    assert_eq!((v(&s, "A1"), v(&s, "A2")), (num(1.0), num(2.0)));
    assert!(s.python_run_log().count() >= 5);
}

#[test]
fn an_object_shows_its_type_with_its_preview_for_the_card() {
    let Some(mut s) = session() else { return };
    s.execute("python.set", json!({"cell": "A1", "code": "{'a': 1}", "output": "object"})).unwrap();
    py(&mut s, "A2", "def f():\n    pass\nf");
    settle(&mut s);
    assert_eq!(v(&s, "A1"), Value::text("dict"));
    let got = s.execute("cell.get", json!({"cell": "A1"})).unwrap();
    assert_eq!((got["python"]["typeName"].as_str(), got["python"]["preview"].as_str()), (Some("dict"), Some("{'a': 1}")));
    assert_eq!(v(&s, "A2"), Value::text("function"), "no cell form, even as an Excel value");
}

#[test]
fn a_range_is_a_dataframe_when_python_has_pandas() {
    let Some(mut s) = pandas_session() else { return };
    s.execute("range.setValues", json!({"range": "A1", "values": [["name", "score"], ["a", 1], ["b", 2], ["c", 4]]})).unwrap();
    py(&mut s, "D1", "type(xl(\"A1:B4\")).__name__");
    py(&mut s, "D2", "xl(\"A1:B4\", headers=True)[\"score\"].sum()");
    py(&mut s, "D3", "list(xl(\"A1:B4\", headers=True).columns)[1]");
    py(&mut s, "D4", "pd.__name__");
    settle(&mut s);
    assert_eq!(v(&s, "D1"), Value::text("DataFrame"));
    assert_eq!(v(&s, "D2"), num(7.0), "a numpy number comes back as a cell's number");
    assert_eq!(v(&s, "D3"), Value::text("score"));
    assert_eq!(v(&s, "D4"), Value::text("pandas"), "Excel's default names are there");
    set(&mut s, "B2", "10");
    settle(&mut s);
    assert_eq!(v(&s, "D2"), num(16.0));
}

#[test]
fn a_dataframe_result_spills_with_its_header_row() {
    let Some(mut s) = pandas_session() else { return };
    py(&mut s, "A1", "pd.DataFrame({'x': [1, 2], 'y': [3.5, 4.5]})");
    settle(&mut s);
    assert_eq!((v(&s, "A1"), v(&s, "B1")), (Value::text("x"), Value::text("y")), "its column names first");
    assert_eq!((v(&s, "A2"), v(&s, "B2")), (num(1.0), num(3.5)));
    assert_eq!((v(&s, "A3"), v(&s, "B3")), (num(2.0), num(4.5)));
    assert_eq!(v(&s, "A4"), Value::Empty);
    // As an object it stays in one cell.
    s.execute("python.output", json!({"cell": "A1", "mode": "object"})).unwrap();
    settle(&mut s);
    assert_eq!(v(&s, "A1"), Value::text("DataFrame"));
    assert_eq!(v(&s, "A2"), Value::Empty);
}

#[test]
fn a_memory_spike_lands_on_its_cell_and_the_next_cell_keeps_its_variables() {
    let limits = Limits { memory_bytes: 200 * 1024 * 1024, ..Limits::default() };
    let Some(mut s) = session_with(limits, None) else { return };
    py(&mut s, "A1", "x = 40\nx + 2");
    py(&mut s, "A2", "spike = b'x' * (400 * 1024 * 1024)\ndel spike\n'done'");
    py(&mut s, "A3", "x + 1");
    py(&mut s, "A4", "x + 2");
    settle(&mut s);
    assert_eq!(v(&s, "A1"), num(42.0));
    assert_eq!(v(&s, "A2"), Value::Error(CellError::Python));
    assert_eq!(kind(&s, "A2"), Some("limit.memory"), "the cell that did it, through the sandbox");
    assert_eq!((v(&s, "A3"), kind(&s, "A3")), (num(41.0), None), "a fresh worker ran A1 first");
    assert_eq!((v(&s, "A4"), kind(&s, "A4")), (num(42.0), None));
}
