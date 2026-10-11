//! The parent side: [`PyWorker`] starts a worker, sends it cells to run, answers its `xl()`
//! requests, and guards it: a run that passes its deadline or a worker over its memory cap is
//! killed, and a worker that dies is replaced on the next run.
//!
//! Nothing here blocks unless asked to: a reader thread turns the worker's output into frames on a
//! channel, and [`PyWorker::poll`] takes what has arrived.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use process_wrap::std::{ChildWrapper, CommandWrap};

use crate::protocol::{Done, FromWorker, Limits, Output, ToWorker, XlReply, parse_from_worker, to_worker_line};
pub use crate::run::*;

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
    /// Stops the worker now, with any process it started.
    fn kill(&mut self);
    /// The end of what the worker wrote to its standard error (why it stopped, when it crashed).
    fn stderr_tail(&mut self) -> String {
        String::new()
    }
}

/// How often the memory watchdog looks.
const RSS_EVERY: Duration = Duration::from_millis(50);
/// Time over the limit a run gets before the parent kills it: the worker's own time limit stops
/// ordinary code first, with a tidier message.
const KILL_GRACE: Duration = Duration::from_millis(1_000);

#[allow(clippy::disallowed_methods)] // the client only exists off wasm, where the clock works
fn now() -> Instant {
    Instant::now()
}

struct Inflight {
    id: u64,
    /// Kept to send again when the worker is restarted before it was ready.
    code: String,
    /// When the run became the worker's current one.
    started: Option<Instant>,
}

struct Conn {
    transport: Box<dyn Transport>,
    /// The worker answered Init (or the last Reset).
    ready: bool,
    last_rss: Instant,
    /// This worker replaces one that stopped before it was ready (see [`PyWorker::retry_start`]).
    retried: bool,
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

    /// How this worker starts.
    pub fn launcher(&self) -> &Launcher {
        &self.launcher
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
            let mut c = Conn { transport, ready: false, last_rss: now(), retried: false };
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
        self.inflight.push_back(Inflight { id, code: code.to_string(), started });
        Ok(id)
    }

    /// The worker stopped, or didn't answer, before it was ready: start it once more and send the
    /// waiting runs again. A program that was just built can be slow to start the first time.
    /// Returns false when this worker was already a retry (or can't start), so the runs fail.
    fn retry_start(&mut self) -> bool {
        let Some(old) = self.conn.as_mut() else { return false };
        if old.ready || old.retried {
            return false;
        }
        old.transport.kill();
        self.conn = None;
        let Ok(transport) = start(&self.launcher) else { return false };
        self.starts += 1;
        let mut c = Conn { transport, ready: false, last_rss: now(), retried: true };
        let mut lines = vec![to_worker_line(&ToWorker::Init { limits: self.limits.clone(), init_code: self.init_code.clone() })];
        lines.extend(self.inflight.iter().map(|r| to_worker_line(&ToWorker::Run { id: r.id, code: r.code.clone() })));
        if lines.iter().any(|l| c.transport.send(l).is_err()) {
            c.transport.kill();
            return false;
        }
        for (i, r) in self.inflight.iter_mut().enumerate() {
            r.started = (i == 0).then(now);
        }
        self.conn = Some(c);
        true
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
                    if self.retry_start() {
                        continue;
                    }
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
            if self.retry_start() {
                return;
            }
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

/// Bytes of the worker's standard error kept (the end of it).
const STDERR_TAIL: usize = 8 * 1024;

/// Drains `input` on a thread, keeping its last [`STDERR_TAIL`] bytes, so a chatty worker never
/// fills the pipe and stalls.
fn tail_reader<R: Read + Send + 'static>(input: R) -> Arc<Mutex<VecDeque<u8>>> {
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let keep = tail.clone();
    let spawned = std::thread::Builder::new().name("python-worker-stderr".into()).spawn(move || {
        let mut input = input;
        let mut buf = [0u8; 4096];
        loop {
            match input.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Ok(mut t) = keep.lock() {
                        t.extend(buf.get(..n).unwrap_or_default());
                        let extra = t.len().saturating_sub(STDERR_TAIL);
                        t.drain(..extra);
                    }
                }
            }
        }
    });
    drop(spawned);
    tail
}

/// A worker process on pipes, in a process group of its own (a job object on Windows), so a kill
/// also stops whatever the user's code started.
struct ProcessTransport {
    child: Box<dyn ChildWrapper>,
    stdin: Option<std::process::ChildStdin>,
    lines: Receiver<String>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    system: sysinfo::System,
}

impl ProcessTransport {
    fn spawn(program: &PathBuf, args: &[String], env: &[(String, String)]) -> std::io::Result<ProcessTransport> {
        let mut cmd = CommandWrap::with_new(program, |c| {
            c.args(args).envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str()))).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        });
        #[cfg(unix)]
        cmd.wrap(process_wrap::std::ProcessGroup::leader());
        #[cfg(windows)]
        {
            // No console window for a console program such as python.exe.
            cmd.wrap(process_wrap::std::CreationFlags(windows::Win32::System::Threading::CREATE_NO_WINDOW));
            cmd.wrap(process_wrap::std::JobObject);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin().take();
        let stdout = child.stdout().take().ok_or_else(|| std::io::Error::other("the worker has no output pipe"))?;
        let stderr = match child.stderr().take() {
            Some(e) => tail_reader(e),
            None => Arc::new(Mutex::new(VecDeque::new())),
        };
        let lines = line_reader(BufReader::new(stdout));
        Ok(ProcessTransport { child, stdin, lines, stderr, system: sysinfo::System::new() })
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
        // Kills the whole group (or job), then reaps the worker.
        let _ = self.child.kill();
    }
    fn stderr_tail(&mut self) -> String {
        self.stderr.lock().map(|t| String::from_utf8_lossy(&t.iter().copied().collect::<Vec<u8>>()).into_owned()).unwrap_or_default()
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
        // The first start fails before it is ready, as a slow first launch would.
        if let Some(marker) = std::env::var_os("GRIDCRAFT_PYTHON_WORKER_FAIL_ONCE")
            && std::fs::OpenOptions::new().write(true).create_new(true).open(&marker).is_ok()
        {
            std::process::exit(3);
        }
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
    fn a_worker_that_stops_before_it_is_ready_starts_once_more() {
        let dir = std::env::temp_dir().join(format!("gridcraft-py-retry-{}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        let mut w = process_worker(Limits::default());
        if let Launcher::Process { env, .. } = &mut w.launcher {
            env.push(("GRIDCRAFT_PYTHON_WORKER_FAIL_ONCE".into(), dir.display().to_string()));
        }
        assert_eq!(value(&w.run_blocking("6 * 7", &mut cells)), Value::Number(42.0));
        assert_eq!(w.starts(), 2, "the failed first start was retried");
        // A worker that keeps failing before it is ready is reported after one retry.
        let mut w = PyWorker::new(
            Launcher::Process { program: "/bin/sh".into(), args: vec!["-c".into(), "exit 1".into()], env: vec![] },
            Limits::default(),
            String::new(),
        );
        if cfg!(unix) {
            let o = w.run_blocking("1", &mut cells);
            assert_eq!(failure(&o).message, CRASH_MESSAGE);
            assert_eq!(w.starts(), 2);
        }
        let _ = std::fs::remove_file(&dir);
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

/// The worker for the user's Python (`shim.py`), run by this machine's python3 when it is 3.11 or
/// newer. Skipped, with a message, when there is none (CI may have none).
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod cpython_tests {
    use super::*;
    use gridcraft_core::{Array, CellError, Value};

    fn cpython(limits: Limits) -> Option<(PyWorker, crate::interp::Interpreter)> {
        let d = crate::interp::detect("");
        match d.found {
            Some(i) => Some((PyWorker::new(i.launcher(), limits, "base = 100".into()), i)),
            None => {
                eprintln!("skipped: no Python 3.11 or newer here ({:?})", d.rejected);
                None
            }
        }
    }

    fn cells(reference: &str, headers: bool) -> XlReply {
        match reference {
            "A1" => XlReply::Value(Value::Number(10.0)),
            "T" => XlReply::Value(
                Array::new(3, 2, vec!["name".into(), "score".into(), "a".into(), 1.0.into(), "b".into(), Value::Error(CellError::Div0)])
                    .map(Value::from)
                    .unwrap_or_default(),
            ),
            r => XlReply::Error(format!("`{r}` is not a reference (headers={headers})")),
        }
    }

    fn value(o: &RunOutcome) -> Value {
        o.result.as_ref().map(|o| o.value.clone()).unwrap_or_else(|f| panic!("{f:?}"))
    }

    fn failure(o: &RunOutcome) -> &Failure {
        o.result.as_ref().err().unwrap_or_else(|| panic!("expected a failure: {o:?}"))
    }

    fn col(v: Vec<Value>) -> Value {
        Array::new(v.len(), 1, v).map(Value::from).unwrap_or_default()
    }

    #[test]
    fn the_shim_speaks_the_protocol() {
        let Some((mut w, i)) = cpython(Limits::default()) else { return };
        let o = w.run_blocking("x = xl('A1')\nprint('x is', x)\nimport sys\nprint('oops', file=sys.stderr)\nx + base", &mut cells);
        assert_eq!(value(&o), Value::Number(110.0), "{o:?}");
        assert_eq!((o.stdout.as_str(), o.stderr.as_str()), ("x is 10\n", "oops\n"));
        // Values both ways.
        let o = w.run_blocking(
            "import datetime\n[1, 2.5, 'a', True, None, float('nan'), datetime.date(2024, 1, 1), datetime.datetime(1900, 1, 1, 12)]",
            &mut cells,
        );
        assert_eq!(
            value(&o),
            col(vec![1.0.into(), 2.5.into(), "a".into(), true.into(), Value::Empty, Value::Error(CellError::Num), 45292.0.into(), 2.5.into()])
        );
        assert_eq!(
            value(&w.run_blocking("{'k': 1}", &mut cells)),
            Array::new(1, 2, vec!["k".into(), 1.0.into()]).map(Value::from).unwrap_or_default()
        );
        assert_eq!(value(&w.run_blocking("10**400", &mut cells)), Value::Error(CellError::Num));
        let o = w.run_blocking("{1, 2}", &mut cells);
        let out = o.result.clone().unwrap_or_default();
        assert_eq!((out.type_name.as_str(), out.preview.as_str(), out.convertible), ("set", "{1, 2}", false));
        if i.pandas.is_none() {
            // Without pandas a range is a list of rows, as in the built-in Python.
            let o = w.run_blocking("t = xl('T', headers=True)\n[r['score'] for r in t]", &mut cells);
            assert_eq!(value(&o), col(vec![1.0.into(), "#DIV/0!".into()]));
            assert_eq!(value(&w.run_blocking("len(xl('T'))", &mut cells)), Value::Number(3.0));
        }
        // Errors carry their line; xl() errors are ValueErrors.
        let o = w.run_blocking("a = 1\nb = 2\n1 / 0", &mut cells);
        assert_eq!((failure(&o).line, failure(&o).kind), (Some(3), FailureKind::Error));
        assert!(failure(&o).message.starts_with("ZeroDivisionError"), "{o:?}");
        assert!(failure(&w.run_blocking("def f(:\n  pass", &mut cells)).message.starts_with("SyntaxError"));
        assert!(failure(&w.run_blocking("xl('nowhere')", &mut cells)).message.starts_with("ValueError"));
        let o = w.run_blocking("[[0] * 2000 for _ in range(1000)]", &mut cells);
        assert!(failure(&o).message.contains("more than 1000000 cells"), "{o:?}");
        // SystemExit is a cell error, not the end of the worker.
        assert_eq!(failure(&w.run_blocking("raise SystemExit(3)", &mut cells)).kind, FailureKind::Error);
        assert_eq!(value(&w.run_blocking("base", &mut cells)), Value::Number(100.0));
        // Reset forgets names and runs the initialization code again.
        w.reset();
        assert!(failure(&w.run_blocking("x", &mut cells)).message.starts_with("NameError"));
        assert_eq!(value(&w.run_blocking("base", &mut cells)), Value::Number(100.0));
        assert_eq!(w.starts(), 1);
    }

    #[test]
    fn the_shim_is_stopped_by_its_time_limit_and_by_the_parent() {
        let Some((mut w, _)) = cpython(Limits { timeout_ms: 1_000, ..Limits::default() }) else { return };
        assert_eq!(value(&w.run_blocking("n = 5\nn", &mut cells)), Value::Number(5.0));
        // An endless loop is interrupted by the shim's own timer: the worker keeps its namespace.
        let o = w.run_blocking("while True: pass", &mut cells);
        assert_eq!(failure(&o).kind, FailureKind::Timeout, "{o:?}");
        assert_eq!(value(&w.run_blocking("n", &mut cells)), Value::Number(5.0));
        // A long C call can't be interrupted: the parent kills the worker.
        let o = w.run_blocking("10 ** 10 ** 9", &mut cells);
        assert_eq!(failure(&o).kind, FailureKind::Timeout, "{o:?}");
        assert!(!w.alive());
        assert_eq!(value(&w.run_blocking("2 * 21", &mut cells)), Value::Number(42.0), "a fresh worker starts");
    }

    #[test]
    fn a_kill_stops_the_processes_a_cell_started() {
        let Some((mut w, _)) = cpython(Limits::default()) else { return };
        let o =
            w.run_blocking("import subprocess, sys\np = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\np.pid", &mut cells);
        let Value::Number(pid) = value(&o) else { panic!("{o:?}") };
        let pid = sysinfo::Pid::from_u32(pid as u32);
        let mut sys = sysinfo::System::new();
        let alive = |sys: &mut sysinfo::System| {
            sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
            sys.process(pid).is_some_and(|p| !matches!(p.status(), sysinfo::ProcessStatus::Zombie))
        };
        assert!(alive(&mut sys), "the child runs");
        w.kill();
        let mut gone = false;
        for _ in 0..200 {
            if !alive(&mut sys) {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(gone, "the child was killed with the worker's group");
    }

    #[test]
    fn pandas_frames_go_both_ways() {
        let Some((mut w, i)) = cpython(Limits::default()) else { return };
        if i.pandas.is_none() {
            eprintln!("skipped: no pandas in {}", i.program.display());
            return;
        }
        let o = w.run_blocking("df = xl('T', headers=True)\ntype(df).__name__", &mut cells);
        assert_eq!(value(&o), Value::text("DataFrame"));
        let o = w.run_blocking("df", &mut cells);
        assert_eq!(
            value(&o),
            Array::new(3, 2, vec!["name".into(), "score".into(), "a".into(), 1.0.into(), "b".into(), "#DIV/0!".into()])
                .map(Value::from)
                .unwrap_or_default()
        );
        // A named index shows as the first column; a Series spills as a column; numpy scalars
        // and arrays convert; Timestamps become serials.
        let o = w.run_blocking("import pandas as pd\npd.DataFrame({'v': [1, 2]}, index=pd.Index(['x', 'y'], name='k'))", &mut cells);
        assert_eq!(
            value(&o),
            Array::new(3, 2, vec!["k".into(), "v".into(), "x".into(), 1.0.into(), "y".into(), 2.0.into()]).map(Value::from).unwrap_or_default()
        );
        assert_eq!(value(&w.run_blocking("pd.Series([1.5, 2.5])", &mut cells)), col(vec![1.5.into(), 2.5.into()]));
        assert_eq!(value(&w.run_blocking("import numpy as np\nnp.int64(7)", &mut cells)), Value::Number(7.0));
        assert_eq!(
            value(&w.run_blocking("np.array([[1, 2], [3, 4]])", &mut cells)),
            Array::new(2, 2, vec![1.0.into(), 2.0.into(), 3.0.into(), 4.0.into()]).map(Value::from).unwrap_or_default()
        );
        assert_eq!(value(&w.run_blocking("pd.Timestamp('2024-01-01')", &mut cells)), Value::Number(45292.0));
    }
}
