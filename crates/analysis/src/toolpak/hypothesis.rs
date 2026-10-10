//! The hypothesis tests: ANOVA (single factor, two-factor with and without replication),
//! the F-test for variances, the three t-tests and the two-sample z-test.

use gridcraft_core::CellError;
use gridcraft_functions::dist;

use super::{Look, Out, Series, V, check_alpha, check_size};
use crate::{Result, input};

type Stat = std::result::Result<f64, CellError>;

fn head() -> Look {
    Look::HEAD.over().under()
}

/// Writes a `(label, values…)` row; the last row of a table gets a bottom border.
fn row(out: &mut Out, r: u32, col: u32, label: &str, values: &[V], last: bool) {
    let look = if last { Look::PLAIN.under() } else { Look::PLAIN };
    out.text(r, col, label, look);
    for (k, v) in values.iter().enumerate() {
        out.put(r, col + 1 + k as u32, v.clone(), look);
    }
}

fn headers(out: &mut Out, r: u32, col: u32, names: &[&str]) {
    for (k, n) in names.iter().enumerate() {
        out.text(r, col + k as u32, n, head());
    }
}

/// F statistic's p-value and critical value.
fn f_test_values(f: f64, d1: f64, d2: f64, alpha: f64) -> (V, V) {
    (V::num_or_err(dist::f_upper(f, d1, d2)), V::num_or_err(dist::f_inv_rt(alpha, d1, d2)))
}

/// One "Source of Variation" table row with an F test: SS, df, MS, F, P-value, F crit.
fn anova_row(ss: f64, df: f64, ms_err: f64, df_err: f64, alpha: f64) -> Vec<V> {
    let ms = ss / df;
    let f = ms / ms_err;
    let (p, crit) = f_test_values(f, df, df_err, alpha);
    vec![V::num_or_err(ss), V::Num(df), V::num_or_err(ms), V::num_or_err(f), p, crit]
}

fn summary_values(x: &[f64]) -> Vec<V> {
    let sum: f64 = x.iter().sum();
    vec![V::Num(x.len() as f64), V::num_or_err(sum), V::stat(dist::mean(x)), V::stat(dist::var_s(x))]
}

const SOURCE: [&str; 7] = ["Source of Variation", "SS", "df", "MS", "F", "P-value", "F crit"];

/// Anova: Single Factor — one group per variable.
pub fn anova_single_factor(series: &[Series], alpha: f64) -> Result<Out> {
    check_size(series)?;
    let alpha = check_alpha(alpha)?;
    let groups: Vec<Vec<f64>> = series.iter().map(Series::numbers).collect();
    if groups.len() < 2 || groups.iter().any(Vec::is_empty) {
        return Err(input("Anova needs at least two groups, each with at least one number."));
    }
    let all: Vec<f64> = groups.iter().flatten().copied().collect();
    let grand = dist::mean(&all).unwrap_or(0.0);
    let ss_between: f64 = groups.iter().map(|g| g.len() as f64 * (dist::mean(g).unwrap_or(0.0) - grand).powi(2)).sum();
    let ss_within: f64 = groups.iter().map(|g| dist::devsq(g)).sum();
    let (k, n) = (groups.len() as f64, all.len() as f64);
    if n - k < 1.0 {
        return Err(input("Anova needs more numbers than groups."));
    }
    let mut out = Out::default();
    out.text(0, 0, "Anova: Single Factor", Look::PLAIN);
    out.text(2, 0, "SUMMARY", Look::PLAIN);
    headers(&mut out, 3, 0, &["Groups", "Count", "Sum", "Average", "Variance"]);
    for (i, (s, g)) in series.iter().zip(&groups).enumerate() {
        row(&mut out, 4 + i as u32, 0, &s.label, &summary_values(g), i + 1 == groups.len());
    }
    let r = 4 + groups.len() as u32 + 2;
    out.text(r, 0, "ANOVA", Look::PLAIN);
    headers(&mut out, r + 1, 0, &SOURCE);
    let ms_within = ss_within / (n - k);
    row(&mut out, r + 2, 0, "Between Groups", &anova_row(ss_between, k - 1.0, ms_within, n - k, alpha), false);
    row(&mut out, r + 3, 0, "Within Groups", &[V::num_or_err(ss_within), V::Num(n - k), V::num_or_err(ms_within)], false);
    row(&mut out, r + 5, 0, "Total", &[V::num_or_err(ss_between + ss_within), V::Num(n - 1.0)], true);
    Ok(out)
}

/// A table of numbers with row and column labels (two-factor ANOVA input).
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub row_labels: Vec<String>,
    pub col_labels: Vec<String>,
    /// `data[row][col]`.
    pub data: Vec<Vec<f64>>,
}

impl Grid {
    fn check(&self) -> Result<(usize, usize)> {
        let rows = self.data.len();
        let cols = self.data.first().map_or(0, Vec::len);
        if rows < 2 || cols < 2 || self.data.iter().any(|r| r.len() != cols) {
            return Err(input("The input needs at least two rows and two columns of numbers."));
        }
        if rows * cols > super::MAX_VALUES {
            return Err(crate::Error::TooLarge("The input range is too large.".into()));
        }
        Ok((rows, cols))
    }
    fn col(&self, j: usize) -> Vec<f64> {
        self.data.iter().filter_map(|r| r.get(j).copied()).collect()
    }
}

/// Anova: Two-Factor Without Replication — one observation per row and column.
pub fn anova_two_factor_without(g: &Grid, alpha: f64) -> Result<Out> {
    let alpha = check_alpha(alpha)?;
    let (r, c) = g.check()?;
    let all: Vec<f64> = g.data.iter().flatten().copied().collect();
    let grand = dist::mean(&all).unwrap_or(0.0);
    let ss_rows: f64 = g.data.iter().map(|row| c as f64 * (dist::mean(row).unwrap_or(0.0) - grand).powi(2)).sum();
    let ss_cols: f64 = (0..c).map(|j| r as f64 * (dist::mean(&g.col(j)).unwrap_or(0.0) - grand).powi(2)).sum();
    let ss_total = dist::devsq(&all);
    let ss_err = ss_total - ss_rows - ss_cols;
    let (dr, dc) = ((r - 1) as f64, (c - 1) as f64);
    let de = dr * dc;
    let mut out = Out::default();
    out.text(0, 0, "Anova: Two-Factor Without Replication", Look::PLAIN);
    headers(&mut out, 2, 0, &["SUMMARY", "Count", "Sum", "Average", "Variance"]);
    let mut at = 3;
    for (i, row_values) in g.data.iter().enumerate() {
        let label = g.row_labels.get(i).cloned().unwrap_or_default();
        row(&mut out, at, 0, &label, &summary_values(row_values), false);
        at += 1;
    }
    at += 1;
    for j in 0..c {
        let label = g.col_labels.get(j).cloned().unwrap_or_default();
        row(&mut out, at, 0, &label, &summary_values(&g.col(j)), j + 1 == c);
        at += 1;
    }
    at += 2;
    out.text(at, 0, "ANOVA", Look::PLAIN);
    headers(&mut out, at + 1, 0, &SOURCE);
    let ms_err = ss_err / de;
    row(&mut out, at + 2, 0, "Rows", &anova_row(ss_rows, dr, ms_err, de, alpha), false);
    row(&mut out, at + 3, 0, "Columns", &anova_row(ss_cols, dc, ms_err, de, alpha), false);
    row(&mut out, at + 4, 0, "Error", &[V::num_or_err(ss_err), V::Num(de), V::num_or_err(ms_err)], false);
    row(&mut out, at + 6, 0, "Total", &[V::num_or_err(ss_total), V::Num((r * c - 1) as f64)], true);
    Ok(out)
}

/// Anova: Two-Factor With Replication — `per` consecutive rows form each sample (row factor
/// level); `g.row_labels` holds one label per sample.
pub fn anova_two_factor_with(g: &Grid, per: usize, alpha: f64) -> Result<Out> {
    let alpha = check_alpha(alpha)?;
    let (rows, c) = g.check()?;
    if per < 2 || rows % per != 0 {
        return Err(input("Rows per sample must be at least 2 and divide the number of data rows evenly."));
    }
    let a = rows / per;
    if a < 2 {
        return Err(input("The input needs at least two samples."));
    }
    let cell =
        |i: usize, j: usize| -> Vec<f64> { (i * per..(i + 1) * per).filter_map(|k| g.data.get(k).and_then(|row| row.get(j)).copied()).collect() };
    let all: Vec<f64> = g.data.iter().flatten().copied().collect();
    let grand = dist::mean(&all).unwrap_or(0.0);
    let sample_rows = |i: usize| -> Vec<f64> { (i * per..(i + 1) * per).filter_map(|k| g.data.get(k)).flatten().copied().collect() };
    let ss_sample: f64 = (0..a).map(|i| (per * c) as f64 * (dist::mean(&sample_rows(i)).unwrap_or(0.0) - grand).powi(2)).sum();
    let ss_cols: f64 = (0..c).map(|j| rows as f64 * (dist::mean(&g.col(j)).unwrap_or(0.0) - grand).powi(2)).sum();
    let ss_within: f64 = (0..a).flat_map(|i| (0..c).map(move |j| (i, j))).map(|(i, j)| dist::devsq(&cell(i, j))).sum();
    let ss_total = dist::devsq(&all);
    let ss_inter = ss_total - ss_sample - ss_cols - ss_within;
    let (da, db) = ((a - 1) as f64, (c - 1) as f64);
    let dw = (a * c * (per - 1)) as f64;
    let mut out = Out::default();
    out.text(0, 0, "Anova: Two-Factor With Replication", Look::PLAIN);
    let mut names: Vec<&str> = vec!["SUMMARY"];
    names.extend(g.col_labels.iter().map(String::as_str));
    names.push("Total");
    headers(&mut out, 2, 0, &names);
    let mut at = 3;
    let block = |out: &mut Out, at: &mut u32, label: &str, cols: Vec<Vec<f64>>, last: bool| {
        out.text(*at, 0, label, Look::PLAIN.fmt("@"));
        *at += 1;
        let total: Vec<f64> = cols.iter().flatten().copied().collect();
        let mut sets = cols;
        sets.push(total);
        let stats: Vec<Vec<V>> = sets.iter().map(|x| summary_values(x)).collect();
        for (k, name) in ["Count", "Sum", "Average", "Variance"].iter().enumerate() {
            let vals: Vec<V> = stats.iter().map(|s| s.get(k).cloned().unwrap_or(V::Error(CellError::NA))).collect();
            row(out, *at, 0, name, &vals, last && k == 3);
            *at += 1;
        }
        *at += 1;
    };
    for i in 0..a {
        let label = g.row_labels.get(i).cloned().unwrap_or_default();
        block(&mut out, &mut at, &label, (0..c).map(|j| cell(i, j)).collect(), false);
    }
    // The column totals block has no Total column.
    out.text(at, 0, "Total", Look::PLAIN);
    at += 1;
    let stats: Vec<Vec<V>> = (0..c).map(|j| summary_values(&g.col(j))).collect();
    for (k, name) in ["Count", "Sum", "Average", "Variance"].iter().enumerate() {
        let vals: Vec<V> = stats.iter().map(|s| s.get(k).cloned().unwrap_or(V::Error(CellError::NA))).collect();
        row(&mut out, at, 0, name, &vals, k == 3);
        at += 1;
    }
    at += 2;
    out.text(at, 0, "ANOVA", Look::PLAIN);
    headers(&mut out, at + 1, 0, &SOURCE);
    let ms_w = ss_within / dw;
    row(&mut out, at + 2, 0, "Sample", &anova_row(ss_sample, da, ms_w, dw, alpha), false);
    row(&mut out, at + 3, 0, "Columns", &anova_row(ss_cols, db, ms_w, dw, alpha), false);
    row(&mut out, at + 4, 0, "Interaction", &anova_row(ss_inter, da * db, ms_w, dw, alpha), false);
    row(&mut out, at + 5, 0, "Within", &[V::num_or_err(ss_within), V::Num(dw), V::num_or_err(ms_w)], false);
    row(&mut out, at + 7, 0, "Total", &[V::num_or_err(ss_total), V::Num((all.len() - 1) as f64)], true);
    Ok(out)
}

/// A two-variable results table: title, a blank row, headers, then `(label, v1, v2)` rows.
fn two_column(title: &str, a: &Series, b: &Series, rows: Vec<(&str, V, Option<V>)>) -> Out {
    let mut out = Out::default();
    out.text(0, 0, title, Look::PLAIN);
    headers(&mut out, 2, 0, &["", &a.label, &b.label]);
    let n = rows.len();
    for (k, (label, v1, v2)) in rows.into_iter().enumerate() {
        let r = 3 + k as u32;
        let look = if k + 1 == n { Look::PLAIN.under() } else { Look::PLAIN };
        out.text(r, 0, label, look);
        out.put(r, 1, v1, look);
        match v2 {
            Some(v) => out.put(r, 2, v, look),
            None => out.text(r, 2, "", look),
        }
    }
    out
}

fn numbers_of(s: &Series) -> Result<Vec<f64>> {
    let x = s.numbers();
    if x.len() < 2 {
        return Err(input(format!("{} needs at least two numbers.", s.label)));
    }
    Ok(x)
}

/// F-Test Two-Sample for Variances (one-tailed, on the side the F statistic falls).
pub fn f_test(a: &Series, b: &Series, alpha: f64) -> Result<Out> {
    let alpha = check_alpha(alpha)?;
    let (x, y) = (numbers_of(a)?, numbers_of(b)?);
    let (v1, v2) = (dist::var_s(&x), dist::var_s(&y));
    let (d1, d2) = ((x.len() - 1) as f64, (y.len() - 1) as f64);
    let f: Stat = match (v1, v2) {
        (Ok(p), Ok(q)) if q != 0.0 => Ok(p / q),
        _ => Err(CellError::Div0),
    };
    let (p, crit) = match f {
        // Below 1 the lower tail is tested.
        Ok(f) if f < 1.0 => (V::num_or_err(dist::f_cdf(f, d1, d2)), V::num_or_err(dist::f_inv(alpha, d1, d2))),
        Ok(f) => f_test_values(f, d1, d2, alpha),
        Err(e) => (V::Error(e), V::Error(e)),
    };
    let rows = vec![
        ("Mean", V::stat(dist::mean(&x)), Some(V::stat(dist::mean(&y)))),
        ("Variance", V::stat(v1), Some(V::stat(v2))),
        ("Observations", V::Num(x.len() as f64), Some(V::Num(y.len() as f64))),
        ("df", V::Num(d1), Some(V::Num(d2))),
        ("F", V::stat(f), None),
        ("P(F<=f) one-tail", p, None),
        ("F Critical one-tail", crit, None),
    ];
    Ok(two_column("F-Test Two-Sample for Variances", a, b, rows))
}

/// Which two-sample t-test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TTest {
    Paired,
    EqualVariances,
    UnequalVariances,
}

/// The t-test results rows after the test-specific ones: t Stat, one- and two-tailed p-values
/// and critical values.
fn t_tail_rows(t: Stat, df: f64, alpha: f64) -> Vec<(&'static str, V, Option<V>)> {
    let p1 = t.map(|t| dist::t_upper(t.abs(), df));
    vec![
        ("t Stat", V::stat(t), None),
        ("P(T<=t) one-tail", V::stat(p1), None),
        ("t Critical one-tail", V::num_or_err(dist::t_inv_2t(2.0 * alpha, df)), None),
        ("P(T<=t) two-tail", V::stat(p1.map(|p| (2.0 * p).min(1.0))), None),
        ("t Critical two-tail", V::num_or_err(dist::t_inv_2t(alpha, df)), None),
    ]
}

/// The three two-sample t-tests, with hypothesized mean difference `diff`.
pub fn t_test(kind: TTest, a: &Series, b: &Series, diff: f64, alpha: f64) -> Result<Out> {
    let alpha = check_alpha(alpha)?;
    if !diff.is_finite() {
        return Err(input("The hypothesized mean difference must be a number."));
    }
    let (x, y) = match kind {
        TTest::Paired => {
            let (x, y) = (a.complete()?, b.complete()?);
            if x.len() != y.len() {
                return Err(input("A paired t-test needs the same number of values in both ranges."));
            }
            if x.len() < 2 {
                return Err(input("A paired t-test needs at least two pairs."));
            }
            (x, y)
        }
        _ => (numbers_of(a)?, numbers_of(b)?),
    };
    let (n1, n2) = (x.len() as f64, y.len() as f64);
    let (m1, m2) = (dist::mean(&x).unwrap_or(0.0), dist::mean(&y).unwrap_or(0.0));
    let (v1, v2) = (dist::var_s(&x).unwrap_or(f64::NAN), dist::var_s(&y).unwrap_or(f64::NAN));
    let mut rows: Vec<(&str, V, Option<V>)> = vec![
        ("Mean", V::num_or_err(m1), Some(V::num_or_err(m2))),
        ("Variance", V::num_or_err(v1), Some(V::num_or_err(v2))),
        ("Observations", V::Num(n1), Some(V::Num(n2))),
    ];
    let div = |num: f64, den: f64| -> Stat { if den > 0.0 && den.is_finite() { Ok(num / den) } else { Err(CellError::Div0) } };
    let (t, df, title) = match kind {
        TTest::Paired => {
            let d: Vec<f64> = x.iter().zip(&y).map(|(p, q)| p - q).collect();
            let sd = dist::var_s(&d).unwrap_or(f64::NAN).sqrt();
            let r = super::descriptive_correl(&x, &y);
            rows.push(("Pearson Correlation", V::stat(r), None));
            rows.push(("Hypothesized Mean Difference", V::Num(diff), None));
            let df = n1 - 1.0;
            rows.push(("df", V::Num(df), None));
            (div(dist::mean(&d).unwrap_or(0.0) - diff, sd / n1.sqrt()), df, "t-Test: Paired Two Sample for Means")
        }
        TTest::EqualVariances => {
            let df = n1 + n2 - 2.0;
            let pooled = ((n1 - 1.0) * v1 + (n2 - 1.0) * v2) / df;
            rows.push(("Pooled Variance", V::num_or_err(pooled), None));
            rows.push(("Hypothesized Mean Difference", V::Num(diff), None));
            rows.push(("df", V::Num(df), None));
            (div(m1 - m2 - diff, (pooled * (1.0 / n1 + 1.0 / n2)).sqrt()), df, "t-Test: Two-Sample Assuming Equal Variances")
        }
        TTest::UnequalVariances => {
            let (a1, a2) = (v1 / n1, v2 / n2);
            // Welch–Satterthwaite degrees of freedom, rounded to a whole number as the ToolPak does.
            let df = ((a1 + a2).powi(2) / (a1 * a1 / (n1 - 1.0) + a2 * a2 / (n2 - 1.0))).round();
            rows.push(("Hypothesized Mean Difference", V::Num(diff), None));
            rows.push(("df", V::num_or_err(df), None));
            (div(m1 - m2 - diff, (a1 + a2).sqrt()), df, "t-Test: Two-Sample Assuming Unequal Variances")
        }
    };
    rows.extend(t_tail_rows(t, df, alpha));
    Ok(two_column(title, a, b, rows))
}

/// z-Test: Two Sample for Means, with known variances.
pub fn z_test(a: &Series, b: &Series, diff: f64, var1: f64, var2: f64, alpha: f64) -> Result<Out> {
    let alpha = check_alpha(alpha)?;
    if !(var1 > 0.0 && var2 > 0.0 && var1.is_finite() && var2.is_finite()) {
        return Err(input("Enter the known variance of each variable (a positive number)."));
    }
    let (x, y) = (a.numbers(), b.numbers());
    if x.is_empty() || y.is_empty() {
        return Err(input("Each variable needs at least one number."));
    }
    let (n1, n2) = (x.len() as f64, y.len() as f64);
    let (m1, m2) = (dist::mean(&x).unwrap_or(0.0), dist::mean(&y).unwrap_or(0.0));
    let z = (m1 - m2 - diff) / (var1 / n1 + var2 / n2).sqrt();
    let p1 = 1.0 - dist::norm_cdf(z.abs());
    let rows = vec![
        ("Mean", V::num_or_err(m1), Some(V::num_or_err(m2))),
        ("Known Variance", V::Num(var1), Some(V::Num(var2))),
        ("Observations", V::Num(n1), Some(V::Num(n2))),
        ("Hypothesized Mean Difference", V::Num(diff), None),
        ("z", V::num_or_err(z), None),
        ("P(Z<=z) one-tail", V::num_or_err(p1), None),
        ("z Critical one-tail", V::num_or_err(dist::norm_inv(1.0 - alpha)), None),
        ("P(Z<=z) two-tail", V::num_or_err((2.0 * p1).min(1.0)), None),
        ("z Critical two-tail", V::num_or_err(dist::norm_inv(1.0 - alpha / 2.0)), None),
    ];
    Ok(two_column("z-Test: Two Sample for Means", a, b, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(out: &Out, label: &str, col: u32) -> V {
        // The last row with that label (ANOVA's "Total" comes after the summary's).
        let r = out.cells.iter().filter(|c| c.1 == 0 && c.2 == V::text(label)).map(|c| c.0).max().unwrap_or_else(|| panic!("no row {label}"));
        out.cells.iter().find(|c| c.0 == r && c.1 == col).map(|c| c.2.clone()).unwrap_or_else(|| panic!("no {label} col {col}"))
    }

    fn close(v: V, want: f64, tol: f64) {
        match v {
            V::Num(x) => assert!((x - want).abs() <= tol * want.abs().max(1.0), "{x} vs {want}"),
            other => panic!("{other:?} vs {want}"),
        }
    }

    fn s(label: &str, v: &[f64]) -> Series {
        Series::new(label, v.iter().map(|x| Some(*x)).collect())
    }

    #[test]
    fn single_factor_anova() {
        let g = [s("A", &[4.0, 5.0, 6.0]), s("B", &[6.0, 7.0, 8.0]), s("C", &[9.0, 10.0, 11.0])];
        let out = anova_single_factor(&g, 0.05).unwrap();
        close(find(&out, "Between Groups", 1), 38.0, 1e-12);
        close(find(&out, "Between Groups", 2), 2.0, 0.0);
        close(find(&out, "Between Groups", 4), 19.0, 1e-12);
        // F(2, 6) upper tail in closed form: (1 + 2F/6)^-3.
        close(find(&out, "Between Groups", 5), (1.0f64 + 2.0 * 19.0 / 6.0).powi(-3), 1e-9);
        close(find(&out, "Between Groups", 6), 5.143_252_849_784_72, 1e-9);
        close(find(&out, "Within Groups", 1), 6.0, 1e-12);
        close(find(&out, "Total", 1), 44.0, 1e-12);
        close(find(&out, "Total", 2), 8.0, 0.0);
        close(find(&out, "B", 3), 7.0, 0.0);
        assert!(anova_single_factor(&g[..1], 0.05).is_err());
        assert!(anova_single_factor(&g, 1.5).is_err());
    }

    #[test]
    fn two_factor_anovas() {
        let g = Grid {
            row_labels: vec!["R1".into(), "R2".into(), "R3".into()],
            col_labels: vec!["C1".into(), "C2".into()],
            data: vec![vec![1.0, 3.0], vec![2.0, 5.0], vec![4.0, 8.0]],
        };
        let out = anova_two_factor_without(&g, 0.05).unwrap();
        // Row means 2, 3.5, 6; column means 7/3, 16/3; grand 23/6.
        close(find(&out, "Rows", 1), 16.333_333_333_333_3, 1e-12);
        close(find(&out, "Columns", 1), 13.5, 1e-12);
        close(find(&out, "Error", 1), 1.0, 1e-12);
        close(find(&out, "Total", 1), 30.833_333_333_333_3, 1e-12);
        close(find(&out, "Rows", 4), 16.333_333_333_333_3, 1e-12);
        // With replication: two samples of two rows each.
        let g = Grid {
            row_labels: vec!["Low".into(), "High".into()],
            col_labels: vec!["A".into(), "B".into()],
            data: vec![vec![1.0, 4.0], vec![3.0, 6.0], vec![5.0, 6.0], vec![7.0, 10.0]],
        };
        let out = anova_two_factor_with(&g, 2, 0.05).unwrap();
        // Sample means 3.5, 7; column means 4, 6.5; grand 5.25.
        close(find(&out, "Sample", 1), 24.5, 1e-12);
        close(find(&out, "Columns", 1), 12.5, 1e-12);
        close(find(&out, "Interaction", 1), 0.5, 1e-12);
        close(find(&out, "Within", 1), 14.0, 1e-12);
        close(find(&out, "Within", 2), 4.0, 0.0);
        close(find(&out, "Total", 1), 51.5, 1e-12);
        assert!(anova_two_factor_with(&g, 3, 0.05).is_err());
    }

    #[test]
    fn f_t_and_z_tests() {
        let a = s("A", &[10.0, 12.0, 9.0, 15.0, 11.0]);
        let b = s("B", &[8.0, 9.0, 7.0, 10.0, 9.0, 8.0]);
        let out = f_test(&a, &b, 0.05).unwrap();
        close(find(&out, "Variance", 1), 5.3, 1e-12);
        close(find(&out, "Variance", 2), 1.1, 1e-12);
        close(find(&out, "F", 1), 5.3 / 1.1, 1e-12);
        close(find(&out, "P(F<=f) one-tail", 1), dist::f_upper(5.3 / 1.1, 4.0, 5.0), 1e-12);
        let out = t_test(TTest::EqualVariances, &a, &b, 0.0, 0.05).unwrap();
        close(find(&out, "df", 1), 9.0, 0.0);
        let pooled = (4.0 * 5.3 + 5.0 * 1.1) / 9.0;
        close(find(&out, "Pooled Variance", 1), pooled, 1e-12);
        let t = (11.4 - 8.5) / (pooled * (1.0 / 5.0 + 1.0 / 6.0)).sqrt();
        close(find(&out, "t Stat", 1), t, 1e-12);
        close(find(&out, "P(T<=t) two-tail", 1), 2.0 * dist::t_upper(t, 9.0), 1e-12);
        close(find(&out, "t Critical two-tail", 1), 2.262_157_162_798_2, 1e-9);
        let out = t_test(TTest::UnequalVariances, &a, &b, 0.0, 0.05).unwrap();
        close(find(&out, "df", 1), 5.0, 0.0);
        let p = t_test(TTest::Paired, &s("X", &[1.0, 2.0, 3.0]), &s("Y", &[2.0, 2.5, 4.5]), 0.0, 0.05).unwrap();
        close(find(&p, "Pearson Correlation", 1), 2.5 / 7.0f64.sqrt(), 1e-12);
        close(find(&p, "df", 1), 2.0, 0.0);
        assert!(t_test(TTest::Paired, &a, &b, 0.0, 0.05).is_err(), "unequal lengths");
        let z = z_test(&a, &b, 0.0, 5.0, 1.0, 0.05).unwrap();
        let zv = 2.9 / (5.0 / 5.0 + 1.0 / 6.0f64).sqrt();
        close(find(&z, "z", 1), zv, 1e-12);
        close(find(&z, "z Critical two-tail", 1), 1.959_963_984_540_05, 1e-9);
        assert!(z_test(&a, &b, 0.0, 0.0, 1.0, 0.05).is_err());
    }
}
