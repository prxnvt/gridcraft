//! Descriptive Statistics, Rank and Percentile, Correlation and Covariance.

use gridcraft_core::CellError;
use gridcraft_functions::dist;

use super::{Look, Out, Series, V, check_size};
use crate::{Result, input};

/// Descriptive Statistics options.
#[derive(Clone, Debug, PartialEq)]
pub struct DescriptiveOptions {
    pub summary: bool,
    /// Confidence level for the mean, in percent (95 → "Confidence Level(95.0%)").
    pub confidence: Option<f64>,
    pub kth_largest: Option<usize>,
    pub kth_smallest: Option<usize>,
}

/// The k-th largest (or smallest) value, 1-based (LARGE / SMALL).
fn kth(v: &[f64], k: usize, largest: bool) -> std::result::Result<f64, CellError> {
    if k == 0 || k > v.len() {
        return Err(CellError::Num);
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let i = if largest { s.len() - k } else { k - 1 };
    s.get(i).copied().ok_or(CellError::Num)
}

/// Two columns per variable: statistic names and values, under the variable's label.
pub fn descriptive_statistics(series: &[Series], o: &DescriptiveOptions) -> Result<Out> {
    check_size(series)?;
    if !o.summary && o.confidence.is_none() && o.kth_largest.is_none() && o.kth_smallest.is_none() {
        return Err(input("Choose at least one of Summary statistics, Confidence Level for Mean, Kth Largest or Kth Smallest."));
    }
    if let Some(c) = o.confidence
        && !(c > 0.0 && c < 100.0)
    {
        return Err(input("The confidence level must be between 0 and 100 percent."));
    }
    let mut out = Out::default();
    for (i, s) in series.iter().enumerate() {
        let col = 2 * i as u32;
        let x = s.numbers();
        if x.is_empty() {
            return Err(input(format!("{} has no numbers.", s.label)));
        }
        let n = x.len() as f64;
        out.text(0, col, &s.label, Look::HEAD.over().under().across());
        out.text(0, col + 1, "", Look::HEAD.over().under().across());
        let mut rows: Vec<(String, V)> = Vec::new();
        if o.summary {
            let mean = dist::mean(&x);
            let var = dist::var_s(&x);
            let sd = var.map(f64::sqrt);
            let min = x.iter().copied().fold(f64::INFINITY, f64::min);
            let max = x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            rows.push(("Mean".into(), V::stat(mean)));
            rows.push(("Standard Error".into(), V::stat(sd.map(|s| s / n.sqrt()))));
            rows.push(("Median".into(), V::stat(dist::median(&x))));
            rows.push(("Mode".into(), V::stat(dist::mode(&x))));
            rows.push(("Standard Deviation".into(), V::stat(sd)));
            rows.push(("Sample Variance".into(), V::stat(var)));
            rows.push(("Kurtosis".into(), V::stat(dist::kurt(&x))));
            rows.push(("Skewness".into(), V::stat(dist::skew(&x))));
            rows.push(("Range".into(), V::num_or_err(max - min)));
            rows.push(("Minimum".into(), V::num_or_err(min)));
            rows.push(("Maximum".into(), V::num_or_err(max)));
            rows.push(("Sum".into(), V::num_or_err(x.iter().sum())));
            rows.push(("Count".into(), V::Num(n)));
        }
        if let Some(k) = o.kth_largest {
            rows.push((format!("Largest({k})"), V::stat(kth(&x, k, true))));
        }
        if let Some(k) = o.kth_smallest {
            rows.push((format!("Smallest({k})"), V::stat(kth(&x, k, false))));
        }
        if let Some(level) = o.confidence {
            let ci = dist::var_s(&x).map(|v| dist::t_inv_2t(1.0 - level / 100.0, n - 1.0) * v.sqrt() / n.sqrt());
            rows.push((format!("Confidence Level({level:.1}%)"), V::stat(ci)));
        }
        let last = rows.len();
        for (k, (name, v)) in rows.into_iter().enumerate() {
            let r = 1 + k as u32;
            let look = if k + 1 == last { Look::PLAIN.under() } else { Look::PLAIN };
            out.text(r, col, &name, look);
            out.put(r, col + 1, v, look);
        }
    }
    Ok(out)
}

/// Rank and Percentile: per variable, its values from largest to smallest with their position
/// in the input (Point), rank (RANK.EQ, 1 = largest) and percent rank (PERCENTRANK.INC).
pub fn rank_and_percentile(series: &[Series]) -> Result<Out> {
    check_size(series)?;
    let mut out = Out::default();
    for (i, s) in series.iter().enumerate() {
        let col = 4 * i as u32;
        let head = Look::HEAD.over().under();
        out.text(0, col, "Point", head);
        out.text(0, col + 1, &s.label, head);
        out.text(0, col + 2, "Rank", head);
        out.text(0, col + 3, "Percent", head);
        // (point, value), largest first; equal values keep input order.
        let mut pts: Vec<(usize, f64)> = s.values.iter().enumerate().filter_map(|(k, v)| v.map(|x| (k + 1, x))).collect();
        if pts.is_empty() {
            return Err(input(format!("{} has no numbers.", s.label)));
        }
        pts.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let n = pts.len();
        let last = n;
        for (k, (point, x)) in pts.iter().enumerate() {
            let greater = pts.iter().filter(|p| p.1 > *x).count();
            let less = pts.iter().filter(|p| p.1 < *x).count();
            // PERCENTRANK.INC keeps three significant digits, rounding down.
            let pct = if n == 1 { 1.0 } else { ((less as f64 / (n - 1) as f64) * 1000.0 + 1e-9).floor() / 1000.0 };
            let r = 1 + k as u32;
            let look = if k + 1 == last { Look::PLAIN.under() } else { Look::PLAIN };
            out.num(r, col, *point as f64, look);
            out.num(r, col + 1, *x, look);
            out.num(r, col + 2, (greater + 1) as f64, look);
            out.num(r, col + 3, pct, look.fmt("0.00%"));
        }
    }
    Ok(out)
}

/// Pairs where both values are present.
fn pairs(a: &Series, b: &Series) -> (Vec<f64>, Vec<f64>) {
    a.values.iter().zip(&b.values).filter_map(|(x, y)| Some(((*x)?, (*y)?))).unzip()
}

pub(crate) fn correl(x: &[f64], y: &[f64]) -> std::result::Result<f64, CellError> {
    let (mx, my) = (dist::mean(x)?, dist::mean(y)?);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    if sxx == 0.0 || syy == 0.0 { Err(CellError::Div0) } else { Ok(sxy / (sxx * syy).sqrt()) }
}

/// Population covariance (the ToolPak's Covariance divides by n, like COVARIANCE.P).
fn covar(x: &[f64], y: &[f64]) -> std::result::Result<f64, CellError> {
    let (mx, my) = (dist::mean(x)?, dist::mean(y)?);
    Ok(x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum::<f64>() / x.len() as f64)
}

/// A lower-triangular matrix of a pairwise statistic, labelled across and down.
fn matrix(series: &[Series], f: fn(&[f64], &[f64]) -> std::result::Result<f64, CellError>) -> Result<Out> {
    check_size(series)?;
    let n = series.len();
    let mut out = Out::default();
    let head = Look::HEAD.over().under();
    out.text(0, 0, "", head);
    for (j, s) in series.iter().enumerate() {
        out.text(0, 1 + j as u32, &s.label, head);
    }
    for (i, a) in series.iter().enumerate() {
        let r = 1 + i as u32;
        let look = if i + 1 == n { Look::PLAIN.under() } else { Look::PLAIN };
        out.text(r, 0, &a.label, look.fmt("@"));
        for (j, b) in series.iter().enumerate().take(i + 1) {
            let (x, y) = pairs(a, b);
            out.put(r, 1 + j as u32, V::stat(f(&x, &y)), look);
        }
        for j in i + 1..n {
            out.text(r, 1 + j as u32, "", look);
        }
    }
    Ok(out)
}

/// Correlation: the Pearson correlation of every pair of variables.
pub fn correlation(series: &[Series]) -> Result<Out> {
    matrix(series, correl)
}

/// Covariance: the population covariance of every pair (variances on the diagonal).
pub fn covariance(series: &[Series]) -> Result<Out> {
    matrix(series, covar)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(out: &Out, label: &str) -> V {
        let (r, c) = out.cells.iter().find(|x| x.2 == V::text(label)).map(|x| (x.0, x.1)).unwrap_or_else(|| panic!("no {label}"));
        out.cells.iter().find(|x| x.0 == r && x.1 == c + 1).map(|x| x.2.clone()).unwrap()
    }

    fn close(v: V, want: f64) {
        match v {
            V::Num(x) => assert!((x - want).abs() < 1e-9 * want.abs().max(1.0), "{x} vs {want}"),
            other => panic!("{other:?} vs {want}"),
        }
    }

    #[test]
    fn descriptive_statistics_match_the_worksheet_functions() {
        let s = Series::new("Score", (1..=10).map(|i| Some(f64::from(i))).chain([None]).collect());
        let o = DescriptiveOptions { summary: true, confidence: Some(95.0), kth_largest: Some(2), kth_smallest: Some(3) };
        let out = descriptive_statistics(&[s], &o).unwrap();
        assert_eq!(out.cells[0].2, V::text("Score"));
        close(find(&out, "Mean"), 5.5);
        close(find(&out, "Standard Error"), 0.957_427_107_756_338);
        close(find(&out, "Median"), 5.5);
        assert_eq!(find(&out, "Mode"), V::Error(CellError::NA));
        close(find(&out, "Standard Deviation"), 3.027_650_354_097_49);
        close(find(&out, "Sample Variance"), 9.166_666_666_666_67);
        close(find(&out, "Kurtosis"), -1.2);
        close(find(&out, "Skewness"), 0.0);
        close(find(&out, "Range"), 9.0);
        close(find(&out, "Sum"), 55.0);
        close(find(&out, "Count"), 10.0);
        close(find(&out, "Largest(2)"), 9.0);
        close(find(&out, "Smallest(3)"), 3.0);
        close(find(&out, "Confidence Level(95.0%)"), 2.165_850_589_668_169);
        // Two variables sit side by side; too few values for kurtosis is #DIV/0!.
        let two = [Series::new("A", vec![Some(1.0), Some(2.0)]), Series::new("B", vec![Some(3.0), Some(3.0), Some(4.0)])];
        let out =
            descriptive_statistics(&two, &DescriptiveOptions { summary: true, confidence: None, kth_largest: None, kth_smallest: None }).unwrap();
        assert!(out.cells.iter().any(|c| c.1 == 2 && c.2 == V::text("B")));
        assert_eq!(find(&out, "Kurtosis"), V::Error(CellError::Div0));
    }

    fn at(out: &Out, r: u32, c: u32) -> V {
        out.cells.iter().find(|x| x.0 == r && x.1 == c).map(|x| x.2.clone()).unwrap_or_else(|| panic!("nothing at {r},{c}"))
    }

    #[test]
    fn rank_and_percentile_orders_largest_first() {
        let s = Series::new("Sales", vec![Some(10.0), Some(30.0), None, Some(20.0), Some(30.0)]);
        let out = rank_and_percentile(&[s]).unwrap();
        assert_eq!(at(&out, 0, 1), V::text("Sales"));
        // Points 2 and 5 tie for first; two of the other three values are smaller (2/3, kept to
        // three digits).
        assert_eq!((at(&out, 1, 0), at(&out, 1, 2), at(&out, 1, 3)), (V::Num(2.0), V::Num(1.0), V::Num(0.666)));
        assert_eq!((at(&out, 2, 0), at(&out, 2, 2)), (V::Num(5.0), V::Num(1.0)));
        assert_eq!((at(&out, 3, 0), at(&out, 3, 1), at(&out, 3, 2)), (V::Num(4.0), V::Num(20.0), V::Num(3.0)));
        close(at(&out, 3, 3), 0.333);
        assert_eq!(at(&out, 4, 3), V::Num(0.0));
        assert!(rank_and_percentile(&[Series::new("x", vec![None])]).is_err());
    }

    #[test]
    fn correlation_and_covariance_matrices() {
        let a = Series::new("A", vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        let b = Series::new("B", vec![Some(2.0), Some(4.0), Some(5.0), Some(9.0)]);
        let c = Series::new("C", vec![Some(4.0), Some(3.0), Some(2.0), None]);
        let out = correlation(&[a.clone(), b.clone(), c.clone()]).unwrap();
        assert_eq!(at(&out, 0, 1), V::text("A"));
        assert_eq!(at(&out, 1, 1), V::Num(1.0));
        close(at(&out, 2, 1), 0.964_763_821_237_732);
        assert_eq!(at(&out, 1, 2), V::text(""), "upper triangle is blank");
        close(at(&out, 3, 1), -1.0);
        let out = covariance(&[a, b]).unwrap();
        close(at(&out, 1, 1), 1.25);
        close(at(&out, 2, 1), 2.75);
        close(at(&out, 2, 2), 6.5);
    }

    #[test]
    fn descriptive_statistics_refuses_bad_options() {
        let s = [Series::new("A", vec![Some(1.0)])];
        let none = DescriptiveOptions { summary: false, confidence: None, kth_largest: None, kth_smallest: None };
        assert!(descriptive_statistics(&s, &none).is_err());
        assert!(descriptive_statistics(&s, &DescriptiveOptions { confidence: Some(100.0), ..none.clone() }).is_err());
        assert!(descriptive_statistics(&[Series::new("A", vec![None])], &DescriptiveOptions { summary: true, ..none.clone() }).is_err());
        assert!(descriptive_statistics(&[], &DescriptiveOptions { summary: true, ..none }).is_err());
        assert_eq!(kth(&[1.0, 2.0], 3, true), Err(CellError::Num));
    }
}
