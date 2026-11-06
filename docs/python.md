# Python in cells

> **Last updated:** 2026-11-05 · **Change:** minor (dates and times, in and out of Python; earlier: what tells you what is wrong: PY tags, the card, the code checks; earlier: the Python tab is contextual; earlier: first version: how to turn Python on, how to set it up with uv, what cells can and can't do, and the limits)

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

## Dates and times

A number in a cell is a date because of its format, and Python sees it that way. `xl("A1")` gives a
`datetime.date`, `datetime.time` or `datetime.datetime`, by itself or in a range. With pandas, a
column of dates and empty cells is a datetime column, with `NaT` for the empty cells. Read a date, add
a day and return it, and the cell shows the next day:

```python
import datetime
xl("A1") + datetime.timedelta(days=1)
```

**What `xl()` gives,** in the workbook's date system (1900 or 1904), to the millisecond:

- a date format gives a `date`, or a `datetime` when the cell holds a time of day too, so nothing is
  lost;
- a date and time format gives a `datetime`;
- a time format gives a `time`, or a `datetime` when the number is 1 or more;
- a format for elapsed time, such as `[h]:mm`, gives the number of days as a number.

A number with no place on the calendar stays a number: one that is negative, one past 31 December
9999, and in the 1900 system the days that never were. Those are serial 60, 29 February 1900, which
that system has for compatibility and Python's calendar doesn't, and anything below 1 under a format
that shows a date. So 28 February 1900 plus a day, in Python, is 1 March 1900, serial 61.

**What comes back.** `date`, `datetime`, `time`, numpy's `datetime64` and pandas's `Timestamp` become
serial numbers in the workbook's date system, in one cell or across a spill, a DataFrame's date column
included. `timedelta`, numpy's `timedelta64` and pandas's `Timedelta` become a number of days. `NaT`
is an empty cell. A value with a time zone uses its local wall-clock time, as Excel has no time zones.

A cell with no format of its own takes one that matches:

- `m/d/yyyy` for a date, and for a `Timestamp` or `datetime64` at midnight, since those types have no
  date without a time;
- `m/d/yyyy h:mm` for a `datetime`, and for every date in a column where some have times;
- `h:mm:ss` for a time;
- `[h]:mm:ss` for a length of time. A negative one stays a plain number of days, as no format shows
  one.

The format goes with the date: when a later run gives no date there, or a spill is blocked, the cell
goes back to General. A format you chose stays, and so does General when you set it on a date a run
gave. Writing a format makes no undo step, like writing the result. In a saved file a format is simply
the cell's format, so after opening it again a run no longer takes it back.

A date a serial can't hold, before 1900 (1904 in a 1904 workbook) or after 9999, gives `#NUM!`, as
does a length of time longer than the whole calendar.

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

## When something is wrong

**The PY tag** at the start of a Python cell shows its state, in its shape as well as its colour:

- filled, the plain tag: the result is up to date;
- filled, with an open ring: it is running or waiting its turn;
- hollow: it keeps a value it didn't just work out, because what it reads changed, it was stopped, or
  Python isn't on;
- red, with an exclamation mark: it failed;
- grey, with a lock: Python can't run here.

**The card** opens from the tag, or with Ctrl+Shift+F5. It says what happened in one sentence, then
Python's own message with its line, and has one button for what to do: Open Settings, Choose
Interpreter, Copy Install Command, Enable, Run Again, Reset or Go To Cell. A failed run that printed
something has Show Printed Output. A result keeps its type and preview. Copy Install Command gives
the uv command; pip is behind **Other package managers**.

**In the editors**, the formula bar in Python mode and the Python editor window check your code as
you type, with Python off too and on systems with no sandbox. They need nothing to run:

- a wavy underline on what the sandbox will refuse (a file, the network, another program, a plot, an
  `xl()` whose reference is computed) and on a package this Python doesn't have, with the reason when
  you hover over it;
- a chip with Python's state, which opens the Environment pane;
- in the editor window, a mark beside each such line, and a strip under the code that lists these
  and what the last run said. A click on a row goes to its line. The last run's line is marked only
  while the code is what ran.

A package is only called missing once GridCraft knows what this Python has.

**Problems** on the Python tab also lists, for the whole workbook, each package the code imports that
this Python doesn't have, with its install command, and says when pandas is missing, which makes
ranges come back as lists of rows.

## For agents

Agents can read all of this and run cells, once a person has turned Python on: `python.environment`,
`python.diagnostics`, `python.runLog`, `python.run`, `python.runAll`, `ui.inspect`. See
[control-protocol.md](control-protocol.md).
