//! A dense two-phase simplex for Solver-sized linear programs (at most a few hundred variables
//! and constraints), with variable bounds, free variables, row scaling and Bland's rule against
//! cycling. At the optimum it also reports the sensitivity numbers of Excel's Sensitivity
//! Report: shadow prices, reduced costs and the allowable ranges of objective coefficients and
//! right-hand sides.

use super::{BIG, Rel};
use crate::linalg::Matrix;

/// Minimise `c·x` subject to `a·x REL b` and `lower ≤ x ≤ upper` (bounds may be infinite).
#[derive(Clone, Debug, PartialEq)]
pub struct Lp {
    pub c: Vec<f64>,
    pub a: Vec<Vec<f64>>,
    pub rel: Vec<Rel>,
    pub b: Vec<f64>,
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LpStatus {
    Optimal,
    Infeasible,
    Unbounded,
    IterationLimit,
    TooLarge,
}

/// The result, with sensitivity in minimisation terms (∂ objective / ∂ …).
#[derive(Clone, Debug, PartialEq)]
pub struct LpSolution {
    pub status: LpStatus,
    pub x: Vec<f64>,
    pub objective: f64,
    pub pivots: u64,
    /// ∂ objective / ∂ b_i per constraint.
    pub duals: Vec<f64>,
    /// ∂ objective / ∂ x_j moving x_j off its bound (0 for basic variables).
    pub reduced: Vec<f64>,
    pub coef_increase: Vec<f64>,
    pub coef_decrease: Vec<f64>,
    pub rhs_increase: Vec<f64>,
    pub rhs_decrease: Vec<f64>,
}

impl LpSolution {
    fn failed(status: LpStatus, n: usize, pivots: u64) -> LpSolution {
        LpSolution {
            status,
            x: vec![0.0; n],
            objective: f64::NAN,
            pivots,
            duals: vec![],
            reduced: vec![],
            coef_increase: vec![],
            coef_decrease: vec![],
            rhs_increase: vec![],
            rhs_decrease: vec![],
        }
    }
}

/// How an original variable maps onto the non-negative tableau columns.
#[derive(Clone, Copy, Debug)]
enum Map {
    /// x = lower + y.
    Shift { col: usize, lower: f64 },
    /// x = upper − y.
    Flip { col: usize, upper: f64 },
    /// x = y⁺ − y⁻.
    Free { pos: usize, neg: usize },
}

/// Largest tableau (rows × columns) accepted.
const MAX_CELLS: usize = 6_000_000;
const EPS: f64 = 1e-9;

struct Tableau {
    t: Matrix,
    basis: Vec<usize>,
    rows: usize,
    cols: usize,
    /// Columns that may enter the basis (artificials are barred in phase 2).
    allowed: Vec<bool>,
    pivots: u64,
}

impl Tableau {
    fn rhs(&self, r: usize) -> f64 {
        self.t.get(r, self.cols)
    }

    fn pivot(&mut self, r: usize, k: usize) {
        let p = self.t.get(r, k);
        if p == 0.0 {
            return;
        }
        for j in 0..=self.cols {
            self.t.set(r, j, self.t.get(r, j) / p);
        }
        for i in 0..self.rows {
            if i == r {
                continue;
            }
            let f = self.t.get(i, k);
            if f == 0.0 {
                continue;
            }
            for j in 0..=self.cols {
                let v = self.t.get(i, j) - f * self.t.get(r, j);
                self.t.set(i, j, if v.abs() < 1e-14 { 0.0 } else { v });
            }
        }
        if let Some(b) = self.basis.get_mut(r) {
            *b = k;
        }
        self.pivots += 1;
    }

    /// Reduced costs d_k = c_k − Σ c_basis(r) T[r][k] for every column.
    fn reduced(&self, cost: &[f64]) -> Vec<f64> {
        let mut d: Vec<f64> = (0..self.cols).map(|k| cost.get(k).copied().unwrap_or(0.0)).collect();
        for r in 0..self.rows {
            let cb = self.basis.get(r).and_then(|&b| cost.get(b)).copied().unwrap_or(0.0);
            if cb == 0.0 {
                continue;
            }
            for (k, dk) in d.iter_mut().enumerate() {
                *dk -= cb * self.t.get(r, k);
            }
        }
        d
    }

    /// Runs simplex iterations on `cost` (Bland's rule). Ok(true) optimal, Ok(false) unbounded,
    /// Err(()) iteration limit.
    fn optimise(&mut self, cost: &[f64], limit: u64) -> Result<bool, ()> {
        loop {
            if limit > 0 && self.pivots >= limit {
                return Err(());
            }
            let d = self.reduced(cost);
            let Some(k) = (0..self.cols).find(|&k| self.allowed.get(k).copied().unwrap_or(false) && d.get(k).copied().unwrap_or(0.0) < -EPS) else {
                return Ok(true);
            };
            let mut best: Option<(usize, f64)> = None;
            for r in 0..self.rows {
                let a = self.t.get(r, k);
                if a > EPS {
                    let ratio = self.rhs(r) / a;
                    best = match best {
                        None => Some((r, ratio)),
                        Some((br, bv)) => {
                            if ratio < bv - 1e-12 || ((ratio - bv).abs() <= 1e-12 && self.basis.get(r) < self.basis.get(br)) {
                                Some((r, ratio))
                            } else {
                                Some((br, bv))
                            }
                        }
                    };
                }
            }
            match best {
                Some((r, _)) => self.pivot(r, k),
                None => return Ok(false),
            }
        }
    }
}

/// Solves `lp`, stopping after `max_pivots` pivots (0 = no limit).
pub fn solve(lp: &Lp, max_pivots: u64) -> LpSolution {
    let n = lp.c.len();
    let m = lp.a.len();
    // Columns for the original variables.
    let mut maps = Vec::with_capacity(n);
    let mut ny = 0usize;
    for j in 0..n {
        let (l, u) = (lp.lower.get(j).copied().unwrap_or(0.0), lp.upper.get(j).copied().unwrap_or(f64::INFINITY));
        if l > u + EPS {
            return LpSolution::failed(LpStatus::Infeasible, n, 0);
        }
        maps.push(if l.is_finite() {
            ny += 1;
            Map::Shift { col: ny - 1, lower: l }
        } else if u.is_finite() {
            ny += 1;
            Map::Flip { col: ny - 1, upper: u }
        } else {
            ny += 2;
            Map::Free { pos: ny - 2, neg: ny - 1 }
        });
    }
    // Rows in y: the constraints, then y ≤ upper − lower for doubly bounded variables.
    let mut rows_a: Vec<Vec<f64>> = Vec::with_capacity(m + n);
    let mut rows_b: Vec<f64> = Vec::with_capacity(m + n);
    let mut rows_rel: Vec<Rel> = Vec::with_capacity(m + n);
    for i in 0..m {
        let a = lp.a.get(i).map(Vec::as_slice).unwrap_or(&[]);
        let mut row = vec![0.0; ny];
        let mut b = lp.b.get(i).copied().unwrap_or(0.0);
        for (j, mp) in maps.iter().enumerate() {
            let aij = a.get(j).copied().unwrap_or(0.0);
            if aij == 0.0 {
                continue;
            }
            match *mp {
                Map::Shift { col, lower } => {
                    row[col] += aij;
                    b -= aij * lower;
                }
                Map::Flip { col, upper } => {
                    row[col] -= aij;
                    b -= aij * upper;
                }
                Map::Free { pos, neg } => {
                    row[pos] += aij;
                    row[neg] -= aij;
                }
            }
        }
        rows_a.push(row);
        rows_b.push(b);
        rows_rel.push(lp.rel.get(i).copied().unwrap_or(Rel::Le));
    }
    for (j, mp) in maps.iter().enumerate() {
        if let Map::Shift { col, lower } = *mp {
            let u = lp.upper.get(j).copied().unwrap_or(f64::INFINITY);
            if u.is_finite() {
                let mut row = vec![0.0; ny];
                row[col] = 1.0;
                rows_a.push(row);
                rows_b.push(u - lower);
                rows_rel.push(Rel::Le);
            }
        }
    }
    let rows = rows_a.len();
    // Scale rows by their largest coefficient; flip rows with negative right sides.
    let mut scale = vec![1.0; rows];
    let mut flip = vec![1.0; rows];
    for i in 0..rows {
        let s = rows_a[i].iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
        if s > 0.0 && s.is_finite() {
            scale[i] = s;
            rows_a[i].iter_mut().for_each(|v| *v /= s);
            rows_b[i] /= s;
        }
        if rows_b[i] < 0.0 {
            flip[i] = -1.0;
            rows_a[i].iter_mut().for_each(|v| *v = -*v);
            rows_b[i] = -rows_b[i];
            rows_rel[i] = match rows_rel[i] {
                Rel::Le => Rel::Ge,
                Rel::Ge => Rel::Le,
                Rel::Eq => Rel::Eq,
            };
        }
    }
    // Slack / surplus / artificial columns.
    let n_slack = rows_rel.iter().filter(|r| **r != Rel::Eq).count();
    let n_art = rows_rel.iter().filter(|r| **r != Rel::Le).count();
    let cols = ny + n_slack + n_art;
    if rows.saturating_mul(cols + 1) > MAX_CELLS {
        return LpSolution::failed(LpStatus::TooLarge, n, 0);
    }
    let Ok(mut t) = Matrix::zeros(rows, cols + 1) else { return LpSolution::failed(LpStatus::TooLarge, n, 0) };
    let mut basis = vec![0usize; rows];
    let mut unit = vec![0usize; rows];
    let mut artificial = vec![false; cols];
    let (mut next_slack, mut next_art) = (ny, ny + n_slack);
    for i in 0..rows {
        for (j, v) in rows_a[i].iter().enumerate() {
            t.set(i, j, *v);
        }
        t.set(i, cols, rows_b[i]);
        match rows_rel[i] {
            Rel::Le => {
                t.set(i, next_slack, 1.0);
                basis[i] = next_slack;
                unit[i] = next_slack;
                next_slack += 1;
            }
            Rel::Ge => {
                t.set(i, next_slack, -1.0);
                next_slack += 1;
                t.set(i, next_art, 1.0);
                artificial[next_art] = true;
                basis[i] = next_art;
                unit[i] = next_art;
                next_art += 1;
            }
            Rel::Eq => {
                t.set(i, next_art, 1.0);
                artificial[next_art] = true;
                basis[i] = next_art;
                unit[i] = next_art;
                next_art += 1;
            }
        }
    }
    let mut tab = Tableau { t, basis, rows, cols, allowed: vec![true; cols], pivots: 0 };
    // Phase 1: drive the artificials to zero.
    if n_art > 0 {
        let cost1: Vec<f64> = (0..cols).map(|k| if artificial[k] { 1.0 } else { 0.0 }).collect();
        if tab.optimise(&cost1, max_pivots).is_err() {
            return LpSolution::failed(LpStatus::IterationLimit, n, tab.pivots);
        }
        let infeas: f64 = (0..rows).filter(|&r| artificial.get(tab.basis[r]).copied().unwrap_or(false)).map(|r| tab.rhs(r)).sum();
        if infeas > 1e-7 {
            return LpSolution::failed(LpStatus::Infeasible, n, tab.pivots);
        }
        // Pivot artificials at zero out of the basis where possible.
        for r in 0..rows {
            if artificial.get(tab.basis[r]).copied().unwrap_or(false)
                && let Some(k) = (0..cols).find(|&k| !artificial[k] && tab.t.get(r, k).abs() > 1e-7)
            {
                tab.pivot(r, k);
            }
        }
        for (k, a) in artificial.iter().enumerate() {
            if *a {
                tab.allowed[k] = false;
            }
        }
    }
    // Phase 2.
    let mut cost2 = vec![0.0; cols];
    for (j, mp) in maps.iter().enumerate() {
        let cj = lp.c.get(j).copied().unwrap_or(0.0);
        match *mp {
            Map::Shift { col, .. } => cost2[col] = cj,
            Map::Flip { col, .. } => cost2[col] = -cj,
            Map::Free { pos, neg } => {
                cost2[pos] = cj;
                cost2[neg] = -cj;
            }
        }
    }
    match tab.optimise(&cost2, max_pivots) {
        Err(()) => return LpSolution::failed(LpStatus::IterationLimit, n, tab.pivots),
        Ok(false) => return LpSolution::failed(LpStatus::Unbounded, n, tab.pivots),
        Ok(true) => {}
    }
    // The solution.
    let mut y = vec![0.0; cols];
    for r in 0..rows {
        if let Some(v) = y.get_mut(tab.basis[r]) {
            *v = tab.rhs(r);
        }
    }
    let x: Vec<f64> = maps
        .iter()
        .map(|mp| match *mp {
            Map::Shift { col, lower } => lower + y[col],
            Map::Flip { col, upper } => upper - y[col],
            Map::Free { pos, neg } => y[pos] - y[neg],
        })
        .collect();
    let objective: f64 = lp.c.iter().zip(&x).map(|(c, v)| c * v).sum();
    // Sensitivity.
    let d = tab.reduced(&cost2);
    let duals: Vec<f64> = (0..m).map(|i| -d.get(unit[i]).copied().unwrap_or(0.0) * flip[i] / scale[i]).collect();
    let reduced: Vec<f64> = (0..n)
        .map(|j| {
            let cj = lp.c.get(j).copied().unwrap_or(0.0);
            let r = cj - (0..m).map(|i| duals[i] * lp.a.get(i).and_then(|a| a.get(j)).copied().unwrap_or(0.0)).sum::<f64>();
            if r.abs() < 1e-10 { 0.0 } else { r }
        })
        .collect();
    let row_of = |col: usize| (0..rows).find(|&r| tab.basis[r] == col);
    let mut coef_increase = vec![BIG; n];
    let mut coef_decrease = vec![BIG; n];
    for (j, mp) in maps.iter().enumerate() {
        let (col, sign) = match *mp {
            Map::Shift { col, .. } => (col, 1.0),
            Map::Flip { col, .. } => (col, -1.0),
            Map::Free { pos, .. } => (pos, 1.0),
        };
        // Allowable change of this column's cost (increase, decrease) in y terms.
        let (inc, dec) = match row_of(col) {
            None => (BIG, d.get(col).copied().unwrap_or(0.0).max(0.0)),
            Some(r) => {
                let (mut inc, mut dec) = (BIG, BIG);
                for q in 0..cols {
                    if !tab.allowed[q] || q == col || row_of(q).is_some() {
                        continue;
                    }
                    let alpha = tab.t.get(r, q);
                    let dq = d.get(q).copied().unwrap_or(0.0).max(0.0);
                    if alpha > EPS {
                        inc = inc.min(dq / alpha);
                    } else if alpha < -EPS {
                        dec = dec.min(-dq / alpha);
                    }
                }
                (inc, dec)
            }
        };
        if sign > 0.0 {
            coef_increase[j] = inc;
            coef_decrease[j] = dec;
        } else {
            coef_increase[j] = dec;
            coef_decrease[j] = inc;
        }
    }
    let mut rhs_increase = vec![BIG; m];
    let mut rhs_decrease = vec![BIG; m];
    for i in 0..m {
        let u = unit[i];
        let (mut up, mut down) = (BIG, BIG);
        for r in 0..rows {
            let a = tab.t.get(r, u);
            let beta = tab.rhs(r);
            if a > EPS {
                down = down.min(beta / a);
            } else if a < -EPS {
                up = up.min(-beta / a);
            }
        }
        // Back to the original row's units and direction.
        let (up, down) = (if up >= BIG { BIG } else { up * scale[i] }, if down >= BIG { BIG } else { down * scale[i] });
        if flip[i] > 0.0 {
            rhs_increase[i] = up;
            rhs_decrease[i] = down;
        } else {
            rhs_increase[i] = down;
            rhs_decrease[i] = up;
        }
    }
    LpSolution {
        status: LpStatus::Optimal,
        x,
        objective,
        pivots: tab.pivots,
        duals,
        reduced,
        coef_increase,
        coef_decrease,
        rhs_increase,
        rhs_decrease,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-7 * b.abs().max(1.0), "{a} vs {b}");
    }

    /// The classic "Wyndor Glass" model: max 3x + 5y, x ≤ 4, 2y ≤ 12, 3x + 2y ≤ 18 (as a min).
    fn wyndor() -> Lp {
        Lp {
            c: vec![-3.0, -5.0],
            a: vec![vec![1.0, 0.0], vec![0.0, 2.0], vec![3.0, 2.0]],
            rel: vec![Rel::Le; 3],
            b: vec![4.0, 12.0, 18.0],
            lower: vec![0.0, 0.0],
            upper: vec![f64::INFINITY; 2],
        }
    }

    #[test]
    fn solves_with_sensitivity() {
        let s = solve(&wyndor(), 0);
        assert_eq!(s.status, LpStatus::Optimal);
        close(s.x[0], 2.0);
        close(s.x[1], 6.0);
        close(s.objective, -36.0);
        // Shadow prices (max form: 0, 1.5, 1) are the negated min duals.
        close(-s.duals[0], 0.0);
        close(-s.duals[1], 1.5);
        close(-s.duals[2], 1.0);
        // Right-hand side ranges: plant 1 can fall by 2, plants 2 and 3 move within known ranges.
        close(s.rhs_decrease[0], 2.0);
        assert!(s.rhs_increase[0] >= BIG);
        close(s.rhs_increase[1], 6.0);
        close(s.rhs_decrease[1], 6.0);
        close(s.rhs_increase[2], 6.0);
        close(s.rhs_decrease[2], 6.0);
        // Objective coefficient ranges (max form): x's 3 may rise by 4.5 or fall by 3; y's 5 may
        // rise without limit or fall by 3. In min form increase and decrease swap.
        close(s.coef_decrease[0], 4.5);
        close(s.coef_increase[0], 3.0);
        assert!(s.coef_decrease[1] >= BIG);
        close(s.coef_increase[1], 3.0);
        assert!(s.reduced.iter().all(|r| *r == 0.0));
    }

    #[test]
    fn bounds_equalities_free_variables_and_failures() {
        // min x + y with x + y ≥ 2, x − y = 0.5, −10 ≤ x ≤ 3, y free: x = 1.25, y = 0.75.
        let lp = Lp {
            c: vec![1.0, 1.0],
            a: vec![vec![1.0, 1.0], vec![1.0, -1.0]],
            rel: vec![Rel::Ge, Rel::Eq],
            b: vec![2.0, 0.5],
            lower: vec![-10.0, f64::NEG_INFINITY],
            upper: vec![3.0, f64::INFINITY],
        };
        let s = solve(&lp, 0);
        assert_eq!(s.status, LpStatus::Optimal);
        close(s.x[0], 1.25);
        close(s.x[1], 0.75);
        // With x ≤ 1 there is no solution.
        let tight = Lp { upper: vec![1.0, f64::INFINITY], ..lp };
        assert_eq!(solve(&tight, 0).status, LpStatus::Infeasible);
        // Upper bound only: x ≤ 3, maximise x (min −x).
        let s = solve(&Lp { c: vec![-1.0], a: vec![], rel: vec![], b: vec![], lower: vec![f64::NEG_INFINITY], upper: vec![3.0] }, 0);
        close(s.x[0], 3.0);
        // Reduced cost of a variable held at its bound.
        let s = solve(
            &Lp { c: vec![2.0, 1.0], a: vec![vec![1.0, 1.0]], rel: vec![Rel::Ge], b: vec![1.0], lower: vec![0.0; 2], upper: vec![f64::INFINITY; 2] },
            0,
        );
        close(s.x[1], 1.0);
        close(s.reduced[0], 1.0);
        close(s.duals[0], 1.0);
        let infeasible = Lp {
            c: vec![1.0],
            a: vec![vec![1.0], vec![1.0]],
            rel: vec![Rel::Ge, Rel::Le],
            b: vec![5.0, 3.0],
            lower: vec![0.0],
            upper: vec![f64::INFINITY],
        };
        assert_eq!(solve(&infeasible, 0).status, LpStatus::Infeasible);
        let unbounded = Lp { c: vec![-1.0], a: vec![vec![1.0]], rel: vec![Rel::Ge], b: vec![1.0], lower: vec![0.0], upper: vec![f64::INFINITY] };
        assert_eq!(solve(&unbounded, 0).status, LpStatus::Unbounded);
        assert_eq!(solve(&Lp { lower: vec![2.0], upper: vec![1.0], ..unbounded.clone() }, 0).status, LpStatus::Infeasible);
        assert_eq!(solve(&wyndor(), 1).status, LpStatus::IterationLimit);
    }

    #[test]
    fn degenerate_problem_terminates() {
        // A classic cycling example (Beale) solves under Bland's rule.
        let lp = Lp {
            c: vec![-0.75, 150.0, -0.02, 6.0],
            a: vec![vec![0.25, -60.0, -0.04, 9.0], vec![0.5, -90.0, -0.02, 3.0], vec![0.0, 0.0, 1.0, 0.0]],
            rel: vec![Rel::Le; 3],
            b: vec![0.0, 0.0, 1.0],
            lower: vec![0.0; 4],
            upper: vec![f64::INFINITY; 4],
        };
        let s = solve(&lp, 10_000);
        assert_eq!(s.status, LpStatus::Optimal);
        close(s.objective, -0.05);
    }
}
