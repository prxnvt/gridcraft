//! Automate › Macros in the window: recording (with "Recording · 7 steps" and Stop in the status
//! bar), running a macro with its questions (approve it, make room for its output), the Macros
//! dialog (Run, Step Into, Edit, Create, Delete, Options) and macro shortcuts (Ctrl+Shift+letter,
//! ⌘⇧letter on a Mac). Everything goes through the `macro.*` commands.

use egui::{Sense, Ui, vec2};
use gridcraft_engine::model::Workbook;
use serde_json::{Value as Json, json};

use crate::dialogs::Dialog;
use crate::theme::{self, Tokens};
use crate::{Confirm, SheetApp};

/// How many macros the Run Macro menu keeps as recent.
const RECENT: usize = 8;

/// Macro state that belongs to the window, not the workbook.
#[derive(Clone, Debug)]
pub struct MacroUi {
    /// Use Relative References: record steps relative to the selection when recording started.
    pub relative: bool,
    /// Macros run from this window, most recent first (Run Macro's menu).
    pub recent: Vec<String>,
    /// The step editor, while it is open.
    pub editor: Option<crate::macro_editor::Editor>,
    /// A debugger result from a prompt's button, for the editor to take in.
    pub debug_result: Option<Json>,
}

impl Default for MacroUi {
    fn default() -> Self {
        MacroUi { relative: true, recent: vec![], editor: None, debug_result: None }
    }
}

impl MacroUi {
    fn note_recent(&mut self, name: &str) {
        self.recent.retain(|n| !n.eq_ignore_ascii_case(name));
        self.recent.insert(0, name.to_string());
        self.recent.truncate(RECENT);
    }
}

/// The macros of the active workbook: (name, description, shortcut).
pub fn macro_list(app: &SheetApp) -> Vec<(String, String, Option<String>)> {
    app.session
        .active()
        .map(|d| d.wb.macros.iter().map(|m| (m.name.clone(), m.description.clone(), m.shortcut.clone())).collect())
        .unwrap_or_default()
}

/// How many commands the recorder has captured, while recording.
pub fn recorded_steps(app: &SheetApp) -> Option<usize> {
    app.session.recorder.as_ref().map(|r| r.steps.len().saturating_add(r.dropped))
}

/// Record Macro / Stop Recording: starts recording (relative or absolute, as Use Relative
/// References says), or stops and saves what was recorded as a macro.
pub fn toggle_recording(app: &mut SheetApp) {
    if app.session.is_recording() {
        match app.run("macro.record", json!({"stop": true})) {
            Ok(v) => match v["saved"].as_str() {
                Some(name) => {
                    let steps = v["steps"].as_u64().unwrap_or(0);
                    app.macros.note_recent(name);
                    app.toast = Some((format!("Saved macro “{name}” ({steps} step{})", if steps == 1 { "" } else { "s" }), crate::now_ms()));
                }
                None => app.toast = Some(("Recording stopped".into(), crate::now_ms())),
            },
            Err(e) => app.message = Some(("Record Macro".into(), crate::clean_error(&e))),
        }
    } else {
        app.run_or_alert("macro.record", json!({"relative": app.macros.relative}));
    }
}

/// Runs macro `name` on the selection; questions (approve, make room) become a prompt.
pub fn run_macro(app: &mut SheetApp, name: &str) {
    call(app, "Run Macro", "macro.run", json!({"name": name}));
}

/// Runs a macro command. Errors are shown; a question (approve it, overwrite or shift the data in
/// the way) becomes a prompt whose buttons run it again. `Some` with the result when it ran.
pub fn call(app: &mut SheetApp, title: &str, id: &str, params: Json) -> Option<Json> {
    match app.run(id, params.clone()) {
        Ok(v) => {
            note_result(app, id, &params, &v);
            Some(v)
        }
        Err(e) => {
            if !ask(app, title, id, &params, &e) {
                app.message = Some((title.into(), crate::clean_error(&e)));
            }
            None
        }
    }
}

/// The question in an "approve it?" error, if it is one.
pub fn approve_text(e: &str) -> Option<String> {
    let rest = e.split_once("approve: ")?.1;
    Some(rest.split_once(" (run again with").map(|(q, _)| q).unwrap_or(rest).to_string())
}

/// Turns a question from command `id` into a prompt ([`SheetApp::confirm`]) whose buttons run it
/// again: Run for a macro that needs approval; Overwrite, Shift Down or Shift Right for a macro
/// whose output would land on data; OK (`overwrite: true`) for anything else. False when `e`
/// isn't a question.
pub fn ask(app: &mut SheetApp, title: &str, id: &str, params: &Json, e: &str) -> bool {
    let is_macro = matches!(id, "macro.run" | "macro.debug" | "automate.runScript");
    let (text, choices) = if let Some(q) = approve_text(e) {
        (q, vec![("Run".to_string(), json!({"approve": true}))])
    } else if let Some(q) = crate::confirm_text(e) {
        if is_macro {
            // The engine's hint for programs ("run again with onConflict …") becomes the buttons.
            let what = q.split_once(" Overwrite them").map(|(a, _)| a).unwrap_or(&q).trim_end_matches(['.', ' ']);
            (
                format!("{what}. Overwrite them, or move the data out of the way?"),
                vec![
                    ("Overwrite".to_string(), json!({"onConflict": "overwrite"})),
                    ("Shift Down".to_string(), json!({"onConflict": "shiftDown"})),
                    ("Shift Right".to_string(), json!({"onConflict": "shiftRight"})),
                ],
            )
        } else {
            (q, vec![])
        }
    } else {
        return false;
    };
    app.confirm = Some(Confirm { title: title.into(), text, command: id.into(), params: params.clone(), choices });
    true
}

/// After a macro command ran (directly or from a prompt's button): remember the macro, say what
/// it did.
pub fn note_result(app: &mut SheetApp, id: &str, params: &Json, v: &Json) {
    if id == "macro.debug" {
        app.macros.debug_result = Some(v.clone());
        return;
    }
    if id != "macro.run" {
        return;
    }
    let name = v["macro"].as_str().or(params["name"].as_str()).unwrap_or_default().to_string();
    if name.is_empty() {
        return;
    }
    app.macros.note_recent(&name);
    let ran = v["ran"].as_u64().unwrap_or(0);
    let mut text = format!("Ran macro “{name}” ({ran} step{})", if ran == 1 { "" } else { "s" });
    if let Some(msg) = v["stopped"].as_str() {
        text = format!("Macro “{name}” stopped: {msg}");
    }
    app.toast = Some((text, crate::now_ms()));
}

/// The recording indicator in the status bar: a red dot, "Recording · 7 steps" and Stop.
pub fn status(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if let Some(name) = app.session.macro_debug.as_ref().map(|d| d.name.clone()) {
        ui.add_space(12.0);
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.5, t.breakpoint);
        ui.label(egui::RichText::new(format!("Debugging “{name}”")).font(theme::ui_font(12.5)).color(t.text));
        if app.macros.editor.is_none() {
            let show = ui.add(egui::Button::new(egui::RichText::new("Show").font(theme::ui_font(12.0))).min_size(vec2(40.0, 18.0)));
            if show.on_hover_text("Open the macro in the step editor").clicked() {
                app.open_dialog("macroEditor", json!({"macro": name}));
            }
        }
    }
    let Some(n) = recorded_steps(app) else { return };
    ui.add_space(12.0);
    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 4.5, t.recording);
    let text = format!("{} · {n} step{}", app.ui.language.tr("Recording"), if n == 1 { "" } else { "s" });
    ui.label(egui::RichText::new(text).font(theme::ui_font(12.5)).color(t.text));
    let stop = ui.add(egui::Button::new(egui::RichText::new(app.ui.language.tr("Stop")).font(theme::ui_font(12.0))).min_size(vec2(40.0, 18.0)));
    if stop.on_hover_text("Stop recording and save the macro").clicked() {
        toggle_recording(app);
    }
}

// ---------------------------------------------------------------- shortcuts

/// Ctrl+Shift (⌘⇧) letters GridCraft already uses, which a macro can't take.
pub const RESERVED: &[(char, &str)] = &[('Z', "Redo"), ('S', "Save As"), ('F', "Filter"), ('T', "AutoSum")];

/// The letter of a macro shortcut: `"Ctrl+Shift+M"`, `"⌘⇧M"`, `"m"` → `'M'`. `None` when it
/// isn't one letter A to Z.
pub fn parse_shortcut(s: &str) -> Option<char> {
    let last = s.trim().rsplit(['+', '⇧', '⌘']).next()?.trim();
    let mut chars = last.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// How a shortcut is stored and shown: `Ctrl+Shift+M`.
pub fn shortcut_text(letter: char) -> String {
    format!("Ctrl+Shift+{}", letter.to_ascii_uppercase())
}

/// Why macro `name` can't use Ctrl+Shift+`letter`: GridCraft uses it, or another macro has it.
pub fn shortcut_conflict(wb: &Workbook, name: &str, letter: char) -> Option<String> {
    let letter = letter.to_ascii_uppercase();
    if let Some((_, what)) = RESERVED.iter().find(|(c, _)| *c == letter) {
        return Some(format!("{} is {what}. Choose another letter.", shortcut_text(letter)));
    }
    wb.macros
        .iter()
        .find(|m| !m.name.eq_ignore_ascii_case(name) && m.shortcut.as_deref().and_then(parse_shortcut) == Some(letter))
        .map(|m| format!("{} already runs macro “{}”. Choose another letter.", shortcut_text(letter), m.name))
}

/// The macro whose shortcut is Ctrl+Shift+`key` in the active workbook. GridCraft's own shortcuts
/// win over a macro's (a file can carry any shortcut).
pub fn shortcut_macro(app: &SheetApp, key: egui::Key) -> Option<String> {
    let letter = parse_shortcut(key.name())?;
    if RESERVED.iter().any(|(c, _)| *c == letter) {
        return None;
    }
    let wb = &app.session.active()?.wb;
    wb.macros.iter().find(|m| m.shortcut.as_deref().and_then(parse_shortcut) == Some(letter)).map(|m| m.name.clone())
}

/// Ctrl+Shift+letter on the grid: runs the macro with that shortcut. True when one ran.
pub fn run_shortcut(app: &mut SheetApp, key: egui::Key) -> bool {
    let Some(name) = shortcut_macro(app, key) else { return false };
    run_macro(app, &name);
    true
}

// ---------------------------------------------------------------- Macros dialog

/// The Macros dialog (View › Macros, Automate › Macros…).
pub fn open_dialog(app: &SheetApp, params: &Json) -> Dialog {
    let first = app.macros.recent.iter().find(|r| macro_list(app).iter().any(|(n, ..)| n.eq_ignore_ascii_case(r))).cloned();
    let name = params["name"].as_str().map(str::to_string).or(first).or_else(|| macro_list(app).first().map(|m| m.0.clone())).unwrap_or_default();
    Dialog::custom("macros", "Macro", json!({"name": name}))
}

/// The Macros dialog's body: the name box and list on the left, the buttons on the right, the
/// description under them; Options… switches to the shortcut and description of one macro.
pub fn macros_dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    if d.values.get("options").and_then(Json::as_bool) == Some(true) {
        options(app, ui, d);
        return;
    }
    let list = macro_list(app);
    let mut name = d.values.get("name").and_then(Json::as_str).unwrap_or("").to_string();
    let exists = list.iter().any(|(n, ..)| n.eq_ignore_ascii_case(name.trim()));
    let t = Tokens::get(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(300.0);
            ui.label("Macro name:");
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(300.0).id_salt("macro_name"));
            egui::Frame::NONE.fill(t.input_bg).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(2.0).show(ui, |ui| {
                ui.set_min_size(vec2(296.0, 200.0));
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    if list.is_empty() {
                        ui.label(egui::RichText::new("No macros yet. Record one, or type a name and press Create.").italics().color(t.text_dim));
                    }
                    for (n, _, shortcut) in &list {
                        let label = match shortcut {
                            Some(k) => format!("{n}    {k}"),
                            None => n.clone(),
                        };
                        let r = ui.add(egui::Button::selectable(n.eq_ignore_ascii_case(name.trim()), label).min_size(vec2(292.0, 20.0)));
                        if r.clicked() {
                            name = n.clone();
                        }
                        if r.double_clicked() {
                            run_macro(app, n);
                            *open = false;
                        }
                    }
                });
            });
        });
        ui.vertical(|ui| {
            ui.add_space(18.0);
            let w = vec2(100.0, 24.0);
            let target = name.trim().to_string();
            if ui.add_enabled(exists, egui::Button::new("Run").min_size(w)).clicked() {
                *open = false;
                run_macro(app, &target);
            }
            if ui
                .add_enabled(exists, egui::Button::new("Step Into").min_size(w))
                .on_hover_text("Open the macro and stop before its first step")
                .clicked()
            {
                *open = false;
                app.dialog_next = Dialog::open(app, "macroEditor", json!({"macro": target, "debug": true}));
            }
            if ui.add_enabled(exists, egui::Button::new("Edit").min_size(w)).clicked() {
                *open = false;
                app.dialog_next = Dialog::open(app, "macroEditor", json!({"macro": target}));
            }
            if ui.add_enabled(!exists && !target.is_empty(), egui::Button::new("Create").min_size(w)).clicked() {
                *open = false;
                app.dialog_next = Dialog::open(app, "macroEditor", json!({"new": target}));
            }
            if ui.add_enabled(exists, egui::Button::new("Delete").min_size(w)).clicked() {
                app.confirm = Some(Confirm {
                    title: "Delete Macro".into(),
                    text: format!("Do you want to delete macro “{target}”?"),
                    command: "macro.delete".into(),
                    params: json!({"name": target}),
                    choices: vec![("Delete".into(), json!({}))],
                });
            }
            if ui.add_enabled(exists, egui::Button::new("Options…").min_size(w)).clicked() {
                let (desc, key) = list
                    .iter()
                    .find(|(n, ..)| n.eq_ignore_ascii_case(&target))
                    .map(|(_, desc, k)| (desc.clone(), k.as_deref().and_then(parse_shortcut).map(String::from).unwrap_or_default()))
                    .unwrap_or_default();
                d.values.insert("options".into(), json!(true));
                d.values.insert("shortcut".into(), json!(key));
                d.values.insert("description".into(), json!(desc));
            }
            ui.add_space(12.0);
            if ui.add(egui::Button::new("Cancel").min_size(w)).clicked() {
                *open = false;
            }
        });
    });
    ui.add_space(6.0);
    ui.label("Description");
    let desc = list.iter().find(|(n, ..)| n.eq_ignore_ascii_case(name.trim())).map(|m| m.1.clone()).unwrap_or_default();
    ui.label(egui::RichText::new(if desc.is_empty() { " ".to_string() } else { desc }).color(t.text_dim));
    d.values.insert("name".into(), json!(name));
}

/// Macro Options: the shortcut letter and the description of the selected macro.
fn options(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog) {
    let name = d.values.get("name").and_then(Json::as_str).unwrap_or("").trim().to_string();
    ui.label(egui::RichText::new(format!("Macro name: {name}")).strong());
    ui.add_space(4.0);
    ui.label("Shortcut key:");
    let mut key = d.values.get("shortcut").and_then(Json::as_str).unwrap_or("").to_string();
    ui.horizontal(|ui| {
        ui.label(if ui.ctx().os() == egui::os::OperatingSystem::Mac { "⌘ ⇧ +" } else { "Ctrl+Shift+" });
        ui.add(egui::TextEdit::singleline(&mut key).char_limit(1).desired_width(24.0).id_salt("macro_shortcut"));
    });
    key = key.to_ascii_uppercase();
    let letter = parse_shortcut(&key);
    let conflict = match (key.trim().is_empty(), letter, app.session.active()) {
        (true, ..) => None,
        (false, None, _) => Some("Use a letter from A to Z.".to_string()),
        (false, Some(c), Some(doc)) => shortcut_conflict(&doc.wb, &name, c),
        (false, Some(_), None) => None,
    };
    if let Some(c) = &conflict {
        ui.label(egui::RichText::new(c).color(Tokens::get(ui.ctx()).danger));
    }
    ui.add_space(4.0);
    ui.label("Description:");
    let mut desc = d.values.get("description").and_then(Json::as_str).unwrap_or("").to_string();
    ui.add(egui::TextEdit::multiline(&mut desc).desired_rows(3).desired_width(f32::INFINITY).id_salt("macro_description"));
    ui.add_space(6.0);
    let mut done = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(" Cancel ").clicked() {
            done = true;
        }
        if ui.add_enabled(conflict.is_none(), egui::Button::new("   OK   ")).clicked() {
            done = save_options(app, &name, letter, &desc);
        }
    });
    d.values.insert("shortcut".into(), json!(key));
    d.values.insert("description".into(), json!(desc));
    if done {
        d.values.remove("options");
    }
}

/// Saves a macro's shortcut and description. True when saved (else the error is shown).
pub fn save_options(app: &mut SheetApp, name: &str, letter: Option<char>, description: &str) -> bool {
    let mut m = match app.run("macro.get", json!({"name": name})) {
        Ok(m) if m.is_object() => m,
        Ok(_) => return false,
        Err(e) => {
            app.message = Some(("Macro Options".into(), crate::clean_error(&e)));
            return false;
        }
    };
    m["shortcut"] = letter.map(|c| json!(shortcut_text(c))).unwrap_or(Json::Null);
    if let Some(o) = m.as_object_mut()
        && letter.is_none()
    {
        o.remove("shortcut");
    }
    m["description"] = json!(description);
    match app.run("macro.save", json!({"macro": m})) {
        Ok(_) => true,
        Err(e) => {
            app.message = Some(("Macro Options".into(), crate::clean_error(&e)));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse_to_one_letter() {
        assert_eq!(parse_shortcut("Ctrl+Shift+M"), Some('M'));
        assert_eq!(parse_shortcut("⌘⇧k"), Some('K'));
        assert_eq!(parse_shortcut(" q "), Some('Q'));
        assert_eq!(parse_shortcut("Ctrl+Shift+F5"), None);
        assert_eq!(parse_shortcut("Ctrl+Shift+"), None);
        assert_eq!(parse_shortcut(""), None);
        assert_eq!(parse_shortcut("Ctrl+Shift+é"), None);
        assert_eq!(shortcut_text('m'), "Ctrl+Shift+M");
    }

    #[test]
    fn a_shortcut_can_not_take_a_reserved_letter_or_another_macros() {
        let mut wb = Workbook::default();
        let mut a = gridcraft_engine::model::Macro { name: "A".into(), ..Default::default() };
        a.shortcut = Some("Ctrl+Shift+M".into());
        wb.macros.push(a);
        wb.macros.push(gridcraft_engine::model::Macro { name: "B".into(), ..Default::default() });
        assert!(shortcut_conflict(&wb, "B", 's').is_some_and(|m| m.contains("Save As")));
        assert!(shortcut_conflict(&wb, "B", 'm').is_some_and(|m| m.contains("“A”")));
        // A macro keeps its own letter.
        assert_eq!(shortcut_conflict(&wb, "a", 'M'), None);
        assert_eq!(shortcut_conflict(&wb, "B", 'K'), None);
    }

    #[test]
    fn approval_questions_are_read_from_the_error() {
        let e = "approve: Run macro “M”? It came with a workbook you opened. (run again with `approve: true`)";
        assert_eq!(approve_text(e).as_deref(), Some("Run macro “M”? It came with a workbook you opened."));
        assert_eq!(approve_text("confirm: Replace it? (run again with `overwrite: true`)"), None);
    }
}
