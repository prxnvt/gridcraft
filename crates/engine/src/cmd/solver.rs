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
