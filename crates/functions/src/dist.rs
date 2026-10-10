//! The numeric core behind the statistical worksheet functions, as a public API.
//!
//! The Analysis ToolPak and Solver (crate `gridcraft-analysis`) compute with exactly the same
//! distributions and statistics as the cells do, so a ToolPak report and `=T.TEST(…)` agree to
//! the last digit. Invalid arguments give `NaN` (distributions) or a [`CellError`] (statistics).

use gridcraft_core::CellError;

use crate::{special, stat};

pub type R<T> = Result<T, CellError>;

/// Standard normal density.
pub fn norm_pdf(z: f64) -> f64 {
    special::norm_pdf(z)
}
/// Standard normal P(Z ≤ z).
pub fn norm_cdf(z: f64) -> f64 {
    special::norm_cdf(z)
}
/// z with P(Z ≤ z) = p.
pub fn norm_inv(p: f64) -> f64 {
    if !(p > 0.0 && p < 1.0) {
        return f64::NAN;
    }
    special::norm_inv(p)
}
pub fn ln_gamma(x: f64) -> f64 {
    special::ln_gamma(x)
}
pub fn gamma(x: f64) -> f64 {
    special::gamma(x)
}
pub fn erf(x: f64) -> f64 {
    special::erf(x)
}
pub fn erfc(x: f64) -> f64 {
    special::erfc(x)
}
/// Regularized incomplete beta I_x(a, b).
pub fn reg_inc_beta(x: f64, a: f64, b: f64) -> f64 {
    special::reg_inc_beta(x, a, b)
}
/// Regularized lower incomplete gamma P(a, x).
pub fn reg_gamma_p(a: f64, x: f64) -> f64 {
    special::reg_gamma_p(a, x)
}
/// Regularized upper incomplete gamma Q(a, x).
pub fn reg_gamma_q(a: f64, x: f64) -> f64 {
    special::reg_gamma_q(a, x)
}

fn ok_df(v: f64) -> bool {
    v.is_finite() && v > 0.0
}

/// Student's t: P(T ≤ t) with `v` degrees of freedom.
pub fn t_cdf(t: f64, v: f64) -> f64 {
    if !ok_df(v) || t.is_nan() {
        return f64::NAN;
    }
    stat::t_cdf(t, v)
}
/// Student's t: P(T > |t|) (one tail).
pub fn t_upper(t: f64, v: f64) -> f64 {
    if !ok_df(v) || t.is_nan() {
        return f64::NAN;
    }
    stat::t_upper(t.abs(), v)
}
/// Left-tailed inverse: t with P(T ≤ t) = p (T.INV).
pub fn t_inv(p: f64, v: f64) -> f64 {
    if !(p > 0.0 && p < 1.0) || !ok_df(v) {
        return f64::NAN;
    }
    if p < 0.5 { -stat::t_inv_upper(p, v) } else { stat::t_inv_upper(1.0 - p, v) }
}
/// Two-tailed critical value: t ≥ 0 with P(|T| > t) = p (T.INV.2T).
pub fn t_inv_2t(p: f64, v: f64) -> f64 {
    if !(p > 0.0 && p <= 1.0) || !ok_df(v) {
        return f64::NAN;
    }
    stat::t_inv_upper(p / 2.0, v)
}

/// F distribution: P(F ≤ x).
pub fn f_cdf(x: f64, d1: f64, d2: f64) -> f64 {
    if !ok_df(d1) || !ok_df(d2) || x.is_nan() {
        return f64::NAN;
    }
    stat::f_cdf(x, d1, d2)
}
/// F distribution: P(F > x).
pub fn f_upper(x: f64, d1: f64, d2: f64) -> f64 {
    if !ok_df(d1) || !ok_df(d2) || x.is_nan() {
        return f64::NAN;
    }
    stat::f_upper(x, d1, d2)
}
/// x with P(F ≤ x) = p (F.INV).
pub fn f_inv(p: f64, d1: f64, d2: f64) -> f64 {
    if !(0.0..1.0).contains(&p) || !ok_df(d1) || !ok_df(d2) {
        return f64::NAN;
    }
    if p == 0.0 {
        return 0.0;
    }
    special::invert(|x| stat::f_cdf(x, d1, d2), p, 0.0, 1.0, 1e60)
}
/// x with P(F > x) = p (F.INV.RT).
pub fn f_inv_rt(p: f64, d1: f64, d2: f64) -> f64 {
    if !(p > 0.0 && p <= 1.0) || !ok_df(d1) || !ok_df(d2) {
        return f64::NAN;
    }
    if p == 1.0 {
        return 0.0;
    }
    special::invert(|x| stat::f_upper(x, d1, d2), p, 0.0, 1.0, 1e60)
}

/// Chi-squared: x with P(X ≤ x) = p.
pub fn chi_inv(p: f64, v: f64) -> f64 {
    if !(0.0..1.0).contains(&p) || !ok_df(v) {
        return f64::NAN;
    }
    stat::chi_inv(p, v)
}
/// Chi-squared: x with P(X > x) = q.
pub fn chi_inv_rt(q: f64, v: f64) -> f64 {
    if !(q > 0.0 && q <= 1.0) || !ok_df(v) {
        return f64::NAN;
    }
    stat::chi_inv_rt(q, v)
}
/// Chi-squared: P(X > x).
pub fn chi_upper(x: f64, v: f64) -> f64 {
    if !ok_df(v) || x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 1.0;
    }
    special::reg_gamma_q(v / 2.0, x / 2.0)
}

pub fn binom_pmf(k: f64, n: f64, p: f64) -> f64 {
    stat::binom_pmf(k, n, p)
}
pub fn binom_cdf(k: f64, n: f64, p: f64) -> f64 {
    stat::binom_cdf(k, n, p)
}
pub fn poisson_pmf(x: f64, m: f64) -> f64 {
    stat::poisson_pmf(x, m)
}
pub fn poisson_cdf(x: f64, m: f64) -> f64 {
    stat::poisson_cdf(x, m)
}

pub fn mean(x: &[f64]) -> R<f64> {
    stat::mean(x)
}
/// Sum of squared deviations from the mean (DEVSQ).
pub fn devsq(x: &[f64]) -> f64 {
    stat::devsq(x)
}
/// Sample variance (VAR.S).
pub fn var_s(x: &[f64]) -> R<f64> {
    stat::var_s(x)
}
/// Population variance (VAR.P).
pub fn var_p(x: &[f64]) -> R<f64> {
    stat::var_p(x)
}
pub fn median(x: &[f64]) -> R<f64> {
    stat::median(x.to_vec())
}
/// Most frequent value, first in order of appearance (MODE.SNGL); `#N/A` when none repeats.
pub fn mode(x: &[f64]) -> R<f64> {
    stat::mode_sngl(x)
}
/// Excess kurtosis of a sample (KURT).
pub fn kurt(x: &[f64]) -> R<f64> {
    stat::kurt(x)
}
/// Sample skewness (SKEW).
pub fn skew(x: &[f64]) -> R<f64> {
    stat::skew(x)
}
/// Inclusive percentile, k in [0, 1] (PERCENTILE.INC).
pub fn percentile_inc(x: &[f64], k: f64) -> R<f64> {
    stat::percentile_inc(x.to_vec(), k)
}
/// Exclusive percentile, k in (0, 1) (PERCENTILE.EXC).
pub fn percentile_exc(x: &[f64], k: f64) -> R<f64> {
    stat::percentile_exc(x.to_vec(), k)
}

/// A least-squares fit (the numbers LINEST reports).
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    /// Slope per x column (0 for collinear columns that were dropped).
    pub coef: Vec<f64>,
    /// Intercept (0 when forced through the origin).
    pub intercept: f64,
    /// Standard error per slope.
    pub se: Vec<f64>,
    /// Standard error of the intercept (`None` when forced through the origin).
    pub se_intercept: Option<f64>,
    pub r2: R<f64>,
    /// Standard error of the y estimate.
    pub se_y: R<f64>,
    pub f: R<f64>,
    /// Residual degrees of freedom.
    pub df: f64,
    pub ss_reg: f64,
    pub ss_resid: f64,
}

/// Fits y ≈ intercept + Σ coef·x by least squares; `x` has one row per observation with `k`
/// values each. `constant = false` forces the line through the origin.
pub fn regress(y: &[f64], x: &[Vec<f64>], k: usize, constant: bool) -> R<Fit> {
    let f = stat::regress(y, x, k, constant)?;
    Ok(Fit {
        coef: f.coef,
        intercept: f.b,
        se: f.se,
        se_intercept: f.se_b,
        r2: f.r2,
        se_y: f.sey,
        f: f.f,
        df: f.df,
        ss_reg: f.ssreg,
        ss_resid: f.ssresid,
    })
}

/// Inverse of a symmetric positive semi-definite matrix; collinear variables are dropped (their
/// rows and columns are zero) and flagged.
pub fn sym_inverse(m: &[Vec<f64>]) -> (Vec<Vec<f64>>, Vec<bool>) {
    stat::sym_inverse(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn matches_known_values() {
        close(t_inv_2t(0.05, 10.0), 2.228_138_851_986_27, 1e-9);
        close(t_upper(2.228_138_851_986_27, 10.0), 0.025, 1e-9);
        close(f_inv_rt(0.05, 3.0, 20.0), 3.098_391_212_074_27, 1e-9);
        close(f_upper(3.098_391_212_074_27, 3.0, 20.0), 0.05, 1e-9);
        close(norm_inv(0.975), 1.959_963_984_540_05, 1e-9);
        close(chi_upper(3.841_458_820_694_12, 1.0), 0.05, 1e-10);
        close(var_s(&[1.0, 2.0, 3.0, 4.0]).unwrap_or(f64::NAN), 5.0 / 3.0, 1e-15);
        assert!(t_inv_2t(0.0, 3.0).is_nan() && f_cdf(1.0, -1.0, 2.0).is_nan() && norm_inv(1.0).is_nan());
        let fit = regress(&[1.0, 3.0, 5.0], &[vec![0.0], vec![1.0], vec![2.0]], 1, true).unwrap_or_else(|e| panic!("{e:?}"));
        close(fit.coef[0], 2.0, 1e-12);
        close(fit.intercept, 1.0, 1e-12);
    }
}
