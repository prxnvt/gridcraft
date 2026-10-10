//! Simplex LP: reads the model's coefficients by evaluating it at the starting point and one
//! step along each variable, checks that the model really is linear, then solves the linear
//! program, with branch and bound for integer, binary and AllDifferent constraints.

use super::lp::{self, Lp, LpSolution, LpStatus};
use super::{BIG, Engine, LpSensitivity, Outcome, Problem, Rel, Sense, Spec, Status, Step};
use crate::rng::Rng;

/// A branch-and-bound node: tightened bounds plus extra rows (AllDifferent branching).
#[derive(Clone, Debug)]
struct Node {
    lower: Vec<f64>,
    upper: Vec<f64>,
    rows: Vec<(Vec<f64>, Rel, f64)>,
}

enum Phase {
    /// Evaluate the base point and the unit steps.
    Extract,
    /// Check linearity at extra points.
    Check,
    /// Solve (branch and bound when there are integers).
    Solve,
    Done(Box<Outcome>),
}

pub struct Simplex {
    spec: Spec,
    phase: Phase,
    base: Vec<f64>,
    /// Points still to evaluate in this phase, and the results so far.
    pending: Vec<Vec<f64>>,
    evals: Vec<super::Eval>,
    lp: Option<Lp>,
    /// Objective coefficients and constant (user's sense), per variable.
    obj_coef: Vec<f64>,
    obj_const: f64,
    rows_from_model: usize,
    stack: Vec<Node>,
    incumbent: Option<(f64, Vec<f64>)>,
    within_tolerance: bool,
    feasible_found: u64,
    subproblems: u64,
    pivots: u64,
    root: Option<LpSolution>,
}

impl Simplex {
    pub fn new(spec: Spec) -> Simplex {
        let n = spec.n();
        let base: Vec<f64> = (0..n)
            .map(|j| {
                let x = spec.x0.get(j).copied().unwrap_or(0.0);
                let x = if x.is_finite() { x } else { 0.0 };
                x.clamp(spec.lower.get(j).copied().unwrap_or(f64::NEG_INFINITY), spec.upper.get(j).copied().unwrap_or(f64::INFINITY))
            })
            .collect();
        let mut pending = vec![base.clone()];
        for j in 0..n {
            let mut p = base.clone();
            if let Some(v) = p.get_mut(j) {
                *v += 1.0;
            }
            pending.push(p);
        }
        Simplex {
            spec,
            phase: Phase::Extract,
            base,
            pending,
            evals: vec![],
            lp: None,
            obj_coef: vec![],
            obj_const: 0.0,
            rows_from_model: 0,
            stack: vec![],
            incumbent: None,
            within_tolerance: false,
            feasible_found: 0,
            subproblems: 0,
            pivots: 0,
            root: None,
        }
    }

    fn finish(&mut self, status: Status, x: Vec<f64>) -> Step {
        let mut o = Outcome::new(status, x);
        o.iterations = self.pivots;
        o.subproblems = self.subproblems;
        if !self.spec.has_integers()
            && let Some(r) = &self.root
            && r.status == LpStatus::Optimal
        {
            o.lp = Some(self.sensitivity(r));
        }
        self.phase = Phase::Done(Box::new(o.clone()));
        Step::Done(Box::new(o))
    }

    /// Sensitivity in the user's objective sense, for the model's constraint rows.
    fn sensitivity(&self, r: &LpSolution) -> LpSensitivity {
        let max = matches!(self.spec.sense, Sense::Max);
        let flip = |v: f64| if max { -v } else { v };
        let rows = self.rows_from_model;
        LpSensitivity {
            reduced_cost: r.reduced.iter().map(|v| flip(*v)).collect(),
            objective_coef: self.obj_coef.clone(),
            coef_increase: if max { r.coef_decrease.clone() } else { r.coef_increase.clone() },
            coef_decrease: if max { r.coef_increase.clone() } else { r.coef_decrease.clone() },
            shadow_price: r.duals.iter().take(rows).map(|v| flip(*v)).collect(),
            rhs_increase: r.rhs_increase.iter().take(rows).copied().collect(),
            rhs_decrease: r.rhs_decrease.iter().take(rows).copied().collect(),
        }
    }

    /// Builds the LP from the base point and unit-step evaluations.
    fn build(&mut self) -> Result<(), Status> {
        let n = self.spec.n();
        let (Some(e0), true) = (self.evals.first(), self.evals.len() == n + 1) else { return Err(Status::BadModel) };
        let m = e0.cons.len();
        let mut c = vec![0.0; n];
        let mut a = vec![vec![0.0; n]; m];
        for j in 0..n {
            let Some(ej) = self.evals.get(j + 1) else { return Err(Status::BadModel) };
            c[j] = ej.objective - e0.objective;
            for (i, row) in a.iter_mut().enumerate() {
                row[j] = ej.cons.get(i).copied().unwrap_or(0.0) - e0.cons.get(i).copied().unwrap_or(0.0);
            }
        }
        let dot = |row: &[f64]| row.iter().zip(&self.base).map(|(p, q)| p * q).sum::<f64>();
        // g(x) = g0 + A(x − base) REL 0  ⇒  A x REL A·base − g0.
        let mut b: Vec<f64> = (0..m).map(|i| dot(&a[i]) - e0.cons.get(i).copied().unwrap_or(0.0)).collect();
        let mut rel = self.spec.rels.clone();
        rel.resize(m, Rel::Le);
        self.obj_coef = c.clone();
        self.obj_const = e0.objective - dot(&c);
        self.rows_from_model = m;
        let lp_c = match self.spec.sense {
            Sense::Max => c.iter().map(|v| -v).collect(),
            Sense::Min => c.clone(),
            Sense::Target(v) => {
                // The objective becomes the equality f(x) = v.
                b.push(v - self.obj_const);
                a.push(c.clone());
                rel.push(Rel::Eq);
                vec![0.0; n]
            }
        };
        self.lp = Some(Lp { c: lp_c, a, rel, b, lower: self.spec.lower.clone(), upper: self.spec.upper.clone() });
        Ok(())
    }

    /// Two extra points for the linearity check, inside the bounds.
    fn check_points(&self) -> Vec<Vec<f64>> {
        let mut rng = Rng::new(0x5EED_1234);
        (0..2)
            .map(|_| {
                self.base
                    .iter()
                    .enumerate()
                    .map(|(j, x)| {
                        let (l, u) =
                            (self.spec.lower.get(j).copied().unwrap_or(f64::NEG_INFINITY), self.spec.upper.get(j).copied().unwrap_or(f64::INFINITY));
                        let (lo, hi) = (if l.is_finite() { l } else { x - 10.0 }, if u.is_finite() { u } else { x + 10.0 });
                        let v = rng.uniform_between(lo, hi);
                        if self.spec.integer.get(j).copied().unwrap_or(false) { v.round().clamp(lo, hi) } else { v }
                    })
                    .collect()
            })
            .collect()
    }

    /// Whether the model is linear at `x` (prediction vs evaluation).
    fn linear_at(&self, x: &[f64], e: &super::Eval) -> bool {
        let Some(lp) = &self.lp else { return false };
        let e0 = match self.evals.first() {
            Some(e) => e,
            None => return false,
        };
        let close = |pred: f64, got: f64| (pred - got).abs() <= 1e-6 * (1.0 + pred.abs().max(got.abs()));
        let pred_obj = self.obj_const + self.obj_coef.iter().zip(x).map(|(c, v)| c * v).sum::<f64>();
        if !close(pred_obj, e.objective) {
            return false;
        }
        (0..self.rows_from_model).all(|i| {
            let row = lp.a.get(i).map(Vec::as_slice).unwrap_or(&[]);
            let pred = e0.cons.get(i).copied().unwrap_or(0.0) + row.iter().zip(x.iter().zip(&self.base)).map(|(a, (v, b))| a * (v - b)).sum::<f64>();
            close(pred, e.cons.get(i).copied().unwrap_or(f64::NAN))
        })
    }

    /// The root node's integrality: binaries in [0, 1], AllDifferent groups in [1, k].
    fn root_node(&self) -> Node {
        let mut lower = self.spec.lower.clone();
        let mut upper = self.spec.upper.clone();
        for g in &self.spec.alldiff {
            for &j in g {
                if let (Some(l), Some(u)) = (lower.get_mut(j), upper.get_mut(j)) {
                    *l = l.max(1.0);
                    *u = u.min(g.len() as f64);
                }
            }
        }
        if self.spec.has_integers() {
            for j in 0..lower.len() {
                if self.spec.integer.get(j).copied().unwrap_or(false) || self.spec.alldiff.iter().any(|g| g.contains(&j)) {
                    lower[j] = lower[j].ceil();
                    upper[j] = upper[j].floor();
                }
            }
        }
        Node { lower, upper, rows: vec![] }
    }

    fn solve_node(&mut self, node: &Node) -> LpSolution {
        let Some(base) = &self.lp else { return lp::solve(&Lp { c: vec![], a: vec![], rel: vec![], b: vec![], lower: vec![], upper: vec![] }, 1) };
        let mut lp = base.clone();
        lp.lower = node.lower.clone();
        lp.upper = node.upper.clone();
        for (a, r, b) in &node.rows {
            lp.a.push(a.clone());
            lp.rel.push(*r);
            lp.b.push(*b);
        }
        let limit = if self.spec.options.max_iterations > 0 { self.spec.options.max_iterations.saturating_sub(self.pivots).max(1) } else { 0 };
        let s = lp::solve(&lp, limit);
        self.pivots += s.pivots;
        s
    }

    fn is_int(&self, j: usize) -> bool {
        self.spec.has_integers() && (self.spec.integer.get(j).copied().unwrap_or(false) || self.spec.alldiff.iter().any(|g| g.contains(&j)))
    }

    /// Runs up to `budget` branch-and-bound nodes.
    fn branch(&mut self, budget: u64) -> Option<Step> {
        let o = &self.spec.options;
        let (tol, max_sub, max_feas) = (o.integer_tolerance.max(0.0), o.max_subproblems, o.max_feasible);
        for _ in 0..budget.max(1) {
            let Some(node) = self.stack.pop() else {
                return Some(match self.incumbent.clone() {
                    Some((_, x)) => self.finish(if self.within_tolerance { Status::IntegerWithinTolerance } else { Status::Optimal }, x),
                    None => self.finish(Status::Infeasible, self.base.clone()),
                });
            };
            if max_sub > 0 && self.subproblems >= max_sub {
                let x = self.incumbent.as_ref().map(|i| i.1.clone()).unwrap_or_else(|| self.base.clone());
                return Some(self.finish(Status::MaxSubproblems, x));
            }
            self.subproblems += 1;
            let s = self.solve_node(&node);
            match s.status {
                LpStatus::Optimal => {}
                LpStatus::Infeasible => continue,
                LpStatus::Unbounded => return Some(self.finish(Status::Unbounded, self.base.clone())),
                LpStatus::IterationLimit => {
                    let x = self.incumbent.as_ref().map(|i| i.1.clone()).unwrap_or_else(|| self.base.clone());
                    return Some(self.finish(Status::MaxIterations, x));
                }
                LpStatus::TooLarge => return Some(self.finish(Status::TooLarge, self.base.clone())),
            }
            if self.subproblems == 1 {
                self.root = Some(s.clone());
            }
            if let Some((best, _)) = &self.incumbent {
                if s.objective >= *best - 1e-9 {
                    continue;
                }
                // Integer optimality: a node that can't beat the incumbent by more than the
                // tolerance isn't explored.
                if s.objective >= *best - tol * best.abs().max(1e-9) {
                    self.within_tolerance = true;
                    continue;
                }
            }
            // The most fractional integer variable.
            let frac = (0..s.x.len())
                .filter(|&j| self.is_int(j))
                .map(|j| (j, s.x[j], (s.x[j] - s.x[j].round()).abs()))
                .filter(|(_, _, f)| *f > 1e-6)
                .max_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((j, v, _)) = frac {
                let mut down = node.clone();
                down.upper[j] = v.floor();
                let mut up = node;
                up.lower[j] = v.ceil();
                // Explore the nearer side first (pushed last).
                if v - v.floor() < 0.5 {
                    self.stack.push(up);
                    self.stack.push(down);
                } else {
                    self.stack.push(down);
                    self.stack.push(up);
                }
                continue;
            }
            // AllDifferent: two equal values split into x_a − x_b ≤ −1 or ≥ 1.
            let n = s.x.len();
            let clash = self.spec.alldiff.iter().find_map(|g| {
                g.iter().enumerate().find_map(|(p, &a)| g.iter().skip(p + 1).find(|&&b| (s.x[a] - s.x[b]).abs() < 0.5).map(|&b| (a, b)))
            });
            if let Some((a, b)) = clash.filter(|_| self.spec.has_integers()) {
                let mut row = vec![0.0; n];
                row[a] = 1.0;
                row[b] = -1.0;
                let mut left = node.clone();
                left.rows.push((row.clone(), Rel::Le, -1.0));
                let mut right = node;
                right.rows.push((row, Rel::Ge, 1.0));
                self.stack.push(right);
                self.stack.push(left);
                continue;
            }
            // A feasible integer solution.
            let x: Vec<f64> = s.x.iter().enumerate().map(|(j, v)| if self.is_int(j) { v.round() } else { *v }).collect();
            if self.incumbent.as_ref().is_none_or(|(best, _)| s.objective < *best) {
                self.incumbent = Some((s.objective, x));
            }
            self.feasible_found += 1;
            if max_feas > 0 && self.feasible_found >= max_feas {
                let x = self.incumbent.as_ref().map(|i| i.1.clone()).unwrap_or_default();
                return Some(self.finish(Status::MaxFeasible, x));
            }
        }
        None
    }
}

impl Engine for Simplex {
    fn step(&mut self, p: &mut dyn Problem, evals: u64) -> Step {
        loop {
            match &self.phase {
                Phase::Done(o) => return Step::Done(o.clone()),
                Phase::Extract | Phase::Check => {
                    let take = (evals.max(1) as usize).min(self.pending.len());
                    let batch: Vec<Vec<f64>> = self.pending.drain(..take).collect();
                    let results = p.eval_many(&batch);
                    for (x, r) in batch.iter().zip(results) {
                        let Some(e) = r else { return self.finish(Status::ErrorValue, self.base.clone()) };
                        if !e.objective.is_finite() || e.cons.iter().any(|v| !v.is_finite()) {
                            return self.finish(Status::ErrorValue, self.base.clone());
                        }
                        if matches!(self.phase, Phase::Check) {
                            if !self.linear_at(x, &e) {
                                return self.finish(Status::NotLinear, self.base.clone());
                            }
                        } else {
                            self.evals.push(e);
                        }
                    }
                    if !self.pending.is_empty() {
                        return Step::Running { iteration: 0, objective: self.evals.first().map_or(f64::NAN, |e| e.objective), x: self.base.clone() };
                    }
                    if matches!(self.phase, Phase::Extract) {
                        if let Err(st) = self.build() {
                            return self.finish(st, self.base.clone());
                        }
                        self.pending = self.check_points();
                        self.phase = Phase::Check;
                    } else {
                        let root = self.root_node();
                        self.stack.push(root);
                        self.phase = Phase::Solve;
                    }
                }
                Phase::Solve => {
                    if let Some(done) = self.branch(evals.max(1)) {
                        return done;
                    }
                    let (objective, x) = match &self.incumbent {
                        Some((o, x)) => (self.user_objective(*o), x.clone()),
                        None => (f64::NAN, self.base.clone()),
                    };
                    return Step::Running { iteration: self.subproblems, objective, x };
                }
            }
        }
    }

    fn stop(&mut self, status: Status) -> Outcome {
        if let Phase::Done(o) = &self.phase {
            return (**o).clone();
        }
        let x = self.incumbent.as_ref().map(|i| i.1.clone()).unwrap_or_else(|| self.base.clone());
        match self.finish(status, x) {
            Step::Done(o) => *o,
            Step::Running { x, .. } => Outcome::new(status, x),
        }
    }
}

impl Simplex {
    /// The LP objective value back in the user's terms.
    fn user_objective(&self, lp_obj: f64) -> f64 {
        match self.spec.sense {
            Sense::Max => -lp_obj + self.obj_const,
            Sense::Min => lp_obj + self.obj_const,
            Sense::Target(v) => v,
        }
    }
}

/// Re-exported for the sensitivity report's "infinity".
pub const INFINITY: f64 = BIG;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver::{Eval, Options};

    /// A linear problem given by its coefficients.
    struct Linear {
        c: Vec<f64>,
        a: Vec<Vec<f64>>,
        b: Vec<f64>,
        evals: usize,
    }

    impl Problem for Linear {
        fn eval(&mut self, x: &[f64]) -> Option<Eval> {
            self.evals += 1;
            let dot = |r: &[f64]| r.iter().zip(x).map(|(p, q)| p * q).sum::<f64>();
            Some(Eval { objective: dot(&self.c), cons: self.a.iter().zip(&self.b).map(|(r, b)| dot(r) - b).collect() })
        }
    }

    fn spec(n: usize, sense: Sense, rels: Vec<Rel>) -> Spec {
        Spec {
            sense,
            x0: vec![0.0; n],
            lower: vec![0.0; n],
            upper: vec![f64::INFINITY; n],
            integer: vec![false; n],
            alldiff: vec![],
            rels,
            options: Options::default(),
        }
    }

    fn run(spec: Spec, p: &mut dyn Problem) -> Outcome {
        let mut e = Simplex::new(spec);
        for _ in 0..10_000 {
            if let Step::Done(o) = e.step(p, 3) {
                return *o;
            }
        }
        panic!("didn't finish");
    }

    #[test]
    fn solves_an_lp_through_evaluations() {
        let mut p = Linear { c: vec![3.0, 5.0], a: vec![vec![1.0, 0.0], vec![0.0, 2.0], vec![3.0, 2.0]], b: vec![4.0, 12.0, 18.0], evals: 0 };
        let o = run(spec(2, Sense::Max, vec![Rel::Le; 3]), &mut p);
        assert_eq!(o.status, Status::Optimal);
        assert!((o.x[0] - 2.0).abs() < 1e-9 && (o.x[1] - 6.0).abs() < 1e-9);
        assert_eq!(p.evals, 2 + 1 + 2, "base, two unit steps and two linearity checks");
        let s = o.lp.unwrap();
        assert!((s.shadow_price[1] - 1.5).abs() < 1e-9);
        assert!((s.coef_increase[0] - 4.5).abs() < 1e-9);
        assert!((s.coef_decrease[0] - 3.0).abs() < 1e-9);
        assert_eq!(s.objective_coef, vec![3.0, 5.0]);
    }

    #[test]
    fn value_of_integer_and_binary() {
        // Make 2x + y equal 7 with x + y ≤ 5.
        let mut p = Linear { c: vec![2.0, 1.0], a: vec![vec![1.0, 1.0]], b: vec![5.0], evals: 0 };
        let o = run(spec(2, Sense::Target(7.0), vec![Rel::Le]), &mut p);
        assert_eq!(o.status, Status::Optimal);
        assert!((2.0 * o.x[0] + o.x[1] - 7.0).abs() < 1e-9);
        // Knapsack: max 5a + 4b + 3c with 2a + 3b + c ≤ 5 and binaries. Taking a and b weighs 5
        // and is worth 9, the best (a and c is worth 8; all three weigh too much).
        let mut p = Linear { c: vec![5.0, 4.0, 3.0], a: vec![vec![2.0, 3.0, 1.0]], b: vec![5.0], evals: 0 };
        let mut sp = spec(3, Sense::Max, vec![Rel::Le]);
        sp.integer = vec![true; 3];
        sp.upper = vec![1.0; 3];
        sp.options.integer_tolerance = 0.0;
        let o = run(sp, &mut p);
        assert_eq!(o.status, Status::Optimal);
        assert_eq!(o.x, vec![1.0, 1.0, 0.0]);
        assert!(o.lp.is_none(), "no sensitivity for integer models");
    }

    #[test]
    fn all_different_assignment() {
        // Assign 1..3 to a, b, c to maximise 1a + 2b + 3c: c = 3, b = 2, a = 1.
        let mut p = Linear { c: vec![1.0, 2.0, 3.0], a: vec![], b: vec![], evals: 0 };
        let mut sp = spec(3, Sense::Max, vec![]);
        sp.alldiff = vec![vec![0, 1, 2]];
        sp.options.integer_tolerance = 0.0;
        let o = run(sp, &mut p);
        assert_eq!(o.status, Status::Optimal);
        assert_eq!(o.x, vec![1.0, 2.0, 3.0]);
    }

    /// x² isn't linear.
    struct Square;
    impl Problem for Square {
        fn eval(&mut self, x: &[f64]) -> Option<Eval> {
            Some(Eval { objective: x[0] * x[0], cons: vec![] })
        }
    }

    struct Broken;
    impl Problem for Broken {
        fn eval(&mut self, _x: &[f64]) -> Option<Eval> {
            None
        }
    }

    #[test]
    fn refuses_nonlinear_and_error_models() {
        assert_eq!(run(spec(1, Sense::Min, vec![]), &mut Square).status, Status::NotLinear);
        assert_eq!(run(spec(1, Sense::Min, vec![]), &mut Broken).status, Status::ErrorValue);
        let mut p = Linear { c: vec![1.0], a: vec![vec![1.0]], b: vec![-1.0], evals: 0 };
        assert_eq!(run(spec(1, Sense::Max, vec![Rel::Ge]), &mut p).status, Status::Unbounded);
        let mut p = Linear { c: vec![1.0], a: vec![vec![1.0]], b: vec![-1.0], evals: 0 };
        assert_eq!(run(spec(1, Sense::Max, vec![Rel::Le]), &mut p).status, Status::Infeasible);
    }
}
