//! Saves that wait for a workbook's Python cells, so the file gets their results.
//!
//! A person's save that starts while the workbook's cells run becomes a [`SaveWait`], which [`poll`]
//! checks every frame from `SheetApp::logic`. Nothing blocks.
//! The wait ends when the cells have finished, at the cap ([`gridcraft_engine::python::PythonWait`]),
//! or at Save Now; then the saves it holds run, in the workbook's window, without counting as an
//! activation, so what the person is looking at and the order they came from stay as they were. A
//! workbook that closes takes its wait with it. Save Changes' Save leaves the close for the wait to do.

use gridcraft_engine::Session;
use gridcraft_engine::python::{PythonWait, WaitBound, WaitState, note_unfinished};
use serde_json::{Value as Json, json};

use crate::{NoticeKind, SheetApp, clean_error};

/// What the bar says while a save waits.
pub const WAITING: &str = "Waiting for Python cells to finish before saving...";
/// The bar's button that saves without waiting any longer.
pub const SAVE_NOW: &str = "Save Now";
/// What a person is told when a save left cells unfinished.
pub const UNFINISHED: &str =
    "The workbook was saved, but these Python cells weren't finished, so the file has their last results, if they had any: {cells}";
/// The end of a list of cells too long to give whole.
pub const AND_MORE: &str = "{cells} and {count} more";

/// How many cells the message names; the rest are counted.
const NAMED_CELLS: usize = 10;
/// The waits of the open workbooks.
#[derive(Default)]
pub struct SaveWaits {
    waits: Vec<SaveWait>,
}

/// The saves waiting for one workbook's cells.
struct SaveWait {
    wait: PythonWait,
    /// In the order they were asked for, one per file.
    saves: Vec<Save>,
    /// A person asked, so the bar shows and the outcome is said in a message.
    person: bool,
    close_after: Option<CloseAfter>,
    /// Save Now was pressed.
    now: bool,
}

/// One file a wait will write.
struct Save {
    id: String,
    params: Json,
    /// The file, as far as it is known when asked: the save's own path, else the workbook's.
    target: Option<String>,
}

/// What follows a save that worked (Save Changes' Save).
#[derive(Clone, Copy)]
struct CloseAfter {
    quitting: bool,
}

impl SaveWaits {
    fn of(&self, doc: u64) -> Option<&SaveWait> {
        self.waits.iter().find(|w| w.wait.doc() == doc)
    }

    fn of_mut(&mut self, doc: u64) -> Option<&mut SaveWait> {
        self.waits.iter_mut().find(|w| w.wait.doc() == doc)
    }

    /// Whether a save of this workbook is waiting.
    pub fn waiting(&self, doc: u64) -> bool {
        self.of(doc).is_some()
    }

    /// How many files are waiting to be written for this workbook.
    pub(crate) fn saves_of(&self, doc: u64) -> usize {
        self.of(doc).map_or(0, |w| w.saves.len())
    }

    /// Whether any save is waiting.
    pub fn any(&self) -> bool {
        !self.waits.is_empty()
    }

    /// Makes a waiting save close its workbook once it has worked. False when none is waiting.
    pub(crate) fn close_after(&mut self, doc: u64, quitting: bool) -> bool {
        match self.of_mut(doc) {
            Some(w) => {
                w.close_after = Some(CloseAfter { quitting });
                true
            }
            None => false,
        }
    }

    /// Whether this workbook's save goes on to the quit, which has asked about it already.
    pub(crate) fn closes_for_quit(&self, doc: u64) -> bool {
        self.of(doc).is_some_and(|w| w.close_after.is_some_and(|c| c.quitting))
    }

    /// Whether a save in an open workbook will carry the quit on once it is done.
    pub(crate) fn quit_waits(&self, session: &Session) -> bool {
        self.waits.iter().any(|w| session.document_index(w.wait.doc()).is_some() && w.close_after.is_some_and(|c| c.quitting))
    }

    /// A shorter cap for a workbook's wait, so a test doesn't sit through the real one.
    #[doc(hidden)]
    pub fn set_cap(&mut self, doc: u64, cap: std::time::Duration) {
        if let Some(w) = self.of_mut(doc) {
            w.wait.set_cap(cap);
        }
    }
}

impl SaveWait {
    fn new(wait: PythonWait, person: bool) -> SaveWait {
        SaveWait { wait, saves: Vec::new(), person, close_after: None, now: false }
    }

    /// Adds a save, unless one to the same file is waiting already. Returns the one that stands for it.
    fn add(&mut self, id: &str, params: &Json, target: Option<String>) -> Option<&mut Save> {
        let found = match &target {
            Some(_) => self.saves.iter().position(|s| s.target == target),
            // A workbook that has no file yet: the save already waiting gives it one.
            None => (!self.saves.is_empty()).then_some(0),
        };
        let at = found.unwrap_or_else(|| {
            self.saves.push(Save { id: id.to_string(), params: without_wait(params.clone()), target });
            self.saves.len().saturating_sub(1)
        });
        self.saves.get_mut(at)
    }
}

/// `wait` and `waitSeconds` would make the engine block.
pub(crate) fn without_wait(mut params: Json) -> Json {
    if let Some(o) = params.as_object_mut() {
        o.remove("wait");
        o.remove("waitSeconds");
    }
    params
}

impl SheetApp {
    /// A person's Save or Save As: includes the cell being edited, then makes the save wait when the
    /// workbook's cells are running or queued, or joins the save already waiting. Returns whether
    /// that took care of it; otherwise the save goes ahead, or asks where to save first.
    pub(crate) fn person_save_waits(&mut self, id: &str, params: &Json) -> Result<bool, String> {
        self.settle_drafts()?;
        let Some(d) = self.session.active() else { return Ok(false) };
        let doc = d.uid;
        let path = params.get("path").and_then(Json::as_str).map(str::to_string);
        // Save As asks where first, and comes back here with the path.
        if id == "file.saveAs" && path.is_none() {
            return Ok(false);
        }
        let target = path.or_else(|| d.path.clone());
        if let Some(w) = self.save_waits.of_mut(doc) {
            w.person = true;
            w.add(id, params, target);
            return Ok(true);
        }
        if target.is_none() {
            return Ok(false);
        }
        let Some(wait) = self.session.python_wait_begin(doc, WaitBound::Cells, false) else { return Ok(false) };
        let mut w = SaveWait::new(wait, true);
        w.add(id, params, target);
        self.save_waits.waits.push(w);
        Ok(true)
    }

    /// Whether Quit or Close must ask about this workbook: it has unsaved changes, or a save is
    /// waiting and would be lost. A save that goes on to the quit has been answered for.
    pub(crate) fn must_ask_before_closing(&self, d: &gridcraft_engine::DocState) -> bool {
        (d.is_dirty() || self.save_waits.waiting(d.uid)) && !self.save_waits.closes_for_quit(d.uid)
    }

    /// A close (`file.close`, `window.close`) of a clean workbook whose save is waiting asks as it
    /// does for unsaved work. An unsaved workbook's question comes from the command itself. Returns
    /// whether it asked.
    pub(crate) fn ask_before_closing_waiting_save(&mut self, id: &str, params: &Json) -> bool {
        if params.get("force").and_then(Json::as_bool) == Some(true) {
            return false;
        }
        let window = match id {
            "window.close" => gridcraft_engine::Target::from_json(params).ok().and_then(|t| self.session.target_window(t).ok()),
            _ => self.session.active_window(),
        };
        let Some((window, doc)) = window.and_then(|w| self.session.window(w).map(|x| (w, x.doc_uid))) else { return false };
        let whole = (id == "file.close" && params.get("document").and_then(Json::as_bool) == Some(true))
            || self.session.windows().iter().filter(|w| w.doc_uid == doc).count() <= 1;
        let clean = self.session.documents().iter().find(|d| d.uid == doc).is_some_and(|d| !d.is_dirty());
        if !whole || !clean || !self.save_waits.waiting(doc) {
            return false;
        }
        self.enter_window(window);
        let title = self.session.active().map(|d| d.display_title()).unwrap_or_default();
        self.open_dialog("saveChanges", json!({"title": title}));
        true
    }

    /// Whether a window on this workbook has a cell or text-box draft open: what a close after a
    /// save would lose, as a draft isn't in the file.
    fn doc_has_draft(&self, doc: u64) -> bool {
        self.session.windows().iter().filter(|w| w.doc_uid == doc).any(|w| {
            if Some(w.id) == self.ui_window() { self.editor.is_some() || self.text_box_editor.is_some() } else { self.parked_has_draft(w.id) }
        })
    }

    /// Adds to the message a window shows, rather than replacing what it says.
    fn say(&mut self, text: String) {
        self.message = Some(match self.message.take() {
            Some((title, old)) => (title, format!("{old}\n\n{text}")),
            None => ("GridCraft".into(), text),
        });
    }

    /// The message about cells a save left unfinished, in the interface's language.
    fn unfinished_message(&self, names: &[String]) -> String {
        let lang = self.ui.language;
        let named = names.iter().take(NAMED_CELLS).cloned().collect::<Vec<_>>().join(", ");
        let cells = match names.len().saturating_sub(NAMED_CELLS) {
            0 => named,
            more => lang.tr(AND_MORE).replace("{count}", &more.to_string()).replace("{cells}", &named),
        };
        lang.tr(UNFINISHED).replace("{cells}", &cells)
    }
}

/// Every frame: ends the waits that are over.
pub(crate) fn poll(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.save_waits.any() {
        return;
    }
    let mut i = 0;
    while let Some(w) = app.save_waits.waits.get_mut(i) {
        let closed = app.session.document_index(w.wait.doc()).is_none();
        let state = app.session.python_wait_poll(&mut w.wait);
        if !closed && !w.now && state == WaitState::Waiting {
            i = i.saturating_add(1);
            continue;
        }
        let w = app.save_waits.waits.remove(i);
        if !closed {
            finish(app, ctx, w);
        }
    }
    if app.save_waits.any() {
        ctx.request_repaint_after(crate::python_ui::BUSY_REPAINT);
    }
}

/// The wait is over: its saves run in the workbook's window, then the close it was asked for.
/// Nothing here counts as an activation, so the order the person came through the windows in stays.
fn finish(app: &mut SheetApp, ctx: &egui::Context, w: SaveWait) {
    let doc = w.wait.doc();
    let unfinished = app.session.python_wait_unfinished(&w.wait);
    let back = app.session.active_window();
    if let Some(window) = app.session.doc_window(doc) {
        app.show_window(window);
    }
    let mut failed = Vec::new();
    for save in w.saves {
        // Not `run`: a draft begun during the wait stays a draft, and goes in with the next save.
        app.store_view_state();
        if let Err(e) = app.session.run(&save.id, save.params) {
            failed.push(clean_error(&e));
        }
        app.after_engine();
    }
    // A person who pressed Save Now knows, and one who then closes the workbook doesn't need telling.
    let closing = w.close_after.is_some() && w.now;
    if w.person && !failed.is_empty() {
        app.say(failed.join("\n"));
    } else if w.person && !unfinished.is_empty() && !closing {
        app.say(app.unfinished_message(&unfinished));
    }
    // Cells that weren't finished keep the workbook open, unless Save Now said not to wait for them:
    // the message about them would go with its window, and so would their results.
    let saved = failed.is_empty() && (unfinished.is_empty() || w.now);
    let quit = w.close_after.is_some_and(|after| close_after_save(app, ctx, doc, after, saved));
    // A quit that goes on brings forward the window it asks in.
    if !quit {
        match back.filter(|b| app.session.window(*b).is_some()) {
            Some(back) => app.show_window(back),
            None => app.follow_active_window(),
        }
    }
    ctx.request_repaint();
}

/// Closes the workbook its save was for, and goes on with the quit, unless that would lose work: the
/// save failed or left cells unfinished (`saved`), it wrote a copy that left the workbook unsaved, or
/// a draft was begun during the wait. Returns whether the quit went on.
fn close_after_save(app: &mut SheetApp, ctx: &egui::Context, doc: u64, after: CloseAfter, saved: bool) -> bool {
    let clean = app.session.documents().iter().find(|d| d.uid == doc).is_some_and(|d| !d.is_dirty());
    if !saved || !clean || app.doc_has_draft(doc) {
        return false;
    }
    app.run_or_alert("file.close", json!({"force": true, "document": true}));
    if after.quitting {
        app.continue_quit(ctx);
    }
    after.quitting
}

/// `ui.saveWait {action?}`: `saveNow` saves without waiting any longer, `cancel` ends the wait with
/// no save, and the reply says where the active workbook's wait stands.
pub(crate) fn command(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let Some(doc) = app.session.active().map(|d| d.uid) else { return Ok(json!({"waiting": false})) };
    match p.get("action").and_then(Json::as_str) {
        None => {}
        Some("saveNow") => {
            if let Some(w) = app.save_waits.of_mut(doc) {
                w.now = true;
            }
        }
        Some("cancel") => {
            app.save_waits.waits.retain(|w| w.wait.doc() != doc);
        }
        Some(other) => return Err(format!("unknown action {other:?} (use saveNow or cancel)")),
    }
    Ok(app.save_waits.of(doc).map_or_else(|| json!({"waiting": false}), |w| wait_json(&app.session, w)))
}

fn wait_json(session: &Session, w: &SaveWait) -> Json {
    let names = session.python_unfinished(w.wait.doc());
    let mut out = json!({
        "waiting": true,
        "doc": w.wait.doc(),
        "ms": w.wait.elapsed().as_secs_f64() * 1000.0,
        "capMs": w.wait.cap().as_secs_f64() * 1000.0,
        "prompt": w.person,
        "saves": w.saves.iter().map(|s| json!(s.target)).collect::<Vec<_>>(),
        "closeAfter": w.close_after.is_some(),
        "quitting": w.close_after.is_some_and(|c| c.quitting),
    });
    note_unfinished(&mut out, true, &names);
    out
}

/// The waits, for `ui.inspect`.
pub fn inspect(app: &SheetApp) -> Json {
    Json::Array(app.save_waits.waits.iter().map(|w| wait_json(&app.session, w)).collect())
}

/// The bar a person's waiting save shows in its workbook's window.
pub fn bar(app: &mut SheetApp, ui: &mut egui::Ui) {
    let Some(doc) = app.session.active().map(|d| d.uid) else { return };
    if !app.save_waits.of(doc).is_some_and(|w| w.person) {
        return;
    }
    let lang = app.ui.language;
    let (save_now, cancel) =
        crate::widgets::choice_bar(ui, "save_wait_bar", NoticeKind::Info, lang.tr(WAITING), lang.tr(SAVE_NOW), lang.tr("Cancel"));
    let action = if save_now {
        "saveNow"
    } else if cancel {
        "cancel"
    } else {
        return;
    };
    app.run_or_alert("ui.saveWait", json!({"action": action}));
    ui.ctx().request_repaint();
}
