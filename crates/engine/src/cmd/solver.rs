//! Data › Analysis › Solver: the model (objective, variable cells, constraints, method and
//! options) and its persistence.
//!
//! A sheet's Solver model is kept the way Excel keeps it: hidden sheet-level defined names
//! (`solver_opt`, `solver_typ`, `solver_adj`, `solver_lhs1`, `solver_rel1`, `solver_rhs1`, …), so
//! models survive an xlsx round trip and open in Excel's Solver.

use gridcraft_model::{Cell, DefinedName, Workbook};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use super::*;

/// What to do with the objective.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Goal {
    #[default]
    Max,
    Min,
    /// Make the objective equal `value_of`.
    Value,
}

/// A constraint's relation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    #[default]
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = "=")]
    Eq,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "int")]
    Int,
    #[serde(rename = "bin")]
    Bin,
    #[serde(rename = "dif")]
    Dif,
}

impl Op {
    /// Excel's `solver_rel` code.
    fn code(self) -> u8 {
        match self {
            Op::Le => 1,
            Op::Eq => 2,
            Op::Ge => 3,
            Op::Int => 4,
            Op::Bin => 5,
            Op::Dif => 6,
        }
    }
    fn from_code(c: u8) -> Option<Op> {
        Some(match c {
            1 => Op::Le,
            2 => Op::Eq,
            3 => Op::Ge,
            4 => Op::Int,
            5 => Op::Bin,
            6 => Op::Dif,
            _ => return None,
        })
    }
    /// The right-hand side text for int / bin / dif constraints.
    pub fn keyword(self) -> Option<&'static str> {
        match self {
            Op::Int => Some("integer"),
            Op::Bin => Some("binary"),
            Op::Dif => Some("AllDifferent"),
            _ => None,
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Op::Le => "<=",
            Op::Eq => "=",
            Op::Ge => ">=",
            Op::Int => "int",
            Op::Bin => "bin",
            Op::Dif => "dif",
        }
    }
    pub fn parse(t: &str) -> Option<Op> {
        Some(match t.trim().to_ascii_lowercase().as_str() {
            "<=" | "le" | "≤" => Op::Le,
            "=" | "==" | "eq" => Op::Eq,
            ">=" | "ge" | "≥" => Op::Ge,
            "int" | "integer" => Op::Int,
            "bin" | "binary" => Op::Bin,
            "dif" | "alldifferent" => Op::Dif,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    /// Cell reference (`$B$5`, `$B$2:$B$4`).
    pub left: String,
    #[serde(rename = "op")]
    pub op: Op,
    /// A number, a cell reference or a formula; empty for int/bin/dif.
    #[serde(default)]
    pub right: String,
}

/// The solving method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    #[default]
    Grg,
    Simplex,
    Evolutionary,
}

/// Solver Options (all methods, GRG Nonlinear, Evolutionary).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SolverOptions {
    pub constraint_precision: f64,
    pub automatic_scaling: bool,
    pub show_iterations: bool,
    pub ignore_integer: bool,
    /// Integer optimality, in percent.
    pub integer_optimality: f64,
    /// Seconds; 0 = no limit.
    pub max_time: f64,
    /// 0 = no limit.
    pub iterations: u64,
    pub max_subproblems: u64,
    pub max_feasible: u64,
    pub convergence: f64,
    pub central_derivatives: bool,
    pub multistart: bool,
    pub population_size: u64,
    /// 0 = from the clock.
    pub random_seed: u64,
    pub require_bounds: bool,
    pub mutation_rate: f64,
    /// Seconds.
    pub max_time_without_improvement: f64,
    /// Evolutionary convergence.
    pub evolutionary_convergence: f64,
}

impl Default for SolverOptions {
    fn default() -> Self {
        SolverOptions {
            constraint_precision: 0.000_001,
            automatic_scaling: true,
            show_iterations: false,
            ignore_integer: false,
            integer_optimality: 1.0,
            max_time: 0.0,
            iterations: 0,
            max_subproblems: 0,
            max_feasible: 0,
            convergence: 0.0001,
            central_derivatives: false,
            multistart: false,
            population_size: 100,
            random_seed: 0,
            require_bounds: true,
            mutation_rate: 0.075,
            max_time_without_improvement: 30.0,
            evolutionary_convergence: 0.0001,
        }
    }
}

/// A sheet's Solver model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SolverModel {
    pub objective: String,
    pub goal: Goal,
    pub value_of: f64,
    pub variables: String,
    pub constraints: Vec<Constraint>,
    pub non_negative: bool,
    pub method: Method,
    pub options: SolverOptions,
}

impl Default for SolverModel {
    fn default() -> Self {
        SolverModel {
            objective: String::new(),
            goal: Goal::Max,
            value_of: 0.0,
            variables: String::new(),
            constraints: vec![],
            non_negative: true,
            method: Method::Grg,
            options: SolverOptions::default(),
        }
    }
}

/// Most variable cells (Excel's limit).
pub const MAX_VARIABLES: usize = 200;
/// Most constraints other than bounds and integer conditions (Excel's limit).
pub const MAX_CONSTRAINTS: usize = 100;

// ---------------------------------------------------------------- persistence

/// "No limit" as Excel stores it.
const UNLIMITED: f64 = 2_147_483_647.0;

fn name_value(wb: &Workbook, si: usize, name: &str) -> Option<String> {
    wb.names.iter().find(|n| n.scope == Some(si) && n.name.eq_ignore_ascii_case(name)).map(|n| n.formula.trim().trim_start_matches('=').to_string())
}

fn unquote(t: &str) -> String {
    let t = t.trim();
    t.strip_prefix('"').and_then(|x| x.strip_suffix('"')).map(|x| x.replace("\"\"", "\"")).unwrap_or_else(|| t.to_string())
}

/// `Sheet1!$B$5` → `$B$5` when it is on `sheet`.
fn local(text: &str, sheet: &str) -> String {
    let t = text.trim();
    let quoted = gridcraft_formula::quote_sheet(sheet);
    t.split(',')
        .map(|part| {
            let p = part.trim();
            p.strip_prefix(&format!("{quoted}!")).or_else(|| p.strip_prefix(&format!("{sheet}!"))).unwrap_or(p).to_string()
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `$B$5` → `Sheet1!$B$5` (each area of a union).
fn qualified(text: &str, sheet: &str) -> String {
    let quoted = gridcraft_formula::quote_sheet(sheet);
    text.split(',')
        .map(|part| {
            let p = part.trim();
            if p.contains('!') || p.is_empty() { p.to_string() } else { format!("{quoted}!{p}") }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Whether `t` reads as a reference (rather than a number or formula).
fn is_reference(t: &str) -> bool {
    let body = t.rsplit('!').next().unwrap_or(t);
    !body.is_empty() && body.split(',').all(|a| gridcraft_core::RangeRef::parse(a.trim()).is_some())
}

/// The model stored on sheet `si` (defaults when there is none).
pub fn load_model(wb: &Workbook, si: usize) -> SolverModel {
    let sheet = wb.sheet(si).map(|s| s.name.clone()).unwrap_or_default();
    let get = |n: &str| name_value(wb, si, n);
    let num = |n: &str| get(n).and_then(|v| unquote(&v).trim().parse::<f64>().ok());
    let flag = |n: &str, default: bool| num(n).map(|v| v == 1.0).unwrap_or(default);
    let limit = |n: &str| num(n).map(|v| if v >= UNLIMITED { 0 } else { v.max(0.0) as u64 }).unwrap_or(0);
    let count = num("solver_num").unwrap_or(0.0).clamp(0.0, 1000.0) as usize;
    let mut constraints = Vec::with_capacity(count);
    for i in 1..=count {
        let (Some(l), Some(op)) = (get(&format!("solver_lhs{i}")), num(&format!("solver_rel{i}")).and_then(|c| Op::from_code(c as u8))) else {
            continue;
        };
        let right = if op.keyword().is_some() {
            String::new()
        } else {
            get(&format!("solver_rhs{i}")).map(|r| if is_reference(&r) { local(&r, &sheet) } else { unquote(&r) }).unwrap_or_default()
        };
        constraints.push(Constraint { left: local(&l, &sheet), op, right });
    }
    let d = SolverOptions::default();
    let options = SolverOptions {
        constraint_precision: num("solver_pre").unwrap_or(d.constraint_precision),
        automatic_scaling: flag("solver_scl", d.automatic_scaling),
        show_iterations: flag("solver_sho", d.show_iterations),
        ignore_integer: flag("solver_rlx", d.ignore_integer),
        integer_optimality: num("solver_tol").map(|v| v * 100.0).unwrap_or(d.integer_optimality),
        max_time: num("solver_tim").map(|v| if v >= UNLIMITED { 0.0 } else { v }).unwrap_or(0.0),
        iterations: limit("solver_itr"),
        max_subproblems: limit("solver_mip"),
        max_feasible: limit("solver_nod"),
        convergence: num("solver_cvg").unwrap_or(d.convergence),
        central_derivatives: num("solver_drv").is_some_and(|v| v == 2.0),
        multistart: flag("solver_msl", d.multistart),
        population_size: num("solver_ssz").map(|v| v.max(1.0) as u64).unwrap_or(d.population_size),
        random_seed: num("solver_rsd").map(|v| v.max(0.0) as u64).unwrap_or(0),
        require_bounds: flag("solver_rbv", d.require_bounds),
        mutation_rate: num("solver_mrt").unwrap_or(d.mutation_rate),
        max_time_without_improvement: num("solver_mni").unwrap_or(d.max_time_without_improvement),
        evolutionary_convergence: num("solver_cvg_evo").unwrap_or(d.evolutionary_convergence),
    };
    SolverModel {
        objective: get("solver_opt").map(|v| local(&v, &sheet)).unwrap_or_default(),
        goal: match num("solver_typ").map(|v| v as i64) {
            Some(2) => Goal::Min,
            Some(3) => Goal::Value,
            _ => Goal::Max,
        },
        value_of: num("solver_val").unwrap_or(0.0),
        variables: get("solver_adj").map(|v| local(&v, &sheet)).unwrap_or_default(),
        constraints,
        non_negative: flag("solver_neg", true),
        method: match num("solver_eng").map(|v| v as i64) {
            Some(2) => Method::Simplex,
            Some(3) => Method::Evolutionary,
            _ => Method::Grg,
        },
        options,
    }
}

/// Whether sheet `si` has a stored model.
pub fn has_model(wb: &Workbook, si: usize) -> bool {
    wb.names.iter().any(|n| n.scope == Some(si) && n.name.to_ascii_lowercase().starts_with("solver_"))
}

/// Replaces the model stored on sheet `si`.
pub fn store_model(wb: &mut Workbook, si: usize, m: &SolverModel) {
    let sheet = wb.sheet(si).map(|s| s.name.clone()).unwrap_or_default();
    wb.names.retain(|n| !(n.scope == Some(si) && n.name.to_ascii_lowercase().starts_with("solver_")));
    let mut put = |name: &str, formula: String| {
        wb.names.push(DefinedName { name: name.into(), scope: Some(si), formula, comment: String::new(), hidden: true });
    };
    let num = |x: f64| gridcraft_core::number_to_text(x);
    let flag = |b: bool| if b { "1".to_string() } else { "2".to_string() };
    let limit = |v: u64| if v == 0 { num(UNLIMITED) } else { v.to_string() };
    if !m.objective.trim().is_empty() {
        put("solver_opt", qualified(&m.objective, &sheet));
    }
    put(
        "solver_typ",
        match m.goal {
            Goal::Max => "1",
            Goal::Min => "2",
            Goal::Value => "3",
        }
        .into(),
    );
    put("solver_val", num(m.value_of));
    if !m.variables.trim().is_empty() {
        put("solver_adj", qualified(&m.variables, &sheet));
    }
    put("solver_num", m.constraints.len().to_string());
    for (k, c) in m.constraints.iter().enumerate() {
        let i = k + 1;
        put(&format!("solver_lhs{i}"), qualified(&c.left, &sheet));
        put(&format!("solver_rel{i}"), c.op.code().to_string());
        let rhs = match c.op.keyword() {
            Some(kw) => format!("\"{kw}\""),
            None if is_reference(&c.right) => qualified(&c.right, &sheet),
            None => c.right.trim().to_string(),
        };
        put(&format!("solver_rhs{i}"), rhs);
    }
    put("solver_neg", flag(m.non_negative));
    put(
        "solver_eng",
        match m.method {
            Method::Grg => "1",
            Method::Simplex => "2",
            Method::Evolutionary => "3",
        }
        .into(),
    );
    put("solver_lin", flag(m.method == Method::Simplex));
    let o = &m.options;
    put("solver_pre", num(o.constraint_precision));
    put("solver_scl", flag(o.automatic_scaling));
    put("solver_sho", flag(o.show_iterations));
    put("solver_rlx", flag(o.ignore_integer));
    put("solver_tol", num(o.integer_optimality / 100.0));
    put("solver_tim", if o.max_time > 0.0 { num(o.max_time) } else { num(UNLIMITED) });
    put("solver_itr", limit(o.iterations));
    put("solver_mip", limit(o.max_subproblems));
    put("solver_nod", limit(o.max_feasible));
    put("solver_cvg", num(o.convergence));
    put("solver_drv", if o.central_derivatives { "2".into() } else { "1".into() });
    put("solver_msl", flag(o.multistart));
    put("solver_ssz", o.population_size.to_string());
    put("solver_rsd", o.random_seed.to_string());
    put("solver_rbv", flag(o.require_bounds));
    put("solver_mrt", num(o.mutation_rate));
    put("solver_mni", num(o.max_time_without_improvement));
    put("solver_ver", "3".into());
}

/// Removes the model stored on sheet `si`.
pub fn clear_model(wb: &mut Workbook, si: usize) {
    wb.names.retain(|n| !(n.scope == Some(si) && n.name.to_ascii_lowercase().starts_with("solver_")));
}

// ---------------------------------------------------------------- commands

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "data.solverModel",
            "Solver Model",
            [],
            None,
            "{sheet?} → the sheet's Solver model {objective, goal: max|min|value, valueOf, variables, constraints: [{left, op: <=|=|>=|int|bin|dif, right}], nonNegative, method: grg|simplex|evolutionary, options}",
            has_doc,
            model_get
        ),
        cmd!(
            "data.solverSetModel",
            "Set Solver Model",
            [],
            None,
            "{objective?, goal?, valueOf?, variables?, constraints?, nonNegative?, method?, options?: {...}} updates the active sheet's Solver model (fields left out keep their values)",
            has_doc,
            model_set
        ),
        cmd!("data.solverReset", "Reset All", [], None, "{} clears the active sheet's Solver model", has_doc, model_reset),
        cmd!(
            "data.solverSaveModel",
            "Save Model",
            [],
            None,
            "{to: \"H1\"} writes the model into cells from `to` down: the objective, the variable cells, one row per constraint and the options",
            has_doc,
            model_save
        ),
        cmd!("data.solverLoadModel", "Load Model", [], None, "{from: \"H1:H6\"} reads a model saved with Save Model", has_doc, model_load),
        cmd!(
            "data.solver",
            "Solver",
            ["Data", "Analysis"],
            None,
            "{objective?: \"$D$10\", goal?: max|min|value, valueOf?, variables?: \"$B$2:$B$5\", constraints?: [{left, op: <=|=|>=|int|bin|dif, right}], nonNegative?, method?: grg|simplex|evolutionary, options?: {maxTime (s), iterations, constraintPrecision, integerOptimality (%), convergence, centralDerivatives, multistart, populationSize, randomSeed, requireBounds, mutationRate, maxTimeWithoutImprovement, ignoreInteger, automaticScaling}, keep?: true} → solves the active sheet's model (fields given update it) and returns {status (Excel's code), message, objective, variables, constraints, sensitivity}; keeps the solution unless `keep: false`",
            has_doc,
            solve
        ),
        cmd!(noundo "data.solverStart", "Solve", [], None, "{model fields?} starts solving in the background (the Solver Results box follows); `job.status` / `job.stop` follow it", has_doc, start),
        cmd!(noundo "data.solverRestore", "Restore Original Values", [], None, "{} puts back the variable cells' values from before the last solve", has_doc, restore),
    ]
}

fn sheet_of(s: &Session, p: &Json) -> Result<usize> {
    if p.get("sheet").is_some() { target_sheet(s, p) } else { Ok(s.doc()?.wb.active_sheet) }
}

fn model_get(s: &mut Session, p: &Json) -> Result<Json> {
    let si = sheet_of(s, p)?;
    let m = load_model(&s.doc()?.wb, si);
    serde_json::to_value(&m).map_err(|e| EngineError::Other(e.to_string()))
}

/// Applies the model fields present in `p` to `m`, checking them.
pub(crate) fn merge_model(m: &mut SolverModel, p: &Json, cmd: &str) -> Result<()> {
    if let Some(t) = str_param(p, "objective") {
        m.objective = t.trim().to_string();
    }
    if let Some(t) = str_param(p, "goal") {
        m.goal = match t.to_ascii_lowercase().as_str() {
            "max" => Goal::Max,
            "min" => Goal::Min,
            "value" | "valueof" => Goal::Value,
            other => return Err(bad(cmd, format!("unknown goal `{other}` (max, min or value)"))),
        };
    }
    if let Some(v) = p.get("valueOf").and_then(Json::as_f64) {
        m.value_of = v;
    }
    if let Some(t) = str_param(p, "variables") {
        m.variables = t.trim().to_string();
    }
    if let Some(list) = p.get("constraints") {
        let arr = list.as_array().ok_or_else(|| bad(cmd, "`constraints` must be a list"))?;
        let mut cs = Vec::with_capacity(arr.len());
        for c in arr {
            let left = c
                .get("left")
                .and_then(Json::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| bad(cmd, "each constraint needs `left`"))?;
            let op = c
                .get("op")
                .and_then(Json::as_str)
                .and_then(Op::parse)
                .ok_or_else(|| bad(cmd, "each constraint needs `op`: <=, =, >=, int, bin or dif"))?;
            let right = match c.get("right") {
                Some(Json::Number(n)) => n.to_string(),
                Some(Json::String(t)) => t.trim().to_string(),
                _ => String::new(),
            };
            if op.keyword().is_none() && right.is_empty() {
                return Err(bad(cmd, format!("constraint on `{left}` needs `right`")));
            }
            cs.push(Constraint { left: left.to_string(), op, right });
        }
        if cs.len() > 1000 {
            return Err(bad(cmd, "too many constraints"));
        }
        m.constraints = cs;
    }
    if let Some(b) = bool_param(p, "nonNegative") {
        m.non_negative = b;
    }
    if let Some(t) = str_param(p, "method").or_else(|| str_param(p, "engine")) {
        m.method = match t.to_ascii_lowercase().as_str() {
            "grg" | "grgnonlinear" | "nonlinear" => Method::Grg,
            "simplex" | "simplexlp" | "lp" => Method::Simplex,
            "evolutionary" => Method::Evolutionary,
            other => return Err(bad(cmd, format!("unknown method `{other}` (grg, simplex or evolutionary)"))),
        };
    }
    if let Some(o) = p.get("options") {
        let mut cur = serde_json::to_value(&m.options).map_err(|e| EngineError::Other(e.to_string()))?;
        if let (Some(dst), Some(src)) = (cur.as_object_mut(), o.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        m.options = serde_json::from_value(cur).map_err(|e| bad(cmd, format!("bad `options`: {e}")))?;
    }
    Ok(())
}

fn model_set(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "data.solverSetModel";
    let si = s.doc()?.wb.active_sheet;
    let mut m = load_model(&s.doc()?.wb, si);
    merge_model(&mut m, p, CMD)?;
    edit(s, |cx| {
        store_model(&mut cx.wb, si, &m);
        Ok(())
    })?;
    serde_json::to_value(&m).map_err(|e| EngineError::Other(e.to_string()))
}

fn model_reset(s: &mut Session, _p: &Json) -> Result<Json> {
    let si = s.doc()?.wb.active_sheet;
    edit(s, |cx| {
        clear_model(&mut cx.wb, si);
        Ok(())
    })?;
    Ok(json!({}))
}

/// The cells a model is saved as: objective, variables, constraints, then options.
fn saved_rows(m: &SolverModel) -> Vec<String> {
    let mut rows = Vec::new();
    rows.push(match m.goal {
        Goal::Max => format!("=MAX({})", m.objective),
        Goal::Min => format!("=MIN({})", m.objective),
        Goal::Value => format!("={}={}", m.objective, gridcraft_core::number_to_text(m.value_of)),
    });
    rows.push(format!("=COUNT({})", m.variables));
    for c in &m.constraints {
        rows.push(match c.op.keyword() {
            Some(kw) => format!("={}=\"{kw}\"", c.left),
            None => format!("={}{}{}", c.left, c.op.symbol(), c.right),
        });
    }
    let o = &m.options;
    let method = match m.method {
        Method::Grg => 1,
        Method::Simplex => 2,
        Method::Evolutionary => 3,
    };
    let b = |x: bool| if x { "1" } else { "2" };
    let n = gridcraft_core::number_to_text;
    rows.push(format!(
        "={{{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}}}",
        n(o.max_time),
        o.iterations,
        n(o.constraint_precision),
        n(o.integer_optimality),
        b(o.automatic_scaling),
        b(o.show_iterations),
        b(o.ignore_integer),
        b(m.non_negative),
        method,
        n(o.convergence),
        b(o.central_derivatives),
        b(o.multistart),
        o.population_size,
        o.random_seed,
        b(o.require_bounds),
        n(o.mutation_rate),
        n(o.max_time_without_improvement),
        o.max_subproblems,
        o.max_feasible,
    ));
    rows
}

fn model_save(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "data.solverSaveModel";
    let si = s.doc()?.wb.active_sheet;
    let m = load_model(&s.doc()?.wb, si);
    if m.objective.is_empty() && m.variables.is_empty() {
        return Err(EngineError::Other("There is no Solver model on this sheet to save.".into()));
    }
    let to = cell_param(p, "to").ok_or_else(|| bad(CMD, "missing `to` (the first cell of the saved model)"))?;
    let rows = saved_rows(&m);
    let n = rows.len() as u32;
    edit(s, |cx| {
        for (i, f) in rows.iter().enumerate() {
            let c = CellRef::new(to.row.saturating_add(i as u32), to.col);
            let style = cx.sheet_mut(si)?.style_id(c);
            let cell = Cell { value: gridcraft_core::Value::Empty, formula: Some(std::sync::Arc::new(gridcraft_model::Formula::new(f))), style };
            cx.sheet_mut(si)?.set_cell(c, cell);
            cx.touch(si, c);
        }
        Ok(())
    })?;
    Ok(json!({"range": RangeRef::new(to, CellRef::new(to.row + n - 1, to.col)).a1()}))
}

/// Reads a model saved by [`model_save`] from formula text rows.
fn parse_saved(rows: &[String], cmd: &str) -> Result<SolverModel> {
    let bad_model = || bad(cmd, "the range doesn't hold a saved Solver model");
    let body = |t: &str| t.trim().trim_start_matches('=').to_string();
    let first = rows.first().map(|t| body(t)).ok_or_else(bad_model)?;
    let mut m = SolverModel::default();
    let inner = |t: &str, f: &str| t.strip_prefix(f).and_then(|x| x.strip_prefix('(')).and_then(|x| x.strip_suffix(')')).map(str::to_string);
    if let Some(o) = inner(&first.to_ascii_uppercase(), "MAX") {
        m.goal = Goal::Max;
        m.objective = first.get(4..4 + o.len()).unwrap_or("").to_string();
    } else if let Some(o) = inner(&first.to_ascii_uppercase(), "MIN") {
        m.goal = Goal::Min;
        m.objective = first.get(4..4 + o.len()).unwrap_or("").to_string();
    } else if let Some((a, b)) = first.split_once('=') {
        m.goal = Goal::Value;
        m.objective = a.trim().to_string();
        m.value_of = b.trim().parse().map_err(|_| bad_model())?;
    } else {
        return Err(bad_model());
    }
    let second = rows.get(1).map(|t| body(t)).ok_or_else(bad_model)?;
    m.variables = inner(&second.to_ascii_uppercase(), "COUNT").map(|v| second.get(6..6 + v.len()).unwrap_or("").to_string()).ok_or_else(bad_model)?;
    let rest = rows.get(2..).unwrap_or(&[]);
    for (i, r) in rest.iter().enumerate() {
        let t = body(r);
        if t.starts_with('{') {
            // Options: the last row.
            let vals: Vec<f64> = t.trim_matches(|c| c == '{' || c == '}').split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect();
            let g = |k: usize| vals.get(k).copied();
            let o = &mut m.options;
            let on = |v: Option<f64>, d: bool| v.map(|x| x == 1.0).unwrap_or(d);
            o.max_time = g(0).unwrap_or(0.0);
            o.iterations = g(1).unwrap_or(0.0) as u64;
            o.constraint_precision = g(2).unwrap_or(o.constraint_precision);
            o.integer_optimality = g(3).unwrap_or(o.integer_optimality);
            o.automatic_scaling = on(g(4), true);
            o.show_iterations = on(g(5), false);
            o.ignore_integer = on(g(6), false);
            m.non_negative = on(g(7), true);
            m.method = match g(8).map(|x| x as i64) {
                Some(2) => Method::Simplex,
                Some(3) => Method::Evolutionary,
                _ => Method::Grg,
            };
            let o = &mut m.options;
            o.convergence = g(9).unwrap_or(o.convergence);
            o.central_derivatives = g(10) == Some(1.0);
            o.multistart = g(11) == Some(1.0);
            o.population_size = g(12).map(|x| x as u64).unwrap_or(o.population_size);
            o.random_seed = g(13).map(|x| x as u64).unwrap_or(0);
            o.require_bounds = g(14).map(|x| x == 1.0).unwrap_or(true);
            o.mutation_rate = g(15).unwrap_or(o.mutation_rate);
            o.max_time_without_improvement = g(16).unwrap_or(o.max_time_without_improvement);
            o.max_subproblems = g(17).map(|x| x as u64).unwrap_or(0);
            o.max_feasible = g(18).map(|x| x as u64).unwrap_or(0);
            if i + 1 != rest.len() {
                return Err(bad_model());
            }
            break;
        }
        let (left, op, right) = ["<=", ">=", "="]
            .iter()
            .find_map(|sym| t.split_once(sym).map(|(a, b)| (a.trim().to_string(), Op::parse(sym), b.trim().to_string())))
            .ok_or_else(bad_model)?;
        let op = op.ok_or_else(bad_model)?;
        let (op, right) = match unquote(&right).to_ascii_lowercase().as_str() {
            "integer" => (Op::Int, String::new()),
            "binary" => (Op::Bin, String::new()),
            "alldifferent" => (Op::Dif, String::new()),
            _ => (op, right),
        };
        m.constraints.push(Constraint { left, op, right });
    }
    Ok(m)
}

fn model_load(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "data.solverLoadModel";
    let si = s.doc()?.wb.active_sheet;
    let text = str_param(p, "from").ok_or_else(|| bad(CMD, "missing `from` (the saved model's range)"))?;
    let r = RangeRef::parse(split_sheet(text).1.trim()).ok_or_else(|| bad(CMD, format!("not a range: `{text}`")))?;
    if r.count() > 2000 {
        return Err(bad(CMD, "the range is too large for a saved model"));
    }
    let rows: Vec<String> = {
        let sh = s.doc()?.wb.sheet(si).ok_or(EngineError::NoDocument)?;
        r.iter().filter_map(|c| sh.cell(c).and_then(|x| x.formula.as_ref()).map(|f| f.text.clone())).collect()
    };
    let m = parse_saved(&rows, CMD)?;
    edit(s, |cx| {
        store_model(&mut cx.wb, si, &m);
        Ok(())
    })?;
    serde_json::to_value(&m).map_err(|e| EngineError::Other(e.to_string()))
}

// ---------------------------------------------------------------- solving

use gridcraft_analysis::solver::{self as sv, Rel, Sense, Spec, Status};
use gridcraft_calc::Trial;
use gridcraft_core::Value;

/// A constraint's right side, per cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Rhs {
    Const(f64),
    Cell(usize, CellRef),
}

/// One cell's share of a user constraint (a constraint over a range covers several cells).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ItemKind {
    /// A general constraint: row `row` of the problem.
    Row {
        row: usize,
        rel: Rel,
        rhs: Rhs,
    },
    /// A bound on variable `var`.
    Bound {
        var: usize,
        rel: Rel,
        value: f64,
    },
    Integer {
        var: usize,
    },
    Binary {
        var: usize,
    },
    AllDiff,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    /// The user's constraint this comes from.
    pub user: usize,
    pub cell: CellRef,
    pub kind: ItemKind,
}

/// A model resolved against the workbook, ready to solve.
pub(crate) struct Prepared {
    pub sheet: usize,
    pub objective: CellRef,
    pub vars: Vec<CellRef>,
    pub items: Vec<Item>,
    /// General rows: left cell and right side.
    pub rows: Vec<(CellRef, Rhs)>,
    pub spec: Spec,
}

fn abs_a1(c: CellRef) -> String {
    format!("${}${}", gridcraft_core::col_to_letters(c.col), c.row + 1)
}

/// Cells of a reference on the model's sheet (a union of areas; other sheets are refused).
fn cells_on(wb: &Workbook, si: usize, text: &str, what: &str) -> std::result::Result<Vec<CellRef>, String> {
    let sheet = wb.sheet(si).map(|s| s.name.clone()).unwrap_or_default();
    let mut out = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (sh, body) = split_sheet(part);
        if sh.as_deref().is_some_and(|n| !n.eq_ignore_ascii_case(&sheet)) {
            return Err(format!("{what} must be on the sheet the model belongs to."));
        }
        let r = gridcraft_core::RangeRef::parse(body.trim()).ok_or_else(|| format!("{what} isn't a valid reference: {part}"))?;
        if r.count() > 10_000 {
            return Err(format!("{what} covers too many cells."));
        }
        for c in r.iter() {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    if out.is_empty() {
        return Err(format!("{what} is missing."));
    }
    Ok(out)
}

/// The right side of a relational constraint, per cell.
fn rhs_of(wb: &Workbook, si: usize, text: &str) -> std::result::Result<Vec<Rhs>, String> {
    let t = text.trim().trim_start_matches('=');
    if let Ok(x) = t.parse::<f64>() {
        return Ok(vec![Rhs::Const(x)]);
    }
    if is_reference(t) {
        let mut out = Vec::new();
        for part in t.split(',') {
            let (sh, body) = split_sheet(part.trim());
            let s2 = match sh {
                Some(n) => wb.sheet_index(&n).ok_or_else(|| format!("No sheet named {n}."))?,
                None => si,
            };
            let r = gridcraft_core::RangeRef::parse(body.trim()).ok_or_else(|| format!("Not a valid reference: {part}"))?;
            if r.count() > 10_000 {
                return Err("A constraint covers too many cells.".into());
            }
            out.extend(r.iter().map(|c| Rhs::Cell(s2, c)));
        }
        return Ok(out);
    }
    // A formula: evaluated once.
    match gridcraft_calc::evaluate(wb, si, CellRef::new(0, 0), t) {
        Value::Number(x) if x.is_finite() => Ok(vec![Rhs::Const(x)]),
        _ => Err(format!("The constraint value {text} isn't a number, a reference or a formula giving a number.")),
    }
}

/// Resolves `m` on sheet `si`.
pub(crate) fn prepare(wb: &Workbook, si: usize, m: &SolverModel) -> std::result::Result<Prepared, (Status, String)> {
    let bad = |msg: String| (Status::BadModel, msg);
    let sh = wb.sheet(si).ok_or_else(|| bad("No such sheet.".into()))?;
    let obj = cells_on(wb, si, &m.objective, "The objective").map_err(bad)?;
    let [objective] = obj.as_slice() else { return Err(bad("The objective must be a single cell.".into())) };
    let objective = *objective;
    let vars = cells_on(wb, si, &m.variables, "The variable cells").map_err(bad)?;
    if vars.len() > MAX_VARIABLES {
        return Err((Status::TooLarge, format!("Solver handles at most {MAX_VARIABLES} variable cells.")));
    }
    if vars.iter().any(|c| sh.cell(*c).is_some_and(|x| x.formula.is_some())) {
        return Err(bad("Variable cells must hold values, not formulas.".into()));
    }
    if vars.contains(&objective) {
        return Err(bad("The objective can't also be a variable cell.".into()));
    }
    let x0: Vec<f64> = vars.iter().map(|c| sh.value(*c).as_f64().filter(|v| v.is_finite()).unwrap_or(0.0)).collect();
    let n = vars.len();
    let mut lower = vec![f64::NEG_INFINITY; n];
    let mut upper = vec![f64::INFINITY; n];
    let mut has_lower = vec![false; n];
    let mut integer = vec![false; n];
    let mut alldiff: Vec<Vec<usize>> = Vec::new();
    let mut items = Vec::new();
    let mut rows: Vec<(CellRef, Rhs)> = Vec::new();
    let mut rels = Vec::new();
    for (u, c) in m.constraints.iter().enumerate() {
        let left = cells_on(wb, si, &c.left, "A constraint's cell reference").map_err(bad)?;
        let var_of = |cell: &CellRef| vars.iter().position(|v| v == cell);
        if c.op.keyword().is_some() {
            let mut group = Vec::new();
            for cell in &left {
                let j = var_of(cell).ok_or_else(|| bad(format!("{} must be variable cells for an int, bin or dif constraint.", c.left)))?;
                match c.op {
                    Op::Int => {
                        integer[j] = true;
                        items.push(Item { user: u, cell: *cell, kind: ItemKind::Integer { var: j } });
                    }
                    Op::Bin => {
                        integer[j] = true;
                        lower[j] = lower[j].max(0.0);
                        upper[j] = upper[j].min(1.0);
                        has_lower[j] = true;
                        items.push(Item { user: u, cell: *cell, kind: ItemKind::Binary { var: j } });
                    }
                    _ => group.push(j),
                }
            }
            if c.op == Op::Dif {
                if group.len() < 2 {
                    return Err(bad("An AllDifferent constraint needs at least two variable cells.".into()));
                }
                for &j in &group {
                    has_lower[j] = true;
                }
                items.push(Item { user: u, cell: left[0], kind: ItemKind::AllDiff });
                alldiff.push(group);
            }
            continue;
        }
        let rel = match c.op {
            Op::Le => Rel::Le,
            Op::Ge => Rel::Ge,
            _ => Rel::Eq,
        };
        let rhs = rhs_of(wb, si, &c.right).map_err(bad)?;
        if rhs.len() != 1 && rhs.len() != left.len() {
            return Err(bad(format!("The two sides of the constraint on {} have different sizes.", c.left)));
        }
        for (k, cell) in left.iter().enumerate() {
            let r = if rhs.len() == 1 { rhs[0] } else { rhs[k] };
            match (var_of(cell), r) {
                (Some(j), Rhs::Const(v)) => {
                    if matches!(rel, Rel::Ge | Rel::Eq) {
                        lower[j] = lower[j].max(v);
                        has_lower[j] = true;
                    }
                    if matches!(rel, Rel::Le | Rel::Eq) {
                        upper[j] = upper[j].min(v);
                    }
                    items.push(Item { user: u, cell: *cell, kind: ItemKind::Bound { var: j, rel, value: v } });
                }
                _ => {
                    items.push(Item { user: u, cell: *cell, kind: ItemKind::Row { row: rows.len(), rel, rhs: r } });
                    rows.push((*cell, r));
                    rels.push(rel);
                }
            }
        }
    }
    if m.method != Method::Simplex && rows.len() > MAX_CONSTRAINTS {
        return Err((Status::TooLarge, format!("This method handles at most {MAX_CONSTRAINTS} constraints besides bounds.")));
    }
    if m.non_negative {
        for j in 0..n {
            if !has_lower[j] {
                lower[j] = lower[j].max(0.0);
            }
        }
    }
    let o = &m.options;
    let sense = match m.goal {
        Goal::Max => Sense::Max,
        Goal::Min => Sense::Min,
        Goal::Value => Sense::Target(m.value_of),
    };
    let options = sv::Options {
        precision: o.constraint_precision.max(1e-12),
        scaling: o.automatic_scaling,
        ignore_integer: o.ignore_integer,
        integer_tolerance: (o.integer_optimality / 100.0).max(0.0),
        max_iterations: o.iterations,
        max_subproblems: o.max_subproblems,
        max_feasible: o.max_feasible,
        convergence: o.convergence.max(1e-15),
        central: o.central_derivatives,
        multistart: o.multistart,
        population: o.population_size.clamp(2, 10_000) as usize,
        seed: o.random_seed,
        require_bounds: o.require_bounds,
        mutation_rate: o.mutation_rate.clamp(0.0, 1.0),
        patience: 20_000,
    };
    let spec = Spec { sense, x0, lower, upper, integer, alldiff, rels, options };
    Ok(Prepared { sheet: si, objective, vars, items, rows, spec })
}

/// A number from a cell for Solver: blank is 0; text and errors are errors.
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(x) if x.is_finite() => Some(*x),
        Value::Empty => Some(0.0),
        _ => None,
    }
}

/// The model evaluated by recalculating the workbook with the variable cells replaced.
#[derive(Clone)]
pub(crate) struct CellProblem {
    trial: Trial,
    sheet: usize,
    vars: Vec<CellRef>,
    rows: Vec<(CellRef, Rhs)>,
    reads: Vec<(usize, CellRef)>,
    /// Where each row's right-hand cell is in `reads`.
    rhs_at: Vec<Option<usize>>,
    pub evaluations: u64,
}

impl CellProblem {
    pub(crate) fn new(d: &DocState, p: &Prepared) -> CellProblem {
        let mut reads = vec![(p.sheet, p.objective)];
        reads.extend(p.rows.iter().map(|(c, _)| (p.sheet, *c)));
        let mut rhs_at = Vec::with_capacity(p.rows.len());
        for (_, r) in &p.rows {
            match r {
                Rhs::Cell(s, c) => {
                    rhs_at.push(Some(reads.len()));
                    reads.push((*s, *c));
                }
                Rhs::Const(_) => rhs_at.push(None),
            }
        }
        CellProblem { trial: Trial::new(&d.wb, &d.calc), sheet: p.sheet, vars: p.vars.clone(), rows: p.rows.clone(), reads, rhs_at, evaluations: 0 }
    }
}

impl sv::Problem for CellProblem {
    fn eval(&mut self, x: &[f64]) -> Option<sv::Eval> {
        self.evaluations += 1;
        let sets: Vec<(usize, CellRef, Value)> = self.vars.iter().zip(x).map(|(c, v)| (self.sheet, *c, Value::Number(*v))).collect();
        let vals = self.trial.run(&sets, &self.reads);
        let objective = number(vals.first()?)?;
        let mut cons = Vec::with_capacity(self.rows.len());
        for (i, (_, r)) in self.rows.iter().enumerate() {
            let lhs = number(vals.get(1 + i)?)?;
            let rhs = match (r, self.rhs_at.get(i).copied().flatten()) {
                (Rhs::Const(v), _) => *v,
                (Rhs::Cell(..), Some(k)) => number(vals.get(k)?)?,
                _ => return None,
            };
            cons.push(lhs - rhs);
        }
        Some(sv::Eval { objective, cons })
    }

    /// Several points at once: in parallel on the desktop (each worker with its own copy of the
    /// trial workbook), one after another in the browser.
    fn eval_many(&mut self, xs: &[Vec<f64>]) -> Vec<Option<sv::Eval>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            let workers = rayon::current_num_threads().clamp(1, 16);
            if xs.len() >= 8 && workers > 1 {
                let size = xs.len().div_ceil(workers).max(1);
                let base: &CellProblem = self;
                let out: Vec<Option<sv::Eval>> = xs
                    .par_chunks(size)
                    .flat_map_iter(|chunk| {
                        let mut worker = base.clone();
                        chunk.iter().map(|x| sv::Problem::eval(&mut worker, x)).collect::<Vec<_>>()
                    })
                    .collect();
                self.evaluations += xs.len() as u64;
                return out;
            }
        }
        xs.iter().map(|x| sv::Problem::eval(self, x)).collect()
    }
}

/// The engine for a method.
fn engine_for(m: &SolverModel, spec: Spec) -> std::result::Result<Box<dyn sv::Engine>, String> {
    match m.method {
        Method::Simplex => Ok(Box::new(sv::simplex::Simplex::new(spec))),
        Method::Grg => Ok(Box::new(sv::grg::Grg::new(spec))),
        Method::Evolutionary => Ok(Box::new(sv::evolutionary::Evolutionary::new(spec))),
    }
}

/// A Solver run as a job.
pub(crate) struct SolverJob {
    doc: u64,
    model: SolverModel,
    prepared: Prepared,
    problem: CellProblem,
    engine: Box<dyn sv::Engine>,
    started: f64,
    /// 0 = no limit.
    max_ms: f64,
    keep: bool,
    /// Started from the dialog: the solution is one undo step and the results dialog follows.
    interactive: bool,
    /// Evolutionary: best objective seen and when it last improved.
    best_seen: f64,
    improved_at: f64,
    /// The workbook before solving (Restore and undo go back to it).
    start_wb: std::sync::Arc<Workbook>,
    /// Trial solutions shown on the sheet (Show Iteration Results).
    wrote_trial: bool,
    last_shown: u64,
}

impl SolverJob {
    pub(crate) fn new(s: &Session, m: SolverModel, keep: bool, interactive: bool) -> Result<SolverJob> {
        let d = s.doc()?;
        let si = d.wb.active_sheet;
        let prepared = prepare(&d.wb, si, &m).map_err(|(_, msg)| EngineError::Other(msg))?;
        let engine = engine_for(&m, prepared.spec.clone()).map_err(EngineError::Other)?;
        let problem = CellProblem::new(d, &prepared);
        let max_ms = if m.options.max_time > 0.0 { m.options.max_time * 1000.0 } else { 0.0 };
        let now = crate::clock::now_ms();
        Ok(SolverJob {
            doc: d.uid,
            model: m,
            prepared,
            problem,
            engine,
            started: now,
            max_ms,
            keep,
            interactive,
            best_seen: f64::NAN,
            improved_at: now,
            start_wb: d.wb.clone(),
            wrote_trial: false,
            last_shown: 0,
        })
    }

    /// Puts `x` into the variable cells (no undo step of its own).
    fn write(&self, s: &mut Session, x: &[f64]) -> Result<()> {
        let Some(d) = s.doc_by_uid_mut(self.doc) else { return Err(EngineError::NoDocument) };
        let (si, vars) = (self.prepared.sheet, self.prepared.vars.clone());
        commit(d, |cx| {
            for (c, v) in vars.iter().zip(x) {
                let sh = cx.sheet_mut(si)?;
                let style = sh.style_id(*c);
                sh.set_cell(*c, Cell { value: Value::Number(*v), formula: None, style });
                cx.touch(si, *c);
            }
            Ok(())
        })
    }

    fn finish(&mut self, s: &mut Session, o: sv::Outcome) -> Result<Json> {
        let seconds = ((crate::clock::now_ms() - self.started) / 1000.0).max(0.0);
        let p = &self.prepared;
        let x = if o.x.len() == p.vars.len() { o.x.clone() } else { p.spec.x0.clone() };
        let apply = self.keep
            && (o.status.feasible()
                || matches!(o.status, Status::Stopped | Status::MaxTime | Status::MaxIterations | Status::MaxFeasible | Status::MaxSubproblems));
        let final_eval = sv::Problem::eval(&mut self.problem, &x);
        let at = self.problem.values_at(&x);
        if apply {
            self.write(s, &x)?;
            if self.interactive
                && let Some(d) = s.doc_by_uid_mut(self.doc)
            {
                let sel = d.selection.clone();
                d.undo.push(crate::HistoryEntry { label: "Solver".into(), wb: self.start_wb.clone(), selection: sel });
                d.redo.clear();
                d.revision += 1;
            }
        } else if self.wrote_trial {
            let x0 = p.spec.x0.clone();
            self.write(s, &x0)?;
        }
        let result = self.report_data(&o, &x, final_eval.as_ref(), &at, seconds, apply);
        s.solver_last = Some(result.clone());
        if self.interactive {
            s.ui_requests.push(crate::UiRequest::Dialog("solverResults".into(), result.clone()));
        }
        Ok(result)
    }

    /// Everything the Solver Results box and the reports need.
    fn report_data(&self, o: &sv::Outcome, x: &[f64], e: Option<&sv::Eval>, at: &(f64, Vec<(f64, f64)>), seconds: f64, kept: bool) -> Json {
        let p = &self.prepared;
        let precision = p.spec.options.precision.max(1e-9);
        let original_obj = self.original_objective();
        let variables: Vec<Json> = p
            .vars
            .iter()
            .enumerate()
            .map(|(j, c)| {
                let kind = if p.items.iter().any(|it| it.kind == ItemKind::Binary { var: j }) {
                    "Binary"
                } else if p.spec.integer.get(j).copied().unwrap_or(false) || p.spec.alldiff.iter().any(|g| g.contains(&j)) {
                    "Integer"
                } else {
                    "Contin"
                };
                json!({"cell": abs_a1(*c), "original": p.spec.x0.get(j), "final": x.get(j), "integer": kind})
            })
            .collect();
        let row_value = |row: usize| -> (f64, f64) { at.1.get(row).copied().unwrap_or((f64::NAN, f64::NAN)) };
        let constraints: Vec<Json> = p
            .items
            .iter()
            .map(|it| {
                let user = self.model.constraints.get(it.user);
                let left = abs_a1(it.cell);
                let (value, formula, binding, slack) = match &it.kind {
                    ItemKind::Row { row, rel, rhs } => {
                        let g = e.and_then(|e| e.cons.get(*row)).copied().unwrap_or(f64::NAN);
                        let (lhs, _) = row_value(*row);
                        let right = match rhs {
                            Rhs::Const(v) => gridcraft_core::number_to_text(*v),
                            Rhs::Cell(_, c) => abs_a1(*c),
                        };
                        let slack = if *rel == Rel::Eq { 0.0 } else { g.abs() };
                        (lhs, format!("{left}{}{right}", sym(*rel)), slack <= precision.max(1e-7 * lhs.abs()), slack)
                    }
                    ItemKind::Bound { var, rel, value } => {
                        let v = x.get(*var).copied().unwrap_or(f64::NAN);
                        let slack = if *rel == Rel::Eq { 0.0 } else { (v - value).abs() };
                        (v, format!("{left}{}{}", sym(*rel), gridcraft_core::number_to_text(*value)), slack <= precision.max(1e-7 * v.abs()), slack)
                    }
                    ItemKind::Integer { var } => (x.get(*var).copied().unwrap_or(f64::NAN), format!("{left}=Integer"), true, 0.0),
                    ItemKind::Binary { var } => (x.get(*var).copied().unwrap_or(f64::NAN), format!("{left}=Binary"), true, 0.0),
                    ItemKind::AllDiff => (f64::NAN, format!("{}=AllDifferent", user.map(|u| u.left.clone()).unwrap_or_default()), true, 0.0),
                };
                json!({"cell": left, "value": if value.is_finite() { json!(value) } else { Json::Null }, "formula": formula, "status": if binding { "Binding" } else { "Not Binding" }, "slack": slack})
            })
            .collect();
        let sensitivity = o.lp.as_ref().map(|s| {
            json!({
                "kind": "lp",
                "variables": p.vars.iter().enumerate().map(|(j, c)| json!({
                    "cell": abs_a1(*c),
                    "final": x.get(j),
                    "reducedCost": s.reduced_cost.get(j),
                    "objectiveCoefficient": s.objective_coef.get(j),
                    "allowableIncrease": s.coef_increase.get(j),
                    "allowableDecrease": s.coef_decrease.get(j),
                })).collect::<Vec<_>>(),
                "constraints": p.rows.iter().enumerate().map(|(i, (c, _))| {
                    let (lhs, rhs_v) = row_value(i);
                    json!({
                        "cell": abs_a1(*c),
                        "final": lhs,
                        "shadowPrice": s.shadow_price.get(i),
                        "rhs": rhs_v,
                        "allowableIncrease": s.rhs_increase.get(i),
                        "allowableDecrease": s.rhs_decrease.get(i),
                    })
                }).collect::<Vec<_>>(),
            })
        });
        let sensitivity = sensitivity.or_else(|| {
            o.nlp.as_ref().map(|s| {
                json!({
                    "kind": "nlp",
                    "variables": p.vars.iter().enumerate().map(|(j, c)| json!({
                        "cell": abs_a1(*c),
                        "final": x.get(j),
                        "reducedGradient": s.reduced_gradient.get(j),
                    })).collect::<Vec<_>>(),
                    "constraints": p.rows.iter().enumerate().map(|(i, (c, _))| json!({
                        "cell": abs_a1(*c),
                        "final": row_value(i).0,
                        "lagrangeMultiplier": s.lagrange.get(i),
                    })).collect::<Vec<_>>(),
                })
            })
        });
        let method = match self.model.method {
            Method::Grg => "GRG Nonlinear",
            Method::Simplex => "Simplex LP",
            Method::Evolutionary => "Evolutionary",
        };
        json!({
            "status": o.status.code(),
            "message": o.status.message(),
            "feasible": o.status.feasible(),
            "kept": kept,
            "method": method,
            "goal": match self.model.goal { Goal::Max => "Max", Goal::Min => "Min", Goal::Value => "Value Of" },
            "iterations": o.iterations,
            "subproblems": o.subproblems,
            "evaluations": self.problem.evaluations,
            "seconds": seconds,
            "sheet": self.sheet_name(),
            "objective": {"cell": abs_a1(p.objective), "original": original_obj, "final": if at.0.is_finite() { json!(at.0) } else { Json::Null }},
            "variables": variables,
            "constraints": constraints,
            "sensitivity": sensitivity,
            "population": o.population.iter().zip(&p.vars).map(|(st, c)| json!({"cell": abs_a1(*c), "mean": st[0], "sd": st[1], "max": st[2], "min": st[3]})).collect::<Vec<_>>(),
            "integers": p.spec.has_integers(),
            "solution": x,
            "model": serde_json::to_value(&self.model).unwrap_or(Json::Null),
        })
    }

    fn sheet_name(&self) -> String {
        self.problem.trial_sheet_name(self.prepared.sheet)
    }

    fn original_objective(&self) -> Option<f64> {
        self.problem.original(self.prepared.objective)
    }
}

impl CellProblem {
    /// Objective, each row's left side and right side at `x` (NaN where not a number).
    fn values_at(&mut self, x: &[f64]) -> (f64, Vec<(f64, f64)>) {
        let sets: Vec<(usize, CellRef, Value)> = self.vars.iter().zip(x).map(|(c, v)| (self.sheet, *c, Value::Number(*v))).collect();
        let vals = self.trial.run(&sets, &self.reads);
        let num = |k: usize| vals.get(k).and_then(number).unwrap_or(f64::NAN);
        let rows = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, (_, r))| {
                let rhs = match (r, self.rhs_at.get(i).copied().flatten()) {
                    (Rhs::Const(v), _) => *v,
                    (Rhs::Cell(..), Some(k)) => num(k),
                    _ => f64::NAN,
                };
                (num(1 + i), rhs)
            })
            .collect();
        (num(0), rows)
    }
    fn trial_sheet_name(&self, si: usize) -> String {
        self.trial.base().sheet(si).map(|s| s.name.clone()).unwrap_or_default()
    }
    fn original(&self, c: CellRef) -> Option<f64> {
        self.trial.base().sheet(self.sheet).map(|s| s.value(c)).and_then(|v| number(&v))
    }
}

fn sym(r: Rel) -> &'static str {
    match r {
        Rel::Le => "<=",
        Rel::Eq => "=",
        Rel::Ge => ">=",
    }
}

impl crate::job::Job for SolverJob {
    fn label(&self) -> String {
        "Solver".into()
    }

    fn step(&mut self, s: &mut Session, budget: crate::job::Budget) -> crate::job::Step {
        let mut units = 0u64;
        loop {
            match self.engine.step(&mut self.problem, 4) {
                sv::Step::Done(o) => return crate::job::Step::Done(self.finish(s, *o)),
                sv::Step::Running { iteration, objective, x } => {
                    let now = crate::clock::now_ms();
                    // Show Iteration Results: pause on each new trial solution, shown on the sheet.
                    if self.interactive && self.model.options.show_iterations && iteration > self.last_shown && x.len() == self.prepared.vars.len() {
                        self.last_shown = iteration;
                        if self.write(s, &x).is_ok() {
                            self.wrote_trial = true;
                        }
                        let obj = if objective.is_finite() { json!(objective) } else { Json::Null };
                        return crate::job::Step::Pause(json!({"dialog": "solverTrial", "iteration": iteration, "objective": obj}));
                    }
                    if self.max_ms > 0.0 && crate::clock::has_clock() && now - self.started >= self.max_ms {
                        let o = self.engine.stop(Status::MaxTime);
                        return crate::job::Step::Done(self.finish(s, o));
                    }
                    if self.model.method == Method::Evolutionary && crate::clock::has_clock() {
                        let better = match self.model.goal {
                            Goal::Max => objective > self.best_seen,
                            _ => objective < self.best_seen,
                        };
                        if objective.is_finite() && (self.best_seen.is_nan() || better) {
                            self.best_seen = objective;
                            self.improved_at = now;
                        } else if now - self.improved_at >= self.model.options.max_time_without_improvement.max(0.1) * 1000.0 {
                            let o = self.engine.stop(Status::CannotImprove);
                            return crate::job::Step::Done(self.finish(s, o));
                        }
                    }
                    units += 4;
                    if budget.expired(units) {
                        let text = if objective.is_finite() {
                            format!("Trial Solution: {iteration}, Objective: {}", gridcraft_core::number_to_text(objective))
                        } else {
                            "Setting up the problem…".to_string()
                        };
                        return crate::job::Step::Continue(text);
                    }
                }
            }
        }
    }

    fn stop(&mut self, s: &mut Session) -> Result<Json> {
        let o = self.engine.stop(Status::Stopped);
        self.finish(s, o)
    }
}

fn solve(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "data.solver";
    let si = s.doc()?.wb.active_sheet;
    let mut m = load_model(&s.doc()?.wb, si);
    merge_model(&mut m, p, CMD)?;
    if m.objective.is_empty() || m.variables.is_empty() {
        return Err(bad(CMD, "give `objective` and `variables` (or set up the model first)"));
    }
    edit(s, |cx| {
        store_model(&mut cx.wb, si, &m);
        Ok(())
    })?;
    let keep = bool_param(p, "keep").unwrap_or(true);
    let max_ms = if m.options.max_time > 0.0 { m.options.max_time * 1000.0 } else { 120_000.0 };
    let job = SolverJob::new(s, m, keep, false)?;
    s.run_job(Box::new(job), max_ms, 5_000_000)
}

fn start(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "data.solverStart";
    let si = s.doc()?.wb.active_sheet;
    let mut m = load_model(&s.doc()?.wb, si);
    merge_model(&mut m, p, CMD)?;
    if m.objective.is_empty() || m.variables.is_empty() {
        return Err(EngineError::Other("Set the objective and the variable cells first.".into()));
    }
    edit(s, |cx| {
        store_model(&mut cx.wb, si, &m);
        Ok(())
    })?;
    let job = SolverJob::new(s, m, true, true)?;
    s.start_job(Box::new(job))?;
    Ok(json!({"started": true}))
}

fn restore(s: &mut Session, _p: &Json) -> Result<Json> {
    let Some(last) = s.solver_last.clone() else { return Err(EngineError::Other("There is no Solver result to restore.".into())) };
    if last["kept"] != json!(true) {
        return Ok(json!({"restored": false}));
    }
    if s.doc()?.undo.last().is_some_and(|h| h.label == "Solver") {
        s.execute("edit.undo", json!({}))?;
    }
    if let Some(l) = s.solver_last.as_mut() {
        l["kept"] = json!(false);
    }
    Ok(json!({"restored": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    fn sample_model() -> Json {
        json!({
            "objective": "$D$1",
            "goal": "max",
            "variables": "$B$1:$B$2,$B$4",
            "constraints": [
                {"left": "$C$1", "op": "<=", "right": 10},
                {"left": "$B$1", "op": ">=", "right": "$E$1"},
                {"left": "$B$2", "op": "int"},
                {"left": "$B$4", "op": "bin"},
            ],
            "method": "simplex",
            "nonNegative": false,
            "options": {"maxTime": 30, "integerOptimality": 5, "centralDerivatives": true},
        })
    }

    #[test]
    fn model_is_stored_as_excel_names_and_round_trips() {
        let mut s = session();
        s.execute("data.solverSetModel", sample_model()).unwrap();
        let wb = &s.doc().unwrap().wb;
        let get = |n: &str| name_value(wb, 0, n).unwrap_or_default();
        assert_eq!(get("solver_opt"), "Sheet1!$D$1");
        assert_eq!(get("solver_typ"), "1");
        assert_eq!(get("solver_adj"), "Sheet1!$B$1:$B$2,Sheet1!$B$4");
        assert_eq!(get("solver_num"), "4");
        assert_eq!(get("solver_rel1"), "1");
        assert_eq!(get("solver_rhs1"), "10");
        assert_eq!(get("solver_rhs2"), "Sheet1!$E$1");
        assert_eq!(get("solver_rel3"), "4");
        assert_eq!(get("solver_rhs3"), "\"integer\"");
        assert_eq!(get("solver_eng"), "2");
        assert_eq!(get("solver_neg"), "2");
        assert_eq!(get("solver_tol"), "0.05");
        assert!(wb.names.iter().filter(|n| n.name.starts_with("solver_")).all(|n| n.hidden && n.scope == Some(0)));
        let m = s.execute("data.solverModel", json!({})).unwrap();
        assert_eq!(m["variables"], "$B$1:$B$2,$B$4");
        assert_eq!(m["constraints"][1]["right"], "$E$1");
        assert_eq!(m["constraints"][3]["op"], "bin");
        assert_eq!(m["options"]["maxTime"], 30.0);
        assert_eq!(m["options"]["centralDerivatives"], true);
        // Through xlsx.
        let b64 = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap()["base64"].as_str().unwrap().to_string();
        s.execute("file.open", json!({"name": "m.xlsx", "base64": b64})).unwrap();
        assert_eq!(s.execute("data.solverModel", json!({})).unwrap(), m);
        // Partial updates keep the rest; reset clears it.
        s.execute("data.solverSetModel", json!({"goal": "min"})).unwrap();
        let m2 = s.execute("data.solverModel", json!({})).unwrap();
        assert_eq!((m2["goal"].as_str(), m2["objective"].as_str()), (Some("min"), Some("$D$1")));
        s.execute("data.solverReset", json!({})).unwrap();
        assert_eq!(s.execute("data.solverModel", json!({})).unwrap()["objective"], "");
        assert!(s.execute("data.solverSetModel", json!({"constraints": [{"left": "A1", "op": "<>"}]})).is_err());
        assert!(s.execute("data.solverSetModel", json!({"constraints": [{"left": "A1", "op": "<="}]})).is_err());
    }

    /// The Wyndor model on a sheet: B1:B2 decisions, D1 profit, C1:C3 plant hours.
    fn wyndor(s: &mut Session) {
        s.execute("range.setValues", json!({"range": "A1", "values": [["Doors", 0], ["Windows", 0]]})).unwrap();
        for (c, f) in [("C1", "=B1"), ("C2", "=2*B2"), ("C3", "=3*B1+2*B2"), ("D1", "=3*B1+5*B2")] {
            s.execute("cell.set", json!({"cell": c, "input": f})).unwrap();
        }
        s.execute("range.setValues", json!({"range": "E1", "values": [[4], [12], [18]]})).unwrap();
    }

    fn n(s: &Session, a1: &str) -> f64 {
        s.doc().unwrap().wb.active().unwrap().value(CellRef::parse(a1).unwrap()).as_f64().unwrap_or(f64::NAN)
    }

    #[test]
    fn solves_a_linear_model_in_the_sheet() {
        let mut s = session();
        wyndor(&mut s);
        let r = s
            .execute(
                "data.solver",
                json!({"objective": "$D$1", "goal": "max", "variables": "$B$1:$B$2", "method": "simplex", "constraints": [{"left": "$C$1:$C$3", "op": "<=", "right": "$E$1:$E$3"}]}),
            )
            .unwrap();
        assert_eq!(r["status"], 0, "{r}");
        assert_eq!((n(&s, "B1"), n(&s, "B2"), n(&s, "D1")), (2.0, 6.0, 36.0));
        assert_eq!(r["objective"]["original"], 0.0);
        assert_eq!(r["objective"]["final"], 36.0);
        assert_eq!(r["constraints"][0]["status"], "Not Binding");
        assert_eq!(r["constraints"][0]["slack"], 2.0);
        assert_eq!(r["constraints"][1]["status"], "Binding");
        assert_eq!(r["sensitivity"]["constraints"][1]["shadowPrice"], 1.5);
        assert_eq!(r["sensitivity"]["constraints"][2]["rhs"], 18.0);
        assert_eq!(r["sensitivity"]["variables"][0]["objectiveCoefficient"], 3.0);
        // One undo step brings back the starting values (and the previous model).
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(n(&s, "B1"), 0.0);
        assert!(!has_model(&s.doc().unwrap().wb, 0));
        // keep: false leaves the cells alone.
        let r = s
            .execute(
                "data.solver",
                json!({"objective": "$D$1", "variables": "$B$1:$B$2", "method": "simplex", "keep": false, "constraints": [{"left": "$C$1:$C$3", "op": "<=", "right": "$E$1:$E$3"}]}),
            )
            .unwrap();
        assert_eq!(r["status"], 0);
        assert_eq!(n(&s, "B1"), 0.0);
    }

    #[test]
    fn integer_bounds_value_of_and_failures() {
        let mut s = session();
        wyndor(&mut s);
        let base = json!({"objective": "$D$1", "variables": "$B$1:$B$2", "method": "simplex", "constraints": [
            {"left": "$C$1:$C$3", "op": "<=", "right": "$E$1:$E$3"},
            {"left": "$B$1:$B$2", "op": "int"},
            {"left": "$B$2", "op": "<=", "right": "5"},
        ]});
        let r = s.execute("data.solver", base.clone()).unwrap();
        assert!(r["status"] == 0 || r["status"] == 14, "{r}");
        // With y ≤ 5: 3x + 2·5 ≤ 18 ⇒ x ≤ 2.67 → x = 2, y = 5, profit 31.
        assert_eq!((n(&s, "B1"), n(&s, "B2")), (2.0, 5.0));
        assert_eq!(r["variables"][0]["integer"], "Integer");
        assert!(r["sensitivity"].is_null());
        // Value Of.
        let r = s.execute("data.solver", json!({"goal": "value", "valueOf": 20, "constraints": []})).unwrap();
        assert_eq!(r["status"], 0);
        assert!((n(&s, "D1") - 20.0).abs() < 1e-9);
        // Infeasible: profit ≥ 100 with the plant limits.
        let r = s
            .execute(
                "data.solver",
                json!({"goal": "max", "constraints": [{"left": "$C$1:$C$3", "op": "<=", "right": "$E$1:$E$3"}, {"left": "$D$1", "op": ">=", "right": "100"}]}),
            )
            .unwrap();
        assert_eq!(r["status"], 5);
        // A nonlinear objective is refused by Simplex LP.
        s.execute("cell.set", json!({"cell": "D1", "input": "=B1*B2"})).unwrap();
        let r = s.execute("data.solver", json!({"constraints": [{"left": "$C$1:$C$3", "op": "<=", "right": "$E$1:$E$3"}]})).unwrap();
        assert_eq!(r["status"], 7);
        // Bad models.
        assert!(s.execute("data.solver", json!({"objective": "$D$1:$D$2"})).is_err());
        assert!(s.execute("data.solver", json!({"objective": "$D$1", "variables": "$C$1"})).is_err(), "formula as a variable");
        assert!(s.execute("data.solver", json!({"objective": "$D$1", "variables": "$B$1", "constraints": [{"left": "$C$1", "op": "int"}]})).is_err());
    }

    #[test]
    fn nonlinear_model_with_grg() {
        let mut s = session();
        // Max area x·y with fence 2x + 2y ≤ 40 → a 10 × 10 square.
        s.execute("range.setValues", json!({"range": "B1", "values": [[1], [1]]})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "=B1*B2"})).unwrap();
        s.execute("cell.set", json!({"cell": "C2", "input": "=2*B1+2*B2"})).unwrap();
        let r = s
            .execute(
                "data.solver",
                json!({"objective": "$C$1", "goal": "max", "variables": "$B$1:$B$2", "constraints": [{"left": "$C$2", "op": "<=", "right": "40"}]}),
            )
            .unwrap();
        assert!(r["status"] == 0 || r["status"] == 1, "{r}");
        assert!((n(&s, "B1") - 10.0).abs() < 1e-3 && (n(&s, "B2") - 10.0).abs() < 1e-3, "{r}");
        assert_eq!(r["method"], "GRG Nonlinear");
        assert_eq!(r["sensitivity"]["kind"], "nlp");
        let lm = r["sensitivity"]["constraints"][0]["lagrangeMultiplier"].as_f64().unwrap();
        assert!((lm - 5.0).abs() < 1e-2, "{lm}");
    }

    #[test]
    fn non_smooth_model_with_evolutionary() {
        let mut s = session();
        // Minimise a stepped cost: ROUND makes the model non-smooth.
        s.execute("range.setValues", json!({"range": "B1", "values": [[0]]})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "=ABS(ROUND(B1,0)-7)+ABS(B1-7.2)/100"})).unwrap();
        let r = s
            .execute(
                "data.solver",
                json!({"objective": "$C$1", "goal": "min", "variables": "$B$1", "method": "evolutionary",
                       "constraints": [{"left": "$B$1", "op": "<=", "right": "20"}],
                       "options": {"randomSeed": 5, "maxTimeWithoutImprovement": 1}}),
            )
            .unwrap();
        assert!(r["status"] == 1 || r["status"] == 2, "{r}");
        assert!((n(&s, "B1") - 7.2).abs() < 0.3, "{r}");
        assert_eq!(r["method"], "Evolutionary");
        // Without an upper bound (and Require Bounds on) it asks for bounds.
        let r = s.execute("data.solver", json!({"constraints": []})).unwrap();
        assert_eq!(r["status"], 18);
    }

    #[test]
    fn interactive_solve_runs_as_a_job() {
        let mut s = session();
        wyndor(&mut s);
        s.execute(
            "data.solverStart",
            json!({"objective": "$D$1", "variables": "$B$1:$B$2", "method": "simplex", "constraints": [{"left": "$C$1:$C$3", "op": "<=", "right": "$E$1:$E$3"}]}),
        )
        .unwrap();
        for _ in 0..100 {
            s.step_job(10.0);
            if s.job.is_none() {
                break;
            }
        }
        let (label, r) = s.take_job_result().unwrap();
        assert_eq!(label, "Solver");
        assert_eq!(r.unwrap()["status"], 0);
        assert_eq!(n(&s, "B2"), 6.0);
        assert!(s.take_ui_requests().iter().any(|u| matches!(u, crate::UiRequest::Dialog(name, _) if name == "solverResults")));
        assert_eq!(s.doc().unwrap().undo.last().map(|h| h.label.as_str()), Some("Solver"));
        // Restore Original Values.
        s.execute("data.solverRestore", json!({})).unwrap();
        assert_eq!(n(&s, "B2"), 0.0);
    }

    #[test]
    fn save_and_load_model_cells() {
        let mut s = session();
        s.execute("data.solverSetModel", sample_model()).unwrap();
        let saved = s.execute("data.solverModel", json!({})).unwrap();
        let r = s.execute("data.solverSaveModel", json!({"to": "H1"})).unwrap();
        assert_eq!(r["range"], "H1:H7");
        assert_eq!(s.execute("cell.get", json!({"cell": "H1"})).unwrap()["formula"], "=MAX($D$1)");
        assert_eq!(s.execute("cell.get", json!({"cell": "H5"})).unwrap()["formula"], "=$B$2=\"integer\"");
        s.execute("data.solverReset", json!({})).unwrap();
        s.execute("data.solverLoadModel", json!({"from": "H1:H7"})).unwrap();
        assert_eq!(s.execute("data.solverModel", json!({})).unwrap(), saved);
        assert!(s.execute("data.solverLoadModel", json!({"from": "A1:A3"})).is_err());
    }
}
