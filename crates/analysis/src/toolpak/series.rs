//! Moving Average, Exponential Smoothing, Histogram and Fourier Analysis.
//!
//! Moving Average and Exponential Smoothing write formulas (as Excel's ToolPak does), so their
//! output updates when the input changes. The caller supplies reference text for input and
//! output blocks.

use gridcraft_core::CellError;
use gridcraft_functions::dist;

use super::{Area, Chart, ChartKind, ChartSeries, Look, Out, V};
use crate::{Error, Result, input};

/// Reference text for blocks of the input and output.
pub struct Refs<'a> {
    /// Input values `from..=to` (indices into the input), e.g. `A2:A4`.
    pub input: &'a dyn Fn(usize, usize) -> String,
    /// Output cells `(row0, col0)..=(row1, col1)`, relative to the output's top-left.
    pub output: &'a dyn Fn(u32, u32, u32, u32) -> String,
    /// All the input values, absolute (for charts).
    pub input_area: String,
}

/// A number as formula text (no float noise: 1 - 0.3 is written 0.7).
fn num_text(x: f64) -> String {
    let r = (x * 1e12).round() / 1e12;
    let s = format!("{r}");
    if s.contains('e') { format!("{x}") } else { s }
}

fn forecast_chart(title: &str, n: usize, col: u32, refs: &Refs) -> Chart {
    Chart {
        kind: ChartKind::Line,
        title: title.into(),
        x_title: Some("Data Point".into()),
        y_title: Some("Value".into()),
        series: vec![
            ChartSeries { name: Some("Actual".into()), x: None, y: Area::Ref(refs.input_area.clone()), line: false },
            ChartSeries { name: Some("Forecast".into()), x: None, y: Area::Out { row: 0, col: 0, rows: n as u32, cols: 1 }, line: false },
        ],
        row: 0,
        col,
        width: 360.0,
        height: 216.0,
        legend: true,
    }
}

/// Moving Average over `interval` values: one formula per input value (`#N/A` until the first
/// full interval), optional standard errors and a chart of actual against forecast.
pub fn moving_average(n: usize, interval: usize, std_errors: bool, chart: bool, refs: &Refs) -> Result<Out> {
    if n == 0 {
        return Err(input("The input range is empty."));
    }
    if n > super::MAX_VALUES {
        return Err(Error::TooLarge("The input range is too large.".into()));
    }
    if interval < 1 || interval > n {
        return Err(input("The interval must be a whole number from 1 to the number of input values."));
    }
    let mut out = Out::default();
    for i in 0..n {
        let r = i as u32;
        if i + 1 < interval {
            out.put(r, 0, V::Error(CellError::NA), Look::PLAIN);
        } else {
            out.put(r, 0, V::Formula(format!("=AVERAGE({})", (refs.input)(i + 1 - interval, i))), Look::PLAIN);
        }
        if std_errors {
            if i + 2 < 2 * interval {
                out.put(r, 1, V::Error(CellError::NA), Look::PLAIN);
            } else {
                let a = (refs.input)(i + 1 - interval, i);
                let f = (refs.output)((i + 1 - interval) as u32, 0, r, 0);
                out.put(r, 1, V::Formula(format!("=SQRT(SUMXMY2({a},{f})/{interval})")), Look::PLAIN);
            }
        }
    }
    if chart {
        out.charts.push(forecast_chart("Moving Average", n, 2 + u32::from(std_errors), refs));
    }
    Ok(out)
}

/// Exponential Smoothing with damping factor `damping` (smoothing constant 1 − damping).
pub fn exponential_smoothing(n: usize, damping: f64, std_errors: bool, chart: bool, refs: &Refs) -> Result<Out> {
    if n == 0 {
        return Err(input("The input range is empty."));
    }
    if n > super::MAX_VALUES {
        return Err(Error::TooLarge("The input range is too large.".into()));
    }
    if !(0.0..=1.0).contains(&damping) {
        return Err(input("The damping factor must be between 0 and 1."));
    }
    let alpha = num_text(1.0 - damping);
    let damp = num_text(damping);
    let mut out = Out::default();
    for i in 0..n {
        let r = i as u32;
        let v = match i {
            0 => V::Error(CellError::NA),
            1 => V::Formula(format!("={}", (refs.input)(0, 0))),
            _ => V::Formula(format!("={alpha}*{}+{damp}*{}", (refs.input)(i - 1, i - 1), (refs.output)(r - 1, 0, r - 1, 0))),
        };
        out.put(r, 0, v, Look::PLAIN);
        if std_errors {
            if i < 3 {
                out.put(r, 1, V::Error(CellError::NA), Look::PLAIN);
            } else {
                let a = (refs.input)(i - 2, i);
                let f = (refs.output)(r - 2, 0, r, 0);
                out.put(r, 1, V::Formula(format!("=SQRT(SUMXMY2({a},{f})/3)")), Look::PLAIN);
            }
        }
    }
    if chart {
        out.charts.push(forecast_chart("Exponential Smoothing", n, 2 + u32::from(std_errors), refs));
    }
    Ok(out)
}

/// Histogram options.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistogramOptions {
    /// Bin upper bounds; `None` makes evenly spaced bins from the minimum to the maximum.
    pub bins: Option<Vec<f64>>,
    pub pareto: bool,
    pub cumulative: bool,
    pub chart: bool,
}

/// Evenly spaced bins between the minimum and the maximum: ⌈√n⌉ intervals.
fn auto_bins(x: &[f64]) -> Vec<f64> {
    let (min, max) = x.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
    if max.is_nan() || min.is_nan() || max <= min {
        return vec![min];
    }
    let k = ((x.len() as f64).sqrt().ceil() as usize).clamp(1, 1000);
    let w = (max - min) / k as f64;
    (0..=k).map(|j| if j == k { max } else { min + w * j as f64 }).collect()
}

/// Histogram: how many values fall in each bin (`(previous bin, bin]`, then "More"), with an
/// optional Pareto ordering, cumulative percentages and a chart.
pub fn histogram(values: &[f64], o: &HistogramOptions) -> Result<Out> {
    if values.is_empty() {
        return Err(input("The input range has no numbers."));
    }
    if values.len() > super::MAX_VALUES {
        return Err(Error::TooLarge("The input range is too large.".into()));
    }
    let mut bins = match &o.bins {
        Some(b) if !b.is_empty() => b.clone(),
        Some(_) => return Err(input("The bin range has no numbers.")),
        None => auto_bins(values),
    };
    if bins.len() > 10_000 {
        return Err(Error::TooLarge("Use at most 10,000 bins.".into()));
    }
    bins.sort_by(|a, b| a.total_cmp(b));
    bins.dedup();
    let mut counts = vec![0usize; bins.len() + 1];
    for v in values {
        let i = bins.partition_point(|b| b < v);
        if let Some(c) = counts.get_mut(i) {
            *c += 1;
        }
    }
    let total = values.len() as f64;
    // (bin label, count) in bin order, "More" last.
    let mut rows: Vec<(V, usize)> = bins.iter().map(|b| V::Num(*b)).zip(counts.iter().copied()).collect();
    rows.push((V::text("More"), counts.last().copied().unwrap_or(0)));
    let mut out = Out::default();
    let head = Look::HEAD.over().under();
    let block = |out: &mut Out, col: u32, rows: &[(V, usize)]| {
        out.text(0, col, "Bin", head);
        out.text(0, col + 1, "Frequency", head);
        if o.cumulative {
            out.text(0, col + 2, "Cumulative %", head);
        }
        let mut acc = 0usize;
        for (k, (bin, c)) in rows.iter().enumerate() {
            let r = 1 + k as u32;
            let look = if k + 1 == rows.len() { Look::PLAIN.under() } else { Look::PLAIN };
            out.put(r, col, bin.clone(), look);
            out.num(r, col + 1, *c as f64, look);
            acc += c;
            if o.cumulative {
                out.num(r, col + 2, acc as f64 / total, look.fmt("0.00%"));
            }
        }
    };
    block(&mut out, 0, &rows);
    let width = 2 + u32::from(o.cumulative);
    let shown_col = if o.pareto {
        let mut sorted = rows.clone();
        // Most frequent first; ties keep bin order.
        sorted.sort_by_key(|r| std::cmp::Reverse(r.1));
        block(&mut out, width + 1, &sorted);
        width + 1
    } else {
        0
    };
    if o.chart {
        let n = rows.len() as u32;
        let mut series = vec![ChartSeries {
            name: Some("Frequency".into()),
            x: Some(Area::Out { row: 1, col: shown_col, rows: n, cols: 1 }),
            y: Area::Out { row: 1, col: shown_col + 1, rows: n, cols: 1 },
            line: false,
        }];
        if o.cumulative {
            series.push(ChartSeries {
                name: Some("Cumulative %".into()),
                x: Some(Area::Out { row: 1, col: shown_col, rows: n, cols: 1 }),
                y: Area::Out { row: 1, col: shown_col + 2, rows: n, cols: 1 },
                line: true,
            });
        }
        out.charts.push(Chart {
            kind: ChartKind::Column,
            title: "Histogram".into(),
            x_title: Some("Bin".into()),
            y_title: Some("Frequency".into()),
            series,
            row: 0,
            col: shown_col + width + 1,
            width: 360.0,
            height: 216.0,
            legend: true,
        });
    }
    Ok(out)
}

/// Fourier Analysis: the discrete Fourier transform (or its inverse) of 2^k complex values,
/// written as complex-number text.
pub fn fourier(values: &[(f64, f64)], inverse: bool, label: Option<&str>) -> Result<Out> {
    let f = crate::fft::fft(values, inverse)?;
    let mut out = Out::default();
    let mut r = 0;
    if let Some(l) = label {
        out.text(0, 0, l, Look::PLAIN);
        r = 1;
    }
    for (re, im) in f {
        let v = match dist::complex_text(re, im) {
            Ok(t) => V::Text(t),
            Err(e) => V::Error(e),
        };
        out.put(r, 0, v, Look::PLAIN);
        r += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(out: &Out, r: u32, c: u32) -> V {
        out.cells.iter().find(|x| x.0 == r && x.1 == c).map(|x| x.2.clone()).unwrap_or_else(|| panic!("nothing at {r},{c}"))
    }

    fn refs() -> (impl Fn(usize, usize) -> String, impl Fn(u32, u32, u32, u32) -> String) {
        let input = |a: usize, b: usize| if a == b { format!("A{}", a + 2) } else { format!("A{}:A{}", a + 2, b + 2) };
        let output = |r0: u32, c0: u32, r1: u32, c1: u32| {
            let cell = |r: u32, c: u32| format!("{}{}", (b'C' + c as u8) as char, r + 2);
            if (r0, c0) == (r1, c1) { cell(r0, c0) } else { format!("{}:{}", cell(r0, c0), cell(r1, c1)) }
        };
        (input, output)
    }

    #[test]
    fn moving_average_formulas() {
        let (i, o) = refs();
        let r = Refs { input: &i, output: &o, input_area: "Sheet1!$A$2:$A$7".into() };
        let out = moving_average(6, 3, true, true, &r).unwrap();
        assert_eq!(at(&out, 1, 0), V::Error(CellError::NA));
        assert_eq!(at(&out, 2, 0), V::Formula("=AVERAGE(A2:A4)".into()));
        assert_eq!(at(&out, 5, 0), V::Formula("=AVERAGE(A5:A7)".into()));
        assert_eq!(at(&out, 3, 1), V::Error(CellError::NA));
        assert_eq!(at(&out, 4, 1), V::Formula("=SQRT(SUMXMY2(A4:A6,C4:C6)/3)".into()));
        assert_eq!(out.charts[0].col, 3);
        assert!(moving_average(2, 3, false, false, &r).is_err());
    }

    #[test]
    fn exponential_smoothing_formulas() {
        let (i, o) = refs();
        let r = Refs { input: &i, output: &o, input_area: String::new() };
        let out = exponential_smoothing(5, 0.3, true, false, &r).unwrap();
        assert_eq!(at(&out, 0, 0), V::Error(CellError::NA));
        assert_eq!(at(&out, 1, 0), V::Formula("=A2".into()));
        assert_eq!(at(&out, 2, 0), V::Formula("=0.7*A3+0.3*C3".into()));
        assert_eq!(at(&out, 3, 1), V::Formula("=SQRT(SUMXMY2(A3:A5,C3:C5)/3)".into()));
        assert!(exponential_smoothing(5, 1.5, false, false, &r).is_err());
    }

    #[test]
    fn histogram_counts_and_pareto() {
        let x = [1.0, 2.0, 2.0, 3.0, 5.0, 9.0];
        let o = HistogramOptions { bins: Some(vec![4.0, 2.0]), pareto: true, cumulative: true, chart: true };
        let out = histogram(&x, &o).unwrap();
        // Bins 2, 4, More: (≤2) 3, (2,4] 1, More 2.
        assert_eq!((at(&out, 1, 0), at(&out, 1, 1)), (V::Num(2.0), V::Num(3.0)));
        assert_eq!((at(&out, 2, 0), at(&out, 2, 1)), (V::Num(4.0), V::Num(1.0)));
        assert_eq!((at(&out, 3, 0), at(&out, 3, 1)), (V::text("More"), V::Num(2.0)));
        assert_eq!(at(&out, 3, 2), V::Num(1.0));
        // Pareto: 3 (bin 2), 2 (More), 1 (bin 4).
        assert_eq!((at(&out, 2, 4), at(&out, 2, 5)), (V::text("More"), V::Num(2.0)));
        assert_eq!(out.charts[0].series.len(), 2);
        assert!(out.charts[0].series[1].line);
        // Automatic bins span the data.
        let out = histogram(&x, &HistogramOptions::default()).unwrap();
        assert_eq!(at(&out, 1, 0), V::Num(1.0));
        assert!(histogram(&[], &HistogramOptions::default()).is_err());
    }

    #[test]
    fn fourier_output() {
        let out = fourier(&[(1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (4.0, 0.0)], false, Some("Signal")).unwrap();
        assert_eq!(at(&out, 0, 0), V::text("Signal"));
        assert_eq!(at(&out, 1, 0), V::text("10"));
        assert_eq!(at(&out, 2, 0), V::text("-2+2i"));
        assert_eq!(at(&out, 4, 0), V::text("-2-2i"));
        assert!(fourier(&[(1.0, 0.0); 3], false, None).is_err());
    }
}
