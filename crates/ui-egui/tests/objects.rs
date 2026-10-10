//! Floating objects through real pointer and keyboard input: selecting every kind, resizing from
//! each of the eight handles, deleting with Delete and Backspace, and double-click routing.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::{Anchor, Color, ControlKind, FormControl, ObjKind, ObjRef, Shape, ShapeKind};
use gridcraft_ui_egui::objects::{Handle, OpenAction, open_action};
use gridcraft_ui_egui::{SheetApp, chartview, grid::Geo};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

struct Objects {
    chart: ObjRef,
    image: ObjRef,
    shape: ObjRef,
    text_box: ObjRef,
    equation: ObjRef,
    button: ObjRef,
    /// A slider lying over the chart: controls paint on top and take the click.
    slider: ObjRef,
}

fn at(a: &str) -> CellRef {
    CellRef::parse(a).unwrap()
}

fn id(v: &serde_json::Value, key: &str) -> u32 {
    v[key].as_u64().unwrap() as u32
}

fn fixture() -> (Harness, Objects) {
    let mut s = Session::new();
    s.new_workbook();
    for (i, n) in [3, 5, 2, 8].into_iter().enumerate() {
        s.execute("cell.set", json!({"cell": CellRef::new(i as u32, 0).a1(), "input": n.to_string()})).unwrap();
    }
    s.execute("cell.set", json!({"cell": "N25", "input": "keep me"})).unwrap();
    s.execute("selection.set", json!({"cell": "N25"})).unwrap();
    let chart =
        id(&s.execute("insert.chart", json!({"range": "A1:A4", "type": "column", "at": "C3", "width": 220, "height": 140})).unwrap(), "chart");
    let shape = id(&s.execute("insert.shape", json!({"kind": "rectangle", "at": "H4", "width": 120, "height": 80})).unwrap(), "shape");
    let text_box = id(&s.execute("insert.textBox", json!({"at": "H11", "width": 140, "height": 60, "text": "Note"})).unwrap(), "shape");
    let image = s.execute("insert.picture", json!({"base64": tiny_png(), "at": "L4"})).unwrap()["image"].as_u64().unwrap() as u32;
    s.execute("object.move", json!({"kind": "image", "id": image, "at": "L4", "width": 60, "height": 60})).unwrap();
    let wb = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb);
    let equation = wb.next_object_id();
    let anchor = |cell: &str, width: f32, height: f32| Anchor { cell: at(cell), width, height, ..Default::default() };
    wb.sheet_mut(0).unwrap().shapes.push(Shape {
        id: equation,
        kind: ShapeKind::Equation,
        anchor: anchor("C14", 120.0, 50.0),
        fill: Color::Auto,
        line: Color::Auto,
        text: "a^2+b^2=c^2".into(),
    });
    let button = wb.next_object_id();
    let sh = wb.sheet_mut(0).unwrap();
    sh.controls.push(FormControl { id: button, kind: ControlKind::Button, anchor: anchor("L11", 90.0, 28.0), text: "Run".into(), link: None });
    let slider = button + 1;
    sh.controls.push(FormControl {
        id: slider,
        kind: ControlKind::Slider,
        anchor: anchor("D5", 120.0, 20.0),
        text: String::new(),
        link: Some("$B$1".into()),
    });
    let objs = Objects {
        chart: ObjRef::new(ObjKind::Chart, chart),
        image: ObjRef::new(ObjKind::Image, image),
        shape: ObjRef::new(ObjKind::Shape, shape),
        text_box: ObjRef::new(ObjKind::Shape, text_box),
        equation: ObjRef::new(ObjKind::Equation, equation),
        button: ObjRef::new(ObjKind::Control, button),
        slider: ObjRef::new(ObjKind::Control, slider),
    };
    let mut h = egui_kittest::Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            |ui, app: &mut SheetApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            SheetApp::new(s, Default::default()),
        );
    h.run_steps(4);
    (h, objs)
}

fn tiny_png() -> String {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([40, 120, 200, 255]))
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    gridcraft_engine::io::base64_encode(&bytes)
}

fn rect(h: &Harness, o: ObjRef) -> Rect {
    let app = h.state();
    let sh = app.session.doc().unwrap().wb.active().unwrap();
    chartview::object_rect(&Geo::new(sh, app.grid.rect.unwrap(), app.view().scroll), sh, o).unwrap()
}

fn exists(h: &Harness, o: ObjRef) -> bool {
    h.state().session.doc().unwrap().wb.active().unwrap().has_object(o)
}

fn pointer(h: &mut Harness, p: Pos2, pressed: bool) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

fn click(h: &mut Harness, p: Pos2) {
    pointer(h, p, true);
    pointer(h, p, false);
    h.run_steps(2);
}

fn double_click(h: &mut Harness, p: Pos2) {
    // Separate the gesture from earlier clicks (egui's triple-click window).
    h.run_steps(45);
    click(h, p);
    click(h, p);
}

fn click_on(h: &mut Harness, o: ObjRef) {
    let p = rect(h, o).center();
    click(h, p);
}

fn double_click_on(h: &mut Harness, o: ObjRef) {
    let p = rect(h, o).center();
    double_click(h, p);
}

fn drag(h: &mut Harness, from: Pos2, to: Pos2) {
    h.run_steps(45);
    pointer(h, from, true);
    h.input_mut().events.push(Event::PointerMoved(from + (to - from) * 0.25));
    h.step();
    h.input_mut().events.push(Event::PointerMoved(to));
    h.step();
    pointer(h, to, false);
    h.run_steps(2);
}

fn key(h: &mut Harness, key: Key) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn n25(h: &Harness) -> Value {
    h.state().session.doc().unwrap().wb.active().unwrap().value(at("N25"))
}

#[test]
fn a_click_selects_every_kind_and_controls_win_over_charts() {
    let (mut h, o) = fixture();
    for obj in [o.chart, o.image, o.shape, o.text_box, o.equation, o.button] {
        let r = rect(&h, obj);
        // Clear of the slider that lies over the chart.
        let p = if obj == o.chart { r.left_bottom() + vec2(12.0, -12.0) } else { r.center() };
        click(&mut h, p);
        assert_eq!(h.state().selected_object, Some(obj), "{obj:?}");
    }
    click_on(&mut h, o.slider);
    assert_eq!(h.state().selected_object, Some(o.slider), "the control over the chart takes the click");
    let r = rect(&h, o.chart);
    click(&mut h, r.right_bottom() + vec2(80.0, 80.0));
    assert_eq!(h.state().selected_object, None, "a click on the cells clears the object selection");
    assert_eq!(n25(&h), Value::text("keep me"));
}

#[test]
fn every_handle_resizes_only_its_own_edges() {
    let (mut h, o) = fixture();
    for obj in [o.shape, o.equation, o.button, o.image] {
        click_on(&mut h, obj);
        for handle in Handle::ALL {
            let before = rect(&h, obj);
            let (sx, sy) = handle.sides();
            let d = vec2(f32::from(sx) * 20.0, f32::from(sy) * 16.0);
            let grab = handle.pos(before);
            drag(&mut h, grab, grab + d);
            let after = rect(&h, obj);
            let moved = |edge: f32, was: f32, by: f32| (edge - (was + by)).abs() < 1.0;
            assert!(moved(after.left(), before.left(), if sx < 0 { d.x } else { 0.0 }), "{obj:?} {handle:?} left");
            assert!(moved(after.right(), before.right(), if sx > 0 { d.x } else { 0.0 }), "{obj:?} {handle:?} right");
            assert!(moved(after.top(), before.top(), if sy < 0 { d.y } else { 0.0 }), "{obj:?} {handle:?} top {before:?} {after:?}");
            assert!(moved(after.bottom(), before.bottom(), if sy > 0 { d.y } else { 0.0 }), "{obj:?} {handle:?} bottom");
            assert_eq!(h.state().selected_object, Some(obj), "resizing keeps the selection");
        }
    }
    // Shrinking past the opposite edge stops short of it.
    let before = rect(&h, o.shape);
    click(&mut h, before.center());
    drag(&mut h, Handle::Right.pos(before), Handle::Right.pos(before) - vec2(before.width() + 50.0, 0.0));
    let after = rect(&h, o.shape);
    assert!(after.width() > 1.0 && (after.left() - before.left()).abs() < 1.0);
    // Dragging the body moves the object without resizing it.
    let before = rect(&h, o.button);
    click(&mut h, before.center());
    drag(&mut h, before.center(), before.center() + vec2(30.0, 25.0));
    let after = rect(&h, o.button);
    assert!((after.min - before.min - vec2(30.0, 25.0)).length() < 1.0 && (after.size() - before.size()).length() < 0.5);
    assert_eq!(n25(&h), Value::text("keep me"));
}

#[test]
fn delete_and_backspace_delete_the_selected_object_not_the_cells() {
    let (mut h, o) = fixture();
    click_on(&mut h, o.shape);
    key(&mut h, Key::Delete);
    assert!(!exists(&h, o.shape));
    assert_eq!(h.state().selected_object, None);
    assert_eq!(n25(&h), Value::text("keep me"));

    click_on(&mut h, o.button);
    key(&mut h, Key::Backspace);
    assert!(!exists(&h, o.button));
    assert!(h.state().editor.is_none(), "Backspace on an object must not start a cell editor");
    assert_eq!(n25(&h), Value::text("keep me"));

    click_on(&mut h, o.equation);
    key(&mut h, Key::Escape);
    assert_eq!(h.state().selected_object, None, "Escape drops the object selection");
    key(&mut h, Key::Delete);
    assert!(exists(&h, o.equation), "with nothing selected, Delete is for the cells");
    assert_eq!(n25(&h), Value::Empty);

    // Undo brings the object back, and an undo that removes the selected object clears it.
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(exists(&h, o.button));
    h.state_mut().selected_object = Some(o.button);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert!(exists(&h, o.shape) && exists(&h, o.button));
    h.state_mut().run("object.delete", json!({"kind": "control", "id": o.button.id})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().selected_object, None);
}

#[test]
fn double_clicks_route_by_kind() {
    let (mut h, o) = fixture();
    {
        let sh = h.state().session.doc().unwrap().wb.active().unwrap();
        assert_eq!(open_action(sh, o.text_box), OpenAction::EditText(o.text_box.id));
        assert_eq!(open_action(sh, o.chart), OpenAction::FormatChart(o.chart.id));
        assert_eq!(open_action(sh, o.equation), OpenAction::EditEquation(o.equation.id));
        assert_eq!(open_action(sh, o.button), OpenAction::EditControl(o.button.id));
        assert_eq!(open_action(sh, o.shape), OpenAction::Select);
        assert_eq!(open_action(sh, o.image), OpenAction::Select);
    }
    double_click_on(&mut h, o.text_box);
    assert!(h.state().text_box_editor.as_ref().is_some_and(|e| e.id == o.text_box.id), "text boxes keep in-place editing");
    let outside = rect(&h, o.text_box).right_bottom() + vec2(200.0, 150.0);
    click(&mut h, outside);
    assert!(h.state().text_box_editor.is_none());

    let r = rect(&h, o.chart);
    double_click(&mut h, r.left_bottom() + vec2(12.0, -12.0));
    assert_eq!(h.state().grid.pane.as_deref(), Some("formatChart"));
    assert_eq!(h.state().selected_object, Some(o.chart));

    for obj in [o.equation, o.button, o.shape] {
        double_click_on(&mut h, obj);
        assert_eq!(h.state().selected_object, Some(obj));
        assert!(h.state().text_box_editor.is_none() && h.state().editor.is_none(), "{obj:?} opens no text editor");
    }
    assert_eq!(n25(&h), Value::text("keep me"));
}
