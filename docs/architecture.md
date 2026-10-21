# Architecture

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** minor (Python in cells: dates and times in `xl()` and in results) · **Target:** Microsoft Excel (Microsoft 365)

How GridCraft is built today. Rules for contributors are in [`AGENTS.md`](../AGENTS.md).

## Workspace and layering

An engine-first Cargo workspace. Layering is enforced by `cargo xtask layers`; nothing below L6
depends on egui, eframe, winit or rfd, so the UI crate is swappable. Line counts are Rust lines
including tests, 2026-10-10.

| Layer | Crate | Lines | Job |
|---|---|---|---|
| L0 | `core` | ~1.3k | Cell and range addresses (A1, R1C1, sheet-qualified; 1,048,576 × 16,384), `Value`, `CellError`, Excel coercion, 1900/1904 date serials, typed-input parsing |
| L0 | `numfmt` | ~2.1k | Excel number-format codes (sections, conditions, colours, dates, elapsed time, fractions, scientific, fill), General, `TEXT()`; en-US only |
| L1 | `formula` | ~1.7k | Tokenizer, parser, AST, printer (A1 and R1C1), reference adjustment for copy, fill and structural edits |
| L1 | `functions` | ~13.4k | The function library: `FnSpec` registry (name, arity, category, scalar args, signature, description) and pure implementations; automatic array lifting |
| L1 | `python` | ~2.8k | Python in cells: the JSON-lines worker protocol, the built-in worker (Monty, compiled out of the web build), the client that starts, watches and kills worker processes, the search for the user's CPython and the shim it runs |
| L2 | `model` | ~2.2k | Workbook, sheets, copy-on-write cell store (64-row bands behind `Arc`), interned styles, names, tables, CF, validation, comments, charts, shapes, images, sparklines, print settings, protection |
| L2 | `calc` | ~2.9k | Evaluator (references, names, tables, LET/LAMBDA, special forms like OFFSET/INDIRECT/SUBTOTAL) and dependency-graph recalculation with dynamic arrays, spills and cycle detection; single-threaded |
| L2 | `xlsx` | ~10.7k | XLSX read/write (transitional and strict read), CSV/TSV, XLSB and ODS data import, PivotTable caches, DrawingML charts and drawings; byte slices only (WASM-safe); lenient reader that reports warnings |
| L2 | `pdf` | ~1k | Small PDF 1.7 writer (14 standard fonts, vectors, images) for print and export |
| L3 | `chart` | ~2.9k | Toolkit-free chart layout to drawing primitives, a rasterizer, sparklines |
| L5 | `engine` | ~17.7k | `Session`: open workbooks, selection, history (COW snapshots), clipboard, fill, sort, filter, PivotTables, what-if, print, file I/O (`io.rs`), and the command registry (`cmd/*`, 313 commands) with the Excel ribbon catalog (`catalog.rs`) |
| L6 | `ui-egui` | ~11.6k | Excel-style UI (title bar, QAT, ribbon, formula bar, virtualised grid, editor, tabs, status bar, dialogs, task panes, chart view) and the control channel |
| L6 | `mcp` | ~1.4k | MCP server over stdio (hand-written JSON-RPC), headless or bridged to a running app |
| L7 | `apps/gridcraft`, `apps/gridcraft-cli`, `apps/gridcraft-web` | ~2.2k | Desktop (eframe/wgpu), CLI (`info`, `convert`, `eval`, `cat`, `run`, `commands`, `functions`, `mcp`, `send`), web (WASM via trunk) |
| — | `xtask` | — | `ci`, `parity`, `assets`, `layers`, `wasm`, `version`, `stats` |

## Data flow

1. **Every action is a command.** The UI, CLI, control channel and MCP call
   `Session::execute(id, params)`. Commands are `CommandSpec { id, label, menu, shortcut, params,
   enabled, run, journal, undoable }`; ids follow Excel's ribbon (`home.bold`, `insert.chart`,
   `data.sortAscending`) plus primitives (`cell.set`, `selection.set`, `range.setValues`).
   `Session::execute` catches escaped panics and restores the workbook (never-crash guard).
2. **Edits** change the model through copy-on-write bands; undo keeps the previous snapshot.
3. **Recalc** marks dependents of changed cells dirty through the dependency graph (with range
   nodes for range dependents), evaluates in topological order, spills dynamic arrays and reports
   `#SPILL!`/cycles. Volatile functions are always dirty.
4. **Rendering**: the egui grid reads display values (number formats applied in `engine/display.rs`)
   for the visible window only; charts render through `crates/chart` primitives.
5. **Files**: `engine/io.rs` sniffs content (XLSX, XLSB, ODS) before trusting the extension and
   dispatches to `crates/xlsx`; imports that lose data return warnings shown to the user.
6. **Python cells** run after the command that affected them (see [Python in cells](#python-in-cells)).

## Engine building blocks for multi-command actions

Actions made of many commands (macros, a debugger that runs one step per call) use these parts of
`Session` and the engine:

- **Undo groups.** `Session::grouped(label, f)` runs `f` as one undo step and rolls every change
  back if `f` fails; `group_begin`, `group_end` and `group_rollback` do the same across calls.
  While a group is open on a document, its commands add no undo steps of their own and Undo and
  Redo are disabled there. Groups nest; the outermost one makes the step.
- **Recording buffer** (`record.rs`). `Session::start_recording` collects each successful
  top-level command with the document, sheet and selection it ran in, without secret parameters.
  Commands run by other commands are not recorded, and the buffer is independent of the journal
  (Automate › Record Actions still records through the journal).
- **Document trust.** `DocState::trust` is `Local` for workbooks made in the session and
  `FromFile` for workbooks opened from a file, and for workbooks that received formulas, formats
  or validation pasted from one. Active content in a `FromFile` workbook asks before it runs;
  `Trusted` marks a file whose active content the person allowed (`python.enable`).
- **Workbook data in hidden names** (`cmd/stash.rs`). An item is split over hidden
  workbook-scope names `_gc_<key>_0`, `_gc_<key>_1`, … of string literals within Excel's 255- and
  8,192-character limits, so it survives an xlsx round trip. The older single-name `_sc_<key>`
  items are read as well.

## Python in cells

A Python cell (`Cell::py`, `model/python.rs`) holds code, an output mode (cell values or a Python
object) and its last result; its `value` is the cached result, so formulas, display and files read
it like any other cell. Code is written with `python.set`, never by typing `=PY(`.

- **Worker processes.** Code never runs in the GridCraft process. Each workbook gets a worker
  the first time one of its cells runs: the user's CPython 3.11 or newer when the search in
  `python/interp.rs` finds one (run as `<python> -P -X utf8 -u -c <shim>`), otherwise the app
  binary started again with `--python-worker`, which runs Monty, a Python interpreter written in
  Rust for sandboxed code. A worker is killed one second after its cell's time limit (5 s by
  default) or when its resident memory passes 512 MB, and a crashed worker is replaced; the
  workbook is never at risk. Workers start in a process group of their own (a job object on
  Windows), so a kill also stops processes the code started. `shim.py`, the CPython worker, is
  data that the user's interpreter runs, embedded with `include_str!`; it is the one piece of the
  application that is not Rust.
- **Protocol** (`python/protocol.rs`). One JSON object per line. The parent sends `Init`, `Run`,
  `XlValue` and `Reset`; the worker answers `Ready`, `Xl` (a request for cell values) and `Done`
  (value, printed output, error and line, time), and the CPython shim also sends `Extend` while
  it imports a module not loaded yet (see time limits below). Worker frames start with a marker,
  so stray output is never taken for a frame. Values travel as `gridcraft_core::Value`.
- **Dates and times.** An `xl()` reply for one area (a cell, a range, a name or table of one
  range) marks the numbers whose number format is a date or time format (`XlReply::Dated`, with a
  `DateCell {at, kind}` per number: `date`, `time` or `dateTime`, from whether the format shows a
  calendar part, a clock part or both); numbers no date stands for (negative, past 9999-12-31)
  are not marked. Serials cross the protocol in the 1900 date system, and the engine converts a
  1904 workbook's in both directions. The shim makes `datetime.date`, `datetime.time` and
  `datetime.datetime` values of them, to the millisecond, with Excel's fictitious 1900-02-29
  (serial 60, like serial 0, stays a number, as does a time of a day or more); a date whose serial
  has a time of day is a `datetime` even under a date-only format. When `xl()` builds a
  DataFrame, a column whose values are all dates becomes a `datetime64` column. The other way,
  `date`, `datetime` (with pandas `Timestamp` and numpy `datetime64`) and `time` results become
  serials marked in `Output::dates`, and the cells that show them get `m/d/yyyy`,
  `m/d/yyyy h:mm` or `h:mm AM/PM` (the formats `TODAY()`, `NOW()` and `TIME()` give) where their
  format is General; a spill that is blocked formats nothing. The built-in Python receives the
  serial numbers, since its results cannot be read back as dates.
- **Built-in and CPython.** Monty has no ambient access: file, environment and network
  operations suspend to the worker, which refuses them. It lacks most of the standard library and
  every third-party package, so a cell that imports one, or uses a construct Monty's compiler does
  not support, is refused before any of it runs, with a message that asks for CPython. CPython runs
  with the access of the person's account, so pandas and numpy work.
- **Packages** (`python/env.rs`, `engine/pyenv.rs`). GridCraft keeps one virtual environment of
  its own, made from the CPython it found (it never downloads an interpreter), in the app's data
  folder: `~/Library/Application Support/GridCraft/python-env` on macOS,
  `$XDG_DATA_HOME/gridcraft/python-env` (by default `~/.local/share`) on Linux and BSD, and
  `%LOCALAPPDATA%\GridCraft\python-env` on Windows; `GRIDCRAFT_PYTHON_ENV` or
  `python.options {environment}` move it. The search for Python tries it after `VIRTUAL_ENV` and
  `CONDA_PREFIX` and before `PATH`. `python.install` installs only after the person approves the
  exact list (`approve: true`; otherwise it answers with the question as `EngineError::Approve`),
  and only into a virtual environment: the person's own venv or conda environment when that runs
  cells, otherwise GridCraft's (made first with `uv venv --seed` or `python -m venv`), never a
  system or externally managed Python. Requirements are a name, plain extras and a version
  specifier only (no URLs, paths, options or markers; at most 32). The install runs `uv pip
  install` when `uv` is on `PATH` (and the target is not a conda environment), else `python -m
  pip install`, with `--only-binary :all:`, from
  an argument list, on a thread with a 10-minute deadline; `python.stop` cancels it. When it
  succeeds, the search runs again and every workbook's Python cells run again. Workbooks declare
  the packages they need with `python.packages` (kept in hidden names, so they travel in xlsx).
- **Time limits and imports.** On CPython, time spent importing modules not loaded yet does not
  count against a cell's time limit, up to `python.options {importSeconds}` (60 s by default):
  the shim wraps `builtins.__import__`, checks the run's time less its import time every 50 ms,
  and sends `Extend {id, ms, module}` heartbeats so the parent moves its own deadline by the same
  amount (never past the allowance); the parent's kill stays the backstop. A worker loads numpy
  and pandas, when installed, before it reports ready, and its start-up is not counted as the
  first cell's time. A time limit that interrupts an import replaces the worker, since a package
  may be half loaded. A cell stopped at its time limit mid-import, or on its first run on a new
  worker, runs once more on a fresh worker (a `retried` row in Diagnostics) before the crash-loop
  guard refuses it. Each install ends by importing what it installed once under a 120 s deadline
  (a warm-up, so the first import in a cell is fast; a failure there is a warning, not a failed
  install), and uv installs compile bytecode.
- **Runtime** (`engine/python.rs`). After every top-level command that changed a workbook,
  `Session::execute` compares the workbook before and after and runs the Python cells whose code
  changed or whose `xl()` references read changed cells, and every Python cell after the first of
  those in row-major order, sheet by sheet, since all of them share one namespace. Results are
  written into the workbook without an undo step of their own, then calc recalculates their
  dependents. Headless callers run to the end inside the command; a host with a frame loop calls
  `set_python_background(true)` and `step_python()`, and waiting cells show `#BUSY!`;
  `python.stop` kills the workbook's worker and leaves every unfinished cell as `#PYTHON!`
  ("Stopped") until the next edit. A cell that took its worker down is not run again until its
  code or input changes (crash-loop guard).
- **Trust gate.** A `FromFile` workbook runs no Python: opening shows the cached values, and new
  code there shows `#BLOCKED!` until `python.enable` makes the workbook `Trusted`.
- **Files.** xlsx stores a Python cell as `PY("…"&"…",0|1)` with its cached value (a spill as a
  dynamic array); `engine/io.rs` adopts `PY(…)` formulas from every format and drops Python content
  over its limits (7,000 characters of code, results over 1,000,000 cells).
- **Web.** `python/client_web.rs` stands in for the client on wasm32 with the same API; there is
  no worker, and Python cells show `#BLOCKED!`.

## Agent control

- Desktop control channel: `gridcraft --control 7979`, JSON lines on localhost
  (`engine.execute`, `ui.screenshot`, pointer and keyboard input…), see
  [`control-protocol.md`](control-protocol.md).
- MCP: `gridcraft-cli mcp`, headless or `--connect` to a running app, see [`mcp.md`](mcp.md).
- CLI: [`cli.md`](cli.md).

## Platforms and builds

macOS universal, Windows x64/x86/arm64, Linux x86_64/aarch64/riscv64 (AppImage, Flatpak, deb,
rpm, tarball), FreeBSD, and web (WASM; WebGPU or WebGL2; no Python cells). Release workflows in
`.github/workflows/release.yml`; see [`releasing.md`](releasing.md).

## Known architectural gaps

- No threading in calc (see [hardware-parity.md](hardware-parity.md)).
- No string catalog or text shaping (see [localization-parity.md](localization-parity.md)).
- XLSX reader builds the whole workbook in memory; no streaming (#175).
- Unmodelled XLSX parts are not carried through a save.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | Python in cells: dates and times in `xl()` and in results (`XlReply::Dated`, `Output::dates`, the 1904 system) |
| 2026-10-12 | minor | Python in cells: import-aware time limits (`Extend` heartbeats, `importSeconds`, one retry, install warm-up) |
| 2026-10-12 | minor | Python in cells: GridCraft's own Python environment and package installs the person approves |
| 2026-10-12 | minor | Python in cells: the `python` crate (L1), worker processes, protocol, built-in and CPython backends, the engine runtime, the trust gate, files and the web build |
| 2026-10-11 | minor | Engine building blocks for multi-command actions: undo groups, recording buffer, document trust, chunked hidden-name storage |
| 2026-10-10 | major | Created from the code on main: crates, layers, data flow, agent control, known gaps |
