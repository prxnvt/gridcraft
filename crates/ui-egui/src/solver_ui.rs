//! Solver's dialogs: Solver Parameters (with its Add/Change Constraint, Options and Load/Save
//! views), Solver Results and Show Trial Solution. They edit the model through `data.solver*`
//! commands; solving runs as a background job.

use egui::vec2;
use serde_json::{Map, Value as Json, json};

use crate::SheetApp;
use crate::dialogs::{Dialog, ref_edit};
use crate::theme;

const METHODS: &[(&str, &str, &str)] = &[
    ("grg", "GRG Nonlinear", "For smooth nonlinear models. Finds a locally optimal solution; Multistart (in Options) searches for a better one."),
    ("simplex", "Simplex LP", "For linear models. Finds the best solution quickly, including with integer, binary and all-different constraints."),
    (
        "evolutionary",
        "Evolutionary",
        "For models that aren't smooth (IF, lookups, rounding). Searches a population of solutions; give every variable cell a lower and an upper bound.",
    ),
];

const OPS: &[(&str, &str)] = &[("<=", "<="), ("=", "="), (">=", ">="), ("int", "int"), ("bin", "bin"), ("dif", "dif")];

/// Opens Solver Parameters with the active sheet's model.
pub fn open(app: &mut SheetApp) -> Option<Dialog> {
    let mut model = app.session.run("data.solverModel", json!({})).unwrap_or_else(|_| json!({}));
    if model["objective"].as_str().is_none_or(str::is_empty)
        && let Some(d) = app.session.active()
    {
        let a = d.selection.active;
        model["objective"] = json!(format!("${}${}", gridcraft_engine::core::col_to_letters(a.col), a.row + 1));
    }
    let mut v = model.as_object().cloned().unwrap_or_default();
    v.insert("view".into(), json!("main"));
    v.insert("sel".into(), json!(-1));
    Some(Dialog::custom("solver", "Solver Parameters", Json::Object(v)))
}

fn text(d: &Dialog, k: &str) -> String {
    match d.values.get(k) {
        Some(Json::String(s)) => s.clone(),
        Some(Json::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn field(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32) {
    let mut s = text(d, key);
    let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(width).id_salt(("solver", key)));
    if r.gained_focus() {
        d.pick = None;
    }
    d.values.insert(key.into(), json!(s));
}

/// The model fields of the dialog, as command parameters.
fn model_params(d: &Dialog) -> Json {
    let mut m = Map::new();
    for k in ["objective", "goal", "variables", "constraints", "nonNegative", "method", "options"] {
        if let Some(v) = d.values.get(k) {
            m.insert(k.into(), v.clone());
        }
    }
    let vo = text(d, "valueOf");
    m.insert("valueOf".into(), json!(vo.trim().parse::<f64>().unwrap_or(0.0)));
    Json::Object(m)
}

fn constraints(d: &Dialog) -> Vec<Json> {
    d.values.get("constraints").and_then(Json::as_array).cloned().unwrap_or_default()
}

fn constraint_text(c: &Json) -> String {
    let op = c["op"].as_str().unwrap_or("<=");
    let right = match op {
        "int" => "integer".to_string(),
        "bin" => "binary".to_string(),
        "dif" => "AllDifferent".to_string(),
        _ => match &c["right"] {
            Json::String(s) => s.clone(),
            Json::Number(n) => n.to_string(),
            _ => String::new(),
        },
    };
    format!("{} {} {}", c["left"].as_str().unwrap_or(""), if matches!(op, "int" | "bin" | "dif") { "=" } else { op }, right)
}

/// Solver Parameters.
pub fn show(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, open: &mut bool) {
    match text(d, "view").as_str() {
        "constraint" => constraint_view(app, ui, d),
        "options" => options_view(ui, d),
        "loadSave" => load_save_view(app, ui, d),
        _ => main_view(app, ui, d, open),
    }
}

fn main_view(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, open: &mut bool) {
    d.title = "Solver Parameters".into();
    let t = theme::Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, 18.0), egui::Label::new("Set Objective:"));
        ref_edit(app, ui, d, "objective", 260.0);
    });
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, 18.0), egui::Label::new("To:"));
        let goal = text(d, "goal");
        for (v, l) in [("max", "Max"), ("min", "Min"), ("value", "Value Of:")] {
            if ui.radio(goal == v, l).clicked() {
                d.values.insert("goal".into(), json!(v));
            }
        }
        ui.add_enabled_ui(goal == "value", |ui| field(ui, d, "valueOf", 70.0));
    });
    ui.add_space(4.0);
    ui.label("By Changing Variable Cells:");
    ref_edit(app, ui, d, "variables", 470.0);
    ui.add_space(4.0);
    ui.label("Subject to the Constraints:");
    let list = constraints(d);
    let sel = d.values.get("sel").and_then(Json::as_i64).unwrap_or(-1);
    ui.horizontal(|ui| {
        egui::Frame::NONE.fill(t.input_bg).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(4.0).show(ui, |ui| {
            ui.set_min_size(vec2(340.0, 130.0));
            ui.set_max_width(340.0);
            egui::ScrollArea::vertical().max_height(130.0).id_salt("solver_cons").show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    for (i, c) in list.iter().enumerate() {
                        let r = ui.selectable_label(sel == i as i64, constraint_text(c));
                        if r.clicked() {
                            d.values.insert("sel".into(), json!(i));
                        }
                        if r.double_clicked() {
                            begin_edit(d, Some(i));
                        }
                    }
                });
            });
        });
        ui.vertical(|ui| {
            let w = vec2(96.0, 22.0);
            if ui.add_sized(w, egui::Button::new("Add")).clicked() {
                begin_edit(d, None);
            }
            if ui.add_enabled(sel >= 0, egui::Button::new("Change").min_size(w)).clicked() {
                begin_edit(d, Some(sel as usize));
            }
            if ui.add_enabled(sel >= 0, egui::Button::new("Delete").min_size(w)).clicked() {
                let mut l = constraints(d);
                if (sel as usize) < l.len() {
                    l.remove(sel as usize);
                }
                d.values.insert("constraints".into(), json!(l));
                d.values.insert("sel".into(), json!(-1));
            }
            ui.add_space(6.0);
            if ui.add_sized(w, egui::Button::new("Reset All")).clicked() {
                let _ = app.session.run("data.solverReset", json!({}));
                if let Some(fresh) = open_fresh(app) {
                    *d = fresh;
                }
            }
            if ui.add_sized(w, egui::Button::new("Load/Save")).clicked() {
                d.values.insert("view".into(), json!("loadSave"));
            }
        });
    });
    let mut nn = d.values.get("nonNegative").and_then(Json::as_bool).unwrap_or(true);
    if ui.checkbox(&mut nn, "Make Unconstrained Variables Non-Negative").changed() {
        d.values.insert("nonNegative".into(), json!(nn));
    }
    ui.horizontal(|ui| {
        ui.label("Select a Solving Method:");
        let cur = text(d, "method");
        let shown = METHODS.iter().find(|m| m.0 == cur).map(|m| m.1).unwrap_or("GRG Nonlinear");
        egui::ComboBox::from_id_salt("solver_method").selected_text(shown).width(150.0).show_ui(ui, |ui| {
            for (v, l, _) in METHODS {
                if ui.selectable_label(cur == *v, *l).clicked() {
                    d.values.insert("method".into(), json!(v));
                }
            }
        });
        if ui.button("Options").clicked() {
            d.values.insert("optionsBackup".into(), d.values.get("options").cloned().unwrap_or(json!({})));
            d.values.insert("view".into(), json!("options"));
            d.values.insert("optTab".into(), json!(text(d, "method")));
        }
    });
    let cur = text(d, "method");
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(470.0);
        ui.label(egui::RichText::new("Solving Method").strong());
        ui.add(egui::Label::new(METHODS.iter().find(|m| m.0 == cur).map(|m| m.2).unwrap_or(METHODS[0].2)).wrap());
    });
    ui.add_space(6.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button("  Close  ").clicked() {
            app.run_or_alert("data.solverSetModel", model_params(d));
            *open = false;
        }
        let solve = ui.add(egui::Button::new(egui::RichText::new("  Solve  ").color(egui::Color32::WHITE)).fill(theme::Tokens::get(ui.ctx()).accent));
        if solve.clicked() {
            match app.session.run("data.solverStart", model_params(d)) {
                Ok(_) => *open = false,
                Err(e) => app.message = Some(("Solver".into(), crate::clean_error(&e))),
            }
        }
    });
}

fn open_fresh(app: &mut SheetApp) -> Option<Dialog> {
    open(app)
}

fn begin_edit(d: &mut Dialog, index: Option<usize>) {
    let list = constraints(d);
    let c = index.and_then(|i| list.get(i).cloned()).unwrap_or(json!({"left": "", "op": "<=", "right": ""}));
    d.values.insert("view".into(), json!("constraint"));
    d.values.insert("editing".into(), json!(index.map(|i| i as i64).unwrap_or(-1)));
    d.values.insert("cLeft".into(), c["left"].clone());
    d.values.insert("cOp".into(), c["op"].clone());
    let right = match &c["right"] {
        Json::Number(n) => json!(n.to_string()),
        other => other.clone(),
    };
    d.values.insert("cRight".into(), right);
    d.pick = None;
}

/// Saves the constraint being edited; false when it isn't complete.
fn save_constraint(app: &mut SheetApp, d: &mut Dialog) -> bool {
    let (left, op, right) = (text(d, "cLeft"), text(d, "cOp"), text(d, "cRight"));
    let keyword = matches!(op.as_str(), "int" | "bin" | "dif");
    if left.trim().is_empty() || (!keyword && right.trim().is_empty()) {
        app.message = Some(("Add Constraint".into(), "Enter a cell reference and a constraint.".into()));
        return false;
    }
    let c = json!({"left": left.trim(), "op": op, "right": if keyword { String::new() } else { right.trim().to_string() }});
    let mut list = constraints(d);
    match d.values.get("editing").and_then(Json::as_i64) {
        Some(i) if i >= 0 && (i as usize) < list.len() => list[i as usize] = c,
        _ => list.push(c),
    }
    d.values.insert("constraints".into(), json!(list));
    true
}

fn constraint_view(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let editing = d.values.get("editing").and_then(Json::as_i64).unwrap_or(-1);
    d.title = if editing >= 0 { "Change Constraint".into() } else { "Add Constraint".into() };
    let op = text(d, "cOp");
    let keyword = matches!(op.as_str(), "int" | "bin" | "dif");
    egui::Grid::new("solver_con").num_columns(3).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
        ui.label("Cell Reference:");
        ui.label("");
        ui.label("Constraint:");
        ui.end_row();
        ref_edit(app, ui, d, "cLeft", 180.0);
        egui::ComboBox::from_id_salt("solver_op").selected_text(op.clone()).width(56.0).show_ui(ui, |ui| {
            for (v, l) in OPS {
                if ui.selectable_label(op == *v, *l).clicked() {
                    d.values.insert("cOp".into(), json!(v));
                }
            }
        });
        if keyword {
            let shown = match op.as_str() {
                "int" => "integer",
                "bin" => "binary",
                _ => "AllDifferent",
            };
            ui.add_enabled(false, egui::TextEdit::singleline(&mut shown.to_string()).desired_width(150.0));
        } else {
            ref_edit(app, ui, d, "cRight", 180.0);
        }
        ui.end_row();
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("   OK   ").clicked() && save_constraint(app, d) {
            d.values.insert("view".into(), json!("main"));
        }
        if ui.button("  Add  ").clicked() && save_constraint(app, d) {
            begin_edit(d, None);
        }
        if ui.button(" Cancel ").clicked() {
            d.values.insert("view".into(), json!("main"));
        }
    });
}

fn opt(d: &Dialog, k: &str) -> Json {
    d.values.get("options").and_then(|o| o.get(k)).cloned().unwrap_or(Json::Null)
}

fn set_opt(d: &mut Dialog, k: &str, v: Json) {
    let mut o = d.values.get("options").cloned().unwrap_or(json!({}));
    o[k] = v;
    d.values.insert("options".into(), o);
}

fn opt_num(ui: &mut egui::Ui, d: &mut Dialog, label: &str, key: &str, integer: bool) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(210.0, 18.0), egui::Label::new(label));
        let cur = opt(d, key);
        let shown = match &cur {
            Json::Number(n) if integer && n.as_f64() == Some(0.0) && matches!(key, "iterations" | "maxSubproblems" | "maxFeasible") => String::new(),
            Json::Number(n) if key == "maxTime" && n.as_f64() == Some(0.0) => String::new(),
            Json::Number(n) => n.to_string(),
            _ => String::new(),
        };
        let buf_key = format!("buf_{key}");
        let mut s = d.values.get(&buf_key).and_then(Json::as_str).map(str::to_string).unwrap_or(shown);
        if ui.add(egui::TextEdit::singleline(&mut s).desired_width(90.0)).changed() {
            let t = s.trim();
            let v = if t.is_empty() {
                json!(0)
            } else {
                t.parse::<f64>().map(|x| if integer { json!(x.max(0.0) as u64) } else { json!(x) }).unwrap_or(cur.clone())
            };
            set_opt(d, key, v);
        }
        d.values.insert(buf_key, json!(s));
    });
}

fn opt_check(ui: &mut egui::Ui, d: &mut Dialog, label: &str, key: &str) {
    let mut b = opt(d, key).as_bool().unwrap_or(false);
    if ui.checkbox(&mut b, label).changed() {
        set_opt(d, key, json!(b));
    }
}

fn options_view(ui: &mut egui::Ui, d: &mut Dialog) {
    d.title = "Options".into();
    let tab = text(d, "optTab");
    ui.horizontal(|ui| {
        for (v, l) in [("simplex", "All Methods"), ("grg", "GRG Nonlinear"), ("evolutionary", "Evolutionary")] {
            let on = tab == v || (v == "simplex" && !matches!(tab.as_str(), "grg" | "evolutionary"));
            if ui.selectable_label(on, l).clicked() {
                d.values.insert("optTab".into(), json!(v));
            }
        }
    });
    ui.separator();
    match tab.as_str() {
        "grg" => {
            opt_num(ui, d, "Convergence:", "convergence", false);
            ui.label("Derivatives:");
            ui.horizontal(|ui| {
                let central = opt(d, "centralDerivatives").as_bool().unwrap_or(false);
                if ui.radio(!central, "Forward").clicked() {
                    set_opt(d, "centralDerivatives", json!(false));
                }
                if ui.radio(central, "Central").clicked() {
                    set_opt(d, "centralDerivatives", json!(true));
                }
            });
            ui.label(egui::RichText::new("Multistart").strong());
            opt_check(ui, d, "Use Multistart", "multistart");
            opt_num(ui, d, "Population Size:", "populationSize", true);
            opt_num(ui, d, "Random Seed:", "randomSeed", true);
            opt_check(ui, d, "Require Bounds on Variables", "requireBounds");
        }
        "evolutionary" => {
            opt_num(ui, d, "Convergence:", "evolutionaryConvergence", false);
            opt_num(ui, d, "Mutation Rate:", "mutationRate", false);
            opt_num(ui, d, "Population Size:", "populationSize", true);
            opt_num(ui, d, "Random Seed:", "randomSeed", true);
            opt_num(ui, d, "Maximum Time without improvement:", "maxTimeWithoutImprovement", false);
            opt_check(ui, d, "Require Bounds on Variables", "requireBounds");
        }
        _ => {
            opt_num(ui, d, "Constraint Precision:", "constraintPrecision", false);
            opt_check(ui, d, "Use Automatic Scaling", "automaticScaling");
            opt_check(ui, d, "Show Iteration Results", "showIterations");
            ui.label(egui::RichText::new("Solving with Integer Constraints").strong());
            opt_check(ui, d, "Ignore Integer Constraints", "ignoreInteger");
            opt_num(ui, d, "Integer Optimality (%):", "integerOptimality", false);
            ui.label(egui::RichText::new("Solving Limits").strong());
            opt_num(ui, d, "Max Time (Seconds):", "maxTime", false);
            opt_num(ui, d, "Iterations:", "iterations", true);
            ui.label(egui::RichText::new("Evolutionary and Integer Constraints:").strong());
            opt_num(ui, d, "Max Subproblems:", "maxSubproblems", true);
            opt_num(ui, d, "Max Feasible Solutions:", "maxFeasible", true);
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("   OK   ").clicked() {
            d.values.retain(|k, _| !k.starts_with("buf_"));
            d.values.insert("view".into(), json!("main"));
        }
        if ui.button(" Cancel ").clicked() {
            let backup = d.values.get("optionsBackup").cloned().unwrap_or(json!({}));
            d.values.insert("options".into(), backup);
            d.values.retain(|k, _| !k.starts_with("buf_"));
            d.values.insert("view".into(), json!("main"));
        }
    });
}

fn load_save_view(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog) {
    d.title = "Load/Save Model".into();
    ui.label("Select a range to load a model from, or the first cell to save the model to:");
    ref_edit(app, ui, d, "lsRange", 300.0);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let range = text(d, "lsRange");
        if ui.button(" Load ").clicked() {
            match app.session.run("data.solverLoadModel", json!({"from": range})) {
                Ok(_) => {
                    if let Some(fresh) = open_fresh(app) {
                        *d = fresh;
                    }
                }
                Err(e) => app.message = Some(("Load Model".into(), crate::clean_error(&e))),
            }
        }
        if ui.button(" Save ").clicked() {
            let first = range.split(':').next().unwrap_or("").to_string();
            let res =
                app.session.run("data.solverSetModel", model_params(d)).and_then(|_| app.session.run("data.solverSaveModel", json!({"to": first})));
            match res {
                Ok(_) => {
                    d.values.insert("view".into(), json!("main"));
                }
                Err(e) => app.message = Some(("Save Model".into(), crate::clean_error(&e))),
            }
        }
        if ui.button(" Cancel ").clicked() {
            d.values.insert("view".into(), json!("main"));
        }
    });
}

/// Solver Results: keep or restore, reports, and saving the result as a scenario.
pub fn results(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, open: &mut bool) {
    let r = Json::Object(d.values.clone());
    let feasible = r["feasible"].as_bool().unwrap_or(false);
    ui.label(egui::RichText::new(r["message"].as_str().unwrap_or("")).strong());
    ui.add_space(6.0);
    let evolutionary = r["method"] == "Evolutionary";
    let integers = r["integers"].as_bool().unwrap_or(false);
    let available: Vec<(&str, &str)> = {
        let mut v = vec![("answer", "Answer")];
        if !integers && !r["sensitivity"].is_null() {
            v.push(("sensitivity", "Sensitivity"));
        }
        if !integers && !evolutionary && feasible {
            v.push(("limits", "Limits"));
        }
        if evolutionary {
            v.push(("population", "Population"));
        }
        v
    };
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            let keep = d.values.get("choice").and_then(Json::as_str).unwrap_or(if r["kept"] == json!(true) { "keep" } else { "restore" }).to_string();
            if ui.radio(keep == "keep", "Keep Solver Solution").clicked() {
                d.values.insert("choice".into(), json!("keep"));
            }
            if ui.radio(keep == "restore", "Restore Original Values").clicked() {
                d.values.insert("choice".into(), json!("restore"));
            }
            ui.add_space(10.0);
            let mut back = d.values.get("back").and_then(Json::as_bool).unwrap_or(false);
            if ui.checkbox(&mut back, "Return to Solver Parameters Dialog").changed() {
                d.values.insert("back".into(), json!(back));
            }
        });
        ui.add_space(20.0);
        ui.vertical(|ui| {
            ui.label("Reports");
            let mut chosen: Vec<String> = d
                .values
                .get("reports")
                .and_then(Json::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            for (k, l) in &available {
                let on = chosen.iter().any(|c| c == k);
                if ui.selectable_label(on, *l).clicked() {
                    if on {
                        chosen.retain(|c| c != k);
                    } else {
                        chosen.push(k.to_string());
                    }
                }
            }
            d.values.insert("reports".into(), json!(chosen));
            let mut outline = d.values.get("outline").and_then(Json::as_bool).unwrap_or(false);
            if ui.checkbox(&mut outline, "Outline Reports").changed() {
                d.values.insert("outline".into(), json!(outline));
            }
        });
    });
    ui.add_space(6.0);
    // Save Scenario.
    ui.horizontal(|ui| {
        ui.label("Scenario name:");
        let mut name = d.values.get("scenario").and_then(Json::as_str).unwrap_or("").to_string();
        ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0));
        d.values.insert("scenario".into(), json!(name.clone()));
        if ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Save Scenario")).clicked() {
            let cells: Vec<String> =
                r["variables"].as_array().map(|a| a.iter().filter_map(|v| v["cell"].as_str().map(str::to_string)).collect()).unwrap_or_default();
            let values: Vec<Json> = r["variables"].as_array().map(|a| a.iter().map(|v| v["final"].clone()).collect()).unwrap_or_default();
            match app.session.run("data.scenarioAdd", json!({"name": name.trim(), "changing": cells.join(","), "values": values, "replace": true})) {
                Ok(_) => app.toast = Some((format!("Saved scenario \u{201c}{}\u{201d}", name.trim()), crate::now_ms())),
                Err(e) => app.message = Some(("Save Scenario".into(), crate::clean_error(&e))),
            }
        }
    });
    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(" Cancel ").clicked() {
            app.run_or_alert("data.solverRestore", json!({}));
            *open = false;
        }
        let ok = ui.add(egui::Button::new(egui::RichText::new("   OK   ").color(egui::Color32::WHITE)).fill(theme::Tokens::get(ui.ctx()).accent));
        if ok.clicked() {
            if d.values.get("choice").and_then(Json::as_str) == Some("restore") {
                app.run_or_alert("data.solverRestore", json!({}));
            }
            let reports = d.values.get("reports").cloned().unwrap_or(json!([]));
            if reports.as_array().is_some_and(|a| !a.is_empty()) {
                let outline = d.values.get("outline").cloned().unwrap_or(json!(false));
                app.run_or_alert("data.solverReports", json!({"reports": reports, "outline": outline}));
            }
            if d.values.get("back").and_then(Json::as_bool) == Some(true) {
                app.dialog_next = self::open(app);
            }
            *open = false;
        }
    });
}

/// Show Trial Solution: the job is paused with the trial values on the sheet.
pub fn trial(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, open: &mut bool) {
    let it = d.values.get("iteration").and_then(Json::as_u64).unwrap_or(0);
    ui.label(egui::RichText::new("Solver paused: the current solution is shown on the sheet.").strong());
    if it > 0 {
        ui.label(format!("Trial Solution: {it}"));
    }
    if let Some(o) = d.values.get("objective").and_then(Json::as_f64) {
        ui.label(format!("Objective: {}", gridcraft_engine::core::number_to_text(o)));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(" Continue ").clicked() {
            app.session.resume_job();
            *open = false;
        }
        if ui.button("  Stop  ").clicked() {
            app.session.stop_job();
            app.after_engine();
            *open = false;
        }
    });
}
