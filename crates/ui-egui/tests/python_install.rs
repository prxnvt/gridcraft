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
    let mut s = Session::new();
    s.new_workbook();
    s.prefs.python_environment = env.display().to_string();
    let found = Interpreter {
        program: "/nonexistent/gc/python3".into(),
        version: "3.14.0".into(),
        externally_managed: true,
        source: "PATH".into(),
        ..Interpreter::default()
    };
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
    d.py_log.push_back(PyRunLog {
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
    });
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
    assert!(h.query_by_label_contains("GridCraft couldn't make its Python environment").is_some());
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
