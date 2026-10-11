//! Python in cells, around the sheet: the Python Diagnostics pane, the Initialization dialog and
//! the Enable Python bar for workbooks opened from a file. (Python mode in the formula bar is in
//! `formula_bar`; the cells' PY tags and preview cards are in `grid`.)

use egui::{RichText, Ui, vec2};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::code_editor::Lang;
use crate::dialogs::Dialog;
use crate::icons::Icon;
use crate::theme::{self, Tokens};

/// The pane's name in `GridState::pane`.
pub const PANE: &str = "pythonDiagnostics";

/// The Diagnostics pane: one row per Python run, newest first, with its cell, status, duration,
/// printed output and error. "This sheet" and "Errors only" filter it; clicking a row's cell
/// selects that cell.
pub fn diagnostics(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(d) = app.session.active() else { return };
    let sheet = d.wb.active().map(|s| s.name.clone()).unwrap_or_default();
    let key = egui::Id::new("python.diagnostics.filters");
    let (mut this_sheet, mut errors_only): (bool, bool) = ui.ctx().data(|m| m.get_temp(key)).unwrap_or((false, false));
    ui.label(RichText::new("Python runs in a sandbox with no file or network access.").color(t.text_dim).font(theme::ui_font(12.0)));
    ui.add_space(4.0);
    let mut clear = false;
    ui.horizontal(|ui| {
        ui.checkbox(&mut this_sheet, "This sheet");
        ui.checkbox(&mut errors_only, "Errors only");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clear = ui.button("Clear").clicked();
        });
    });
    ui.ctx().data_mut(|m| m.insert_temp(key, (this_sheet, errors_only)));
    ui.separator();
    let runs: Vec<gridcraft_engine::python::PyRunLog> = d
        .py_log
        .iter()
        .rev()
        .filter(|r| !this_sheet || r.sheet == sheet || r.cell.is_empty())
        .filter(|r| !errors_only || r.status != "ok")
        .cloned()
        .collect();
    if clear {
        app.run_or_alert("python.clearDiagnostics", json!({}));
        return;
    }
    if runs.is_empty() {
        ui.label(RichText::new("No Python runs yet.").italics().color(t.text_dim));
    }
    let mut goto: Option<(String, String)> = None;
    for r in &runs {
        egui::Frame::NONE.fill(t.menu_bg).corner_radius(6.0).inner_margin(8.0).stroke(egui::Stroke::new(1.0, t.separator)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if r.cell.is_empty() {
                    ui.label(RichText::new("Initialization").strong());
                } else if ui.link(RichText::new(format!("{}!{}", gridcraft_engine::formula::quote_sheet(&r.sheet), r.cell)).strong()).clicked() {
                    goto = Some((r.sheet.clone(), r.cell.clone()));
                }
                let (status, color) = match r.status {
                    "ok" => ("OK", t.text_dim),
                    "error" => ("Error", t.danger),
                    "timeout" => ("Time limit", t.danger),
                    "memory" => ("Memory limit", t.danger),
                    "stopped" => ("Stopped", t.danger),
                    "blocked" => ("Turned off", t.text_dim),
                    "cycle" => ("Cycle", t.danger),
                    other => (other, t.text_dim),
                };
                ui.label(RichText::new(status).color(color).font(theme::ui_bold(12.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{:.1} ms", r.ms)).color(t.text_dim).font(theme::ui_font(12.0)));
                });
            });
            if let Some(e) = &r.error {
                let text = match r.line {
                    Some(l) => format!("Line {l}: {e}"),
                    None => e.clone(),
                };
                ui.add(egui::Label::new(RichText::new(text).color(t.danger)).wrap());
            }
            for (label, out) in [("Output", &r.stdout), ("Errors", &r.stderr)] {
                if !out.is_empty() {
                    ui.label(RichText::new(label).color(t.text_dim).font(theme::ui_font(11.5)));
                    ui.add(egui::Label::new(RichText::new(out.trim_end()).font(crate::code_editor::font()).color(t.text)).wrap());
                }
            }
        });
        ui.add_space(4.0);
    }
    if let Some((sheet, cell)) = goto {
        let other = app.session.active().and_then(|d| d.wb.active()).is_some_and(|s| s.name != sheet);
        if other {
            app.run_or_alert("sheet.activate", json!({"sheet": sheet}));
        }
        app.run_or_alert("selection.set", json!({"cell": cell}));
        app.grid.ensure_visible = true;
    }
}

/// Opens the Initialization dialog with the workbook's initialization code.
pub fn open_initialization(app: &mut SheetApp) -> Option<Dialog> {
    let code = app.session.run("python.initialization", json!({})).ok()?.get("code").and_then(Json::as_str).unwrap_or("").to_string();
    Some(Dialog::custom("pythonInitialization", "Python Initialization", json!({"code": code})))
}

/// The Initialization dialog: the code editor over the workbook's initialization code, which runs
/// before any Python cell. OK sets it (every Python cell then runs again).
pub fn initialization(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    ui.label("This code runs first, before any Python cell, each time Python starts. Put imports and shared definitions here.");
    ui.add_space(6.0);
    let mut code = d.values.get("code").and_then(Json::as_str).unwrap_or("").to_string();
    if crate::widgets::code_editor(ui, egui::Id::new("python.initialization.code"), &mut code, Lang::Python, 12).changed() {
        d.values.insert("code".into(), json!(code));
    }
    ui.add_space(8.0);
    // A row of its own height (the shared OK/Cancel row would fill the window's height).
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let t = Tokens::get(ui.ctx());
        if ui.add(egui::Button::new(RichText::new("   OK   ").color(t.on_accent)).fill(t.accent)).clicked() {
            *confirm = true;
        }
        if ui.button(" Cancel ").clicked() {
            *open = false;
        }
    });
    if *confirm {
        *confirm = false;
        match app.run("python.initialization", json!({"code": code})) {
            Ok(_) => *open = false,
            Err(e) => app.message = Some((d.title.clone(), crate::clean_error(&e))),
        }
    }
}

/// Whether the active workbook came from a file, is not yet trusted, and has Python in it (cells
/// or initialization code). Cached per document revision.
fn needs_enable(app: &SheetApp, ctx: &egui::Context) -> bool {
    let Some(d) = app.session.active() else { return false };
    if d.trust.allows_code() {
        return false;
    }
    let key = egui::Id::new("python.enable_bar.has_python");
    if let Some((uid, rev, has)) = ctx.data(|m| m.get_temp::<(u64, u64, bool)>(key))
        && uid == d.uid
        && rev == d.revision
    {
        return has;
    }
    let has = d.wb.sheets.iter().any(|s| s.cells.iter().any(|(_, c)| c.py.is_some()))
        || gridcraft_engine::cmd::stash::get(&d.wb, gridcraft_engine::python::INIT_KEY).is_some_and(|c| !c.is_empty());
    ctx.data_mut(|m| m.insert_temp(key, (d.uid, d.revision, has)));
    has
}

/// The message bar under the formula bar for a workbook from a file whose Python is turned off.
pub fn enable_bar(app: &mut SheetApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if !needs_enable(app, &ctx) {
        return;
    }
    let Some(uid) = app.session.active().map(|d| d.uid) else { return };
    let dismissed = egui::Id::new("python.enable_bar.dismissed").with(uid);
    if ctx.data(|m| m.get_temp::<bool>(dismissed)).unwrap_or(false) {
        return;
    }
    let t = Tokens::get(&ctx);
    egui::Panel::top("python_enable_bar")
        .exact_size(34.0)
        .frame(egui::Frame::NONE.fill(t.ribbon).stroke(egui::Stroke::new(1.0, t.separator)).inner_margin(egui::Margin::symmetric(12, 4)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
                crate::icons::paint_accent(ui.painter(), r, Icon::Python, t.text, Some(t.cat(theme::Cat::Green)));
                ui.label(RichText::new("Python cells are turned off for this workbook.").font(theme::ui_bold(12.5)).color(t.text));
                ui.label(RichText::new("Python runs in a sandbox with no file or network access.").font(theme::ui_font(12.5)).color(t.text));
                let enable = ui.add(egui::Button::new(RichText::new("Enable Python").color(t.on_accent).font(theme::ui_font(12.5))).fill(t.accent));
                if enable.clicked() {
                    app.run_or_alert("python.enable", json!({}));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::widgets::icon_button(ui, Icon::Close, t.text_dim, "Dismiss this message", vec2(20.0, 20.0)).clicked() {
                        ctx.data_mut(|m| m.insert_temp(dismissed, true));
                    }
                });
            });
        });
}
