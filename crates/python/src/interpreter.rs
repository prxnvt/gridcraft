//! What an interpreter is, as the rest of the app reads it: on every build, the web one included,
//! where no interpreter is ever found. `describe` works one out.

use std::path::PathBuf;

use crate::interp::Version;

/// Where a Python came from, as the person would name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An environment made with `uv venv`.
    UvEnvironment,
    /// One of uv's own Pythons.
    UvPython,
    /// A virtual environment made another way.
    Venv,
    Conda,
    Homebrew,
    /// python.org's installer, or another framework build.
    Framework,
    System,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::UvEnvironment => "uv environment",
            Kind::UvPython => "uv Python",
            Kind::Venv => "virtual environment",
            Kind::Conda => "conda environment",
            Kind::Homebrew => "Homebrew",
            Kind::Framework => "python.org",
            Kind::System => "system",
        }
    }

    /// Whether packages can be installed into it: a Python outside an environment is usually
    /// managed by something else, which refuses them.
    pub fn is_environment(self) -> bool {
        matches!(self, Kind::UvEnvironment | Kind::Venv | Kind::Conda)
    }
}

/// An interpreter, described: what to start, its package folders, what it must read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interpreter {
    /// The path found or chosen.
    pub path: PathBuf,
    /// The binary to start.
    pub run_as: PathBuf,
    pub version: Version,
    pub kind: Kind,
    /// Folders of installed packages, to go on `sys.path`.
    pub site_dirs: Vec<PathBuf>,
    /// What the sandbox must let it read.
    pub read_paths: Vec<PathBuf>,
}
