//! Packages for Python cells: the packages a workbook declares, GridCraft's own Python
//! environment, and installs the person approves.
//!
//! A workbook lists the packages it needs (requirement strings, kept with the initialization
//! code in hidden names, so they travel in xlsx files). GridCraft installs packages only after the
//! person approves the exact list (`python.install` with `approve: true`), into a virtual
//! environment: the person's own venv or conda environment when that is what runs cells (chosen
//! in Python Options, or active through `VIRTUAL_ENV`/`CONDA_PREFIX`), otherwise GridCraft's own
//! shared environment, made from the CPython it found. A system or externally managed Python is
//! never installed into. The install runs on a thread with a 10-minute deadline; Stop cancels it.
//! When it succeeds, the search for Python runs again (finding the environment), workers restart
//! on it and the Python cells run again.

use std::path::PathBuf;

use gridcraft_model::Workbook;
/// Requirement checks, the environment's folder and installs (for hosts).
pub use gridcraft_python::env;
use gridcraft_python::env::{Install, Os, Plan, Target};
use gridcraft_python::interp::{GRIDCRAFT_SOURCE, Interpreter};
use serde_json::{Value as Json, json};

use crate::python::{PyRunLog, push_log};
use crate::{EngineError, Result, Session, clock};

/// The hidden-data key of the packages a workbook declares.
pub const PACKAGES_KEY: &str = "py_packages";

/// The packages the workbook declares: valid requirements only, at most
/// [`env::MAX_PACKAGES`] (a file may hold anything).
pub fn declared(wb: &Workbook) -> Vec<String> {
    let Some(Json::Array(items)) = crate::cmd::stash::get_json(wb, PACKAGES_KEY) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for req in items.iter().filter_map(Json::as_str).filter_map(|r| env::check_requirement(r).ok()) {
        let name = env::normalize(env::requirement_name(&req));
        if !out.iter().any(|r| env::normalize(env::requirement_name(r)) == name) {
            out.push(req);
        }
        if out.len() >= env::MAX_PACKAGES {
            break;
        }
    }
    out
}

/// Stores the workbook's declared packages (already checked); an empty list removes them.
pub fn set_declared(wb: &mut Workbook, list: &[String]) -> Result<()> {
    if list.is_empty() {
        crate::cmd::stash::remove(wb, PACKAGES_KEY);
        Ok(())
    } else {
        crate::cmd::stash::set_json(wb, PACKAGES_KEY, &json!(list))
    }
}

/// The packages a workbook declares plus `more`, without duplicates (by name), checked again.
pub fn merged(wb: &Workbook, more: &[String]) -> std::result::Result<Vec<String>, String> {
    let mut all = declared(wb);
    for m in more {
        let name = env::normalize(env::requirement_name(m));
        if !all.iter().any(|r| env::normalize(env::requirement_name(r)) == name) {
            all.push(m.clone());
        }
    }
    env::check_requirements(&all)
}

/// How to install packages by hand, for when GridCraft can't (the text the shim used to add
/// to a missing module's error).
pub fn manual_hint(found: Option<&Interpreter>, packages: &[String]) -> String {
    let list = packages.join(" ");
    let Some(i) = found else {
        return format!("Install Python 3.11 or newer, then install {} with pip.", packages.join(", "));
    };
    let exe = i.program.display().to_string();
    if i.externally_managed {
        let venv_python = if cfg!(windows) { "<folder>\\Scripts\\python.exe" } else { "<folder>/bin/python" };
        format!(
            "This Python is managed by your system, so pip can't install into it: make a virtual environment with \"{exe}\" -m venv <folder>, install {list} into it with \"{venv_python}\" -m pip install {list}, then choose {venv_python} in Python Options."
        )
    } else {
        format!("Install it with: \"{exe}\" -m pip install {list}")
    }
}

/// The install running now.
pub(crate) struct InstallJob {
    /// The workbook it was started from (its Diagnostics get the log).
    uid: Option<u64>,
    packages: Vec<String>,
    target: String,
    runner: Install,
    started_ms: f64,
}

#[derive(Default)]
pub(crate) struct InstallState {
    job: Option<InstallJob>,
    /// The last install's result (`python.installStatus`, the status bar).
    last: Option<Json>,
    /// Whether installs may use `uv` when it is on PATH (`None`: yes).
    uv: Option<bool>,
}

impl Session {
    /// Where GridCraft's own Python environment lives (Python Options, `GRIDCRAFT_PYTHON_ENV`, or
    /// the app's data folder).
    pub fn python_env_dir(&self) -> Option<PathBuf> {
        env::env_dir(&self.prefs.python_environment)
    }

    /// Where packages would be installed now, or why they can't be (with how to install them by
    /// hand). Headless sessions wait for the search for Python.
    pub fn python_install_target(&mut self, packages: &[String]) -> std::result::Result<Target, String> {
        if !gridcraft_python::AVAILABLE {
            return Err("Python isn't available in the web version".into());
        }
        let dir = self.python_env_dir();
        let searching = self.python_backend()["kind"] == "detecting";
        if searching {
            return Err("GridCraft is still looking for Python; try again in a moment.".into());
        }
        let found = self.python_detection().and_then(|d| d.found.clone());
        let Some(i) = found else {
            return Err(format!("GridCraft needs Python 3.11 or newer to install packages. {}", manual_hint(None, packages)));
        };
        if i.own_environment() {
            return Ok(Target::Own { python: i.program, conda: i.conda && !i.venv });
        }
        match dir {
            Some(dir) => Ok(Target::GridCraft { dir, base: i.program }),
            None => Err(format!("GridCraft couldn't find its data folder to keep a Python environment in. {}", manual_hint(Some(&i), packages))),
        }
    }

    /// Whether installs may use `uv` when it is on PATH (tests turn it off to stay offline: pip
    /// and venv need no network to make an environment).
    pub fn set_python_uv(&mut self, on: bool) {
        self.python.install.uv = Some(on);
    }

    /// Whether a package install is running.
    pub fn python_installing(&self) -> bool {
        self.python.install.job.is_some()
    }

    /// Starts installing `packages` into `target` (making GridCraft's environment first when it
    /// isn't there; with no packages, only that). In the background it returns at once and
    /// [`Session::step_python`] finishes it; headless callers wait for the result.
    pub(crate) fn python_start_install(&mut self, target: Target, packages: Vec<String>) -> Result<Json> {
        if self.python_installing() {
            return Err(EngineError::Other("A Python install is already running: wait for it, or stop it.".into()));
        }
        let create = match &target {
            Target::GridCraft { dir, .. } => !env::env_python(dir, Os::current()).is_file(),
            Target::Own { .. } => false,
        };
        let described = target.describe();
        let uv = if self.python.install.uv.unwrap_or(true) { env::find_uv() } else { None };
        let plan = Plan { target, packages: packages.clone(), uv, os: Os::current() };
        let uid = self.doc().ok().map(|d| d.uid);
        self.python.install.job = Some(InstallJob {
            uid,
            packages: packages.clone(),
            target: described.clone(),
            runner: Install::start(plan, create),
            started_ms: clock::now_ms(),
        });
        self.python.install.last = None;
        if self.python.background {
            return Ok(json!({"state": "running", "target": described, "packages": packages, "create": create}));
        }
        let result = self.wait_install().unwrap_or_else(|| json!({"state": "idle"}));
        if result["ok"].as_bool() == Some(true) {
            Ok(result)
        } else {
            Err(EngineError::Other(result["message"].as_str().unwrap_or("The install failed").to_string()))
        }
    }

    /// Waits for a running install to end, and finishes it.
    pub(crate) fn wait_install(&mut self) -> Option<Json> {
        loop {
            let job = self.python.install.job.as_mut()?;
            if job.runner.wait(std::time::Duration::from_millis(100)).is_some() {
                return self.finish_install();
            }
        }
    }

    /// Finishes the install if it has ended; returns whether it did.
    pub(crate) fn poll_install(&mut self) -> bool {
        let ended = self.python.install.job.as_mut().is_some_and(|j| j.runner.poll().is_some());
        if ended {
            self.finish_install();
        }
        ended
    }

    /// Stops a running install. Returns whether one was running.
    pub fn python_cancel_install(&mut self) -> bool {
        let Some(job) = self.python.install.job.as_ref() else { return false };
        job.runner.cancel();
        self.wait_install();
        true
    }

    /// Logs the ended install in Diagnostics and, when it worked, restarts Python on the new
    /// environment and runs the cells again.
    fn finish_install(&mut self) -> Option<Json> {
        let mut job = self.python.install.job.take()?;
        let outcome = job.runner.poll()?.clone();
        let status = if outcome.ok {
            "installed"
        } else if outcome.cancelled {
            "cancelled"
        } else {
            "installFailed"
        };
        let result = json!({
            "state": status,
            "ok": outcome.ok,
            "message": outcome.message,
            "packages": job.packages,
            "target": job.target,
            "output": outcome.output,
        });
        let uid = job.uid.or_else(|| self.doc().ok().map(|d| d.uid));
        if let Some(d) = self.docs.iter_mut().find(|d| Some(d.uid) == uid) {
            push_log(
                d,
                PyRunLog {
                    seq: 0,
                    sheet: String::new(),
                    cell: String::new(),
                    status,
                    ms: (clock::now_ms() - job.started_ms).max(0.0),
                    stdout: outcome.output.clone(),
                    stderr: String::new(),
                    error: (!outcome.ok).then(|| outcome.message.clone()),
                    line: None,
                    missing: None,
                },
            );
        }
        self.python.install.last = Some(result.clone());
        if outcome.ok {
            // Python starts again on what the search finds now (the environment), and every
            // workbook's Python cells run again on it.
            self.python_detect();
            let uids: Vec<u64> = self
                .docs
                .iter()
                .filter(|d| d.trust.allows_code() && d.wb.sheets.iter().any(|s| s.cells.iter().any(|(_, c)| c.py.is_some())))
                .map(|d| d.uid)
                .collect();
            for uid in uids {
                self.python_rerun_all(uid);
            }
        }
        Some(result)
    }

    /// The install's state: `{state: idle|running|installed|installFailed|cancelled, packages?,
    /// target?, message?, output?}`.
    pub fn python_install_status(&self) -> Json {
        match &self.python.install.job {
            Some(j) => json!({"state": "running", "packages": j.packages, "target": j.target, "output": j.runner.output()}),
            None => self.python.install.last.clone().unwrap_or_else(|| json!({"state": "idle"})),
        }
    }

    /// GridCraft's environment: `{path, python, exists, active, packages: [{name, version}],
    /// custom}` (`active`: it runs cells now; `custom`: its place was chosen in Python Options).
    pub fn python_environment(&mut self) -> Json {
        let Some(dir) = self.python_env_dir() else {
            return json!({"path": null, "exists": false, "active": false, "packages": [], "custom": false});
        };
        let exists = env::is_env(&dir);
        let active = self.python_detection().and_then(|d| d.found.as_ref()).is_some_and(|i| i.source == GRIDCRAFT_SOURCE);
        let packages: Vec<Json> =
            if exists { env::packages_in(&dir).into_iter().map(|(n, v)| json!({"name": n, "version": v})).collect() } else { Vec::new() };
        json!({
            "path": dir.display().to_string(),
            "python": env::env_python(&dir, Os::current()).display().to_string(),
            "exists": exists,
            "active": active,
            "packages": packages,
            "custom": !self.prefs.python_environment.trim().is_empty(),
        })
    }

    /// Deletes GridCraft's environment (only a folder that holds one), stopping the workers that
    /// run on it; cells then run on whatever Python the search finds.
    pub(crate) fn python_delete_environment(&mut self) -> Result<()> {
        let dir = self.python_env_dir().ok_or_else(|| EngineError::Other("GridCraft has no Python environment folder here".into()))?;
        if !env::is_env(&dir) {
            return Err(EngineError::Other(format!("There is no GridCraft Python environment at {}", dir.display())));
        }
        if self.python_installing() {
            return Err(EngineError::Other("A Python install is running: wait for it, or stop it.".into()));
        }
        // Its python may be running cells.
        let uids: Vec<u64> = self.docs.iter().map(|d| d.uid).collect();
        for uid in uids {
            self.python_stop_worker(uid);
        }
        #[cfg(not(target_arch = "wasm32"))]
        std::fs::remove_dir_all(&dir).map_err(|e| EngineError::Other(format!("GridCraft couldn't delete {}: {e}", dir.display())))?;
        self.python_detect();
        Ok(())
    }
}
