//! PivotTables: reading the source list, grouping and aggregating, laying out the report and
//! writing it onto the sheet as static, styled cells.
//!
//! Behaviour follows Excel's documented PivotTable model (rows/columns/values/filters areas,
//! Compact/Outline/Tabular layouts, subtotals, grand totals, show-values-as); the code and the
//! look (our own palette) are original.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use gridcraft_core::date::datetime_from_serial;
use gridcraft_core::{CellError, CellRef, MAX_COLS, MAX_ROWS, RangeRef, Value};
use gridcraft_model::*;

/// Largest source block read (cells).
pub const MAX_SOURCE_CELLS: u64 = 10_000_000;
/// Largest report written (cells).
pub const MAX_OUTPUT_CELLS: u64 = 2_000_000;
/// Most fields in the Rows plus Columns areas.
pub const MAX_AXIS_FIELDS: usize = 32;
/// Most fields in the Values area.
pub const MAX_VALUE_FIELDS: usize = 256;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// ---------------------------------------------------------------- source

/// The source list: header names, per-column number formats and the records.
#[derive(Clone, Debug)]
pub struct Source {
    pub sheet: usize,
    pub range: RangeRef,
    pub headers: Vec<String>,
    pub formats: Vec<String>,
    /// Every non-blank value in the column is a number.
    pub numeric: Vec<bool>,
    /// Numeric with a date number format.
    pub dates: Vec<bool>,
    pub rows: Vec<Vec<Value>>,
}

impl Source {
    pub fn col(&self, name: &str) -> Option<usize> {
        self.headers.iter().position(|h| h.eq_ignore_ascii_case(name))
    }
}

/// `'My Sheet'!A1` → (`Some("My Sheet")`, `A1`).
fn split_sheet(r: &str) -> (Option<String>, &str) {
    match r.rsplit_once('!') {
        Some((sh, body)) => (Some(sh.trim_matches('\'').replace("''", "'")), body),
        None => (None, r),
    }
}

/// Quotes a sheet name for a reference when needed.
pub fn quote_sheet(name: &str) -> String {
    if !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Absolute reference text for a range on a sheet: `Sheet1!$A$1:$E$20`.
pub fn range_text(sheet_name: &str, r: RangeRef) -> String {
    let abs = |c: CellRef| format!("${}${}", gridcraft_core::col_to_letters(c.col), c.row as u64 + 1);
    format!("{}!{}:{}", quote_sheet(sheet_name), abs(r.start), abs(r.end))
}

/// Resolves source text (range, table name or defined name) to (sheet, range including header).
pub fn resolve_source(wb: &Workbook, text: &str, default_sheet: usize) -> Result<(usize, RangeRef), String> {
    resolve_inner(wb, text, default_sheet, 0)
}

fn resolve_inner(wb: &Workbook, text: &str, default_sheet: usize, depth: u32) -> Result<(usize, RangeRef), String> {
    const INVALID: &str = "The reference isn't valid.";
    let t = text.trim().trim_start_matches('=').trim();
    if t.is_empty() || depth > 4 {
        return Err(INVALID.into());
    }
    if let Some((si, ti)) = wb.table(t)
        && let Some(tb) = wb.sheet(si).and_then(|s| s.tables.get(ti))
    {
        if !tb.header_row {
            return Err("The table needs a header row to be used as a PivotTable source.".into());
        }
        let end_row = tb.range.end.row.saturating_sub(tb.totals_row as u32).max(tb.range.start.row);
        return Ok((si, RangeRef::new(tb.range.start, CellRef::new(end_row, tb.range.end.col))));
    }
    let (sheet, body) = split_sheet(t);
    if let Some(r) = RangeRef::parse(body) {
        let si = match sheet {
            Some(n) => wb.sheet_index(&n).ok_or_else(|| INVALID.to_string())?,
            None => default_sheet,
        };
        let sh = wb.sheet(si).ok_or_else(|| INVALID.to_string())?;
        let r = if r.is_full_cols() || r.is_full_rows() {
            let used = sh.used_range().ok_or_else(|| INVALID.to_string())?;
            r.intersection(&used).ok_or_else(|| INVALID.to_string())?
        } else {
            r
        };
        return Ok((si, r));
    }
    if sheet.is_none()
        && let Some(n) = wb.name(t, default_sheet)
    {
        let f = n.formula.clone();
        return resolve_inner(wb, &f, default_sheet, depth + 1);
    }
    Err(INVALID.into())
}

/// What identifies a pivot's cache: the resolved source plus how it was named (a table, a
/// defined name or a range). PivotTables with the same key share grouping and refresh together,
/// as PivotTables on one PivotCache do in Excel.
pub fn cache_key(wb: &Workbook, text: &str, default_sheet: usize) -> Option<(usize, RangeRef, String)> {
    let (si, r) = resolve_source(wb, text, default_sheet).ok()?;
    let t = text.trim().trim_start_matches('=').trim();
    let kind = if wb.table(t).is_some() {
        format!("table:{}", t.to_lowercase())
    } else if RangeRef::parse(split_sheet(t).1).is_some() {
        "range".to_string()
    } else {
        format!("name:{}", t.to_lowercase())
    };
    Some((si, r, kind))
}

/// Every PivotTable sharing a cache with the one at (sheet, idx), itself included.
pub fn sharing(wb: &Workbook, sheet: usize, idx: usize) -> Vec<(usize, usize)> {
    let Some(key) = wb.sheet(sheet).and_then(|s| s.pivots.get(idx)).and_then(|pt| cache_key(wb, &pt.source, sheet)) else {
        return vec![(sheet, idx)];
    };
    let mut out = Vec::new();
    for (si, sh) in wb.sheets.iter().enumerate() {
        for (pi, pt) in sh.pivots.iter().enumerate() {
            if (si, pi) == (sheet, idx) || cache_key(wb, &pt.source, si).as_ref() == Some(&key) {
                out.push((si, pi));
            }
        }
    }
    out
}

const FIELD_NAME_ERR: &str =
    "The PivotTable field name is not valid. To create a PivotTable, you must use data that is organized as a list with labeled columns.";

/// Reads the source list.
pub fn read_source(wb: &Workbook, text: &str, default_sheet: usize) -> Result<Source, String> {
    let (si, r) = resolve_source(wb, text, default_sheet)?;
    let sh = wb.sheet(si).ok_or("The reference isn't valid.")?;
    if r.count() > MAX_SOURCE_CELLS {
        return Err("The source range is too large for a PivotTable.".into());
    }
    let mut headers: Vec<String> = Vec::new();
    // Names taken so far (lower-case), and the next suffix to try for each repeated header, so
    // that thousands of equal headers don't take cubic time.
    let mut taken: HashSet<String> = HashSet::new();
    let mut next_suffix: HashMap<String, usize> = HashMap::new();
    let mut formats = Vec::new();
    for c in r.start.col..=r.end.col {
        let h = crate::display::cell_text(wb, sh, CellRef::new(r.start.row, c)).trim().to_string();
        if h.is_empty() {
            return Err(FIELD_NAME_ERR.into());
        }
        let key = h.to_ascii_lowercase();
        let mut n = h.clone();
        let mut k = next_suffix.get(&key).copied().unwrap_or(2);
        while taken.contains(&n.to_ascii_lowercase()) {
            n = format!("{h}{k}");
            k += 1;
        }
        next_suffix.insert(key, k);
        taken.insert(n.to_ascii_lowercase());
        headers.push(n);
        let fmt = if r.end.row > r.start.row {
            wb.styles.get(sh.style_id(CellRef::new(r.start.row + 1, c))).num_fmt.as_str().to_string()
        } else {
            "General".into()
        };
        formats.push(fmt);
    }
    let w = headers.len();
    let mut rows = Vec::new();
    let mut numeric = vec![true; w];
    let mut any = vec![false; w];
    if r.end.row > r.start.row {
        for row in r.start.row + 1..=r.end.row {
            let rec: Vec<Value> = (r.start.col..=r.end.col).map(|c| sh.value(CellRef::new(row, c)).scalar()).collect();
            if rec.iter().all(Value::is_empty) {
                continue;
            }
            for (i, v) in rec.iter().enumerate() {
                if !v.is_empty() {
                    if let Some(a) = any.get_mut(i) {
                        *a = true;
                    }
                    if !v.is_number()
                        && let Some(n) = numeric.get_mut(i)
                    {
                        *n = false;
                    }
                }
            }
            rows.push(rec);
        }
    }
    for (n, a) in numeric.iter_mut().zip(any.iter()) {
        *n = *n && *a;
    }
    let dates = numeric.iter().zip(formats.iter()).map(|(n, f)| *n && crate::display::number_format(f).is_date()).collect();
    Ok(Source { sheet: si, range: r, headers, formats, numeric, dates, rows })
}

/// The source as a PivotTable sees it: the list plus its manual group fields.
pub fn read_pivot_source(wb: &Workbook, pt: &PivotTable, default_sheet: usize) -> Result<Source, String> {
    let mut src = read_source(wb, &pt.source, default_sheet)?;
    for mg in &pt.manual_groups {
        let Some(c) = src.col(&mg.base) else { continue };
        if src.col(&mg.name).is_some() {
            continue;
        }
        let fmt = src.formats.get(c).cloned().unwrap_or_else(|| "General".into());
        src.headers.push(mg.name.clone());
        src.formats.push("General".into());
        src.numeric.push(false);
        src.dates.push(false);
        for row in src.rows.iter_mut() {
            let label = item_label(wb, row.get(c).unwrap_or(&Value::Empty), &Grouping::default(), &fmt);
            let v = mg.groups.iter().find(|g| g.items.iter().any(|i| i.eq_ignore_ascii_case(&label))).map(|g| g.name.clone()).unwrap_or(label);
            row.push(Value::text(v));
        }
    }
    Ok(src)
}

// ---------------------------------------------------------------- items

#[derive(Clone, Debug)]
struct Item {
    value: Value,
    label: String,
    class: u8,
    num: f64,
    text: String,
    first: usize,
    /// The value is a number shown with the source format.
    formatted: bool,
}

#[derive(Default)]
struct FieldItems {
    items: Vec<Item>,
    index: HashMap<String, u32>,
}

fn cmp_items(a: &Item, b: &Item) -> Ordering {
    a.class.cmp(&b.class).then(a.num.total_cmp(&b.num)).then_with(|| a.text.cmp(&b.text))
}

/// How a field's values become items: as they are, by date part (with an optional day span and
/// bounds), or by number range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grouping {
    pub date: PivotDateGroup,
    pub days: u32,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub number: Option<PivotNumberGroup>,
}

impl Grouping {
    pub fn of(f: &PivotField) -> Grouping {
        Grouping { date: f.date_group, days: f.group_days, start: f.group_start, end: f.group_end, number: f.number_group }
    }
}

fn date_text(wb: &Workbook, serial: f64) -> String {
    crate::display::format(&Value::Number(serial), "m/d/yyyy", wb).text
}

fn num_text(x: f64) -> String {
    gridcraft_core::number_to_text((x * 1e9).round() / 1e9)
}

/// The "a-b" label of the number range containing `n`.
fn range_item(g: &PivotNumberGroup, n: f64) -> (String, f64) {
    let by = if g.by > 0.0 && g.by.is_finite() { g.by } else { 1.0 };
    let k = ((n - g.start) / by).floor();
    let lo = g.start + k * by;
    // Whole-number ranges read 0-9, 10-19; others 0-0.5, 0.5-1.
    let whole = g.start.fract() == 0.0 && by.fract() == 0.0 && by >= 1.0;
    let hi = if whole { lo + by - 1.0 } else { lo + by };
    (format!("{}-{}", num_text(lo), num_text(hi)), lo)
}

/// (identity key, item) for a source value under a field's grouping.
fn make_item(wb: &Workbook, v: &Value, g: &Grouping, fmt: &str, first: usize) -> (String, Item) {
    let it = |value: Value, label: String, class: u8, num: f64, formatted: bool| Item {
        text: label.to_lowercase(),
        value,
        label,
        class,
        num,
        first,
        formatted,
    };
    let text_item = |label: String, num: f64, key: String| (key, it(Value::text(label.as_str()), label, 0, num, false));
    match v {
        Value::Empty => ("b".into(), it(Value::text("(blank)"), "(blank)".into(), 4, 0.0, false)),
        Value::Number(n) => {
            let n = *n;
            if let Some(ng) = &g.number {
                if n < ng.start {
                    return text_item(format!("<{}", num_text(ng.start)), f64::MIN, "lt".into());
                }
                if n > ng.end {
                    return text_item(format!(">{}", num_text(ng.end)), f64::MAX, "gt".into());
                }
                let (label, lo) = range_item(ng, n);
                return text_item(label, lo, format!("r{}", lo.to_bits()));
            }
            if g.date != PivotDateGroup::None
                && let Some(dt) = datetime_from_serial(wb.date_system, n)
            {
                if let Some(s0) = g.start
                    && n < s0
                {
                    return text_item(format!("<{}", date_text(wb, s0)), f64::MIN, "lt".into());
                }
                // Ending at is a date shown as is; days after it fall into ">end".
                if let Some(e0) = g.end
                    && n.floor() > e0
                {
                    return text_item(format!(">{}", date_text(wb, e0)), f64::MAX, "gt".into());
                }
                let mon = MONTHS.get(dt.month.saturating_sub(1) as usize).copied().unwrap_or("Jan");
                let (label, key) = match g.date {
                    PivotDateGroup::Years => (dt.year.to_string(), dt.year as f64),
                    PivotDateGroup::Quarters => {
                        let q = (dt.month.saturating_sub(1)) / 3 + 1;
                        (format!("Qtr{q}"), q as f64)
                    }
                    PivotDateGroup::Months => (mon.to_string(), dt.month as f64),
                    PivotDateGroup::Days if g.days > 1 => {
                        // Spans of N days from the starting date.
                        let base = g.start.unwrap_or(0.0).floor();
                        let k = ((n.floor() - base) / g.days as f64).floor();
                        let lo = base + k * g.days as f64;
                        (format!("{} - {}", date_text(wb, lo), date_text(wb, lo + g.days as f64 - 1.0)), lo)
                    }
                    PivotDateGroup::Hours => {
                        let h = dt.hour;
                        (format!("{} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" }), h as f64)
                    }
                    PivotDateGroup::Minutes => (format!(":{:02}", dt.minute), dt.minute as f64),
                    PivotDateGroup::Seconds => (format!(":{:02}", dt.second), dt.second as f64),
                    _ => (format!("{}-{mon}", dt.day), (dt.month * 100 + dt.day) as f64),
                };
                return (format!("g{key}"), it(Value::text(label.as_str()), label, 0, key, false));
            }
            let label = crate::display::format(v, fmt, wb).text;
            (format!("n{}", n.to_bits()), it(v.clone(), label, 0, n, true))
        }
        Value::Text(t) => (format!("t{}", t.to_lowercase()), it(v.clone(), t.to_string(), 1, 0.0, false)),
        Value::Bool(b) => {
            let l = if *b { "TRUE" } else { "FALSE" };
            (format!("l{l}"), it(Value::Bool(*b), l.into(), 2, *b as u8 as f64, false))
        }
        Value::Error(e) => (format!("e{}", e.as_str()), it(v.clone(), e.as_str().into(), 3, 0.0, false)),
        Value::Array(_) => make_item(wb, &v.scalar(), g, fmt, first),
    }
}

impl FieldItems {
    fn intern(&mut self, wb: &Workbook, v: &Value, group: &Grouping, fmt: &str, first: usize) -> u32 {
        let (key, item) = make_item(wb, v, group, fmt, first);
        if let Some(i) = self.index.get(&key) {
            return *i;
        }
        let i = self.items.len() as u32;
        self.items.push(item);
        self.index.insert(key, i);
        i
    }
    fn get(&self, i: u32) -> Option<&Item> {
        self.items.get(i as usize)
    }
    fn label(&self, i: u32) -> String {
        self.get(i).map(|x| x.label.clone()).unwrap_or_default()
    }
}

/// The label of a source value under a grouping (for filters and field lists).
pub fn item_label(wb: &Workbook, v: &Value, group: &Grouping, fmt: &str) -> String {
    make_item(wb, v, group, fmt, 0).1.label
}

/// Distinct item labels of a source column, sorted like the field list shows them.
pub fn distinct_labels(wb: &Workbook, src: &Source, col: usize, group: &Grouping, limit: usize) -> Vec<String> {
    let mut fi = FieldItems::default();
    let fmt = src.formats.get(col).map(String::as_str).unwrap_or("General");
    for (i, r) in src.rows.iter().enumerate() {
        if let Some(v) = r.get(col) {
            fi.intern(wb, v, group, fmt, i);
        }
        if fi.items.len() > 100_000 {
            break;
        }
    }
    let mut items = fi.items;
    items.sort_by(cmp_items);
    items.into_iter().take(limit).map(|i| i.label).collect()
}

// ---------------------------------------------------------------- aggregation

#[derive(Clone, Debug, Default)]
struct Acc {
    count: u64,
    nums: u64,
    sum: f64,
    prod: f64,
    min: f64,
    max: f64,
    mean: f64,
    m2: f64,
    err: Option<CellError>,
}

impl Acc {
    fn add(&mut self, v: &Value) {
        match v {
            Value::Empty => {}
            Value::Number(n) => {
                let n = *n;
                self.count += 1;
                self.nums += 1;
                self.sum += n;
                if self.nums == 1 {
                    self.prod = n;
                    self.min = n;
                    self.max = n;
                } else {
                    self.prod *= n;
                    self.min = self.min.min(n);
                    self.max = self.max.max(n);
                }
                let d = n - self.mean;
                self.mean += d / self.nums as f64;
                self.m2 += d * (n - self.mean);
            }
            Value::Error(e) => {
                self.count += 1;
                if self.err.is_none() {
                    self.err = Some(*e);
                }
            }
            _ => self.count += 1,
        }
    }
    fn result(&self, f: PivotFunc) -> Value {
        let n = self.nums as f64;
        match f {
            PivotFunc::Count => return Value::number(self.count as f64),
            PivotFunc::CountNumbers => return Value::number(n),
            _ => {}
        }
        if let Some(e) = self.err {
            return Value::Error(e);
        }
        let div0 = Value::Error(CellError::Div0);
        match f {
            PivotFunc::Sum => Value::number(self.sum),
            PivotFunc::Average => {
                if self.nums == 0 {
                    div0
                } else {
                    Value::number(self.sum / n)
                }
            }
            PivotFunc::Max => Value::number(if self.nums == 0 { 0.0 } else { self.max }),
            PivotFunc::Min => Value::number(if self.nums == 0 { 0.0 } else { self.min }),
            PivotFunc::Product => Value::number(if self.nums == 0 { 0.0 } else { self.prod }),
            PivotFunc::StdDev | PivotFunc::Var => {
                if self.nums < 2 {
                    div0
                } else {
                    let var = (self.m2 / (n - 1.0)).max(0.0);
                    Value::number(if f == PivotFunc::Var { var } else { var.sqrt() })
                }
            }
            PivotFunc::StdDevP | PivotFunc::VarP => {
                if self.nums < 1 {
                    div0
                } else {
                    let var = (self.m2 / n).max(0.0);
                    Value::number(if f == PivotFunc::VarP { var } else { var.sqrt() })
                }
            }
            PivotFunc::Count | PivotFunc::CountNumbers => Value::Empty,
        }
    }
}

/// Excel's caption verb for a function ("Sum of Sales").
pub fn func_caption(f: PivotFunc) -> &'static str {
    match f {
        PivotFunc::Sum => "Sum",
        PivotFunc::Count => "Count",
        PivotFunc::Average => "Average",
        PivotFunc::Max => "Max",
        PivotFunc::Min => "Min",
        PivotFunc::Product => "Product",
        PivotFunc::CountNumbers => "Count Numbers",
        PivotFunc::StdDev => "StdDev",
        PivotFunc::StdDevP => "StdDevp",
        PivotFunc::Var => "Var",
        PivotFunc::VarP => "Varp",
    }
}

// ---------------------------------------------------------------- trees

#[derive(Clone, Debug)]
struct Node {
    item: u32,
    children: Vec<Node>,
}

fn build_tree(paths: &[&[u32]], depth: usize, fields: &[&PivotField], items: &[FieldItems]) -> Vec<Node> {
    let (Some(field), Some(fi)) = (fields.get(depth), items.get(depth)) else { return vec![] };
    let mut order: Vec<u32> = Vec::new();
    let mut groups: HashMap<u32, Vec<&[u32]>> = HashMap::new();
    for p in paths {
        if let Some(&it) = p.get(depth) {
            let g = groups.entry(it).or_default();
            if g.is_empty() {
                order.push(it);
            }
            g.push(p);
        }
    }
    let mut nodes: Vec<Node> = order
        .into_iter()
        .map(|it| {
            let ps = groups.remove(&it).unwrap_or_default();
            Node { item: it, children: build_tree(&ps, depth + 1, fields, items) }
        })
        .collect();
    let by = |a: &Node, b: &Node| match (fi.get(a.item), fi.get(b.item)) {
        (Some(x), Some(y)) => match field.sort {
            PivotSort::Asc => cmp_items(x, y),
            PivotSort::Desc => cmp_items(y, x),
            PivotSort::None => x.first.cmp(&y.first),
        },
        _ => Ordering::Equal,
    };
    nodes.sort_by(by);
    nodes
}

fn is_collapsed(field: &PivotField, label: &str) -> bool {
    field.collapsed_items.iter().any(|c| c.eq_ignore_ascii_case(label))
}

fn collect_siblings(nodes: &[Node], prefix: &mut Vec<u32>, out: &mut HashMap<Vec<u32>, Vec<u32>>) {
    out.insert(prefix.clone(), nodes.iter().map(|n| n.item).collect());
    for n in nodes {
        prefix.push(n.item);
        collect_siblings(&n.children, prefix, out);
        prefix.pop();
    }
}

// ---------------------------------------------------------------- output

/// What a report cell is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum CellKind {
    Empty,
    Header,
    RowLabel,
    ColLabel,
    Data,
    Subtotal,
    GrandTotal,
    Filter,
    Placeholder,
}

/// What a report row is (for row-wide styling).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum RowKind {
    Filter,
    Blank,
    Header,
    /// Item row; `true` for a parent item (bold).
    Item(bool),
    Subtotal,
    Grand,
    Placeholder,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutCell {
    pub value: Value,
    pub kind: CellKind,
    pub indent: u8,
    pub num_fmt: Option<String>,
}

impl OutCell {
    fn empty() -> OutCell {
        OutCell { value: Value::Empty, kind: CellKind::Empty, indent: 0, num_fmt: None }
    }
}

/// A computed report: a dense grid relative to its top-left, with row kinds.
#[derive(Clone, Debug)]
pub struct Output {
    pub grid: Vec<Vec<OutCell>>,
    pub row_kinds: Vec<RowKind>,
    /// Rows above the body (report filters plus a blank row).
    pub body_offset: u32,
    pub width: u32,
    /// Field buttons (filter drop-downs): (row, column, field) relative to the top-left.
    pub buttons: Vec<(u32, u32, String)>,
}

impl Output {
    pub fn height(&self) -> u32 {
        self.grid.len() as u32
    }
    /// Value at (row, col) relative to the top-left.
    pub fn value(&self, r: usize, c: usize) -> Value {
        self.grid.get(r).and_then(|row| row.get(c)).map(|x| x.value.clone()).unwrap_or_default()
    }
}

struct Grid {
    rows: Vec<Vec<OutCell>>,
    kinds: Vec<RowKind>,
    width: usize,
    buttons: Vec<(u32, u32, String)>,
}

impl Grid {
    fn push_row(&mut self, kind: RowKind) -> Result<usize, String> {
        if (self.rows.len() as u64 + 1) * self.width.max(1) as u64 > MAX_OUTPUT_CELLS || self.rows.len() as u64 >= MAX_ROWS as u64 {
            return Err("The PivotTable report is too large.".into());
        }
        self.rows.push(vec![OutCell::empty(); self.width]);
        self.kinds.push(kind);
        Ok(self.rows.len() - 1)
    }
    fn set(&mut self, r: usize, c: usize, value: Value, kind: CellKind, indent: u8, fmt: Option<String>) {
        if let Some(cell) = self.rows.get_mut(r).and_then(|row| row.get_mut(c)) {
            *cell = OutCell { value, kind, indent, num_fmt: fmt };
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Item(bool),
    Subtotal,
    Grand,
    /// Field Settings › Insert blank line after each item.
    Blank,
}

type Label = (usize, Value, u8, Option<String>);

struct Line {
    kind: LineKind,
    path: Vec<u32>,
    values: bool,
    /// (column, value, indent, number format)
    labels: Vec<Label>,
    /// A custom subtotal's function (instead of each value field's own).
    func: Option<PivotFunc>,
}

impl Line {
    fn new(kind: LineKind, path: Vec<u32>, values: bool, labels: Vec<Label>) -> Line {
        Line { kind, path, values, labels, func: None }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColKind {
    Leaf,
    Subtotal,
    Grand,
}

struct ColSpec {
    kind: ColKind,
    path: Vec<u32>,
    func: Option<PivotFunc>,
}

/// A field's subtotal lines: `None` is the value fields' own functions (Automatic).
fn subtotal_funcs(pt: &PivotTable, f: &PivotField) -> Vec<Option<PivotFunc>> {
    match &f.subtotals {
        PivotSubtotals::Automatic if pt.subtotals => vec![None],
        PivotSubtotals::Automatic | PivotSubtotals::None => vec![],
        PivotSubtotals::Custom(list) => list.iter().map(|x| Some(*x)).collect(),
    }
}

/// "East Total", or "East Sum" for a custom subtotal.
fn subtotal_label(label: &str, func: Option<PivotFunc>) -> String {
    match func {
        Some(f) => format!("{label} {}", func_caption(f)),
        None => format!("{label} Total"),
    }
}

struct Ctx<'a> {
    pt: &'a PivotTable,
    rows: Vec<&'a PivotField>,
    row_items: Vec<FieldItems>,
    row_fmts: Vec<String>,
}

impl Ctx<'_> {
    fn label_cell(&self, depth: usize, item: u32) -> (Value, Option<String>, String) {
        let Some(it) = self.row_items.get(depth).and_then(|fi| fi.get(item)) else { return (Value::Empty, None, String::new()) };
        let fmt = if it.formatted { self.row_fmts.get(depth).cloned() } else { None };
        (it.value.clone(), fmt, it.label.clone())
    }

    /// Compact and outline forms. `repeat` holds outer labels to show on every line (outline
    /// form with Repeat Item Labels).
    fn flatten(&self, nodes: &[Node], depth: usize, prefix: &mut Vec<u32>, repeat: &[Label], out: &mut Vec<Line>) {
        let n = self.rows.len();
        let outline = self.pt.layout == PivotLayout::Outline;
        let step = self.pt.options.indent.min(127);
        let Some(field) = self.rows.get(depth) else { return };
        for node in nodes {
            prefix.push(node.item);
            let (v, fmt, label) = self.label_cell(depth, node.item);
            let (col, indent) = if outline { (depth, 0) } else { (0, (depth.min(250) as u8).saturating_mul(step)) };
            let mut labels: Vec<Label> = repeat.to_vec();
            labels.push((col, v.clone(), indent, fmt.clone()));
            let collapsed = depth + 1 < n && is_collapsed(field, &label);
            if depth + 1 >= n || collapsed || node.children.is_empty() {
                out.push(Line::new(LineKind::Item(depth + 1 < n), prefix.clone(), true, labels));
            } else {
                let subs = subtotal_funcs(self.pt, field);
                let top = subs == [None] && self.pt.subtotals_top && field.subtotals == PivotSubtotals::Automatic;
                out.push(Line::new(LineKind::Item(true), prefix.clone(), top, labels));
                let inner: Vec<Label> = if outline && field.repeat_labels {
                    let mut r = repeat.to_vec();
                    r.push((col, v, indent, fmt));
                    r
                } else {
                    vec![]
                };
                self.flatten(&node.children, depth + 1, prefix, &inner, out);
                if !top {
                    for func in subs {
                        let mut l =
                            Line::new(LineKind::Subtotal, prefix.clone(), true, vec![(col, Value::text(subtotal_label(&label, func)), indent, None)]);
                        l.func = func;
                        out.push(l);
                    }
                }
            }
            if field.blank_line {
                out.push(Line::new(LineKind::Blank, vec![], false, vec![]));
            }
            prefix.pop();
        }
    }

    fn flatten_tabular(&self, nodes: &[Node], depth: usize, prefix: &mut Vec<u32>, pending: &[Label], out: &mut Vec<Line>) {
        let n = self.rows.len();
        let Some(field) = self.rows.get(depth) else { return };
        for (i, node) in nodes.iter().enumerate() {
            prefix.push(node.item);
            let (v, fmt, label) = self.label_cell(depth, node.item);
            // Outer labels show on the first line of their group, or on every line when the
            // outer field repeats its labels.
            let mut labels: Vec<Label> =
                if i == 0 { pending.to_vec() } else { pending.iter().filter(|l| l.2 == 1).map(|l| (l.0, l.1.clone(), 0, l.3.clone())).collect() };
            labels.push((depth, v, u8::from(field.repeat_labels), fmt));
            let collapsed = depth + 1 < n && is_collapsed(field, &label);
            if depth + 1 >= n || collapsed || node.children.is_empty() {
                let shown: Vec<Label> = labels.iter().map(|l| (l.0, l.1.clone(), 0, l.3.clone())).collect();
                out.push(Line::new(LineKind::Item(false), prefix.clone(), true, shown));
            } else {
                self.flatten_tabular(&node.children, depth + 1, prefix, &labels, out);
                for func in subtotal_funcs(self.pt, field) {
                    let mut l =
                        Line::new(LineKind::Subtotal, prefix.clone(), true, vec![(depth, Value::text(subtotal_label(&label, func)), 0, None)]);
                    l.func = func;
                    out.push(l);
                }
            }
            if field.blank_line {
                out.push(Line::new(LineKind::Blank, vec![], false, vec![]));
            }
            prefix.pop();
        }
    }
}

fn flatten_cols(
    nodes: &[Node],
    depth: usize,
    pt: &PivotTable,
    fields: &[&PivotField],
    items: &[FieldItems],
    prefix: &mut Vec<u32>,
    out: &mut Vec<ColSpec>,
) {
    let n = fields.len();
    let Some(field) = fields.get(depth) else { return };
    for node in nodes {
        prefix.push(node.item);
        let label = items.get(depth).map(|fi| fi.label(node.item)).unwrap_or_default();
        let collapsed = depth + 1 < n && is_collapsed(field, &label);
        if depth + 1 >= n || collapsed || node.children.is_empty() {
            out.push(ColSpec { kind: ColKind::Leaf, path: prefix.clone(), func: None });
        } else {
            flatten_cols(&node.children, depth + 1, pt, fields, items, prefix, out);
            for func in subtotal_funcs(pt, field) {
                out.push(ColSpec { kind: ColKind::Subtotal, path: prefix.clone(), func });
            }
        }
        prefix.pop();
    }
}

/// Adds the items a field has no data for under every parent (Show items with no data).
fn add_empty_items(nodes: &mut Vec<Node>, depth: usize, fields: &[&PivotField], items: &[FieldItems]) {
    let (Some(field), Some(fi)) = (fields.get(depth), items.get(depth)) else { return };
    if field.show_empty {
        for i in 0..fi.items.len() as u32 {
            if !nodes.iter().any(|n| n.item == i) {
                nodes.push(Node { item: i, children: vec![] });
            }
        }
        nodes.sort_by(|a, b| match (fi.get(a.item), fi.get(b.item)) {
            (Some(x), Some(y)) => match field.sort {
                PivotSort::Asc => cmp_items(x, y),
                PivotSort::Desc => cmp_items(y, x),
                PivotSort::None => x.first.cmp(&y.first),
            },
            _ => Ordering::Equal,
        });
    }
    for n in nodes.iter_mut() {
        add_empty_items(&mut n.children, depth + 1, fields, items);
    }
}

type AggMap = HashMap<(Vec<u32>, Vec<u32>), Vec<Acc>>;

/// The column of a value field that is a calculated field (records add nothing to it).
const CALC_COL: usize = usize::MAX;

/// What a calculated field's formula uses.
#[derive(Clone, Copy)]
enum CalcRef {
    /// The sum of a source field, by its place in `CalcFields::hidden`.
    Col(usize),
    /// Another calculated field.
    Calc(usize),
}

/// The PivotTable's calculated fields, prepared once per compute.
struct CalcFields {
    names: Vec<String>,
    prepared: Vec<Option<(crate::pivot_formula::Prepared, Vec<CalcRef>)>>,
    /// Source columns the formulas use, summed alongside the value fields.
    hidden: Vec<usize>,
}

impl CalcFields {
    fn new(pt: &PivotTable, src: &Source) -> CalcFields {
        let names: Vec<String> = pt.calc_fields.iter().map(|f| f.name.clone()).collect();
        let all: Vec<String> = src.headers.iter().cloned().chain(names.iter().cloned()).collect();
        let mut hidden: Vec<usize> = Vec::new();
        let prepared = pt
            .calc_fields
            .iter()
            .map(|f| {
                let p = crate::pivot_formula::prepare(&f.formula, &all, None).ok()?;
                let refs = p
                    .refs
                    .iter()
                    .map(|r| match r.checked_sub(src.headers.len()) {
                        Some(ci) => CalcRef::Calc(ci),
                        None => CalcRef::Col(match hidden.iter().position(|h| h == r) {
                            Some(h) => h,
                            None => {
                                hidden.push(*r);
                                hidden.len() - 1
                            }
                        }),
                    })
                    .collect();
                Some((p, refs))
            })
            .collect();
        CalcFields { names, prepared, hidden }
    }
    fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.eq_ignore_ascii_case(name))
    }
}

/// Reads value field results from a cell's accumulators.
struct CellVals<'a> {
    vals: &'a [(usize, &'a PivotValue)],
    calc: &'a CalcFields,
    /// Number of value fields; hidden sums follow them.
    k: usize,
    wb: &'a Workbook,
    sheet: usize,
    at: CellRef,
}

impl CellVals<'_> {
    fn get(&self, accs: &[Acc], vi: usize, func: Option<PivotFunc>) -> Option<Value> {
        let (c, v) = self.vals.get(vi)?;
        if *c == CALC_COL {
            let i = self.calc.index(&v.source_col)?;
            return Some(self.calc_value(accs, i, 0));
        }
        accs.get(vi).map(|a| a.result(func.unwrap_or(v.func)))
    }
    /// A calculated field: its formula on the sums of the fields it names.
    fn calc_value(&self, accs: &[Acc], i: usize, depth: usize) -> Value {
        if depth > crate::pivot_formula::MAX_DEPTH {
            return Value::Error(CellError::Ref);
        }
        let Some(Some((p, refs))) = self.calc.prepared.get(i) else { return Value::Error(CellError::Name) };
        let args: Vec<Value> = refs
            .iter()
            .map(|r| match r {
                CalcRef::Col(h) => accs.get(self.k + h).map(|a| a.result(PivotFunc::Sum)).unwrap_or(Value::Number(0.0)),
                CalcRef::Calc(j) => self.calc_value(accs, *j, depth + 1),
            })
            .collect();
        crate::pivot_formula::eval(self.wb, self.sheet, self.at, p, &args)
    }
}

/// Computes a PivotTable report from the workbook.
pub fn compute(wb: &Workbook, pt: &PivotTable, pivot_sheet: usize) -> Result<Output, String> {
    let src = read_pivot_source(wb, pt, pivot_sheet)?;
    let fmt_of = |c: usize| src.formats.get(c).cloned().unwrap_or_else(|| "General".into());
    let rows: Vec<(usize, &PivotField)> = pt.rows.iter().filter_map(|f| src.col(&f.source_col).map(|c| (c, f))).collect();
    let cols: Vec<(usize, &PivotField)> = pt.columns.iter().filter_map(|f| src.col(&f.source_col).map(|c| (c, f))).collect();
    let calc = CalcFields::new(pt, &src);
    let vals: Vec<(usize, &PivotValue)> = pt
        .values
        .iter()
        .filter_map(|v| src.col(&v.source_col).map(|c| (c, v)).or_else(|| calc.index(&v.source_col).map(|_| (CALC_COL, v))))
        .collect();
    if rows.len() + cols.len() > MAX_AXIS_FIELDS {
        return Err(format!("A PivotTable can have at most {MAX_AXIS_FIELDS} row and column fields."));
    }
    if vals.len() > MAX_VALUE_FIELDS {
        return Err(format!("A PivotTable can have at most {MAX_VALUE_FIELDS} value fields."));
    }
    let axis_group = |name: &str| -> Option<Grouping> {
        pt.rows.iter().chain(pt.columns.iter()).find(|f| f.source_col.eq_ignore_ascii_case(name)).map(Grouping::of)
    };
    // (column, filter, grouping used for labels, shown as a report filter)
    let filters: Vec<(usize, &PivotFilter, Grouping, bool)> = pt
        .filters
        .iter()
        .filter_map(|f| {
            let c = src.col(&f.source_col)?;
            let g = axis_group(&f.source_col);
            let report = g.is_none();
            Some((c, f, g.unwrap_or_default(), report))
        })
        .collect();
    let shown_filters: Vec<&(usize, &PivotFilter, Grouping, bool)> = filters.iter().filter(|f| f.3).collect();

    let k = vals.len();
    // Columns each record adds to: the value fields, then the fields calculated fields use.
    let add_cols: Vec<usize> = vals.iter().map(|(c, _)| *c).chain(calc.hidden.iter().copied()).collect();
    let k_all = add_cols.len();
    let cv = CellVals { vals: &vals, calc: &calc, k, wb, sheet: pivot_sheet, at: pt.anchor };
    let empty = rows.is_empty() && cols.is_empty() && vals.is_empty();

    // ---- aggregate
    let mut row_items: Vec<FieldItems> = rows.iter().map(|_| FieldItems::default()).collect();
    let mut col_items: Vec<FieldItems> = cols.iter().map(|_| FieldItems::default()).collect();
    let filter_fmts: Vec<String> = filters.iter().map(|f| fmt_of(f.0)).collect();
    let row_fmts: Vec<String> = rows.iter().map(|(c, _)| fmt_of(*c)).collect();
    let col_fmts: Vec<String> = cols.iter().map(|(c, _)| fmt_of(*c)).collect();
    // Fields showing items with no data know every item of the source, except those their own
    // filter hides.
    for (ri, rec) in src.rows.iter().enumerate() {
        for (((c, f), fi), fmt) in
            rows.iter().zip(row_items.iter_mut()).zip(row_fmts.iter()).chain(cols.iter().zip(col_items.iter_mut()).zip(col_fmts.iter()))
        {
            if !f.show_empty {
                continue;
            }
            let v = rec.get(*c).unwrap_or(&Value::Empty);
            let own = pt.filters.iter().find(|x| x.source_col.eq_ignore_ascii_case(&f.source_col)).and_then(|x| x.selected.as_ref());
            if let Some(sel) = own {
                let label = item_label(wb, v, &Grouping::of(f), fmt);
                if !sel.iter().any(|x| x.eq_ignore_ascii_case(&label)) {
                    continue;
                }
            }
            fi.intern(wb, v, &Grouping::of(f), fmt, ri);
        }
    }
    let today = crate::cmd::pivot_today(wb);
    // Records that pass the item, label and date filters, with their row and column paths.
    let mut recs: Vec<(usize, Vec<u32>, Vec<u32>)> = Vec::new();
    'rec: for (ri, rec) in src.rows.iter().enumerate() {
        for ((c, f, g, report), fmt) in filters.iter().zip(filter_fmts.iter()) {
            let v = rec.get(*c).unwrap_or(&Value::Empty);
            if f.selected.is_some() || f.label.is_some() {
                let label = item_label(wb, v, g, fmt);
                if let Some(sel) = &f.selected
                    && !sel.iter().any(|s| s.eq_ignore_ascii_case(&label))
                {
                    continue 'rec;
                }
                if let Some(rule) = &f.label
                    && !*report
                    && !label_rule_ok(rule, &label, v, today)
                {
                    continue 'rec;
                }
            }
        }
        if empty {
            continue;
        }
        let mut rp = Vec::with_capacity(rows.len());
        for (((c, f), fi), fmt) in rows.iter().zip(row_items.iter_mut()).zip(row_fmts.iter()) {
            rp.push(fi.intern(wb, rec.get(*c).unwrap_or(&Value::Empty), &Grouping::of(f), fmt, ri));
        }
        let mut cp = Vec::with_capacity(cols.len());
        for (((c, f), fi), fmt) in cols.iter().zip(col_items.iter_mut()).zip(col_fmts.iter()) {
            cp.push(fi.intern(wb, rec.get(*c).unwrap_or(&Value::Empty), &Grouping::of(f), fmt, ri));
        }
        recs.push((ri, rp, cp));
    }
    let aggregate =
        |excl_rows: &HashSet<Vec<u32>>, excl_cols: &HashSet<Vec<u32>>| -> Result<(AggMap, HashSet<Vec<u32>>, HashSet<Vec<u32>>), String> {
            let mut aggs: AggMap = HashMap::new();
            let mut row_set: HashSet<Vec<u32>> = HashSet::new();
            let mut col_set: HashSet<Vec<u32>> = HashSet::new();
            let hit = |excl: &HashSet<Vec<u32>>, p: &[u32]| !excl.is_empty() && (1..=p.len()).any(|n| p.get(..n).is_some_and(|x| excl.contains(x)));
            for (ri, rp, cp) in &recs {
                if hit(excl_rows, rp) || hit(excl_cols, cp) {
                    continue;
                }
                let Some(rec) = src.rows.get(*ri) else { continue };
                for a in 0..=rp.len() {
                    for b in 0..=cp.len() {
                        let e = aggs.entry((rp[..a].to_vec(), cp[..b].to_vec())).or_insert_with(|| vec![Acc::default(); k_all]);
                        for (acc, c) in e.iter_mut().zip(add_cols.iter()) {
                            acc.add(rec.get(*c).unwrap_or(&Value::Empty));
                        }
                    }
                }
                row_set.insert(rp.clone());
                col_set.insert(cp.clone());
                if aggs.len() > 5_000_000 {
                    return Err("The PivotTable report is too large.".into());
                }
            }
            Ok((aggs, row_set, col_set))
        };
    let none = HashSet::new();
    let (mut aggs, mut row_set, mut col_set) = aggregate(&none, &none)?;
    // Value and Top 10 filters drop items (within each parent); totals then count only the rest.
    let value_rule = |f: &PivotField| pt.filters.iter().find(|x| x.source_col.eq_ignore_ascii_case(&f.source_col)).and_then(|x| x.value.as_ref());
    let row_rules: Vec<Option<&PivotRule>> = rows.iter().map(|(_, f)| value_rule(f)).collect();
    let col_rules: Vec<Option<&PivotRule>> = cols.iter().map(|(_, f)| value_rule(f)).collect();
    if k > 0 && (row_rules.iter().any(Option::is_some) || col_rules.iter().any(Option::is_some)) {
        let value_index =
            |field: &str| vals.iter().position(|(_, v)| v.name.eq_ignore_ascii_case(field) || v.source_col.eq_ignore_ascii_case(field)).unwrap_or(0);
        let total = |rp: &[u32], cp: &[u32], vi: usize| -> Option<f64> {
            aggs.get(&(rp.to_vec(), cp.to_vec())).and_then(|a| cv.get(a, vi, None)).and_then(|v| v.as_f64())
        };
        let mut excl_rows = HashSet::new();
        let mut excl_cols = HashSet::new();
        for (on_rows, rules, set, excl) in [(true, &row_rules, &row_set, &mut excl_rows), (false, &col_rules, &col_set, &mut excl_cols)] {
            let mut parents: HashMap<Vec<u32>, Vec<u32>> = HashMap::new();
            for path in set {
                for d in 0..path.len() {
                    let (Some(parent), Some(item)) = (path.get(..d), path.get(d)) else { continue };
                    let e = parents.entry(parent.to_vec()).or_default();
                    if !e.contains(item) {
                        e.push(*item);
                    }
                }
            }
            for (parent, items) in &parents {
                let Some(Some(rule)) = rules.get(parent.len()) else { continue };
                let field = match rule {
                    PivotRule::Value { field, .. } | PivotRule::Top { field, .. } => field.as_str(),
                    _ => continue,
                };
                let vi = value_index(field);
                let scored: Vec<(u32, Option<f64>)> = items
                    .iter()
                    .map(|it| {
                        let mut path = parent.clone();
                        path.push(*it);
                        (*it, if on_rows { total(&path, &[], vi) } else { total(&[], &path, vi) })
                    })
                    .collect();
                for it in rule_drops(rule, &scored) {
                    let mut path = parent.clone();
                    path.push(it);
                    excl.insert(path);
                }
            }
        }
        if !excl_rows.is_empty() || !excl_cols.is_empty() {
            (aggs, row_set, col_set) = aggregate(&excl_rows, &excl_cols)?;
        }
    }

    // Calculated items: each of their cells is the item's formula over the same cells of its
    // sibling items; totals above the field include them, as Excel's do.
    let mut over: HashMap<(Vec<u32>, Vec<u32>), Vec<Option<Value>>> = HashMap::new();
    for ci in &pt.calc_items {
        let (on_rows, d) = match rows.iter().position(|(_, f)| f.source_col.eq_ignore_ascii_case(&ci.field)) {
            Some(d) => (true, d),
            None => match cols.iter().position(|(_, f)| f.source_col.eq_ignore_ascii_case(&ci.field)) {
                Some(d) => (false, d),
                None => continue,
            },
        };
        // Hidden by the field's own item filter.
        let hidden = pt
            .filters
            .iter()
            .find(|f| f.source_col.eq_ignore_ascii_case(&ci.field))
            .and_then(|f| f.selected.as_ref())
            .is_some_and(|sel| !sel.iter().any(|x| x.eq_ignore_ascii_case(&ci.name)));
        if hidden || empty {
            continue;
        }
        let fi = match if on_rows { row_items.get_mut(d) } else { col_items.get_mut(d) } {
            Some(fi) => fi,
            None => continue,
        };
        let labels: Vec<String> = fi.items.iter().map(|i| i.label.clone()).collect();
        let q = |f: &str, i: &str| if f.eq_ignore_ascii_case(&ci.field) { labels.iter().position(|l| l.eq_ignore_ascii_case(i)) } else { None };
        let Ok(prep) = crate::pivot_formula::prepare(&ci.formula, &labels, Some(&q)) else { continue };
        let x = fi.intern(wb, &Value::text(ci.name.clone()), &Grouping::default(), "General", usize::MAX);
        let side = |key: &(Vec<u32>, Vec<u32>)| if on_rows { key.0.len() } else { key.1.len() };
        let with_item = |key: &(Vec<u32>, Vec<u32>), item: u32| {
            let mut k2 = key.clone();
            if let Some(slot) = if on_rows { k2.0.get_mut(d) } else { k2.1.get_mut(d) } {
                *slot = item;
            }
            k2
        };
        let made: HashSet<(Vec<u32>, Vec<u32>)> = aggs.keys().filter(|key| side(key) > d).map(|key| with_item(key, x)).collect();
        if made.len() > 1_000_000 {
            return Err("The PivotTable report is too large.".into());
        }
        let lookup = |over: &HashMap<(Vec<u32>, Vec<u32>), Vec<Option<Value>>>, key: &(Vec<u32>, Vec<u32>), vi: usize| -> Value {
            if let Some(v) = over.get(key).and_then(|o| o.get(vi)).cloned().flatten() {
                return v;
            }
            aggs.get(key).and_then(|a| cv.get(a, vi, None)).unwrap_or(Value::Number(0.0))
        };
        let mut results: Vec<((Vec<u32>, Vec<u32>), Vec<Option<Value>>)> = Vec::with_capacity(made.len());
        for key in &made {
            let cells: Vec<Option<Value>> = (0..k)
                .map(|vi| {
                    let args: Vec<Value> = prep.refs.iter().map(|r| lookup(&over, &with_item(key, *r as u32), vi)).collect();
                    Some(crate::pivot_formula::eval(wb, pivot_sheet, pt.anchor, &prep, &args))
                })
                .collect();
            results.push((key.clone(), cells));
        }
        // What each total above the field gains.
        let additive: Vec<bool> =
            vals.iter().map(|(c, v)| *c != CALC_COL && matches!(v.func, PivotFunc::Sum | PivotFunc::Count | PivotFunc::CountNumbers)).collect();
        let mut gains: HashMap<(Vec<u32>, Vec<u32>), Vec<f64>> = HashMap::new();
        for (key, cells) in &results {
            if side(key) != d + 1 {
                continue;
            }
            for j in 0..=d {
                let anc = if on_rows {
                    (key.0.get(..j).unwrap_or_default().to_vec(), key.1.clone())
                } else {
                    (key.0.clone(), key.1.get(..j).unwrap_or_default().to_vec())
                };
                let g = gains.entry(anc).or_insert_with(|| vec![0.0; k]);
                for (vi, c) in cells.iter().enumerate() {
                    if additive.get(vi).copied().unwrap_or(false)
                        && let (Some(slot), Some(n)) = (g.get_mut(vi), c.as_ref().and_then(Value::as_f64))
                    {
                        *slot += n;
                    }
                }
            }
        }
        for (anc, g) in gains {
            let cells: Vec<Option<Value>> = (0..k)
                .map(|vi| match lookup(&over, &anc, vi) {
                    Value::Number(n) if additive.get(vi).copied().unwrap_or(false) => Some(Value::Number(n + g.get(vi).copied().unwrap_or(0.0))),
                    _ => None,
                })
                .collect();
            let e = over.entry(anc).or_insert_with(|| vec![None; k]);
            for (slot, c) in e.iter_mut().zip(cells) {
                if c.is_some() {
                    *slot = c;
                }
            }
        }
        for (key, cells) in results {
            if on_rows && key.0.len() == rows.len() {
                row_set.insert(key.0.clone());
            }
            if !on_rows && key.1.len() == cols.len() {
                col_set.insert(key.1.clone());
            }
            aggs.entry(key.clone()).or_insert_with(|| vec![Acc::default(); k_all]);
            over.insert(key, cells);
        }
    }

    // ---- axes
    let row_fields: Vec<&PivotField> = rows.iter().map(|(_, f)| *f).collect();
    let col_fields: Vec<&PivotField> = cols.iter().map(|(_, f)| *f).collect();
    let row_paths: Vec<&[u32]> = row_set.iter().map(Vec::as_slice).collect();
    let col_paths: Vec<&[u32]> = col_set.iter().map(Vec::as_slice).collect();
    let mut row_tree = build_tree(&row_paths, 0, &row_fields, &row_items);
    let mut col_tree = build_tree(&col_paths, 0, &col_fields, &col_items);
    add_empty_items(&mut row_tree, 0, &row_fields, &row_items);
    add_empty_items(&mut col_tree, 0, &col_fields, &col_items);
    let mut siblings: HashMap<Vec<u32>, Vec<u32>> = HashMap::new();
    collect_siblings(&row_tree, &mut Vec::new(), &mut siblings);

    let ctx = Ctx { pt, rows: row_fields.clone(), row_items, row_fmts: row_fmts.clone() };
    let mut lines: Vec<Line> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::new(LineKind::Item(false), vec![], true, vec![]));
    } else {
        if pt.layout == PivotLayout::Tabular {
            ctx.flatten_tabular(&row_tree, 0, &mut Vec::new(), &[], &mut lines);
        } else {
            ctx.flatten(&row_tree, 0, &mut Vec::new(), &[], &mut lines);
        }
        if pt.grand_totals_cols {
            lines.push(Line::new(LineKind::Grand, vec![], true, vec![(0, Value::text("Grand Total"), 0, None)]));
        }
    }
    let mut specs: Vec<ColSpec> = Vec::new();
    if cols.is_empty() {
        specs.push(ColSpec { kind: ColKind::Leaf, path: vec![], func: None });
    } else {
        flatten_cols(&col_tree, 0, pt, &col_fields, &col_items, &mut Vec::new(), &mut specs);
        if pt.grand_totals_rows {
            specs.push(ColSpec { kind: ColKind::Grand, path: vec![], func: None });
        }
    }
    let mut data_cols: Vec<(usize, Option<usize>)> = Vec::new();
    for (si, _) in specs.iter().enumerate() {
        if k == 0 {
            if !cols.is_empty() {
                data_cols.push((si, None));
            }
        } else {
            for vi in 0..k {
                data_cols.push((si, Some(vi)));
            }
        }
        if data_cols.len() > MAX_COLS as usize {
            return Err("The PivotTable report will not fit on the sheet.".into());
        }
    }

    let compact = pt.layout == PivotLayout::Compact;
    let l = if rows.is_empty() && cols.is_empty() {
        0
    } else if compact {
        1
    } else {
        rows.len().max(1)
    };
    let mut width = l + data_cols.len();
    // Report filter area: label and value pairs, arranged down then over (columns of N) or
    // over then down (rows of N), groups separated by a blank column.
    let per = pt.options.filters_per_column as usize;
    let nf = shown_filters.len();
    let slot = |i: usize| -> (usize, usize) {
        match (per, pt.options.filters_over_then_down) {
            (0, false) => (i, 0),
            (0, true) => (0, i),
            (n, false) => (i % n, i / n),
            (n, true) => (i / n, i % n),
        }
    };
    let filter_rows = (0..nf).map(|i| slot(i).0 + 1).max().unwrap_or(0);
    let filter_groups = (0..nf).map(|i| slot(i).1 + 1).max().unwrap_or(0);
    if nf > 0 {
        width = width.max(filter_groups * 3 - 1);
    }
    if empty {
        width = width.max(1);
    }
    let width = width.max(1);
    let mut g = Grid { rows: vec![], kinds: vec![], width, buttons: vec![] };

    // ---- report filters
    for _ in 0..filter_rows {
        g.push_row(RowKind::Filter)?;
    }
    for (i, (c, f, _, _)) in shown_filters.iter().enumerate() {
        let (r, grp) = slot(i);
        let name = src.headers.get(*c).cloned().unwrap_or_default();
        let shown = match &f.selected {
            None => "(All)".to_string(),
            Some(v) if v.len() == 1 => v.first().cloned().unwrap_or_default(),
            Some(_) => "(Multiple Items)".to_string(),
        };
        g.set(r, grp * 3, Value::text(name), CellKind::Filter, 0, None);
        g.set(r, grp * 3 + 1, Value::text(shown), CellKind::Filter, 0, None);
        g.buttons.push((r as u32, (grp * 3 + 1) as u32, f.source_col.clone()));
    }
    if !shown_filters.is_empty() {
        g.push_row(RowKind::Blank)?;
    }
    let body_offset = g.rows.len() as u32;

    if empty {
        let r = g.push_row(RowKind::Placeholder)?;
        g.set(r, 0, Value::text(format!("{} — choose fields", pt.name)), CellKind::Placeholder, 0, None);
        return Ok(Output { width: g.width as u32, grid: g.rows, row_kinds: g.kinds, body_offset, buttons: g.buttons });
    }

    let val_name = |vi: usize| vals.get(vi).map(|(_, v)| v.name.clone()).unwrap_or_default();
    let headers_on = pt.show_headers;
    let row_header = |g: &mut Grid, r: usize| {
        if rows.is_empty() || !headers_on {
            return;
        }
        if compact {
            let caption = pt.options.row_header_caption.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| "Row Labels".into());
            g.set(r, 0, Value::text(caption), CellKind::Header, 0, None);
            if let Some(f) = row_fields.first() {
                g.buttons.push((r as u32, 0, f.source_col.clone()));
            }
        } else {
            for (i, f) in row_fields.iter().enumerate() {
                g.set(r, i, Value::text(f.shown_name()), CellKind::Header, 0, None);
                g.buttons.push((r as u32, i as u32, f.source_col.clone()));
            }
        }
    };

    // ---- header rows
    if cols.is_empty() {
        let r = g.push_row(RowKind::Header)?;
        row_header(&mut g, r);
        for (j, (_, vi)) in data_cols.iter().enumerate() {
            if let Some(vi) = vi {
                g.set(r, l + j, Value::text(val_name(*vi)), CellKind::Header, 0, None);
            }
        }
    } else {
        let cap = g.push_row(RowKind::Header)?;
        if l > 0 && k == 1 {
            g.set(cap, 0, Value::text(val_name(0)), CellKind::Header, 0, None);
        }
        if headers_on {
            let caption = if compact {
                pt.options.column_header_caption.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| "Column Labels".into())
            } else {
                col_fields.first().map(|f| f.shown_name().to_string()).unwrap_or_default()
            };
            g.set(cap, l, Value::text(caption), CellKind::Header, 0, None);
            if let Some(f) = col_fields.first() {
                g.buttons.push((cap as u32, l as u32, f.source_col.clone()));
            }
        }
        let levels = cols.len() + usize::from(k > 1);
        let first_level = g.rows.len();
        for _ in 0..levels {
            g.push_row(RowKind::Header)?;
        }
        let mut prev: Option<&ColSpec> = None;
        for (j, (si, vi)) in data_cols.iter().enumerate() {
            let Some(spec) = specs.get(*si) else { continue };
            let c = l + j;
            match spec.kind {
                ColKind::Leaf => {
                    for (lv, &it) in spec.path.iter().enumerate() {
                        let same = prev.is_some_and(|p| p.kind != ColKind::Grand && p.path.get(..=lv) == spec.path.get(..=lv));
                        if !same {
                            let Some(item) = col_items.get(lv).and_then(|fi| fi.get(it)) else { continue };
                            let fmt = if item.formatted { cols.get(lv).map(|(cc, _)| fmt_of(*cc)) } else { None };
                            g.set(first_level + lv, c, item.value.clone(), CellKind::ColLabel, 0, fmt);
                        }
                    }
                    if k > 1
                        && let Some(vi) = vi
                    {
                        g.set(first_level + cols.len(), c, Value::text(val_name(*vi)), CellKind::ColLabel, 0, None);
                    }
                }
                ColKind::Subtotal => {
                    let lv = spec.path.len().saturating_sub(1);
                    let label = spec.path.last().and_then(|it| col_items.get(lv).map(|fi| fi.label(*it))).unwrap_or_default();
                    let text = match (vi, spec.func) {
                        (_, Some(f)) => subtotal_label(&label, Some(f)),
                        (Some(vi), None) if k > 1 => format!("{label} {}", val_name(*vi)),
                        _ => format!("{label} Total"),
                    };
                    g.set(first_level + lv, c, Value::text(text), CellKind::ColLabel, 0, None);
                }
                ColKind::Grand => {
                    let text = match vi {
                        Some(vi) if k > 1 => format!("Total {}", val_name(*vi)),
                        _ => "Grand Total".to_string(),
                    };
                    g.set(first_level, c, Value::text(text), CellKind::ColLabel, 0, None);
                }
            }
            prev = Some(spec);
        }
        let last = g.rows.len() - 1;
        row_header(&mut g, last);
    }

    // ---- body
    let get_with = |rp: &[u32], cp: &[u32], vi: usize, func: Option<PivotFunc>| -> Option<Value> {
        let key = (rp.to_vec(), cp.to_vec());
        if let Some(v) = over.get(&key).and_then(|o| o.get(vi)).cloned().flatten() {
            return Some(v);
        }
        aggs.get(&key).and_then(|a| cv.get(a, vi, func))
    };
    let get = |rp: &[u32], cp: &[u32], vi: usize| get_with(rp, cp, vi, None);
    let num = |v: &Option<Value>| v.as_ref().and_then(Value::as_f64);
    let ratio = |a: Value, b: Option<Value>| -> Value {
        match (a, b) {
            (Value::Error(e), _) => Value::Error(e),
            (_, Some(Value::Error(e))) => Value::Error(e),
            (Value::Number(x), Some(Value::Number(y))) => {
                if y == 0.0 {
                    Value::Error(CellError::Div0)
                } else {
                    Value::number(x / y)
                }
            }
            (v, _) => v,
        }
    };
    let data_fmt: Vec<Option<String>> = vals
        .iter()
        .map(|(c, v)| match v.show_as {
            sa if sa.is_percent() => Some(v.number_format.clone().unwrap_or_else(|| "0.00%".into())),
            PivotShowAs::Rank | PivotShowAs::RankAscending | PivotShowAs::Index => v.number_format.clone(),
            _ => v.number_format.clone().or_else(|| match v.func {
                PivotFunc::Count | PivotFunc::CountNumbers => None,
                _ => {
                    let f = fmt_of(*c);
                    (f != "General").then_some(f)
                }
            }),
        })
        .collect();
    let mut col_siblings: HashMap<Vec<u32>, Vec<u32>> = HashMap::new();
    collect_siblings(&col_tree, &mut Vec::new(), &mut col_siblings);
    // Per value field: the base field (axis, depth) and base item.
    let bases: Vec<Option<(bool, usize, BaseItem)>> = vals
        .iter()
        .map(|(_, v)| {
            if !v.show_as.needs_base_field() {
                return None;
            }
            let (is_row, depth) = match v.base_field.as_deref() {
                Some(b) => match row_fields.iter().position(|f| f.source_col.eq_ignore_ascii_case(b)) {
                    Some(d) => (true, d),
                    None => (false, col_fields.iter().position(|f| f.source_col.eq_ignore_ascii_case(b))?),
                },
                None if !row_fields.is_empty() => (true, 0),
                None if !col_fields.is_empty() => (false, 0),
                None => return None,
            };
            let items = if is_row { ctx.row_items.get(depth) } else { col_items.get(depth) };
            let item = match v.base_item.as_deref() {
                Some(BASE_PREVIOUS) => BaseItem::Previous,
                Some(BASE_NEXT) => BaseItem::Next,
                Some(label) => items
                    .and_then(|fi| fi.items.iter().position(|it| it.label.eq_ignore_ascii_case(label)))
                    .map_or(BaseItem::Missing, |i| BaseItem::At(i as u32)),
                None => BaseItem::Missing,
            };
            Some((is_row, depth, item))
        })
        .collect();
    let show = |raw: Value, rp: &[u32], cp: &[u32], vi: usize| -> Value {
        let Some((_, vdef)) = vals.get(vi) else { return raw };
        match vdef.show_as {
            PivotShowAs::Normal => raw,
            PivotShowAs::PercentOfGrandTotal => ratio(raw, get(&[], &[], vi)),
            PivotShowAs::PercentOfColumnTotal => ratio(raw, get(&[], cp, vi)),
            PivotShowAs::PercentOfRowTotal => ratio(raw, get(rp, &[], vi)),
            PivotShowAs::PercentOfParentRowTotal => match rp.split_last() {
                Some((_, parent)) => ratio(raw, get(parent, cp, vi)),
                None => ratio(raw.clone(), Some(raw)),
            },
            PivotShowAs::PercentOfParentColumnTotal => match cp.split_last() {
                Some((_, parent)) => ratio(raw, get(rp, parent, vi)),
                None => ratio(raw.clone(), Some(raw)),
            },
            PivotShowAs::Index => match (raw.as_f64(), num(&get(&[], &[], vi)), num(&get(rp, &[], vi)), num(&get(&[], cp, vi))) {
                (Some(v), Some(g), Some(r), Some(c)) => {
                    if r == 0.0 || c == 0.0 {
                        Value::Error(CellError::Div0)
                    } else {
                        Value::number(v * g / (r * c))
                    }
                }
                _ => raw,
            },
            _ => {
                let Some(Some((is_row, d, item))) = bases.get(vi) else { return raw };
                let (is_row, d) = (*is_row, *d);
                let path = if is_row { rp } else { cp };
                // Rows or columns above the base field's level (subtotals, grand totals).
                let (Some(&mine), Some(parent)) = (path.get(d), path.get(..d)) else {
                    return match vdef.show_as {
                        PivotShowAs::RunningTotal => raw,
                        _ => Value::Empty,
                    };
                };
                let sibs: Vec<u32> = (if is_row { &siblings } else { &col_siblings }).get(parent).cloned().unwrap_or_default();
                let at = |it: u32| -> Option<Value> {
                    let mut p = path.to_vec();
                    if let Some(x) = p.get_mut(d) {
                        *x = it;
                    }
                    if is_row { get(&p, cp, vi) } else { get(rp, &p, vi) }
                };
                let pos = sibs.iter().position(|x| *x == mine);
                let base = match item {
                    BaseItem::At(i) => Some(*i),
                    BaseItem::Previous => pos.and_then(|p| p.checked_sub(1)).and_then(|p| sibs.get(p)).copied(),
                    BaseItem::Next => pos.and_then(|p| sibs.get(p + 1)).copied(),
                    BaseItem::Missing => None,
                };
                let running = || -> f64 {
                    let upto = pos.map_or(0, |p| p + 1);
                    sibs.iter().take(upto).map(|s| num(&at(*s)).unwrap_or(0.0)).sum()
                };
                match vdef.show_as {
                    PivotShowAs::PercentOf => match base {
                        Some(b) if b == mine => ratio(raw.clone(), Some(raw)),
                        Some(b) => match at(b) {
                            Some(bv) => ratio(raw, Some(bv)),
                            None => Value::Error(CellError::NA),
                        },
                        None => Value::Error(CellError::NA),
                    },
                    PivotShowAs::DifferenceFrom | PivotShowAs::PercentDifferenceFrom => match base {
                        Some(b) if b == mine => Value::Empty,
                        Some(b) => match (raw.as_f64(), at(b)) {
                            (Some(v), Some(bv)) => match (bv.as_f64(), vdef.show_as) {
                                (Some(x), PivotShowAs::DifferenceFrom) => Value::number(v - x),
                                (Some(0.0), _) => Value::Error(CellError::Div0),
                                (Some(x), _) => Value::number((v - x) / x),
                                (None, _) => bv,
                            },
                            (_, None) => Value::Error(CellError::NA),
                            (None, _) => raw,
                        },
                        None => Value::Empty,
                    },
                    PivotShowAs::RunningTotal => match raw {
                        Value::Number(_) => Value::number(running()),
                        other => other,
                    },
                    PivotShowAs::PercentRunningTotal => match raw {
                        Value::Number(_) => {
                            let total: f64 = sibs.iter().map(|s| num(&at(*s)).unwrap_or(0.0)).sum();
                            if total == 0.0 { Value::Error(CellError::Div0) } else { Value::number(running() / total) }
                        }
                        other => other,
                    },
                    PivotShowAs::Rank | PivotShowAs::RankAscending => match raw.as_f64() {
                        Some(me) => {
                            let desc = vdef.show_as == PivotShowAs::Rank;
                            let ahead = sibs.iter().filter(|s| num(&at(**s)).is_some_and(|x| if desc { x > me } else { x < me })).count();
                            Value::number((ahead + 1) as f64)
                        }
                        None => raw,
                    },
                    PivotShowAs::PercentOfParentTotal => {
                        let top = path.get(..=d).unwrap_or(path);
                        let denom = if is_row { get(top, cp, vi) } else { get(rp, top, vi) };
                        ratio(raw, denom)
                    }
                    _ => raw,
                }
            }
        }
    };
    for line in &lines {
        let rk = match line.kind {
            LineKind::Item(b) => RowKind::Item(b),
            LineKind::Subtotal => RowKind::Subtotal,
            LineKind::Grand => RowKind::Grand,
            LineKind::Blank => RowKind::Blank,
        };
        let ck = match line.kind {
            LineKind::Item(_) | LineKind::Blank => (CellKind::RowLabel, CellKind::Data),
            LineKind::Subtotal => (CellKind::Subtotal, CellKind::Subtotal),
            LineKind::Grand => (CellKind::GrandTotal, CellKind::GrandTotal),
        };
        let r = g.push_row(rk)?;
        for (c, v, indent, fmt) in &line.labels {
            g.set(r, *c, v.clone(), ck.0, *indent, fmt.clone());
        }
        if !line.values {
            continue;
        }
        let rp = line.path.as_slice();
        for (j, (si, vi)) in data_cols.iter().enumerate() {
            let (Some(spec), Some(vi)) = (specs.get(*si), vi) else { continue };
            let cp = spec.path.as_slice();
            let Some(raw) = get_with(rp, cp, *vi, line.func.or(spec.func)) else {
                // PivotTable Options › For empty cells show.
                if let Some(t) = pt.options.empty_text.as_ref().filter(|t| !t.is_empty()) {
                    g.set(r, l + j, Value::text(t.as_str()), ck.1, 0, None);
                }
                continue;
            };
            let shown = if line.func.or(spec.func).is_some() { raw } else { show(raw, rp, cp, *vi) };
            // PivotTable Options › For error values show.
            let shown = match (&shown, pt.options.error_text.as_ref()) {
                (Value::Error(_), Some(t)) => Value::text(t.as_str()),
                _ => shown,
            };
            g.set(r, l + j, shown, ck.1, 0, data_fmt.get(*vi).cloned().flatten());
        }
    }
    Ok(Output { width: g.width as u32, grid: g.rows, row_kinds: g.kinds, body_offset, buttons: g.buttons })
}

/// Whether `text` matches `pattern` with `*` and `?` wildcards (case-insensitive).
fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s0) = star {
            pi = s0 + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Ordering of a label against a filter value: as numbers when both are numbers.
fn label_cmp(a: &str, b: &str) -> Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.total_cmp(&y),
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

fn compare_text(op: PivotCompare, label: &str, a: &str, b: &str) -> bool {
    let (l, x) = (label.to_lowercase(), a.to_lowercase());
    match op {
        PivotCompare::Equals => wildcard(a, label),
        PivotCompare::NotEquals => !wildcard(a, label),
        PivotCompare::BeginsWith => l.starts_with(&x),
        PivotCompare::NotBeginsWith => !l.starts_with(&x),
        PivotCompare::EndsWith => l.ends_with(&x),
        PivotCompare::NotEndsWith => !l.ends_with(&x),
        PivotCompare::Contains => wildcard(&format!("*{a}*"), label),
        PivotCompare::NotContains => !wildcard(&format!("*{a}*"), label),
        PivotCompare::Greater => label_cmp(label, a) == Ordering::Greater,
        PivotCompare::GreaterOrEqual => label_cmp(label, a) != Ordering::Less,
        PivotCompare::Less => label_cmp(label, a) == Ordering::Less,
        PivotCompare::LessOrEqual => label_cmp(label, a) != Ordering::Greater,
        PivotCompare::Between => label_cmp(label, a) != Ordering::Less && label_cmp(label, b) != Ordering::Greater,
        PivotCompare::NotBetween => !(label_cmp(label, a) != Ordering::Less && label_cmp(label, b) != Ordering::Greater),
    }
}

fn compare_num(op: PivotCompare, v: f64, a: f64, b: f64) -> bool {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    match op {
        PivotCompare::Equals => v == a,
        PivotCompare::NotEquals => v != a,
        PivotCompare::Greater => v > a,
        PivotCompare::GreaterOrEqual => v >= a,
        PivotCompare::Less => v < a,
        PivotCompare::LessOrEqual => v <= a,
        PivotCompare::Between => v >= lo && v <= hi,
        PivotCompare::NotBetween => v < lo || v > hi,
        _ => true,
    }
}

/// Year, month (1–12) and quarter (1–4) of a serial.
fn ymq(serial: f64) -> Option<(i32, u32, u32)> {
    let d = datetime_from_serial(gridcraft_core::DateSystem::D1900, serial)?;
    Some((d.year, d.month, (d.month - 1) / 3 + 1))
}

/// Whether a date (serial `x`) meets a date filter, with `today` as today's serial.
fn date_rule_ok(rule: PivotDateRule, x: f64, a: Option<f64>, b: Option<f64>, today: f64) -> bool {
    let day = x.floor();
    let t = today.floor();
    let weekday = |s: f64| datetime_from_serial(gridcraft_core::DateSystem::D1900, s).map(|d| d.weekday as f64).unwrap_or(0.0);
    let week = |offset: f64| {
        let start = t - weekday(t) + 7.0 * offset;
        day >= start && day < start + 7.0
    };
    let (Some((y, m, q)), Some((ty, tm, tq))) = (ymq(day), ymq(t)) else { return false };
    let month = |offset: i32| {
        let total = ty * 12 + tm as i32 - 1 + offset;
        y == total.div_euclid(12) && m as i32 == total.rem_euclid(12) + 1
    };
    let quarter = |offset: i32| {
        let total = ty * 4 + tq as i32 - 1 + offset;
        y == total.div_euclid(4) && q as i32 == total.rem_euclid(4) + 1
    };
    match rule {
        PivotDateRule::Equals => a.is_some_and(|a| day == a.floor()),
        PivotDateRule::NotEquals => a.is_none_or(|a| day != a.floor()),
        PivotDateRule::Before => a.is_some_and(|a| day < a.floor()),
        PivotDateRule::After => a.is_some_and(|a| day > a.floor()),
        PivotDateRule::Between => match (a, b) {
            (Some(a), Some(b)) => day >= a.floor().min(b.floor()) && day <= a.floor().max(b.floor()),
            _ => false,
        },
        PivotDateRule::Tomorrow => day == t + 1.0,
        PivotDateRule::Today => day == t,
        PivotDateRule::Yesterday => day == t - 1.0,
        PivotDateRule::NextWeek => week(1.0),
        PivotDateRule::ThisWeek => week(0.0),
        PivotDateRule::LastWeek => week(-1.0),
        PivotDateRule::NextMonth => month(1),
        PivotDateRule::ThisMonth => month(0),
        PivotDateRule::LastMonth => month(-1),
        PivotDateRule::NextQuarter => quarter(1),
        PivotDateRule::ThisQuarter => quarter(0),
        PivotDateRule::LastQuarter => quarter(-1),
        PivotDateRule::NextYear => y == ty + 1,
        PivotDateRule::ThisYear => y == ty,
        PivotDateRule::LastYear => y == ty - 1,
        PivotDateRule::YearToDate => y == ty && day <= t,
        PivotDateRule::Quarter(n) => q == n as u32,
        PivotDateRule::Month(n) => m == n as u32,
    }
}

/// Whether a record passes a label or date filter on its item.
fn label_rule_ok(rule: &PivotRule, label: &str, raw: &Value, today: f64) -> bool {
    match rule {
        PivotRule::Label { op, value, value2 } => compare_text(*op, label, value, value2),
        PivotRule::Date { rule, value, value2 } => raw.as_f64().is_some_and(|x| date_rule_ok(*rule, x, *value, *value2, today)),
        _ => true,
    }
}

/// The items a value or Top 10 filter drops among siblings `(item, total)`.
fn rule_drops(rule: &PivotRule, scored: &[(u32, Option<f64>)]) -> Vec<u32> {
    match rule {
        PivotRule::Value { op, value, value2, .. } => {
            scored.iter().filter(|(_, v)| !v.is_some_and(|x| compare_num(*op, x, *value, *value2))).map(|(i, _)| *i).collect()
        }
        PivotRule::Top { top, count, by, .. } => {
            let mut order: Vec<(u32, f64)> = scored.iter().map(|(i, v)| (*i, v.unwrap_or(f64::NEG_INFINITY))).collect();
            order.sort_by(|a, b| if *top { b.1.total_cmp(&a.1) } else { a.1.total_cmp(&b.1) });
            let keep: usize = match by {
                PivotTopBy::Items => (count.max(0.0).floor() as usize).min(order.len()),
                PivotTopBy::Percent | PivotTopBy::Sum => {
                    let total: f64 = order.iter().map(|x| x.1.max(0.0)).sum();
                    let goal = if *by == PivotTopBy::Percent { total * count / 100.0 } else { *count };
                    let mut acc = 0.0;
                    let mut n = 0;
                    for (_, v) in &order {
                        if acc >= goal {
                            break;
                        }
                        acc += v.max(0.0);
                        n += 1;
                    }
                    n
                }
            };
            order.iter().skip(keep).map(|(i, _)| *i).collect()
        }
        _ => vec![],
    }
}

/// A Show Values As base item, resolved against the base field's items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BaseItem {
    At(u32),
    Previous,
    Next,
    /// None given, or not among the field's items.
    Missing,
}

// ---------------------------------------------------------------- styling and writing

/// Colours for a PivotTable style name (our own palette, keyed by the familiar
/// `PivotStyleLight1…28 / Medium1…28 / Dark1…28` names).
struct Look {
    header_fill: Color,
    header_font: Color,
    line: Color,
    filter_fill: Color,
}

fn look(style: &str) -> Look {
    let rest = style.strip_prefix("PivotStyle").unwrap_or(style);
    let (fam, n) = ["Light", "Medium", "Dark"]
        .iter()
        .find_map(|f| rest.strip_prefix(f).map(|num| (*f, num.parse::<u32>().unwrap_or(16))))
        .unwrap_or(("Light", 16));
    let k = n.saturating_sub(1) % 7;
    let acc = if k == 0 { None } else { Some(3 + k as u8) };
    let tone = |tint: i16| match acc {
        Some(a) => Color::Theme(a, tint),
        None => Color::Theme(1, tint.clamp(-1000, 1000)),
    };
    match fam {
        "Dark" => Look { header_fill: Color::Theme(1, 0), header_font: Color::Theme(0, 0), line: tone(0), filter_fill: tone(600) },
        "Medium" => Look { header_fill: tone(0), header_font: Color::Theme(0, 0), line: tone(0), filter_fill: tone(800) },
        _ => Look { header_fill: tone(800), header_font: Color::Auto, line: tone(400), filter_fill: tone(800) },
    }
}

/// Row/column banding for a cell (Design › Banded Rows / Banded Columns).
#[derive(Clone, Copy, Default)]
struct Band {
    row: bool,
    col: bool,
}

fn cell_style(lk: &Look, opts: &PivotOptions, rk: RowKind, cell: &OutCell, last_header: bool, band: Band) -> Style {
    let mut s = Style::default();
    match rk {
        RowKind::Header => {
            // Column Headers off: the header rows lose their emphasis.
            if opts.column_headers {
                s.font.bold = true;
                s.fill = Fill::solid(lk.header_fill);
                s.font.color = lk.header_font;
            }
            if last_header {
                s.borders.bottom = BorderLine { style: BorderStyle::Thin, color: lk.line };
            }
        }
        RowKind::Filter => {
            s.fill = Fill::solid(lk.filter_fill);
            s.font.bold = last_header;
        }
        RowKind::Item(parent) => {
            s.font.bold = parent && opts.row_headers;
            if (band.row && opts.banded_rows) || (band.col && opts.banded_columns && cell.kind == CellKind::Data) {
                s.fill = Fill::solid(lk.filter_fill);
            }
        }
        RowKind::Subtotal => s.font.bold = true,
        RowKind::Grand => {
            s.font.bold = true;
            s.borders.top = BorderLine { style: BorderStyle::Thin, color: lk.line };
        }
        RowKind::Placeholder => {
            s.font.italic = true;
            s.font.color = Color::Theme(1, 500);
        }
        RowKind::Blank => {}
    }
    if matches!(cell.kind, CellKind::RowLabel | CellKind::ColLabel | CellKind::Subtotal | CellKind::GrandTotal) && !cell.value.is_number() {
        s.align.h = HAlign::Left;
    }
    if matches!(cell.kind, CellKind::RowLabel | CellKind::ColLabel) && cell.value.is_number() {
        s.align.h = HAlign::Left;
    }
    s.align.indent = cell.indent.min(15);
    // Merged outer labels are centred.
    if cell.kind == CellKind::RowLabel && opts.merge_labels && cell.indent == 0 && !cell.value.is_empty() {
        s.align.v = VAlign::Center;
    }
    if let Some(f) = &cell.num_fmt {
        s.num_fmt = NumFmt::new(f);
    }
    s
}

/// Recomputes a pivot and writes its report. With `replace` false, refuses when the target area
/// holds other data. Returns the written area.
pub fn refresh(wb: &mut Workbook, sheet: usize, idx: usize, replace: bool) -> Result<RangeRef, String> {
    let pt = wb.sheet(sheet).and_then(|s| s.pivots.get(idx)).cloned().ok_or("There's no such PivotTable.")?;
    let out = compute(wb, &pt, sheet)?;
    let h = out.height().max(1);
    let w = out.width.max(1);
    let k = out.body_offset;
    let origin_row = if pt.anchor.row >= k { pt.anchor.row - k } else { pt.anchor.row };
    let origin = CellRef::new(origin_row, pt.anchor.col);
    let end_row = origin.row as u64 + h as u64 - 1;
    let end_col = origin.col as u64 + w as u64 - 1;
    if end_row >= MAX_ROWS as u64 || end_col >= MAX_COLS as u64 {
        return Err("The PivotTable report will not fit on the sheet. Choose a different location.".into());
    }
    let area = RangeRef::new(origin, CellRef::new(end_row as u32, end_col as u32));
    let sh = wb.sheet(sheet).ok_or("There's no such sheet.")?;
    for (j, other) in sh.pivots.iter().enumerate() {
        if j != idx && other.last_range.is_some_and(|r| r.intersects(&area)) {
            return Err("A PivotTable report cannot overlap another PivotTable report.".into());
        }
    }
    if sh.is_protected() {
        return Err("The cell or chart you're trying to change is on a protected sheet.".into());
    }
    if let Ok((ssi, sr)) = resolve_source(wb, &pt.source, sheet)
        && ssi == sheet
        && sr.intersects(&area)
    {
        return Err("The PivotTable report can't overlap its source data. Choose a different location.".into());
    }
    if sh.tables.iter().any(|t| t.range.intersects(&area)) {
        return Err("A PivotTable report cannot overlap a table.".into());
    }
    if !replace {
        for (c, cell) in sh.cells.iter_range(area) {
            if (!cell.value.is_empty() || cell.formula.is_some()) && !pt.last_range.is_some_and(|r| r.contains(c)) {
                return Err(format!(
                    "There's already data in {}!{}. Do you want to replace it? (Run again with replace: true to overwrite.)",
                    quote_sheet(&sh.name),
                    c.a1()
                ));
            }
        }
        if let Some(c) = sh.spill.keys().find(|c| area.contains(**c) && !pt.last_range.is_some_and(|r| r.contains(**c))) {
            return Err(format!(
                "There's already data in {}!{}. Do you want to replace it? (Run again with replace: true to overwrite.)",
                quote_sheet(&sh.name),
                c.a1()
            ));
        }
    }
    // Styles first (the style table is workbook-wide).
    let lk = look(&pt.style);
    let opts = pt.options.clone();
    let header_last = out.row_kinds.iter().rposition(|k| *k == RowKind::Header);
    // Banding counts item rows, and the data columns after the label columns.
    let label_cols = match pt.layout {
        PivotLayout::Compact => 1,
        _ => pt.rows.len().max(1),
    };
    let mut item_index = 0usize;
    let mut row_band: Vec<bool> = Vec::with_capacity(out.row_kinds.len());
    for k in &out.row_kinds {
        row_band.push(matches!(k, RowKind::Item(_)) && item_index.is_multiple_of(2));
        if matches!(k, RowKind::Item(_)) {
            item_index += 1;
        }
    }
    let mut cache: HashMap<Style, StyleId> = HashMap::new();
    let mut cells: Vec<(CellRef, Cell)> = Vec::new();
    for (ri, row) in out.grid.iter().enumerate() {
        let rk = out.row_kinds.get(ri).copied().unwrap_or(RowKind::Blank);
        if rk == RowKind::Blank {
            continue;
        }
        for (ci, oc) in row.iter().enumerate() {
            if rk == RowKind::Filter && oc.value.is_empty() {
                continue;
            }
            if rk == RowKind::Placeholder && oc.value.is_empty() {
                continue;
            }
            let flag = match rk {
                RowKind::Header => Some(ri) == header_last,
                RowKind::Filter => ci % 3 == 0,
                _ => false,
            };
            let band = Band { row: row_band.get(ri).copied().unwrap_or(false), col: ci >= label_cols && (ci - label_cols).is_multiple_of(2) };
            let st = cell_style(&lk, &opts, rk, oc, flag, band);
            let id = match cache.get(&st) {
                Some(id) => *id,
                None => {
                    let id = wb.styles.intern(st.clone());
                    cache.insert(st, id);
                    id
                }
            };
            if oc.value.is_empty() && id == StyleId::default() {
                continue;
            }
            cells.push((
                CellRef::new(origin.row + ri as u32, origin.col + ci as u32),
                Cell { value: oc.value.clone(), formula: None, style: id, py: None },
            ));
        }
    }
    // Column widths that fit the labels.
    let mut widths: HashMap<u32, f32> = HashMap::new();
    for (c, cell) in &cells {
        if cell.value.is_empty() {
            continue;
        }
        let st = wb.styles.get(cell.style);
        let text = crate::display::format(&cell.value, st.num_fmt.as_str(), wb).text;
        let wpt = crate::cmd::format::approx_text_width(&text, st.font.size, st.font.bold) + st.align.indent as f32 * 9.0 + 14.0;
        let e = widths.entry(c.col).or_insert(0.0);
        *e = e.max(wpt.min(320.0));
    }
    // Merge and center cells with labels: an outer label spans the rows of its group.
    let mut merges: Vec<RangeRef> = Vec::new();
    if opts.merge_labels && pt.layout != PivotLayout::Compact {
        for col in 0..pt.rows.len().saturating_sub(1) {
            let mut start: Option<usize> = None;
            for (ri, row) in out.grid.iter().enumerate() {
                let rk = out.row_kinds.get(ri).copied().unwrap_or(RowKind::Blank);
                let has = row.get(col).is_some_and(|c| c.kind == CellKind::RowLabel && !c.value.is_empty());
                let continues = matches!(rk, RowKind::Item(_)) && !has;
                if let Some(s0) = start
                    && !continues
                {
                    if ri - 1 > s0 {
                        merges.push(RangeRef::new(
                            CellRef::new(origin.row + s0 as u32, origin.col + col as u32),
                            CellRef::new(origin.row + ri as u32 - 1, origin.col + col as u32),
                        ));
                    }
                    start = None;
                }
                if has && matches!(rk, RowKind::Item(_)) {
                    start = Some(ri);
                }
            }
            if let Some(s0) = start
                && out.grid.len() - 1 > s0
            {
                merges.push(RangeRef::new(
                    CellRef::new(origin.row + s0 as u32, origin.col + col as u32),
                    CellRef::new(origin.row + out.grid.len() as u32 - 1, origin.col + col as u32),
                ));
            }
        }
    }
    let shm = wb.sheet_mut(sheet).ok_or("There's no such sheet.")?;
    if let Some(old) = pt.last_range {
        shm.take_cells(old);
        shm.merges.retain(|m| !old.contains(m.start));
    }
    shm.take_cells(area);
    shm.merges.retain(|m| !area.contains(m.start));
    shm.merges.extend(merges);
    for (c, cell) in cells {
        shm.set_cell(c, cell);
    }
    // Autofit column widths on update.
    if opts.autofit {
        for (col, wpt) in widths {
            if wpt > shm.col_width(col) && !shm.is_col_hidden(col) {
                shm.cols.entry(col).or_default().size = Some(wpt);
            }
        }
    }
    if let Some(p) = shm.pivots.get_mut(idx) {
        p.last_range = Some(area);
        p.buttons = place(&out.buttons, origin);
    }
    Ok(area)
}

fn place(buttons: &[(u32, u32, String)], origin: CellRef) -> Vec<(CellRef, String)> {
    buttons.iter().map(|(r, c, f)| (CellRef::new(origin.row.saturating_add(*r), origin.col.saturating_add(*c)), f.clone())).collect()
}

/// Finds where each PivotTable's field buttons are, for PivotTables read from a file (their cells
/// are as saved; nothing is rewritten).
pub fn place_buttons(wb: &mut Workbook) {
    let mut found: Vec<(usize, usize, Vec<(CellRef, String)>)> = Vec::new();
    for (si, sh) in wb.sheets.iter().enumerate() {
        for (pi, pt) in sh.pivots.iter().enumerate() {
            let Some(r) = pt.last_range else { continue };
            if let Ok(out) = compute(wb, pt, si) {
                found.push((si, pi, place(&out.buttons, r.start)));
            }
        }
    }
    for (si, pi, b) in found {
        if let Some(p) = wb.sheet_mut(si).and_then(|s| s.pivots.get_mut(pi)) {
            p.buttons = b;
        }
    }
}

/// Clears a pivot's last output area.
pub fn clear_output(wb: &mut Workbook, sheet: usize, idx: usize) {
    let Some(shm) = wb.sheet_mut(sheet) else { return };
    let Some(r) = shm.pivots.get(idx).and_then(|p| p.last_range) else { return };
    shm.take_cells(r);
    shm.merges.retain(|m| !r.contains(m.start));
    if let Some(p) = shm.pivots.get_mut(idx) {
        p.last_range = None;
        p.buttons.clear();
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    fn serial(y: i64, m: i64, d: i64) -> f64 {
        gridcraft_core::date::serial_from_ymd(gridcraft_core::DateSystem::D1900, y, m, d).unwrap()
    }

    #[test]
    fn wildcards_and_comparisons() {
        assert!(wildcard("?ast", "East"));
        assert!(wildcard("e*", "EAST"));
        assert!(wildcard("*s*t", "East"));
        assert!(!wildcard("?ast", "Coast"));
        assert!(compare_text(PivotCompare::Contains, "Northwest", "WEST", ""));
        assert!(compare_text(PivotCompare::NotBeginsWith, "West", "E", ""));
        // Numbers compare as numbers, text as text.
        assert!(compare_text(PivotCompare::Greater, "10", "9", ""));
        assert!(compare_text(PivotCompare::Between, "Melon", "b", "n"));
        assert!(compare_text(PivotCompare::NotBetween, "Zebra", "b", "n"));
        assert!(compare_num(PivotCompare::Between, 5.0, 10.0, 1.0));
        assert!(!compare_num(PivotCompare::NotBetween, 5.0, 1.0, 10.0));
    }

    #[test]
    fn relative_dates() {
        // Thursday 15 October 2026.
        let today = serial(2026, 10, 15);
        let ok = |r: PivotDateRule, y: i64, m: i64, d: i64| date_rule_ok(r, serial(y, m, d), None, None, today);
        assert!(ok(PivotDateRule::Today, 2026, 10, 15));
        assert!(ok(PivotDateRule::Yesterday, 2026, 10, 14));
        assert!(ok(PivotDateRule::Tomorrow, 2026, 10, 16));
        // Weeks run Sunday to Saturday.
        assert!(ok(PivotDateRule::ThisWeek, 2026, 10, 11));
        assert!(ok(PivotDateRule::ThisWeek, 2026, 10, 17));
        assert!(!ok(PivotDateRule::ThisWeek, 2026, 10, 18));
        assert!(ok(PivotDateRule::NextWeek, 2026, 10, 18));
        assert!(ok(PivotDateRule::LastWeek, 2026, 10, 10));
        assert!(ok(PivotDateRule::LastMonth, 2026, 9, 30));
        assert!(ok(PivotDateRule::NextQuarter, 2027, 1, 1));
        assert!(ok(PivotDateRule::LastQuarter, 2026, 7, 1));
        assert!(ok(PivotDateRule::LastYear, 2025, 12, 31));
        assert!(ok(PivotDateRule::YearToDate, 2026, 1, 1));
        assert!(!ok(PivotDateRule::YearToDate, 2026, 10, 16));
        assert!(ok(PivotDateRule::Quarter(2), 2019, 5, 1));
        assert!(ok(PivotDateRule::Month(12), 2001, 12, 25));
        // December rolls into next January.
        let dec = serial(2026, 12, 3);
        assert!(date_rule_ok(PivotDateRule::NextMonth, serial(2027, 1, 9), None, None, dec));
        assert!(date_rule_ok(PivotDateRule::Between, serial(2026, 3, 1), Some(serial(2026, 4, 1)), Some(serial(2026, 2, 1)), dec));
    }

    #[test]
    fn top_and_value_drops() {
        let scored = [(0, Some(415.0)), (1, Some(285.0)), (2, Some(135.0))];
        let top = |top, count, by| {
            let mut d = rule_drops(&PivotRule::Top { top, count, by, field: String::new() }, &scored);
            d.sort();
            d
        };
        assert_eq!(top(true, 1.0, PivotTopBy::Items), vec![1, 2]);
        assert_eq!(top(false, 1.0, PivotTopBy::Items), vec![0, 1]);
        assert_eq!(top(true, 50.0, PivotTopBy::Percent), vec![2]);
        assert_eq!(top(true, 400.0, PivotTopBy::Sum), vec![1, 2]);
        assert_eq!(top(true, 99.0, PivotTopBy::Items), Vec::<u32>::new());
        let v = rule_drops(&PivotRule::Value { op: PivotCompare::Greater, field: String::new(), value: 200.0, value2: 0.0 }, &scored);
        assert_eq!(v, vec![2]);
    }
}
