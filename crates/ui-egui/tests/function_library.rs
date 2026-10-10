//! Formulas › Function Library: the labelled buttons, their menus, the AutoSum split and the
//! width budget.

use egui::vec2;
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::ribbon::RibbonOverflow;
use serde_json::json;

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
    }
}
