# Python in cells

> **Last updated:** 2026-11-05 · **Change:** minor (the Python tab is contextual; earlier: first version: how to turn Python on, how to set it up with uv, what cells can and can't do, and the limits)

A Python cell runs code you write, in your own Python, on your computer. GridCraft never downloads
Python and never installs packages. It finds the Python you have, and it runs your code in a sandbox
that keeps the code away from everything but the cells it names.

Python runs on macOS and on Linux (x86_64 and aarch64, kernel 5.13 or newer). On Windows, FreeBSD and
the web version GridCraft has no sandbox, so cells don't run there: they keep the values saved in
the file, or show `#BLOCKED!`. The code is still stored and saved.

## Turning Python on

Python is off until you turn it on.

1. Open **File › Options › Python** and tick **Enable Python in cells**.
2. GridCraft looks for a Python. The page shows what it found, with its version and kind.
3. A workbook opened from a file needs its own **Enable**, on the bar that shows under the ribbon.
   Until then its cells keep the values saved in the file. **This Workbook** on the Python tab is the
   same switch. A workbook made in GridCraft needs none.
4. Select a cell, choose **Formulas › Insert Python** (Ctrl+Alt+Shift+P), type `1 + 1`, and press
   Ctrl+Enter. The cell shows `2`.

Only a person can do this, in the app. No file, macro, command, MCP call or control-channel request
can turn Python on, choose the interpreter or enable a workbook. The CLI and MCP never run Python
cells: they read the values saved in the file. Anyone who can change your preferences file, or who
operates your mouse and keyboard, acts as you do; this doesn't protect against that.

## Setting up Python with uv

When GridCraft finds no usable Python, the Options page gives these commands with Copy buttons.
GridCraft copies them; it never runs them. Run them in a terminal, in order.

```sh
curl -LsSf https://astral.sh/uv/install.sh | sh   # only if you don't have uv
uv venv ~/.gridcraft/python --python 3.12
uv pip install --python ~/.gridcraft/python numpy pandas statsmodels
```

GridCraft looks in `~/.gridcraft/python` first, so there is nothing more to choose. Then choose
**Check Again**. To add a package later, use **Packages** on the Python tab: type its name and choose
**Copy Install Command**, which gives the uv command for the Python in use.

**Which Python.** Automatic tries, in order: `~/.gridcraft/python`, the active virtual environment,
uv's own Pythons, the active conda environment, `PATH`, and the places Pythons are usually installed.
Python 3.11 or newer is needed. **Interpreter** on the Python tab and the Options page list what was found and
take any other path with **Browse…**. While Python is off GridCraft starts nothing, so the list shows
paths only, without versions.

**Other package managers.** Any Python 3.11 or newer can be chosen by path. The Options page and the
Packages pane have a collapsed section with the pip command.

Packages must be installed inside the Python that GridCraft uses. Packages installed for your user
(`pip install --user`) and editable installs (`pip install -e`) aren't seen.

## What cells can and can't do

**What Python Can Do** on the Python tab lists this for your system, with what enforces each line.

Cells can:

- read cells through `xl("A1:B5")`, with the reference written as text in the code;
- import the packages installed in that Python;
- compute, print (the output is kept in the Run Log) and use threads.

Cells can't:

- read your files or write files;
- use the network;
- start other programs;
- see your environment variables;
- open windows, or ask for input;
- draw plots. Python cells don't draw plots: the data goes to a sheet and a chart is built from it,
  so `matplotlib` and `seaborn` are not supported.

Cells run in row order, sheet by sheet in tab order, as in Excel, and share one set of variables.
**Reset** clears them and runs every cell again. Each time Python starts, GridCraft first tries to
read a file outside Python, write a file, open a network connection and start a program, and it
doesn't run Python unless every one is refused. [sandbox.md](sandbox.md) has the details.

## Limits

- **Time:** 30 seconds per cell by default, from 1 second to 1 hour, plus 10 seconds for the first
  cell while Python starts.
- **Memory:** 1 GB by default, from 128 MB to 256 GB. On macOS this is a check twice a second, not a
  hard cap.
- **Results:** at most 1,000,000 cells, and 128 KiB of text in one value.
- **Printed output:** 1 MiB, then it is cut off.

The time and memory limits are in **File › Options › Python**. At a limit Python is restarted and its
variables are cleared; the next run starts fresh.

## The Python tab

The Python tab is contextual, like Table Design. It is there while the active cell is a Python cell,
or while you edit Python code, in the formula bar or the editor window, and it sits at the right end
of the tab bar. Selecting a Python cell adds the tab and leaves the ribbon where it was; click the
tab, or press Alt, Y, to show it. When you leave the cell the tab goes, and if it was showing, the
ribbon returns to Home. A pane you opened from it stays open.

**Formulas › Python** is the way in from anywhere: Insert Python starts Python mode, which brings the
tab up, and Reset and Diagnostics are there too. File › Options › Python and the status bar item work
anywhere, and each pane has its own command.

- **Code:** Insert Python, Editor, Output As (Python Object or Excel Value). Initialization is
  disabled: GridCraft has no initialization code yet.
- **Run:** Run Cell, Run All, Stop, Reset, and Python Calculation (Automatic or Manual; under Manual,
  cells whose inputs changed wait for Run Cell, Run All or F9, and are marked stale).
- **Environment:** a button that shows the state (Off, Not Set Up, Ready, Running, Blocked) and opens
  the Environment pane; Interpreter; Packages; Check Sandbox.
- **Permissions:** What Python Can Do; This Workbook.
- **Diagnostics:** Problems, every cell with something to say, grouped by kind, a click going to the
  cell; Run Log, each run's cell, time and printed output; Clear.

The status bar shows the state when you have Python on or the workbook has Python cells, and while a
cell runs, the cell and how long it has run. A click on it opens the Environment pane.

## For agents

Agents can read all of this and run cells, once a person has turned Python on: `python.environment`,
`python.diagnostics`, `python.runLog`, `python.run`, `python.runAll`, `ui.inspect`. See
[control-protocol.md](control-protocol.md).
