//! Macros (Automate › Macros): typed command lists that run as one undo step. See
//! [`crate::macros`] for the model, tokens, runner, debugger and policy. The older
//! `automate.*` script commands are aliases of these.

use serde_json::{Value as Json, json};

use super::*;
use crate::macros::recorder::RecordingStart;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "macro.list", "Macros", [], None, "{} → [{name, description, shortcut?, steps, inputs: [names], onConflict, permissions}]", has_doc, list),
        cmd!(query "macro.get", "Get Macro", [], None, "{name} → the macro: {v, name, description, shortcut?, inputs, steps, output, permissions}", has_doc, get),
        cmd!(
            "macro.save",
            "Save Macro",
            [],
            None,
            "{macro: {name, description?, shortcut?, inputs?: [{name, kind: cell|range, rows: [min, max], cols: [min, max], header, colTypes: [number|text|date|bool|any], nonEmpty, defaultRef?}], steps: [{type: command, id, params, enabled?, breakpoint?, note?} | {type: repeat, over, of: rows|columns, max, body} | {type: if, cell, op: eq|ne|gt|ge|lt|le|empty|notEmpty|contains, value, then, else} | {type: stop, message, rollback?}], output?: {anchor?, onConflict: ask|fail|overwrite|shiftDown|shiftRight}, permissions?: [command ids]} (or those fields at the top level), replace?: bool (default true)} → {name, steps}",
            has_doc,
            save
        ),
        cmd!("macro.delete", "Delete Macro", [], None, "{name}", has_doc, delete),
        cmd!("macro.rename", "Rename Macro", [], None, "{name, to}", has_doc, rename),
        cmd!(
            "macro.run",
            "Run Macro",
            ["Automate", "Macros"],
            None,
            "{name, inputs?: {Input: \"B2:D9\"} | \"B2:D9\", onConflict?: ask|fail|overwrite|shiftDown|shiftRight, approve?: bool, timeLimit?: seconds} runs as one undo step; any error rolls it all back → {macro, ran, inputs, stopped?, shifted?}",
            has_doc,
            run
        ),
        cmd!(
            query "macro.check",
            "Check Macro",
            [],
            None,
            "{name, inputs?} validates the inputs and dry-runs the macro on the workbook (then puts everything back) → {ok, errors?, needsApproval?, inputs, output?, conflicts, footprint}",
            has_doc,
            check
        ),
        cmd!(
            noundo "macro.record",
            "Record Macro",
            ["Automate", "Macros"],
            None,
            "{stop?: bool, relative?: bool (default true; when starting), name?, description? (when stopping)} toggles recording; the selection when it starts is the example input → {recording, relative?, input?} or {recording: false, saved, steps, dropped, skipped, inputs}",
            has_doc,
            record
        ),
        cmd!(
            "macro.debug",
            "Step Into",
            ["Automate", "Macros"],
            None,
            "{action: start|step|continue|runTo|stop|state, name? and inputs?, onConflict?, approve? (start), step: path (runTo), keep?: bool (stop), breakpoints?: [\"3\", \"2.1\"], breakpointsOnly?: bool (the list replaces the steps' own breakpoints)} debugs a macro one step at a time as one undo step; edits of the workbooks it touched wait until it finishes or stops (other workbooks stay editable) → {state: paused|finished|stopped|failed, current: {step, kind, command?, params?, iteration?}, lastParams, executed, changed, atBreakpoint?, error?}",
            has_doc,
            |s, p| crate::macros::debug::debug(s, p)
        ),
        cmd!(query "view.macros", "Macros", ["View", "Macros"], None, "{} opens the Macros list → the macros (as macro.list)", has_doc, view_macros),
        // Scripts from earlier versions: the same macros under their old names.
        cmd!(
            noundo "automate.recordActions",
            "Record Actions",
            ["Automate", "Scripting Tools"],
            None,
            "{stop?: bool, name?: script name when stopping} records top-level commands only, with absolute references; stopping saves them as a macro → {saved, steps, dropped} (alias of macro.record)",
            has_doc,
            record_actions
        ),
        cmd!(
            "automate.newScript",
            "New Script",
            ["Automate", "Scripting Tools"],
            None,
            "{name, commands: [{command, params}] } (saves a macro)",
            has_doc,
            new_script
        ),
        cmd!(query "automate.allScripts", "All Scripts", ["Automate", "Scripting Tools"], None, "{} → [{name, steps}] (alias of macro.list)", has_doc, all_scripts),
        cmd!("automate.runScript", "Run Script", [], None, "{name} (alias of macro.run)", has_doc, run),
        cmd!("automate.deleteScript", "Delete Script", [], None, "{name} (alias of macro.delete)", has_doc, delete),
    ]
}

fn name_param<'a>(cmd: &str, p: &'a Json) -> Result<&'a str> {
    str_param(p, "name").filter(|n| !n.trim().is_empty()).ok_or_else(|| bad(cmd, "missing `name`"))
}

/// The index of macro `name` in the active workbook, or an error that says there is none.
fn index_of(s: &Session, name: &str) -> Result<usize> {
    crate::macros::find(&s.doc()?.wb, name).ok_or_else(|| EngineError::Other(format!("There is no macro called “{name}”.")))
}

fn list(s: &mut Session, _: &Json) -> Result<Json> {
    let wb = &s.doc()?.wb;
    Ok(Json::Array(
        wb.macros
            .iter()
            .map(|m| {
                let mut o = json!({
                    "name": m.name,
                    "description": m.description,
                    "steps": m.step_count(),
                    "inputs": m.inputs.iter().map(|i| i.name.clone()).collect::<Vec<_>>(),
                    "onConflict": m.output.on_conflict.name(),
                    "permissions": m.permissions,
                });
                if let Some(k) = &m.shortcut {
                    o["shortcut"] = json!(k);
                }
                o
            })
            .collect(),
    ))
}

fn get(s: &mut Session, p: &Json) -> Result<Json> {
    let i = index_of(s, name_param("macro.get", p)?)?;
    let wb = &s.doc()?.wb;
    Ok(wb.macros.get(i).map(crate::macros::macro_json).unwrap_or(Json::Null))
}

fn save(s: &mut Session, p: &Json) -> Result<Json> {
    let v = p.get("macro").filter(|m| m.is_object()).unwrap_or(p);
    let m = crate::macros::parse_macro(v).map_err(|e| bad("macro.save", e))?;
    let replace = bool_param(p, "replace").unwrap_or(true);
    let (name, steps) = (m.name.clone(), m.step_count());
    edit(s, |cx| {
        let at = crate::macros::find(&cx.wb, &m.name);
        match at {
            Some(_) if !replace => return Err(EngineError::Other(format!("A macro called “{}” already exists.", m.name))),
            Some(i) => {
                if let Some(old) = cx.wb.macros.get_mut(i) {
                    *old = m;
                }
            }
            None if cx.wb.macros.len() >= gridcraft_model::MAX_MACROS => {
                return Err(EngineError::Other(format!("A workbook can hold at most {} macros.", gridcraft_model::MAX_MACROS)));
            }
            None => cx.wb.macros.push(m),
        }
        Ok(())
    })?;
    Ok(json!({"name": name, "steps": steps}))
}

fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    let i = index_of(s, name_param("macro.delete", p)?)?;
    edit(s, |cx| {
        if i < cx.wb.macros.len() {
            cx.wb.macros.remove(i);
        }
        Ok(Json::Null)
    })
}

fn rename(s: &mut Session, p: &Json) -> Result<Json> {
    let i = index_of(s, name_param("macro.rename", p)?)?;
    let to = str_param(p, "to").map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad("macro.rename", "missing `to`"))?.to_string();
    if crate::macros::find(&s.doc()?.wb, &to).is_some_and(|j| j != i) {
        return Err(EngineError::Other(format!("A macro called “{to}” already exists.")));
    }
    edit(s, |cx| {
        let Some(m) = cx.wb.macros.get_mut(i) else { return Ok(()) };
        let mut renamed = m.clone();
        renamed.name = to.clone();
        renamed.check_limits().map_err(EngineError::Other)?;
        *m = renamed;
        Ok(())
    })?;
    Ok(json!({"name": to}))
}

fn run(s: &mut Session, p: &Json) -> Result<Json> {
    let name = name_param("macro.run", p)?;
    crate::macros::run::run(s, name, p)
}

fn check(s: &mut Session, p: &Json) -> Result<Json> {
    let name = name_param("macro.check", p)?;
    crate::macros::run::check(s, name, p)
}

fn view_macros(s: &mut Session, p: &Json) -> Result<Json> {
    s.ui_requests.push(crate::UiRequest::Dialog("macros".into(), json!({})));
    list(s, p)
}

fn record(s: &mut Session, p: &Json) -> Result<Json> {
    if bool_param(p, "stop").unwrap_or(false) || s.is_recording() {
        return stop_recording(s, p, false);
    }
    start_recording(s, bool_param(p, "relative").unwrap_or(true))
}

fn start_recording(s: &mut Session, relative: bool) -> Result<Json> {
    let d = s.doc()?;
    let sheet = d.wb.active_sheet;
    let start = RecordingStart {
        doc: d.uid,
        sheet,
        sheet_name: d.wb.active().map(|sh| sh.name.clone()).unwrap_or_default(),
        selection: d.selection.clone(),
        relative,
    };
    let input = start.selection.current().a1();
    s.start_recording();
    s.macro_recording = Some(start);
    Ok(if relative { json!({"recording": true, "relative": true, "input": input}) } else { json!({"recording": true}) })
}

/// Stops recording and saves what was recorded as a macro. `legacy`: the old script recorder
/// (absolute references, "Script N" names, overwriting as scripts did).
fn stop_recording(s: &mut Session, p: &Json, legacy: bool) -> Result<Json> {
    let Some(rec) = s.stop_recording() else { return Ok(json!({"recording": false})) };
    let start = s.macro_recording.take();
    let d = s.doc()?;
    let start = start.unwrap_or_else(|| RecordingStart {
        doc: d.uid,
        sheet: d.wb.active_sheet,
        sheet_name: d.wb.active().map(|sh| sh.name.clone()).unwrap_or_default(),
        selection: d.selection.clone(),
        relative: !legacy,
    });
    let wb = d.wb.clone();
    let name = match str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None if legacy => (1..).map(|n| format!("Script {n}")).find(|n| crate::macros::find(&wb, n).is_none()).unwrap_or_else(|| "Script".into()),
        None => crate::macros::next_name(&wb),
    };
    let mut converted = crate::macros::recorder::convert(&name, &start, &rec.steps, &wb);
    converted.mac.steps.truncate(gridcraft_model::MAX_MACRO_STEPS);
    if let Some(desc) = str_param(p, "description") {
        converted.mac.description = desc.chars().take(gridcraft_model::MAX_MACRO_TEXT).collect();
    }
    if legacy {
        converted.mac.output.on_conflict = gridcraft_model::OnConflict::Overwrite;
    }
    let steps = converted.mac.steps.len();
    let inputs: Vec<String> = converted.mac.inputs.iter().map(|i| i.name.clone()).collect();
    s.execute("macro.save", json!({"macro": crate::macros::macro_json(&converted.mac)}))?;
    Ok(json!({"recording": false, "saved": name, "steps": steps, "dropped": rec.dropped, "skipped": converted.skipped, "inputs": inputs}))
}

fn record_actions(s: &mut Session, p: &Json) -> Result<Json> {
    if bool_param(p, "stop").unwrap_or(false) || s.is_recording() {
        return stop_recording(s, p, true);
    }
    start_recording(s, false)
}

fn new_script(s: &mut Session, p: &Json) -> Result<Json> {
    let name = name_param("automate.newScript", p)?.to_string();
    let cmds = p.get("commands").cloned().unwrap_or(json!([]));
    if !cmds.is_array() {
        return Err(bad("automate.newScript", "`commands` must be an array"));
    }
    let m = crate::macros::legacy_script(&name, &cmds).map_err(|e| bad("automate.newScript", e))?;
    save(s, &json!({"macro": crate::macros::macro_json(&m)}))?;
    Ok(json!({"name": name}))
}

fn all_scripts(s: &mut Session, _: &Json) -> Result<Json> {
    let wb = &s.doc()?.wb;
    Ok(Json::Array(wb.macros.iter().map(|m| json!({"name": m.name, "steps": m.step_count()})).collect()))
}
