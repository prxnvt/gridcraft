//! A workbook's declared packages, GridCraft's own Python environment, and installs the person
//! approves (python.packages, python.install, python.environment).

use std::path::PathBuf;

use gridcraft_python::client::Launcher;
use gridcraft_python::interp::{Detection, Interpreter};
use serde_json::json;

use crate::{EngineError, Session, Trust};

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_launcher(Launcher::InProcess);
    s
}

/// A folder for an environment of this test's own (never the real one).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gridcraft-pyenv-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A session whose cells "run" on `found`, with GridCraft's environment at `env`.
fn on(found: Interpreter, env: &std::path::Path) -> Session {
    let mut s = session();
    s.prefs.python_environment = env.display().to_string();
    s.set_python_detection(Detection { found: Some(found), rejected: vec![] });
    s
}

fn system_python() -> Interpreter {
    Interpreter {
        program: "/opt/homebrew/bin/python3".into(),
        version: "3.14.0".into(),
        externally_managed: true,
        source: "PATH".into(),
        ..Interpreter::default()
    }
}

fn approve_question(r: crate::Result<serde_json::Value>) -> String {
    match r {
        Err(EngineError::Approve(q)) => q,
        other => panic!("expected a question: {other:?}"),
    }
}

#[test]
fn a_workbook_declares_its_packages_and_keeps_them_in_files() {
    let mut s = session();
    assert_eq!(s.execute("python.packages", json!({})).unwrap(), json!({"packages": []}));
    let r = s.execute("python.packages", json!({"packages": ["pandas", " numpy >= 1.26 ", "sklearn", "Pandas"]})).unwrap();
    assert_eq!(r["packages"], json!(["pandas", "numpy>=1.26", "scikit-learn"]), "tidied, pip names, no duplicates");
    for bad in [json!(["https://x.test/p.whl"]), json!(["-e ."]), json!(["numpy; os_name=='nt'"]), json!("pandas"), json!([1])] {
        assert!(s.execute("python.packages", json!({"packages": bad})).is_err(), "{bad}");
    }
    let many: Vec<String> = (0..33).map(|i| format!("p{i}")).collect();
    assert!(s.execute("python.packages", json!({"packages": many})).is_err());
    // Undo takes the list back.
    s.execute("python.packages", json!({"packages": ["polars"]})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.execute("python.packages", json!({})).unwrap()["packages"], json!(["pandas", "numpy>=1.26", "scikit-learn"]));
    // xlsx and native JSON keep them.
    for format in ["xlsx", "json"] {
        let r = s.execute("file.saveBytes", json!({"format": format})).unwrap();
        let mut t = Session::new();
        t.execute("file.open", json!({"name": format!("x.{format}"), "base64": r["base64"]})).unwrap();
        assert_eq!(t.execute("python.packages", json!({})).unwrap()["packages"], json!(["pandas", "numpy>=1.26", "scikit-learn"]), "{format}");
    }
    // A file's list is checked when read: bad entries go, and at most 32 stay.
    let mut wb = s.doc().unwrap().wb.as_ref().clone();
    let mut hostile: Vec<String> = vec!["--index-url=http://evil.test".into(), "ok-one".into(), "../x".into()];
    hostile.extend((0..40).map(|i| format!("p{i}")));
    crate::cmd::stash::set_json(&mut wb, crate::pyenv::PACKAGES_KEY, &json!(hostile)).unwrap();
    let list = crate::pyenv::declared(&wb);
    assert_eq!(list.len(), 32);
    assert_eq!(list[0], "ok-one");
}

#[test]
fn installs_are_checked_and_need_approval() {
    let env = scratch("approve");
    let mut s = on(system_python(), &env);
    let e = s.execute("python.install", json!({})).unwrap_err().to_string();
    assert!(e.contains("missing `packages`"), "{e}");
    for (bad, why) in [
        (json!(["https://x.test/p.whl"]), "URL"),
        (json!(["-r", "reqs.txt"]), "option"),
        (json!(["./pkg"]), "path"),
        (json!(["numpy; python_version<'3'"]), "marker"),
        (json!(["pkg[a&b]"]), "extras"),
    ] {
        let e = s.execute("python.install", json!({"packages": bad, "approve": true})).unwrap_err().to_string();
        assert!(e.contains(why), "{bad}: {e}");
    }
    let many: Vec<String> = (0..33).map(|i| format!("p{i}")).collect();
    assert!(s.execute("python.install", json!({"packages": many, "approve": true})).unwrap_err().to_string().contains("At most 32"));
    // Without approval it asks, naming the packages (as pip names) and where they go.
    let q = approve_question(s.execute("python.install", json!({"packages": ["sklearn", "pandas"]})));
    assert!(q.starts_with("Install scikit-learn, pandas into GridCraft's Python environment ("), "{q}");
    assert!(q.contains(&env.display().to_string()), "{q}");
    assert!(q.contains("runs their code on this computer"), "{q}");
    assert!(!s.python_installing(), "nothing started");
    // A workbook from a file can't install until Python is enabled for it.
    s.doc_mut().unwrap().trust = Trust::FromFile;
    let e = s.execute("python.install", json!({"packages": ["pandas"], "approve": true})).unwrap_err().to_string();
    assert!(e.contains("enable Python"), "{e}");
    // With no CPython, nothing can be installed: the error says how to install by hand.
    let mut s = session();
    s.set_python_detection(Detection::default());
    let e = s.execute("python.install", json!({"packages": ["pandas"], "approve": true})).unwrap_err().to_string();
    assert!(e.contains("needs Python 3.11 or newer") && e.contains("install pandas with pip"), "{e}");
}

#[test]
fn packages_go_into_the_persons_own_environment_or_gridcrafts() {
    let env = scratch("target");
    let own = Interpreter { program: "/home/u/venv/bin/python".into(), venv: true, source: "VIRTUAL_ENV".into(), ..system_python() };
    let own = Interpreter { externally_managed: false, ..own };
    let mut s = on(own, &env);
    let q = approve_question(s.execute("python.install", json!({"packages": ["pandas"]})));
    assert!(q.contains("into your Python environment (/home/u/venv/bin/python)"), "{q}");
    // A system Python (externally managed, or just found on PATH) is never installed into.
    let mut s = on(system_python(), &env);
    assert!(
        matches!(s.python_install_target(&[]), Ok(gridcraft_python::env::Target::GridCraft { dir, base }) if dir == env && base.as_path() == std::path::Path::new("/opt/homebrew/bin/python3"))
    );
    let mut s = on(Interpreter { externally_managed: false, ..system_python() }, &env);
    assert!(matches!(s.python_install_target(&[]), Ok(gridcraft_python::env::Target::GridCraft { .. })));
    let conda = Interpreter { conda: true, externally_managed: false, source: "CONDA_PREFIX".into(), ..system_python() };
    let mut s = on(conda, &env);
    assert!(matches!(s.python_install_target(&[]), Ok(gridcraft_python::env::Target::Own { conda: true, .. })));
}

#[test]
fn approving_with_declare_adds_the_packages_to_the_workbook() {
    let env = scratch("declare");
    // A program that isn't Python: the install fails at once, but the approval was given.
    let fake = Interpreter {
        program: "/nonexistent/python3".into(),
        externally_managed: false,
        venv: true,
        source: "Python Options".into(),
        ..system_python()
    };
    let mut s = on(fake, &env);
    s.set_python_uv(false);
    s.execute("python.packages", json!({"packages": ["pandas"]})).unwrap();
    let e = s.execute("python.install", json!({"packages": ["numpy", "pandas>=2"], "approve": true, "declare": true})).unwrap_err().to_string();
    assert!(e.contains("Diagnostics") || e.contains("failed"), "{e}");
    assert_eq!(s.execute("python.packages", json!({})).unwrap()["packages"], json!(["pandas", "numpy"]));
    let status = s.execute("python.installStatus", json!({})).unwrap();
    assert_eq!(status["state"], "installFailed");
    assert!(status["output"].as_str().unwrap().contains("/nonexistent/python3"), "{status}");
    // The failure is in Diagnostics.
    let log = s.execute("python.diagnostics", json!({})).unwrap();
    let last = log["runs"].as_array().unwrap().last().cloned().unwrap();
    assert_eq!(last["status"], "installFailed");
    assert!(last["stdout"].as_str().unwrap().contains("-m pip install"), "{last}");
}

/// GridCraft's environment made for real from this machine's Python (pip and venv, offline),
/// listed, then deleted. Skipped without a Python 3.11 or newer.
#[test]
fn the_environment_is_made_listed_and_deleted() {
    let d = gridcraft_python::interp::detect(&std::env::var("GRIDCRAFT_TEST_PYTHON").unwrap_or_default(), None);
    let Some(found) = d.found else {
        eprintln!("skipped: no Python 3.11 or newer here");
        return;
    };
    let env = scratch("make");
    let mut s = on(found, &env);
    s.set_python_uv(false);
    let info = s.execute("python.environment", json!({})).unwrap();
    assert_eq!((info["exists"].as_bool(), info["path"].as_str()), (Some(false), Some(env.display().to_string().as_str())));
    let info = s.execute("python.environment", json!({"action": "create"})).unwrap();
    assert_eq!(info["exists"], true, "{info}");
    let names: Vec<&str> = info["packages"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).collect();
    assert!(names.contains(&"pip"), "{names:?}");
    // The search now finds it (before PATH).
    let found = gridcraft_python::interp::detect("", Some(&env)).found.unwrap();
    if std::env::var_os("VIRTUAL_ENV").is_none() && std::env::var_os("CONDA_PREFIX").is_none() && std::env::var_os("GRIDCRAFT_TEST_PYTHON").is_none()
    {
        assert_eq!(found.source, gridcraft_python::interp::GRIDCRAFT_SOURCE);
    }
    assert_eq!(
        approve_question(s.execute("python.environment", json!({"action": "delete"}))),
        format!("Delete GridCraft's Python environment at {}, with every package installed in it?", env.display())
    );
    let info = s.execute("python.environment", json!({"action": "delete", "approve": true})).unwrap();
    assert_eq!(info["exists"], false);
    assert!(!env.exists());
    // Only a folder holding an environment is ever deleted.
    std::fs::create_dir_all(&env).unwrap();
    std::fs::write(env.join("keep.txt"), "mine").unwrap();
    assert!(s.execute("python.environment", json!({"action": "delete", "approve": true})).is_err());
    assert!(env.join("keep.txt").exists());
    let _ = std::fs::remove_dir_all(&env);
}

/// Stop cancels an install running in the background.
#[test]
fn stop_cancels_an_install() {
    let d = gridcraft_python::interp::detect(&std::env::var("GRIDCRAFT_TEST_PYTHON").unwrap_or_default(), None);
    let Some(found) = d.found else { return };
    let env = scratch("cancel");
    let mut s = on(found, &env);
    s.set_python_uv(false);
    s.set_python_background(true);
    let r = s.execute("python.environment", json!({"action": "create"})).unwrap();
    assert_eq!(r["exists"], false, "still being made");
    assert!(s.python_busy());
    assert_eq!(s.execute("python.installStatus", json!({})).unwrap()["state"], "running");
    s.execute("python.stop", json!({})).unwrap();
    assert!(!s.python_busy());
    assert_eq!(s.execute("python.installStatus", json!({})).unwrap()["state"], "cancelled");
    let _ = std::fs::remove_dir_all(&env);
}

/// A cell that imports a missing module names its pip package, so the app can offer it.
#[test]
fn a_missing_module_names_its_pip_package() {
    let d = gridcraft_python::interp::detect(&std::env::var("GRIDCRAFT_TEST_PYTHON").unwrap_or_default(), None);
    if d.found.is_none() {
        return;
    }
    let mut s = Session::new();
    s.new_workbook();
    s.set_python_detection(d);
    s.execute("python.set", json!({"cell": "A1", "code": "import gridcraft_missing_mod.sub"})).unwrap();
    s.execute("python.set", json!({"cell": "A2", "code": "import yaml_gc_missing as y\n1"})).unwrap();
    let log = s.execute("python.diagnostics", json!({})).unwrap();
    let runs = log["runs"].as_array().unwrap();
    let a1 = runs.iter().find(|r| r["cell"] == "A1").unwrap();
    assert_eq!(a1["missing"], "gridcraft_missing_mod");
    assert_eq!(a1["error"], "ModuleNotFoundError: No module named 'gridcraft_missing_mod'");
    assert_eq!(crate::pyenv::manual_hint(None, &["pandas".into()]), "Install Python 3.11 or newer, then install pandas with pip.");
}
