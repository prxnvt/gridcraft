//! The formula bar (Name Box, ✕ ✓ fx, a badge slot, formula field) and the shared cell editor widget.

use egui::{FontId, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, TextEdit, pos2, vec2};
use serde_json::json;

use crate::SheetApp;
use crate::code_editor::{self, Lang};
use crate::editor::{formula_refs, python_refs};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// Lines the formula bar (and the in-cell editor) grows to before it scrolls.
pub const MAX_LINES: usize = 5;
/// The bar's height at one line.
const ONE_LINE: f32 = 30.0;
/// How long the bar takes to grow or shrink to a new line count.
const EASE_SECONDS: f32 = 0.12;

/// A badge in the formula bar's badge slot, beside fx (the PY badge in Python mode).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormulaBadge {
    pub label: String,
    pub tip: String,
    /// Set when the badge is clicked; whoever shows the badge clears it.
    pub clicked: bool,
}

/// Where the formula bar put its parts last frame (for tests and the control channel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarLayout {
    pub name_box: Rect,
    /// ✕ ✓ fx and the badge slot.
    pub buttons: Rect,
    pub field: Rect,
    pub height: f32,
    /// Laid-out lines of the content (wrapped to the field), and how many show without scrolling.
    pub lines: usize,
    pub visible_lines: usize,
    /// The caret while editing in the bar.
    pub caret: Option<Rect>,
}

impl Default for BarLayout {
    fn default() -> Self {
        BarLayout { name_box: Rect::NOTHING, buttons: Rect::NOTHING, field: Rect::NOTHING, height: 0.0, lines: 1, visible_lines: 1, caret: None }
    }
}

impl BarLayout {
    pub fn id() -> egui::Id {
        egui::Id::new("formula_bar.layout")
    }

    /// Last frame's layout.
    pub fn get(ctx: &egui::Context) -> BarLayout {
        ctx.data(|d| d.get_temp(Self::id())).unwrap_or_default()
    }
}

/// What the editor widget laid out: its line count and caret.
#[derive(Clone, Copy, Debug, Default)]
pub struct EditorInfo {
    pub lines: usize,
    pub caret: Option<Rect>,
}

pub fn show(app: &mut SheetApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let font = theme::ui_font(13.0);
    let row_h = ui.ctx().fonts_mut(|f| f.row_height(&font));
    let last = BarLayout::get(ui.ctx());
    // Height follows the content's line count (last frame's), 1 to MAX_LINES; the chevron pins it
    // at MAX_LINES. It eases only when that target changes.
    let lines = if app.ui.formula_bar_expanded { MAX_LINES } else { last.lines.clamp(1, MAX_LINES) };
    let target = ONE_LINE + (lines - 1) as f32 * row_h;
    let h = ui.ctx().animate_value_with_time(egui::Id::new("formula_bar.height"), target, EASE_SECONDS);
    let mut layout = BarLayout { height: h, ..Default::default() };
    egui::Panel::top("formula_bar")
        .exact_size(h)
        .frame(egui::Frame::NONE.fill(t.window).inner_margin(egui::Margin { left: 8, right: 8, top: 3, bottom: 3 }))
        .show(ui, |ui| {
            // Top-aligned: the Name Box and buttons stay on the first line as the field grows.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                layout.name_box = name_box(app, ui, &t);
                ui.add_space(4.0);
                let editing = app.editor.is_some();
                let c = if editing { t.text } else { t.text_disabled };
                let cancel = crate::widgets::icon_button(ui, Icon::Close, c, "Cancel", vec2(22.0, 24.0));
                if cancel.clicked() && editing {
                    app.cancel_edit();
                }
                if crate::widgets::icon_button(ui, Icon::Check, c, "Enter", vec2(22.0, 24.0)).clicked() && editing {
                    app.commit_edit(0, 0, false, false);
                }
                let fx = crate::widgets::icon_button(ui, Icon::Fx, t.text, "Insert Function", vec2(26.0, 24.0));
                if fx.clicked() {
                    app.open_dialog("insertFunction", json!({}));
                }
                let mut buttons = cancel.rect.union(fx.rect);
                python_badge(app);
                if let Some(rect) = badge(app, ui, &t) {
                    buttons = buttons.union(rect);
                }
                if let Some(rect) = run_button(app, ui, &t) {
                    buttons = buttons.union(rect);
                }
                if app.formula_badge.as_ref().is_some_and(|b| b.clicked) {
                    if let Some(b) = app.formula_badge.as_mut() {
                        b.clicked = false;
                    }
                    app.leave_python();
                }
                layout.buttons = buttons;
                ui.add_space(4.0);
                // The formula field fills the rest, as tall as the bar.
                let avail = ui.available_width() - 24.0;
                let field = Rect::from_min_size(ui.cursor().min, vec2(avail, ui.available_height()));
                ui.painter().rect_filled(field, 4.0, t.input_bg);
                ui.painter().rect_stroke(field, 4.0, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
                let inner = field.shrink2(vec2(6.0, 3.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                let id = egui::Id::new("gridcraft.formula_bar");
                let info = if app.editor.is_some() {
                    editor_widget(app, &mut child, id, font.clone(), true, inner.width(), inner.height())
                } else {
                    // Show the active cell's content (a Python cell's code in colour); clicking
                    // starts editing in the bar.
                    let text = active_input(app);
                    let galley = if active_python(app) {
                        let job = code_editor::layout_job(&text, Lang::Python, &code_editor::font(), t.text, &t.syntax, inner.width());
                        child.fonts_mut(|f| f.layout_job(job))
                    } else {
                        child.painter().layout(text, font.clone(), t.text, inner.width())
                    };
                    let lines = galley.rows.len();
                    egui::ScrollArea::vertical().id_salt(id.with("view")).max_height(inner.height()).auto_shrink([false, true]).show(
                        &mut child,
                        |ui| {
                            if ui.add(egui::Label::new(galley).sense(Sense::click())).clicked() {
                                app.begin_edit(None, true);
                            }
                        },
                    );
                    if ui.interact(field, id.with("click"), Sense::click()).clicked() {
                        app.begin_edit(None, true);
                    }
                    EditorInfo { lines, caret: None }
                };
                layout.field = field;
                layout.lines = info.lines.max(1);
                layout.visible_lines = ((inner.height() / row_h).floor() as usize).max(1);
                layout.caret = info.caret;
                ui.allocate_space(vec2(avail, field.height()));
                let chevron = if app.ui.formula_bar_expanded { Icon::ChevronUp } else { Icon::Chevron };
                if crate::widgets::icon_button(ui, chevron, t.text_dim, "Expand formula bar", vec2(20.0, 24.0)).clicked() {
                    app.ui.formula_bar_expanded = !app.ui.formula_bar_expanded;
                }
            });
        });
    ui.ctx().data_mut(|d| d.insert_temp(BarLayout::id(), layout));
}

/// Whether the active cell is a Python cell.
fn active_python(app: &SheetApp) -> bool {
    app.session.active().and_then(|d| d.wb.active().and_then(|sh| sh.cell(d.selection.active))).is_some_and(|c| c.py.is_some())
}

/// The PY badge shows while editing in Python mode, and on a selected Python cell.
fn python_badge(app: &mut SheetApp) {
    let show = match &app.editor {
        Some(ed) => ed.python,
        None => active_python(app),
    };
    if !show {
        app.formula_badge = None;
    } else if app.formula_badge.as_ref().is_none_or(|b| b.label != "PY") {
        app.formula_badge = Some(FormulaBadge {
            label: "PY".into(),
            tip: "Python cell. Click to turn Python off for this cell (Enter then makes it a normal cell; undo brings the code back)".into(),
            clicked: false,
        });
    }
}

/// What the formula bar's Python button does now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunButton {
    /// Run (a play triangle): the cell hasn't run, is out of date, or its code is being edited;
    /// in Python edit mode it commits the code first. Disabled while Python is turned off for
    /// the workbook.
    Run { enabled: bool },
    /// Stop (a square): the cell, or the Python cells it waits on, run or wait to run.
    Stop,
    /// Re-run (a circular arrow): the cell has a result or an error.
    Rerun,
}

impl RunButton {
    pub fn tooltip(self) -> &'static str {
        match self {
            RunButton::Run { enabled: true } => "Run Python cell",
            RunButton::Run { enabled: false } => {
                "Run Python cell: Python is turned off for this workbook. Choose Enable Python in the message bar to run it"
            }
            RunButton::Stop => "Stop Python",
            RunButton::Rerun => "Re-run Python cell",
        }
    }
}

/// The Python button's state for the active cell, or `None` when it isn't a Python cell and the
/// formula bar isn't in Python mode.
pub fn run_button_state(app: &SheetApp) -> Option<RunButton> {
    let d = app.session.active()?;
    let (sheet, at, editing) = match &app.editor {
        Some(ed) if ed.python => (ed.sheet, ed.cell, Some(ed.text.as_str())),
        Some(_) => return None,
        None => (d.wb.active_sheet, d.selection.active, None),
    };
    let cell = d.wb.sheet(sheet).and_then(|s| s.cell(at));
    let py = cell.and_then(|c| c.py.as_deref());
    if editing.is_none() && py.is_none() {
        return None;
    }
    if !d.trust.allows_code() {
        return Some(RunButton::Run { enabled: false });
    }
    if app.session.python_cell_pending(sheet, at) {
        return Some(RunButton::Stop);
    }
    let Some(py) = py else { return Some(RunButton::Run { enabled: true }) };
    if editing.is_some_and(|text| gridcraft_engine::model::python::normalize_code(text) != py.code) || py.stale {
        return Some(RunButton::Run { enabled: true });
    }
    // A result, or an error (a #BUSY! left by a stop is an error too).
    let ran = py.output.is_some() || matches!(cell.map(|c| &c.value), Some(gridcraft_engine::core::Value::Error(_)));
    Some(if ran { RunButton::Rerun } else { RunButton::Run { enabled: true } })
}

/// The Python button beside the PY badge (see [`RunButton`]); returns its rect when shown.
fn run_button(app: &mut SheetApp, ui: &mut egui::Ui, t: &Tokens) -> Option<Rect> {
    let state = run_button_state(app)?;
    let (icon, color) = match state {
        RunButton::Run { enabled: true } => (Icon::RunCell, t.cat(theme::Cat::Green)),
        RunButton::Run { enabled: false } => (Icon::RunCell, t.text_disabled),
        RunButton::Stop => (Icon::StopCell, t.danger),
        RunButton::Rerun => (Icon::Rerun, t.text),
    };
    let enabled = state != RunButton::Run { enabled: false };
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 4.0, t.pressed);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, color);
    let label = state.tooltip();
    let short = match state {
        RunButton::Run { .. } => "Run Python cell",
        RunButton::Stop => "Stop Python",
        RunButton::Rerun => "Re-run Python cell",
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, short));
    let resp = resp.on_hover_text(label);
    if enabled && resp.clicked() {
        press_run_button(app, state);
    }
    Some(rect)
}

/// What the Python button does: Run commits the code being edited (as Ctrl+Enter does) and runs
/// the cell; Stop stops Python; Re-run runs the cell again.
pub fn press_run_button(app: &mut SheetApp, state: RunButton) {
    match state {
        RunButton::Run { enabled: false } => {}
        RunButton::Stop => app.run_or_alert("python.stop", json!({})),
        RunButton::Run { enabled: true } | RunButton::Rerun => {
            let target = match &app.editor {
                Some(ed) if ed.python => Some((ed.sheet, ed.cell)),
                Some(_) => None,
                None => app.session.active().map(|d| (d.wb.active_sheet, d.selection.active)),
            };
            let Some((sheet, cell)) = target else { return };
            if app.editor.is_some() && !app.commit_edit(0, 0, false, false) {
                return;
            }
            // New or changed code already runs from the commit; unchanged code runs here.
            if app.session.python_cell_pending(sheet, cell) {
                return;
            }
            let name = app.session.active().and_then(|d| d.wb.sheet(sheet)).map(|s| s.name.clone()).unwrap_or_default();
            app.run_or_alert("python.run", json!({"cell": cell.a1(), "sheet": name}));
        }
    }
}

/// Ctrl+Alt+Shift+P (also with Cmd on macOS), taken from this frame's input: Python mode on or off.
pub fn take_python_shortcut(i: &mut egui::InputState) -> bool {
    let hit = |e: &egui::Event| matches!(e, egui::Event::Key { key: Key::P, pressed: true, modifiers: m, .. } if m.shift && m.alt && (m.ctrl || m.mac_cmd || m.command));
    let found = i.events.iter().any(hit);
    if found {
        // The key, and the character Option+Shift+P types on macOS.
        i.events.retain(|e| !hit(e) && !matches!(e, egui::Event::Text(t) if t == "∏"));
    }
    found
}

/// Ctrl+Alt+Shift+M (also with Cmd on macOS), taken from this frame's input: switch the active
/// Python cell between a Python object and Excel values.
pub fn take_output_shortcut(i: &mut egui::InputState) -> bool {
    let hit = |e: &egui::Event| matches!(e, egui::Event::Key { key: Key::M, pressed: true, modifiers: m, .. } if m.shift && m.alt && (m.ctrl || m.mac_cmd || m.command));
    let found = i.events.iter().any(hit);
    if found {
        // The key, and the character Option+Shift+M types on macOS.
        i.events.retain(|e| !hit(e) && !matches!(e, egui::Event::Text(t) if t == "\u{c2}"));
    }
    found
}

/// The PY badge's menu (the arrow beside it, or a right-click on it): the active Python cell's
/// output, as a Python object or as Excel values.
fn output_menu(app: &mut SheetApp, ui: &mut egui::Ui, t: &Tokens, badge: &egui::Response) {
    let Some(object) = app.session.active().and_then(|d| d.wb.active()?.cell(d.selection.active)?.py.as_ref().map(|p| p.object)) else { return };
    let arrow = crate::widgets::icon_button(ui, Icon::Chevron, t.text_dim, "Python Output (Ctrl+Alt+Shift+M)", vec2(14.0, 24.0));
    let mut pick: Option<bool> = None;
    for anchor in [&arrow, badge] {
        let menu = if std::ptr::eq(anchor, &arrow) { egui::Popup::menu(anchor) } else { egui::Popup::context_menu(anchor) };
        menu.show(|ui| {
            ui.label(egui::RichText::new("Python Output").color(t.text_dim).font(theme::ui_font(11.5)));
            for (label, as_object) in [("Python Object", true), ("Excel Values", false)] {
                let mark = if object == as_object { "• " } else { "   " };
                if ui.add(egui::Button::new(format!("{mark}{label}")).frame(false).min_size(vec2(160.0, 20.0))).clicked() {
                    pick = Some(as_object);
                }
            }
        });
    }
    if let Some(as_object) = pick
        && as_object != object
    {
        app.toggle_python_output();
    }
}

/// The badge slot: paints `app.formula_badge`, if any, and returns its rect.
fn badge(app: &mut SheetApp, ui: &mut egui::Ui, t: &Tokens) -> Option<Rect> {
    let (rect, resp) = badge_pill(app, ui, t)?;
    output_menu(app, ui, t, &resp);
    Some(rect)
}

fn badge_pill(app: &mut SheetApp, ui: &mut egui::Ui, t: &Tokens) -> Option<(Rect, egui::Response)> {
    let b = app.formula_badge.as_mut()?;
    let font = theme::ui_bold(11.0);
    let text = ui.painter().layout_no_wrap(b.label.clone(), font, t.on_badge);
    let (rect, resp) = ui.allocate_exact_size(vec2(text.size().x + 12.0, 24.0), Sense::click());
    let pill = Rect::from_center_size(rect.center(), vec2(rect.width(), 18.0));
    ui.painter().rect_filled(pill, 9.0, t.badge_py);
    ui.painter().galley(pill.center() - text.size() / 2.0, text, t.on_badge);
    let label = b.label.clone();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let resp = resp.on_hover_text(b.tip.clone());
    if resp.clicked() {
        b.clicked = true;
    }
    Some((rect, resp))
}

fn active_input(app: &SheetApp) -> String {
    let Some(d) = app.session.active() else { return String::new() };
    let Some(sh) = d.wb.active() else { return String::new() };
    let a = sh.merge_at(d.selection.active).map(|m| m.start).unwrap_or(d.selection.active);
    if let Some(picture) = sh.cell_pictures.get(&a) {
        return if picture.alt.is_empty() { "Picture in cell".into() } else { format!("Picture: {}", picture.alt) };
    }
    if let Some(t) = sh.data_table_at(a) {
        return format!("{{={}}}", t.formula_text());
    }
    if let Some(c) = sh.cell(a) {
        if c.formula.is_some() && d.wb.styles.get(c.style).protection.hidden && sh.is_protected() {
            return String::new();
        }
        return if let Some(f) = &c.formula {
            let shown = crate::formula_locale::display(&c.input_text(), app.ui.language.formula_locale(), &d.wb, d.wb.active_sheet);
            // A legacy array formula shows in braces (`{=A1:A2*2}`), as `Cell::bar_text` does.
            if f.array.is_some() { format!("{{{shown}}}") } else { shown }
        } else {
            c.input_text()
        };
    }
    // A spilled cell shows the anchor's formula greyed out in Excel; we show it plainly.
    for (anchor, r) in &sh.spill_ranges {
        if r.contains(a)
            && let Some(f) = sh.cell(*anchor).and_then(|c| c.formula.as_ref())
        {
            if gridcraft_engine::display::formula_hidden(&d.wb, sh, *anchor) {
                return String::new();
            }
            return crate::formula_locale::display(&format!("={}", f.text), app.ui.language.formula_locale(), &d.wb, d.wb.active_sheet);
        }
    }
    String::new()
}

fn name_box(app: &mut SheetApp, ui: &mut egui::Ui, t: &Tokens) -> Rect {
    let id = egui::Id::new("gridcraft.name_box");
    let shown = app.name_box_text();
    let mut text = app.name_box.clone().unwrap_or(shown.clone());
    let rect = Rect::from_min_size(ui.cursor().min, vec2(118.0, 24.0));
    ui.painter().rect_filled(rect, 4.0, t.input_bg);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.min + vec2(6.0, 3.0), vec2(96.0, 18.0))));
    let resp = child.add(TextEdit::singleline(&mut text).id(id).frame(egui::Frame::NONE).font(theme::ui_font(13.0)).desired_width(96.0));
    if resp.gained_focus() {
        app.name_box = Some(shown.clone());
    }
    if resp.changed() {
        app.name_box = Some(text.clone());
    }
    if resp.lost_focus() {
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            let target = text.trim().to_string();
            if !target.is_empty() && target != shown {
                // A new name for the selection, or a place to go.
                let is_ref = gridcraft_engine::core::RangeRef::parse(target.split('!').next_back().unwrap_or("")).is_some();
                let known = app.session.active().is_some_and(|d| d.wb.name(&target, d.wb.active_sheet).is_some() || d.wb.table(&target).is_some());
                let r = if is_ref || known {
                    app.run("edit.goTo", json!({"reference": target}))
                } else {
                    app.run("formulas.defineName", json!({"name": target}))
                };
                if let Err(e) = r {
                    app.message = Some(("GridCraft".into(), crate::clean_error(&e)));
                }
                app.grid.ensure_visible = true;
            }
        }
        app.name_box = None;
    }
    ui.allocate_space(vec2(118.0, 24.0));
    // Dropdown of names.
    let names: Vec<String> =
        app.session.active().map(|d| d.wb.names.iter().filter(|n| !n.hidden).map(|n| n.name.clone()).collect()).unwrap_or_default();
    let mb = Rect::from_min_size(pos2(rect.right() - 16.0, rect.top() + 4.0), vec2(14.0, 16.0));
    icons::paint(ui.painter(), mb, Icon::Chevron, t.text_dim);
    let r = ui.interact(mb, id.with("drop"), Sense::click());
    egui::Popup::menu(&r).show(|ui| {
        if names.is_empty() {
            ui.label(egui::RichText::new("No names defined").italics());
        }
        for n in names {
            if ui.button(&n).clicked() {
                let _ = app.run("edit.goTo", json!({"reference": n}));
                app.grid.ensure_visible = true;
            }
        }
    });
    rect
}

/// The text editor used in the cell and in the formula bar (they share `app.editor`). It shows
/// `max_height` of text and scrolls beyond that, keeping the caret in view.
pub fn editor_widget(app: &mut SheetApp, ui: &mut egui::Ui, id: egui::Id, font: FontId, multiline: bool, width: f32, max_height: f32) -> EditorInfo {
    if app.editor.as_ref().is_some_and(|e| e.python) {
        return python_editor(app, ui, id, width, max_height);
    }
    let Some(mut ed) = app.editor.take() else { return EditorInfo::default() };
    let mine = ed.from_formula_bar == (id == egui::Id::new("gridcraft.formula_bar"));
    // Keys the editor handles itself (before the TextEdit sees them), only where it has focus.
    let focused = ui.ctx().memory(|m| m.focused());
    let has_focus = focused == Some(id) || (mine && (ed.request_focus || focused.is_none()));
    let mut action: Option<(i64, i64, bool, bool)> = None;
    let mut cancel = false;
    let mut to_python = false;
    if has_focus && mine {
        to_python = ui.input_mut(take_python_shortcut);
        let (enter, tab, esc, alt_enter, cmd_enter, ctrl_shift_enter, shift) = ui.input_mut(|i| {
            let m = i.modifiers;
            let alt_enter =
                i.consume_key(egui::Modifiers::ALT, Key::Enter) || i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::ALT, Key::Enter);
            let ctrl_shift_enter = i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, Key::Enter);
            let cmd_enter = i.consume_key(egui::Modifiers::COMMAND, Key::Enter);
            let enter = i.consume_key(egui::Modifiers::NONE, Key::Enter) || i.consume_key(egui::Modifiers::SHIFT, Key::Enter);
            let tab = i.consume_key(egui::Modifiers::NONE, Key::Tab) || i.consume_key(egui::Modifiers::SHIFT, Key::Tab);
            let esc = i.consume_key(egui::Modifiers::NONE, Key::Escape);
            (enter, tab, esc, alt_enter, cmd_enter, ctrl_shift_enter, m.shift)
        });
        if alt_enter {
            let b = ed.text.char_indices().nth(ed.caret).map(|(i, _)| i).unwrap_or(ed.text.len());
            ed.text.insert(b, '\n');
            ed.caret += 1;
            ed.caret_to_end = false;
            ed.sync_caret = true;
        }
        if !ed.autocomplete.is_empty() && tab {
            ed.accept_autocomplete();
            // `=PY` and Tab (through AutoComplete) start Python mode.
            to_python |= ed.is_py_trigger();
        } else if enter {
            action = Some((if shift { -1 } else { 1 }, 0, false, false));
        } else if tab {
            action = Some((0, if shift { -1 } else { 1 }, false, false));
        }
        if cmd_enter {
            action = Some((0, 0, false, true));
        }
        if ctrl_shift_enter {
            action = Some((0, 0, true, false));
        }
        if esc {
            if !ed.autocomplete.is_empty() {
                ed.autocomplete.clear();
            } else {
                cancel = true;
            }
        }
        // Arrow keys: autocomplete navigation, point mode, or (Enter mode) commit and move.
        let arrows = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
                i.modifiers.shift,
            )
        });
        let (up, down, left, right, ashift) = arrows;
        if (up || down) && !ed.autocomplete.is_empty() {
            ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp);
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown);
            });
            let n = ed.autocomplete.len();
            ed.ac_index = if down { (ed.ac_index + 1) % n } else { (ed.ac_index + n - 1) % n };
        } else if ed.enter_mode && (up || down || left || right) && !ed.from_formula_bar {
            let (dr, dc) = if up {
                (-1, 0)
            } else if down {
                (1, 0)
            } else if left {
                (0, -1)
            } else {
                (0, 1)
            };
            if ed.can_point() {
                ui.input_mut(|i| {
                    for k in [Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight] {
                        i.consume_key(egui::Modifiers::NONE, k);
                        i.consume_key(egui::Modifiers::SHIFT, k);
                    }
                });
                crate::grid::point_move(app, &mut ed, dr, dc, ashift);
            } else {
                action = Some((dr, dc, false, false));
            }
        }
        // F4 cycles $ anchors of the reference before the caret.
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::F4)) {
            cycle_anchor(&mut ed);
        }
    }
    // The text widget.
    let refs = formula_refs(&ed.text);
    let t = Tokens::get(ui.ctx());
    let on_sheet = id == egui::Id::new("gridcraft.cell_editor");
    let cell_text_color = if on_sheet { t.cell_text } else { t.text };
    let font2 = font.clone();
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let s = buf.as_str();
        let mut job = egui::text::LayoutJob::default();
        let mut pos = 0usize;
        let mut spans: Vec<(usize, usize, usize)> = refs.iter().filter(|r| r.1 <= s.len()).map(|r| (r.0, r.1, r.4)).collect();
        spans.sort();
        for (a, b, idx) in spans {
            if a < pos || !s.is_char_boundary(a) || !s.is_char_boundary(b) {
                continue;
            }
            job.append(s.get(pos..a).unwrap_or(""), 0.0, egui::TextFormat::simple(font2.clone(), cell_text_color));
            job.append(s.get(a..b).unwrap_or(""), 0.0, egui::TextFormat::simple(font2.clone(), t.ref_color(idx, on_sheet)));
            pos = b;
        }
        job.append(s.get(pos..).unwrap_or(""), 0.0, egui::TextFormat::simple(font2.clone(), cell_text_color));
        job.wrap.max_width = if multiline { wrap_width } else { f32::INFINITY };
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let synced = mine && (ed.request_focus || ed.sync_caret);
    let edit = if multiline || ed.text.contains('\n') { TextEdit::multiline(&mut ed.text) } else { TextEdit::singleline(&mut ed.text) };
    let caret_index = ed.caret;
    let scrolled =
        egui::ScrollArea::vertical().id_salt(id.with("scroll")).max_height(max_height.max(1.0)).auto_shrink([false, true]).show(ui, |ui| {
            let output = edit
                .id(id)
                .frame(egui::Frame::NONE)
                .font(font)
                .desired_width(width)
                .desired_rows(1)
                .layouter(&mut layouter)
                .lock_focus(true)
                .event_filter(egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
                .show(ui);
            // A caret moved by our own keys (Alt+Enter, F4, point mode) isn't the TextEdit's doing,
            // so it doesn't scroll to it itself.
            if synced {
                let caret = output.galley.pos_from_cursor(egui::text::CCursor::new(caret_index)).translate(output.galley_pos.to_vec2());
                ui.scroll_to_rect(caret.expand(2.0), None);
            }
            output
        });
    let mut output = scrolled.inner;
    let resp = output.response.clone();
    if synced {
        if ed.request_focus {
            resp.request_focus();
        }
        ed.request_focus = false;
        ed.sync_caret = false;
        let cursor = egui::text::CCursor::new(ed.caret);
        output.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(cursor)));
        output.state.store(ui.ctx(), id);
    } else if resp.clicked() || resp.gained_focus() {
        // Clicking into the other editor moves editing there.
        ed.from_formula_bar = id == egui::Id::new("gridcraft.formula_bar");
        ed.enter_mode = false;
        app.session.mode = gridcraft_engine::Mode::Edit;
    }
    if !synced && let Some(r) = output.cursor_range {
        ed.caret = r.primary.index.into();
    }
    if resp.changed() {
        ed.completion = None;
        if !ed.is_formula()
            && ed.caret == ed.text.chars().count()
            && let Some(sh) = app.session.active().and_then(|d| d.wb.sheet(ed.sheet))
        {
            ed.completion = crate::editor::column_completion(sh, ed.cell, &ed.text);
        }
        ed.point = None;
        let names: Vec<String> = function_names()
            .iter()
            .map(|name| {
                app.session.active().map(|d| crate::formula_locale::completion_name(name, ed.locale, &d.wb, ed.sheet)).unwrap_or(name).to_string()
            })
            .collect();
        ed.update_autocomplete(&names);
        app.session.mode = if ed.can_point() {
            gridcraft_engine::Mode::Point
        } else if ed.enter_mode {
            gridcraft_engine::Mode::Enter
        } else {
            gridcraft_engine::Mode::Edit
        };
    }
    // Column AutoComplete: the rest of the suggested entry, shown selected after the caret.
    if let Some(full) = &ed.completion
        && full.len() > ed.text.len()
        && let Some(rest) = full.get(ed.text.len()..)
    {
        let pos = resp.rect.left_top() + egui::vec2(output.galley.rect.width(), 0.0);
        let g = ui.painter().layout_no_wrap(
            rest.to_string(),
            output.galley.job.sections.first().map(|s| s.format.font_id.clone()).unwrap_or_default(),
            t.on_accent,
        );
        let r = egui::Rect::from_min_size(pos, g.size());
        ui.painter().rect_filled(r, 0.0, t.accent);
        ui.painter().galley(pos, g, t.on_accent);
    }
    // Autocomplete list and argument hint under the editor.
    if mine && (!ed.autocomplete.is_empty() || ed.current_function().is_some()) {
        let below = resp.rect.left_bottom() + vec2(0.0, 4.0);
        egui::Area::new(id.with("ac")).fixed_pos(below).order(egui::Order::Tooltip).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(4.0).show(ui, |ui| {
                if !ed.autocomplete.is_empty() {
                    let mut pick = None;
                    for (i, n) in ed.autocomplete.iter().enumerate() {
                        let sel = i == ed.ac_index;
                        let r = ui.add(
                            egui::Button::selectable(sel, egui::RichText::new(format!("ƒ  {n}")).font(theme::ui_font(12.5)))
                                .min_size(vec2(220.0, 20.0)),
                        );
                        if r.clicked() {
                            pick = Some(i);
                        }
                        if sel && let Some(desc) = crate::formula_locale::description(n, ed.locale) {
                            r.on_hover_text(desc);
                        }
                    }
                    if let Some(desc) = ed.autocomplete.get(ed.ac_index).and_then(|n| crate::formula_locale::description(n, ed.locale)) {
                        ui.separator();
                        ui.add(egui::Label::new(egui::RichText::new(desc).small()).wrap());
                    }
                    if let Some(i) = pick {
                        ed.ac_index = i;
                        ed.accept_autocomplete();
                        ed.request_focus = true;
                    }
                } else if let Some((name, arg)) = ed.current_function()
                    && let Some(sig) = crate::formula_locale::signature(&name, ed.locale)
                {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let (head, rest) = sig.split_once('(').unwrap_or((&sig, ""));
                        ui.label(egui::RichText::new(format!("{}(", ed.locale.function_name(head))).font(theme::ui_font(12.0)));
                        let inner = rest.trim_end_matches(')');
                        let parts: Vec<&str> = inner.split(&format!("{} ", ed.locale.list_separator())).collect();
                        for (i, part) in parts.iter().enumerate() {
                            let is_cur = i == arg.min(parts.len().saturating_sub(1)) || (part.contains("...") && arg >= i);
                            let txt = egui::RichText::new(*part).font(if is_cur { theme::ui_bold(12.0) } else { theme::ui_font(12.0) });
                            ui.label(txt);
                            if i + 1 < parts.len() {
                                ui.label(egui::RichText::new(format!("{} ", ed.locale.list_separator())).font(theme::ui_font(12.0)));
                            }
                        }
                        ui.label(egui::RichText::new(")").font(theme::ui_font(12.0)));
                    });
                }
            });
        });
    }
    let lines = output.galley.rows.len().max(1);
    let caret = Some(output.galley.pos_from_cursor(egui::text::CCursor::new(ed.caret)).translate(output.galley_pos.to_vec2()));
    if to_python {
        ed.enter_python();
        app.session.mode = gridcraft_engine::Mode::Edit;
        app.editor = Some(ed);
        return EditorInfo { lines, caret };
    }
    app.editor = Some(ed);
    if cancel {
        app.cancel_edit();
    } else if let Some((dr, dc, array, fill)) = action {
        app.commit_edit(dr, dc, array, fill);
    }
    EditorInfo { lines, caret }
}

/// The formula bar in Python mode: the code editor (monospace, Python colours, `xl("…")`
/// references in their reference colours). Its keys come before the formula editor's: Enter
/// starts a new line with the indentation, Tab and Shift+Tab indent, Ctrl+Enter (Cmd+Enter on
/// macOS) commits through `python.set`, Esc cancels, F2 toggles pointing at cells, and
/// Ctrl+Alt+Shift+P or Backspace in an empty editor leaves Python mode.
fn python_editor(app: &mut SheetApp, ui: &mut egui::Ui, id: egui::Id, width: f32, max_height: f32) -> EditorInfo {
    let Some(mut ed) = app.editor.take() else { return EditorInfo::default() };
    let focused = ui.ctx().memory(|m| m.focused());
    let has_focus = focused == Some(id) || ed.request_focus || focused.is_none();
    let (mut commit, mut cancel, mut leave) = (false, false, false);
    let mut edited: Option<code_editor::Edit> = None;
    if has_focus {
        let empty = ed.text.is_empty();
        let keys = ui.input_mut(|i| {
            let shortcut = take_python_shortcut(i);
            let output = take_output_shortcut(i);
            let commit = i.consume_key(Modifiers::COMMAND, Key::Enter) || i.consume_key(Modifiers::CTRL, Key::Enter);
            let enter = i.consume_key(Modifiers::NONE, Key::Enter);
            let shift_tab = i.consume_key(Modifiers::SHIFT, Key::Tab);
            let tab = i.consume_key(Modifiers::NONE, Key::Tab);
            let esc = i.consume_key(Modifiers::NONE, Key::Escape);
            let f2 = i.consume_key(Modifiers::NONE, Key::F2);
            // F4 (anchors) is a formula key; it does nothing to code.
            i.consume_key(Modifiers::NONE, Key::F4);
            let backspace = empty && i.consume_key(Modifiers::NONE, Key::Backspace);
            (shortcut, commit, enter, tab, shift_tab, esc, f2, backspace, output)
        });
        let (shortcut, commit_key, enter, tab, shift_tab, esc, f2, backspace, output) = keys;
        if output {
            app.toggle_python_output();
        }
        commit = commit_key;
        cancel = esc;
        leave = shortcut || backspace;
        if f2 {
            ed.py_point = !ed.py_point;
            ed.point = None;
            ed.point_cell = None;
        }
        if enter || tab || shift_tab {
            let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            let count = ed.text.chars().count();
            let sel = state
                .cursor
                .char_range()
                .map(|r| (r.secondary.index.into(), r.primary.index.into()))
                .unwrap_or((ed.caret.min(count), ed.caret.min(count)));
            let e = if enter { code_editor::newline(&mut ed.text, sel, Lang::Python) } else { code_editor::indent(&mut ed.text, sel, shift_tab) };
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(e.anchor), egui::text::CCursor::new(e.caret))));
            state.store(ui.ctx(), id);
            ed.caret = e.caret;
            ed.point = None;
            edited = Some(e);
        }
    }
    let t = Tokens::get(ui.ctx());
    let font = code_editor::font();
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let s = buf.as_str();
        let marks: Vec<(std::ops::Range<usize>, egui::Color32)> = python_refs(s).iter().map(|r| (r.0..r.1, t.ref_color(r.4, false))).collect();
        let job = code_editor::layout_job_marked(s, Lang::Python, &code_editor::font(), t.text, &t.syntax, &marks, wrap_width);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let synced = ed.request_focus || ed.sync_caret;
    let caret_index = ed.caret;
    let scroll_to = synced || edited.is_some();
    let scrolled =
        egui::ScrollArea::vertical().id_salt(id.with("scroll")).max_height(max_height.max(1.0)).auto_shrink([false, true]).show(ui, |ui| {
            let output = TextEdit::multiline(&mut ed.text)
                .id(id)
                .frame(egui::Frame::NONE)
                .font(font)
                .desired_width(width)
                .desired_rows(1)
                .layouter(&mut layouter)
                .lock_focus(true)
                .event_filter(egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
                .show(ui);
            // A caret moved by our own keys isn't the TextEdit's doing, so it doesn't scroll to it.
            if scroll_to {
                let caret = output.galley.pos_from_cursor(egui::text::CCursor::new(caret_index)).translate(output.galley_pos.to_vec2());
                ui.scroll_to_rect(caret.expand(2.0), None);
            }
            output
        });
    let mut output = scrolled.inner;
    let resp = output.response.clone();
    if synced {
        if ed.request_focus {
            resp.request_focus();
        }
        ed.request_focus = false;
        ed.sync_caret = false;
        let cursor = egui::text::CCursor::new(ed.caret);
        output.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(cursor)));
        output.state.store(ui.ctx(), id);
    } else if let Some(r) = output.cursor_range {
        ed.caret = r.primary.index.into();
    }
    if resp.changed() {
        // Typing ends the pointed reference: the next click inserts a new one.
        ed.point = None;
    }
    app.session.mode = if ed.py_point { gridcraft_engine::Mode::Point } else { gridcraft_engine::Mode::Edit };
    let lines = output.galley.rows.len().max(1);
    let caret = Some(output.galley.pos_from_cursor(egui::text::CCursor::new(ed.caret)).translate(output.galley_pos.to_vec2()));
    app.editor = Some(ed);
    if cancel {
        app.cancel_edit();
    } else if leave {
        app.leave_python();
    } else if commit {
        app.commit_edit(0, 0, false, false);
    }
    EditorInfo { lines, caret }
}

fn cycle_anchor(ed: &mut crate::editor::EditState) {
    let caret_b = ed.text.char_indices().nth(ed.caret).map(|(i, _)| i).unwrap_or(ed.text.len());
    for (a, b, ..) in formula_refs(&ed.text) {
        if a <= caret_b && caret_b <= b {
            let Some(r) = ed.text.get(a..b) else { return };
            let cycled: String = r
                .split(':')
                .map(|part| {
                    let p = part.split('!').next_back().unwrap_or(part);
                    let prefix = &part[..part.len() - p.len()];
                    let bare = p.replace('$', "");
                    let letters: String = bare.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                    let digits = &bare[letters.len()..];
                    let state = (p.starts_with('$'), p[letters.len().min(p.len())..].contains('$') || p.trim_start_matches('$').contains('$'));
                    let next = match state {
                        (false, false) => format!("${letters}${digits}"),
                        (true, true) => format!("{letters}${digits}"),
                        (false, true) => format!("${letters}{digits}"),
                        (true, false) => format!("{letters}{digits}"),
                    };
                    format!("{prefix}{next}")
                })
                .collect::<Vec<_>>()
                .join(":");
            let (Some(x), Some(y)) = (ed.text.get(..a), ed.text.get(b..)) else { return };
            ed.text = format!("{x}{cycled}{y}");
            ed.caret = ed.text.get(..a + cycled.len()).map(|s| s.chars().count()).unwrap_or(ed.caret);
            ed.caret_to_end = false;
            ed.request_focus = true;
            ed.sync_caret = true;
            return;
        }
    }
}

fn function_names() -> Vec<String> {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES
        .get_or_init(|| {
            let mut names: Vec<String> =
                gridcraft_engine::cmd::formulas::function_list().iter().filter_map(|f| f["name"].as_str().map(str::to_string)).collect();
            // PY isn't a worksheet function: picking it (=PY, Tab) starts a Python cell.
            if !names.iter().any(|n| n == "PY") {
                names.push("PY".into());
                names.sort();
            }
            names
        })
        .clone()
}

fn function_info(name: &str) -> Option<serde_json::Value> {
    static LIST: std::sync::OnceLock<Vec<serde_json::Value>> = std::sync::OnceLock::new();
    LIST.get_or_init(gridcraft_engine::cmd::formulas::function_list)
        .iter()
        .find(|f| f["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .cloned()
}

pub fn function_signature(name: &str) -> Option<String> {
    function_info(name).and_then(|f| f["signature"].as_str().map(str::to_string))
}

pub fn function_description(name: &str) -> Option<String> {
    function_info(name).and_then(|f| f["description"].as_str().map(str::to_string))
}
