//! Solver's reports: Answer, Sensitivity, Limits and Population, each written to a new sheet
//! ("Answer Report 1"…) before the model's sheet, from the last run's result.

use gridcraft_analysis::solver as sv;
use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Sheet, Workbook};
use serde_json::{Value as Json, json};

use super::solver::{self, Method, SolverModel};
use super::*;
use crate::report::{self, OutputKind, RStyle, Report, Target};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "data.solverReports",
        "Solver Reports",
        [],
        None,
        "{reports: [answer|sensitivity|limits|population], outline?: bool} writes reports for the last Solver run on new sheets (Sensitivity and Limits need a model without integer constraints; Population needs Evolutionary)",
        has_doc,
        reports
    )]
}

/// The text labels Excel uses to name a cell: the nearest text to its left and above it.
pub(crate) fn cell_name(sh: &Sheet, c: CellRef) -> String {
    let text = |c: CellRef| match sh.value(c) {
        Value::Text(t) => Some(t.to_string()),
        _ => None,
    };
    let left = (0..c.col).rev().find_map(|col| text(CellRef::new(c.row, col)));
    let top = (0..c.row).rev().find_map(|row| text(CellRef::new(row, c.col)));
    [left, top].into_iter().flatten().collect::<Vec<_>>().join(" ")
}

fn now_text(wb: &Workbook) -> String {
    crate::display::format(&Value::Number(gridcraft_calc::now_serial()), "m/d/yyyy h:mm:ss AM/PM", wb).text
}

fn bold() -> RStyle {
    RStyle { bold: true, ..RStyle::default() }
}

fn head() -> RStyle {
    RStyle { bottom: true, ..RStyle::default() }
}

fn limit(v: u64) -> String {
    if v == 0 { "Unlimited".into() } else { v.to_string() }
}

/// Title, worksheet and date lines; returns the next row.
fn title(rep: &mut Report, wb: &Workbook, sheet: &str, what: &str) -> u32 {
    rep.text(0, 0, &format!("GridCraft {what} Report"), bold());
    rep.text(1, 0, &format!("Worksheet: {sheet}"), RStyle::default());
    rep.text(2, 0, &format!("Report Created: {}", now_text(wb)), RStyle::default());
    3
}

/// Writes `cells` as a table with its header row, from `row`; returns the row after it.
fn table(rep: &mut Report, row: u32, headers: &[&str], rows: Vec<Vec<Json>>) -> u32 {
    for (k, h) in headers.iter().enumerate() {
        rep.text(row, 1 + k as u32, h, head());
    }
    let n = rows.len();
    for (i, cells) in rows.into_iter().enumerate() {
        let r = row + 1 + i as u32;
        let style = if i + 1 == n { RStyle { bottom: true, ..RStyle::default() } } else { RStyle::default() };
        for (k, v) in cells.into_iter().enumerate() {
            let value = match v {
                Json::Number(x) => Value::Number(x.as_f64().unwrap_or(0.0)),
                Json::String(t) => Value::text(t),
                Json::Bool(b) => Value::Bool(b),
                _ => Value::Error(gridcraft_core::CellError::NA),
            };
            rep.put(r, 1 + k as u32, value, style.clone());
        }
    }
    row + 1 + n as u32
}

/// A big number in sensitivity reports prints as 1E+30, as Excel shows "infinity".
fn sens_num(v: &Json) -> Json {
    match v.as_f64() {
        Some(x) if x >= sv::BIG => json!(1e30),
        Some(x) => json!(x),
        None => Json::Null,
    }
}

fn answer(wb: &Workbook, sh: &Sheet, last: &Json, model: &SolverModel) -> Report {
    let mut rep = Report::new();
    let mut r = title(&mut rep, wb, &sh.name, "Answer");
    rep.text(r, 0, &format!("Result: {}", last["message"].as_str().unwrap_or("")), RStyle::default());
    r += 1;
    rep.text(r, 0, "Solver Engine", RStyle::default());
    rep.text(r + 1, 1, &format!("Engine: {}", last["method"].as_str().unwrap_or("")), RStyle::default());
    rep.text(r + 2, 1, &format!("Solution Time: {:.3} Seconds.", last["seconds"].as_f64().unwrap_or(0.0)), RStyle::default());
    rep.text(r + 3, 1, &format!("Iterations: {} Subproblems: {}", last["iterations"], last["subproblems"]), RStyle::default());
    rep.text(r + 4, 0, "Solver Options", RStyle::default());
    let o = &model.options;
    let time = if o.max_time > 0.0 { format!("{} Sec", gridcraft_core::number_to_text(o.max_time)) } else { "Unlimited".into() };
    rep.text(
        r + 5,
        1,
        &format!(
            "Max Time {time}, Iterations {}, Precision {}{}{}",
            limit(o.iterations),
            gridcraft_core::number_to_text(o.constraint_precision),
            if o.automatic_scaling { ", Use Automatic Scaling" } else { "" },
            if o.show_iterations { ", Show Iteration Results" } else { "" }
        ),
        RStyle::default(),
    );
    let mut line = match model.method {
        Method::Grg => format!(
            "Convergence {}, Derivatives {}{}",
            gridcraft_core::number_to_text(o.convergence),
            if o.central_derivatives { "Central" } else { "Forward" },
            if o.multistart { format!(", Multistart, Population Size {}, Random Seed {}", o.population_size, o.random_seed) } else { String::new() }
        ),
        Method::Evolutionary => format!(
            "Convergence {}, Population Size {}, Mutation Rate {}, Time w/o Improvement {} Sec",
            gridcraft_core::number_to_text(o.evolutionary_convergence),
            o.population_size,
            gridcraft_core::number_to_text(o.mutation_rate),
            gridcraft_core::number_to_text(o.max_time_without_improvement)
        ),
        Method::Simplex => String::new(),
    };
    let ints = format!(
        "Max Subproblems {}, Max Integer Sols {}, Integer Tolerance {}%{}",
        limit(o.max_subproblems),
        limit(o.max_feasible),
        gridcraft_core::number_to_text(o.integer_optimality),
        if model.non_negative { ", Assume NonNegative" } else { "" }
    );
    line = if line.is_empty() { ints } else { format!("{line}, {ints}") };
    rep.text(r + 6, 1, &line, RStyle::default());
    r += 8;
    rep.text(r, 0, &format!("Objective Cell ({})", last["goal"].as_str().unwrap_or("Max")), RStyle::default());
    let obj = &last["objective"];
    let objc = CellRef::parse(obj["cell"].as_str().unwrap_or("").trim_start_matches('$').replace('$', "").as_str()).unwrap_or_default();
    r = table(
        &mut rep,
        r + 1,
        &["Cell", "Name", "Original Value", "Final Value"],
        vec![vec![obj["cell"].clone(), json!(cell_name(sh, objc)), obj["original"].clone(), obj["final"].clone()]],
    );
    r += 2;
    rep.text(r, 0, "Variable Cells", RStyle::default());
    let vars: Vec<Vec<Json>> = last["variables"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| {
                    let c = CellRef::parse(&v["cell"].as_str().unwrap_or("").replace('$', "")).unwrap_or_default();
                    vec![v["cell"].clone(), json!(cell_name(sh, c)), v["original"].clone(), v["final"].clone(), v["integer"].clone()]
                })
                .collect()
        })
        .unwrap_or_default();
    r = table(&mut rep, r + 1, &["Cell", "Name", "Original Value", "Final Value", "Integer"], vars);
    r += 2;
    rep.text(r, 0, "Constraints", RStyle::default());
    let cons: Vec<Vec<Json>> = last["constraints"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| {
                    let c = CellRef::parse(&v["cell"].as_str().unwrap_or("").replace('$', "")).unwrap_or_default();
                    vec![
                        v["cell"].clone(),
                        json!(cell_name(sh, c)),
                        v["value"].clone(),
                        v["formula"].clone(),
                        v["status"].clone(),
                        v["slack"].clone(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&mut rep, r + 1, &["Cell", "Name", "Cell Value", "Formula", "Status", "Slack"], cons);
    rep.col_widths = vec![(0, 14.0), (1, 64.0), (2, 140.0), (3, 90.0), (4, 90.0), (5, 80.0), (6, 64.0)];
    rep
}

fn sensitivity(wb: &Workbook, sh: &Sheet, last: &Json) -> std::result::Result<Report, String> {
    let s = &last["sensitivity"];
    if s.is_null() {
        return Err(if last["integers"] == json!(true) {
            "Sensitivity and Limits reports don't apply to models with integer constraints.".into()
        } else {
            "There is no sensitivity information for this result.".into()
        });
    }
    let mut rep = Report::new();
    let mut r = title(&mut rep, wb, &sh.name, "Sensitivity") + 1;
    let name = |v: &Json| -> Json {
        let c = CellRef::parse(&v["cell"].as_str().unwrap_or("").replace('$', "")).unwrap_or_default();
        json!(cell_name(sh, c))
    };
    let lp = s["kind"] == "lp";
    rep.text(r, 0, "Variable Cells", RStyle::default());
    let vars = s["variables"].as_array().cloned().unwrap_or_default();
    let cons = s["constraints"].as_array().cloned().unwrap_or_default();
    if lp {
        for (k, h) in ["", "", "Final", "Reduced", "Objective", "Allowable", "Allowable"].iter().enumerate() {
            rep.text(r + 1, 1 + k as u32, h, RStyle::default());
        }
        let rows = vars
            .iter()
            .map(|v| {
                vec![
                    v["cell"].clone(),
                    name(v),
                    v["final"].clone(),
                    v["reducedCost"].clone(),
                    v["objectiveCoefficient"].clone(),
                    sens_num(&v["allowableIncrease"]),
                    sens_num(&v["allowableDecrease"]),
                ]
            })
            .collect();
        r = table(&mut rep, r + 2, &["Cell", "Name", "Value", "Cost", "Coefficient", "Increase", "Decrease"], rows) + 1;
        rep.text(r, 0, "Constraints", RStyle::default());
        for (k, h) in ["", "", "Final", "Shadow", "Constraint", "Allowable", "Allowable"].iter().enumerate() {
            rep.text(r + 1, 1 + k as u32, h, RStyle::default());
        }
        let rows = cons
            .iter()
            .map(|v| {
                vec![
                    v["cell"].clone(),
                    name(v),
                    v["final"].clone(),
                    v["shadowPrice"].clone(),
                    v["rhs"].clone(),
                    sens_num(&v["allowableIncrease"]),
                    sens_num(&v["allowableDecrease"]),
                ]
            })
            .collect();
        table(&mut rep, r + 2, &["Cell", "Name", "Value", "Price", "R.H. Side", "Increase", "Decrease"], rows);
    } else {
        for (k, h) in ["", "", "Final", "Reduced"].iter().enumerate() {
            rep.text(r + 1, 1 + k as u32, h, RStyle::default());
        }
        let rows = vars.iter().map(|v| vec![v["cell"].clone(), name(v), v["final"].clone(), v["reducedGradient"].clone()]).collect();
        r = table(&mut rep, r + 2, &["Cell", "Name", "Value", "Gradient"], rows) + 1;
        rep.text(r, 0, "Constraints", RStyle::default());
        for (k, h) in ["", "", "Final", "Lagrange"].iter().enumerate() {
            rep.text(r + 1, 1 + k as u32, h, RStyle::default());
        }
        let rows = cons.iter().map(|v| vec![v["cell"].clone(), name(v), v["final"].clone(), v["lagrangeMultiplier"].clone()]).collect();
        table(&mut rep, r + 2, &["Cell", "Name", "Value", "Multiplier"], rows);
    }
    rep.col_widths = vec![(0, 14.0), (1, 64.0), (2, 140.0), (3, 80.0), (4, 80.0), (5, 80.0), (6, 80.0), (7, 80.0)];
    Ok(rep)
}

/// Optimises variable `j` (others held at the solution) with the model's constraints.
struct VarObjective<'a> {
    inner: &'a mut solver::CellProblem,
    j: usize,
}

impl sv::Problem for VarObjective<'_> {
    fn eval(&mut self, x: &[f64]) -> Option<sv::Eval> {
        let e = self.inner.eval(x)?;
        Some(sv::Eval { objective: x.get(self.j).copied()?, cons: e.cons })
    }
}

fn limits(s: &Session, last: &Json, model: &SolverModel) -> std::result::Result<Report, String> {
    if last["integers"] == json!(true) {
        return Err("Sensitivity and Limits reports don't apply to models with integer constraints.".into());
    }
    if model.method == Method::Evolutionary {
        return Err("The Limits report isn't available for the Evolutionary method.".into());
    }
    let d = s.doc().map_err(|e| e.to_string())?;
    let si = d.wb.sheet_index(last["sheet"].as_str().unwrap_or("")).unwrap_or(d.wb.active_sheet);
    let sh = d.wb.sheet(si).ok_or("No such sheet.")?;
    let prepared = solver::prepare(&d.wb, si, model).map_err(|(_, m)| m)?;
    let x: Vec<f64> = last["solution"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default();
    if x.len() != prepared.vars.len() {
        return Err("The last Solver result doesn't match the model.".into());
    }
    let mut problem = solver::CellProblem::new(d, &prepared);
    let mut rows = Vec::new();
    for j in 0..x.len() {
        let mut spec = prepared.spec.clone();
        spec.x0 = x.clone();
        // Every other variable is held at its solution value.
        for (k, xk) in x.iter().enumerate() {
            if k != j
                && let (Some(l), Some(u)) = (spec.lower.get_mut(k), spec.upper.get_mut(k))
            {
                *l = *xk;
                *u = *xk;
            }
        }
        let mut ends = Vec::new();
        for sense in [sv::Sense::Min, sv::Sense::Max] {
            spec.sense = sense;
            let mut engine: Box<dyn sv::Engine> = match model.method {
                Method::Simplex => Box::new(sv::simplex::Simplex::new(spec.clone())),
                _ => Box::new(sv::grg::Grg::new(spec.clone())),
            };
            let mut wrapper = VarObjective { inner: &mut problem, j };
            let mut out = None;
            for _ in 0..20_000 {
                if let sv::Step::Done(o) = engine.step(&mut wrapper, 50) {
                    out = Some(*o);
                    break;
                }
            }
            let point = out.filter(|o| o.status.feasible()).map(|o| o.x);
            let value = point.as_ref().and_then(|p| p.get(j).copied());
            let objective = point.and_then(|p| sv::Problem::eval(&mut problem, &p)).map(|e| e.objective);
            ends.push((value, objective));
        }
        let num = |v: Option<f64>| v.map(|x| json!(x)).unwrap_or(Json::Null);
        let c = prepared.vars[j];
        rows.push(vec![
            json!(format!("${}${}", gridcraft_core::col_to_letters(c.col), c.row + 1)),
            json!(cell_name(sh, c)),
            json!(x[j]),
            num(ends[0].0),
            num(ends[0].1),
            num(ends[1].0),
            num(ends[1].1),
        ]);
    }
    let mut rep = Report::new();
    let mut r = title(&mut rep, &d.wb, &sh.name, "Limits") + 1;
    let obj = &last["objective"];
    let objc = CellRef::parse(&obj["cell"].as_str().unwrap_or("").replace('$', "")).unwrap_or_default();
    rep.text(r, 1, "", RStyle::default());
    r = table(&mut rep, r, &["Objective Cell", "Name", "Value"], vec![vec![obj["cell"].clone(), json!(cell_name(sh, objc)), obj["final"].clone()]])
        + 1;
    table(&mut rep, r, &["Variable Cell", "Name", "Value", "Lower Limit", "Objective Result", "Upper Limit", "Objective Result"], rows);
    rep.col_widths = vec![(0, 14.0), (1, 80.0), (2, 140.0), (3, 80.0), (4, 80.0), (5, 90.0), (6, 80.0), (7, 90.0)];
    Ok(rep)
}

fn population(wb: &Workbook, sh: &Sheet, last: &Json) -> std::result::Result<Report, String> {
    let pop = last["population"].as_array().cloned().unwrap_or_default();
    if pop.is_empty() {
        return Err("The Population report is only available after an Evolutionary solve.".into());
    }
    let mut rep = Report::new();
    let r = title(&mut rep, wb, &sh.name, "Population") + 1;
    let vars = last["variables"].as_array().cloned().unwrap_or_default();
    let rows = pop
        .iter()
        .enumerate()
        .map(|(j, v)| {
            let c = CellRef::parse(&v["cell"].as_str().unwrap_or("").replace('$', "")).unwrap_or_default();
            vec![
                v["cell"].clone(),
                json!(cell_name(sh, c)),
                vars.get(j).map(|x| x["final"].clone()).unwrap_or(Json::Null),
                v["mean"].clone(),
                v["sd"].clone(),
                v["max"].clone(),
                v["min"].clone(),
            ]
        })
        .collect();
    rep.text(r, 0, "Variable Cells", RStyle::default());
    table(&mut rep, r + 1, &["Cell", "Name", "Best Value", "Mean", "Standard Deviation", "Maximum Value", "Minimum Value"], rows);
    rep.col_widths = vec![(0, 14.0), (1, 64.0), (2, 140.0), (3, 80.0), (4, 80.0), (5, 110.0), (6, 90.0), (7, 90.0)];
    Ok(rep)
}

/// The next free "<kind> Report N" sheet name.
fn report_name(wb: &Workbook, kind: &str) -> String {
    (1..).map(|n| format!("{kind} Report {n}")).find(|n| wb.sheet_index(n).is_none()).unwrap_or_else(|| format!("{kind} Report"))
}

fn reports(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "data.solverReports";
    let last = s.solver_last.clone().ok_or_else(|| EngineError::Other("Run Solver first: the reports describe its last result.".into()))?;
    let model: SolverModel = serde_json::from_value(last["model"].clone()).map_err(|e| EngineError::Other(e.to_string()))?;
    let kinds: Vec<String> = p
        .get("reports")
        .and_then(Json::as_array)
        .ok_or_else(|| bad(CMD, "give `reports`: a list of answer, sensitivity, limits, population"))?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_ascii_lowercase))
        .collect();
    if kinds.is_empty() || kinds.len() > 4 {
        return Err(bad(CMD, "give between one and four reports"));
    }
    let outline = bool_param(p, "outline").unwrap_or(false);
    let model_sheet = {
        let d = s.doc()?;
        d.wb.sheet_index(last["sheet"].as_str().unwrap_or("")).unwrap_or(d.wb.active_sheet)
    };
    // Build every report first, so a refused one writes nothing.
    let mut built: Vec<(String, Report)> = Vec::new();
    {
        let d = s.doc()?;
        let sh = d.wb.sheet(model_sheet).ok_or(EngineError::NoDocument)?;
        for k in &kinds {
            let (label, rep) = match k.as_str() {
                "answer" => ("Answer", answer(&d.wb, sh, &last, &model)),
                "sensitivity" => ("Sensitivity", sensitivity(&d.wb, sh, &last).map_err(EngineError::Other)?),
                "limits" => ("Limits", limits(s, &last, &model).map_err(EngineError::Other)?),
                "population" => ("Population", population(&d.wb, sh, &last).map_err(EngineError::Other)?),
                other => return Err(bad(CMD, format!("unknown report `{other}`"))),
            };
            built.push((label.to_string(), rep));
        }
    }
    let model_name = s.doc()?.wb.sheet(model_sheet).map(|x| x.name.clone()).unwrap_or_default();
    let mut names = Vec::new();
    // Each new sheet goes before the active one: write in reverse so they end up in order.
    s.execute("sheet.activate", json!({"sheet": model_sheet})).ok();
    for (label, mut rep) in built.into_iter().rev() {
        if outline {
            let (rows, _) = rep.extent();
            if rows > 4 {
                rep.outline_rows.push((4, rows - 1));
            }
        }
        let name = report_name(&s.doc()?.wb, &label);
        report::write(s, &Target { kind: OutputKind::NewSheet { name: name.clone() }, origin: CellRef::new(0, 0) }, rep, false)?;
        names.push(name);
    }
    names.reverse();
    if let Some(i) = s.doc()?.wb.sheet_index(&model_name) {
        s.doc_mut()?.wb_switch_sheet(i);
    }
    Ok(json!({"sheets": names}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    fn v(s: &Session, sheet: &str, a1: &str) -> Value {
        let d = s.doc().unwrap();
        d.wb.sheet(d.wb.sheet_index(sheet).unwrap()).unwrap().value(CellRef::parse(a1).unwrap())
    }

    /// Wyndor with labels: products across, plants down.
    fn wyndor(s: &mut Session) {
        s.execute(
            "range.setValues",
            json!({"range": "A1", "values": [[null, "Doors", "Windows", "Used", "Available"], ["Batch", 0, 0, null, null], ["Plant 1", 1, 0, null, 4], ["Plant 2", 0, 2, null, 12], ["Plant 3", 3, 2, null, 18], ["Profit", 3, 5, null, null]]}),
        )
        .unwrap();
        for (c, f) in [
            ("D3", "=SUMPRODUCT(B3:C3,$B$2:$C$2)"),
            ("D4", "=SUMPRODUCT(B4:C4,$B$2:$C$2)"),
            ("D5", "=SUMPRODUCT(B5:C5,$B$2:$C$2)"),
            ("D6", "=SUMPRODUCT(B6:C6,$B$2:$C$2)"),
        ] {
            s.execute("cell.set", json!({"cell": c, "input": f})).unwrap();
        }
    }

    #[test]
    fn answer_sensitivity_and_limits_reports() {
        let mut s = session();
        wyndor(&mut s);
        let r = s
            .execute(
                "data.solver",
                json!({"objective": "$D$6", "goal": "max", "variables": "$B$2:$C$2", "method": "simplex", "constraints": [{"left": "$D$3:$D$5", "op": "<=", "right": "$E$3:$E$5"}]}),
            )
            .unwrap();
        assert_eq!(r["status"], 0, "{r}");
        let out = s.execute("data.solverReports", json!({"reports": ["answer", "sensitivity", "limits"]})).unwrap();
        assert_eq!(out["sheets"], json!(["Answer Report 1", "Sensitivity Report 1", "Limits Report 1"]));
        let d = s.doc().unwrap();
        let names: Vec<&str> = d.wb.sheets.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, vec!["Answer Report 1", "Sensitivity Report 1", "Limits Report 1", "Sheet1"]);
        assert_eq!(d.wb.active().unwrap().name, "Sheet1");
        assert_eq!(v(&s, "Answer Report 1", "A1"), Value::from("GridCraft Answer Report"));
        // Find the variable rows by their names.
        let ans = d.wb.sheet(0).unwrap();
        let find = |t: &str| ans.cells.iter().find(|(_, c)| c.value == Value::from(t)).map(|(r, _)| r);
        let doors = find("Batch Doors").expect("named from labels");
        assert_eq!(ans.value(CellRef::new(doors.row, doors.col + 2)), Value::Number(2.0));
        let binding = ans.cells.iter().filter(|(_, c)| c.value == Value::from("Binding")).count();
        assert_eq!(binding, 2);
        let sens = d.wb.sheet(1).unwrap();
        assert!(sens.cells.iter().any(|(_, c)| c.value == Value::from("Shadow")));
        assert!(sens.cells.iter().any(|(_, c)| c.value == Value::Number(1.5)), "plant 2's shadow price");
        let lim = d.wb.sheet(2).unwrap();
        // Doors at most 2 with windows fixed at 6 (plant 3): objective 36; at least 0: objective 30.
        assert!(lim.cells.iter().any(|(_, c)| c.value == Value::Number(30.0)));
        // Integer models have no sensitivity report.
        s.execute(
            "data.solver",
            json!({"constraints": [{"left": "$D$3:$D$5", "op": "<=", "right": "$E$3:$E$5"}, {"left": "$B$2:$C$2", "op": "int"}]}),
        )
        .unwrap();
        assert!(s.execute("data.solverReports", json!({"reports": ["sensitivity"]})).is_err());
        assert!(s.execute("data.solverReports", json!({"reports": ["population"]})).is_err());
        assert!(s.execute("data.solverReports", json!({"reports": []})).is_err());
    }

    #[test]
    fn reports_need_a_run() {
        let mut s = session();
        assert!(s.execute("data.solverReports", json!({"reports": ["answer"]})).is_err());
    }
}
