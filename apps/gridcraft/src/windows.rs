//! Several workbook windows in one process (File › New, Cmd+N), so the app has one Dock icon and
//! one Quit. The first window is eframe's root viewport; each further window is an immediate
//! viewport with its own [`SheetApp`].
//!
//! Closing the root window while others are open hands the root to the next window (it moves to
//! where that window was), because closing eframe's root viewport ends the app. Quit asks each
//! window with unsaved changes in turn; Cancel on any of them stops quitting.

use egui::{Rect, ViewportBuilder, ViewportCommand, ViewportId, vec2};
use gridcraft_ui_egui::SheetApp;

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

    /// Open `app` in a new window shaped like `base`, cascaded from the window in front.
    pub fn open(&mut self, ctx: &egui::Context, mut app: SheetApp, base: ViewportBuilder) {
        app.multi_window = true;
        self.next_id += 1;
        let id = ViewportId::from_hash_of(("gridcraft-window", self.next_id));
        let front = match self.extras.iter().find(|e| Some(e.id) == self.focused) {
            Some(e) => e.outer,
            None => ctx.input(|i| i.viewport().outer_rect),
        };
        let mut builder = base;
        if let Some(r) = front {
            builder = builder.with_position(r.min + vec2(28.0, 28.0)).with_inner_size(r.size());
        }
        self.focused = Some(id);
        self.extras.push(Extra { id, app, builder, outer: None, closed: false });
    }

    /// Requests the windows made during the last frame: (new windows wanted, quit wanted).
    pub fn take_requests(&mut self) -> (usize, bool) {
        let mut new = 0;
        let mut quit = false;
        for e in &mut self.extras {
            new += usize::from(std::mem::take(&mut e.app.new_window_request));
            quit |= std::mem::take(&mut e.app.quit_all_request);
        }
        (new, quit)
    }

    /// Start quitting: every window closes, asking about unsaved changes one window at a time.
    pub fn quit(&mut self) {
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
                *root = e.app;
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
                self.extras.clear();
                root.quit_confirmed = true;
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            }
        }
    }

    /// Draw the other windows. Call once per frame from the root viewport's `ui`.
    pub fn show(&mut self, ctx: &egui::Context) {
        self.extras.retain(|e| !e.closed);
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
        self.extras.retain(|e| !e.closed);
    }
}
