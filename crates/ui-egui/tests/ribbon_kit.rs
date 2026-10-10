//! The M14 ribbon kit: split buttons, accent icons, the new icons, and ribbon overflow.

use egui::{FontDefinitions, FontFamily, Rect, Shape, pos2, vec2};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::icons::{self, FnGlyph, Icon};
use gridcraft_ui_egui::ribbon::{RibbonOverflow, TABS};
use gridcraft_ui_egui::theme::{self, Cat, Tokens};
use gridcraft_ui_egui::{SheetApp, widgets};

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn app_at(width: f32, tab: &str) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.ribbon_tab = tab.into();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 700.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The width budget. The Formulas tab joins the 1280 px list when M14.10 replaces the seven big
/// Function Library buttons with three columns of small ones; until then it fits at 1366 px.
#[test]
fn formulas_insert_and_automate_fit_the_width_budget() {
    for (tab, width) in [("Insert", 1280.0), ("Automate", 1280.0), ("Formulas", 1366.0)] {
        let h = app_at(width, tab);
        let o = RibbonOverflow::get(&h.ctx);
        assert!(o.visible > 0.0);
        assert!(!o.overflows(), "{tab} is {} px wide in a {} px card", o.content, o.visible);
    }
}

#[test]
fn every_tab_fits_the_card_height() {
    // Columns of small buttons hold three rows (Formula Auditing used to stack four).
    for tab in TABS {
        let h = app_at(1440.0, tab);
        let o = RibbonOverflow::get(&h.ctx);
        assert!(o.content_height <= o.visible_height + 0.5, "{tab}: {} px of groups in {} px", o.content_height, o.visible_height);
    }
}

#[test]
fn an_overflowing_tab_shows_scroll_arrows_that_scroll() {
    let mut h = app_at(700.0, "Home");
    let o = RibbonOverflow::get(&h.ctx);
    assert!(o.overflows());
    assert_eq!(o.offset, 0.0);
    assert!(h.query_by_label("Scroll ribbon left").is_none(), "nothing to the left yet");
    h.get_by_label("Scroll ribbon right").click();
    h.run_steps(3);
    let scrolled = RibbonOverflow::get(&h.ctx).offset;
    assert!(scrolled > 0.0);
    h.get_by_label("Scroll ribbon left").click();
    h.run_steps(3);
    assert!(RibbonOverflow::get(&h.ctx).offset < scrolled);
    // A tab that fits shows no arrows.
    let h = app_at(1440.0, "Automate");
    assert!(h.query_by_label("Scroll ribbon right").is_none());
}

#[derive(Default)]
struct Clicks {
    small_main: u32,
    small_arrow: u32,
    big_main: u32,
    big_arrow: u32,
}

fn with_ui_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.insert(FontFamily::Name(theme::UI.into()), proportional.clone());
    fonts.families.insert(FontFamily::Name(theme::CELL.into()), proportional.clone());
    fonts.families.insert(FontFamily::Name(theme::CELL_BOLD.into()), proportional.clone());
    fonts.families.insert(FontFamily::Name(theme::CELL_ITALIC.into()), proportional);
    ctx.set_fonts(fonts);
}

#[test]
fn split_buttons_separate_the_default_action_from_the_menu() {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 200.0)).build_ui_state(
        |ui, c: &mut Clicks| {
            with_ui_fonts(ui.ctx());
            if ui.ctx().cumulative_pass_nr() == 0 {
                return; // the fonts apply from the next pass
            }
            ui.horizontal(|ui| {
                let accent = Some(Tokens::get(ui.ctx()).cat(Cat::Blue));
                let (main, arrow) = widgets::small_split_button(ui, Icon::Sum, None, "AutoSum", "AutoSum");
                c.small_main += u32::from(main.clicked());
                c.small_arrow += u32::from(arrow.clicked());
                let (main, arrow) = widgets::big_split_button(ui, Icon::Python, accent, "Insert\nPython", "Insert Python");
                c.big_main += u32::from(main.clicked());
                c.big_arrow += u32::from(arrow.clicked());
            });
        },
        Clicks::default(),
    );
    h.run_steps(2);
    h.get_by_label("AutoSum").click();
    h.run_steps(2);
    h.get_by_label("AutoSum options").click();
    h.run_steps(2);
    h.get_by_label("AutoSum options").click();
    h.run_steps(2);
    h.get_by_label("Insert Python").click();
    h.run_steps(2);
    h.get_by_label("Insert Python options").click();
    h.run_steps(2);
    let c = h.state();
    assert_eq!((c.small_main, c.small_arrow, c.big_main, c.big_arrow), (1, 2, 1, 1));
    // The main part sits above (big) or left of (small) the arrow part.
    let main = h.get_by_label("Insert Python").rect();
    let arrow = h.get_by_label("Insert Python options").rect();
    assert!(main.max.y <= arrow.min.y + 0.5 && (main.min.x - arrow.min.x).abs() < 0.5);
    let main = h.get_by_label("AutoSum").rect();
    let arrow = h.get_by_label("AutoSum options").rect();
    assert!(main.max.x <= arrow.min.x + 0.5 && (main.min.y - arrow.min.y).abs() < 0.5);
}

const NEW_ICONS: [Icon; 18] = [
    Icon::FnBook(FnGlyph::Star),
    Icon::FnBook(FnGlyph::Coins),
    Icon::FnBook(FnGlyph::Question),
    Icon::FnBook(FnGlyph::Letter),
    Icon::FnBook(FnGlyph::Clock),
    Icon::FnBook(FnGlyph::Magnifier),
    Icon::FnBook(FnGlyph::Theta),
    Icon::FnBook(FnGlyph::Dots),
    Icon::Python,
    Icon::Reset,
    Icon::Diagnostics,
    Icon::Macro,
    Icon::Record,
    Icon::Breakpoint,
    Icon::Equation,
    Icon::Button,
    Icon::Slider,
    Icon::Spin,
];

fn shapes_of(icon: Icon, accent: Option<egui::Color32>, dark: bool) -> Vec<Shape> {
    let ctx = egui::Context::default();
    with_ui_fonts(&ctx);
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    let mut output = ctx
        .run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0))), ..Default::default() }, |ui| {
            icons::paint_accent(ui.painter(), Rect::from_min_size(pos2(10.0, 10.0), vec2(32.0, 32.0)), icon, Tokens::get(ui.ctx()).text, accent)
        });
    output.textures_delta.clear();
    output.shapes.into_iter().map(|s| s.shape).collect()
}

fn colors(shapes: &[Shape]) -> Vec<egui::Color32> {
    let mut v = Vec::new();
    for s in shapes {
        match s {
            Shape::Path(p) => {
                v.push(p.fill);
                if let egui::epaint::ColorMode::Solid(c) = p.stroke.color {
                    v.push(c);
                }
            }
            Shape::Rect(r) => {
                v.push(r.fill);
                v.push(r.stroke.color);
            }
            Shape::Circle(c) => {
                v.push(c.fill);
                v.push(c.stroke.color);
            }
            Shape::Text(t) => {
                v.extend(t.galley.job.sections.iter().map(|s| s.format.color).chain([t.fallback_color, t.override_text_color.unwrap_or_default()]))
            }
            _ => {}
        }
    }
    v
}

#[test]
fn new_icons_draw_inside_their_box_in_both_themes() {
    for dark in [false, true] {
        for icon in NEW_ICONS {
            let shapes = shapes_of(icon, None, dark);
            assert!(!shapes.is_empty(), "{icon:?} draws nothing");
            for s in &shapes {
                let r = s.visual_bounding_rect();
                if r.is_positive() {
                    assert!(Rect::from_min_size(pos2(6.0, 6.0), vec2(40.0, 40.0)).contains_rect(r), "{icon:?} spills out: {r:?}");
                }
            }
        }
    }
}

#[test]
fn an_accent_colours_the_book_spine_and_glyph() {
    let accent = egui::Color32::from_rgb(1, 2, 3);
    for glyph in [FnGlyph::Star, FnGlyph::Coins, FnGlyph::Letter, FnGlyph::Clock, FnGlyph::Dots] {
        let c = colors(&shapes_of(Icon::FnBook(glyph), Some(accent), false));
        assert!(c.contains(&accent), "{glyph:?} ignores the accent");
    }
    // Icons with no coloured part of their own take the accent as their main colour.
    let c = colors(&shapes_of(Icon::Spin, Some(accent), true));
    assert!(c.contains(&accent));
    // The theme's paper colour, not white, fills icon pages in dark mode.
    let c = colors(&shapes_of(Icon::Macro, None, true));
    assert!(c.contains(&Tokens::dark().paper) && !c.contains(&egui::Color32::WHITE));
}

/// `GRIDCRAFT_ICON_SHEET=/path.png` saves the new icons at 16 and 32 px in both themes for review.
#[test]
fn icon_sheet() {
    let Ok(path) = std::env::var("GRIDCRAFT_ICON_SHEET") else { return };
    let mut h = egui_kittest::Harness::builder().with_size(vec2(760.0, 220.0)).with_pixels_per_point(2.5).build_ui(|ui| {
        with_ui_fonts(ui.ctx());
        if ui.ctx().cumulative_pass_nr() == 0 {
            return;
        }
        let full = ui.max_rect();
        for (row, dark) in [(0usize, false), (1, true)] {
            let t = if dark { Tokens::dark() } else { Tokens::light() };
            let band = Rect::from_min_size(pos2(full.left(), full.top() + row as f32 * 110.0), vec2(full.width(), 110.0));
            ui.painter().rect_filled(band, 0.0, t.ribbon);
            for (i, icon) in NEW_ICONS.iter().enumerate() {
                let x = band.left() + 8.0 + i as f32 * 41.0;
                let accent = match icon {
                    Icon::FnBook(_) => Some(t.cat(Cat::ALL[i % 8])),
                    _ => None,
                };
                let pal = icons::Palette::of(&t).with_accent(accent);
                icons::paint_with(ui.painter(), Rect::from_min_size(pos2(x, band.top() + 8.0), vec2(32.0, 32.0)), *icon, t.text, &pal);
                icons::paint_with(ui.painter(), Rect::from_min_size(pos2(x + 8.0, band.top() + 60.0), vec2(16.0, 16.0)), *icon, t.text, &pal);
            }
        }
    });
    h.run_steps(2);
    h.render().unwrap().save(path).unwrap();
}
