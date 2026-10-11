//! Evolutionary: a genetic algorithm for models that aren't smooth (IF, lookups, rounding…).
//!
//! A population of candidate solutions within the variable bounds evolves by tournament
//! selection, blend crossover and mutation (at the Mutation Rate), keeping the best member each
//! generation. Feasibility comes first: a point that satisfies the constraints beats one that
//! doesn't; between infeasible points, the smaller violation wins. Integer variables are
//! rounded and AllDifferent groups repaired into permutations. The search stops when the
//! population has converged, when it stops improving, or when a limit is reached.

use super::{Engine, Eval, Outcome, Problem, Sense, Spec, Status, Step, violation};
use crate::rng::Rng;

#[derive(Clone, Debug)]
struct Member {
    x: Vec<f64>,
    /// Objective in minimisation form (lower is better).
    obj: f64,
    viol: f64,
}

impl Member {
    /// Feasibility first, then the objective.
    fn better(&self, o: &Member) -> bool {
        match (self.viol == 0.0, o.viol == 0.0) {
            (true, false) => true,
            (false, true) => false,
            (true, true) => self.obj < o.obj,
            (false, false) => self.viol < o.viol,
        }
    }
}

pub struct Evolutionary {
    spec: Spec,
    rng: Rng,
    lo: Vec<f64>,
    hi: Vec<f64>,
    pop: Vec<Member>,
    best: Option<Member>,
    generation: u64,
    evaluations: u64,
    last_improvement: u64,
    started: bool,
    done: Option<Box<Outcome>>,
}

impl Evolutionary {
    pub fn new(spec: Spec) -> Evolutionary {
        let seed = if spec.options.seed == 0 { 0x9E37_79B9 } else { spec.options.seed };
        Evolutionary {
            rng: Rng::new(seed),
            lo: vec![],
            hi: vec![],
            pop: vec![],
            best: None,
            generation: 0,
            evaluations: 0,
            last_improvement: 0,
            started: false,
            done: None,
            spec,
        }
    }

    fn is_int(&self, j: usize) -> bool {
        self.spec.has_integers() && (self.spec.integer.get(j).copied().unwrap_or(false) || self.spec.alldiff.iter().any(|g| g.contains(&j)))
    }

    /// Clamps, rounds integers and turns AllDifferent groups into permutations of 1..=k.
    fn repair(&self, x: &mut [f64]) {
        for (j, v) in x.iter_mut().enumerate() {
            *v = v.clamp(self.lo[j], self.hi[j]);
            if self.is_int(j) {
                *v = v.round().clamp(self.lo[j].ceil(), self.hi[j].floor());
            }
        }
        if !self.spec.has_integers() {
            return;
        }
        for g in &self.spec.alldiff {
            // Rank the group's values: the smallest gets 1, the next 2…
            let mut order: Vec<(usize, f64)> = g.iter().map(|&j| (j, x.get(j).copied().unwrap_or(0.0))).collect();
            order.sort_by(|a, b| a.1.total_cmp(&b.1));
            for (rank, (j, _)) in order.into_iter().enumerate() {
                if let Some(v) = x.get_mut(j) {
                    *v = (rank + 1) as f64;
                }
            }
        }
    }

    fn score(&self, x: Vec<f64>, e: Option<Eval>) -> Member {
        match e {
            None => Member { x, obj: f64::INFINITY, viol: f64::INFINITY },
            Some(e) => {
                let obj = match self.spec.sense {
                    Sense::Max => -e.objective,
                    Sense::Min => e.objective,
                    Sense::Target(v) => (e.objective - v).abs(),
                };
                let mut viol = violation(&self.spec.rels, &e, self.spec.options.precision);
                if let Sense::Target(v) = self.spec.sense {
                    // Value Of: hitting the target is the requirement.
                    let miss = (e.objective - v).abs();
                    if miss > self.spec.options.precision.max(1e-9 * v.abs()) {
                        viol += miss;
                    }
                }
                Member { x, obj: if obj.is_finite() { obj } else { f64::INFINITY }, viol: if viol.is_finite() { viol } else { f64::INFINITY } }
            }
        }
    }

    fn evaluate(&mut self, p: &mut dyn Problem, xs: Vec<Vec<f64>>) -> Vec<Member> {
        let evs = p.eval_many(&xs);
        self.evaluations += xs.len() as u64;
        let members: Vec<Member> = xs.into_iter().zip(evs).map(|(x, e)| self.score(x, e)).collect();
        for m in &members {
            if self.best.as_ref().is_none_or(|b| m.better(b)) {
                let improved = self.best.as_ref().is_none_or(|b| m.viol < b.viol || m.obj < b.obj - 1e-12 * b.obj.abs().max(1.0));
                self.best = Some(m.clone());
                if improved {
                    self.last_improvement = self.evaluations;
                }
            }
        }
        members
    }

    fn random_point(&mut self) -> Vec<f64> {
        let mut x: Vec<f64> = (0..self.lo.len()).map(|j| self.rng.uniform_between(self.lo[j], self.hi[j])).collect();
        self.repair(&mut x);
        x
    }

    fn tournament(&mut self) -> usize {
        let n = self.pop.len() as u64;
        let a = self.rng.below(n) as usize;
        let b = self.rng.below(n) as usize;
        match (self.pop.get(a), self.pop.get(b)) {
            (Some(x), Some(y)) if y.better(x) => b,
            _ => a,
        }
    }

    fn child(&mut self) -> Vec<f64> {
        let (i, k) = (self.tournament(), self.tournament());
        let (pa, pb) = match (self.pop.get(i), self.pop.get(k)) {
            (Some(a), Some(b)) => (a.x.clone(), b.x.clone()),
            _ => return self.random_point(),
        };
        let rate = self.spec.options.mutation_rate;
        let mut x: Vec<f64> = (0..pa.len())
            .map(|j| {
                // Blend crossover with a little extrapolation.
                let t = self.rng.uniform_between(-0.25, 1.25);
                let mut v = pa[j] + t * (pb[j] - pa[j]);
                if self.rng.uniform() < rate {
                    let span = (self.hi[j] - self.lo[j]).max(1e-9);
                    v += self.rng.normal(0.0, 0.1 * span);
                }
                v
            })
            .collect();
        self.repair(&mut x);
        x
    }

    fn finish(&mut self, status: Status) -> Step {
        let (x, st) = match &self.best {
            Some(b) if b.viol == 0.0 => (b.x.clone(), status),
            Some(b) => (b.x.clone(), if status.feasible() { Status::Infeasible } else { status }),
            None => (self.spec.x0.clone(), if status.feasible() { Status::Infeasible } else { status }),
        };
        let mut o = Outcome::new(st, x);
        o.iterations = self.generation;
        o.subproblems = self.evaluations;
        // The final population's spread per variable.
        if !self.pop.is_empty() {
            let k = self.pop.len() as f64;
            o.population = (0..self.spec.n())
                .map(|j| {
                    let vals: Vec<f64> = self.pop.iter().filter_map(|m| m.x.get(j).copied()).collect();
                    let mean = vals.iter().sum::<f64>() / k;
                    let sd = if vals.len() > 1 { (vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (k - 1.0)).sqrt() } else { 0.0 };
                    let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
                    [mean, sd, max, min]
                })
                .collect();
        }
        self.done = Some(Box::new(o.clone()));
        Step::Done(Box::new(o))
    }

    fn user_objective(&self) -> f64 {
        match (&self.best, self.spec.sense) {
            (Some(b), Sense::Max) if b.viol == 0.0 => -b.obj,
            (Some(b), Sense::Min) if b.viol == 0.0 => b.obj,
            _ => f64::NAN,
        }
    }

    /// Whether the population has converged: almost all members within Convergence of the best.
    fn converged(&self) -> bool {
        let Some(b) = &self.best else { return false };
        if b.viol > 0.0 || self.generation < 5 {
            return false;
        }
        let tol = self.spec.options.convergence * b.obj.abs().max(1.0);
        let close = self.pop.iter().filter(|m| m.viol == 0.0 && (m.obj - b.obj).abs() <= tol).count();
        close * 100 >= self.pop.len() * 99
    }
}

impl Engine for Evolutionary {
    fn step(&mut self, p: &mut dyn Problem, evals: u64) -> Step {
        if let Some(o) = &self.done {
            return Step::Done(o.clone());
        }
        let n = self.spec.n();
        if !self.started {
            self.started = true;
            let missing = self.spec.lower.iter().zip(&self.spec.upper).any(|(l, u)| !l.is_finite() || !u.is_finite());
            if missing && self.spec.options.require_bounds {
                return self.finish(Status::NeedsBounds);
            }
            // Bounds to search in (±100 around the start where none are given).
            self.lo = (0..n).map(|j| if self.spec.lower[j].is_finite() { self.spec.lower[j] } else { self.spec.x0[j] - 100.0 }).collect();
            self.hi = (0..n).map(|j| if self.spec.upper[j].is_finite() { self.spec.upper[j] } else { self.spec.x0[j] + 100.0 }).collect();
            for g in &self.spec.alldiff {
                for &j in g {
                    self.lo[j] = self.lo[j].max(1.0);
                    self.hi[j] = self.hi[j].min(g.len() as f64);
                }
            }
            if self.lo.iter().zip(&self.hi).any(|(l, h)| l > h) {
                return self.finish(Status::Infeasible);
            }
            let size = self.spec.options.population.clamp(10, 1000);
            let mut x0 = self.spec.x0.clone();
            self.repair(&mut x0);
            let mut xs = vec![x0];
            while xs.len() < size {
                xs.push(self.random_point());
            }
            self.pop = self.evaluate(p, xs);
        }
        let mut spent = 0u64;
        while spent < evals.max(1) {
            let o = &self.spec.options;
            if o.max_iterations > 0 && self.generation >= o.max_iterations {
                return self.finish(Status::MaxIterations);
            }
            if self.converged() {
                return self.finish(Status::Converged);
            }
            if self.evaluations.saturating_sub(self.last_improvement) > o.patience.max(self.pop.len() as u64 * 5) {
                return self.finish(Status::CannotImprove);
            }
            // A generation: the best survives, the rest are children.
            let size = self.pop.len();
            let kids: Vec<Vec<f64>> = (1..size).map(|_| self.child()).collect();
            let mut next = Vec::with_capacity(size);
            if let Some(b) = &self.best {
                next.push(b.clone());
            }
            next.extend(self.evaluate(p, kids));
            self.pop = next;
            self.generation += 1;
            spent += size as u64;
        }
        let x = self.best.as_ref().map(|b| b.x.clone()).unwrap_or_else(|| self.spec.x0.clone());
        Step::Running { iteration: self.generation, objective: self.user_objective(), x }
    }

    fn stop(&mut self, status: Status) -> Outcome {
        if let Some(o) = &self.done {
            return (**o).clone();
        }
        match self.finish(status) {
            Step::Done(o) => *o,
            Step::Running { x, .. } => Outcome::new(status, x),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver::{Options, Rel};

    struct F<T: FnMut(&[f64]) -> (f64, Vec<f64>)>(T);
    impl<T: FnMut(&[f64]) -> (f64, Vec<f64>)> Problem for F<T> {
        fn eval(&mut self, x: &[f64]) -> Option<Eval> {
            let (objective, cons) = (self.0)(x);
            Some(Eval { objective, cons })
        }
    }

    fn spec(n: usize, sense: Sense, rels: Vec<Rel>) -> Spec {
        let o = Options { seed: 11, patience: 4000, ..Options::default() };
        Spec { sense, x0: vec![0.0; n], lower: vec![-10.0; n], upper: vec![10.0; n], integer: vec![false; n], alldiff: vec![], rels, options: o }
    }

    fn run(spec: Spec, p: &mut dyn Problem) -> Outcome {
        let mut e = Evolutionary::new(spec);
        for _ in 0..100_000 {
            if let Step::Done(o) = e.step(p, 200) {
                return *o;
            }
        }
        panic!("didn't finish");
    }

    #[test]
    fn finds_a_non_smooth_minimum() {
        // |x − 3| + |y + 2| with a kink at the minimum.
        let mut p = F(|x: &[f64]| ((x[0] - 3.0).abs() + (x[1] + 2.0).abs(), vec![]));
        let o = run(spec(2, Sense::Min, vec![]), &mut p);
        assert!(o.status.feasible(), "{:?}", o.status);
        assert!((o.x[0] - 3.0).abs() < 0.05 && (o.x[1] + 2.0).abs() < 0.05, "{:?}", o.x);
    }

    #[test]
    fn respects_constraints_and_integers() {
        // Max x + y with x + 2y ≤ 7, integers in [0, 10] → (7, 0).
        let mut p = F(|x: &[f64]| (x[0] + x[1], vec![x[0] + 2.0 * x[1] - 7.0]));
        let mut s = spec(2, Sense::Max, vec![Rel::Le]);
        s.lower = vec![0.0, 0.0];
        s.integer = vec![true, true];
        let o = run(s, &mut p);
        assert_eq!(o.x, vec![7.0, 0.0]);
        // A permutation: minimise Σ position·value over AllDifferent 1..=3 → (3, 2, 1).
        let mut p = F(|x: &[f64]| (x[0] + 2.0 * x[1] + 3.0 * x[2], vec![]));
        let mut s = spec(3, Sense::Min, vec![]);
        s.alldiff = vec![vec![0, 1, 2]];
        let o = run(s, &mut p);
        assert_eq!(o.x, vec![3.0, 2.0, 1.0]);
    }

    #[test]
    fn bounds_and_infeasibility() {
        let mut p = F(|x: &[f64]| (x[0], vec![]));
        let mut s = spec(1, Sense::Min, vec![]);
        s.upper = vec![f64::INFINITY];
        assert_eq!(run(s, &mut p).status, Status::NeedsBounds);
        let mut p = F(|x: &[f64]| (x[0], vec![x[0] - 50.0]));
        assert_eq!(run(spec(1, Sense::Min, vec![Rel::Ge]), &mut p).status, Status::Infeasible);
    }
}
