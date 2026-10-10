//! Solver engines: Simplex LP (with branch and bound), GRG Nonlinear and Evolutionary.
//!
//! An engine works on a [`Problem`]: something that, given values for the decision variables,
//! returns the objective and each constraint's `left − right`. The engine crate implements it
//! by recalculating the workbook with the variable cells replaced. Engines run a slice at a time
//! ([`Engine::step`]) so a long solve can be shown, paused and stopped.

pub mod lp;
pub mod simplex;

/// A constraint row's relation (`left − right` REL 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rel {
    Le,
    Eq,
    Ge,
}

/// What to do with the objective.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sense {
    Max,
    Min,
    /// Make the objective equal this value.
    Target(f64),
}

/// The objective and every constraint's `left − right` at a point.
#[derive(Clone, Debug, PartialEq)]
pub struct Eval {
    pub objective: f64,
    pub cons: Vec<f64>,
}

/// The model being optimised.
pub trait Problem {
    /// Evaluates at `x`; `None` when the model gives an error value there.
    fn eval(&mut self, x: &[f64]) -> Option<Eval>;
    /// Evaluates several points (implementations may do this in parallel).
    fn eval_many(&mut self, xs: &[Vec<f64>]) -> Vec<Option<Eval>> {
        xs.iter().map(|x| self.eval(x)).collect()
    }
}

/// Options shared by the engines (Excel's Solver Options).
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Constraint precision: how far a constraint may be off and still count as satisfied.
    pub precision: f64,
    pub scaling: bool,
    pub ignore_integer: bool,
    /// Integer optimality as a fraction (0.01 = 1%).
    pub integer_tolerance: f64,
    /// 0 = no limit.
    pub max_iterations: u64,
    pub max_subproblems: u64,
    pub max_feasible: u64,
    pub convergence: f64,
    pub central: bool,
    pub multistart: bool,
    pub population: usize,
    pub seed: u64,
    pub require_bounds: bool,
    pub mutation_rate: f64,
    /// Evolutionary: stop after this many evaluations without improvement (the engine crate turns
    /// "max time without improvement" into a count).
    pub patience: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            precision: 1e-6,
            scaling: true,
            ignore_integer: false,
            integer_tolerance: 0.01,
            max_iterations: 0,
            max_subproblems: 0,
            max_feasible: 0,
            convergence: 1e-4,
            central: false,
            multistart: false,
            population: 100,
            seed: 0,
            require_bounds: true,
            mutation_rate: 0.075,
            patience: 20_000,
        }
    }
}

/// The problem's structure: variables, bounds, integrality and constraint relations.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub sense: Sense,
    /// Starting values.
    pub x0: Vec<f64>,
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
    pub integer: Vec<bool>,
    /// Groups of variables that must take the values 1..=k in some order.
    pub alldiff: Vec<Vec<usize>>,
    /// One relation per constraint the problem evaluates.
    pub rels: Vec<Rel>,
    pub options: Options,
}

impl Spec {
    pub fn n(&self) -> usize {
        self.x0.len()
    }
    /// Whether any integrality applies (after Ignore Integer Constraints).
    pub fn has_integers(&self) -> bool {
        !self.options.ignore_integer && (self.integer.iter().any(|b| *b) || !self.alldiff.is_empty())
    }
}

/// How a solve ended. The codes are those Excel's Solver reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// 0: optimal; all constraints and optimality conditions satisfied.
    Optimal,
    /// 1: converged to the current solution; constraints satisfied.
    Converged,
    /// 2: can't improve the current solution; constraints satisfied.
    CannotImprove,
    /// 3: the iteration limit was reached.
    MaxIterations,
    /// 4: the objective doesn't converge (unbounded).
    Unbounded,
    /// 5: no feasible solution.
    Infeasible,
    /// 6: stopped by the user.
    Stopped,
    /// 7: the model isn't linear (Simplex LP).
    NotLinear,
    /// 8: too large.
    TooLarge,
    /// 9: an error value in the objective or a constraint.
    ErrorValue,
    /// 10: the time limit was reached.
    MaxTime,
    /// 13: the model is invalid.
    BadModel,
    /// 14: an integer solution within the integer optimality tolerance.
    IntegerWithinTolerance,
    /// 15: the limit on feasible integer solutions was reached.
    MaxFeasible,
    /// 16: the limit on integer subproblems was reached.
    MaxSubproblems,
    /// 18: some variables lack the bounds the method needs.
    NeedsBounds,
}

impl Status {
    pub fn code(self) -> u8 {
        match self {
            Status::Optimal => 0,
            Status::Converged => 1,
            Status::CannotImprove => 2,
            Status::MaxIterations => 3,
            Status::Unbounded => 4,
            Status::Infeasible => 5,
            Status::Stopped => 6,
            Status::NotLinear => 7,
            Status::TooLarge => 8,
            Status::ErrorValue => 9,
            Status::MaxTime => 10,
            Status::BadModel => 13,
            Status::IntegerWithinTolerance => 14,
            Status::MaxFeasible => 15,
            Status::MaxSubproblems => 16,
            Status::NeedsBounds => 18,
        }
    }
    /// Whether the result satisfies the constraints (worth keeping).
    pub fn feasible(self) -> bool {
        matches!(self, Status::Optimal | Status::Converged | Status::CannotImprove | Status::IntegerWithinTolerance)
    }
    /// What happened, for the Solver Results box.
    pub fn message(self) -> &'static str {
        match self {
            Status::Optimal => "Solver found a solution. All constraints and optimality conditions are satisfied.",
            Status::Converged => "Solver has converged to the current solution. All constraints are satisfied.",
            Status::CannotImprove => "Solver can't improve the current solution. All constraints are satisfied.",
            Status::MaxIterations => "Solver stopped after the maximum number of iterations.",
            Status::Unbounded => "The objective cell's value keeps improving without limit: the model is unbounded.",
            Status::Infeasible => "Solver couldn't find a solution that satisfies all the constraints.",
            Status::Stopped => "Solver stopped at your request.",
            Status::NotLinear => "The model isn't linear, so Simplex LP can't solve it. Try GRG Nonlinear.",
            Status::TooLarge => "The problem is too large for Solver.",
            Status::ErrorValue => "An error value appeared in the objective or a constraint cell.",
            Status::MaxTime => "Solver stopped after the maximum time.",
            Status::BadModel => "The model isn't valid: check the objective, variable cells and constraints.",
            Status::IntegerWithinTolerance => {
                "Solver found an integer solution within the integer optimality tolerance. All constraints are satisfied."
            }
            Status::MaxFeasible => "Solver stopped after the maximum number of feasible integer solutions.",
            Status::MaxSubproblems => "Solver stopped after the maximum number of integer subproblems.",
            Status::NeedsBounds => "Every variable cell needs a lower and an upper bound for this method.",
        }
    }
}

/// Sensitivity of an LP solution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LpSensitivity {
    /// Per variable: reduced cost, allowable increase and decrease of its objective coefficient.
    pub reduced_cost: Vec<f64>,
    pub objective_coef: Vec<f64>,
    pub coef_increase: Vec<f64>,
    pub coef_decrease: Vec<f64>,
    /// Per constraint row: shadow price, allowable increase and decrease of its right side.
    pub shadow_price: Vec<f64>,
    pub rhs_increase: Vec<f64>,
    pub rhs_decrease: Vec<f64>,
}

/// Sensitivity of a nonlinear solution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NlpSensitivity {
    pub reduced_gradient: Vec<f64>,
    pub lagrange: Vec<f64>,
}

/// "Infinity" in sensitivity reports (as Excel prints it).
pub const BIG: f64 = 1e30;

/// A finished solve.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub status: Status,
    pub x: Vec<f64>,
    pub iterations: u64,
    pub subproblems: u64,
    pub lp: Option<LpSensitivity>,
    pub nlp: Option<NlpSensitivity>,
}

impl Outcome {
    pub fn new(status: Status, x: Vec<f64>) -> Outcome {
        Outcome { status, x, iterations: 0, subproblems: 0, lp: None, nlp: None }
    }
}

/// Where a solve is.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Still working: iteration count and the current objective.
    Running {
        iteration: u64,
        objective: f64,
        x: Vec<f64>,
    },
    Done(Box<Outcome>),
}

/// A solving method, advanced a slice at a time.
pub trait Engine {
    /// Works until about `evals` evaluations are spent (pure computation counts a little too).
    fn step(&mut self, p: &mut dyn Problem, evals: u64) -> Step;
    /// Stops early with the best point so far.
    fn stop(&mut self, status: Status) -> Outcome;
}

/// Total violation of the constraints at `e` beyond `precision`.
pub fn violation(rels: &[Rel], e: &Eval, precision: f64) -> f64 {
    rels.iter()
        .zip(&e.cons)
        .map(|(r, g)| {
            let v = match r {
                Rel::Le => g.max(0.0),
                Rel::Ge => (-g).max(0.0),
                Rel::Eq => g.abs(),
            };
            if v <= precision { 0.0 } else { v }
        })
        .sum()
}
