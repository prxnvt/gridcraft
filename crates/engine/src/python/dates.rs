//! Dates and times in Python cells. The worker never sees a serial number: only here is the
//! workbook's date system known, so results are turned into serials here.

use gridcraft_core::date::DateSystem;
use gridcraft_core::{CellError, Value};
use gridcraft_model::PyOutput;
use gridcraft_python::{Dated, Stamp};
use std::sync::Arc;

/// A result with dates in it, as a cell keeps it: its dates are serial numbers in `sys`, or `#NUM!`
/// where a serial can't hold one.
pub(super) fn converted(dated: Dated, sys: DateSystem) -> PyOutput {
    let Dated { mut value, stamps } = dated;
    let number = |stamp: Stamp| stamp.serial(sys).map_or_else(Value::Error, Value::Number);
    if let Value::Array(a) = &mut value {
        let (rows, cols) = (a.rows, a.cols);
        let array = Arc::make_mut(a);
        for (at, stamp) in &stamps {
            if let Some(slot) = array.data.get_mut(*at as usize) {
                *slot = number(*stamp);
            }
        }
        return PyOutput { value, type_name: "list".into(), preview: format!("{rows} rows, {cols} columns"), convertible: true };
    }
    match stamps.first() {
        Some((_, stamp)) => PyOutput { value: number(*stamp), type_name: stamp.type_name().into(), preview: stamp.preview(), convertible: true },
        None => PyOutput { value: Value::Error(CellError::Value), type_name: "error".into(), preview: String::new(), convertible: true },
    }
}
