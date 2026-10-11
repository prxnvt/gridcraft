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

/// The pane's name in `GridState::pane`.
pub const PANE: &str = "pythonDiagnostics";

/// The Diagnostics pane: one row per Python run, newest first, with its cell, status, duration,
/// printed output and error. "This sheet" and "Errors only" filter it; clicking a row's cell
/// selects that cell.
pub fn diagnostics(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(d) = app.session.active() else { return };
    let sheet = d.wb.active().map(|s| s.name.clone()).unwrap_or_default();
    let key = app.widget_id("python.diagnostics.filters");
    let (mut this_sheet, mut errors_only): (bool, bool) = ui.ctx().data(|m| m.get_temp(key)).unwrap_or((false, false));
    let summary = backend_summary(app, ui.ctx());
    ui.add(egui::Label::new(RichText::new(summary).color(t.text).font(theme::ui_font(12.0))).wrap());
    // The workbook's packages, each found (with its version) or missing.
    let packages = app.session.python_declared_status();
    let mut install_missing: Vec<String> = Vec::new();
    if !packages.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new("Workbook packages:").color(t.text_dim).font(theme::ui_font(12.0)));
            for p in &packages {
                let name = p["name"].as_str().unwrap_or_default();
                let (text, color) = match p["version"].as_str() {
                    Some(v) => (format!("{name} {v}"), t.text),
                    None => {
                        install_missing.push(p["requirement"].as_str().unwrap_or(name).to_string());
                        (format!("{name} (missing)"), t.danger)
                    }
                };
                ui.label(RichText::new(text).color(color).font(theme::ui_font(12.0)));
            }
        });
        if !install_missing.is_empty()
            && !app.session.python_installing()
            && ui
                .button(format!("Install {}…", install_missing.join(", ")))
                .on_hover_text("Install the missing packages, after you approve")
                .clicked()
        {
            app.open_dialog("pythonInstall", json!({"packages": install_missing}));
        }
    }
    ui.add_space(4.0);
    let (mut clear, mut options) = (false, false);
    ui.horizontal(|ui| {
        ui.checkbox(&mut this_sheet, "This sheet");
        ui.checkbox(&mut errors_only, "Errors only");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clear = ui.button("Clear").clicked();
            options = ui.button("Options…").on_hover_text("Python Options: which Python runs cells, and the time limit").clicked();
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
        ui.label(RichText::new("No Python runs yet.").italics().color(t.text_dim));
    } else {
        // The rows say how long ago they ran.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(5));
    }
    let mut goto: Option<(String, String)> = None;
    let mut offer: Option<Json> = None;
    for r in &runs {
        egui::Frame::NONE.fill(t.menu_bg).corner_radius(6.0).inner_margin(8.0).stroke(egui::Stroke::new(1.0, t.separator)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let install = r.cell.is_empty() && gridcraft_engine::python::is_install_status(r.status);
            ui.horizontal(|ui| {
                if install {
                    ui.label(RichText::new("Package install").strong());
                } else if r.cell.is_empty() {
                    ui.label(RichText::new("Initialization").strong());
                } else if ui.link(RichText::new(format!("{}!{}", gridcraft_engine::formula::quote_sheet(&r.sheet), r.cell)).strong()).clicked() {
                    goto = Some((r.sheet.clone(), r.cell.clone()));
                }
                let (status, color) = status_label(r.status, &t);
                ui.label(RichText::new(status).color(color).font(theme::ui_bold(12.0)));
                if r.count > 1 {
                    ui.label(RichText::new(format!("\u{d7}{}", r.count)).color(t.text_dim).font(theme::ui_bold(12.0)))
                        .on_hover_text(format!("{} runs in a row ended this way; the time and output are the latest", r.count));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let age = ago(gridcraft_engine::clock::now_ms() - r.time_ms);
                    let took = if install { format!("{:.1} s", r.ms / 1000.0) } else { format!("{:.1} ms", r.ms) };
                    ui.label(RichText::new(format!("{took}, {age}")).color(t.text_dim).font(theme::ui_font(12.0)));
                });
            });
            if install {
                install_row(ui, &t, r, &runs);
                return;
            }
            if let Some(e) = &r.error {
                let text = match r.line {
                    Some(l) => format!("Line {l}: {e}"),
                    None => e.clone(),
                };
                ui.add(egui::Label::new(RichText::new(text).color(t.danger)).wrap());
            }
            // A package installed since this run isn't offered again.
            let installed_since = |pkg: &str| {
                let pkg = gridcraft_engine::pyenv::env::normalize(pkg);
                runs.iter().any(|x| {
                    x.status == "installed"
                        && x.seq > r.seq
                        && x.packages
                            .iter()
                            .any(|p| gridcraft_engine::pyenv::env::normalize(gridcraft_engine::pyenv::env::requirement_name(p)) == pkg)
                })
            };
            if let Some((label, params)) = install_offer(r).filter(|_| !r.missing.as_deref().is_some_and(installed_since))
                && ui.button(label).on_hover_text("Install the package that provides this module, after you approve").clicked()
            {
                offer = Some(params);
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
    if let Some(params) = offer {
        app.open_dialog("pythonInstall", params);
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

/// A run's status as Diagnostics shows it, with its colour.
pub fn status_label<'a>(status: &'a str, t: &Tokens) -> (&'a str, egui::Color32) {
    match status {
        "ok" => ("OK", t.text_dim),
        "error" => ("Error", t.danger),
        "timeout" => ("Time limit", t.danger),
        "memory" => ("Memory limit", t.danger),
        "stopped" => ("Stopped", t.danger),
        "blocked" => ("Turned off", t.text_dim),
        "cycle" => ("Cycle", t.danger),
        "refused" => ("Not run", t.danger),
        "retried" => ("Retried", t.text_dim),
        "installed" => ("Installed", t.text_dim),
        "installFailed" => ("Install failed", t.danger),
        "cancelled" => ("Cancelled", t.text_dim),
        other => (other, t.text_dim),
    }
}

/// The body of a package install's row in Diagnostics: how it ended (with the versions), where
/// it installed, whether cells still fail after it (for another reason), and the installer's
/// output, folded.
fn install_row(ui: &mut Ui, t: &Tokens, r: &gridcraft_engine::python::PyRunLog, runs: &[gridcraft_engine::python::PyRunLog]) {
    let summary = r.summary.clone().or_else(|| r.error.clone()).unwrap_or_default();
    let color = match r.status {
        "installed" => t.text,
        "installFailed" => t.danger,
        _ => t.text_dim,
    };
    ui.add(egui::Label::new(RichText::new(summary).color(color).font(theme::ui_bold(12.5))).wrap());
    if let Some(target) = &r.target {
        ui.add(egui::Label::new(RichText::new(format!("Into {target}")).color(t.text_dim).font(theme::ui_font(12.0))).wrap());
    }
    // An install that worked whose warm-up (importing it once) didn't.
    if r.status == "installed"
        && let Some(warning) = &r.error
    {
        ui.add(egui::Label::new(RichText::new(warning).color(t.text).font(theme::ui_font(12.0))).wrap());
    }
    if r.status == "installed" {
        // Cells that failed after it, for a reason other than a missing module: the install
        // itself worked.
        let mut failing: Vec<String> = runs
            .iter()
            .filter(|x| x.seq > r.seq && !x.cell.is_empty() && x.status != "ok" && x.missing.is_none())
            .map(|x| format!("{}!{}", gridcraft_engine::formula::quote_sheet(&x.sheet), x.cell))
            .collect();
        failing.dedup();
        if !failing.is_empty() {
            let shown = failing.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            let more = if failing.len() > 3 { format!(" and {} more", failing.len() - 3) } else { String::new() };
            let verb = if failing.len() == 1 { "still fails" } else { "still fail" };
            let text = format!("The install worked. {shown}{more} {verb} for another reason: see their rows.");
            ui.add(egui::Label::new(RichText::new(text).color(t.text_dim).font(theme::ui_font(12.0))).wrap());
        }
    }
    if !r.stdout.trim().is_empty() {
        egui::CollapsingHeader::new(RichText::new("Installer output").color(t.text_dim).font(theme::ui_font(11.5)))
            .id_salt(("python.install.output", r.seq))
            .show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(r.stdout.trim_end()).font(crate::code_editor::font()).color(t.text)).wrap());
            });
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
    if crate::widgets::code_editor(ui, app.widget_id("python.initialization.code"), &mut code, Lang::Python, 12).changed() {
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
                crate::icons::paint_accent(ui.painter(), r, Icon::Python, t.text, Some(t.cat(theme::Cat::Green)));
                ui.label(RichText::new("Python cells are turned off for this workbook.").font(theme::ui_bold(12.5)).color(t.text));
                let mut note = match cpython_path(app) {
                    Some(path) => format!(
                        "Python cells run with your Python at {path}, with full access to your files and network. Enable only for workbooks you trust."
                    ),
                    None => "Python runs in a sandbox with no file or network access.".to_string(),
                };
                let needs = app.session.active().map(|d| gridcraft_engine::pyenv::declared(&d.wb)).unwrap_or_default();
                if !needs.is_empty() {
                    note.push_str(&format!(
                        " It needs {}; GridCraft installs packages only after you approve, into its own environment.",
                        needs.join(", ")
                    ));
                }
                // Room is kept for the button: a long path is cut, with the whole text on hover.
                let room = (ui.available_width() - 150.0).max(80.0);
                ui.allocate_ui(vec2(room, 24.0), |ui| {
                    ui.add(egui::Label::new(RichText::new(&note).font(theme::ui_font(12.5)).color(t.text)).truncate()).on_hover_text(&note);
                });
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
    let r = ui.add(egui::Label::new(RichText::new(label).font(theme::ui_font(12.0)).color(t.text_dim)).sense(egui::Sense::click()));
    if r.on_hover_text(summary).clicked() {
        app.open_dialog("pythonOptions", json!({}));
    }
}

/// Opens Python Options with the current settings.
pub fn open_options(app: &mut SheetApp) -> Dialog {
    let p = &app.session.prefs;
    let mut d = Dialog::custom(
        "pythonOptions",
        "Python Options",
        json!({"interpreter": p.python_interpreter, "timeoutSeconds": p.python_timeout_seconds, "importSeconds": p.python_import_seconds}),
    );
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
            crate::icons::paint_accent(ui.painter(), r, Icon::Python, t.text, Some(t.cat(theme::Cat::Green)));
            if kind == "detecting" {
                ui.add(egui::Spinner::new().size(14.0).color(t.text_dim));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            ui.add(egui::Label::new(RichText::new(s("summary").unwrap_or_default()).font(theme::ui_bold(12.5)).color(t.text)).wrap());
        });
        ui.add_space(4.0);
        if kind == "cpython" {
            egui::Grid::new("python.options.backend").num_columns(2).spacing(vec2(12.0, 3.0)).show(ui, |ui| {
                let not = || "not installed".to_string();
                for (label, value) in [
                    ("Version", s("version").unwrap_or_default()),
                    ("Program", s("path").unwrap_or_default()),
                    ("Found in", s("source").unwrap_or_default()),
                    ("pandas", s("pandas").unwrap_or_else(not)),
                    ("numpy", s("numpy").unwrap_or_else(not)),
                ] {
                    ui.label(RichText::new(label).color(t.text_dim));
                    ui.add(egui::Label::new(RichText::new(value).color(t.text)).wrap());
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            let note = if b["externallyManaged"].as_bool().unwrap_or(false) {
                "Cells have full access to your files and network. This Python is managed by your system, so GridCraft never installs into it: when a cell needs a package, it offers to install it, only after you approve, into GridCraft's own environment below."
            } else {
                "Cells have full access to your files and network. When a cell needs a package that isn't installed, GridCraft offers to install it, only after you approve, into your own virtual environment when it runs cells, otherwise into GridCraft's own environment (never into a system Python)."
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
            egui::CollapsingHeader::new(RichText::new(format!("Not used ({})", rejected.len())).color(t.text_dim)).id_salt("python.options.rejected").show(ui, |ui| {
                for r in &rejected {
                    let text = format!("{}: {}", r["path"].as_str().unwrap_or_default(), r["reason"].as_str().unwrap_or_default());
                    ui.add(egui::Label::new(RichText::new(text).color(t.text_dim).font(theme::ui_font(12.0))).wrap());
                }
            });
        }
    });
    ui.add_space(10.0);
    ui.label(RichText::new("Python interpreter").strong());
    let mut path = d.values.get("interpreter").and_then(Json::as_str).unwrap_or("").to_string();
    let mut detect = false;
    ui.horizontal(|ui| {
        let browse = app.services.pick_program.is_some();
        let width = ui.available_width() - if browse { 190.0 } else { 110.0 };
        let edit = egui::TextEdit::singleline(&mut path).hint_text("Automatic").desired_width(width.max(120.0));
        if ui.add(edit).on_hover_text("A Python 3.11 or newer program, or a name on PATH such as python3.12").changed() {
            d.values.insert("interpreter".into(), json!(path));
        }
        if browse
            && ui.button("Browse…").clicked()
            && let Some(picked) = app.services.pick_program.as_ref().and_then(|pick| pick())
        {
            path = picked;
            d.values.insert("interpreter".into(), json!(path));
        }
        detect = ui.button("Detect Again").on_hover_text("Look for Python again (after installing Python or a package)").clicked();
    });
    ui.label(
        RichText::new("Leave it empty to look for Python 3.11 or newer: an active virtual environment first, then GridCraft's own environment, then python3 on PATH.")
            .color(t.text_dim)
            .font(theme::ui_font(12.0)),
    );
    if detect && let Err(e) = app.run("python.options", json!({"interpreter": path, "detect": true})) {
        app.message = Some((d.title.clone(), crate::clean_error(&e)));
    }
    ui.add_space(10.0);
    environment_section(app, ui, open);
    // The packages this workbook needs (kept in the workbook).
    let has_doc = app.session.active().is_some();
    if has_doc {
        ui.add_space(10.0);
        ui.label(RichText::new("Packages this workbook needs").strong());
        if !d.values.contains_key("packages") {
            let list = app.session.active().map(|doc| gridcraft_engine::pyenv::declared(&doc.wb)).unwrap_or_default();
            d.values.insert("packages".into(), json!(list.join(", ")));
        }
        let mut text = d.values.get("packages").and_then(Json::as_str).unwrap_or_default().to_string();
        let edit = egui::TextEdit::singleline(&mut text).hint_text("None (for example: pandas, numpy)").desired_width(ui.available_width());
        if ui.add(edit).on_hover_text("Package names, each with an optional version; saved with the workbook").changed() {
            d.values.insert("packages".into(), json!(text));
        }
        if let Err(e) = gridcraft_engine::pyenv::env::check_requirements(&gridcraft_engine::pyenv::env::split_requirements(&text)) {
            ui.add(egui::Label::new(RichText::new(e).color(t.danger).font(theme::ui_font(12.0))).wrap());
        }
    }
    ui.add_space(10.0);
    let mut secs = d.values.get("timeoutSeconds").and_then(Json::as_f64).unwrap_or(5.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Time limit per cell").strong());
        if ui.add(egui::DragValue::new(&mut secs).range(0.1..=3_600.0).speed(0.1).max_decimals(1).suffix(" s")).changed() {
            d.values.insert("timeoutSeconds".into(), json!(secs));
        }
    });
    let mut imports = d.values.get("importSeconds").and_then(Json::as_f64).unwrap_or(60.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Time allowed for imports").strong());
        let edit = egui::DragValue::new(&mut imports).range(1.0..=600.0).speed(1.0).max_decimals(0).suffix(" s");
        if ui.add(edit).on_hover_text("Loading a package that isn't loaded yet doesn't count against the time limit, up to this much per cell (the first import after an install can be slow)").changed() {
            d.values.insert("importSeconds".into(), json!(imports));
        }
    });
    ui.add_space(8.0);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add(egui::Button::new(RichText::new("   OK   ").color(t.on_accent)).fill(t.accent)).clicked() {
            *confirm = true;
        }
        if ui.button(" Cancel ").clicked() {
            *open = false;
        }
    });
    if *confirm {
        *confirm = false;
        let mut result = app.run("python.options", json!({"interpreter": path, "timeoutSeconds": secs, "importSeconds": imports}));
        // The workbook's packages, when they changed.
        if result.is_ok()
            && let Some(text) = d.values.get("packages").and_then(Json::as_str)
        {
            let list = gridcraft_engine::pyenv::env::split_requirements(text);
            let now = app.session.active().map(|doc| gridcraft_engine::pyenv::declared(&doc.wb)).unwrap_or_default();
            if gridcraft_engine::pyenv::env::check_requirements(&list).is_ok_and(|l| l != now) {
                result = app.run("python.packages", json!({"packages": list}));
            }
        }
        match result {
            Ok(_) => *open = false,
            Err(e) => app.message = Some((d.title.clone(), crate::clean_error(&e))),
        }
    }
}

/// The status bar while Python works in the background: "Running Python…" or "Installing
/// pandas, numpy…", with Stop (Esc does the same); then, for a while, how the install ended
/// (click it for Diagnostics).
pub fn busy_status(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let install = app.session.python_install_status();
    let state = install["state"].as_str().unwrap_or("idle").to_string();
    let packages = || install["packages"].as_array().map(|a| a.iter().filter_map(Json::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    if app.session.python_busy() {
        ui.add_space(12.0);
        ui.add(egui::Spinner::new().size(12.0).color(t.text_dim));
        let installing = state == "running";
        let (text, tip) = if installing {
            let p = packages();
            let text = match install["stage"].as_str() {
                // After the install: "Preparing scikit-learn…" (importing it once).
                Some(stage) => stage.to_string(),
                None if p.is_empty() => "Making the Python environment…".to_string(),
                None => format!("Installing {p}…"),
            };
            (text, "Cancel the install (Esc)")
        } else {
            (app.ui.language.tr("Running Python…").to_string(), app.ui.language.tr("Stop the Python cells that are running (Esc)"))
        };
        let tip = tip.to_string();
        // An install is the one thing running: its line is in the normal text colour.
        ui.label(RichText::new(text).font(theme::ui_font(12.5)).color(if installing { t.text } else { t.text_dim }));
        let label = if installing { "Cancel" } else { app.ui.language.tr("Stop") };
        let stop = ui.add(egui::Button::new(RichText::new(label).font(theme::ui_font(12.0)).color(t.text)).min_size(vec2(44.0, 18.0)));
        if stop.on_hover_text(tip).clicked() {
            app.run_or_alert("python.stop", json!({}));
        }
        return;
    }
    if !matches!(state.as_str(), "installed" | "installFailed" | "cancelled") {
        return;
    }
    // The result shows for 20 seconds from when it first appeared.
    let message = install["summary"].as_str().or(install["message"].as_str()).unwrap_or_default().to_string();
    let key = egui::Id::new("python.install.shown").with(&message).with(&state);
    let now = ui.input(|i| i.time);
    let since: f64 = ui.ctx().data_mut(|m| *m.get_temp_mut_or_insert_with(key, || now));
    if now - since > 20.0 {
        return;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    ui.add_space(12.0);
    let color = if state == "installFailed" { t.danger } else { t.text_dim };
    let short: String = if message.chars().count() > 90 { format!("{}…", message.chars().take(90).collect::<String>()) } else { message.clone() };
    let r = ui.add(egui::Label::new(RichText::new(short).font(theme::ui_font(12.5)).color(color)).sense(egui::Sense::click()));
    if r.on_hover_text(format!("{message}\nClick for Diagnostics")).clicked() {
        app.grid.pane = Some(PANE.into());
    }
}

/// The toast for a package install that ended since the last one shown: "Installed
/// scikit-learn 1.9.1"; on failure the short reason and See Diagnostics; "Cancelled installing …".
pub fn install_toast(app: &mut SheetApp) {
    let ended = app.session.python_installs_ended();
    if ended == app.installs_shown {
        return;
    }
    app.installs_shown = ended;
    let status = app.session.python_install_status();
    let state = status["state"].as_str().unwrap_or_default();
    if !matches!(state, "installed" | "installFailed" | "cancelled") {
        return;
    }
    let text = status["summary"].as_str().or(status["message"].as_str()).unwrap_or("The install ended").to_string();
    let at = crate::now_ms();
    let failed = state == "installFailed";
    app.toast = Some((text, at));
    app.toast_extra = Some(crate::ToastExtra {
        at,
        ms: if failed { 15_000.0 } else { 6_000.0 },
        danger: failed,
        action: failed.then(|| ("See Diagnostics".to_string(), PANE.to_string())),
    });
}

/// Opens the Install Python Packages dialog for `{packages: [pip names], declare?}`, or says
/// why nothing can be installed (with how to install by hand).
pub fn open_install(app: &mut SheetApp, params: &Json) -> Option<Dialog> {
    let packages: Vec<String> =
        params["packages"].as_array().map(|a| a.iter().filter_map(Json::as_str).map(str::to_string).collect()).unwrap_or_default();
    let declare = params["declare"].as_bool().unwrap_or(false);
    match app.session.python_install_target(&packages) {
        Err(why) => {
            app.message = Some(("Install Python Packages".into(), why));
            None
        }
        Ok(target) => {
            let (label, path, create) = match &target {
                gridcraft_engine::pyenv::env::Target::Own { python, .. } => ("Your Python environment", python.display().to_string(), false),
                gridcraft_engine::pyenv::env::Target::GridCraft { dir, .. } => {
                    ("GridCraft's Python environment", dir.display().to_string(), !gridcraft_engine::pyenv::env::is_env(dir))
                }
            };
            Some(Dialog::custom(
                "pythonInstall",
                "Install Python Packages",
                json!({"packages": packages.join(", "), "targetLabel": label, "target": path, "create": create, "declare": declare}),
            ))
        }
    }
}

/// The "Install <packages>…" offer for a run that couldn't import a module, if there is one.
pub fn install_offer(run: &gridcraft_engine::python::PyRunLog) -> Option<(String, Json)> {
    let pkg = run.missing.clone()?;
    Some((format!("Install {pkg}…"), json!({"packages": [pkg], "declare": true})))
}

/// The approval to install packages: where they go, the exact list (editable), and a warning
/// that installing runs code from the internet. Install runs `python.install` with
/// `approve: true` in the background.
pub fn install_dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let s = |k: &str| d.values.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
    ui.label(RichText::new("Install into").strong());
    ui.label(RichText::new(s("targetLabel")).color(t.text));
    ui.add(egui::Label::new(RichText::new(s("target")).font(crate::code_editor::font()).color(t.text_dim)).wrap());
    if d.values.get("create").and_then(Json::as_bool).unwrap_or(false) {
        ui.label(RichText::new("GridCraft makes this environment first, from the Python it found.").color(t.text_dim).font(theme::ui_font(12.0)));
    }
    ui.add_space(8.0);
    ui.label(RichText::new("Packages").strong());
    let mut text = s("packages");
    let edit = egui::TextEdit::singleline(&mut text).hint_text("pandas, numpy>=1.26").desired_width(ui.available_width());
    if ui.add(edit).on_hover_text("Package names from the Python Package Index, each with an optional version").changed() {
        d.values.insert("packages".into(), json!(text));
        d.values.remove("error");
    }
    let list = gridcraft_engine::pyenv::env::split_requirements(&text);
    let checked = gridcraft_engine::pyenv::env::check_requirements(&list);
    let error = match &checked {
        Ok(l) if l.is_empty() => Some("List at least one package.".to_string()),
        Ok(_) => d.values.get("error").and_then(Json::as_str).map(str::to_string),
        Err(e) => Some(e.clone()),
    };
    if let Some(e) = &error {
        ui.add(egui::Label::new(RichText::new(e).color(t.danger).font(theme::ui_font(12.0))).wrap());
    }
    ui.add_space(8.0);
    egui::Frame::NONE.fill(t.menu_bg).corner_radius(6.0).inner_margin(8.0).stroke(egui::Stroke::new(1.0, t.danger)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Installing downloads packages from the internet and runs their code on this computer, with access to your files and network. Install only packages you trust.",
                )
                .color(t.text)
                .font(theme::ui_font(12.0)),
            )
            .wrap(),
        );
    });
    ui.add_space(8.0);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let ok = matches!(&checked, Ok(l) if !l.is_empty());
        if ui.add_enabled(ok, egui::Button::new(RichText::new("  Install  ").color(t.on_accent)).fill(t.accent)).clicked() {
            *confirm = true;
        }
        if ui.button(" Cancel ").clicked() {
            *open = false;
        }
    });
    if *confirm {
        *confirm = false;
        let Ok(list) = checked else { return };
        let declare = d.values.get("declare").and_then(Json::as_bool).unwrap_or(false);
        match app.run("python.install", json!({"packages": list, "approve": true, "declare": declare})) {
            Ok(_) => *open = false,
            Err(e) => {
                d.values.insert("error".into(), json!(crate::clean_error(&e)));
            }
        }
    }
}

/// GridCraft's environment in Python Options: its folder, whether it exists and runs cells, its
/// packages, and Create, Install packages… and Delete environment.
fn environment_section(app: &mut SheetApp, ui: &mut Ui, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let env = app.session.python_environment();
    ui.label(RichText::new("GridCraft's Python environment").strong());
    let path = env["path"].as_str().unwrap_or("(no data folder)").to_string();
    ui.add(egui::Label::new(RichText::new(&path).font(crate::code_editor::font()).color(t.text_dim)).wrap());
    let exists = env["exists"].as_bool().unwrap_or(false);
    let packages = env["packages"].as_array().cloned().unwrap_or_default();
    let note = if !exists {
        "Not made yet. GridCraft makes it from the Python it found when you first install a package, and installs only after you approve.".to_string()
    } else if env["active"].as_bool().unwrap_or(false) {
        format!("Runs Python cells now. {} packages installed.", packages.len())
    } else {
        format!("{} packages installed.", packages.len())
    };
    ui.add(egui::Label::new(RichText::new(note).color(t.text_dim).font(theme::ui_font(12.0))).wrap());
    if exists && !packages.is_empty() {
        egui::CollapsingHeader::new(RichText::new(format!("Packages ({})", packages.len())).color(t.text_dim))
            .id_salt("python.options.envpackages")
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for p in &packages {
                        let text = format!("{} {}", p["name"].as_str().unwrap_or_default(), p["version"].as_str().unwrap_or_default());
                        ui.label(RichText::new(text).color(t.text).font(theme::ui_font(12.0)));
                    }
                });
            });
    }
    let busy = app.session.python_installing();
    let (mut create, mut install, mut delete) = (false, false, false);
    ui.horizontal(|ui| {
        if !exists {
            create = ui
                .add_enabled(!busy, egui::Button::new("Create"))
                .on_hover_text("Make the environment now, from the Python GridCraft found")
                .clicked();
        }
        install = ui.add_enabled(!busy, egui::Button::new("Install packages…")).clicked();
        if exists {
            delete =
                ui.add_enabled(!busy, egui::Button::new("Delete environment")).on_hover_text("Delete the folder and every package in it").clicked();
        }
    });
    if create {
        app.run_or_alert("python.environment", json!({"action": "create"}));
    }
    if install && let Some(dialog) = open_install(app, &json!({"packages": []})) {
        // Python Options closes, and the install dialog opens in its place.
        app.dialog_next = Some(dialog);
        *open = false;
    }
    if delete {
        app.confirm = Some(crate::Confirm {
            title: "Python Options".into(),
            text: format!("Delete GridCraft's Python environment at {path}, with every package installed in it?"),
            command: "python.environment".into(),
            params: json!({"action": "delete"}),
            choices: vec![("Delete".into(), json!({"approve": true}))],
        });
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

/// `python.saveCsv {cell?, path?}`: saves a Python cell's tabular result as CSV, to `path`, or
/// where the save dialog says (a download on the web).
pub fn save_csv(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let cell = p.get("cell").and_then(Json::as_str);
    let (rows, _) = cell_table(app, cell).ok_or("The cell isn't a Python cell with a table result")?;
    let csv = gridcraft_engine::python::to_csv(&rows);
    let path = match p.get("path").and_then(Json::as_str) {
        Some(path) => Some(path.to_string()),
        None => match &app.services.pick_save {
            Some(pick) => match pick("python-result.csv") {
                Some(path) => Some(path),
                None => return Ok(json!({"saved": false})),
            },
            None => None,
        },
    };
    match path {
        Some(path) => {
            let path = if std::path::Path::new(&path).extension().is_none() { format!("{path}.csv") } else { path };
            #[cfg(not(target_arch = "wasm32"))]
            std::fs::write(&path, csv.as_bytes()).map_err(|e| format!("Couldn't save {path}: {e}"))?;
            Ok(json!({"saved": true, "path": path, "rows": rows.len()}))
        }
        None => match &app.services.download {
            Some(download) => {
                download("python-result.csv", csv.as_bytes());
                Ok(json!({"saved": true, "rows": rows.len()}))
            }
            None => Err("Saving needs a file dialog, which this host doesn't have; pass `path`".into()),
        },
    }
}

/// The tabular result in the preview card: a small scrollable grid with headers (a numbered
/// header when the table has none).
pub fn table_view(ui: &mut Ui, rows: &[Vec<String>], headers: bool) {
    const MAX_ROWS: usize = 200;
    const MAX_COLS: usize = 30;
    let t = Tokens::get(ui.ctx());
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).min(MAX_COLS);
    let numbered: Vec<String> = (0..cols).map(|c| c.to_string()).collect();
    let (head, body) = match rows.split_first() {
        Some((h, rest)) if headers => (h.as_slice(), rest),
        _ => (numbered.as_slice(), rows),
    };
    let cut = |s: &str| if s.chars().count() > 40 { format!("{}…", s.chars().take(40).collect::<String>()) } else { s.to_string() };
    egui::ScrollArea::both().id_salt("py_card.table").max_height(220.0).max_width(520.0).auto_shrink([true, true]).show(ui, |ui| {
        egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.separator)).show(ui, |ui| {
            egui::Grid::new("py_card.table.grid").striped(true).spacing(vec2(12.0, 3.0)).show(ui, |ui| {
                for h in head.iter().take(cols) {
                    ui.label(RichText::new(cut(h)).font(theme::ui_bold(12.0)).color(t.text));
                }
                ui.end_row();
                for row in body.iter().take(MAX_ROWS) {
                    for c in 0..cols {
                        ui.label(RichText::new(cut(row.get(c).map(String::as_str).unwrap_or(""))).font(theme::ui_font(12.0)).color(t.text));
                    }
                    ui.end_row();
                }
            });
        });
    });
    let more_rows = body.len() > MAX_ROWS;
    let more_cols = rows.iter().map(Vec::len).max().unwrap_or(0) > MAX_COLS;
    if more_rows || more_cols {
        let text = format!("Showing {} of {} rows; Copy and Save have them all.", body.len().min(MAX_ROWS), body.len());
        ui.label(RichText::new(text).color(t.text_dim).font(theme::ui_font(11.5)));
    }
}

/// How long ago something happened, from its age in milliseconds: "just now", "12 s ago",
/// "3 min ago", "2 h ago", "4 d ago".
pub fn ago(ms: f64) -> String {
    let secs = if ms.is_finite() { (ms / 1000.0).max(0.0) as u64 } else { 0 };
    match secs {
        0..=4 => "just now".into(),
        5..=59 => format!("{secs} s ago"),
        60..=3_599 => format!("{} min ago", secs / 60),
        3_600..=86_399 => format!("{} h ago", secs / 3_600),
        _ => format!("{} d ago", secs / 86_400),
    }
}
