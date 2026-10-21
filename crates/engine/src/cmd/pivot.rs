//! PivotTable commands (Insert › PivotTable, PivotTable Analyze, PivotTable Design, Data ›
//! Refresh All). Every change re-renders the report as one undo step.
//!
//! `pivot?` parameters name a PivotTable (name or numeric id); by default it is the pivot whose
//! report contains the active cell, else the last pivot on the active sheet.

use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef};
use gridcraft_model::*;
use serde_json::{Value as Json, json};

use super::*;
use crate::pivot as pv;
use crate::selection::current_region;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "insert.pivotTable",
            "PivotTable",
            ["Insert", "Tables"],
            None,
            "{source?: \"Sheet1!$A$1:$E$200\" | table name (default: the table or current region of the selection), destination?: \"new\" (default: new sheet before the current one, report at A3) | \"Sheet1!H3\", name?, rows?: [field | {field, group?: years|quarters|months|days}], columns?: [..], values?: [field | {field, func?, name?}], filters?: [field], replace?: bool} → {id, name, sheet, anchor, fields}",
            has_doc,
            insert_pivot
        ),
        cmd!(
            "insert.recommendedPivotTables",
            "Recommended PivotTables",
            ["Insert", "Tables"],
            None,
            "{source?, apply?: index, destination?} → suggestions [{index, title, rows, columns, values}]; with apply, creates that one",
            has_doc,
            recommended
        ),
        cmd!(
            "table.summarizePivot",
            "Summarize with PivotTable",
            ["Table Design", "Tools"],
            None,
            "{table?, destination?} creates a PivotTable from the table",
            has_doc,
            summarize_table
        ),
        cmd!(
            "pivot.calculatedField",
            "Calculated Field",
            ["PivotTable Analyze", "Calculations"],
            None,
            "{pivot?, name, formula?: \"=Sales*0.1\" (fields bare or 'quoted'; evaluated on each cell's sums), delete?: bool, values?: bool (add a new field to Values, default true)} adds, changes or deletes a calculated field; without `name` opens the dialog → {name, formula, calculatedFields}",
            has_doc,
            calculated_field
        ),
        cmd!(
            "insert.pivotChart",
            "PivotChart",
            ["Insert", "Charts"],
            None,
            "{pivot?: name | id | a cell of the report (\"Sheet2!B5\"; default: the PivotTable at the active cell), type?: column|bar|line|pie|area|… (default column), subtype?, title?, at?: \"H2\", width?, height?} adds a chart bound to the PivotTable's report: categories are the innermost row items, series the column items × value fields; it follows refreshes and layout changes → {chart, pivot, series}",
            has_doc,
            insert_pivot_chart
        ),
        cmd!(query "pivot.fields", "PivotTable Fields", [], None, "{pivot?} → source fields (name, numeric, date, samples, areas) and the current layout", has_doc, fields),
        cmd!(query "pivot.fieldList", "Field List", ["PivotTable Analyze", "Show"], None, "{pivot?} (same as pivot.fields)", has_doc, fields),
        cmd!(query "pivot.list", "PivotTables", [], None, "{} → every PivotTable in the workbook", has_doc, list),
        cmd!(
            "pivot.addField",
            "Add Field",
            [],
            None,
            "{pivot?, field, area: rows|columns|values|filters, func?: sum|count|average|max|min|product|countNumbers|stdDev|stdDevP|var|varP, name?, position?: index, replace?}",
            has_doc,
            add_field
        ),
        cmd!(
            "pivot.removeField",
            "Remove Field",
            [],
            None,
            "{pivot?, field (or value caption), area?: rows|columns|values|filters, replace?}",
            has_doc,
            remove_field
        ),
        cmd!(
            "pivot.moveField",
            "Move Field",
            [],
            None,
            "{pivot?, field, from: rows|columns|values|filters, to: rows|columns|values|filters, position?: index, replace?}",
            has_doc,
            move_field
        ),
        cmd!(
            "pivot.valueSettings",
            "Value Field Settings",
            ["PivotTable Analyze", "Active Field"],
            None,
            "{pivot?, field: caption or source field | index: n, func?, name?, showAs?: normal|percentOfGrandTotal|percentOfColumnTotal|percentOfRowTotal|percentOf|percentOfParentRowTotal|percentOfParentColumnTotal|percentOfParentTotal|differenceFrom|percentDifferenceFrom|runningTotal|percentRunningTotal|rankAscending|rankDescending|index, baseField?: row or column field, baseItem?: item label | \"(previous)\" | \"(next)\", numberFormat?: \"#,##0\" | null, replace?}",
            has_doc,
            value_settings
        ),
        cmd!(
            "pivot.filter",
            "Filter Items",
            [],
            None,
            "{pivot?, field, selected: [\"East\", ..] | null (all), replace?} (clears the field's label, value or Top 10 filter)",
            has_doc,
            filter
        ),
        cmd!(
            "pivot.labelFilter",
            "Label Filter",
            [],
            None,
            "{pivot?, field: row or column field, op: equals|doesNotEqual|beginsWith|doesNotBeginWith|endsWith|doesNotEndWith|contains|doesNotContain|greaterThan|greaterThanOrEqual|lessThan|lessThanOrEqual|between|notBetween, value, value2? (between), replace?} keeps the items whose caption passes (* and ? are wildcards); replaces the field's other filters",
            has_doc,
            label_filter
        ),
        cmd!(
            "pivot.valueFilter",
            "Value Filter",
            [],
            None,
            "{pivot?, field: row or column field, op: equals|doesNotEqual|greaterThan|greaterThanOrEqual|lessThan|lessThanOrEqual|between|notBetween, value: number, value2? (between), valueField?: caption (default the first value field), replace?} keeps the items whose total passes, within each parent item; replaces the field's other filters",
            has_doc,
            value_filter
        ),
        cmd!(
            "pivot.top10",
            "Top 10",
            [],
            None,
            "{pivot?, field: row or column field, top?: true (false: bottom), count?: 10, by?: items|percent|sum, valueField?: caption (default the first value field), replace?} keeps the top or bottom items by their totals, within each parent item; replaces the field's other filters",
            has_doc,
            top10
        ),
        cmd!(
            "pivot.clearFilter",
            "Clear Filter",
            [],
            None,
            "{pivot?, field?, replace?} clears the field's item, label, value and Top 10 filters (report filters show all); without `field`, every field's",
            has_doc,
            clear_filter
        ),
        cmd!("pivot.sort", "Sort Field", [], None, "{pivot?, field, order: asc|desc|none, replace?}", has_doc, sort),
        cmd!(
            "pivot.group",
            "Group Field",
            ["PivotTable Analyze", "Group"],
            None,
            "{pivot?, field, by: years|quarters|months|days|range|none, start?, end?, interval? (by: range; default the smallest and largest numbers, and 10), replace?}",
            has_doc,
            group
        ),
        cmd!(
            "pivot.collapse",
            "Expand/Collapse Field",
            ["PivotTable Analyze", "Active Field"],
            None,
            "{pivot?, field, items?: [labels] (default: all), collapse?: true, replace?}",
            has_doc,
            collapse
        ),
        cmd!(
            "pivot.layout",
            "PivotTable Layout",
            ["PivotTable Design", "Layout"],
            None,
            "{pivot?, layout?: compact|outline|tabular, grandTotalsRows?: bool, grandTotalsCols?: bool, subtotals?: bool|top|bottom|none, style?: \"PivotStyleLight16\", showHeaders?: bool, replace?}",
            has_doc,
            layout
        ),
        cmd!(
            "pivot.grandTotals",
            "Grand Totals",
            ["PivotTable Design", "Layout"],
            None,
            "{pivot?, mode?: off|on|rows|columns, rows?: bool, cols?: bool}",
            has_doc,
            grand_totals
        ),
        cmd!("pivot.subtotals", "Subtotals", ["PivotTable Design", "Layout"], None, "{pivot?, mode: none|top|bottom}", has_doc, |s, p| {
            let mode = str_param(p, "mode").unwrap_or("top");
            layout(s, &with(p, "subtotals", json!(mode)))
        }),
        cmd!(
            "pivot.reportLayout",
            "Report Layout",
            ["PivotTable Design", "Layout"],
            None,
            "{pivot?, layout: compact|outline|tabular}",
            has_doc,
            layout
        ),
        cmd!(
            "pivot.style",
            "PivotTable Styles",
            ["PivotTable Design", "PivotTable Styles"],
            None,
            "{pivot?, style: \"PivotStyleMedium9\"}",
            has_doc,
            layout
        ),
        cmd!("pivot.refresh", "Refresh", ["PivotTable Analyze", "Data"], Some("Alt+F5"), "{pivot?, replace?}", has_doc, refresh),
        cmd!(
            "data.refreshAll",
            "Refresh All",
            ["Data", "Queries & Connections"],
            Some("Cmd+Alt+F5"),
            "{replace?} refreshes every PivotTable → {refreshed, errors}",
            has_doc,
            refresh_all
        ),
        cmd!(
            "pivot.changeSource",
            "Change Data Source",
            ["PivotTable Analyze", "Data"],
            None,
            "{pivot?, source: \"Sheet1!$A$1:$E$300\" | table name, replace?} (fields missing from the new source are removed)",
            has_doc,
            change_source
        ),
        cmd!(
            "pivot.delete",
            "Delete PivotTable",
            ["PivotTable Analyze", "Actions"],
            None,
            "{pivot?} clears the report and removes the PivotTable",
            has_doc,
            delete
        ),
    ]
}

fn with(p: &Json, k: &str, v: Json) -> Json {
    let mut m = p.as_object().cloned().unwrap_or_default();
    m.insert(k.into(), v);
    Json::Object(m)
}

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

// ---------------------------------------------------------------- lookup

/// (sheet, index) of the pivot a call targets.
fn find_pivot(s: &Session, p: &Json) -> Result<(usize, usize)> {
    let d = s.doc()?;
    let active = d.wb.active_sheet;
    if let Some(v) = p.get("pivot").filter(|v| !v.is_null()) {
        let mut order: Vec<usize> = vec![active];
        order.extend((0..d.wb.sheets.len()).filter(|i| *i != active));
        for si in order {
            let Some(sh) = d.wb.sheet(si) else { continue };
            let hit = sh.pivots.iter().position(|pt| match v {
                Json::String(n) => pt.name.eq_ignore_ascii_case(n),
                Json::Number(n) => n.as_u64() == Some(pt.id as u64),
                _ => false,
            });
            if let Some(i) = hit {
                return Ok((si, i));
            }
        }
        return Err(bad("pivot", format!("no PivotTable `{}`", v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))));
    }
    let sh = d.wb.active().ok_or(EngineError::NoDocument)?;
    let at = d.selection.active;
    if let Some(i) = sh.pivots.iter().position(|pt| pt.last_range.is_some_and(|r| r.contains(at))) {
        return Ok((active, i));
    }
    if !sh.pivots.is_empty() {
        return Ok((active, sh.pivots.len() - 1));
    }
    Err(EngineError::Disabled("pivot".into(), "select a cell in a PivotTable (or pass `pivot`)".into()))
}

fn area_param(cmd: &str, v: Option<&str>) -> Result<&'static str> {
    match v.map(|x| x.to_ascii_lowercase()) {
        Some(a) if a == "rows" || a == "row" => Ok("rows"),
        Some(a) if a == "columns" || a == "column" || a == "cols" => Ok("columns"),
        Some(a) if a == "values" || a == "value" || a == "data" => Ok("values"),
        Some(a) if a == "filters" || a == "filter" => Ok("filters"),
        Some(a) => Err(bad(cmd, format!("unknown area `{a}` (rows|columns|values|filters)"))),
        None => Err(bad(cmd, "missing `area` (rows|columns|values|filters)")),
    }
}

fn func_param(v: Option<&str>) -> Option<PivotFunc> {
    Some(match v?.to_ascii_lowercase().as_str() {
        "sum" => PivotFunc::Sum,
        "count" | "counta" => PivotFunc::Count,
        "average" | "avg" | "mean" => PivotFunc::Average,
        "max" => PivotFunc::Max,
        "min" => PivotFunc::Min,
        "product" => PivotFunc::Product,
        "countnumbers" | "countnums" | "count numbers" => PivotFunc::CountNumbers,
        "stddev" => PivotFunc::StdDev,
        "stddevp" => PivotFunc::StdDevP,
        "var" => PivotFunc::Var,
        "varp" => PivotFunc::VarP,
        _ => return None,
    })
}

fn func_name(f: PivotFunc) -> &'static str {
    match f {
        PivotFunc::Sum => "sum",
        PivotFunc::Count => "count",
        PivotFunc::Average => "average",
        PivotFunc::Max => "max",
        PivotFunc::Min => "min",
        PivotFunc::Product => "product",
        PivotFunc::CountNumbers => "countNumbers",
        PivotFunc::StdDev => "stdDev",
        PivotFunc::StdDevP => "stdDevP",
        PivotFunc::Var => "var",
        PivotFunc::VarP => "varP",
    }
}

/// Show Values As modes: (command name, mode).
const SHOW_AS: &[(&str, PivotShowAs)] = &[
    ("normal", PivotShowAs::Normal),
    ("percentOfGrandTotal", PivotShowAs::PercentOfGrandTotal),
    ("percentOfColumnTotal", PivotShowAs::PercentOfColumnTotal),
    ("percentOfRowTotal", PivotShowAs::PercentOfRowTotal),
    ("percentOf", PivotShowAs::PercentOf),
    ("percentOfParentRowTotal", PivotShowAs::PercentOfParentRowTotal),
    ("percentOfParentColumnTotal", PivotShowAs::PercentOfParentColumnTotal),
    ("percentOfParentTotal", PivotShowAs::PercentOfParentTotal),
    ("differenceFrom", PivotShowAs::DifferenceFrom),
    ("percentDifferenceFrom", PivotShowAs::PercentDifferenceFrom),
    ("runningTotal", PivotShowAs::RunningTotal),
    ("percentRunningTotal", PivotShowAs::PercentRunningTotal),
    ("rankAscending", PivotShowAs::RankAscending),
    ("rankDescending", PivotShowAs::Rank),
    ("index", PivotShowAs::Index),
];

fn show_as_param(v: &str) -> Option<PivotShowAs> {
    match v.to_ascii_lowercase().as_str() {
        "none" => Some(PivotShowAs::Normal),
        "rank" => Some(PivotShowAs::Rank),
        l => SHOW_AS.iter().find(|(n, _)| n.eq_ignore_ascii_case(l)).map(|(_, m)| *m),
    }
}

fn show_as_name(v: PivotShowAs) -> &'static str {
    SHOW_AS.iter().find(|(_, m)| *m == v).map(|(n, _)| *n).unwrap_or("normal")
}

fn layout_name(l: PivotLayout) -> &'static str {
    match l {
        PivotLayout::Compact => "compact",
        PivotLayout::Outline => "outline",
        PivotLayout::Tabular => "tabular",
    }
}

fn group_name(g: PivotDateGroup) -> &'static str {
    match g {
        PivotDateGroup::None => "none",
        PivotDateGroup::Years => "years",
        PivotDateGroup::Quarters => "quarters",
        PivotDateGroup::Months => "months",
        PivotDateGroup::Days => "days",
    }
}

/// A row or column field as `pivot.list` and `pivot.fields` report it.
fn axis_json(f: &PivotField) -> Json {
    let group = if f.number_group.is_some() { "range" } else { group_name(f.date_group) };
    let range = f.number_group.map(|g| json!({"start": g.start, "end": g.end, "interval": g.by}));
    json!({"field": f.source_col, "sort": sort_name(f.sort), "group": group, "range": range, "collapsed": f.collapsed_items})
}

fn sort_name(o: PivotSort) -> &'static str {
    match o {
        PivotSort::Asc => "asc",
        PivotSort::Desc => "desc",
        PivotSort::None => "none",
    }
}

// ---------------------------------------------------------------- layout editing (pure)

fn in_axes(pt: &PivotTable, field: &str) -> bool {
    pt.rows.iter().chain(pt.columns.iter()).any(|f| f.source_col.eq_ignore_ascii_case(field))
}

/// The canonical source field name for `field`.
fn canonical(cmd: &str, src: &pv::Source, field: &str) -> Result<String> {
    src.col(field).and_then(|c| src.headers.get(c).cloned()).ok_or_else(|| bad(cmd, format!("no field `{field}` in the source data")))
}

/// The calculated field named `field`, if any.
fn calc_field<'a>(pt: &'a PivotTable, field: &str) -> Option<&'a PivotCalcField> {
    pt.calculated_fields.iter().find(|c| c.name.eq_ignore_ascii_case(field.trim()))
}

/// The canonical name of a source or calculated field.
fn canonical_any(cmd: &str, pt: &PivotTable, src: &pv::Source, field: &str) -> Result<String> {
    match calc_field(pt, field) {
        Some(c) if src.col(field).is_none() => Ok(c.name.clone()),
        _ => canonical(cmd, src, field),
    }
}

fn unique_value_name(pt: &PivotTable, base: &str, except: Option<usize>) -> String {
    let taken = |n: &str| pt.values.iter().enumerate().any(|(i, v)| Some(i) != except && v.name.eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    (2..10_000).map(|k| format!("{base}{k}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

fn auto_name(f: PivotFunc, field: &str) -> String {
    format!("{} of {field}", pv::func_caption(f))
}

/// Adds `field` to an area (moving it out of the other axis areas).
fn add_to_area(
    cmd: &str,
    pt: &mut PivotTable,
    src: &pv::Source,
    field: &str,
    area: &str,
    func: Option<PivotFunc>,
    name: Option<&str>,
    position: Option<usize>,
) -> Result<()> {
    let is_calc = src.col(field).is_none() && calc_field(pt, field).is_some();
    if is_calc && area != "values" {
        return Err(other("A calculated field can only be placed in the Values area."));
    }
    let field = canonical_any(cmd, pt, src, field)?;
    match area {
        "rows" | "columns" => {
            if pt.rows.len() + pt.columns.len() >= pv::MAX_AXIS_FIELDS && !in_axes(pt, &field) {
                return Err(other(format!("A PivotTable can have at most {} row and column fields.", pv::MAX_AXIS_FIELDS)));
            }
            let existing = pt
                .rows
                .iter()
                .chain(pt.columns.iter())
                .find(|f| f.source_col.eq_ignore_ascii_case(&field))
                .cloned()
                .unwrap_or_else(|| PivotField { source_col: field.clone(), ..Default::default() });
            pt.rows.retain(|f| !f.source_col.eq_ignore_ascii_case(&field));
            pt.columns.retain(|f| !f.source_col.eq_ignore_ascii_case(&field));
            // A report filter that selects nothing in particular goes away; a selection stays
            // as the field's hidden items.
            pt.filters.retain(|f| !(f.source_col.eq_ignore_ascii_case(&field) && f.selected.is_none()));
            let list = if area == "rows" { &mut pt.rows } else { &mut pt.columns };
            let at = position.unwrap_or(list.len()).min(list.len());
            list.insert(at, existing);
        }
        "values" => {
            if pt.values.len() >= pv::MAX_VALUE_FIELDS {
                return Err(other(format!("A PivotTable can have at most {} value fields.", pv::MAX_VALUE_FIELDS)));
            }
            let numeric = is_calc || src.col(&field).and_then(|c| src.numeric.get(c).copied()).unwrap_or(false);
            let f = func.unwrap_or(if numeric { PivotFunc::Sum } else { PivotFunc::Count });
            let base = match name {
                Some(n) if !n.trim().is_empty() => {
                    if src.col(n).is_some() {
                        return Err(other("PivotTable field name already exists."));
                    }
                    n.to_string()
                }
                _ => auto_name(f, &field),
            };
            let nm = unique_value_name(pt, &base, None);
            let v = PivotValue { source_col: field, func: f, name: nm, ..PivotValue::default() };
            let at = position.unwrap_or(pt.values.len()).min(pt.values.len());
            pt.values.insert(at, v);
        }
        _ => {
            pt.rows.retain(|f| !f.source_col.eq_ignore_ascii_case(&field));
            pt.columns.retain(|f| !f.source_col.eq_ignore_ascii_case(&field));
            let existing = pt.filters.iter().position(|f| f.source_col.eq_ignore_ascii_case(&field));
            let entry = match existing {
                // Label, value and Top 10 filters apply to row and column fields only.
                Some(i) => PivotFilter { rule: None, ..pt.filters.remove(i) },
                None => PivotFilter { source_col: field, ..PivotFilter::default() },
            };
            let at = position.unwrap_or(pt.filters.len()).min(pt.filters.len());
            pt.filters.insert(at, entry);
        }
    }
    Ok(())
}

/// Removes `field` from an area; returns the source field it referred to.
fn remove_from_area(cmd: &str, pt: &mut PivotTable, field: &str, area: &str) -> Result<String> {
    match area {
        "rows" | "columns" => {
            let list = if area == "rows" { &mut pt.rows } else { &mut pt.columns };
            let i = list
                .iter()
                .position(|f| f.source_col.eq_ignore_ascii_case(field))
                .ok_or_else(|| bad(cmd, format!("`{field}` isn't in the {area} area")))?;
            let f = list.remove(i);
            pt.filters.retain(|x| !x.source_col.eq_ignore_ascii_case(&f.source_col));
            Ok(f.source_col)
        }
        "values" => {
            let i = pt
                .values
                .iter()
                .position(|v| v.name.eq_ignore_ascii_case(field))
                .or_else(|| pt.values.iter().position(|v| v.source_col.eq_ignore_ascii_case(field)))
                .ok_or_else(|| bad(cmd, format!("`{field}` isn't in the values area")))?;
            let v = pt.values.remove(i);
            retarget_rules(pt, &v.name, None);
            Ok(v.source_col)
        }
        _ => {
            let i = pt
                .filters
                .iter()
                .position(|f| f.source_col.eq_ignore_ascii_case(field) && !in_axes(pt, &f.source_col))
                .ok_or_else(|| bad(cmd, format!("`{field}` isn't in the filters area")))?;
            Ok(pt.filters.remove(i).source_col)
        }
    }
}

// ---------------------------------------------------------------- shared update

/// Applies `f` to the targeted pivot and re-renders it, as one undo step.
fn update(s: &mut Session, p: &Json, f: impl FnOnce(&mut PivotTable, &pv::Source, &Workbook) -> Result<Json>) -> Result<Json> {
    let (si, pi) = find_pivot(s, p)?;
    let replace = bool_param(p, "replace").unwrap_or(false);
    edit(s, |cx| {
        let pt = cx.wb.sheet(si).and_then(|sh| sh.pivots.get(pi)).cloned().ok_or_else(|| other("There's no such PivotTable."))?;
        let src = pv::read_source(&cx.wb, &pt.source, si).map_err(other)?;
        let mut new = pt.clone();
        let mut out = f(&mut new, &src, &cx.wb)?;
        if let Some(slot) = cx.sheet_mut(si)?.pivots.get_mut(pi) {
            *slot = new;
        }
        let area = pv::refresh(&mut cx.wb, si, pi, replace).map_err(other)?;
        cx.structural = true;
        if out.is_null() {
            out = json!({});
        }
        if let Some(m) = out.as_object_mut() {
            m.insert("range".into(), json!(area.a1()));
        }
        Ok(out)
    })
}

// ---------------------------------------------------------------- insert

/// The default source: the table at the selection, else the selected block / current region.
fn default_source(s: &Session) -> Result<String> {
    let d = s.doc()?;
    let sh = d.wb.active().ok_or(EngineError::NoDocument)?;
    if let Some(t) = sh.table_at(d.selection.active) {
        return Ok(t.name.clone());
    }
    let sel = d.selection.current();
    let r = if sel.is_single() {
        current_region(sh, d.selection.active)
    } else {
        let used = sh.used_range().unwrap_or(sel);
        sel.intersection(&used).unwrap_or(sel)
    };
    if r.is_single() && sh.value(r.start).is_empty() {
        return Err(other("The reference isn't valid. Select a cell in a range with labeled columns, or pass `source`."));
    }
    Ok(pv::range_text(&sh.name, r))
}

fn source_param(s: &Session, p: &Json) -> Result<String> {
    match str_param(p, "source").or_else(|| str_param(p, "range")) {
        Some(t) if !t.trim().is_empty() => {
            // Bare ranges are on the active sheet; store them with the sheet name.
            let d = s.doc()?;
            let t = t.trim().trim_start_matches('=');
            if !t.contains('!')
                && RangeRef::parse(t).is_some()
                && let Some(sh) = d.wb.active()
                && let Some(r) = RangeRef::parse(t)
            {
                return Ok(pv::range_text(&sh.name, r));
            }
            Ok(t.to_string())
        }
        _ => default_source(s),
    }
}

fn next_pivot_name(wb: &Workbook) -> String {
    (1..)
        .map(|n| format!("PivotTable{n}"))
        .find(|n| !wb.sheets.iter().any(|s| s.pivots.iter().any(|p| p.name.eq_ignore_ascii_case(n))))
        .unwrap_or_default()
}

fn next_pivot_id(wb: &Workbook) -> u32 {
    let m = wb.sheets.iter().flat_map(|s| s.pivots.iter().map(|p| p.id)).max().unwrap_or(0);
    wb.next_object_id().max(m.saturating_add(1))
}

fn field_list(v: Option<&Json>) -> Vec<Json> {
    match v {
        Some(Json::Array(a)) => a.iter().take(512).cloned().collect(),
        Some(Json::String(s)) => vec![Json::String(s.clone())],
        _ => vec![],
    }
}

fn insert_pivot(s: &mut Session, p: &Json) -> Result<Json> {
    if s.doc()?.wb.protected_structure && matches!(str_param(p, "destination"), None | Some("new")) {
        return Err(other("The workbook is protected and cannot be changed."));
    }
    let source = source_param(s, p)?;
    let active = s.doc()?.wb.active_sheet;
    // Validate the source now (header names, size).
    let src = pv::read_source(&s.doc()?.wb, &source, active).map_err(other)?;
    let name = match str_param(p, "name") {
        Some(n) if !n.trim().is_empty() => {
            let n = n.trim().to_string();
            if s.doc()?.wb.sheets.iter().any(|sh| sh.pivots.iter().any(|x| x.name.eq_ignore_ascii_case(&n))) {
                return Err(other("A PivotTable with that name already exists."));
            }
            n
        }
        _ => next_pivot_name(&s.doc()?.wb),
    };
    let dest = str_param(p, "destination").or_else(|| str_param(p, "location")).unwrap_or("new").trim().to_string();
    // Existing sheet destination: (sheet index, anchor).
    let target: Option<(usize, CellRef)> = if dest.eq_ignore_ascii_case("new") || dest.is_empty() {
        None
    } else {
        let d = s.doc()?;
        let (sh, body) = super::split_sheet(&dest);
        let si = match sh {
            Some(n) => d.wb.sheet_index(&n).ok_or_else(|| bad("insert.pivotTable", format!("no sheet named `{n}`")))?,
            None => active,
        };
        let at = RangeRef::parse(body).map(|r| r.start).ok_or_else(|| bad("insert.pivotTable", format!("not a cell: `{dest}`")))?;
        Some((si, at))
    };
    let replace = bool_param(p, "replace").unwrap_or(false);
    let rows = field_list(p.get("rows"));
    let columns = field_list(p.get("columns"));
    let values = field_list(p.get("values"));
    let filters = field_list(p.get("filters"));
    let mut new_sheet_at: Option<usize> = None;
    let res = edit(s, |cx| {
        let id = next_pivot_id(&cx.wb);
        let (si, anchor) = match target {
            Some(t) => t,
            None => {
                let at = cx.wb.active_sheet.min(cx.wb.sheets.len());
                let nm = cx.wb.next_sheet_name();
                cx.wb.sheets.insert(at, Arc::new(Sheet::new(nm)));
                for n in cx.wb.names.iter_mut() {
                    if let Some(sc) = n.scope.as_mut()
                        && *sc >= at
                    {
                        *sc += 1;
                    }
                }
                new_sheet_at = Some(at);
                (at, CellRef::new(2, 0))
            }
        };
        let mut pt = PivotTable { id, name: name.clone(), source: source.clone(), anchor, ..Default::default() };
        let add = |pt: &mut PivotTable, list: &[Json], area: &str| -> Result<()> {
            for f in list {
                match f {
                    Json::String(n) => add_to_area("insert.pivotTable", pt, &src, n, area, None, None, None)?,
                    Json::Object(_) => {
                        let n = str_param(f, "field").ok_or_else(|| bad("insert.pivotTable", "a field object needs `field`"))?;
                        add_to_area("insert.pivotTable", pt, &src, n, area, func_param(str_param(f, "func")), str_param(f, "name"), None)?;
                        let by = group_param(str_param(f, "group"));
                        if let Some(by) = by.filter(|b| *b != PivotDateGroup::None)
                            && matches!(area, "rows" | "columns")
                            && src.col(n).and_then(|c| src.numeric.get(c).copied()).unwrap_or(false)
                            && let Some(af) = pt.rows.iter_mut().chain(pt.columns.iter_mut()).find(|x| x.source_col.eq_ignore_ascii_case(n))
                        {
                            af.date_group = by;
                        }
                    }
                    _ => return Err(bad("insert.pivotTable", "fields are names or {field, func?, name?}")),
                }
            }
            Ok(())
        };
        add(&mut pt, &rows, "rows")?;
        add(&mut pt, &columns, "columns")?;
        add(&mut pt, &values, "values")?;
        add(&mut pt, &filters, "filters")?;
        let sh = cx.sheet_mut(si)?;
        sh.pivots.push(pt);
        let pi = sh.pivots.len() - 1;
        let area = pv::refresh(&mut cx.wb, si, pi, replace).map_err(other)?;
        cx.wb.active_sheet = si;
        *cx.sel = crate::Selection::at(anchor);
        cx.structural = true;
        let sheet_name = cx.wb.sheet(si).map(|s| s.name.clone()).unwrap_or_default();
        Ok(json!({
            "id": id,
            "name": name,
            "sheet": sheet_name,
            "anchor": anchor.a1(),
            "range": area.a1(),
            "source": source,
            "fields": src.headers,
        }))
    })?;
    if let Some(at) = new_sheet_at
        && let Ok(d) = s.doc_mut()
    {
        d.sheet_selections.insert(at.min(d.sheet_selections.len()), crate::Selection::default());
    }
    Ok(res)
}

fn summarize_table(s: &mut Session, p: &Json) -> Result<Json> {
    let d = s.doc()?;
    let name = match str_param(p, "table") {
        Some(n) => d.wb.table(n).and_then(|(si, ti)| d.wb.sheet(si).and_then(|sh| sh.tables.get(ti))).map(|t| t.name.clone()),
        None => d.wb.active().and_then(|sh| sh.table_at(d.selection.active)).map(|t| t.name.clone()),
    }
    .ok_or_else(|| EngineError::Disabled("table.summarizePivot".into(), "select a cell in a table".into()))?;
    let mut q = with(p, "source", json!(name));
    if let Some(m) = q.as_object_mut() {
        m.remove("table");
    }
    insert_pivot(s, &q)
}

// ---------------------------------------------------------------- queries

fn pivot_json(wb: &Workbook, si: usize, pt: &PivotTable) -> Json {
    json!({
        "id": pt.id,
        "name": pt.name,
        "sheet": wb.sheet(si).map(|s| s.name.clone()).unwrap_or_default(),
        "anchor": pt.anchor.a1(),
        "range": pt.last_range.map(|r| r.a1()),
        "source": pt.source,
        "rows": pt.rows.iter().map(axis_json).collect::<Vec<_>>(),
        "columns": pt.columns.iter().map(axis_json).collect::<Vec<_>>(),
        "values": pt.values.iter().map(|v| json!({"field": v.source_col, "func": func_name(v.func), "name": v.name, "showAs": show_as_name(v.show_as), "baseField": v.base_field, "baseItem": v.base_item, "numberFormat": v.number_format})).collect::<Vec<_>>(),
        "filters": pt.filters.iter().map(|f| json!({"field": f.source_col, "selected": f.selected, "rule": f.rule.as_ref().map(rule_json), "report": !in_axes(pt, &f.source_col)})).collect::<Vec<_>>(),
        "layout": layout_name(pt.layout),
        "grandTotalsRows": pt.grand_totals_rows,
        "grandTotalsCols": pt.grand_totals_cols,
        "subtotals": if !pt.subtotals { "none" } else if pt.subtotals_top { "top" } else { "bottom" },
        "style": pt.style,
        "showHeaders": pt.show_headers,
        "calculatedFields": pt.calculated_fields.iter().map(|c| json!({"name": c.name, "formula": c.formula})).collect::<Vec<_>>(),
    })
}

fn fields(s: &mut Session, p: &Json) -> Result<Json> {
    let (si, pi) = find_pivot(s, p)?;
    let d = s.doc()?;
    let pt = d.wb.sheet(si).and_then(|sh| sh.pivots.get(pi)).ok_or_else(|| other("There's no such PivotTable."))?;
    let mut out = pivot_json(&d.wb, si, pt);
    let src = pv::read_source(&d.wb, &pt.source, si);
    let list: Vec<Json> = match &src {
        Ok(src) => src
            .headers
            .iter()
            .enumerate()
            .map(|(c, h)| {
                let mut areas = vec![];
                if pt.rows.iter().any(|f| f.source_col.eq_ignore_ascii_case(h)) {
                    areas.push("rows");
                }
                if pt.columns.iter().any(|f| f.source_col.eq_ignore_ascii_case(h)) {
                    areas.push("columns");
                }
                if pt.values.iter().any(|f| f.source_col.eq_ignore_ascii_case(h)) {
                    areas.push("values");
                }
                if pt.filters.iter().any(|f| f.source_col.eq_ignore_ascii_case(h)) && !in_axes(pt, h) {
                    areas.push("filters");
                }
                json!({
                    "name": h,
                    "numeric": src.numeric.get(c).copied().unwrap_or(false),
                    "date": src.dates.get(c).copied().unwrap_or(false),
                    "samples": pv::distinct_labels(&d.wb, src, c, pv::Grouping::default(), 5),
                    "areas": areas,
                })
            })
            .collect(),
        Err(_) => vec![],
    };
    let mut list = list;
    for c in &pt.calculated_fields {
        let areas: Vec<&str> = if pt.values.iter().any(|v| v.source_col.eq_ignore_ascii_case(&c.name)) { vec!["values"] } else { vec![] };
        list.push(json!({"name": c.name, "numeric": true, "date": false, "samples": [], "areas": areas, "calculated": true, "formula": c.formula}));
    }
    if let Some(m) = out.as_object_mut() {
        m.insert("fields".into(), Json::Array(list));
        if let Ok(src) = &src {
            m.insert("records".into(), json!(src.rows.len()));
        }
        if let Err(e) = src {
            m.insert("error".into(), json!(e));
        }
    }
    Ok(out)
}

fn list(s: &mut Session, _: &Json) -> Result<Json> {
    let d = s.doc()?;
    let mut v = vec![];
    for (si, sh) in d.wb.sheets.iter().enumerate() {
        for pt in &sh.pivots {
            v.push(pivot_json(&d.wb, si, pt));
        }
    }
    Ok(Json::Array(v))
}

// ---------------------------------------------------------------- field commands

fn position_param(p: &Json) -> Option<usize> {
    u32_param(p, "position").map(|v| v as usize)
}

fn add_field(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.addField";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let area = area_param(C, str_param(p, "area"))?;
    let func = match str_param(p, "func") {
        Some(f) => Some(func_param(Some(f)).ok_or_else(|| bad(C, format!("unknown function `{f}`")))?),
        None => None,
    };
    let name = str_param(p, "name").map(str::to_string);
    let pos = position_param(p);
    update(s, p, |pt, src, _| {
        add_to_area(C, pt, src, &field, area, func, name.as_deref(), pos)?;
        Ok(json!({"field": canonical_any(C, pt, src, &field)?, "area": area}))
    })
}

fn calculated_field(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.calculatedField";
    let Some(name) = str_param(p, "name").map(|n| n.trim().to_string()) else {
        // Menu-style call: the Insert Calculated Field dialog.
        let (si, pi) = find_pivot(s, p)?;
        let d = s.doc()?;
        let pt = d.wb.sheet(si).and_then(|sh| sh.pivots.get(pi)).ok_or_else(|| other("There's no such PivotTable."))?;
        let n = (1..).map(|k| format!("Field{k}")).find(|n| calc_field(pt, n).is_none()).unwrap_or_else(|| "Field1".into());
        let pivot = pt.name.clone();
        s.ui_requests.push(crate::UiRequest::Dialog("pivotCalculatedField".into(), json!({"pivot": pivot, "name": n, "formula": "= 0"})));
        return ok();
    };
    if name.is_empty() || name.chars().count() > 255 {
        return Err(bad(C, "`name` must be 1 to 255 characters"));
    }
    let delete = bool_param(p, "delete").unwrap_or(false);
    let formula = str_param(p, "formula").map(|f| {
        let f = f.trim();
        if f.starts_with('=') { f.to_string() } else { format!("={f}") }
    });
    let to_values = bool_param(p, "values").unwrap_or(true);
    if !delete && formula.is_none() {
        return Err(bad(C, "missing `formula`"));
    }
    update(s, p, |pt, src, _| {
        let existing = pt.calculated_fields.iter().position(|c| c.name.eq_ignore_ascii_case(&name));
        if delete {
            let i = existing.ok_or_else(|| bad(C, format!("no calculated field `{name}`")))?;
            let gone = pt.calculated_fields.remove(i);
            pt.values.retain(|v| !v.source_col.eq_ignore_ascii_case(&gone.name));
            return Ok(json!({"name": gone.name, "deleted": true}));
        }
        let formula = formula.clone().unwrap_or_default();
        if src.col(&name).is_some() {
            return Err(other("PivotTable field name already exists."));
        }
        let canon = match existing {
            Some(i) => {
                let c = pt.calculated_fields.get_mut(i).ok_or_else(|| other("There's no such calculated field."))?;
                c.formula = formula.clone();
                c.name.clone()
            }
            None => {
                if pt.calculated_fields.len() >= crate::pivot_calc::MAX_CALC_FIELDS {
                    return Err(other(format!("A PivotTable can have at most {} calculated fields.", crate::pivot_calc::MAX_CALC_FIELDS)));
                }
                pt.calculated_fields.push(PivotCalcField { name: name.clone(), formula: formula.clone() });
                name.clone()
            }
        };
        let set = crate::pivot_calc::CalcSet::new(pt, &src.headers);
        let i = set.index(&canon).ok_or_else(|| other("There's no such calculated field."))?;
        if let Some(e) = set.error(i) {
            return Err(bad(C, e.to_string()));
        }
        if set.is_cyclic(i) {
            return Err(bad(C, format!("`{canon}` refers to itself through its formula")));
        }
        if existing.is_none() && to_values && !pt.values.iter().any(|v| v.source_col.eq_ignore_ascii_case(&canon)) {
            add_to_area(C, pt, src, &canon, "values", Some(PivotFunc::Sum), None, None)?;
        }
        Ok(json!({
            "name": canon,
            "formula": formula,
            "calculatedFields": pt.calculated_fields.iter().map(|c| json!({"name": c.name, "formula": c.formula})).collect::<Vec<_>>(),
        }))
    })
}

fn remove_field(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.removeField";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let area = match str_param(p, "area") {
        Some(a) => Some(area_param(C, Some(a))?),
        None => None,
    };
    update(s, p, |pt, _, _| {
        match area {
            Some(a) => {
                remove_from_area(C, pt, &field, a)?;
            }
            None => {
                let mut any = false;
                for a in ["rows", "columns", "values", "filters"] {
                    any |= remove_from_area(C, pt, &field, a).is_ok();
                }
                if !any {
                    return Err(bad(C, format!("`{field}` isn't in the PivotTable")));
                }
            }
        }
        Ok(json!({}))
    })
}

fn move_field(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.moveField";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let from = area_param(C, str_param(p, "from"))?;
    let to = area_param(C, str_param(p, "to"))?;
    let pos = position_param(p);
    update(s, p, |pt, src, _| {
        if from == to {
            // Reorder within the area.
            match from {
                "rows" | "columns" => {
                    let list = if from == "rows" { &mut pt.rows } else { &mut pt.columns };
                    let i = list
                        .iter()
                        .position(|f| f.source_col.eq_ignore_ascii_case(&field))
                        .ok_or_else(|| bad(C, format!("`{field}` isn't in the {from} area")))?;
                    let f = list.remove(i);
                    let at = pos.unwrap_or(list.len()).min(list.len());
                    list.insert(at, f);
                }
                "values" => {
                    let i = pt
                        .values
                        .iter()
                        .position(|v| v.name.eq_ignore_ascii_case(&field))
                        .or_else(|| pt.values.iter().position(|v| v.source_col.eq_ignore_ascii_case(&field)))
                        .ok_or_else(|| bad(C, format!("`{field}` isn't in the values area")))?;
                    let v = pt.values.remove(i);
                    let at = pos.unwrap_or(pt.values.len()).min(pt.values.len());
                    pt.values.insert(at, v);
                }
                _ => {
                    let i = pt
                        .filters
                        .iter()
                        .position(|f| f.source_col.eq_ignore_ascii_case(&field))
                        .ok_or_else(|| bad(C, format!("`{field}` isn't in the filters area")))?;
                    let f = pt.filters.remove(i);
                    let at = pos.unwrap_or(pt.filters.len()).min(pt.filters.len());
                    pt.filters.insert(at, f);
                }
            }
            return Ok(json!({}));
        }
        // Keep axis settings (sort, grouping) when moving between rows and columns.
        let kept = pt.rows.iter().chain(pt.columns.iter()).find(|f| f.source_col.eq_ignore_ascii_case(&field)).cloned();
        let hidden = pt.filters.iter().find(|f| f.source_col.eq_ignore_ascii_case(&field)).cloned();
        let src_field = remove_from_area(C, pt, &field, from)?;
        add_to_area(C, pt, src, &src_field, to, None, None, pos)?;
        if matches!(to, "rows" | "columns") {
            let list = if to == "rows" { &mut pt.rows } else { &mut pt.columns };
            if let (Some(k), Some(slot)) = (kept, list.iter_mut().find(|f| f.source_col.eq_ignore_ascii_case(&src_field))) {
                *slot = k;
            }
            if let Some(h) = hidden.filter(|h| h.selected.is_some() || h.rule.is_some())
                && !pt.filters.iter().any(|f| f.source_col.eq_ignore_ascii_case(&src_field))
            {
                pt.filters.push(h);
            }
        }
        Ok(json!({"field": src_field, "to": to}))
    })
}

fn value_settings(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.valueSettings";
    let field = str_param(p, "field").map(str::to_string);
    let index = u32_param(p, "index").map(|v| v as usize);
    let func = match str_param(p, "func") {
        Some(f) => Some(func_param(Some(f)).ok_or_else(|| bad(C, format!("unknown function `{f}`")))?),
        None => None,
    };
    let show = match str_param(p, "showAs") {
        Some(v) => Some(show_as_param(v).ok_or_else(|| bad(C, format!("unknown showAs `{v}`")))?),
        None => None,
    };
    let name = str_param(p, "name").map(str::to_string);
    let fmt = p.get("numberFormat").cloned();
    update(s, p, |pt, src, _| {
        let i = match (index, &field) {
            (Some(i), _) => i,
            (None, Some(f)) => pt
                .values
                .iter()
                .position(|v| v.name.eq_ignore_ascii_case(f))
                .or_else(|| pt.values.iter().position(|v| v.source_col.eq_ignore_ascii_case(f)))
                .ok_or_else(|| bad(C, format!("`{f}` isn't in the values area")))?,
            (None, None) => {
                if pt.values.len() == 1 {
                    0
                } else {
                    return Err(bad(C, "pass `field` or `index`"));
                }
            }
        };
        let cur = pt.values.get(i).cloned().ok_or_else(|| bad(C, "no value field at that index"))?;
        let mut v = cur.clone();
        if let Some(f) = func {
            v.func = f;
            // An automatic caption follows the function.
            let auto = auto_name(cur.func, &cur.source_col);
            if cur.name.eq_ignore_ascii_case(&auto) || cur.name.strip_prefix(auto.as_str()).is_some_and(|r| r.chars().all(|c| c.is_ascii_digit())) {
                v.name = unique_value_name(pt, &auto_name(f, &cur.source_col), Some(i));
            }
        }
        if let Some(n) = &name {
            let n = n.trim();
            if n.is_empty() {
                return Err(other("The name isn't valid."));
            }
            if src.col(n).is_some() {
                return Err(other("PivotTable field name already exists."));
            }
            if pt.values.iter().enumerate().any(|(j, x)| j != i && x.name.eq_ignore_ascii_case(n)) {
                return Err(other("PivotTable field name already exists."));
            }
            v.name = n.to_string();
        }
        if let Some(sa) = show {
            v.show_as = sa;
        }
        if let Some(b) = str_param(p, "baseField") {
            if !in_axes(pt, b) {
                return Err(bad(C, format!("the base field `{b}` must be a row or column field")));
            }
            v.base_field = Some(b.to_string());
        }
        if let Some(b) = str_param(p, "baseItem") {
            v.base_item = Some(b.to_string());
        }
        if v.show_as.needs_base_field() && v.base_field.is_none() {
            v.base_field = pt.rows.first().or(pt.columns.first()).map(|f| f.source_col.clone());
        }
        if v.show_as.needs_base_item() && v.base_item.is_none() {
            v.base_item = Some(BASE_PREVIOUS.to_string());
        }
        match &fmt {
            Some(Json::String(f)) if !f.is_empty() && f.len() <= 255 => v.number_format = Some(f.clone()),
            Some(Json::Null) => v.number_format = None,
            Some(Json::String(f)) if f.is_empty() => v.number_format = None,
            Some(_) => return Err(bad(C, "`numberFormat` is a format code or null")),
            None => {}
        }
        let name = v.name.clone();
        if !name.eq_ignore_ascii_case(&cur.name) {
            retarget_rules(pt, &cur.name, Some(&name));
        }
        if let Some(slot) = pt.values.get_mut(i) {
            *slot = v;
        }
        Ok(json!({"name": name}))
    })
}

fn filter(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.filter";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let selected: Option<Vec<String>> = match p.get("selected") {
        None | Some(Json::Null) => None,
        Some(Json::Array(a)) => {
            if a.len() > 100_000 {
                return Err(bad(C, "too many items"));
            }
            let v: Vec<String> = a
                .iter()
                .map(|x| match x {
                    Json::String(s) => s.clone(),
                    Json::Null => "(blank)".into(),
                    other => other.to_string(),
                })
                .collect();
            if v.is_empty() {
                return Err(other("You must select at least one item."));
            }
            Some(v)
        }
        Some(Json::String(s)) => Some(vec![s.clone()]),
        Some(_) => return Err(bad(C, "`selected` is a list of item labels or null")),
    };
    update(s, p, |pt, src, _| {
        let field = canonical(C, src, &field)?;
        let i = pt.filters.iter().position(|f| f.source_col.eq_ignore_ascii_case(&field));
        match (i, selected) {
            (Some(i), None) => {
                // An axis field's filter with no rule left has nothing to keep.
                if in_axes(pt, &field) && pt.filters.get(i).is_some_and(|f| f.rule.is_none()) {
                    pt.filters.remove(i);
                } else if let Some(f) = pt.filters.get_mut(i) {
                    f.selected = None;
                }
            }
            (Some(i), Some(sel)) => {
                if let Some(f) = pt.filters.get_mut(i) {
                    f.selected = Some(sel);
                    f.rule = None;
                }
            }
            (None, Some(sel)) => pt.filters.push(PivotFilter { source_col: field.clone(), selected: Some(sel), rule: None }),
            (None, None) => {}
        }
        Ok(json!({"field": field}))
    })
}

// ---------------------------------------------------------------- label, value and Top 10 filters

fn compare_param(cmd: &str, v: Option<&str>) -> Result<PivotCompare> {
    let raw = v.ok_or_else(|| bad(cmd, "missing `op`"))?;
    PivotCompare::ALL.into_iter().find(|c| compare_name(*c).eq_ignore_ascii_case(raw)).ok_or_else(|| bad(cmd, format!("unknown comparison `{raw}`")))
}

fn compare_name(c: PivotCompare) -> &'static str {
    match c {
        PivotCompare::Equals => "equals",
        PivotCompare::NotEquals => "doesNotEqual",
        PivotCompare::BeginsWith => "beginsWith",
        PivotCompare::NotBeginsWith => "doesNotBeginWith",
        PivotCompare::EndsWith => "endsWith",
        PivotCompare::NotEndsWith => "doesNotEndWith",
        PivotCompare::Contains => "contains",
        PivotCompare::NotContains => "doesNotContain",
        PivotCompare::Greater => "greaterThan",
        PivotCompare::GreaterOrEqual => "greaterThanOrEqual",
        PivotCompare::Less => "lessThan",
        PivotCompare::LessOrEqual => "lessThanOrEqual",
        PivotCompare::Between => "between",
        PivotCompare::NotBetween => "notBetween",
    }
}

fn top_by_name(b: PivotTopBy) -> &'static str {
    match b {
        PivotTopBy::Items => "items",
        PivotTopBy::Percent => "percent",
        PivotTopBy::Sum => "sum",
    }
}

/// A filter rule as `pivot.list` and `pivot.fields` report it, in the parameters of the command
/// that sets it.
fn rule_json(r: &PivotRule) -> Json {
    match r {
        PivotRule::Label { op, value, value2 } => json!({"type": "label", "op": compare_name(*op), "value": value, "value2": value2}),
        PivotRule::Value { op, field, value, value2 } => {
            json!({"type": "value", "op": compare_name(*op), "valueField": field, "value": value, "value2": value2})
        }
        PivotRule::Top { top, count, by, field } => json!({"type": "top10", "top": top, "count": count, "by": top_by_name(*by), "valueField": field}),
    }
}

/// A label filter value: text, or a number or logical written as text.
fn text_param(cmd: &str, p: &Json, k: &str) -> Result<Option<String>> {
    match p.get(k) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(t)) if t.len() <= 255 => Ok(Some(t.clone())),
        Some(Json::Number(n)) => Ok(Some(n.to_string())),
        Some(Json::Bool(b)) => Ok(Some(if *b { "TRUE" } else { "FALSE" }.into())),
        Some(_) => Err(bad(cmd, format!("`{k}` is text of at most 255 characters"))),
    }
}

fn number_param(cmd: &str, p: &Json, k: &str) -> Result<Option<f64>> {
    match p.get(k) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|x| x.is_finite()).map(Some).ok_or_else(|| bad(cmd, format!("`{k}` must be a number"))),
    }
}

/// The caption of the value field a value filter or Top 10 uses: the one named, else the first.
fn value_field_param(cmd: &str, pt: &PivotTable, p: &Json) -> Result<String> {
    match str_param(p, "valueField") {
        Some(n) => pt
            .values
            .iter()
            .find(|v| v.name.eq_ignore_ascii_case(n))
            .or_else(|| pt.values.iter().find(|v| v.source_col.eq_ignore_ascii_case(n)))
            .map(|v| v.name.clone())
            .ok_or_else(|| bad(cmd, format!("`{n}` isn't in the values area"))),
        None => pt.values.first().map(|v| v.name.clone()).ok_or_else(|| other("A value filter needs a field in the Values area.")),
    }
}

/// Sets the filter rule of a row or column field, replacing its other filters.
fn set_rule(cmd: &str, pt: &mut PivotTable, field: &str, rule: PivotRule) -> Result<String> {
    let field = pt
        .rows
        .iter()
        .chain(pt.columns.iter())
        .find(|f| f.source_col.eq_ignore_ascii_case(field))
        .map(|f| f.source_col.clone())
        .ok_or_else(|| bad(cmd, format!("`{field}` isn't in the rows or columns area")))?;
    pt.filters.retain(|f| !f.source_col.eq_ignore_ascii_case(&field));
    pt.filters.push(PivotFilter { source_col: field.clone(), selected: None, rule: Some(rule) });
    Ok(field)
}

/// Follows a value field's change of caption in the rules that use it, or drops those rules
/// (`to: None`) when the value field is removed.
fn retarget_rules(pt: &mut PivotTable, from: &str, to: Option<&str>) {
    for f in pt.filters.iter_mut() {
        let Some(rule) = f.rule.as_mut() else { continue };
        let (PivotRule::Value { field, .. } | PivotRule::Top { field, .. }) = rule else { continue };
        if !field.eq_ignore_ascii_case(from) {
            continue;
        }
        match to {
            Some(n) => *field = n.to_string(),
            None => f.rule = None,
        }
    }
    let axis: Vec<String> = pt.rows.iter().chain(pt.columns.iter()).map(|f| f.source_col.clone()).collect();
    pt.filters.retain(|f| f.selected.is_some() || f.rule.is_some() || !axis.iter().any(|a| a.eq_ignore_ascii_case(&f.source_col)));
}

fn label_filter(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.labelFilter";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let op = compare_param(C, str_param(p, "op"))?;
    let value = text_param(C, p, "value")?.ok_or_else(|| bad(C, "missing `value`"))?;
    let value2 = text_param(C, p, "value2")?;
    if op.is_range() && value2.is_none() {
        return Err(bad(C, "`between` and `notBetween` need `value2`"));
    }
    let rule = PivotRule::Label { op, value, value2: value2.unwrap_or_default() };
    update(s, p, |pt, _, _| Ok(json!({"field": set_rule(C, pt, &field, rule)?})))
}

fn value_filter(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.valueFilter";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let op = compare_param(C, str_param(p, "op"))?;
    if op.is_text_only() {
        return Err(bad(C, "a value filter compares numbers"));
    }
    let value = number_param(C, p, "value")?.ok_or_else(|| bad(C, "missing `value`"))?;
    let value2 = number_param(C, p, "value2")?;
    if op.is_range() && value2.is_none() {
        return Err(bad(C, "`between` and `notBetween` need `value2`"));
    }
    update(s, p, |pt, _, _| {
        let rule = PivotRule::Value { op, field: value_field_param(C, pt, p)?, value, value2: value2.unwrap_or(0.0) };
        Ok(json!({"field": set_rule(C, pt, &field, rule)?}))
    })
}

fn top10(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.top10";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let top = match p.get("top") {
        None | Some(Json::Null) => true,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err(bad(C, "`top` is true (top) or false (bottom)")),
    };
    let by = match str_param(p, "by").unwrap_or("items") {
        b if b.eq_ignore_ascii_case("items") => PivotTopBy::Items,
        b if b.eq_ignore_ascii_case("percent") => PivotTopBy::Percent,
        b if b.eq_ignore_ascii_case("sum") => PivotTopBy::Sum,
        b => return Err(bad(C, format!("unknown `by` `{b}` (items|percent|sum)"))),
    };
    let count = number_param(C, p, "count")?.unwrap_or(10.0);
    let ok = match by {
        PivotTopBy::Items => (1.0..=2_147_483_647.0).contains(&count) && count.fract() == 0.0,
        PivotTopBy::Percent => count > 0.0 && count <= 100.0,
        PivotTopBy::Sum => true,
    };
    if !ok {
        return Err(other(match by {
            PivotTopBy::Items => "Enter a whole number of items, 1 or more.",
            PivotTopBy::Percent => "Enter a percent greater than 0 and at most 100.",
            PivotTopBy::Sum => "Enter a number.",
        }));
    }
    update(s, p, |pt, _, _| {
        let rule = PivotRule::Top { top, count, by, field: value_field_param(C, pt, p)? };
        Ok(json!({"field": set_rule(C, pt, &field, rule)?}))
    })
}

fn clear_filter(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.clearFilter";
    let field = str_param(p, "field").map(str::to_string);
    update(s, p, |pt, _, _| {
        if let Some(n) = &field
            && !pt.filters.iter().any(|f| f.source_col.eq_ignore_ascii_case(n))
            && !in_axes(pt, n)
        {
            return Err(bad(C, format!("`{n}` isn't a field of the PivotTable")));
        }
        let hit = |f: &PivotFilter| field.as_ref().is_none_or(|n| f.source_col.eq_ignore_ascii_case(n));
        // A row or column field's filter goes; a report filter stays in its area, showing all.
        let axis: Vec<String> = pt.rows.iter().chain(pt.columns.iter()).map(|f| f.source_col.clone()).collect();
        pt.filters.retain(|f| !(hit(f) && axis.iter().any(|a| a.eq_ignore_ascii_case(&f.source_col))));
        for f in pt.filters.iter_mut().filter(|f| hit(f)) {
            f.selected = None;
            f.rule = None;
        }
        Ok(json!({}))
    })
}

fn axis_field<'a>(cmd: &str, pt: &'a mut PivotTable, field: &str) -> Result<&'a mut PivotField> {
    pt.rows
        .iter_mut()
        .chain(pt.columns.iter_mut())
        .find(|f| f.source_col.eq_ignore_ascii_case(field))
        .ok_or_else(|| bad(cmd, format!("`{field}` isn't in the rows or columns area")))
}

fn sort(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.sort";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let order = match str_param(p, "order").unwrap_or("asc").to_ascii_lowercase().as_str() {
        "asc" | "ascending" | "a-z" => PivotSort::Asc,
        "desc" | "descending" | "z-a" => PivotSort::Desc,
        "none" | "manual" | "source" => PivotSort::None,
        o => return Err(bad(C, format!("unknown order `{o}` (asc|desc|none)"))),
    };
    update(s, p, |pt, _, _| {
        axis_field(C, pt, &field)?.sort = order;
        Ok(json!({}))
    })
}

fn group_param(v: Option<&str>) -> Option<PivotDateGroup> {
    Some(match v?.to_ascii_lowercase().as_str() {
        "years" | "year" => PivotDateGroup::Years,
        "quarters" | "quarter" => PivotDateGroup::Quarters,
        "months" | "month" => PivotDateGroup::Months,
        "days" | "day" => PivotDateGroup::Days,
        "none" | "ungroup" => PivotDateGroup::None,
        _ => return None,
    })
}

fn group(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.group";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let raw = str_param(p, "by").unwrap_or("months");
    let range = raw.eq_ignore_ascii_case("range");
    let by = if range {
        PivotDateGroup::None
    } else {
        group_param(Some(raw)).ok_or_else(|| bad(C, format!("unknown grouping `{raw}` (years|quarters|months|days|range|none)")))?
    };
    let number = |key: &str| -> Result<Option<f64>> {
        match p.get(key) {
            None | Some(Json::Null) => Ok(None),
            Some(v) => v.as_f64().filter(|x| x.is_finite()).map(Some).ok_or_else(|| bad(C, format!("`{key}` must be a number"))),
        }
    };
    let (start, end, interval) = (number("start")?, number("end")?, number("interval")?);
    update(s, p, |pt, src, _| {
        let mut ranges = None;
        if range || by != PivotDateGroup::None {
            let c = src.col(&field).ok_or_else(|| bad(C, format!("no field `{field}` in the source data")))?;
            if !src.numeric.get(c).copied().unwrap_or(false) {
                return Err(other("Cannot group that selection."));
            }
            if range {
                // Excel's defaults: from the smallest to the largest number, in tens.
                let (lo, hi) = src
                    .rows
                    .iter()
                    .filter_map(|r| r.get(c).and_then(|v| if let gridcraft_core::Value::Number(n) = v { Some(*n) } else { None }))
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), n| (lo.min(n), hi.max(n)));
                let g = PivotNumberGroup::checked(start.unwrap_or(lo), end.unwrap_or(hi), interval.unwrap_or(10.0)).ok_or_else(|| {
                    other("Enter a starting number no greater than the ending number and a positive By value (at most 100,000 groups).")
                })?;
                ranges = Some(g);
            }
        }
        let f = axis_field(C, pt, &field)?;
        f.date_group = by;
        f.number_group = ranges;
        f.collapsed_items.clear();
        // The field's items change, so a selection of its items or a label filter no longer applies.
        for x in pt.filters.iter_mut().filter(|x| x.source_col.eq_ignore_ascii_case(&field)) {
            x.selected = None;
            if matches!(x.rule, Some(PivotRule::Label { .. })) {
                x.rule = None;
            }
        }
        Ok(json!({}))
    })
}

fn collapse(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.collapse";
    let field = str_param(p, "field").ok_or_else(|| bad(C, "missing `field`"))?.to_string();
    let on = bool_param(p, "collapse").unwrap_or(true);
    let items: Option<Vec<String>> =
        p.get("items").and_then(Json::as_array).map(|a| a.iter().take(100_000).filter_map(|x| x.as_str().map(str::to_string)).collect());
    update(s, p, |pt, src, wb| {
        let all = match &items {
            Some(v) => v.clone(),
            None => {
                let f = pt.rows.iter().chain(pt.columns.iter()).find(|f| f.source_col.eq_ignore_ascii_case(&field));
                let g = f.map(pv::Grouping::of).unwrap_or_default();
                match src.col(&field) {
                    Some(c) => pv::distinct_labels(wb, src, c, g, 100_000),
                    None => vec![],
                }
            }
        };
        let f = axis_field(C, pt, &field)?;
        if on {
            for it in all {
                if !f.collapsed_items.iter().any(|x| x.eq_ignore_ascii_case(&it)) {
                    f.collapsed_items.push(it);
                }
            }
        } else if items.is_none() {
            f.collapsed_items.clear();
        } else {
            f.collapsed_items.retain(|x| !all.iter().any(|a| a.eq_ignore_ascii_case(x)));
        }
        Ok(json!({"collapsed": f.collapsed_items}))
    })
}

fn layout(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.layout";
    let lay = match str_param(p, "layout").map(|x| x.to_ascii_lowercase()) {
        Some(l) if l == "compact" => Some(PivotLayout::Compact),
        Some(l) if l == "outline" => Some(PivotLayout::Outline),
        Some(l) if l == "tabular" => Some(PivotLayout::Tabular),
        Some(l) => return Err(bad(C, format!("unknown layout `{l}` (compact|outline|tabular)"))),
        None => None,
    };
    let subtotals: Option<(bool, Option<bool>)> = match p.get("subtotals") {
        Some(Json::Bool(b)) => Some((*b, None)),
        Some(Json::String(m)) => match m.to_ascii_lowercase().as_str() {
            "top" => Some((true, Some(true))),
            "bottom" => Some((true, Some(false))),
            "none" | "off" => Some((false, None)),
            o => return Err(bad(C, format!("unknown subtotals `{o}` (top|bottom|none)"))),
        },
        _ => None,
    };
    let style = str_param(p, "style").map(str::to_string);
    if style.as_ref().is_some_and(|s| s.len() > 64) {
        return Err(bad(C, "style name too long"));
    }
    let gr = bool_param(p, "grandTotalsRows");
    let gc = bool_param(p, "grandTotalsCols");
    let hd = bool_param(p, "showHeaders");
    update(s, p, |pt, _, _| {
        if let Some(l) = lay {
            pt.layout = l;
            if l == PivotLayout::Tabular {
                pt.subtotals_top = false;
            }
        }
        if let Some((on, top)) = subtotals {
            pt.subtotals = on;
            if let Some(t) = top {
                pt.subtotals_top = t;
            }
        }
        if let Some(st) = &style {
            pt.style = st.clone();
        }
        if let Some(v) = gr {
            pt.grand_totals_rows = v;
        }
        if let Some(v) = gc {
            pt.grand_totals_cols = v;
        }
        if let Some(v) = hd {
            pt.show_headers = v;
        }
        Ok(json!({"layout": layout_name(pt.layout)}))
    })
}

fn grand_totals(s: &mut Session, p: &Json) -> Result<Json> {
    let (r, c) = match str_param(p, "mode").map(|m| m.to_ascii_lowercase()) {
        Some(m) if m == "off" => (Some(false), Some(false)),
        Some(m) if m == "on" => (Some(true), Some(true)),
        Some(m) if m == "rows" => (Some(true), Some(false)),
        Some(m) if m == "columns" || m == "cols" => (Some(false), Some(true)),
        Some(m) => return Err(bad("pivot.grandTotals", format!("unknown mode `{m}` (off|on|rows|columns)"))),
        None => (bool_param(p, "rows"), bool_param(p, "cols")),
    };
    let mut q = p.as_object().cloned().unwrap_or_default();
    if let Some(v) = r {
        q.insert("grandTotalsRows".into(), json!(v));
    }
    if let Some(v) = c {
        q.insert("grandTotalsCols".into(), json!(v));
    }
    layout(s, &Json::Object(q))
}

fn refresh(s: &mut Session, p: &Json) -> Result<Json> {
    update(s, p, |_, src, _| Ok(json!({"records": src.rows.len()})))
}

fn refresh_all(s: &mut Session, p: &Json) -> Result<Json> {
    let replace = bool_param(p, "replace").unwrap_or(false);
    edit(s, |cx| {
        let mut n = 0;
        let mut errors = vec![];
        for si in 0..cx.wb.sheets.len() {
            let count = cx.wb.sheet(si).map(|sh| sh.pivots.len()).unwrap_or(0);
            for pi in 0..count {
                let name = cx.wb.sheet(si).and_then(|sh| sh.pivots.get(pi)).map(|p| p.name.clone()).unwrap_or_default();
                match pv::refresh(&mut cx.wb, si, pi, replace) {
                    Ok(_) => n += 1,
                    Err(e) => errors.push(json!({"pivot": name, "error": e})),
                }
            }
        }
        cx.structural = n > 0;
        Ok(json!({"refreshed": n, "errors": errors}))
    })
}

fn change_source(s: &mut Session, p: &Json) -> Result<Json> {
    const C: &str = "pivot.changeSource";
    if str_param(p, "source").or_else(|| str_param(p, "range")).is_none() {
        return Err(bad(C, "missing `source`"));
    }
    let source = source_param(s, p)?;
    let (si, _) = find_pivot(s, p)?;
    let src = pv::read_source(&s.doc()?.wb, &source, si).map_err(other)?;
    update(s, p, move |pt, _, _| {
        pt.source = source.clone();
        let mut dropped: Vec<String> = vec![];
        let keep = |name: &str, dropped: &mut Vec<String>| {
            let ok = src.col(name).is_some();
            if !ok {
                dropped.push(name.to_string());
            }
            ok
        };
        pt.rows.retain(|f| keep(&f.source_col, &mut dropped));
        pt.columns.retain(|f| keep(&f.source_col, &mut dropped));
        pt.values.retain(|f| keep(&f.source_col, &mut dropped));
        pt.filters.retain(|f| keep(&f.source_col, &mut dropped));
        Ok(json!({"source": source, "dropped": dropped}))
    })
}

fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    let (si, pi) = find_pivot(s, p)?;
    edit(s, |cx| {
        // Its PivotCharts become ordinary charts.
        let link = cx.wb.sheet(si).and_then(|sh| sh.pivots.get(pi).map(|pt| (sh.name.clone(), pt.name.clone())));
        if let Some((sheet, name)) = link {
            for i in 0..cx.wb.sheets.len() {
                let linked = |c: &Chart| c.pivot.as_deref().is_some_and(|l| pv::links_to(l, &sheet, &name));
                if cx.wb.sheet(i).is_some_and(|sh| sh.charts.iter().any(linked)) {
                    for c in cx.sheet_mut(i)?.charts.iter_mut().filter(|c| linked(c)) {
                        c.pivot = None;
                    }
                }
            }
        }
        pv::clear_output(&mut cx.wb, si, pi);
        let sh = cx.sheet_mut(si)?;
        if pi < sh.pivots.len() {
            sh.pivots.remove(pi);
        }
        cx.structural = true;
        Ok(json!({}))
    })
}

// ---------------------------------------------------------------- PivotChart

/// The PivotTable a PivotChart call targets: `pivot` may also be a cell of its report.
fn chart_pivot(s: &Session, p: &Json) -> Result<(usize, usize)> {
    if let Some(t) = str_param(p, "pivot") {
        let d = s.doc()?;
        let (sheet, addr) = match t.rsplit_once('!') {
            Some((sh, a)) => {
                let sh = sh.trim();
                let sh = sh.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')).map(|x| x.replace("''", "'")).unwrap_or_else(|| sh.to_string());
                (d.wb.sheet_index(&sh), a)
            }
            None => (Some(d.wb.active_sheet), t),
        };
        if let (Some(si), Some(c)) = (sheet, CellRef::parse(addr.trim().replace('$', "").as_str())) {
            let hit = d.wb.sheet(si).and_then(|sh| sh.pivots.iter().position(|pt| pt.last_range.is_some_and(|r| r.contains(c)) || pt.anchor == c));
            return hit.map(|pi| (si, pi)).ok_or_else(|| bad("insert.pivotChart", format!("{t} is not in a PivotTable")));
        }
    }
    find_pivot(s, p)
}

fn insert_pivot_chart(s: &mut Session, p: &Json) -> Result<Json> {
    let (si, pi) = chart_pivot(s, p)?;
    let d = s.doc()?;
    let sh = d.wb.sheet(si).ok_or(EngineError::NoDocument)?;
    let pt = sh.pivots.get(pi).ok_or_else(|| other("There's no such PivotTable."))?;
    let area = pt.last_range.ok_or_else(|| other("Refresh the PivotTable before adding a PivotChart."))?;
    let out = pv::compute(&d.wb, pt, si).map_err(other)?;
    if out.chart_rows.is_empty() || out.chart_cols.is_empty() {
        return Err(other("Add a field to the Values area and one to Rows or Columns before creating a PivotChart."));
    }
    let series = pv::chart_series(&sh.name, area.start, &out);
    let t = str_param(p, "type").unwrap_or("column");
    let kind = super::insert::chart_kind(t, str_param(p, "subtype").unwrap_or(""));
    let at =
        cell_param(s, p, "at")?.unwrap_or_else(|| CellRef::new(area.start.row, area.end.col.saturating_add(2).min(gridcraft_core::MAX_COLS - 1)));
    let title = match p.get("title") {
        Some(Json::String(t)) => Some(t.clone()),
        Some(Json::Null) => None,
        _ => Some(if series.len() == 1 { "Total".to_string() } else { "Chart Title".to_string() }),
    };
    let id = d.wb.next_object_id();
    let link = pv::chart_link(&sh.name, &pt.name);
    let pivot_name = pt.name.clone();
    let n = series.len();
    let chart = Chart {
        id,
        kind,
        anchor: Anchor {
            cell: at,
            dx: 0.0,
            dy: 0.0,
            width: f64_param(p, "width").unwrap_or(480.0).clamp(10.0, 10_000.0) as f32,
            height: f64_param(p, "height").unwrap_or(288.0).clamp(10.0, 10_000.0) as f32,
            mode: anchor_mode_param(p).unwrap_or_default(),
        },
        title,
        series,
        legend: if matches!(kind, ChartKind::Pie | ChartKind::Doughnut) { LegendPos::Right } else { LegendPos::Bottom },
        data_labels: false,
        gridlines: !matches!(kind, ChartKind::Pie | ChartKind::Doughnut | ChartKind::Treemap | ChartKind::Sunburst | ChartKind::Funnel),
        style: 1,
        x_title: None,
        y_title: None,
        source: None,
        by_rows: false,
        pivot: Some(link),
    };
    edit(s, |cx| {
        cx.sheet_mut(si)?.charts.push(chart);
        Ok(json!({"chart": id, "pivot": pivot_name, "series": n}))
    })
}

// ---------------------------------------------------------------- recommended

fn suggestions(wb: &Workbook, src: &pv::Source) -> Vec<Json> {
    let n = src.headers.len();
    let h = |i: usize| src.headers.get(i).cloned().unwrap_or_default();
    let distinct = |i: usize| pv::distinct_labels(wb, src, i, pv::Grouping::default(), 1001).len();
    let records = src.rows.len().max(1);
    let is_num = |i: usize| src.numeric.get(i).copied().unwrap_or(false);
    let is_date = |i: usize| src.dates.get(i).copied().unwrap_or(false);
    // Category columns: text with repeats.
    let cats: Vec<usize> = (0..n)
        .filter(|&i| {
            !is_num(i) && {
                let d = distinct(i);
                d >= 1 && (d < records || records <= 2) && d <= 1000
            }
        })
        .collect();
    let nums: Vec<usize> = (0..n).filter(|&i| is_num(i) && !is_date(i)).collect();
    let dates: Vec<usize> = (0..n).filter(|&i| is_date(i)).collect();
    let mut out = vec![];
    let mut push = |title: String, rows: Vec<Json>, cols: Vec<Json>, vals: Vec<Json>| {
        if out.len() < 5 {
            let idx = out.len();
            out.push(json!({"index": idx, "title": title, "rows": rows, "columns": cols, "values": vals}));
        }
    };
    if let (Some(&c), Some(&v)) = (cats.first(), nums.first()) {
        push(format!("Sum of {} by {}", h(v), h(c)), vec![json!(h(c))], vec![], vec![json!({"field": h(v), "func": "sum"})]);
    }
    if let (Some(&c), Some(&c2), Some(&v)) = (cats.first(), cats.get(1), nums.first()) {
        push(
            format!("Sum of {} by {} and {}", h(v), h(c), h(c2)),
            vec![json!(h(c))],
            vec![json!(h(c2))],
            vec![json!({"field": h(v), "func": "sum"})],
        );
    }
    if let (Some(&d), Some(&v)) = (dates.first(), nums.first()) {
        push(
            format!("Sum of {} by {} (quarters)", h(v), h(d)),
            vec![json!({"field": h(d), "group": "quarters"})],
            vec![],
            vec![json!({"field": h(v), "func": "sum"})],
        );
    }
    if let (Some(&c), Some(&v), Some(&v2)) = (cats.first(), nums.first(), nums.get(1)) {
        push(
            format!("Sum of {} and {} by {}", h(v), h(v2), h(c)),
            vec![json!(h(c))],
            vec![],
            vec![json!({"field": h(v), "func": "sum"}), json!({"field": h(v2), "func": "sum"})],
        );
    }
    if let Some(&c) = cats.first() {
        push(format!("Count of {} by {}", h(c), h(c)), vec![json!(h(c))], vec![], vec![json!({"field": h(c), "func": "count"})]);
    }
    if let (Some(&c), Some(&v)) = (cats.get(1), nums.first()) {
        push(format!("Average of {} by {}", h(v), h(c)), vec![json!(h(c))], vec![], vec![json!({"field": h(v), "func": "average"})]);
    }
    if cats.is_empty()
        && dates.is_empty()
        && let Some(&v) = nums.first()
    {
        push(format!("Sum of {}", h(v)), vec![], vec![], vec![json!({"field": h(v), "func": "sum"})]);
    }
    out
}

fn recommended(s: &mut Session, p: &Json) -> Result<Json> {
    let source = source_param(s, p)?;
    let active = s.doc()?.wb.active_sheet;
    let src = pv::read_source(&s.doc()?.wb, &source, active).map_err(other)?;
    let list = suggestions(&s.doc()?.wb, &src);
    let Some(apply) = p.get("apply").and_then(Json::as_u64) else {
        return Ok(json!({"source": source, "suggestions": list}));
    };
    let pick = list.get(apply as usize).cloned().ok_or_else(|| bad("insert.recommendedPivotTables", "no suggestion with that index"))?;
    let mut q = json!({
        "source": source,
        "rows": pick.get("rows").cloned().unwrap_or(json!([])),
        "columns": pick.get("columns").cloned().unwrap_or(json!([])),
        "values": pick.get("values").cloned().unwrap_or(json!([])),
    });
    if let (Some(m), Some(d)) = (q.as_object_mut(), p.get("destination")) {
        m.insert("destination".into(), d.clone());
    }
    if let (Some(m), Some(r)) = (q.as_object_mut(), p.get("replace")) {
        m.insert("replace".into(), r.clone());
    }
    let mut res = insert_pivot(s, &q)?;
    if let Some(m) = res.as_object_mut() {
        m.insert("suggestion".into(), pick);
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use gridcraft_core::{CellError, CellRef, Value};
    use serde_json::{Value as Json, json};

    use crate::Session;

    /// Region, Product, Date, Units, Sales — 12 records over 2026.
    fn sales() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        let rows = json!([
            ["Region", "Product", "Date", "Units", "Sales"],
            ["East", "Apples", "1/15/2026", 10, 100],
            ["West", "Apples", "2/10/2026", 5, 50],
            ["East", "Pears", "3/3/2026", 8, 120],
            ["North", "Pears", "4/20/2026", 2, 30],
            ["West", "Plums", "5/5/2026", 7, 70],
            ["East", "Plums", "6/30/2026", 4, 60],
            ["North", "Apples", "7/4/2026", 9, 90],
            ["West", "Pears", "8/8/2026", 3, 45],
            ["East", "Apples", "9/9/2026", 6, 60],
            ["North", "Plums", "10/10/2026", 1, 15],
            ["West", "Apples", "11/11/2026", 12, 120],
            ["East", "Pears", "12/12/2026", 5, 75]
        ]);
        s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
        s.execute("selection.set", json!({"range": "A1"})).unwrap();
        s
    }
    const TOTAL: f64 = 835.0;

    fn val(s: &Session, a: &str) -> Value {
        s.doc().unwrap().wb.active().unwrap().value(CellRef::parse(a).unwrap())
    }
    fn text(s: &Session, a: &str) -> String {
        val(s, a).display()
    }
    fn pt(s: &Session) -> gridcraft_model::PivotTable {
        s.doc().unwrap().wb.active().unwrap().pivots.last().unwrap().clone()
    }
    /// Finds the row (0-based) whose first report column holds `label`.
    fn find_row(s: &Session, label: &str) -> Option<u32> {
        let p = pt(s);
        let r = p.last_range.unwrap();
        (r.start.row..=r.end.row).find(|row| val(s, &CellRef::new(*row, r.start.col).a1()).display() == label)
    }

    #[test]
    fn insert_creates_placeholder_on_new_sheet() {
        let mut s = sales();
        let r = s.execute("insert.pivotTable", json!({})).unwrap();
        assert_eq!(r["name"], "PivotTable1");
        assert_eq!(r["anchor"], "A3");
        assert_eq!(r["fields"], json!(["Region", "Product", "Date", "Units", "Sales"]));
        let d = s.doc().unwrap();
        assert_eq!(d.wb.sheets.len(), 2);
        assert_eq!(d.wb.active_sheet, 0, "new sheet goes before the current one");
        assert_eq!(d.wb.active().unwrap().name, "Sheet2");
        assert!(text(&s, "A3").starts_with("PivotTable1"));
        assert_eq!(pt(&s).source, "Sheet1!$A$1:$E$13");
        // One undo step removes it.
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb.sheets.len(), 1);
    }

    #[test]
    fn every_show_values_as_mode() {
        // Sum of Sales, Region down (East, North, West), Product across (Apples, Pears, Plums):
        //          Apples Pears Plums Total
        //   East     160   195    60   415
        //   North     90    30    15   135
        //   West     170    45    70   285
        //   Total    420   270   145   835
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "columns": ["Product"], "values": ["Sales"]})).unwrap();
        let close = |s: &Session, a: &str, want: f64| {
            let got = val(s, a).as_f64().unwrap_or_else(|| panic!("{a} = {:?}", val(s, a)));
            assert!((got - want).abs() < 1e-12, "{a}: {got} vs {want}");
        };
        let set = |s: &mut Session, p: serde_json::Value| {
            let mut p = p;
            p["field"] = json!("Sum of Sales");
            s.execute("pivot.valueSettings", p).unwrap();
        };
        set(&mut s, json!({"showAs": "percentOf", "baseField": "Region", "baseItem": "East"}));
        close(&s, "B6", 90.0 / 160.0);
        close(&s, "B5", 1.0);
        assert_eq!(val(&s, "B8"), Value::Empty, "the grand total row has no Region item");
        let st = s.execute("cell.get", json!({"cell": "B6"})).unwrap();
        assert_eq!(st["style"]["num_fmt"], "0.00%");
        set(&mut s, json!({"showAs": "differenceFrom", "baseItem": "(previous)"}));
        close(&s, "B6", 90.0 - 160.0);
        assert_eq!(val(&s, "B5"), Value::Empty, "East has no previous item");
        set(&mut s, json!({"showAs": "percentDifferenceFrom", "baseItem": "East"}));
        close(&s, "C7", (45.0 - 195.0) / 195.0);
        set(&mut s, json!({"showAs": "runningTotal"}));
        close(&s, "B7", 160.0 + 90.0 + 170.0);
        set(&mut s, json!({"showAs": "percentRunningTotal"}));
        close(&s, "B6", (160.0 + 90.0) / 420.0);
        set(&mut s, json!({"showAs": "rankAscending"}));
        close(&s, "B6", 1.0);
        close(&s, "B7", 3.0);
        set(&mut s, json!({"showAs": "rankDescending"}));
        close(&s, "B7", 1.0);
        set(&mut s, json!({"showAs": "index"}));
        close(&s, "B5", 160.0 * 835.0 / (415.0 * 420.0));
        set(&mut s, json!({"showAs": "percentOfParentRowTotal"}));
        close(&s, "B5", 160.0 / 420.0);
        set(&mut s, json!({"showAs": "percentOfParentColumnTotal"}));
        close(&s, "B5", 160.0 / 415.0);
        // % of Parent Total over nested rows: Region > Product, each product as a share of its
        // region.
        s.execute("pivot.moveField", json!({"field": "Product", "from": "columns", "to": "rows"})).unwrap();
        set(&mut s, json!({"showAs": "percentOfParentTotal", "baseField": "Region"}));
        let row = find_row(&s, "Apples").unwrap();
        close(&s, &CellRef::new(row, 1).a1(), 160.0 / 415.0);
        assert!(s.execute("pivot.valueSettings", json!({"field": "Sum of Sales", "baseField": "Units"})).is_err());
        // Through xlsx.
        set(&mut s, json!({"showAs": "percentOf", "baseField": "Region", "baseItem": "North"}));
        let want = pt(&s).values;
        let b64 = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap()["base64"].as_str().unwrap().to_string();
        s.execute("file.open", json!({"name": "p.xlsx", "base64": b64})).unwrap();
        s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
        assert_eq!(pt(&s).values, want);
    }

    #[test]
    fn region_by_product_sum_of_sales() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({})).unwrap();
        s.execute("pivot.addField", json!({"field": "region", "area": "rows"})).unwrap();
        s.execute("pivot.addField", json!({"field": "Product", "area": "columns"})).unwrap();
        let r = s.execute("pivot.addField", json!({"field": "Sales", "area": "values"})).unwrap();
        assert_eq!(r["range"], "A3:E8");
        assert_eq!(text(&s, "A3"), "Sum of Sales");
        assert_eq!(text(&s, "B3"), "Column Labels");
        assert_eq!(text(&s, "A4"), "Row Labels");
        assert_eq!(text(&s, "B4"), "Apples");
        assert_eq!(text(&s, "C4"), "Pears");
        assert_eq!(text(&s, "D4"), "Plums");
        assert_eq!(text(&s, "E4"), "Grand Total");
        assert_eq!(text(&s, "A5"), "East");
        assert_eq!(text(&s, "A6"), "North");
        assert_eq!(text(&s, "A7"), "West");
        assert_eq!(text(&s, "A8"), "Grand Total");
        assert_eq!(val(&s, "B5"), Value::Number(160.0));
        assert_eq!(val(&s, "C5"), Value::Number(195.0));
        assert_eq!(val(&s, "E5"), Value::Number(415.0));
        assert_eq!(val(&s, "E8"), Value::Number(TOTAL));
        // Column grand totals add up too.
        let sum: f64 = ["B8", "C8", "D8"].iter().map(|a| val(&s, a).as_f64().unwrap()).sum();
        assert_eq!(sum, TOTAL);
        let rows: f64 = ["E5", "E6", "E7"].iter().map(|a| val(&s, a).as_f64().unwrap()).sum();
        assert_eq!(rows, TOTAL);
        // Styles: header bold, grand total bold with a top border.
        let d = s.doc().unwrap();
        let sh = d.wb.active().unwrap();
        assert!(d.wb.styles.get(sh.style_id(CellRef::parse("A4").unwrap())).font.bold);
        let g = d.wb.styles.get(sh.style_id(CellRef::parse("B8").unwrap()));
        assert!(g.font.bold && !g.borders.top.is_none());
        // Listing.
        let l = s.execute("pivot.list", json!({})).unwrap();
        assert_eq!(l[0]["values"][0]["name"], "Sum of Sales");
    }

    #[test]
    fn multiple_value_fields_make_a_values_dimension() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales", {"field": "Units", "func": "average"}]})).unwrap();
        assert_eq!(text(&s, "A3"), "Row Labels");
        assert_eq!(text(&s, "B3"), "Sum of Sales");
        assert_eq!(text(&s, "C3"), "Average of Units");
        assert_eq!(text(&s, "A7"), "Grand Total");
        assert_eq!(val(&s, "B7"), Value::Number(TOTAL));
        assert_eq!(val(&s, "C7"), Value::Number(72.0 / 12.0));
        // With a column field the values become the innermost column level.
        s.execute("pivot.addField", json!({"field": "Product", "area": "columns"})).unwrap();
        assert_eq!(text(&s, "B4"), "Apples");
        assert_eq!(text(&s, "B5"), "Sum of Sales");
        assert_eq!(text(&s, "C5"), "Average of Units");
        assert_eq!(text(&s, "D4"), "Pears");
        assert_eq!(text(&s, "H4"), "Total Sum of Sales");
        assert_eq!(text(&s, "I4"), "Total Average of Units");
        let last = pt(&s).last_range.unwrap();
        assert_eq!(val(&s, &CellRef::new(last.end.row, 7).a1()), Value::Number(TOTAL));
    }

    fn grand_total(s: &Session) -> Value {
        let row = find_row(s, "Grand Total").expect("a Grand Total row");
        val(s, &CellRef::new(row, pt(s).last_range.unwrap().start.col + 1).a1())
    }

    #[test]
    fn label_filters() {
        // Sum of Sales by Region: East 415, North 135, West 285.
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"]})).unwrap();
        let mut label = |op: &str, value: &str, value2: Option<&str>| {
            let mut p = json!({"field": "Region", "op": op, "value": value});
            if let Some(v) = value2 {
                p["value2"] = json!(v);
            }
            s.execute("pivot.labelFilter", p).unwrap();
            grand_total(&s)
        };
        assert_eq!(label("beginsWith", "E", None), Value::Number(415.0));
        assert_eq!(label("contains", "ST", None), Value::Number(700.0), "case does not matter");
        assert_eq!(label("equals", "?orth", None), Value::Number(135.0), "wildcards");
        assert_eq!(label("doesNotEqual", "e*t", None), Value::Number(420.0));
        assert_eq!(label("endsWith", "h", None), Value::Number(135.0));
        assert_eq!(label("doesNotBeginWith", "w", None), Value::Number(550.0));
        assert_eq!(label("greaterThan", "N", None), Value::Number(420.0));
        assert_eq!(label("between", "Z", Some("M")), Value::Number(420.0), "bounds in either order");
        assert_eq!(label("notBetween", "M", Some("Z")), Value::Number(415.0));
        assert!(find_row(&s, "West").is_none());
        // One filter per field: selecting items clears the label filter, and the reverse.
        s.execute("pivot.filter", json!({"field": "Region", "selected": ["North", "West"]})).unwrap();
        assert_eq!(pt(&s).filters.len(), 1);
        assert!(pt(&s).filters[0].rule.is_none());
        assert_eq!(grand_total(&s), Value::Number(420.0));
        s.execute("pivot.labelFilter", json!({"field": "Region", "op": "equals", "value": "East"})).unwrap();
        assert_eq!(pt(&s).filters[0].selected, None);
        assert_eq!(grand_total(&s), Value::Number(415.0));
        // Selecting all items keeps the label filter.
        s.execute("pivot.filter", json!({"field": "Region", "selected": null})).unwrap();
        assert_eq!(grand_total(&s), Value::Number(415.0));
        let info = s.execute("pivot.list", json!({})).unwrap();
        assert_eq!(info[0]["filters"][0]["rule"], json!({"type": "label", "op": "equals", "value": "East", "value2": ""}));
        s.execute("pivot.clearFilter", json!({"field": "Region"})).unwrap();
        assert!(pt(&s).filters.is_empty());
        assert_eq!(grand_total(&s), Value::Number(TOTAL));
        // Numbers compare as numbers.
        s.execute("pivot.addField", json!({"field": "Units", "area": "rows", "position": 0})).unwrap();
        s.execute("pivot.removeField", json!({"field": "Region"})).unwrap();
        s.execute("pivot.labelFilter", json!({"field": "Units", "op": "greaterThanOrEqual", "value": 10})).unwrap();
        assert_eq!(grand_total(&s), Value::Number(220.0));
    }

    #[test]
    fn value_and_top10_filters() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales", "Units"]})).unwrap();
        let mut run = |c: &str, p: Json| {
            s.execute(c, p).unwrap();
            grand_total(&s)
        };
        assert_eq!(run("pivot.valueFilter", json!({"field": "Region", "op": "greaterThan", "value": 200})), Value::Number(700.0));
        assert_eq!(run("pivot.valueFilter", json!({"field": "Region", "op": "between", "value": 300, "value2": 100})), Value::Number(420.0));
        // By another value field: Units are East 33, North 12, West 27.
        let units = json!({"field": "Region", "op": "lessThan", "value": 20, "valueField": "Sum of Units"});
        assert_eq!(run("pivot.valueFilter", units), Value::Number(135.0));
        assert_eq!(run("pivot.top10", json!({"field": "Region", "count": 1})), Value::Number(415.0));
        assert_eq!(run("pivot.top10", json!({"field": "Region", "top": false, "count": 1})), Value::Number(135.0));
        assert_eq!(run("pivot.top10", json!({"field": "Region", "count": 50, "by": "percent"})), Value::Number(700.0));
        assert_eq!(run("pivot.top10", json!({"field": "Region", "count": 400, "by": "sum"})), Value::Number(415.0));
        assert_eq!(run("pivot.top10", json!({"field": "Region", "count": 99})), Value::Number(TOTAL));
        // A renamed value field keeps its filter; a removed one takes the filter with it.
        s.execute("pivot.top10", json!({"field": "Region", "count": 1, "valueField": "Sum of Units"})).unwrap();
        s.execute("pivot.valueSettings", json!({"field": "Sum of Units", "name": "Units Sold"})).unwrap();
        assert_eq!(pt(&s).filters[0].rule.as_ref().and_then(|r| r.value_field()), Some("Units Sold"));
        s.execute("pivot.removeField", json!({"field": "Units Sold", "area": "values"})).unwrap();
        assert!(pt(&s).filters.is_empty());
        assert_eq!(grand_total(&s), Value::Number(TOTAL));
        // Within each parent: the top Product of each Region, with totals of the items shown.
        s.execute("pivot.addField", json!({"field": "Product", "area": "rows"})).unwrap();
        s.execute("pivot.top10", json!({"field": "Product", "count": 1})).unwrap();
        assert_eq!(grand_total(&s), Value::Number(195.0 + 90.0 + 170.0));
        let east = find_row(&s, "East").unwrap();
        assert_eq!(val(&s, &CellRef::new(east, 1).a1()), Value::Number(195.0), "East's subtotal counts only Pears");
        assert_eq!(text(&s, &CellRef::new(east + 1, 0).a1()), "Pears");
        // A filter follows its field between rows and columns, and goes with it to the filters area.
        s.execute("pivot.moveField", json!({"field": "Product", "from": "rows", "to": "columns"})).unwrap();
        assert!(pt(&s).filters[0].rule.is_some());
        s.execute("pivot.moveField", json!({"field": "Product", "from": "columns", "to": "filters"})).unwrap();
        assert_eq!(pt(&s).filters[0].rule, None);
        assert_eq!(pt(&s).filters[0].selected, None);
        // On a column field: Apples 420 and Pears 270 pass.
        s.execute("pivot.moveField", json!({"field": "Product", "from": "filters", "to": "columns"})).unwrap();
        s.execute("pivot.valueFilter", json!({"field": "Product", "op": "greaterThanOrEqual", "value": 270})).unwrap();
        let r = find_row(&s, "Grand Total").unwrap();
        assert_eq!(text(&s, "D4"), "Grand Total", "two product columns");
        assert_eq!(val(&s, &CellRef::new(r, 3).a1()), Value::Number(690.0));
        // Clear Filters on the whole PivotTable; a report filter stays, showing all.
        s.execute("pivot.addField", json!({"field": "Date", "area": "filters"})).unwrap();
        s.execute("pivot.filter", json!({"field": "Date", "selected": ["1/15/2026"]})).unwrap();
        s.execute("pivot.clearFilter", json!({})).unwrap();
        assert_eq!(pt(&s).filters.len(), 1);
        assert_eq!(pt(&s).filters[0].source_col, "Date");
        assert_eq!(pt(&s).filters[0].selected, None);
    }

    #[test]
    fn filters_round_trip_and_refusals() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region", "Product"], "columns": [], "values": ["Sales", "Units"]})).unwrap();
        s.execute("pivot.labelFilter", json!({"field": "Region", "op": "doesNotContain", "value": "or"})).unwrap();
        s.execute("pivot.top10", json!({"field": "Product", "top": false, "count": 2, "valueField": "Sum of Units"})).unwrap();
        let want = pt(&s).filters;
        let total = grand_total(&s);
        // West: Pears 3 and Plums 7 units (45 + 70); East: Plums 4 and Pears 13 units (60 + 195).
        assert_eq!(total, Value::Number(370.0));
        let b64 = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap()["base64"].as_str().unwrap().to_string();
        s.execute("file.open", json!({"name": "f.xlsx", "base64": b64})).unwrap();
        s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
        assert_eq!(pt(&s).filters, want);
        s.execute("pivot.refresh", json!({})).unwrap();
        assert_eq!(grand_total(&s), total);
        // Refusals.
        let refused = [
            ("pivot.labelFilter", json!({"field": "Units", "op": "equals", "value": "1"})),
            ("pivot.labelFilter", json!({"field": "Region", "op": "between", "value": "A"})),
            ("pivot.labelFilter", json!({"field": "Region", "op": "near", "value": "A"})),
            ("pivot.labelFilter", json!({"field": "Region", "op": "equals"})),
            ("pivot.valueFilter", json!({"field": "Region", "op": "contains", "value": 1})),
            ("pivot.valueFilter", json!({"field": "Region", "op": "equals", "value": "x"})),
            ("pivot.valueFilter", json!({"field": "Region", "op": "equals", "value": 1, "valueField": "Nope"})),
            ("pivot.top10", json!({"field": "Region", "count": 0})),
            ("pivot.top10", json!({"field": "Region", "count": 1.5})),
            ("pivot.top10", json!({"field": "Region", "count": 120, "by": "percent"})),
            ("pivot.top10", json!({"field": "Region", "by": "rows"})),
            ("pivot.clearFilter", json!({"field": "Nope"})),
        ];
        for (c, p) in refused {
            assert!(s.execute(c, p.clone()).is_err(), "{c} {p}");
        }
        assert_eq!(pt(&s).filters, want, "refusals change nothing");
        s.execute("pivot.removeField", json!({"field": "Sum of Sales", "area": "values"})).unwrap();
        s.execute("pivot.removeField", json!({"field": "Sum of Units", "area": "values"})).unwrap();
        assert!(s.execute("pivot.top10", json!({"field": "Region"})).is_err(), "no value field");
    }

    #[test]
    fn number_range_grouping() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Units"], "values": ["Sales"]})).unwrap();
        // Units in fives: 0-4 (2, 4, 3, 1), 5-9 (5, 8, 7, 9, 6, 5), 10-14 (10, 12).
        s.execute("pivot.group", json!({"field": "Units", "by": "range", "start": 0, "end": 14, "interval": 5})).unwrap();
        assert_eq!(text(&s, "A4"), "0-4");
        assert_eq!(val(&s, "B4"), Value::Number(150.0));
        assert_eq!(text(&s, "A5"), "5-9");
        assert_eq!(val(&s, "B5"), Value::Number(465.0));
        assert_eq!(text(&s, "A6"), "10-14");
        assert_eq!(val(&s, "B6"), Value::Number(220.0));
        assert_eq!(val(&s, "B7"), Value::Number(TOTAL));
        let l = s.execute("pivot.list", json!({})).unwrap();
        assert_eq!(l[0]["rows"][0]["group"], "range");
        assert_eq!(l[0]["rows"][0]["range"], json!({"start": 0.0, "end": 14.0, "interval": 5.0}));
        // Values outside the bounds fall into "<start" and ">end", sorted first and last.
        s.execute("pivot.group", json!({"field": "Units", "by": "range", "start": 3, "end": 9, "interval": 5})).unwrap();
        assert_eq!(text(&s, "A4"), "<3");
        assert_eq!(val(&s, "B4"), Value::Number(45.0));
        assert_eq!(text(&s, "A5"), "3-7");
        assert_eq!(text(&s, "A7"), ">9");
        assert_eq!(val(&s, "B7"), Value::Number(220.0));
        // Fractional widths show both ends; the defaults are the data's bounds, in tens.
        s.execute("pivot.group", json!({"field": "Units", "by": "range", "start": 0, "end": 12, "interval": 2.5})).unwrap();
        assert_eq!(text(&s, "A4"), "0-2.5");
        s.execute("pivot.group", json!({"field": "Units", "by": "range"})).unwrap();
        assert_eq!(text(&s, "A4"), "1-10");
        assert_eq!(val(&s, "B4"), Value::Number(TOTAL - 120.0));
        assert_eq!(text(&s, "A5"), "11-20");
        // Through xlsx.
        let want = pt(&s).rows;
        let b64 = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap()["base64"].as_str().unwrap().to_string();
        s.execute("file.open", json!({"name": "p.xlsx", "base64": b64})).unwrap();
        s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
        assert_eq!(pt(&s).rows, want);
        // Unusable bounds, widths and fields are refused; `none` ungroups.
        for bad in [
            json!({"field": "Units", "by": "range", "interval": 0}),
            json!({"field": "Units", "by": "range", "interval": -5}),
            json!({"field": "Units", "by": "range", "start": 10, "end": 1}),
            json!({"field": "Units", "by": "range", "interval": 1e-9}),
            json!({"field": "Units", "by": "range", "start": "low"}),
            json!({"field": "Region", "by": "range"}),
        ] {
            assert!(s.execute("pivot.group", bad.clone()).is_err(), "{bad}");
        }
        s.execute("pivot.group", json!({"field": "Units", "by": "none"})).unwrap();
        assert_eq!(text(&s, "A4"), "1");
        assert_eq!(pt(&s).rows[0].number_group, None);
    }

    #[test]
    fn date_grouping_by_quarter_and_month() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Date"], "values": ["Sales"]})).unwrap();
        // Ungrouped: one row per date, shown with the source date format.
        assert_eq!(pt(&s).last_range.unwrap().height(), 14);
        s.execute("pivot.group", json!({"field": "Date", "by": "quarters"})).unwrap();
        assert_eq!(text(&s, "A4"), "Qtr1");
        assert_eq!(text(&s, "A7"), "Qtr4");
        assert_eq!(val(&s, "B4"), Value::Number(270.0));
        assert_eq!(val(&s, "B5"), Value::Number(160.0));
        assert_eq!(val(&s, "B8"), Value::Number(TOTAL));
        s.execute("pivot.group", json!({"field": "Date", "by": "months"})).unwrap();
        assert_eq!(text(&s, "A4"), "Jan");
        assert_eq!(text(&s, "A15"), "Dec");
        s.execute("pivot.group", json!({"field": "Date", "by": "years"})).unwrap();
        assert_eq!(text(&s, "A4"), "2026");
        assert!(s.execute("pivot.group", json!({"field": "Region", "by": "years"})).is_err());
    }

    #[test]
    fn filters_and_report_filter_rows() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Product"], "values": ["Sales"], "filters": ["Region"]})).unwrap();
        // Report filter above the body; the body stays at A3.
        assert_eq!(text(&s, "A1"), "Region");
        assert_eq!(text(&s, "B1"), "(All)");
        assert_eq!(text(&s, "A3"), "Row Labels");
        s.execute("pivot.filter", json!({"field": "Region", "selected": ["East"]})).unwrap();
        assert_eq!(text(&s, "B1"), "East");
        let gt = find_row(&s, "Grand Total").unwrap();
        assert_eq!(val(&s, &CellRef::new(gt, 1).a1()), Value::Number(415.0));
        s.execute("pivot.filter", json!({"field": "Region", "selected": ["East", "west"]})).unwrap();
        assert_eq!(text(&s, "B1"), "(Multiple Items)");
        s.execute("pivot.filter", json!({"field": "Region", "selected": null})).unwrap();
        let gt = find_row(&s, "Grand Total").unwrap();
        assert_eq!(val(&s, &CellRef::new(gt, 1).a1()), Value::Number(TOTAL));
        // Hidden items on a row field.
        s.execute("pivot.filter", json!({"field": "Product", "selected": ["Apples"]})).unwrap();
        assert_eq!(text(&s, "A4"), "Apples");
        assert_eq!(text(&s, "A5"), "Grand Total");
        assert_eq!(val(&s, "B5"), Value::Number(420.0));
        assert!(s.execute("pivot.filter", json!({"field": "Product", "selected": []})).is_err());
    }

    #[test]
    fn show_values_as() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"]})).unwrap();
        s.execute("pivot.valueSettings", json!({"field": "Sum of Sales", "showAs": "percentOfGrandTotal"})).unwrap();
        let east = val(&s, "B4").as_f64().unwrap();
        assert!((east - 415.0 / TOTAL).abs() < 1e-12);
        assert_eq!(val(&s, "B7"), Value::Number(1.0));
        assert_eq!(text(&s, "B7"), "1");
        let d = s.doc().unwrap();
        let sh = d.wb.active().unwrap();
        assert_eq!(d.wb.styles.get(sh.style_id(CellRef::parse("B4").unwrap())).num_fmt.as_str(), "0.00%");
        s.execute("pivot.valueSettings", json!({"index": 0, "showAs": "runningTotal"})).unwrap();
        assert_eq!(val(&s, "B4"), Value::Number(415.0));
        assert_eq!(val(&s, "B5"), Value::Number(415.0 + 135.0));
        assert_eq!(val(&s, "B6"), Value::Number(TOTAL));
        s.execute("pivot.valueSettings", json!({"index": 0, "showAs": "rank"})).unwrap();
        assert_eq!(val(&s, "B4"), Value::Number(1.0));
        assert_eq!(val(&s, "B5"), Value::Number(3.0));
        assert_eq!(val(&s, "B6"), Value::Number(2.0));
        s.execute("pivot.valueSettings", json!({"index": 0, "showAs": "normal", "func": "max", "numberFormat": "#,##0"})).unwrap();
        assert_eq!(text(&s, "B3"), "Max of Sales");
        assert_eq!(val(&s, "B4"), Value::Number(120.0));
        assert!(s.execute("pivot.valueSettings", json!({"index": 0, "name": "Sales"})).is_err());
        s.execute("pivot.valueSettings", json!({"index": 0, "name": "Biggest sale"})).unwrap();
        assert_eq!(text(&s, "B3"), "Biggest sale");
    }

    #[test]
    fn compact_vs_tabular_shapes() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region", "Product"], "values": ["Sales"]})).unwrap();
        // Compact: one label column; parents carry subtotals at the top; children indented.
        let r = pt(&s).last_range.unwrap();
        assert_eq!(r.width(), 2);
        // header + 3 regions + 9 (region, product) pairs present + grand total
        assert_eq!(r.height(), 1 + 3 + 9 + 1);
        assert_eq!(text(&s, "A4"), "East");
        assert_eq!(val(&s, "B4"), Value::Number(415.0));
        assert_eq!(text(&s, "A5"), "Apples");
        let d = s.doc().unwrap();
        let sh = d.wb.active().unwrap();
        assert_eq!(d.wb.styles.get(sh.style_id(CellRef::parse("A5").unwrap())).align.indent, 1);
        assert!(d.wb.styles.get(sh.style_id(CellRef::parse("A4").unwrap())).font.bold);
        // Outline: one column per field.
        s.execute("pivot.layout", json!({"layout": "outline"})).unwrap();
        let r = pt(&s).last_range.unwrap();
        assert_eq!(r.width(), 3);
        assert_eq!(text(&s, "A3"), "Region");
        assert_eq!(text(&s, "B3"), "Product");
        assert_eq!(text(&s, "A4"), "East");
        assert_eq!(text(&s, "B5"), "Apples");
        // Tabular: first child on its parent's row; "East Total" rows below each group.
        s.execute("pivot.layout", json!({"layout": "tabular"})).unwrap();
        let r = pt(&s).last_range.unwrap();
        assert_eq!(r.width(), 3);
        assert_eq!(r.height(), 1 + 9 + 3 + 1);
        assert_eq!(text(&s, "A4"), "East");
        assert_eq!(text(&s, "B4"), "Apples");
        assert_eq!(text(&s, "A5"), "");
        assert_eq!(text(&s, "B5"), "Pears");
        assert_eq!(text(&s, "A7"), "East Total");
        assert_eq!(val(&s, "C7"), Value::Number(415.0));
        // No subtotals, no grand totals.
        s.execute("pivot.layout", json!({"subtotals": false, "grandTotalsCols": false})).unwrap();
        assert_eq!(pt(&s).last_range.unwrap().height(), 1 + 9);
        // Collapse East in compact layout: its children disappear.
        s.execute("pivot.layout", json!({"layout": "compact", "subtotals": "top"})).unwrap();
        s.execute("pivot.collapse", json!({"field": "Region", "items": ["East"]})).unwrap();
        assert_eq!(text(&s, "A4"), "East");
        assert_eq!(text(&s, "A5"), "North");
        s.execute("pivot.collapse", json!({"field": "Region", "collapse": false})).unwrap();
        assert_eq!(text(&s, "A5"), "Apples");
    }

    #[test]
    fn refresh_after_source_edits() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"]})).unwrap();
        s.execute("sheet.activate", json!({"sheet": "Sheet1"})).unwrap();
        s.execute("cell.set", json!({"cell": "E2", "input": "1100"})).unwrap();
        // Refresh All picks the change up.
        let r = s.execute("data.refreshAll", json!({})).unwrap();
        assert_eq!(r["refreshed"], 1);
        let d = s.doc().unwrap();
        let si = d.wb.sheet_index("Sheet2").unwrap();
        let sh = d.wb.sheet(si).unwrap();
        assert_eq!(sh.value(CellRef::parse("B4").unwrap()), Value::Number(1415.0));
        assert_eq!(sh.value(CellRef::parse("B7").unwrap()), Value::Number(TOTAL + 1000.0));
        // pivot.refresh by name; change source to fewer rows.
        s.execute("pivot.changeSource", json!({"pivot": "PivotTable1", "source": "Sheet1!A1:E3"})).unwrap();
        let d = s.doc().unwrap();
        let sh = d.wb.sheet(si).unwrap();
        assert_eq!(sh.value(CellRef::parse("A6").unwrap()).display(), "Grand Total");
        assert_eq!(sh.value(CellRef::parse("B6").unwrap()), Value::Number(1150.0));
        // Old rows were cleared.
        assert_eq!(sh.value(CellRef::parse("A7").unwrap()), Value::Empty);
        s.execute("pivot.refresh", json!({"pivot": "PivotTable1"})).unwrap();
    }

    fn pivot_chart(s: &Session) -> gridcraft_model::Chart {
        s.doc().unwrap().wb.active().unwrap().charts.last().unwrap().clone()
    }
    fn plotted(s: &Session) -> Vec<Vec<Option<f64>>> {
        let d = s.doc().unwrap();
        let data = gridcraft_chart::resolve(&d.wb, d.wb.active_sheet, &pivot_chart(s));
        data.series.into_iter().map(|x| x.values).collect()
    }

    #[test]
    fn pivot_chart_follows_the_report() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"]})).unwrap();
        // Any cell of the report picks the PivotTable.
        let r = s.execute("insert.pivotChart", json!({"pivot": "B5"})).unwrap();
        assert_eq!(r["series"], 1);
        let ch = pivot_chart(&s);
        assert_eq!(ch.pivot.as_deref(), Some("Sheet2!PivotTable1"));
        assert_eq!(ch.title.as_deref(), Some("Total"));
        assert_eq!(ch.series[0].values, "Sheet2!$B$4:$B$6");
        assert_eq!(ch.series[0].categories.as_deref(), Some("Sheet2!$A$4:$A$6"));
        assert_eq!(plotted(&s), vec![vec![Some(415.0), Some(135.0), Some(285.0)]]);
        // A column field: one series per item, rebound by the layout change.
        s.execute("pivot.addField", json!({"field": "Product", "area": "columns"})).unwrap();
        let ch = pivot_chart(&s);
        let names: Vec<_> = ch.series.iter().map(|x| x.name.clone().unwrap_or_default()).collect();
        assert_eq!(names, vec!["\"Apples\"", "\"Pears\"", "\"Plums\""]);
        assert_eq!(plotted(&s)[0], vec![Some(160.0), Some(90.0), Some(170.0)]);
        // Nested row fields: the innermost items only, as a union around the parent rows.
        s.execute("pivot.moveField", json!({"field": "Product", "from": "columns", "to": "rows"})).unwrap();
        let ch = pivot_chart(&s);
        assert!(ch.series[0].values.starts_with('('), "{}", ch.series[0].values);
        let pts = plotted(&s);
        assert_eq!(pts[0].len(), 9);
        assert_eq!(pts[0].iter().flatten().sum::<f64>(), TOTAL);
        // Survives an XLSX round trip as a PivotChart.
        let wb = (*s.doc().unwrap().wb).clone();
        let (back, _) = gridcraft_xlsx::read_xlsx(&gridcraft_xlsx::write_xlsx(&wb).unwrap()).unwrap();
        let si = back.sheet_index("Sheet2").unwrap();
        assert_eq!(back.sheet(si).unwrap().charts.first().and_then(|c| c.pivot.clone()).as_deref(), Some("Sheet2!PivotTable1"));
        // Renaming the sheet keeps the link; a refresh still rebinds.
        s.execute("sheet.rename", json!({"name": "Report"})).unwrap();
        assert_eq!(pivot_chart(&s).pivot.as_deref(), Some("Report!PivotTable1"));
        s.execute("pivot.moveField", json!({"field": "Region", "from": "rows", "to": "columns"})).unwrap();
        assert_eq!(pivot_chart(&s).series.len(), 3);
        // Deleting the PivotTable leaves an ordinary chart.
        s.execute("pivot.delete", json!({})).unwrap();
        assert_eq!(pivot_chart(&s).pivot, None);
    }

    #[test]
    fn pivot_chart_errors() {
        let mut s = sales();
        // No PivotTable yet.
        assert!(s.execute("insert.pivotChart", json!({})).is_err());
        s.execute("insert.pivotTable", json!({"rows": ["Region"]})).unwrap();
        // Not in the report; no values.
        assert!(s.execute("insert.pivotChart", json!({"pivot": "Z99"})).is_err());
        assert!(s.execute("insert.pivotChart", json!({})).unwrap_err().to_string().contains("Values"));
        s.execute("pivot.addField", json!({"field": "Units", "area": "values"})).unwrap();
        let r = s.execute("insert.pivotChart", json!({"pivot": "PivotTable1", "type": "line", "title": null})).unwrap();
        assert_eq!(r["series"], 1);
        let ch = pivot_chart(&s);
        assert_eq!(ch.kind, gridcraft_model::ChartKind::Line);
        assert_eq!(ch.title, None);
        // One undo step.
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.doc().unwrap().wb.active().unwrap().charts.is_empty());
    }

    #[test]
    fn calculated_field_on_sums() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"]})).unwrap();
        let r = s.execute("pivot.calculatedField", json!({"name": "Price", "formula": "Sales / Units"})).unwrap();
        assert_eq!(r["formula"], "=Sales / Units");
        assert_eq!(text(&s, "C3"), "Sum of Price");
        assert_eq!(text(&s, "A4"), "East");
        // East: 415 sales over 33 units, from the sums.
        let east = val(&s, "C4").as_f64().unwrap();
        assert!((east - 415.0 / 33.0).abs() < 1e-12, "{east}");
        // The grand total applies the formula to the totals, not the sum of the rows.
        assert_eq!(text(&s, "A7"), "Grand Total");
        let total = val(&s, "C7").as_f64().unwrap();
        assert!((total - TOTAL / 72.0).abs() < 1e-12, "{total}");
        // Quoted names, functions, and another calculated field.
        s.execute("pivot.calculatedField", json!({"name": "Rounded", "formula": "=ROUND('Price'*2,1)"})).unwrap();
        assert_eq!(val(&s, "D4"), Value::Number(25.2));
        // Division by zero shows #DIV/0! cells.
        s.execute("pivot.calculatedField", json!({"name": "Zero", "formula": "=Sales/(Units-Units)"})).unwrap();
        assert_eq!(val(&s, "E4"), Value::Error(CellError::Div0));
        // Change and delete.
        s.execute("pivot.calculatedField", json!({"name": "zero", "formula": "=Units*2"})).unwrap();
        assert_eq!(val(&s, "E4"), Value::Number(66.0));
        s.execute("pivot.calculatedField", json!({"name": "Zero", "delete": true})).unwrap();
        assert_eq!(pt(&s).values.len(), 3);
        assert_eq!(pt(&s).calculated_fields.len(), 2);
        // The field list shows calculated fields.
        let f = s.execute("pivot.fields", json!({})).unwrap();
        assert!(f["fields"].as_array().unwrap().iter().any(|x| x["name"] == "Price" && x["calculated"] == true));
    }

    #[test]
    fn calculated_field_errors() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"]})).unwrap();
        let mut e = |p: Json| s.execute("pivot.calculatedField", p).unwrap_err().to_string();
        assert!(e(json!({"name": "Bad", "formula": "=Sales*Bogus"})).contains("Bogus"));
        assert!(e(json!({"name": "Bad", "formula": "=A1*2"})).contains("cells"));
        assert!(e(json!({"name": "Bad", "formula": "=Sales*("})).contains("error"));
        assert!(e(json!({"name": "Sales", "formula": "=1"})).contains("already exists"));
        assert!(e(json!({"name": "Bad"})).contains("formula"));
        assert!(e(json!({"name": "Gone", "delete": true})).contains("Gone"));
        // A cycle is refused.
        s.execute("pivot.calculatedField", json!({"name": "X", "formula": "=Sales"})).unwrap();
        s.execute("pivot.calculatedField", json!({"name": "Y", "formula": "=X*2"})).unwrap();
        let err = s.execute("pivot.calculatedField", json!({"name": "X", "formula": "=Y"})).unwrap_err().to_string();
        assert!(err.contains("itself"), "{err}");
        // Calculated fields only go to Values; nothing changed after the failures.
        assert!(s.execute("pivot.addField", json!({"field": "Y", "area": "rows"})).is_err());
        assert_eq!(pt(&s).calculated_fields.len(), 2);
        // Without a name the dialog opens (menu-style call).
        s.ui_requests.clear();
        s.execute("pivot.calculatedField", json!({})).unwrap();
        assert!(matches!(s.ui_requests.last(), Some(crate::UiRequest::Dialog(n, _)) if n == "pivotCalculatedField"));
    }

    #[test]
    fn replace_protection() {
        let mut s = sales();
        s.execute("cell.set", json!({"cell": "H5", "input": "keep me"})).unwrap();
        let e = s.execute("insert.pivotTable", json!({"destination": "Sheet1!H3", "rows": ["Region"], "values": ["Sales"]})).unwrap_err().to_string();
        assert!(e.contains("There's already data in Sheet1!H5"), "{e}");
        assert_eq!(s.doc().unwrap().wb.active().unwrap().pivots.len(), 0);
        // Growing into data is refused as well.
        s.execute("insert.pivotTable", json!({"destination": "Sheet1!H3"})).unwrap();
        assert!(s.execute("pivot.addField", json!({"field": "Region", "area": "rows"})).is_err());
        s.execute("pivot.addField", json!({"field": "Region", "area": "rows", "replace": true})).unwrap();
        assert_eq!(text(&s, "H5"), "North");
        // A second pivot can't overlap the first.
        let e =
            s.execute("insert.pivotTable", json!({"source": "Sheet1!A1:E13", "destination": "Sheet1!H6", "replace": true})).unwrap_err().to_string();
        assert!(e.contains("overlap"), "{e}");
        // Nor its own source data.
        let e =
            s.execute("insert.pivotTable", json!({"source": "Sheet1!A1:E13", "destination": "Sheet1!B2", "replace": true})).unwrap_err().to_string();
        assert!(e.contains("source data"), "{e}");
        // Delete clears the report.
        s.execute("pivot.delete", json!({})).unwrap();
        assert_eq!(val(&s, "H5"), Value::Empty);
        assert!(s.doc().unwrap().wb.active().unwrap().pivots.is_empty());
    }

    #[test]
    fn fields_move_remove_sort() {
        let mut s = sales();
        s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"]})).unwrap();
        let f = s.execute("pivot.fields", json!({})).unwrap();
        assert_eq!(f["fields"][0]["name"], "Region");
        assert_eq!(f["fields"][0]["samples"], json!(["East", "North", "West"]));
        assert_eq!(f["fields"][0]["areas"], json!(["rows"]));
        assert_eq!(f["fields"][4]["numeric"], true);
        assert_eq!(f["fields"][2]["date"], true);
        s.execute("pivot.sort", json!({"field": "Region", "order": "desc"})).unwrap();
        assert_eq!(text(&s, "A4"), "West");
        s.execute("pivot.moveField", json!({"field": "Region", "from": "rows", "to": "columns"})).unwrap();
        assert_eq!(text(&s, "B4"), "West");
        s.execute("pivot.removeField", json!({"field": "Sum of Sales", "area": "values"})).unwrap();
        s.execute("pivot.addField", json!({"field": "Region", "area": "values"})).unwrap();
        assert_eq!(pt(&s).values[0].name, "Count of Region");
        s.execute("pivot.removeField", json!({"field": "Region"})).unwrap();
        assert!(pt(&s).columns.is_empty() && pt(&s).values.is_empty());
        assert!(text(&s, "A3").contains("choose fields"));
        assert!(s.execute("pivot.addField", json!({"field": "Nope", "area": "rows"})).is_err());
        assert!(s.execute("pivot.addField", json!({"field": "Region", "area": "sideways"})).is_err());
    }

    #[test]
    fn recommended_and_summarize() {
        let mut s = sales();
        let r = s.execute("insert.recommendedPivotTables", json!({})).unwrap();
        let list = r["suggestions"].as_array().unwrap();
        assert!((3..=5).contains(&list.len()), "{list:?}");
        assert_eq!(list[0]["title"], "Sum of Units by Region");
        let r = s.execute("insert.recommendedPivotTables", json!({"apply": 2})).unwrap();
        assert!(r["suggestion"]["title"].as_str().unwrap().contains("quarters"));
        assert_eq!(text(&s, "A4"), "Qtr1");
        // From a table.
        s.execute("sheet.activate", json!({"sheet": "Sheet1"})).unwrap();
        s.execute("insert.table", json!({"range": "A1:E13"})).unwrap();
        s.execute("selection.set", json!({"range": "B2"})).unwrap();
        let r = s.execute("table.summarizePivot", json!({"rows": ["Product"], "values": ["Units"]})).unwrap();
        assert_eq!(pt(&s).source, "Table1");
        assert_eq!(r["name"], "PivotTable2");
        let gt = find_row(&s, "Grand Total").unwrap();
        assert_eq!(val(&s, &CellRef::new(gt, 1).a1()), Value::Number(72.0));
    }

    #[test]
    fn repeated_headers_get_numbered_names() {
        let mut s = Session::new();
        s.new_workbook();
        let mut header = vec![json!("a"), json!("A"), json!("a"), json!("a2")];
        // Thousands of equal headers (a row of check boxes, say) used to take cubic time; this would
        // hang rather than fail a wall-clock check, which is flaky on a loaded machine.
        header.extend((0..3000).map(|_| json!("x")));
        let data: Vec<_> = (0..header.len()).map(|i| json!(i)).collect();
        let end = CellRef::new(1, header.len() as u32 - 1).a1();
        s.execute("range.setValues", json!({"range": "A1", "values": [header, data]})).unwrap();
        let src = crate::pivot::read_source(&s.doc().unwrap().wb, &format!("Sheet1!A1:{end}"), 0).unwrap();
        assert_eq!(&src.headers[..4], ["a", "A2", "a3", "a22"]);
        assert_eq!(src.headers[4], "x");
        assert_eq!(src.headers.last().map(String::as_str), Some("x3000"));
        // Table column names follow the same rule.
        s.execute("insert.table", json!({"range": format!("A1:{end}"), "header": true})).unwrap();
        let text = |s: &mut Session, a: &str| s.execute("cell.get", json!({"cell": a})).unwrap()["text"].clone();
        assert_eq!(text(&mut s, "B1"), "A2");
        let last = CellRef::new(0, 3003).a1();
        assert_eq!(text(&mut s, &last), "x3000");
    }

    #[test]
    fn hostile_params_never_panic() {
        let mut s = sales();
        let cmds = [
            "insert.pivotTable",
            "insert.recommendedPivotTables",
            "table.summarizePivot",
            "pivot.fields",
            "pivot.fieldList",
            "pivot.list",
            "pivot.addField",
            "pivot.removeField",
            "pivot.moveField",
            "pivot.valueSettings",
            "pivot.filter",
            "pivot.labelFilter",
            "pivot.valueFilter",
            "pivot.top10",
            "pivot.clearFilter",
            "pivot.sort",
            "pivot.group",
            "pivot.collapse",
            "pivot.layout",
            "pivot.grandTotals",
            "pivot.subtotals",
            "pivot.reportLayout",
            "pivot.style",
            "pivot.refresh",
            "data.refreshAll",
            "pivot.changeSource",
            "pivot.delete",
        ];
        let hostile = [
            json!({}),
            json!([1, 2]),
            json!({"pivot": 99999, "field": 5, "area": null}),
            json!({"source": "ZZZ999999999", "destination": "Nope!A1"}),
            json!({"source": "A:XFD", "destination": "Sheet1!XFD1048576"}),
            json!({"source": "''!A1", "destination": "!"}),
            json!({"field": "Sales", "area": "values", "func": "bogus", "position": 4294967295u64}),
            json!({"field": "Region", "selected": [null, 1, {"x": 1}], "index": 18446744073709551615u64, "showAs": "rank"}),
            json!({"apply": 99, "layout": "weird", "subtotals": 7, "style": "x".repeat(500)}),
            json!({"field": "Date", "by": "decades", "items": "nope", "order": "sideways"}),
            json!({"field": "Region", "op": "between", "value": 1e308, "value2": "x", "count": -1, "by": "sum", "top": "x", "valueField": 7}),
            json!({"field": "Region", "op": "notBetween", "value": ["x"], "count": 1e300, "by": "percent", "valueField": "nope"}),
        ];
        for c in cmds {
            for h in &hostile {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let spec = crate::find_command(c).unwrap();
                    let _ = (spec.run)(&mut s, h);
                }));
                assert!(r.is_ok(), "{c} panicked on {h}");
            }
        }
        // And on a real pivot.
        s.execute("insert.pivotTable", json!({"source": "Sheet1!A1:E13", "rows": ["Region"], "values": ["Sales"]})).unwrap();
        for c in cmds {
            for h in &hostile {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let spec = crate::find_command(c).unwrap();
                    let _ = (spec.run)(&mut s, h);
                }));
                assert!(r.is_ok(), "{c} panicked on {h}");
            }
        }
    }

    #[test]
    fn empty_header_is_refused() {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Region", ""], ["East", 1]]})).unwrap();
        let e = s.execute("insert.pivotTable", json!({"source": "A1:B2"})).unwrap_err().to_string();
        assert!(e.contains("field name is not valid"), "{e}");
        assert!(s.execute("insert.pivotTable", json!({"source": "NoSuchTable"})).is_err());
    }
}
