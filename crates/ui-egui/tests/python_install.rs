//! Installing Python packages from the app: the offer on a missing module, the approval dialog,
//! the status bar while it runs and after, and GridCraft's environment in Python Options.

use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellError, CellRef, Value};
use gridcraft_engine::python::PyRunLog;
use gridcraft_python::interp::{Detection, Interpreter};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gridcraft-ui-pyenv-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// An app whose cells "run" on a Python that doesn't exist (so nothing really runs or installs),
/// with GridCraft's environment in `env`.
fn harness(env: &std::path::Path) -> Harness {
    harness_with(env, "/nonexistent/gc/python3")
}

fn harness_with(env: &std::path::Path, program: &str) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.prefs.python_environment = env.display().to_string();
    let found =
        Interpreter { program: program.into(), version: "3.14.0".into(), externally_managed: true, source: "PATH".into(), ..Interpreter::default() };
    s.set_python_detection(Detection { found: Some(found), rejected: vec![] });
    s.set_python_uv(false);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 860.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h
}

/// A1 failed importing scikit-learn's module, as a run on CPython reports it.
fn missing_sklearn(h: &mut Harness) {
    let d = h.state_mut().session.active_mut().unwrap();
    let mut wb = (*d.wb).clone();
    let sh = wb.sheet_mut(0).unwrap();
    sh.set_cell(
        CellRef::new(0, 0),
        gridcraft_engine::model::Cell {
            value: Value::Error(CellError::Python),
            py: Some(std::sync::Arc::new(gridcraft_engine::model::PyCode::new("import sklearn"))),
            ..Default::default()
        },
    );
    d.wb = std::sync::Arc::new(wb);
    let sheet = d.wb.sheets[0].name.clone();
    gridcraft_engine::python::push_log(
        d,
        PyRunLog {
            seq: 1,
            sheet,
            cell: "A1".into(),
            status: "error",
            ms: 1.0,
            stdout: String::new(),
            stderr: String::new(),
            error: Some("ModuleNotFoundError: No module named 'sklearn'".into()),
            line: Some(1),
            missing: Some("scikit-learn".into()),
            count: 1,
            time_ms: 0.0,
            ..Default::default()
        },
    );
    h.run_steps(2);
}

fn wait_idle(h: &mut Harness) {
    for _ in 0..2000 {
        h.run_steps(1);
        if !h.state().session.python_busy() {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the install never finished");
}

#[test]
fn a_missing_module_offers_its_package_and_the_dialog_asks_first() {
    let env = scratch("offer");
    let mut h = harness(&env);
    missing_sklearn(&mut h);
    // The preview card offers the pip package.
    h.state_mut().grid.py_card = Some((0, CellRef::new(0, 0)));
    h.run_steps(2);
    h.get_by_label("Install scikit-learn…").click();
    h.run_steps(3);
    let d = h.state().dialog.clone().expect("the install dialog");
    assert_eq!(d.name, "pythonInstall");
    assert_eq!(d.values["packages"], "scikit-learn");
    assert_eq!(d.values["target"], env.display().to_string());
    assert!(h.query_by_label("GridCraft's Python environment").is_some());
    assert!(h.query_by_label("GridCraft makes this environment first, from the Python it found.").is_some());
    assert!(h.query_by_label_contains("runs their code on this computer").is_some(), "the warning");
    // A list it can't install is refused right there.
    h.state_mut().dialog.as_mut().unwrap().values.insert("packages".into(), json!("scikit-learn, https://evil.test/x.whl"));
    h.run_steps(2);
    assert!(h.query_by_label_contains("is a URL").is_some());
    h.get_by_label(" Cancel ").click();
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert!(!h.state().session.python_installing(), "nothing installed without approval");
    // The Diagnostics row offers it too.
    h.state_mut().grid.pane = Some(gridcraft_ui_egui::python_ui::PANE.into());
    h.run_steps(3);
    h.get_by_label("Install scikit-learn…").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("pythonInstall"));
    assert_eq!(h.state().dialog.as_ref().unwrap().values["declare"], true);
    let _ = std::fs::remove_dir_all(&env);
}

#[test]
fn installing_shows_progress_then_the_result_and_logs_it() {
    let env = scratch("run");
    let mut h = harness(&env);
    missing_sklearn(&mut h);
    h.state_mut().open_dialog("pythonInstall", json!({"packages": ["scikit-learn"], "declare": true}));
    h.run_steps(2);
    h.get_by_label("  Install  ").click();
    h.run_steps(1);
    wait_idle(&mut h);
    // The (fake) Python can't make the environment: the status bar and Diagnostics say so.
    let status = h.state().session.python_install_status();
    assert_eq!(status["state"], "installFailed", "{status}");
    assert!(h.query_all_by_label_contains("GridCraft couldn't make its Python environment").next().is_some());
    assert_eq!(
        h.state_mut().run("python.packages", json!({})).unwrap()["packages"],
        json!(["scikit-learn"]),
        "approving declared the package for the workbook"
    );
    h.state_mut().grid.pane = Some(gridcraft_ui_egui::python_ui::PANE.into());
    h.run_steps(3);
    assert!(h.query_by_label("Package install").is_some());
    assert!(h.query_by_label("Install failed").is_some());
    let _ = std::fs::remove_dir_all(&env);
}

#[test]
fn python_options_shows_gridcrafts_environment_and_the_workbooks_packages() {
    let env = scratch("options");
    let mut h = harness(&env);
    h.state_mut().run("python.packages", json!({"packages": ["pandas", "numpy"]})).unwrap();
    h.state_mut().open_dialog("pythonOptions", json!({}));
    h.run_steps(3);
    assert!(h.query_by_label("GridCraft's Python environment").is_some());
    assert!(h.query_by_label(&env.display().to_string()).is_some());
    assert!(h.query_by_label_contains("Not made yet.").is_some());
    assert!(h.query_by_label("Create").is_some());
    assert!(h.query_by_label("Delete environment").is_none(), "nothing to delete");
    assert_eq!(h.state().dialog.as_ref().unwrap().values["packages"], "pandas, numpy");
    // Editing the list and pressing OK saves it with the workbook.
    h.state_mut().dialog.as_mut().unwrap().values.insert("packages".into(), json!("pandas, polars"));
    h.run_steps(1);
    h.get_by_label("   OK   ").click();
    h.run_steps(2);
    assert_eq!(h.state_mut().run("python.packages", json!({})).unwrap()["packages"], json!(["pandas", "polars"]));
    // Install packages… opens the install dialog in place of Options.
    h.state_mut().open_dialog("pythonOptions", json!({}));
    h.run_steps(2);
    h.get_by_label("Install packages…").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("pythonInstall"));
    let _ = std::fs::remove_dir_all(&env);
}

/// An install that already ended this way, followed by the app as it follows its own.
fn ended(h: &mut Harness, ok: bool, output: &str, message: &str) {
    let outcome =
        gridcraft_engine::pyenv::env::InstallOutcome { ok, message: message.into(), output: output.into(), cancelled: false, warning: None };
    h.state_mut().session.python_track_install(
        vec!["scikit-learn".into()],
        "/scratch/gc-env".into(),
        None,
        gridcraft_engine::pyenv::env::Install::finished(outcome),
    );
    h.run_steps(3);
}

#[test]
fn an_install_that_worked_says_installed_with_the_version() {
    let env = scratch("toast-ok");
    let mut h = harness(&env);
    ended(&mut h, true, "Resolved 7 packages\n + scikit-learn==1.9.1\n", "Installed scikit-learn.");
    assert!(h.query_all_by_label("Installed scikit-learn 1.9.1").next().is_some(), "the toast");
    assert!(h.query_by_label("See Diagnostics").is_none(), "nothing to look into");
    let toast = h.state().toast.clone().unwrap();
    // It stays for a while, then goes.
    h.state_mut().toast = Some((toast.0, toast.1 - 7_000.0));
    if let Some(e) = h.state_mut().toast_extra.as_mut() {
        e.at -= 7_000.0;
    }
    h.run_steps(2);
    assert!(h.state().toast.is_none());
    let _ = std::fs::remove_dir_all(&env);
}

#[test]
fn a_failed_install_toasts_the_reason_with_see_diagnostics() {
    let env = scratch("toast-fail");
    let mut h = harness(&env);
    ended(&mut h, false, "ERROR: No matching distribution found for scikit-learn\n", "No ready-made package of scikit-learn.");
    assert!(h.query_all_by_label("Couldn't install scikit-learn: No ready-made package of scikit-learn.").next().is_some());
    h.get_by_label("See Diagnostics").click();
    h.run_steps(3);
    assert_eq!(h.state().grid.pane.as_deref(), Some(gridcraft_ui_egui::python_ui::PANE));
    assert!(h.state().toast.is_none(), "the toast goes once used");
    assert!(h.query_by_label("Install failed").is_some());
    let _ = std::fs::remove_dir_all(&env);
}

/// Diagnostics shows the install whatever the filters, with where it went, the cells that still
/// fail for another reason, and the installer's output folded away.
#[test]
fn diagnostics_always_shows_the_install_row() {
    let env = scratch("row");
    let mut h = harness(&env);
    missing_sklearn(&mut h);
    ended(&mut h, true, " + scikit-learn==1.9.1\n", "Installed scikit-learn.");
    // A1 ran again and now fails for another reason (as the person's FileNotFoundError did).
    {
        let d = h.state_mut().session.active_mut().unwrap();
        let sheet = d.wb.sheets[0].name.clone();
        gridcraft_engine::python::push_log(
            d,
            PyRunLog {
                sheet,
                cell: "A1".into(),
                status: "error",
                error: Some("FileNotFoundError: [Errno 2] No such file or directory: 'python_test.xlsx'".into()),
                ..Default::default()
            },
        );
    }
    h.state_mut().run("home.insertSheet", json!({})).unwrap();
    h.state_mut().grid.pane = Some(gridcraft_ui_egui::python_ui::PANE.into());
    h.run_steps(3);
    h.get_by_label("This sheet").click();
    h.get_by_label("Errors only").click();
    h.run_steps(3);
    assert!(h.query_by_label("Package install").is_some(), "kept whatever the sheet");
    assert!(h.query_by_label("Installed").is_some());
    assert!(h.query_all_by_label("Installed scikit-learn 1.9.1").next().is_some());
    assert!(h.query_by_label("Into /scratch/gc-env").is_some());
    assert!(h.query_by_label_contains("still fail for another reason").is_none(), "A1's run is on the other sheet, filtered out");
    // On A1's sheet the row says the install worked though A1 still fails.
    h.get_by_label("This sheet").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("still fails for another reason: see their rows.").is_some(), "the install worked; A1 fails otherwise");
    assert!(h.query_by_label("Install scikit-learn…").is_none(), "not offered again once installed");
    // The installer's output is folded until asked for.
    assert!(h.query_by_label_contains("+ scikit-learn==1.9.1").is_none());
    h.get_by_label("Installer output").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("+ scikit-learn==1.9.1").is_some());
    let _ = std::fs::remove_dir_all(&env);
}

/// While an install runs the status bar says what it installs, with Cancel.
#[cfg(unix)]
#[test]
fn the_status_bar_shows_a_running_install_with_cancel() {
    use std::os::unix::fs::PermissionsExt;
    let env = scratch("slow");
    let tools = scratch("slow-tools");
    std::fs::create_dir_all(&tools).unwrap();
    let python = tools.join("python3");
    std::fs::write(&python, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut h = harness_with(&env, &python.display().to_string());
    h.state_mut().run("python.install", json!({"packages": ["scikit-learn"], "approve": true})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Installing scikit-learn…").is_some());
    // The formula bar has a Cancel button too: the status bar's is the last.
    h.get_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(3);
    wait_idle(&mut h);
    assert_eq!(h.state().session.python_install_status()["state"], "cancelled");
    assert!(h.query_all_by_label_contains("Cancelled installing scikit-learn").next().is_some());
    let _ = std::fs::remove_dir_all(&env);
    let _ = std::fs::remove_dir_all(&tools);
}

/// After the install, the warm-up (importing the package once) shows as "Preparing …"; stopped
/// or failed, the install still worked, with the warning in its Diagnostics row.
#[cfg(unix)]
#[test]
fn the_status_bar_shows_the_warm_up_and_its_warning_stays_in_diagnostics() {
    use std::os::unix::fs::PermissionsExt;
    let env = scratch("warm");
    // GridCraft's environment "exists": its python installs at once and imports slowly.
    std::fs::create_dir_all(env.join("bin")).unwrap();
    std::fs::write(env.join("pyvenv.cfg"), "home = /nowhere\n").unwrap();
    let python = env.join("bin").join("python");
    std::fs::write(&python, "#!/bin/sh\ncase \"$1\" in\n  -c) sleep 30 ;;\n  *) echo ' + scikit-learn==1.9.1' ;;\nesac\n").unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut h = harness_with(&env, "/nonexistent/gc/python3");
    h.state_mut().run("python.install", json!({"packages": ["scikit-learn"], "approve": true})).unwrap();
    let mut seen = false;
    for _ in 0..300 {
        h.run_steps(1);
        if h.query_by_label("Preparing scikit-learn\u{2026}").is_some() {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(seen, "the warm-up shows in the status bar");
    h.get_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(3);
    wait_idle(&mut h);
    let status = h.state().session.python_install_status();
    assert_eq!(status["state"], "installed", "{status}");
    assert!(status["warning"].as_str().is_some_and(|w| w.contains("was stopped")), "{status}");
    h.state_mut().grid.pane = Some(gridcraft_ui_egui::python_ui::PANE.into());
    h.run_steps(3);
    assert!(h.query_by_label_contains("preparing it for its first import was stopped").is_some(), "the warning in the row");
    let _ = std::fs::remove_dir_all(&env);
}
