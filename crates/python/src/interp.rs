//! Finding the user's Python: `$VIRTUAL_ENV`, `$CONDA_PREFIX`, then `python3` and `python` on
//! `PATH`, or one explicit path given by the caller. Only absolute paths to existing files count:
//! a relative `PATH` entry (`.` included) is skipped, so the folder the app runs in can't plant
//! an interpreter. Each candidate is asked its version with a hard deadline and a bounded read;
//! one that hangs or floods is killed and counted as not found. Python 3.11 or newer is needed.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A Python version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Version {
    /// `3.12.1` (with an optional newline).
    pub fn parse(s: &str) -> Option<Version> {
        let mut parts = s.trim().split('.');
        let v = Version { major: parts.next()?.parse().ok()?, minor: parts.next()?.parse().ok()?, patch: parts.next()?.parse().ok()? };
        parts.next().is_none().then_some(v)
    }
}

/// The oldest Python that runs cells.
pub const MIN_VERSION: Version = Version { major: 3, minor: 11, patch: 0 };
/// How long a version probe may take.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Bytes a version probe may print.
#[cfg(not(target_arch = "wasm32"))]
const PROBE_BYTES: usize = 64;
/// The probe's script: the version, on a Python that may be too old for anything newer.
pub const VERSION_SCRIPT: &str = "import sys; sys.stdout.write('%d.%d.%d\\n' % tuple(sys.version_info[:3]))";

/// What the search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detection {
    Found {
        path: PathBuf,
        version: Version,
    },
    /// No usable interpreter answered.
    NoneFound,
    /// The first interpreter that answered is older than [`MIN_VERSION`], and none newer was found.
    TooOld {
        path: PathBuf,
        version: Version,
    },
}

/// Why a probe gave no version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// The path isn't absolute, or isn't an existing file.
    NotAFile,
    /// It didn't run, or didn't answer within the deadline, or printed too much or something
    /// that isn't a version.
    NoAnswer,
}

/// Where to look: the environment variables the search reads, so tests can give their own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchEnv {
    pub virtual_env: Option<OsString>,
    pub conda_prefix: Option<OsString>,
    pub path: Option<OsString>,
}

impl SearchEnv {
    /// This process's environment.
    pub fn from_process() -> SearchEnv {
        SearchEnv { virtual_env: std::env::var_os("VIRTUAL_ENV"), conda_prefix: std::env::var_os("CONDA_PREFIX"), path: std::env::var_os("PATH") }
    }
}

#[cfg(windows)]
const NAMES: [&str; 2] = ["python3.exe", "python.exe"];
#[cfg(not(windows))]
const NAMES: [&str; 2] = ["python3", "python"];
/// Where an environment keeps its interpreter.
#[cfg(windows)]
const ENV_BIN: &[&str] = &["Scripts", ""];
#[cfg(not(windows))]
const ENV_BIN: &[&str] = &["bin"];

/// The paths to try, in order, without duplicates: absolute and existing files only.
pub fn candidates(env: &SearchEnv) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for prefix in [&env.virtual_env, &env.conda_prefix].into_iter().flatten() {
        let prefix = PathBuf::from(prefix);
        dirs.extend(ENV_BIN.iter().map(|b| if b.is_empty() { prefix.clone() } else { prefix.join(b) }));
    }
    let on_path: Vec<PathBuf> = env.path.as_ref().map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if usable_file(&p) && !out.contains(&p) {
            out.push(p);
        }
    };
    for d in dirs.iter().filter(|d| d.is_absolute()) {
        for n in NAMES {
            push(d.join(n));
        }
    }
    // `python3` anywhere on PATH comes before `python`.
    for n in NAMES {
        for d in on_path.iter().filter(|d| d.is_absolute()) {
            push(d.join(n));
        }
    }
    out
}

fn usable_file(p: &Path) -> bool {
    p.is_absolute() && p.is_file()
}

/// Finds an interpreter: `explicit` alone when given, otherwise the
/// [`candidates`] in order, the first new enough winning.
pub fn detect(env: &SearchEnv, explicit: Option<&Path>, timeout: Duration) -> Detection {
    let list = match explicit {
        Some(p) => vec![p.to_path_buf()],
        None => candidates(env),
    };
    let mut too_old = None;
    for path in list {
        match probe(&path, timeout) {
            Ok(version) if version >= MIN_VERSION => return Detection::Found { path, version },
            Ok(version) => {
                too_old.get_or_insert(Detection::TooOld { path, version });
            }
            Err(_) => {}
        }
    }
    too_old.unwrap_or(Detection::NoneFound)
}

/// Asks the interpreter at `path` its version: `<python> -I -S -E -c <script>`, with an empty
/// environment, within `timeout`, reading at most a few bytes.
#[cfg(not(target_arch = "wasm32"))]
pub fn probe(path: &Path, timeout: Duration) -> Result<Version, ProbeError> {
    if !usable_file(path) {
        return Err(ProbeError::NotAFile);
    }
    let mut cmd = std::process::Command::new(path);
    cmd.args(crate::process::PYTHON_FLAGS)
        .args(["-c", VERSION_SCRIPT])
        .env_clear()
        .envs(crate::process::WORKER_ENV)
        .current_dir(std::env::temp_dir());
    let out = crate::process::run_bounded(&mut cmd, timeout, PROBE_BYTES).ok_or(ProbeError::NoAnswer)?;
    Version::parse(&String::from_utf8_lossy(&out)).ok_or(ProbeError::NoAnswer)
}

/// The web build runs no Python.
#[cfg(target_arch = "wasm32")]
pub fn probe(_path: &Path, _timeout: Duration) -> Result<Version, ProbeError> {
    Err(ProbeError::NoAnswer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_order() {
        assert_eq!(Version::parse("3.12.1\n"), Some(Version { major: 3, minor: 12, patch: 1 }));
        for bad in ["", "3.12", "3.12.1.4", "three", "3.x.1", "Python 3.12.1"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert!(Version::parse("3.10.14").unwrap() < MIN_VERSION);
        assert!(Version::parse("3.11.0").unwrap() >= MIN_VERSION);
        assert!(Version::parse("4.0.0").unwrap() >= MIN_VERSION);
    }

    #[cfg(unix)]
    #[allow(clippy::disallowed_methods)] // the clock, off wasm
    mod unix {
        use super::super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::time::Instant;

        /// A folder with fake interpreters: shell scripts that answer the probe.
        struct Bin(PathBuf);

        impl Bin {
            fn new(tag: &str) -> Bin {
                let dir = std::env::temp_dir().join(format!("gridcraft-interp-{tag}-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                Bin(dir)
            }
            fn script(&self, name: &str, body: &str) -> PathBuf {
                let p = self.0.join(name);
                std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
                p
            }
        }

        impl Drop for Bin {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        fn path_of(dirs: &[&Path]) -> Option<OsString> {
            Some(std::env::join_paths(dirs).unwrap())
        }

        #[test]
        fn relative_path_entries_are_skipped() {
            let good = Bin::new("good");
            let py = good.script("python3", "echo 3.12.4");
            // A relative entry that does lead to an interpreter, from the folder the tests run in.
            let planted = Bin::new("planted");
            planted.script("python3", "echo 3.13.0");
            let depth = std::env::current_dir().unwrap().components().count().saturating_sub(1);
            let relative = PathBuf::from("../".repeat(depth)).join(planted.0.strip_prefix("/").unwrap());
            assert!(relative.join("python3").is_file(), "{}", relative.display());
            // `.`, an empty entry and relative folders come first and are never used.
            let mut path = OsString::from(".:bin::");
            path.push(relative.as_os_str());
            path.push(":");
            path.push(good.0.as_os_str());
            let env = SearchEnv { path: Some(path), ..SearchEnv::default() };
            assert_eq!(candidates(&env), vec![py.clone()]);
            assert_eq!(detect(&env, None, PROBE_TIMEOUT), Detection::Found { path: py, version: Version { major: 3, minor: 12, patch: 4 } });
            // A relative explicit path is refused too.
            assert_eq!(probe(Path::new("python3"), PROBE_TIMEOUT), Err(ProbeError::NotAFile));
            assert_eq!(detect(&SearchEnv::default(), Some(Path::new("bin/python3")), PROBE_TIMEOUT), Detection::NoneFound);
        }

        #[test]
        fn an_old_interpreter_is_reported_and_a_newer_one_wins() {
            let old = Bin::new("old");
            let old_py = old.script("python3", "echo 3.9.18");
            let env = SearchEnv { path: path_of(&[&old.0]), ..SearchEnv::default() };
            assert_eq!(detect(&env, None, PROBE_TIMEOUT), Detection::TooOld { path: old_py, version: Version { major: 3, minor: 9, patch: 18 } });
            let venv = Bin::new("venv");
            std::fs::create_dir_all(venv.0.join("bin")).unwrap();
            let venv_py = venv.script("bin/python", "echo 3.13.0");
            let env = SearchEnv { virtual_env: Some(venv.0.clone().into_os_string()), path: path_of(&[&old.0]), ..SearchEnv::default() };
            assert_eq!(candidates(&env).first(), Some(&venv_py), "the virtual environment comes first");
            assert_eq!(detect(&env, None, PROBE_TIMEOUT), Detection::Found { path: venv_py, version: Version { major: 3, minor: 13, patch: 0 } });
            // An explicit interpreter is the only one tried.
            assert!(matches!(detect(&env, Some(&old.0.join("python3")), PROBE_TIMEOUT), Detection::TooOld { .. }));
        }

        #[test]
        fn a_probe_that_hangs_or_floods_is_killed_and_not_found() {
            let bin = Bin::new("hang");
            let hang = bin.script("python3", "exec sleep 30");
            let t = Instant::now();
            assert_eq!(probe(&hang, Duration::from_millis(300)), Err(ProbeError::NoAnswer));
            assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
            let flood = bin.script("python", "exec yes 3.12.0");
            let t = Instant::now();
            assert_eq!(probe(&flood, Duration::from_secs(5)), Err(ProbeError::NoAnswer));
            assert!(t.elapsed() < Duration::from_secs(4), "a flood is cut at the byte cap, not the deadline: {:?}", t.elapsed());
            let env = SearchEnv { path: path_of(&[&bin.0]), ..SearchEnv::default() };
            assert_eq!(detect(&env, None, Duration::from_millis(300)), Detection::NoneFound);
            let junk = bin.script("python3", "echo not a version");
            assert_eq!(probe(&junk, PROBE_TIMEOUT), Err(ProbeError::NoAnswer));
        }
    }
}
