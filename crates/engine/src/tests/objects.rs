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

fn anchor_of(s: &Session, sheet_i: usize, o: ObjRef) -> Anchor {
    sheet(s, sheet_i).object_anchor(o).unwrap()
}

fn link(s: &Session, sheet_i: usize, id: u32) -> String {
    sheet(s, sheet_i).controls.iter().find(|c| c.id == id).and_then(|c| c.link.clone()).unwrap_or_default()
}

#[test]
fn shapes_equations_and_controls_follow_row_and_column_edits() {
    let mut s = s();
    let shape =
        ObjRef::new(ObjKind::Shape, s.execute("insert.shape", json!({"kind": "rectangle", "at": "C5"})).unwrap()["shape"].as_u64().unwrap() as u32);
    let equation = ObjRef::new(ObjKind::Equation, add_equation(&mut s, 0, "D8", "e^{i\\pi}+1=0"));
    let control = ObjRef::new(ObjKind::Control, add_control(&mut s, 0, "E6", ControlKind::Spin, None));
    let pinned = ObjRef::new(ObjKind::Control, add_control(&mut s, 0, "F9", ControlKind::Button, None));
    s.execute("object.setAnchorMode", json!({"kind": "control", "id": pinned.id, "mode": "absolute"})).unwrap();
    let cells = |s: &Session| [shape, equation, control, pinned].map(|o| anchor_of(s, 0, o).cell.a1());

    s.execute("home.insertRows", json!({"rows": "2:3"})).unwrap();
    assert_eq!(cells(&s), ["C7", "D10", "E8", "F9"]);
    s.execute("home.insertColumns", json!({"cols": "A:A"})).unwrap();
    assert_eq!(cells(&s), ["D7", "E10", "F8", "F9"]);
    s.execute("home.deleteColumns", json!({"cols": "A:A"})).unwrap();
    s.execute("home.deleteRows", json!({"rows": "2:3"})).unwrap();
    assert_eq!(cells(&s), ["C5", "D8", "E6", "F9"]);
    // Deleting the control's own row collapses it onto the row that takes its place.
    s.execute("home.deleteRows", json!({"rows": "6:6"})).unwrap();
    assert_eq!(anchor_of(&s, 0, control).cell.a1(), "E6");
    assert_eq!(anchor_of(&s, 0, equation).cell.a1(), "D7");
    // Sorting moves a control anchored inside the block with its row.
    s.execute("range.setValues", json!({"range": "H1", "values": [["k"], [3], [1], [2]]})).unwrap();
    let row_ctl = ObjRef::new(ObjKind::Control, add_control(&mut s, 0, "H2", ControlKind::Button, None));
    s.execute("selection.set", json!({"range": "H1:H4"})).unwrap();
    s.execute("data.sortAscending", json!({"header": true, "column": "H"})).unwrap();
    assert_eq!(anchor_of(&s, 0, row_ctl).cell.a1(), "H4", "the row holding 3 sorts last");
    // One undo restores the anchors.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(anchor_of(&s, 0, row_ctl).cell.a1(), "H2");
}

#[test]
fn control_links_follow_inserts_deletes_renames_and_cut_moves() {
    let mut s = s();
    s.execute("home.insertSheet", json!({"name": "Data"})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    let local = add_control(&mut s, 0, "E2", ControlKind::Slider, Some("$B$2"));
    let remote = add_control(&mut s, 0, "E6", ControlKind::Spin, Some("Data!$A$1"));
    // A control on the other sheet pointing back at this one.
    let back = add_control(&mut s, 1, "C3", ControlKind::Spin, Some("Sheet1!$B$2"));

    s.execute("home.insertRows", json!({"rows": "1:1"})).unwrap();
    s.execute("home.insertColumns", json!({"cols": "A:A"})).unwrap();
    assert_eq!(link(&s, 0, local), "$C$3");
    assert_eq!(link(&s, 1, back), "Sheet1!$C$3");
    assert_eq!(link(&s, 0, remote), "Data!$A$1", "edits on Sheet1 leave links into Data alone");

    s.execute("cell.set", json!({"cell": "C3", "input": "7"})).unwrap();
    s.execute("edit.cut", json!({"range": "C3"})).unwrap();
    s.execute("edit.paste", json!({"at": "G9"})).unwrap();
    assert_eq!(link(&s, 0, local), "$G$9", "a cut-move takes the link with the cell");
    assert_eq!(link(&s, 1, back), "Sheet1!$G$9");

    s.execute("sheet.rename", json!({"sheet": 1, "name": "Inputs"})).unwrap();
    assert_eq!(link(&s, 0, remote), "Inputs!$A$1");
    s.execute("home.deleteRows", json!({"rows": "9:9"})).unwrap();
    assert!(link(&s, 0, local).contains("#REF!"), "{}", link(&s, 0, local));
    s.execute("home.deleteSheet", json!({"sheet": 1})).unwrap();
    assert!(link(&s, 0, remote).contains("#REF!"), "{}", link(&s, 0, remote));
    // Undo walks it all back.
    for _ in 0..2 {
        s.execute("edit.undo", json!({})).unwrap();
    }
    assert_eq!((link(&s, 0, local), link(&s, 0, remote)), ("$G$9".to_string(), "Inputs!$A$1".to_string()));
}

#[test]
fn chart_series_follow_a_cut_move_and_a_sheet_delete() {
    let mut s = s();
    s.execute("range.setValues", json!({"range": "A1", "values": [[3], [5], [2], [8]]})).unwrap();
    let chart = s.execute("insert.chart", json!({"range": "A1:A4", "type": "column"})).unwrap()["chart"].as_u64().unwrap() as u32;
    let values = |s: &Session| sheet(s, 0).charts.iter().find(|c| c.id == chart).map(|c| c.series[0].values.clone()).unwrap_or_default();
    s.execute("edit.cut", json!({"range": "A1:A4"})).unwrap();
    s.execute("edit.paste", json!({"at": "F2"})).unwrap();
    assert!(values(&s).contains("$F$2:$F$5"), "{}", values(&s));
    s.execute("home.insertSheet", json!({"name": "Other"})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).sheet_mut(0).unwrap().charts[0].series[0].values = "Other!$A$1:$A$3".into();
    s.execute("home.deleteSheet", json!({"sheet": 1})).unwrap();
    assert!(values(&s).contains("#REF!"), "{}", values(&s));
}

#[test]
fn a_copied_sheet_gives_its_objects_fresh_ids() {
    let mut s = s();
    let objs = every_kind(&mut s);
    s.execute("sheet.move", json!({"sheet": 0, "to": 1, "copy": true})).unwrap();
    let wb = &s.doc().unwrap().wb;
    let mut ids: Vec<u32> = wb
        .sheets
        .iter()
        .flat_map(|sh| {
            sh.charts
                .iter()
                .map(|c| c.id)
                .chain(sh.images.iter().map(|c| c.id))
                .chain(sh.shapes.iter().map(|c| c.id))
                .chain(sh.controls.iter().map(|c| c.id))
        })
        .collect();
    assert_eq!(ids.len(), 2 * objs.len());
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 2 * objs.len(), "every object id stays unique across the workbook");
    let copy = sheet(&s, 1);
    assert_eq!(copy.controls.len(), 1);
    assert_eq!(copy.object_ref(copy.controls[0].id).map(|o| o.kind), Some(ObjKind::Control));
    assert!(s.doc().unwrap().wb.next_object_id() > *ids.last().unwrap_or(&0));
}
