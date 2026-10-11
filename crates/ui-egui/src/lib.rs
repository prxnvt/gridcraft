//! GridCraft's egui frontend: an Excel-style window (title bar with Quick Access Toolbar,
//! ribbon, formula bar, grid, sheet tabs, status bar) over `gridcraft-engine`.
//!
//! The UI is thin: it reads engine state and acts through [`SheetApp::run`] (engine commands by
//! id). Everything it can do is also reachable over the control channel ([`control`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod border_preview;
mod cell_pictures;
pub mod chartview;
pub mod code_editor;
pub mod control;
pub mod control_ui;
pub mod credits;
pub mod dialogs;
pub mod dnd;
pub mod editor;
pub mod equation;
pub mod form_controls;
pub mod formula_bar;
mod formula_locale;
pub mod function_library;
pub mod grid;
pub mod i18n;
pub mod icons;
pub mod keytips;
pub mod macro_editor;
pub mod macro_ui;
pub mod objects;
pub mod panes;
pub mod pivot_calc_ui;
pub mod pivot_filter;
pub mod pivot_pane;
pub mod python_ui;
pub mod ribbon;
pub mod solver_ui;
pub mod tabs;
pub mod text_box;
pub mod theme;
pub mod toolpak;
pub mod widgets;

use std::collections::HashMap;

use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::{Session, UiRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

pub use control::{ControlRequest, ControlResponse};

/// Persisted UI preferences.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    /// The manual preference, retained when following the system appearance.
    pub dark: bool,
    pub system_theme: bool,
    pub ribbon_tab: String,
    pub ribbon_collapsed: bool,
    pub formula_bar: bool,
    pub formula_bar_expanded: bool,
    pub status_bar: bool,
    pub recent: Vec<String>,
    /// Formulas › Recently Used and Insert Function's Most Recently Used category, newest first
    /// (at most [`function_library::RECENT_MAX`]).
    #[serde(deserialize_with = "function_library::lenient_recent")]
    pub recent_functions: Vec<String>,
    /// Interface language. English by default (so tests and headless renders never depend on the
    /// host's locale); the desktop app starts from [`i18n::Language::system`] when `ui.json` has
    /// no usable language (see [`i18n::saved_language`]). An unknown saved value reads as English
    /// rather than failing the whole `UiState`.
    #[serde(deserialize_with = "i18n::lenient")]
    pub language: i18n::Language,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            dark: false,
            system_theme: false,
            ribbon_tab: "Home".into(),
            ribbon_collapsed: false,
            formula_bar: true,
            formula_bar_expanded: false,
            status_bar: true,
            recent: vec![],
            recent_functions: function_library::default_recent(),
            language: i18n::Language::En,
        }
    }
}

impl UiState {
    pub fn theme_preference(&self) -> egui::ThemePreference {
        if self.system_theme {
            egui::ThemePreference::System
        } else if self.dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        }
    }

    pub fn theme_mode(&self) -> &'static str {
        if self.system_theme {
            "system"
        } else if self.dark {
            "dark"
        } else {
            "light"
        }
    }
}

/// Per-sheet view state (scroll position in sheet points).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SheetView {
    pub scroll: egui::Vec2,
}

/// Host services the platform shell provides (file pickers, opening URLs).
#[derive(Default)]
pub struct Services {
    pub pick_open: Option<Box<dyn Fn() -> Option<String>>>,
    pub pick_save: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// Native picker for a program (Python Options' interpreter).
    pub pick_program: Option<Box<dyn Fn() -> Option<String>>>,
    /// Native picture picker (PNG/JPEG), separate from workbook opening.
    pub pick_picture: Option<Box<dyn Fn() -> Option<String>>>,
    /// Browser picture picker. Each dialog owns its inbox, so canceled picks cannot
    /// insert into a subsequent dialog or open the image as a workbook.
    pub pick_picture_async: Option<Box<dyn Fn(Inbox)>>,
    pub open_url: Option<Box<dyn Fn(&str)>>,
    /// Publish HTML and its plain-text alternative together. `Ok` means the host owns the
    /// write (including any asynchronous fallback); an immediate error uses egui's text path.
    pub copy_html: Option<Box<dyn FnMut(&str, &str) -> Result<(), String>>>,
    /// Web: deliver a file to the user (name, bytes).
    pub download: Option<Box<dyn Fn(&str, &[u8])>>,
    /// Web: start an asynchronous file pick; the bytes arrive later in `inbox`.
    pub open_async: Option<Box<dyn Fn()>>,
    /// Files delivered asynchronously (name, bytes), opened on the next frame.
    pub inbox: Option<Inbox>,
}

/// Shared queue of files read asynchronously (browser file picker, drag and drop).
pub type Inbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

#[derive(Clone, Debug, Default)]
pub struct Perf {
    pub frame_ms: f64,
    pub grid_ms: f64,
    pub fps: f64,
    last: f64,
}

pub struct SheetApp {
    pub session: Session,
    pub ui: UiState,
    pub views: HashMap<(u64, usize), SheetView>,
    pub editor: Option<editor::EditState>,
    pub text_box_editor: Option<text_box::EditState>,
    pub dialog: Option<dialogs::Dialog>,
    pub message: Option<(String, String)>,
    pub toast: Option<(String, f64)>,
    /// How long the toast with the same time stays, and the button it offers (a pane to open).
    pub toast_extra: Option<ToastExtra>,
    /// Package installs whose outcome was already shown ([`Session::python_installs_ended`]).
    pub installs_shown: u64,
    pub services: Services,
    /// Synthetic input from agents, one batch per frame (drags need several frames).
    pub synthetic: std::collections::VecDeque<Vec<egui::Event>>,
    pub control_rx: Option<std::sync::mpsc::Receiver<ControlRequest>>,
    pub grid: grid::GridState,
    pub perf: Perf,
    pub fonts_ready: bool,
    fonts_set: bool,
    /// Han face order the installed fonts were built with; a language switch rebuilds the fonts
    /// only when it changes this (see `theme::HanOrder`).
    fonts_han: Option<theme::HanOrder>,
    effective_dark: bool,
    pub name_box: Option<String>,
    /// Transient ribbon keyboard navigation; never saved with UI preferences.
    pub keytips: keytips::KeyTips,
    /// The formula bar's badge slot (the PY badge in Python mode); empty otherwise.
    pub formula_badge: Option<formula_bar::FormulaBadge>,
    pub(crate) shots: control::Shots,
    /// The floating object selected on the sheet (chart, picture, shape, equation or control).
    pub selected_object: Option<gridcraft_engine::model::ObjRef>,
    pub started: f64,
    /// A command asked before replacing something; OK re-runs it with `overwrite: true`.
    pub confirm: Option<Confirm>,
    /// Dialog to open when the current one closes (Data Analysis → a tool's form).
    pub dialog_next: Option<dialogs::Dialog>,
    /// Automate › Macros: relative recording, recent macros.
    pub macros: macro_ui::MacroUi,
    /// How much of the session's journal Recently Used has read (see [`function_library::harvest`]).
    pub(crate) journal_seen: usize,
}

/// A pending "replace it?" question from a command (see [`confirm_text`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Confirm {
    pub title: String,
    pub text: String,
    pub command: String,
    pub params: Json,
    /// Buttons other than OK, each with the params it adds when it runs the command again
    /// (Overwrite, Shift Down, Shift Right for a macro). Empty: OK adds `overwrite: true`.
    pub choices: Vec<(String, Json)>,
}

/// What [`SheetApp::run_confirming`] did.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done(Json),
    /// The command asked a question (shown now; OK re-runs it).
    Asked,
    /// It failed; the error is shown.
    Failed,
}

/// The question in a command's "needs confirmation" error, if it is one.
pub fn confirm_text(e: &str) -> Option<String> {
    let rest = e.split_once("confirm: ")?.1;
    Some(rest.split_once(" (run again with").map(|(q, _)| q).unwrap_or(rest).to_string())
}

impl SheetApp {
    pub fn new(mut session: Session, services: Services) -> SheetApp {
        // Python cells run in the background, stepped every frame, so a slow cell never blocks
        // the window.
        session.set_python_background(true);
        let journal_seen = session.journal.len();
        SheetApp {
            session,
            ui: UiState::default(),
            views: HashMap::new(),
            editor: None,
            text_box_editor: None,
            dialog: None,
            message: None,
            toast: None,
            toast_extra: None,
            installs_shown: 0,
            services,
            synthetic: Default::default(),
            control_rx: None,
            grid: grid::GridState {
                last_border: "bottom".into(),
                last_fill: "#FFFF00".into(),
                last_font_color: "#C00000".into(),
                ..Default::default()
            },
            perf: Perf::default(),
            fonts_ready: false,
            fonts_set: false,
            fonts_han: None,
            effective_dark: false,
            name_box: None,
            keytips: keytips::KeyTips::default(),
            formula_badge: None,
            shots: control::Shots::default(),
            selected_object: None,
            started: now_ms(),
            confirm: None,
            dialog_next: None,
            macros: macro_ui::MacroUi::default(),
            journal_seen,
        }
    }

    /// One-time context setup: fonts and visuals (default Han order; never reads the host locale).
    pub fn setup_context(ctx: &egui::Context, dark: bool) {
        Self::setup_context_for_language(ctx, dark, i18n::Language::En);
    }

    /// Sets up fonts and visuals for a specific persisted interface language.
    pub fn setup_context_for_language(ctx: &egui::Context, dark: bool, language: i18n::Language) {
        ctx.set_fonts(theme::font_definitions_for_language(language));
        theme::apply(ctx, dark);
    }

    /// Runs an engine command (or a UI command) and handles UI requests it makes.
    pub fn run(&mut self, id: &str, params: Json) -> Result<Json, String> {
        if id.starts_with("file.")
            || matches!(
                id,
                "window.activate"
                    | "sheet.activate"
                    | "sheet.next"
                    | "sheet.previous"
                    | "sheet.move"
                    | "sheet.hide"
                    | "sheet.unhide"
                    | "view.newWindow"
                    | "view.hideWindow"
                    | "view.unhideWindow"
                    | "home.insertSheet"
                    | "home.deleteSheet"
                    | "edit.undo"
                    | "edit.redo"
                    | "object.delete"
                    | "object.move"
            )
        {
            self.commit_text_box_edit()?;
        }
        if let Some(r) = self.run_ui_command(id, &params) {
            return r;
        }
        let r = self.session.run(id, params);
        self.after_engine();
        if id == "file.exportPdf"
            && let Ok(r) = &r
        {
            self.deliver_pdf(r);
        }
        r
    }

    /// Copy or cut the selected cells to the system clipboard. Engine-only commands keep
    /// their existing internal clipboard behavior and do not write to the host clipboard.
    pub fn copy_to_clipboard(&mut self, ctx: &egui::Context, command: &str) {
        // Only ask the engine for HTML when the host can publish it.
        let Ok(result) = self.run(command, json!({"html": self.services.copy_html.is_some()})) else { return };
        let Some(text) = result.get("text").and_then(Json::as_str) else { return };
        if let Some(html) = result.get("html").and_then(Json::as_str)
            && let Some(copy_html) = &mut self.services.copy_html
        {
            match copy_html(html, text) {
                Ok(()) => return,
                Err(error) => log::warn!("HTML clipboard unavailable; copying plain text: {error}"),
            }
        }
        ctx.copy_text(text.to_string());
    }

    /// Web: `file.exportPdf` without a path returns the PDF as base64; hand it to the browser as a download.
    fn deliver_pdf(&self, result: &Json) {
        if result.get("path").is_some() {
            return;
        }
        if let Some(dl) = &self.services.download
            && let Some(b) = result.get("base64").and_then(Json::as_str).and_then(gridcraft_engine::io::base64_decode)
        {
            let title = self.session.active().map(|d| d.display_title()).unwrap_or_else(|| "Book1".into());
            dl(&std::path::Path::new(&title).with_extension("pdf").to_string_lossy(), &b);
        }
    }

    /// Runs a command and shows its error in a message box (for menu/ribbon clicks).
    pub fn run_or_alert(&mut self, id: &str, params: Json) {
        let _ = self.run_confirming("GridCraft", id, params);
    }

    /// Runs a command; errors become a message box, and a "replace it?" error becomes a
    /// question whose OK runs the command again with `overwrite: true`.
    pub fn run_confirming(&mut self, title: &str, id: &str, params: Json) -> Outcome {
        match self.run(id, params.clone()) {
            Ok(v) => Outcome::Done(v),
            Err(e) => {
                if macro_ui::ask(self, title, id, &params, &e) {
                    return Outcome::Asked;
                }
                if e.contains("is not available right now") {
                    self.toast = Some((e, now_ms()));
                } else {
                    self.message = Some((title.into(), clean_error(&e)));
                }
                Outcome::Failed
            }
        }
    }

    /// UI-only commands (dialogs, view toggles that aren't saved in the file).
    fn run_ui_command(&mut self, id: &str, p: &Json) -> Option<Result<Json, String>> {
        let r = match id {
            "ui.ribbonTab" => {
                self.ui.ribbon_tab = p.get("tab").and_then(Json::as_str).unwrap_or("Home").to_string();
                Ok(Json::Null)
            }
            "view.formulaBar" => {
                self.ui.formula_bar = p.get("on").and_then(Json::as_bool).unwrap_or(!self.ui.formula_bar);
                Ok(json!({"on": self.ui.formula_bar}))
            }
            "view.collapseRibbon" => {
                self.ui.ribbon_collapsed = !self.ui.ribbon_collapsed;
                Ok(Json::Null)
            }
            "view.darkMode" => {
                let current = if self.ui.system_theme { self.effective_dark } else { self.ui.dark };
                self.ui.dark = p.get("on").and_then(Json::as_bool).unwrap_or(!current);
                self.ui.system_theme = false;
                Ok(json!({"on": self.ui.dark}))
            }
            "view.theme" => {
                match p.get("mode").and_then(Json::as_str) {
                    Some("system") => self.ui.system_theme = true,
                    Some(mode @ ("light" | "dark")) => {
                        self.ui.dark = mode == "dark";
                        self.ui.system_theme = false;
                    }
                    _ => return Some(Err("theme mode must be system, light, or dark".into())),
                }
                Ok(json!({"mode": self.ui.theme_mode()}))
            }
            "view.zoom100" => return Some(self.session.run("view.zoom", json!({"percent": 100})).inspect(|_| self.after_engine())),
            "app.language.set" => {
                // `{"language": "ja"}`; `code` is accepted as an alias.
                let code = p.get("language").or_else(|| p.get("code")).and_then(Json::as_str).unwrap_or("");
                match i18n::Language::parse(code) {
                    Some(l) => {
                        self.ui.language = l;
                        Ok(json!({"language": l}))
                    }
                    None => Err(format!("unknown language {code:?} (use \"en\", \"es\", \"zh\", \"ja\", \"ko\", \"ru\" or \"pt\")")),
                }
            }
            "app.language.english" => {
                self.ui.language = i18n::Language::En;
                Ok(json!({"language": i18n::Language::En}))
            }
            "app.language.japanese" => {
                self.ui.language = i18n::Language::Ja;
                Ok(json!({"language": i18n::Language::Ja}))
            }
            "ui.dialog" => {
                let name = p.get("name").and_then(Json::as_str).unwrap_or("");
                self.open_dialog(name, p.clone());
                Ok(json!({"dialog": self.dialog.as_ref().map(|d| d.name())}))
            }
            "ui.pane" => {
                // `{name}` shows a task pane (comments, watch, selection, formatChart,
                // pythonDiagnostics); no name closes it.
                self.grid.pane = p.get("name").and_then(Json::as_str).filter(|n| !n.is_empty()).map(str::to_string);
                Ok(json!({"pane": self.grid.pane}))
            }
            "ui.message.dismiss" => {
                self.message = None;
                Ok(Json::Null)
            }
            "python.saveCsv" => python_ui::save_csv(self, p),
            _ => return None,
        };
        Some(r)
    }

    /// Picks up requests commands made (dialogs, edit mode…).
    pub fn after_engine(&mut self) {
        self.prune_selected_object();
        for req in self.session.take_ui_requests() {
            match req {
                UiRequest::Dialog(name, params) => self.open_dialog(&name, params),
                UiRequest::Message(m) => self.message = Some(("GridCraft".into(), m)),
                UiRequest::EditCell(text) => {
                    let text = text.map(|text| {
                        self.session
                            .active()
                            .map(|d| formula_locale::display_template(&text, self.ui.language.formula_locale(), &d.wb, d.wb.active_sheet))
                            .unwrap_or(text)
                    });
                    self.begin_edit(text, false);
                }
                UiRequest::OpenUrl(u) => {
                    if let Some(f) = &self.services.open_url {
                        f(&u);
                    }
                }
            }
        }
    }

    pub fn open_dialog(&mut self, name: &str, params: Json) {
        if let Err(e) = self.commit_text_box_edit() {
            self.message = Some(("Text Box".into(), clean_error(&e)));
            return;
        }
        match name {
            "open" => {
                if let Some(start) = &self.services.open_async {
                    start();
                } else if let Some(pick) = &self.services.pick_open
                    && let Some(path) = pick()
                {
                    self.open_path(&path);
                }
            }
            "saveAs" => {
                let suggested = self.session.active().map(|d| d.display_title()).unwrap_or_else(|| "Book1".into());
                if let Some(pick) = &self.services.pick_save {
                    if let Some(path) = pick(&suggested) {
                        let path = if std::path::Path::new(&path).extension().is_none() { format!("{path}.xlsx") } else { path };
                        self.run_or_alert("file.save", json!({"path": path}));
                    }
                } else if let Some(dl) = &self.services.download
                    && let Ok(r) = self.session.run("file.saveBytes", json!({"format": "xlsx"}))
                    && let Some(b) = r.get("base64").and_then(Json::as_str).and_then(gridcraft_engine::io::base64_decode)
                {
                    let name = r.get("name").and_then(Json::as_str).unwrap_or("Book.xlsx").to_string();
                    dl(&name, &b);
                }
            }
            other => self.dialog = dialogs::Dialog::open(self, other, params),
        }
    }

    pub fn open_path(&mut self, path: &str) {
        match self.run("file.open", json!({"path": path})) {
            Ok(_) => {
                self.ui.recent.retain(|p| p != path);
                self.ui.recent.insert(0, path.to_string());
                self.ui.recent.truncate(20);
            }
            Err(e) => self.message = Some(("GridCraft".into(), clean_error(&e))),
        }
        self.after_engine();
    }

    pub fn view_key(&self) -> Option<(u64, usize)> {
        self.session.active().map(|d| (d.uid, d.wb.active_sheet))
    }

    pub fn view(&self) -> SheetView {
        self.view_key().and_then(|k| self.views.get(&k).copied()).unwrap_or_default()
    }

    pub fn view_mut(&mut self) -> Option<&mut SheetView> {
        let k = self.view_key()?;
        Some(self.views.entry(k).or_default())
    }

    /// Starts editing the active cell. `text` replaces the content (typing) or pre-fills it.
    /// Editing a Python cell (rather than typing over it) happens in the formula bar, in Python
    /// mode.
    pub fn begin_edit(&mut self, text: Option<String>, from_formula_bar: bool) {
        if let Err(e) = self.commit_text_box_edit() {
            self.message = Some(("Text Box".into(), clean_error(&e)));
            return;
        }
        let Some(d) = self.session.active() else { return };
        let Some(sh) = d.wb.active() else { return };
        let at = sh.merge_at(d.selection.active).map(|m| m.start).unwrap_or(d.selection.active);
        if text.is_none() && sh.cell(at).is_some_and(|c| c.py.is_some()) {
            self.begin_python_edit();
            return;
        }
        // A picture has no editable scalar text. Selecting it, F2, or clicking
        // the formula bar must not turn its fallback value into a text edit.
        // Typing still starts an intentional replacement.
        if text.is_none() && sh.cell_pictures.contains_key(&at) {
            return;
        }
        let locale = self.ui.language.formula_locale();
        let current = if sh.cell_pictures.contains_key(&at) {
            String::new()
        } else {
            sh.cell(at)
                .map(
                    |c| if c.formula.is_some() { formula_locale::display(&c.input_text(), locale, &d.wb, d.wb.active_sheet) } else { c.input_text() },
                )
                .unwrap_or_default()
        };
        let (text, replace) = match text {
            Some(t) => (t, true),
            None => (current.clone(), false),
        };
        let mut ed = editor::EditState::new(d.wb.active_sheet, at, text, replace, from_formula_bar);
        ed.locale = locale;
        if replace && !ed.is_formula() {
            ed.completion = editor::column_completion(sh, at, &ed.text);
        }
        self.editor = Some(ed);
        self.session.mode = if replace { gridcraft_engine::Mode::Enter } else { gridcraft_engine::Mode::Edit };
    }

    /// Starts editing the active cell in Python mode, in the formula bar, with its code (empty
    /// for a cell that isn't a Python cell yet). Nothing changes until the code is committed.
    pub fn begin_python_edit(&mut self) {
        if let Err(e) = self.commit_text_box_edit() {
            self.message = Some(("Text Box".into(), clean_error(&e)));
            return;
        }
        let Some(d) = self.session.active() else { return };
        let Some(sh) = d.wb.active() else { return };
        let at = sh.merge_at(d.selection.active).map(|m| m.start).unwrap_or(d.selection.active);
        let code = sh.cell(at).and_then(|c| c.py.as_ref()).map(|p| p.code.clone()).unwrap_or_default();
        let mut ed = editor::EditState::python(d.wb.active_sheet, at, code);
        ed.locale = self.ui.language.formula_locale();
        self.editor = Some(ed);
        self.session.mode = gridcraft_engine::Mode::Edit;
    }

    /// Formulas › Insert Python › Insert xl() Reference: turns on Python mode's pointing (as F2
    /// does), starting a Python edit of the active cell if none is open, so the next click or drag
    /// on the sheet inserts `xl("…")`.
    pub fn point_python(&mut self) {
        if !self.editor.as_ref().is_some_and(|e| e.python) {
            if self.editor.is_some() {
                self.toggle_python();
            } else {
                self.begin_python_edit();
            }
        }
        if let Some(ed) = self.editor.as_mut().filter(|e| e.python) {
            ed.py_point = true;
            ed.point = None;
            ed.point_cell = None;
            ed.request_focus = true;
            self.session.mode = gridcraft_engine::Mode::Point;
            self.toast = Some(("Click or drag on the sheet to insert an xl() reference".into(), now_ms()));
        }
    }

    /// Whether the cell is a Python cell.
    pub fn is_python_cell(&self, sheet: usize, at: CellRef) -> bool {
        self.session.active().and_then(|d| d.wb.sheet(sheet)).and_then(|sh| sh.cell(at)).is_some_and(|c| c.py.is_some())
    }

    /// Leaves Python mode (the PY badge, Ctrl+Alt+Shift+P, Backspace in an empty editor): Python
    /// goes off and the input is cleared. Committing the empty input then turns a Python cell
    /// back into a normal cell, which undo reverses; Esc keeps the cell as it was. Not editing, it
    /// starts that edit on the active Python cell.
    pub fn leave_python(&mut self) {
        if self.editor.is_none() {
            let Some(d) = self.session.active() else { return };
            if !self.is_python_cell(d.wb.active_sheet, d.selection.active) {
                return;
            }
            self.begin_python_edit();
        }
        let Some((sheet, cell)) = self.editor.as_ref().filter(|e| e.python).map(|e| (e.sheet, e.cell)) else { return };
        let was = self.is_python_cell(sheet, cell);
        if let Some(ed) = self.editor.as_mut() {
            ed.leave_python(was);
        }
        self.session.mode = gridcraft_engine::Mode::Edit;
    }

    /// Ctrl+Alt+Shift+P: Python mode on or off. Not editing, it starts a Python edit of the
    /// active cell; in a formula edit, the text becomes code.
    pub fn toggle_python(&mut self) {
        match self.editor.as_mut() {
            Some(ed) if ed.python => self.leave_python(),
            Some(ed) => {
                ed.enter_python();
                self.session.mode = gridcraft_engine::Mode::Edit;
            }
            None => self.begin_python_edit(),
        }
    }

    /// Switches the active Python cell between a Python object and Excel values
    /// (Ctrl+Alt+Shift+M, the PY badge menu, Insert Python's menu, the cell menu).
    pub fn toggle_python_output(&mut self) {
        let Some(d) = self.session.active() else { return };
        let at = d.selection.active;
        let Some(object) = d.wb.active().and_then(|sh| sh.cell(at)).and_then(|c| c.py.as_ref()).map(|p| p.object) else {
            self.toast = Some(("Select a Python cell to change its output".into(), now_ms()));
            return;
        };
        self.run_or_alert("python.output", json!({"cell": at.a1(), "mode": if object { "value" } else { "object" }}));
    }

    /// Commits a Python-mode edit through `python.set`; the cell stays selected.
    fn commit_python(&mut self, ed: editor::EditState) -> bool {
        self.session.mode = gridcraft_engine::Mode::Ready;
        let sheet = self.session.active().and_then(|d| d.wb.sheet(ed.sheet)).map(|s| s.name.clone());
        let mut p = json!({"cell": ed.cell.a1(), "code": ed.text});
        if let Some(sheet) = sheet {
            p["sheet"] = json!(sheet);
        }
        match self.session.run("python.set", p) {
            Ok(_) => {
                self.after_engine();
                true
            }
            Err(e) => {
                self.message = Some(("GridCraft".into(), clean_error(&e)));
                self.editor = Some(ed);
                self.session.mode = gridcraft_engine::Mode::Edit;
                false
            }
        }
    }

    /// Commits the edit; `then` moves the selection (dr, dc) afterwards.
    pub fn commit_edit(&mut self, dr: i64, dc: i64, array: bool, fill_selection: bool) -> bool {
        let Some(mut ed) = self.editor.take() else { return true };
        if ed.python {
            return self.commit_python(ed);
        }
        // A bare `=PY` entered with Enter or Tab starts Python mode instead.
        if ed.is_py_trigger() {
            ed.enter_python();
            self.editor = Some(ed);
            self.session.mode = gridcraft_engine::Mode::Edit;
            return false;
        }
        // Left Python mode and entered nothing: the Python cell becomes a normal cell.
        if ed.py_left && ed.text.trim().is_empty() && self.is_python_cell(ed.sheet, ed.cell) {
            self.session.mode = gridcraft_engine::Mode::Ready;
            let sheet = self.session.active().and_then(|d| d.wb.sheet(ed.sheet)).map(|s| s.name.clone()).unwrap_or_default();
            return match self.session.run("python.clear", json!({"cell": ed.cell.a1(), "sheet": sheet})) {
                Ok(_) => {
                    if dr != 0 || dc != 0 {
                        self.move_after_enter(dr, dc);
                    }
                    self.after_engine();
                    true
                }
                Err(e) => {
                    self.message = Some(("GridCraft".into(), clean_error(&e)));
                    self.editor = Some(ed);
                    self.session.mode = gridcraft_engine::Mode::Edit;
                    false
                }
            };
        }
        self.session.mode = gridcraft_engine::Mode::Ready;
        let text = match &ed.completion {
            Some(full) if full.to_lowercase().starts_with(&ed.text.to_lowercase()) => full.clone(),
            _ => ed.text.clone(),
        };
        let text = match self.session.active().map(|d| formula_locale::canonical(&text, ed.locale, &d.wb, ed.sheet, ed.cell)) {
            Some(Ok(text)) => text,
            Some(Err(error)) => {
                self.message = Some(("GridCraft".into(), error.to_string()));
                self.editor = Some(ed);
                self.session.mode = gridcraft_engine::Mode::Edit;
                return false;
            }
            None => text,
        };
        // Data validation.
        if let Some(d) = self.session.active()
            && let Some((dv, msg)) = gridcraft_engine::cmd::data::check_validation(&d.wb, ed.sheet, ed.cell, &text)
        {
            if dv.error_style == gridcraft_engine::model::ErrorStyle::Stop {
                let title = if dv.error_title.is_empty() { "GridCraft".to_string() } else { dv.error_title.clone() };
                self.message = Some((title, msg));
                self.editor = Some(ed);
                self.session.mode = gridcraft_engine::Mode::Edit;
                return false;
            }
            self.toast = Some((msg, now_ms()));
        }
        let r = if fill_selection {
            self.session.run("range.fill", json!({"input": text}))
        } else {
            self.session.run("cell.set", json!({"cell": ed.cell.a1(), "input": text, "array": array}))
        };
        match r {
            Ok(_) => {
                if dr != 0 || dc != 0 {
                    self.move_after_enter(dr, dc);
                }
                self.after_engine();
                true
            }
            Err(e) => {
                self.message = Some(("GridCraft".into(), clean_error(&e)));
                self.editor = Some(ed);
                self.session.mode = gridcraft_engine::Mode::Edit;
                false
            }
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editor = None;
        self.session.mode = gridcraft_engine::Mode::Ready;
    }

    /// Enter/Tab move inside a multi-cell selection, otherwise to the next cell.
    pub fn move_after_enter(&mut self, dr: i64, dc: i64) {
        let multi = self.session.active().is_some_and(|d| !d.selection.is_single_cell());
        if multi {
            let fwd = dr > 0 || dc > 0;
            let _ = self.session.run("selection.next", json!({"forward": fwd, "byRow": dc != 0}));
        } else {
            let _ = self.session.run("selection.move", json!({"dr": dr, "dc": dc}));
        }
        self.grid.ensure_visible = true;
    }

    /// Per-frame logic (control channel, screenshots). Call before `ui`.
    pub fn logic(&mut self, ctx: &egui::Context) {
        if self.fonts_han.is_some_and(|han| han != theme::HanOrder::of(self.ui.language)) {
            self.fonts_ready = false;
            self.fonts_set = false;
        }
        if !self.fonts_ready {
            // New fonts apply from the next frame on: paint nothing until then.
            if self.fonts_set {
                self.fonts_ready = true;
            } else {
                SheetApp::setup_context_for_language(ctx, self.ui.dark, self.ui.language);
                self.fonts_set = true;
                self.fonts_han = Some(theme::HanOrder::of(self.ui.language));
                ctx.request_repaint();
            }
        }
        let preference = self.ui.theme_preference();
        if ctx.options(|o| o.theme_preference) != preference {
            ctx.set_theme(preference);
            ctx.request_repaint();
        }
        self.effective_dark = ctx.theme() == egui::Theme::Dark;
        control::poll(self, ctx);
        self.drive_job(ctx);
        // Python cells running in the background: take results, start the next cell.
        if self.session.python_busy() {
            self.session.step_python();
            ctx.request_repaint_after(std::time::Duration::from_millis(15));
        }
        // A package install that ended: say how, once.
        python_ui::install_toast(self);
        // Formulas entered since the last frame, from any source, feed Recently Used.
        function_library::harvest(self);
        // Files read asynchronously (web file picker, dropped files).
        let arrived: Vec<(String, Vec<u8>)> = self
            .services
            .inbox
            .as_ref()
            .map(|i| std::mem::take(&mut *i.lock().unwrap_or_else(std::sync::PoisonError::into_inner)))
            .unwrap_or_default();
        for (name, bytes) in arrived {
            let b64 = gridcraft_engine::io::base64_encode(&bytes);
            if let Err(e) = self.run("file.open", json!({"name": name, "base64": b64})) {
                self.message = Some(("GridCraft".into(), clean_error(&e)));
            }
            self.after_engine();
        }
        if self.session.active().is_none() {
            self.session.new_workbook();
        }
    }

    /// Advances a long-running job (Solver) a slice per frame; Esc pauses it.
    fn drive_job(&mut self, ctx: &egui::Context) {
        use gridcraft_engine::job::JobState;
        if matches!(self.session.job_state, JobState::Running { .. }) {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                // Solver pauses into its Show Trial Solution box; other jobs ask generically.
                let solver = matches!(&self.session.job_state, JobState::Running { label, .. } if label == "Solver");
                self.session.pause_job(if solver { json!({"dialog": "solverTrial", "reason": "user"}) } else { json!({"reason": "user"}) });
            } else {
                self.session.step_job(16.0);
                self.after_engine();
                ctx.request_repaint();
            }
        }
        // A job paused with a dialog of its own (Solver's Show Trial Solution) shows it.
        if let JobState::Paused { info, .. } = &self.session.job_state
            && let Some(name) = info.get("dialog").and_then(Json::as_str)
            && self.dialog.as_ref().is_none_or(|d| d.name != name)
        {
            let (name, info) = (name.to_string(), info.clone());
            self.open_dialog(&name, info);
        }
        if let Some((label, Err(e))) = self.session.take_job_result() {
            self.message = Some((label, clean_error(&e)));
        }
    }

    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        if let Some(mut batch) = self.synthetic.pop_front() {
            raw.events.append(&mut batch);
        }
    }

    /// Lays out the whole window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.fonts_ready {
            ctx.request_repaint();
            return;
        }
        let t0 = now_ms();
        text_box::before_ui(self, &ctx);
        // The language the widgets translate with this frame (see `i18n::current`).
        i18n::set_current(&ctx, self.ui.language);
        let t = theme::Tokens::get(&ctx);
        self.ribbon_keys(&ctx);
        ribbon::title_bar(self, ui);
        ribbon::show(self, ui);
        // Python code is edited in the formula bar, so it shows while that happens.
        if self.ui.formula_bar || self.editor.as_ref().is_some_and(|e| e.python) {
            formula_bar::show(self, ui);
        }
        python_ui::enable_bar(self, ui);
        if self.ui.status_bar {
            tabs::status_bar(self, ui);
        }
        tabs::sheet_tabs(self, ui);
        pivot_pane::show(self, ui);
        panes::show(self, ui);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.grid_bg)).show(ui, |ui| {
            let g0 = now_ms();
            grid::show(self, ui);
            self.perf.grid_ms = now_ms() - g0;
        });
        dialogs::show(self, &ctx);
        widgets::message_box(self, &ctx);
        widgets::confirm_box(self, &ctx);
        widgets::job_paused(self, &ctx);
        widgets::toast(self, &ctx);
        control::issue_screenshots(self, &ctx);
        control::collect_screenshots(self, &ctx);
        if !ctx.input(|i| i.focused) {
            // Nothing else to do.
        }
        let now = now_ms();
        self.perf.frame_ms = now - t0;
        if self.perf.last > 0.0 {
            let dt = now - self.perf.last;
            if dt > 0.0 {
                self.perf.fps = self.perf.fps * 0.9 + 100.0 / dt;
            }
        }
        self.perf.last = now;
        // AutoSave: save workbooks that have a file shortly after they change.
        if self.grid.autosave
            && self.editor.is_none()
            && self.text_box_editor.is_none()
            && now - self.grid.last_autosave > 2000.0
            && self.session.active().is_some_and(|d| d.is_dirty() && d.path.as_deref().is_some_and(|p| p.ends_with(".xlsx")))
        {
            self.grid.last_autosave = now;
            let _ = self.session.run("file.save", json!({}));
        }
        // Window title.
        if let Some(d) = self.session.active() {
            let title = format!("{}{}", d.display_title(), if d.is_dirty() { " — Edited" } else { "" });
            if self.grid.last_title.as_deref() != Some(title.as_str()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
                self.grid.last_title = Some(title);
            }
        }
    }

    fn ribbon_keys(&mut self, ctx: &egui::Context) {
        let before = self.keytips.prefix().map(str::to_string);
        let enabled = self.editor.is_none()
            && self.dialog.is_none()
            && self.message.is_none()
            && self.name_box.is_none()
            && !ctx.text_edit_focused()
            && self.grid.context_menu.is_none()
            && self.grid.header_menu.is_none()
            && self.grid.filter_menu.is_none()
            && self.grid.list_picker.is_none()
            && self.grid.renaming_tab.is_none()
            && (before.is_some() || !egui::Popup::is_any_open(ctx));
        let actions = self.keytips.process(ctx, enabled);
        if before.is_some() && before.as_deref() != self.keytips.prefix() && !ctx.input(|i| i.pointer.any_pressed()) {
            // Leave pointer cancellation to the popup so clicking a menu item still works.
            egui::Popup::close_all(ctx);
        }
        // The legacy Edit/Format paths lead to the same existing Home menus. Switch once, when
        // the prefix changes, not every frame (that would reset a tab the user picked meanwhile).
        let after = self.keytips.prefix();
        if after != before.as_deref() && matches!(after, Some("E" | "O" | "OC")) {
            self.run_or_alert("ui.ribbonTab", json!({"tab": "Home"}));
            self.ui.ribbon_collapsed = false;
        }
        for action in actions {
            match action {
                keytips::Action::Tab(tab) => {
                    self.run_or_alert("ui.ribbonTab", json!({"tab": tab}));
                    self.ui.ribbon_collapsed = false;
                }
                keytips::Action::Command(id) => self.run_or_alert(id, json!({})),
                keytips::Action::Dialog(name) => self.open_dialog(name, json!({})),
            }
        }
    }

    /// Current selection's top-left cell text as A1 (for the Name Box).
    pub fn name_box_text(&self) -> String {
        let Some(d) = self.session.active() else { return String::new() };
        let sel = &d.selection;
        let cur = sel.current();
        // A named range that exactly matches shows its name.
        let sh = d.wb.active();
        if let Some(sh) = sh {
            for n in &d.wb.names {
                let target = n.formula.replace('$', "");
                let (sheet, body) = match target.rsplit_once('!') {
                    Some((s, b)) => (Some(s.trim_matches('\'').to_string()), b.to_string()),
                    None => (None, target.clone()),
                };
                if sheet.as_deref().is_none_or(|s| s.eq_ignore_ascii_case(&sh.name)) && RangeRef::parse(&body) == Some(cur) {
                    return n.name.clone();
                }
            }
        }
        if self.grid.drag_select && !cur.is_single() {
            return format!("{}R x {}C", cur.height(), cur.width());
        }
        sel.active.a1()
    }
}

/// Strips the engine's error prefixes for message boxes.
pub fn clean_error(e: &str) -> String {
    if let Some(rest) = e.split_once("`: ").map(|(_, r)| r) {
        return rest.to_string();
    }
    e.to_string()
}

/// More about the toast shown at `at`: how long it stays, whether it reports a failure, and a
/// button that opens a pane (`(label, pane)`).
#[derive(Clone, Debug, PartialEq)]
pub struct ToastExtra {
    pub at: f64,
    pub ms: f64,
    pub danger: bool,
    pub action: Option<(String, String)>,
}

#[allow(clippy::disallowed_methods)] // the clock is read only off wasm
pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

pub fn cell(r: u32, c: u32) -> CellRef {
    CellRef::new(r, c)
}
