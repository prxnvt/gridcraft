//! The Python editor window: a resizable window, about 40 lines tall, for writing a Python cell's
//! code, opened from the formula bar's expand button, the PY badge menu, Insert Python's menu and
//! `ui.pythonEditor`. It has Run / Stop / Re-run, the output mode, Insert xl() Reference and
//! Diagnostics over the code, and under it a strip with the cell's last run: status, result,
//! printed output and the error, whose line a click goes to.
//!
//! The window edits the same `app.editor` as the formula bar (which shows the code read-only
//! while the window is open), so committing still goes through `python.set`, with undo, `#BUSY!`
//! and Run as before. Its state is kept apart from `app.dialog`, so other dialogs (installing a
//! package) don't close it. Esc and the X close it and keep the edit.

use egui::{Id, RichText, Sense, Ui, pos2, vec2};
use gridcraft_engine::core::{CellRef, Value};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::code_editor::{self, Lang};
use crate::formula_bar::{self, PyBody, RunButton};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// The cell the window shows: the one being edited, otherwise the active cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PyWindow {
    pub sheet: usize,
    pub cell: CellRef,
}

/// Lines of code the window shows when it opens.
pub const LINES: usize = 40;

/// The window's code editor.
pub fn editor_id() -> Id {
    Id::new("gridcraft.python_window")
}

/// The window (egui keeps its size and position under this id).
pub fn window_id() -> Id {
    Id::new("python_editor")
}

/// The active cell (the top left of a merged area) and its sheet.
fn active_cell(app: &SheetApp) -> Option<(usize, CellRef)> {
    let d = app.session.active()?;
    let sh = d.wb.active()?;
    let at = sh.merge_at(d.selection.active).map(|m| m.start).unwrap_or(d.selection.active);
    Some((d.wb.active_sheet, at))
}

/// Opens the window on the Python cell being edited, or the active cell. A selected Python cell
/// starts being edited in it.
pub fn open(app: &mut SheetApp) {
    if app.editor.is_none() && formula_bar::active_python(app) {
        app.begin_python_edit();
        // The window opens at the top of the code (the formula bar's edit starts at its end).
        if let Some(ed) = app.editor.as_mut() {
            ed.caret = 0;
            ed.sync_caret = true;
        }
    }
    let target = match &app.editor {
        Some(ed) if ed.python => Some((ed.sheet, ed.cell)),
        _ => active_cell(app),
    };
    let Some((sheet, cell)) = target else { return };
    app.py_window = Some(PyWindow { sheet, cell });
    focus(app);
}

/// Puts the keyboard in the window's editor (starting an edit of the selected Python cell).
pub fn focus(app: &mut SheetApp) {
    if app.py_window.is_none() {
        return;
    }
    if app.editor.is_none() && formula_bar::active_python(app) {
        app.begin_python_edit();
    }
    if let Some(ed) = app.editor.as_mut().filter(|e| e.python) {
        ed.request_focus = true;
    }
}

/// Closes the window; an edit stays open, in the formula bar.
pub fn close(app: &mut SheetApp) {
    app.py_window = None;
    if let Some(ed) = app.editor.as_mut() {
        ed.request_focus = true;
    }
}

/// Whether the code being edited differs from the cell's.
fn dirty(app: &SheetApp) -> bool {
    let Some(ed) = app.editor.as_ref().filter(|e| e.python) else { return false };
    let saved =
        app.session.active().and_then(|d| d.wb.sheet(ed.sheet)).and_then(|s| s.cell(ed.cell)).and_then(|c| c.py.as_ref()).map(|p| p.code.clone());
    match saved {
        Some(code) => gridcraft_engine::model::python::normalize_code(&ed.text) != code,
        None => !ed.text.is_empty(),
    }
}

/// The window's state for `ui.inspect` and `ui.pythonEditor`.
pub fn state(app: &SheetApp) -> Json {
    let Some(w) = app.py_window else { return json!({"open": false}) };
    let sheet = app.session.active().and_then(|d| d.wb.sheet(w.sheet)).map(|s| s.name.clone()).unwrap_or_default();
    let editing = app.editor.as_ref().is_some_and(|e| e.python && e.sheet == w.sheet && e.cell == w.cell);
    json!({"open": true, "sheet": sheet, "cell": w.cell.a1(), "editing": editing, "dirty": dirty(app), "python": app.is_python_cell(w.sheet, w.cell)})
}

/// `ui.pythonEditor {cell?, sheet?, close?}`: opens the window (on that cell, selecting it
/// first), or closes it (`close: true`; an edit stays open). Replies with the window's state.
pub fn command(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    if p.get("close").and_then(Json::as_bool).unwrap_or(false) {
        close(app);
        return Ok(state(app));
    }
    let cell = p.get("cell").and_then(Json::as_str).filter(|c| !c.is_empty());
    let sheet = p.get("sheet").and_then(Json::as_str).filter(|s| !s.is_empty());
    if cell.is_some() || sheet.is_some() {
        let d = app.session.active().ok_or("no workbook is open")?;
        let si = match sheet {
            Some(name) => d.wb.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("no sheet named {name:?}"))?,
            None => d.wb.active_sheet,
        };
        let at = match cell {
            Some(c) => CellRef::parse(c).ok_or_else(|| format!("{c:?} is not a cell"))?,
            None if si == d.wb.active_sheet => d.selection.active,
            None => CellRef::new(0, 0),
        };
        let at = d.wb.sheet(si).and_then(|sh| sh.merge_at(at)).map(|m| m.start).unwrap_or(at);
        if let Some(ed) = &app.editor
            && (ed.sheet != si || ed.cell != at)
        {
            return Err(format!("{} is being edited: commit or cancel that edit first", ed.cell.a1()));
        }
        if si != d.wb.active_sheet {
            let name = d.wb.sheet(si).map(|s| s.name.clone()).unwrap_or_default();
            app.run("sheet.activate", json!({"sheet": name}))?;
        }
        app.run("selection.set", json!({"cell": at.a1()}))?;
    }
    open(app);
    Ok(state(app))
}

/// The window follows the edit, or else the selection.
fn follow(app: &SheetApp, w: &mut PyWindow) {
    match &app.editor {
        Some(ed) if ed.python => (w.sheet, w.cell) = (ed.sheet, ed.cell),
        // A formula being typed somewhere: the window stays where it was.
        Some(_) => {}
        None => {
            if let Some((sheet, cell)) = active_cell(app) {
                (w.sheet, w.cell) = (sheet, cell);
            }
        }
    }
}

/// Draws the window, if open.
pub fn show(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(mut w) = app.py_window else { return };
    follow(app, &mut w);
    app.py_window = Some(w);
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let row_h = ctx.fonts_mut(|f| f.row_height(&code_editor::font()));
    // Below the formula bar (or the ribbon, with the bar hidden).
    let bar = formula_bar::BarLayout::get(ctx).field;
    let top = if bar.is_positive() && bar.bottom() > screen.top() { bar.bottom() + 8.0 } else { screen.top() + 150.0 };
    let want = LINES as f32 * row_h + STRIP + 64.0;
    let height = want.min((screen.bottom() - top - 12.0).max(320.0));
    let sheet_name = app.session.active().and_then(|d| d.wb.sheet(w.sheet)).map(|s| s.name.clone()).unwrap_or_default();
    let editing = app.editor.as_ref().is_some_and(|e| e.python);
    let title = if editing || app.is_python_cell(w.sheet, w.cell) {
        let mark = if dirty(app) { " \u{2022}" } else { "" };
        format!("Python: {}!{}{mark}", gridcraft_engine::formula::quote_sheet(&sheet_name), w.cell.a1())
    } else {
        "Python Editor".to_string()
    };
    let mut open = true;
    let mut close_now = false;
    egui::Window::new(title)
        .id(window_id())
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(760.0, height))
        .min_size(vec2(480.0, 320.0))
        // On the right, so the data the code reads stays in view.
        .default_pos(pos2((screen.right() - 780.0).max(screen.left() + 20.0), top))
        .open(&mut open)
        .show(ctx, |ui| {
            close_now = contents(app, ui, w, &t);
        });
    if !open || close_now {
        close(app);
    }
}

/// The output strip's default height.
const STRIP: f32 = 150.0;

/// The window's contents; returns true when it asked to close (Esc).
fn contents(app: &mut SheetApp, ui: &mut Ui, w: PyWindow, t: &Tokens) -> bool {
    let editing = app.editor.as_ref().is_some_and(|e| e.python);
    let python = editing || app.is_python_cell(w.sheet, w.cell);
    if !python {
        placeholder(app, ui, t);
        return false;
    }
    egui::Panel::top("python_editor.toolbar")
        .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 4 }))
        .show(ui, |ui| toolbar(app, ui, w, t));
    egui::Panel::bottom("python_editor.output")
        .resizable(true)
        .default_size(STRIP)
        .size_range(48.0..=600.0)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 0, right: 0, top: 6, bottom: 0 }))
        .show(ui, |ui| output_strip(app, ui, w, t));
    let mut close = false;
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.input_bg).inner_margin(4.0)).show(ui, |ui| {
        close = code(app, ui, w, t);
    });
    close
}

/// The error line of the cell's last run, as a byte range of `code`, when `code` is what ran.
fn error_mark(app: &SheetApp, w: PyWindow, code: &str) -> Option<std::ops::Range<usize>> {
    let d = app.session.active()?;
    let sh = d.wb.sheet(w.sheet)?;
    let py = sh.cell(w.cell)?.py.as_ref()?;
    if py.code != gridcraft_engine::model::python::normalize_code(code) || !matches!(sh.value(w.cell), Value::Error(_)) {
        return None;
    }
    let line = last_run(app, w)?.line?;
    code_editor::line_range(code, line)
}

/// The code: the editor while the cell is edited, otherwise the cell's code, which a click starts
/// editing (with the caret where it was clicked). Returns true on Esc.
fn code(app: &mut SheetApp, ui: &mut Ui, w: PyWindow, t: &Tokens) -> bool {
    let id = editor_id();
    let size = ui.available_size();
    if let Some(mut ed) = app.editor.take() {
        if !ed.python {
            app.editor = Some(ed);
            return false;
        }
        let keys = formula_bar::python_keys(app, ui, &mut ed, id);
        let mark = error_mark(app, w, &ed.text);
        let body = PyBody { width: size.x, max_height: size.y, fill: true, gutter: true, mark };
        formula_bar::python_body(ui, &mut ed, id, &body, keys.edited);
        app.session.mode = if ed.py_point { gridcraft_engine::Mode::Point } else { gridcraft_engine::Mode::Edit };
        app.editor = Some(ed);
        if keys.escape {
            return true;
        } else if keys.leave {
            app.leave_python();
        } else if keys.commit {
            app.commit_edit(0, 0, false, false);
        }
        return false;
    }
    // Not editing: the cell's code, read-only until clicked.
    let text = app.session.active().and_then(|d| d.wb.sheet(w.sheet)?.cell(w.cell)?.py.as_ref().map(|p| p.code.clone())).unwrap_or_default();
    let font = code_editor::font();
    let gutter = code_editor::gutter_width(ui, text.lines().count() + 1, &font);
    let mut job = code_editor::layout_job(&text, Lang::Python, &font, t.text, &t.syntax, (size.x - gutter).max(1.0));
    if let Some(m) = error_mark(app, w, &text) {
        code_editor::underline(&mut job, m, t.danger);
    }
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let mut caret: Option<usize> = None;
    egui::ScrollArea::vertical().id_salt(id.with("view")).max_height(size.y.max(1.0)).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(gutter);
            let h = galley.size().y.max(size.y);
            let (rect, resp) = ui.allocate_exact_size(vec2((size.x - gutter).max(1.0), h), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::Text);
            let x = rect.left() - 4.0;
            ui.painter().vline(x, rect.top()..=rect.bottom(), egui::Stroke::new(1.0, t.separator));
            ui.painter().galley(rect.min, galley.clone(), t.text);
            code_editor::paint_line_numbers(ui.painter(), &galley, rect.min, x - 6.0, &font, t.text_dim);
            if resp.clicked() {
                let at = resp.interact_pointer_pos().map(|p| usize::from(galley.cursor_from_pos(p - rect.min).index)).unwrap_or(text.chars().count());
                caret = Some(at);
            }
        });
    });
    if let Some(at) = caret {
        edit_at(app, w, at);
    }
    false
}

/// Starts editing the window's cell (when it isn't already) with the caret at char `at`.
fn edit_at(app: &mut SheetApp, w: PyWindow, at: usize) {
    if app.editor.is_none() && active_cell(app) == Some((w.sheet, w.cell)) {
        app.begin_python_edit();
    }
    if let Some(ed) = app.editor.as_mut().filter(|e| e.python && e.sheet == w.sheet && e.cell == w.cell) {
        ed.caret = at.min(ed.text.chars().count());
        ed.sync_caret = true;
        ed.request_focus = true;
    }
}

/// "Select a Python cell", with Insert Python Here on a normal cell.
fn placeholder(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    ui.set_min_size(ui.available_size());
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 40.0).max(8.0));
        let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        icons::paint_accent(ui.painter(), r, Icon::Python, t.text_dim, Some(t.cat(theme::Cat::Green)));
        ui.label(RichText::new("Select a Python cell").color(t.text_dim).font(theme::ui_font(14.0)));
        ui.add_space(6.0);
        if app.editor.is_none()
            && ui.button("Insert Python Here").on_hover_text("Make the selected cell a Python cell and write its code here").clicked()
        {
            app.run_or_alert("python.insert", json!({}));
            if formula_bar::active_python(app) {
                app.begin_python_edit();
                focus(app);
            }
        }
    });
}

/// A flat toolbar button: an icon and a label, highlighted when hovered or `on`.
fn tool_button(ui: &mut Ui, icon: Icon, color: egui::Color32, label: &str, tip: &str, enabled: bool, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let font = theme::ui_font(12.5);
    let text_color = if enabled { t.text } else { t.text_disabled };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, text_color);
    let size = vec2(galley.size().x + 32.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if on {
        ui.painter().rect_filled(rect, 4.0, t.pressed);
        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
    } else if enabled && resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 4.0, t.pressed);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let icon_rect = egui::Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint(ui.painter(), icon_rect, icon, color);
    ui.painter().galley(pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0), galley, text_color);
    let name = label.to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
    resp.on_hover_text(tip)
}

/// Run / Stop / Re-run, the output mode, Insert xl() Reference and Diagnostics.
fn toolbar(app: &mut SheetApp, ui: &mut Ui, w: PyWindow, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if let Some(state) = formula_bar::run_button_state(app) {
            let (icon, color, label) = match state {
                RunButton::Run { enabled: true } => (Icon::RunCell, t.cat(theme::Cat::Green), "Run"),
                RunButton::Run { enabled: false } => (Icon::RunCell, t.text_disabled, "Run"),
                RunButton::Stop => (Icon::StopCell, t.danger, "Stop"),
                RunButton::Rerun => (Icon::Rerun, t.text, "Re-run"),
            };
            let enabled = state != RunButton::Run { enabled: false };
            let tip = format!("{} (Ctrl+Enter commits and runs)", state.tooltip());
            if tool_button(ui, icon, color, label, &tip, enabled, false).clicked() {
                formula_bar::press_run_button(app, state);
            }
        }
        ui.separator();
        // The output mode, for a cell that is a Python cell already.
        let object = app.session.active().and_then(|d| d.wb.sheet(w.sheet)?.cell(w.cell)?.py.as_ref().map(|p| p.object));
        if let Some(object) = object {
            let sheet = app.session.active().and_then(|d| d.wb.sheet(w.sheet)).map(|s| s.name.clone()).unwrap_or_default();
            for (label, as_object, tip) in
                [("Object", true, "Output as a Python object (Ctrl+Alt+Shift+M)"), ("Values", false, "Output as Excel values (Ctrl+Alt+Shift+M)")]
            {
                if ui.add(egui::Button::selectable(object == as_object, label)).on_hover_text(tip).clicked() && object != as_object {
                    let mode = if as_object { "object" } else { "value" };
                    app.run_or_alert("python.output", json!({"cell": w.cell.a1(), "sheet": sheet, "mode": mode}));
                }
            }
            ui.separator();
        }
        let pointing = app.editor.as_ref().is_some_and(|e| e.python && e.py_point);
        let tip = "Insert xl() Reference: click or drag on the sheet to insert a reference (F2 turns it on or off)";
        if tool_button(ui, Icon::Python, t.text, "Insert xl() Reference", tip, true, pointing).clicked() {
            match app.editor.as_mut().filter(|e| e.python) {
                Some(ed) if ed.py_point => {
                    ed.py_point = false;
                    ed.point = None;
                    ed.point_cell = None;
                    ed.request_focus = true;
                }
                _ => app.point_python(),
            }
        }
        if tool_button(ui, Icon::Diagnostics, t.text, "Diagnostics", "Python Diagnostics: every run, its output and errors", true, false).clicked() {
            app.grid.pane = Some(crate::python_ui::PANE.into());
        }
    });
}

/// The cell's latest run in Diagnostics.
fn last_run(app: &SheetApp, w: PyWindow) -> Option<gridcraft_engine::python::PyRunLog> {
    let d = app.session.active()?;
    let sheet = d.wb.sheet(w.sheet)?.name.clone();
    let cell = w.cell.a1();
    d.py_log.iter().rev().find(|r| r.sheet == sheet && r.cell == cell).cloned()
}

/// The output strip: the cell's last run (status and time, the result, printed output, the error
/// as "Line N: …", which a click goes to).
fn output_strip(app: &mut SheetApp, ui: &mut Ui, w: PyWindow, t: &Tokens) {
    let pending = app.session.python_cell_pending(w.sheet, w.cell);
    let run = last_run(app, w);
    let Some(d) = app.session.active() else { return };
    let Some(sh) = d.wb.sheet(w.sheet) else { return };
    let value = sh.value(w.cell);
    let py = sh.cell(w.cell).and_then(|c| c.py.clone());
    let shown = gridcraft_engine::display::cell_text(&d.wb, sh, w.cell);
    let mut goto_line: Option<u32> = None;
    let mut show_table = false;
    egui::ScrollArea::vertical().id_salt("python_editor.output.scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if pending {
                ui.add(egui::Spinner::new().size(12.0).color(t.text_dim));
                ui.label(RichText::new("Running\u{2026}").color(t.text_dim).font(theme::ui_bold(12.0)));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            } else if let Some(r) = &run {
                let (status, color) = crate::python_ui::status_label(r.status, t);
                ui.label(RichText::new(status).color(color).font(theme::ui_bold(12.0)));
                let age = crate::python_ui::ago(gridcraft_engine::clock::now_ms() - r.time_ms);
                ui.label(RichText::new(format!("{:.1} ms, {age}", r.ms)).color(t.text_dim).font(theme::ui_font(12.0)));
            } else {
                ui.label(
                    RichText::new("Not run yet. Ctrl+Enter (Cmd+Enter on macOS) commits the code and runs it.")
                        .color(t.text_dim)
                        .font(theme::ui_font(12.0)),
                );
            }
        });
        // The result: an object's label (with its table), or the value.
        let has_result =
            !pending && !matches!(value, Value::Error(_)) && (py.as_ref().is_some_and(|p| p.output.is_some()) || !matches!(value, Value::Empty));
        if has_result {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Result").color(t.text_dim).font(theme::ui_font(12.0)));
                let label = py
                    .as_ref()
                    .and_then(|p| p.output.as_ref())
                    .map(|o| if o.label.is_empty() { o.type_name.clone() } else { o.label.clone() })
                    .filter(|l| !l.is_empty());
                let text = match label {
                    Some(l) if py.as_ref().is_some_and(|p| p.object) => l,
                    // Spilled values: the object they came from ("DataFrame 6×4").
                    Some(l) if py.as_ref().is_some_and(|p| p.table().is_some()) => format!("{shown}  ({l})"),
                    Some(_) => shown.clone(),
                    None => shown.clone(),
                };
                ui.add(egui::Label::new(RichText::new(text).font(code_editor::font()).color(t.text)).truncate());
                if py.as_ref().is_some_and(|p| p.table().is_some())
                    && ui.button("Show table").on_hover_text("Show the result as a table, beside the cell").clicked()
                {
                    show_table = true;
                }
            });
        }
        if let Some(r) = &run {
            // (A stop's error, "Stopped", only repeats its status.)
            if let Some(e) = r.error.as_ref().filter(|e| e.as_str() != crate::python_ui::status_label(r.status, t).0) {
                match r.line {
                    Some(l) => {
                        let link = ui.add(egui::Link::new(RichText::new(format!("Line {l}: {e}")).color(t.danger).font(theme::ui_font(12.0))));
                        if link.on_hover_text("Go to the line").clicked() {
                            goto_line = Some(l);
                        }
                    }
                    None => {
                        ui.add(egui::Label::new(RichText::new(e).color(t.danger).font(theme::ui_font(12.0))).wrap());
                    }
                }
            }
            for (label, out) in [("Output", &r.stdout), ("Errors", &r.stderr)] {
                if !out.trim().is_empty() {
                    ui.label(RichText::new(label).color(t.text_dim).font(theme::ui_font(11.5)));
                    ui.add(egui::Label::new(RichText::new(out.trim_end()).font(code_editor::font()).color(t.text)).wrap());
                }
            }
        }
    });
    if show_table && app.session.active().is_some_and(|d| d.wb.active_sheet == w.sheet) {
        app.grid.py_card = Some((w.sheet, w.cell));
    }
    if let Some(line) = goto_line {
        go_to_line(app, w, line);
    }
}

/// Puts the caret at the start of line `line` (1-based) of the code, editing it if needed.
pub fn go_to_line(app: &mut SheetApp, w: PyWindow, line: u32) {
    if app.editor.is_none() && active_cell(app) == Some((w.sheet, w.cell)) {
        app.begin_python_edit();
    }
    let at = app.editor.as_ref().map(|ed| code_editor::line_start_char(&ed.text, line)).unwrap_or(0);
    edit_at(app, w, at);
}
