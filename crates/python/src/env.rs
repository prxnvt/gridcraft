//! GridCraft's own Python environment, and installing packages into it after the user approves.
//!
//! GridCraft keeps one shared virtual environment (not one per workbook), made from the CPython it
//! found on the machine; it never bundles or downloads an interpreter. It lives in the app's data
//! folder: `~/Library/Application Support/GridCraft/python-env` on macOS,
//! `$XDG_DATA_HOME/gridcraft/python-env` (default `~/.local/share`) on Linux and BSD, and
//! `%LOCALAPPDATA%\GridCraft\python-env` on Windows. Python Options or `GRIDCRAFT_PYTHON_ENV` move
//! it (tests always do, so they never touch the real one).
//!
//! Packages are installed only after the person approves the exact list, and only into a virtual
//! environment: GridCraft's own, or the person's venv or conda environment that they chose (never
//! a system or externally managed Python). Every requirement is checked strictly first: a package
//! name with an optional version specifier, nothing else (no URLs, paths, options or markers).
//! Programs run from an argument list, never through a shell; [`Install`] runs them on a thread
//! of its own, with a deadline and a cancel switch.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Overrides where GridCraft's environment lives (a folder path).
pub const ENV_VAR: &str = "GRIDCRAFT_PYTHON_ENV";
/// Requirements one install, or one workbook, may list.
pub const MAX_PACKAGES: usize = 32;
/// Characters in one requirement.
pub const MAX_REQUIREMENT_CHARS: usize = 100;
/// How long one install may run.
pub const INSTALL_SECONDS: u64 = 600;
/// How long the warm-up after an install (importing what was installed once) may run.
pub const WARMUP_SECONDS: u64 = 120;
/// Bytes of an install's output kept (the end of it).
pub const OUTPUT_BYTES: usize = 64 * 1024;
/// Packages listed from an environment's folder.
const MAX_LISTED: usize = 5_000;

/// The operating system, for paths (tests pass each one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    /// Linux and the BSDs.
    Unix,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Unix
        }
    }
}

/// The default folder of GridCraft's environment, from the environment variables `var` reads.
pub fn default_dir(var: &dyn Fn(&str) -> Option<OsString>, os: Os) -> Option<PathBuf> {
    let set = |k: &str| var(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    match os {
        Os::MacOs => Some(set("HOME")?.join("Library").join("Application Support").join("GridCraft").join("python-env")),
        Os::Windows => Some(set("LOCALAPPDATA")?.join("GridCraft").join("python-env")),
        Os::Unix => {
            let data = set("XDG_DATA_HOME").or_else(|| set("HOME").map(|h| h.join(".local").join("share")))?;
            Some(data.join("gridcraft").join("python-env"))
        }
    }
}

/// Where GridCraft's environment is: `configured` (Python Options) when set, else
/// [`ENV_VAR`], else the default folder.
pub fn env_dir(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        return Some(PathBuf::from(configured));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let var = |k: &str| std::env::var_os(k);
        if let Some(over) = var(ENV_VAR).filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(over));
        }
        default_dir(&var, Os::current())
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

/// The python program inside an environment folder.
pub fn env_python(dir: &Path, os: Os) -> PathBuf {
    match os {
        Os::Windows => dir.join("Scripts").join("python.exe"),
        _ => dir.join("bin").join("python"),
    }
}

/// Whether `dir` holds a virtual environment (it has a `pyvenv.cfg`). Only such a folder is ever
/// deleted.
pub fn is_env(dir: &Path) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        dir.join("pyvenv.cfg").is_file()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = dir;
        false
    }
}

/// The packages installed in the environment at `dir` (name, version), from its
/// `site-packages/*.dist-info` folders, sorted by name. Nothing runs.
pub fn packages_in(dir: &Path) -> Vec<(String, String)> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let read = |p: &Path| -> Vec<PathBuf> {
            std::fs::read_dir(p).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).take(MAX_LISTED).collect()).unwrap_or_default()
        };
        let mut sites = vec![dir.join("Lib").join("site-packages")];
        for lib in [dir.join("lib"), dir.join("lib64")] {
            sites.extend(
                read(&lib)
                    .into_iter()
                    .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("python")))
                    .map(|p| p.join("site-packages")),
            );
        }
        let mut out: Vec<(String, String)> = Vec::new();
        for site in sites {
            for entry in read(&site) {
                let Some(name) = entry.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
                let Some(stem) = name.strip_suffix(".dist-info") else { continue };
                // `name-version`: the name has no `-` in a wheel's folder (it is `_` there).
                let (n, v) = stem.split_once('-').unwrap_or((stem, ""));
                if !out.iter().any(|(m, _)| normalize(m) == normalize(n)) {
                    out.push((n.to_string(), v.to_string()));
                }
                if out.len() >= MAX_LISTED {
                    break;
                }
            }
        }
        out.sort_by_key(|(n, _)| normalize(n));
        out
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = dir;
        Vec::new()
    }
}

/// The packages (name, version) an installer's output says it installed or found already
/// there: uv's ` + name==version` lines, pip's `Successfully installed name-version …` and
/// `Requirement already satisfied: name in … (version)`. Later mentions of a name win.
pub fn installed_versions(output: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |name: &str, version: &str| {
        let (name, version) = (name.trim(), version.trim());
        if name.is_empty() || version.is_empty() || name.len() > MAX_REQUIREMENT_CHARS || version.len() > 64 {
            return;
        }
        out.retain(|(n, _)| normalize(n) != normalize(name));
        out.push((name.to_string(), version.to_string()));
    };
    for line in output.lines().take(20_000) {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("+ ")
            && let Some((name, version)) = rest.split_once("==")
        {
            // uv: ` + six==1.17.0` (a URL install has `@` instead; it has no version to show).
            add(name, version.split_whitespace().next().unwrap_or_default());
        } else if let Some(rest) = line.strip_prefix("Successfully installed ") {
            for item in rest.split_whitespace() {
                if let Some((name, version)) = item.rsplit_once('-') {
                    add(name, version);
                }
            }
        } else if let Some(rest) = line.strip_prefix("Requirement already satisfied: ")
            && let Some(name) = rest.split_whitespace().next()
            && let Some(version) = rest.rsplit_once('(').and_then(|(_, v)| v.strip_suffix(')'))
        {
            // The requirement may carry a specifier (`six>=1`): the name is before it.
            let name = name.split(['<', '>', '=', '!', '~', '[']).next().unwrap_or(name);
            add(name, version);
        }
    }
    out
}

/// The environment folder a Python program belongs to: the folder above `bin` (or `Scripts`),
/// else the program's own folder (a conda `python.exe` on Windows).
pub fn env_root(python: &Path) -> Option<PathBuf> {
    let parent = python.parent()?;
    match parent.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()) {
        Some(n) if n == "bin" || n == "scripts" => parent.parent().map(Path::to_path_buf),
        _ => Some(parent.to_path_buf()),
    }
}

/// Import names whose pip package is called something else.
pub const PIP_NAMES: &[(&str, &str)] = &[
    ("sklearn", "scikit-learn"),
    ("cv2", "opencv-python"),
    ("PIL", "pillow"),
    ("yaml", "pyyaml"),
    ("bs4", "beautifulsoup4"),
    ("dateutil", "python-dateutil"),
    ("skimage", "scikit-image"),
    ("docx", "python-docx"),
    ("pptx", "python-pptx"),
    ("dotenv", "python-dotenv"),
];

/// The pip package that provides the module `import`s (the first part of a dotted name).
pub fn pip_name(module: &str) -> String {
    let top = module.split('.').next().unwrap_or_default().trim();
    PIP_NAMES.iter().find(|(m, _)| *m == top).map(|(_, p)| (*p).to_string()).unwrap_or_else(|| top.to_string())
}

/// A package name compared the way pip compares them (PEP 503): lower case, with runs of `-`,
/// `_` and `.` as one `-`.
pub fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut sep = false;
    for c in name.trim().chars() {
        if matches!(c, '-' | '_' | '.') {
            sep = true;
            continue;
        }
        if sep && !out.is_empty() {
            out.push('-');
        }
        sep = false;
        out.push(c.to_ascii_lowercase());
    }
    out
}

fn is_name(s: &str) -> bool {
    let ok_end = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    ok_end(s.chars().next()) && ok_end(s.chars().last()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The package name of a requirement (`pandas` in `pandas>=2`).
pub fn requirement_name(req: &str) -> &str {
    let t = req.trim();
    let end = t.find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))).unwrap_or(t.len());
    t.get(..end).unwrap_or_default()
}

/// Checks one requirement: a PEP 508 package name, optional extras of plain names, and an
/// optional version specifier (`pandas`, `numpy>=1.26,<3`, `requests[socks]==2.32.3`). Anything
/// else (URLs, paths, options, environment markers, odd characters) is refused. Returns it
/// tidied (no spaces), with a known import name turned into its pip name (`sklearn` becomes
/// `scikit-learn`).
pub fn check_requirement(text: &str) -> Result<String, String> {
    let t = text.trim();
    let shown = || t.chars().take(40).collect::<String>();
    if t.is_empty() {
        return Err("a package name is empty".into());
    }
    if t.chars().count() > MAX_REQUIREMENT_CHARS {
        return Err(format!("`{}…` is longer than {MAX_REQUIREMENT_CHARS} characters", shown()));
    }
    if t.starts_with('-') {
        return Err(format!("`{}` is an option; only package names can be installed", shown()));
    }
    if t.contains("://") || t.contains('@') {
        return Err(format!("`{}` is a URL; only package names from the package index can be installed", shown()));
    }
    if t.contains(['/', '\\']) || t.starts_with('.') || t.starts_with('~') {
        return Err(format!("`{}` is a path; only package names from the package index can be installed", shown()));
    }
    if t.contains(';') {
        return Err(format!("`{}` has an environment marker; give just the name and version", shown()));
    }
    let bad = || format!("`{}` isn't a package name with an optional version (such as pandas or numpy>=1.26)", shown());
    let name = requirement_name(t);
    if !is_name(name) {
        return Err(bad());
    }
    let mut out = match PIP_NAMES.iter().find(|(m, _)| *m == name) {
        Some((_, pip)) => (*pip).to_string(),
        None => name.to_string(),
    };
    let mut rest = t.get(name.len()..).unwrap_or_default().trim_start();
    if let Some(after) = rest.strip_prefix('[') {
        let Some((extras, tail)) = after.split_once(']') else { return Err(bad()) };
        let extras: Vec<&str> = extras.split(',').map(str::trim).collect();
        if extras.iter().any(|e| !is_name(e)) {
            return Err(format!("`{}` has extras with characters a package name can't have", shown()));
        }
        out.push('[');
        out.push_str(&extras.join(","));
        out.push(']');
        rest = tail.trim_start();
    }
    if rest.is_empty() {
        return Ok(out);
    }
    let mut clauses = Vec::new();
    for clause in rest.split(',') {
        let c = clause.trim();
        let op = ["~=", "==", "!=", "<=", ">=", "<", ">"].into_iter().find(|op| c.starts_with(op)).ok_or_else(bad)?;
        let version = c.get(op.len()..).unwrap_or_default().trim();
        let wildcard = version.strip_suffix(".*");
        let core = wildcard.unwrap_or(version);
        let valid = !core.is_empty()
            && core.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && core.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '!' | '-' | '_'));
        if !valid || (wildcard.is_some() && !matches!(op, "==" | "!=")) || version.starts_with('=') {
            return Err(bad());
        }
        clauses.push(format!("{op}{version}"));
    }
    out.push_str(&clauses.join(","));
    Ok(out)
}

/// Checks a list of requirements: at most [`MAX_PACKAGES`], each by [`check_requirement`],
/// duplicates (by name) dropped.
pub fn check_requirements<S: AsRef<str>>(list: &[S]) -> Result<Vec<String>, String> {
    if list.len() > MAX_PACKAGES {
        return Err(format!("At most {MAX_PACKAGES} packages can be listed at once ({} given)", list.len()));
    }
    let mut out: Vec<String> = Vec::new();
    for item in list {
        let req = check_requirement(item.as_ref())?;
        if !out.iter().any(|r| normalize(requirement_name(r)) == normalize(requirement_name(&req))) {
            out.push(req);
        }
    }
    Ok(out)
}

/// Splits text a person typed (commas, spaces or new lines between packages) into requirements.
/// A comma inside a version specifier (`numpy>=1,<3`) stays with it.
pub fn split_requirements(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split(|c: char| c.is_whitespace() || c == ',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let continues = ["~=", "==", "!=", "<=", ">=", "<", ">"].iter().any(|op| part.starts_with(op));
        match out.last_mut() {
            // `numpy>=1,<3` or `numpy >= 1`: the specifier belongs to the package before it.
            Some(last) if continues || last.ends_with(['=', '<', '>', '~', '!']) => {
                if continues && !last.ends_with(['=', '<', '>', '~', '!']) && last.contains(['=', '<', '>']) {
                    last.push(',');
                }
                last.push_str(part);
            }
            _ => out.push(part.to_string()),
        }
    }
    out
}

/// The module a `ModuleNotFoundError` message names (`No module named 'x'` or `"x"`).
pub fn missing_module(message: &str) -> Option<String> {
    let rest = message.split_once("No module named ")?.1;
    let quote = rest.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let name = rest.get(1..)?.split(quote).next()?;
    let top = name.split('.').next().unwrap_or_default();
    (!top.is_empty() && top.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| top.to_string())
}

/// Where packages go.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// The person's own virtual environment (or conda environment) whose python runs cells.
    Own { python: PathBuf, conda: bool },
    /// GridCraft's environment at `dir`, made from `base` when it doesn't exist yet.
    GridCraft { dir: PathBuf, base: PathBuf },
}

impl Target {
    /// The environment's folder or program, for the approval prompt.
    pub fn describe(&self) -> String {
        match self {
            Target::Own { python, .. } => python.display().to_string(),
            Target::GridCraft { dir, .. } => dir.display().to_string(),
        }
    }
}

/// One install: where, what, and with which tool.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub target: Target,
    pub packages: Vec<String>,
    /// `uv`, when it is on PATH: it makes environments and installs faster than pip.
    pub uv: Option<PathBuf>,
    pub os: Os,
}

/// What one step of an install does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    CreateEnv,
    Install,
    /// After the install: import what was installed once, in the environment's Python, so the
    /// first import in a cell is fast (a fresh package's native libraries load slowly the first
    /// time, which on macOS includes checking each one). A failure here is only a warning.
    Warmup,
}

/// The script the warm-up runs: imports each module named in its arguments, saying how long each
/// took, and exits with 1 when one failed.
pub const WARMUP_SCRIPT: &str = "import importlib, sys, time
failed = 0
for name in sys.argv[1:]:
    t = time.perf_counter()
    try:
        importlib.import_module(name)
        print('imported %s in %.1f s' % (name, time.perf_counter() - t), flush=True)
    except BaseException as e:
        failed += 1
        print('could not import %s: %s: %s' % (name, type(e).__name__, e), flush=True)
sys.exit(1 if failed else 0)
";

/// The modules to import to warm up `packages` (requirements): the known import name of each
/// (`sklearn` for scikit-learn), else its name with `-` and `.` as `_`, in lower case; only
/// names that are Python identifiers.
pub fn import_names(packages: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for req in packages {
        let name = requirement_name(req);
        let module = PIP_NAMES
            .iter()
            .find(|(_, pip)| normalize(pip) == normalize(name))
            .map(|(m, _)| (*m).to_string())
            .unwrap_or_else(|| name.replace(['-', '.'], "_").to_ascii_lowercase());
        let ok = module.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && module.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if ok && !out.contains(&module) {
            out.push(module);
        }
    }
    out
}

/// A program to run, with its arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub kind: StepKind,
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl Step {
    /// The command line, for the log.
    pub fn display(&self) -> String {
        let mut s = self.program.display().to_string();
        for a in &self.args {
            s.push(' ');
            // A script given with `-c` shows as a placeholder (the warm-up's).
            let a = a.to_string_lossy();
            s.push_str(if a.contains('\n') { "<script>" } else { &a });
        }
        s
    }
}

/// The programs an install runs, in order: make GridCraft's environment if it isn't there
/// (`create`), then install the packages (if any) with `--only-binary`, so nothing is built
/// from source.
pub fn steps(plan: &Plan, create: bool) -> Vec<Step> {
    let os = |s: &str| OsString::from(s);
    let mut out = Vec::new();
    let python = match &plan.target {
        Target::Own { python, .. } => python.clone(),
        Target::GridCraft { dir, base } => {
            if create {
                let args = match &plan.uv {
                    // `--seed` puts pip in too, so the environment works without uv later.
                    Some(_) => vec![os("venv"), os("--seed"), os("--python"), base.clone().into_os_string(), dir.clone().into_os_string()],
                    None => vec![os("-m"), os("venv"), dir.clone().into_os_string()],
                };
                let program = plan.uv.clone().unwrap_or_else(|| base.clone());
                out.push(Step { kind: StepKind::CreateEnv, program, args });
            }
            env_python(dir, plan.os)
        }
    };
    if plan.packages.is_empty() {
        return out;
    }
    let conda = matches!(plan.target, Target::Own { conda: true, .. });
    let (program, mut args) = match (&plan.uv, conda) {
        (Some(uv), false) => {
            // `--compile-bytecode`: uv doesn't compile `.pyc` files by default (pip does), which
            // makes the first import of a large package slower.
            let python = python.clone().into_os_string();
            (uv.clone(), vec![os("pip"), os("install"), os("--python"), python, os("--only-binary"), os(":all:"), os("--compile-bytecode")])
        }
        _ => {
            let mut a = vec![os("-m"), os("pip"), os("install"), os("--only-binary=:all:"), os("--disable-pip-version-check"), os("--no-input")];
            // A conda environment isn't a virtualenv to pip; it was checked to be one anyway.
            if !conda {
                a.push(os("--require-virtualenv"));
            }
            (python.clone(), a)
        }
    };
    args.extend(plan.packages.iter().map(|p| os(p)));
    out.push(Step { kind: StepKind::Install, program, args });
    let modules = import_names(&plan.packages);
    if !modules.is_empty() {
        let mut args = vec![os("-c"), os(WARMUP_SCRIPT)];
        args.extend(modules.iter().map(|m| os(m)));
        out.push(Step { kind: StepKind::Warmup, program: python, args });
    }
    out
}

/// How an install ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstallOutcome {
    pub ok: bool,
    /// One line saying what happened, or why it failed.
    pub message: String,
    /// What the programs printed (the end of it), with the command lines.
    pub output: String,
    pub cancelled: bool,
    /// The install worked but something after it didn't (the warm-up's imports).
    pub warning: Option<String>,
}

/// What an install reports while `kind` runs for `packages` (`python.installStatus {stage}`),
/// when it isn't "Installing …".
pub fn stage_label(kind: StepKind, packages: &[String]) -> Option<String> {
    match kind {
        StepKind::Warmup => {
            let names = packages.iter().map(|p| requirement_name(p)).collect::<Vec<_>>().join(", ");
            Some(format!("Preparing {names}\u{2026}"))
        }
        StepKind::CreateEnv | StepKind::Install => None,
    }
}

/// Why a failed step failed, from what it printed.
pub fn explain(kind: StepKind, output: &str, packages: &[String]) -> String {
    // Tools wrap long lines: compare with every run of white space as one space.
    let o = output.to_ascii_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    let has = |needles: &[&str]| needles.iter().any(|n| o.contains(n));
    if kind == StepKind::Warmup {
        let line = output.lines().rev().find(|l| l.starts_with("could not import ")).map(|l| l.trim_start_matches("could not import ").to_string());
        return match line {
            Some(l) => format!("Installed, but importing {l}"),
            None => "Installed, but importing it once afterwards failed. Diagnostics has the details.".into(),
        };
    }
    if kind == StepKind::CreateEnv {
        if has(&["no module named venv", "ensurepip is not available", "no module named ensurepip"]) {
            return "This Python can't make virtual environments (its venv module is missing). Install the python3-venv package, or install uv, and try again.".into();
        }
        return "GridCraft couldn't make its Python environment. Diagnostics has the details.".into();
    }
    if has(&[
        "failed to establish a new connection",
        "temporary failure in name resolution",
        "nodename nor servname",
        "network is unreachable",
        "name or service not known",
        "connection refused",
        "connecterror",
        "connection error",
        "dns error",
        "failed to fetch",
        "error sending request",
        "could not fetch url",
        "timed out",
    ]) {
        return "No network: the package index couldn't be reached. Check the connection and try again.".into();
    }
    if has(&["no module named pip"]) {
        return "pip isn't installed in this Python environment. Install pip (python -m ensurepip) or uv, and try again.".into();
    }
    if has(&["no matching distribution found", "could not find a version that satisfies", "has no wheels", "no wheels with a matching", "no wheel"])
        || has(&["not found in the package registry", "no solution found when resolving"])
    {
        return format!(
            "No ready-made package (wheel) of {} is available for this computer and Python version, or the name is wrong. GridCraft installs only ready-made packages.",
            packages.join(", ")
        );
    }
    if has(&["externally-managed-environment", "externally managed"]) {
        return "This Python is managed by the system, so GridCraft won't install into it.".into();
    }
    "The install failed. Diagnostics has the details.".into()
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{Install, find_uv};

#[cfg(target_arch = "wasm32")]
pub use web::{Install, find_uv};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::VecDeque;
    use std::io::Read;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use process_wrap::std::{ChildWrapper, CommandWrap};

    use super::{INSTALL_SECONDS, InstallOutcome, OUTPUT_BYTES, Plan, Step, StepKind, WARMUP_SECONDS, explain, stage_label, steps};

    #[allow(clippy::disallowed_methods)] // native only, where the clock works
    fn now() -> Instant {
        Instant::now()
    }

    /// `uv` on PATH, if it is there.
    pub fn find_uv() -> Option<PathBuf> {
        let name = if cfg!(windows) { "uv.exe" } else { "uv" };
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
    }

    /// What the install is doing now, when that isn't installing (see [`stage_label`]).
    type Stage = Arc<Mutex<Option<String>>>;

    /// An install running on a thread of its own.
    pub struct Install {
        stage: Stage,
        rx: Option<Receiver<InstallOutcome>>,
        result: Option<InstallOutcome>,
        cancel: Arc<AtomicBool>,
        output: Arc<Mutex<VecDeque<u8>>>,
    }

    type Output = Arc<Mutex<VecDeque<u8>>>;

    fn push(out: &Output, bytes: &[u8]) {
        if let Ok(mut o) = out.lock() {
            o.extend(bytes);
            let extra = o.len().saturating_sub(OUTPUT_BYTES);
            o.drain(..extra);
        }
    }

    fn text(out: &Output) -> String {
        out.lock().map(|o| String::from_utf8_lossy(&o.iter().copied().collect::<Vec<u8>>()).into_owned()).unwrap_or_default()
    }

    /// Copies a pipe into the output on a thread, so a chatty program never stalls.
    fn drain<R: Read + Send + 'static>(input: R, out: Output) -> Option<std::thread::JoinHandle<()>> {
        std::thread::Builder::new()
            .name("python-install-output".into())
            .spawn(move || {
                let mut input = input;
                let mut buf = [0u8; 4096];
                loop {
                    match input.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => push(&out, buf.get(..n).unwrap_or_default()),
                    }
                }
            })
            .ok()
    }

    enum Ended {
        Ok,
        Failed(String),
        Cancelled,
        Deadline,
    }

    fn run_step(step: &Step, out: &Output, cancel: &AtomicBool, deadline: Instant) -> Ended {
        push(out, format!("$ {}\n", step.display()).as_bytes());
        let mut cmd = CommandWrap::with_new(&step.program, |c| {
            c.args(&step.args)
                .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
                .env("PYTHONUTF8", "1")
                .env("NO_COLOR", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        });
        #[cfg(unix)]
        cmd.wrap(process_wrap::std::ProcessGroup::leader());
        #[cfg(windows)]
        {
            cmd.wrap(process_wrap::std::CreationFlags(windows::Win32::System::Threading::CREATE_NO_WINDOW));
            cmd.wrap(process_wrap::std::JobObject);
        }
        let mut child: Box<dyn ChildWrapper> = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Ended::Failed(format!("{} didn't start ({e})", step.program.display())),
        };
        let readers: Vec<_> = [child.stdout().take().map(|s| drain(s, out.clone())), child.stderr().take().map(|s| drain(s, out.clone()))]
            .into_iter()
            .flatten()
            .flatten()
            .collect();
        let ended = loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break Ended::Ok,
                Ok(Some(status)) => break Ended::Failed(format!("it stopped with an error ({status})")),
                Ok(None) => {}
                Err(e) => break Ended::Failed(format!("it couldn't be checked ({e})")),
            }
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                break Ended::Cancelled;
            }
            if now() >= deadline {
                let _ = child.kill();
                break Ended::Deadline;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        for r in readers {
            let _ = r.join();
        }
        ended
    }

    fn run_all(plan: &Plan, list: &[Step], out: &Output, cancel: &AtomicBool, stage: &Stage) -> InstallOutcome {
        let deadline = now() + Duration::from_secs(INSTALL_SECONDS);
        let mut warning = None;
        for step in list {
            if let Ok(mut s) = stage.lock() {
                *s = stage_label(step.kind, &plan.packages);
            }
            if step.kind == StepKind::Warmup {
                // Its own deadline; however it ends, the packages are installed.
                warning = match run_step(step, out, cancel, now() + Duration::from_secs(WARMUP_SECONDS)) {
                    Ended::Ok => None,
                    Ended::Cancelled => Some("Installed; preparing it for its first import was stopped.".to_string()),
                    Ended::Deadline => Some(format!("Installed; importing it once afterwards took longer than {WARMUP_SECONDS} s and was stopped.")),
                    Ended::Failed(why) => {
                        push(out, format!("\n{why}\n").as_bytes());
                        Some(explain(step.kind, &text(out), &plan.packages))
                    }
                };
                continue;
            }
            match run_step(step, out, cancel, deadline) {
                Ended::Ok => {}
                Ended::Cancelled => {
                    return InstallOutcome {
                        ok: false,
                        message: "The install was cancelled.".into(),
                        output: text(out),
                        cancelled: true,
                        warning: None,
                    };
                }
                Ended::Deadline => {
                    return InstallOutcome {
                        ok: false,
                        message: format!("The install took longer than {} minutes and was stopped.", INSTALL_SECONDS / 60),
                        output: text(out),
                        cancelled: false,
                        warning: None,
                    };
                }
                Ended::Failed(why) => {
                    push(out, format!("\n{why}\n").as_bytes());
                    let output = text(out);
                    let message = explain(step.kind, &output, &plan.packages);
                    return InstallOutcome { ok: false, message, output, cancelled: false, warning: None };
                }
            }
        }
        let message = if plan.packages.is_empty() {
            format!("Made GridCraft's Python environment at {}.", plan.target.describe())
        } else {
            format!("Installed {} into {}.", plan.packages.join(", "), plan.target.describe())
        };
        InstallOutcome { ok: true, message, output: text(out), cancelled: false, warning }
    }

    impl Install {
        /// Starts `plan` (making GridCraft's environment first when `create`).
        pub fn start(plan: Plan, create: bool) -> Install {
            let list = steps(&plan, create);
            Install::run(plan, list)
        }

        /// Runs `list` for `plan` on a thread.
        pub(crate) fn run(plan: Plan, list: Vec<Step>) -> Install {
            let cancel = Arc::new(AtomicBool::new(false));
            let output: Output = Arc::new(Mutex::new(VecDeque::new()));
            let stage: Stage = Arc::new(Mutex::new(None));
            let (tx, rx) = mpsc::channel();
            let (c, o, st) = (cancel.clone(), output.clone(), stage.clone());
            let spawned = std::thread::Builder::new().name("python-install".into()).spawn(move || {
                let _ = tx.send(run_all(&plan, &list, &o, &c, &st));
            });
            match spawned {
                Ok(_) => Install { stage, rx: Some(rx), result: None, cancel, output },
                Err(e) => Install::finished(InstallOutcome {
                    ok: false,
                    message: format!("GridCraft couldn't start the install: {e}"),
                    output: String::new(),
                    cancelled: false,
                    warning: None,
                }),
            }
        }

        /// An install that already ended (tests, and when it couldn't start).
        pub fn finished(outcome: InstallOutcome) -> Install {
            Install {
                stage: Arc::new(Mutex::new(None)),
                rx: None,
                result: Some(outcome),
                cancel: Arc::new(AtomicBool::new(false)),
                output: Arc::new(Mutex::new(VecDeque::new())),
            }
        }

        /// What it is doing now, when that isn't installing ("Preparing scikit-learn…").
        pub fn stage(&self) -> Option<String> {
            if self.result.is_some() {
                return None;
            }
            self.stage.lock().ok().and_then(|s| s.clone())
        }

        /// Asks the running programs to stop; [`Install::poll`] then reports it cancelled.
        pub fn cancel(&self) {
            self.cancel.store(true, Ordering::Relaxed);
        }

        /// What it printed so far.
        pub fn output(&self) -> String {
            text(&self.output)
        }

        /// The outcome, once it has ended.
        pub fn poll(&mut self) -> Option<&InstallOutcome> {
            if self.result.is_none()
                && let Some(rx) = &self.rx
            {
                match rx.try_recv() {
                    Ok(o) => self.result = Some(o),
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.result = Some(InstallOutcome {
                            ok: false,
                            message: "The install stopped unexpectedly.".into(),
                            output: self.output(),
                            cancelled: false,
                            warning: None,
                        })
                    }
                }
            }
            self.result.as_ref()
        }

        /// Waits up to `max` for the outcome.
        pub fn wait(&mut self, max: Duration) -> Option<&InstallOutcome> {
            if self.result.is_none()
                && let Some(rx) = &self.rx
                && let Ok(o) = rx.recv_timeout(max)
            {
                self.result = Some(o);
            }
            self.poll()
        }
    }
}

/// The web build installs nothing (Python cells are `#BLOCKED!` there).
#[cfg(target_arch = "wasm32")]
mod web {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{InstallOutcome, Plan};

    pub fn find_uv() -> Option<PathBuf> {
        None
    }

    pub struct Install {
        result: InstallOutcome,
    }

    impl Install {
        pub fn start(_plan: Plan, _create: bool) -> Install {
            Install::finished(InstallOutcome {
                ok: false,
                message: "Python isn't available in the web version".into(),
                output: String::new(),
                cancelled: false,
                warning: None,
            })
        }
        pub fn finished(result: InstallOutcome) -> Install {
            Install { result }
        }
        pub fn cancel(&self) {}
        pub fn stage(&self) -> Option<String> {
            None
        }
        pub fn output(&self) -> String {
            String::new()
        }
        pub fn poll(&mut self) -> Option<&InstallOutcome> {
            Some(&self.result)
        }
        pub fn wait(&mut self, _max: Duration) -> Option<&InstallOutcome> {
            Some(&self.result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| m.get(k).map(OsString::from)
    }

    fn norm(p: &Path) -> String {
        p.display().to_string().replace('\\', "/")
    }

    #[test]
    fn the_environment_lives_in_the_data_folder_of_each_system() {
        let v = vars(&[("HOME", "/Users/u"), ("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local")]);
        assert_eq!(norm(&default_dir(&v, Os::MacOs).unwrap()), "/Users/u/Library/Application Support/GridCraft/python-env");
        assert_eq!(norm(&default_dir(&v, Os::Windows).unwrap()), "C:/Users/u/AppData/Local/GridCraft/python-env");
        assert_eq!(norm(&default_dir(&v, Os::Unix).unwrap()), "/Users/u/.local/share/gridcraft/python-env");
        let v = vars(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/data")]);
        assert_eq!(norm(&default_dir(&v, Os::Unix).unwrap()), "/data/gridcraft/python-env");
        assert_eq!(default_dir(&vars(&[]), Os::MacOs), None);
        assert_eq!(default_dir(&vars(&[("XDG_DATA_HOME", "")]), Os::Unix), None);
        assert_eq!(norm(&env_dir(" /tmp/x ").unwrap()), "/tmp/x", "Python Options wins");
        assert_eq!(norm(&env_python(Path::new("/e"), Os::Unix)), "/e/bin/python");
        assert_eq!(norm(&env_python(Path::new("C:\\e"), Os::Windows)), "C:\\e/Scripts/python.exe".replace('\\', "/"));
    }

    #[test]
    fn requirements_are_names_with_an_optional_version_only() {
        for (given, want) in [
            ("pandas", "pandas"),
            (" numpy >= 1.26 , < 3 ", "numpy>=1.26,<3"),
            ("requests[socks, security]==2.32.3", "requests[socks,security]==2.32.3"),
            ("Django~=5.0", "Django~=5.0"),
            ("scipy==1.*", "scipy==1.*"),
            ("zope.interface", "zope.interface"),
            ("sklearn", "scikit-learn"),
            ("PIL", "pillow"),
            ("cv2>=4", "opencv-python>=4"),
            ("torch==2.4.0+cpu", "torch==2.4.0+cpu"),
        ] {
            assert_eq!(check_requirement(given).as_deref(), Ok(want), "{given}");
        }
        for (bad, why) in [
            ("", "empty"),
            ("-r requirements.txt", "option"),
            ("--index-url=http://evil", "option"),
            ("https://example.com/x.whl", "URL"),
            ("pkg @ https://example.com/pkg.zip", "URL"),
            ("git+ssh://git@host/repo", "URL"),
            ("./local/pkg", "path"),
            ("C:\\pkgs\\x.whl", "path"),
            ("/tmp/x.whl", "path"),
            ("~/x", "path"),
            ("numpy; python_version<'3.8'", "marker"),
            ("pkg[a$b]", "extras"),
            ("pkg[a", "isn't a package name"),
            ("pandas numpy", "isn't a package name"),
            ("pandas>=", "isn't a package name"),
            ("pandas===1.0", "isn't a package name"),
            ("pandas>=1.*", "isn't a package name"),
            ("_private", "isn't a package name"),
            ("pandas$(rm -rf ~)", "isn't a package name"),
            ("pan\u{0434}as", "isn't a package name"),
            ("pandas|cat", "isn't a package name"),
        ] {
            let e = check_requirement(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
        assert!(check_requirement(&"a".repeat(MAX_REQUIREMENT_CHARS + 1)).unwrap_err().contains("longer than"));
        let many: Vec<String> = (0..=MAX_PACKAGES).map(|i| format!("p{i}")).collect();
        assert!(check_requirements(&many).unwrap_err().contains("At most 32"));
        assert_eq!(check_requirements(&["pandas", "Pandas>=2", "numpy"]).unwrap(), ["pandas", "numpy"], "duplicates by name go");
    }

    #[test]
    fn installed_versions_come_from_uv_and_pip_output() {
        let uv =
            "Resolved 7 packages in 386ms\nDownloading scipy (19.5MiB)\nInstalled 7 packages in 33ms\n + joblib==1.6.0\n + scikit-learn==1.9.1\n";
        assert_eq!(installed_versions(uv), [("joblib".to_string(), "1.6.0".to_string()), ("scikit-learn".to_string(), "1.9.1".to_string())]);
        let pip = "Collecting six\nInstalling collected packages: six, threadpoolctl\nSuccessfully installed six-1.17.0 threadpool_ctl-3.7.0\n";
        assert_eq!(installed_versions(pip), [("six".to_string(), "1.17.0".to_string()), ("threadpool_ctl".to_string(), "3.7.0".to_string())]);
        let already = "Requirement already satisfied: six>=1 in /e/lib/python3.14/site-packages (1.17.0)\n";
        assert_eq!(installed_versions(already), [("six".to_string(), "1.17.0".to_string())]);
        assert!(installed_versions("Audited 1 package in 2ms\n + \n+ x==\nSuccessfully installed\n").is_empty());
        assert_eq!(norm(&env_root(Path::new("/e/bin/python")).unwrap()), "/e");
        assert_eq!(norm(&env_root(Path::new("/c/envs/x/python")).unwrap()), "/c/envs/x");
    }

    #[test]
    fn typed_lists_split_and_names_compare_like_pip() {
        assert_eq!(split_requirements("pandas, numpy\nscikit-learn"), ["pandas", "numpy", "scikit-learn"]);
        assert_eq!(split_requirements("numpy>=1,<3 pandas"), ["numpy>=1,<3", "pandas"]);
        assert_eq!(split_requirements("numpy >= 1.26"), ["numpy>=1.26"]);
        assert_eq!(normalize("Python_Dateutil"), "python-dateutil");
        assert_eq!(normalize("zope..interface"), "zope-interface");
        assert_eq!(pip_name("sklearn.linear_model"), "scikit-learn");
        assert_eq!(pip_name("yaml"), "pyyaml");
        assert_eq!(pip_name("polars"), "polars");
        assert_eq!(missing_module("ModuleNotFoundError: No module named 'sklearn.linear_model'").as_deref(), Some("sklearn"));
        assert_eq!(missing_module("No module named \"bs4\""), Some("bs4".into()));
        assert_eq!(missing_module("NameError: x"), None);
        assert_eq!(missing_module("No module named '../x'"), None);
    }

    #[test]
    fn installs_run_these_programs() {
        let gc = Target::GridCraft { dir: "/env".into(), base: "/usr/bin/python3".into() };
        let plan = Plan { target: gc.clone(), packages: vec!["pandas".into(), "numpy>=2".into()], uv: None, os: Os::Unix };
        let s = steps(&plan, true);
        assert_eq!(
            s.iter().map(Step::display).collect::<Vec<_>>(),
            [
                "/usr/bin/python3 -m venv /env",
                "/env/bin/python -m pip install --only-binary=:all: --disable-pip-version-check --no-input --require-virtualenv pandas numpy>=2",
                "/env/bin/python -c <script> pandas numpy",
            ]
        );
        let plan = Plan { uv: Some("/bin/uv".into()), ..plan };
        assert_eq!(
            steps(&plan, true).iter().map(Step::display).collect::<Vec<_>>(),
            [
                "/bin/uv venv --seed --python /usr/bin/python3 /env",
                "/bin/uv pip install --python /env/bin/python --only-binary :all: --compile-bytecode pandas numpy>=2",
                "/env/bin/python -c <script> pandas numpy",
            ]
        );
        assert_eq!(steps(&plan, false).len(), 2, "an environment that exists isn't made again");
        let created = Plan { packages: vec![], ..plan.clone() };
        assert_eq!(steps(&created, true).iter().map(|s| s.kind).collect::<Vec<_>>(), [StepKind::CreateEnv]);
        let conda = Plan { target: Target::Own { python: "/c/bin/python".into(), conda: true }, ..plan };
        assert_eq!(
            steps(&conda, true).iter().map(Step::display).collect::<Vec<_>>(),
            [
                "/c/bin/python -m pip install --only-binary=:all: --disable-pip-version-check --no-input pandas numpy>=2",
                "/c/bin/python -c <script> pandas numpy",
            ]
        );
    }

    #[test]
    fn the_warm_up_imports_each_package_by_its_module_name() {
        let names = |l: &[&str]| import_names(&l.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(names(&["scikit-learn>=1.5", "Pillow", "PyYAML", "python-dateutil"]), ["sklearn", "PIL", "yaml", "dateutil"]);
        assert_eq!(
            names(&["pandas", "numpy==2.1", "pandas", "typing-extensions", "zope.interface"]),
            ["pandas", "numpy", "typing_extensions", "zope_interface"]
        );
        assert!(names(&["9lives", ""]).is_empty(), "only identifiers");
        assert_eq!(stage_label(StepKind::Warmup, &["scikit-learn>=1".into()]).as_deref(), Some("Preparing scikit-learn\u{2026}"));
        assert_eq!(stage_label(StepKind::Install, &["pandas".into()]), None);
        assert_eq!(
            explain(StepKind::Warmup, "imported numpy in 0.2 s\ncould not import sklearn: ImportError: dlopen failed\n", &[]),
            "Installed, but importing sklearn: ImportError: dlopen failed"
        );
    }

    /// The warm-up after an install: its failure is a warning on an install that worked, and
    /// while it runs the install says it is preparing the packages.
    #[test]
    #[cfg(unix)]
    fn a_failed_warm_up_is_a_warning_not_a_failed_install() {
        let plan =
            Plan { target: Target::Own { python: "/bin/sh".into(), conda: false }, packages: vec!["scikit-learn".into()], uv: None, os: Os::Unix };
        let ok = Step { kind: StepKind::Install, program: "/bin/sh".into(), args: vec!["-c".into(), "exit 0".into()] };
        let warm = |script: &str| Step { kind: StepKind::Warmup, program: "/bin/sh".into(), args: vec!["-c".into(), script.into()] };
        let mut i = Install::run(plan.clone(), vec![ok.clone(), warm("echo 'imported sklearn in 0.1 s'")]);
        let o = i.wait(Duration::from_secs(20)).cloned().unwrap();
        assert!(o.ok && o.warning.is_none(), "{o:?}");
        assert!(o.output.contains("imported sklearn"), "{o:?}");
        let mut i = Install::run(plan.clone(), vec![ok.clone(), warm("echo 'could not import sklearn: ImportError: boom'; exit 1")]);
        let o = i.wait(Duration::from_secs(20)).cloned().unwrap();
        assert!(o.ok && !o.cancelled, "the install still worked: {o:?}");
        assert_eq!(o.warning.as_deref(), Some("Installed, but importing sklearn: ImportError: boom"));
        // While it runs, it says so.
        let mut i = Install::run(plan, vec![ok, warm("sleep 60")]);
        let mut stage = None;
        for _ in 0..200 {
            stage = i.stage();
            if stage.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(stage.as_deref(), Some("Preparing scikit-learn\u{2026}"));
        i.cancel();
        let o = i.wait(Duration::from_secs(20)).cloned().unwrap();
        assert!(o.ok && o.warning.is_some(), "{o:?}");
        assert_eq!(i.stage(), None, "over");
    }

    #[test]
    fn failures_say_why() {
        let p = ["pandas".to_string()];
        assert!(
            explain(StepKind::Install, "WARNING: Retrying ... Failed to establish a new connection: [Errno 8] nodename nor servname provided", &p)
                .starts_with("No network")
        );
        assert!(explain(StepKind::Install, "error: Failed to fetch: `https://pypi.org/simple/pandas/`", &p).starts_with("No network"));
        assert!(
            explain(StepKind::Install, "ERROR: No matching distribution found for pandas", &p).starts_with("No ready-made package (wheel) of pandas")
        );
        assert!(explain(StepKind::Install, "/x/python: No module named pip", &p).starts_with("pip isn't installed"));
        assert!(explain(StepKind::CreateEnv, "Error: No module named venv", &p).contains("venv module is missing"));
        assert!(explain(StepKind::Install, "something else", &p).starts_with("The install failed"));
    }

    /// A real install run: a program that fails, then one that is cancelled.
    #[test]
    #[cfg(unix)]
    fn an_install_reports_failure_and_can_be_cancelled() {
        let plan = Plan { target: Target::Own { python: "/bin/sh".into(), conda: false }, packages: vec!["pandas".into()], uv: None, os: Os::Unix };
        // `/bin/sh -m pip …` fails at once.
        let mut i = Install::start(plan.clone(), false);
        let o = i.wait(Duration::from_secs(20)).cloned().unwrap();
        assert!(!o.ok && !o.cancelled, "{o:?}");
        assert!(o.output.starts_with("$ /bin/sh -m pip install"), "{o:?}");
        // A step that would run for a minute stops when cancelled.
        let sleep = Step { kind: StepKind::Install, program: "/bin/sleep".into(), args: vec!["60".into()] };
        let mut i = Install::run(plan, vec![sleep]);
        assert!(i.poll().is_none(), "still running");
        i.cancel();
        let o = i.wait(Duration::from_secs(20)).cloned().unwrap();
        assert!(!o.ok && o.cancelled, "{o:?}");
        assert_eq!(o.message, "The install was cancelled.");
    }

    // Only the tests that run real programs (Unix) wait.
    #[cfg(unix)]
    use std::time::Duration;
}
