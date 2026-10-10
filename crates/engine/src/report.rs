//! Analysis reports: blocks of labelled results that a command writes to an output range, a new
//! worksheet or a new workbook in one step. The Analysis ToolPak, Solver and Scenario Manager
//! all produce their output this way.
//!
//! A command first resolves where the output goes ([`parse_output`]), so it knows the report's
//! top-left cell (formulas such as Moving Average's refer to it), then builds a [`Report`] with
//! offsets relative to that cell and hands both to [`write`].

use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef, Value, col_to_letters};
use gridcraft_model::{Anchor, BorderLine, BorderStyle, Cell, Chart, ChartKind, Formula, LegendPos, LineInfo, Series, StyleId};
use serde_json::{Value as Json, json};

use crate::cmd::{Ctx, bad, edit};
use crate::{EngineError, Result, Session};

/// Most cells a report may write (inputs are capped well below this; it guards the writer).
pub const MAX_REPORT_CELLS: usize = 2_000_000;

/// Where a report goes.
#[derive(Clone, Debug, PartialEq)]
pub enum OutputKind {
    /// An existing sheet, starting at a cell.
    Range { sheet: usize },
    /// A new worksheet inserted before the active one (Excel's "New Worksheet Ply").
    NewSheet { name: String },
    /// A new workbook.
    NewWorkbook,
}

/// A resolved output destination.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub kind: OutputKind,
    /// Top-left cell of the report (A1 on a new sheet or workbook).
    pub origin: CellRef,
}

impl Target {
    /// Absolute A1 text of the cell at `(row, col)` relative to the origin (for report formulas).
    pub fn abs(&self, row: u32, col: u32) -> String {
        let c = CellRef::new(self.origin.row.saturating_add(row), self.origin.col.saturating_add(col));
        format!("${}${}", col_to_letters(c.col), c.row + 1)
    }
    /// Relative A1 text of the cell at `(row, col)` relative to the origin.
    pub fn rel(&self, row: u32, col: u32) -> String {
        CellRef::new(self.origin.row.saturating_add(row), self.origin.col.saturating_add(col)).a1()
    }
}

/// What a report cell holds.
#[derive(Clone, Debug, PartialEq)]
pub enum RValue {
    Value(Value),
    /// Formula text (with or without `=`), already in terms of the output position.
    Formula(String),
}

/// Formatting of a report cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RStyle {
    pub bold: bool,
    pub italic: bool,
    pub center: bool,
    pub top: bool,
    pub bottom: bool,
    pub num_fmt: Option<String>,
}

impl RStyle {
    /// A column or row heading inside a results table.
    pub fn header() -> RStyle {
        RStyle { italic: true, center: true, ..RStyle::default() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RCell {
    pub row: u32,
    pub col: u32,
    pub value: RValue,
    pub style: RStyle,
}

/// A chart range: a block of the report itself, or any reference text (for input data).
#[derive(Clone, Debug, PartialEq)]
pub enum RRange {
    Out { row: u32, col: u32, rows: u32, cols: u32 },
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RSeries {
    pub name: Option<String>,
    pub categories: Option<RRange>,
    pub values: RRange,
    /// On a combo chart: drawn as a line on the secondary axis.
    pub line: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RChart {
    pub kind: ChartKind,
    pub title: Option<String>,
    pub x_title: Option<String>,
    pub y_title: Option<String>,
    pub series: Vec<RSeries>,
    /// Top-left cell, relative to the origin.
    pub row: u32,
    pub col: u32,
    pub width: f32,
    pub height: f32,
    pub legend: bool,
}

/// A block of results.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub cells: Vec<RCell>,
    /// Column widths in points, by column offset.
    pub col_widths: Vec<(u32, f32)>,
    /// Rows (offsets, inclusive) grouped one outline level deep.
    pub outline_rows: Vec<(u32, u32)>,
    pub charts: Vec<RChart>,
}

impl Report {
    pub fn new() -> Report {
        Report::default()
    }
    pub fn put(&mut self, row: u32, col: u32, v: impl Into<Value>, style: RStyle) {
        self.cells.push(RCell { row, col, value: RValue::Value(v.into()), style });
    }
    /// A number cell (non-finite numbers become `#NUM!`).
    pub fn num(&mut self, row: u32, col: u32, x: f64, style: RStyle) {
        let v = if x.is_finite() { Value::Number(x) } else { Value::Error(gridcraft_core::CellError::Num) };
        self.cells.push(RCell { row, col, value: RValue::Value(v), style });
    }
    pub fn text(&mut self, row: u32, col: u32, t: &str, style: RStyle) {
        self.cells.push(RCell { row, col, value: RValue::Value(Value::text(t)), style });
    }
    pub fn formula(&mut self, row: u32, col: u32, f: impl Into<String>, style: RStyle) {
        self.cells.push(RCell { row, col, value: RValue::Formula(f.into()), style });
    }
    /// The block the cells cover: (rows, cols), at least 1×1.
    pub fn extent(&self) -> (u32, u32) {
        let rows = self.cells.iter().map(|c| c.row.saturating_add(1)).max().unwrap_or(1);
        let cols = self.cells.iter().map(|c| c.col.saturating_add(1)).max().unwrap_or(1);
        (rows.max(1), cols.max(1))
    }
}

/// Reads the `output` parameter: `"H1"` / `"Sheet2!$H$1"` (an output range; its top-left cell is
/// used), `{"newSheet": "name"}` or `"newSheet"` (a new worksheet, default), or `"newWorkbook"`.
pub fn parse_output(s: &Session, p: &Json, cmd: &str) -> Result<Target> {
    let d = s.doc()?;
    let out = p.get("output").cloned().unwrap_or(Json::Null);
    let new_sheet = |name: Option<&str>| -> Result<Target> {
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => d.wb.next_sheet_name(),
        };
        d.wb.check_sheet_name(&name, None).map_err(EngineError::Other)?;
        Ok(Target { kind: OutputKind::NewSheet { name }, origin: CellRef::new(0, 0) })
    };
    match &out {
        Json::Null => new_sheet(None),
        Json::Object(m) => {
            if m.contains_key("newWorkbook") {
                return Ok(Target { kind: OutputKind::NewWorkbook, origin: CellRef::new(0, 0) });
            }
            new_sheet(m.get("newSheet").and_then(Json::as_str))
        }
        Json::String(t) => match t.trim() {
            "" | "newSheet" => new_sheet(None),
            "newWorkbook" => Ok(Target { kind: OutputKind::NewWorkbook, origin: CellRef::new(0, 0) }),
            text => {
                let (sheet, body) = crate::cmd::split_sheet(text);
                let si = match sheet {
                    Some(n) => d.wb.sheet_index(&n).ok_or_else(|| bad(cmd, format!("no sheet named `{n}`")))?,
                    None => d.wb.active_sheet,
                };
                let r = RangeRef::parse(body.trim()).ok_or_else(|| bad(cmd, format!("`output` is not a reference: `{text}`")))?;
                Ok(Target { kind: OutputKind::Range { sheet: si }, origin: r.start })
            }
        },
        _ => Err(bad(cmd, "`output` must be a reference, \"newSheet\", {newSheet: name} or \"newWorkbook\"")),
    }
}

/// Error text for "the output range holds data": the UI asks and re-runs with `overwrite: true`.
pub const CONFIRM_OVERWRITE: &str = "The output range already contains data. Replace it?";

/// Writes `report` at `target` in one undoable step and returns `{sheet, range}`. Writing over
/// existing cells on a sheet needs `overwrite` (else [`EngineError::Confirm`]).
pub fn write(s: &mut Session, target: &Target, report: Report, overwrite: bool) -> Result<Json> {
    if report.cells.len() > MAX_REPORT_CELLS {
        return Err(EngineError::Other("The output is too large to write.".into()));
    }
    let (rows, cols) = report.extent();
    let end_row = target.origin.row.checked_add(rows - 1).filter(|r| *r < gridcraft_core::MAX_ROWS);
    let end_col = target.origin.col.checked_add(cols - 1).filter(|c| *c < gridcraft_core::MAX_COLS);
    let (Some(er), Some(ec)) = (end_row, end_col) else {
        return Err(EngineError::Other("The output doesn't fit on the sheet from that cell.".into()));
    };
    let area = RangeRef::new(target.origin, CellRef::new(er, ec));
    match &target.kind {
        OutputKind::Range { sheet } => {
            let d = s.doc()?;
            let sh = d.wb.sheet(*sheet).ok_or(EngineError::NoDocument)?;
            if !overwrite && sh.cells.iter_range(area).any(|(_, c)| !c.value.is_empty() || c.formula.is_some()) {
                return Err(EngineError::Confirm(CONFIRM_OVERWRITE.into()));
            }
            if sh.pivots.iter().any(|p| p.last_range.is_some_and(|r| r.intersection(&area).is_some())) {
                return Err(EngineError::Other("The output range overlaps a PivotTable.".into()));
            }
            write_into(s, *sheet, target.origin, report)?;
            let name = s.doc()?.wb.sheet(*sheet).map(|x| x.name.clone()).unwrap_or_default();
            Ok(json!({"sheet": name, "range": area.a1()}))
        }
        OutputKind::NewSheet { name } => {
            let before = s.doc()?.wb.active_sheet;
            crate::cmd::sheet::insert_sheet(s, &json!({"name": name, "before": before}))?;
            let si = s.doc()?.wb.active_sheet;
            write_into(s, si, target.origin, report)?;
            Ok(json!({"sheet": name, "range": area.a1()}))
        }
        OutputKind::NewWorkbook => {
            s.new_workbook();
            let si = s.doc()?.wb.active_sheet;
            write_into(s, si, target.origin, report)?;
            let name = s.doc()?.wb.sheet(si).map(|x| x.name.clone()).unwrap_or_default();
            Ok(json!({"sheet": name, "range": area.a1(), "workbook": s.doc()?.title.clone()}))
        }
    }
}

fn thin() -> BorderLine {
    BorderLine { style: BorderStyle::Thin, ..BorderLine::default() }
}

fn style_id(cx: &mut Ctx, st: &RStyle) -> StyleId {
    if *st == RStyle::default() {
        return StyleId::DEFAULT;
    }
    cx.wb.styles.derive(StyleId::DEFAULT, |s| {
        s.font.bold = st.bold;
        s.font.italic = st.italic;
        if st.center {
            s.align.h = gridcraft_model::HAlign::Center;
        }
        if st.top {
            s.borders.top = thin();
        }
        if st.bottom {
            s.borders.bottom = thin();
        }
        if let Some(f) = &st.num_fmt {
            s.num_fmt = gridcraft_model::NumFmt::new(f);
        }
    })
}

fn range_text(sheet_name: &str, origin: CellRef, r: &RRange) -> String {
    match r {
        RRange::Text(t) => t.clone(),
        RRange::Out { row, col, rows, cols } => {
            let a = CellRef::new(origin.row.saturating_add(*row), origin.col.saturating_add(*col));
            let b = CellRef::new(a.row.saturating_add(rows.saturating_sub(1)), a.col.saturating_add(cols.saturating_sub(1)));
            let abs = |c: CellRef| format!("${}${}", col_to_letters(c.col), c.row + 1);
            let body = if a == b { abs(a) } else { format!("{}:{}", abs(a), abs(b)) };
            format!("{}!{body}", gridcraft_formula::quote_sheet(sheet_name))
        }
    }
}

fn write_into(s: &mut Session, si: usize, origin: CellRef, report: Report) -> Result<()> {
    edit(s, |cx| {
        let sheet_name = cx.sheet_mut(si)?.name.clone();
        let mut next_id = cx.wb.next_object_id();
        for rc in &report.cells {
            let at = CellRef::new(origin.row.saturating_add(rc.row), origin.col.saturating_add(rc.col));
            let style = style_id(cx, &rc.style);
            let cell = match &rc.value {
                RValue::Value(v) => Cell { value: v.clone(), formula: None, style },
                RValue::Formula(f) => Cell { value: Value::Empty, formula: Some(Arc::new(Formula::new(f))), style },
            };
            cx.sheet_mut(si)?.set_cell(at, cell);
            cx.touch(si, at);
        }
        let sh = cx.sheet_mut(si)?;
        for (col, w) in &report.col_widths {
            let c = origin.col.saturating_add(*col);
            let e = sh.cols.entry(c).or_insert_with(LineInfo::default);
            e.size = Some(w.clamp(2.0, 1000.0));
        }
        for (a, b) in &report.outline_rows {
            for r in origin.row.saturating_add(*a)..=origin.row.saturating_add(*b) {
                let e = sh.rows.entry(r).or_insert_with(LineInfo::default);
                e.outline = e.outline.saturating_add(1).min(7);
            }
        }
        for ch in &report.charts {
            let series = ch
                .series
                .iter()
                .map(|x| Series {
                    name: x.name.clone(),
                    categories: x.categories.as_ref().map(|r| range_text(&sheet_name, origin, r)),
                    values: range_text(&sheet_name, origin, &x.values),
                    bubble_sizes: None,
                    color: None,
                    secondary: x.line,
                    kind: x.line.then_some(ChartKind::Line),
                    smooth: false,
                })
                .collect();
            sh.charts.push(Chart {
                id: next_id,
                kind: ch.kind,
                anchor: Anchor {
                    cell: CellRef::new(origin.row.saturating_add(ch.row), origin.col.saturating_add(ch.col)),
                    dx: 0.0,
                    dy: 0.0,
                    width: ch.width.clamp(40.0, 4000.0),
                    height: ch.height.clamp(40.0, 4000.0),
                    mode: gridcraft_model::AnchorMode::default(),
                },
                title: ch.title.clone(),
                series,
                legend: if ch.legend { LegendPos::Right } else { LegendPos::None },
                data_labels: false,
                gridlines: true,
                style: 1,
                x_title: ch.x_title.clone(),
                y_title: ch.y_title.clone(),
                source: None,
                by_rows: false,
            });
            next_id = next_id.saturating_add(1);
        }
        Ok(())
    })
}

/// Reads a boolean `overwrite` parameter (false when absent).
pub fn overwrite_param(p: &Json) -> bool {
    p.get("overwrite").and_then(Json::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    fn value(s: &Session, sheet: &str, a1: &str) -> Value {
        let d = s.doc().unwrap();
        let si = d.wb.sheet_index(sheet).unwrap();
        d.wb.sheet(si).unwrap().value(CellRef::parse(a1).unwrap())
    }

    fn sample() -> Report {
        let mut r = Report::new();
        r.text(0, 0, "Mean", RStyle::header());
        r.num(1, 0, 2.5, RStyle { bottom: true, num_fmt: Some("0.00".into()), ..RStyle::default() });
        r.num(1, 1, f64::NAN, RStyle::default());
        r
    }

    #[test]
    fn parses_output_targets() {
        let s = session();
        let t = parse_output(&s, &json!({"output": "Sheet1!$H$3:$J$9"}), "x").unwrap();
        assert_eq!(t, Target { kind: OutputKind::Range { sheet: 0 }, origin: CellRef::parse("H3").unwrap() });
        assert_eq!(t.abs(1, 1), "$I$4");
        assert_eq!(parse_output(&s, &json!({}), "x").unwrap().kind, OutputKind::NewSheet { name: "Sheet2".into() });
        assert_eq!(parse_output(&s, &json!({"output": {"newSheet": "Stats"}}), "x").unwrap().kind, OutputKind::NewSheet { name: "Stats".into() });
        assert_eq!(parse_output(&s, &json!({"output": "newWorkbook"}), "x").unwrap().kind, OutputKind::NewWorkbook);
        assert!(parse_output(&s, &json!({"output": {"newSheet": "Sheet1"}}), "x").is_err(), "duplicate sheet name");
        assert!(parse_output(&s, &json!({"output": "Nope!A1"}), "x").is_err());
        assert!(parse_output(&s, &json!({"output": "not a ref"}), "x").is_err());
        assert!(parse_output(&s, &json!({"output": 5}), "x").is_err());
    }

    #[test]
    fn writes_to_a_range_and_asks_before_overwriting() {
        let mut s = session();
        let t = parse_output(&s, &json!({"output": "B2"}), "x").unwrap();
        let r = write(&mut s, &t, sample(), false).unwrap();
        assert_eq!(r["range"], "B2:C3");
        assert_eq!(value(&s, "Sheet1", "B2"), Value::from("Mean"));
        assert_eq!(value(&s, "Sheet1", "B3"), Value::Number(2.5));
        assert_eq!(value(&s, "Sheet1", "C3"), Value::Error(gridcraft_core::CellError::Num));
        let d = s.doc().unwrap();
        let sh = d.wb.active().unwrap();
        let st = d.wb.styles.get(sh.style_id(CellRef::parse("B3").unwrap()));
        assert_eq!(st.borders.bottom.style, BorderStyle::Thin);
        assert_eq!(&*st.num_fmt.0, "0.00");
        assert!(d.wb.styles.get(sh.style_id(CellRef::parse("B2").unwrap())).font.italic);
        // Writing again over the same cells needs confirmation; one undo step removes the output.
        assert!(matches!(write(&mut s, &t, sample(), false), Err(EngineError::Confirm(_))));
        write(&mut s, &t, sample(), true).unwrap();
    }

    #[test]
    fn new_sheet_goes_before_the_active_one() {
        let mut s = session();
        s.execute("range.setValues", json!({"range": "A1", "values": [[1, 2]]})).unwrap();
        let mut rep = sample();
        rep.formula(2, 0, "=SUM(Sheet1!A1:B1)+A2", RStyle::default());
        rep.outline_rows.push((1, 2));
        rep.col_widths.push((0, 90.0));
        rep.charts.push(RChart {
            kind: ChartKind::Scatter,
            title: Some("Fit".into()),
            x_title: None,
            y_title: None,
            series: vec![RSeries {
                name: None,
                categories: Some(RRange::Text("Sheet1!$A$1:$B$1".into())),
                values: RRange::Out { row: 1, col: 0, rows: 2, cols: 1 },
                line: false,
            }],
            row: 4,
            col: 2,
            width: 300.0,
            height: 200.0,
            legend: false,
        });
        let t = parse_output(&s, &json!({"output": {"newSheet": "Out"}}), "x").unwrap();
        let before_sheets = s.doc().unwrap().wb.sheets.len();
        write(&mut s, &t, rep, false).unwrap();
        let d = s.doc().unwrap();
        assert_eq!(d.wb.sheets.len(), before_sheets + 1);
        assert_eq!(d.wb.sheet_index("Out"), Some(0));
        assert_eq!(d.wb.active().unwrap().name, "Out");
        assert_eq!(value(&s, "Out", "A3"), Value::Number(5.5));
        let sh = d.wb.sheet(0).unwrap();
        assert_eq!(sh.rows.get(&1).map(|r| r.outline), Some(1));
        assert_eq!(sh.cols.get(&0).and_then(|c| c.size), Some(90.0));
        let ch = &sh.charts[0];
        assert_eq!(ch.series[0].values, "Out!$A$2:$A$3");
        assert_eq!(ch.anchor.cell, CellRef::parse("C5").unwrap());
    }

    #[test]
    fn new_workbook_and_oversized_output() {
        let mut s = session();
        let t = parse_output(&s, &json!({"output": "newWorkbook"}), "x").unwrap();
        write(&mut s, &t, sample(), false).unwrap();
        assert_eq!(s.documents().len(), 2);
        assert_eq!(s.doc().unwrap().wb.active().unwrap().value(CellRef::new(0, 0)), Value::from("Mean"));
        let far = Target { kind: OutputKind::Range { sheet: 0 }, origin: CellRef::new(gridcraft_core::MAX_ROWS - 1, 0) };
        assert!(write(&mut s, &far, sample(), false).is_err());
    }
}
