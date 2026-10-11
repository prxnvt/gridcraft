//! The parent side: [`PyWorker`] starts a worker, sends it cells to run, answers its `xl()`
//! requests, and guards it: a run that passes its deadline or a worker over its memory cap is
//! killed, and a worker that dies is replaced on the next run.
//!
//! Nothing here blocks unless asked to: a reader thread turns the worker's output into frames on a
//! channel, and [`PyWorker::poll`] takes what has arrived.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::protocol::{Done, FromWorker, Limits, Output, PyError, ToWorker, XlReply, parse_from_worker, to_worker_line};

/// How to start a worker.
#[derive(Clone, Debug, PartialEq)]
pub enum Launcher {
    /// Run `program` with `args` and extra environment variables (the app binary with
    /// [`crate::WORKER_FLAG`]).
    Process { program: PathBuf, args: Vec<String>, env: Vec<(String, String)> },
    /// A worker thread in this process (tests): no memory watchdog, and a run that never ends
    /// leaves its thread behind when killed.
    InProcess,
    /// Python can't run here; runs fail with this reason.
    Unavailable(String),
}

impl Launcher {
    /// This program started again as the worker.
    pub fn current_exe() -> Launcher {
        match std::env::current_exe() {
            Ok(program) => Launcher::Process { program, args: vec![crate::WORKER_FLAG.to_string()], env: vec![] },
            Err(e) => Launcher::Unavailable(format!("GridCraft couldn't find its own program to start Python: {e}")),
        }
    }
}

/// Why a run produced no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The code raised an exception (or didn't compile).
    Error,
    /// It ran longer than the time limit.
    Timeout,
    /// The worker used more memory than allowed and was stopped.
    Memory,
    /// The worker stopped unexpectedly.
    Crashed,
    /// Python can't run in this build or on this machine.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub kind: FailureKind,
    /// The line in the cell's code, for errors.
    pub line: Option<u32>,
    pub message: String,
}

/// What a run produced.
#[derive(Clone, Debug, PartialEq)]
pub struct RunOutcome {
    pub id: u64,
    pub result: Result<Output, Failure>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: f64,
}

/// Something that happened while polling.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A run finished.
    Done(RunOutcome),
    /// The initialization code raised an error (the worker still runs cells).
    InitError(PyError),
}

/// A line read from the worker.
pub enum Recv {
    Line(String),
    /// Nothing arrived in time.
    Empty,
    /// The worker closed its output (it exited).
    Closed,
}

/// The pipe to one worker.
pub trait Transport: Send {
    fn send(&mut self, line: &str) -> std::io::Result<()>;
    /// The next line, waiting up to `wait`.
    fn recv(&mut self, wait: Duration) -> Recv;
    /// The worker's resident memory in bytes, when it can be measured.
    fn rss(&mut self) -> Option<u64>;
    /// Stops the worker now.
    fn kill(&mut self);
}

/// How often the memory watchdog looks.
const RSS_EVERY: Duration = Duration::from_millis(50);
/// Time over the limit a run gets before the parent kills it: the worker's own time limit stops
/// ordinary code first, with a tidier message.
const KILL_GRACE: Duration = Duration::from_millis(1_000);

pub const TIMEOUT_MESSAGE: &str = "Python took longer than the time limit";
pub const CRASH_MESSAGE: &str = "Python stopped unexpectedly";

pub fn memory_message(limits: &Limits) -> String {
    format!("Python used more than {} MB", limits.memory_bytes / (1024 * 1024))
}

#[allow(clippy::disallowed_methods)] // the client only exists off wasm, where the clock works
fn now() -> Instant {
    Instant::now()
}

struct Inflight {
    id: u64,
    /// When the run became the worker's current one.
    started: Option<Instant>,
}

struct Conn {
    transport: Box<dyn Transport>,
    /// The worker answered Init (or the last Reset).
    ready: bool,
    last_rss: Instant,
}

/// A worker process and the runs sent to it, oldest first.
pub struct PyWorker {
    launcher: Launcher,
    limits: Limits,
    init_code: String,
    conn: Option<Conn>,
    inflight: VecDeque<Inflight>,
    next_id: u64,
    /// Workers started so far (tests watch restarts).
    starts: u64,
}

impl PyWorker {
    pub fn new(launcher: Launcher, limits: Limits, init_code: String) -> PyWorker {
        PyWorker { launcher, limits, init_code, conn: None, inflight: VecDeque::new(), next_id: 1, starts: 0 }
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// New limits or initialization code take effect when the worker next starts or resets.
    pub fn configure(&mut self, limits: Limits, init_code: String) {
        self.limits = limits;
        self.init_code = init_code;
    }

    /// How many workers have been started.
    pub fn starts(&self) -> u64 {
        self.starts
    }

    /// Whether runs are waiting for results.
    pub fn busy(&self) -> bool {
        !self.inflight.is_empty()
    }

    /// Whether a worker is running.
    pub fn alive(&self) -> bool {
        self.conn.is_some()
    }

    /// Stops the worker: the namespace is gone, and runs in flight end without results. The next
    /// run starts a fresh worker.
    pub fn kill(&mut self) {
        if let Some(mut c) = self.conn.take() {
            c.transport.kill();
        }
        self.inflight.clear();
    }

    /// Starts a fresh namespace (the initialization code runs again). Runs in flight still finish.
    pub fn reset(&mut self) {
        let line = to_worker_line(&ToWorker::Reset);
        let sent = self.conn.as_mut().map(|c| {
            c.ready = false;
            c.transport.send(&line)
        });
        if !matches!(sent, Some(Ok(()))) {
            self.kill();
        }
    }

    fn connect(&mut self) -> Result<&mut Conn, Failure> {
        if self.conn.is_none() {
            let transport = start(&self.launcher)?;
            self.starts += 1;
            let mut c = Conn { transport, ready: false, last_rss: now() };
            let init = to_worker_line(&ToWorker::Init { limits: self.limits.clone(), init_code: self.init_code.clone() });
            if c.transport.send(&init).is_err() {
                c.transport.kill();
                return Err(Failure { kind: FailureKind::Crashed, line: None, message: CRASH_MESSAGE.into() });
            }
            self.conn = Some(c);
        }
        self.conn.as_mut().ok_or_else(|| Failure { kind: FailureKind::Crashed, line: None, message: CRASH_MESSAGE.into() })
    }

    /// Sends `code` to run after the runs already in flight; returns its id.
    pub fn submit(&mut self, code: &str) -> Result<u64, Failure> {
        let id = self.next_id;
        self.next_id += 1;
        let line = to_worker_line(&ToWorker::Run { id, code: code.to_string() });
        let conn = self.connect()?;
        if conn.transport.send(&line).is_err() {
            self.kill();
            return Err(Failure { kind: FailureKind::Crashed, line: None, message: CRASH_MESSAGE.into() });
        }
        let started = if self.inflight.is_empty() { Some(now()) } else { None };
        self.inflight.push_back(Inflight { id, started });
        Ok(id)
    }

    /// The deadline of the current run: its time limit plus a grace period, plus another time
    /// limit while the initialization code may still be running.
    fn deadline(&self) -> Option<Instant> {
        let head = self.inflight.front()?;
        let started = head.started?;
        let limit = Duration::from_millis(self.limits.timeout_ms);
        let init = if self.conn.as_ref().is_some_and(|c| c.ready) { Duration::ZERO } else { limit };
        Some(started + limit + init + KILL_GRACE)
    }

    /// Takes what the worker sent, waiting up to `wait` for the first line. `resolve` answers
    /// `xl()` requests: (reference, headers) to the cells' value. Kills the worker when the current
    /// run is past its deadline or the worker is over its memory cap.
    pub fn poll(&mut self, wait: Duration, resolve: &mut dyn FnMut(&str, bool) -> XlReply) -> Vec<Event> {
        let mut events = Vec::new();
        let mut wait = wait;
        while let Some(conn) = self.conn.as_mut() {
            let line = conn.transport.recv(wait);
            wait = Duration::ZERO;
            match line {
                Recv::Line(l) => {
                    let Some(msg) = parse_from_worker(&l) else { continue };
                    self.handle(msg, resolve, &mut events);
                }
                Recv::Empty => break,
                Recv::Closed => {
                    self.fail_all(FailureKind::Crashed, CRASH_MESSAGE.into(), &mut events);
                    break;
                }
            }
        }
        self.watch(&mut events);
        events
    }

    fn handle(&mut self, msg: FromWorker, resolve: &mut dyn FnMut(&str, bool) -> XlReply, events: &mut Vec<Event>) {
        match msg {
            FromWorker::Ready { init_error } => {
                if let Some(c) = self.conn.as_mut() {
                    c.ready = true;
                }
                if let Some(e) = init_error {
                    events.push(Event::InitError(e));
                }
            }
            FromWorker::Xl { id, reference, headers } => {
                let reply = resolve(&reference, headers);
                let line = to_worker_line(&ToWorker::XlValue { id, reply });
                if let Some(c) = self.conn.as_mut()
                    && c.transport.send(&line).is_err()
                {
                    self.fail_all(FailureKind::Crashed, CRASH_MESSAGE.into(), events);
                }
            }
            FromWorker::Done(done) => {
                let Some(at) = self.inflight.iter().position(|r| r.id == done.id) else { return };
                self.inflight.remove(at);
                if let Some(next) = self.inflight.front_mut()
                    && next.started.is_none()
                {
                    next.started = Some(now());
                }
                events.push(Event::Done(outcome(done)));
            }
        }
    }

    /// The deadline and memory checks.
    fn watch(&mut self, events: &mut Vec<Event>) {
        if self.deadline().is_some_and(|d| now() >= d) {
            self.fail_all(FailureKind::Timeout, TIMEOUT_MESSAGE.into(), events);
            return;
        }
        let Some(conn) = self.conn.as_mut() else { return };
        if conn.last_rss.elapsed() < RSS_EVERY {
            return;
        }
        conn.last_rss = now();
        if conn.transport.rss().is_some_and(|rss| rss > self.limits.memory_bytes) {
            let msg = memory_message(&self.limits);
            self.fail_all(FailureKind::Memory, msg, events);
        }
    }

    /// Kills the worker; the current run fails with `kind`, the ones queued behind it as stopped.
    fn fail_all(&mut self, kind: FailureKind, message: String, events: &mut Vec<Event>) {
        let runs: Vec<Inflight> = self.inflight.drain(..).collect();
        if let Some(mut c) = self.conn.take() {
            c.transport.kill();
        }
        for (i, r) in runs.into_iter().enumerate() {
            let (kind, message) = if i == 0 { (kind, message.clone()) } else { (FailureKind::Crashed, CRASH_MESSAGE.to_string()) };
            let elapsed_ms = r.started.map(|s| s.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0);
            events.push(Event::Done(RunOutcome {
                id: r.id,
                result: Err(Failure { kind, line: None, message }),
                stdout: String::new(),
                stderr: String::new(),
                elapsed_ms,
            }));
        }
    }

    /// Runs `code` and waits for its result (headless callers: the CLI, MCP, tests).
    pub fn run_blocking(&mut self, code: &str, resolve: &mut dyn FnMut(&str, bool) -> XlReply) -> RunOutcome {
        let id = match self.submit(code) {
            Ok(id) => id,
            Err(f) => return RunOutcome { id: 0, result: Err(f), stdout: String::new(), stderr: String::new(), elapsed_ms: 0.0 },
        };
        loop {
            for e in self.poll(Duration::from_millis(10), resolve) {
                if let Event::Done(o) = e
                    && o.id == id
                {
                    return o;
                }
            }
        }
    }
}

impl Drop for PyWorker {
    fn drop(&mut self) {
        self.kill();
    }
}

fn outcome(done: Done) -> RunOutcome {
    let result = match (done.output, done.error) {
        (_, Some(e)) if e.timeout => Err(Failure { kind: FailureKind::Timeout, line: e.line, message: e.message }),
        (_, Some(e)) => Err(Failure { kind: FailureKind::Error, line: e.line, message: e.message }),
        (Some(o), None) => Ok(o),
        (None, None) => Ok(Output::default()),
    };
    RunOutcome { id: done.id, result, stdout: done.stdout, stderr: done.stderr, elapsed_ms: done.elapsed_ms }
}

fn start(launcher: &Launcher) -> Result<Box<dyn Transport>, Failure> {
    match launcher {
        Launcher::Process { program, args, env } => ProcessTransport::spawn(program, args, env)
            .map(|t| Box::new(t) as Box<dyn Transport>)
            .map_err(|e| Failure { kind: FailureKind::Unavailable, line: None, message: format!("Python couldn't start: {e}") }),
        Launcher::InProcess => in_process(),
        Launcher::Unavailable(why) => Err(Failure { kind: FailureKind::Unavailable, line: None, message: why.clone() }),
    }
}

#[cfg(feature = "python")]
fn in_process() -> Result<Box<dyn Transport>, Failure> {
    Ok(Box::new(ChannelTransport::spawn()))
}

#[cfg(not(feature = "python"))]
fn in_process() -> Result<Box<dyn Transport>, Failure> {
    Err(Failure { kind: FailureKind::Unavailable, line: None, message: "This build has no Python interpreter".into() })
}

/// Reads lines from `input` on a thread into a channel; the channel closes at the end.
fn line_reader<R: BufRead + Send + 'static>(input: R) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("python-worker-output".into()).spawn(move || {
        let mut input = input;
        loop {
            let mut line = String::new();
            match input.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    // Without the thread the receiver closes at once, which reads as a stopped worker.
    drop(spawned);
    rx
}

fn recv_line(rx: &Receiver<String>, wait: Duration) -> Recv {
    match rx.recv_timeout(wait) {
        Ok(l) => Recv::Line(l),
        Err(RecvTimeoutError::Timeout) => Recv::Empty,
        Err(RecvTimeoutError::Disconnected) => Recv::Closed,
    }
}

/// A worker process on pipes.
struct ProcessTransport {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    lines: Receiver<String>,
    system: sysinfo::System,
}

impl ProcessTransport {
    fn spawn(program: &PathBuf, args: &[String], env: &[(String, String)]) -> std::io::Result<ProcessTransport> {
        let mut child = Command::new(program)
            .args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("the worker has no output pipe"))?;
        let lines = line_reader(BufReader::new(stdout));
        Ok(ProcessTransport { child, stdin, lines, system: sysinfo::System::new() })
    }
}

impl Transport for ProcessTransport {
    fn send(&mut self, line: &str) -> std::io::Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| std::io::Error::other("the worker's input is closed"))?;
        stdin.write_all(line.as_bytes())?;
        stdin.flush()
    }
    fn recv(&mut self, wait: Duration) -> Recv {
        recv_line(&self.lines, wait)
    }
    fn rss(&mut self) -> Option<u64> {
        let pid = sysinfo::Pid::from_u32(self.child.id());
        self.system.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]), true, sysinfo::ProcessRefreshKind::nothing().with_memory());
        self.system.process(pid).map(|p| p.memory())
    }
    fn kill(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ProcessTransport {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A worker thread in this process, on channels.
#[cfg(feature = "python")]
struct ChannelTransport {
    to_worker: Option<mpsc::Sender<String>>,
    lines: Receiver<String>,
}

#[cfg(feature = "python")]
impl ChannelTransport {
    fn spawn() -> ChannelTransport {
        let (to_worker, worker_in) = mpsc::channel::<String>();
        let (pipe_out, pipe_in) = pipe();
        let spawned = std::thread::Builder::new().name("python-worker".into()).stack_size(64 * 1024 * 1024).spawn(move || {
            crate::worker::serve(ChannelReader::new(worker_in), pipe_out, crate::worker::OnClose::Return);
        });
        drop(spawned);
        ChannelTransport { to_worker: Some(to_worker), lines: line_reader(BufReader::new(pipe_in)) }
    }
}

#[cfg(feature = "python")]
impl Transport for ChannelTransport {
    fn send(&mut self, line: &str) -> std::io::Result<()> {
        let tx = self.to_worker.as_ref().ok_or_else(|| std::io::Error::other("the worker is stopped"))?;
        tx.send(line.to_string()).map_err(|_| std::io::Error::other("the worker is stopped"))
    }
    fn recv(&mut self, wait: Duration) -> Recv {
        recv_line(&self.lines, wait)
    }
    fn rss(&mut self) -> Option<u64> {
        None
    }
    fn kill(&mut self) {
        // The worker thread ends when it next reads; a run that never ends keeps its thread.
        self.to_worker = None;
    }
}

/// Lines from a channel as a reader.
#[cfg(feature = "python")]
struct ChannelReader {
    rx: Receiver<String>,
    buf: Vec<u8>,
    pos: usize,
}

#[cfg(feature = "python")]
impl ChannelReader {
    fn new(rx: Receiver<String>) -> ChannelReader {
        ChannelReader { rx, buf: Vec::new(), pos: 0 }
    }
}

#[cfg(feature = "python")]
impl std::io::Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let avail = std::io::BufRead::fill_buf(self)?;
        let n = avail.len().min(out.len());
        out.get_mut(..n).unwrap_or_default().copy_from_slice(avail.get(..n).unwrap_or_default());
        self.consume(n);
        Ok(n)
    }
}

#[cfg(feature = "python")]
impl BufRead for ChannelReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(s) => {
                    self.buf = s.into_bytes();
                    self.pos = 0;
                }
                Err(_) => return Ok(&[]),
            }
        }
        Ok(self.buf.get(self.pos..).unwrap_or_default())
    }
    fn consume(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n);
    }
}

/// An in-memory byte pipe: the writer end sends chunks, the reader end reads them.
#[cfg(feature = "python")]
fn pipe() -> (PipeWriter, ChannelReader) {
    let (tx, rx) = mpsc::channel();
    (PipeWriter { tx }, ChannelReader::new(rx))
}

#[cfg(feature = "python")]
struct PipeWriter {
    tx: mpsc::Sender<String>,
}

#[cfg(feature = "python")]
impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tx.send(String::from_utf8_lossy(buf).into_owned()).map_err(|_| std::io::Error::other("closed"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[cfg(feature = "python")]
mod tests {
    use super::*;
    use gridcraft_core::{Array, Value};

    /// Answers `xl()`: A1 is 10, A1:B2 a small table, anything else an error.
    fn cells(reference: &str, _headers: bool) -> XlReply {
        match reference {
            "A1" => XlReply::Value(Value::Number(10.0)),
            "A1:B2" => XlReply::Value(Array::new(2, 2, vec!["n".into(), "v".into(), "a".into(), 2.0.into()]).map(Value::from).unwrap()),
            r => XlReply::Error(format!("`{r}` is not a reference")),
        }
    }

    fn value(o: &RunOutcome) -> Value {
        o.result.as_ref().map(|o| o.value.clone()).unwrap_or_else(|f| panic!("{f:?}"))
    }

    fn failure(o: &RunOutcome) -> &Failure {
        o.result.as_ref().err().unwrap_or_else(|| panic!("expected a failure: {o:?}"))
    }

    #[test]
    fn runs_share_a_namespace_read_cells_and_reset() {
        let mut w = PyWorker::new(Launcher::InProcess, Limits::default(), "base = 100".into());
        assert_eq!(value(&w.run_blocking("x = xl('A1')\nprint('x is', x)\nx + base", &mut cells)), Value::Number(110.0));
        let o = w.run_blocking("[r['v'] for r in xl('A1:B2', headers=True)]", &mut cells);
        assert_eq!(value(&o), Array::new(1, 1, vec![2.0.into()]).map(Value::from).unwrap());
        let o = w.run_blocking("x * 2", &mut cells);
        assert_eq!(value(&o), Value::Number(20.0));
        let o = w.run_blocking("xl('nowhere')", &mut cells);
        assert!(failure(&o).message.contains("ValueError"), "{o:?}");
        let o = w.run_blocking("print('out')\nimport sys\nprint('err', file=sys.stderr)\n1", &mut cells);
        assert_eq!((o.stdout.as_str(), o.stderr.as_str()), ("out\n", "err\n"));
        w.reset();
        let o = w.run_blocking("x", &mut cells);
        assert!(failure(&o).message.contains("NameError"), "reset forgets names: {o:?}");
        assert_eq!(value(&w.run_blocking("base", &mut cells)), Value::Number(100.0), "the initialization code runs again");
        assert_eq!(w.starts(), 1);
    }

    #[test]
    fn initialization_errors_are_reported_and_cells_still_run() {
        let mut w = PyWorker::new(Launcher::InProcess, Limits::default(), "1/0".into());
        let id = w.submit("2 + 2").unwrap();
        let mut events = Vec::new();
        while !events.iter().any(|e| matches!(e, Event::Done(o) if o.id == id)) {
            events.extend(w.poll(Duration::from_millis(10), &mut cells));
        }
        assert!(events.iter().any(|e| matches!(e, Event::InitError(err) if err.message.starts_with("ZeroDivisionError"))), "{events:?}");
    }

    #[test]
    fn unavailable_launcher_fails_runs() {
        let mut w = PyWorker::new(Launcher::Unavailable("no Python here".into()), Limits::default(), String::new());
        let o = w.run_blocking("1", &mut cells);
        assert_eq!(failure(&o).kind, FailureKind::Unavailable);
        assert_eq!(failure(&o).message, "no Python here");
    }

    /// The real worker process for the harness tests: this test binary started again, running only
    /// this test with the worker's environment variable set.
    #[test]
    fn worker_process_entry() {
        if std::env::var_os("GRIDCRAFT_PYTHON_WORKER_TEST").is_some() {
            crate::worker::serve(std::io::BufReader::new(std::io::stdin()), std::io::stdout(), crate::worker::OnClose::ExitProcess);
            std::process::exit(0);
        }
    }

    fn process_worker(limits: Limits) -> PyWorker {
        let program = std::env::current_exe().unwrap();
        let args = ["client::tests::worker_process_entry", "--exact", "--nocapture", "--test-threads=1"].map(String::from).to_vec();
        let env = vec![("GRIDCRAFT_PYTHON_WORKER_TEST".to_string(), "1".to_string())];
        PyWorker::new(Launcher::Process { program, args, env }, limits, String::new())
    }

    #[test]
    fn hostile_code_fails_while_the_host_keeps_running() {
        let limits = Limits { timeout_ms: 1_000, ..Limits::default() };
        let mut w = process_worker(limits);
        assert_eq!(value(&w.run_blocking("n = xl('A1')\nn + 1", &mut cells)), Value::Number(11.0));

        // An endless loop is stopped by the time limit.
        let o = w.run_blocking("while True: pass", &mut cells);
        assert_eq!(failure(&o).kind, FailureKind::Timeout, "{o:?}");
        assert!(o.elapsed_ms < 5_000.0, "{o:?}");
        assert_eq!(value(&w.run_blocking("n", &mut cells)), Value::Number(10.0), "the worker survived its own time limit");

        // A huge integer power never checks the time limit: the parent kills the worker.
        let o = w.run_blocking("10 ** 10 ** 9", &mut cells);
        assert_eq!(failure(&o).kind, FailureKind::Timeout, "{o:?}");
        assert_eq!(failure(&o).message, TIMEOUT_MESSAGE);
        assert!(!w.alive());
        assert_eq!(value(&w.run_blocking("2 * 21", &mut cells)), Value::Number(42.0), "a fresh worker starts");

        // A huge string allocates until the memory watchdog (or the allocator) stops the worker.
        let o = w.run_blocking("len('x' * 10**12)", &mut cells);
        let f = failure(&o);
        if cfg!(target_os = "macos") {
            assert_eq!(f.kind, FailureKind::Memory, "{o:?}");
        } else {
            assert!(matches!(f.kind, FailureKind::Memory | FailureKind::Crashed | FailureKind::Error), "{o:?}");
        }
        if f.kind == FailureKind::Memory {
            assert_eq!(f.message, "Python used more than 512 MB");
        }
        assert_eq!(value(&w.run_blocking("'still here'", &mut cells)), Value::text("still here"));

        // Deep recursion is a Python error, not a crash.
        let o = w.run_blocking("def f(n): return f(n + 1)\nf(0)", &mut cells);
        assert_eq!(failure(&o).kind, FailureKind::Error, "{o:?}");
        assert!(w.starts() >= 3, "restarted after each kill: {}", w.starts());
    }

    #[test]
    fn a_worker_that_dies_is_replaced() {
        let mut w = process_worker(Limits::default());
        assert_eq!(value(&w.run_blocking("1", &mut cells)), Value::Number(1.0));
        // Kill the process behind the client's back: the next poll sees it gone.
        if let Some(c) = w.conn.as_mut() {
            c.transport.kill();
        }
        let o = w.run_blocking("2", &mut cells);
        assert_eq!(failure(&o).kind, FailureKind::Crashed, "{o:?}");
        assert_eq!(failure(&o).message, CRASH_MESSAGE);
        assert_eq!(value(&w.run_blocking("3", &mut cells)), Value::Number(3.0));
        assert_eq!(w.starts(), 2);
    }
}
