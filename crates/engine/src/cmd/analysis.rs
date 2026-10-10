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
        cmd!(
            "analysis.rankAndPercentile",
            "Rank and Percentile",
            [],
            None,
            "{input, groupedBy?, labels?, output?, overwrite?} → per variable: Point (position), value, Rank (1 = largest) and Percent (percent rank), largest first",
            has_doc,
            |s, p| simple(s, p, "analysis.rankAndPercentile", false, toolpak::descriptive::rank_and_percentile)
        ),
        cmd!(
            "analysis.correlation",
            "Correlation",
            [],
            None,
            "{input, groupedBy?, labels?, output?, overwrite?} → lower-triangular matrix of Pearson correlations between the variables",
            has_doc,
            |s, p| simple(s, p, "analysis.correlation", false, toolpak::descriptive::correlation)
        ),
        cmd!(
            "analysis.covariance",
            "Covariance",
            [],
            None,
            "{input, groupedBy?, labels?, output?, overwrite?} → lower-triangular matrix of population covariances (variances on the diagonal)",
            has_doc,
            |s, p| simple(s, p, "analysis.covariance", false, toolpak::descriptive::covariance)
        ),
        cmd!(
            "analysis.anovaSingleFactor",
            "Anova: Single Factor",
            [],
            None,
            "{input, groupedBy?, labels?, alpha?: 0.05, output?, overwrite?} → SUMMARY (count, sum, average, variance per group) and the ANOVA table (between/within groups: SS, df, MS, F, P-value, F crit)",
            has_doc,
            anova_single
        ),
        cmd!(
            "analysis.anovaTwoFactorWithReplication",
            "Anova: Two-Factor With Replication",
            [],
            None,
            "{input (column labels in the first row, sample labels in the first column), rowsPerSample, alpha?: 0.05, output?, overwrite?} → SUMMARY per sample and column, and the ANOVA table (sample, columns, interaction, within)",
            has_doc,
            anova_with
        ),
        cmd!(
            "analysis.anovaTwoFactorWithoutReplication",
            "Anova: Two-Factor Without Replication",
            [],
            None,
            "{input, labels? (first row and column), alpha?: 0.05, output?, overwrite?} → SUMMARY per row and column, and the ANOVA table (rows, columns, error)",
            has_doc,
            anova_without
        ),
        cmd!(
            "analysis.fTest",
            "F-Test Two-Sample for Variances",
            [],
            None,
            "{input1, input2 (one column or row each), labels?, alpha?: 0.05, output?, overwrite?} → means, variances, observations, df, F, one-tailed P and critical F",
            has_doc,
            f_test
        ),
        cmd!(
            "analysis.tTestPaired",
            "t-Test: Paired Two Sample for Means",
            [],
            None,
            "{input1, input2, hypothesizedDifference?: 0, labels?, alpha?: 0.05, output?, overwrite?} → means, variances, Pearson correlation, df, t Stat, one- and two-tailed P and critical t",
            has_doc,
            |s, p| t_test(s, p, "analysis.tTestPaired", toolpak::hypothesis::TTest::Paired)
        ),
        cmd!(
            "analysis.tTestEqualVariances",
            "t-Test: Two-Sample Assuming Equal Variances",
            [],
            None,
            "{input1, input2, hypothesizedDifference?: 0, labels?, alpha?: 0.05, output?, overwrite?} → means, variances, pooled variance, df, t Stat, one- and two-tailed P and critical t",
            has_doc,
            |s, p| t_test(s, p, "analysis.tTestEqualVariances", toolpak::hypothesis::TTest::EqualVariances)
        ),
        cmd!(
            "analysis.tTestUnequalVariances",
            "t-Test: Two-Sample Assuming Unequal Variances",
            [],
            None,
            "{input1, input2, hypothesizedDifference?: 0, labels?, alpha?: 0.05, output?, overwrite?} → means, variances, df (Welch, rounded), t Stat, one- and two-tailed P and critical t",
            has_doc,
            |s, p| t_test(s, p, "analysis.tTestUnequalVariances", toolpak::hypothesis::TTest::UnequalVariances)
        ),
        cmd!(
            "analysis.zTest",
            "z-Test: Two Sample for Means",
            [],
            None,
            "{input1, input2, variance1, variance2 (known variances), hypothesizedDifference?: 0, labels?, alpha?: 0.05, output?, overwrite?} → means, z, one- and two-tailed P and critical z",
            has_doc,
            z_test
        ),
        cmd!(
            "analysis.regression",
            "Regression",
            [],
            None,
            "{inputY (one column), inputX (up to 16 adjacent columns), labels?, constantIsZero?, confidence?: 95, residuals?, standardizedResiduals?, residualPlots?, lineFitPlots?, normalProbabilityPlots?, output?, overwrite?} → SUMMARY OUTPUT (regression statistics, ANOVA, coefficients with 95% and chosen-level bounds), optional RESIDUAL and PROBABILITY OUTPUT and scatter plots",
            has_doc,
            regression
        ),
        cmd!(
            "analysis.movingAverage",
            "Moving Average",
            [],
            None,
            "{input (one column or row), labels?, interval?: 3, standardErrors?, chartOutput?, output?, overwrite?} → AVERAGE formulas over each interval (#N/A until the first full one), optional standard errors and a chart",
            has_doc,
            moving_average
        ),
        cmd!(
            "analysis.exponentialSmoothing",
            "Exponential Smoothing",
            [],
            None,
            "{input (one column or row), labels?, damping?: 0.3, standardErrors?, chartOutput?, output?, overwrite?} → smoothing formulas (1 − damping)·actual + damping·forecast, optional standard errors and a chart",
            has_doc,
            exponential_smoothing
        ),
        cmd!(
            "analysis.histogram",
            "Histogram",
            [],
            None,
            "{input, bins? (bin upper bounds; default evenly spaced), labels?, pareto?, cumulative?, chartOutput?, output?, overwrite?} → Bin/Frequency table ending in More, optional Pareto (sorted) table, cumulative percentages and chart",
            has_doc,
            histogram
        ),
        cmd!(
            "analysis.fourierAnalysis",
            "Fourier Analysis",
            [],
            None,
            "{input (one column or row of 1, 2, 4 … 4096 numbers or complex text like 3+4i), labels?, inverse?, output?, overwrite?} → the discrete Fourier transform as complex numbers",
            has_doc,
            fourier
        ),
        cmd!(
            "analysis.randomNumberGeneration",
            "Random Number Generation",
            [],
            None,
            "{variables?: 1, count, distribution: uniform{from, to} | normal{mean, sd} | bernoulli{p} | binomial{p, trials} | poisson{lambda} | patterned{from, to, step, repeatEach, repeatSequence} | discrete{values: two-column range of value and probability}, seed?, output?, overwrite?} → columns of random numbers (the same seed repeats them)",
            has_doc,
            random_numbers
        ),
        cmd!(
            "analysis.sampling",
            "Sampling",
            [],
            None,
            "{input, labels?, method?: periodic|random, period? (periodic), samples? (random, with replacement), seed?, output?, overwrite?} → a column of values taken from the input",
            has_doc,
            sampling
        ),
    ]
}

/// A tool that only needs the input variables.
fn simple(s: &mut Session, p: &Json, cmd: &str, spaced: bool, tool: fn(&[Series]) -> gridcraft_analysis::Result<Out>) -> Result<Json> {
    let input = read_input(s, p, "input", cmd, spaced)?;
    run_with_input(s, p, cmd, &input, |_| tool(&input.series))
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

/// One variable from `p[key]` (a single column or row), named `default` unless labelled.
pub(crate) fn read_one(s: &Session, p: &Json, key: &str, cmd: &str, default: &str) -> Result<Input> {
    let text = str_param(p, key).unwrap_or("");
    let (_, body) = split_sheet(text);
    let row_shaped = RangeRef::parse(body.trim()).is_some_and(|r| r.height() == 1 && r.width() > 1);
    let mut q = p.clone();
    if let Some(m) = q.as_object_mut() {
        m.insert("groupedBy".into(), json!(if row_shaped { "rows" } else { "columns" }));
    }
    let mut input = read_input(s, &q, key, cmd, false)?;
    if input.series.len() != 1 {
        return Err(EngineError::Other("Each variable range must be a single column or row.".into()));
    }
    if !bool_param(p, "labels").unwrap_or(false)
        && let Some(x) = input.series.first_mut()
    {
        x.label = default.to_string();
    }
    Ok(input)
}

/// Two inputs as one (for the overlap check).
fn merge(mut a: Input, b: Input) -> (Input, Series, Series) {
    let sa = a.series.first().cloned().unwrap_or_else(|| Series::new("", vec![]));
    let sb = b.series.first().cloned().unwrap_or_else(|| Series::new("", vec![]));
    a.cells.extend(b.cells);
    (a, sa, sb)
}

/// A table of numbers with optional labels in the first row and column (two-factor ANOVA).
pub(crate) fn read_grid(s: &Session, p: &Json, cmd: &str, labels: bool) -> Result<(Input, toolpak::hypothesis::Grid)> {
    let text = str_param(p, "input").map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad(cmd, "missing `input` (the input range)"))?;
    let (sheet, body) = split_sheet(text);
    let r = RangeRef::parse(body.trim()).ok_or_else(|| bad(cmd, format!("`input` is not a range: `{text}`")))?;
    if labels && r.height() < 2 {
        return Err(EngineError::Other("The input range has no values below its labels.".into()));
    }
    // The rows below the column labels, each labelled by its first cell.
    let data = if labels { RangeRef::new(CellRef::new(r.start.row + 1, r.start.col), r.end) } else { r };
    let prefix = sheet.as_deref().map(|n| format!("{}!", gridcraft_formula::quote_sheet(n))).unwrap_or_default();
    let mut q = p.clone();
    if let Some(m) = q.as_object_mut() {
        m.insert("input".into(), json!(format!("{prefix}{}", data.a1())));
        m.insert("groupedBy".into(), json!("rows"));
        m.insert("labels".into(), json!(labels));
    }
    let input = read_input(s, &q, "input", cmd, true)?;
    let width = input.series.first().map_or(0, |x| x.values.len());
    let col_labels: Vec<String> = if labels {
        let d = s.doc()?;
        let sh = d.wb.sheet(input.sheet).ok_or(EngineError::NoDocument)?;
        (0..width as u32).map(|j| sh.value(CellRef::new(r.start.row, r.start.col + 1 + j)).display()).collect()
    } else {
        (0..width).map(|j| toolpak::default_label(false, j, true)).collect()
    };
    let mut rows = Vec::with_capacity(input.series.len());
    for x in &input.series {
        rows.push(x.complete().map_err(|_| EngineError::Other("Every cell of the input must hold a number.".into()))?);
    }
    let row_labels = input.series.iter().map(|x| x.label.clone()).collect();
    Ok((input, toolpak::hypothesis::Grid { row_labels, col_labels, data: rows }))
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
            across: look.across,
            top: look.top,
            bottom: look.bottom,
            num_fmt: look.fmt.map(str::to_string),
        };
        let value = match v {
            V::Num(x) => RValue::Value(Value::Number(x)),
            // Blank cells that only carry a border or alignment.
            V::Text(t) if t.is_empty() => RValue::Value(Value::Empty),
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

fn anova_single(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.anovaSingleFactor";
    let input = read_input(s, p, "input", CMD, true)?;
    let alpha = num_param(p, "alpha", 0.05, CMD)?;
    run_with_input(s, p, CMD, &input, |_| toolpak::hypothesis::anova_single_factor(&input.series, alpha))
}

fn anova_without(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.anovaTwoFactorWithoutReplication";
    let (input, grid) = read_grid(s, p, CMD, bool_param(p, "labels").unwrap_or(false))?;
    let alpha = num_param(p, "alpha", 0.05, CMD)?;
    run_with_input(s, p, CMD, &input, |_| toolpak::hypothesis::anova_two_factor_without(&grid, alpha))
}

fn anova_with(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.anovaTwoFactorWithReplication";
    let per = count_param(p, "rowsPerSample", CMD)?.ok_or_else(|| bad(CMD, "missing `rowsPerSample`"))?;
    // The input always has labels: column labels on top, sample labels down the left.
    let (input, mut grid) = read_grid(s, p, CMD, true)?;
    // One label per sample: the label beside its first row.
    grid.row_labels = grid.row_labels.iter().step_by(per.max(1)).cloned().collect();
    let alpha = num_param(p, "alpha", 0.05, CMD)?;
    run_with_input(s, p, CMD, &input, |_| toolpak::hypothesis::anova_two_factor_with(&grid, per, alpha))
}

fn two_inputs(s: &Session, p: &Json, cmd: &str) -> Result<(Input, Series, Series)> {
    let a = read_one(s, p, "input1", cmd, "Variable 1")?;
    let b = read_one(s, p, "input2", cmd, "Variable 2")?;
    Ok(merge(a, b))
}

fn f_test(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.fTest";
    let (input, a, b) = two_inputs(s, p, CMD)?;
    let alpha = num_param(p, "alpha", 0.05, CMD)?;
    run_with_input(s, p, CMD, &input, |_| toolpak::hypothesis::f_test(&a, &b, alpha))
}

fn t_test(s: &mut Session, p: &Json, cmd: &str, kind: toolpak::hypothesis::TTest) -> Result<Json> {
    let (input, a, b) = two_inputs(s, p, cmd)?;
    let alpha = num_param(p, "alpha", 0.05, cmd)?;
    let diff = num_param(p, "hypothesizedDifference", 0.0, cmd)?;
    run_with_input(s, p, cmd, &input, |_| toolpak::hypothesis::t_test(kind, &a, &b, diff, alpha))
}

fn z_test(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.zTest";
    let (input, a, b) = two_inputs(s, p, CMD)?;
    let alpha = num_param(p, "alpha", 0.05, CMD)?;
    let diff = num_param(p, "hypothesizedDifference", 0.0, CMD)?;
    let v1 = num_param(p, "variance1", f64::NAN, CMD)?;
    let v2 = num_param(p, "variance2", f64::NAN, CMD)?;
    run_with_input(s, p, CMD, &input, |_| toolpak::hypothesis::z_test(&a, &b, diff, v1, v2, alpha))
}

/// Absolute reference (with sheet) to the cells of variable `k` of `input`.
fn series_ref(s: &Session, input: &Input, k: usize) -> String {
    let name = s.doc().ok().and_then(|d| d.wb.sheet(input.sheet).map(|sh| sh.name.clone())).unwrap_or_default();
    let cells = input.cells.get(k).map(Vec::as_slice).unwrap_or(&[]);
    let (Some(a), Some(b)) = (cells.first(), cells.last()) else { return String::new() };
    let abs = |c: &CellRef| format!("${}${}", gridcraft_core::col_to_letters(c.col), c.row + 1);
    format!("{}!{}:{}", gridcraft_formula::quote_sheet(&name), abs(a), abs(b))
}

fn regression(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.regression";
    let yin = read_one(s, p, "inputY", CMD, "Y")?;
    let y_rows = yin.cells.first().is_some_and(|c| c.len() > 1 && c.iter().all(|x| x.row == c[0].row));
    let mut q = p.clone();
    if let Some(m) = q.as_object_mut() {
        m.insert("groupedBy".into(), json!(if y_rows { "rows" } else { "columns" }));
    }
    let mut xin = read_input(s, &q, "inputX", CMD, false)?;
    if !bool_param(p, "labels").unwrap_or(false) {
        for (i, x) in xin.series.iter_mut().enumerate() {
            x.label = format!("X Variable {}", i + 1);
        }
    }
    let confidence = match p.get("confidence") {
        None | Some(Json::Null) | Some(Json::Bool(_)) => 95.0,
        _ => num_param(p, "confidence", 95.0, CMD)?,
    };
    let flag = |k: &str| bool_param(p, k).unwrap_or(false);
    let o = toolpak::regression::RegressionOptions {
        constant_is_zero: flag("constantIsZero"),
        confidence,
        residuals: flag("residuals"),
        standardized_residuals: flag("standardizedResiduals"),
        residual_plots: flag("residualPlots"),
        line_fit_plots: flag("lineFitPlots"),
        normal_probability_plots: flag("normalProbabilityPlots"),
    };
    let refs = toolpak::regression::InputRefs { y: series_ref(s, &yin, 0), x: (0..xin.series.len()).map(|k| series_ref(s, &xin, k)).collect() };
    let y = yin.series.first().cloned().unwrap_or_else(|| Series::new("Y", vec![]));
    let xs = std::mem::take(&mut xin.series);
    let mut input = yin;
    input.cells.extend(xin.cells);
    run_with_input(s, p, CMD, &input, |_| toolpak::regression::regression(&y, &xs, &o, &refs))
}

/// Reference text for formula tools: input blocks (sheet-qualified when the output is on another
/// sheet) and output blocks.
fn formula_refs(s: &Session, input: &Input, target: &Target, cmd: &str) -> Result<(Vec<CellRef>, Option<String>, String)> {
    let d = s.doc()?;
    let in_name = d.wb.sheet(input.sheet).map(|x| x.name.clone()).unwrap_or_default();
    let same = matches!(target.kind, report::OutputKind::Range { sheet } if sheet == input.sheet);
    if target.kind == report::OutputKind::NewWorkbook {
        return Err(bad(cmd, "the output must be on a worksheet of this workbook (the formulas refer to the input)"));
    }
    let cells = input.cells.first().cloned().unwrap_or_default();
    let prefix = (!same).then(|| format!("{}!", gridcraft_formula::quote_sheet(&in_name)));
    let area = match (cells.first(), cells.last()) {
        (Some(a), Some(b)) => {
            let abs = |c: &CellRef| format!("${}${}", gridcraft_core::col_to_letters(c.col), c.row + 1);
            format!("{}!{}:{}", gridcraft_formula::quote_sheet(&in_name), abs(a), abs(b))
        }
        _ => String::new(),
    };
    Ok((cells, prefix, area))
}

fn smoothing(s: &mut Session, p: &Json, cmd: &str, moving: bool) -> Result<Json> {
    let input = read_one(s, p, "input", cmd, "")?;
    let target = report::parse_output(s, p, cmd)?;
    let (cells, prefix, area) = formula_refs(s, &input, &target, cmd)?;
    let n = input.series.first().map_or(0, |x| x.values.len());
    if input.series.first().is_some_and(|x| x.values.iter().any(Option::is_none)) {
        return Err(EngineError::Other("The input range has blank cells.".into()));
    }
    let pre = prefix.unwrap_or_default();
    let in_ref = |a: usize, b: usize| -> String {
        let (Some(x), Some(y)) = (cells.get(a), cells.get(b)) else { return String::new() };
        if a == b { format!("{pre}{}", x.a1()) } else { format!("{pre}{}:{}", x.a1(), y.a1()) }
    };
    let origin = target.origin;
    let out_ref = |r0: u32, c0: u32, r1: u32, c1: u32| -> String {
        let a = CellRef::new(origin.row + r0, origin.col + c0);
        let b = CellRef::new(origin.row + r1, origin.col + c1);
        if a == b { a.a1() } else { format!("{}:{}", a.a1(), b.a1()) }
    };
    let refs = toolpak::series::Refs { input: &in_ref, output: &out_ref, input_area: area };
    let std_err = bool_param(p, "standardErrors").unwrap_or(false);
    let chart = bool_param(p, "chartOutput").unwrap_or(false);
    let out = if moving {
        let interval = count_param(p, "interval", cmd)?.unwrap_or(3);
        toolpak::series::moving_average(n, interval, std_err, chart, &refs)
    } else {
        let damping = num_param(p, "damping", 0.3, cmd)?;
        toolpak::series::exponential_smoothing(n, damping, std_err, chart, &refs)
    }
    .map_err(tool_error)?;
    check_overlap(&target, &input, &out)?;
    report::write(s, &target, to_report(out), report::overwrite_param(p))
}

fn moving_average(s: &mut Session, p: &Json) -> Result<Json> {
    smoothing(s, p, "analysis.movingAverage", true)
}

fn exponential_smoothing(s: &mut Session, p: &Json) -> Result<Json> {
    smoothing(s, p, "analysis.exponentialSmoothing", false)
}

fn histogram(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.histogram";
    let input = read_input(s, p, "input", CMD, false)?;
    let values: Vec<f64> = input.series.iter().flat_map(Series::numbers).collect();
    let bins = if str_param(p, "bins").is_some_and(|t| !t.trim().is_empty()) {
        let b = read_input(s, p, "bins", CMD, false)?;
        Some(b.series.iter().flat_map(Series::numbers).collect::<Vec<f64>>())
    } else {
        None
    };
    let o = toolpak::series::HistogramOptions {
        bins,
        pareto: bool_param(p, "pareto").unwrap_or(false),
        cumulative: bool_param(p, "cumulative").unwrap_or(false),
        chart: bool_param(p, "chartOutput").unwrap_or(false),
    };
    run_with_input(s, p, CMD, &input, |_| toolpak::series::histogram(&values, &o))
}

fn fourier(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.fourierAnalysis";
    let d = s.doc()?;
    let text = str_param(p, "input").map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad(CMD, "missing `input` (the input range)"))?;
    let (sheet, body) = split_sheet(text);
    let si = match sheet {
        Some(n) => d.wb.sheet_index(&n).ok_or_else(|| bad(CMD, format!("no sheet named `{n}`")))?,
        None => d.wb.active_sheet,
    };
    let sh = d.wb.sheet(si).ok_or(EngineError::NoDocument)?;
    let r = RangeRef::parse(body.trim()).ok_or_else(|| bad(CMD, format!("`input` is not a range: `{text}`")))?;
    if r.width() > 1 && r.height() > 1 {
        return Err(EngineError::Other("The input range must be a single column or row.".into()));
    }
    if r.count() > gridcraft_analysis::fft::MAX_LEN as u64 + 1 {
        return Err(EngineError::Other(format!("Fourier Analysis takes at most {} values.", gridcraft_analysis::fft::MAX_LEN)));
    }
    let labels = bool_param(p, "labels").unwrap_or(false);
    let cells: Vec<CellRef> = r.iter().collect();
    let label = labels.then(|| cells.first().map(|c| sh.value(*c).display()).unwrap_or_default());
    let data: Vec<CellRef> = cells.iter().skip(usize::from(labels)).copied().collect();
    let mut values = Vec::with_capacity(data.len());
    for c in &data {
        let v = sh.value(*c);
        if v.is_empty() {
            return Err(EngineError::Other("The input range has blank cells.".into()));
        }
        values.push(
            gridcraft_functions::dist::complex_of(&v)
                .map_err(|_| EngineError::Other("The input must hold numbers or complex numbers such as 3+4i.".into()))?,
        );
    }
    let input = Input { sheet: si, series: vec![], cells: vec![cells] };
    let inverse = bool_param(p, "inverse").unwrap_or(false);
    run_with_input(s, p, CMD, &input, |_| toolpak::series::fourier(&values, inverse, label.as_deref()))
}

fn seed_param(p: &Json, cmd: &str) -> Result<Option<u64>> {
    match p.get("seed") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(t)) if t.trim().is_empty() => Ok(None),
        _ => {
            let x = num_param(p, "seed", 0.0, cmd)?;
            if x.is_finite() && x >= 0.0 && x.fract() == 0.0 && x < 1e18 {
                Ok(Some(x as u64))
            } else {
                Err(bad(cmd, "`seed` must be a whole number from 0"))
            }
        }
    }
}

fn random_numbers(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.randomNumberGeneration";
    use toolpak::generate::Distribution as D;
    let num = |k: &str, default: f64| num_param(p, k, default, CMD);
    let dist = match str_param(p, "distribution").unwrap_or("uniform").to_ascii_lowercase().as_str() {
        "uniform" => D::Uniform { from: num("from", 0.0)?, to: num("to", 1.0)? },
        "normal" => D::Normal { mean: num("mean", 0.0)?, sd: num("sd", 1.0)? },
        "bernoulli" => D::Bernoulli { p: num("p", 0.5)? },
        "binomial" => D::Binomial { p: num("p", 0.5)?, trials: count_param(p, "trials", CMD)?.unwrap_or(1) as u64 },
        "poisson" => D::Poisson { lambda: num("lambda", 1.0)? },
        "patterned" => D::Patterned {
            from: num("from", 1.0)?,
            to: num("to", 1.0)?,
            step: num("step", 1.0)?,
            repeat_each: count_param(p, "repeatEach", CMD)?.unwrap_or(1),
            repeat_sequence: count_param(p, "repeatSequence", CMD)?.unwrap_or(1),
        },
        "discrete" => {
            let table = read_input(s, &json!({"values": str_param(p, "values").unwrap_or(""), "groupedBy": "columns"}), "values", CMD, false)?;
            let (values, probs) = match table.series.as_slice() {
                [v, pr] => (v.complete().map_err(tool_error)?, pr.complete().map_err(tool_error)?),
                _ => return Err(EngineError::Other("The value and probability range must have two columns.".into())),
            };
            D::Discrete { values, probs }
        }
        other => return Err(bad(CMD, format!("unknown distribution `{other}`"))),
    };
    let variables = count_param(p, "variables", CMD)?.unwrap_or(1);
    let count = match &dist {
        D::Patterned { .. } => 0,
        _ => count_param(p, "count", CMD)?.ok_or_else(|| bad(CMD, "missing `count` (how many random numbers per variable)"))?,
    };
    let seed = seed_param(p, CMD)?;
    let target = report::parse_output(s, p, CMD)?;
    let out = toolpak::generate::random_numbers(variables, count, &dist, seed).map_err(tool_error)?;
    report::write(s, &target, to_report(out), report::overwrite_param(p))
}

fn sampling(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "analysis.sampling";
    let input = read_input(s, p, "input", CMD, false)?;
    let values: Vec<f64> = input.series.iter().flat_map(Series::numbers).collect();
    let how = match str_param(p, "method").unwrap_or("periodic") {
        "random" => toolpak::generate::Sampling::Random(count_param(p, "samples", CMD)?.ok_or_else(|| bad(CMD, "missing `samples`"))?),
        _ => toolpak::generate::Sampling::Periodic(count_param(p, "period", CMD)?.ok_or_else(|| bad(CMD, "missing `period`"))?),
    };
    let seed = seed_param(p, CMD)?;
    run_with_input(s, p, CMD, &input, |_| toolpak::generate::sample(&values, how, seed))
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
    fn rank_correlation_and_covariance() {
        let mut s = session();
        s.execute("range.setValues", json!({"range": "A1", "values": [["X", "Y"], [1, 2], [2, 4], [3, 5], [4, 9]]})).unwrap();
        s.execute("analysis.correlation", json!({"input": "A1:B5", "labels": true, "output": "D1"})).unwrap();
        assert_eq!(val(&s, "Sheet1", "E1"), Value::from("X"));
        assert_eq!(val(&s, "Sheet1", "D3"), Value::from("Y"));
        assert!((val(&s, "Sheet1", "E3").as_f64().unwrap() - 0.964_763_821_237_732).abs() < 1e-12);
        s.execute("analysis.covariance", json!({"input": "A1:B5", "labels": true, "output": "D6"})).unwrap();
        assert_eq!(val(&s, "Sheet1", "E8"), Value::Number(2.75));
        s.execute("analysis.rankAndPercentile", json!({"input": "B2:B5", "output": "D10"})).unwrap();
        assert_eq!(val(&s, "Sheet1", "E10"), Value::from("Column1"));
        assert_eq!(val(&s, "Sheet1", "D11"), Value::Number(4.0));
        assert_eq!(val(&s, "Sheet1", "E11"), Value::Number(9.0));
    }

    #[test]
    fn anova_and_two_sample_tests() {
        let mut s = session();
        s.execute("range.setValues", json!({"range": "A1", "values": [["A", "B", "C"], [4, 6, 9], [5, 7, 10], [6, 8, 11]]})).unwrap();
        s.execute("analysis.anovaSingleFactor", json!({"input": "A1:C4", "labels": true, "output": {"newSheet": "Anova"}})).unwrap();
        assert_eq!(val(&s, "Anova", "A1"), Value::from("Anova: Single Factor"));
        assert_eq!(val(&s, "Anova", "A5"), Value::from("A"));
        assert_eq!(val(&s, "Anova", "A12"), Value::from("Between Groups"));
        assert!((val(&s, "Anova", "B12").as_f64().unwrap() - 38.0).abs() < 1e-9);
        // Two-factor without replication on the same block (labels in first row and column).
        s.execute("range.setValues", json!({"range": "Sheet1!A10", "values": [[null, "C1", "C2"], ["R1", 1, 3], ["R2", 2, 5], ["R3", 4, 8]]}))
            .unwrap();
        s.execute("analysis.anovaTwoFactorWithoutReplication", json!({"input": "Sheet1!A10:C13", "labels": true, "output": {"newSheet": "Two"}}))
            .unwrap();
        assert_eq!(val(&s, "Two", "A4"), Value::from("R1"));
        assert_eq!(val(&s, "Two", "A8"), Value::from("C1"));
        // With replication: two samples of two rows.
        s.execute(
            "range.setValues",
            json!({"range": "Sheet1!A20", "values": [[null, "A", "B"], ["Low", 1, 4], [null, 3, 6], ["High", 5, 6], [null, 7, 10]]}),
        )
        .unwrap();
        s.execute("analysis.anovaTwoFactorWithReplication", json!({"input": "Sheet1!A20:C24", "rowsPerSample": 2, "output": {"newSheet": "Rep"}}))
            .unwrap();
        assert_eq!(val(&s, "Rep", "A4"), Value::from("Low"));
        assert!(s.execute("analysis.anovaTwoFactorWithReplication", json!({"input": "Sheet1!A20:C24"})).is_err());
        // Two-sample tests.
        s.execute("range.setValues", json!({"range": "Sheet1!E1", "values": [["Before", "After"], [10, 8], [12, 9], [9, 7], [15, 10], [11, 9]]}))
            .unwrap();
        for cmd in ["analysis.fTest", "analysis.tTestPaired", "analysis.tTestEqualVariances", "analysis.tTestUnequalVariances"] {
            s.execute(cmd, json!({"input1": "Sheet1!E1:E6", "input2": "Sheet1!F1:F6", "labels": true, "output": "newSheet"})).unwrap();
            let name = s.doc().unwrap().wb.active().unwrap().name.clone();
            assert_eq!(val(&s, &name, "B3"), Value::from("Before"), "{cmd}");
            assert_eq!(val(&s, &name, "B4"), Value::Number(11.4), "{cmd}");
        }
        s.execute(
            "analysis.zTest",
            json!({"input1": "Sheet1!E2:E6", "input2": "Sheet1!F2:F6", "variance1": 5, "variance2": 1, "output": "newSheet"}),
        )
        .unwrap();
        let name = s.doc().unwrap().wb.active().unwrap().name.clone();
        assert_eq!(val(&s, &name, "B3"), Value::from("Variable 1"));
        assert!(s.execute("analysis.zTest", json!({"input1": "Sheet1!E2:E6", "input2": "Sheet1!F2:F6"})).is_err(), "variances are required");
        assert!(s.execute("analysis.fTest", json!({"input1": "Sheet1!E2:F6", "input2": "Sheet1!F2:F6"})).is_err(), "one column each");
    }

    #[test]
    fn regression_with_plots() {
        let mut s = session();
        s.execute(
            "range.setValues",
            json!({"range": "A1", "values": [["Ads", "Price", "Sales"], [1, 9, 3], [2, 8, 5], [3, 8, 7], [4, 6, 10], [5, 5, 11]]}),
        )
        .unwrap();
        let r = s
            .execute(
                "analysis.regression",
                json!({"inputY": "C1:C6", "inputX": "A1:B6", "labels": true, "residuals": true, "lineFitPlots": true, "normalProbabilityPlots": true, "output": {"newSheet": "Fit"}}),
            )
            .unwrap();
        assert_eq!(r["sheet"], "Fit");
        assert_eq!(val(&s, "Fit", "A1"), Value::from("SUMMARY OUTPUT"));
        assert_eq!(val(&s, "Fit", "A18"), Value::from("Ads"));
        assert_eq!(val(&s, "Fit", "A19"), Value::from("Price"));
        assert_eq!(val(&s, "Fit", "A23"), Value::from("RESIDUAL OUTPUT"));
        assert_eq!(val(&s, "Fit", "B25"), Value::from("Predicted Sales"));
        let sh = s.doc().unwrap().wb.active().unwrap().clone();
        assert_eq!(sh.charts.len(), 3, "two line fit plots and a normal probability plot");
        assert_eq!(sh.charts[0].series[0].values, "Sheet1!$C$2:$C$6");
        assert_eq!(sh.charts[0].series[0].categories.as_deref(), Some("Sheet1!$A$2:$A$6"));
        assert_eq!(sh.charts[0].series[1].values, "Fit!$B$26:$B$30");
        // Without labels the variables are named like Excel's.
        s.execute("analysis.regression", json!({"inputY": "Sheet1!C2:C6", "inputX": "Sheet1!A2:A6", "output": "newSheet"})).unwrap();
        let name = s.doc().unwrap().wb.active().unwrap().name.clone();
        assert_eq!(val(&s, &name, "A18"), Value::from("X Variable 1"));
        assert!(s.execute("analysis.regression", json!({"inputY": "Sheet1!C2:C6", "inputX": "Sheet1!A2:A4"})).is_err());
    }

    #[test]
    fn series_tools() {
        let mut s = session();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Demand"], [10], [12], [14], [13], [15], [18]]})).unwrap();
        s.execute("analysis.movingAverage", json!({"input": "A1:A7", "labels": true, "standardErrors": true, "chartOutput": true, "output": "C2"}))
            .unwrap();
        assert_eq!(val(&s, "Sheet1", "C3"), Value::Error(gridcraft_core::CellError::NA));
        assert_eq!(val(&s, "Sheet1", "C4"), Value::Number(12.0));
        let f = s.execute("cell.get", json!({"cell": "C4"})).unwrap();
        assert_eq!(f["formula"], "=AVERAGE(A2:A4)");
        // Live: the forecast follows the input.
        s.execute("cell.set", json!({"cell": "A2", "input": "1"})).unwrap();
        assert_eq!(val(&s, "Sheet1", "C4"), Value::Number(9.0));
        assert_eq!(s.doc().unwrap().wb.active().unwrap().charts.len(), 1);
        s.execute("analysis.exponentialSmoothing", json!({"input": "A2:A7", "output": {"newSheet": "Smooth"}})).unwrap();
        assert_eq!(val(&s, "Smooth", "A2"), Value::Number(1.0));
        let f = s.execute("cell.get", json!({"cell": "A3"})).unwrap();
        assert_eq!(f["formula"], "=0.7*Sheet1!A3+0.3*A2");
        assert!(s.execute("analysis.movingAverage", json!({"input": "Sheet1!A2:A7", "output": "newWorkbook"})).is_err());
        // Histogram with bins from the sheet.
        s.execute("sheet.activate", json!({"sheet": "Sheet1"})).unwrap();
        s.execute("range.setValues", json!({"range": "H1", "values": [[2], [4]]})).unwrap();
        s.execute("analysis.histogram", json!({"input": "A2:A7", "bins": "H1:H2", "output": "J1", "chartOutput": true})).unwrap();
        assert_eq!(val(&s, "Sheet1", "J1"), Value::from("Bin"));
        assert_eq!(val(&s, "Sheet1", "K2"), Value::Number(1.0));
        assert_eq!(val(&s, "Sheet1", "J4"), Value::from("More"));
        assert_eq!(val(&s, "Sheet1", "K4"), Value::Number(5.0));
        // Fourier of complex text.
        s.execute("range.setValues", json!({"range": "M1", "values": [["1"], ["2"], ["3+i"], ["4"]]})).unwrap();
        s.execute("analysis.fourierAnalysis", json!({"input": "M1:M4", "output": "N1"})).unwrap();
        assert_eq!(val(&s, "Sheet1", "N1"), Value::from("10+i"));
        assert!(s.execute("analysis.fourierAnalysis", json!({"input": "M1:M3", "output": "O1"})).is_err());
    }

    #[test]
    fn random_numbers_and_sampling() {
        let mut s = session();
        s.execute(
            "analysis.randomNumberGeneration",
            json!({"variables": 2, "count": 5, "distribution": "uniform", "from": 1, "to": 2, "seed": 42, "output": "A1"}),
        )
        .unwrap();
        let a = val(&s, "Sheet1", "B5").as_f64().unwrap();
        assert!((1.0..=2.0).contains(&a));
        // The same seed repeats the numbers.
        s.execute(
            "analysis.randomNumberGeneration",
            json!({"variables": 2, "count": 5, "distribution": "uniform", "from": 1, "to": 2, "seed": 42, "output": "D1"}),
        )
        .unwrap();
        assert_eq!(val(&s, "Sheet1", "E5").as_f64().unwrap(), a);
        s.execute("range.setValues", json!({"range": "G1", "values": [[1, 0.25], [2, 0.75]]})).unwrap();
        s.execute("analysis.randomNumberGeneration", json!({"count": 10, "distribution": "discrete", "values": "G1:H2", "output": "J1"})).unwrap();
        let v = val(&s, "Sheet1", "J10").as_f64().unwrap();
        assert!(v == 1.0 || v == 2.0);
        s.execute("analysis.randomNumberGeneration", json!({"distribution": "patterned", "from": 1, "to": 2, "repeatEach": 2, "output": "L1"}))
            .unwrap();
        assert_eq!(val(&s, "Sheet1", "L3"), Value::Number(2.0));
        assert!(s.execute("analysis.randomNumberGeneration", json!({"distribution": "uniform"})).is_err(), "count is required");
        assert!(s.execute("analysis.randomNumberGeneration", json!({"count": 3, "distribution": "weird"})).is_err());
        s.execute("range.setValues", json!({"range": "N1", "values": [[10], [20], [30], [40]]})).unwrap();
        s.execute("analysis.sampling", json!({"input": "N1:N4", "period": 2, "output": "P1"})).unwrap();
        assert_eq!((val(&s, "Sheet1", "P1"), val(&s, "Sheet1", "P2")), (Value::Number(20.0), Value::Number(40.0)));
        s.execute("analysis.sampling", json!({"input": "N1:N4", "method": "random", "samples": 3, "seed": 1, "output": "Q1"})).unwrap();
        assert!([10.0, 20.0, 30.0, 40.0].contains(&val(&s, "Sheet1", "Q3").as_f64().unwrap()));
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
