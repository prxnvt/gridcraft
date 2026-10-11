# Macros

GridCraft macros are typed, declarative lists of commands, not VBA. Every step runs a GridCraft
command (the same ids the ribbon, CLI and MCP use) or one of three bounded control steps. A macro
declares the inputs it works on, runs as one undo step, rolls everything back on an error, checks
where its results will land before it writes them, and never touches files without permission.
Agents run macros with `macro.run` like any other command.

## A macro

```json
{
  "name": "Totals",
  "description": "Adds a total under the amounts",
  "inputs": [{"name": "Input", "header": true, "cols": [2, 2], "colTypes": ["text", "number"]}],
  "output": {"anchor": "Input.right(2).cell(1,2)", "onConflict": "ask"},
  "steps": [
    {"type": "command", "id": "cell.set", "params": {"cell": "Output", "input": "Total"}},
    {"type": "command", "id": "cell.set", "params": {"cell": "Output.offset(1,0)", "input": "=SUM({{Input.data.col(2)}})"}},
    {"type": "repeat", "over": "Input.data", "of": "rows", "max": 1000, "body": [
      {"type": "if", "cell": "Row.cell(1,2)", "op": "gt", "value": 100,
       "then": [{"type": "command", "id": "home.bold", "params": {"range": "Row"}}]}
    ]},
    {"type": "stop", "message": "Done."}
  ],
  "permissions": []
}
```

- **Steps**: `command` (`id`, `params`), `repeat` (each row or column of `over`, at most `max`
  times), `if` (`cell` compared with `value` by `eq`, `ne`, `gt`, `ge`, `lt`, `le`, `empty`,
  `notEmpty` or `contains`; `then` and `else` lists) and `stop` (`message`; `rollback: true`
  undoes the run). Every step can carry `enabled`, `breakpoint` and `note`. The short form
  `{"command": "cell.set", "params": {…}}` is accepted for a command step.
- **Inputs**: `kind` (`cell` or `range`), `rows` and `cols` as `[min, max]` (0 for no maximum;
  rows exclude the header), `header`, `colTypes` (`number`, `text`, `date`, `bool`, `any`),
  `nonEmpty` and `defaultRef`. A run binds each input to the range it is given, its default, or
  (for the first input) the selection, and refuses with messages that name the cells:
  "Input column 2 (Amount) must hold numbers; C4 holds text."
- **Limits**: 1,000 macros a workbook, 10,000 steps a macro, Repeat and If nested 4 deep,
  100,000 steps executed per run, and 60 seconds a run (`timeLimit` to change).

## Tokens

References in step parameters can be tokens, worked out when the step runs:

| Token | Means |
|---|---|
| `Input`, or another input's name | the range the input is bound to |
| `Output` | the output rule's anchor |
| `Row`, `Column` | the current row or column of the Repeat around the step |
| `Name:Total` | a defined name, or a table's data |

Methods narrow or move a range and chain: `col(2)` or `col("Amount")`, `row(1)`, `cols(2..3)`,
`rows(2..)`, `cell(r,c)` (relative to the top-left cell; `cell(0,1)` is the cell above),
`offset(r,c)`, `resize(h,w)`, `data` (without the header row), `below(n)` and `right(n)`.

Write a token as the whole value of a reference parameter (`cell`, `range`, `target`, keys ending
in `Cell` or `Range`…), as `{"$ref": "Input.col(2)"}` anywhere, or as `{{Input.col(2)}}` inside
any string, such as a formula. Plain A1 references stay absolute; a `Name:` anchor follows inserted
rows.

## Running

`macro.run {name, inputs?, onConflict?, approve?, timeLimit?}`:

1. **Policy.** File, app, window, job and macro commands, undo and redo, `python.enable`,
   `python.initialization` and `control.press` never run in a macro. Reading or writing a file
   (`data.getData`, `data.fromTextCsv` or `insert.picture` with a `path`, or an export or print
   with a `path`) needs the command in the macro's `permissions`. A macro that touches files, or
   came with a workbook someone else made, runs once the person approves it: the call fails with
   `approve: …` and runs with `approve: true`. Approval lasts for the session, until the macro
   changes.
2. **Footprint.** The macro first runs in dry-run mode on the real workbook, and everything is put
   back. Cells it would write outside its inputs that hold data, sheets and objects it would
   delete, are conflicts, handled by `onConflict` (or the macro's `output.onConflict`): `ask` (the
   call fails with `confirm: …`; `overwrite: true` answers it), `fail`, `overwrite`, `shiftDown` or
   `shiftRight` (Insert Cells makes room, then the footprint is checked again).
3. **Run.** One undo step, "Run Macro: Totals". Any error rolls the whole run back and names the
   step: `Step 3.1 (cell.set): …` (paths are 1-based; `2.1` is inside a Repeat, `4.then.1` inside
   an If).

`macro.check {name, inputs?}` reports the bound inputs, the output anchor, any approval needed and
the footprint, without changing anything.

## Debugging

`macro.debug {action, …}` with `start` (`name`, `inputs`, `breakpoints`), `step`, `continue`,
`runTo` (`step`: a path), `stop` (`keep: true` keeps the changes) and `state`. The run stays one
undo step; while it is open, other edits are refused with "Finish or stop the macro first". Each
answer gives the next step with its parameters as they would run, the parameters of the step that
just ran, and the cells changed so far.

## Recording

`macro.record` toggles recording (`relative: false` for absolute references). The selection when
it starts is the example input: references in and near it become tokens, and the input's shape,
header and column types are inferred. Toggles such as Bold are recorded with the state they set
(`"on": true`), so playing them back sets them rather than flipping them. Recorded file commands are
left out; a macro run while recording is replaced by its steps when it has no inputs.

## Other commands and storage

`macro.list`, `macro.get`, `macro.save {macro, replace?}`, `macro.delete`, `macro.rename {name,
to}` and `view.macros` (the Macros list). The older `automate.recordActions`, `newScript`,
`allScripts`, `runScript` and `deleteScript` are aliases.

Macros are saved in the workbook: as a field in GridCraft's JSON format, and in xlsx as hidden
defined names (`_gc_macro_<n>_<k>`, JSON split into string literals), which other spreadsheet apps
keep but ignore. Scripts from earlier versions become macros when the file is opened.
