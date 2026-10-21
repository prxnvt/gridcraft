# GridCraft's Python worker for the user's own Python (CPython 3.11 or newer).
#
# GridCraft runs this with `python -P -X utf8 -u -c <this file>`. It speaks the same JSON-lines
# protocol as the built-in worker (crates/python/src/protocol.rs): requests arrive one per line on
# standard input, and each reply is one line on the original standard output, after a marker.
# This file is data that the user's interpreter runs, not part of the app's code.

import ast
import builtins
import datetime as _dt
import io
import json
import math
import os
import queue
import signal
import sys
import threading
import time
import _thread

MARK = "\x1egcpy "
PREVIEW_CHARS = 255
ERRORS = {
    "Null": "#NULL!", "Div0": "#DIV/0!", "Value": "#VALUE!", "Ref": "#REF!", "Name": "#NAME?",
    "Num": "#NUM!", "NA": "#N/A", "GettingData": "#GETTING_DATA", "Spill": "#SPILL!",
    "Calc": "#CALC!", "Circ": "#CIRC!", "Python": "#PYTHON!", "Busy": "#BUSY!",
    "Timeout": "#TIMEOUT!", "Blocked": "#BLOCKED!",
}
# Dates travel as serial numbers in the 1900 date system: days since 1899-12-30, except that,
# as in Excel, serial 60 is the fictitious 1900-02-29 and earlier serials are one day lower.
EPOCH = _dt.datetime(1899, 12, 30)
# Serial 2958465 is 9999-12-31, the last day Excel shows.
LAST_DAY = 2958465
DAY_MS = 86400000

# Frames get a private copy of standard output; the real one now goes to standard error, so text
# that C code or child processes print can't break a frame.
_frames = os.fdopen(os.dup(1), "w", encoding="utf-8", newline="\n")
os.dup2(2, 1)
_inbox = queue.Queue()
# The time limit interrupts the main thread through SIGINT's handler; a parent started with SIGINT
# ignored (a background job) passes that on, and Python then leaves the handler out.
signal.signal(signal.SIGINT, signal.default_int_handler)


def _read_input():
    for line in sys.stdin:
        _inbox.put(line)
    # GridCraft went away: stop at once, even while code runs.
    os._exit(0)


threading.Thread(target=_read_input, daemon=True).start()


_send_lock = threading.Lock()


def _send(msg):
    try:
        text = json.dumps(msg, ensure_ascii=False, allow_nan=False)
    except (TypeError, ValueError):
        text = "{}"
    # The watchdog thread sends too (its heartbeats while a module imports).
    with _send_lock:
        _frames.write(MARK + text + "\n")
        _frames.flush()


class _Limits:
    timeout = 5.0
    # Time spent importing modules that weren't loaded yet doesn't count against the time limit,
    # up to this much per run (a package's first import after an install can be slow).
    imports = 60.0
    stdout_bytes = 1024 * 1024
    output_cells = 1_000_000
    xl_calls = 10_000


_state = {"run": None, "fired": False, "xl_calls": 0, "pandas": False}
_lock = threading.Lock()


# Printed output, kept per run up to a size.
class _Capture:
    def __init__(self, limit):
        self.parts = {"out": [], "err": []}
        self.left = limit
        self.cut = False

    def push(self, which, text):
        size = len(text.encode("utf-8", "replace"))
        if size <= self.left:
            self.parts[which].append(text)
            self.left -= size
            return
        keep = text.encode("utf-8", "replace")[: max(self.left, 0)].decode("utf-8", "ignore")
        self.parts[which].append(keep)
        self.left = 0
        if not self.cut:
            self.cut = True
            self.parts[which].append("\n[output cut off]\n")

    def text(self, which):
        return "".join(self.parts[which])


class _Stream(io.TextIOBase):
    def __init__(self, capture, which):
        self.capture, self.which = capture, which

    def writable(self):
        return True

    def write(self, s):
        s = str(s)
        self.capture.push(self.which, s)
        return len(s)

    @property
    def encoding(self):
        return "utf-8"


def _cut(s, n=PREVIEW_CHARS):
    return s if len(s) <= n else s[:n] + "..."


def _repr(v):
    try:
        return _cut(repr(v))
    except BaseException:
        return "<%s>" % type(v).__name__


# Python values to cells.
def _error(name):
    return {"t": "Error", "v": name}


def _number(x):
    try:
        x = float(x)
    except (OverflowError, ValueError, TypeError):
        return _error("Num")
    return {"t": "Number", "v": x} if math.isfinite(x) else _error("Num")


def _leap_bug(days):
    # Days since 1899-12-30 as a serial: before 1900-03-01, one lower (serial 60 is 1900-02-29).
    return days - 1 if days < 61 else days


def _dated(x, kind):
    # A date's serial as a cell, marked with its kind; a date before 1900 has no serial that
    # Excel shows, so it stays a plain number.
    cell = _number(x)
    if cell["t"] == "Number" and (kind == "time" or cell["v"] >= 1):
        cell["k"] = kind
    return cell


def _serial(v):
    if isinstance(v, _dt.datetime):
        if v.tzinfo is not None:
            v = v.replace(tzinfo=None)
        return _dated(_leap_bug((v - EPOCH).total_seconds() / 86400.0), "dateTime")
    if isinstance(v, _dt.date):
        return _dated(_leap_bug((v - EPOCH.date()).days), "date")
    seconds = v.hour * 3600 + v.minute * 60 + v.second + v.microsecond / 1e6
    return _dated(seconds / 86400.0, "time")


def _scalar(v):
    if v is None:
        return {"t": "Empty"}
    if isinstance(v, bool):
        return {"t": "Bool", "v": v}
    np = sys.modules.get("numpy")
    if np is not None and isinstance(v, np.generic):
        if isinstance(v, np.bool_):
            return {"t": "Bool", "v": bool(v)}
        if isinstance(v, (np.integer, np.floating)):
            return _number(v)
        if isinstance(v, np.datetime64):
            if np.isnat(v):
                return {"t": "Empty"}
            return _dated(_leap_bug(float((v - np.datetime64("1899-12-30")) / np.timedelta64(1, "D"))), "dateTime")
        if isinstance(v, np.str_):
            return {"t": "Text", "v": str(v)}
        return None
    if isinstance(v, (int, float)):
        return _number(v)
    if isinstance(v, str):
        return {"t": "Text", "v": v}
    if isinstance(v, (_dt.datetime, _dt.date, _dt.time)):
        pd = sys.modules.get("pandas")
        if pd is not None and v is pd.NaT:
            return {"t": "Empty"}
        return _serial(v)
    pd = sys.modules.get("pandas")
    if pd is not None and (v is pd.NaT or v is pd.NA):
        return {"t": "Empty"}
    return None


def _element(v):
    s = _scalar(v)
    return s if s is not None else {"t": "Text", "v": _repr(v)}


class _TooBig(Exception):
    pass


def _array(rows, cols, data):
    return {"t": "Array", "v": {"rows": rows, "cols": cols, "data": data}}


def _check(rows, cols):
    if rows * cols > _Limits.output_cells:
        raise _TooBig("The result has more than %d cells" % _Limits.output_cells)


def _grid(rows):
    rows = [list(r) for r in rows]
    if not rows:
        return _error("Calc")
    cols = max(1, max(len(r) for r in rows))
    _check(len(rows), cols)
    data = []
    for r in rows:
        data.extend(_element(x) for x in r)
        # Short rows are padded with #N/A, as Excel pads arrays.
        data.extend(_error("NA") for _ in range(cols - len(r)))
    return _array(len(rows), cols, data)


def _table(v):
    pd = sys.modules.get("pandas")
    np = sys.modules.get("numpy")
    if pd is not None and isinstance(v, pd.DataFrame):
        idx = v.index
        default = isinstance(idx, pd.RangeIndex) and idx.start == 0 and idx.step == 1 and idx.name is None
        _check(len(v) + 1, len(v.columns) + (0 if default else 1))
        head = ([] if default else [str(idx.name) if idx.name is not None else ""]) + [str(c) for c in v.columns]
        body = [([] if default else [i]) + list(row) for i, row in zip(idx, v.itertuples(index=False, name=None))]
        return _grid([head] + body)
    if pd is not None and isinstance(v, pd.Series):
        idx = v.index
        if isinstance(idx, pd.RangeIndex) and idx.start == 0 and idx.step == 1:
            _check(len(v), 1)
            return _grid([[x] for x in v.tolist()])
        _check(len(v), 2)
        return _grid([[i, x] for i, x in zip(idx, v.tolist())])
    if np is not None and isinstance(v, np.ndarray):
        if v.ndim == 1:
            _check(v.shape[0], 1)
            return _grid([[x] for x in v.tolist()])
        if v.ndim == 2:
            _check(v.shape[0], v.shape[1])
            return _grid(v.tolist())
        return None
    if isinstance(v, (list, tuple)):
        if not v:
            return _error("Calc")
        if all(isinstance(r, (list, tuple)) for r in v):
            return _grid(v)
        _check(len(v), 1)
        return _grid([[x] for x in v])
    if isinstance(v, dict):
        if not v:
            return _error("Calc")
        _check(len(v), 2)
        return _grid([[k, x] for k, x in v.items()])
    return None


def _take_dates(value):
    # The positions of the cells marked as dates, with their kinds; the marks are removed.
    dates = []
    if value.get("t") == "Array":
        for i, cell in enumerate(value["v"]["data"]):
            kind = cell.pop("k", None)
            if kind:
                dates.append({"at": i, "kind": kind})
    else:
        kind = value.pop("k", None)
        if kind:
            dates.append({"at": 0, "kind": kind})
    return dates


def _output(v):
    preview = _repr(v)
    value = _scalar(v)
    if value is None:
        np = sys.modules.get("numpy")
        if np is not None and isinstance(v, np.ndarray) and v.ndim == 0:
            value = _element(v.item())
        else:
            value = _table(v)
    convertible = value is not None
    if value is None:
        value = {"t": "Text", "v": preview}
    dates = _take_dates(value)
    return {"value": value, "typeName": type(v).__name__, "preview": preview, "convertible": convertible, "dates": dates}


# Cells to Python values.
def _py(v):
    t = v.get("t")
    if t == "Number":
        x = v.get("v", 0.0)
        return int(x) if float(x).is_integer() and abs(x) < 2**53 else x
    if t in ("Text", "Bool"):
        return v.get("v")
    if t == "Error":
        return ERRORS.get(v.get("v"), "#VALUE!")
    if t == "Array":
        a = v.get("v", {})
        flat = [_py(x) for x in a.get("data", [])]
        cols = max(1, a.get("cols", 1))
        return [flat[i:i + cols] for i in range(0, len(flat), cols)]
    return None


def _from_serial(x, kind):
    # A serial as a datetime.date, datetime.datetime or datetime.time, to the millisecond (the
    # precision of Excel's times). A number that is no such value stays a number: one before
    # 1900-01-01, the fictitious 1900-02-29, one after 9999-12-31, or, for a time, one of a day
    # or more. A date with a time of day is a datetime even when only the date is shown.
    if isinstance(x, bool) or not isinstance(x, (int, float)) or not math.isfinite(x):
        return x
    if not (0 <= x < (1 if kind == "time" else LAST_DAY + 1)):
        return x
    day = math.floor(x)
    ms = round((x - day) * DAY_MS)
    if ms >= DAY_MS:
        day, ms = day + 1, ms - DAY_MS
    clock = _dt.time(ms // 3600000, ms // 60000 % 60, ms // 1000 % 60, ms % 1000 * 1000)
    if kind == "time":
        return clock if day == 0 else x
    if day < 1 or day == 60 or day > LAST_DAY:
        return x
    date = EPOCH.date() + _dt.timedelta(days=day + 1 if day < 60 else day)
    if kind == "date" and ms == 0:
        return date
    return _dt.datetime.combine(date, clock)


def _with_dates(v, dates):
    # The cells marked as dates or times become datetime values; also returns the columns that
    # hold dates (pandas makes datetime columns of them).
    columns = set()
    if not isinstance(v, list):
        for d in dates:
            if isinstance(d, dict) and d.get("at") == 0:
                return _from_serial(v, d.get("kind")), columns
        return v, columns
    cols = max(1, len(v[0]) if v else 1)
    for d in dates:
        if not isinstance(d, dict) or not isinstance(d.get("at"), int):
            continue
        r, c = divmod(d["at"], cols)
        if 0 <= r < len(v) and c < len(v[r]):
            v[r][c] = _from_serial(v[r][c], d.get("kind"))
            if d.get("kind") != "time":
                columns.add(c)
    return v, columns


def _date_columns(pd, df, columns):
    # A column whose values are all dates (or empty) becomes a datetime64 column.
    for c in sorted(columns):
        if c >= len(df.columns):
            continue
        col = df.iloc[:, c]
        if all(x is None or isinstance(x, _dt.date) for x in col):
            try:
                df.isetitem(c, pd.to_datetime(col))
            except Exception:  # noqa: BLE001 - the column keeps its values as they are
                pass
    return df


def _xl_value(value, headers, dates=()):
    v, columns = _with_dates(_py(value), dates)
    if not isinstance(v, list):
        return v
    if _state["pandas"]:
        try:
            import pandas as pd

            if headers and v:
                return _date_columns(pd, pd.DataFrame(v[1:], columns=[str(h) for h in v[0]]), columns)
            return _date_columns(pd, pd.DataFrame(v), columns)
        except ImportError:
            _state["pandas"] = False
    if headers and v:
        keys = [str(h) for h in v[0]]
        return [dict(zip(keys, r)) for r in v[1:]]
    return v


def xl(reference, headers=False):
    """xl(reference, headers=False): the value of a cell, or a range as a pandas DataFrame (a list
    of rows without pandas; a list of dicts with headers=True). Cells formatted as dates or times
    are datetime values."""
    if not isinstance(reference, str):
        raise TypeError('xl() takes a reference as text, such as xl("A1:B5")')
    run = _state["run"]
    if run is None:
        raise RuntimeError("xl() only works while a cell runs")
    _state["xl_calls"] += 1
    if _state["xl_calls"] > _Limits.xl_calls:
        raise RuntimeError("A cell may call xl() at most %d times" % _Limits.xl_calls)
    _send({"type": "xl", "id": run, "reference": reference, "headers": bool(headers)})
    while True:
        try:
            msg = json.loads(_inbox.get())
        except ValueError:
            continue
        if msg.get("type") == "xlValue" and msg.get("id") == run:
            break
    reply = msg.get("reply") or {}
    if "error" in reply:
        raise ValueError(reply["error"])
    if "dated" in reply:
        dated = reply.get("dated") or {}
        return _xl_value(dated.get("value") or {"t": "Empty"}, headers, dated.get("dates") or [])
    return _xl_value(reply.get("value") or {"t": "Empty"}, headers)


# Imports: the time a run spends loading modules that weren't loaded yet, timed on the main
# thread for the outermost import only (the modules it pulls in are inside its time).
_imp = {"depth": 0, "start": 0.0, "total": 0.0, "module": None}
_original_import = builtins.__import__


def _timed_import(name, globals=None, locals=None, fromlist=(), level=0):
    if (
        _imp["depth"]
        or _state["run"] is None
        or level != 0
        or name in sys.modules
        or threading.current_thread() is not threading.main_thread()
    ):
        return _original_import(name, globals, locals, fromlist, level)
    _imp["depth"], _imp["start"], _imp["module"] = 1, time.perf_counter(), name.partition(".")[0]
    try:
        return _original_import(name, globals, locals, fromlist, level)
    finally:
        _imp["total"] += time.perf_counter() - _imp["start"]
        _imp["depth"], _imp["module"] = 0, None


builtins.__import__ = _timed_import


def _import_seconds():
    # The run's import time so far, with the import in progress.
    total = _imp["total"]
    if _imp["depth"]:
        total += time.perf_counter() - _imp["start"]
    return total


# Running code.
def _fire(token):
    with _lock:
        if _state["run"] == token and not _state["fired"]:
            _state["fired"] = True
            _thread.interrupt_main()


def _line(e):
    if isinstance(e, SyntaxError) and e.filename == "<cell>":
        return e.lineno
    line = None
    tb = e.__traceback__
    while tb is not None:
        if tb.tb_frame.f_code.co_filename == "<cell>":
            line = tb.tb_lineno
        tb = tb.tb_next
    return line


def _message(e):
    if isinstance(e, SyntaxError):
        return "%s: %s" % (type(e).__name__, e.msg)
    if isinstance(e, ModuleNotFoundError) and e.name:
        # GridCraft finds the package to install from this message.
        return "ModuleNotFoundError: No module named '%s'" % e.name
    text = str(e)
    return "%s: %s" % (type(e).__name__, text) if text else type(e).__name__


def _compile(code):
    tree = ast.parse(code, "<cell>", "exec")
    last = None
    if tree.body and isinstance(tree.body[-1], ast.Expr):
        last = ast.Expression(tree.body.pop().value)
    body = compile(tree, "<cell>", "exec")
    return body, (compile(last, "<cell>", "eval") if last is not None else None)


_namespace = {}


def _fresh():
    global _namespace
    _namespace = {"__name__": "__main__", "__builtins__": builtins, "xl": xl}


def _watch(token, started, stop):
    # The time limit: every 50 ms, the time the run has taken less what it spent importing (up to
    # the allowance) is checked against the limit. While an import takes long, the parent hears
    # how long, so its own deadline (the backstop) moves too.
    sent = 0
    while not stop.wait(0.05):
        with _lock:
            if _state["run"] != token or _state["fired"]:
                return
        imports = min(_import_seconds(), _Limits.imports)
        if time.perf_counter() - started - imports >= _Limits.timeout:
            _state["importing"] = _imp["module"]
            _fire(token)
            return
        ms = int(imports * 1000)
        if ms - sent >= 100 or (_imp["module"] is None and ms != sent):
            sent = ms
            _send({"type": "extend", "id": token, "ms": ms, "module": _imp["module"]})


def _timeout_error():
    return {"line": None, "message": "TimeoutError: time limit exceeded", "timeout": True, "importing": _state.get("importing")}


def _run(token, code):
    capture = _Capture(_Limits.stdout_bytes)
    started = time.perf_counter()
    done = {"type": "done", "id": token, "output": None, "stdout": "", "stderr": "", "error": None, "elapsedMs": 0.0}
    saved = sys.stdout, sys.stderr
    stop = threading.Event()
    timer = threading.Thread(target=_watch, args=(token, started, stop), daemon=True)
    modules = len(sys.modules)
    _imp["total"], _imp["depth"], _imp["module"] = 0.0, 0, None
    with _lock:
        _state["run"], _state["fired"], _state["xl_calls"], _state["importing"] = token, False, 0, None
    try:
        sys.stdout, sys.stderr = _Stream(capture, "out"), _Stream(capture, "err")
        timer.start()
        try:
            body, last = _compile(code)
            exec(body, _namespace)
            value = eval(last, _namespace) if last is not None else None
            with _lock:
                _state["run"] = None
            stop.set()
            sys.stdout, sys.stderr = saved
            try:
                done["output"] = _output(value)
            except _TooBig as e:
                done["error"] = {"line": None, "message": str(e), "timeout": False}
        except KeyboardInterrupt:
            done["error"] = _timeout_error()
        except BaseException as e:  # noqa: BLE001 - a cell's error, whatever it is, is its result
            done["error"] = {"line": _line(e), "message": _message(e), "timeout": False}
    except KeyboardInterrupt:
        done["error"] = _timeout_error()
    finally:
        stop.set()
        with _lock:
            _state["run"] = None
        sys.stdout, sys.stderr = saved
    # A time limit that interrupted an import can leave a package half loaded, which breaks every
    # later import of it: GridCraft starts a fresh worker (and may run the cell again on it).
    err = done["error"]
    if err and err.get("timeout") and (len(sys.modules) != modules or err.get("importing") is not None):
        err["restart"] = True
    done["stdout"], done["stderr"] = capture.text("out"), capture.text("err")
    done["elapsedMs"] = (time.perf_counter() - started) * 1000.0
    return done


def _restart(init_code):
    _fresh()
    error = None
    if init_code.strip():
        error = _run(0, init_code)["error"]
    _send({"type": "ready", "initError": error})


def _main():
    init_code = ""
    while True:
        try:
            try:
                msg = json.loads(_inbox.get())
            except ValueError:
                continue
            kind = msg.get("type")
            if kind == "init":
                limits = msg.get("limits") or {}
                _Limits.timeout = max(0.001, float(limits.get("timeoutMs", 5000)) / 1000.0)
                _Limits.stdout_bytes = int(limits.get("stdoutBytes", _Limits.stdout_bytes))
                _Limits.output_cells = int(limits.get("outputCells", _Limits.output_cells))
                _Limits.xl_calls = int(limits.get("xlCalls", _Limits.xl_calls))
                _Limits.imports = max(0.0, float(limits.get("importMs", 60000)) / 1000.0)
                # Packages such as pandas call deeper than the built-in Python's limit allows.
                sys.setrecursionlimit(max(1000, int(limits.get("recursion", 200)) + 100))
                try:
                    import importlib.util

                    _state["pandas"] = importlib.util.find_spec("pandas") is not None
                except Exception:
                    _state["pandas"] = False
                # numpy and pandas load now, before any cell's time limit runs (a cold start
                # of pandas can take seconds).
                for name in ("numpy", "pandas"):
                    try:
                        if importlib.util.find_spec(name) is not None:
                            __import__(name)
                    except Exception:
                        pass
                init_code = msg.get("initCode") or ""
                _restart(init_code)
            elif kind == "reset":
                _restart(init_code)
            elif kind == "run":
                _send(_run(msg.get("id", 0), msg.get("code") or ""))
        except KeyboardInterrupt:
            # A time limit that fired just as its run ended.
            continue


_main()
