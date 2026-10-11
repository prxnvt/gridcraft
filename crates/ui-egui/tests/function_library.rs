//! Formulas › Function Library: the labelled buttons, their menus, the AutoSum split and the
//! width budget.

use egui::vec2;
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::ribbon::RibbonOverflow;
use gridcraft_ui_egui::{SheetApp, UiState, function_library};
use serde_json::{Value as Json, json};

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn formulas_tab(width: f32, dark: bool) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.ribbon_tab = "Formulas".into();
    app.ui.dark = dark;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 760.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn editor_text(h: &Harness) -> Option<String> {
    h.state().editor.as_ref().map(|e| e.text.clone())
}

fn input(h: &Harness, cell: &str) -> String {
    let d = h.state().session.active().unwrap();
    d.wb.active().unwrap().input_text(CellRef::parse(cell).unwrap())
}

fn click(h: &mut Harness, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

/// Clicks a menu row through accessibility, which reaches rows scrolled out of a long menu.
fn pick(h: &mut Harness, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}

fn close_menus(h: &mut Harness) {
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.state_mut().editor = None;
    h.run_steps(2);
}

#[test]
fn the_formulas_tab_fits_1280_px_in_both_themes() {
    for dark in [false, true] {
        let h = formulas_tab(1280.0, dark);
        let o = RibbonOverflow::get(&h.ctx);
        assert!(o.visible > 0.0);
        assert!(!o.overflows(), "Formulas is {} px wide in a {} px card (dark: {dark})", o.content, o.visible);
    }
}

#[test]
fn the_library_is_nine_labelled_buttons_in_three_columns_of_three() {
    let h = formulas_tab(1440.0, false);
    let labels =
        [["AutoSum", "Recently Used", "Financial"], ["Logical", "Text", "Date & Time"], ["Lookup & Reference", "Math & Trig", "More Functions"]];
    let rect = |l: &str| h.get_by_label(l).rect();
    for column in labels {
        let [a, b, c] = column.map(rect);
        assert!((a.left() - b.left()).abs() < 0.5 && (b.left() - c.left()).abs() < 0.5, "{column:?} share a left edge");
        assert!(a.bottom() <= b.top() + 0.5 && b.bottom() <= c.top() + 0.5, "{column:?} stack in order");
    }
    for row in 0..3 {
        let [a, b, c] = labels.map(|column| rect(column[row]));
        assert!(a.right() <= b.left() && b.right() <= c.left(), "row {row} runs left to right");
        assert!((a.top() - b.top()).abs() < 0.5 && (b.top() - c.top()).abs() < 0.5);
    }
}

#[test]
fn each_category_menu_lists_its_functions_then_insert_function() {
    let mut h = formulas_tab(1440.0, false);
    for (button, function) in [
        ("Recently Used", "SUM"),
        ("Financial", "PMT"),
        ("Logical", "IF"),
        ("Text", "CONCAT"),
        ("Date & Time", "TODAY"),
        ("Lookup & Reference", "XLOOKUP"),
        ("Math & Trig", "ROUND"),
    ] {
        click(&mut h, button);
        h.get_by_label("Insert Function…");
        pick(&mut h, function);
        assert_eq!(editor_text(&h).as_deref(), Some(format!("={function}(").as_str()), "{button} › {function} starts the formula");
        close_menus(&mut h);
    }
    // The menu's last row opens Insert Function on the menu's category.
    click(&mut h, "Date & Time");
    click(&mut h, "Insert Function…");
    let d = h.state().dialog.as_ref().expect("Insert Function opens");
    assert_eq!(d.name(), "insertFunction");
    assert_eq!(d.values.get("category"), Some(&json!("Date & Time")));
}

#[test]
fn more_functions_cascades_into_the_remaining_categories() {
    let mut h = formulas_tab(1440.0, false);
    click(&mut h, "More Functions");
    for c in ["Statistical", "Engineering", "Cube", "Information", "Compatibility", "Web"] {
        h.get_by_label(&format!("{c} ⏵"));
    }
    click(&mut h, "Engineering ⏵");
    h.get_by_label("Insert Function…");
    pick(&mut h, "BIN2DEC");
    assert_eq!(editor_text(&h).as_deref(), Some("=BIN2DEC("));
    close_menus(&mut h);
    click(&mut h, "More Functions");
    click(&mut h, "Statistical ⏵");
    pick(&mut h, "Insert Function…");
    assert_eq!(h.state().dialog.as_ref().and_then(|d| d.values.get("category")), Some(&json!("Statistical")));
}

#[test]
fn autosum_runs_sum_and_its_arrow_opens_the_shared_menu() {
    let mut h = formulas_tab(1440.0, false);
    {
        let s = &mut h.state_mut().session;
        s.execute("range.setValues", json!({"range": "A1", "values": [[1, 4], [2, 5], [3, 6]]})).unwrap();
        s.execute("selection.set", json!({"cell": "A4"})).unwrap();
    }
    h.run_steps(2);
    click(&mut h, "AutoSum");
    assert_eq!(input(&h, "A4"), "=SUM(A1:A3)");
    h.state_mut().session.execute("selection.set", json!({"cell": "B4"})).unwrap();
    click(&mut h, "AutoSum options");
    for item in ["Sum", "Average", "Max", "Min", "More Functions…"] {
        h.get_by_label(item);
    }
    click(&mut h, "Count Numbers");
    assert_eq!(input(&h, "B4"), "=COUNT(B1:B3)");
    click(&mut h, "AutoSum options");
    click(&mut h, "More Functions…");
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name().to_string()).as_deref(), Some("insertFunction"));
}

#[test]
fn recently_used_survives_a_ui_state_round_trip() {
    let mut ui = UiState::default();
    assert_eq!(ui.recent_functions.len(), function_library::RECENT_MAX, "a fresh install lists common functions");
    function_library::push_recent(&mut ui.recent_functions, &["NPV".into()]);
    let back: UiState = serde_json::from_str(&serde_json::to_string(&ui).unwrap()).unwrap();
    assert_eq!(back.recent_functions, ui.recent_functions);
    // A ui.json from before Recently Used gets the default list.
    let old: UiState = serde_json::from_value(json!({"dark": true, "ribbonTab": "Formulas"})).unwrap();
    assert_eq!(old.recent_functions, UiState::default().recent_functions);
    // A hostile list loses only itself, never the other preferences.
    let bad: UiState = serde_json::from_value(json!({"dark": true, "recentFunctions": 5})).unwrap();
    assert!(bad.dark);
    assert_eq!(bad.recent_functions, UiState::default().recent_functions);
    let names: Vec<Json> = ["npv", "NOT_A_FUNCTION", "SUM", "NPV"].iter().map(|n| json!(n)).chain([json!(3), json!(null)]).collect();
    let mut many = names.clone();
    many.extend(gridcraft_ui_egui::function_library::all().iter().take(40).map(|f| json!(f.name)));
    let odd: UiState = serde_json::from_value(json!({"recentFunctions": many})).unwrap();
    assert_eq!(&odd.recent_functions[..2], ["NPV", "SUM"]);
    assert_eq!(odd.recent_functions.len(), function_library::RECENT_MAX);
}

#[test]
fn formulas_from_any_source_feed_recently_used() {
    let mut h = formulas_tab(1440.0, false);
    // The ribbon and the control channel go through app.run; MCP and scripts through the session.
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "=NPV(0.1,1,2)"})).unwrap();
    h.state_mut().session.execute("cell.set", json!({"cell": "A2", "input": "=IRR({-10,6,6})+ROUND(1,0)"})).unwrap();
    h.state_mut().session.execute("range.setValues", json!({"range": "B1", "values": [["=FV(0.1,2,-1)", 4, "text"]]})).unwrap();
    h.state_mut().session.execute("cell.set", json!({"cell": "A3", "input": "PMT is just text"})).unwrap();
    h.run_steps(2);
    assert_eq!(&h.state().ui.recent_functions[..4], ["FV", "IRR", "ROUND", "NPV"]);
    // Recently Used lists them newest first.
    click(&mut h, "Recently Used");
    let fv = h.get_by_label("FV").rect();
    assert!(fv.top() < h.get_by_label("IRR").rect().top() && h.get_by_label("IRR").rect().top() < h.get_by_label("NPV").rect().top());
    close_menus(&mut h);
    // The journal trims its oldest half when it passes 10,000 entries; new entries still count.
    let filler = ("selection.set".to_string(), json!({"cell": "A1"}));
    h.state_mut().session.journal.extend(std::iter::repeat_n(filler.clone(), 9_990));
    h.run_steps(1);
    let journal = &mut h.state_mut().session.journal;
    journal.extend(std::iter::repeat_n(filler, 20));
    journal.drain(..5_000);
    journal.push(("cell.set".into(), json!({"cell": "C1", "input": "=DB(1000,100,5,1)"})));
    h.run_steps(1);
    assert_eq!(h.state().ui.recent_functions[0], "DB");
}

/// Opens the Insert Function category list (the dialog's only combo box).
fn open_categories(h: &mut Harness) {
    h.get_by_role(egui::accesskit::Role::ComboBox).click();
    h.run_steps(3);
}

/// Picks a category from the open list (the ribbon has buttons with the same names).
fn choose(h: &mut Harness, category: &str) {
    h.get_all_by_label(category).last().expect("the category is listed").click();
    h.run_steps(3);
}

fn row(name: &str, category: &str) -> String {
    format!("{name} ({category})")
}

#[test]
fn insert_function_has_its_title_and_a_working_category_list() {
    let mut h = formulas_tab(1440.0, false);
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "=NPV(0.1,1,2)"})).unwrap();
    h.run_steps(2);
    click(&mut h, "Insert Function");
    let d = h.state().dialog.as_ref().expect("Insert Function opens");
    assert_eq!(d.title, "Insert Function");
    // Most Recently Used comes first, newest first.
    let npv = h.get_by_label(&row("NPV", "Financial")).rect();
    assert!(npv.top() < h.get_by_label(&row("SUM", "Math & Trig")).rect().top());
    assert!(h.query_by_label(&row("PMT", "Financial")).is_some(), "the default list follows");
    // Pick a category from the list.
    open_categories(&mut h);
    choose(&mut h, "Date & Time");
    assert!(h.query_by_label(&row("TODAY", "Date & Time")).is_some());
    assert!(h.query_by_label(&row("NPV", "Financial")).is_none());
    // All lists everything; a category given by its engine id works too (control channel).
    open_categories(&mut h);
    choose(&mut h, "All");
    assert!(h.query_by_label(&row("TODAY", "Date & Time")).is_some() && h.query_by_label(&row("NPV", "Financial")).is_some());
    h.state_mut().dialog.as_mut().unwrap().values.insert("category".into(), json!("MathTrig"));
    h.run_steps(2);
    assert!(h.query_by_label(&row("ROUND", "Math & Trig")).is_some() && h.query_by_label(&row("TODAY", "Date & Time")).is_none());
    // A search looks through every category.
    h.state_mut().dialog.as_mut().unwrap().search = "lookup".into();
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().unwrap().values.get("category"), Some(&json!("All")));
    assert!(h.query_by_label(&row("XLOOKUP", "Lookup & Reference")).is_some());
    // OK starts the formula and moves the function to the front of Recently Used.
    pick(&mut h, &row("XLOOKUP", "Lookup & Reference"));
    click(&mut h, "   OK   ");
    assert_eq!(editor_text(&h).as_deref(), Some("=XLOOKUP("));
    assert_eq!(h.state().ui.recent_functions[0], "XLOOKUP");
}

/// `GRIDCRAFT_FNLIB_SNAPSHOTS=/dir` saves the Formulas tab at 1280 and 1440 px in both themes,
/// and its menus open, for review.
#[test]
fn snapshots() {
    let Ok(dir) = std::env::var("GRIDCRAFT_FNLIB_SNAPSHOTS") else { return };
    let save = |h: &mut Harness, name: &str| {
        h.run_steps(3);
        h.render().unwrap().save(format!("{dir}/{name}.png")).unwrap();
    };
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        for width in [1280.0, 1440.0] {
            let mut h = formulas_tab(width, dark);
            save(&mut h, &format!("formulas-{theme}-{width}"));
        }
        for (name, path) in [
            ("financial", &["Financial"][..]),
            ("recent", &["Recently Used"][..]),
            ("autosum", &["AutoSum options"][..]),
            ("more-statistical", &["More Functions", "Statistical ⏵"][..]),
        ] {
            let mut h = formulas_tab(1280.0, dark);
            for label in path {
                click(&mut h, label);
            }
            save(&mut h, &format!("menu-{name}-{theme}"));
        }
        let mut h = formulas_tab(1280.0, dark);
        click(&mut h, "Insert Function");
        save(&mut h, &format!("dialog-insert-function-{theme}"));
        open_categories(&mut h);
        save(&mut h, &format!("dialog-insert-function-categories-{theme}"));
    }
}
