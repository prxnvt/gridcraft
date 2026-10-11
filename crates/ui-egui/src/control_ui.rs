//! Form controls in the window, apart from drawing them: placing a new control (draw it on the
//! grid, or click for the default size), the control's context menu (Edit Text, Assign Action,
//! Format Control, Delete), and the Assign Action and Format Control dialogs. Everything goes
//! through the `insert.*` and `control.*` commands.

use egui::{Pos2, Rect, Stroke, StrokeKind, Ui, vec2};
use gridcraft_engine::model::{ControlAction, ControlKind, FormControl, MacroInput, ObjKind, ObjRef, Sheet};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::{Dialog, ref_edit};
use crate::grid::Geo;
use crate::theme::Tokens;

/// The control `id` on the active sheet.
pub fn control(app: &SheetApp, id: u32) -> Option<FormControl> {
    app.session.active()?.wb.active()?.controls.iter().find(|c| c.id == id).cloned()
}

fn insert_id(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::Button => "insert.button",
        ControlKind::Slider => "insert.slider",
        ControlKind::Spin => "insert.spinButton",
    }
}

/// Sheet points (from the sheet's top-left) of a screen position.
fn sheet_point(geo: &Geo, sh: &Sheet, p: Pos2) -> (f32, f32) {
    let x = sh.col_left(geo.fc) as f32 + geo.scroll.x + (p.x - geo.cells.left() - geo.frozen_w) / geo.z;
    let y = sh.row_top(geo.fr) as f32 + geo.scroll.y + (p.y - geo.cells.top() - geo.frozen_h) / geo.z;
    (x.max(0.0), y.max(0.0))
}

/// Insert › Controls placement: while a kind is armed, a drag on the grid draws the control and
/// a click places it at its default size. Escape disarms. True when it used the pointer.
pub fn place(app: &mut SheetApp, ui: &Ui, resp: &egui::Response, geo: &Geo, sh: &Sheet) -> bool {
    let Some(kind) = app.grid.placing else {
        app.grid.place_start = None;
        return false;
    };
    let ctx = ui.ctx().clone();
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.grid.placing = None;
        app.grid.place_start = None;
        return true;
    }
    let (pointer, pos) = ctx.input(|i| (i.pointer.clone(), i.pointer.interact_pos().or(i.pointer.hover_pos())));
    if pos.is_some_and(|p| geo.cells.contains(p)) {
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if pointer.primary_pressed()
        && resp.contains_pointer()
        && let Some(p) = pointer.press_origin().or(pos).filter(|p| geo.cells.contains(*p))
    {
        app.grid.place_start = Some(p);
    }
    let Some(start) = app.grid.place_start else { return resp.contains_pointer() && pointer.any_down() };
    let end = pos.unwrap_or(start);
    let rect = Rect::from_two_pos(start, end);
    if pointer.primary_down() {
        let t = Tokens::get(&ctx);
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, t.sheet_control_border), StrokeKind::Inside);
        return true;
    }
    app.grid.place_start = None;
    app.grid.placing = None;
    let (x0, y0) = sheet_point(geo, sh, rect.min);
    let (x1, y1) = sheet_point(geo, sh, rect.max);
    let cell = gridcraft_engine::core::CellRef::new(sh.row_at(y0 as f64), sh.col_at(x0 as f64));
    let mut params = json!({"at": cell.a1(), "dx": x0 - sh.col_left(cell.col) as f32, "dy": y0 - sh.row_top(cell.row) as f32});
    // A drag sets the size (and a slider's orientation); a click keeps the default size.
    if rect.width() >= 6.0 || rect.height() >= 6.0 {
        params["width"] = json!((x1 - x0).max(8.0));
        params["height"] = json!((y1 - y0).max(8.0));
        if kind == ControlKind::Slider {
            params["horizontal"] = json!(rect.width() >= rect.height());
        }
    }
    match app.run(insert_id(kind), params) {
        Ok(v) => {
            if let Some(id) = v["control"].as_u64().and_then(|id| u32::try_from(id).ok()) {
                app.selected_object = Some(ObjRef::new(ObjKind::Control, id));
                // A new button asks what it should run, as Excel's Assign Macro does.
                if kind == ControlKind::Button {
                    app.open_dialog("assignAction", json!({"id": id}));
                }
            }
        }
        Err(e) => app.message = Some(("Insert Control".into(), crate::clean_error(&e))),
    }
    true
}

/// Opens a control's editor (double-click): a button's text, else Format Control.
pub fn edit(app: &mut SheetApp, id: u32) {
    match control(app, id).map(|c| c.kind) {
        Some(ControlKind::Button) => app.open_dialog("formatControl", json!({"id": id, "textOnly": true})),
        Some(_) => app.open_dialog("formatControl", json!({"id": id})),
        None => {}
    }
}

/// The context menu of a selected control.
pub fn menu(app: &mut SheetApp, ui: &mut Ui) {
    let Some((id, at)) = app.grid.control_menu else { return };
    let Some(c) = control(app, id) else {
        app.grid.control_menu = None;
        return;
    };
    let mut close = false;
    let resp = egui::Area::new(egui::Id::new("control_context")).fixed_pos(at).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_min_width(180.0);
            let item = |ui: &mut Ui, label: &str| ui.add(egui::Button::new(label).frame(false).min_size(vec2(170.0, 20.0))).clicked();
            if c.kind == ControlKind::Button {
                if item(ui, "Edit Text") {
                    close = true;
                    app.open_dialog("formatControl", json!({"id": id, "textOnly": true}));
                }
                if item(ui, "Assign Action…") {
                    close = true;
                    app.open_dialog("assignAction", json!({"id": id}));
                }
            }
            if item(ui, "Format Control…") {
                close = true;
                app.open_dialog("formatControl", json!({"id": id}));
            }
            ui.separator();
            if item(ui, "Delete") {
                close = true;
                app.run_or_alert("object.delete", json!({"kind": "control", "id": id}));
            }
        });
    });
    let outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !resp.response.rect.contains(p)));
    if close || outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.grid.control_menu = None;
    }
}

// ---------------------------------------------------------------- Assign Action

/// Assign Action: a macro (on the selection or a fixed range, with an optional output cell, and
/// New or Record to make one) or a command with its parameters.
pub fn open_assign(app: &SheetApp, params: &Json) -> Option<Dialog> {
    let id = params["id"].as_u64().and_then(|i| u32::try_from(i).ok())?;
    let c = control(app, id)?;
    let mut v =
        json!({"id": id, "mode": "macro", "macro": "", "input": "selection", "range": "", "output": "", "command": "", "params": "{}", "search": ""});
    match &c.action {
        Some(ControlAction::Macro { name, input, output }) => {
            v["macro"] = json!(name);
            if let MacroInput::Fixed(r) = input {
                v["input"] = json!("fixed");
                v["range"] = json!(r);
            }
            v["output"] = json!(output.clone().unwrap_or_default());
        }
        Some(ControlAction::Command { id, params }) => {
            v["mode"] = json!("command");
            v["command"] = json!(id);
            v["params"] = json!(serde_json::to_string_pretty(params).unwrap_or_else(|_| "{}".into()));
        }
        None => {
            // A new button suggests a new macro named after it, as Excel does.
            v["macro"] = json!(crate::macro_ui::macro_list(app).first().map(|m| m.0.clone()).unwrap_or_default());
        }
    }
    Some(Dialog::custom("assignAction", &format!("Assign Action: {}", c.display_name()), v))
}

fn val(d: &Dialog, key: &str) -> String {
    d.values.get(key).and_then(Json::as_str).unwrap_or("").to_string()
}

/// The `control.assign` params the dialog describes, or why it can't yet.
pub fn assign_params(d: &Dialog) -> Result<Json, String> {
    let id = d.values.get("id").cloned().unwrap_or(Json::Null);
    if val(d, "mode") == "command" {
        let command = val(d, "command");
        if command.trim().is_empty() {
            return Err("Choose a command.".into());
        }
        let text = val(d, "params");
        let params: Json = if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&text).map_err(|e| format!("The parameters aren't valid JSON: {e}"))?
        };
        if !params.is_object() {
            return Err("The parameters must be a JSON object, such as {\"range\": \"A1:B2\"}.".into());
        }
        return Ok(json!({"id": id, "command": command.trim(), "params": params}));
    }
    let name = val(d, "macro");
    if name.trim().is_empty() {
        return Err("Type or choose a macro name.".into());
    }
    let input = if val(d, "input") == "fixed" { val(d, "range") } else { "selection".into() };
    if input.trim().is_empty() {
        return Err("Choose the range the macro always runs on.".into());
    }
    Ok(json!({"id": id, "macro": name.trim(), "input": input.trim(), "output": val(d, "output").trim()}))
}

pub fn assign_dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let mut mode = val(d, "mode");
    ui.horizontal(|ui| {
        ui.radio_value(&mut mode, "macro".to_string(), "Run a macro");
        ui.radio_value(&mut mode, "command".to_string(), "Run a command");
    });
    d.values.insert("mode".into(), json!(mode));
    ui.add_space(4.0);
    let macros = crate::macro_ui::macro_list(app);
    if mode == "macro" {
        let mut name = val(d, "macro");
        ui.label("Macro name:");
        ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).id_salt("assign_macro"));
        egui::Frame::NONE.fill(t.input_bg).stroke(Stroke::new(1.0, t.input_border)).inner_margin(2.0).show(ui, |ui| {
            ui.set_min_size(vec2(ui.available_width(), 110.0));
            egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                if macros.is_empty() {
                    ui.label(egui::RichText::new("No macros yet. Type a name and choose New or Record.").italics().color(t.text_dim));
                }
                for (n, desc, _) in &macros {
                    let r =
                        ui.add(egui::Button::selectable(n.eq_ignore_ascii_case(name.trim()), n.as_str()).min_size(vec2(ui.available_width(), 20.0)));
                    let r = if desc.is_empty() { r } else { r.on_hover_text(desc) };
                    if r.clicked() {
                        name = n.clone();
                    }
                }
            });
        });
        let exists = macros.iter().any(|(n, ..)| n.eq_ignore_ascii_case(name.trim()));
        d.values.insert("macro".into(), json!(name));
        ui.horizontal(|ui| {
            let fresh = !exists && !name.trim().is_empty();
            let new = ui.add_enabled(fresh, egui::Button::new("New…")).on_hover_text("Write this macro step by step, then come back");
            if new.clicked() && assign(app, d) {
                *open = false;
                app.dialog_next = Dialog::open(app, "macroEditor", json!({"new": name.trim()}));
            }
            let record = ui
                .add_enabled(fresh && !app.session.is_recording(), egui::Button::new("Record…"))
                .on_hover_text("Record this macro from what you do");
            if record.clicked() && assign(app, d) {
                *open = false;
                crate::macro_ui::record_named(app, name.trim());
            }
        });
        ui.add_space(4.0);
        let mut input = val(d, "input");
        ui.label("Run it on:");
        ui.radio_value(&mut input, "selection".to_string(), "The selection when the button is pressed");
        ui.radio_value(&mut input, "fixed".to_string(), "Always this range:");
        d.values.insert("input".into(), json!(input));
        if input == "fixed" {
            ref_edit(app, ui, d, "range", 360.0);
        }
        ui.label("Output to (optional; the macro's own output otherwise):");
        ref_edit(app, ui, d, "output", 360.0);
    } else {
        let mut search = val(d, "search");
        let command = val(d, "command");
        ui.label("Command:");
        ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search commands").desired_width(f32::INFINITY).id_salt("assign_search"));
        egui::Frame::NONE.fill(t.input_bg).stroke(Stroke::new(1.0, t.input_border)).inner_margin(2.0).show(ui, |ui| {
            ui.set_min_size(vec2(ui.available_width(), 130.0));
            egui::ScrollArea::vertical().max_height(130.0).show(ui, |ui| {
                for (cid, label, place) in crate::macro_editor::picker_items(&search).into_iter().filter(|(i, ..)| !i.starts_with('@')) {
                    let text = if place.is_empty() { format!("{label}  ({cid})") } else { format!("{label}  ({cid})  {place}") };
                    if ui.add(egui::Button::selectable(cid == command, text).min_size(vec2(ui.available_width(), 20.0))).clicked() {
                        d.values.insert("command".into(), json!(cid));
                        let doc = gridcraft_engine::find_command(&cid).map(|c| c.params).unwrap_or("");
                        d.values.insert("doc".into(), json!(doc));
                    }
                }
            });
        });
        d.values.insert("search".into(), json!(search));
        let doc = val(d, "doc");
        if !doc.is_empty() {
            ui.label(egui::RichText::new(doc).small().color(t.text_dim));
        }
        ui.label("Parameters (JSON):");
        let mut params = val(d, "params");
        ui.add(egui::TextEdit::multiline(&mut params).code_editor().desired_rows(3).desired_width(f32::INFINITY).id_salt("assign_params"));
        d.values.insert("params".into(), json!(params));
    }
    let problem = assign_params(d).err();
    if let Some(p) = &problem {
        ui.label(egui::RichText::new(p).color(t.text_dim));
    }
    ui.add_space(6.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(" Cancel ").clicked() {
            *open = false;
        }
        if ui.add_enabled(problem.is_none(), egui::Button::new("   OK   ")).clicked() && assign(app, d) {
            *open = false;
        }
    });
}

/// Runs `control.assign` from the dialog. False (with the error shown) when it failed.
fn assign(app: &mut SheetApp, d: &Dialog) -> bool {
    let params = match assign_params(d) {
        Ok(p) => p,
        Err(e) => {
            app.message = Some(("Assign Action".into(), e));
            return false;
        }
    };
    match app.run("control.assign", params) {
        Ok(_) => true,
        Err(e) => {
            app.message = Some(("Assign Action".into(), crate::clean_error(&e)));
            false
        }
    }
}

// ---------------------------------------------------------------- Format Control

/// Format Control: a slider's or spin button's cell link, value and limits (orientation and page
/// change for sliders); a button's text. `textOnly` shows just the text (Edit Text).
pub fn open_format(app: &SheetApp, params: &Json) -> Option<Dialog> {
    let id = params["id"].as_u64().and_then(|i| u32::try_from(i).ok())?;
    let c = control(app, id)?;
    let text_only = params["textOnly"].as_bool() == Some(true) && c.kind == ControlKind::Button;
    let v = json!({
        "id": id,
        "textOnly": text_only,
        "text": c.text,
        "name": c.name,
        "value": c.value.to_string(),
        "min": c.min.to_string(),
        "max": c.max.to_string(),
        "step": c.step.to_string(),
        "page": c.page.to_string(),
        "link": c.link.clone().unwrap_or_default(),
        "horizontal": c.horizontal,
        "print": c.print,
        "locked": c.locked,
    });
    let title = if text_only { "Edit Text".to_string() } else { format!("Format Control: {}", c.display_name()) };
    Some(Dialog::custom("formatControl", &title, v))
}

/// The `control.format` params the dialog describes, or the first problem with them (the same
/// rules the engine checks, so OK is only offered when it will work).
pub fn format_params(app: &SheetApp, d: &Dialog) -> Result<Json, String> {
    let id = d.values.get("id").and_then(Json::as_u64).and_then(|i| u32::try_from(i).ok()).ok_or("The control is gone.")?;
    let c = control(app, id).ok_or("The control is gone.")?;
    let flag = |k: &str| d.values.get(k).and_then(Json::as_bool).unwrap_or(false);
    if d.values.get("textOnly").and_then(Json::as_bool) == Some(true) {
        return Ok(json!({"id": id, "text": val(d, "text")}));
    }
    let mut out = json!({"id": id, "name": val(d, "name"), "print": flag("print"), "locked": flag("locked")});
    if c.kind == ControlKind::Button {
        out["text"] = json!(val(d, "text"));
        return Ok(out);
    }
    let mut draft = c.clone();
    for (key, label, slot) in [
        ("min", "Minimum value", &mut draft.min),
        ("max", "Maximum value", &mut draft.max),
        ("step", "Incremental change", &mut draft.step),
        ("page", "Page change", &mut draft.page),
    ] {
        if key == "page" && c.kind != ControlKind::Slider {
            continue;
        }
        let text = val(d, key);
        *slot = whole(&text).ok_or_else(|| format!("{label} must be a whole number from 0 to {}.", gridcraft_engine::model::CONTROL_MAX))?;
        out[key] = json!(*slot);
    }
    draft.check_numbers()?;
    let value =
        whole(&val(d, "value")).ok_or_else(|| format!("Current value must be a whole number from 0 to {}.", gridcraft_engine::model::CONTROL_MAX))?;
    if value < draft.min || value > draft.max {
        return Err(format!("Current value must be between {} and {}.", draft.min, draft.max));
    }
    out["value"] = json!(value);
    out["link"] = json!(val(d, "link").trim());
    if c.kind == ControlKind::Slider {
        out["horizontal"] = json!(flag("horizontal"));
    }
    Ok(out)
}

fn whole(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|v| *v <= gridcraft_engine::model::CONTROL_MAX)
}

pub fn format_dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let Some(c) = d.values.get("id").and_then(Json::as_u64).and_then(|i| u32::try_from(i).ok()).and_then(|id| control(app, id)) else {
        *open = false;
        return;
    };
    let text_only = d.values.get("textOnly").and_then(Json::as_bool) == Some(true);
    let field = |ui: &mut Ui, d: &mut Dialog, key: &str, label: &str| {
        let mut s = val(d, key);
        ui.label(label);
        ui.add(egui::TextEdit::singleline(&mut s).desired_width(90.0).id_salt(("format_control", key.to_string())));
        ui.end_row();
        d.values.insert(key.into(), json!(s));
    };
    if c.kind == ControlKind::Button {
        let mut text = val(d, "text");
        ui.label("Text:");
        ui.add(egui::TextEdit::multiline(&mut text).desired_rows(2).desired_width(f32::INFINITY).id_salt("control_text"));
        d.values.insert("text".into(), json!(text));
    } else {
        egui::Grid::new("format_control_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            field(ui, d, "value", "Current value:");
            field(ui, d, "min", "Minimum value:");
            field(ui, d, "max", "Maximum value:");
            field(ui, d, "step", "Incremental change:");
            if c.kind == ControlKind::Slider {
                field(ui, d, "page", "Page change:");
            }
        });
        ui.add_space(4.0);
        ui.label("Cell link:");
        ref_edit(app, ui, d, "link", 300.0);
        if c.kind == ControlKind::Slider {
            let mut h = d.values.get("horizontal").and_then(Json::as_bool).unwrap_or(true);
            ui.horizontal(|ui| {
                ui.label("Orientation:");
                ui.radio_value(&mut h, true, "Horizontal");
                ui.radio_value(&mut h, false, "Vertical");
            });
            d.values.insert("horizontal".into(), json!(h));
        }
    }
    if !text_only {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Name:");
            let mut name = val(d, "name");
            ui.add(egui::TextEdit::singleline(&mut name).hint_text(c.display_name()).desired_width(200.0).id_salt("control_name"));
            d.values.insert("name".into(), json!(name));
        });
        for (key, label) in [("locked", "Locked (when the sheet is protected)"), ("print", "Print object")] {
            let mut on = d.values.get(key).and_then(Json::as_bool).unwrap_or(true);
            ui.checkbox(&mut on, label);
            d.values.insert(key.into(), json!(on));
        }
    }
    let params = format_params(app, d);
    if let Err(p) = &params {
        ui.label(egui::RichText::new(p).color(t.danger));
    }
    ui.add_space(6.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(" Cancel ").clicked() {
            *open = false;
        }
        if ui.add_enabled(params.is_ok(), egui::Button::new("   OK   ")).clicked()
            && let Ok(p) = params
        {
            match app.run("control.format", p) {
                Ok(_) => *open = false,
                Err(e) => app.message = Some(("Format Control".into(), crate::clean_error(&e))),
            }
        }
    });
}
