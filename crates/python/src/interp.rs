//! Finding the user's own Python (CPython 3.11 or newer), which runs Python cells when it is
//! there, so packages such as pandas and numpy work. GridCraft never installs an interpreter or
//! packages: it looks for one, or uses the path the user set in Python Options.
//!
//! Search order: the path from Python Options; an active virtual environment (`VIRTUAL_ENV`, then
//! `CONDA_PREFIX`); `python3` and then `python` on `PATH`; on Windows, the `py -3` launcher and
//! then the usual install folders. On Windows the Microsoft Store shortcuts under
//! `%LOCALAPPDATA%\Microsoft\WindowsApps` are skipped: they open the Store instead of running
//! Python. Each candidate is checked by running it with a small probe under a 5 s deadline.
//!
//! The search itself is pure (it asks a [`Host`] for environment variables, files and probes), so
//! tests run it against fake candidates. It never runs on the web build.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The oldest Python that runs cells (`-P`, which keeps the current folder off `sys.path`, is
/// new in 3.11).
pub const MIN_VERSION: (u32, u32) = (3, 11);

/// How long a candidate may take to answer the probe.
pub const PROBE_SECONDS: u64 = 5;

/// What a probe learned about an interpreter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    /// `sys.executable`: the program cells run with (a virtual environment's own python, or the
    /// real program behind the `py` launcher). Empty when the interpreter didn't say.
    pub executable: PathBuf,
    pub version: (u32, u32, u32),
    pub pandas: Option<String>,
    pub numpy: Option<String>,
    /// The interpreter is managed by the system (PEP 668: Homebrew, Debian…), so `pip install`
    /// into it is refused and packages belong in a virtual environment.
    pub externally_managed: bool,
}

/// A program to try.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub program: PathBuf,
    /// Arguments before Python's own (`-3` for the `py` launcher).
    pub args: Vec<String>,
    /// Where it was found: "Python Options", "VIRTUAL_ENV", "PATH"…
    pub source: String,
}

/// The interpreter cells run with.
#[derive(Clone, Debug, PartialEq)]
pub struct Interpreter {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub version: String,
    pub pandas: Option<String>,
    pub numpy: Option<String>,
    pub externally_managed: bool,
    pub source: String,
}

/// The worker for the user's Python: a small program speaking the same protocol as the built-in
/// worker. It is data the user's interpreter runs (a documented exception to GridCraft's
/// Rust-only rule), passed with `-c`.
pub const SHIM: &str = include_str!("shim.py");

impl Interpreter {
    /// How to start a worker on this interpreter: `-P` keeps the current folder off `sys.path`,
    /// `-X utf8` makes text UTF-8 everywhere and `-u` unbuffers the output. Not `-I` or `-s`: they
    /// hide the user's site-packages, where `pip install --user` puts pandas.
    pub fn launcher(&self) -> crate::run::Launcher {
        let mut args = self.args.clone();
        args.extend(["-P", "-X", "utf8", "-u", "-c", SHIM].map(String::from));
        crate::run::Launcher::Process { program: self.program.clone(), args, env: vec![] }
    }

    /// `3.14` from `3.14.0`.
    pub fn short_version(&self) -> String {
        self.version.split('.').take(2).collect::<Vec<_>>().join(".")
    }
}

/// A candidate that can't run cells, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    pub path: String,
    pub reason: String,
}

/// The result of a search.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detection {
    pub found: Option<Interpreter>,
    pub rejected: Vec<Rejected>,
}

/// What the search needs from the machine.
pub trait Host {
    fn var(&self, key: &str) -> Option<OsString>;
    fn is_file(&self, path: &Path) -> bool;
    /// The entries of a folder (empty when it can't be read).
    fn list_dir(&self, path: &Path) -> Vec<PathBuf>;
    /// Runs the probe with `program args`: the interpreter's version and, for 3.11 or newer, the
    /// rest of [`Probe`]. `Err` says why it didn't answer.
    fn probe(&self, program: &Path, args: &[String]) -> Result<Probe, String>;
    fn windows(&self) -> bool;
}

static SEARCH: AtomicBool = AtomicBool::new(false);

/// Lets sessions look for the user's Python. The apps call it at start-up; tests and library
/// users that don't call it keep the built-in Python (or the launcher they set).
pub fn enable_search() {
    SEARCH.store(true, Ordering::Relaxed);
}

/// Whether sessions look for the user's Python.
pub fn search_enabled() -> bool {
    SEARCH.load(Ordering::Relaxed) && crate::AVAILABLE
}

fn exe(host: &dyn Host, name: &str) -> String {
    if host.windows() { format!("{name}.exe") } else { name.to_string() }
}

/// `name` on `PATH`, if a file of that name is in one of its folders.
fn on_path(host: &dyn Host, name: &str) -> Option<PathBuf> {
    let path = host.var("PATH")?;
    split_path(host, &path).into_iter().map(|dir| dir.join(name)).find(|p| host.is_file(p))
}

/// `PATH`'s folders. Split by hand so a fake Windows host works on any machine.
fn split_path(host: &dyn Host, path: &OsString) -> Vec<PathBuf> {
    let sep = if host.windows() { ';' } else { ':' };
    path.to_string_lossy().split(sep).filter(|s| !s.trim().is_empty()).map(|s| PathBuf::from(s.trim().trim_matches('"'))).collect()
}

/// Whether `path` is one of the Microsoft Store shortcuts, which open the Store instead of running
/// Python.
pub fn is_store_stub(host: &dyn Host, path: &Path) -> bool {
    if !host.windows() {
        return false;
    }
    let Some(local) = host.var("LOCALAPPDATA") else { return false };
    let stubs = PathBuf::from(local).join("Microsoft").join("WindowsApps");
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
    let (path, stubs) = (norm(path), norm(&stubs));
    path.strip_prefix(&stubs).is_some_and(|rest| rest.starts_with('\\'))
}

fn same_path(host: &dyn Host, a: &Path, b: &Path) -> bool {
    if host.windows() { a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) } else { a == b }
}

/// The Python versions in folder names such as `Python312` or `Python3.12`, newest first.
fn install_folders(host: &dyn Host, root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(u32, PathBuf)> = host
        .list_dir(root)
        .into_iter()
        .filter_map(|dir| {
            let name = dir.file_name()?.to_string_lossy().to_ascii_lowercase();
            let digits: String = name.strip_prefix("python3")?.chars().filter(char::is_ascii_digit).collect();
            Some((digits.parse().ok()?, dir))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, d)| d.join("python.exe")).collect()
}

/// The programs to try, in order, and the ones skipped without running them.
pub fn candidates(host: &dyn Host, configured: &str) -> (Vec<Candidate>, Vec<Rejected>) {
    let mut out: Vec<Candidate> = Vec::new();
    let mut rejected = Vec::new();
    let add = |out: &mut Vec<Candidate>, rejected: &mut Vec<Rejected>, program: PathBuf, args: &[&str], source: &str| {
        if is_store_stub(host, &program) {
            rejected.push(Rejected {
                path: program.display().to_string(),
                reason: "it is the Microsoft Store shortcut, which opens the Store instead of running Python".into(),
            });
            return;
        }
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        if !out.iter().any(|c| same_path(host, &c.program, &program) && c.args == args) {
            out.push(Candidate { program, args, source: source.into() });
        }
    };
    let configured = configured.trim();
    if !configured.is_empty() {
        let given = PathBuf::from(configured);
        // A bare name (`python3.12`) is looked up on PATH.
        let bare = given.components().count() == 1 && !configured.contains(['/', '\\']);
        let resolved = if bare { on_path(host, configured).or_else(|| on_path(host, &exe(host, configured))) } else { Some(given.clone()) };
        match resolved.filter(|p| host.is_file(p)) {
            Some(p) => add(&mut out, &mut rejected, p, &[], "Python Options"),
            None => rejected.push(Rejected { path: configured.to_string(), reason: "the file doesn't exist".into() }),
        }
    }
    for (var, unix, windows) in [("VIRTUAL_ENV", "bin/python", "Scripts\\python.exe"), ("CONDA_PREFIX", "bin/python3", "python.exe")] {
        let Some(root) = host.var(var).filter(|v| !v.is_empty()) else { continue };
        let program = PathBuf::from(root).join(if host.windows() { windows } else { unix });
        if host.is_file(&program) {
            add(&mut out, &mut rejected, program, &[], var);
        } else {
            rejected.push(Rejected { path: program.display().to_string(), reason: format!("{var} is set, but this file doesn't exist") });
        }
    }
    for name in ["python3", "python"] {
        let Some(path) = host.var("PATH") else { break };
        for dir in split_path(host, &path) {
            let p = dir.join(exe(host, name));
            if host.is_file(&p) {
                add(&mut out, &mut rejected, p, &[], "PATH");
                // The first match on PATH is what a terminal runs; later ones are shadowed.
                break;
            }
        }
    }
    if host.windows() {
        if let Some(py) = on_path(host, "py.exe") {
            add(&mut out, &mut rejected, py, &["-3"], "py launcher");
        }
        let roots = [host.var("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Programs").join("Python")), host.var("ProgramFiles").map(PathBuf::from)];
        for root in roots.into_iter().flatten() {
            for p in install_folders(host, &root) {
                if host.is_file(&p) {
                    add(&mut out, &mut rejected, p, &[], "installed");
                }
            }
        }
    }
    (out, rejected)
}

fn version_text(v: (u32, u32, u32)) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// Finds the interpreter cells run with: the first candidate that answers the probe with Python
/// 3.11 or newer. Every other candidate tried, or skipped, is listed with its reason.
pub fn detect_with(host: &dyn Host, configured: &str) -> Detection {
    let (cands, mut rejected) = candidates(host, configured);
    for c in cands {
        let shown = if c.args.is_empty() { c.program.display().to_string() } else { format!("{} {}", c.program.display(), c.args.join(" ")) };
        match host.probe(&c.program, &c.args) {
            Err(reason) => rejected.push(Rejected { path: shown, reason }),
            Ok(p) if (p.version.0, p.version.1) < MIN_VERSION => rejected.push(Rejected {
                path: shown,
                reason: format!("it is Python {}; Python cells need {}.{} or newer", version_text(p.version), MIN_VERSION.0, MIN_VERSION.1),
            }),
            Ok(p) => {
                // The program the interpreter reports (behind `py -3`, or a virtual environment's
                // own python); the candidate itself when it didn't say.
                let (program, args) = if p.executable.as_os_str().is_empty() { (c.program, c.args) } else { (p.executable, Vec::new()) };
                let found = Interpreter {
                    program,
                    args,
                    version: version_text(p.version),
                    pandas: p.pandas,
                    numpy: p.numpy,
                    externally_managed: p.externally_managed,
                    source: c.source,
                };
                return Detection { found: Some(found), rejected };
            }
        }
    }
    Detection { found: None, rejected }
}

/// Searches this machine.
pub fn detect(configured: &str) -> Detection {
    #[cfg(not(target_arch = "wasm32"))]
    {
        detect_with(&native::Machine, configured)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = configured;
        Detection::default()
    }
}

/// A search running on its own thread, so a slow candidate never holds up a frame.
pub struct Detector {
    rx: Option<std::sync::mpsc::Receiver<Detection>>,
    result: Option<Detection>,
}

impl Detector {
    /// Starts a search of this machine.
    pub fn start(configured: String) -> Detector {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let spawned = std::thread::Builder::new().name("python-detect".into()).spawn(move || {
                let _ = tx.send(detect(&configured));
            });
            match spawned {
                Ok(_) => Detector { rx: Some(rx), result: None },
                Err(e) => Detector::ready(Detection {
                    found: None,
                    rejected: vec![Rejected { path: String::new(), reason: format!("GridCraft couldn't start looking for Python: {e}") }],
                }),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = configured;
            Detector::ready(Detection::default())
        }
    }

    /// A finished search (tests, and the web build).
    pub fn ready(d: Detection) -> Detector {
        Detector { rx: None, result: Some(d) }
    }

    /// The result, once the search has finished.
    pub fn poll(&mut self) -> Option<&Detection> {
        if self.result.is_none()
            && let Some(rx) = &self.rx
        {
            match rx.try_recv() {
                Ok(d) => self.result = Some(d),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.result = Some(Detection::default()),
            }
        }
        self.result.as_ref()
    }

    /// Waits up to `max` for the result.
    pub fn wait(&mut self, max: std::time::Duration) -> Option<&Detection> {
        if self.result.is_none()
            && let Some(rx) = &self.rx
        {
            match rx.recv_timeout(max) {
                Ok(d) => self.result = Some(d),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => self.result = Some(Detection::default()),
            }
        }
        self.result.as_ref()
    }
}

/// Prints the version (`-I`: nothing from the environment or the current folder is imported; it
/// runs on any Python 3).
pub const VERSION_PROBE: &str = "import sys; print('%d.%d.%d' % sys.version_info[:3])";

/// Prints a JSON report (run with `-P` on 3.11 or newer). Package versions come from their
/// metadata, so pandas isn't imported (which takes a second).
pub const FULL_PROBE: &str = r#"
import json, os, sys, sysconfig, importlib.util, importlib.metadata as md
def ver(name):
    try:
        if importlib.util.find_spec(name) is None:
            return None
    except Exception:
        return None
    try:
        return md.version(name)
    except Exception:
        return "unknown version"
stdlib = sysconfig.get_path("stdlib") or ""
venv = sys.prefix != getattr(sys, "base_prefix", sys.prefix)
managed = (not venv) and os.path.isfile(os.path.join(stdlib, "EXTERNALLY-MANAGED"))
print(json.dumps({"executable": sys.executable or "", "version": list(sys.version_info[:3]), "pandas": ver("pandas"), "numpy": ver("numpy"), "externallyManaged": managed}))
"#;

/// Parses `3.14.7`.
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut it = text.trim().split('.').map(|p| p.trim().parse::<u32>());
    Some((it.next()?.ok()?, it.next()?.ok()?, it.next().and_then(Result::ok).unwrap_or(0)))
}

/// Parses [`FULL_PROBE`]'s report.
pub fn parse_report(text: &str) -> Option<Probe> {
    let v: serde_json::Value = serde_json::from_str(text.lines().rev().find(|l| l.trim_start().starts_with('{'))?).ok()?;
    let version = v.get("version")?.as_array()?;
    let part = |i: usize| version.get(i).and_then(serde_json::Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let s = |k: &str| v.get(k).and_then(serde_json::Value::as_str).map(str::to_string);
    Some(Probe {
        executable: PathBuf::from(s("executable").unwrap_or_default()),
        version: (part(0)?, part(1)?, part(2).unwrap_or(0)),
        pandas: s("pandas"),
        numpy: s("numpy"),
        externally_managed: v.get("externallyManaged").and_then(serde_json::Value::as_bool).unwrap_or(false),
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod native {
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::{FULL_PROBE, Host, PROBE_SECONDS, Probe, VERSION_PROBE, parse_report, parse_version};

    /// This machine.
    pub struct Machine;

    #[allow(clippy::disallowed_methods)] // native only, where the clock works
    fn now() -> Instant {
        Instant::now()
    }

    /// Hides the console window a console program would open on Windows.
    pub(crate) fn no_window(cmd: &mut Command) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(not(windows))]
        let _ = cmd;
    }

    /// Runs `program args` and returns what it printed, killing it after the probe deadline.
    pub(crate) fn run(program: &Path, args: &[String], deadline: Duration) -> Result<String, String> {
        let mut cmd = Command::new(program);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        no_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| format!("it didn't start ({e})"))?;
        let mut stdout = child.stdout.take();
        // Read on a thread, so a chatty program can't fill the pipe and stall.
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(s) = stdout.as_mut() {
                let _ = s.take(64 * 1024).read_to_end(&mut out);
            }
            out
        });
        let start = now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() < deadline => std::thread::sleep(Duration::from_millis(10)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("it didn't answer within {} s", deadline.as_secs()));
                }
                Err(e) => return Err(format!("it couldn't be checked ({e})")),
            }
        };
        let out = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
        if !status.success() {
            return Err(format!("it stopped with an error ({status})"));
        }
        Ok(out)
    }

    impl Host for Machine {
        fn var(&self, key: &str) -> Option<std::ffi::OsString> {
            std::env::var_os(key)
        }
        fn is_file(&self, path: &Path) -> bool {
            path.is_file()
        }
        fn list_dir(&self, path: &Path) -> Vec<PathBuf> {
            std::fs::read_dir(path).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default()
        }
        fn probe(&self, program: &Path, args: &[String]) -> Result<Probe, String> {
            let deadline = Duration::from_secs(PROBE_SECONDS);
            let mut a = args.to_vec();
            a.extend(["-I", "-c", VERSION_PROBE].map(String::from));
            let text = run(program, &a, deadline)?;
            let version = parse_version(&text).ok_or_else(|| "it didn't report a Python version".to_string())?;
            if (version.0, version.1) < super::MIN_VERSION {
                return Ok(Probe { version, ..Probe::default() });
            }
            let mut a = args.to_vec();
            a.extend(["-P", "-X", "utf8", "-c", FULL_PROBE].map(String::from));
            let text = run(program, &a, deadline)?;
            parse_report(&text).ok_or_else(|| "its report couldn't be read".to_string())
        }
        fn windows(&self) -> bool {
            cfg!(windows)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::OsString;

    /// A machine made of a few environment variables, files and canned probe answers.
    #[derive(Default)]
    struct Fake {
        windows: bool,
        vars: HashMap<String, String>,
        files: Vec<String>,
        dirs: HashMap<String, Vec<String>>,
        probes: HashMap<String, Result<Probe, String>>,
    }

    impl Host for Fake {
        fn var(&self, key: &str) -> Option<OsString> {
            self.vars.get(key).map(OsString::from)
        }
        fn is_file(&self, path: &Path) -> bool {
            self.files.iter().any(|f| norm(f) == norm(&path.display().to_string()))
        }
        fn list_dir(&self, path: &Path) -> Vec<PathBuf> {
            let dir = norm(&path.display().to_string());
            self.dirs.iter().find(|(k, _)| norm(k) == dir).map(|(_, v)| v.iter().map(|s| path.join(s)).collect()).unwrap_or_default()
        }
        fn probe(&self, program: &Path, args: &[String]) -> Result<Probe, String> {
            let key = norm(&if args.is_empty() { program.display().to_string() } else { format!("{} {}", program.display(), args.join(" ")) });
            self.probes.iter().find(|(k, _)| norm(k) == key).map(|(_, v)| v.clone()).unwrap_or_else(|| Err("it didn't start".into()))
        }
        fn windows(&self) -> bool {
            self.windows
        }
    }

    /// Paths compared with either separator, so the tests pass on every OS.
    fn norm(p: &str) -> String {
        p.replace('\\', "/")
    }

    fn py(v: (u32, u32, u32)) -> Result<Probe, String> {
        Ok(Probe { version: v, ..Probe::default() })
    }

    #[test]
    fn the_first_new_enough_python_wins_and_the_rest_say_why() {
        let mut f = Fake::default();
        f.vars.insert("PATH".into(), "/old/bin:/usr/bin:/opt/bin".into());
        f.files = vec!["/old/bin/python3".into(), "/usr/bin/python3".into(), "/opt/bin/python".into()];
        f.probes.insert("/old/bin/python3".into(), py((3, 9, 6)));
        f.probes.insert(
            "/opt/bin/python".into(),
            Ok(Probe {
                executable: "/opt/bin/python3.12".into(),
                version: (3, 12, 1),
                pandas: Some("2.2.3".into()),
                numpy: Some("2.1.0".into()),
                externally_managed: true,
            }),
        );
        let d = detect_with(&f, "");
        let found = d.found.unwrap();
        assert_eq!(norm(&found.program.display().to_string()), "/opt/bin/python3.12", "cells run with sys.executable");
        assert_eq!((found.version.as_str(), found.short_version().as_str()), ("3.12.1", "3.12"));
        assert_eq!(found.pandas.as_deref(), Some("2.2.3"));
        assert!(found.externally_managed);
        assert_eq!(found.source, "PATH");
        // python3 is taken from the first PATH folder that has one; /usr/bin/python3 is shadowed.
        assert_eq!(d.rejected.len(), 1, "{:?}", d.rejected);
        assert_eq!(norm(&d.rejected[0].path), "/old/bin/python3");
        assert_eq!(d.rejected[0].reason, "it is Python 3.9.6; Python cells need 3.11 or newer");
    }

    #[test]
    fn options_then_virtual_environment_then_path() {
        let mut f = Fake::default();
        f.vars.insert("PATH".into(), "/usr/bin".into());
        f.vars.insert("VIRTUAL_ENV".into(), "/home/u/venv".into());
        f.files = vec!["/home/u/venv/bin/python".into(), "/usr/bin/python3".into(), "/custom/python".into()];
        let (c, _) = candidates(&f, "/custom/python");
        let order: Vec<_> = c.iter().map(|c| (norm(&c.program.display().to_string()), c.source.as_str())).collect();
        assert_eq!(
            order,
            [
                ("/custom/python".to_string(), "Python Options"),
                ("/home/u/venv/bin/python".into(), "VIRTUAL_ENV"),
                ("/usr/bin/python3".into(), "PATH")
            ]
        );
        // A path that doesn't exist is reported, and the search goes on.
        f.probes.insert("/usr/bin/python3".into(), py((3, 13, 0)));
        let d = detect_with(&f, "/missing/python");
        assert_eq!(d.rejected[0], Rejected { path: "/missing/python".into(), reason: "the file doesn't exist".into() });
        assert_eq!(norm(&d.rejected[1].path), "/home/u/venv/bin/python");
        assert_eq!(d.rejected[1].reason, "it didn't start");
        assert_eq!(norm(&d.found.unwrap().program.display().to_string()), "/usr/bin/python3", "no executable reported: the candidate itself");
        // A bare name is looked up on PATH.
        let (c, _) = candidates(&f, "python3");
        assert_eq!(norm(&c[0].program.display().to_string()), "/usr/bin/python3");
    }

    #[test]
    fn a_probe_that_times_out_or_fails_is_skipped() {
        let mut f = Fake::default();
        f.vars.insert("PATH".into(), "/a:/b".into());
        f.files = vec!["/a/python3".into(), "/b/python".into()];
        f.probes.insert("/a/python3".into(), Err("it didn't answer within 5 s".into()));
        f.probes.insert("/b/python".into(), Err("it stopped with an error (exit status: 1)".into()));
        let d = detect_with(&f, "");
        assert!(d.found.is_none());
        assert_eq!(
            d.rejected.iter().map(|r| r.reason.as_str()).collect::<Vec<_>>(),
            ["it didn't answer within 5 s", "it stopped with an error (exit status: 1)"]
        );
        assert_eq!(detect_with(&Fake::default(), ""), Detection::default(), "nothing on the machine");
    }

    #[test]
    fn windows_skips_store_shortcuts_and_tries_the_launcher_and_install_folders() {
        let mut f = Fake { windows: true, ..Fake::default() };
        f.vars.insert("LOCALAPPDATA".into(), "C:\\Users\\u\\AppData\\Local".into());
        f.vars.insert("ProgramFiles".into(), "C:\\Program Files".into());
        f.vars.insert("PATH".into(), "C:\\Users\\u\\AppData\\Local\\Microsoft\\WindowsApps;C:\\Windows".into());
        f.files = vec![
            "C:\\Users\\u\\AppData\\Local\\Microsoft\\WindowsApps/python3.exe".into(),
            "C:\\Users\\u\\AppData\\Local\\Microsoft\\WindowsApps/python.exe".into(),
            "C:\\Windows/py.exe".into(),
            "C:\\Users\\u\\AppData\\Local/Programs/Python/Python313/python.exe".into(),
            "C:\\Users\\u\\AppData\\Local/Programs/Python/Python39/python.exe".into(),
        ];
        f.dirs.insert("C:\\Users\\u\\AppData\\Local/Programs/Python".into(), vec!["Python39".into(), "Python313".into(), "Launcher".into()]);
        f.probes.insert("C:\\Windows/py.exe -3".into(), Err("it stopped with an error (exit code: 103)".into()));
        f.probes.insert(
            "C:\\Users\\u\\AppData\\Local/Programs/Python/Python313/python.exe".into(),
            Ok(Probe {
                executable: "C:\\Users\\u\\AppData\\Local\\Programs\\Python\\Python313\\python.exe".into(),
                version: (3, 13, 2),
                ..Probe::default()
            }),
        );
        let d = detect_with(&f, "");
        let stubs: Vec<_> = d.rejected.iter().filter(|r| r.reason.contains("Microsoft Store")).collect();
        assert_eq!(stubs.len(), 2, "{:?}", d.rejected);
        assert!(d.rejected.iter().any(|r| norm(&r.path) == "C:/Windows/py.exe -3"), "{:?}", d.rejected);
        let found = d.found.unwrap();
        assert_eq!(found.version, "3.13.2", "the newest install folder first");
        assert_eq!(found.source, "installed");
        assert!(is_store_stub(&f, Path::new("c:/users/u/appdata/local/microsoft/windowsapps/python.exe")));
        assert!(!is_store_stub(&f, Path::new("C:\\Users\\u\\AppData\\Local\\Microsoft\\WindowsAppsX\\python.exe")));
        assert!(!is_store_stub(&Fake::default(), Path::new("/usr/bin/python3")));
    }

    #[test]
    fn probe_output_parses() {
        assert_eq!(parse_version("3.14.7\n"), Some((3, 14, 7)));
        assert_eq!(parse_version("3.11"), Some((3, 11, 0)));
        assert_eq!(parse_version("garbage"), None);
        let p = parse_report(
            "noise\n{\"executable\": \"/x/python\", \"version\": [3, 12, 4], \"pandas\": null, \"numpy\": \"2.0.1\", \"externallyManaged\": false}\n",
        )
        .unwrap();
        assert_eq!((p.version, p.pandas, p.numpy.as_deref()), ((3, 12, 4), None, Some("2.0.1")));
        assert_eq!(parse_report("{\"version\": \"3\"}"), None);
        assert_eq!(parse_report(""), None);
    }

    /// The real probe against this machine's python3, when there is one.
    #[test]
    fn the_real_probe_reads_this_machines_python() {
        let d = detect("");
        match &d.found {
            Some(i) => {
                assert!(i.program.is_file(), "{i:?}");
                let v = parse_version(&i.version).unwrap();
                assert!((v.0, v.1) >= MIN_VERSION, "{i:?}");
            }
            None => eprintln!("no Python 3.11 or newer here; skipped ({:?})", d.rejected),
        }
    }
}
