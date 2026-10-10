//! Regression: least squares of one Y variable on up to 16 X variables, with the ToolPak's
//! summary output, residual and probability tables, and residual, line fit and normal
//! probability plots.

use gridcraft_core::CellError;
use gridcraft_functions::dist;

use super::{Area, Chart, ChartKind, ChartSeries, Look, Out, Series, V};
use crate::{Error, Result, input};

/// Most X variables (Excel's limit).
pub const MAX_X: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub struct RegressionOptions {
    pub constant_is_zero: bool,
    /// Confidence level in percent for the second pair of coefficient bounds (95 by default).
    pub confidence: f64,
    pub residuals: bool,
    pub standardized_residuals: bool,
    pub residual_plots: bool,
    pub line_fit_plots: bool,
    pub normal_probability_plots: bool,
}

impl Default for RegressionOptions {
    fn default() -> Self {
        RegressionOptions {
            constant_is_zero: false,
            confidence: 95.0,
            residuals: false,
            standardized_residuals: false,
            residual_plots: false,
            line_fit_plots: false,
            normal_probability_plots: false,
        }
    }
}

/// References to the input values (for the plots): Y, then each X variable.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputRefs {
    pub y: String,
    pub x: Vec<String>,
}

fn head() -> Look {
    Look::HEAD.over().under()
}

fn level_text(level: f64) -> String {
    format!("{level:.1}%")
}

/// Runs the regression of `y` on `xs` (each a variable with the same number of values).
pub fn regression(y: &Series, xs: &[Series], o: &RegressionOptions, refs: &InputRefs) -> Result<Out> {
    if xs.is_empty() {
        return Err(input("Give at least one X variable."));
    }
    if xs.len() > MAX_X {
        return Err(Error::TooLarge(format!("Regression takes at most {MAX_X} X variables.")));
    }
    if !(o.confidence > 0.0 && o.confidence < 100.0) {
        return Err(input("The confidence level must be between 0 and 100 percent."));
    }
    let yv = y.complete()?;
    let n = yv.len();
    let mut cols: Vec<Vec<f64>> = Vec::with_capacity(xs.len());
    for x in xs {
        let v = x.complete()?;
        if v.len() != n {
            return Err(input("The Y and X ranges must have the same number of values."));
        }
        cols.push(v);
    }
    if n > super::MAX_VALUES / (xs.len() + 1) {
        return Err(Error::TooLarge("The input ranges are too large.".into()));
    }
    let k = xs.len();
    let constant = !o.constant_is_zero;
    if n <= k + usize::from(constant) {
        return Err(input("Regression needs more observations than coefficients."));
    }
    let rows: Vec<Vec<f64>> = (0..n).map(|i| cols.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).collect()).collect();
    let fit = dist::regress(&yv, &rows, k, constant).map_err(|_| Error::Numeric("The regression could not be computed.".into()))?;
    let nf = n as f64;
    let df_reg = (k - fit.coef.iter().zip(&fit.se).filter(|(c, s)| **c == 0.0 && **s == 0.0).count().min(k)) as f64;
    let df_res = fit.df;
    let r2 = fit.r2.unwrap_or(f64::NAN);
    let mut out = Out::default();
    out.text(0, 0, "SUMMARY OUTPUT", Look::PLAIN);
    out.text(2, 0, "Regression Statistics", head().across());
    out.text(2, 1, "", head().across());
    let adj = 1.0 - (1.0 - r2) * (nf - f64::from(u8::from(constant))) / df_res;
    let stats = [
        ("Multiple R", r2.sqrt()),
        ("R Square", r2),
        ("Adjusted R Square", adj),
        ("Standard Error", fit.se_y.unwrap_or(f64::NAN)),
        ("Observations", nf),
    ];
    for (i, (name, v)) in stats.iter().enumerate() {
        let look = if i + 1 == stats.len() { Look::PLAIN.under() } else { Look::PLAIN };
        out.text(3 + i as u32, 0, name, look);
        out.num(3 + i as u32, 1, *v, look);
    }
    // ANOVA.
    out.text(9, 0, "ANOVA", Look::PLAIN);
    for (j, h) in ["", "df", "SS", "MS", "F", "Significance F"].iter().enumerate() {
        out.text(10, j as u32, h, head());
    }
    let ms_reg = fit.ss_reg / df_reg;
    let ms_res = fit.ss_resid / df_res;
    let f = ms_reg / ms_res;
    out.text(11, 0, "Regression", Look::PLAIN);
    out.num(11, 1, df_reg, Look::PLAIN);
    out.num(11, 2, fit.ss_reg, Look::PLAIN);
    out.num(11, 3, ms_reg, Look::PLAIN);
    out.num(11, 4, f, Look::PLAIN);
    out.num(11, 5, dist::f_upper(f, df_reg, df_res), Look::PLAIN);
    out.text(12, 0, "Residual", Look::PLAIN);
    out.num(12, 1, df_res, Look::PLAIN);
    out.num(12, 2, fit.ss_resid, Look::PLAIN);
    out.num(12, 3, ms_res, Look::PLAIN);
    let total_df = if constant { nf - 1.0 } else { nf };
    let under = Look::PLAIN.under();
    out.text(13, 0, "Total", under);
    out.num(13, 1, total_df, under);
    out.num(13, 2, fit.ss_reg + fit.ss_resid, under);
    for c in 3..6 {
        out.text(13, c, "", under);
    }
    // Coefficients.
    let lvl = level_text(o.confidence);
    let names = ["", "Coefficients", "Standard Error", "t Stat", "P-value", "Lower 95%", "Upper 95%"];
    for (j, h) in names.iter().enumerate() {
        out.text(15, j as u32, h, head());
    }
    out.text(15, 7, &format!("Lower {lvl}"), head());
    out.text(15, 8, &format!("Upper {lvl}"), head());
    let t95 = dist::t_inv_2t(0.05, df_res);
    let tlvl = dist::t_inv_2t(1.0 - o.confidence / 100.0, df_res);
    let coef_row = |out: &mut Out, r: u32, name: &str, c: f64, se: f64, last: bool| {
        let look = if last { Look::PLAIN.under() } else { Look::PLAIN };
        out.text(r, 0, name, look);
        out.num(r, 1, c, look);
        out.num(r, 2, se, look);
        let t = c / se;
        out.num(r, 3, t, look);
        out.num(r, 4, 2.0 * dist::t_upper(t.abs(), df_res), look);
        out.num(r, 5, c - t95 * se, look);
        out.num(r, 6, c + t95 * se, look);
        out.num(r, 7, c - tlvl * se, look);
        out.num(r, 8, c + tlvl * se, look);
    };
    if constant {
        coef_row(&mut out, 16, "Intercept", fit.intercept, fit.se_intercept.unwrap_or(f64::NAN), false);
    } else {
        out.text(16, 0, "Intercept", Look::PLAIN);
        out.num(16, 1, 0.0, Look::PLAIN);
        for c in 2..9 {
            out.put(16, c, V::Error(CellError::NA), Look::PLAIN);
        }
    }
    for (j, x) in xs.iter().enumerate() {
        let c = fit.coef.get(j).copied().unwrap_or(0.0);
        let se = fit.se.get(j).copied().unwrap_or(f64::NAN);
        coef_row(&mut out, 17 + j as u32, &x.label, c, se, j + 1 == k);
    }
    // Residuals.
    let predicted: Vec<f64> = rows.iter().map(|r| fit.intercept + fit.coef.iter().zip(r).map(|(c, v)| c * v).sum::<f64>()).collect();
    let resid: Vec<f64> = yv.iter().zip(&predicted).map(|(a, p)| a - p).collect();
    let want_resid = o.residuals || o.standardized_residuals || o.residual_plots || o.line_fit_plots;
    let base = 17 + k as u32 + 3;
    let data0 = base + 3;
    if want_resid {
        out.text(base, 0, "RESIDUAL OUTPUT", Look::PLAIN);
        let mut cols = vec!["Observation".to_string(), format!("Predicted {}", y.label), "Residuals".to_string()];
        if o.standardized_residuals {
            cols.push("Standard Residuals".to_string());
        }
        for (j, h) in cols.iter().enumerate() {
            out.text(base + 2, j as u32, h, head());
        }
        let sd = (fit.ss_resid / (nf - 1.0)).sqrt();
        for i in 0..n {
            let r = data0 + i as u32;
            let look = if i + 1 == n { Look::PLAIN.under() } else { Look::PLAIN };
            out.num(r, 0, (i + 1) as f64, look);
            out.num(r, 1, predicted.get(i).copied().unwrap_or(f64::NAN), look);
            out.num(r, 2, resid.get(i).copied().unwrap_or(f64::NAN), look);
            if o.standardized_residuals {
                out.num(r, 3, resid.get(i).copied().unwrap_or(f64::NAN) / sd, look);
            }
        }
    }
    if o.normal_probability_plots {
        // One blank column after the residual table.
        let pc = if want_resid { 4 + u32::from(o.standardized_residuals) } else { 0 };
        out.text(base, pc, "PROBABILITY OUTPUT", Look::PLAIN);
        out.text(base + 2, pc, "Percentile", head());
        out.text(base + 2, pc + 1, &y.label, head());
        let mut sorted = yv.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        for (i, v) in sorted.iter().enumerate() {
            let look = if i + 1 == n { Look::PLAIN.under() } else { Look::PLAIN };
            out.num(data0 + i as u32, pc, 100.0 * (i as f64 + 0.5) / nf, look);
            out.num(data0 + i as u32, pc + 1, *v, look);
        }
    }
    // Plots, stacked to the right of the summary.
    let mut chart_row = 0u32;
    let chart_col = 10u32;
    let mut add = |out: &mut Out, c: Chart| {
        out.charts.push(Chart { row: chart_row, col: chart_col, ..c });
        chart_row += 12;
    };
    let rows_n = n as u32;
    for (j, x) in xs.iter().enumerate() {
        let xref = refs.x.get(j).cloned().map(Area::Ref);
        if o.residual_plots {
            add(
                &mut out,
                Chart {
                    kind: ChartKind::Scatter,
                    title: format!("{} Residual Plot", x.label),
                    x_title: Some(x.label.clone()),
                    y_title: Some("Residuals".into()),
                    series: vec![ChartSeries {
                        name: Some("Residuals".into()),
                        x: xref.clone(),
                        y: Area::Out { row: data0, col: 2, rows: rows_n, cols: 1 },
                        line: false,
                    }],
                    row: 0,
                    col: 0,
                    width: 360.0,
                    height: 200.0,
                    legend: false,
                },
            );
        }
        if o.line_fit_plots {
            add(
                &mut out,
                Chart {
                    kind: ChartKind::Scatter,
                    title: format!("{} Line Fit Plot", x.label),
                    x_title: Some(x.label.clone()),
                    y_title: Some(y.label.clone()),
                    series: vec![
                        ChartSeries { name: Some(y.label.clone()), x: xref.clone(), y: Area::Ref(refs.y.clone()), line: false },
                        ChartSeries {
                            name: Some(format!("Predicted {}", y.label)),
                            x: xref,
                            y: Area::Out { row: data0, col: 1, rows: rows_n, cols: 1 },
                            line: false,
                        },
                    ],
                    row: 0,
                    col: 0,
                    width: 360.0,
                    height: 200.0,
                    legend: true,
                },
            );
        }
    }
    if o.normal_probability_plots {
        // One blank column after the residual table.
        let pc = if want_resid { 4 + u32::from(o.standardized_residuals) } else { 0 };
        add(
            &mut out,
            Chart {
                kind: ChartKind::Scatter,
                title: "Normal Probability Plot".into(),
                x_title: Some("Sample Percentile".into()),
                y_title: Some(y.label.clone()),
                series: vec![ChartSeries {
                    name: Some(y.label.clone()),
                    x: Some(Area::Out { row: data0, col: pc, rows: rows_n, cols: 1 }),
                    y: Area::Out { row: data0, col: pc + 1, rows: rows_n, cols: 1 },
                    line: false,
                }],
                row: 0,
                col: 0,
                width: 360.0,
                height: 200.0,
                legend: false,
            },
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(out: &Out, r: u32, c: u32) -> V {
        out.cells.iter().find(|x| x.0 == r && x.1 == c).map(|x| x.2.clone()).unwrap_or_else(|| panic!("nothing at {r},{c}"))
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

    /// NIST StRD "Norris" (public domain): certified values for a simple linear regression.
    const NORRIS_Y: [f64; 36] = [
        0.1, 338.8, 118.1, 888.0, 9.2, 228.1, 668.5, 998.5, 449.1, 778.9, 559.2, 0.3, 0.1, 778.1, 668.8, 339.3, 448.9, 10.8, 557.7, 228.3, 998.0,
        888.8, 119.6, 0.3, 0.6, 557.6, 339.3, 888.0, 998.5, 778.9, 10.2, 117.6, 228.9, 668.4, 449.2, 0.2,
    ];
    const NORRIS_X: [f64; 36] = [
        0.2, 337.4, 118.2, 884.6, 10.1, 226.5, 666.3, 996.3, 448.6, 777.0, 558.2, 0.4, 0.6, 775.5, 666.9, 338.0, 447.5, 11.6, 556.0, 228.1, 995.8,
        887.6, 120.2, 0.3, 0.3, 556.8, 339.1, 887.2, 999.0, 779.0, 11.1, 118.3, 229.2, 669.1, 448.9, 0.5,
    ];

    #[test]
    fn matches_nist_norris() {
        let out = regression(&s("y", &NORRIS_Y), &[s("x", &NORRIS_X)], &RegressionOptions::default(), &InputRefs::default()).unwrap();
        close(at(&out, 16, 1), -0.262_323_073_774_029, 1e-9);
        close(at(&out, 16, 2), 0.232_818_234_301_152, 1e-9);
        close(at(&out, 17, 1), 1.002_116_818_020_45, 1e-12);
        close(at(&out, 17, 2), 0.429_796_848_199_937e-3, 1e-9);
        close(at(&out, 6, 1), 0.884_796_396_144_373, 1e-9);
        close(at(&out, 4, 1), 0.999_993_745_883_712, 1e-12);
        close(at(&out, 11, 2), 4_255_954.132_323_37, 1e-12);
        close(at(&out, 12, 2), 26.617_398_529_422_4, 1e-9);
        close(at(&out, 11, 4), 5_436_385.540_797_85, 1e-9);
        close(at(&out, 7, 1), 36.0, 0.0);
        assert_eq!(at(&out, 15, 7), V::text("Lower 95.0%"));
    }

    #[test]
    fn residuals_probability_and_plots() {
        let y = s("Sales", &[3.0, 5.0, 7.0, 10.0]);
        let x = s("Ads", &[1.0, 2.0, 3.0, 4.0]);
        let o = RegressionOptions {
            residuals: true,
            standardized_residuals: true,
            residual_plots: true,
            line_fit_plots: true,
            normal_probability_plots: true,
            confidence: 90.0,
            ..RegressionOptions::default()
        };
        let refs = InputRefs { y: "Sheet1!$B$2:$B$5".into(), x: vec!["Sheet1!$A$2:$A$5".into()] };
        let out = regression(&y, std::slice::from_ref(&x), &o, &refs).unwrap();
        assert_eq!(at(&out, 15, 7), V::text("Lower 90.0%"));
        assert_eq!(at(&out, 21, 0), V::text("RESIDUAL OUTPUT"));
        assert_eq!(at(&out, 23, 1), V::text("Predicted Sales"));
        // Slope 2.3, intercept 0.5: first prediction 2.8, residual 0.2.
        close(at(&out, 24, 1), 2.8, 1e-12);
        close(at(&out, 24, 2), 0.2, 1e-12);
        assert_eq!(at(&out, 21, 5), V::text("PROBABILITY OUTPUT"));
        close(at(&out, 24, 5), 12.5, 1e-12);
        assert_eq!(out.charts.len(), 3);
        assert_eq!(out.charts[1].series[0].y, Area::Ref("Sheet1!$B$2:$B$5".into()));
        // Through the origin: the intercept row is 0 and #N/A.
        let z = regression(&y, &[x], &RegressionOptions { constant_is_zero: true, ..RegressionOptions::default() }, &refs).unwrap();
        assert_eq!(at(&z, 16, 1), V::Num(0.0));
        assert_eq!(at(&z, 16, 2), V::Error(CellError::NA));
        close(at(&z, 13, 1), 4.0, 0.0);
    }

    #[test]
    fn refuses_bad_inputs() {
        let y = s("y", &[1.0, 2.0]);
        assert!(regression(&y, &[], &RegressionOptions::default(), &InputRefs::default()).is_err());
        assert!(regression(&y, &[s("x", &[1.0])], &RegressionOptions::default(), &InputRefs::default()).is_err());
        assert!(regression(&y, &[s("x", &[1.0, 2.0])], &RegressionOptions::default(), &InputRefs::default()).is_err(), "too few observations");
        let blank = Series::new("x", vec![Some(1.0), None]);
        assert!(regression(&y, &[blank], &RegressionOptions::default(), &InputRefs::default()).is_err());
    }
}
