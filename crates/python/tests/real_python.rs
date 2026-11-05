//! The shim on a real interpreter. Without one the tests print why and pass, except on CI with a
//! `python3` on PATH, where a missing interpreter is a failure.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::disallowed_methods)] // the clock, off wasm

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gridcraft_core::{Array, CellError, Value};
use gridcraft_python::interp::{self, Detection, SearchEnv};
use gridcraft_python::process::ProcessTransport;
use gridcraft_python::{Client, Input, Limits, Outcome, Output, PyError, RunResult, SelfTest, Transport};

fn python() -> Option<PathBuf> {
    let env = SearchEnv::from_process();
    let found = match interp::detect(&env, None, interp::PROBE_TIMEOUT) {
        Detection::Found { path, .. } => Some(path),
        _ => None,
    };
    if found.is_none() {
        let ci = std::env::var_os("CI").is_some();
        let on_path = !interp::candidates(&SearchEnv { path: env.path.clone(), ..SearchEnv::default() }).is_empty();
        assert!(!(ci && on_path), "CI has a python on PATH, but no Python 3.11 or newer was found");
        println!("skipped: no Python 3.11 or newer found");
    }
    found
}

fn client(python: PathBuf) -> Client {
    Client::new(
        Box::new(ProcessTransport::new(python)),
        Limits { timeout: Duration::from_secs(10), start_timeout: Duration::from_secs(20), ..Limits::default() },
    )
}

fn run(c: &mut Client, code: &str) -> Outcome {
    c.run(code, BTreeMap::new()).unwrap()
}

fn value(o: Outcome) -> Value {
    match o {
        Outcome::Ok { output: Output::Value(v), .. } => v,
        other => panic!("not a value: {other:?}"),
    }
}

fn array(rows: usize, cols: usize, data: Vec<Value>) -> Value {
    Array::new(rows, cols, data).map(Value::from).unwrap()
}

#[test]
fn values_and_errors() {
    let Some(py) = python() else { return };
    let mut c = client(py);
    assert_eq!(value(run(&mut c, "1 + 2")), Value::Number(3.0));
    assert_eq!(c.self_test(), Some(&SelfTest::NotChecked));
    assert_eq!(value(run(&mut c, "'text'")), Value::text("text"));
    assert_eq!(value(run(&mut c, "True")), Value::Bool(true));
    assert_eq!(value(run(&mut c, "x = 1")), Value::Empty, "no last expression");
    assert_eq!(value(run(&mut c, "None")), Value::Empty);
    assert_eq!(value(run(&mut c, "float('nan')")), Value::Error(CellError::Num));
    assert_eq!(value(run(&mut c, "10 ** 400")), Value::Error(CellError::Num));
    assert_eq!(
        value(run(&mut c, "[[1, 'a'], [True]]")),
        array(2, 2, vec![1.0.into(), Value::text("a"), Value::Bool(true), Value::Error(CellError::NA)])
    );
    assert_eq!(value(run(&mut c, "[1, 2, 3]")), array(3, 1, vec![1.0.into(), 2.0.into(), 3.0.into()]), "a flat list is a column");
    assert_eq!(value(run(&mut c, "[]")), Value::Error(CellError::Calc));
    match run(&mut c, "{'a': 1}") {
        Outcome::Ok { output: Output::Object { type_name, preview }, .. } => {
            assert_eq!(type_name, "dict");
            assert_eq!(preview, "{'a': 1}");
        }
        other => panic!("{other:?}"),
    }
    match run(&mut c, "list(range(1000))[0:0] or set(range(10**4))") {
        Outcome::Ok { output: Output::Object { preview, .. }, .. } => assert_eq!(preview.chars().count(), 255),
        other => panic!("{other:?}"),
    }

    // An error carries its line in the cell's code, and the message.
    assert_eq!(
        run(&mut c, "a = 1\nb = a + undefined_name\nb"),
        Outcome::PythonError {
            error: PyError {
                message: "NameError: name 'undefined_name' is not defined".into(),
                line: Some(2),
                kind: Some("python.name".into()),
                subject: Some("undefined_name".into())
            },
            stdout: String::new()
        }
    );
    match run(&mut c, "x = (\n1 +\n") {
        Outcome::PythonError { error, .. } => assert!(error.message.starts_with("SyntaxError"), "{error:?}"),
        other => panic!("{other:?}"),
    }
    match run(&mut c, "def f():\n    return 1 / 0\n\nf()") {
        Outcome::PythonError { error, .. } => {
            assert!(error.message.starts_with("ZeroDivisionError"), "{error:?}");
            assert_eq!(error.line, Some(2), "the line inside the function, in the cell's code");
        }
        other => panic!("{other:?}"),
    }
    match run(&mut c, "import sys\nsys.exit(3)") {
        Outcome::PythonError { error, .. } => assert_eq!(error.message, "SystemExit: 3"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn globals_last_until_a_reset() {
    let Some(py) = python() else { return };
    let mut c = client(py);
    // Globals persist across runs, until a reset.
    assert_eq!(value(run(&mut c, "shared = 41")), Value::Empty);
    assert_eq!(value(run(&mut c, "shared + 1")), Value::Number(42.0));
    assert_eq!(value(run(&mut c, "import __main__\n__main__.shared")), Value::Number(41.0), "the cells' globals are __main__");
    assert_eq!(value(c.reset().unwrap()), Value::Empty);
    match run(&mut c, "shared") {
        Outcome::PythonError { error, .. } => assert!(error.message.starts_with("NameError"), "{error:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.starts(), 1, "all on one worker");
}

#[test]
fn xl_reads_only_the_inputs_sent() {
    let Some(py) = python() else { return };
    let mut c = client(py);
    let mut inputs = BTreeMap::new();
    inputs.insert("A1".to_string(), Input::new(Value::Number(5.0)));
    inputs.insert("B1:C2".to_string(), Input::new(array(2, 2, vec![1.5.into(), Value::text("x"), Value::Empty, Value::Error(CellError::Div0)])));
    let code = "a = xl(\"A1\")\nr = xl(\"B1:C2\")\n[[a, type(a).__name__], [r[0][0], r[0][1]], [r[1][0], r[1][1]]]";
    assert_eq!(
        value(c.run(code, inputs.clone()).unwrap()),
        array(3, 2, vec![5.0.into(), Value::text("int"), 1.5.into(), Value::text("x"), Value::Empty, Value::Error(CellError::Div0)]),
        "a cell is a scalar, a range a list of rows, and an error cell is the error again"
    );
    assert_eq!(value(c.run("repr(xl('B1:C2')[1][1])", inputs.clone()).unwrap()), Value::text("#DIV/0!"));
    for bad in ["xl('Z9')", "xl('A' + '1') if False else xl('a1')", "xl(1)"] {
        match c.run(bad, inputs.clone()).unwrap() {
            Outcome::PythonError { error, .. } => assert!(error.message.contains("xl("), "{bad}: {error:?}"),
            other => panic!("{bad}: {other:?}"),
        }
    }
    // A function kept from an earlier cell can't read inputs outside a run.
    assert_eq!(value(c.run("def later():\n    return xl('A1')", inputs.clone()).unwrap()), Value::Empty);
    assert_eq!(value(c.run("later()", inputs).unwrap()), Value::Number(5.0));
}

#[test]
fn stdout_is_captured_and_the_protocol_is_out_of_reach() {
    let Some(py) = python() else { return };
    let mut c = client(py);
    match run(&mut c, "print('hello')\nprint('world')\n7") {
        Outcome::Ok { output: Output::Value(v), stdout } => {
            assert_eq!(v, Value::Number(7.0));
            assert_eq!(stdout, "hello\nworld\n");
        }
        other => panic!("{other:?}"),
    }
    // Printed output is cut at the cap, and the run still answers.
    match run(&mut c, "for i in range(300000): print('0123456789')\n1") {
        Outcome::Ok { stdout, .. } => {
            assert!(stdout.len() <= gridcraft_python::protocol::MAX_STDOUT_BYTES);
            assert!(stdout.ends_with("[output cut off]\n"));
        }
        other => panic!("{other:?}"),
    }
    // Writing a forged reply to standard output, at the Python or the file-descriptor level,
    // reaches standard error, not the protocol.
    let forged = r#"{"type":"done","id":99,"ok":true,"value":1}"#;
    assert_eq!(
        value(run(&mut c, &format!("import os, sys\nsys.__stdout__.write('{forged}\\n')\nsys.__stdout__.flush()\nos.write(1, b'{forged}\\n')\n5"))),
        Value::Number(5.0)
    );
    // input() finds an empty pipe, not the requests.
    match run(&mut c, "input()") {
        Outcome::PythonError { error, .. } => assert!(error.message.starts_with("EOFError"), "{error:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(value(run(&mut c, "import sys\nsys.stdin.read()")), Value::text(""));
    // The shim's names aren't in __main__ (after a reset, which drops the names set above).
    assert_eq!(value(c.reset().unwrap()), Value::Empty);
    assert_eq!(value(run(&mut c, "','.join(sorted(k for k in globals() if not k.startswith('__')))")), Value::text("np,pd,plt,sm,sns,xl"));
    assert_eq!(
        value(run(&mut c, "','.join(sorted(k for k in vars(__import__('sys').modules['__main__']) if not k.startswith('__')))")),
        Value::text("np,pd,plt,sm,sns,xl")
    );
    // The same worker still answers after all that.
    assert_eq!(value(run(&mut c, "2")), Value::Number(2.0));
    assert_eq!(c.starts(), 1);
}

/// The shim keeps its replies within the protocol's caps, so a big result is an error in the
/// cell, not a killed worker.
#[test]
fn big_results_stay_within_the_caps() {
    let Some(py) = python() else { return };
    let mut c = client(py);
    match value(run(&mut c, "[[0] * 1000 for _ in range(1000)]")) {
        Value::Array(a) => assert_eq!((a.rows, a.cols), (1000, 1000)),
        other => panic!("{other:?}"),
    }
    for (code, says) in [
        ("[[0] * 1001 for _ in range(1000)]", "at most 1000000 cells"),
        ("list(range(1000001))", "at most 1000000 cells"),
        ("'x' * (200 * 1024)", "A text result can have at most"),
        // 1,000,000 texts of 100 bytes: within every field's cap, but over the line's.
        ("['y' * 100] * 1000000", "too large to send"),
    ] {
        match run(&mut c, code) {
            Outcome::PythonError { error, .. } => assert!(error.message.contains(says), "{code}: {error:?}"),
            other => panic!("{code}: {other:?}"),
        }
    }
    let long_message = run(&mut c, "raise ValueError('m' * 100000)");
    match long_message {
        Outcome::PythonError { error, .. } => assert!(error.message.len() <= gridcraft_python::protocol::MAX_MESSAGE_BYTES),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.starts(), 1, "none of these killed the worker");
}

#[test]
fn the_interpreter_is_isolated() {
    let Some(py) = python() else { return };
    let mut c = client(py);
    // Only the variable the shim sets itself, a fresh empty working directory, no site module,
    // isolated flags. Python adds LC_CTYPE when it coerces the C locale to UTF-8, and macOS adds its
    // text encoding, both inside the worker.
    let env = value(run(&mut c, "import os\n','.join(sorted(k for k in os.environ if k not in ('LC_CTYPE', '__CF_USER_TEXT_ENCODING')))"));
    assert_eq!(env, Value::text("OPENBLAS_NUM_THREADS"));
    assert_eq!(value(run(&mut c, "import os\nos.listdir('.')")), Value::Error(CellError::Calc), "an empty list");
    assert_eq!(value(run(&mut c, "import sys\n'site' in sys.modules")), Value::Bool(false));
    assert_eq!(
        value(run(&mut c, "import sys\n[bool(f) for f in (sys.flags.isolated, sys.flags.no_site, sys.flags.ignore_environment)]")),
        array(3, 1, vec![Value::Bool(true); 3])
    );
    assert_eq!(value(run(&mut c, "import sys, sysconfig\nsysconfig.get_paths()['purelib'] in sys.path")), Value::Bool(true));
}

#[test]
fn deadlines_and_kills_with_a_real_worker() {
    let Some(py) = python() else { return };
    let mut c = Client::new(
        Box::new(ProcessTransport::new(py)),
        Limits { timeout: Duration::from_millis(1_500), start_timeout: Duration::from_secs(20), ..Limits::default() },
    );
    assert_eq!(value(run(&mut c, "warm = 1")), Value::Empty);
    let t = Instant::now();
    assert_eq!(run(&mut c, "while True: pass"), Outcome::TimedOut);
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    // A fresh worker, with fresh globals.
    match run(&mut c, "warm") {
        Outcome::PythonError { error, .. } => assert!(error.message.starts_with("NameError")),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.starts(), 2);
    // A crash (the process exits) is reported, and the next request starts again.
    assert!(matches!(run(&mut c, "import os\nos._exit(9)"), Outcome::Crashed(_)));
    assert_eq!(value(run(&mut c, "3")), Value::Number(3.0));
    assert_eq!(c.starts(), 3);
}

#[test]
fn the_worker_exits_when_its_input_closes() {
    let Some(py) = python() else { return };
    let mut t = ProcessTransport::new(py);
    let pipes = t.start().unwrap();
    drop(pipes.stdin);
    let mut out = Vec::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut stdout = pipes.stdout;
    std::thread::spawn(move || {
        let r = stdout.by_ref().take(4096).read_to_end(&mut out);
        let _ = tx.send((r.is_ok(), out));
    });
    let (ok, out) = rx.recv_timeout(Duration::from_secs(20)).expect("the worker ended at EOF");
    assert!(ok);
    assert!(String::from_utf8_lossy(&out).starts_with("{\"type\":\"ready\""), "{}", String::from_utf8_lossy(&out));
    t.kill();
}

#[test]
fn a_missing_interpreter_fails_to_start() {
    let mut c = Client::new(Box::new(ProcessTransport::new(PathBuf::from("/nonexistent/python3"))), Limits::default());
    let o = run(&mut c, "1");
    assert!(matches!(&o, Outcome::FailedToStart(_)), "{o:?}");
    assert_eq!(o.cell_error(), Some(CellError::Connect));
}

/// The worker says what kind of failure an error is.
#[test]
fn errors_say_what_kind_they_are() {
    let Some(p) = python() else { return };
    let mut c = client(p);
    let kind = |c: &mut Client, code: &str| match run(c, code) {
        Outcome::PythonError { error, .. } => (error.kind, error.subject),
        other => panic!("{code}: {other:?}"),
    };
    let k = |s: &str| Some(s.to_string());
    assert_eq!(kind(&mut c, "1 +"), (k("python.syntax"), None));
    assert_eq!(kind(&mut c, "import not_a_module_anywhere"), (k("module.missing"), k("not_a_module_anywhere")));
    assert_eq!(kind(&mut c, "not_a_name"), (k("python.name"), k("not_a_name")));
    assert_eq!(kind(&mut c, "1 / 0"), (k("python.exception"), None));
    assert_eq!(kind(&mut c, "r = 'A1'\nxl(r)"), (k("xl.notLiteral"), None));
    // A default name of Excel's initialization only matters once it is used.
    assert_eq!(value(run(&mut c, "x = 1\nx")), Value::Number(1.0));
    let fake = fake_pandas("lazy");
    let mut with = Client::new(Box::new(ProcessTransport::new(python().unwrap()).with_site_dirs(vec![fake.clone()])), Limits::default());
    assert_eq!(value(run(&mut with, "pd.DataFrame([[1]]).rows[0][0]")), Value::Number(1.0));
    std::fs::remove_dir_all(fake).unwrap();
    assert_eq!(kind(&mut c, "raise PermissionError(1, 'no', '/some/file')"), (k("denied.file"), k("/some/file")));
    assert_eq!(kind(&mut c, "import os\nraise PermissionError(1, 'no') # os.fork()"), (k("denied.process"), None));
    assert_eq!(kind(&mut c, "raise PermissionError(1, 'no') # socket"), (k("denied.network"), None));
    // Wrapped, as urllib and requests do.
    assert_eq!(
        kind(&mut c, "try:\n    raise PermissionError(1, 'no') # socket\nexcept OSError as e:\n    raise ValueError('wrapped') from e"),
        (k("denied.network"), None)
    );
}

/// A folder with a stand-in for pandas: what `xl()` gives it is the test, not pandas itself.
fn fake_pandas(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gridcraft-fake-pandas-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("pandas")).unwrap();
    std::fs::write(
        dir.join("pandas/__init__.py"),
        "class DataFrame:\n    def __init__(self, rows, columns=None):\n        self.rows = rows\n        self.columns = columns\n",
    )
    .unwrap();
    dir
}

#[test]
fn a_range_is_a_dataframe_when_pandas_is_there() {
    let Some(p) = python() else { return };
    let site = fake_pandas("frame");
    let transport = ProcessTransport::new(p).with_site_dirs(vec![site.clone()]);
    let mut c = Client::new(Box::new(transport), Limits::default());
    let range = Value::Array(std::sync::Arc::new(Array { rows: 2, cols: 1, data: vec![Value::text("name"), Value::Number(5.0)] }));
    let inputs = || BTreeMap::from([("A1:A2".to_string(), Input::new(range.clone()))]);
    let run_xl = |c: &mut Client, code: &str| match c.run(code, inputs()).unwrap() {
        Outcome::Ok { output: Output::Value(v), .. } => v,
        other => panic!("{code}: {other:?}"),
    };
    assert_eq!(run_xl(&mut c, "type(xl('A1:A2')).__name__"), Value::text("DataFrame"));
    assert_eq!(run_xl(&mut c, "xl('A1:A2').rows[1][0]"), Value::Number(5.0));
    assert_eq!(run_xl(&mut c, "xl('A1:A2', headers=True).columns[0]"), Value::text("name"));
    assert_eq!(run_xl(&mut c, "xl('A1:A2', headers=True).rows[0][0]"), Value::Number(5.0));
    std::fs::remove_dir_all(site).unwrap();
}

/// A list spills, unless the cell's return type is a Python object.
#[test]
fn the_return_type_decides_between_cells_and_an_object() {
    let Some(p) = python() else { return };
    let mut c = client(p);
    let id = c.submit_cell("[1, 2]", BTreeMap::new(), false).unwrap();
    let Some(RunResult { outcome: Outcome::Ok { output: Output::Value(Value::Array(a)), .. }, .. }) = c.wait().filter(|r| r.id == id) else {
        panic!("cells")
    };
    assert_eq!((a.rows, a.cols), (2, 1));
    let id = c.submit_cell("[1, 2]", BTreeMap::new(), true).unwrap();
    let Some(RunResult { outcome: Outcome::Ok { output: Output::Object { type_name, preview }, .. }, .. }) = c.wait().filter(|r| r.id == id) else {
        panic!("object")
    };
    assert_eq!((type_name.as_str(), preview.as_str()), ("list", "[1, 2]"));
    // A number stays a number either way.
    let id = c.submit_cell("3", BTreeMap::new(), true).unwrap();
    let Some(RunResult { outcome: Outcome::Ok { output: Output::Value(Value::Number(n)), .. }, .. }) = c.wait().filter(|r| r.id == id) else {
        panic!("number")
    };
    assert_eq!(n, 3.0);
}
