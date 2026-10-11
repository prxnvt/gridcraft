//! Macros: typed, declarative command lists (not VBA). The model is plain data in
//! `gridcraft_model::Macro`; this module reads and writes it, and keeps it within its limits.
//!
//! Storage: the native JSON format keeps `Workbook::macros` as a field. In xlsx each macro is one
//! item of GridCraft's hidden data (`cmd::stash`), `_gc_macro_<n>_<k>`, written when the file is
//! saved and taken back out of the names when it is opened. Scripts saved by earlier versions (the
//! `scripts` item, `_sc_scripts` or `_gc_scripts_<k>`) become macros when the file is opened.

use gridcraft_model::{Macro, OnConflict, Step, StepKind, Workbook};
use serde_json::{Value as Json, json};

use crate::cmd::stash;

/// The hidden-data key prefix of a stored macro (`macro_<n>`).
const MACRO_KEY: &str = "macro_";
/// The hidden-data key of scripts saved before macros existed.
const LEGACY_SCRIPTS_KEY: &str = "scripts";

/// Reads a macro from JSON, as `macro.save` and files give it. Steps may use the short form of
/// earlier scripts, `{"command": "cell.set", "params": {…}}`, for a command step.
pub fn parse_macro(v: &Json) -> Result<Macro, String> {
    let mut v = v.clone();
    if let Some(steps) = v.get_mut("steps") {
        normalize_steps(steps, 1)?;
    }
    if v.get("v").is_none()
        && let Some(o) = v.as_object_mut()
    {
        o.insert("v".into(), json!(gridcraft_model::MACRO_VERSION));
    }
    let mut m: Macro = serde_json::from_value(v).map_err(|e| format!("This isn't a valid macro: {e}"))?;
    m.name = m.name.trim().to_string();
    m.check_limits()?;
    if m.name.chars().any(char::is_control) {
        return Err("A macro name can't hold line breaks or other control characters.".into());
    }
    Ok(m)
}

/// Rewrites the short step form into the tagged one, in place, refusing nesting past the limit.
fn normalize_steps(steps: &mut Json, level: usize) -> Result<(), String> {
    let Some(list) = steps.as_array_mut() else { return Err("`steps` must be a list.".into()) };
    if level > gridcraft_model::MAX_MACRO_DEPTH {
        return Err(format!("Repeat and If steps can nest at most {} deep.", gridcraft_model::MAX_MACRO_DEPTH));
    }
    for st in list {
        let Some(o) = st.as_object_mut() else { return Err("Every step must be an object.".into()) };
        if !o.contains_key("type")
            && let Some(id) = o.remove("command")
        {
            o.insert("type".into(), json!("command"));
            o.insert("id".into(), id);
        }
        for key in ["body", "then", "else"] {
            if let Some(body) = o.get_mut(key) {
                normalize_steps(body, level + 1)?;
            }
        }
    }
    Ok(())
}

/// A macro as JSON (the shape `macro.get` returns and `macro.save` takes).
pub fn macro_json(m: &Macro) -> Json {
    serde_json::to_value(m).unwrap_or(Json::Null)
}

/// The index of the macro called `name` (case-insensitive).
pub fn find(wb: &Workbook, name: &str) -> Option<usize> {
    let name = name.trim();
    wb.macros.iter().position(|m| m.name.eq_ignore_ascii_case(name))
}

/// A stable fingerprint of a macro's content: approval to run a macro from a file holds until
/// the macro changes.
pub fn hash(m: &Macro) -> u64 {
    // FNV-1a over the canonical JSON (object keys are sorted).
    let text = macro_json(m).to_string();
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// A copy of `wb` for xlsx with each macro in hidden names (`_gc_macro_<n>_<k>`) instead of the
/// field, or `None` when there is nothing to change.
pub fn with_names(wb: &Workbook) -> crate::Result<Option<Workbook>> {
    let stale = !stash::keys(wb, MACRO_KEY).is_empty();
    if wb.macros.is_empty() && !stale {
        return Ok(None);
    }
    let mut out = wb.clone();
    for key in stash::keys(&out, MACRO_KEY) {
        stash::remove(&mut out, &key);
    }
    for (i, m) in wb.macros.iter().enumerate() {
        stash::set(&mut out, &format!("{MACRO_KEY}{i}"), &macro_json(m).to_string())?;
    }
    out.macros.clear();
    Ok(Some(out))
}

/// After a file is opened: moves macros out of hidden names into `Workbook::macros`, turns old
/// scripts into macros, and drops macros over the limits. Returns warnings for what was dropped.
pub fn load(wb: &mut Workbook) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut bad = 0usize;
    // Stored macros, in their saved order.
    let mut keyed: Vec<(u64, String)> =
        stash::keys(wb, MACRO_KEY).into_iter().filter_map(|k| k.strip_prefix(MACRO_KEY).and_then(|n| n.parse().ok()).map(|n| (n, k))).collect();
    keyed.sort();
    let mut found = Vec::new();
    for (_, key) in &keyed {
        match stash::get_json(wb, key).ok_or_else(|| "unreadable".to_string()).and_then(|v| parse_macro(&v)) {
            Ok(m) => found.push(m),
            Err(_) => bad += 1,
        }
    }
    for key in stash::keys(wb, MACRO_KEY) {
        stash::remove(wb, &key);
    }
    // Scripts from earlier versions.
    if let Some(scripts) = stash::get_json(wb, LEGACY_SCRIPTS_KEY) {
        if let Some(o) = scripts.as_object() {
            for (name, steps) in o {
                match legacy_script(name, steps) {
                    Ok(m) => found.push(m),
                    Err(_) => bad += 1,
                }
            }
        }
        stash::remove(wb, LEGACY_SCRIPTS_KEY);
    }
    // Macros that came as a field (the JSON format) go through the same checks.
    let mut all = std::mem::take(&mut wb.macros);
    all.extend(found);
    for m in all {
        if wb.macros.len() >= gridcraft_model::MAX_MACROS {
            bad += 1;
            continue;
        }
        match m.check_limits() {
            Ok(()) => {
                let mut m = m;
                m.name = unique_name(wb, m.name.trim());
                wb.macros.push(m);
            }
            Err(_) => bad += 1,
        }
    }
    if bad > 0 {
        warnings.push(format!(
            "{bad} macro{} in the workbook couldn't be opened: {} damaged or over the limits ({} macros, {} steps each, Repeat and If nested {} deep).",
            if bad == 1 { "" } else { "s" },
            if bad == 1 { "it is" } else { "they are" },
            gridcraft_model::MAX_MACROS,
            gridcraft_model::MAX_MACRO_STEPS,
            gridcraft_model::MAX_MACRO_DEPTH
        ));
    }
    warnings
}

/// A script saved before macros existed (`[{command, params}]`) as a macro. Scripts had no inputs
/// and wrote wherever their steps said, so the macro overwrites without asking, as they did.
pub fn legacy_script(name: &str, steps: &Json) -> Result<Macro, String> {
    let list = steps.as_array().ok_or("a script is a list of steps")?;
    let steps: Vec<Step> = list
        .iter()
        .take(gridcraft_model::MAX_MACRO_STEPS)
        .filter_map(|st| {
            let id = st.get("command").and_then(Json::as_str)?;
            // Scripts never ran other scripts.
            (!id.starts_with("automate.")).then(|| Step::command(id, st.get("params").cloned().unwrap_or(json!({}))))
        })
        .collect();
    let mut m = Macro { name: name.trim().to_string(), steps, ..Macro::default() };
    m.output.on_conflict = OnConflict::Overwrite;
    m.check_limits()?;
    Ok(m)
}

/// `name`, or `name (2)`, `name (3)`… if a macro already has it.
pub fn unique_name(wb: &Workbook, name: &str) -> String {
    let name = if name.is_empty() { "Macro" } else { name };
    if find(wb, name).is_none() {
        return name.to_string();
    }
    (2..).map(|n| format!("{name} ({n})")).find(|n| find(wb, n).is_none()).unwrap_or_else(|| name.to_string())
}

/// The first free `Macro1`, `Macro2`… name.
pub fn next_name(wb: &Workbook) -> String {
    (1..).map(|n| format!("Macro{n}")).find(|n| find(wb, n).is_none()).unwrap_or_else(|| "Macro".into())
}

/// Every command step of `steps` (inside Repeat and If too), with its path.
pub fn command_steps(steps: &[Step]) -> Vec<(String, &str, &Json)> {
    fn walk<'a>(steps: &'a [Step], prefix: &str, out: &mut Vec<(String, &'a str, &'a Json)>) {
        for (i, st) in steps.iter().enumerate() {
            let path = if prefix.is_empty() { (i + 1).to_string() } else { format!("{prefix}.{}", i + 1) };
            match &st.kind {
                StepKind::Command { id, params } => out.push((path, id.as_str(), params)),
                StepKind::Repeat { body, .. } => walk(body, &path, out),
                StepKind::If { then, otherwise, .. } => {
                    walk(then, &format!("{path}.then"), out);
                    walk(otherwise, &format!("{path}.else"), out);
                }
                StepKind::Stop { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(steps, "", &mut out);
    out
}

#[cfg(test)]
mod tests;
