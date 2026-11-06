//! Dates and times in Python cells, through the fake worker: what reaches `xl()`, and what a result
//! leaves in the cells.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use gridcraft_core::date::DateSystem;
use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_python::fake::{FakeHandle, FakeTransport};
use gridcraft_python::protocol::Request;
use gridcraft_python::{Clock, Day, Input, Limits, Stamp};
use serde_json::json;

use super::v;
use crate::Session;
use crate::python::PythonCapability;

const JAN15: Day = Day { year: 2024, month: 1, day: 15 };
const NOON: Clock = Clock { hour: 12, minute: 0, second: 0, micro: 0 };

/// A session whose worker answers every cell with nothing and shows what it was sent.
fn session_with_inputs() -> (Session, FakeHandle) {
    session_on(gridcraft_python::Output::Value(Value::Empty))
}

fn session_on(answer: gridcraft_python::Output) -> (Session, FakeHandle) {
    let t = FakeTransport::answering(move |_, _| Ok(answer.clone()));
    let h = t.handle();
    let mut s = Session::new();
    s.execute("file.new", json!({})).unwrap();
    s.set_python_capability(PythonCapability {
        enabled: true,
        limits: Limits::default(),
        transport: Some(Arc::new(move || Box::new(t.sharing()))),
        ..PythonCapability::default()
    });
    (s, h)
}

fn settle(s: &mut Session) {
    let mut left = 4000;
    while s.python_busy() {
        s.python_poll(Duration::from_millis(5));
        left -= 1;
        assert!(left > 0, "Python never settled");
    }
}

fn set(s: &mut Session, cell: &str, input: &str) {
    s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
}

fn py(s: &mut Session, cell: &str, code: &str) {
    s.execute("python.set", json!({"cell": cell, "code": code})).unwrap();
}

fn format(s: &mut Session, range: &str, code: &str) {
    s.execute("home.numberFormat", json!({"range": range, "code": code})).unwrap();
}

fn on_1904(s: &mut Session) {
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).date_system = DateSystem::D1904;
}

/// What the last run of the cells' code was sent, by reference.
fn sent(h: &FakeHandle) -> BTreeMap<String, Input> {
    h.requests()
        .into_iter()
        .rev()
        .find_map(|r| match r {
            Request::Run { inputs, .. } => Some(inputs),
            Request::Reset { .. } => None,
        })
        .expect("a cell ran")
}

fn runs(h: &FakeHandle) -> usize {
    h.requests().iter().filter(|r| matches!(r, Request::Run { .. })).count()
}

fn stamp_of(i: &Input) -> Option<Stamp> {
    i.stamps.first().map(|(_, s)| *s)
}

#[test]
fn a_date_format_makes_a_cell_a_date_in_xl() {
    let (mut s, h) = session_with_inputs();
    for (cell, input, code) in [
        ("A1", "45306", "m/d/yyyy"),
        ("A2", "45306.5", "m/d/yyyy h:mm"),
        ("A3", "0.5", "h:mm:ss"),
        ("A4", "45306", "0.00"),
        ("A5", "1.5", "[h]:mm:ss"),
        ("A6", "45306", "General"),
    ] {
        set(&mut s, cell, input);
        format(&mut s, cell, code);
    }
    set(&mut s, "A7", "text");
    format(&mut s, "A7", "m/d/yyyy");
    py(&mut s, "B1", r#"[xl("A1"), xl("A2"), xl("A3"), xl("A4"), xl("A5"), xl("A6"), xl("A7")]"#);
    settle(&mut s);
    let inputs = sent(&h);
    assert_eq!(stamp_of(&inputs["A1"]), Some(Stamp::Date(JAN15)));
    assert_eq!(stamp_of(&inputs["A2"]), Some(Stamp::DateTime(JAN15, NOON)));
    assert_eq!(stamp_of(&inputs["A3"]), Some(Stamp::Time(NOON)));
    for (reference, value) in
        [("A4", Value::Number(45306.0)), ("A5", Value::Number(1.5)), ("A6", Value::Number(45306.0)), ("A7", Value::text("text"))]
    {
        assert_eq!((&inputs[reference].value, stamp_of(&inputs[reference])), (&value, None), "{reference}");
    }
}

#[test]
fn a_range_says_which_of_its_cells_are_dates() {
    let (mut s, h) = session_with_inputs();
    s.execute("range.setValues", json!({"range": "A1", "values": [["when", 45306], [45307, 5], [true, "x"]]})).unwrap();
    format(&mut s, "A1:A3", "m/d/yyyy");
    format(&mut s, "B1", "h:mm");
    py(&mut s, "D1", r#"xl("A1:B3")"#);
    settle(&mut s);
    let input = &sent(&h)["A1:B3"];
    let Value::Array(a) = &input.value else { panic!("{input:?}") };
    assert_eq!(a.data, vec![Value::text("when"), Value::Empty, Value::Empty, Value::Number(5.0), Value::Bool(true), Value::text("x")]);
    // In place order, and a time format with a whole number of days keeps them.
    assert_eq!(input.stamps, vec![(1, Stamp::DateTime(JAN15, Clock::MIDNIGHT)), (2, Stamp::Date(Day { day: 16, ..JAN15 }))]);
}

#[test]
fn a_spill_and_a_whole_column_read_by_their_formats() {
    let (mut s, h) = session_with_inputs();
    set(&mut s, "A1", "=SEQUENCE(3,1,45306)");
    format(&mut s, "A2", "m/d/yyyy");
    py(&mut s, "C1", r#"[xl("A1#"), xl("A:A")]"#);
    settle(&mut s);
    let inputs = sent(&h);
    for reference in ["A1#", "A:A"] {
        assert_eq!(inputs[reference].stamps, vec![(1, Stamp::Date(Day { day: 16, ..JAN15 }))], "{reference}");
    }
}

#[test]
fn the_date_system_of_the_workbook_decides_the_day() {
    let (mut s, h) = session_with_inputs();
    on_1904(&mut s);
    set(&mut s, "A1", "43844");
    format(&mut s, "A1", "m/d/yyyy");
    set(&mut s, "A2", "0");
    format(&mut s, "A2", "m/d/yyyy");
    set(&mut s, "A3", "60");
    format(&mut s, "A3", "m/d/yyyy");
    py(&mut s, "B1", r#"[xl("A1"), xl("A2"), xl("A3")]"#);
    settle(&mut s);
    let inputs = sent(&h);
    assert_eq!(stamp_of(&inputs["A1"]), Some(Stamp::Date(JAN15)));
    assert_eq!(stamp_of(&inputs["A2"]), Some(Stamp::Date(Day { year: 1904, month: 1, day: 1 })), "a real day here");
    assert_eq!(stamp_of(&inputs["A3"]), Some(Stamp::Date(Day { year: 1904, month: 3, day: 1 })), "a real day here");
}

/// What no calendar has stays a number, and the cell runs.
#[test]
fn a_number_with_no_place_on_the_calendar_goes_as_a_number() {
    let (mut s, h) = session_with_inputs();
    for (cell, input) in [("A1", "60"), ("A2", "0"), ("A3", "0.5"), ("A4", "-1"), ("A5", "2958466"), ("A6", "1E+300"), ("A7", "60.5")] {
        set(&mut s, cell, input);
    }
    format(&mut s, "A1:A7", "m/d/yyyy");
    py(&mut s, "B1", r#"[xl("A1"), xl("A2"), xl("A3"), xl("A4"), xl("A5"), xl("A6"), xl("A7")]"#);
    settle(&mut s);
    let inputs = sent(&h);
    assert_eq!(inputs.len(), 7);
    for (reference, input) in &inputs {
        assert!(input.stamps.is_empty() && matches!(input.value, Value::Number(_)), "{reference}: {input:?}");
    }
    // The days on either side of the one that never was are real.
    set(&mut s, "A1", "59");
    set(&mut s, "A7", "61");
    settle(&mut s);
    let inputs = sent(&h);
    assert_eq!(stamp_of(&inputs["A1"]), Some(Stamp::Date(Day { year: 1900, month: 2, day: 28 })));
    assert_eq!(stamp_of(&inputs["A7"]), Some(Stamp::Date(Day { year: 1900, month: 3, day: 1 })));
}

#[test]
fn a_time_format_gives_a_time_below_one_and_a_date_above() {
    let (mut s, h) = session_with_inputs();
    for (cell, input) in [("A1", "0"), ("A2", "0.5"), ("A3", "45306.5"), ("A4", "60.5"), ("A5", "-0.5")] {
        set(&mut s, cell, input);
    }
    format(&mut s, "A1:A5", "h:mm");
    py(&mut s, "B1", r#"[xl("A1"), xl("A2"), xl("A3"), xl("A4"), xl("A5")]"#);
    settle(&mut s);
    let inputs = sent(&h);
    assert_eq!(stamp_of(&inputs["A1"]), Some(Stamp::Time(Clock::MIDNIGHT)));
    assert_eq!(stamp_of(&inputs["A2"]), Some(Stamp::Time(NOON)));
    assert_eq!(stamp_of(&inputs["A3"]), Some(Stamp::DateTime(JAN15, NOON)));
    assert_eq!(inputs["A4"].value, Value::Number(60.5), "the day that never was");
    assert_eq!(inputs["A5"].value, Value::Number(-0.5));
}

#[test]
fn a_new_format_runs_what_reads_the_cell_and_others_do_not() {
    let (mut s, h) = session_with_inputs();
    set(&mut s, "A1", "45306");
    py(&mut s, "B1", r#"xl("A1")"#);
    settle(&mut s);
    assert_eq!(runs(&h), 1);
    assert_eq!(stamp_of(&sent(&h)["A1"]), None);
    // A number to a date: Python sees something else.
    format(&mut s, "A1", "m/d/yyyy");
    settle(&mut s);
    assert_eq!(runs(&h), 2);
    assert_eq!(stamp_of(&sent(&h)["A1"]), Some(Stamp::Date(JAN15)));
    // Another date format: Python sees the same.
    format(&mut s, "A1", "yyyy-mm-dd");
    settle(&mut s);
    assert_eq!(runs(&h), 2);
    format(&mut s, "A1", "0.00");
    settle(&mut s);
    assert_eq!(runs(&h), 3);
    assert_eq!(stamp_of(&sent(&h)["A1"]), None);
    // A format on something else runs nothing.
    format(&mut s, "A1", "0.0");
    set(&mut s, "C1", "text");
    format(&mut s, "C1", "m/d/yyyy");
    settle(&mut s);
    assert_eq!(runs(&h), 3);
}

/// A spilled value has no cell to change, so the format of its column is all that tells.
#[test]
fn a_new_column_format_reaches_what_reads_a_spill() {
    let (mut s, h) = session_with_inputs();
    set(&mut s, "A1", "=SEQUENCE(2,1,45306)");
    py(&mut s, "C1", r#"xl("A2")"#);
    settle(&mut s);
    assert_eq!(runs(&h), 1);
    assert_eq!(stamp_of(&sent(&h)["A2"]), None);
    format(&mut s, "A:A", "m/d/yyyy");
    settle(&mut s);
    assert_eq!(runs(&h), 2);
    assert_eq!(stamp_of(&sent(&h)["A2"]), Some(Stamp::Date(Day { day: 16, ..JAN15 })));
    // A row's format comes before its column's.
    format(&mut s, "2:2", "0.00");
    settle(&mut s);
    assert_eq!(runs(&h), 3);
    assert_eq!(stamp_of(&sent(&h)["A2"]), None);
    // A column's width isn't its format.
    s.execute("home.columnWidth", json!({"cols": "A", "width": 30})).unwrap();
    settle(&mut s);
    assert_eq!(runs(&h), 3);
}

#[test]
fn a_constant_name_has_no_format() {
    let (mut s, h) = session_with_inputs();
    s.execute("formulas.defineName", json!({"name": "Rate", "refersTo": "=45306"})).unwrap();
    py(&mut s, "B1", r#"xl("Rate")"#);
    settle(&mut s);
    assert_eq!(v(&s, "B1"), Value::Empty);
    assert_eq!(sent(&h)["Rate"].value, Value::Number(45306.0));
}

#[test]
fn an_error_in_a_date_cell_stays_an_error() {
    let (mut s, h) = session_with_inputs();
    set(&mut s, "A1", "=1/0");
    format(&mut s, "A1", "m/d/yyyy");
    py(&mut s, "B1", r#"xl("A1")"#);
    settle(&mut s);
    assert_eq!(sent(&h)["A1"].value, Value::Error(CellError::Div0));
    assert!(s.doc().unwrap().wb.active().unwrap().cell(CellRef::parse("B1").unwrap()).is_some());
}
