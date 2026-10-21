//! Formulas › Function Library: the labelled buttons, their category menus, the More Functions
//! cascade, the AutoSum big split button, the Recently Used keytip and list, and the Insert Function
//! dialog's categories.

use egui::os::OperatingSystem;
use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::{SheetApp, UiState, function_library};
use serde_json::{Value as Json, json};

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn formulas_tab(width: f32) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.ribbon_tab = "Formulas".into();
    let mut h = egui_kittest::Harness::builder().with_os(OperatingSystem::Nix).with_size(vec2(width, 760.0)).build_ui_state(
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
    h.key_press(Key::Escape);
    h.run_steps(2);
    h.state_mut().editor = None;
    h.run_steps(2);
}

#[test]
fn autosum_is_a_big_split_button_then_the_categories_in_columns_of_three() {
    let h = formulas_tab(1440.0);
    let columns: [&[&str]; 3] =
        [&["Recently Used", "Financial", "Logical"], &["Text", "Date & Time", "Lookup & Reference"], &["Math & Trig", "More Functions"]];
    let rect = |l: &str| h.get_by_label(l).rect();
    for column in columns {
        for pair in column.windows(2) {
            let (a, b) = (rect(pair[0]), rect(pair[1]));
            assert!((a.left() - b.left()).abs() < 0.5, "{pair:?} share a left edge");
            assert!(a.bottom() <= b.top() + 0.5, "{pair:?} stack in order");
        }
    }
    for pair in columns.windows(2) {
        assert!(rect(pair[0][0]).right() <= rect(pair[1][0]).left(), "columns run left to right");
    }
    // Insert Function and AutoSum are big buttons, to the left of the columns.
    let (insert, sum) = (rect("Insert Function"), rect("AutoSum"));
    assert!(insert.right() <= sum.left() && sum.right() <= rect("Recently Used").left());
    assert!(sum.top() <= rect("Recently Used").top() + 0.5, "AutoSum starts at the top of the group");
    assert!(h.get_by_label("AutoSum options").rect().bottom() > rect("Financial").bottom(), "its arrow part reaches below the second row");
}

#[test]
fn each_category_menu_lists_its_functions_then_insert_function() {
    let mut h = formulas_tab(1440.0);
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
    let mut h = formulas_tab(1440.0);
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
    let mut h = formulas_tab(1440.0);
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

fn key_event(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
}

#[test]
fn the_recently_used_keytip_opens_its_menu() {
    let mut h = formulas_tab(1440.0);
    h.input_mut().events.extend([key_event(Key::F10, true), key_event(Key::F10, false)]);
    h.run_steps(2);
    for k in [Key::M, Key::R] {
        h.input_mut().events.extend([key_event(k, true), Event::Text(k.name().to_ascii_lowercase()), key_event(k, false)]);
        h.run_steps(3);
    }
    assert_eq!(h.state().keytips.prefix(), None, "MR finishes the sequence");
    h.get_by_label("Insert Function…");
    pick(&mut h, "AVERAGE");
    assert_eq!(editor_text(&h).as_deref(), Some("=AVERAGE("));
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
    // A hostile value loses only itself, never the other preferences.
    let bad: UiState = serde_json::from_value(json!({"dark": true, "recentFunctions": 5})).unwrap();
    assert!(bad.dark);
    assert_eq!(bad.recent_functions, UiState::default().recent_functions);
    let mut odd: Vec<Json> = ["npv", "NOT_A_FUNCTION", "SUM", "NPV"].iter().map(|n| json!(n)).chain([json!(3), json!(null)]).collect();
    odd.extend(function_library::all().iter().take(40).map(|f| json!(f.name)));
    let odd: UiState = serde_json::from_value(json!({"recentFunctions": odd})).unwrap();
    assert_eq!(&odd.recent_functions[..2], ["NPV", "SUM"]);
    assert_eq!(odd.recent_functions.len(), function_library::RECENT_MAX);
}

#[test]
fn formulas_from_any_source_feed_recently_used() {
    let mut h = formulas_tab(1440.0);
    // The ribbon and the control channel go through app.run; MCP and scripts through the session.
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "=NPV(0.1,1,2)"})).unwrap();
    h.state_mut().session.execute("cell.set", json!({"cell": "A2", "input": "=IRR({-10,6,6})+ROUND(1,0)"})).unwrap();
    h.state_mut().session.execute("range.setValues", json!({"range": "B1", "values": [["=FV(0.1,2,-1)", 4, "text"]]})).unwrap();
    h.state_mut().session.execute("cell.set", json!({"cell": "A3", "input": "PMT is just text"})).unwrap();
    h.run_steps(2);
    assert_eq!(&h.state().ui.recent_functions[..4], ["FV", "IRR", "ROUND", "NPV"]);
    // Recently Used lists them newest first.
    click(&mut h, "Recently Used");
    let top = |h: &Harness, name: &str| h.get_by_label(name).rect().top();
    assert!(top(&h, "FV") < top(&h, "IRR") && top(&h, "IRR") < top(&h, "NPV"));
    close_menus(&mut h);
    // A full journal drops its oldest entries; the entries after the trim still count.
    let filler = ("selection.set".to_string(), json!({"cell": "A1"}));
    let len = h.state().session.journal.len();
    h.state_mut().session.journal.extend(std::iter::repeat_n(filler.clone(), gridcraft_engine::JOURNAL_LIMIT - 10 - len));
    h.run_steps(1);
    let journal = &mut h.state_mut().session.journal;
    journal.extend(std::iter::repeat_n(filler, 20));
    journal.drain(..gridcraft_engine::JOURNAL_TRIM);
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
    let mut h = formulas_tab(1440.0);
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
