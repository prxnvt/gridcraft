//! Dates and times in Python cells. The worker never sees a serial number: only here is the
//! workbook's date system known, so a cell's number becomes a date here when its format says it is
//! one, and a result's dates become serials here.

use std::collections::HashMap;
use std::sync::Arc;

use gridcraft_core::date::DateSystem;
use gridcraft_core::{CellError, Value};
use gridcraft_model::{PyOutput, StyleId, Workbook};
use gridcraft_numfmt::NumberFormat;
use gridcraft_python::{Dated, Stamp};

/// The date formats among a workbook's styles, so that reading a column of numbers parses no format
/// more than once.
#[derive(Default)]
pub(super) struct DateFormats(HashMap<StyleId, Option<NumberFormat>>);

impl DateFormats {
    /// What a number in a cell of this style stands for, or `None` for a plain number. An elapsed
    /// format such as `[h]:mm` shows a length of time, not a moment, so it stays a number of days.
    pub fn stamp(&mut self, wb: &Workbook, style: StyleId, n: f64) -> Option<Stamp> {
        let format = self
            .0
            .entry(style)
            .or_insert_with(|| Some(crate::display::number_format(wb.styles.get(style).num_fmt.as_str())).filter(NumberFormat::is_date));
        Stamp::from_serial(wb.date_system, format.as_ref()?.date_shape(n)?, n)
    }
}

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
