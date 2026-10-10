//! Floating objects of every kind: delete, move, list and stack them by `{kind, id}`.

use gridcraft_core::CellRef;
use gridcraft_model::{Anchor, Color, ControlKind, FormControl, ObjKind, ObjRef, Shape, ShapeKind, Sheet};
use serde_json::json;

use super::s;
use crate::Session;

fn at(a: &str) -> CellRef {
    CellRef::parse(a).unwrap_or_default()
}

fn anchor(cell: &str) -> Anchor {
    Anchor { cell: at(cell), dx: 0.0, dy: 0.0, width: 80.0, height: 40.0, ..Default::default() }
}

fn sheet_mut(s: &mut Session, i: usize) -> &mut Sheet {
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).sheet_mut(i).unwrap()
}

fn sheet(s: &Session, i: usize) -> &Sheet {
    s.doc().unwrap().wb.sheet(i).unwrap()
}

/// An equation (stored as a shape) on sheet `i`; the commands that make one come later.
pub(super) fn add_equation(s: &mut Session, i: usize, cell: &str, latex: &str) -> u32 {
    let id = s.doc().unwrap().wb.next_object_id();
    let shape = Shape { id, kind: ShapeKind::Equation, anchor: anchor(cell), fill: Color::Auto, line: Color::Auto, text: latex.into() };
    sheet_mut(s, i).shapes.push(shape);
    id
}

/// A form control on sheet `i`; the commands that make one come later.
pub(super) fn add_control(s: &mut Session, i: usize, cell: &str, kind: ControlKind, link: Option<&str>) -> u32 {
    let id = s.doc().unwrap().wb.next_object_id();
    let control = FormControl { id, kind, anchor: anchor(cell), text: String::new(), link: link.map(str::to_string) };
    sheet_mut(s, i).controls.push(control);
    id
}

/// One object of each kind on the first sheet.
fn every_kind(s: &mut Session) -> [ObjRef; 5] {
    for (i, n) in [3, 5, 2, 8].into_iter().enumerate() {
        s.execute("cell.set", json!({"cell": CellRef::new(i as u32, 0).a1(), "input": n.to_string()})).unwrap();
    }
    let chart = s.execute("insert.chart", json!({"range": "A1:A4", "type": "column"})).unwrap()["chart"].as_u64().unwrap() as u32;
    let image = super::add_image(s, "F2", Default::default());
    let shape = s.execute("insert.shape", json!({"kind": "ellipse", "at": "F8"})).unwrap()["shape"].as_u64().unwrap() as u32;
    let equation = add_equation(s, 0, "J2", "x^2");
    let control = add_control(s, 0, "J8", ControlKind::Slider, Some("$B$1"));
    [
        ObjRef::new(ObjKind::Chart, chart),
        ObjRef::new(ObjKind::Image, image),
        ObjRef::new(ObjKind::Shape, shape),
        ObjRef::new(ObjKind::Equation, equation),
        ObjRef::new(ObjKind::Control, control),
    ]
}

#[test]
fn every_kind_is_found_moved_and_deleted_by_kind_and_id() {
    let mut s = s();
    let objs = every_kind(&mut s);
    for o in objs {
        assert_eq!(sheet(&s, 0).object_ref(o.id), Some(o), "the id finds its kind");
        s.execute("object.move", json!({"kind": o.kind.name(), "id": o.id, "at": "M20", "width": 50, "height": 30})).unwrap();
        let a = sheet(&s, 0).object_anchor(o).unwrap();
        assert_eq!((a.cell, a.width, a.height), (at("M20"), 50.0, 30.0));
        s.execute("object.setAnchorMode", json!({"kind": o.kind.name(), "id": o.id, "mode": "absolute"})).unwrap();
    }
    // An equation is a shape too; a plain shape is not an equation.
    assert!(sheet(&s, 0).has_object(ObjRef::new(ObjKind::Shape, objs[3].id)));
    assert!(s.execute("object.delete", json!({"kind": "equation", "id": objs[2].id})).is_err());
    assert!(s.execute("object.delete", json!({"kind": "control", "id": objs[0].id})).is_err());
    assert!(s.execute("object.delete", json!({"kind": "chart", "id": 9999})).is_err());
    assert!(s.execute("object.delete", json!({"kind": "widget", "id": objs[0].id})).is_err());
    assert!(s.execute("object.delete", json!({"id": objs[0].id})).is_err(), "kind is required");
    for (n, o) in objs.into_iter().enumerate() {
        let out = s.execute("object.delete", json!({"kind": o.kind.name(), "id": o.id})).unwrap();
        assert_eq!(out["deleted"], json!({"kind": o.kind.name(), "id": o.id}));
        assert!(!sheet(&s, 0).has_object(o));
        // The others are untouched.
        for later in &objs[n + 1..] {
            assert!(sheet(&s, 0).has_object(*later));
        }
    }
    // One undo step per delete brings the control back.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(sheet(&s, 0).has_object(objs[4]));
}

#[test]
fn selection_pane_lists_and_stacks_equations_and_controls() {
    let mut s = s();
    let objs = every_kind(&mut s);
    let second = add_control(&mut s, 0, "A20", ControlKind::Button, None);
    let list = s.execute("arrange.selectionPane", json!({})).unwrap();
    let kinds: Vec<&str> = list.as_array().unwrap().iter().map(|o| o["kind"].as_str().unwrap()).collect();
    for k in ["chart", "image", "shape", "equation", "control"] {
        assert!(kinds.contains(&k), "{k} listed in {kinds:?}");
    }
    assert_eq!(kinds.last(), Some(&"control"), "controls stack on top");
    let names: Vec<&str> = list.as_array().unwrap().iter().map(|o| o["name"].as_str().unwrap()).collect();
    assert!(names.contains(&format!("Slider {}", objs[4].id).as_str()) && names.contains(&format!("Button {second}").as_str()));
    s.execute("arrange.sendBackward", json!({"kind": "control", "id": second, "toBack": true})).unwrap();
    assert_eq!(sheet(&s, 0).controls.first().map(|c| c.id), Some(second));
}

#[test]
fn hostile_object_params_are_refused() {
    let mut s = s();
    let objs = every_kind(&mut s);
    for p in [
        json!({}),
        json!({"kind": "chart"}),
        json!({"kind": "chart", "id": -1}),
        json!({"kind": "chart", "id": u64::MAX}),
        json!({"kind": 7, "id": objs[0].id}),
        json!({"kind": null, "id": objs[0].id}),
    ] {
        for cmd in ["object.delete", "object.move", "object.setAnchorMode"] {
            assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
        }
    }
    s.execute("object.move", json!({"kind": "control", "id": objs[4].id, "dx": 1e300, "dy": -1e300, "width": 1e300, "height": -5})).unwrap();
    let a = sheet(&s, 0).object_anchor(objs[4]).unwrap();
    assert!(a.dx.is_finite() && a.dy.is_finite() && a.width <= 10000.0 && a.height >= 0.0);
}
