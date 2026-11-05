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
    MAX_PREVIEW_CHARS = 255
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
            return pandas.DataFrame(rows[1:], columns=rows[0])
        return pandas.DataFrame(rows)

    def xl(reference, headers=False):
        """xl(reference, headers=False): the value of a cell, or a range as a pandas DataFrame (a
        list of rows when pandas isn't installed). With headers=True the first row names the
        columns. The reference must be written as text in the cell's code, such as xl("A1:B5")."""
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
        # A numpy scalar as the Python value it holds.
        get = getattr(x, "item", None)
        if get is not None and getattr(x, "shape", None) == ():
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
        if module == "numpy" and name == "ndarray" and v.ndim in (1, 2):
            if v.size > MAX_ARRAY_CELLS:
                raise TooBig("A result can have at most %d cells" % MAX_ARRAY_CELLS)
            return [[x] for x in v.tolist()] if v.ndim == 1 else v.tolist()
        return None

    def number(x):
        try:
            f = float(x)
        except (OverflowError, ValueError, TypeError):
            return {"error": "#NUM!"}
        return f if math.isfinite(f) else {"error": "#NUM!"}

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
        return NOT_SCALAR

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

    def as_object(v):
        return {"object": {"typeName": type_name(v), "preview": cut_chars(safe_repr(v), MAX_PREVIEW_CHARS)}}

    def output(v, want_object):
        s = scalar(v)
        if s is not NOT_SCALAR:
            return s
        if want_object:
            return as_object(v)
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
        return as_object(v)

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
        capture = Capture()
        saved = sys.stdout
        try:
            if not isinstance(code, str):
                raise TypeError("no code")
            state["inputs"] = inputs if isinstance(inputs, dict) else {}
            sys.stdout = capture
            try:
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
            send(run(req))
        elif kind == "reset":
            fresh()
            send({"type": "done", "id": req.get("id"), "ok": True, "value": None})
        else:
            break
    # GridCraft closed the pipe: stop now, without waiting for threads the cells started.
    os._exit(0)


_gridcraft_worker()
