//! Formulas › Function Library: the labelled buttons, their category menus, the More Functions
//! cascade, the AutoSum big split button and the Recently Used keytip.

use egui::os::OperatingSystem;
use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

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
    // A full column stands where the tab's other columns of three small buttons do (Formula
    // Auditing's), so it fits the ribbon in any font.
    let (top, bottom) = (rect("Show Formulas").top(), rect("Evaluate Formula").bottom());
    for column in &columns[..2] {
        let (t, b) = (rect(column[0]).top(), rect(column[2]).bottom());
        assert!((t - top).abs() < 0.5 && (b - bottom).abs() < 0.5, "{column:?} spans {t}..{b}, Formula Auditing {top}..{bottom}");
    }
}

#[test]
fn each_category_menu_lists_its_functions_then_insert_function() {
    let mut h = formulas_tab(1440.0);
    // Recently Used lists the functions of formulas entered.
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "=SUM(1)+AVERAGE(2)"})).unwrap();
    h.run_steps(2);
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
    // Recently Used lists the functions of formulas entered.
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "=SUM(1)+AVERAGE(2)"})).unwrap();
    h.run_steps(2);
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
