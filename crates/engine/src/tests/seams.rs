//! Engine seams for macros and Python: undo groups, the recording buffer, document trust, load
//! caps and chunked hidden-name storage.

use serde_json::json;

use super::{s, v};
use crate::{EngineError, Session, Trust};

fn undo_labels(s: &Session) -> Vec<String> {
    s.doc().unwrap().undo.iter().map(|e| e.label.clone()).collect()
}

fn set(s: &mut Session, cell: &str, input: &str) {
    s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
}

// ---------------------------------------------------------------- undo groups

#[test]
fn grouped_commands_are_one_undo_step() {
    let mut s = s();
    set(&mut s, "A1", "before");
    let r = s.grouped("Fill Three", |s| {
        set(s, "A1", "1");
        set(s, "A2", "2");
        s.execute("home.bold", json!({"range": "A1:A2"}))?;
        Ok(7)
    });
    assert_eq!(r.unwrap(), 7);
    assert_eq!(undo_labels(&s), ["Enter Cell", "Fill Three"]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(v(&s, "A1").display(), "before");
    assert!(v(&s, "A2").is_empty());
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(v(&s, "A2").display(), "2");
    assert!(!s.in_group(s.doc().unwrap().uid));
}

#[test]
fn a_failing_group_rolls_everything_back() {
    let mut s = s();
    set(&mut s, "B1", "base");
    set(&mut s, "A1", "undone");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("selection.set", json!({"range": "C3"})).unwrap();
    let redo_before = s.doc().unwrap().redo.len();
    assert_eq!(redo_before, 1);
    let r: crate::Result<()> = s.grouped("Doomed", |s| {
        set(s, "B1", "changed");
        s.execute("selection.set", json!({"range": "D4"}))?;
        set(s, "B2", "new");
        Err(EngineError::Other("step 3 failed".into()))
    });
    assert!(r.unwrap_err().to_string().contains("step 3 failed"));
    assert_eq!(v(&s, "B1").display(), "base");
    assert!(v(&s, "B2").is_empty());
    assert_eq!(s.doc().unwrap().selection.a1(), "C3");
    assert_eq!(undo_labels(&s), ["Enter Cell"]);
    assert_eq!(s.doc().unwrap().redo.len(), redo_before, "the redo stack comes back too");
    // The recalculation follows the restored workbook.
    set(&mut s, "C1", "=B1&\"!\"");
    assert_eq!(v(&s, "C1").display(), "base!");
}

#[test]
fn groups_nest_and_the_outermost_makes_the_step() {
    let mut s = s();
    let r = s.grouped("Outer", |s| {
        set(s, "A1", "1");
        s.grouped("Inner", |s| {
            set(s, "A2", "2");
            Ok(())
        })?;
        // A failing inner group rolls back only its own changes; the outer one carries on.
        let inner: crate::Result<()> = s.grouped("Inner fails", |s| {
            set(s, "A3", "3");
            Err(EngineError::Other("no".into()))
        });
        assert!(inner.is_err());
        assert!(v(s, "A3").is_empty());
        set(s, "A4", "4");
        Ok(())
    });
    r.unwrap();
    assert_eq!(undo_labels(&s), ["Outer"]);
    assert_eq!(v(&s, "A2").display(), "2");
    s.execute("edit.undo", json!({})).unwrap();
    for c in ["A1", "A2", "A3", "A4"] {
        assert!(v(&s, c).is_empty(), "{c}");
    }
}

#[test]
fn an_unchanged_group_adds_no_step() {
    let mut s = s();
    set(&mut s, "A1", "x");
    s.grouped("Look Around", |s| {
        s.execute("selection.set", json!({"range": "B2"}))?;
        s.execute("cell.get", json!({"cell": "A1"}))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(undo_labels(&s), ["Enter Cell"]);
}

#[test]
fn a_group_spans_calls_and_blocks_undo_until_it_ends() {
    let mut s = s();
    set(&mut s, "A1", "start");
    let token = s.group_begin("Debug Macro").unwrap();
    // Frame 1, frame 2, …: each step is its own call.
    set(&mut s, "A1", "step 1");
    set(&mut s, "A2", "step 2");
    let undo = s.execute("edit.undo", json!({}));
    assert!(matches!(undo, Err(EngineError::Disabled(..))), "{undo:?}");
    assert_eq!(s.execute("history.list", json!({})).unwrap()["grouping"], json!(true));
    assert!(s.group_end(token).unwrap());
    assert_eq!(undo_labels(&s), ["Enter Cell", "Debug Macro"]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(v(&s, "A1").display(), "start");
    assert!(v(&s, "A2").is_empty());
}

#[test]
fn rolling_back_a_spanning_group_restores_the_start() {
    let mut s = s();
    set(&mut s, "A1", "start");
    let token = s.group_begin("Debug Macro").unwrap();
    set(&mut s, "A1", "step 1");
    s.execute("home.insertSheet", json!({})).unwrap();
    s.group_rollback(token).unwrap();
    assert_eq!(v(&s, "A1").display(), "start");
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 1);
    assert_eq!(undo_labels(&s), ["Enter Cell"]);
}

#[test]
fn ending_an_outer_group_ends_the_inner_ones() {
    let mut s = s();
    let outer = s.group_begin("Outer").unwrap();
    let inner = s.group_begin("Inner").unwrap();
    set(&mut s, "A1", "1");
    let inner_id = format!("{inner:?}");
    s.group_end(outer).unwrap();
    assert_eq!(undo_labels(&s), ["Outer"]);
    assert!(s.group_end(inner).is_err(), "{inner_id} already ended with its outer group");
    assert!(!s.in_group(s.doc().unwrap().uid));
}

#[test]
fn a_long_group_keeps_the_history_before_it() {
    let mut s = s();
    for i in 0..100 {
        set(&mut s, "A1", &i.to_string());
    }
    let first = s.doc().unwrap().undo.first().map(|e| e.wb.clone()).unwrap();
    s.grouped("Many Steps", |s| {
        for i in 0..250 {
            set(s, "B1", &i.to_string());
        }
        Ok(())
    })
    .unwrap();
    let d = s.doc().unwrap();
    assert_eq!(d.undo.len(), 100, "the undo limit still holds");
    assert_eq!(d.undo.last().map(|e| e.label.as_str()), Some("Many Steps"));
    // Only the oldest entry made room for the group's one step.
    assert!(!d.undo.iter().any(|e| std::sync::Arc::ptr_eq(&e.wb, &first)));
    assert_eq!(d.undo.iter().filter(|e| e.label == "Enter Cell").count(), 99);
}

#[test]
fn closing_a_document_drops_its_groups() {
    let mut s = s();
    let token = s.group_begin("Debug Macro").unwrap();
    let uid = token.doc();
    s.close_document(s.active_index());
    assert!(!s.in_group(uid));
    assert!(s.group_end(token).is_err());
    assert!(matches!(s.group_begin("x"), Err(EngineError::NoDocument)));
}

#[test]
fn a_script_run_is_one_undo_step_and_rolls_back_on_error() {
    let mut s = s();
    set(&mut s, "A1", "before");
    let steps = json!([
        {"command": "cell.set", "params": {"cell": "A1", "input": "1"}},
        {"command": "cell.set", "params": {"cell": "A2", "input": "2"}},
    ]);
    s.execute("automate.newScript", json!({"name": "Two", "commands": steps})).unwrap();
    let n = undo_labels(&s).len();
    s.execute("automate.runScript", json!({"name": "Two"})).unwrap();
    assert_eq!(undo_labels(&s).len(), n + 1);
    assert_eq!(undo_labels(&s).last().map(String::as_str), Some("Run Macro: Two"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(v(&s, "A1").display(), "before");
    assert!(v(&s, "A2").is_empty());

    let bad = json!([
        {"command": "cell.set", "params": {"cell": "A1", "input": "changed"}},
        {"command": "no.such.command", "params": {}},
    ]);
    s.execute("automate.newScript", json!({"name": "Bad", "commands": bad})).unwrap();
    let n = undo_labels(&s).len();
    let err = s.execute("automate.runScript", json!({"name": "Bad"})).unwrap_err().to_string();
    assert!(err.contains("Step 2 (no.such.command)"), "{err}");
    assert_eq!(v(&s, "A1").display(), "before", "the first step was rolled back");
    assert_eq!(undo_labels(&s).len(), n);
}

// ---------------------------------------------------------------- recording buffer

fn recorded_ids(s: &Session) -> Vec<String> {
    s.recorder.as_ref().unwrap().steps.iter().map(|st| st.id.clone()).collect()
}

#[test]
fn the_recorder_keeps_top_level_calls_with_where_they_ran() {
    let mut s = s();
    s.execute("home.insertSheet", json!({})).unwrap();
    s.execute("selection.set", json!({"range": "B2:C3"})).unwrap();
    s.start_recording();
    assert_eq!(s.depth(), 0);
    set(&mut s, "B2", "10");
    s.execute("cell.get", json!({"cell": "B2"})).unwrap(); // a query: not recorded
    assert!(s.execute("sheet.move", json!({})).is_err()); // failed: not recorded
    s.execute("view.customViews", json!({"save": "Mine"})).unwrap();
    // Show runs sheet.activate, selection.set and view.zoom inside: one step.
    s.execute("view.customViews", json!({"show": "Mine"})).unwrap();
    assert_eq!(recorded_ids(&s), ["cell.set", "view.customViews", "view.customViews"]);
    let first = &s.recorder.as_ref().unwrap().steps[0];
    let d = s.doc().unwrap();
    assert_eq!((first.doc, first.sheet), (d.uid, d.wb.active_sheet));
    assert_eq!(first.sheet_name, d.wb.active().unwrap().name);
    assert_eq!(first.selection.a1(), "B2:C3", "the selection before the command ran");
    assert_eq!(first.params, json!({"cell": "B2", "input": "10"}));
    assert_eq!(s.depth(), 0);
    let rec = s.stop_recording().unwrap();
    assert_eq!(rec.dropped, 0);
    set(&mut s, "A1", "after");
    assert!(s.recorder.is_none());
}

#[test]
fn the_recorder_survives_journal_trimming() {
    let mut s = s();
    s.start_recording();
    for i in 0..10_050u32 {
        s.execute("selection.set", json!({"range": format!("A{}", i % 50 + 1)})).unwrap();
    }
    set(&mut s, "C1", "last");
    assert!(s.journal.len() < 10_051, "the journal was trimmed");
    let steps = &s.recorder.as_ref().unwrap().steps;
    assert_eq!(steps.len(), 10_051);
    assert_eq!(steps[0].params, json!({"range": "A1"}));
    assert_eq!(steps[10_050].id, "cell.set");
}

#[test]
fn the_recorder_has_a_cap() {
    let mut rec = crate::record::Recorder::default();
    let step = crate::record::RecordedStep {
        doc: 1,
        sheet: 0,
        sheet_name: "Sheet1".into(),
        selection: crate::Selection::default(),
        id: "cell.set".into(),
        params: json!({}),
    };
    for _ in 0..crate::record::RECORD_LIMIT + 3 {
        rec.push(step.clone());
    }
    assert_eq!(rec.steps.len(), crate::record::RECORD_LIMIT);
    assert_eq!(rec.dropped, 3);
}

#[test]
fn record_actions_saves_top_level_steps_without_duplicates() {
    let mut s = s();
    let steps = json!([{"command": "cell.set", "params": {"cell": "B1", "input": "from script"}}]);
    s.execute("automate.newScript", json!({"name": "Inner", "commands": steps})).unwrap();
    assert_eq!(s.execute("automate.recordActions", json!({})).unwrap(), json!({"recording": true}));
    assert!(s.is_recording());
    set(&mut s, "A1", "typed");
    s.execute("automate.runScript", json!({"name": "Inner"})).unwrap();
    s.execute("home.bold", json!({"range": "A1"})).unwrap();
    let r = s.execute("automate.recordActions", json!({"stop": true})).unwrap();
    assert!(!s.is_recording());
    assert_eq!(r["saved"], "Script 1");
    assert_eq!(r["steps"], 3, "{r}");
    let saved: Vec<String> = {
        let all = s.execute("automate.allScripts", json!({})).unwrap();
        all.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect()
    };
    assert!(saved.contains(&"Script 1".to_string()));
    // Playing it back on a new sheet does the same thing.
    s.execute("home.insertSheet", json!({})).unwrap();
    assert!(v(&s, "A1").is_empty());
    s.execute("automate.runScript", json!({"name": "Script 1"})).unwrap();
    assert_eq!(v(&s, "A1").display(), "typed");
    assert_eq!(v(&s, "B1").display(), "from script");
    let d = s.doc().unwrap();
    assert!(d.wb.styles.get(d.wb.active().unwrap().style_id(gridcraft_core::CellRef::parse("A1").unwrap())).font.bold);
}

// ---------------------------------------------------------------- trust

fn open_json(s: &mut Session, wb: &gridcraft_model::Workbook) {
    let bytes = serde_json::to_vec(wb).unwrap();
    s.execute("file.open", json!({"name": "theirs.json", "base64": crate::io::base64_encode(&bytes)})).unwrap();
}

fn trust(s: &Session) -> Trust {
    s.doc().unwrap().trust
}

#[test]
fn documents_know_where_they_came_from() {
    let mut s = s();
    assert_eq!(trust(&s), Trust::Local);
    assert!(Trust::Local.allows_code() && Trust::Trusted.allows_code() && !Trust::FromFile.allows_code());
    s.execute("file.new", json!({"sample": "sales"})).unwrap();
    assert_eq!(trust(&s), Trust::Local);
    open_json(&mut s, &gridcraft_model::Workbook::new());
    assert_eq!(trust(&s), Trust::FromFile);
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["trust"], "fromFile");
    // A second window on the same workbook is the same workbook.
    s.execute("view.newWindow", json!({})).unwrap();
    assert_eq!(trust(&s), Trust::FromFile);
    // Moving and copying sheets inside it changes nothing.
    s.execute("sheet.move", json!({"to": 0, "copy": true})).unwrap();
    assert_eq!(trust(&s), Trust::FromFile);
}

#[test]
fn pasting_from_a_file_carries_its_trust() {
    let mut s = s(); // doc 0: Local
    open_json(&mut s, &gridcraft_model::Workbook::new()); // doc 1: FromFile
    let theirs = s.active_index();
    set(&mut s, "A1", "=1+1");
    s.execute("edit.copy", json!({"range": "A1"})).unwrap();
    s.new_workbook();
    let mine = s.active_index();
    // Values carry nothing that runs.
    s.execute("edit.pasteSpecial", json!({"what": "values", "at": "B1"})).unwrap();
    assert_eq!(trust(&s), Trust::Local);
    s.execute("edit.paste", json!({"at": "C1"})).unwrap();
    assert_eq!(trust(&s), Trust::FromFile);
    assert_eq!(v(&s, "C1").display(), "2");

    // Content from a local or trusted workbook leaves the destination as it was.
    s.set_active(theirs);
    s.doc_mut().unwrap().trust = Trust::Trusted;
    set(&mut s, "A2", "=3");
    s.execute("edit.copy", json!({"range": "A2"})).unwrap();
    s.execute("edit.paste", json!({"at": "A3"})).unwrap();
    assert_eq!(trust(&s), Trust::Trusted, "pasting within a workbook");
    s.set_active(mine);
    s.execute("edit.paste", json!({"at": "D1"})).unwrap();
    assert_eq!(trust(&s), Trust::FromFile, "still untrusted from the earlier paste");
    s.new_workbook();
    s.execute("edit.paste", json!({"at": "D1"})).unwrap();
    assert_eq!(trust(&s), Trust::Local);
    assert_eq!(Trust::Trusted.receive(Trust::FromFile), Trust::FromFile);
    assert_eq!(Trust::Trusted.receive(Trust::Local), Trust::Trusted);
}

// ---------------------------------------------------------------- load caps

fn formula_cell(text: &str) -> gridcraft_model::Cell {
    gridcraft_model::Cell {
        value: gridcraft_core::Value::text("cached"),
        formula: Some(std::sync::Arc::new(gridcraft_model::Formula::new(text))),
        style: Default::default(),
        py: None,
    }
}

fn name(n: &str, formula: String) -> gridcraft_model::DefinedName {
    gridcraft_model::DefinedName { name: n.into(), scope: None, formula, comment: String::new(), hidden: true }
}

/// A workbook with one of everything over the limits, next to things just under them.
fn hostile_workbook() -> gridcraft_model::Workbook {
    let mut wb = gridcraft_model::Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    let cell = |a: &str| gridcraft_core::CellRef::parse(a).unwrap();
    // Long but shallow (a long `1+1+…` chain is deep, and that is another matter).
    sh.set_cell(cell("A1"), formula_cell(&format!("LEN(\"{}\")", "a".repeat(8_190)))); // 8,197 characters
    sh.set_cell(cell("A2"), formula_cell(&format!("LEN(\"{}\")", "a".repeat(8_000)))); // 8,007: fine
    sh.set_cell(cell("B1"), formula_cell(&format!("{}1{}", "(".repeat(65), ")".repeat(65))));
    sh.set_cell(cell("B2"), formula_cell(&format!("{}1{}", "(".repeat(64), ")".repeat(64))));
    // Parentheses inside text and quoted sheet names don't count.
    sh.set_cell(cell("B3"), formula_cell(&format!("LEN(\"{}\")", "(".repeat(100))));
    let chunks = |n: usize| vec![format!("\"{}\"", "x".repeat(250)); n.div_ceil(250)].join("&");
    sh.set_cell(cell("C1"), formula_cell(&format!("PY({}&\"y\",0)", chunks(7_000))));
    sh.set_cell(cell("C2"), formula_cell(&format!("PY({},0)", chunks(7_000))));
    wb.names.push(name("LongName", format!("\"{}\"", "z".repeat(9_000))));
    wb.names.push(name("ShortName", "1".into()));
    wb.names.push(name("_gc_blob_0", format!("\"{}\"", "b".repeat(6_000))));
    wb.names.push(name("_gc_blob_1", format!("\"{}\"", "b".repeat(6_000))));
    wb.names.push(name("_gc_small_0", "\"ok\"".into()));
    wb.names.push(name("_sc_scripts", format!("\"{}\"", "s".repeat(20_000))));
    for i in 0..5 {
        wb.names.push(name(&format!("_gc_macro_{i}_0"), "\"{}\"".into()));
    }
    wb
}

#[test]
fn load_limits_drop_what_is_over_and_keep_the_rest() {
    let mut wb = hostile_workbook();
    let limits = crate::io::LoadLimits { stash_len: 11_000, macros: 3, ..crate::io::LOAD_LIMITS };
    let warnings = crate::io::enforce_limits(&mut wb, &limits);
    let sh = wb.sheet(0).unwrap();
    let has_formula = |a: &str| sh.cell(gridcraft_core::CellRef::parse(a).unwrap()).unwrap().formula.is_some();
    for (a, kept) in [("A1", false), ("A2", true), ("B1", false), ("B2", true), ("B3", true), ("C1", false), ("C2", true)] {
        assert_eq!(has_formula(a), kept, "{a}");
    }
    assert_eq!(sh.value(gridcraft_core::CellRef::parse("A1").unwrap()).display(), "cached", "the cached value stays");
    let names: Vec<&str> = wb.names.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["ShortName", "_gc_small_0", "_gc_macro_0_0", "_gc_macro_1_0", "_gc_macro_2_0"]);
    let all = warnings.join("\n");
    for needle in
        ["longer than 8,192 characters, so Sheet1!A1 kept", "more than 64 levels", "Python cell", "LongName", "blob, scripts", "3 macros; 2 were"]
    {
        assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
    }
}

#[test]
fn load_limits_apply_to_every_format() {
    let wb = hostile_workbook();
    let json = serde_json::to_vec(&wb).unwrap();
    let xlsx = gridcraft_xlsx::write_xlsx(&wb).unwrap();
    for (file, bytes) in [("hostile.json", json), ("hostile.xlsx", xlsx)] {
        let (wb, warnings) = crate::io::open_bytes(file, &bytes).unwrap();
        let sh = wb.sheet(0).unwrap();
        let formula = |a: &str| sh.cell(gridcraft_core::CellRef::parse(a).unwrap()).and_then(|c| c.formula.clone());
        assert!(formula("A1").is_none() && formula("B1").is_none() && formula("C1").is_none(), "{file}");
        assert!(formula("A2").is_some(), "{file}");
        // A PY() formula within the limits is a Python cell; the one over them is only a value.
        let py = |a: &str| sh.cell(gridcraft_core::CellRef::parse(a).unwrap()).and_then(|c| c.py.clone());
        assert_eq!(py("C2").map(|p| p.code.chars().count()), Some(7_000), "{file}");
        assert!(py("C1").is_none(), "{file}");
        assert!(!wb.names.iter().any(|n| n.name == "LongName"), "{file}");
        assert!(warnings.len() >= 4, "{file}: {warnings:?}");
    }
    // Text formats go through the same checks; they only ever hold values.
    let csv = format!("\"=LEN(\"\"{}\"\")\"\n=2\n", "a".repeat(8_190));
    let (wb, warnings) = crate::io::open_bytes("hostile.csv", csv.as_bytes()).unwrap();
    assert!(wb.sheet(0).unwrap().cells.iter().all(|(_, c)| c.formula.is_none()));
    assert!(warnings.is_empty(), "{warnings:?}");
}

// ---------------------------------------------------------------- chunked hidden names

#[test]
fn workbooks_saved_with_the_old_names_still_work() {
    let mut wb = gridcraft_model::Workbook::new();
    let script = r#"{"Old":[{"command":"cell.set","params":{"cell":"A1","input":"from an old file"}}]}"#;
    wb.names.push(name("_sc_scripts", format!("\"{}\"", script.replace('"', "\"\""))));
    wb.names.push(name("_sc_watch", "\"[\"\"Sheet1!A1\"\"]\"".into()));
    let scenarios = r#"[{"name":"Old","sheet":"Sheet1","changing":"B1","values":[5]}]"#;
    wb.names.push(name("_sc_scenarios", format!("\"{}\"", scenarios.replace('"', "\"\""))));
    let mut s = s();
    open_json(&mut s, &wb);
    assert_eq!(s.execute("automate.allScripts", json!({})).unwrap(), json!([{"name": "Old", "steps": 1}]));
    // A script from a file runs once the person approves it.
    let asked = s.execute("automate.runScript", json!({"name": "Old"}));
    assert!(matches!(asked, Err(EngineError::Approve(_))), "{asked:?}");
    s.execute("automate.runScript", json!({"name": "Old", "approve": true})).unwrap();
    assert_eq!(v(&s, "A1").display(), "from an old file");
    assert_eq!(s.execute("formulas.watchWindow", json!({})).unwrap()[0]["cell"], "Sheet1!A1");
    assert_eq!(s.execute("data.scenarioManager", json!({})).unwrap()["scenarios"][0]["name"], "Old");
    // Scripts became macros when the file opened; the next write of another item moves it to
    // the new names.
    let names: Vec<String> = s.doc().unwrap().wb.names.iter().map(|n| n.name.clone()).collect();
    assert!(!names.iter().any(|n| n.ends_with("scripts") || n.contains("scripts_")), "{names:?}");
    s.execute("automate.newScript", json!({"name": "New", "commands": []})).unwrap();
    assert_eq!(s.execute("automate.allScripts", json!({})).unwrap().as_array().unwrap().len(), 2);
    s.execute("formulas.watchWindow", json!({"add": "Sheet1!B2"})).unwrap();
    let names: Vec<String> = s.doc().unwrap().wb.names.iter().map(|n| n.name.clone()).collect();
    assert!(names.contains(&"_gc_watch_0".to_string()) && !names.contains(&"_sc_watch".to_string()), "{names:?}");
}

#[test]
fn a_large_script_round_trips_through_xlsx() {
    let mut s = s();
    let steps: Vec<serde_json::Value> =
        (1..=400).map(|r| json!({"command": "cell.set", "params": {"cell": format!("A{r}"), "input": format!("row \"{r}\"")}})).collect();
    s.execute("automate.newScript", json!({"name": "Big", "commands": steps})).unwrap();
    let b64 = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap()["base64"].as_str().unwrap().to_string();
    let (raw, _) = gridcraft_xlsx::read_xlsx(&crate::io::base64_decode(&b64).unwrap()).unwrap();
    let names = raw.names.iter().filter(|n| n.name.starts_with("_gc_macro_0_")).count();
    assert!(names >= 3, "more than 8,192 characters needs several names, got {names}");
    s.execute("file.open", json!({"name": "big.xlsx", "base64": b64})).unwrap();
    assert_eq!(s.execute("automate.allScripts", json!({})).unwrap(), json!([{"name": "Big", "steps": 400}]));
    s.execute("automate.runScript", json!({"name": "Big", "approve": true})).unwrap();
    assert_eq!(v(&s, "A400").display(), "row \"400\"");
}
