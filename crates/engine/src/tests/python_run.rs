//! Running Python cells: which run, in what order, what they write, and how failures, trust,
//! manual calculation, undo and background runs behave.

use std::sync::Arc;

use gridcraft_core::{Array, CellError, CellRef, Value};
use gridcraft_model::{Cell, PyCode};
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

/// Makes `cell` a Python cell with `code` (not yet run: it shows 0).
fn plant(s: &mut Session, cell: &str, code: &str) {
    let d = s.doc_mut().unwrap();
    let mut wb = (*d.wb).clone();
    let sh = wb.sheet_mut(0).unwrap();
    sh.set_cell(
        CellRef::parse(cell).unwrap(),
        Cell { value: Value::Number(0.0), formula: None, style: Default::default(), py: Some(Arc::new(PyCode::new(code))) },
    );
    d.wb = Arc::new(wb);
    d.calc.rebuild(&d.wb);
}

fn set(s: &mut Session, cell: &str, input: &str) {
    s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
}

fn err(e: CellError) -> Value {
    Value::Error(e)
}

#[test]
fn cells_run_in_row_major_order_sharing_one_namespace() {
    let mut s = py_session();
    plant(&mut s, "B1", "x = 10\nx");
    plant(&mut s, "B2", "x * 2");
    plant(&mut s, "A3", "x + 1");
    s.python_reset().unwrap();
    assert_eq!((v(&s, "B1"), v(&s, "B2"), v(&s, "A3")), (Value::Number(10.0), Value::Number(20.0), Value::Number(11.0)));
    let log = &s.doc().unwrap().py_log;
    let order: Vec<&str> = log.iter().map(|e| e.cell.as_str()).collect();
    assert_eq!(order, ["B1", "B2", "A3"]);
    assert!(log.iter().all(|e| e.status == "ok"));
}

#[test]
fn edits_rerun_the_cells_that_read_them_and_formulas_follow() {
    let mut s = py_session();
    plant(&mut s, "B1", "xl('A1') * 2");
    set(&mut s, "C1", "=B1+1");
    set(&mut s, "A1", "5");
    assert_eq!(v(&s, "B1"), Value::Number(10.0));
    assert_eq!(v(&s, "C1"), Value::Number(11.0));
    let runs = s.doc().unwrap().py_log.iter().map(|e| e.count as usize).sum::<usize>();
    // An edit nothing reads runs nothing.
    set(&mut s, "Z9", "1");
    assert_eq!(s.doc().unwrap().py_log.iter().map(|e| e.count as usize).sum::<usize>(), runs);
    set(&mut s, "A1", "7");
    assert_eq!(v(&s, "B1"), Value::Number(14.0));
    // The results aren't undo steps of their own: one undo takes back the edit, and the cell
    // shows what it showed before it.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(v(&s, "A1"), Value::Number(5.0));
    assert_eq!(v(&s, "B1"), Value::Number(10.0));
}

#[test]
fn python_to_formula_to_python_and_cycles() {
    let mut s = py_session();
    plant(&mut s, "A1", "5");
    set(&mut s, "B1", "=A1*2");
    plant(&mut s, "A2", "xl('B1') + 1");
    s.python_reset().unwrap();
    assert_eq!(v(&s, "B1"), Value::Number(10.0));
    assert_eq!(v(&s, "A2"), Value::Number(11.0));

    // A later Python cell feeding (through a formula) an earlier one is a cycle: entering the
    // formula runs A1 (reading the old A2), then A2, whose new result changes what A1 read.
    let mut s = py_session();
    plant(&mut s, "A1", "xl('B1')");
    plant(&mut s, "A2", "3");
    set(&mut s, "B1", "=A2");
    assert_eq!(v(&s, "A2"), Value::Number(3.0));
    assert_eq!(v(&s, "A1"), err(CellError::Calc));
    assert!(s.doc().unwrap().py_log.iter().any(|e| e.status == "cycle" && e.cell == "A1"));
    // Run again, the cells agree, so there is no cycle any more.
    s.python_reset().unwrap();
    assert_eq!(v(&s, "A1"), Value::Number(3.0));
}

#[test]
fn arrays_spill_and_spill_again_when_unblocked() {
    let mut s = py_session();
    plant(&mut s, "A1", "[1, 2, 3]");
    set(&mut s, "C1", "=SUM(A1:A3)");
    s.python_reset().unwrap();
    assert_eq!((v(&s, "A1"), v(&s, "A2"), v(&s, "A3")), (Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)));
    assert_eq!(v(&s, "C1"), Value::Number(6.0));
    let runs = s.doc().unwrap().py_log.iter().map(|e| e.count as usize).sum::<usize>();
    set(&mut s, "A2", "x");
    assert_eq!(v(&s, "A1"), err(CellError::Spill));
    s.execute("edit.clearContents", json!({"range": "A2"})).unwrap();
    assert_eq!(v(&s, "A2"), Value::Number(2.0), "clearing the blocker lets the kept result spill again");
    assert_eq!(v(&s, "C1"), Value::Number(6.0));
    assert_eq!(s.doc().unwrap().py_log.iter().map(|e| e.count as usize).sum::<usize>(), runs, "without running the code again");
    // A structural edit recalculates everything; the spill moves with its cell.
    s.execute("selection.set", json!({"cell": "A1"})).unwrap();
    s.execute("home.insertColumns", json!({})).unwrap();
    assert_eq!(v(&s, "B3"), Value::Number(3.0));
    assert_eq!(v(&s, "D1"), Value::Number(6.0));
    // A table result: rows and columns.
    plant(&mut s, "E1", "[[1, 'a'], [2, 'b']]");
    s.python_reset().unwrap();
    assert_eq!(v(&s, "F2"), Value::text("b"));
}

#[test]
fn errors_show_in_the_cell_and_in_the_diagnostics() {
    let mut s = py_session();
    plant(&mut s, "A1", "print('before')\nx = 1\n1 / 0");
    plant(&mut s, "A2", "'next cell runs'");
    s.python_reset().unwrap();
    assert_eq!(v(&s, "A1"), err(CellError::Python));
    assert_eq!(v(&s, "A2"), Value::text("next cell runs"));
    let e = s.doc().unwrap().py_log.iter().find(|e| e.cell == "A1").unwrap().clone();
    assert_eq!((e.status, e.line), ("error", Some(3)));
    assert!(e.error.unwrap().starts_with("ZeroDivisionError"));
    assert_eq!(e.stdout, "before\n");
    // An endless loop stops at the time limit; the next cell still runs.
    s.prefs.python_timeout_seconds = 0.3;
    plant(&mut s, "A1", "while True: pass");
    s.python_reset().unwrap();
    assert_eq!(v(&s, "A1"), err(CellError::Timeout));
    assert_eq!(v(&s, "A2"), Value::text("next cell runs"));
}

#[test]
fn xl_reads_ranges_names_and_other_sheets() {
    let mut s = py_session();
    s.execute("range.setValues", json!({"range": "A1", "values": [["n", "v"], ["a", 1], ["b", 2]]})).unwrap();
    s.execute("formulas.defineName", json!({"name": "Data", "refersTo": "=Sheet1!$A$1:$B$3"})).unwrap();
    plant(&mut s, "D1", "sum(r['v'] for r in xl('A1:B3', headers=True))");
    plant(&mut s, "D2", "len(xl('Data'))");
    plant(&mut s, "D3", "xl('Sheet1!B2')");
    plant(&mut s, "D4", "xl('nope!A1')");
    s.python_reset().unwrap();
    assert_eq!(v(&s, "D1"), Value::Number(3.0));
    assert_eq!(v(&s, "D2"), Value::Number(3.0));
    assert_eq!(v(&s, "D3"), Value::Number(1.0));
    assert_eq!(v(&s, "D4"), err(CellError::Python));
    // A name's cells changing reruns the cell that reads the name.
    set(&mut s, "B3", "40");
    assert_eq!(v(&s, "D1"), Value::Number(41.0));
}

#[test]
fn untrusted_workbooks_and_manual_calculation_leave_cells_as_they_are() {
    let mut s = py_session();
    plant(&mut s, "B1", "xl('A1') * 2");
    s.doc_mut().unwrap().trust = Trust::FromFile;
    set(&mut s, "A1", "5");
    assert_eq!(v(&s, "B1"), Value::Number(0.0), "nothing runs until Python is enabled");
    assert!(s.doc().unwrap().py_log.is_empty());

    let mut s = py_session();
    plant(&mut s, "B1", "xl('A1') * 2");
    s.execute("formulas.calculationOptions", json!({"mode": "manual"})).unwrap();
    set(&mut s, "A1", "5");
    assert_eq!(v(&s, "B1"), Value::Number(0.0));
    assert_eq!(s.execute("cell.get", json!({"cell": "B1"})).unwrap()["python"]["stale"], true);
    s.execute("formulas.calculateNow", json!({})).unwrap();
    assert_eq!(v(&s, "B1"), Value::Number(10.0));
    assert_eq!(s.execute("cell.get", json!({"cell": "B1"})).unwrap()["python"]["stale"], false);
}

#[test]
fn undo_resets_the_namespace() {
    let mut s = py_session();
    plant(&mut s, "B1", "try:\n    n = n + 1\nexcept NameError:\n    n = 1\nn + xl('A1') * 0");
    set(&mut s, "A1", "1");
    assert_eq!(v(&s, "B1"), Value::Number(1.0));
    set(&mut s, "A1", "2");
    assert_eq!(v(&s, "B1"), Value::Number(2.0), "the namespace carries `n` from run to run");
    // Undo changes A1 back, so B1 runs again, in a fresh namespace.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(v(&s, "B1"), Value::Number(1.0), "undo reset the namespace before running");
    set(&mut s, "A1", "3");
    assert_eq!(v(&s, "B1"), Value::Number(2.0));
}

#[test]
fn background_runs_show_busy_and_restart_on_edits() {
    let mut s = py_session();
    s.set_python_background(true);
    plant(&mut s, "B1", "xl('A1') * 2");
    set(&mut s, "A1", "5");
    assert_eq!(v(&s, "B1"), err(CellError::Busy));
    assert!(s.python_busy());
    // Another edit while it runs restarts the refresh.
    set(&mut s, "A1", "6");
    for _ in 0..2000 {
        if !s.step_python() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!s.python_busy());
    assert_eq!(v(&s, "B1"), Value::Number(12.0));
    // Solver's job slot is untouched.
    assert!(s.job.is_none());
}

#[test]
fn objects_and_values() {
    let mut s = py_session();
    plant(&mut s, "A1", "{1, 2}");
    plant(&mut s, "A2", "[1, 2]");
    s.python_reset().unwrap();
    assert_eq!(v(&s, "A1"), Value::text("{1, 2}"), "a result with no cell form shows its preview");
    let got = s.execute("cell.get", json!({"cell": "A2"})).unwrap();
    assert_eq!(got["python"]["typeName"], "list");
    assert_eq!(v(&s, "A3"), Value::Number(2.0));
    let arr = Array::new(2, 1, vec![1.0.into(), 2.0.into()]).map(Value::from).unwrap();
    let d = s.doc().unwrap();
    let py = d.wb.sheet(0).unwrap().cell(CellRef::parse("A2").unwrap()).unwrap().py.clone().unwrap();
    assert_eq!(py.output.as_ref().unwrap().value, arr);
}

/// This test binary started again as a real worker process (see `worker_process_entry`).
fn process_launcher() -> Launcher {
    let program = std::env::current_exe().unwrap();
    let args = ["tests::python_run::worker_process_entry", "--exact", "--nocapture", "--test-threads=1"].map(String::from).to_vec();
    Launcher::Process { program, args, env: vec![("GRIDCRAFT_ENGINE_PYTHON_WORKER".into(), "1".into())] }
}

#[test]
fn worker_process_entry() {
    if std::env::var_os("GRIDCRAFT_ENGINE_PYTHON_WORKER").is_some() {
        gridcraft_python::worker::serve(std::io::BufReader::new(std::io::stdin()), std::io::stdout(), gridcraft_python::worker::OnClose::ExitProcess);
        std::process::exit(0);
    }
}

#[test]
fn a_killed_worker_is_replaced_and_earlier_cells_run_again() {
    let mut s = py_session();
    s.set_python_launcher(process_launcher());
    s.prefs.python_timeout_seconds = 0.5;
    plant(&mut s, "A1", "base = 7\nbase");
    plant(&mut s, "A2", "10 ** 10 ** 9");
    plant(&mut s, "A3", "base + 1");
    s.python_reset().unwrap();
    assert_eq!(v(&s, "A1"), Value::Number(7.0));
    assert_eq!(v(&s, "A2"), err(CellError::Timeout));
    assert_eq!(v(&s, "A3"), Value::Number(8.0), "A1 ran again in the new worker before A3");
    // A1's two runs (before A2, then again in the new worker) share a row with a count.
    let cells: Vec<String> = s.doc().unwrap().py_log.iter().map(|e| format!("{}:{}x{}", e.cell, e.status, e.count)).collect();
    assert_eq!(cells, ["A2:timeoutx1", "A1:okx2", "A3:okx1"]);
}

#[test]
fn stop_kills_a_cell_that_loops_forever_and_nothing_reruns_until_an_edit() {
    let mut s = py_session();
    // A real process, so the kill really ends the endless loop.
    s.set_python_launcher(process_launcher());
    s.set_python_background(true);
    s.prefs.python_timeout_seconds = 60.0;
    plant(&mut s, "A1", "base = 1\nbase");
    plant(&mut s, "A2", "while True:\n    pass");
    plant(&mut s, "A3", "base + 1");
    s.python_reset().unwrap();
    // A1 finishes; A2 then runs until stopped.
    for _ in 0..2000 {
        s.step_python();
        if v(&s, "A1") == Value::Number(1.0) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    for _ in 0..20 {
        s.step_python();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(v(&s, "A1"), Value::Number(1.0));
    assert_eq!(v(&s, "A2"), err(CellError::Busy));
    assert_eq!(v(&s, "A3"), err(CellError::Busy));
    let r = s.execute("python.stop", json!({})).unwrap();
    assert_eq!(r["stopped"], 2);
    assert!(!s.python_busy());
    assert_eq!(v(&s, "A1"), Value::Number(1.0), "a finished cell keeps its value");
    assert_eq!(v(&s, "A2"), err(CellError::Python));
    assert_eq!(v(&s, "A3"), err(CellError::Python));
    let log: Vec<(String, &str, Option<String>)> = s.doc().unwrap().py_log.iter().map(|e| (e.cell.clone(), e.status, e.error.clone())).collect();
    assert!(log.ends_with(&[("A2".into(), "stopped", Some("Stopped".into())), ("A3".into(), "stopped", Some("Stopped".into()))]), "{log:?}");
    // Nothing reruns by itself, and stopping again does nothing.
    for _ in 0..20 {
        assert!(!s.step_python());
    }
    assert_eq!(s.execute("python.stop", json!({})).unwrap()["stopped"], 0);
    assert_eq!(v(&s, "A3"), err(CellError::Python));
    // An edit runs again from a fresh namespace: A1 first, so A3 still finds `base`.
    s.execute("python.set", json!({"cell": "A2", "code": "2"})).unwrap();
    for _ in 0..2000 {
        if !s.step_python() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(v(&s, "A2"), Value::Number(2.0));
    assert_eq!(v(&s, "A3"), Value::Number(2.0));
}

#[test]
fn stop_without_python_running_is_harmless() {
    let mut s = py_session();
    assert_eq!(s.execute("python.stop", json!({})).unwrap()["stopped"], 0);
}

#[test]
fn background_edits_during_the_first_run_of_a_worker_process() {
    let mut s = py_session();
    s.set_python_launcher(process_launcher());
    s.set_python_background(true);
    s.execute("python.set", json!({"cell": "A1", "code": "1"})).unwrap();
    s.step_python();
    s.execute("python.set", json!({"cell": "A3", "code": "3"})).unwrap();
    s.step_python();
    s.execute("python.set", json!({"cell": "A5", "code": "5"})).unwrap();
    for _ in 0..3000 {
        if !s.step_python() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let log: Vec<(String, &str, Option<String>)> = s.doc().unwrap().py_log.iter().map(|e| (e.cell.clone(), e.status, e.error.clone())).collect();
    assert_eq!(v(&s, "A1"), Value::Number(1.0), "{log:?}");
    assert_eq!(v(&s, "A3"), Value::Number(3.0), "{log:?}");
    assert_eq!(v(&s, "A5"), Value::Number(5.0), "{log:?}");
}

/// Cells on this machine's own Python (3.11 or newer), found by the search, when there is one.
#[test]
fn cells_run_on_the_users_python_when_the_search_finds_one() {
    let detection = gridcraft_python::interp::detect(&std::env::var("GRIDCRAFT_TEST_PYTHON").unwrap_or_default(), None);
    if detection.found.is_none() {
        eprintln!("skipped: no Python 3.11 or newer here ({:?})", detection.rejected);
        return;
    }
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_detection(detection);
    assert_eq!(s.python_backend()["kind"], "cpython");
    s.execute("range.setValues", json!({"range": "A1:B3", "values": [["k", "v"], ["a", 1], ["b", 2]]})).unwrap();
    s.execute("python.initialization", json!({"code": "import sys\nscale = 10"})).unwrap();
    s.execute("python.set", json!({"cell": "D1", "code": "rows = xl('A1:B3')\n# A frame when pandas is there.\nrows = rows.values.tolist() if hasattr(rows, 'values') else rows\nsys.version_info[0]"})).unwrap();
    assert_eq!(v(&s, "D1"), Value::Number(3.0));
    s.execute("python.set", json!({"cell": "D2", "code": "[r[1] * scale for r in rows[1:]]"})).unwrap();
    assert_eq!(v(&s, "D2"), Value::Number(10.0));
    assert_eq!(v(&s, "D3"), Value::Number(20.0), "the list spills");
    // An edit reruns the cells that read it, on the same Python.
    s.execute("cell.set", json!({"cell": "B3", "input": "5"})).unwrap();
    assert_eq!(v(&s, "D3"), Value::Number(50.0));
    let log = s.execute("python.diagnostics", json!({})).unwrap();
    assert!(log["runs"].as_array().unwrap().iter().all(|r| r["status"] == "ok"), "{log}");
}

/// The crash-loop guard: a cell that stopped the worker doesn't run again (and can't stop it
/// again) until its code or the cells it reads change, or Reset Runtime.
#[test]
fn a_cell_that_stops_the_worker_is_not_rerun_until_it_changes() {
    let mut s = py_session();
    s.set_python_launcher(process_launcher());
    s.prefs.python_timeout_seconds = 0.5;
    s.execute("cell.set", json!({"cell": "C1", "input": "9"})).unwrap();
    s.execute("cell.set", json!({"cell": "C2", "input": "1"})).unwrap();
    plant(&mut s, "A1", "base = xl('C2')\nbase");
    plant(&mut s, "A2", "xl('C1') + 10 ** 10 ** 9");
    plant(&mut s, "A3", "base + 1");
    s.python_reset().unwrap();
    assert_eq!(v(&s, "A2"), err(CellError::Timeout));
    let starts = |s: &Session| s.doc().unwrap().py_log.iter().map(|e| e.count as usize).sum::<usize>();
    // An edit before it reruns A1 and the cells after it, but not A2: no kill, no restart.
    let before = starts(&s);
    s.execute("cell.set", json!({"cell": "C2", "input": "2"})).unwrap();
    assert_eq!(v(&s, "A1"), Value::Number(2.0));
    assert_eq!(v(&s, "A2"), err(CellError::Python));
    assert_eq!(v(&s, "A3"), Value::Number(3.0));
    let log = s.execute("python.diagnostics", json!({})).unwrap();
    let runs = log["runs"].as_array().unwrap();
    let a2 = runs.iter().rev().find(|r| r["cell"] == "A2").unwrap();
    assert_eq!(a2["status"], "refused");
    assert_eq!(a2["error"], gridcraft_python::client::REFUSED_MESSAGE);
    assert_eq!(starts(&s) - before, 3, "A1, A2 (refused) and A3; nothing ran twice");
    // Changing what it reads runs it again.
    s.execute("cell.set", json!({"cell": "C1", "input": "8"})).unwrap();
    assert_eq!(v(&s, "A2"), err(CellError::Timeout));
    // So does Reset Runtime.
    s.execute("cell.set", json!({"cell": "C2", "input": "3"})).unwrap();
    assert_eq!(v(&s, "A2"), err(CellError::Python), "refused again");
    s.python_reset().unwrap();
    assert_eq!(v(&s, "A2"), err(CellError::Timeout));
    // Changing its code too.
    s.execute("python.set", json!({"cell": "A2", "code": "xl('C1') * 2"})).unwrap();
    assert_eq!(v(&s, "A2"), Value::Number(16.0));
}
