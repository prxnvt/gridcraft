//! Data › Analysis › Data Analysis: the Analysis ToolPak. Each tool is a command
//! (`analysis.<tool>`) taking an input range and an output destination; the statistics live in
//! `gridcraft_analysis::toolpak`. Output is written by [`crate::report`] (an output range, a new
//! worksheet or a new workbook).
//!
//! Common parameters: `input` (a range), `groupedBy` (`columns` | `rows`), `labels` (first row /
//! column holds labels), `output` (see [`crate::report::parse_output`]) and `overwrite`.

use gridcraft_analysis::toolpak::{self, Out, Series, V};
use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::ChartKind;
use serde_json::{Value as Json, json};

use super::*;
use crate::report::{self, RChart, RRange, RSeries, RStyle, RValue, Report, Target};

/// The tools, in the order the Data Analysis box lists them: (command, name).
pub const TOOLS: &[(&str, &str)] = &[
    ("analysis.anovaSingleFactor", "Anova: Single Factor"),
    ("analysis.anovaTwoFactorWithReplication", "Anova: Two-Factor With Replication"),
    ("analysis.anovaTwoFactorWithoutReplication", "Anova: Two-Factor Without Replication"),
    ("analysis.correlation", "Correlation"),
    ("analysis.covariance", "Covariance"),
    ("analysis.descriptiveStatistics", "Descriptive Statistics"),
    ("analysis.exponentialSmoothing", "Exponential Smoothing"),
    ("analysis.fTest", "F-Test Two-Sample for Variances"),
    ("analysis.fourierAnalysis", "Fourier Analysis"),
    ("analysis.histogram", "Histogram"),
    ("analysis.movingAverage", "Moving Average"),
    ("analysis.randomNumberGeneration", "Random Number Generation"),
    ("analysis.rankAndPercentile", "Rank and Percentile"),
    ("analysis.regression", "Regression"),
    ("analysis.sampling", "Sampling"),
    ("analysis.tTestPaired", "t-Test: Paired Two Sample for Means"),
    ("analysis.tTestEqualVariances", "t-Test: Two-Sample Assuming Equal Variances"),
    ("analysis.tTestUnequalVariances", "t-Test: Two-Sample Assuming Unequal Variances"),
    ("analysis.zTest", "z-Test: Two Sample for Means"),
];

const COMMON: &str = "input: \"A1:C20\", groupedBy?: columns|rows, labels?: bool, output?: \"E1\" | \"newSheet\" | {newSheet: name} | \"newWorkbook\" (default a new worksheet), overwrite?: bool";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "data.analysisToolPak",
            "Data Analysis",
            ["Data", "Analysis"],
            None,
            "{} → the Analysis ToolPak's tools [{command, name}]; run one with its command",
            has_doc,
            tool_list
        ),
        cmd!(
            "analysis.descriptiveStatistics",
            "Descriptive Statistics",
            [],
            None,
            "{input, groupedBy?, labels?, summary?: true, confidence?: 95 (percent; omit for none), kthLargest?: k, kthSmallest?: k, output?, overwrite?} → mean, standard error, median, mode, standard deviation, variance, kurtosis, skewness, range, min, max, sum, count per variable",
            has_doc,
            descriptive_statistics
        ),
    ]
}

fn tool_list(_s: &mut Session, _p: &Json) -> Result<Json> {
    let tools: Vec<Json> =
        TOOLS.iter().map(|(id, name)| json!({"command": id, "name": name, "available": crate::cmd::find_command(id).is_some()})).collect();
    Ok(json!({ "tools": tools, "common": COMMON }))
}

// ---------------------------------------------------------------- input

/// The input range read as variables.
pub(crate) struct Input {
    pub sheet: usize,
    pub series: Vec<Series>,
    /// Cell of each value of each series (for formulas and chart references).
    pub cells: Vec<Vec<CellRef>>,
}

fn not_numeric() -> EngineError {
    EngineError::Other("The input range must hold only numbers (blank cells are skipped).".into())
}

/// Reads `p[key]` (a range) as variables: one per column, or per row with `groupedBy: rows`;
/// the first row (column) is the labels with `labels: true`. `spaced` picks ANOVA's default names
/// (`Column 1`) over the others' (`Column1`).
pub(crate) fn read_input(s: &Session, p: &Json, key: &str, cmd: &str, spaced: bool) -> Result<Input> {
    let d = s.doc()?;
    let text = str_param(p, key).map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad(cmd, format!("missing `{key}` (the input range)")))?;
    if text.contains(',') {
        return Err(EngineError::Other("The input range must be a single rectangular area.".into()));
    }
    let (sheet, body) = split_sheet(text);
    let si = match sheet {
        Some(n) => d.wb.sheet_index(&n).ok_or_else(|| bad(cmd, format!("no sheet named `{n}`")))?,
        None => d.wb.active_sheet,
    };
    let sh = d.wb.sheet(si).ok_or(EngineError::NoDocument)?;
    let mut r = RangeRef::parse(body).ok_or_else(|| bad(cmd, format!("`{key}` is not a range: `{text}`")))?;
    if r.is_full_cols() || r.is_full_rows() {
        r = match sh.used_range() {
            Some(u) => r.intersection(&RangeRef::new(CellRef::new(0, 0), u.end)).unwrap_or(RangeRef::cell(r.start)),
            None => RangeRef::cell(r.start),
        };
    }
    if r.count() > toolpak::MAX_VALUES as u64 {
        return Err(EngineError::Other(format!("The input range has more than {} cells.", toolpak::MAX_VALUES)));
    }
    let by_rows = str_param(p, "groupedBy").is_some_and(|g| g.eq_ignore_ascii_case("rows"));
    let labels = bool_param(p, "labels").unwrap_or(false);
    let (n_series, n_values) = if by_rows { (r.height(), r.width()) } else { (r.width(), r.height()) };
    let at = |series: u32, i: u32| {
        if by_rows { CellRef::new(r.start.row + series, r.start.col + i) } else { CellRef::new(r.start.row + i, r.start.col + series) }
    };
    let first = u32::from(labels);
    if n_values <= first {
        return Err(EngineError::Other("The input range has no values below its labels.".into()));
    }
    let mut series = Vec::with_capacity(n_series as usize);
    let mut cells = Vec::with_capacity(n_series as usize);
    for k in 0..n_series {
        let label = if labels {
            let v = sh.value(at(k, 0));
            match v {
                Value::Empty => toolpak::default_label(by_rows, k as usize, spaced),
                v => v.display(),
            }
        } else {
            toolpak::default_label(by_rows, k as usize, spaced)
        };
        let mut values = Vec::with_capacity((n_values - first) as usize);
        let mut refs = Vec::with_capacity((n_values - first) as usize);
        for i in first..n_values {
            let c = at(k, i);
            values.push(match sh.value(c) {
                Value::Number(x) => Some(x),
                Value::Empty => None,
                _ => return Err(not_numeric()),
            });
            refs.push(c);
        }
        series.push(Series::new(label, values));
        cells.push(refs);
    }
    Ok(Input { sheet: si, series, cells })
}

/// A number parameter with a default, refusing non-numbers.
pub(crate) fn num_param(p: &Json, key: &str, default: f64, cmd: &str) -> Result<f64> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(default),
        Some(Json::Number(n)) => n.as_f64().ok_or_else(|| bad(cmd, format!("`{key}` must be a number"))),
        Some(Json::String(t)) if t.trim().is_empty() => Ok(default),
        Some(Json::String(t)) => t.trim().parse::<f64>().map_err(|_| bad(cmd, format!("`{key}` must be a number"))),
        Some(_) => Err(bad(cmd, format!("`{key}` must be a number"))),
    }
}

/// An optional positive whole-number parameter.
pub(crate) fn count_param(p: &Json, key: &str, cmd: &str) -> Result<Option<usize>> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(t)) if t.trim().is_empty() => Ok(None),
        Some(Json::Bool(false)) => Ok(None),
        _ => {
            let x = num_param(p, key, 0.0, cmd)?;
            if x >= 1.0 && x.fract() == 0.0 && x <= 1e9 {
                Ok(Some(x as usize))
            } else {
                Err(bad(cmd, format!("`{key}` must be a whole number of at least 1")))
            }
        }
    }
}

// ---------------------------------------------------------------- output

fn tool_error(e: gridcraft_analysis::Error) -> EngineError {
    EngineError::Other(e.to_string())
}

fn area(a: &toolpak::Area) -> RRange {
    match a {
        toolpak::Area::Out { row, col, rows, cols } => RRange::Out { row: *row, col: *col, rows: *rows, cols: *cols },
        toolpak::Area::Ref(t) => RRange::Text(t.clone()),
    }
}

/// Converts a tool's output into a report.
pub(crate) fn to_report(out: Out) -> Report {
    let mut rep = Report::new();
    for (row, col, v, look) in out.cells {
        let style = RStyle {
            bold: look.bold,
            italic: look.italic,
            center: look.center,
            top: look.top,
            bottom: look.bottom,
            num_fmt: look.fmt.map(str::to_string),
        };
        let value = match v {
            V::Num(x) => RValue::Value(Value::Number(x)),
            V::Text(t) => RValue::Value(Value::text(t)),
            V::Formula(f) => RValue::Formula(f),
            V::Error(e) => RValue::Value(Value::Error(e)),
        };
        rep.cells.push(report::RCell { row, col, value, style });
    }
    for ch in out.charts {
        let kind = match ch.kind {
            toolpak::ChartKind::Column if ch.series.iter().any(|s| s.line) => ChartKind::Combo,
            toolpak::ChartKind::Column => ChartKind::ColumnClustered,
            toolpak::ChartKind::Line => ChartKind::Line,
            toolpak::ChartKind::Scatter => ChartKind::Scatter,
        };
        rep.charts.push(RChart {
            kind,
            title: Some(ch.title),
            x_title: ch.x_title,
            y_title: ch.y_title,
            series: ch
                .series
                .iter()
                .map(|s| RSeries { name: s.name.clone(), categories: s.x.as_ref().map(area), values: area(&s.y), line: s.line })
                .collect(),
            row: ch.row,
            col: ch.col,
            width: ch.width,
            height: ch.height,
            legend: ch.legend,
        });
    }
    rep
}

/// Output cells may not overlap the input.
pub(crate) fn check_overlap(target: &Target, input: &Input, out: &Out) -> Result<()> {
    if let report::OutputKind::Range { sheet } = target.kind
        && sheet == input.sheet
    {
        let (rows, cols) = out.extent();
        let area = RangeRef::new(target.origin, CellRef::new(target.origin.row.saturating_add(rows - 1), target.origin.col.saturating_add(cols - 1)));
        if input.cells.iter().flatten().any(|c| area.contains(*c)) {
            return Err(EngineError::Other("The output range overlaps the input range.".into()));
        }
    }
    Ok(())
}

fn run_with_input(
    s: &mut Session,
    p: &Json,
    cmd: &str,
    input: &Input,
    tool: impl FnOnce(&Target) -> gridcraft_analysis::Result<Out>,
) -> Result<Json> {
    let target = report::parse_output(s, p, cmd)?;
    let out = tool(&target).map_err(tool_error)?;
    check_overlap(&target, input, &out)?;
    report::write(s, &target, to_report(out), report::overwrite_param(p))
}

// ---------------------------------------------------------------- tools

fn descriptive_statistics(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.descriptiveStatistics";
    let input = read_input(s, p, "input", CMD, false)?;
    let confidence = match p.get("confidence") {
        None | Some(Json::Null) | Some(Json::Bool(false)) => None,
        Some(Json::Bool(true)) => Some(95.0),
        _ => Some(num_param(p, "confidence", 95.0, CMD)?),
    };
    let o = toolpak::descriptive::DescriptiveOptions {
        summary: bool_param(p, "summary").unwrap_or(true),
        confidence,
        kth_largest: count_param(p, "kthLargest", CMD)?,
        kth_smallest: count_param(p, "kthSmallest", CMD)?,
    };
    run_with_input(s, p, CMD, &input, |_| toolpak::descriptive::descriptive_statistics(&input.series, &o))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn session() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    pub(crate) fn val(s: &Session, sheet: &str, a1: &str) -> Value {
        let d = s.doc().unwrap();
        let si = d.wb.sheet_index(sheet).unwrap();
        d.wb.sheet(si).unwrap().value(CellRef::parse(a1).unwrap())
    }

    #[test]
    fn lists_the_tools() {
        let mut s = session();
        let r = s.execute("data.analysisToolPak", json!({})).unwrap();
        assert_eq!(r["tools"].as_array().unwrap().len(), 19);
        assert_eq!(r["tools"][5]["name"], "Descriptive Statistics");
        assert_eq!(r["tools"][5]["available"], true);
    }

    #[test]
    fn descriptive_statistics_to_a_new_sheet_and_a_range() {
        let mut s = session();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Score", "Age"], [1, 30], [2, 40], [3, null], [4, 50]]})).unwrap();
        let r = s
            .execute("analysis.descriptiveStatistics", json!({"input": "A1:B5", "labels": true, "confidence": 95, "output": {"newSheet": "Stats"}}))
            .unwrap();
        assert_eq!(r["sheet"], "Stats");
        assert_eq!(val(&s, "Stats", "A1"), Value::from("Score"));
        assert_eq!(val(&s, "Stats", "A2"), Value::from("Mean"));
        assert_eq!(val(&s, "Stats", "B2"), Value::Number(2.5));
        assert_eq!(val(&s, "Stats", "C1"), Value::from("Age"));
        assert_eq!(val(&s, "Stats", "D14"), Value::Number(3.0), "Count skips the blank");
        assert_eq!(val(&s, "Stats", "C15"), Value::from("Confidence Level(95.0%)"));
        // Into a range on the input sheet; overlapping the input is refused; writing over data asks.
        s.execute("sheet.activate", json!({"sheet": "Sheet1"})).unwrap();
        assert!(s.execute("analysis.descriptiveStatistics", json!({"input": "A1:A5", "labels": true, "output": "A3"})).is_err());
        s.execute("analysis.descriptiveStatistics", json!({"input": "A1:A5", "labels": true, "output": "$F$1"})).unwrap();
        assert_eq!(val(&s, "Sheet1", "G2"), Value::Number(2.5));
        let again = s.execute("analysis.descriptiveStatistics", json!({"input": "A1:A5", "labels": true, "output": "F1"}));
        assert!(matches!(again, Err(EngineError::Confirm(_))));
        s.execute("analysis.descriptiveStatistics", json!({"input": "A1:A5", "labels": true, "output": "F1", "overwrite": true})).unwrap();
        // Grouped by rows, default labels.
        s.execute("range.setValues", json!({"range": "A10", "values": [[1, 2, 3], [4, 5, 6]]})).unwrap();
        s.execute("analysis.descriptiveStatistics", json!({"input": "A10:C11", "groupedBy": "rows", "output": "newSheet"})).unwrap();
        let name = s.doc().unwrap().wb.active().unwrap().name.clone();
        assert_eq!(val(&s, &name, "A1"), Value::from("Row1"));
        assert_eq!(val(&s, &name, "D2"), Value::Number(5.0));
        // Text in the input is refused.
        s.execute("sheet.activate", json!({"sheet": "Sheet1"})).unwrap();
        s.execute("cell.set", json!({"cell": "A12", "input": "x"})).unwrap();
        assert!(s.execute("analysis.descriptiveStatistics", json!({"input": "A10:C12"})).is_err());
        assert!(s.execute("analysis.descriptiveStatistics", json!({})).is_err());
    }
}
