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

fn on_1904(s: &mut Session) {
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).date_system = gridcraft_core::DateSystem::D1904;
}

fn format_cell(s: &mut Session, range: &str, code: &str) {
    s.execute("home.numberFormat", json!({"range": range, "code": code})).unwrap();
}

fn format_of(s: &Session, cell: &str) -> String {
    let wb = &s.doc().unwrap().wb;
    wb.styles.get(wb.active().unwrap().style_id(CellRef::parse(cell).unwrap())).num_fmt.as_str().to_string()
}

/// What `xl()` gives, and what comes back, on a real Python through the real sandbox, in one date
/// system: a date, a time and a date-time read, a day added, and returned.
fn dates_round_trip(s: &mut Session, serial: f64) {
    set(s, "A1", &serial.to_string());
    format_cell(s, "A1", "m/d/yyyy");
    set(s, "A2", &format!("{}", serial + 0.5));
    format_cell(s, "A2", "m/d/yyyy h:mm");
    set(s, "A3", "0.25");
    format_cell(s, "A3", "h:mm");
    py(s, "B1", "type(xl(\"A1\")).__name__");
    py(s, "B2", "xl(\"A1\").isoformat()");
    py(s, "B3", "import datetime\nxl(\"A1\") + datetime.timedelta(days=1)");
    py(s, "B4", "type(xl(\"A2\")).__name__");
    py(s, "B5", "xl(\"A2\").isoformat()");
    py(s, "B6", "xl(\"A2\") + datetime.timedelta(days=1)");
    py(s, "B7", "type(xl(\"A3\")).__name__");
    py(s, "B8", "xl(\"A3\").isoformat()");
    py(s, "B9", "xl(\"A3\")");
    settle(s);
    assert_eq!(v(s, "B1"), Value::text("date"));
    assert_eq!(v(s, "B2"), Value::text("2024-01-15"));
    assert_eq!((v(s, "B3"), format_of(s, "B3")), (num(serial + 1.0), "m/d/yyyy".into()), "read a date, add a day, return it");
    assert_eq!(v(s, "B4"), Value::text("datetime"));
    assert_eq!(v(s, "B5"), Value::text("2024-01-15T12:00:00"));
    assert_eq!((v(s, "B6"), format_of(s, "B6")), (num(serial + 1.5), "m/d/yyyy h:mm".into()));
    assert_eq!(v(s, "B7"), Value::text("time"));
    assert_eq!(v(s, "B8"), Value::text("06:00:00"));
    assert_eq!((v(s, "B9"), format_of(s, "B9")), (num(0.25), "h:mm:ss".into()));
}

#[test]
fn dates_round_trip_in_the_1900_system() {
    let Some(mut s) = session() else { return };
    dates_round_trip(&mut s, 45306.0);
}

#[test]
fn dates_round_trip_in_the_1904_system() {
    let Some(mut s) = session() else { return };
    on_1904(&mut s);
    dates_round_trip(&mut s, 43844.0);
}

#[test]
fn the_days_that_never_were_stay_numbers_and_their_neighbours_are_real() {
    let Some(mut s) = session() else { return };
    for (cell, input) in [("A1", "60"), ("A2", "0"), ("A3", "0.5"), ("A4", "-1"), ("A5", "59"), ("A6", "61"), ("A7", "1")] {
        set(&mut s, cell, input);
    }
    format_cell(&mut s, "A1:A7", "m/d/yyyy");
    for (cell, source) in [("B1", "A1"), ("B2", "A2"), ("B3", "A3"), ("B4", "A4")] {
        py(&mut s, cell, &format!("[[type(xl(\"{source}\")).__name__, xl(\"{source}\")]]"));
    }
    py(&mut s, "E1", "xl(\"A5\").isoformat()");
    py(&mut s, "E2", "xl(\"A6\").isoformat()");
    py(&mut s, "E3", "xl(\"A7\").isoformat()");
    // Going over the day that never was, in Python's calendar, which has none.
    py(&mut s, "E4", "import datetime\nxl(\"A5\") + datetime.timedelta(days=1)");
    settle(&mut s);
    assert_eq!(v(&s, "E1"), Value::text("1900-02-28"));
    assert_eq!(v(&s, "E2"), Value::text("1900-03-01"));
    assert_eq!(v(&s, "E3"), Value::text("1900-01-01"));
    assert_eq!(v(&s, "E4"), num(61.0));
    for (row, type_name, number) in [(1, "int", 60.0), (2, "int", 0.0), (3, "float", 0.5), (4, "int", -1.0)] {
        assert_eq!(v(&s, &format!("B{row}")), Value::text(type_name), "{row}");
        assert_eq!(v(&s, &format!("C{row}")), num(number), "{row}: it stays the number");
    }
}

#[test]
fn a_range_reads_its_dates_and_a_spill_shows_them() {
    let Some(mut s) = session() else { return };
    s.execute("range.setValues", json!({"range": "A1", "values": [[45306, "x"], [45307, 3]]})).unwrap();
    format_cell(&mut s, "A1:A2", "m/d/yyyy");
    py(
        &mut s,
        "D1",
        "import datetime\n[datetime.date(2024, 1, 15), datetime.time(12), datetime.datetime(2024, 1, 15, 6), 'text', 7, datetime.timedelta(hours=36)]",
    );
    settle(&mut s);
    assert_eq!((v(&s, "D1"), format_of(&s, "D1")), (num(45306.0), "m/d/yyyy h:mm".into()), "a column with a time shows it on all its dates");
    assert_eq!((v(&s, "D2"), format_of(&s, "D2")), (num(0.5), "h:mm:ss".into()));
    assert_eq!((v(&s, "D3"), format_of(&s, "D3")), (num(45306.25), "m/d/yyyy h:mm".into()));
    assert_eq!((v(&s, "D4"), format_of(&s, "D4")), (Value::text("text"), "General".into()));
    assert_eq!((v(&s, "D5"), format_of(&s, "D5")), (num(7.0), "General".into()));
    assert_eq!((v(&s, "D6"), format_of(&s, "D6")), (num(1.5), "[h]:mm:ss".into()));
    // Something in the way: nothing is formatted.
    py(&mut s, "F1", "[xl(\"A1\"), xl(\"A2\")]");
    set(&mut s, "F2", "5");
    settle(&mut s);
    assert_eq!(v(&s, "F1"), Value::Error(CellError::Spill));
    assert_eq!(format_of(&s, "F2"), "General");
    set(&mut s, "F2", "");
    settle(&mut s);
    assert_eq!((v(&s, "F2"), format_of(&s, "F2")), (num(45307.0), "m/d/yyyy".into()));
}

#[test]
fn what_a_serial_can_not_hold_is_num_and_the_worker_stays() {
    let Some(mut s) = session() else { return };
    py(&mut s, "A1", "keep = 7\nkeep");
    py(&mut s, "A2", "import datetime\ndatetime.date(1800, 1, 1)");
    py(&mut s, "A3", "datetime.datetime.min");
    py(&mut s, "A4", "datetime.timedelta.max");
    py(&mut s, "A5", "datetime.date(1899, 12, 31)");
    py(&mut s, "A6", "datetime.date.max");
    py(&mut s, "A7", "datetime.datetime.max");
    py(&mut s, "A8", "keep + 1");
    settle(&mut s);
    for cell in ["A2", "A3", "A4", "A5"] {
        assert_eq!((v(&s, cell), kind(&s, cell)), (Value::Error(CellError::Num), Some("result.date")), "{cell}");
    }
    assert_eq!(v(&s, "A6"), num(2_958_465.0));
    let Value::Number(last) = v(&s, "A7") else { panic!() };
    assert!((2_958_465.0..2_958_466.0).contains(&last), "{last}: the last moment stays in the last day");
    assert_eq!(v(&s, "A8"), num(8.0), "the same worker, with its variables");
    // And it reads back as the date it is.
    py(&mut s, "B7", "xl(\"A7\").isoformat()");
    settle(&mut s);
    assert_eq!(v(&s, "B7"), Value::text("9999-12-31T23:59:59.999000"));
}

#[test]
fn a_time_zone_is_only_where_the_wall_clock_is() {
    let Some(mut s) = session() else { return };
    py(&mut s, "A1", "import datetime\ndatetime.datetime(2024, 1, 15, 12, tzinfo=datetime.timezone(datetime.timedelta(hours=-5)))");
    settle(&mut s);
    assert_eq!(v(&s, "A1"), num(45306.5));
}

#[test]
fn a_cells_own_format_stays_and_a_format_is_no_undo_step() {
    let Some(mut s) = session() else { return };
    format_cell(&mut s, "A1", "yyyy-mm-dd");
    py(&mut s, "A1", "import datetime\ndatetime.date(2024, 1, 15)");
    py(&mut s, "A2", "datetime.date(2024, 1, 15)");
    let undo = s.doc().unwrap().undo.len();
    settle(&mut s);
    assert_eq!(s.doc().unwrap().undo.len(), undo, "a result and its format are no step");
    assert_eq!((format_of(&s, "A1"), format_of(&s, "A2")), ("yyyy-mm-dd".into(), "m/d/yyyy".into()));
}

/// A date column in and out of a DataFrame: what the brief's round trip looks like with pandas.
fn dataframe_round_trip(s: &mut Session, serial: f64) {
    s.execute("range.setValues", json!({"range": "A1", "values": [["when", "n"], [serial, 1], [serial + 1.0, 2], [null, 3]]})).unwrap();
    format_cell(s, "A2:A4", "m/d/yyyy");
    py(s, "D1", "xl(\"A1:B4\", headers=True).dtypes.iloc[0].kind");
    py(s, "D2", "str(xl(\"A1:B4\", headers=True)[\"when\"].iloc[2])");
    py(s, "F1", "d = xl(\"A1:B4\", headers=True)\nd[\"when\"] = d[\"when\"] + pd.Timedelta(days=1)\nd");
    settle(s);
    assert_eq!(v(s, "D1"), Value::text("M"), "a datetime column");
    assert_eq!(v(s, "D2"), Value::text("NaT"), "an empty cell");
    assert_eq!((v(s, "F1"), v(s, "G1")), (Value::text("when"), Value::text("n")));
    assert_eq!((v(s, "F2"), format_of(s, "F2")), (num(serial + 1.0), "m/d/yyyy".into()));
    assert_eq!((v(s, "F3"), format_of(s, "F3")), (num(serial + 2.0), "m/d/yyyy".into()));
    assert_eq!((v(s, "F4"), format_of(s, "F4")), (Value::Empty, "General".into()), "NaT is an empty cell");
    assert_eq!((v(s, "G2"), format_of(s, "G2")), (num(1.0), "General".into()));
    assert_eq!(format_of(s, "F1"), "General", "the header isn't a date");
}

#[test]
fn a_dataframe_date_column_round_trips_in_the_1900_system() {
    let Some(mut s) = pandas_session() else { return };
    dataframe_round_trip(&mut s, 45306.0);
}

#[test]
fn a_dataframe_date_column_round_trips_in_the_1904_system() {
    let Some(mut s) = pandas_session() else { return };
    on_1904(&mut s);
    dataframe_round_trip(&mut s, 43844.0);
}

#[test]
fn numpy_and_pandas_dates_and_durations_come_back_as_serials() {
    let Some(mut s) = pandas_session() else { return };
    for (cell, code, number, format) in [
        ("A1", "np.datetime64(\"2024-01-15\")", num(45306.0), "m/d/yyyy"),
        ("A2", "np.datetime64(\"2024-01-15T12:00\", \"ns\")", num(45306.5), "m/d/yyyy h:mm"),
        ("A3", "np.timedelta64(36, \"h\")", num(1.5), "[h]:mm:ss"),
        ("A4", "pd.Timestamp(\"2024-01-15\")", num(45306.0), "m/d/yyyy"),
        ("A5", "pd.Timestamp(\"2024-01-15 12:00\", tz=\"Asia/Tokyo\")", num(45306.5), "m/d/yyyy h:mm"),
        ("A6", "pd.Timedelta(days=1, hours=12)", num(1.5), "[h]:mm:ss"),
        ("A7", "pd.NaT", Value::Empty, "General"),
        ("A8", "np.datetime64(\"NaT\")", Value::Empty, "General"),
        ("A9", "np.datetime64(10**15, \"Y\")", Value::Error(CellError::Num), "General"),
        ("A10", "pd.Timedelta(days=-1)", num(-1.0), "General"),
    ] {
        py(&mut s, cell, code);
        settle(&mut s);
        assert_eq!((v(&s, cell), format_of(&s, cell)), (number, format.to_string()), "{code}");
    }
    // Across a spill, from a Series, the values of a column, and an index of dates.
    py(&mut s, "C1", "pd.Series(pd.to_datetime([\"2024-01-15\", \"2024-01-16\"]))");
    py(&mut s, "D1", "pd.Series(pd.to_datetime([\"2024-01-15\", \"2024-01-16\"])).values");
    py(&mut s, "E1", "pd.date_range(\"2024-01-15\", periods=2)");
    py(&mut s, "F1", "pd.DataFrame({\"at\": pd.to_datetime([\"2024-01-15 12:00\", \"2024-01-15 00:00\"])})");
    settle(&mut s);
    for column in ["C", "D", "E"] {
        let (top, under) = (format!("{column}1"), format!("{column}2"));
        assert_eq!((v(&s, &top), v(&s, &under)), (num(45306.0), num(45307.0)), "{column}");
        assert_eq!(format_of(&s, &under), "m/d/yyyy", "{column}");
    }
    assert_eq!((v(&s, "F2"), v(&s, "F3")), (num(45306.5), num(45306.0)));
    assert_eq!((format_of(&s, "F2"), format_of(&s, "F3")), ("m/d/yyyy h:mm".into(), "m/d/yyyy h:mm".into()), "one format down a column");
}
