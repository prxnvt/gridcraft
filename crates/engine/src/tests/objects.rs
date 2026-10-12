//! Floating objects of every kind: found, moved, listed, stacked and deleted by `{kind, id}`.

use gridcraft_core::CellRef;
use gridcraft_model::{AnchorMode, ObjKind, ObjRef, Sheet};
use serde_json::json;

use super::s;
use crate::Session;

fn at(a: &str) -> CellRef {
    CellRef::parse(a).unwrap()
}

fn sheet(s: &Session, i: usize) -> &Sheet {
    s.doc().unwrap().wb.sheet(i).unwrap()
}

fn id_of(v: &serde_json::Value, key: &str) -> u32 {
    v[key].as_u64().unwrap() as u32
}

/// A table with one object of each kind on the first sheet: a chart, a picture, a shape and a
/// slicer, in `ObjKind::ALL` order.
fn every_kind(s: &mut Session) -> [ObjRef; 4] {
    s.execute("range.setValues", json!({"range": "A1", "values": [["Region", "Qty"], ["East", 3], ["West", 5], ["East", 2], ["North", 8]]})).unwrap();
    s.execute("insert.table", json!({"range": "A1:B5", "name": "Sales"})).unwrap();
    let chart = id_of(&s.execute("insert.chart", json!({"range": "B1:B5", "type": "column", "at": "D2"})).unwrap(), "chart");
    let image = super::add_image(s, "D18", AnchorMode::MoveAndSize);
    let shape = id_of(&s.execute("insert.shape", json!({"kind": "ellipse", "at": "H18"})).unwrap(), "shape");
    s.execute("selection.set", json!({"cell": "A2"})).unwrap();
    let slicer = id_of(&s.execute("insert.slicer", json!({"column": "Region", "at": "L2"})).unwrap(), "slicer");
    [ObjRef::new(ObjKind::Chart, chart), ObjRef::new(ObjKind::Image, image), ObjRef::new(ObjKind::Shape, shape), ObjRef::new(ObjKind::Slicer, slicer)]
}

#[test]
fn every_kind_is_found_moved_and_deleted_by_kind_and_id() {
    let mut s = s();
    let objs = every_kind(&mut s);
    for o in objs {
        assert_eq!(sheet(&s, 0).object_ref(o.id), Some(o), "the id finds its kind");
        assert_eq!(ObjKind::parse(o.kind.name()), Some(o.kind));
        s.execute("object.move", json!({"kind": o.kind.name(), "id": o.id, "at": "M20", "width": 50, "height": 30})).unwrap();
        let a = sheet(&s, 0).object_anchor(o).unwrap();
        assert_eq!((a.cell, a.width, a.height), (at("M20"), 50.0, 30.0));
        s.execute("object.setAnchorMode", json!({"kind": o.kind.name(), "id": o.id, "mode": "absolute"})).unwrap();
        assert_eq!(sheet(&s, 0).object_anchor(o).unwrap().mode, AnchorMode::Absolute);
    }
    // A kind that does not match the id, an unknown id or kind, and a missing kind are refused.
    assert!(s.execute("object.delete", json!({"kind": "slicer", "id": objs[0].id})).is_err());
    assert!(s.execute("object.delete", json!({"kind": "chart", "id": 9999})).is_err());
    assert!(s.execute("object.delete", json!({"kind": "widget", "id": objs[0].id})).is_err());
    assert!(s.execute("object.delete", json!({"id": objs[0].id})).is_err(), "kind is required");
    for (n, o) in objs.into_iter().enumerate() {
        let out = s.execute("object.delete", json!({"kind": o.kind.name(), "id": o.id})).unwrap();
        assert_eq!(out["deleted"], json!({"kind": o.kind.name(), "id": o.id}));
        assert!(!sheet(&s, 0).has_object(o));
        for later in &objs[n + 1..] {
            assert!(sheet(&s, 0).has_object(*later), "deleting {o:?} leaves {later:?}");
        }
    }
    // One undo step per delete brings the slicer back.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(sheet(&s, 0).has_object(objs[3]));
}

#[test]
fn selection_pane_lists_and_stacks_slicers() {
    let mut s = s();
    let objs = every_kind(&mut s);
    let second = id_of(&s.execute("insert.slicer", json!({"column": "Qty", "at": "P2"})).unwrap(), "slicer");
    let list = s.execute("arrange.selectionPane", json!({})).unwrap();
    let items = list.as_array().unwrap();
    let kinds: Vec<&str> = items.iter().map(|o| o["kind"].as_str().unwrap()).collect();
    for k in ObjKind::ALL {
        assert!(kinds.contains(&k.name()), "{k:?} listed in {kinds:?}");
    }
    assert_eq!(kinds.last(), Some(&"slicer"), "slicers stack on top");
    let names: Vec<&str> = items.iter().map(|o| o["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Region") && names.contains(&"Qty"), "{names:?}");
    assert_eq!(sheet(&s, 0).slicers.first().map(|c| c.id), Some(objs[3].id));
    s.execute("arrange.sendBackward", json!({"kind": "slicer", "id": second, "toBack": true})).unwrap();
    assert_eq!(sheet(&s, 0).slicers.first().map(|c| c.id), Some(second));
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
    let o = objs[3];
    s.execute("object.move", json!({"kind": o.kind.name(), "id": o.id, "dx": 1e300, "dy": -1e300, "width": 1e300, "height": -5})).unwrap();
    let a = sheet(&s, 0).object_anchor(o).unwrap();
    assert!(a.dx.is_finite() && a.dy.is_finite() && a.width <= 10000.0 && a.height >= 0.0, "{a:?}");
}
