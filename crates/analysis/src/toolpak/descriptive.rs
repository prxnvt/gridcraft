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
        out.text(0, col, &s.label, Look::HEAD.over().under());
        out.text(0, col + 1, "", Look::PLAIN.over().under());
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
