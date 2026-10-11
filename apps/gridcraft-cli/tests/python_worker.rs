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

/// The whole path a headless caller takes: commands through the CLI, Python in a real worker
/// process (this binary with --python-worker), results in the cells.
#[test]
fn cli_commands_run_python_cells_and_contain_hostile_code() {
    // Never GridCraft's real Python environment: a folder that doesn't exist.
    let env = std::env::temp_dir().join(format!("gridcraft-cli-test-no-env-{}", std::process::id()));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_gridcraft-cli"))
        .env("GRIDCRAFT_PYTHON_ENV", &env)
        .args([
            "run",
            "--cmd",
            r#"cell.set={"cell":"A1","input":"21"}"#,
            "--cmd",
            r#"python.set={"cell":"B1","code":"xl('A1') * 2"}"#,
            "--cmd",
            r#"python.options={"timeoutSeconds":1}"#,
            "--cmd",
            r#"python.set={"cell":"B2","code":"while True: pass"}"#,
            "--cmd",
            r#"python.set={"cell":"B3","code":"'x' * 10**12"}"#,
            "--cmd",
            r#"python.set={"cell":"B4","code":"'still running'"}"#,
            "--print",
            "B1:B4",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines.get(lines.len().saturating_sub(4)..).unwrap_or_default().join("|");
    assert!(tail.contains("42"), "{stdout}");
    assert!(tail.contains("#TIMEOUT!"), "{stdout}");
    assert!(tail.contains("#PYTHON!"), "{stdout}");
    assert!(tail.contains("still running"), "{stdout}");
}
