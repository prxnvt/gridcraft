//! The web build has no worker processes or threads: every run fails as unavailable, so Python
//! cells show `#BLOCKED!`. Same API as the native client.

use std::time::Duration;

use crate::protocol::{Limits, XlReply};
pub use crate::run::*;

pub struct PyWorker {
    launcher: Launcher,
    limits: Limits,
    next_id: u64,
}

impl PyWorker {
    pub fn new(launcher: Launcher, limits: Limits, _init_code: String) -> PyWorker {
        PyWorker { launcher, limits, next_id: 1 }
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
    pub fn configure(&mut self, limits: Limits, _init_code: String) {
        self.limits = limits;
    }
    pub fn starts(&self) -> u64 {
        0
    }
    pub fn busy(&self) -> bool {
        false
    }
    pub fn alive(&self) -> bool {
        false
    }
    pub fn kill(&mut self) {}
    pub fn reset(&mut self) {}
    fn unavailable(&self) -> Failure {
        let message = match &self.launcher {
            Launcher::Unavailable(why) => why.clone(),
            _ => "Python isn't available in the web version".into(),
        };
        Failure { kind: FailureKind::Unavailable, line: None, message }
    }
    pub fn submit(&mut self, _code: &str) -> Result<u64, Failure> {
        Err(self.unavailable())
    }
    pub fn poll(&mut self, _wait: Duration, _resolve: &mut dyn FnMut(&str, bool) -> XlReply) -> Vec<Event> {
        Vec::new()
    }
    pub fn run_blocking(&mut self, _code: &str, _resolve: &mut dyn FnMut(&str, bool) -> XlReply) -> RunOutcome {
        let id = self.next_id;
        self.next_id += 1;
        RunOutcome { id, result: Err(self.unavailable()), stdout: String::new(), stderr: String::new(), elapsed_ms: 0.0, importing: None }
    }
}
