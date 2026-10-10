//! PivotTables: pivot cache definitions and records (ECMA-376 Part 1 §18.10.1) and PivotTable
//! definitions (§18.10.1.73), written from the model's [`PivotTable`]s and read back into them.
//!
//! Writing: one cache per PivotTable, with a records snapshot of the source list (omitted, with
//! `saveData="0"`, for very large sources) and `refreshOnLoad="1"` so a reader recomputes the
//! report from the source on open. Fields used on an axis get shared items (or, for date
//! grouping, `fieldGroup` group items) so the table's `<items>` can refer to them. The report
//! cells the engine wrote stay on the sheet as plain cells.
//!
//! Reading: the cache's source and field names plus the table's areas, functions, show-values-as,
//! layout flags, grand totals and style become a model [`PivotTable`]. Anything we don't model
//! (OLAP sources, calculated fields, numeric grouping…) becomes a warning.

use std::collections::HashMap;
use std::fmt::Write as _;

use gridcraft_core::date::datetime_from_serial;
use gridcraft_core::{CellRef, DateSystem, RangeRef, Value, number_to_text};
use gridcraft_formula::quote_sheet;
use gridcraft_model::{
    BASE_NEXT, BASE_PREVIOUS, PivotCompare, PivotDateGroup, PivotDateRule, PivotField, PivotFilter, PivotFunc, PivotLayout, PivotManualGroup,
    PivotNamedGroup, PivotNumberGroup, PivotRule, PivotShowAs, PivotSort, PivotSubtotals, PivotTable, PivotTopBy, PivotValue, Workbook,
};

use crate::IoError;
use crate::read::Ctx;
use crate::write::{NS_MAIN, NS_REL, Out, Rels, XML_DECL, abs_range};
use crate::xml::{El, esc_attr};

pub const CT_PIVOT_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml";
pub const CT_PIVOT_CACHE_DEF: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml";
pub const CT_PIVOT_CACHE_REC: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml";

/// Largest source block scanned for a cache (cells); bigger sources are not written.
const MAX_SOURCE_CELLS: u64 = 10_000_000;
/// Largest records snapshot written (cells); bigger caches are written without records.
const MAX_RECORD_CELLS: u64 = 2_000_000;
/// Most shared items kept per field (Excel's limit is 1,048,576 unique items).
const MAX_ITEMS: usize = 1_048_576;
/// Most cache fields read.
const MAX_FIELDS: usize = 16_384;
/// The `x14:dataField` extension (Excel 2010 show-values-as additions such as rank).
const EXT_X14_DATA_FIELD: &str = "{E15A36E0-9728-4e99-A89B-3F7291B0FE68}";
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
/// The `x14:pivotField` extension (repeat item labels).
const EXT_X14_PIVOT_FIELD: &str = "{2946ED86-A175-432a-8AC1-64E0C546D7DE}";
/// The `x14:pivotTableDefinition` extension (alt text, hidden Values row).
const EXT_X14_PIVOT_TABLE: &str = "{962EF5D1-5CA2-4c93-8EF4-DBF5C05439D2}";

/// A custom subtotal's item type, which also names its pivotField flag (`sumSubtotal`).
fn subtotal_type(f: PivotFunc) -> &'static str {
    match f {
        PivotFunc::Sum => "sum",
        PivotFunc::Count => "countA",
        PivotFunc::Average => "avg",
        PivotFunc::Max => "max",
        PivotFunc::Min => "min",
        PivotFunc::Product => "product",
        PivotFunc::CountNumbers => "count",
        PivotFunc::StdDev => "stdDev",
        PivotFunc::StdDevP => "stdDevP",
        PivotFunc::Var => "var",
        PivotFunc::VarP => "varP",
    }
}

/// The x14 extension element `name` inside an `extLst`.
fn x14_ext<'a>(e: &'a El, name: &str) -> Option<&'a El> {
    e.child("extLst")?.kids("ext").find_map(|x| x.child(name))
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// ================================================================ shared helpers

/// The pivot item label of a value (blank shows as "(blank)").
fn label(v: &Value) -> String {
    match v {
        Value::Empty => "(blank)".into(),
        Value::Number(n) => number_to_text(*n),
        Value::Text(t) => t.to_string(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Value::Error(e) => e.as_str().into(),
        Value::Array(_) => label(&v.scalar()),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn func_name(f: PivotFunc) -> &'static str {
    match f {
        PivotFunc::Sum => "sum",
        PivotFunc::Count => "count",
        PivotFunc::Average => "average",
        PivotFunc::Max => "max",
        PivotFunc::Min => "min",
        PivotFunc::Product => "product",
        PivotFunc::CountNumbers => "countNums",
        PivotFunc::StdDev => "stdDev",
        PivotFunc::StdDevP => "stdDevp",
        PivotFunc::Var => "var",
        PivotFunc::VarP => "varp",
    }
}

fn func_from(s: &str) -> Option<PivotFunc> {
    Some(match s {
        "sum" => PivotFunc::Sum,
        "count" => PivotFunc::Count,
        "average" => PivotFunc::Average,
        "max" => PivotFunc::Max,
        "min" => PivotFunc::Min,
        "product" => PivotFunc::Product,
        "countNums" => PivotFunc::CountNumbers,
        "stdDev" => PivotFunc::StdDev,
        "stdDevp" => PivotFunc::StdDevP,
        "var" => PivotFunc::Var,
        "varp" => PivotFunc::VarP,
        _ => return None,
    })
}

fn func_caption(f: PivotFunc) -> &'static str {
    match f {
        PivotFunc::Sum => "Sum",
        PivotFunc::Count => "Count",
        PivotFunc::Average => "Average",
        PivotFunc::Max => "Max",
        PivotFunc::Min => "Min",
        PivotFunc::Product => "Product",
        PivotFunc::CountNumbers => "Count",
        PivotFunc::StdDev => "StdDev",
        PivotFunc::StdDevP => "StdDevp",
        PivotFunc::Var => "Var",
        PivotFunc::VarP => "Varp",
    }
}

fn group_by_name(g: PivotDateGroup) -> &'static str {
    match g {
        PivotDateGroup::Years => "years",
        PivotDateGroup::Quarters => "quarters",
        PivotDateGroup::Months => "months",
        PivotDateGroup::Days => "days",
        PivotDateGroup::Hours => "hours",
        PivotDateGroup::Minutes => "minutes",
        PivotDateGroup::Seconds => "seconds",
        PivotDateGroup::None => "range",
    }
}

fn group_from_name(s: &str) -> Option<PivotDateGroup> {
    Some(match s {
        "years" => PivotDateGroup::Years,
        "quarters" => PivotDateGroup::Quarters,
        "months" => PivotDateGroup::Months,
        "days" => PivotDateGroup::Days,
        "hours" => PivotDateGroup::Hours,
        "minutes" => PivotDateGroup::Minutes,
        "seconds" => PivotDateGroup::Seconds,
        _ => return None,
    })
}

fn group_caption(g: PivotDateGroup) -> &'static str {
    match g {
        PivotDateGroup::Years => "Years",
        PivotDateGroup::Quarters => "Quarters",
        PivotDateGroup::Months => "Months",
        PivotDateGroup::Days => "Days",
        PivotDateGroup::Hours => "Hours",
        PivotDateGroup::Minutes => "Minutes",
        PivotDateGroup::Seconds => "Seconds",
        PivotDateGroup::None => "Group",
    }
}

/// The "a-b" caption of the number range starting at `lo` (as the engine labels it).
fn range_caption(start: f64, by: f64, lo: f64) -> String {
    let t = |x: f64| number_to_text((x * 1e9).round() / 1e9);
    let whole = start.fract() == 0.0 && by.fract() == 0.0 && by >= 1.0;
    let hi = if whole { lo + by - 1.0 } else { lo + by };
    format!("{}-{}", t(lo), t(hi))
}

/// `2024-03-05T00:00:00` for a date serial.
fn iso(sys: DateSystem, serial: f64) -> Option<String> {
    let d = datetime_from_serial(sys, serial)?;
    Some(format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", d.year, d.month, d.day, d.hour, d.minute, d.second))
}

/// `3/5/2024` for a date serial (group boundary captions).
fn mdy(sys: DateSystem, serial: f64) -> String {
    match datetime_from_serial(sys, serial) {
        Some(d) => format!("{}/{}/{}", d.month, d.day, d.year),
        None => number_to_text(serial),
    }
}

// ================================================================ writing

/// Identity of a shared item (text items match case-insensitively, as in Excel).
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Blank,
    Num(u64),
    Text(String),
    Bool(bool),
    Err(&'static str),
}

fn key(v: &Value) -> Key {
    match v {
        Value::Empty => Key::Blank,
        Value::Number(n) => Key::Num(if *n == 0.0 { 0 } else { n.to_bits() }),
        Value::Text(t) => Key::Text(t.to_lowercase()),
        Value::Bool(b) => Key::Bool(*b),
        Value::Error(e) => Key::Err(e.as_str()),
        Value::Array(_) => key(&v.scalar()),
    }
}

#[derive(Default)]
struct Stats {
    blank: bool,
    text: bool,
    number: bool,
    non_int: bool,
    boolean: bool,
    error: bool,
    long: bool,
    min: f64,
    max: f64,
}

impl Stats {
    fn add(&mut self, v: &Value) {
        match v {
            Value::Empty => self.blank = true,
            Value::Number(n) => {
                if !self.number {
                    self.min = *n;
                    self.max = *n;
                }
                self.number = true;
                self.min = self.min.min(*n);
                self.max = self.max.max(*n);
                if n.fract() != 0.0 {
                    self.non_int = true;
                }
            }
            Value::Text(t) => {
                self.text = true;
                if t.chars().count() > 255 {
                    self.long = true;
                }
            }
            Value::Bool(_) => self.boolean = true,
            Value::Error(_) => self.error = true,
            Value::Array(_) => self.add(&v.scalar()),
        }
    }
    fn kinds(&self) -> usize {
        [self.text, self.number, self.boolean, self.error].iter().filter(|b| **b).count()
    }
}

/// Date grouping of a cache field.
struct Group {
    by: PivotDateGroup,
    /// The database field the grouping applies to.
    base: usize,
    /// The next (coarser) grouping field of the same base.
    par: Option<usize>,
    start: f64,
    end: f64,
    /// Group item captions, including the `<start` and `>end` items.
    items: Vec<String>,
    /// Days per group (days grouping) or the number range step.
    interval: f64,
    /// A number range grouping (`groupBy="range"`).
    number: bool,
    /// The bounds were set by the user (`autoStart` / `autoEnd` off).
    fixed: bool,
    /// Manual grouping: for each base shared item, its group item.
    discrete: Option<Vec<u32>>,
}

struct CField {
    name: String,
    /// Column offset in the source (database fields only).
    col: Option<u32>,
    stats: Stats,
    /// Shared items (written when `itemized`).
    items: Vec<Value>,
    index: HashMap<Key, u32>,
    itemized: bool,
    group: Option<Group>,
}

impl CField {
    /// Grouped by date parts (its values are dates).
    fn date_grouped(&self) -> bool {
        self.group.as_ref().is_some_and(|g| !g.number && g.discrete.is_none())
    }
    /// Item captions the PivotTable's `<item x>` indices refer to.
    fn item_labels(&self) -> Vec<String> {
        match &self.group {
            Some(g) => g.items.clone(),
            None => self.items.iter().map(label).collect(),
        }
    }
}

/// A cache built from a PivotTable's source.
struct Cache {
    fields: Vec<CField>,
    source_xml: String,
    records: Option<(u64, String)>,
}

/// PivotTables being written; the tables themselves are finished after the styles part (their
/// value number formats need style number-format ids). PivotTables on the same source share one
/// cache, as in Excel: its fields cover every member's axis fields and groupings.
#[derive(Default)]
pub struct PivotWriter {
    /// (cache id, workbook-relative part name).
    caches: Vec<(u32, String)>,
    /// The shared caches, by cache id − 1.
    built: Vec<Cache>,
    /// (sheet, pivot index) → cache id.
    cache_of: HashMap<(usize, usize), u32>,
    pending: Vec<Pending>,
    tables: u32,
}

struct Pending {
    part: String,
    sheet: usize,
    idx: usize,
    cache_id: u32,
    axis_map: Vec<Option<usize>>,
}

impl PivotWriter {
    /// Builds and writes one cache per distinct source. Call before [`PivotWriter::add_sheet`].
    pub fn prepare(&mut self, wb: &Workbook, out: &mut Out) {
        // Group the pivots by source: the same range (or the same table or name) shares a cache;
        // a table and a range over the same cells don't.
        let mut groups: Vec<((usize, RangeRef, String), Vec<(usize, usize)>)> = Vec::new();
        for (si, sheet) in wb.sheets.iter().enumerate() {
            for (idx, pt) in sheet.pivots.iter().enumerate() {
                let Some(key) = resolve_source(wb, &pt.source, si, 0) else { continue };
                match groups.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, members)) => members.push((si, idx)),
                    None => groups.push((key, vec![(si, idx)])),
                }
            }
        }
        for (_, members) in groups {
            let Some(&(si0, i0)) = members.first() else { continue };
            let Some(first) = wb.sheet(si0).and_then(|s| s.pivots.get(i0)) else { continue };
            // A stand-in pivot with every member's axis fields and filters.
            let mut union = first.clone();
            union.rows.clear();
            union.columns.clear();
            union.filters.clear();
            for &(si, idx) in &members {
                let Some(pt) = wb.sheet(si).and_then(|s| s.pivots.get(idx)) else { continue };
                for f in pt.rows.iter().chain(pt.columns.iter()) {
                    if !union.rows.iter().any(|u| same(&u.source_col, &f.source_col) && u.date_group == f.date_group) {
                        union.rows.push(f.clone());
                    }
                }
                for f in &pt.filters {
                    if !union.filters.iter().any(|u| same(&u.source_col, &f.source_col)) {
                        union.filters.push(f.clone());
                    }
                }
            }
            let Some(cache) = build_cache(wb, &union, si0) else { continue };
            let n = self.caches.len() as u32 + 1;
            let cache_part = format!("xl/pivotCache/pivotCacheDefinition{n}.xml");
            let rec_rid = if let Some((count, xml)) = &cache.records {
                let mut r = Rels::default();
                let rid = r.add("pivotCacheRecords", &format!("pivotCacheRecords{n}.xml"));
                out.part(&format!("xl/pivotCache/_rels/pivotCacheDefinition{n}.xml.rels"), None, r.xml().into_bytes());
                let s = format!("{XML_DECL}<pivotCacheRecords xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\" count=\"{count}\">{xml}</pivotCacheRecords>");
                out.part(&format!("xl/pivotCache/pivotCacheRecords{n}.xml"), Some(CT_PIVOT_CACHE_REC), s.into_bytes());
                Some(rid)
            } else {
                None
            };
            out.part(&cache_part, Some(CT_PIVOT_CACHE_DEF), cache_xml(&cache, rec_rid.as_deref(), wb.date_system).into_bytes());
            self.caches.push((n, format!("pivotCache/pivotCacheDefinition{n}.xml")));
            self.built.push(cache);
            for m in members {
                self.cache_of.insert(m, n);
            }
        }
    }

    /// Adds the sheet's relationships to its PivotTable parts. Pivots whose source couldn't be
    /// resolved (no cache) are not written.
    pub fn add_sheet(&mut self, wb: &Workbook, si: usize, out: &mut Out, rels: &mut Rels) {
        let Some(sheet) = wb.sheet(si) else { return };
        for (idx, pt) in sheet.pivots.iter().enumerate() {
            let Some(&n) = self.cache_of.get(&(si, idx)) else { continue };
            let Some(cache) = self.built.get(n as usize - 1) else { continue };
            let axis_map = axis_map_for(&cache.fields, pt);
            self.tables += 1;
            let t = self.tables;
            rels.add("pivotTable", &format!("../pivotTables/pivotTable{t}.xml"));
            let mut tr = Rels::default();
            tr.add("pivotCacheDefinition", &format!("../pivotCache/pivotCacheDefinition{n}.xml"));
            out.part(&format!("xl/pivotTables/_rels/pivotTable{t}.xml.rels"), None, tr.xml().into_bytes());
            self.pending.push(Pending { part: format!("xl/pivotTables/pivotTable{t}.xml"), sheet: si, idx, cache_id: n, axis_map });
        }
    }

    /// Adds the workbook relationships to the caches; returns the `<pivotCaches>` element (empty
    /// when there are none).
    pub fn workbook_xml(&self, wb_rels: &mut Rels) -> String {
        if self.caches.is_empty() {
            return String::new();
        }
        let mut s = String::from("<pivotCaches>");
        for (id, target) in &self.caches {
            let rid = wb_rels.add("pivotCacheDefinition", target);
            let _ = write!(s, "<pivotCache cacheId=\"{id}\" r:id=\"{rid}\"/>");
        }
        s.push_str("</pivotCaches>");
        s
    }

    /// Number formats used by value fields (to be registered in the styles part).
    pub fn number_formats(&self, wb: &Workbook) -> Vec<String> {
        let mut v = Vec::new();
        for p in &self.pending {
            if let Some(pt) = wb.sheet(p.sheet).and_then(|s| s.pivots.get(p.idx)) {
                for val in &pt.values {
                    if let Some(f) = &val.number_format
                        && !v.contains(f)
                    {
                        v.push(f.clone());
                    }
                }
            }
        }
        v
    }

    /// Writes the PivotTable parts. `fmt_id` maps a number format code to its styles id.
    pub fn finish(self, wb: &Workbook, out: &mut Out, fmt_id: &dyn Fn(&str) -> Option<u32>) {
        for p in &self.pending {
            let Some(pt) = wb.sheet(p.sheet).and_then(|s| s.pivots.get(p.idx)) else { continue };
            let Some(cache) = self.built.get(p.cache_id as usize - 1) else { continue };
            let xml = table_xml(pt, p.idx, p.cache_id, cache, &p.axis_map, fmt_id);
            out.part(&p.part, Some(CT_PIVOT_TABLE), xml.into_bytes());
        }
    }
}

/// Resolves a pivot source to (sheet, range including the header row, worksheetSource XML).
fn resolve_source(wb: &Workbook, text: &str, default_sheet: usize, depth: u32) -> Option<(usize, RangeRef, String)> {
    let t = text.trim().trim_start_matches('=').trim();
    if t.is_empty() || depth > 4 {
        return None;
    }
    if let Some((si, ti)) = wb.table(t)
        && let Some(sh) = wb.sheet(si)
        && let Some(tb) = sh.tables.get(ti)
    {
        if !tb.header_row {
            return None;
        }
        let end_row = tb.range.end.row.saturating_sub(tb.totals_row as u32).max(tb.range.start.row);
        let r = RangeRef::new(tb.range.start, CellRef::new(end_row, tb.range.end.col));
        // Table names are sanitized when written; refer by range when this one would change.
        let valid = !tb.name.is_empty()
            && tb.name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
            && tb.name.starts_with(|c: char| c.is_alphabetic() || c == '_')
            && CellRef::parse(&tb.name).is_none();
        let xml = if valid { format!("<worksheetSource name=\"{}\"/>", esc_attr(&tb.name)) } else { range_source_xml(&sh.name, r) };
        return Some((si, r, xml));
    }
    let (sheet, body) = match t.rsplit_once('!') {
        Some((sh, body)) => (Some(sh.trim_matches('\'').replace("''", "'")), body),
        None => (None, t),
    };
    if let Some(r) = RangeRef::parse(body) {
        let si = match &sheet {
            Some(n) => wb.sheet_index(n)?,
            None => default_sheet,
        };
        let sh = wb.sheet(si)?;
        let r = if r.is_full_cols() || r.is_full_rows() { r.intersection(&sh.used_range()?)? } else { r };
        return Some((si, r, range_source_xml(&sh.name, r)));
    }
    if sheet.is_none()
        && let Some(n) = wb.name(t, default_sheet)
    {
        let (si, r, _) = resolve_source(wb, &n.formula.clone(), default_sheet, depth + 1)?;
        let xml = if n.scope.is_none() {
            format!("<worksheetSource name=\"{}\"/>", esc_attr(&n.name))
        } else {
            wb.sheet(si).map(|s| range_source_xml(&s.name, r))?
        };
        return Some((si, r, xml));
    }
    None
}

fn range_source_xml(sheet: &str, r: RangeRef) -> String {
    let name: String = sheet.chars().take(31).collect();
    format!("<worksheetSource ref=\"{}\" sheet=\"{}\"/>", r.a1(), esc_attr(&name))
}

fn header_text(v: &Value) -> String {
    match v {
        Value::Empty => String::new(),
        v => label(v).trim().to_string(),
    }
}

fn build_cache(wb: &Workbook, pt: &PivotTable, pivot_sheet: usize) -> Option<Cache> {
    let (si, r, source_xml) = resolve_source(wb, &pt.source, pivot_sheet, 0)?;
    if r.count() > MAX_SOURCE_CELLS {
        return None;
    }
    let sh = wb.sheet(si)?;
    let mut fields: Vec<CField> = Vec::new();
    // Lower-case names taken, and the next suffix for each header (linear for repeated headers).
    let mut taken = std::collections::HashSet::new();
    let mut next_suffix: HashMap<String, usize> = HashMap::new();
    for (k, c) in (r.start.col..=r.end.col).enumerate() {
        let mut h = header_text(&sh.value(CellRef::new(r.start.row, c)).scalar());
        if h.is_empty() {
            h = format!("Column{}", k + 1);
        }
        let mut n = h.clone();
        let mut i = next_suffix.get(&h.to_lowercase()).copied().unwrap_or(2);
        while taken.contains(&n.to_lowercase()) {
            n = format!("{h}{i}");
            i += 1;
        }
        next_suffix.insert(h.to_lowercase(), i);
        taken.insert(n.to_lowercase());
        fields.push(CField {
            name: n,
            col: Some(k as u32),
            stats: Stats::default(),
            items: vec![],
            index: HashMap::new(),
            itemized: false,
            group: None,
        });
    }
    let find = |fields: &[CField], name: &str| fields.iter().position(|f| f.col.is_some() && same(&f.name, name));

    // Records (rows that aren't entirely blank), read once.
    let mut rows: Vec<u32> = Vec::new();
    if r.end.row > r.start.row {
        for row in r.start.row + 1..=r.end.row {
            let blank = (r.start.col..=r.end.col).all(|c| sh.value(CellRef::new(row, c)).scalar().is_empty());
            if !blank {
                rows.push(row);
            }
        }
    }
    for &row in &rows {
        for f in fields.iter_mut() {
            let c = r.start.col + f.col.unwrap_or(0);
            f.stats.add(&sh.value(CellRef::new(row, c)).scalar());
        }
    }

    // Which fields are on an axis, and how dates are grouped: per base column, the base field
    // takes the finest grouping and coarser ones become extra (non-database) fields chained
    // through `par`, as Excel lays them out.
    let axis: Vec<&PivotField> = pt.rows.iter().chain(pt.columns.iter()).collect();
    let groupable = |f: &CField| {
        let s = &f.stats;
        s.number
            && !s.text
            && !s.boolean
            && !s.error
            && datetime_from_serial(wb.date_system, s.min).is_some()
            && datetime_from_serial(wb.date_system, s.max).is_some()
    };
    // Number ranges group the base field in place.
    for pf in &axis {
        let (Some(ng), Some(b)) = (pf.number_group, find(&fields, &pf.source_col)) else { continue };
        if let Some(f) = fields.get_mut(b)
            && f.group.is_none()
            && f.stats.number
            && !f.stats.text
        {
            f.group = Some(Group {
                by: PivotDateGroup::None,
                base: b,
                par: None,
                start: ng.start,
                end: ng.end,
                items: range_items(ng.start, ng.end, ng.by),
                interval: ng.by,
                number: true,
                fixed: true,
                discrete: None,
            });
        }
    }
    // (base column field, groupings fine → coarse, day interval, fixed start, fixed end)
    let mut groupings: Vec<(usize, Vec<PivotDateGroup>, u32, Option<f64>, Option<f64>)> = Vec::new();
    for pf in &axis {
        let Some(b) = find(&fields, &pf.source_col) else { continue };
        if pf.date_group == PivotDateGroup::None || pf.number_group.is_some() || !fields.get(b).is_some_and(groupable) {
            continue;
        }
        match groupings.iter_mut().find(|g| g.0 == b) {
            Some(g) => {
                if !g.1.contains(&pf.date_group) {
                    g.1.push(pf.date_group);
                }
                if pf.date_group == PivotDateGroup::Days && pf.group_days > 1 {
                    g.2 = pf.group_days;
                }
                g.3 = g.3.or(pf.group_start);
                g.4 = g.4.or(pf.group_end);
            }
            None => groupings.push((
                b,
                vec![pf.date_group],
                if pf.date_group == PivotDateGroup::Days { pf.group_days } else { 0 },
                pf.group_start,
                pf.group_end,
            )),
        }
    }
    let fineness = |g: &PivotDateGroup| match g {
        PivotDateGroup::Seconds => 0,
        PivotDateGroup::Minutes => 1,
        PivotDateGroup::Hours => 2,
        PivotDateGroup::Days => 3,
        PivotDateGroup::Months => 4,
        PivotDateGroup::Quarters => 5,
        PivotDateGroup::Years => 6,
        PivotDateGroup::None => 7,
    };
    for (b, list, interval, fixed_start, fixed_end) in groupings.iter_mut() {
        list.sort_by_key(fineness);
        let (min, max) = fields.get(*b).map(|f| (f.stats.min, f.stats.max)).unwrap_or((0.0, 0.0));
        let start = fixed_start.unwrap_or(min.floor());
        let end = fixed_end.unwrap_or(max.floor() + 1.0);
        let fixed = fixed_start.is_some() || fixed_end.is_some();
        let interval = *interval;
        let make = |by: PivotDateGroup| Group {
            by,
            base: *b,
            par: None,
            start,
            end,
            items: group_items(wb.date_system, by, start, end, if by == PivotDateGroup::Days { interval } else { 0 }),
            interval: if by == PivotDateGroup::Days { interval.max(1) as f64 } else { 1.0 },
            number: false,
            fixed,
            discrete: None,
        };
        let mut prev = *b;
        for (k, by) in list.iter().enumerate() {
            if k == 0 {
                if let Some(f) = fields.get_mut(*b) {
                    f.group = Some(make(*by));
                }
                continue;
            }
            let new = fields.len();
            let caption = group_caption(*by);
            let mut n = caption.to_string();
            let mut i = 2;
            while fields.iter().any(|f| same(&f.name, &n)) {
                n = format!("{caption}{i}");
                i += 1;
            }
            if let Some(g) = fields.get_mut(prev).and_then(|f| f.group.as_mut()) {
                g.par = Some(new);
            }
            fields.push(CField {
                name: n,
                col: None,
                stats: Stats::default(),
                items: vec![],
                index: HashMap::new(),
                itemized: false,
                group: Some(make(*by)),
            });
            prev = new;
        }
    }
    // Manual groups need their base field's shared items.
    for mg in &pt.manual_groups {
        if let Some(b) = find(&fields, &mg.base)
            && let Some(f) = fields.get_mut(b)
        {
            f.itemized = true;
        }
    }
    for pf in &axis {
        let Some(b) = find(&fields, &pf.source_col) else { continue };
        let grouped = fields.iter().any(|f| f.group.as_ref().is_some_and(|g| g.base == b && (g.number || g.by == pf.date_group)));
        if !(grouped && (pf.date_group != PivotDateGroup::None || pf.number_group.is_some()))
            && let Some(f) = fields.get_mut(b)
            && f.group.is_none()
        {
            f.itemized = true;
        }
    }
    // Page fields (filters not on an axis) need items too.
    for f in &pt.filters {
        let on_axis = axis.iter().any(|a| same(&a.source_col, &f.source_col));
        if !on_axis
            && let Some(i) = find(&fields, &f.source_col)
            && let Some(cf) = fields.get_mut(i)
        {
            cf.itemized = true;
        }
    }
    for f in fields.iter_mut() {
        if f.group.is_some() {
            f.itemized = false;
        } else if f.col.is_some() && (f.stats.text || f.stats.boolean || f.stats.error) {
            f.itemized = true;
        }
    }
    // Shared items, in order of first appearance.
    for &row in &rows {
        for f in fields.iter_mut().filter(|f| f.itemized) {
            let c = r.start.col + f.col.unwrap_or(0);
            let v = sh.value(CellRef::new(row, c)).scalar();
            let k = key(&v);
            if !f.index.contains_key(&k) && f.items.len() < MAX_ITEMS {
                f.index.insert(k, f.items.len() as u32);
                f.items.push(v);
            }
        }
    }
    // Manual group fields: each base item maps to its group, or to an item of its own.
    for mg in &pt.manual_groups {
        let Some(b) = find(&fields, &mg.base) else { continue };
        if fields.iter().any(|f| same(&f.name, &mg.name)) {
            continue;
        }
        let base_labels: Vec<String> = fields.get(b).map(|f| f.items.iter().map(label).collect()).unwrap_or_default();
        let mut items: Vec<String> = Vec::new();
        let mut map: Vec<u32> = Vec::with_capacity(base_labels.len());
        for l in &base_labels {
            let name = mg.groups.iter().find(|g| g.items.iter().any(|i| same(i, l))).map(|g| g.name.clone()).unwrap_or_else(|| l.clone());
            let at = match items.iter().position(|x| same(x, &name)) {
                Some(p) => p,
                None => {
                    items.push(name);
                    items.len() - 1
                }
            };
            map.push(at as u32);
        }
        fields.push(CField {
            name: mg.name.clone(),
            col: None,
            stats: Stats::default(),
            items: vec![],
            index: HashMap::new(),
            itemized: false,
            group: Some(Group {
                by: PivotDateGroup::None,
                base: b,
                par: None,
                start: 0.0,
                end: 0.0,
                items,
                interval: 0.0,
                number: false,
                fixed: false,
                discrete: Some(map),
            }),
        });
    }
    // Records snapshot.
    let db_fields = fields.iter().filter(|f| f.col.is_some()).count() as u64;
    let records = if (rows.len() as u64).saturating_mul(db_fields) <= MAX_RECORD_CELLS {
        let mut s = String::new();
        for &row in &rows {
            s.push_str("<r>");
            for f in fields.iter().filter(|f| f.col.is_some()) {
                let c = r.start.col + f.col.unwrap_or(0);
                let v = sh.value(CellRef::new(row, c)).scalar();
                if f.itemized {
                    match f.index.get(&key(&v)) {
                        Some(i) => {
                            let _ = write!(s, "<x v=\"{i}\"/>");
                        }
                        None => s.push_str(&item_xml(&v, wb.date_system, false)),
                    }
                } else {
                    s.push_str(&item_xml(&v, wb.date_system, f.date_grouped()));
                }
            }
            s.push_str("</r>");
        }
        Some((rows.len() as u64, s))
    } else {
        None
    };
    Some(Cache { fields, source_xml, records })
}

fn group_items(sys: DateSystem, by: PivotDateGroup, start: f64, end: f64, interval: u32) -> Vec<String> {
    let mut v = vec![format!("<{}", mdy(sys, start))];
    match by {
        PivotDateGroup::Years => {
            let y0 = datetime_from_serial(sys, start).map(|d| d.year).unwrap_or(1900);
            let y1 = datetime_from_serial(sys, end - 1.0).map(|d| d.year).unwrap_or(y0).max(y0);
            for y in y0..=y1.min(y0 + 10_000) {
                v.push(y.to_string());
            }
        }
        PivotDateGroup::Quarters => v.extend((1..=4).map(|q| format!("Qtr{q}"))),
        PivotDateGroup::Months => v.extend(MONTHS.iter().map(|m| m.to_string())),
        PivotDateGroup::Hours => v.extend((0..24).map(|h| format!("{} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" }))),
        PivotDateGroup::Minutes | PivotDateGroup::Seconds => v.extend((0..60).map(|m| format!(":{m:02}"))),
        PivotDateGroup::Days if interval > 1 => {
            let step = interval as f64;
            let mut lo = start.floor();
            let mut n = 0;
            while lo < end && n < 100_000 {
                v.push(format!("{} - {}", mdy(sys, lo), mdy(sys, lo + step - 1.0)));
                lo += step;
                n += 1;
            }
        }
        PivotDateGroup::Days | PivotDateGroup::None => {
            const DAYS: [u32; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            for (m, n) in MONTHS.iter().zip(DAYS) {
                for d in 1..=n {
                    v.push(format!("{d}-{m}"));
                }
            }
        }
    }
    v.push(format!(">{}", mdy(sys, end)));
    v
}

/// Group items of a number range grouping: `<start`, the ranges, `>end`.
fn range_items(start: f64, end: f64, by: f64) -> Vec<String> {
    let t = |x: f64| number_to_text((x * 1e9).round() / 1e9);
    let by = if by > 0.0 && by.is_finite() { by } else { 1.0 };
    let mut v = vec![format!("<{}", t(start))];
    let mut lo = start;
    let mut n = 0;
    while lo <= end && n < 100_000 {
        v.push(range_caption(start, by, lo));
        lo += by;
        n += 1;
    }
    v.push(format!(">{}", t(end)));
    v
}

/// A cache value element (`<n>`, `<s>`, `<m>`…); `date` writes numbers as `<d>`.
fn item_xml(v: &Value, sys: DateSystem, date: bool) -> String {
    match v {
        Value::Empty => "<m/>".into(),
        Value::Number(n) => {
            if date && let Some(d) = iso(sys, *n) {
                return format!("<d v=\"{d}\"/>");
            }
            format!("<n v=\"{}\"/>", crate::xml::num(*n))
        }
        Value::Text(t) => format!("<s v=\"{}\"/>", esc_attr(t)),
        Value::Bool(b) => format!("<b v=\"{}\"/>", u8::from(*b)),
        Value::Error(e) => format!("<e v=\"{}\"/>", esc_attr(e.as_str())),
        Value::Array(_) => item_xml(&v.scalar(), sys, date),
    }
}

fn cache_xml(c: &Cache, records_rid: Option<&str>, sys: DateSystem) -> String {
    let mut s = format!("{XML_DECL}<pivotCacheDefinition xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\"");
    match (records_rid, &c.records) {
        (Some(rid), Some((count, _))) => {
            let _ = write!(s, " r:id=\"{rid}\" recordCount=\"{count}\"");
        }
        _ => s.push_str(" saveData=\"0\""),
    }
    s.push_str(" refreshOnLoad=\"1\" refreshedBy=\"GridCraft\" createdVersion=\"6\" refreshedVersion=\"6\" minRefreshableVersion=\"3\">");
    let _ = write!(s, "<cacheSource type=\"worksheet\">{}</cacheSource>", c.source_xml);
    let _ = write!(s, "<cacheFields count=\"{}\">", c.fields.len());
    for f in &c.fields {
        let fmt = if f.date_grouped() { 14 } else { 0 };
        let _ = write!(s, "<cacheField name=\"{}\" numFmtId=\"{fmt}\"", esc_attr(&f.name));
        if f.col.is_none() {
            s.push_str(" databaseField=\"0\"");
        }
        s.push('>');
        let st = &f.stats;
        if f.col.is_some() {
            s.push_str("<sharedItems");
            if let Some(g) = &f.group {
                s.push_str(" containsSemiMixedTypes=\"0\" containsNonDate=\"0\" containsDate=\"1\" containsString=\"0\"");
                if st.blank {
                    s.push_str(" containsBlank=\"1\"");
                }
                if let (Some(a), Some(b)) = (iso(sys, st.min), iso(sys, st.max)) {
                    let _ = write!(s, " minDate=\"{a}\" maxDate=\"{b}\"");
                }
                let _ = g;
            } else {
                let semi = st.text || st.blank || st.boolean || st.error;
                if !semi {
                    s.push_str(" containsSemiMixedTypes=\"0\"");
                }
                if !st.text {
                    s.push_str(" containsString=\"0\"");
                }
                if st.blank {
                    s.push_str(" containsBlank=\"1\"");
                }
                if st.kinds() > 1 {
                    s.push_str(" containsMixedTypes=\"1\"");
                }
                if st.number {
                    s.push_str(" containsNumber=\"1\"");
                    if !st.non_int {
                        s.push_str(" containsInteger=\"1\"");
                    }
                    let _ = write!(s, " minValue=\"{}\" maxValue=\"{}\"", crate::xml::num(st.min), crate::xml::num(st.max));
                }
                if st.long {
                    s.push_str(" longText=\"1\"");
                }
            }
            if f.itemized {
                let _ = write!(s, " count=\"{}\">", f.items.len());
                for v in &f.items {
                    s.push_str(&item_xml(v, sys, false));
                }
                s.push_str("</sharedItems>");
            } else {
                s.push_str("/>");
            }
        }
        if let Some(g) = &f.group {
            s.push_str("<fieldGroup");
            if let Some(p) = g.par {
                let _ = write!(s, " par=\"{p}\"");
            }
            let _ = write!(s, " base=\"{}\">", g.base);
            match &g.discrete {
                Some(map) => {
                    let _ = write!(s, "<discretePr count=\"{}\">", map.len());
                    for v in map {
                        let _ = write!(s, "<x v=\"{v}\"/>");
                    }
                    s.push_str("</discretePr>");
                }
                None => {
                    let _ = write!(s, "<rangePr");
                    if g.fixed {
                        s.push_str(" autoStart=\"0\" autoEnd=\"0\"");
                    }
                    let _ = write!(s, " groupBy=\"{}\"", group_by_name(g.by));
                    if g.number {
                        let _ = write!(
                            s,
                            " startNum=\"{}\" endNum=\"{}\" groupInterval=\"{}\"",
                            crate::xml::num(g.start),
                            crate::xml::num(g.end),
                            crate::xml::num(g.interval)
                        );
                    } else {
                        if let (Some(a), Some(b)) = (iso(sys, g.start), iso(sys, g.end)) {
                            let _ = write!(s, " startDate=\"{a}\" endDate=\"{b}\"");
                        }
                        if g.interval > 1.0 {
                            let _ = write!(s, " groupInterval=\"{}\"", crate::xml::num(g.interval));
                        }
                    }
                    s.push_str("/>");
                }
            }
            let _ = write!(s, "<groupItems count=\"{}\">", g.items.len());
            for it in &g.items {
                let _ = write!(s, "<s v=\"{}\"/>", esc_attr(it));
            }
            s.push_str("</groupItems></fieldGroup>");
        }
        s.push_str("</cacheField>");
    }
    s.push_str("</cacheFields></pivotCacheDefinition>");
    s
}

/// Where each of `pt`'s axis fields (rows, then columns) is among the cache fields.
fn axis_map_for(fields: &[CField], pt: &PivotTable) -> Vec<Option<usize>> {
    // Source columns, else manual group fields.
    let find = |name: &str| {
        fields
            .iter()
            .position(|f| f.col.is_some() && same(&f.name, name))
            .or_else(|| fields.iter().position(|f| f.group.as_ref().is_some_and(|g| g.discrete.is_some()) && same(&f.name, name)))
    };
    pt.rows
        .iter()
        .chain(pt.columns.iter())
        .map(|pf| {
            let b = find(&pf.source_col)?;
            if pf.date_group != PivotDateGroup::None
                && let Some(i) = fields.iter().position(|f| f.group.as_ref().is_some_and(|g| g.base == b && g.by == pf.date_group))
            {
                return Some(i);
            }
            Some(b)
        })
        .collect()
}

fn table_xml(pt: &PivotTable, idx: usize, cache_id: u32, c: &Cache, axis_map: &[Option<usize>], fmt_id: &dyn Fn(&str) -> Option<u32>) -> String {
    let nf = c.fields.len();
    let find_db = |name: &str| c.fields.iter().position(|f| f.col.is_some() && same(&f.name, name));
    let row_idx: Vec<usize> = axis_map.iter().take(pt.rows.len()).filter_map(|x| *x).collect();
    let col_idx: Vec<usize> = axis_map.iter().skip(pt.rows.len()).filter_map(|x| *x).collect();
    // Filters: on an axis field → hidden items there; otherwise a page field.
    let mut hidden: HashMap<usize, &PivotFilter> = HashMap::new();
    let mut pages: Vec<(usize, &PivotFilter)> = Vec::new();
    for f in &pt.filters {
        let axis_pos = pt.rows.iter().chain(pt.columns.iter()).position(|a| same(&a.source_col, &f.source_col));
        match axis_pos {
            Some(p) => {
                if let Some(Some(i)) = axis_map.get(p) {
                    hidden.insert(*i, f);
                }
            }
            None => {
                if let Some(i) = find_db(&f.source_col)
                    && !pages.iter().any(|(j, _)| *j == i)
                {
                    pages.push((i, f));
                    hidden.insert(i, f);
                }
            }
        }
    }
    let vals: Vec<(usize, &PivotValue)> = pt.values.iter().filter_map(|v| find_db(&v.source_col).map(|i| (i, v))).collect();
    let axis_field = |i: usize| -> Option<&PivotField> {
        let p = axis_map.iter().position(|x| *x == Some(i))?;
        pt.rows.iter().chain(pt.columns.iter()).nth(p)
    };

    let compact = pt.layout == PivotLayout::Compact;
    let name = if pt.name.trim().is_empty() { format!("PivotTable{}", idx + 1) } else { pt.name.clone() };
    let mut s = format!(
        "{XML_DECL}<pivotTableDefinition xmlns=\"{NS_MAIN}\" name=\"{}\" cacheId=\"{cache_id}\" applyNumberFormats=\"0\" applyBorderFormats=\"0\" applyFontFormats=\"0\" applyPatternFormats=\"0\" applyAlignmentFormats=\"0\" applyWidthHeightFormats=\"1\" dataCaption=\"Values\" updatedVersion=\"6\" minRefreshableVersion=\"3\" createdVersion=\"6\"",
        esc_attr(&name)
    );
    // PivotTable Options.
    let o = &pt.options;
    let flag = |s: &mut String, attr: &str, on: bool, default: bool| {
        if on != default {
            let _ = write!(s, " {attr}=\"{}\"", u8::from(on));
        }
    };
    if o.indent != 1 {
        let _ = write!(s, " indent=\"{}\"", o.indent);
    }
    flag(&mut s, "useAutoFormatting", o.autofit, false);
    flag(&mut s, "itemPrintTitles", o.repeat_row_labels, false);
    flag(&mut s, "fieldPrintTitles", o.print_titles, false);
    flag(&mut s, "printDrill", o.print_buttons, false);
    flag(&mut s, "mergeItem", o.merge_labels, false);
    flag(&mut s, "pageOverThenDown", o.filters_over_then_down, false);
    if o.filters_per_column > 0 {
        let _ = write!(s, " pageWrap=\"{}\"", o.filters_per_column);
    }
    if let Some(t) = &o.error_text {
        let _ = write!(s, " showError=\"1\" errorCaption=\"{}\"", esc_attr(t));
    }
    if let Some(t) = &o.empty_text {
        let _ = write!(s, " missingCaption=\"{}\"", esc_attr(t));
    }
    flag(&mut s, "preserveFormatting", o.preserve_formatting, true);
    flag(&mut s, "customListSort", o.custom_list_sort, true);
    flag(&mut s, "showDrill", o.expand_buttons, true);
    flag(&mut s, "showDataTips", o.contextual_tooltips, true);
    flag(&mut s, "gridDropZones", o.classic_layout, false);
    flag(&mut s, "showEmptyRow", o.empty_rows, false);
    flag(&mut s, "showEmptyCol", o.empty_columns, false);
    flag(&mut s, "fieldListSortAscending", o.field_list_az, false);
    flag(&mut s, "enableDrill", o.enable_drilldown, true);
    if let Some(t) = &o.row_header_caption {
        let _ = write!(s, " rowHeaderCaption=\"{}\"", esc_attr(t));
    }
    if let Some(t) = &o.column_header_caption {
        let _ = write!(s, " colHeaderCaption=\"{}\"", esc_attr(t));
    }
    match pt.layout {
        PivotLayout::Compact => s.push_str(" outline=\"1\" outlineData=\"1\""),
        PivotLayout::Outline => s.push_str(" compact=\"0\" compactData=\"0\" outline=\"1\" outlineData=\"1\""),
        PivotLayout::Tabular => s.push_str(" compact=\"0\" compactData=\"0\""),
    }
    if !pt.show_headers {
        s.push_str(" showHeaders=\"0\"");
    }
    if !pt.grand_totals_rows {
        s.push_str(" rowGrandTotals=\"0\"");
    }
    if !pt.grand_totals_cols {
        s.push_str(" colGrandTotals=\"0\"");
    }
    let _ = write!(s, " multipleFieldFilters=\"{}\">", u8::from(o.multiple_filters));

    // Location: the body (report filters sit above it).
    let a = pt.anchor;
    let end = pt.last_range.map(|r| r.end).unwrap_or(a);
    let loc = RangeRef::new(a, CellRef::new(end.row.max(a.row), end.col.max(a.col)));
    let k = vals.len();
    let first_data_row = if col_idx.is_empty() { 1 } else { 1 + col_idx.len() + usize::from(k > 1) };
    let first_data_col = if row_idx.is_empty() && col_idx.is_empty() {
        0
    } else if compact {
        1
    } else {
        row_idx.len().max(1)
    };
    let _ = write!(s, "<location ref=\"{}\" firstHeaderRow=\"1\" firstDataRow=\"{first_data_row}\" firstDataCol=\"{first_data_col}\"", loc.a1());
    if !pages.is_empty() {
        let _ = write!(s, " rowPageCount=\"{}\" colPageCount=\"1\"", pages.len());
    }
    s.push_str("/>");

    // Fields.
    let _ = write!(s, "<pivotFields count=\"{nf}\">");
    for (i, f) in c.fields.iter().enumerate() {
        s.push_str("<pivotField");
        let axis = if row_idx.contains(&i) {
            Some("axisRow")
        } else if col_idx.contains(&i) {
            Some("axisCol")
        } else if pages.iter().any(|(j, _)| *j == i) {
            Some("axisPage")
        } else {
            None
        };
        if let Some(ax) = axis {
            let _ = write!(s, " axis=\"{ax}\"");
        }
        if vals.iter().any(|(j, _)| *j == i) {
            s.push_str(" dataField=\"1\"");
        }
        match pt.layout {
            PivotLayout::Compact => {}
            PivotLayout::Outline => s.push_str(" compact=\"0\""),
            PivotLayout::Tabular => s.push_str(" compact=\"0\" outline=\"0\""),
        }
        if f.date_grouped() {
            s.push_str(" numFmtId=\"14\"");
        }
        let filter = hidden.get(&i).copied();
        let labels = f.item_labels();
        let is_page = axis == Some("axisPage");
        // Hidden items; never hide every item (a field needs one visible item).
        let mut hide: Vec<bool> = match filter.and_then(|f| f.selected.as_ref()) {
            Some(sel) if !(is_page && sel.len() == 1) => labels.iter().map(|l| !sel.iter().any(|x| same(x, l))).collect(),
            _ => vec![false; labels.len()],
        };
        if !hide.is_empty() && hide.iter().all(|h| *h) {
            hide.iter_mut().for_each(|h| *h = false);
        }
        if is_page && hide.iter().any(|h| *h) {
            s.push_str(" multipleItemSelectionAllowed=\"1\"");
        }
        let af_settings = axis_field(i);
        if let Some(c) = af_settings.and_then(|f| f.caption.as_deref()) {
            let _ = write!(s, " name=\"{}\"", esc_attr(c));
        }
        let _ = write!(s, " showAll=\"{}\"", u8::from(af_settings.is_some_and(|f| f.show_empty)));
        if af_settings.is_some_and(|f| f.blank_line) {
            s.push_str(" insertBlankRow=\"1\"");
        }
        if af_settings.is_some_and(|f| f.page_break) {
            s.push_str(" insertPageBreak=\"1\"");
        }
        if let Some(af) = axis_field(i) {
            match af.sort {
                PivotSort::Asc => s.push_str(" sortType=\"ascending\""),
                PivotSort::Desc => s.push_str(" sortType=\"descending\""),
                PivotSort::None => {}
            }
        }
        // Subtotals: the field's own setting (Automatic follows the PivotTable).
        let subs: Vec<PivotFunc> = match af_settings.map(|f| &f.subtotals) {
            Some(PivotSubtotals::Custom(list)) => list.clone(),
            _ => vec![],
        };
        let automatic = match af_settings.map(|f| &f.subtotals) {
            Some(PivotSubtotals::Automatic) | None => pt.subtotals,
            _ => false,
        };
        if !automatic {
            s.push_str(" defaultSubtotal=\"0\"");
        }
        for f in &subs {
            let _ = write!(s, " {}Subtotal=\"1\"", subtotal_type(*f));
        }
        if !pt.subtotals_top {
            s.push_str(" subtotalTop=\"0\"");
        }
        if axis.is_some() {
            let collapsed: &[String] = axis_field(i).map(|f| f.collapsed_items.as_slice()).unwrap_or(&[]);
            let count = labels.len() + usize::from(automatic) + subs.len();
            let _ = write!(s, "><items count=\"{count}\">");
            for (x, l) in labels.iter().enumerate() {
                let _ = write!(s, "<item x=\"{x}\"");
                if hide.get(x).copied().unwrap_or(false) {
                    s.push_str(" h=\"1\"");
                }
                if collapsed.iter().any(|cl| same(cl, l)) {
                    s.push_str(" sd=\"0\"");
                }
                s.push_str("/>");
            }
            if automatic {
                s.push_str("<item t=\"default\"/>");
            }
            for f in &subs {
                let _ = write!(s, "<item t=\"{}\"/>", subtotal_type(*f));
            }
            s.push_str("</items>");
            if af_settings.is_some_and(|f| f.repeat_labels) {
                let _ = write!(
                    s,
                    "<extLst><ext uri=\"{EXT_X14_PIVOT_FIELD}\" xmlns:x14=\"{NS_X14}\"><x14:pivotField fillDownLabels=\"1\"/></ext></extLst>"
                );
            }
            s.push_str("</pivotField>");
        } else {
            s.push_str("/>");
        }
    }
    s.push_str("</pivotFields>");

    if !row_idx.is_empty() {
        let _ = write!(s, "<rowFields count=\"{}\">", row_idx.len());
        for i in &row_idx {
            let _ = write!(s, "<field x=\"{i}\"/>");
        }
        s.push_str("</rowFields>");
    }
    let values_field = k > 1;
    if !col_idx.is_empty() || values_field {
        let _ = write!(s, "<colFields count=\"{}\">", col_idx.len() + usize::from(values_field));
        for i in &col_idx {
            let _ = write!(s, "<field x=\"{i}\"/>");
        }
        if values_field {
            s.push_str("<field x=\"-2\"/>");
        }
        s.push_str("</colFields>");
    }
    if !pages.is_empty() {
        let _ = write!(s, "<pageFields count=\"{}\">", pages.len());
        for (i, f) in &pages {
            let _ = write!(s, "<pageField fld=\"{i}\"");
            if let Some(sel) = &f.selected
                && sel.len() == 1
                && let Some(one) = sel.first()
                && let Some(x) = c.fields.get(*i).and_then(|cf| cf.item_labels().iter().position(|l| same(l, one)))
            {
                let _ = write!(s, " item=\"{x}\"");
            }
            s.push_str(" hier=\"-1\"/>");
        }
        s.push_str("</pageFields>");
    }
    if !vals.is_empty() {
        let _ = write!(s, "<dataFields count=\"{k}\">");
        let mut used: Vec<String> = Vec::new();
        for (i, v) in &vals {
            let base = if v.name.trim().is_empty() {
                format!("{} of {}", func_caption(v.func), c.fields.get(*i).map(|f| f.name.as_str()).unwrap_or(""))
            } else {
                v.name.clone()
            };
            let mut n = base.clone();
            let mut j = 2;
            while used.iter().any(|u| same(u, &n)) {
                n = format!("{base}{j}");
                j += 1;
            }
            used.push(n.clone());
            let _ = write!(s, "<dataField name=\"{}\" fld=\"{i}\"", esc_attr(&n));
            if v.func != PivotFunc::Sum {
                let _ = write!(s, " subtotal=\"{}\"", func_name(v.func));
            }
            // The base field: the named axis field, else the first row (or column) field.
            let named = v.base_field.as_deref().and_then(|b| {
                let p = pt.rows.iter().chain(pt.columns.iter()).position(|a| same(&a.source_col, b))?;
                axis_map.get(p).copied().flatten()
            });
            let base_field = named.or(row_idx.first().or(col_idx.first()).copied()).unwrap_or(0);
            let base_item = match v.base_item.as_deref() {
                Some(BASE_PREVIOUS) => 1_048_828,
                Some(BASE_NEXT) => 1_048_829,
                Some(label) => c.fields.get(base_field).and_then(|f| f.item_labels().iter().position(|l| same(l, label))).unwrap_or(0),
                None => 0,
            };
            let (data_as, x14): (Option<&str>, Option<&str>) = match v.show_as {
                PivotShowAs::Normal => (None, None),
                PivotShowAs::PercentOfGrandTotal => (Some("percentOfTotal"), None),
                PivotShowAs::PercentOfColumnTotal => (Some("percentOfCol"), None),
                PivotShowAs::PercentOfRowTotal => (Some("percentOfRow"), None),
                PivotShowAs::PercentOf => (Some("percent"), None),
                PivotShowAs::DifferenceFrom => (Some("difference"), None),
                PivotShowAs::PercentDifferenceFrom => (Some("percentDiff"), None),
                PivotShowAs::RunningTotal => (Some("runTotal"), None),
                PivotShowAs::Index => (Some("index"), None),
                PivotShowAs::PercentOfParentRowTotal => (None, Some("percentOfParentRow")),
                PivotShowAs::PercentOfParentColumnTotal => (None, Some("percentOfParentCol")),
                PivotShowAs::PercentOfParentTotal => (None, Some("percentOfParent")),
                PivotShowAs::PercentRunningTotal => (None, Some("percentOfRunningTotal")),
                PivotShowAs::RankAscending => (None, Some("rankAscending")),
                PivotShowAs::Rank => (None, Some("rankDescending")),
            };
            if let Some(a) = data_as {
                let _ = write!(s, " showDataAs=\"{a}\"");
            }
            let _ = write!(s, " baseField=\"{base_field}\" baseItem=\"{base_item}\"");
            if let Some(id) = v.number_format.as_deref().and_then(fmt_id) {
                let _ = write!(s, " numFmtId=\"{id}\"");
            }
            match x14 {
                Some(x) => {
                    let _ = write!(
                        s,
                        "><extLst><ext uri=\"{EXT_X14_DATA_FIELD}\" xmlns:x14=\"{NS_X14}\"><x14:dataField pivotShowAs=\"{x}\"/></ext></extLst></dataField>"
                    );
                }
                None => s.push_str("/>"),
            }
        }
        s.push_str("</dataFields>");
    }
    s.push_str("<pivotTableStyleInfo");
    if !pt.style.is_empty() {
        let _ = write!(s, " name=\"{}\"", esc_attr(&pt.style));
    }
    let _ = write!(
        s,
        " showRowHeaders=\"{}\" showColHeaders=\"{}\" showRowStripes=\"{}\" showColStripes=\"{}\" showLastColumn=\"1\"/>",
        u8::from(o.row_headers),
        u8::from(o.column_headers),
        u8::from(o.banded_rows),
        u8::from(o.banded_columns)
    );
    filters_xml(&mut s, pt, axis_map, &vals);
    // Excel 2010 additions: alt text and the Values row.
    if !o.alt_title.is_empty() || !o.alt_description.is_empty() || !o.show_values_row {
        let _ = write!(s, "<extLst><ext uri=\"{EXT_X14_PIVOT_TABLE}\" xmlns:x14=\"{NS_X14}\"><x14:pivotTableDefinition");
        if !o.alt_title.is_empty() {
            let _ = write!(s, " altText=\"{}\"", esc_attr(&o.alt_title));
        }
        if !o.alt_description.is_empty() {
            let _ = write!(s, " altTextSummary=\"{}\"", esc_attr(&o.alt_description));
        }
        if !o.show_values_row {
            s.push_str(" hideValuesRow=\"1\"");
        }
        s.push_str("/></ext></extLst>");
    }
    s.push_str("</pivotTableDefinition>");
    s
}

fn compare_suffix(op: PivotCompare) -> &'static str {
    match op {
        PivotCompare::Equals => "Equal",
        PivotCompare::NotEquals => "NotEqual",
        PivotCompare::BeginsWith => "BeginsWith",
        PivotCompare::NotBeginsWith => "NotBeginsWith",
        PivotCompare::EndsWith => "EndsWith",
        PivotCompare::NotEndsWith => "NotEndsWith",
        PivotCompare::Contains => "Contains",
        PivotCompare::NotContains => "NotContains",
        PivotCompare::Greater => "GreaterThan",
        PivotCompare::GreaterOrEqual => "GreaterThanOrEqual",
        PivotCompare::Less => "LessThan",
        PivotCompare::LessOrEqual => "LessThanOrEqual",
        PivotCompare::Between => "Between",
        PivotCompare::NotBetween => "NotBetween",
    }
}

const COMPARES: [PivotCompare; 14] = [
    PivotCompare::Equals,
    PivotCompare::NotEquals,
    PivotCompare::BeginsWith,
    PivotCompare::NotBeginsWith,
    PivotCompare::EndsWith,
    PivotCompare::NotEndsWith,
    PivotCompare::Contains,
    PivotCompare::NotContains,
    PivotCompare::Greater,
    PivotCompare::GreaterOrEqual,
    PivotCompare::Less,
    PivotCompare::LessOrEqual,
    PivotCompare::Between,
    PivotCompare::NotBetween,
];

/// Dynamic date periods by their filter type names.
const DATE_PERIODS: [(PivotDateRule, &str); 15] = [
    (PivotDateRule::Tomorrow, "tomorrow"),
    (PivotDateRule::Today, "today"),
    (PivotDateRule::Yesterday, "yesterday"),
    (PivotDateRule::NextWeek, "nextWeek"),
    (PivotDateRule::ThisWeek, "thisWeek"),
    (PivotDateRule::LastWeek, "lastWeek"),
    (PivotDateRule::NextMonth, "nextMonth"),
    (PivotDateRule::ThisMonth, "thisMonth"),
    (PivotDateRule::LastMonth, "lastMonth"),
    (PivotDateRule::NextQuarter, "nextQuarter"),
    (PivotDateRule::ThisQuarter, "thisQuarter"),
    (PivotDateRule::LastQuarter, "lastQuarter"),
    (PivotDateRule::NextYear, "nextYear"),
    (PivotDateRule::ThisYear, "thisYear"),
    (PivotDateRule::LastYear, "lastYear"),
];

fn date_type(rule: PivotDateRule) -> String {
    match rule {
        PivotDateRule::Equals => "dateEqual".into(),
        PivotDateRule::NotEquals => "dateNotEqual".into(),
        PivotDateRule::Before => "dateOlderThan".into(),
        PivotDateRule::After => "dateNewerThan".into(),
        PivotDateRule::Between => "dateBetween".into(),
        PivotDateRule::YearToDate => "yearToDate".into(),
        PivotDateRule::Quarter(q) => format!("Q{q}"),
        PivotDateRule::Month(m) => format!("M{m}"),
        r => DATE_PERIODS.iter().find(|(x, _)| *x == r).map(|(_, n)| (*n).to_string()).unwrap_or_else(|| "today".into()),
    }
}

fn date_rule_from(t: &str) -> Option<PivotDateRule> {
    Some(match t {
        "dateEqual" => PivotDateRule::Equals,
        "dateNotEqual" => PivotDateRule::NotEquals,
        "dateOlderThan" | "dateOlderThanOrEqual" => PivotDateRule::Before,
        "dateNewerThan" | "dateNewerThanOrEqual" => PivotDateRule::After,
        "dateBetween" => PivotDateRule::Between,
        "yearToDate" => PivotDateRule::YearToDate,
        _ => {
            if let Some(r) = DATE_PERIODS.iter().find(|(_, n)| *n == t) {
                return Some(r.0);
            }
            let n = |p: &str| t.strip_prefix(p).and_then(|d| d.parse::<u8>().ok());
            if let Some(q) = n("Q").filter(|q| (1..=4).contains(q)) {
                return Some(PivotDateRule::Quarter(q));
            }
            PivotDateRule::Month(n("M").filter(|m| (1..=12).contains(m))?)
        }
    })
}

/// The `customFilters` of an AutoFilter column for a comparison.
fn custom_filters(op: PivotCompare, a: &str, b: &str) -> String {
    let one = |oper: Option<&str>, v: &str| match oper {
        Some(o) => format!("<customFilter operator=\"{o}\" val=\"{}\"/>", esc_attr(v)),
        None => format!("<customFilter val=\"{}\"/>", esc_attr(v)),
    };
    let (and, parts) = match op {
        PivotCompare::Equals => (false, vec![one(None, a)]),
        PivotCompare::NotEquals => (false, vec![one(Some("notEqual"), a)]),
        PivotCompare::BeginsWith => (false, vec![one(None, &format!("{a}*"))]),
        PivotCompare::NotBeginsWith => (false, vec![one(Some("notEqual"), &format!("{a}*"))]),
        PivotCompare::EndsWith => (false, vec![one(None, &format!("*{a}"))]),
        PivotCompare::NotEndsWith => (false, vec![one(Some("notEqual"), &format!("*{a}"))]),
        PivotCompare::Contains => (false, vec![one(None, &format!("*{a}*"))]),
        PivotCompare::NotContains => (false, vec![one(Some("notEqual"), &format!("*{a}*"))]),
        PivotCompare::Greater => (false, vec![one(Some("greaterThan"), a)]),
        PivotCompare::GreaterOrEqual => (false, vec![one(Some("greaterThanOrEqual"), a)]),
        PivotCompare::Less => (false, vec![one(Some("lessThan"), a)]),
        PivotCompare::LessOrEqual => (false, vec![one(Some("lessThanOrEqual"), a)]),
        PivotCompare::Between => (true, vec![one(Some("greaterThanOrEqual"), a), one(Some("lessThanOrEqual"), b)]),
        PivotCompare::NotBetween => (false, vec![one(Some("lessThan"), a), one(Some("greaterThan"), b)]),
    };
    format!("<customFilters{}>{}</customFilters>", if and { " and=\"1\"" } else { "" }, parts.concat())
}

/// `<filters>`: label, date, value and Top 10 filters on axis fields.
fn filters_xml(s: &mut String, pt: &PivotTable, axis_map: &[Option<usize>], vals: &[(usize, &PivotValue)]) {
    let measure = |name: &str| vals.iter().position(|(_, v)| same(&v.name, name)).unwrap_or(0);
    let mut out = String::new();
    let mut n = 0;
    for f in &pt.filters {
        let Some(pos) = pt.rows.iter().chain(pt.columns.iter()).position(|a| same(&a.source_col, &f.source_col)) else { continue };
        let Some(Some(fld)) = axis_map.get(pos) else { continue };
        for rule in f.label.iter().chain(f.value.iter()) {
            let (ty, extra, inner) = match rule {
                PivotRule::Label { op, value, value2 } => {
                    let mut extra = format!(" stringValue1=\"{}\"", esc_attr(value));
                    if matches!(op, PivotCompare::Between | PivotCompare::NotBetween) {
                        let _ = write!(extra, " stringValue2=\"{}\"", esc_attr(value2));
                    }
                    (format!("caption{}", compare_suffix(*op)), extra, custom_filters(*op, value, value2))
                }
                PivotRule::Value { op, field, value, value2 } => (
                    format!("value{}", compare_suffix(*op)),
                    format!(" iMeasureFld=\"{}\"", measure(field)),
                    custom_filters(*op, &crate::xml::num(*value), &crate::xml::num(*value2)),
                ),
                PivotRule::Top { top, count, by, field } => {
                    let ty = match by {
                        PivotTopBy::Items => "count",
                        PivotTopBy::Percent => "percent",
                        PivotTopBy::Sum => "sum",
                    };
                    let inner = format!(
                        "<top10 top=\"{}\" percent=\"{}\" val=\"{}\"/>",
                        u8::from(*top),
                        u8::from(*by == PivotTopBy::Percent),
                        crate::xml::num(*count)
                    );
                    (ty.to_string(), format!(" iMeasureFld=\"{}\"", measure(field)), inner)
                }
                PivotRule::Date { rule, value, value2 } => {
                    let v1 = value.map(crate::xml::num).unwrap_or_default();
                    let v2 = value2.map(crate::xml::num).unwrap_or_default();
                    let inner = match rule {
                        PivotDateRule::Equals => custom_filters(PivotCompare::Equals, &v1, ""),
                        PivotDateRule::NotEquals => custom_filters(PivotCompare::NotEquals, &v1, ""),
                        PivotDateRule::Before => custom_filters(PivotCompare::Less, &v1, ""),
                        PivotDateRule::After => custom_filters(PivotCompare::Greater, &v1, ""),
                        PivotDateRule::Between => custom_filters(PivotCompare::Between, &v1, &v2),
                        r => format!("<dynamicFilter type=\"{}\"/>", date_type(*r)),
                    };
                    let mut extra = String::new();
                    if value.is_some() {
                        let _ = write!(extra, " stringValue1=\"{v1}\"");
                    }
                    if value2.is_some() {
                        let _ = write!(extra, " stringValue2=\"{v2}\"");
                    }
                    (date_type(*rule), extra, inner)
                }
            };
            n += 1;
            let _ = write!(
                out,
                "<filter fld=\"{fld}\" type=\"{ty}\" evalOrder=\"-1\" id=\"{n}\"{extra}><autoFilter ref=\"A1\"><filterColumn colId=\"0\">{inner}</filterColumn></autoFilter></filter>"
            );
        }
    }
    if n > 0 {
        let _ = write!(s, "<filters count=\"{n}\">{out}</filters>");
    }
}

/// Reads a `<filter>` into a rule; `measure` names a data field by index.
fn read_filter(e: &El, measure: &dyn Fn(u32) -> Option<String>) -> Option<PivotRule> {
    let ty = e.attr("type")?;
    let col = e.child("autoFilter").and_then(|a| a.child("filterColumn"));
    let nums: Vec<f64> = col
        .and_then(|c| c.child("customFilters"))
        .map(|c| c.kids("customFilter").filter_map(|x| x.attr("val").and_then(|v| v.parse::<f64>().ok())).collect())
        .unwrap_or_default();
    let s1 = e.attr("stringValue1").unwrap_or("").to_string();
    let s2 = e.attr("stringValue2").unwrap_or("").to_string();
    let compare = |suffix: &str| COMPARES.iter().copied().find(|c| compare_suffix(*c) == suffix);
    if let Some(op) = ty.strip_prefix("caption").and_then(compare) {
        return Some(PivotRule::Label { op, value: s1, value2: s2 });
    }
    let field = || e.attr_u32("iMeasureFld").and_then(measure).unwrap_or_default();
    if let Some(op) = ty.strip_prefix("value").and_then(compare) {
        let n = |k: usize, s: &str| nums.get(k).copied().or_else(|| s.parse::<f64>().ok()).unwrap_or(0.0);
        return Some(PivotRule::Value { op, field: field(), value: n(0, &s1), value2: n(1, &s2) });
    }
    let by = match ty {
        "count" => Some(PivotTopBy::Items),
        "percent" => Some(PivotTopBy::Percent),
        "sum" => Some(PivotTopBy::Sum),
        _ => None,
    };
    if let Some(by) = by {
        let t = col.and_then(|c| c.child("top10"));
        let count = t.and_then(|t| t.attr("val")).and_then(|v| v.parse::<f64>().ok()).unwrap_or(10.0);
        let top = t.is_none_or(|t| t.flag("top", true));
        return Some(PivotRule::Top { top, count, by, field: field() });
    }
    let rule = date_rule_from(ty)?;
    let n = |k: usize, s: &str| s.parse::<f64>().ok().or_else(|| nums.get(k).copied());
    Some(PivotRule::Date { rule, value: n(0, &s1), value2: n(1, &s2) })
}

// ================================================================ reading

/// A cache field as read.
#[derive(Clone, Debug, Default)]
struct FieldIn {
    name: String,
    database: bool,
    /// Shared item captions.
    items: Vec<String>,
    group: Option<GroupIn>,
    formula: bool,
}

/// A cache field's `fieldGroup`.
#[derive(Clone, Debug, Default)]
struct GroupIn {
    base: Option<usize>,
    /// Date part; `None` with neither `range` nor `discrete` = a grouping we don't know.
    by: Option<PivotDateGroup>,
    items: Vec<String>,
    /// Number range: (start, end, step).
    range: Option<(f64, f64, f64)>,
    days: u32,
    /// User-set bounds (autoStart / autoEnd off), as date serials.
    start: Option<f64>,
    end: Option<f64>,
    /// Manual grouping: base shared item → group item.
    discrete: Option<Vec<u32>>,
}

impl FieldIn {
    fn labels(&self) -> &[String] {
        match &self.group {
            Some(g) if !g.items.is_empty() => &g.items,
            _ => &self.items,
        }
    }
}

/// What a cache field means for a PivotTable field.
#[derive(Clone, Debug, Default)]
struct FieldRef {
    name: String,
    date: PivotDateGroup,
    days: u32,
    start: Option<f64>,
    end: Option<f64>,
    number: Option<PivotNumberGroup>,
    manual: Option<PivotManualGroup>,
}

impl FieldRef {
    fn apply(&self, f: &mut PivotField) {
        f.date_group = self.date;
        f.group_days = self.days;
        f.group_start = self.start;
        f.group_end = self.end;
        f.number_group = self.number;
    }
}

#[derive(Clone, Debug, Default)]
struct CacheIn {
    source: String,
    fields: Vec<FieldIn>,
}

/// Workbook-level pivot cache lookup, filled from `<pivotCaches>`; caches are parsed once.
#[derive(Default)]
pub struct CachesIn {
    by_id: HashMap<u32, String>,
    parsed: HashMap<String, Option<CacheIn>>,
    next_id: u32,
}

impl CachesIn {
    pub fn new(wb_xml: &El, wb_rels: &[crate::package::Rel]) -> CachesIn {
        let mut by_id = HashMap::new();
        if let Some(pc) = wb_xml.child("pivotCaches") {
            for c in pc.kids("pivotCache") {
                if let (Some(id), Some(rid)) = (c.attr_u32("cacheId"), c.attr("id"))
                    && let Some(r) = wb_rels.iter().find(|r| r.id == rid)
                {
                    by_id.insert(id, r.target.clone());
                }
            }
        }
        CachesIn { by_id, parsed: HashMap::new(), next_id: 1 }
    }
}

fn item_label_el(e: &El, sys: DateSystem) -> String {
    let v = e.attr("v").unwrap_or("");
    match e.name.as_str() {
        "m" => "(blank)".into(),
        "n" => v.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(number_to_text).unwrap_or_else(|| v.to_string()),
        "b" => if matches!(v.trim(), "1" | "true") { "TRUE" } else { "FALSE" }.into(),
        "d" => crate::sheet_read::parse_iso_datetime(v, sys).map(number_to_text).unwrap_or_else(|| v.to_string()),
        _ => v.to_string(),
    }
}

fn read_cache(cx: &mut Ctx<'_>, part: &str, sheet_name: &str) -> Option<CacheIn> {
    let x = cx.optional_xml(part).ok().flatten()?;
    if x.name != "pivotCacheDefinition" {
        cx.warn(format!("{part} is not a pivot cache definition; its PivotTable was kept as plain cells"));
        return None;
    }
    let src = x.child("cacheSource");
    let ty = src.and_then(|s| s.attr("type")).unwrap_or("worksheet");
    if ty != "worksheet" {
        cx.warn(format!("PivotTables with a {ty} data source are not supported; kept as plain cells"));
        return None;
    }
    let ws = src.and_then(|s| s.child("worksheetSource"));
    let source = match ws {
        Some(w) if w.attr("name").is_some_and(|n| !n.trim().is_empty()) => w.attr("name").unwrap_or("").trim().to_string(),
        Some(w) => {
            if w.attr("id").is_some() {
                cx.warn("a PivotTable's source is in another workbook; its source reference may not resolve");
            }
            let r = w.attr("ref").map(|r| r.replace('$', "")).and_then(|r| RangeRef::parse(&r));
            let Some(r) = r else {
                cx.warn("a PivotTable's source reference is missing or invalid; kept as plain cells");
                return None;
            };
            let sheet = w.attr("sheet").filter(|s| !s.is_empty()).unwrap_or(sheet_name);
            format!("{}!{}", quote_sheet(sheet), abs_range(&r))
        }
        None => {
            cx.warn("a PivotTable's cache has no worksheet source; kept as plain cells");
            return None;
        }
    };
    let sys = cx.date_system;
    let mut fields = Vec::new();
    if let Some(cf) = x.child("cacheFields") {
        for f in cf.kids("cacheField").take(MAX_FIELDS) {
            let items = f.child("sharedItems").map(|si| si.children.iter().map(|e| item_label_el(e, sys)).collect()).unwrap_or_default();
            let group = f.child("fieldGroup").map(|g| {
                let base = g.attr_u32("base").map(|b| b as usize);
                let items = g.child("groupItems").map(|gi| gi.children.iter().map(|e| item_label_el(e, sys)).collect()).unwrap_or_default();
                let mut gin = GroupIn { base, items, ..GroupIn::default() };
                if let Some(rp) = g.child("rangePr") {
                    let fixed = !rp.flag("autoStart", true) || !rp.flag("autoEnd", true);
                    let interval = rp.attr("groupInterval").and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
                    match rp.attr("groupBy").unwrap_or("range") {
                        "range" => {
                            let n = |k: &str| rp.attr(k).and_then(|v| v.parse::<f64>().ok());
                            if let (Some(a), Some(b)) = (n("startNum"), n("endNum")) {
                                gin.range = Some((a, b, interval));
                            }
                        }
                        other => {
                            gin.by = group_from_name(other);
                            let date = |k: &str| rp.attr(k).and_then(|v| crate::sheet_read::parse_iso_datetime(v, sys));
                            if fixed {
                                gin.start = date("startDate");
                                gin.end = date("endDate");
                            }
                            if gin.by == Some(PivotDateGroup::Days) && interval > 1.0 {
                                gin.days = interval as u32;
                            }
                        }
                    }
                }
                if let Some(dp) = g.child("discretePr") {
                    gin.discrete = Some(dp.children.iter().filter_map(|x| x.attr_u32("v")).collect());
                }
                gin
            });
            fields.push(FieldIn {
                name: f.attr("name").unwrap_or("").to_string(),
                database: f.flag("databaseField", true),
                items,
                group,
                formula: f.attr("formula").is_some(),
            });
        }
    }
    if fields.is_empty() {
        cx.warn("a PivotTable's cache has no fields; kept as plain cells");
        return None;
    }
    Some(CacheIn { source, fields })
}

/// Reads the PivotTables related to a worksheet part.
pub fn read_pivots(cx: &mut Ctx<'_>, caches: &mut CachesIn, sheet_part: &str, sheet_name: &str) -> Result<Vec<PivotTable>, IoError> {
    let rels = cx.pkg.rels(sheet_part)?;
    let mut out = Vec::new();
    for rel in rels.iter().filter(|r| r.kind == "pivotTable" && !r.external) {
        let Some(x) = cx.optional_xml(&rel.target)? else { continue };
        if x.name != "pivotTableDefinition" {
            cx.warn(format!("{} is not a PivotTable definition; skipped", rel.target));
            continue;
        }
        // The cache: the table's own relationship, else the workbook's cache id.
        let trels = cx.pkg.rels(&rel.target)?;
        let cache_part = trels
            .iter()
            .find(|r| r.kind == "pivotCacheDefinition" && !r.external)
            .map(|r| r.target.clone())
            .or_else(|| x.attr_u32("cacheId").and_then(|id| caches.by_id.get(&id).cloned()));
        let Some(cache_part) = cache_part else {
            cx.warn(format!("PivotTable in {} has no cache; kept as plain cells", rel.target));
            continue;
        };
        let cache = match caches.parsed.get(&cache_part) {
            Some(c) => c.clone(),
            None => {
                let c = read_cache(cx, &cache_part, sheet_name);
                caches.parsed.insert(cache_part.clone(), c.clone());
                c
            }
        };
        let Some(cache) = cache else { continue };
        if let Some(pt) = read_table(cx, &x, &cache, caches.next_id) {
            caches.next_id = caches.next_id.saturating_add(1);
            out.push(pt);
        }
    }
    Ok(out)
}

/// A pivot field's data items (not subtotal or grand-total items).
fn items_of(e: &El) -> Vec<&El> {
    e.child("items").map(|it| it.kids("item").filter(|i| i.attr("t").is_none_or(|t| t == "data")).collect()).unwrap_or_default()
}

/// (source column name, date grouping) of a cache field used on an axis.
fn field_ref(cx: &mut Ctx<'_>, cache: &CacheIn, i: usize) -> Option<FieldRef> {
    let f = cache.fields.get(i)?;
    if f.formula {
        cx.warn("calculated PivotTable fields are not supported");
        return None;
    }
    let Some(g) = &f.group else {
        if !f.database {
            cx.warn(format!("PivotTable field \"{}\" (grouped items) is not supported", f.name));
            return None;
        }
        return Some(FieldRef { name: f.name.clone(), ..FieldRef::default() });
    };
    let base = g.base.and_then(|b| cache.fields.get(b));
    // Manual groups: a field of its own whose items group the base field's items.
    if let Some(map) = &g.discrete {
        let base = base?;
        let mut groups: Vec<PivotNamedGroup> = Vec::new();
        for (k, gi) in map.iter().enumerate() {
            let (Some(item), Some(group)) = (base.items.get(k), g.items.get(*gi as usize)) else { continue };
            if same(item, group) {
                continue;
            }
            match groups.iter_mut().find(|x| same(&x.name, group)) {
                Some(x) => x.items.push(item.clone()),
                None => groups.push(PivotNamedGroup { name: group.clone(), items: vec![item.clone()] }),
            }
        }
        let manual = PivotManualGroup { name: f.name.clone(), base: base.name.clone(), groups };
        return Some(FieldRef { name: f.name.clone(), manual: Some(manual), ..FieldRef::default() });
    }
    let name = if f.database { f.name.clone() } else { base?.name.clone() };
    if let Some((start, end, by)) = g.range {
        return Some(FieldRef { name, number: Some(PivotNumberGroup { start, end, by }), ..FieldRef::default() });
    }
    let Some(date) = g.by else {
        cx.warn(format!("PivotTable grouping of \"{}\" is not supported; the field is ungrouped", f.name));
        return Some(FieldRef { name, ..FieldRef::default() });
    };
    Some(FieldRef { name, date, days: g.days, start: g.start, end: g.end, ..FieldRef::default() })
}

fn read_table(cx: &mut Ctx<'_>, x: &El, cache: &CacheIn, id: u32) -> Option<PivotTable> {
    let mut pt = PivotTable { id, ..PivotTable::default() };
    if let Some(n) = x.attr("name").filter(|n| !n.trim().is_empty()) {
        pt.name = n.to_string();
    } else {
        pt.name = format!("PivotTable{id}");
    }
    pt.source = cache.source.clone();
    let Some(loc) = x.child("location").and_then(|l| l.attr("ref")).and_then(|r| RangeRef::parse(&r.replace('$', ""))) else {
        cx.warn(format!("PivotTable \"{}\" has no valid location; kept as plain cells", pt.name));
        return None;
    };
    let pfs: Vec<&El> = x.child("pivotFields").map(|p| p.kids("pivotField").collect()).unwrap_or_default();
    let pf = |i: usize| pfs.get(i).copied();
    let idx_list = |name: &str, attr: &str| -> Vec<i64> {
        x.child(name).map(|e| e.children.iter().filter_map(|f| f.attr_i64(attr)).collect()).unwrap_or_default()
    };
    let row_x = idx_list("rowFields", "x");
    let col_x = idx_list("colFields", "x");
    let pages: Vec<&El> = x.child("pageFields").map(|p| p.kids("pageField").collect()).unwrap_or_default();

    // Layout from the first axis field (pivotField defaults: compact and outline on).
    let first_axis = row_x.iter().chain(col_x.iter()).find(|i| **i >= 0).and_then(|i| pf(*i as usize));
    let (compact, outline) = match first_axis {
        Some(f) => (f.flag("compact", true), f.flag("outline", true)),
        None => (x.flag("compact", true), x.flag("outline", false)),
    };
    pt.layout = if compact {
        PivotLayout::Compact
    } else if outline {
        PivotLayout::Outline
    } else {
        PivotLayout::Tabular
    };
    if let Some(f) = first_axis {
        pt.subtotals = f.flag("defaultSubtotal", true);
        pt.subtotals_top = f.flag("subtotalTop", true);
    }
    // PivotTable Options.
    let o = &mut pt.options;
    o.indent = x.attr_u32("indent").map(|v| v.min(127) as u8).unwrap_or(1);
    o.autofit = x.flag("useAutoFormatting", false);
    o.repeat_row_labels = x.flag("itemPrintTitles", false);
    o.print_titles = x.flag("fieldPrintTitles", false);
    o.print_buttons = x.flag("printDrill", false);
    o.merge_labels = x.flag("mergeItem", false);
    o.filters_over_then_down = x.flag("pageOverThenDown", false);
    o.filters_per_column = x.attr_u32("pageWrap").unwrap_or(0);
    o.error_text = x.flag("showError", false).then(|| x.attr("errorCaption").unwrap_or("").to_string());
    o.empty_text = x.attr("missingCaption").map(str::to_string);
    o.preserve_formatting = x.flag("preserveFormatting", true);
    o.custom_list_sort = x.flag("customListSort", true);
    o.expand_buttons = x.flag("showDrill", true);
    o.contextual_tooltips = x.flag("showDataTips", true);
    o.classic_layout = x.flag("gridDropZones", false);
    o.empty_rows = x.flag("showEmptyRow", false);
    o.empty_columns = x.flag("showEmptyCol", false);
    o.field_list_az = x.flag("fieldListSortAscending", false);
    o.enable_drilldown = x.flag("enableDrill", true);
    o.multiple_filters = x.flag("multipleFieldFilters", true);
    o.row_header_caption = x.attr("rowHeaderCaption").map(str::to_string);
    o.column_header_caption = x.attr("colHeaderCaption").map(str::to_string);
    if let Some(si) = x.child("pivotTableStyleInfo") {
        o.row_headers = si.flag("showRowHeaders", true);
        o.column_headers = si.flag("showColHeaders", true);
        o.banded_rows = si.flag("showRowStripes", false);
        o.banded_columns = si.flag("showColStripes", false);
    }
    if let Some(ext) = x14_ext(x, "pivotTableDefinition") {
        o.alt_title = ext.attr("altText").unwrap_or("").to_string();
        o.alt_description = ext.attr("altTextSummary").unwrap_or("").to_string();
        o.show_values_row = !ext.flag("hideValuesRow", false);
    }
    pt.grand_totals_rows = x.flag("rowGrandTotals", true);
    pt.grand_totals_cols = x.flag("colGrandTotals", true);
    pt.show_headers = x.flag("showHeaders", true);
    pt.style = x.child("pivotTableStyleInfo").and_then(|s| s.attr("name")).unwrap_or("").to_string();
    if x.flag("dataOnRows", false) {
        cx.warn(format!("PivotTable \"{}\": values on rows are shown on columns", pt.name));
    }
    if x.child("calculatedItems").is_some() {
        cx.warn(format!("PivotTable \"{}\": calculated items are not supported", pt.name));
    }

    let mut filters: Vec<PivotFilter> = Vec::new();

    let axis = |cx: &mut Ctx<'_>, list: &[i64], filters: &mut Vec<PivotFilter>| -> Vec<PivotField> {
        let mut v = Vec::new();
        for &i in list {
            if i < 0 {
                continue;
            }
            let i = i as usize;
            let Some(fr) = field_ref(cx, cache, i) else { continue };
            let name = fr.name.clone();
            let labels = cache.fields.get(i).map(|f| f.labels().to_vec()).unwrap_or_default();
            let mut f = PivotField { source_col: name.clone(), sort: PivotSort::None, ..PivotField::default() };
            fr.apply(&mut f);
            if let Some(e) = pf(i) {
                f.caption = e.attr("name").map(str::to_string).filter(|c| !c.is_empty());
                f.show_empty = e.flag("showAll", true);
                f.blank_line = e.flag("insertBlankRow", false);
                f.page_break = e.flag("insertPageBreak", false);
                f.repeat_labels = x14_ext(e, "pivotField").is_some_and(|x| x.flag("fillDownLabels", false));
                let custom: Vec<PivotFunc> = [
                    PivotFunc::Sum,
                    PivotFunc::Count,
                    PivotFunc::Average,
                    PivotFunc::Max,
                    PivotFunc::Min,
                    PivotFunc::Product,
                    PivotFunc::CountNumbers,
                    PivotFunc::StdDev,
                    PivotFunc::StdDevP,
                    PivotFunc::Var,
                    PivotFunc::VarP,
                ]
                .into_iter()
                .filter(|func| e.flag(&format!("{}Subtotal", subtotal_type(*func)), false))
                .collect();
                f.subtotals = if !custom.is_empty() {
                    PivotSubtotals::Custom(custom)
                } else if e.flag("defaultSubtotal", true) {
                    PivotSubtotals::Automatic
                } else {
                    PivotSubtotals::None
                };
                f.sort = match e.attr("sortType") {
                    Some("ascending") => PivotSort::Asc,
                    Some("descending") => PivotSort::Desc,
                    _ => PivotSort::None,
                };
                let items = items_of(e);
                let lab = |it: &El| it.attr_u32("x").and_then(|k| labels.get(k as usize)).cloned();
                f.collapsed_items = items.iter().filter(|it| !it.flag("sd", true)).filter_map(|it| lab(it)).collect();
                if items.iter().any(|it| it.flag("h", false)) {
                    let shown: Vec<String> = items.iter().filter(|it| !it.flag("h", false)).filter_map(|it| lab(it)).collect();
                    if !filters.iter().any(|ff| same(&ff.source_col, &name)) {
                        filters.push(PivotFilter { source_col: name.clone(), selected: Some(shown), ..PivotFilter::default() });
                    }
                }
            }
            v.push(f);
        }
        v
    };
    let mut page_filters = Vec::new();
    for p in &pages {
        let Some(i) = p.attr_i64("fld").filter(|i| *i >= 0).map(|i| i as usize) else { continue };
        let Some(name) = field_ref(cx, cache, i).map(|r| r.name) else { continue };
        let labels = cache.fields.get(i).map(|f| f.labels().to_vec()).unwrap_or_default();
        let selected = if let Some(k) = p.attr_u32("item") {
            labels.get(k as usize).map(|l| vec![l.clone()])
        } else {
            pf(i).and_then(|e| {
                let items = items_of(e);
                items.iter().any(|it| it.flag("h", false)).then(|| {
                    items
                        .iter()
                        .filter(|it| !it.flag("h", false))
                        .filter_map(|it| it.attr_u32("x").and_then(|k| labels.get(k as usize)).cloned())
                        .collect()
                })
            })
        };
        page_filters.push(PivotFilter { source_col: name, selected, ..PivotFilter::default() });
    }
    let mut axis_filters = Vec::new();
    pt.rows = axis(cx, &row_x, &mut axis_filters);
    pt.columns = axis(cx, &col_x, &mut axis_filters);
    filters.extend(page_filters);
    filters.extend(axis_filters);
    pt.filters = filters;

    // Values.
    if let Some(df) = x.child("dataFields") {
        for d in df.kids("dataField") {
            let Some(i) = d.attr_u32("fld").map(|i| i as usize) else { continue };
            let Some(f) = cache.fields.get(i) else { continue };
            if f.formula {
                cx.warn("calculated PivotTable fields are not supported");
                continue;
            }
            let func = match d.attr("subtotal") {
                None => PivotFunc::Sum,
                Some(s) => func_from(s).unwrap_or_else(|| {
                    cx.warn(format!("PivotTable function {s} is not supported; using Sum"));
                    PivotFunc::Sum
                }),
            };
            let ext = d
                .child("extLst")
                .into_iter()
                .flat_map(|l| l.kids("ext"))
                .filter_map(|e| e.child("dataField"))
                .find_map(|e| e.attr("pivotShowAs").map(str::to_string));
            let show_as = match (ext.as_deref(), d.attr("showDataAs")) {
                (Some("rankDescending"), _) => PivotShowAs::Rank,
                (Some("rankAscending"), _) => PivotShowAs::RankAscending,
                (Some("percentOfParentRow"), _) => PivotShowAs::PercentOfParentRowTotal,
                (Some("percentOfParentCol"), _) => PivotShowAs::PercentOfParentColumnTotal,
                (Some("percentOfParent"), _) => PivotShowAs::PercentOfParentTotal,
                (Some("percentOfRunningTotal"), _) => PivotShowAs::PercentRunningTotal,
                (Some(other), _) => {
                    cx.warn(format!("PivotTable show-values-as {other} is not supported"));
                    PivotShowAs::Normal
                }
                (None, None | Some("normal")) => PivotShowAs::Normal,
                (None, Some("percentOfTotal")) => PivotShowAs::PercentOfGrandTotal,
                (None, Some("percentOfCol")) => PivotShowAs::PercentOfColumnTotal,
                (None, Some("percentOfRow")) => PivotShowAs::PercentOfRowTotal,
                (None, Some("percent")) => PivotShowAs::PercentOf,
                (None, Some("difference")) => PivotShowAs::DifferenceFrom,
                (None, Some("percentDiff")) => PivotShowAs::PercentDifferenceFrom,
                (None, Some("runTotal")) => PivotShowAs::RunningTotal,
                (None, Some("index")) => PivotShowAs::Index,
                (None, Some(other)) => {
                    cx.warn(format!("PivotTable show-values-as {other} is not supported"));
                    PivotShowAs::Normal
                }
            };
            // Base field and item, where the transform uses them.
            let (mut base_field, mut base_item) = (None, None);
            if show_as.needs_base_field()
                && let Some(bf) = d.attr_u32("baseField").map(|b| b as usize)
                && let Some(name) = field_ref(cx, cache, bf).map(|r| r.name)
            {
                base_field = Some(name);
                if show_as.needs_base_item() {
                    base_item = match d.attr_u32("baseItem") {
                        Some(1_048_828) => Some(BASE_PREVIOUS.to_string()),
                        Some(1_048_829) => Some(BASE_NEXT.to_string()),
                        Some(k) => cache.fields.get(bf).and_then(|f| f.labels().get(k as usize).cloned()),
                        None => None,
                    };
                }
            }
            let number_format = d
                .attr_u32("numFmtId")
                .filter(|id| *id != 0)
                .and_then(|id| crate::tables::builtin_format(id).map(str::to_string).or_else(|| cx.num_fmts.get(&id).cloned()));
            let name = d.attr("name").map(str::to_string).unwrap_or_else(|| format!("{} of {}", func_caption(func), f.name));
            pt.values.push(PivotValue { source_col: f.name.clone(), func, name, show_as, number_format, base_field, base_item });
        }
    }

    // Label, date, value and Top 10 filters.
    if let Some(fs) = x.child("filters") {
        let names: Vec<String> = pt.values.iter().map(|v| v.name.clone()).collect();
        let measure = |i: u32| names.get(i as usize).cloned();
        for e in fs.kids("filter").take(1000) {
            let Some(fld) = e.attr_u32("fld").map(|i| i as usize) else { continue };
            let Some(name) = field_ref(cx, cache, fld).map(|r| r.name) else { continue };
            let Some(rule) = read_filter(e, &measure) else {
                cx.warn(format!("PivotTable filter type \"{}\" is not supported", e.attr("type").unwrap_or("")));
                continue;
            };
            let at = match pt.filters.iter().position(|f| same(&f.source_col, &name)) {
                Some(i) => i,
                None => {
                    pt.filters.push(PivotFilter { source_col: name, ..PivotFilter::default() });
                    pt.filters.len() - 1
                }
            };
            if let Some(f) = pt.filters.get_mut(at) {
                match rule {
                    PivotRule::Label { .. } | PivotRule::Date { .. } => f.label = Some(rule),
                    _ => f.value = Some(rule),
                }
            }
        }
    }
    // Manual group fields used by the PivotTable.
    let used: Vec<usize> = row_x
        .iter()
        .chain(col_x.iter())
        .filter(|i| **i >= 0)
        .map(|i| *i as usize)
        .chain(pages.iter().filter_map(|p| p.attr_u32("fld").map(|v| v as usize)))
        .collect();
    for i in used {
        if cache.fields.get(i).and_then(|f| f.group.as_ref()).is_some_and(|g| g.discrete.is_some())
            && let Some(mg) = field_ref(cx, cache, i).and_then(|r| r.manual)
            && !pt.manual_groups.iter().any(|m| same(&m.name, &mg.name))
        {
            pt.manual_groups.push(mg);
        }
    }
    // With subtotals off for the whole PivotTable, a field without subtotals is just following it.
    if !pt.subtotals {
        for f in pt.rows.iter_mut().chain(pt.columns.iter_mut()) {
            if f.subtotals == PivotSubtotals::None {
                f.subtotals = PivotSubtotals::Automatic;
            }
        }
    }
    pt.anchor = loc.start;
    let above = if pages.is_empty() { 0 } else { pages.len() as u32 + 1 };
    pt.last_range = Some(RangeRef::new(CellRef::new(loc.start.row.saturating_sub(above), loc.start.col), loc.end));
    Some(pt)
}
