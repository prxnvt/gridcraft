//! Long-running work (Solver) as a job the host advances a slice at a time, so the window stays
//! responsive and Esc can interrupt it. There are no threads involved, which keeps the browser
//! build identical to the desktop one.
//!
//! The UI calls [`Session::step_job`] every frame while a job runs; programmatic callers use
//! [`Session::run_job`], which steps until the job finishes or a time limit passes. A job can
//! pause itself (Solver's "Show Iteration Results") or be paused by the user; the host then
//! asks whether to continue or stop.

use serde_json::{Value as Json, json};

use crate::{EngineError, Result, Session, clock};

/// How much work one slice may do.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    /// Clock time ([`clock::now_ms`]) to return by.
    pub deadline_ms: f64,
    /// Most work units (trial evaluations) in this slice: the limit when there's no clock.
    pub units: u64,
}

impl Budget {
    pub fn expired(&self, units_done: u64) -> bool {
        units_done >= self.units || (clock::has_clock() && clock::now_ms() >= self.deadline_ms)
    }
}

/// What a slice of work ended with.
#[derive(Debug)]
pub enum Step {
    /// More to do; the text describes progress ("Trial Solution: 12").
    Continue(String),
    /// Stopped for the user to look at the current state; `info` describes it.
    Pause(Json),
    Done(Result<Json>),
}

/// A unit of long-running work.
pub trait Job {
    /// What the status bar calls it ("Solver").
    fn label(&self) -> String;
    /// Does work until the budget runs out or something happens.
    fn step(&mut self, s: &mut Session, budget: Budget) -> Step;
    /// Finishes early (Esc then Stop, or a time limit): wrap up with the best result so far.
    fn stop(&mut self, s: &mut Session) -> Result<Json>;
}

/// Where the current job is, for the host.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum JobState {
    #[default]
    Idle,
    Running {
        label: String,
        progress: String,
    },
    Paused {
        label: String,
        info: Json,
    },
    Finished {
        label: String,
        result: std::result::Result<Json, String>,
    },
}

impl JobState {
    pub fn to_json(&self) -> Json {
        match self {
            JobState::Idle => json!({"state": "idle"}),
            JobState::Running { label, progress } => json!({"state": "running", "job": label, "progress": progress}),
            JobState::Paused { label, info } => json!({"state": "paused", "job": label, "info": info}),
            JobState::Finished { label, result } => match result {
                Ok(v) => json!({"state": "finished", "job": label, "result": v}),
                Err(e) => json!({"state": "finished", "job": label, "error": e}),
            },
        }
    }
}

/// Work units per slice when there is no clock to measure time with.
const UNITS_WITHOUT_CLOCK: u64 = 50;

impl Session {
    /// Starts `job` (replacing a running one, which is stopped first).
    pub fn start_job(&mut self, job: Box<dyn Job>) -> Result<()> {
        if self.job.is_some() {
            self.stop_job();
        }
        self.job_state = JobState::Running { label: job.label(), progress: String::new() };
        self.job = Some(job);
        Ok(())
    }

    /// Advances the running job for about `slice_ms` milliseconds. Does nothing when idle or
    /// paused. Returns the state afterwards.
    pub fn step_job(&mut self, slice_ms: f64) -> &JobState {
        if matches!(self.job_state, JobState::Paused { .. }) {
            return &self.job_state;
        }
        let Some(mut job) = self.job.take() else { return &self.job_state };
        let budget =
            Budget { deadline_ms: clock::now_ms() + slice_ms.max(1.0), units: if clock::has_clock() { u64::MAX } else { UNITS_WITHOUT_CLOCK } };
        let label = job.label();
        match job.step(self, budget) {
            Step::Continue(progress) => {
                self.job_state = JobState::Running { label, progress };
                self.job = Some(job);
            }
            Step::Pause(info) => {
                self.job_state = JobState::Paused { label, info };
                self.job = Some(job);
            }
            Step::Done(r) => self.job_state = JobState::Finished { label, result: r.map_err(|e| e.to_string()) },
        }
        &self.job_state
    }

    /// Pauses the running job (Esc) so the host can ask whether to continue.
    pub fn pause_job(&mut self, info: Json) {
        if let (Some(job), JobState::Running { .. }) = (&self.job, &self.job_state) {
            self.job_state = JobState::Paused { label: job.label(), info };
        }
    }

    /// Continues a paused job.
    pub fn resume_job(&mut self) {
        if let (Some(job), JobState::Paused { .. }) = (&self.job, &self.job_state) {
            self.job_state = JobState::Running { label: job.label(), progress: String::new() };
        }
    }

    /// Stops the job, keeping its best result.
    pub fn stop_job(&mut self) {
        if let Some(mut job) = self.job.take() {
            let label = job.label();
            let r = job.stop(self);
            self.job_state = JobState::Finished { label, result: r.map_err(|e| e.to_string()) };
        }
    }

    /// The finished job's outcome, clearing it (the host shows it once).
    pub fn take_job_result(&mut self) -> Option<(String, std::result::Result<Json, String>)> {
        if let JobState::Finished { .. } = self.job_state
            && let JobState::Finished { label, result } = std::mem::take(&mut self.job_state)
        {
            return Some((label, result));
        }
        None
    }

    /// Runs `job` to the end without a UI: steps until it finishes, stopping it after `max_ms`
    /// (or, without a clock, after `max_units` work units). Pauses are continued automatically.
    pub fn run_job(&mut self, job: Box<dyn Job>, max_ms: f64, max_units: u64) -> Result<Json> {
        let mut job = job;
        let start = clock::now_ms();
        let mut spent = 0u64;
        loop {
            let left = (start + max_ms - clock::now_ms()).max(0.0);
            let units = if clock::has_clock() { u64::MAX } else { UNITS_WITHOUT_CLOCK.min(max_units.saturating_sub(spent)).max(1) };
            let budget = Budget { deadline_ms: clock::now_ms() + left.min(250.0), units };
            match job.step(self, budget) {
                Step::Done(r) => return r,
                Step::Continue(_) | Step::Pause(_) => {}
            }
            spent = spent.saturating_add(units.min(UNITS_WITHOUT_CLOCK));
            let out_of_time = if clock::has_clock() { clock::now_ms() - start >= max_ms } else { spent >= max_units };
            if out_of_time {
                return job.stop(self);
            }
        }
    }
}

/// `job.status` / `job.continue` / `job.stop`: lets agents watch and steer a running job.
pub(crate) fn specs() -> Vec<crate::cmd::CommandSpec> {
    use crate::cmd::{always, cmd};
    vec![
        cmd!(query "job.status", "Job Status", [], None, "{} → {state: idle|running|paused|finished, job?, progress?, info?, result?|error?} of the long-running job (Solver)", always, status),
        cmd!(noundo "job.continue", "Continue", [], None, "{} continues a paused job", always, resume),
        cmd!(noundo "job.stop", "Stop", [], None, "{} stops the running job, keeping its best result", always, stop),
    ]
}

pub(crate) fn status(s: &mut Session, _p: &Json) -> Result<Json> {
    Ok(s.job_state.to_json())
}

pub(crate) fn resume(s: &mut Session, _p: &Json) -> Result<Json> {
    if !matches!(s.job_state, JobState::Paused { .. }) {
        return Err(EngineError::Other("No job is paused.".into()));
    }
    s.resume_job();
    Ok(s.job_state.to_json())
}

pub(crate) fn stop(s: &mut Session, _p: &Json) -> Result<Json> {
    if s.job.is_none() {
        return Err(EngineError::Other("No job is running.".into()));
    }
    s.stop_job();
    Ok(s.job_state.to_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts to `target`, a few units per step; pauses once at `pause_at`.
    struct Count {
        n: u64,
        target: u64,
        pause_at: Option<u64>,
    }

    impl Job for Count {
        fn label(&self) -> String {
            "Count".into()
        }
        fn step(&mut self, _s: &mut Session, budget: Budget) -> Step {
            let mut done = 0;
            while !budget.expired(done) {
                if self.n >= self.target {
                    return Step::Done(Ok(json!({"n": self.n})));
                }
                self.n += 1;
                done += 1;
                if Some(self.n) == self.pause_at {
                    self.pause_at = None;
                    return Step::Pause(json!({"n": self.n}));
                }
                if done >= 3 {
                    break;
                }
            }
            Step::Continue(format!("{}", self.n))
        }
        fn stop(&mut self, _s: &mut Session) -> Result<Json> {
            Ok(json!({"n": self.n, "stopped": true}))
        }
    }

    #[test]
    fn steps_pauses_resumes_and_finishes() {
        let mut s = Session::new();
        s.start_job(Box::new(Count { n: 0, target: 10, pause_at: Some(4) })).unwrap();
        assert!(matches!(s.step_job(50.0), JobState::Running { .. }));
        assert!(matches!(s.step_job(50.0), JobState::Paused { .. }));
        // Paused jobs don't advance until resumed.
        assert!(matches!(s.step_job(50.0), JobState::Paused { .. }));
        assert!(s.execute("job.continue", json!({})).is_ok());
        for _ in 0..10 {
            s.step_job(50.0);
        }
        let (label, r) = s.take_job_result().unwrap();
        assert_eq!(label, "Count");
        assert_eq!(r.unwrap()["n"], 10);
        assert_eq!(s.job_state, JobState::Idle);
        assert!(s.execute("job.stop", json!({})).is_err());
    }

    #[test]
    fn stop_keeps_the_best_result_and_run_job_finishes() {
        let mut s = Session::new();
        s.start_job(Box::new(Count { n: 0, target: 1_000_000, pause_at: None })).unwrap();
        s.step_job(1.0);
        assert_eq!(s.execute("job.status", json!({})).unwrap()["state"], "running");
        s.execute("job.stop", json!({})).unwrap();
        assert_eq!(s.take_job_result().unwrap().1.unwrap()["stopped"], true);
        let r = s.run_job(Box::new(Count { n: 0, target: 25, pause_at: Some(5) }), 10_000.0, 1_000).unwrap();
        assert_eq!(r["n"], 25);
        // A job that can't finish in time is stopped.
        let r = s.run_job(Box::new(Count { n: 0, target: u64::MAX, pause_at: None }), 20.0, 1_000).unwrap();
        assert_eq!(r["stopped"], true);
    }
}
