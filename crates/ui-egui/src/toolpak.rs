//! Data › Data Analysis: the tool list and one form per Analysis ToolPak tool. The forms are
//! declared as data ([`form`]) and drawn by one renderer; OK runs the tool's `analysis.*`
//! command with the form's values.

use egui::vec2;
use serde_json::{Map, Value as Json, json};

use crate::SheetApp;
use crate::dialogs::{Dialog, ref_edit};

/// A control on a tool's form.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ctl {
    /// A range picked on the grid: (param, label).
    Ref(&'static str, &'static str),
    /// Grouped By: Columns / Rows.
    Grouped,
    Check(&'static str, &'static str),
    /// A number field: (param, label).
    Num(&'static str, &'static str),
    /// A check box enabling a number: (check key, label, number param).
    CheckNum(&'static str, &'static str, &'static str),
    /// Radio buttons: (param, label, [(value, text)]).
    Radio(&'static str, &'static str, &'static [(&'static str, &'static str)]),
    /// Random Number Generation's distribution and its parameters.
    Distribution,
    /// Output Range / New Worksheet Ply / New Workbook (`range_only`: just Output Range).
    Output {
        range_only: bool,
    },
    /// A section title.
    Heading(&'static str),
}

/// A tool's form: command, title, controls and default values.
pub struct Form {
    pub command: &'static str,
    pub title: &'static str,
    pub controls: Vec<Ctl>,
    pub defaults: Json,
}

/// The form for the tool `command`.
pub fn form(command: &str) -> Option<Form> {
    use Ctl::*;
    let input = |label: &'static str| Ref("input", label);
    let out = Output { range_only: false };
    let (title, controls, defaults): (&'static str, Vec<Ctl>, Json) = match command {
        "analysis.anovaSingleFactor" => (
            "Anova: Single Factor",
            vec![
                Heading("Input"),
                input("Input Range:"),
                Grouped,
                Check("labels", "Labels in first row"),
                Num("alpha", "Alpha:"),
                Heading("Output options"),
                out,
            ],
            json!({"alpha": "0.05"}),
        ),
        "analysis.anovaTwoFactorWithReplication" => (
            "Anova: Two-Factor With Replication",
            vec![
                Heading("Input"),
                input("Input Range:"),
                Num("rowsPerSample", "Rows per sample:"),
                Num("alpha", "Alpha:"),
                Heading("Output options"),
                out,
            ],
            json!({"alpha": "0.05", "rowsPerSample": ""}),
        ),
        "analysis.anovaTwoFactorWithoutReplication" => (
            "Anova: Two-Factor Without Replication",
            vec![Heading("Input"), input("Input Range:"), Check("labels", "Labels"), Num("alpha", "Alpha:"), Heading("Output options"), out],
            json!({"alpha": "0.05"}),
        ),
        "analysis.correlation" => (
            "Correlation",
            vec![Heading("Input"), input("Input Range:"), Grouped, Check("labels", "Labels in first row"), Heading("Output options"), out],
            json!({}),
        ),
        "analysis.covariance" => (
            "Covariance",
            vec![Heading("Input"), input("Input Range:"), Grouped, Check("labels", "Labels in first row"), Heading("Output options"), out],
            json!({}),
        ),
        "analysis.descriptiveStatistics" => (
            "Descriptive Statistics",
            vec![
                Heading("Input"),
                input("Input Range:"),
                Grouped,
                Check("labels", "Labels in first row"),
                Heading("Output options"),
                out,
                Check("summary", "Summary statistics"),
                CheckNum("useConfidence", "Confidence Level for Mean:", "confidence"),
                CheckNum("useKthLargest", "Kth Largest:", "kthLargest"),
                CheckNum("useKthSmallest", "Kth Smallest:", "kthSmallest"),
            ],
            json!({"summary": true, "confidence": "95", "kthLargest": "1", "kthSmallest": "1"}),
        ),
        "analysis.exponentialSmoothing" => (
            "Exponential Smoothing",
            vec![
                Heading("Input"),
                input("Input Range:"),
                Num("damping", "Damping factor:"),
                Check("labels", "Labels"),
                Heading("Output options"),
                Output { range_only: true },
                Check("chartOutput", "Chart Output"),
                Check("standardErrors", "Standard Errors"),
            ],
            json!({"damping": "0.3", "outputMode": "range"}),
        ),
        "analysis.fTest" => (
            "F-Test Two-Sample for Variances",
            vec![
                Heading("Input"),
                Ref("input1", "Variable 1 Range:"),
                Ref("input2", "Variable 2 Range:"),
                Check("labels", "Labels"),
                Num("alpha", "Alpha:"),
                Heading("Output options"),
                out,
            ],
            json!({"alpha": "0.05"}),
        ),
        "analysis.fourierAnalysis" => (
            "Fourier Analysis",
            vec![
                Heading("Input"),
                input("Input Range:"),
                Check("labels", "Labels in First Row"),
                Heading("Output options"),
                out,
                Check("inverse", "Inverse"),
            ],
            json!({}),
        ),
        "analysis.histogram" => (
            "Histogram",
            vec![
                Heading("Input"),
                input("Input Range:"),
                Ref("bins", "Bin Range:"),
                Check("labels", "Labels"),
                Heading("Output options"),
                out,
                Check("pareto", "Pareto (sorted histogram)"),
                Check("cumulative", "Cumulative Percentage"),
                Check("chartOutput", "Chart Output"),
            ],
            json!({}),
        ),
        "analysis.movingAverage" => (
            "Moving Average",
            vec![
                Heading("Input"),
                input("Input Range:"),
                Check("labels", "Labels in First Row"),
                Num("interval", "Interval:"),
                Heading("Output options"),
                Output { range_only: true },
                Check("chartOutput", "Chart Output"),
                Check("standardErrors", "Standard Errors"),
            ],
            json!({"interval": "3", "outputMode": "range"}),
        ),
        "analysis.randomNumberGeneration" => (
            "Random Number Generation",
            vec![
                Num("variables", "Number of Variables:"),
                Num("count", "Number of Random Numbers:"),
                Distribution,
                Num("seed", "Random Seed:"),
                Heading("Output options"),
                out,
            ],
            json!({"distribution": "uniform", "from": "0", "to": "1", "mean": "0", "sd": "1", "p": "0.5", "trials": "10", "lambda": "1", "step": "1", "repeatEach": "1", "repeatSequence": "1"}),
        ),
        "analysis.rankAndPercentile" => (
            "Rank and Percentile",
            vec![Heading("Input"), input("Input Range:"), Grouped, Check("labels", "Labels in first row"), Heading("Output options"), out],
            json!({}),
        ),
        "analysis.regression" => (
            "Regression",
            vec![
                Heading("Input"),
                Ref("inputY", "Input Y Range:"),
                Ref("inputX", "Input X Range:"),
                Check("labels", "Labels"),
                Check("constantIsZero", "Constant is Zero"),
                CheckNum("useConfidence", "Confidence Level:", "confidence"),
                Heading("Output options"),
                out,
                Heading("Residuals"),
                Check("residuals", "Residuals"),
                Check("standardizedResiduals", "Standardized Residuals"),
                Check("residualPlots", "Residual Plots"),
                Check("lineFitPlots", "Line Fit Plots"),
                Heading("Normal Probability"),
                Check("normalProbabilityPlots", "Normal Probability Plots"),
            ],
            json!({"confidence": "95"}),
        ),
        "analysis.sampling" => (
            "Sampling",
            vec![
                Heading("Input"),
                input("Input Range:"),
                Check("labels", "Labels"),
                Heading("Sampling Method"),
                Radio("method", "", &[("periodic", "Periodic"), ("random", "Random")]),
                Num("period", "Period:"),
                Num("samples", "Number of Samples:"),
                Heading("Output options"),
                out,
            ],
            json!({"method": "periodic", "period": "", "samples": ""}),
        ),
        "analysis.tTestPaired" | "analysis.tTestEqualVariances" | "analysis.tTestUnequalVariances" => (
            match command {
                "analysis.tTestPaired" => "t-Test: Paired Two Sample for Means",
                "analysis.tTestEqualVariances" => "t-Test: Two-Sample Assuming Equal Variances",
                _ => "t-Test: Two-Sample Assuming Unequal Variances",
            },
            vec![
                Heading("Input"),
                Ref("input1", "Variable 1 Range:"),
                Ref("input2", "Variable 2 Range:"),
                Num("hypothesizedDifference", "Hypothesized Mean Difference:"),
                Check("labels", "Labels"),
                Num("alpha", "Alpha:"),
                Heading("Output options"),
                out,
            ],
            json!({"alpha": "0.05", "hypothesizedDifference": ""}),
        ),
        "analysis.zTest" => (
            "z-Test: Two Sample for Means",
            vec![
                Heading("Input"),
                Ref("input1", "Variable 1 Range:"),
                Ref("input2", "Variable 2 Range:"),
                Num("hypothesizedDifference", "Hypothesized Mean Difference:"),
                Num("variance1", "Variable 1 Variance (known):"),
                Num("variance2", "Variable 2 Variance (known):"),
                Check("labels", "Labels"),
                Num("alpha", "Alpha:"),
                Heading("Output options"),
                out,
            ],
            json!({"alpha": "0.05", "hypothesizedDifference": "", "variance1": "", "variance2": ""}),
        ),
        _ => return None,
    };
    let command = gridcraft_engine::cmd::analysis::TOOLS.iter().find(|(id, _)| *id == command).map(|(id, _)| *id)?;
    Some(Form { command, title, controls, defaults })
}

/// Opens the form for `command` as a dialog.
pub fn open(command: &str) -> Option<Dialog> {
    let f = form(command)?;
    let mut values = f.defaults.as_object().cloned().unwrap_or_default();
    values.insert("tool".into(), json!(f.command));
    values.entry("groupedBy").or_insert(json!("columns"));
    values.entry("outputMode").or_insert(json!("newSheet"));
    Some(Dialog::custom("toolpak", f.title, Json::Object(values)))
}

const DISTRIBUTIONS: &[(&str, &str)] = &[
    ("uniform", "Uniform"),
    ("normal", "Normal"),
    ("bernoulli", "Bernoulli"),
    ("binomial", "Binomial"),
    ("poisson", "Poisson"),
    ("patterned", "Patterned"),
    ("discrete", "Discrete"),
];

fn text_of(d: &Dialog, k: &str) -> String {
    match d.values.get(k) {
        Some(Json::String(s)) => s.clone(),
        Some(Json::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn text_field(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32) {
    let mut s = text_of(d, key);
    let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(width).id_salt(("tp", key)));
    if r.gained_focus() {
        d.pick = None;
    }
    d.values.insert(key.into(), json!(s));
}

fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    let mut b = d.values.get(key).and_then(Json::as_bool).unwrap_or(false);
    if ui.checkbox(&mut b, label).changed() {
        d.values.insert(key.into(), json!(b));
    }
}

fn radio(ui: &mut egui::Ui, d: &mut Dialog, key: &str, options: &[(&str, &str)]) {
    let cur = text_of(d, key);
    ui.horizontal(|ui| {
        for (v, t) in options {
            if ui.radio(cur == *v, *t).clicked() {
                d.values.insert(key.into(), json!(v));
            }
        }
    });
}

fn heading(ui: &mut egui::Ui, t: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(t).strong());
}

fn distribution(app: &SheetApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let cur = text_of(d, "distribution");
    ui.horizontal(|ui| {
        ui.label("Distribution:");
        let shown = DISTRIBUTIONS.iter().find(|(v, _)| *v == cur).map(|(_, t)| *t).unwrap_or("Uniform");
        egui::ComboBox::from_id_salt("tp_dist").selected_text(shown).show_ui(ui, |ui| {
            for (v, t) in DISTRIBUTIONS {
                if ui.selectable_label(cur == *v, *t).clicked() {
                    d.values.insert("distribution".into(), json!(v));
                }
            }
        });
    });
    heading(ui, "Parameters");
    let row = |ui: &mut egui::Ui, d: &mut Dialog, label: &str, key: &str| {
        ui.horizontal(|ui| {
            ui.label(label);
            text_field(ui, d, key, 80.0);
        });
    };
    match cur.as_str() {
        "normal" => {
            row(ui, d, "Mean =", "mean");
            row(ui, d, "Standard deviation =", "sd");
        }
        "bernoulli" => row(ui, d, "p Value =", "p"),
        "binomial" => {
            row(ui, d, "p Value =", "p");
            row(ui, d, "Number of trials =", "trials");
        }
        "poisson" => row(ui, d, "Lambda =", "lambda"),
        "patterned" => {
            ui.horizontal(|ui| {
                ui.label("From");
                text_field(ui, d, "from", 60.0);
                ui.label("to");
                text_field(ui, d, "to", 60.0);
            });
            row(ui, d, "in steps of", "step");
            row(ui, d, "repeating each number", "repeatEach");
            row(ui, d, "repeating the sequence", "repeatSequence");
        }
        "discrete" => {
            ui.label("Value and Probability Input Range:");
            ref_edit(app, ui, d, "values", 300.0);
        }
        _ => {
            ui.horizontal(|ui| {
                ui.label("Between");
                text_field(ui, d, "from", 60.0);
                ui.label("and");
                text_field(ui, d, "to", 60.0);
            });
        }
    }
}

fn output(app: &SheetApp, ui: &mut egui::Ui, d: &mut Dialog, range_only: bool) {
    let mode = text_of(d, "outputMode");
    egui::Grid::new("tp_out").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
        if range_only {
            ui.label("Output Range:");
            d.values.insert("outputMode".into(), json!("range"));
        } else if ui.radio(mode == "range", "Output Range:").clicked() {
            d.values.insert("outputMode".into(), json!("range"));
        }
        ref_edit(app, ui, d, "outputRange", 200.0);
        ui.end_row();
        if !range_only {
            if ui.radio(mode == "newSheet", "New Worksheet Ply:").clicked() {
                d.values.insert("outputMode".into(), json!("newSheet"));
            }
            text_field(ui, d, "newSheetName", 170.0);
            ui.end_row();
            if ui.radio(mode == "newWorkbook", "New Workbook").clicked() {
                d.values.insert("outputMode".into(), json!("newWorkbook"));
            }
            ui.end_row();
        }
    });
    // Picking an output range on the grid chooses Output Range.
    if d.pick.as_deref() == Some("outputRange") && mode != "range" && !text_of(d, "outputRange").is_empty() {
        d.values.insert("outputMode".into(), json!("range"));
    }
}

/// Draws the tool form; OK runs the tool.
pub fn show(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let tool = text_of(d, "tool");
    let Some(f) = form(&tool) else {
        *open = false;
        return;
    };
    for c in &f.controls {
        match *c {
            Ctl::Heading(t) => heading(ui, t),
            Ctl::Ref(key, label) => {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(130.0, 18.0), egui::Label::new(label));
                    ref_edit(app, ui, d, key, 220.0);
                });
            }
            Ctl::Grouped => {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(130.0, 18.0), egui::Label::new("Grouped By:"));
                    radio(ui, d, "groupedBy", &[("columns", "Columns"), ("rows", "Rows")]);
                });
            }
            Ctl::Check(key, label) => check(ui, d, key, label),
            Ctl::Num(key, label) => {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(130.0, 18.0), egui::Label::new(label));
                    text_field(ui, d, key, 80.0);
                });
            }
            Ctl::CheckNum(ck, label, key) => {
                ui.horizontal(|ui| {
                    check(ui, d, ck, label);
                    let on = d.values.get(ck).and_then(Json::as_bool).unwrap_or(false);
                    ui.add_enabled_ui(on, |ui| text_field(ui, d, key, 60.0));
                });
            }
            Ctl::Radio(key, label, options) => {
                if !label.is_empty() {
                    ui.label(label);
                }
                radio(ui, d, key, options);
            }
            Ctl::Distribution => distribution(app, ui, d),
            Ctl::Output { range_only } => output(app, ui, d, range_only),
        }
    }
    let mut keep = true;
    crate::dialogs::ok_cancel(ui, confirm, &mut keep);
    if !keep {
        *open = false;
        return;
    }
    if *confirm {
        *confirm = false;
        let params = params(&f, &d.values);
        match app.run_confirming(f.title, f.command, params) {
            crate::Outcome::Done(_) | crate::Outcome::Asked => *open = false,
            crate::Outcome::Failed => {}
        }
    }
}

/// The command parameters from the form values.
pub fn params(f: &Form, values: &Map<String, Json>) -> Json {
    let mut p = Map::new();
    let text = |k: &str| match values.get(k) {
        Some(Json::String(s)) => s.trim().to_string(),
        Some(Json::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let flag = |k: &str| values.get(k).and_then(Json::as_bool).unwrap_or(false);
    let number = |t: String| -> Json { t.parse::<f64>().map(|x| json!(x)).unwrap_or(json!(t)) };
    for c in &f.controls {
        match *c {
            Ctl::Ref(k, _) => {
                let t = text(k);
                if !t.is_empty() {
                    p.insert(k.into(), json!(t));
                }
            }
            Ctl::Grouped => {
                p.insert("groupedBy".into(), json!(text("groupedBy")));
            }
            Ctl::Check(k, _) => {
                p.insert(k.into(), json!(flag(k)));
            }
            Ctl::Num(k, _) => {
                let t = text(k);
                if !t.is_empty() {
                    p.insert(k.into(), number(t));
                }
            }
            Ctl::CheckNum(ck, _, k) => {
                p.insert(k.into(), if flag(ck) { number(text(k)) } else { Json::Null });
            }
            Ctl::Radio(k, _, _) => {
                p.insert(k.into(), json!(text(k)));
            }
            Ctl::Distribution => {
                let dist = text("distribution");
                p.insert("distribution".into(), json!(dist));
                let keys: &[&str] = match dist.as_str() {
                    "normal" => &["mean", "sd"],
                    "bernoulli" => &["p"],
                    "binomial" => &["p", "trials"],
                    "poisson" => &["lambda"],
                    "patterned" => &["from", "to", "step", "repeatEach", "repeatSequence"],
                    "discrete" => &["values"],
                    _ => &["from", "to"],
                };
                for k in keys {
                    let t = text(k);
                    if !t.is_empty() {
                        p.insert((*k).into(), if *k == "values" { json!(t) } else { number(t) });
                    }
                }
            }
            Ctl::Output { .. } => {
                let o = match text("outputMode").as_str() {
                    "range" => json!(text("outputRange")),
                    "newWorkbook" => json!("newWorkbook"),
                    _ => {
                        let name = text("newSheetName");
                        if name.is_empty() { json!("newSheet") } else { json!({"newSheet": name}) }
                    }
                };
                p.insert("output".into(), o);
            }
            Ctl::Heading(_) => {}
        }
    }
    Json::Object(p)
}

/// The Data Analysis box: pick a tool, OK opens its form.
pub fn list(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, open: &mut bool) {
    ui.label("Analysis Tools");
    let tools = gridcraft_engine::cmd::analysis::TOOLS;
    let mut chosen: Option<&'static str> = None;
    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        for (i, (id, name)) in tools.iter().enumerate() {
            let available = gridcraft_engine::cmd::find_command(id).is_some();
            let r = ui.add_enabled(available, egui::Button::selectable(d.list_index == i, *name).min_size(vec2(ui.available_width(), 18.0)));
            if r.clicked() {
                d.list_index = i;
            }
            if r.double_clicked() {
                chosen = Some(id);
            }
        }
    });
    let mut confirm = false;
    let mut keep = true;
    crate::dialogs::ok_cancel(ui, &mut confirm, &mut keep);
    if confirm && let Some((id, _)) = tools.get(d.list_index) {
        chosen = Some(id);
    }
    if !keep {
        *open = false;
    }
    if let Some(id) = chosen
        && gridcraft_engine::cmd::find_command(id).is_some()
    {
        app.dialog_next = self::open(id);
        *open = false;
    }
}
