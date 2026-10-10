//! GRG Nonlinear: a local optimiser for smooth nonlinear models.
//!
//! Each local solve is an augmented Lagrangian method: the constraints are folded into a penalty
//! with multipliers, and the resulting bound-constrained problem is minimised by a projected
//! quasi-Newton (BFGS) method with finite-difference derivatives (forward or central). Like
//! Excel's GRG it stops when every constraint holds and the optimality conditions are met
//! (status 0), when the objective has changed by less than Convergence over the last five
//! iterations (1), or when no further progress is possible (2). Multistart repeats the local
//! solve from random points within the bounds; integer constraints use branch and bound with a
//! local solve per node. At the end the reduced gradients and Lagrange multipliers are reported.

use super::{Engine, Eval, NlpSensitivity, Outcome, Problem, Rel, Sense, Spec, Status, Step, violation};
use crate::rng::Rng;

const MAX_OUTER: u32 = 60;
const MAX_INNER: u32 = 200;

/// Constraint values split into equalities (h = 0) and inequalities (g ≤ 0).
struct Split {
    h: Vec<f64>,
    g: Vec<f64>,
}

/// One local solve from a starting point within `lo..=hi`.
struct Local {
    x: Vec<f64>,
    lo: Vec<f64>,
    hi: Vec<f64>,
    lam: Vec<f64>,
    mu: Vec<f64>,
    rho: f64,
    /// Current augmented Lagrangian value and gradient (valid when `have`).
    fx: f64,
    grad: Vec<f64>,
    have: bool,
    /// Inverse Hessian approximation over the variables.
    hinv: Vec<Vec<f64>>,
    inner: u32,
    outer: u32,
    /// Objective after each outer iteration (for the five-iteration convergence test).
    history: Vec<f64>,
    prev_viol: f64,
    /// Inner minimisation can't improve.
    stuck: bool,
    status: Option<Status>,
    iterations: u64,
}

pub struct Grg {
    spec: Spec,
    sign: f64,
    scale: f64,
    rng: Rng,
    starts: Vec<Vec<f64>>,
    nodes: Vec<(Vec<f64>, Vec<f64>)>,
    cur: Option<Local>,
    best: Option<(f64, Vec<f64>, Status)>,
    within_tolerance: bool,
    feasible_found: u64,
    subproblems: u64,
    iterations: u64,
    done: Option<Box<Outcome>>,
    started: bool,
}

impl Grg {
    pub fn new(spec: Spec) -> Grg {
        let sign = match spec.sense {
            Sense::Max => -1.0,
            Sense::Min => 1.0,
            Sense::Target(_) => 0.0,
        };
        let seed = if spec.options.seed == 0 { 0x6A09_E667 } else { spec.options.seed };
        Grg {
            sign,
            scale: 1.0,
            rng: Rng::new(seed),
            starts: vec![],
            nodes: vec![],
            cur: None,
            best: None,
            within_tolerance: false,
            feasible_found: 0,
            subproblems: 0,
            iterations: 0,
            done: None,
            started: false,
            spec,
        }
    }

    fn n(&self) -> usize {
        self.spec.n()
    }

    fn split(&self, e: &Eval) -> Split {
        let mut h = Vec::new();
        let mut g = Vec::new();
        for (r, c) in self.spec.rels.iter().zip(&e.cons) {
            match r {
                Rel::Le => g.push(*c),
                Rel::Ge => g.push(-*c),
                Rel::Eq => h.push(*c),
            }
        }
        if let Sense::Target(v) = self.spec.sense {
            h.push((e.objective - v) * self.scale);
        }
        Split { h, g }
    }

    fn obj(&self, e: &Eval) -> f64 {
        self.sign * e.objective * self.scale
    }

    /// Augmented Lagrangian value.
    fn la(&self, l: &Local, e: &Eval) -> f64 {
        let s = self.split(e);
        let mut v = self.obj(e);
        for (i, h) in s.h.iter().enumerate() {
            let lam = l.lam.get(i).copied().unwrap_or(0.0);
            v += lam * h + 0.5 * l.rho * h * h;
        }
        for (i, g) in s.g.iter().enumerate() {
            let mu = l.mu.get(i).copied().unwrap_or(0.0);
            let t = (mu + l.rho * g).max(0.0);
            v += (t * t - mu * mu) / (2.0 * l.rho);
        }
        v
    }

    /// Finite-difference step for variable `j` at `x`.
    fn step_size(&self, x: f64) -> f64 {
        let base = if self.spec.options.central { 6e-6 } else { 1.5e-8 };
        base * x.abs().max(1.0)
    }

    /// Jacobian rows (objective, then each constraint) by finite differences around `x`.
    fn jacobian(&self, p: &mut dyn Problem, x: &[f64], e0: &Eval, lo: &[f64], hi: &[f64]) -> Option<(Vec<f64>, Vec<Vec<f64>>)> {
        let n = x.len();
        let m = e0.cons.len();
        let central = self.spec.options.central;
        let mut pts = Vec::with_capacity(if central { 2 * n } else { n });
        let mut steps = Vec::with_capacity(n);
        for j in 0..n {
            let h = self.step_size(x[j]);
            // Step inward at a bound.
            let fwd = if x[j] + h <= hi[j] { h } else { -h };
            steps.push(fwd);
            let mut a = x.to_vec();
            a[j] += fwd;
            pts.push(a);
            if central {
                let mut b = x.to_vec();
                b[j] -= fwd;
                b[j] = b[j].clamp(lo[j], hi[j]);
                pts.push(b);
            }
        }
        let evs = p.eval_many(&pts);
        let mut df = vec![0.0; n];
        let mut dc = vec![vec![0.0; n]; m];
        for j in 0..n {
            let h = steps[j];
            if central {
                let a = evs.get(2 * j)?.as_ref()?;
                let b = evs.get(2 * j + 1)?.as_ref()?;
                let span = pts[2 * j][j] - pts[2 * j + 1][j];
                if span == 0.0 {
                    return None;
                }
                df[j] = (a.objective - b.objective) / span;
                for (i, row) in dc.iter_mut().enumerate().take(m) {
                    row[j] = (a.cons.get(i)? - b.cons.get(i)?) / span;
                }
            } else {
                let a = evs.get(j)?.as_ref()?;
                df[j] = (a.objective - e0.objective) / h;
                for (i, row) in dc.iter_mut().enumerate().take(m) {
                    row[j] = (a.cons.get(i)? - e0.cons.get(i)?) / h;
                }
            }
        }
        Some((df, dc))
    }

    /// Gradient of the augmented Lagrangian at the point evaluated as `e` with Jacobian (df, dc).
    fn la_grad(&self, l: &Local, e: &Eval, df: &[f64], dc: &[Vec<f64>]) -> Vec<f64> {
        let n = df.len();
        let mut g: Vec<f64> = df.iter().map(|d| self.sign * self.scale * d).collect();
        let s = self.split(e);
        let (mut hi, mut gi) = (0usize, 0usize);
        for (i, r) in self.spec.rels.iter().enumerate() {
            let row = dc.get(i).map(Vec::as_slice).unwrap_or(&[]);
            match r {
                Rel::Eq => {
                    let w = l.lam.get(hi).copied().unwrap_or(0.0) + l.rho * s.h.get(hi).copied().unwrap_or(0.0);
                    hi += 1;
                    for (j, gj) in g.iter_mut().enumerate() {
                        *gj += w * row.get(j).copied().unwrap_or(0.0);
                    }
                }
                Rel::Le | Rel::Ge => {
                    let sgn = if *r == Rel::Le { 1.0 } else { -1.0 };
                    let w = (l.mu.get(gi).copied().unwrap_or(0.0) + l.rho * s.g.get(gi).copied().unwrap_or(0.0)).max(0.0);
                    gi += 1;
                    for (j, gj) in g.iter_mut().enumerate() {
                        *gj += w * sgn * row.get(j).copied().unwrap_or(0.0);
                    }
                }
            }
        }
        if let Sense::Target(_) = self.spec.sense {
            let w = l.lam.get(hi).copied().unwrap_or(0.0) + l.rho * s.h.get(hi).copied().unwrap_or(0.0);
            for j in 0..n {
                g[j] += w * self.scale * df[j];
            }
        }
        g
    }

    fn new_local(&self, x0: &[f64], lo: Vec<f64>, hi: Vec<f64>, e0: &Eval) -> Local {
        let s = self.split(e0);
        let n = x0.len();
        let x: Vec<f64> = x0.iter().enumerate().map(|(j, v)| v.clamp(lo[j], hi[j])).collect();
        Local {
            x,
            lo,
            hi,
            lam: vec![0.0; s.h.len()],
            mu: vec![0.0; s.g.len()],
            rho: 10.0,
            fx: f64::INFINITY,
            grad: vec![0.0; n],
            have: false,
            hinv: identity(n),
            inner: 0,
            outer: 0,
            history: vec![],
            prev_viol: f64::INFINITY,
            stuck: false,
            status: None,
            iterations: 0,
        }
    }

    /// One unit of work on the local solve: compute the gradient, or take one quasi-Newton step,
    /// or (at the end of an inner minimisation) update the multipliers.
    fn advance(&self, l: &mut Local, p: &mut dyn Problem) {
        let n = l.x.len();
        if !l.have {
            let Some(e) = p.eval(&l.x) else {
                l.status = Some(Status::ErrorValue);
                return;
            };
            let Some((df, dc)) = self.jacobian(p, &l.x, &e, &l.lo, &l.hi) else {
                l.status = Some(Status::ErrorValue);
                return;
            };
            l.fx = self.la(l, &e);
            l.grad = self.la_grad(l, &e, &df, &dc);
            l.have = true;
            return;
        }
        let tol = 1e-9;
        // Variables held at a bound by the gradient are fixed this step.
        let free: Vec<bool> =
            (0..n).map(|j| !((l.x[j] <= l.lo[j] + tol && l.grad[j] > 0.0) || (l.x[j] >= l.hi[j] - tol && l.grad[j] < 0.0))).collect();
        let pg: f64 = (0..n).filter(|&j| free[j]).map(|j| l.grad[j] * l.grad[j]).sum::<f64>().sqrt();
        let mut done_inner = pg <= 1e-8 * (1.0 + l.fx.abs()) || l.inner >= MAX_INNER || l.stuck;
        if !done_inner {
            let mut d: Vec<f64> =
                (0..n).map(|j| if free[j] { -(0..n).filter(|&k| free[k]).map(|k| l.hinv[j][k] * l.grad[k]).sum::<f64>() } else { 0.0 }).collect();
            // Not a descent direction: fall back to steepest descent.
            if d.iter().zip(&l.grad).map(|(a, b)| a * b).sum::<f64>() >= 0.0 {
                l.hinv = identity(n);
                d = (0..n).map(|j| if free[j] { -l.grad[j] } else { 0.0 }).collect();
            }
            // Projected backtracking line search.
            let mut alpha = 1.0;
            let mut accepted: Option<(Vec<f64>, Eval, f64)> = None;
            for _ in 0..40 {
                let xn: Vec<f64> = (0..n).map(|j| (l.x[j] + alpha * d[j]).clamp(l.lo[j], l.hi[j])).collect();
                if xn == l.x {
                    break;
                }
                if let Some(e) = p.eval(&xn) {
                    let f = self.la(l, &e);
                    let dec: f64 = l.grad.iter().zip(xn.iter().zip(&l.x)).map(|(g, (a, b))| g * (a - b)).sum();
                    if f.is_finite() && f <= l.fx + 1e-4 * dec {
                        accepted = Some((xn, e, f));
                        break;
                    }
                }
                alpha *= 0.5;
            }
            match accepted {
                None => l.stuck = true,
                Some((xn, e, f)) => {
                    let Some((df, dc)) = self.jacobian(p, &xn, &e, &l.lo, &l.hi) else {
                        l.status = Some(Status::ErrorValue);
                        return;
                    };
                    let gn = self.la_grad(l, &e, &df, &dc);
                    let s: Vec<f64> = xn.iter().zip(&l.x).map(|(a, b)| a - b).collect();
                    let y: Vec<f64> = gn.iter().zip(&l.grad).map(|(a, b)| a - b).collect();
                    bfgs(&mut l.hinv, &s, &y);
                    let rel = (l.fx - f).abs() / (1.0 + l.fx.abs());
                    l.x = xn;
                    l.fx = f;
                    l.grad = gn;
                    l.inner += 1;
                    l.iterations += 1;
                    if rel < 1e-12 {
                        done_inner = true;
                    }
                }
            }
            if !done_inner && !l.stuck {
                return;
            }
        }
        // End of an inner minimisation: check the constraints and update the multipliers.
        let Some(e) = p.eval(&l.x) else {
            l.status = Some(Status::ErrorValue);
            return;
        };
        let s = self.split(&e);
        let viol = s.h.iter().map(|v| v.abs()).chain(s.g.iter().map(|v| v.max(0.0))).fold(0.0f64, f64::max);
        let feasible = violation(&self.spec.rels, &e, self.spec.options.precision) == 0.0
            && match self.spec.sense {
                Sense::Target(v) => (e.objective - v).abs() <= self.spec.options.precision.max(1e-9 * v.abs()),
                _ => true,
            };
        let obj = e.objective;
        l.history.push(obj);
        l.outer += 1;
        let pg: f64 = {
            let free: Vec<bool> =
                (0..n).map(|j| !((l.x[j] <= l.lo[j] + tol && l.grad[j] > 0.0) || (l.x[j] >= l.hi[j] - tol && l.grad[j] < 0.0))).collect();
            (0..n).filter(|&j| free[j]).map(|j| l.grad[j] * l.grad[j]).sum::<f64>().sqrt()
        };
        let conv = self.spec.options.convergence;
        let settled = l.history.len() >= 5 && {
            let k = l.history.len();
            let a = l.history[k - 5];
            let b = l.history[k - 1];
            (a - b).abs() <= conv * (1.0 + b.abs())
        };
        // Complementarity: an inequality with slack must have no multiplier left.
        let comp =
            s.g.iter()
                .enumerate()
                .map(|(i, g)| {
                    let mu = (l.mu.get(i).copied().unwrap_or(0.0) + l.rho * g).max(0.0);
                    (-g).min(mu).abs()
                })
                .fold(0.0f64, f64::max);
        if feasible && pg <= 1e-5 * (1.0 + l.fx.abs()) && comp <= self.spec.options.precision.max(1e-9) && !l.stuck {
            l.status = Some(Status::Optimal);
            return;
        }
        if feasible && settled {
            l.status = Some(Status::Converged);
            return;
        }
        if l.outer >= MAX_OUTER || (l.stuck && viol < 1e-12) {
            l.status = Some(if feasible { Status::CannotImprove } else { Status::Infeasible });
            return;
        }
        // Multiplier updates.
        for (i, h) in s.h.iter().enumerate() {
            if let Some(lam) = l.lam.get_mut(i) {
                *lam += l.rho * h;
            }
        }
        for (i, g) in s.g.iter().enumerate() {
            if let Some(mu) = l.mu.get_mut(i) {
                *mu = (*mu + l.rho * g).max(0.0);
            }
        }
        if viol > 0.25 * l.prev_viol {
            l.rho = (l.rho * 10.0).min(1e10);
        }
        l.prev_viol = viol;
        l.stuck = false;
        l.inner = 0;
        l.have = false;
        l.hinv = identity(n);
    }

    fn requires_bounds(&self) -> bool {
        (self.spec.options.multistart && self.spec.options.require_bounds)
            && self.spec.lower.iter().zip(&self.spec.upper).any(|(l, u)| !l.is_finite() || !u.is_finite())
    }

    fn is_int(&self, j: usize) -> bool {
        self.spec.has_integers() && (self.spec.integer.get(j).copied().unwrap_or(false) || self.spec.alldiff.iter().any(|g| g.contains(&j)))
    }

    /// Bounds for the root, with integer bounds rounded inward and AllDifferent groups in 1..=k.
    fn root_bounds(&self) -> (Vec<f64>, Vec<f64>) {
        let mut lo = self.spec.lower.clone();
        let mut hi = self.spec.upper.clone();
        for g in &self.spec.alldiff {
            for &j in g {
                lo[j] = lo[j].max(1.0);
                hi[j] = hi[j].min(g.len() as f64);
            }
        }
        for j in 0..lo.len() {
            if self.is_int(j) {
                lo[j] = lo[j].ceil();
                hi[j] = hi[j].floor();
            }
        }
        (lo, hi)
    }

    /// Starting points for multistart: the user's point, then random points in the bounds.
    fn make_starts(&mut self) {
        self.starts = vec![self.spec.x0.clone()];
        if self.spec.options.multistart {
            let (lo, hi) = (self.spec.lower.clone(), self.spec.upper.clone());
            let extra = self.spec.options.population.clamp(1, 1000);
            for _ in 0..extra {
                let x: Vec<f64> = (0..self.n())
                    .map(|j| {
                        let (l, u) = (
                            if lo[j].is_finite() { lo[j] } else { self.spec.x0[j] - 100.0 },
                            if hi[j].is_finite() { hi[j] } else { self.spec.x0[j] + 100.0 },
                        );
                        self.rng.uniform_between(l, u)
                    })
                    .collect();
                self.starts.push(x);
            }
        }
    }

    /// Records a finished local solve; branches on integer violations.
    fn local_done(&mut self, l: Local, p: &mut dyn Problem) {
        self.iterations += l.iterations;
        let Some(status) = l.status else { return };
        if !status.feasible() {
            return;
        }
        // Integer feasibility.
        if self.spec.has_integers() {
            let frac = (0..l.x.len())
                .filter(|&j| self.is_int(j))
                .map(|j| (j, l.x[j], (l.x[j] - l.x[j].round()).abs()))
                .filter(|(_, _, f)| *f > 1e-6)
                .max_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((j, v, _)) = frac {
                let (mut lo_d, mut hi_d) = (l.lo.clone(), l.hi.clone());
                hi_d[j] = v.floor();
                let (mut lo_u, hi_u) = (l.lo.clone(), l.hi.clone());
                lo_u[j] = v.ceil();
                lo_d[j] = lo_d[j].min(hi_d[j]);
                self.nodes.push((lo_u, hi_u));
                self.nodes.push((lo_d, hi_d));
                return;
            }
            let x: Vec<f64> = l.x.iter().enumerate().map(|(j, v)| if self.is_int(j) { v.round() } else { *v }).collect();
            // AllDifferent: equal values branch apart by bounds.
            let clash = self
                .spec
                .alldiff
                .iter()
                .find_map(|g| g.iter().enumerate().find_map(|(k, &a)| g.iter().skip(k + 1).find(|&&b| x[a] == x[b]).map(|&b| (a, b))));
            if let Some((a, _)) = clash {
                let v = x[a];
                let (lo, mut hi) = (l.lo.clone(), l.hi.clone());
                hi[a] = v - 1.0;
                let (mut lo2, hi2) = (l.lo.clone(), l.hi.clone());
                lo2[a] = v + 1.0;
                if lo[a] <= hi[a] {
                    self.nodes.push((lo, hi));
                }
                if lo2[a] <= hi2[a] {
                    self.nodes.push((lo2, hi2));
                }
                return;
            }
            let Some(e) = p.eval(&x) else { return };
            if violation(&self.spec.rels, &e, self.spec.options.precision * 10.0) > 0.0 {
                return;
            }
            self.feasible_found += 1;
            self.consider(&e, x, status);
            return;
        }
        if let Some(e) = p.eval(&l.x) {
            self.consider(&e, l.x, status);
        }
    }

    fn consider(&mut self, e: &Eval, x: Vec<f64>, status: Status) {
        let key = match self.spec.sense {
            Sense::Target(v) => (e.objective - v).abs(),
            _ => self.sign * e.objective,
        };
        if let Some((best, _, _)) = &self.best
            && key >= *best - 1e-12 * best.abs().max(1.0)
        {
            return;
        }
        self.best = Some((key, x, status));
    }

    fn finish(&mut self, p: &mut dyn Problem, status: Status) -> Step {
        let (x, st) = match &self.best {
            Some((_, x, s)) => (x.clone(), if status == Status::Optimal { *s } else { status }),
            None => (self.spec.x0.clone(), if status.feasible() { Status::Infeasible } else { status }),
        };
        let st = if self.within_tolerance && st == Status::Optimal { Status::IntegerWithinTolerance } else { st };
        let mut o = Outcome::new(st, x.clone());
        o.iterations = self.iterations;
        o.subproblems = self.subproblems;
        if st.feasible() && !self.spec.has_integers() {
            o.nlp = self.sensitivity(p, &x);
        }
        self.done = Some(Box::new(o.clone()));
        Step::Done(Box::new(o))
    }

    /// Reduced gradients and Lagrange multipliers at `x`, in the user's objective terms.
    fn sensitivity(&self, p: &mut dyn Problem, x: &[f64]) -> Option<NlpSensitivity> {
        let e = p.eval(x)?;
        let (lo, hi) = (self.spec.lower.clone(), self.spec.upper.clone());
        let (df, dc) = self.jacobian(p, x, &e, &lo, &hi)?;
        let n = x.len();
        let m = e.cons.len();
        // Multipliers from least squares on the active constraints: ∇f = Σ λ ∇c.
        let tol = self.spec.options.precision.max(1e-7) * 10.0;
        let active: Vec<usize> = (0..m)
            .filter(|&i| {
                let c = e.cons[i];
                match self.spec.rels[i] {
                    Rel::Eq => true,
                    Rel::Le | Rel::Ge => c.abs() <= tol.max(1e-6 * (1.0 + c.abs())),
                }
            })
            .collect();
        let free: Vec<usize> = (0..n).filter(|&j| x[j] > lo[j] + 1e-9 && x[j] < hi[j] - 1e-9).collect();
        let k = active.len();
        let mut lambda = vec![0.0; m];
        if k > 0 && !free.is_empty() {
            // Normal equations (A Aᵀ) λ = A ∇f over the free variables.
            let a: Vec<Vec<f64>> = active.iter().map(|&i| free.iter().map(|&j| dc[i][j]).collect()).collect();
            let rhs: Vec<f64> = a.iter().map(|row| row.iter().zip(&free).map(|(v, &j)| v * df[j]).sum()).collect();
            let mut aat = vec![vec![0.0; k]; k];
            for r in 0..k {
                for c in 0..k {
                    aat[r][c] = a[r].iter().zip(&a[c]).map(|(p, q)| p * q).sum::<f64>() + if r == c { 1e-12 } else { 0.0 };
                }
            }
            let mat = crate::linalg::Matrix::from_rows(&aat).ok()?;
            if let Ok(sol) = crate::linalg::solve(&mat, &rhs) {
                for (t, &i) in active.iter().enumerate() {
                    lambda[i] = sol.get(t).copied().unwrap_or(0.0);
                }
            }
        }
        // With L = f − λ·c and c = left − right, the envelope theorem gives
        // ∂(optimal objective)/∂(right side) = λ.
        let lagrange: Vec<f64> = lambda.iter().map(|l| clean(*l)).collect();
        let reduced_gradient: Vec<f64> = (0..n)
            .map(|j| {
                let g = df[j] - (0..m).map(|i| lambda[i] * dc[i][j]).sum::<f64>();
                clean(g)
            })
            .collect();
        Some(NlpSensitivity { reduced_gradient, lagrange })
    }
}

fn clean(v: f64) -> f64 {
    if v.abs() < 1e-7 || !v.is_finite() { 0.0 } else { v }
}

fn identity(n: usize) -> Vec<Vec<f64>> {
    (0..n).map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect()
}

/// BFGS update of the inverse Hessian approximation.
fn bfgs(h: &mut [Vec<f64>], s: &[f64], y: &[f64]) {
    let sy: f64 = s.iter().zip(y).map(|(a, b)| a * b).sum();
    if sy <= 1e-12 {
        return;
    }
    let n = s.len();
    let hy: Vec<f64> = (0..n).map(|i| (0..n).map(|j| h[i][j] * y[j]).sum()).collect();
    let yhy: f64 = y.iter().zip(&hy).map(|(a, b)| a * b).sum();
    let rho = 1.0 / sy;
    for i in 0..n {
        for j in 0..n {
            h[i][j] += (1.0 + yhy * rho) * rho * s[i] * s[j] - rho * (hy[i] * s[j] + s[i] * hy[j]);
        }
    }
}

impl Engine for Grg {
    fn step(&mut self, p: &mut dyn Problem, evals: u64) -> Step {
        if let Some(o) = &self.done {
            return Step::Done(o.clone());
        }
        if !self.started {
            self.started = true;
            if self.requires_bounds() {
                return self.finish(p, Status::NeedsBounds);
            }
            let Some(e0) = p.eval(&self.spec.x0) else { return self.finish(p, Status::ErrorValue) };
            // Automatic scaling: work with an objective of order one.
            if self.spec.options.scaling && e0.objective.is_finite() && e0.objective.abs() > 1.0 {
                self.scale = 1.0 / e0.objective.abs();
            }
            self.make_starts();
        }
        let mut budget = evals.max(1);
        while budget > 0 {
            if self.cur.is_none() {
                // Next local solve: a branch-and-bound node, else the next multistart point.
                let (lo, hi, x0) = if let Some((lo, hi)) = self.nodes.pop() {
                    self.subproblems += 1;
                    let o = &self.spec.options;
                    if o.max_subproblems > 0 && self.subproblems > o.max_subproblems {
                        return self.finish(p, Status::MaxSubproblems);
                    }
                    if o.max_feasible > 0 && self.feasible_found >= o.max_feasible {
                        return self.finish(p, Status::MaxFeasible);
                    }
                    let start = self.best.as_ref().map(|b| b.1.clone()).unwrap_or_else(|| self.spec.x0.clone());
                    (lo, hi, start)
                } else if let Some(x0) = self.starts.pop() {
                    let (lo, hi) = self.root_bounds();
                    (lo, hi, x0)
                } else {
                    return self.finish(p, Status::Optimal);
                };
                if lo.iter().zip(&hi).any(|(a, b)| a > b) {
                    continue;
                }
                let x0: Vec<f64> = x0.iter().enumerate().map(|(j, v)| v.clamp(lo[j], hi[j])).collect();
                let Some(e0) = p.eval(&x0) else { continue };
                self.cur = Some(self.new_local(&x0, lo, hi, &e0));
            }
            let Some(mut l) = self.cur.take() else { continue };
            self.advance(&mut l, p);
            budget = budget.saturating_sub(1 + l.x.len() as u64 / 4);
            let limit = self.spec.options.max_iterations;
            if limit > 0 && self.iterations + l.iterations >= limit {
                self.iterations += l.iterations;
                if l.status.is_none() {
                    l.status = Some(Status::MaxIterations);
                }
                let st = l.status.unwrap_or(Status::MaxIterations);
                if st.feasible()
                    && let Some(e) = p.eval(&l.x)
                {
                    self.consider(&e, l.x.clone(), st);
                }
                return self.finish(p, Status::MaxIterations);
            }
            if l.status.is_some() {
                if l.status == Some(Status::ErrorValue) && self.best.is_none() && self.starts.is_empty() && self.nodes.is_empty() {
                    return self.finish(p, Status::ErrorValue);
                }
                self.local_done(l, p);
            } else {
                self.cur = Some(l);
            }
        }
        let (iteration, objective, x) = match (&self.cur, &self.best) {
            (Some(l), _) => (self.iterations + l.iterations, f64::NAN, l.x.clone()),
            (None, Some(b)) => (self.iterations, f64::NAN, b.1.clone()),
            (None, None) => (self.iterations, f64::NAN, self.spec.x0.clone()),
        };
        Step::Running { iteration, objective, x }
    }

    fn stop(&mut self, status: Status) -> Outcome {
        if let Some(o) = &self.done {
            return (**o).clone();
        }
        if let Some(l) = self.cur.take() {
            self.iterations += l.iterations;
        }
        let x = self.best.as_ref().map(|b| b.1.clone()).unwrap_or_else(|| self.spec.x0.clone());
        let mut o = Outcome::new(status, x);
        o.iterations = self.iterations;
        o.subproblems = self.subproblems;
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver::Options;

    struct F<T: FnMut(&[f64]) -> (f64, Vec<f64>)>(T);
    impl<T: FnMut(&[f64]) -> (f64, Vec<f64>)> Problem for F<T> {
        fn eval(&mut self, x: &[f64]) -> Option<Eval> {
            let (objective, cons) = (self.0)(x);
            Some(Eval { objective, cons })
        }
    }

    fn spec(x0: Vec<f64>, sense: Sense, rels: Vec<Rel>) -> Spec {
        let n = x0.len();
        Spec {
            sense,
            x0,
            lower: vec![f64::NEG_INFINITY; n],
            upper: vec![f64::INFINITY; n],
            integer: vec![false; n],
            alldiff: vec![],
            rels,
            options: Options::default(),
        }
    }

    fn run(spec: Spec, p: &mut dyn Problem) -> Outcome {
        let mut e = Grg::new(spec);
        for _ in 0..200_000 {
            if let Step::Done(o) = e.step(p, 20) {
                return *o;
            }
        }
        panic!("didn't finish");
    }

    #[test]
    fn rosenbrock_minimum() {
        let mut p = F(|x: &[f64]| ((1.0 - x[0]).powi(2) + 100.0 * (x[1] - x[0] * x[0]).powi(2), vec![]));
        let o = run(spec(vec![-1.2, 1.0], Sense::Min, vec![]), &mut p);
        assert!(o.status.feasible(), "{:?}", o.status);
        assert!((o.x[0] - 1.0).abs() < 1e-3 && (o.x[1] - 1.0).abs() < 2e-3, "{:?}", o.x);
    }

    #[test]
    fn constrained_maximum_with_multipliers() {
        // Max x·y with x + y ≤ 10 and x, y ≥ 0 → x = y = 5, objective 25, multiplier 5.
        let mut p = F(|x: &[f64]| (x[0] * x[1], vec![x[0] + x[1] - 10.0]));
        let mut s = spec(vec![1.0, 1.0], Sense::Max, vec![Rel::Le]);
        s.lower = vec![0.0, 0.0];
        let o = run(s, &mut p);
        assert!(o.status.feasible(), "{:?}", o.status);
        assert!((o.x[0] - 5.0).abs() < 1e-3 && (o.x[1] - 5.0).abs() < 1e-3, "{:?} {:?} {}", o.x, o.status, o.iterations);
        let sens = o.nlp.unwrap();
        assert!((sens.lagrange[0] - 5.0).abs() < 1e-2, "{:?}", sens.lagrange);
        // Equality and Value Of: x² + y² = 1 nearest to (2, 1) … minimise (x−2)² + (y−1)².
        let mut p = F(|x: &[f64]| ((x[0] - 2.0).powi(2) + (x[1] - 1.0).powi(2), vec![x[0] * x[0] + x[1] * x[1] - 1.0]));
        let o = run(spec(vec![0.5, 0.5], Sense::Min, vec![Rel::Eq]), &mut p);
        let r = (5.0f64).sqrt();
        assert!((o.x[0] - 2.0 / r).abs() < 1e-3 && (o.x[1] - 1.0 / r).abs() < 1e-3, "{:?}", o.x);
        let mut p = F(|x: &[f64]| (x[0] * x[0] * x[0], vec![]));
        let o = run(spec(vec![1.0], Sense::Target(8.0), vec![]), &mut p);
        assert!((o.x[0] - 2.0).abs() < 1e-3, "{:?}", o.x);
    }

    #[test]
    fn bounds_integers_and_infeasible() {
        // Min (x − 2.6)² with x integer in [0, 10] → 3.
        let mut p = F(|x: &[f64]| ((x[0] - 2.6).powi(2), vec![]));
        let mut s = spec(vec![0.0], Sense::Min, vec![]);
        s.lower = vec![0.0];
        s.upper = vec![10.0];
        s.integer = vec![true];
        let o = run(s, &mut p);
        assert_eq!(o.x, vec![3.0]);
        // x ≥ 5 and x ≤ 1 can't both hold.
        let mut p = F(|x: &[f64]| (x[0], vec![x[0] - 5.0, x[0] - 1.0]));
        let o = run(spec(vec![0.0], Sense::Min, vec![Rel::Ge, Rel::Le]), &mut p);
        assert_eq!(o.status, Status::Infeasible);
        // Multistart needs bounds.
        let mut p = F(|x: &[f64]| (x[0] * x[0], vec![]));
        let mut s = spec(vec![1.0], Sense::Min, vec![]);
        s.options.multistart = true;
        assert_eq!(run(s, &mut p).status, Status::NeedsBounds);
    }

    #[test]
    fn multistart_finds_the_global_minimum() {
        // Two valleys: near −2 (deeper) and near 2.
        let mut p = F(|x: &[f64]| ((x[0] * x[0] - 4.0).powi(2) + x[0], vec![]));
        let mut s = spec(vec![2.0], Sense::Min, vec![]);
        s.lower = vec![-5.0];
        s.upper = vec![5.0];
        s.options.multistart = true;
        s.options.population = 10;
        s.options.seed = 3;
        let o = run(s, &mut p);
        assert!(o.x[0] < 0.0, "{:?}", o.x);
    }
}
