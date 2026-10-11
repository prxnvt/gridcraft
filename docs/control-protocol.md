# GridCraft control protocol

Start the desktop app with a control port:

```sh
gridcraft --control 7979            # or GRIDCRAFT_CONTROL_PORT=7979
```

Then send newline-delimited JSON to `127.0.0.1:7979`. Every request line gets one reply line:

```json
{"id": 1, "method": "engine.execute", "params": {"command": "cell.set", "params": {"cell": "B2", "input": "=SUM(A1:A9)"}}}
{"id": 1, "ok": true, "result": {"cell": "B2", "value": "45"}}
```

Errors come back as `{"id": …, "ok": false, "error": "…"}`.

## Methods

| Method | Params | What it does |
|---|---|---|
| `engine.execute` | `{command, params, wait?, waitSeconds?}` | Runs any engine command (see `engine.commands` or `gridcraft-cli commands`); with `wait: true` the reply waits until Python cells finish (see below) |
| `engine.commands` | `{search?}` | Every command: id, label, ribbon path, shortcut, params doc, enabled; `search` keeps those whose id, label or ribbon path contains it (case-insensitive) |
| `engine.journal` | | Commands run so far (replayable) |
| `document.inspect` | | Workbook summary: sheets, used ranges, tables, charts, names, selection, undo labels |
| `sheet.read` | `{range?, sheet?, formulas?, formatted?}` | Values (or formulas / displayed text) of a range |
| `cell.get` | `{cell?}` | One cell: value, formula, displayed text, style, merge, comment, link |
| `ui.inspect` | | UI state: ribbon tab, editor, open dialog, message, selected object, grid rect, scroll, perf |
| `ui.object.select` | `{id?, kind?}` | Selects a floating object on the active sheet (no `id` clears it); replies `{selected, rect}`, the rect in window points |
| `ui.click` / `ui.drag` / `ui.move` | `{x, y, toX?, toY?, button?, count?, shift?, cmd?, alt?}` | Real pointer input in window points |
| `ui.cell.click` / `ui.cell.drag` | `{cell}` / `{from, to}` | Pointer input at a cell's centre (works in point mode while editing) |
| `ui.key` / `ui.text` | `{key, shift?, cmd?, alt?, ctrl?}` / `{text}` | Keyboard input (typing in the grid starts editing) |
| `ui.edit.begin` / `ui.edit.text` / `ui.edit.commit` / `ui.edit.cancel` | `{text?}` / `{move?: down\|right\|up\|left\|none}` | Drive the cell editor |
| `ui.ribbon` | `{tab}` | Switch ribbon tab |
| `ui.dialog` / `ui.dialog.set` / `ui.dialog.confirm` / `ui.dialog.cancel` | `{name}` / `{field, value}` | Open, fill in and confirm dialogs. `{name: "macroEditor", macro, debug?, check?}` opens the macro step editor (`debug: true` pauses before the first step, `check: true` shows the dry run on the grid); `{name: "macros"}` the Macros dialog |
| `ui.dialog` / `ui.dialog.set` / `ui.dialog.confirm` / `ui.dialog.cancel` | `{name}` / `{field, value}` | Open, fill in and confirm dialogs |
| `ui.pane` (through `engine.execute`) | `{name?}` | Show a task pane (`comments`, `watch`, `selection`, `formatChart`, `pythonDiagnostics`); no name closes it |
| `ui.pythonEditor` (through `engine.execute`) | `{cell?, sheet?, close?}` | Open the Python editor window on a cell (default: the active cell, or the Python cell being edited; a selected Python cell starts being edited in it); `close: true` closes it and keeps any edit. Replies `{open, sheet, cell, editing, dirty, python}`, which `ui.inspect` also reports as `pythonEditor` |
| `ui.screenshot` | `{path?}` | PNG of the window (base64 when no path) |
| `ui.set` | `{dark?, formulaBar?, ribbonCollapsed?}` | UI preferences |
| `ui.resize` / `ui.focus` | `{width, height}` | Window control |
| `app.open` / `app.save` / `app.quit` | `{path}` / `{path?}` | Files and lifetime |

Any other method name that is a command id (e.g. `home.bold`) runs that command.

`ui.inspect` reports the selected floating object as `selectedObject: {kind, id}` (or `null`),
where `kind` is `chart`, `image`, `shape`, `equation` or `control`; the same names go in the
`kind` parameter of `object.delete`, `object.move`, `object.setAnchorMode` and the `arrange.*`
commands. `selectedChart` is kept as an alias: the id when the selection is a chart, otherwise
`null`. With an object selected, `ui.key {key: "Delete"}` (or Backspace) deletes the object
instead of clearing cells, and Escape clears the selection.

Python cells run in the background in the app, so `python.set` answers at once and the cells it
starts show `#BUSY!` until they finish. `ui.inspect` reports `pythonBusy: true` while any are
running or waiting; poll it (or `cell.get`) for the results, or pass `wait: true` to
`engine.execute`: the reply then waits, without holding up the window, until Python is idle or
`waitSeconds` pass (default: the Python time limit plus one second). A result with a `cell` and
its `value` (such as `python.set`'s) then carries the cell's final value, and the reply has
`pythonBusy: false` (true when the wait ran out). `python.stop` (or `ui.key {key:
"Escape"}` with no editor open) stops them. `python.run {cell?}` runs a Python cell again (the
active cell by default) and the Python cells after it, which share its namespace; it goes through
the same background run, so `wait: true` works with it too. `editor.python` is true while the
formula bar edits a Python cell's code; Ctrl+Enter (Cmd+Enter on macOS) commits it, as does the
formula bar's Run button (Stop while the cell runs or waits, Re-run once it has a result).

`python.install {packages, approve: true}` installs Python packages in the background too (into
the person's own virtual environment when it runs cells, else GridCraft's own environment, made
first if needed; never into a system Python). Without `approve: true` it answers with the
approval question instead. `pythonBusy` stays true while it runs, so `wait: true` with a
generous `waitSeconds` (an install may take minutes; the limit is 10) waits for it;
`python.installStatus` reports `running`, `installed`, `installFailed` or `cancelled` with the
captured output, a `summary` such as "Installed scikit-learn 1.9.1" and the `versions` installed,
and `python.stop` cancels it. Every install is also a row in `python.diagnostics` (no sheet or
cell; kept whatever `sheet` says) and a line in the app log. When it succeeds, Python restarts on the
environment and the Python cells run again. `python.environment` reports GridCraft's environment
(`{action: "create"}` makes it, `{action: "delete", approve: true}` removes it); its folder is the
app's data folder, or `GRIDCRAFT_PYTHON_ENV`, or `python.options {environment}`.

Use `engine.execute` with `{command: "view.theme", params: {mode: "system"}}`
to choose the display theme; `mode` accepts `"system"`, `"light"`, or `"dark"`.
System follows live OS appearance changes and falls back to Light when the platform supplies no preference.
The saved choice remains System; new users still start in Light. The legacy
`view.darkMode` command and `ui.set {dark}` select a fixed Light or Dark theme.

`{command: "app.language.set", params: {language: "ja"}}` switches the interface language
(`code` is accepted as an alias for `language`); `app.language.english` and
`app.language.japanese` are shortcuts. Only interface chrome is translated, never command ids or
document text. The choice is saved in `ui.json`; with nothing saved the desktop app follows the
system language.

For a headless equivalent (no window), use `gridcraft-cli mcp` or `gridcraft-cli run`;
see [`mcp.md`](mcp.md) and [`cli.md`](cli.md).
