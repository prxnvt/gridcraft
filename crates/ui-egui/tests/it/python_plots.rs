//! What a cell that gave a picture shows and does: its card, the Python Output menu, Display Plot over
//! Cells, and what a linked picture shows. A fake worker stands in for Python.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use egui::{Event, Modifiers, Rect, vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gridcraft_engine::core::{CellError, CellRef, Value};
use gridcraft_engine::model::{ObjKind, ObjRef};
use gridcraft_engine::python::{Interpreter, InterpreterKind, Version};
use gridcraft_engine::{PythonEnvironment, PythonState, Session};
use gridcraft_python::fake::FakeTransport;
use gridcraft_python::protocol::Output;
use gridcraft_python::{Picture, PyError};
use gridcraft_ui_egui::python_plots::{COPY_IMAGE, CREATE_REFERENCE, DISPLAY_OVER_CELLS, IMAGE_COPIED, IMAGE_SAVED, OUTPUT_MENU, SAVE_IMAGE};
use gridcraft_ui_egui::{Services, SheetApp};
use serde_json::json;

use crate::common::Harness;

const PYTHON: &str = "/home/you/.gridcraft/python/bin/python";

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(width, height, image::Rgba([10, 120, 200, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn answer(code: &str) -> Result<Output, PyError> {
    match code {
        "plot" => Ok(Output::picture(Picture::from_bytes(png(80, 60), 2).unwrap())),
        "other" => Ok(Output::picture(Picture::from_bytes(png(60, 60), 1).unwrap())),
        "fail" => Err(PyError { message: "ValueError: boom".into(), ..PyError::default() }),
        _ => Ok(Output::Value(Value::Number(1.0))),
    }
}

fn report() -> PythonEnvironment {
    PythonEnvironment {
        state: PythonState::Ready,
        diagnostic: None,
        sandbox: "a test sandbox".into(),
        uv: None,
        interpreter: Some(Interpreter {
            path: PathBuf::from(PYTHON),
            run_as: PathBuf::from(PYTHON),
            version: Version { major: 3, minor: 12, patch: 7 },
            kind: InterpreterKind::UvEnvironment,
            site_dirs: Vec::new(),
            read_paths: Vec::new(),
        }),
        packages: Vec::new(),
    }
}

fn harness_with(services: Services) -> Harness {
    let t = FakeTransport::answering(|code, _| answer(code));
    let mut s = Session::new();
    s.new_workbook();
    let mut app = crate::common::prepare(SheetApp::new(s, services));
    app.ui.python.enabled = true;
    app.python.set_sandbox(Ok("a test sandbox".into()));
    app.python.report = Some(report());
    app.python.transport = Some(Arc::new(move || Box::new(t.sharing())));
    let mut h = crate::common::harness_with(crate::common::builder().with_size(vec2(1300.0, 900.0)), app, 3);
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.run_steps(2);
    h
}

fn harness() -> Harness {
    harness_with(Services::default())
}

fn settle(h: &mut Harness) {
    for _ in 0..1000 {
        h.step();
        if !h.state().session.python_busy() {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("Python never settled");
}

fn c(a1: &str) -> CellRef {
    CellRef::parse(a1).unwrap()
}

fn plot_at(h: &mut Harness, cell: &str) {
    h.state_mut().run("python.set", json!({"cell": cell, "code": "plot"})).unwrap();
    settle(h);
    h.state_mut().session.execute("selection.set", json!({"cell": cell})).unwrap();
    h.run_steps(2);
}

fn card_on(h: &mut Harness, cell: &str) {
    plot_at(h, cell);
    h.state_mut().run("ui.pythonCard", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().grid.py_card.is_some(), "the card is open");
}

fn sheet_images(h: &Harness) -> usize {
    h.state().session.active().unwrap().wb.active().unwrap().images.len()
}

fn click(h: &mut Harness, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(4);
}

fn toast(h: &Harness) -> Option<String> {
    h.state().toast.as_ref().map(|(text, _)| text.clone())
}

fn in_cell(h: &Harness, cell: &str) -> bool {
    h.state().session.active().unwrap().wb.active().unwrap().picture_at(c(cell)).is_some()
}

#[test]
fn the_cell_says_image_and_the_card_shows_the_picture_and_its_size() {
    let mut h = harness();
    card_on(&mut h, "B2");
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().value(c("B2")), Value::text("Image"));
    assert!(h.query_by_label("Image").is_some());
    assert!(h.query_by_label("size").is_some());
    assert!(h.query_by_label("40, 30").is_some(), "its own size, not the pixels it is drawn with");
    assert!(h.query_by_label("Python Object").is_some());
    assert!(h.query_by_label("Excel Value").is_some());
    assert!(h.query_by_label(COPY_IMAGE).is_some());
    assert!(h.query_by_label(DISPLAY_OVER_CELLS).is_some());
    h.check_accessibility();
}

#[test]
fn the_cards_corner_resizes_it_within_the_window() {
    let mut h = harness();
    card_on(&mut h, "B2");
    let area = |h: &Harness| h.ctx.memory(|m| m.area_rect(egui::Id::unique("python_card"))).expect("the card is shown");
    let before = area(&h);
    let drag = |h: &mut Harness, by: f32| {
        let grip = h.get_by_label("Resize").accesskit_node().bounding_box().expect("a place on screen");
        let at = egui::pos2(((grip.x0 + grip.x1) / 2.0) as f32, ((grip.y0 + grip.y1) / 2.0) as f32);
        let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        h.input_mut().events.push(Event::PointerMoved(at));
        h.step();
        h.input_mut().events.push(press(at, true));
        h.step();
        let to = at + vec2(by, 0.0);
        h.input_mut().events.push(Event::PointerMoved(to));
        h.step();
        h.input_mut().events.push(press(to, false));
        h.run_steps(3);
    };
    drag(&mut h, 120.0);
    assert!(area(&h).width() > before.width() + 60.0, "{:?} from {:?}", area(&h), before);
    // However far it goes, it stays in the window.
    drag(&mut h, 5000.0);
    let window: Rect = h.ctx.content_rect();
    assert!(area(&h).width() <= window.width(), "{:?}", area(&h));
}

#[test]
fn copy_image_gives_the_clipboard_the_pixels() {
    let copied: Rc<RefCell<Vec<(u32, u32)>>> = Rc::default();
    let seen = copied.clone();
    let services = Services {
        copy_image: Some(Box::new(move |image| {
            seen.borrow_mut().push((image.width(), image.height()));
            Ok(())
        })),
        ..Services::default()
    };
    let mut h = harness_with(services);
    card_on(&mut h, "B2");
    click(&mut h, COPY_IMAGE);
    assert_eq!(*copied.borrow(), [(80, 60)], "the picture's own pixels");
    assert_eq!(toast(&h).as_deref(), Some(IMAGE_COPIED));
}

#[test]
fn save_image_asks_where_and_cancelling_does_nothing() {
    let dir = std::env::temp_dir().join(format!("gridcraft-py-plots-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("saved.png");
    let answer_with: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let (pick, asked): (Rc<RefCell<Option<String>>>, Rc<RefCell<Vec<String>>>) = (answer_with.clone(), Rc::default());
    let names = asked.clone();
    let services = Services {
        pick_save: Some(Box::new(move |name: &str| {
            names.borrow_mut().push(name.to_string());
            pick.borrow().clone()
        })),
        ..Services::default()
    };
    let mut h = harness_with(services);
    card_on(&mut h, "B2");
    click(&mut h, SAVE_IMAGE);
    assert!(asked.borrow()[0].ends_with("-B2.png"), "{:?}", asked.borrow());
    assert!(!path.exists(), "cancelled");
    assert_eq!(toast(&h), None);
    *answer_with.borrow_mut() = Some(path.to_str().unwrap().to_string());
    click(&mut h, SAVE_IMAGE);
    assert_eq!(std::fs::read(&path).unwrap(), png(80, 60));
    assert_eq!(toast(&h).as_deref(), Some(IMAGE_SAVED));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_output_menu_sets_the_return_type_and_needs_a_cell() {
    let mut h = harness();
    plot_at(&mut h, "B2");
    assert!(!in_cell(&h, "B2"));
    h.get_by_label(OUTPUT_MENU).click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label(OUTPUT_MENU).count(), 2, "the button, and the menu's title");
    h.get_by_label("Excel Value").click();
    settle(&mut h);
    assert!(in_cell(&h, "B2"), "the picture is in the cell now");
    h.get_by_label(OUTPUT_MENU).click();
    h.run_steps(3);
    h.get_by_label("Python Object").click();
    settle(&mut h);
    assert!(!in_cell(&h, "B2"));
    // Off a Python cell the menu isn't there.
    h.state_mut().session.execute("selection.set", json!({"cell": "F9"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label(OUTPUT_MENU).is_none());
    // On a draft there is no cell to set it on yet.
    h.state_mut().insert_python();
    h.run_steps(3);
    assert!(h.get_by_label(OUTPUT_MENU).accesskit_node().is_disabled());
}

#[test]
fn the_shortcut_and_the_button_display_a_plot_over_the_cells() {
    let mut h = harness();
    plot_at(&mut h, "B2");
    let shortcut = |h: &mut Harness| {
        let modifiers = Modifiers { alt: true, ctrl: true, shift: true, ..Modifiers::NONE };
        h.input_mut().events.push(Event::Key { key: egui::Key::C, physical_key: None, pressed: true, repeat: false, modifiers });
        h.input_mut().events.push(Event::Key { key: egui::Key::C, physical_key: None, pressed: false, repeat: false, modifiers });
        h.run_steps(3);
    };
    shortcut(&mut h);
    assert_eq!(sheet_images(&h), 1);
    // On a cell with no picture, it does nothing.
    h.state_mut().run("python.set", json!({"cell": "D2", "code": "number"})).unwrap();
    settle(&mut h);
    h.state_mut().session.execute("selection.set", json!({"cell": "D2"})).unwrap();
    h.run_steps(2);
    shortcut(&mut h);
    assert_eq!(sheet_images(&h), 1);
    h.state_mut().session.execute("selection.set", json!({"cell": "F9"})).unwrap();
    h.run_steps(2);
    shortcut(&mut h);
    assert_eq!(sheet_images(&h), 1);
    // From the card.
    card_on(&mut h, "B4");
    click(&mut h, DISPLAY_OVER_CELLS);
    assert_eq!(sheet_images(&h), 2);
}

#[test]
fn a_selected_linked_picture_offers_create_reference_and_an_ordinary_one_doesnt() {
    let mut h = harness();
    plot_at(&mut h, "B2");
    h.state_mut().run("python.displayOverCells", json!({"cell": "B2", "at": "D2"})).unwrap();
    h.run_steps(2);
    assert!(h.query_by_label(CREATE_REFERENCE).is_none(), "not until it is selected");
    let id = h.state().session.active().unwrap().wb.active().unwrap().images[0].id;
    h.state_mut().selected_object = Some(ObjRef::new(ObjKind::Image, id));
    h.run_steps(3);
    click(&mut h, CREATE_REFERENCE);
    assert_eq!(sheet_images(&h), 2);
    assert_eq!(h.state().selected_object, Some(ObjRef::new(ObjKind::Image, id)), "the click stays with the button");
    // A picture placed by hand has no cell to refer to.
    let bytes = gridcraft_engine::io::base64_encode(&png(4, 4));
    h.state_mut().run("insert.picture", json!({"base64": bytes, "at": "H2"})).unwrap();
    let plain = h.state().session.active().unwrap().wb.active().unwrap().images[2].id;
    h.state_mut().selected_object = Some(ObjRef::new(ObjKind::Image, plain));
    h.run_steps(3);
    assert!(h.query_by_label(CREATE_REFERENCE).is_none());
}

#[test]
fn a_picture_whose_cell_failed_says_so_and_the_next_picture_clears_it() {
    let mut h = harness();
    plot_at(&mut h, "B2");
    h.state_mut().run("python.displayOverCells", json!({"cell": "B2", "at": "D2"})).unwrap();
    h.state_mut().run("python.set", json!({"cell": "B2", "code": "fail"})).unwrap();
    settle(&mut h);
    assert_eq!(sheet_images(&h), 1, "a failure deletes nothing");
    let said = format!("B2: {} This picture shows the last result that cell gave.", CellError::Python.as_str());
    assert!(h.query_by_label(&said).is_some(), "{said}");
    h.state_mut().run("python.set", json!({"cell": "B2", "code": "other"})).unwrap();
    settle(&mut h);
    assert!(h.query_by_label(&said).is_none());
    assert_eq!(sheet_images(&h), 1);
    // And code that gives no picture takes it away, with Undo to bring both back.
    h.state_mut().run("python.set", json!({"cell": "B2", "code": "number"})).unwrap();
    settle(&mut h);
    assert_eq!(sheet_images(&h), 0);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    settle(&mut h);
    assert_eq!(sheet_images(&h), 1);
}

#[test]
fn the_python_tab_offers_it_with_the_reason_when_it_cant() {
    let mut h = harness();
    plot_at(&mut h, "B2");
    h.state_mut().run("ui.ribbonTab", json!({"tab": "Python"})).unwrap();
    h.run_steps(3);
    h.get_by_label(DISPLAY_OVER_CELLS).click();
    h.run_steps(4);
    assert_eq!(sheet_images(&h), 1);
}
