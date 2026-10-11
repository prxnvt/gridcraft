//! Several workbook windows in one process (File › New, Cmd+N), so the app has one Dock icon and
//! one Quit. The first window is eframe's root viewport; each further window is an immediate
//! viewport with its own [`SheetApp`].
//!
//! Closing the root window while others are open hands the root to the next window (it moves to
//! where that window was), because closing eframe's root viewport ends the app. Quit asks each
//! window with unsaved changes in turn; Cancel on any of them stops quitting.
//!
//! The control channel belongs to the app, not to a window: it follows the window in front, so
//! automation acts where a person would (see `docs/control-protocol.md`).

use std::sync::mpsc::Receiver;

use egui::{Rect, ViewportBuilder, ViewportCommand, ViewportId, vec2};
use gridcraft_ui_egui::{ControlRequest, SheetApp};

/// A workbook window other than the root one.
struct Extra {
    id: ViewportId,
    app: SheetApp,
    builder: ViewportBuilder,
    /// Outer rectangle last frame (to place the root window when it takes this one's place).
    outer: Option<Rect>,
    closed: bool,
}

#[derive(Default)]
pub struct Windows {
    extras: Vec<Extra>,
    next_id: u64,
    /// The window in front (menu commands go to it); `None` is the root window.
    focused: Option<ViewportId>,
    quitting: bool,
    /// The window whose Save / Don't Save / Cancel prompt Quit opened last.
    asked: Option<ViewportId>,
    /// The control channel, taken from a window that closed, until the next frame hands it to
    /// the window in front.
    spare_control: Option<Receiver<ControlRequest>>,
    /// The settings every window shares (see [`Windows::sync_shared`]).
    shared: Shared,
}

/// Settings every window shares, as the last value seen in all of them (see
/// [`Windows::sync_shared`]).
///
/// Display: egui draws every window with one context, which has one theme and one set of fonts,
/// so windows with different themes or languages would fight over it each frame (garbled text,
/// the theme flickering). Theme (light, dark, follow the system) and interface language are
/// app-wide, as the Mac's appearance is.
///
/// Preferences: the Options (`session.prefs`), the recent files and the recently used functions
/// are the app's, not a window's: a change in any window reaches every window, and is saved on
/// exit whichever window made it.
#[derive(Default)]
struct Shared {
    display: Option<(bool, bool, gridcraft_ui_egui::i18n::Language)>,
    prefs: Option<gridcraft_engine::Prefs>,
    recent: Option<Vec<String>>,
    recent_functions: Option<Vec<String>>,
}

/// Gives every app the value of one shared setting last changed in any of them: the front
/// window's (`apps[front]`) when it changed, else the first that changed. `last` is the value
/// they all had.
fn sync_part<T: Clone + PartialEq>(
    last: &mut Option<T>,
    apps: &mut [&mut SheetApp],
    front: usize,
    get: impl Fn(&SheetApp) -> T,
    set: impl Fn(&mut SheetApp, &T),
) {
    let values: Vec<T> = apps.iter().map(|a| get(a)).collect();
    let changed = |v: &&T| last.as_ref() != Some(*v);
    let Some(next) = values.get(front).filter(changed).or_else(|| values.iter().find(changed)).cloned() else { return };
    for (app, value) in apps.iter_mut().zip(&values) {
        if *value != next {
            set(app, &next);
        }
    }
    *last = Some(next);
}

impl Windows {
    /// The app of the window in front (the root's when that is in front or unknown).
    pub fn front<'a>(&'a mut self, root: &'a mut SheetApp) -> &'a mut SheetApp {
        let focused = self.focused;
        match self.extras.iter_mut().find(|e| Some(e.id) == focused) {
            Some(e) => &mut e.app,
            None => root,
        }
    }

    /// The open windows: the root one first, then the others in the order they opened.
    pub fn ids(&self) -> Vec<ViewportId> {
        std::iter::once(ViewportId::ROOT).chain(self.extras.iter().filter(|e| !e.closed).map(|e| e.id)).collect()
    }

    /// The app of one window (`ViewportId::ROOT` is the root's).
    pub fn app<'a>(&'a mut self, root: &'a mut SheetApp, id: ViewportId) -> Option<&'a mut SheetApp> {
        if id == ViewportId::ROOT {
            return Some(root);
        }
        self.extras.iter_mut().find(|e| e.id == id && !e.closed).map(|e| &mut e.app)
    }

    /// Bring a window to the front.
    pub fn focus(&mut self, ctx: &egui::Context, id: ViewportId) {
        self.focused = (id != ViewportId::ROOT).then_some(id);
        ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
    }

    /// Open documents sent from outside the app (Finder, the Dock, `open -a`), each where Cmd+N
    /// would put a new workbook: a workbook already open comes to the front in its window; the
    /// window in front takes it if it holds nothing but an untouched blank workbook (Book1 at
    /// start-up, so the document that launched the app opens in the first window); otherwise it
    /// opens in a new window, made by `new_app` (from the window in front) shaped by `builder`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn open_documents(
        &mut self,
        ctx: &egui::Context,
        root: &mut SheetApp,
        paths: &[String],
        new_app: impl Fn(&SheetApp) -> SheetApp,
        builder: impl Fn() -> ViewportBuilder,
    ) {
        for path in paths {
            let owner = self.ids().into_iter().find(|id| self.app(root, *id).is_some_and(|a| a.has_document(path)));
            if let Some(id) = owner {
                if let Some(app) = self.app(root, id) {
                    app.open_path(path); // Makes it the window's active workbook.
                }
                self.focus(ctx, id);
                continue;
            }
            let front = self.front(root);
            if front.is_blank() {
                front.open_path(path);
                continue;
            }
            let mut app = new_app(front);
            app.open_path(path);
            if app.session.documents().is_empty() {
                // It didn't open: say why in the window in front rather than in an empty window.
                let message = app.message.take();
                let front = self.front(root);
                front.message = message.or(front.message.take());
                continue;
            }
            self.open(ctx, app, builder());
        }
    }

    /// Give every window the shared settings (theme, language, Options, recent files and
    /// functions) last changed in any of them (the front window's when several changed at once).
    /// Call once per frame before the windows run, and before saving the preferences.
    pub fn sync_shared(&mut self, root: &mut SheetApp) {
        let front = self.extras.iter().position(|e| Some(e.id) == self.focused).map_or(0, |i| i + 1);
        let mut apps: Vec<&mut SheetApp> = std::iter::once(root).chain(self.extras.iter_mut().map(|e| &mut e.app)).collect();
        let shared = &mut self.shared;
        sync_part(
            &mut shared.display,
            &mut apps,
            front,
            |a| (a.ui.dark, a.ui.system_theme, a.ui.language),
            |a, &(dark, system_theme, language)| {
                a.ui.dark = dark;
                a.ui.system_theme = system_theme;
                a.ui.language = language;
            },
        );
        sync_part(&mut shared.prefs, &mut apps, front, |a| a.session.prefs.clone(), |a, p| a.session.prefs.clone_from(p));
        sync_part(&mut shared.recent, &mut apps, front, |a| a.ui.recent.clone(), |a, r| a.ui.recent.clone_from(r));
        sync_part(&mut shared.recent_functions, &mut apps, front, |a| a.ui.recent_functions.clone(), |a, r| a.ui.recent_functions.clone_from(r));
    }

    /// Hand the control channel to the window in front (from whichever window holds it).
    pub fn route_control(&mut self, root: &mut SheetApp) {
        let rx =
            self.spare_control.take().or_else(|| root.control_rx.take()).or_else(|| self.extras.iter_mut().find_map(|e| e.app.control_rx.take()));
        if let Some(rx) = rx {
            self.front(root).control_rx = Some(rx);
        }
    }

    /// Open `app` in a new window shaped like `base`, cascaded from the window in front.
    pub fn open(&mut self, ctx: &egui::Context, mut app: SheetApp, base: ViewportBuilder) {
        app.multi_window = true;
        self.next_id += 1;
        let id = ViewportId::from_hash_of(("gridcraft-window", self.next_id));
        // Where the window in front is (one opened this frame isn't shown yet: where it will be).
        let front = match self.extras.iter().find(|e| Some(e.id) == self.focused) {
            Some(e) => e.outer.or_else(|| Some(Rect::from_min_size(e.builder.position?, e.builder.inner_size?))),
            None => ctx.input(|i| i.viewport().outer_rect),
        };
        let mut builder = base;
        if let Some(r) = front {
            builder = builder.with_position(r.min + vec2(28.0, 28.0)).with_inner_size(r.size());
        }
        self.focused = Some(id);
        self.extras.push(Extra { id, app, builder, outer: None, closed: false });
    }

    /// Close the window in front the way its close button does, so unsaved changes are asked about
    /// (the macOS menu bar's Close, Cmd+W).
    #[cfg(target_os = "macos")]
    pub fn close_front(&self, ctx: &egui::Context) {
        let id = self.focused.filter(|f| self.extras.iter().any(|e| e.id == *f && !e.closed)).unwrap_or(ViewportId::ROOT);
        ctx.send_viewport_cmd_to(id, ViewportCommand::Close);
    }

    /// Requests the windows made during the last frame: (new windows wanted, quit wanted).
    pub fn take_requests(&mut self) -> (usize, Option<Quit>) {
        let mut new = 0;
        let mut quit = None;
        for e in &mut self.extras {
            new += usize::from(std::mem::take(&mut e.app.new_window_request));
            quit = Quit::merge(quit, Quit::take(&mut e.app));
        }
        (new, quit)
    }

    /// Start quitting: every window closes, asking about unsaved changes one window at a time
    /// (or, for automation, discarding them).
    pub fn quit(&mut self, how: Quit, root: &mut SheetApp) {
        if how == Quit::Discard {
            root.quit_confirmed = true;
            for e in &mut self.extras {
                e.app.quit_confirmed = true;
            }
        }
        self.quitting = true;
        self.asked = None;
    }

    /// Per-frame bookkeeping for the root window, after the root app's `logic`: focus, the root
    /// window closing, and quitting. Call from the root viewport.
    pub fn root_logic(&mut self, ctx: &egui::Context, root: &mut SheetApp) {
        if ctx.input(|i| i.viewport().focused) == Some(true) {
            self.focused = None;
        }
        let root_closing = ctx.input(|i| i.viewport().close_requested()) && !root.has_unsaved();
        if root_closing && self.extras.iter().any(|e| !e.closed) {
            // Ending the root viewport would end the app: keep it, and (unless quitting) give it
            // the next window's workbooks, where that window was.
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if !self.quitting
                && let Some(i) = self.extras.iter().position(|e| !e.closed)
            {
                let e = self.extras.remove(i);
                if let Some(r) = e.outer {
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(r.min));
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(r.size()));
                }
                // The control channel belongs to the app, not to a window: keep it.
                let control = root.control_rx.take();
                *root = e.app;
                if control.is_some() {
                    root.control_rx = control;
                }
                root.quit_confirmed = false;
                self.focused = None;
            }
        }
        if self.quitting {
            self.step_quit(ctx, root);
        }
    }

    fn step_quit(&mut self, ctx: &egui::Context, root: &mut SheetApp) {
        for e in &mut self.extras {
            if !e.app.has_unsaved() {
                e.closed = true;
            }
        }
        let asking = root.asking_before_closing() || self.extras.iter().any(|e| !e.closed && e.app.asking_before_closing());
        if asking {
            return;
        }
        // The last prompt closed but its window still has unsaved changes: Cancel (or a cancelled
        // Save As). Stop quitting.
        if let Some(asked) = self.asked.take() {
            let still_unsaved = if asked == ViewportId::ROOT {
                root.has_unsaved()
            } else {
                self.extras.iter().any(|e| e.id == asked && !e.closed && e.app.has_unsaved())
            };
            if still_unsaved {
                // The root window can't close while others are open, so one answered with Don't
                // Save is still here: it asks again next time rather than closing without asking.
                root.quit_confirmed = false;
                self.quitting = false;
                return;
            }
        }
        let next =
            if root.has_unsaved() { Some(ViewportId::ROOT) } else { self.extras.iter().find(|e| !e.closed && e.app.has_unsaved()).map(|e| e.id) };
        match next {
            Some(id) => {
                if id == ViewportId::ROOT {
                    root.ask_before_closing();
                } else if let Some(e) = self.extras.iter_mut().find(|e| e.id == id) {
                    e.app.ask_before_closing();
                }
                ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
                self.asked = Some(id);
            }
            None => {
                // Everything saved or discarded: end the app.
                self.drop_closed();
                self.extras.clear();
                root.quit_confirmed = true;
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            }
        }
    }

    /// Draw the other windows. Call once per frame from the root viewport's `ui`.
    pub fn show(&mut self, ctx: &egui::Context) {
        self.drop_closed();
        let mut focused = None;
        for e in &mut self.extras {
            let builder = e.builder.clone();
            ctx.show_viewport_immediate(e.id, builder, |ui, _class| {
                let ctx = ui.ctx().clone();
                // Files dropped on this window open here (the root's raw-input hook only sees the
                // root window's input).
                for f in ctx.input(|i| i.raw.dropped_files.clone()) {
                    if let Some(p) = f.path().to_str().filter(|p| !p.is_empty()) {
                        e.app.open_path(p);
                    }
                }
                e.app.logic(&ctx);
                e.app.ui(ui);
                let (has_focus, outer, close) =
                    ctx.input(|i| (i.viewport().focused == Some(true), i.viewport().outer_rect, i.viewport().close_requested()));
                if has_focus {
                    focused = Some(e.id);
                }
                e.outer = outer.or(e.outer);
                // `logic` cancelled the close and opened the prompt if there are unsaved changes.
                if close && !e.app.has_unsaved() {
                    e.closed = true;
                }
            });
        }
        if focused.is_some() {
            self.focused = focused;
        }
        if self.focused.is_some_and(|f| !self.extras.iter().any(|e| e.id == f && !e.closed)) {
            self.focused = None;
        }
        self.drop_closed();
    }

    /// Forget the windows that closed (their workbooks, and with them their Python workers, go
    /// too), keeping the control channel if one of them held it.
    fn drop_closed(&mut self) {
        for e in self.extras.iter_mut().filter(|e| e.closed) {
            if let Some(rx) = e.app.control_rx.take() {
                self.spare_control = Some(rx);
            }
        }
        self.extras.retain(|e| !e.closed);
    }
}

/// How the app quits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quit {
    /// Ask about each window's unsaved changes (Cmd+Q, the Quit menu item, the Dock's Quit).
    Ask,
    /// Close everything without asking (automation's `app.quit`).
    Discard,
}

impl Quit {
    /// The quit `app` asked for since the last frame, if any.
    pub fn take(app: &mut SheetApp) -> Option<Quit> {
        let discard = std::mem::take(&mut app.quit_all_discard);
        std::mem::take(&mut app.quit_all_request).then_some(if discard { Quit::Discard } else { Quit::Ask })
    }

    /// Two requests in one frame: discarding wins.
    pub fn merge(a: Option<Quit>, b: Option<Quit>) -> Option<Quit> {
        match (a, b) {
            (Some(Quit::Discard), _) | (_, Some(Quit::Discard)) => Some(Quit::Discard),
            (a, b) => a.or(b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::Session;
    use serde_json::json;

    fn blank() -> SheetApp {
        let mut s = Session::new();
        s.new_workbook();
        SheetApp::new(s, Default::default())
    }

    fn empty(_front: &SheetApp) -> SheetApp {
        SheetApp::new(Session::new(), Default::default())
    }

    /// A saved workbook with `text` in A1.
    fn book(name: &str, text: &str) -> String {
        let path = std::env::temp_dir().join(format!("gridcraft-windows-{}-{name}.xlsx", std::process::id()));
        let path = path.to_str().unwrap().to_string();
        let mut s = Session::new();
        s.new_workbook();
        s.execute("cell.set", json!({"cell": "A1", "input": text})).unwrap();
        s.execute("file.save", json!({"path": path})).unwrap();
        path
    }

    fn paths(app: &SheetApp) -> Vec<String> {
        app.session.documents().iter().filter_map(|d| d.path.clone()).collect()
    }

    #[test]
    fn finder_opens_go_to_a_blank_window_then_to_new_windows() {
        let ctx = egui::Context::default();
        let (a, b) = (book("a", "1"), book("b", "2"));
        let mut root = blank();
        let mut w = Windows::default();
        // The document that launched the app replaces Book1 in the first window.
        w.open_documents(&ctx, &mut root, std::slice::from_ref(&a), empty, ViewportBuilder::default);
        assert_eq!(paths(&root), vec![a.clone()]);
        assert_eq!(w.ids().len(), 1);
        // The next one gets its own window, which comes to the front.
        w.open_documents(&ctx, &mut root, std::slice::from_ref(&b), empty, ViewportBuilder::default);
        assert_eq!(w.ids().len(), 2);
        assert_eq!(paths(w.front(&mut root)), vec![b.clone()]);
        assert!(w.front(&mut root).multi_window);
        // Opening one that is already open brings its window forward without another window.
        w.open_documents(&ctx, &mut root, std::slice::from_ref(&a), empty, ViewportBuilder::default);
        assert_eq!(w.ids().len(), 2);
        assert_eq!(paths(w.front(&mut root)), vec![a.clone()]);
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn several_documents_in_one_event_each_get_a_window() {
        let ctx = egui::Context::default();
        let docs = vec![book("c", "1"), book("d", "2"), book("e", "3")];
        let mut root = blank();
        let mut w = Windows::default();
        w.open_documents(&ctx, &mut root, &docs, empty, ViewportBuilder::default);
        assert_eq!(w.ids().len(), 3);
        assert_eq!(paths(&root), vec![docs[0].clone()]);
        for p in &docs {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn a_changed_window_is_not_replaced() {
        let ctx = egui::Context::default();
        let f = book("f", "1");
        let mut root = blank();
        root.run("cell.set", json!({"cell": "B2", "input": "typed"})).unwrap();
        let mut w = Windows::default();
        w.open_documents(&ctx, &mut root, std::slice::from_ref(&f), empty, ViewportBuilder::default);
        assert_eq!(root.session.documents().len(), 1);
        assert!(root.session.documents().iter().all(|d| d.path.is_none()));
        assert_eq!(paths(w.front(&mut root)), vec![f.clone()]);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn a_document_that_fails_to_open_says_so_in_the_front_window() {
        let ctx = egui::Context::default();
        let mut root = blank();
        root.run("cell.set", json!({"cell": "A1", "input": "x"})).unwrap();
        let mut w = Windows::default();
        let missing = std::env::temp_dir().join("gridcraft-windows-missing-file.xlsx").to_str().unwrap().to_string();
        w.open_documents(&ctx, &mut root, &[missing], empty, ViewportBuilder::default);
        assert_eq!(w.ids().len(), 1);
        assert!(root.message.is_some());
    }

    #[test]
    fn the_control_channel_follows_the_front_window_and_survives_it_closing() {
        let ctx = egui::Context::default();
        let mut root = blank();
        let (_tx, rx) = std::sync::mpsc::channel();
        root.control_rx = Some(rx);
        let mut w = Windows::default();
        w.open(&ctx, blank(), ViewportBuilder::default());
        w.route_control(&mut root);
        assert!(root.control_rx.is_none());
        assert!(w.front(&mut root).control_rx.is_some());
        // The front window closes: the channel is kept and goes back to the root window.
        if let Some(e) = w.extras.first_mut() {
            e.closed = true;
        }
        w.drop_closed();
        w.focused = None;
        w.route_control(&mut root);
        assert!(root.control_rx.is_some());
    }

    #[test]
    fn cancelling_quit_after_dont_save_keeps_asking_about_the_first_window() {
        let ctx = egui::Context::default();
        let mut root = blank();
        root.run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
        let mut other = blank();
        other.run("cell.set", json!({"cell": "A1", "input": "2"})).unwrap();
        let mut w = Windows::default();
        w.open(&ctx, other, ViewportBuilder::default());
        w.quit(Quit::Ask, &mut root);
        w.root_logic(&ctx, &mut root);
        assert!(root.asking_before_closing());
        // Don't Save in the first window: the next window is asked.
        root.dialog = None;
        root.quit_confirmed = true;
        w.root_logic(&ctx, &mut root);
        assert!(w.front(&mut root).asking_before_closing());
        // Cancel there: quitting stops, and the first window's changes are asked about again.
        w.front(&mut root).dialog = None;
        w.root_logic(&ctx, &mut root);
        assert!(!w.quitting);
        assert!(root.has_unsaved());
        assert_eq!(w.ids().len(), 2);
    }

    #[test]
    fn quitting_asks_about_an_unsaved_macro_draft() {
        let ctx = egui::Context::default();
        let mut root = blank();
        let mut w = Windows::default();
        w.open(&ctx, blank(), ViewportBuilder::default());
        w.front(&mut root).open_dialog("macroEditor", json!({}));
        if let Some(ed) = w.front(&mut root).macros.editor.as_mut() {
            ed.draft.steps.push(gridcraft_engine::model::Step::command("cell.set", json!({"cell": "A1", "input": "1"})));
            ed.dirty = true;
        }
        w.quit(Quit::Ask, &mut root);
        w.root_logic(&ctx, &mut root);
        // The first window has nothing to lose; the one with the draft asks.
        assert!(!root.asking_before_closing());
        assert!(w.front(&mut root).asking_before_closing());
        assert_eq!(w.ids().len(), 2);
    }

    #[test]
    fn theme_and_language_are_shared_by_every_window() {
        let ctx = egui::Context::default();
        let mut root = blank();
        let mut w = Windows::default();
        w.open(&ctx, blank(), ViewportBuilder::default());
        w.sync_shared(&mut root);
        // The window in front turns dark: every window follows.
        w.front(&mut root).ui.dark = true;
        w.sync_shared(&mut root);
        assert!(root.ui.dark);
        // A window behind changes the language: the others follow it too.
        root.ui.language = gridcraft_ui_egui::i18n::Language::Ja;
        w.sync_shared(&mut root);
        assert_eq!(w.front(&mut root).ui.language, gridcraft_ui_egui::i18n::Language::Ja);
        assert!(w.front(&mut root).ui.dark);
    }

    #[test]
    fn recents_and_preferences_are_shared_by_every_window() {
        let ctx = egui::Context::default();
        let mut root = blank();
        let mut w = Windows::default();
        w.open(&ctx, blank(), ViewportBuilder::default());
        w.open(&ctx, blank(), ViewportBuilder::default());
        w.sync_shared(&mut root);
        // A file opened in the window in front is a recent file in every window.
        let f = book("recent", "1");
        w.front(&mut root).open_path(&f);
        w.sync_shared(&mut root);
        for id in w.ids() {
            assert_eq!(w.app(&mut root, id).map(|a| a.ui.recent.clone()), Some(vec![f.clone()]));
        }
        // Options changed in a window behind reach the others, and so do recently used functions.
        root.session.prefs.r1c1 = true;
        root.ui.recent_functions.insert(0, "XLOOKUP".into());
        w.sync_shared(&mut root);
        for id in w.ids() {
            let app = w.app(&mut root, id).unwrap();
            assert!(app.session.prefs.r1c1);
            assert_eq!(app.ui.recent_functions.first().map(String::as_str), Some("XLOOKUP"));
        }
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn automation_quit_discards_and_wins_over_asking() {
        assert_eq!(Quit::merge(Some(Quit::Ask), Some(Quit::Discard)), Some(Quit::Discard));
        assert_eq!(Quit::merge(None, Some(Quit::Ask)), Some(Quit::Ask));
        assert_eq!(Quit::merge(None, None), None);
        let mut app = blank();
        assert_eq!(Quit::take(&mut app), None);
        app.quit_all_request = true;
        app.quit_all_discard = true;
        assert_eq!(Quit::take(&mut app), Some(Quit::Discard));
        assert!(!app.quit_all_request && !app.quit_all_discard);
        let mut root = blank();
        root.run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
        let mut w = Windows::default();
        let ctx = egui::Context::default();
        let mut other = blank();
        other.run("cell.set", json!({"cell": "A1", "input": "2"})).unwrap();
        w.open(&ctx, other, ViewportBuilder::default());
        w.quit(Quit::Discard, &mut root);
        assert!(!root.has_unsaved());
        assert!(!w.front(&mut root).has_unsaved());
    }
}
