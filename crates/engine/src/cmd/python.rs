//! Python in cells: making and editing Python cells, the output mode, the runtime, diagnostics,
//! the workbook's initialization code, and enabling Python in a workbook from a file. How cells
//! run is in [`crate::python`].

use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Cell, PyCode};
use serde_json::{Value as Json, json};

use super::{CommandSpec, always, bad, bool_param, cell_param, cmd, edit, f64_param, has_doc, str_param, target_sheet};
use crate::python::INIT_KEY;
use crate::{EngineError, Result, Session, Trust};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "python.insert",
            "Insert Python",
            ["Formulas", "Python"],
            Some("Ctrl+Alt+Shift+P"),
            "{cell?} makes the cell (default: the active cell) a Python cell with no code yet",
            has_doc,
            insert
        ),
        cmd!(
            "python.set",
            "Set Python Code",
            [],
            None,
            "{cell?, code, output?: object|value} sets a Python cell's code (at most 7,000 characters) and runs it",
            has_doc,
            set
        ),
        cmd!(
            "python.clear",
            "Convert to Normal Cell",
            [],
            None,
            "{cell?} turns Python off for the cell, clearing its code and value",
            has_doc,
            clear
        ),
        cmd!(
            "python.output",
            "Python Output",
            [],
            None,
            "{cell?, mode: object|value} shows the result as a Python object or converts it to cell values",
            has_doc,
            output
        ),
        cmd!(noundo "python.stop", "Stop", ["Formulas", "Python"], None, "{} → {stopped}: stops the Python cells running or waiting in the active workbook; they show #PYTHON! (\"Stopped\") until the next edit or Reset", has_doc, stop),
        cmd!(noundo "python.reset", "Reset Runtime", ["Formulas", "Python"], None, "{clearDiagnostics?: bool} starts Python afresh and runs every Python cell again", has_doc, reset),
        cmd!(
            query "python.diagnostics",
            "Diagnostics",
            ["Formulas", "Python"],
            None,
            "{sheet?, errorsOnly?: bool} → {backend: {kind, summary, path?, version?, pandas?, numpy?, rejected}, runs: [{seq, sheet, cell, status, ms, stdout, stderr, error?, line?}]}: which Python runs cells, and the last 500 runs, oldest first",
            has_doc,
            diagnostics
        ),
        cmd!(noundo "python.clearDiagnostics", "Clear Diagnostics", [], None, "{} empties the active workbook's Python diagnostics", has_doc, clear_diagnostics),
        cmd!(
            "python.initialization",
            "Initialization…",
            ["Formulas", "Python"],
            None,
            "{code?} → {code}: the workbook's initialization code, run before any Python cell; with `code`, sets it and runs every Python cell again",
            has_doc,
            initialization
        ),
        cmd!(
            noundo "python.options",
            "Python Options",
            [],
            None,
            "{timeoutSeconds?: 0.1..3600, interpreter?: path (empty: look for one), environment?: folder (empty: the default), detect?: bool} → {timeoutSeconds, interpreter, environmentPath, environment, backend}: how long one Python cell may run (default 5), the user's Python (3.11 or newer) that runs cells instead of the built-in one, and where GridCraft keeps its own Python environment; `detect` looks for Python again",
            always,
            options
        ),
        cmd!(noundo "python.enable", "Enable Python", [], None, "{} lets Python run in a workbook opened from a file, and runs its Python cells", has_doc, enable),
        cmd!(
            "python.packages",
            "Workbook Packages",
            [],
            None,
            "{packages?: [requirement]} → {packages}: the Python packages the workbook needs (names with an optional version, such as pandas or numpy>=1.26; at most 32); with `packages`, sets them",
            has_doc,
            packages
        ),
        cmd!(
            noundo "python.install",
            "Install Python Packages",
            [],
            None,
            "{packages: [requirement], approve: bool, declare?: bool} → {state, target, packages, message?}: installs packages (names with an optional version; no URLs, paths or options; at most 32) into the person's own virtual environment when it runs cells, else GridCraft's own environment (made first if needed). Asks for approval unless `approve` is true; `declare` also adds them to the workbook's packages. Runs in the background in the app (python.stop cancels it)",
            always,
            install
        ),
        cmd!(
            query "python.installStatus",
            "Python Install Status",
            [],
            None,
            "{} → {state: idle|running|installed|installFailed|cancelled, packages?, target?, message?, output?}: the running or last package install",
            always,
            install_status
        ),
        cmd!(
            noundo "python.environment",
            "Python Environment",
            [],
            None,
            "{action?: create|delete, approve?: bool} → {path, python, exists, active, packages: [{name, version}], custom}: GridCraft's own Python environment; `create` makes it from the Python found, `delete` removes it (with `approve: true`)",
            always,
            environment
        ),
    ]
}

fn target_cell(s: &Session, p: &Json) -> Result<(usize, CellRef)> {
    let sheet = target_sheet(s, p)?;
    let at = match cell_param(p, "cell") {
        Some(c) => c,
        None if str_param(p, "cell").is_some() => return Err(bad("python", "`cell` is not a cell reference")),
        None => s.doc()?.selection.active,
    };
    Ok((sheet, at))
}

/// Puts `py` into the cell, keeping its style; a formula or value there goes, and so does a
/// formula's spill.
fn put(s: &mut Session, sheet: usize, at: CellRef, py: PyCode) -> Result<()> {
    let before = s.doc()?.wb.clone();
    // A table header is the column's name, never a formula or code (as in Excel).
    let header = before.sheet(sheet).is_some_and(|sh| sh.tables.iter().any(|t| t.header_row && t.range.start.row == at.row && t.range.contains(at)));
    if header {
        return Err(EngineError::Other(format!("{} is a table header; Python cells can't go in a table's header row", at.a1())));
    }
    edit(s, |cx| {
        let sh = cx.sheet_mut(sheet)?;
        let style = sh.style_id(at);
        let value = sh.cell(at).filter(|c| c.py.is_some()).map(|c| c.value.clone()).unwrap_or(Value::Empty);
        if let Some(r) = sh.spill_ranges.remove(&at) {
            for c in r.iter() {
                sh.spill.remove(&c);
            }
        }
        sh.set_cell(at, Cell { value, formula: None, style, py: Some(Arc::new(py)) });
        cx.touch(sheet, at);
        Ok(())
    })?;
    // Run now, so the command can answer with the result.
    s.python_refresh_now("python.set", before)
}

fn existing(s: &Session, sheet: usize, at: CellRef) -> Result<Option<PyCode>> {
    Ok(s.doc()?.wb.sheet(sheet).and_then(|sh| sh.cell(at)).and_then(|c| c.py.as_deref()).cloned())
}

fn result(s: &Session, sheet: usize, at: CellRef) -> Result<Json> {
    let d = s.doc()?;
    let sh = d.wb.sheet(sheet).ok_or(EngineError::NoDocument)?;
    Ok(json!({"cell": at.a1(), "value": crate::cmd::inspect::value_json(&sh.value(at))}))
}

fn output_mode(p: &Json, key: &str) -> Result<Option<bool>> {
    match str_param(p, key) {
        None => Ok(None),
        Some(m) if m.eq_ignore_ascii_case("object") => Ok(Some(true)),
        Some(m) if m.eq_ignore_ascii_case("value") => Ok(Some(false)),
        Some(m) => Err(bad("python.output", format!("`{key}` is `object` or `value`, not `{m}`"))),
    }
}

fn insert(s: &mut Session, p: &Json) -> Result<Json> {
    let (sheet, at) = target_cell(s, p)?;
    if existing(s, sheet, at)?.is_none() {
        put(s, sheet, at, PyCode::new(""))?;
    }
    result(s, sheet, at)
}

fn set(s: &mut Session, p: &Json) -> Result<Json> {
    let (sheet, at) = target_cell(s, p)?;
    let code = str_param(p, "code").ok_or_else(|| bad("python.set", "missing `code`"))?;
    if code.chars().nth(gridcraft_model::python::MAX_CODE_CHARS).is_some() {
        return Err(bad("python.set", format!("Python code is limited to {} characters per cell", gridcraft_model::python::MAX_CODE_CHARS)));
    }
    let mode = output_mode(p, "output")?;
    let old = existing(s, sheet, at)?;
    let object = mode.or(old.as_ref().map(|o| o.object)).unwrap_or(false);
    let mut py = PyCode { object, ..PyCode::new(code) };
    // Unchanged code keeps its result; the cell runs only when something it reads changes.
    if let Some(old) = old.filter(|o| o.code == py.code) {
        py.output = old.output;
    }
    put(s, sheet, at, py)?;
    result(s, sheet, at)
}

fn clear(s: &mut Session, p: &Json) -> Result<Json> {
    let (sheet, at) = target_cell(s, p)?;
    if existing(s, sheet, at)?.is_none() {
        return Err(EngineError::Other(format!("{} is not a Python cell", at.a1())));
    }
    let before = s.doc()?.wb.clone();
    edit(s, |cx| {
        let sh = cx.sheet_mut(sheet)?;
        if let Some(mut cell) = sh.cell(at).cloned() {
            cell.clear_content();
            sh.set_cell(at, cell);
        }
        cx.touch(sheet, at);
        Ok(())
    })?;
    // The cells after it share its namespace and run again.
    s.python_refresh_now("python.clear", before)?;
    result(s, sheet, at)
}

fn output(s: &mut Session, p: &Json) -> Result<Json> {
    let (sheet, at) = target_cell(s, p)?;
    let object = output_mode(p, "mode")?.ok_or_else(|| bad("python.output", "missing `mode` (object or value)"))?;
    let Some(mut py) = existing(s, sheet, at)? else { return Err(EngineError::Other(format!("{} is not a Python cell", at.a1()))) };
    if py.object != object {
        py.object = object;
        put(s, sheet, at, py)?;
    }
    result(s, sheet, at)
}

fn reset(s: &mut Session, p: &Json) -> Result<Json> {
    if bool_param(p, "clearDiagnostics").unwrap_or(false) {
        s.doc_mut()?.py_log.clear();
    }
    // The user may have installed Python or a package since: look again.
    s.python_detect();
    s.python_reset()?;
    Ok(json!({"runs": s.doc()?.py_log.len()}))
}

fn stop(s: &mut Session, _p: &Json) -> Result<Json> {
    let stopped = s.python_stop()?;
    Ok(json!({"stopped": stopped}))
}

fn diagnostics(s: &mut Session, p: &Json) -> Result<Json> {
    let d = s.doc()?;
    let sheet = match p.get("sheet") {
        None => None,
        Some(_) => Some(target_sheet(s, p)?),
    };
    let sheet_name = sheet.and_then(|i| d.wb.sheet(i)).map(|sh| sh.name.clone());
    let errors_only = bool_param(p, "errorsOnly").unwrap_or(false);
    let runs: Vec<crate::python::PyRunLog> =
        d.py_log.iter().filter(|r| sheet_name.as_ref().is_none_or(|n| r.sheet == *n)).filter(|r| !errors_only || r.status != "ok").cloned().collect();
    let backend = s.python_backend();
    Ok(json!({"backend": backend, "runs": runs}))
}

fn clear_diagnostics(s: &mut Session, _p: &Json) -> Result<Json> {
    s.doc_mut()?.py_log.clear();
    Ok(json!({"runs": 0}))
}

fn initialization(s: &mut Session, p: &Json) -> Result<Json> {
    let Some(code) = str_param(p, "code") else {
        return Ok(json!({"code": crate::cmd::stash::get(&s.doc()?.wb, INIT_KEY).unwrap_or_default()}));
    };
    if code.chars().nth(gridcraft_model::python::MAX_CODE_CHARS).is_some() {
        return Err(bad(
            "python.initialization",
            format!("Initialization code is limited to {} characters", gridcraft_model::python::MAX_CODE_CHARS),
        ));
    }
    let code = gridcraft_model::python::normalize_code(code);
    edit(s, |cx| {
        if code.is_empty() {
            crate::cmd::stash::remove(&mut cx.wb, INIT_KEY);
            Ok(())
        } else {
            crate::cmd::stash::set(&mut cx.wb, INIT_KEY, &code)
        }
    })?;
    s.python_reset()?;
    Ok(json!({"code": code}))
}

fn enable(s: &mut Session, _p: &Json) -> Result<Json> {
    let d = s.doc_mut()?;
    if d.trust == Trust::FromFile {
        d.trust = Trust::Trusted;
        s.python_reset()?;
    }
    Ok(json!({"trust": s.doc()?.trust}))
}

/// A list of requirement strings from `packages`.
fn requirements(p: &Json, id: &str) -> Result<Option<Vec<String>>> {
    let Some(list) = p.get("packages") else { return Ok(None) };
    let Some(items) = list.as_array() else { return Err(bad(id, "`packages` is a list of package names, such as [\"pandas\", \"numpy>=1.26\"]")) };
    if items.len() > gridcraft_python::env::MAX_PACKAGES {
        return Err(bad(id, format!("At most {} packages can be listed at once", gridcraft_python::env::MAX_PACKAGES)));
    }
    let texts: Vec<&str> =
        items.iter().map(|i| i.as_str().ok_or_else(|| bad(id, "each package is text, such as \"pandas\""))).collect::<Result<_>>()?;
    gridcraft_python::env::check_requirements(&texts).map(Some).map_err(|e| bad(id, e))
}

fn packages(s: &mut Session, p: &Json) -> Result<Json> {
    if let Some(list) = requirements(p, "python.packages")? {
        edit(s, |cx| crate::pyenv::set_declared(&mut cx.wb, &list))?;
    }
    Ok(json!({"packages": crate::pyenv::declared(&s.doc()?.wb)}))
}

fn install(s: &mut Session, p: &Json) -> Result<Json> {
    let list = requirements(p, "python.install")?.unwrap_or_default();
    if list.is_empty() {
        return Err(bad("python.install", "missing `packages` (a list of package names, such as [\"pandas\"])"));
    }
    // A workbook from a file can't install anything until Python is enabled for it.
    if s.doc().is_ok_and(|d| !d.trust.allows_code()) {
        return Err(EngineError::Other("Python is turned off for this workbook: enable Python before installing packages for it.".into()));
    }
    let target = s.python_install_target(&list).map_err(EngineError::Other)?;
    if !bool_param(p, "approve").unwrap_or(false) {
        let what = match &target {
            gridcraft_python::env::Target::Own { python, .. } => format!("your Python environment ({})", python.display()),
            gridcraft_python::env::Target::GridCraft { dir, .. } => format!("GridCraft's Python environment ({})", dir.display()),
        };
        return Err(EngineError::Approve(format!(
            "Install {} into {what}? Installing downloads packages from the internet and runs their code on this computer: install only packages you trust.",
            list.join(", ")
        )));
    }
    if bool_param(p, "declare").unwrap_or(false) && s.doc().is_ok() {
        let all = crate::pyenv::merged(&s.doc()?.wb, &list).map_err(|e| bad("python.install", e))?;
        edit(s, |cx| crate::pyenv::set_declared(&mut cx.wb, &all))?;
    }
    s.python_start_install(target, list)
}

fn install_status(s: &mut Session, _p: &Json) -> Result<Json> {
    Ok(s.python_install_status())
}

fn environment(s: &mut Session, p: &Json) -> Result<Json> {
    match str_param(p, "action") {
        None => {}
        Some("create") => {
            if s.doc().is_ok_and(|d| !d.trust.allows_code()) {
                return Err(EngineError::Other("Python is turned off for this workbook: enable Python first.".into()));
            }
            let base = s.python_detection().and_then(|d| d.found.clone()).map(|i| i.program);
            let (Some(dir), Some(base)) = (s.python_env_dir(), base) else {
                return Err(EngineError::Other(
                    "GridCraft makes its environment from Python 3.11 or newer on this computer, and found none. Install Python, then try again."
                        .into(),
                ));
            };
            if gridcraft_python::env::is_env(&dir) {
                return Ok(s.python_environment());
            }
            s.python_start_install(gridcraft_python::env::Target::GridCraft { dir, base }, Vec::new())?;
        }
        Some("delete") => {
            let path = s.python_env_dir().map(|d| d.display().to_string()).unwrap_or_default();
            if !bool_param(p, "approve").unwrap_or(false) {
                return Err(EngineError::Approve(format!("Delete GridCraft's Python environment at {path}, with every package installed in it?")));
            }
            s.python_delete_environment()?;
        }
        Some(other) => return Err(bad("python.environment", format!("`action` is `create` or `delete`, not `{other}`"))),
    }
    Ok(s.python_environment())
}

fn options(s: &mut Session, p: &Json) -> Result<Json> {
    if let Some(t) = f64_param(p, "timeoutSeconds") {
        if !(0.1..=3_600.0).contains(&t) {
            return Err(bad("python.options", "`timeoutSeconds` is between 0.1 and 3600"));
        }
        s.prefs.python_timeout_seconds = t;
    }
    let mut detect = bool_param(p, "detect").unwrap_or(false);
    if let Some(path) = p.get("interpreter") {
        let Some(path) = path.as_str() else { return Err(bad("python.options", "`interpreter` is a path, or empty to look for Python")) };
        if path.len() > 4_096 || path.contains('\0') {
            return Err(bad("python.options", "`interpreter` is not a usable path"));
        }
        let path = path.trim().to_string();
        detect |= path != s.prefs.python_interpreter;
        s.prefs.python_interpreter = path;
    }
    if let Some(path) = p.get("environment") {
        let Some(path) = path.as_str() else { return Err(bad("python.options", "`environment` is a folder path, or empty for the default")) };
        if path.len() > 4_096 || path.contains('\0') {
            return Err(bad("python.options", "`environment` is not a usable path"));
        }
        let path = path.trim().to_string();
        detect |= path != s.prefs.python_environment;
        s.prefs.python_environment = path;
    }
    if detect {
        s.python_detect();
        // Cells run again on whatever Python the search finds.
        if s.doc().is_ok() {
            s.python_reset()?;
        }
    }
    let backend = s.python_backend();
    let environment = s.python_environment();
    Ok(json!({
        "timeoutSeconds": s.prefs.python_timeout_seconds,
        "interpreter": s.prefs.python_interpreter,
        "environmentPath": s.prefs.python_environment,
        "environment": environment,
        "backend": backend,
    }))
}
