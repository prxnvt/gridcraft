//! The CLI binary is the Python worker when started with `--python-worker`: the parent drives it
//! through the protocol and kills it when the code misbehaves.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gridcraft_core::Value;
use gridcraft_python::XlReply;
use gridcraft_python::client::{FailureKind, Launcher, PyWorker};

fn worker() -> PyWorker {
    let launcher =
        Launcher::Process { program: env!("CARGO_BIN_EXE_gridcraft-cli").into(), args: vec![gridcraft_python::WORKER_FLAG.into()], env: vec![] };
    let limits = gridcraft_python::Limits { timeout_ms: 1_000, ..Default::default() };
    PyWorker::new(launcher, limits, "greeting = 'hello'".into())
}

#[test]
fn the_binary_serves_python_and_survives_hostile_code() {
    let mut w = worker();
    let mut cells = |r: &str, _: bool| if r == "B2" { XlReply::Value(Value::Number(4.0)) } else { XlReply::Error("no".into()) };
    let o = w.run_blocking("print(greeting)\nxl('B2') ** 0.5", &mut cells);
    assert_eq!(o.result.as_ref().map(|o| o.value.clone()), Ok(Value::Number(2.0)), "{o:?}");
    assert_eq!(o.stdout, "hello\n");
    let o = w.run_blocking("while True: pass", &mut cells);
    assert_eq!(o.result.as_ref().err().map(|f| f.kind), Some(FailureKind::Timeout), "{o:?}");
    let o = w.run_blocking("'x' * 10**12", &mut cells);
    assert!(o.result.is_err(), "{o:?}");
    let o = w.run_blocking("greeting.upper()", &mut cells);
    assert_eq!(o.result.as_ref().map(|o| o.value.clone()), Ok(Value::text("HELLO")), "{o:?}");
}
