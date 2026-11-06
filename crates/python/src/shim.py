# GridCraft's Python worker, run by the user's own Python (3.11 or newer).
#
# GridCraft starts it as `python -I -S -E shim.py <package folders...>` with an empty environment,
# from a private copy of this file, confined by the sandbox launcher. It speaks the JSON-lines protocol in crates/python/src/protocol.rs: requests arrive
# one per line on standard input; each gets exactly one reply line on standard output. This file
# is data that the user's interpreter runs; it is the one file in the repository that is not Rust.
#
# Everything lives inside one function, whose name is removed from the module before it runs, so
# the cells' code finds no helpers in `__main__`, and the protocol's pipes are held in closures.
# That keeps honest code away from the protocol; it can't stop code that goes looking, which is
# why GridCraft treats every line from here as untrusted.


def _gridcraft_worker():
    import ast
    import builtins
    import datetime
    import errno
    import importlib
    import io
    import json
    import math
    import os
    import sys
    import sysconfig
    import types

    globals().pop("_gridcraft_worker", None)

    # The caps of protocol.rs. GridCraft kills the worker for a line that breaks one, so the shim
    # keeps its replies within them.
    MAX_LINE_BYTES = 67108864
    MAX_TEXT_BYTES = 131072
    MAX_MESSAGE_BYTES = 8192
    MAX_SUBJECT_BYTES = 1024
    MAX_TYPE_NAME_BYTES = 256
    MAX_DATE_BYTES = 64
    MAX_PREVIEW_CHARS = 255
    MAX_SIZE_DIMS = 8
    MAX_SIZE = 9007199254740992
    MAX_TABLE_ROWS = 200
    MAX_TABLE_COLS = 30
    MAX_TABLE_TEXT_BYTES = 4096
    MAX_TABLE_BYTES = 1048576
    MAX_STDOUT_BYTES = 1048576
    MAX_ARRAY_CELLS = 1000000

    ERRORS = frozenset(
        ("#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA", "#SPILL!",
         "#CALC!", "#PYTHON!", "#BUSY!", "#TIMEOUT!", "#BLOCKED!", "#CONNECT!")
    )

    # Installed packages, without `site` (which would run .pth files): GridCraft worked out the
    # package folders and hands them over, since this process may not even know it belongs to a
    # virtual environment. Without any, the interpreter's own folders are used.
    folders = sys.argv[1:]
    if not folders:
        try:
            paths = sysconfig.get_paths()
        except Exception:
            paths = {}
        folders = [paths.get(key) for key in ("purelib", "platlib")]
    for p in folders:
        if p and p not in sys.path and os.path.isdir(p):
            sys.path.append(p)

    # The sandbox lets nothing be written, so bytecode caches aren't tried. Numerical libraries stay
    # on one thread.
    sys.dont_write_bytecode = True
    os.environ["OPENBLAS_NUM_THREADS"] = "1"

    # The protocol's pipes are private copies. Standard input becomes an empty pipe, so input()
    # can't read requests, and standard output goes to standard error, so text that C code or a
    # child process prints can't break a line.
    proto_in = os.fdopen(os.dup(0), "rb")
    proto_out = os.fdopen(os.dup(1), "wb")
    r, w = os.pipe()
    os.close(w)
    os.dup2(r, 0)
    os.close(r)
    os.dup2(2, 1)
    sys.stdin = io.StringIO("")
    encode = json.JSONEncoder(ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode
    decode = json.JSONDecoder().decode
    try:
        import signal

        signal.signal(signal.SIGINT, signal.SIG_IGN)
    except Exception:
        pass

    def send(msg):
        try:
            data = encode(msg).encode("utf-8", "replace")
        except Exception:
            data = None
        if data is None or len(data) > MAX_LINE_BYTES:
            data = encode(done_error(msg.get("id"), "The result is too large to send to GridCraft", None)).encode("utf-8")
        proto_out.write(data + b"\n")
        proto_out.flush()

    def done_error(rid, message, line, stdout="", kind=None, subject=None):
        err = {"message": cut_bytes(message, MAX_MESSAGE_BYTES)}
        if isinstance(line, int) and not isinstance(line, bool) and 0 <= line <= 4294967295:
            err["line"] = line
        if kind:
            err["kind"] = kind
        if isinstance(subject, str) and subject:
            err["subject"] = cut_bytes(subject, MAX_SUBJECT_BYTES)
        msg = {"type": "done", "id": rid, "ok": False, "error": err}
        if stdout:
            msg["stdout"] = stdout
        return msg

    def cut_bytes(s, n):
        b = s.encode("utf-8", "replace")
        return s if len(b) <= n else b[:n].decode("utf-8", "ignore")

    def cut_chars(s, n):
        return s if len(s) <= n else s[: n - 3] + "..."

    class TooBig(Exception):
        pass

    # Excel's error values, as Python sees them: xl() gives them for error cells, and a result
    # that holds one becomes that error again.
    class ExcelError:
        __slots__ = ("text",)

        def __init__(self, text):
            self.text = text

        def __repr__(self):
            return self.text

        __str__ = __repr__

        def __eq__(self, other):
            return isinstance(other, ExcelError) and other.text == self.text

        def __hash__(self):
            return hash(self.text)

    ExcelError.__module__ = "gridcraft"

    # Inputs: wire values to Python.
    def to_py(v):
        if isinstance(v, float):
            return int(v) if v.is_integer() and abs(v) < 2**53 else v
        if v is None or isinstance(v, (bool, int, str)):
            return v
        if isinstance(v, dict):
            if "error" in v:
                return ExcelError(str(v["error"]))
            a = v.get("array")
            if isinstance(a, dict):
                rows, cols, data = a.get("rows", 0), a.get("cols", 0), a.get("data", [])
                return [[to_py(x) for x in data[i * cols:(i + 1) * cols]] for i in range(rows)]
            return date_to_py(v)
        return None

    def date_to_py(v):
        # GridCraft works out dates from serial numbers, in the workbook's date system. A serial that
        # names no day never gets here: it arrives as a number.
        try:
            if "date" in v:
                return datetime.date.fromisoformat(v["date"])
            if "dateTime" in v:
                return datetime.datetime.fromisoformat(v["dateTime"])
            if "time" in v:
                return datetime.time.fromisoformat(v["time"])
        except (TypeError, ValueError):
            return ExcelError("#VALUE!")
        return None

    state = {"inputs": None}

    class XlReference(ValueError):
        pass

    XlReference.__name__ = "ValueError"
    XlReference.__qualname__ = "ValueError"

    # The names Excel's default initialization imports, so code written for it runs here. Each is
    # imported the first time it is used, so a package that isn't installed only matters to the code
    # that uses it, and starting Python stays quick.
    class Lazy:
        __slots__ = ("_module",)

        def __init__(self, module):
            object.__setattr__(self, "_module", module)

        def __getattr__(self, name):
            return getattr(importlib.import_module(object.__getattribute__(self, "_module")), name)

        def __repr__(self):
            return repr(importlib.import_module(object.__getattribute__(self, "_module")))

    DEFAULT_IMPORTS = {"np": "numpy", "pd": "pandas", "sm": "statsmodels.api"}

    # Python cells don't draw plots: the data goes to a sheet and charts are built from it. The
    # names Excel's initialization gives its plotting modules say so when they are used.
    class NoPlots(Exception):
        pass

    NoPlots.__name__ = "RuntimeError"
    NoPlots.__qualname__ = "RuntimeError"
    PLOT_MESSAGE = "Python cells don't draw plots. Return the data to a sheet and build a chart from it."
    PLOT_MODULES = ("matplotlib", "seaborn")
    PLOT_NAMES = ("plt", "sns")

    class Plotting:
        __slots__ = ()

        def __getattr__(self, name):
            raise NoPlots(PLOT_MESSAGE)

    def table(rows, headers):
        # A range is a pandas DataFrame, as in Excel; `headers=True` makes its first row the column
        # names. Without pandas it stays a list of rows.
        try:
            import pandas
        except Exception:
            return rows
        if headers and rows:
            return date_columns(pandas, pandas.DataFrame(rows[1:], columns=rows[0]))
        return date_columns(pandas, pandas.DataFrame(rows))

    def date_columns(pandas, frame):
        # pandas keeps datetime.date values as objects, and some versions datetime.datetime values
        # past its range too. A column of dates and empty cells becomes a datetime column, in the
        # units it can hold them in. Past what this pandas can do, or with something else in the
        # column, it stays as it is: the values are right either way.
        try:
            for i in range(frame.shape[1]):
                column = frame.iloc[:, i]
                if column.dtype == object and holds_only_dates(column.tolist()):
                    frame.isetitem(i, as_dates(pandas, column))
        except Exception:
            pass
        return frame

    def holds_only_dates(values):
        found = False
        for x in values:
            if isinstance(x, datetime.date):
                found = True
            elif x is not None:
                return False
        return found

    def as_dates(pandas, column):
        try:
            return pandas.to_datetime(column)
        except Exception:
            import numpy

            return pandas.Series(numpy.array(column.tolist(), dtype="datetime64[us]"), index=column.index)

    def xl(reference, headers=False):
        """xl(reference, headers=False): the value of a cell, or a range as a pandas DataFrame (a
        list of rows when pandas isn't installed). With headers=True the first row names the
        columns. A cell formatted as a date or a time is a datetime.date, datetime.time or
        datetime.datetime. The reference must be written as text in the cell's code, such as
        xl("A1:B5")."""
        inputs = state["inputs"]
        if inputs is None:
            raise RuntimeError("xl() only works while a cell runs")
        if not isinstance(reference, str):
            raise XlReference('xl() takes a reference written as text, such as xl("A1:B5")')
        entry = inputs.get(reference)
        if not isinstance(entry, dict):
            raise XlReference(
                'xl("%s"): only references written as text in this cell\'s code can be read' % cut_chars(reference, 100)
            )
        value = to_py(entry.get("value"))
        return table(value, headers) if isinstance(value, list) else value

    # Results: Python to wire values.
    NOT_SCALAR = object()

    def item(x):
        # A numpy scalar as the Python value it holds. A numpy date or duration stays what it is: its
        # own item() gives a bare integer in nanoseconds or past Python's years.
        get = getattr(x, "item", None)
        if get is not None and getattr(x, "shape", None) == ():
            if getattr(getattr(x, "dtype", None), "kind", None) in ("M", "m"):
                return x[()]
            try:
                return get()
            except Exception:
                return x
        return x

    def blank(x):
        # pandas's missing values are empty cells.
        x = item(x)
        if type(x).__name__ in ("NaTType", "NAType"):
            return None
        try:
            return None if x != x else x
        except Exception:
            return x

    def default_index(index):
        return type(index).__name__ == "RangeIndex" and index.start == 0 and index.step == 1

    def rows_of(v):
        """A pandas or numpy result as rows, or None for anything else. A table's column names are
        its first row, and its row labels its first column unless they are just 0, 1, 2..."""
        module = (type(v).__module__ or "").split(".")[0]
        name = type(v).__name__
        if module == "pandas" and name == "DataFrame":
            if v.shape[0] * v.shape[1] > MAX_ARRAY_CELLS:
                raise TooBig("A result can have at most %d cells" % MAX_ARRAY_CELLS)
            labels = not default_index(v.index)
            rows = [([v.index.name if v.index.name is not None else ""] if labels else []) + [item(c) for c in v.columns]]
            for label, row in zip(v.index, v.itertuples(index=False, name=None)):
                rows.append(([item(label)] if labels else []) + [blank(x) for x in row])
            return rows
        if module == "pandas" and name == "Series":
            if len(v) > MAX_ARRAY_CELLS:
                raise TooBig("A result can have at most %d cells" % MAX_ARRAY_CELLS)
            labels = not default_index(v.index)
            return [([item(label)] if labels else []) + [blank(x)] for label, x in zip(v.index, v.tolist())]
        if module == "pandas" and name in ("DatetimeIndex", "TimedeltaIndex"):
            if len(v) > MAX_ARRAY_CELLS:
                raise TooBig("A result can have at most %d cells" % MAX_ARRAY_CELLS)
            return [[x] for x in v.tolist()]
        if module == "numpy" and name == "ndarray" and v.ndim in (1, 2):
            if v.size > MAX_ARRAY_CELLS:
                raise TooBig("A result can have at most %d cells" % MAX_ARRAY_CELLS)
            if v.dtype.kind in ("M", "m"):
                # tolist() gives bare integers for nanoseconds: the elements keep their units.
                return [[x] for x in v] if v.ndim == 1 else [list(r) for r in v]
            return [[x] for x in v.tolist()] if v.ndim == 1 else v.tolist()
        return None

    def number(x):
        try:
            f = float(x)
        except (OverflowError, ValueError, TypeError):
            return {"error": "#NUM!"}
        return f if math.isfinite(f) else {"error": "#NUM!"}

    # Dates and durations, for a result. What Python can spell goes out as it is; GridCraft alone turns
    # it into a serial number, and gives #NUM! for what a serial can't hold.
    DAY_ATTOSECONDS = 86400 * 10**18
    NUMPY_UNITS = {
        "W": 7 * DAY_ATTOSECONDS,
        "D": DAY_ATTOSECONDS,
        "h": 3600 * 10**18,
        "m": 60 * 10**18,
        "s": 10**18,
        "ms": 10**15,
        "us": 10**12,
        "ns": 10**9,
        "ps": 10**6,
        "fs": 10**3,
        "as": 1,
    }
    NUMPY_NAT = -(2**63)

    def day_text(v):
        return "%04d-%02d-%02d" % (v.year, v.month, v.day)

    def clock_text(v):
        text = "%02d:%02d:%02d" % (v.hour, v.minute, v.second)
        return text + (".%06d" % v.microsecond if v.microsecond else "")

    def date_on_wire(key, text):
        return {key: text} if len(text) <= MAX_DATE_BYTES else {"error": "#NUM!"}

    def moment_on_wire(v, date_only_at_midnight):
        # The attributes are the local wall-clock time, whatever time zone the value has: Excel has
        # no time zones.
        clock = clock_text(v)
        if date_only_at_midnight and clock == "00:00:00":
            return date_on_wire("date", day_text(v))
        return date_on_wire("dateTime", day_text(v) + "T" + clock)

    def numpy_ticks(x):
        # How many of its unit a numpy date or duration holds, as a Python integer, since numpy's own
        # conversions between units overflow without a word. None for NaT.
        unit, count = sys.modules["numpy"].datetime_data(x.dtype)
        n = int(x.astype("int64"))
        return None if n == NUMPY_NAT else (unit, n * count)

    def numpy_date_on_wire(x):
        ticks = numpy_ticks(x)
        if ticks is None:
            return None
        unit, n = ticks
        try:
            if unit in ("Y", "M"):
                years, months = (n, 0) if unit == "Y" else divmod(n, 12)
                return date_on_wire("date", day_text(datetime.date(1970 + years, months + 1, 1)))
            days, rest = divmod(n * NUMPY_UNITS[unit], DAY_ATTOSECONDS)
            moment = datetime.datetime(1970, 1, 1) + datetime.timedelta(days=days, microseconds=rest // 10**12)
        except (KeyError, OverflowError, ValueError):
            # Past Python's years: no serial holds it either.
            return {"error": "#NUM!"}
        return moment_on_wire(moment, True)

    def numpy_duration_on_wire(x):
        ticks = numpy_ticks(x)
        if ticks is None:
            return None
        unit, n = ticks
        # Years and months have no length in days.
        if unit not in NUMPY_UNITS:
            return {"error": "#NUM!"}
        try:
            return {"duration": n * NUMPY_UNITS[unit] / DAY_ATTOSECONDS}
        except OverflowError:
            return {"error": "#NUM!"}

    def date_scalar(v):
        """A date, a time or a duration in its wire form, or NOT_SCALAR for anything else. pandas's
        Timestamp, Timedelta and NaT are subclasses of the datetime classes. pandas and numpy have
        no type for a date without a time, so midnight there means just the date."""
        name = type(v).__name__
        if name == "NaTType":
            return None
        if (type(v).__module__ or "").split(".")[0] == "numpy":
            if name == "datetime64":
                return numpy_date_on_wire(v)
            return numpy_duration_on_wire(v) if name == "timedelta64" else NOT_SCALAR
        if isinstance(v, datetime.datetime):
            return moment_on_wire(v, name == "Timestamp")
        if isinstance(v, datetime.date):
            return date_on_wire("date", day_text(v))
        if isinstance(v, datetime.time):
            return date_on_wire("time", clock_text(v))
        if isinstance(v, datetime.timedelta):
            return {"duration": float(v / datetime.timedelta(days=1))}
        return NOT_SCALAR

    def scalar(v):
        v = item(v)
        if v is None:
            return None
        if isinstance(v, bool):
            return v
        if isinstance(v, (int, float)):
            return number(v)
        if isinstance(v, str):
            if len(v) * 4 > MAX_TEXT_BYTES and len(str.encode(v, "utf-8", "replace")) > MAX_TEXT_BYTES:
                raise TooBig("A text result can have at most %d bytes" % MAX_TEXT_BYTES)
            return str(v)
        if isinstance(v, ExcelError):
            return {"error": v.text} if v.text in ERRORS else {"error": "#VALUE!"}
        try:
            return date_scalar(v)
        except Exception:
            return {"error": "#VALUE!"}

    def safe_repr(v):
        try:
            return str(repr(v))
        except BaseException:
            return "<%s object>" % type_name(v)

    def type_name(v):
        try:
            return cut_bytes(str(type(v).__name__), MAX_TYPE_NAME_BYTES)
        except BaseException:
            return "object"

    def element(x):
        s = scalar(x)
        return cut_chars(safe_repr(x), MAX_PREVIEW_CHARS) if s is NOT_SCALAR else s

    def grid(rows):
        cols = max((len(r) for r in rows), default=0)
        if cols == 0:
            return {"error": "#CALC!"}
        if len(rows) * cols > MAX_ARRAY_CELLS:
            raise TooBig("A result can have at most %d cells" % MAX_ARRAY_CELLS)
        data = []
        for r in rows:
            data.extend(element(x) for x in r)
            # Short rows are padded with #N/A, as Excel pads arrays.
            data.extend({"error": "#N/A"} for _ in range(cols - len(r)))
        return {"array": {"rows": len(rows), "cols": cols, "data": data}}

    def instance_of(v, module, name):
        # Only a package that is loaded can have made one, and a subclass counts.
        cls = getattr(sys.modules.get(module), name, None)
        return isinstance(cls, type) and isinstance(v, cls)

    def size_of(v):
        """What a Python object's name says about its size, or None when it has none to say."""
        if instance_of(v, "pandas", "DataFrame") or instance_of(v, "numpy", "ndarray"):
            dims = v.shape
        elif instance_of(v, "pandas", "Series") or isinstance(v, (list, tuple, dict, set, frozenset)):
            dims = (len(v),)
        else:
            return None
        dims = [int(n) for n in dims]
        if not 1 <= len(dims) <= MAX_SIZE_DIMS or not all(0 <= n <= MAX_SIZE for n in dims):
            return None
        return dims

    def wire_bytes(v):
        return len(encode(v).encode("utf-8", "replace")) + 1

    def preview_table(names, rows, total_rows, total_cols, cols, labels):
        """A table's preview, within the caps of protocol.rs. `rows` gives its rows, each already at
        most `cols` long: a short one is padded with #N/A, as a spill is. A long text is cut, and rows
        stop at the budget in bytes, and the totals say what is missing."""
        if total_rows > MAX_SIZE or total_cols > MAX_SIZE:
            return None
        cut = []

        def text(s):
            shown = cut_bytes(s, MAX_TABLE_TEXT_BYTES)
            if len(shown) != len(s):
                cut.append(True)
            return shown

        def name(n):
            n = item(n)
            if n is None:
                return ""
            try:
                return text(str(n))
            except BaseException:
                return text(safe_repr(n))

        def cell(x):
            x = item(x)
            try:
                if isinstance(x, str):
                    return text(str(x))
                s = scalar(x)
            except BaseException:
                s = NOT_SCALAR
            if s is not NOT_SCALAR:
                return s
            whole = safe_repr(x)
            shown = cut_chars(whole, MAX_PREVIEW_CHARS)
            if len(shown) != len(whole):
                cut.append(True)
            return shown

        names = [name(n) for n in names] if names is not None else []
        spent = sum(wire_bytes(n) for n in names)
        data = []
        count = 0
        for row in rows:
            cells = [cell(x) for x in row]
            cells.extend({"error": "#N/A"} for _ in range(cols - len(cells)))
            spent += sum(wire_bytes(c) for c in cells)
            if spent > MAX_TABLE_BYTES:
                break
            data.extend(cells)
            count += 1
        table = {"rows": count, "cols": cols, "data": data, "totalRows": total_rows, "totalCols": total_cols}
        if any(names):
            table["columns"] = names
        if labels:
            table["rowLabels"] = True
        if cut:
            table["textCut"] = True
        return table

    def frame_table(v):
        # As a spill has it: the column names, and the row labels unless they are just 0, 1, 2...
        labels = not default_index(v.index)
        part = v.iloc[: MAX_TABLE_ROWS, : MAX_TABLE_COLS - (1 if labels else 0)]
        names = ([v.index.name] if labels else []) + list(part.columns)

        def rows():
            for label, row in zip(part.index, part.itertuples(index=False, name=None)):
                yield ([label] if labels else []) + [blank(x) for x in row]

        return preview_table(names, rows(), int(v.shape[0]), int(v.shape[1]), len(names), labels)

    def series_table(v):
        labels = not default_index(v.index)
        part = v.iloc[: MAX_TABLE_ROWS]
        names = ([v.index.name] if labels else []) + [v.name]

        def rows():
            for label, x in zip(part.index, part.tolist()):
                yield ([label] if labels else []) + [blank(x)]

        return preview_table(names, rows(), len(v), 1, len(names), labels)

    def array_table(v):
        part = v[: MAX_TABLE_ROWS, : MAX_TABLE_COLS]
        # tolist() gives bare integers for nanoseconds: the elements keep their units.
        rows = [list(r) for r in part] if part.dtype.kind in ("M", "m") else part.tolist()
        return preview_table(None, rows, int(v.shape[0]), int(v.shape[1]), int(part.shape[1]), False)

    def rows_table(v):
        cols = max(len(r) for r in v)
        rows = (list(r[: MAX_TABLE_COLS]) for r in v[: MAX_TABLE_ROWS])
        return preview_table(None, rows, len(v), cols, min(cols, MAX_TABLE_COLS), False)

    def table_of(v):
        """A bounded look at a table-like object, or None for anything else."""
        if instance_of(v, "pandas", "DataFrame"):
            return frame_table(v)
        if instance_of(v, "pandas", "Series"):
            return series_table(v)
        if instance_of(v, "numpy", "ndarray"):
            return array_table(v) if v.ndim == 2 else None
        if isinstance(v, (list, tuple)) and len(v) and all(isinstance(r, (list, tuple)) for r in v):
            return rows_table(v)
        return None

    def unless_it_fails(find, v):
        # The object's own methods run here, and the cell has its result whatever they do.
        try:
            return find(v)
        except BaseException:
            return None

    def as_object(v, sized):
        o = {"typeName": type_name(v), "preview": cut_chars(safe_repr(v), MAX_PREVIEW_CHARS)}
        if sized:
            size = unless_it_fails(size_of, v)
            table = unless_it_fails(table_of, v)
            if size:
                o["size"] = size
            if table:
                o["table"] = table
        return {"object": o}

    def output(v, want_object):
        s = scalar(v)
        if s is not NOT_SCALAR:
            return s
        if want_object:
            return as_object(v, True)
        rows = rows_of(v)
        if rows is not None:
            return grid(rows) if rows else {"error": "#CALC!"}
        if isinstance(v, (list, tuple)):
            if not v:
                return {"error": "#CALC!"}
            if all(isinstance(r, (list, tuple)) for r in v):
                return grid(v)
            # A flat list is one column.
            if len(v) > MAX_ARRAY_CELLS:
                raise TooBig("A result can have at most %d cells" % MAX_ARRAY_CELLS)
            return grid([[x] for x in v])
        return as_object(v, False)

    # Printed output, kept up to the cap.
    class Capture(io.TextIOBase):
        CUT = "\n[output cut off]\n"

        def __init__(self):
            self.parts = []
            self.left = MAX_STDOUT_BYTES - len(self.CUT)
            self.cut = False

        def writable(self):
            return True

        def write(self, s):
            s = str(s)
            if self.cut:
                return len(s)
            b = s.encode("utf-8", "replace")
            if len(b) <= self.left:
                self.parts.append(s)
                self.left -= len(b)
            else:
                self.parts.append(b[: self.left].decode("utf-8", "ignore"))
                self.parts.append(self.CUT)
                self.left = 0
                self.cut = True
            return len(s)

        def text(self):
            return "".join(self.parts)

    def error_line(e):
        if isinstance(e, SyntaxError) and e.filename == "<cell>":
            return e.lineno
        line = None
        tb = e.__traceback__
        while tb is not None:
            if tb.tb_frame.f_code.co_filename == "<cell>":
                line = tb.tb_lineno
            tb = tb.tb_next
        return line

    def error_message(e):
        name = type_name(e)
        if isinstance(e, SyntaxError):
            return "%s: %s" % (name, e.msg)
        try:
            text = str(e)
        except BaseException:
            text = ""
        return "%s: %s" % (name, text) if text else name

    # What kind of failure an error is, in the words of the diagnostics catalog (diagnostics.rs), so
    # the person is told what to do about it. A guess is fine: the message goes along with it.
    NETWORK_FILES = ("socket.py", "ssl.py", "selectors.py", "connection.py", "connectionpool.py", "client.py", "request.py", "adapters.py")
    NETWORK_FOLDERS = ("asyncio", "urllib", "urllib3", "http", "requests", "httpx", "aiohttp")
    PROCESS_FILES = ("subprocess.py", "popen_fork.py", "popen_spawn_posix.py", "popen_forkserver.py", "spawn.py")
    PROCESS_FOLDERS = ("multiprocessing", "joblib", "loky")
    NETWORK_WORDS = ("socket", "connect", "urlopen", "getaddrinfo")
    PROCESS_WORDS = ("fork", "exec", "spawn", "system(", "popen", "Popen", "subprocess")

    def chain(e):
        seen = 0
        while e is not None and seen < 16:
            yield e
            seen += 1
            link = e.__cause__ or e.__context__ or getattr(e, "reason", None)
            e = link if isinstance(link, BaseException) else None

    def denied_kind(e, code):
        """Which kind of denial a permission error is, from where it was raised."""
        tb = e.__traceback__
        line = None
        while tb is not None:
            name = tb.tb_frame.f_code.co_filename
            base = os.path.basename(name)
            parts = name.replace("\\", "/").split("/")
            if base in PROCESS_FILES or any(f in parts for f in PROCESS_FOLDERS):
                return "denied.process"
            if base in NETWORK_FILES or any(f in parts for f in NETWORK_FOLDERS):
                return "denied.network"
            if name == "<cell>":
                line = tb.tb_lineno
            tb = tb.tb_next
        lines = code.split("\n") if isinstance(code, str) else []
        text = lines[line - 1] if isinstance(line, int) and 0 < line <= len(lines) else ""
        if any(w in text for w in PROCESS_WORDS):
            return "denied.process"
        if any(w in text for w in NETWORK_WORDS):
            return "denied.network"
        return "denied.file"

    def through_plotting(e):
        """Whether an error came from a plotting package, or pandas's way into one."""
        for link in chain(e):
            tb = link.__traceback__
            while tb is not None:
                parts = tb.tb_frame.f_code.co_filename.replace("\\", "/").split("/")
                if any(m in parts for m in PLOT_MODULES) or ("pandas" in parts and "plotting" in parts):
                    return True
                tb = tb.tb_next
        return False

    def classify(e, code):
        """The catalog's kind for an error, and what it was about."""
        if isinstance(e, SyntaxError):
            return "python.syntax", None
        if isinstance(e, XlReference):
            return "xl.notLiteral", None
        if isinstance(e, NoPlots) or through_plotting(e):
            return "plot.unsupported", None
        if isinstance(e, ModuleNotFoundError):
            if (e.name or "").split(".")[0] in PLOT_MODULES:
                return "plot.unsupported", None
            return "module.missing", e.name
        if isinstance(e, ImportError):
            text = str(e)
            if "sandbox" in text or "not permitted" in text or "Permission denied" in text:
                return "module.blockedLibrary", getattr(e, "name", None)
            return "python.exception", None
        if isinstance(e, NameError):
            return "python.name", getattr(e, "name", None)
        for link in chain(e):
            if isinstance(link, PermissionError) or (isinstance(link, OSError) and link.errno in (errno.EPERM, errno.EACCES)):
                kind = denied_kind(link, code)
                return kind, str(link.filename) if kind == "denied.file" and link.filename else None
            if isinstance(link, socket_errors()):
                return "denied.network", None
        if isinstance(e, FileNotFoundError) and "No usable temporary directory" in str(e):
            return "denied.file", None
        return "python.exception", None

    def socket_errors():
        s = sys.modules.get("socket")
        return (s.gaierror,) if s is not None else ()

    def compile_cell(code):
        tree = ast.parse(code, "<cell>", "exec")
        last = None
        if tree.body and isinstance(tree.body[-1], ast.Expr):
            last = ast.Expression(tree.body.pop().value)
        body = compile(tree, "<cell>", "exec")
        return body, (compile(last, "<cell>", "eval") if last is not None else None)

    # Initialization code. What a line binds, and which package a line that can't run is about: that
    # is reported, and an empty answer is a line left out without a word (a plotting module, which
    # cells don't use). None means the error is the code's own.
    def bound_by(stmt):
        if isinstance(stmt, (ast.Import, ast.ImportFrom)):
            return [a.asname or a.name.split(".")[0] for a in stmt.names]
        return [n.id for n in ast.walk(stmt) if isinstance(n, ast.Name) and isinstance(n.ctx, ast.Store)]

    def modules_of(stmt):
        if isinstance(stmt, ast.Import):
            return [a.name.split(".")[0] for a in stmt.names]
        if isinstance(stmt, ast.ImportFrom) and stmt.level == 0:
            return [(stmt.module or "").split(".")[0]]
        return []

    def module_call(stmt, g):
        # A call on a module at the top level, such as warnings.simplefilter("ignore"): what setting
        # an option looks like. The module's name, or None.
        call = stmt.value if isinstance(stmt, ast.Expr) else None
        if not isinstance(call, ast.Call) or not isinstance(call.func, ast.Attribute):
            return None
        root = call.func
        while isinstance(root, ast.Attribute):
            root = root.value
        if isinstance(root, ast.Name) and isinstance(g.get(root.id), (types.ModuleType, Lazy, Plotting)):
            return root.id
        return None

    def left_to_lazy(stmt):
        # What the worker already gives lazily, and plots, which cells don't draw: Excel's default
        # initialization imports these, and doing so would only cost the time it takes.
        if isinstance(stmt, ast.ImportFrom):
            return any(m in PLOT_MODULES for m in modules_of(stmt))
        if not isinstance(stmt, ast.Import):
            return False
        for alias in stmt.names:
            top = alias.name.split(".")[0]
            lazy = DEFAULT_IMPORTS.get(alias.asname or top, "").split(".")[0]
            if top not in PLOT_MODULES and lazy != top:
                return False
        return True

    def why_skipped(e, stmt, gone, g, from_file):
        if isinstance(e, NameError) and getattr(e, "name", None) in gone:
            return gone[e.name]
        modules = modules_of(stmt)
        if not from_file:
            return "excel" if modules == ["excel"] and isinstance(e, ImportError) else None
        if isinstance(e, NoPlots) or through_plotting(e):
            return ""
        if modules:
            name = getattr(e, "name", None)
            return (name if isinstance(name, str) and name else modules[0]).split(".")[0]
        if isinstance(e, ImportError) and e.name:
            return e.name.split(".")[0]
        return module_call(stmt, g) if isinstance(e, Exception) else None

    def initialize(code, from_file):
        tree = ast.parse(code, "<cell>", "exec")
        g = namespace["globals"]
        if not from_file and not any("excel" in modules_of(s) for s in tree.body):
            exec(compile(tree, "<cell>", "exec"), g)
            return None
        skipped = {}
        gone = {}
        for stmt in tree.body:
            if from_file and left_to_lazy(stmt):
                gone.update((name, "") for name in bound_by(stmt) if name not in g)
                continue
            try:
                exec(compile(ast.Module([stmt], []), "<cell>", "exec"), g)
            except BaseException as e:  # noqa: BLE001 - what the rules don't skip is raised again
                why = why_skipped(e, stmt, gone, g, from_file)
                if why is None:
                    raise
                if why:
                    skipped.setdefault(why, stmt.lineno)
                for name in bound_by(stmt):
                    gone.setdefault(name, why)
        return [[module, line] for module, line in skipped.items()] or None

    namespace = {}

    def fresh():
        # The cells share one `__main__`, as in Excel; Reset starts a new one.
        main = types.ModuleType("__main__")
        main.__dict__["__builtins__"] = builtins
        main.__dict__["xl"] = xl
        for name, module in DEFAULT_IMPORTS.items():
            main.__dict__[name] = Lazy(module)
        for name in PLOT_NAMES:
            main.__dict__[name] = Plotting()
        sys.modules["__main__"] = main
        namespace["globals"] = main.__dict__

    def run(req):
        rid = req.get("id")
        code = req.get("code")
        inputs = req.get("inputs")
        init = req.get("init")
        capture = Capture()
        saved = sys.stdout
        try:
            if not isinstance(code, str):
                raise TypeError("no code")
            # xl() has no cell to read for while initialization code runs.
            state["inputs"] = None if init else inputs if isinstance(inputs, dict) else {}
            sys.stdout = capture
            try:
                if init:
                    value = initialize(code, init == "file")
                else:
                    body, last = compile_cell(code)
                    g = namespace["globals"]
                    exec(body, g)
                    value = eval(last, g) if last is not None else None
            finally:
                sys.stdout = saved
                state["inputs"] = None
            msg = {"type": "done", "id": rid, "ok": True, "value": output(value, req.get("output") == "object")}
            if capture.text():
                msg["stdout"] = capture.text()
            return msg
        except TooBig as e:
            return done_error(rid, str(e), None, capture.text())
        except BaseException as e:  # noqa: BLE001 - whatever the cell raised is its result
            try:
                kind, subject = classify(e, code)
            except Exception:
                kind, subject = None, None
            return done_error(rid, error_message(e), error_line(e), capture.text(), kind, subject)

    try:
        import resource

        # ru_maxrss counts bytes on macOS and KiB elsewhere.
        peak_unit = 1 if sys.platform == "darwin" else 1024

        def peak():
            return int(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss) * peak_unit
    except Exception:
        peak = None

    def with_peak(msg):
        # The most this process has used, so that a spike counts against the request that made it,
        # however short it was. Left out when it can't be read.
        try:
            if peak is not None:
                msg["peak"] = peak()
        except Exception:
            pass
        return msg

    fresh()
    send({"type": "ready", "selfTest": {"status": "notChecked"}})
    while True:
        line = proto_in.readline()
        if not line:
            break
        try:
            req = decode(line.decode("utf-8"))
        except ValueError:
            break
        kind = req.get("type") if isinstance(req, dict) else None
        if kind == "run":
            send(with_peak(run(req)))
        elif kind == "reset":
            fresh()
            send(with_peak({"type": "done", "id": req.get("id"), "ok": True, "value": None}))
        else:
            break
    # GridCraft closed the pipe: stop now, without waiting for threads the cells started.
    os._exit(0)


_gridcraft_worker()
