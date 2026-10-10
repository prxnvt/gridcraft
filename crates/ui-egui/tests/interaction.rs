//! UI interaction tests: real egui input through the whole app (no GPU needed).

use egui::{Event, Key, Modifiers, PointerButton};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::{SheetApp, grid::Geo};
use serde_json::json;

fn harness(session: Session) -> egui_kittest::Harness<'static, SheetApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    h
}

fn blank() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

#[test]
fn vertical_scrollbar_pages_and_drags_without_changing_selection() {
    let mut h = harness(blank());
    let grid = h.state().grid.rect.unwrap();
    let cells = h.state().grid.cells_rect.unwrap();
    let p = egui::pos2(grid.right() + 7.0, cells.bottom() - 20.0);
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().view().scroll.y > 0.0, "clicking the scrollbar track scrolls down");
    assert_eq!(h.state().session.active().unwrap().selection.active, CellRef::new(0, 0));

    h.state_mut().view_mut().unwrap().scroll.y = 0.0;
    h.run_steps(2);
    let start = egui::pos2(grid.right() + 7.0, cells.top() + 8.0);
    h.input_mut().events.push(Event::PointerMoved(start));
    h.input_mut().events.push(Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let end = start + egui::vec2(0.0, 80.0);
    h.input_mut().events.push(Event::PointerMoved(end));
    h.run_steps(2);
    let dragged = h.state().view().scroll.y;
    assert!(dragged > 0.0, "dragging the thumb scrolls down");
    h.run_steps(3);
    assert_eq!(h.state().view().scroll.y, dragged, "holding the thumb still must not keep scrolling");
    h.input_mut().events.push(Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().selection.active, CellRef::new(0, 0));
}

fn key(h: &mut egui_kittest::Harness<'static, SheetApp>, k: Key, m: Modifiers) {
    h.input_mut().events.push(Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.input_mut().events.push(Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.run_steps(2);
}

fn text(h: &mut egui_kittest::Harness<'static, SheetApp>, t: &str) {
    h.input_mut().events.push(Event::Text(t.into()));
    h.run_steps(2);
}

fn value(h: &egui_kittest::Harness<'static, SheetApp>, a: &str) -> Value {
    h.state().session.active().and_then(|d| d.wb.active().map(|s| s.value(CellRef::parse(a).unwrap_or_default()))).unwrap_or_default()
}

fn click_cell(h: &mut egui_kittest::Harness<'static, SheetApp>, cell: &str) {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    let geo = Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll);
    let pos = geo.cell_rect(sheet, CellRef::parse(cell).unwrap()).center();
    h.input_mut().events.extend([
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(2);
}

#[test]
fn keyboard_navigation_does_not_replay_the_last_pointer_click() {
    let mut h = harness(blank());
    click_cell(&mut h, "B2");
    for (key_code, modifiers, expected) in [
        (Key::Enter, Modifiers::NONE, "B3"),
        (Key::Enter, Modifiers::NONE, "B4"),
        (Key::Enter, Modifiers::NONE, "B5"),
        (Key::Enter, Modifiers::SHIFT, "B4"),
        (Key::Tab, Modifiers::NONE, "C4"),
        (Key::Enter, Modifiers::NONE, "C5"),
    ] {
        key(&mut h, key_code, modifiers);
        let selection = &h.state().session.active().unwrap().selection;
        assert_eq!(selection.active.a1(), expected);
        assert_eq!(selection.anchor, selection.active);
        assert!(h.state().editor.is_none(), "navigation must not start editing at the old pointer location");
    }
    // A subsequent real pointer click must still select the clicked cell.
    click_cell(&mut h, "D6");
    assert_eq!(h.state().session.active().unwrap().selection.active.a1(), "D6");
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().session.active().unwrap().selection.active.a1(), "D7");
}

#[test]
fn space_toggles_the_active_checkbox_without_clicking_the_hovered_cell() {
    let mut session = blank();
    session.execute("insert.checkbox", json!({"range": "A1:B1"})).unwrap();
    let mut h = harness(session);
    click_cell(&mut h, "A1");
    assert_eq!(value(&h, "A1"), Value::Bool(true));
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    key(&mut h, Key::Space, Modifiers::NONE);
    assert_eq!(h.state().session.active().unwrap().selection.active.a1(), "B1");
    assert_eq!(value(&h, "A1"), Value::Bool(true), "the old pointer location must remain unchanged");
    assert_eq!(value(&h, "B1"), Value::Bool(true));
    assert!(h.state().editor.is_none());
}

#[test]
fn typing_enters_values_and_moves_down() {
    let mut h = harness(blank());
    text(&mut h, "42");
    assert!(h.state().editor.is_some(), "typing starts editing");
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(value(&h, "A1"), Value::Number(42.0));
    text(&mut h, "=A1*2");
    key(&mut h, Key::Tab, Modifiers::NONE);
    assert_eq!(value(&h, "A2"), Value::Number(84.0));
    let active = h.state().session.active().map(|d| d.selection.active.a1());
    assert_eq!(active.as_deref(), Some("B2"));
}

#[test]
fn arrows_escape_and_undo() {
    let mut h = harness(blank());
    key(&mut h, Key::ArrowDown, Modifiers::NONE);
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(h.state().session.active().map(|d| d.selection.active.a1()).as_deref(), Some("B2"));
    text(&mut h, "hello");
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(value(&h, "B2"), Value::Empty);
    text(&mut h, "x");
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(value(&h, "B2"), Value::from("x"));
    key(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(value(&h, "B2"), Value::Empty);
}

#[test]
fn shortcuts_format_and_extend() {
    let mut s = blank();
    s.execute("range.setValues", json!({"range": "A1", "values": [[1, 2], [3, 4]]})).unwrap();
    let mut h = harness(s);
    key(&mut h, Key::ArrowRight, Modifiers::SHIFT);
    key(&mut h, Key::ArrowDown, Modifiers::SHIFT);
    assert_eq!(h.state().session.active().map(|d| d.selection.a1()).as_deref(), Some("A1:B2"));
    key(&mut h, Key::B, Modifiers::COMMAND);
    let bold = h.state_mut().session.run("cell.get", json!({"cell": "B2"})).unwrap()["style"]["font"]["bold"].clone();
    assert_eq!(bold, json!(true));
    key(&mut h, Key::Delete, Modifiers::NONE);
    assert_eq!(value(&h, "A1"), Value::Empty);
}

#[test]
fn renders_every_ribbon_tab_without_panicking() {
    let mut s = Session::new();
    s.execute("file.new", json!({"sample": "sales"})).unwrap();
    let mut h = harness(s);
    for tab in ["Home", "Insert", "Draw", "Page Layout", "Formulas", "Data", "Review", "View", "Automate"] {
        h.state_mut().ui.ribbon_tab = tab.into();
        h.run_steps(2);
    }
    h.state_mut().ui.dark = true;
    h.run_steps(3);
}

#[test]
fn language_switch_translates_the_ribbon_and_persists() {
    use gridcraft_ui_egui::i18n::Language;
    let mut h = harness(blank());
    // English by default whatever the host's locale: only the desktop app reads the system language.
    assert_eq!(h.state().ui.language, Language::En);
    // The language commands are listed with the other UI commands.
    let ctx = h.ctx.clone();
    let gridcraft_ui_egui::control::Outcome::Done(listed) = gridcraft_ui_egui::control::handle(h.state_mut(), &ctx, "engine.commands", &json!({}))
    else {
        panic!("engine.commands returns a response");
    };
    let ids: Vec<&str> = listed["result"].as_array().into_iter().flatten().filter_map(|c| c["id"].as_str()).collect();
    for id in ["app.language.set", "app.language.english", "app.language.japanese"] {
        assert!(ids.contains(&id), "engine.commands lists {id}");
    }
    for (language, expected) in
        [(Language::Zh, "数据"), (Language::Ja, "データ"), (Language::Ko, "데이터"), (Language::Ru, "Данные"), (Language::PtBr, "Dados")]
    {
        h.state_mut().run("app.language.set", json!({"language": language.code()})).unwrap();
        h.run_steps(2); // every locale renders the ribbon without panicking
        assert_eq!(h.state().ui.language, language);
        assert_eq!(language.tr("Data"), expected);
        let restored: gridcraft_ui_egui::UiState = serde_json::from_str(&serde_json::to_string(&h.state().ui).unwrap()).unwrap();
        assert_eq!(restored.language, language);
    }
    h.state_mut().run("app.language.set", json!({"language": "en"})).unwrap();
    assert_eq!(h.state().ui.language, Language::En);
}

#[test]
fn dialogs_open_and_close() {
    let mut h = harness(blank());
    for name in [
        "formatCells",
        "insertFunction",
        "find",
        "sort",
        "nameManager",
        "dataValidation",
        "pasteSpecial",
        "commandSearch",
        "spelling",
        "goTo",
        "about",
    ] {
        h.state_mut().open_dialog(name, json!({}));
        h.run_steps(2);
        key(&mut h, Key::Escape, Modifiers::NONE);
        h.state_mut().dialog = None;
        h.state_mut().message = None;
    }
}

#[test]
fn about_renders_every_tab() {
    let mut h = harness(blank());
    for tab in ["About", "Contributors", "Models"] {
        h.state_mut().open_dialog("about", json!({"tab": tab}));
        h.run_steps(2);
        assert_eq!(h.state().dialog.as_ref().map(|d| d.tab.clone()).as_deref(), Some(tab));
        h.state_mut().dialog = None;
    }
}

#[test]
fn column_autocomplete_completes_on_enter() {
    let mut s = blank();
    s.execute("range.setValues", json!({"range": "A1", "values": [["North"], ["East"], ["Eastern"]]})).unwrap();
    s.execute("selection.set", json!({"cell": "A4"})).unwrap();
    let mut h = harness(s);
    text(&mut h, "No");
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(value(&h, "A4"), Value::from("North"));
    text(&mut h, "Ea"); // ambiguous: East / Eastern
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(value(&h, "A5"), Value::from("Ea"));
}

#[test]
fn general_numbers_fit_by_the_cells_own_font() {
    // 9pt marks in narrow columns (a stored width of 4, i.e. 28px; common in mark sheets): "100" fits.
    let mut s = blank();
    s.execute("range.setValues", json!({"range": "A1", "values": [[100, 123456789]]})).unwrap();
    s.execute("home.fontSize", json!({"range": "A1:B1", "size": 9})).unwrap();
    s.execute("home.columnWidth", json!({"cols": "A:B", "width": 28})).unwrap();
    s.execute("selection.set", json!({"cell": "C3"})).unwrap();
    let h = harness(s);
    let texts: Vec<String> = h
        .output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|t| t == "100"), "100 drawn in full: {texts:?}");
    // A number that really doesn't fit still shows #### (once: only B1).
    assert_eq!(texts.iter().filter(|t| t.starts_with('#')).count(), 1, "{texts:?}");
}

fn text_shapes(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<(egui::epaint::TextShape, egui::Rect)>) {
    match shape {
        egui::Shape::Text(t) => out.push((t.clone(), clip)),
        egui::Shape::Vec(v) => v.iter().for_each(|s| text_shapes(s, clip, out)),
        _ => {}
    }
}

#[test]
fn rotated_wrapped_text_is_drawn_rotated_inside_its_cell() {
    // Narrow columns with tall, upright headers (mark sheets): rotation and Wrap Text together.
    let header = "Term 1 : Drama : Continuous Assessment";
    let mut s = blank();
    s.execute("range.setValues", json!({"range": "A1", "values": [[header, header]]})).unwrap();
    s.execute("home.wrapText", json!({"range": "A1:B1", "on": true})).unwrap();
    s.execute("home.orientation", json!({"range": "A1", "angle": "up"})).unwrap();
    s.execute("home.orientation", json!({"range": "B1", "angle": "down"})).unwrap();
    s.execute("home.columnWidth", json!({"cols": "A:B", "chars": 6})).unwrap();
    s.execute("home.rowHeight", json!({"rows": "1:1", "height": 267})).unwrap();
    s.execute("selection.set", json!({"cell": "C3"})).unwrap();
    let h = harness(s);
    let mut found = Vec::new();
    for c in &h.output().shapes {
        text_shapes(&c.shape, c.clip_rect, &mut found);
    }
    let rotated: Vec<_> = found.iter().filter(|(t, _)| t.galley.job.text == header).collect();
    assert_eq!(rotated.len(), 2, "both headers drawn as one text block each");
    for (t, clip) in rotated {
        assert!((t.angle.abs() - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "drawn at ±90°, got {}", t.angle);
        // Wrapped along the row height into a couple of long lines, not across the narrow column.
        assert!(t.galley.rows.len() <= 3, "{} lines", t.galley.rows.len());
        let bb = t.visual_bounding_rect();
        assert!(bb.height() > bb.width(), "upright, not flat: {bb:?}");
        assert!(clip.expand(1.0).contains_rect(bb), "inside its cell: {bb:?} not in {clip:?}");
        // Upright text never spills into the neighbouring columns: the clip is the cell itself.
        assert!(clip.width() < 70.0, "clip spans one 6-character column, got {clip:?}");
    }
}

#[test]
fn ref_edit_takes_the_grid_selection_and_restores_it() {
    let mut s = blank();
    s.execute("selection.set", json!({"range": "B2"})).unwrap();
    let mut h = harness(s);
    h.state_mut().open_dialog("goalSeek", json!({}));
    h.run_steps(2);
    let field = |h: &egui_kittest::Harness<'static, SheetApp>, k: &str| {
        h.state().dialog.as_ref().and_then(|d| d.values.get(k).and_then(|v| v.as_str()).map(str::to_string)).unwrap_or_default()
    };
    assert_eq!(field(&h, "set"), "$B$2", "Set cell starts at the active cell");
    // Clicking into "By changing cell" makes it the pick target; the grid selection then fills it.
    let app = h.state_mut();
    if let Some(d) = app.dialog.as_mut() {
        d.pick = Some("changing".into());
        d.pick_seen = "$B$2".into();
        d.home = Some((0, "B2".into(), "B2".into()));
    }
    app.session.run("selection.set", json!({"range": "C3:D4"})).unwrap();
    h.run_steps(2);
    assert_eq!(field(&h, "changing"), "$C$3:$D$4");
    assert_eq!(field(&h, "set"), "$B$2", "only the pick target changes");
    // A selection on another sheet carries the sheet name.
    h.state_mut().session.run("home.insertSheet", json!({"name": "Data", "before": 1})).unwrap();
    h.state_mut().session.run("selection.set", json!({"range": "A5"})).unwrap();
    h.run_steps(2);
    assert!(field(&h, "changing").ends_with("!$A$5"), "{}", field(&h, "changing"));
    // Closing puts the original sheet and selection back.
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().dialog.is_none());
    let d = h.state().session.active().map(|d| (d.wb.active_sheet, d.selection.a1()));
    assert_eq!(d, Some((0, "B2".to_string())));
}

#[test]
fn confirm_question_reruns_the_command_on_ok() {
    assert_eq!(
        gridcraft_ui_egui::confirm_text("invalid parameters for `x`: confirm: Replace it? (run again with `overwrite: true`)").as_deref(),
        Some("Replace it?")
    );
    assert_eq!(gridcraft_ui_egui::confirm_text("no workbook open"), None);
    let mut h = harness(blank());
    h.state_mut().confirm = Some(gridcraft_ui_egui::Confirm {
        title: "Test".into(),
        text: "Replace it?".into(),
        command: "cell.set".into(),
        params: json!({"cell": "A1", "input": "5"}),
    });
    h.run_steps(2);
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().confirm.is_none());
    assert_eq!(value(&h, "A1"), Value::Number(5.0));
    // Cancel leaves things alone.
    h.state_mut().confirm = Some(gridcraft_ui_egui::Confirm {
        title: "Test".into(),
        text: "Replace it?".into(),
        command: "cell.set".into(),
        params: json!({"cell": "A1", "input": "6"}),
    });
    h.run_steps(2);
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().confirm.is_none());
    assert_eq!(value(&h, "A1"), Value::Number(5.0));
}

/// Counts up a little each frame until it reaches 50.
struct Ticker(u32);

impl gridcraft_engine::job::Job for Ticker {
    fn label(&self) -> String {
        "Ticker".into()
    }
    fn step(&mut self, _s: &mut Session, _b: gridcraft_engine::job::Budget) -> gridcraft_engine::job::Step {
        self.0 += 5;
        if self.0 >= 50 {
            gridcraft_engine::job::Step::Done(Ok(json!({"n": self.0})))
        } else {
            gridcraft_engine::job::Step::Continue(self.0.to_string())
        }
    }
    fn stop(&mut self, _s: &mut Session) -> gridcraft_engine::Result<serde_json::Value> {
        Ok(json!({"n": self.0}))
    }
}

#[test]
fn jobs_run_per_frame_and_escape_pauses_them() {
    use gridcraft_engine::job::JobState;
    let mut h = harness(blank());
    h.state_mut().session.start_job(Box::new(Ticker(0))).unwrap();
    h.run_steps(2);
    assert!(matches!(h.state().session.job_state, JobState::Running { .. }));
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(matches!(h.state().session.job_state, JobState::Paused { .. }), "{:?}", h.state().session.job_state);
    h.run_steps(3);
    assert!(matches!(h.state().session.job_state, JobState::Paused { .. }), "a paused job waits for an answer");
    // The pause question's Continue (Enter) resumes it; it then runs to the end.
    key(&mut h, Key::Enter, Modifiers::NONE);
    h.run_steps(12);
    assert!(h.state().session.job.is_none());
    assert!(h.state().message.is_none());
}

#[test]
fn data_table_dialog_creates_a_live_table() {
    let mut s = blank();
    s.execute("range.setValues", json!({"range": "B1", "values": [[2]]})).unwrap();
    s.execute("range.setValues", json!({"range": "D2", "values": [[1], [2], [3]]})).unwrap();
    s.execute("cell.set", json!({"cell": "E1", "input": "=B1*10"})).unwrap();
    s.execute("selection.set", json!({"range": "D1:E4"})).unwrap();
    let mut h = harness(s);
    h.state_mut().open_dialog("dataTable", json!({}));
    h.run_steps(2);
    if let Some(d) = h.state_mut().dialog.as_mut() {
        d.values.insert("columnInput".into(), json!("$B$1"));
    }
    h.run_steps(1);
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().dialog.is_none(), "{:?}", h.state().message);
    assert_eq!(value(&h, "E3"), Value::Number(20.0));
    h.state_mut().session.run("cell.set", json!({"cell": "E1", "input": "=B1*100"})).unwrap();
    h.run_steps(1);
    assert_eq!(value(&h, "E4"), Value::Number(300.0));
}

#[test]
fn data_analysis_runs_a_tool_from_its_form() {
    let mut s = blank();
    s.execute("range.setValues", json!({"range": "A1", "values": [["Score"], [1], [2], [3], [4]]})).unwrap();
    let mut h = harness(s);
    h.state_mut().open_dialog("dataAnalysis", json!({}));
    h.run_steps(2);
    // Pick Descriptive Statistics and press OK: its form replaces the list.
    if let Some(d) = h.state_mut().dialog.as_mut() {
        d.list_index = 5;
    }
    key(&mut h, Key::Enter, Modifiers::NONE);
    h.run_steps(2);
    let d = h.state().dialog.as_ref().map(|d| (d.name.clone(), d.title.clone()));
    assert_eq!(d, Some(("toolpak".to_string(), "Descriptive Statistics".to_string())));
    if let Some(d) = h.state_mut().dialog.as_mut() {
        d.values.insert("input".into(), json!("$A$1:$A$5"));
        d.values.insert("labels".into(), json!(true));
        d.values.insert("newSheetName".into(), json!("Stats"));
    }
    h.run_steps(1);
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().dialog.is_none(), "{:?}", h.state().message);
    let doc = h.state().session.active().unwrap();
    assert_eq!(doc.wb.active().unwrap().name, "Stats");
    assert_eq!(doc.wb.active().unwrap().value(CellRef::parse("B2").unwrap()), Value::Number(2.5));
}

#[test]
fn toolpak_forms_build_command_params() {
    use gridcraft_ui_egui::toolpak;
    // Every tool has a form.
    for (id, _) in gridcraft_engine::cmd::analysis::TOOLS {
        assert!(toolpak::form(id).is_some(), "{id}");
    }
    let f = toolpak::form("analysis.descriptiveStatistics").unwrap();
    let mut v = serde_json::Map::new();
    for (k, x) in [
        ("input", json!(" $A$1:$B$9 ")),
        ("groupedBy", json!("columns")),
        ("labels", json!(true)),
        ("summary", json!(true)),
        ("useConfidence", json!(true)),
        ("confidence", json!("90")),
        ("useKthLargest", json!(false)),
        ("kthLargest", json!("2")),
        ("outputMode", json!("range")),
        ("outputRange", json!("$D$1")),
    ] {
        v.insert(k.into(), x);
    }
    let p = toolpak::params(&f, &v);
    assert_eq!(p["input"], "$A$1:$B$9");
    assert_eq!(p["confidence"], 90.0);
    assert_eq!(p["kthLargest"], serde_json::Value::Null);
    assert_eq!(p["output"], "$D$1");
    v.insert("outputMode".into(), json!("newWorkbook"));
    assert_eq!(toolpak::params(&f, &v)["output"], "newWorkbook");
    v.insert("outputMode".into(), json!("newSheet"));
    assert_eq!(toolpak::params(&f, &v)["output"], "newSheet");
}

fn wyndor_session() -> Session {
    let mut s = blank();
    s.execute("range.setValues", json!({"range": "A1", "values": [["Doors", 0], ["Windows", 0]]})).unwrap();
    for (c, f) in [("C1", "=B1"), ("C2", "=2*B2"), ("C3", "=3*B1+2*B2"), ("D1", "=3*B1+5*B2")] {
        s.execute("cell.set", json!({"cell": c, "input": f})).unwrap();
    }
    s.execute("range.setValues", json!({"range": "E1", "values": [[4], [12], [18]]})).unwrap();
    s.execute(
        "data.solverSetModel",
        json!({"objective": "$D$1", "variables": "$B$1:$B$2", "method": "simplex", "constraints": [{"left": "$C$1:$C$3", "op": "<=", "right": "$E$1:$E$3"}]}),
    )
    .unwrap();
    s
}

#[test]
fn solver_runs_in_the_background_and_shows_results() {
    let mut h = harness(wyndor_session());
    h.state_mut().open_dialog("solver", json!({}));
    h.run_steps(2);
    let d = h.state().dialog.as_ref().map(|d| (d.name.clone(), d.values.get("objective").cloned()));
    assert_eq!(d, Some(("solver".to_string(), Some(json!("$D$1")))));
    // Solve (what the Solve button does), then let the job run frame by frame.
    h.state_mut().dialog = None;
    h.state_mut().session.run("data.solverStart", json!({})).unwrap();
    h.run_steps(10);
    let dlg = h.state().dialog.as_ref().map(|d| (d.name.clone(), d.values.get("status").cloned()));
    assert_eq!(dlg, Some(("solverResults".to_string(), Some(json!(0)))));
    assert_eq!(value(&h, "B2"), Value::Number(6.0));
}

#[test]
fn show_iteration_results_pauses_with_the_trial_dialog() {
    let mut s = wyndor_session();
    s.execute("data.solverSetModel", json!({"method": "grg", "options": {"showIterations": true}})).unwrap();
    let mut h = harness(s);
    h.state_mut().session.run("data.solverStart", json!({})).unwrap();
    for _ in 0..40 {
        h.run_steps(1);
        if h.state().dialog.is_some() {
            break;
        }
    }
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.clone()).as_deref(), Some("solverTrial"));
    // Stop: the best solution so far is offered in Solver Results.
    h.state_mut().dialog = None;
    h.state_mut().session.stop_job();
    h.state_mut().after_engine();
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.clone()).as_deref(), Some("solverResults"));
}

fn pivot_session() -> Session {
    let mut s = blank();
    let rows = json!([["Region", "Sales"], ["East", 100], ["West", 50], ["North", 30], ["East", 20]]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"]})).unwrap();
    s
}

/// The value beside a row label in the active sheet's PivotTable.
fn pivot_value(h: &egui_kittest::Harness<'static, SheetApp>, label: &str) -> Option<Value> {
    let d = h.state().session.active()?;
    let sh = d.wb.active()?;
    let r = sh.pivots.last()?.last_range?;
    let row = (r.start.row..=r.end.row).find(|row| sh.value(CellRef::new(*row, r.start.col)).display() == label)?;
    Some(sh.value(CellRef::new(row, r.start.col + 1)))
}

#[test]
fn pivot_field_button_sorts_and_filters() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(pivot_session());
    let (id, buttons) = {
        let sh = h.state().session.active().and_then(|d| d.wb.active()).unwrap();
        let pt = sh.pivots.last().unwrap();
        (pt.id, gridcraft_ui_egui::pivot_filter::buttons(sh))
    };
    // Row Labels has a button, unfiltered.
    assert_eq!(buttons, vec![(CellRef::new(2, 0), false)]);
    h.state_mut().grid.pivot_menu = Some((id, "Region".into(), egui::pos2(400.0, 200.0)));
    h.run_steps(2);
    h.get_by_label("Sort Z to A").click();
    h.run_steps(2);
    let first = h.state().session.active().and_then(|d| d.wb.active()).map(|s| s.value(CellRef::new(3, 0)).display());
    assert_eq!(first.as_deref(), Some("West"));
    // Uncheck West and press OK.
    h.state_mut().grid.pivot_menu = Some((id, "Region".into(), egui::pos2(400.0, 200.0)));
    h.run_steps(2);
    h.get_by_label("West").click();
    h.run_steps(1);
    h.get_by_label("   OK   ").click();
    h.run_steps(2);
    assert!(h.state().grid.pivot_menu.is_none());
    assert_eq!(pivot_value(&h, "West"), None);
    assert_eq!(pivot_value(&h, "Grand Total"), Some(Value::Number(150.0)));
    let sh = h.state().session.active().and_then(|d| d.wb.active()).unwrap();
    assert_eq!(gridcraft_ui_egui::pivot_filter::buttons(sh), vec![(CellRef::new(2, 0), true)], "the button shows the filter");
}

#[test]
fn pivot_label_and_top10_dialogs() {
    let mut h = harness(pivot_session());
    let id = h.state().session.active().and_then(|d| d.wb.active()).and_then(|s| s.pivots.last()).map(|p| p.id).unwrap();
    h.state_mut()
        .open_dialog("pivotFilter", json!({"kind": "label", "pivot": id, "field": "Region", "op": "beginsWith", "values": ["Sum of Sales"]}));
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.title.clone()).as_deref(), Some("Label Filter (Region)"));
    if let Some(d) = h.state_mut().dialog.as_mut() {
        d.values.insert("value".into(), json!("E"));
    }
    h.run_steps(1);
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().dialog.is_none(), "{:?}", h.state().message);
    assert_eq!(pivot_value(&h, "Grand Total"), Some(Value::Number(120.0)));
    // Top 10: the bottom item by Sum of Sales.
    h.state_mut().open_dialog("pivotFilter", json!({"kind": "top10", "pivot": id, "field": "Region", "values": ["Sum of Sales"]}));
    h.run_steps(2);
    if let Some(d) = h.state_mut().dialog.as_mut() {
        d.values.insert("top".into(), json!("bottom"));
        d.values.insert("count".into(), json!("1"));
    }
    h.run_steps(1);
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().dialog.is_none(), "{:?}", h.state().message);
    assert_eq!(pivot_value(&h, "Grand Total"), Some(Value::Number(30.0)));
    // A bad count keeps the dialog open with a message.
    h.state_mut().open_dialog("pivotFilter", json!({"kind": "top10", "pivot": id, "field": "Region", "values": ["Sum of Sales"]}));
    h.run_steps(2);
    if let Some(d) = h.state_mut().dialog.as_mut() {
        d.values.insert("count".into(), json!("x"));
    }
    h.run_steps(1);
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().dialog.is_some());
    assert!(h.state().message.is_some());
}
