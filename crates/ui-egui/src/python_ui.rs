//! Python in cells, around the sheet: the Python Diagnostics pane, the Initialization and Python
//! Options dialogs, the Enable Python bar for workbooks opened from a file, and the status bar's
//! note of which Python runs cells. (Python mode in the formula bar is in
//! `formula_bar`; the cells' PY tags and preview cards are in `grid`.)

use egui::{RichText, Ui, vec2};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::code_editor::Lang;
use crate::dialogs::Dialog;
use crate::icons::Icon;
use crate::theme::{self, Tokens};

/// Translates a label for the language of the frame being drawn.
fn tr<'a>(ui: &Ui, text: &'a str) -> &'a str {
    crate::i18n::current(ui.ctx()).tr(text)
}

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
    let summary = backend_summary(app, ui.ctx());
    ui.add(egui::Label::new(RichText::new(summary).color(t.text).font(theme::ui_font(12.0))).wrap());
    ui.add_space(4.0);
    let (mut clear, mut options) = (false, false);
    ui.horizontal(|ui| {
        ui.checkbox(&mut this_sheet, tr(ui, "This sheet"));
        ui.checkbox(&mut errors_only, tr(ui, "Errors only"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clear = ui.button(tr(ui, "Clear")).clicked();
            options = ui.button(tr(ui, "Options…")).on_hover_text(tr(ui, "Python Options: which Python runs cells, and the time limit")).clicked();
        });
    });
    if options {
        app.open_dialog("pythonOptions", json!({}));
    }
    let Some(d) = app.session.active() else { return };
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
        ui.label(RichText::new(tr(ui, "No Python runs yet.")).italics().color(t.text_dim));
    }
    let mut goto: Option<(String, String)> = None;
    for r in &runs {
        egui::Frame::NONE.fill(t.menu_bg).corner_radius(6.0).inner_margin(8.0).stroke(egui::Stroke::new(1.0, t.separator)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if r.cell.is_empty() {
                    ui.label(RichText::new(tr(ui, "Initialization")).strong());
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
                ui.label(RichText::new(tr(ui, status)).color(color).font(theme::ui_bold(12.0)));
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
                    ui.label(RichText::new(tr(ui, label)).color(t.text_dim).font(theme::ui_font(11.5)));
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
    ui.label(tr(ui, "This code runs first, before any Python cell, each time Python starts. Put imports and shared definitions here."));
    ui.add_space(6.0);
    let mut code = d.values.get("code").and_then(Json::as_str).unwrap_or("").to_string();
    if crate::widgets::code_editor(ui, egui::Id::new("python.initialization.code"), &mut code, Lang::Python, 12).changed() {
        d.values.insert("code".into(), json!(code));
    }
    ui.add_space(8.0);
    // A row of its own height (the shared OK/Cancel row would fill the window's height).
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let t = Tokens::get(ui.ctx());
        if ui.add(egui::Button::new(RichText::new(format!("   {}   ", tr(ui, "OK"))).color(t.on_accent)).fill(t.accent)).clicked() {
            *confirm = true;
        }
        if ui.button(format!(" {} ", tr(ui, "Cancel"))).clicked() {
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
/// or initialization code).
fn needs_enable(app: &SheetApp, ctx: &egui::Context) -> bool {
    app.session.active().is_some_and(|d| !d.trust.allows_code()) && has_python(app, ctx)
}

/// Whether the active workbook has Python in it (cells or initialization code). Cached per
/// document revision.
fn has_python(app: &SheetApp, ctx: &egui::Context) -> bool {
    let Some(d) = app.session.active() else { return false };
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
                crate::icons::paint(ui.painter(), r, Icon::Python, t.text);
                ui.label(RichText::new(tr(ui, "Python cells are turned off for this workbook.")).font(theme::ui_bold(12.5)).color(t.text));
                let note = match cpython_path(app) {
                    Some(path) => tr(
                        ui,
                        "Python cells run with your Python at {path}, with full access to your files and network. Enable only for workbooks you trust.",
                    )
                    .replace("{path}", &path),
                    None => tr(ui, "Python runs in a sandbox with no file or network access.").to_string(),
                };
                // Room is kept for the button: a long path is cut, with the whole text on hover.
                let room = (ui.available_width() - 150.0).max(80.0);
                ui.allocate_ui(vec2(room, 24.0), |ui| {
                    ui.add(egui::Label::new(RichText::new(&note).font(theme::ui_font(12.5)).color(t.text)).truncate()).on_hover_text(&note);
                });
                let enable = ui.add(egui::Button::new(RichText::new(tr(ui, "Enable Python")).color(t.on_accent).font(theme::ui_font(12.5))).fill(t.accent));
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

/// The path of the user's Python when it runs cells (`None`: the built-in Python, or still looking).
fn cpython_path(app: &mut SheetApp) -> Option<String> {
    let b = app.session.python_backend();
    (b["kind"] == "cpython").then(|| b["path"].as_str().unwrap_or_default().to_string())
}

/// The line saying which Python runs cells; repaints while the search runs.
fn backend_summary(app: &mut SheetApp, ctx: &egui::Context) -> String {
    let b = app.session.python_backend();
    if b["kind"] == "detecting" {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    b["summary"].as_str().unwrap_or_default().to_string()
}

/// The status bar's "Py: CPython" or "Py: built-in", on workbooks with Python in them (or while
/// cells run), with the full line on hover.
pub fn status(app: &mut SheetApp, ui: &mut Ui) {
    if !app.session.python_busy() && !has_python(app, ui.ctx()) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let b = app.session.python_backend();
    let label = match b["kind"].as_str() {
        Some("cpython") => "Py: CPython",
        Some("builtin" | "custom") => "Py: built-in",
        Some("detecting") => {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            "Py: …"
        }
        _ => "Py: off",
    };
    ui.add_space(12.0);
    let summary = b["summary"].as_str().unwrap_or_default().to_string();
    let r = ui.add(egui::Label::new(RichText::new(tr(ui, label)).font(theme::ui_font(12.0)).color(t.text_dim)).sense(egui::Sense::click()));
    if r.on_hover_text(summary).clicked() {
        app.open_dialog("pythonOptions", json!({}));
    }
}

/// Opens Python Options with the current settings.
pub fn open_options(app: &mut SheetApp) -> Dialog {
    let p = &app.session.prefs;
    let mut d =
        Dialog::custom("pythonOptions", "Python Options", json!({"interpreter": p.python_interpreter, "timeoutSeconds": p.python_timeout_seconds}));
    d.command = Some("python.options");
    d
}

/// Python Options: which Python runs cells (the user's CPython or the built-in one, its version
/// and packages, and the Pythons that were passed over), the interpreter path with Browse and
/// Detect Again, and the time limit per cell.
pub fn options(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let b = app.session.python_backend();
    let kind = b["kind"].as_str().unwrap_or_default().to_string();
    let s = |k: &str| b[k].as_str().map(str::to_string);
    egui::Frame::NONE.fill(t.menu_bg).corner_radius(6.0).inner_margin(10.0).stroke(egui::Stroke::new(1.0, t.separator)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), r, Icon::Python, t.text);
            if kind == "detecting" {
                ui.add(egui::Spinner::new().size(14.0).color(t.text_dim));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            ui.add(egui::Label::new(RichText::new(s("summary").unwrap_or_default()).font(theme::ui_bold(12.5)).color(t.text)).wrap());
        });
        ui.add_space(4.0);
        if kind == "cpython" {
            egui::Grid::new("python.options.backend").num_columns(2).spacing(vec2(12.0, 3.0)).show(ui, |ui| {
                let missing = tr(ui, "not installed").to_string();
                let not = || missing.clone();
                for (label, value) in [
                    ("Version", s("version").unwrap_or_default()),
                    ("Program", s("path").unwrap_or_default()),
                    ("Found in", s("source").unwrap_or_default()),
                    ("pandas", s("pandas").unwrap_or_else(not)),
                    ("numpy", s("numpy").unwrap_or_else(not)),
                ] {
                    ui.label(RichText::new(tr(ui, label)).color(t.text_dim));
                    ui.add(egui::Label::new(RichText::new(value).color(t.text)).wrap());
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            let note = if b["externallyManaged"].as_bool().unwrap_or(false) {
                "Cells have full access to your files and network. This Python is managed by your system, so GridCraft never installs packages into it; to use your own packages, make a virtual environment (python3 -m venv) and choose its python here."
            } else {
                "Cells have full access to your files and network."
            };
            ui.add(egui::Label::new(RichText::new(note).color(t.text_dim).font(theme::ui_font(12.0))).wrap());
        } else if kind == "builtin" || kind == "custom" {
            ui.add(
                egui::Label::new(
                    RichText::new("The built-in Python runs a subset of Python, with no packages, in a sandbox with no file or network access. Cells that need more show what to install.")
                        .color(t.text_dim)
                        .font(theme::ui_font(12.0)),
                )
                .wrap(),
            );
        }
        let rejected = b["rejected"].as_array().cloned().unwrap_or_default();
        if !rejected.is_empty() {
            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new(tr(ui, "Not used ({count})").replace("{count}", &rejected.len().to_string())).color(t.text_dim)).id_salt("python.options.rejected").show(ui, |ui| {
                for r in &rejected {
                    let text = format!("{}: {}", r["path"].as_str().unwrap_or_default(), r["reason"].as_str().unwrap_or_default());
                    ui.add(egui::Label::new(RichText::new(text).color(t.text_dim).font(theme::ui_font(12.0))).wrap());
                }
            });
        }
    });
    ui.add_space(10.0);
    ui.label(RichText::new(tr(ui, "Python interpreter")).strong());
    let mut path = d.values.get("interpreter").and_then(Json::as_str).unwrap_or("").to_string();
    let mut detect = false;
    ui.horizontal(|ui| {
        let browse = app.services.pick_program.is_some();
        let width = ui.available_width() - if browse { 190.0 } else { 110.0 };
        let edit = egui::TextEdit::singleline(&mut path).hint_text(tr(ui, "Automatic")).desired_width(width.max(120.0));
        if ui.add(edit).on_hover_text("A Python 3.11 or newer program, or a name on PATH such as python3.12").changed() {
            d.values.insert("interpreter".into(), json!(path));
        }
        if browse
            && ui.button(tr(ui, "Browse…")).clicked()
            && let Some(picked) = app.services.pick_program.as_ref().and_then(|pick| pick())
        {
            path = picked;
            d.values.insert("interpreter".into(), json!(path));
        }
        detect = ui.button(tr(ui, "Detect Again")).on_hover_text(tr(ui, "Look for Python again (after installing Python or a package)")).clicked();
    });
    ui.label(
        RichText::new("Leave it empty to look for Python 3.11 or newer: an active virtual environment first, then python3 on PATH.")
            .color(t.text_dim)
            .font(theme::ui_font(12.0)),
    );
    if detect && let Err(e) = app.run("python.options", json!({"interpreter": path, "detect": true})) {
        app.message = Some((d.title.clone(), crate::clean_error(&e)));
    }
    ui.add_space(10.0);
    let mut secs = d.values.get("timeoutSeconds").and_then(Json::as_f64).unwrap_or(5.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr(ui, "Time limit per cell")).strong());
        if ui.add(egui::DragValue::new(&mut secs).range(0.1..=3_600.0).speed(0.1).max_decimals(1).suffix(" s")).changed() {
            d.values.insert("timeoutSeconds".into(), json!(secs));
        }
    });
    ui.add_space(8.0);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add(egui::Button::new(RichText::new(format!("   {}   ", tr(ui, "OK"))).color(t.on_accent)).fill(t.accent)).clicked() {
            *confirm = true;
        }
        if ui.button(format!(" {} ", tr(ui, "Cancel"))).clicked() {
            *open = false;
        }
    });
    if *confirm {
        *confirm = false;
        match app.run("python.options", json!({"interpreter": path, "timeoutSeconds": secs})) {
            Ok(_) => *open = false,
            Err(e) => app.message = Some((d.title.clone(), crate::clean_error(&e))),
        }
    }
}

/// The Python cell `cell` (A1 on the active sheet; the active cell when `None`) as a table of
/// texts with whether its first row is a header, when its result is tabular.
pub fn cell_table(app: &SheetApp, cell: Option<&str>) -> Option<(Vec<Vec<String>>, bool)> {
    let d = app.session.active()?;
    let at = match cell {
        Some(c) => gridcraft_engine::core::CellRef::parse(c)?,
        None => d.selection.active,
    };
    let py = d.wb.active()?.cell(at)?.py.clone()?;
    let (table, headers) = py.table()?;
    Some((gridcraft_engine::python::table_text(table), headers))
}

/// `python.saveCsv {cell?, path?}`: a Python cell's tabular result as CSV. With `path` it is
/// written to that file (`.csv` added when the name has no extension); without one the reply
/// carries the text as `csv`. It never opens a dialog: the preview card's Save as .csv… asks
/// for the path first (see [`save_csv_as`]).
pub fn save_csv(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let cell = p.get("cell").and_then(Json::as_str);
    let (rows, _) = cell_table(app, cell).ok_or("The cell isn't a Python cell with a table result")?;
    let csv = gridcraft_engine::python::to_csv(&rows);
    let Some(path) = p.get("path").and_then(Json::as_str) else {
        return Ok(json!({"saved": false, "csv": csv, "rows": rows.len()}));
    };
    let path = if std::path::Path::new(path).extension().is_none() { format!("{path}.csv") } else { path.to_string() };
    write_file(&path, csv.as_bytes())?;
    Ok(json!({"saved": true, "path": path, "rows": rows.len()}))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("Couldn't save {path}: {e}"))
}

#[cfg(target_arch = "wasm32")]
fn write_file(_path: &str, _bytes: &[u8]) -> Result<(), String> {
    Err("The web version has no file system; omit `path` to get the CSV text".into())
}

/// Save as .csv… in the preview card: asks the host's save dialog for a path, then runs
/// `python.saveCsv`; on the web, where there is no dialog, the CSV is downloaded.
pub fn save_csv_as(app: &mut SheetApp, cell: &str) {
    if let Some(pick) = &app.services.pick_save {
        if let Some(path) = pick("python-result.csv") {
            app.run_or_alert("python.saveCsv", json!({"cell": cell, "path": path}));
        }
        return;
    }
    match app.run("python.saveCsv", json!({"cell": cell})) {
        Ok(r) => {
            if let (Some(download), Some(csv)) = (&app.services.download, r.get("csv").and_then(Json::as_str)) {
                download("python-result.csv", csv.as_bytes());
            }
        }
        Err(e) => app.message = Some(("Save as .csv".into(), crate::clean_error(&e))),
    }
}

/// The tabular result in the preview card: a small scrollable grid with headers (a numbered
/// header when the table has none). Only the cells shown are turned into text, so a large
/// result costs no more than a small one.
pub fn table_view(ui: &mut Ui, table: &gridcraft_engine::core::Array, headers: bool) {
    const MAX_ROWS: usize = 200;
    const MAX_COLS: usize = 30;
    const MAX_CHARS: usize = 40;
    let lang = crate::i18n::current(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let cols = table.cols.min(MAX_COLS);
    let first = usize::from(headers && table.rows > 0);
    let body = table.rows.saturating_sub(first);
    let text = |r: usize, c: usize| {
        let s = table.get(r, c).map(gridcraft_engine::core::Value::display).unwrap_or_default();
        if s.chars().count() > MAX_CHARS { format!("{}…", s.chars().take(MAX_CHARS).collect::<String>()) } else { s }
    };
    egui::ScrollArea::both().id_salt("py_card.table").max_height(220.0).max_width(520.0).auto_shrink([true, true]).show(ui, |ui| {
        egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.separator)).show(ui, |ui| {
            egui::Grid::new("py_card.table.grid").striped(true).spacing(vec2(12.0, 3.0)).show(ui, |ui| {
                for c in 0..cols {
                    let head = if first == 1 { text(0, c) } else { c.to_string() };
                    ui.label(RichText::new(head).font(theme::ui_bold(12.0)).color(t.text));
                }
                ui.end_row();
                for r in (first..table.rows).take(MAX_ROWS) {
                    for c in 0..cols {
                        ui.label(RichText::new(text(r, c)).font(theme::ui_font(12.0)).color(t.text));
                    }
                    ui.end_row();
                }
            });
        });
    });
    if body > MAX_ROWS || table.cols > MAX_COLS {
        let text = lang
            .tr("Showing {shown} of {rows} rows and {cols} of {all} columns; Copy and Save include them all.")
            .replace("{shown}", &body.min(MAX_ROWS).to_string())
            .replace("{rows}", &body.to_string())
            .replace("{cols}", &cols.to_string())
            .replace("{all}", &table.cols.to_string());
        ui.label(RichText::new(text).color(t.text_dim).font(theme::ui_font(11.5)));
    }
}
