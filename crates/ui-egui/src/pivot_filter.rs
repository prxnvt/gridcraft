//! PivotTable field buttons: the sort and filter drop-down on Row Labels, Column Labels, field
//! headers and report filters, and the Label, Value, Top 10 and Date Filter dialogs.

use egui::{Color32, vec2};
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::Sheet;
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::{Dialog, ok_cancel};

/// What the open drop-down shows; loaded once per field.
#[derive(Clone, Default)]
struct MenuState {
    field: String,
    search: String,
    checks: Vec<(String, bool)>,
    info: Json,
}

/// Label filter comparisons: (menu label, op).
const LABEL_OPS: [(&str, &str); 14] = [
    ("Equals…", "equals"),
    ("Does Not Equal…", "doesNotEqual"),
    ("Begins With…", "beginsWith"),
    ("Does Not Begin With…", "doesNotBeginWith"),
    ("Ends With…", "endsWith"),
    ("Does Not End With…", "doesNotEndWith"),
    ("Contains…", "contains"),
    ("Does Not Contain…", "doesNotContain"),
    ("Greater Than…", "greaterThan"),
    ("Greater Than Or Equal To…", "greaterThanOrEqual"),
    ("Less Than…", "lessThan"),
    ("Less Than Or Equal To…", "lessThanOrEqual"),
    ("Between…", "between"),
    ("Not Between…", "notBetween"),
];

/// Value filter comparisons.
const VALUE_OPS: [(&str, &str); 8] = [
    ("Equals…", "equals"),
    ("Does Not Equal…", "doesNotEqual"),
    ("Greater Than…", "greaterThan"),
    ("Greater Than Or Equal To…", "greaterThanOrEqual"),
    ("Less Than…", "lessThan"),
    ("Less Than Or Equal To…", "lessThanOrEqual"),
    ("Between…", "between"),
    ("Not Between…", "notBetween"),
];

/// Date filters that need a date.
const DATE_OPS: [(&str, &str); 4] = [("Equals…", "equals"), ("Before…", "before"), ("After…", "after"), ("Between…", "between")];

/// Date filters on a period relative to today.
const DATE_PERIODS: [&[(&str, &str)]; 5] = [
    &[("Tomorrow", "tomorrow"), ("Today", "today"), ("Yesterday", "yesterday")],
    &[("Next Week", "nextWeek"), ("This Week", "thisWeek"), ("Last Week", "lastWeek")],
    &[("Next Month", "nextMonth"), ("This Month", "thisMonth"), ("Last Month", "lastMonth")],
    &[("Next Quarter", "nextQuarter"), ("This Quarter", "thisQuarter"), ("Last Quarter", "lastQuarter")],
    &[("Next Year", "nextYear"), ("This Year", "thisYear"), ("Last Year", "lastYear"), ("Year to Date", "yearToDate")],
];

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// The PivotTable field button at `c`: (pivot id, field).
pub fn button_at(sh: &Sheet, c: CellRef) -> Option<(u32, String)> {
    sh.pivots.iter().find_map(|pt| {
        let (_, f) = pt.buttons.iter().find(|(b, _)| *b == c && pt.last_range.is_some_and(|r| r.contains(c)))?;
        Some((pt.id, f.clone()))
    })
}

/// Field buttons to draw: (cell, filtered).
pub fn buttons(sh: &Sheet) -> Vec<(CellRef, bool)> {
    let mut out = Vec::new();
    for pt in &sh.pivots {
        let Some(r) = pt.last_range else { continue };
        for (c, field) in &pt.buttons {
            if !r.contains(*c) {
                continue;
            }
            // Compact Row/Column Labels speak for every field on that axis.
            let compact = pt.layout == gridcraft_engine::model::PivotLayout::Compact;
            let axis: Vec<&str> = if compact && pt.rows.first().is_some_and(|f| f.source_col == *field) {
                pt.rows.iter().map(|f| f.source_col.as_str()).collect()
            } else if compact && pt.columns.first().is_some_and(|f| f.source_col == *field) {
                pt.columns.iter().map(|f| f.source_col.as_str()).collect()
            } else {
                vec![field.as_str()]
            };
            let filtered = pt.filters.iter().any(|f| {
                axis.iter().any(|a| a.eq_ignore_ascii_case(&f.source_col)) && (f.selected.is_some() || f.label.is_some() || f.value.is_some())
            });
            out.push((*c, filtered));
        }
    }
    out
}

fn load(app: &mut SheetApp, pivot: u32, field: &str) -> MenuState {
    let info = app.session.run("pivot.items", json!({"pivot": pivot, "field": field})).unwrap_or(Json::Null);
    let checks = info["items"]
        .as_array()
        .map(|a| a.iter().map(|i| (i["name"].as_str().unwrap_or("").to_string(), i["shown"].as_bool().unwrap_or(true))).collect())
        .unwrap_or_default();
    MenuState { field: field.to_string(), search: String::new(), checks, info }
}

/// The open field drop-down, if any.
pub fn menu(app: &mut SheetApp, ui: &mut egui::Ui) {
    let Some((pivot, field0, at)) = app.grid.pivot_menu.clone() else { return };
    let key = egui::Id::new(("pivot_menu", pivot, field0.clone()));
    let mut st: MenuState = ui.ctx().data_mut(|d| d.get_temp::<MenuState>(key)).unwrap_or_default();
    if st.field.is_empty() {
        st = load(app, pivot, &field0);
    }
    let mut close = false;
    let tokens = crate::theme::Tokens::get(ui.ctx());
    let area = egui::Area::new(egui::Id::new("pivot_filter_menu")).fixed_pos(at + vec2(-120.0, 10.0)).constrain(true).order(egui::Order::Foreground);
    let resp = area.show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(250.0);
            let field = st.field.clone();
            let axis = st.info["axis"].as_bool().unwrap_or(false);
            // Compact layout: one button for every field on the axis.
            let fields: Vec<String> = ["rows", "columns"]
                .iter()
                .filter_map(|a| st.info[*a].as_array())
                .find(|list| list.len() > 1 && list.first().and_then(Json::as_str).is_some_and(|f| f.eq_ignore_ascii_case(&field0)))
                .map(|list| list.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if !fields.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("Select field:");
                    let mut pick = field.clone();
                    egui::ComboBox::from_id_salt("pivot_menu_field").selected_text(pick.clone()).width(140.0).show_ui(ui, |ui| {
                        for f in &fields {
                            ui.selectable_value(&mut pick, f.clone(), f.as_str());
                        }
                    });
                    if pick != field {
                        st = load(app, pivot, &pick);
                    }
                });
                ui.separator();
            }
            let field = st.field.clone();
            let p = |extra: Json| {
                let mut m = extra.as_object().cloned().unwrap_or_default();
                m.insert("pivot".into(), json!(pivot));
                m.insert("field".into(), json!(field));
                Json::Object(m)
            };
            let item = |ui: &mut egui::Ui, label: &str| ui.add(egui::Button::new(label).frame(false).min_size(vec2(230.0, 20.0))).clicked();
            if axis {
                if item(ui, "Sort A to Z") {
                    app.run_or_alert("pivot.sort", p(json!({"order": "asc"})));
                    close = true;
                }
                if item(ui, "Sort Z to A") {
                    app.run_or_alert("pivot.sort", p(json!({"order": "desc"})));
                    close = true;
                }
                ui.separator();
            }
            let filtered = st.checks.iter().any(|c| !c.1) || !st.info["label"].is_null() || !st.info["value"].is_null();
            if ui
                .add_enabled(filtered, egui::Button::new(format!("Clear Filter From \"{field}\"")).frame(false).min_size(vec2(230.0, 20.0)))
                .clicked()
            {
                app.run_or_alert("pivot.clearFilter", p(json!({})));
                close = true;
            }
            // Submenus look like the other menu items.
            ui.style_mut().visuals.button_frame = false;
            if axis {
                let date = st.info["date"].as_bool().unwrap_or(false);
                let has_values = st.info["values"].as_array().is_some_and(|v| !v.is_empty());
                if date {
                    ui.menu_button("Date Filters", |ui| {
                        for (label, op) in DATE_OPS {
                            if ui.button(label).clicked() {
                                open_dialog(app, "date", pivot, &field, op, &st.info);
                                close = true;
                            }
                        }
                        for group in DATE_PERIODS {
                            ui.separator();
                            for (label, rule) in group {
                                if ui.button(*label).clicked() {
                                    app.run_or_alert("pivot.dateFilter", p(json!({"rule": rule})));
                                    close = true;
                                }
                            }
                        }
                        ui.separator();
                        ui.menu_button("All Dates in the Period", |ui| {
                            for q in 1..=4 {
                                if ui.button(format!("Quarter {q}")).clicked() {
                                    app.run_or_alert("pivot.dateFilter", p(json!({"rule": format!("q{q}")})));
                                    close = true;
                                }
                            }
                            ui.separator();
                            for m in MONTHS {
                                if ui.button(m).clicked() {
                                    app.run_or_alert("pivot.dateFilter", p(json!({"rule": m.to_ascii_lowercase()})));
                                    close = true;
                                }
                            }
                        });
                    });
                } else {
                    ui.menu_button("Label Filters", |ui| {
                        for (i, (label, op)) in LABEL_OPS.iter().enumerate() {
                            if i == 2 || i == 8 || i == 12 {
                                ui.separator();
                            }
                            if ui.button(*label).clicked() {
                                open_dialog(app, "label", pivot, &field, op, &st.info);
                                close = true;
                            }
                        }
                    });
                }
                ui.add_enabled_ui(has_values, |ui| {
                    ui.menu_button("Value Filters", |ui| {
                        for (i, (label, op)) in VALUE_OPS.iter().enumerate() {
                            if i == 2 || i == 6 {
                                ui.separator();
                            }
                            if ui.button(*label).clicked() {
                                open_dialog(app, "value", pivot, &field, op, &st.info);
                                close = true;
                            }
                        }
                        ui.separator();
                        if ui.button("Top 10…").clicked() {
                            open_dialog(app, "top10", pivot, &field, "", &st.info);
                            close = true;
                        }
                    });
                });
            }
            ui.style_mut().visuals.button_frame = true;
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut st.search).hint_text("Search").desired_width(230.0));
            let needle = st.search.to_lowercase();
            let visible = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
            let all = st.checks.iter().filter(|c| visible(&c.0)).all(|c| c.1);
            let mut all_new = all;
            if ui.checkbox(&mut all_new, if needle.is_empty() { "(Select All)" } else { "(Select All Search Results)" }).changed() {
                for c in st.checks.iter_mut().filter(|c| visible(&c.0)) {
                    c.1 = all_new;
                }
            }
            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                for (name, on) in st.checks.iter_mut() {
                    if visible(name) {
                        ui.checkbox(on, if name.is_empty() { "(blank)" } else { name.as_str() });
                    }
                }
            });
            if st.info["more"].as_bool().unwrap_or(false) {
                ui.label(egui::RichText::new("Not all items are shown.").color(tokens.text_dim));
            }
            let none = st.checks.iter().all(|c| !c.1);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(" Cancel ").clicked() {
                        close = true;
                    }
                    let ok = egui::Button::new(egui::RichText::new("   OK   ").color(Color32::WHITE)).fill(tokens.accent);
                    if ui.add_enabled(!none, ok).clicked() {
                        // A search applies to the matching items only; everything else is hidden.
                        let shown: Vec<String> = st.checks.iter().filter(|c| c.1 && visible(&c.0)).map(|c| c.0.clone()).collect();
                        let selected = if shown.len() == st.checks.len() { Json::Null } else { json!(shown) };
                        app.run_or_alert("pivot.filter", p(json!({"selected": selected})));
                        close = true;
                    }
                });
            });
        });
    });
    ui.ctx().data_mut(|d| d.insert_temp(key, st));
    let outside = ui.input(|i| i.pointer.any_pressed())
        && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !resp.response.rect.contains(p))
        && !ui.ctx().is_pointer_over_egui();
    if close || outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.grid.pivot_menu = None;
        ui.ctx().data_mut(|d| d.remove::<MenuState>(key));
    }
}

fn open_dialog(app: &mut SheetApp, kind: &str, pivot: u32, field: &str, op: &str, info: &Json) {
    let current = match kind {
        "value" | "top10" => info["value"].clone(),
        _ => info["label"].clone(),
    };
    app.open_dialog("pivotFilter", json!({"kind": kind, "pivot": pivot, "field": field, "op": op, "current": current, "values": info["values"]}));
}

/// The Label, Value, Top 10 or Date Filter dialog.
pub fn dialog(params: &Json) -> Dialog {
    let kind = params["kind"].as_str().unwrap_or("label");
    let field = params["field"].as_str().unwrap_or("");
    let title = match kind {
        "value" => format!("Value Filter ({field})"),
        "top10" => format!("Top 10 Filter ({field})"),
        "date" => format!("Date Filter ({field})"),
        _ => format!("Label Filter ({field})"),
    };
    let values: Vec<String> =
        params["values"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    // Start from the field's current rule when it's of the same kind.
    let cur = &params["current"];
    let rule = cur.as_object().and_then(|m| m.iter().next()).map(|(k, v)| (k.as_str(), v.clone()));
    let mut v = json!({
        "kind": kind,
        "pivot": params["pivot"],
        "field": field,
        "op": params["op"],
        "value": "",
        "value2": "",
        "valueField": values.first().cloned().unwrap_or_default(),
        "valueFields": values,
        "top": "top",
        "count": "10",
        "by": "items",
    });
    let s = |x: &Json| match x {
        Json::String(t) => t.clone(),
        Json::Number(n) => n.to_string(),
        _ => String::new(),
    };
    match (kind, rule) {
        ("label", Some(("label", r))) if r["op"] == params["op"] || params["op"].as_str().is_none_or(str::is_empty) => {
            v["value"] = json!(s(&r["value"]));
            v["value2"] = json!(s(&r["value2"]));
        }
        ("value", Some(("value", r))) => {
            v["valueField"] = r["field"].clone();
            v["value"] = json!(s(&r["value"]));
            v["value2"] = json!(s(&r["value2"]));
        }
        ("top10", Some(("top", r))) => {
            v["top"] = json!(if r["top"].as_bool().unwrap_or(true) { "top" } else { "bottom" });
            v["count"] = json!(s(&r["count"]));
            v["by"] = r["by"].clone();
            v["valueField"] = r["field"].clone();
        }
        _ => {}
    }
    if kind == "value" && v["value"] == json!("") {
        v["value"] = json!("0");
    }
    Dialog::custom("pivotFilter", &title, v)
}

fn combo(ui: &mut egui::Ui, d: &mut Dialog, key: &str, salt: &str, options: &[(&str, &str)], width: f32) {
    let mut cur = d.values.get(key).and_then(Json::as_str).unwrap_or("").to_string();
    let shown = options.iter().find(|o| o.1 == cur).map(|o| o.0.trim_end_matches('…')).unwrap_or("").to_string();
    egui::ComboBox::from_id_salt(salt).selected_text(shown).width(width).show_ui(ui, |ui| {
        for (label, id) in options {
            ui.selectable_value(&mut cur, id.to_string(), label.trim_end_matches('…'));
        }
    });
    d.values.insert(key.into(), json!(cur));
}

fn text(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32) {
    let mut t = d.values.get(key).and_then(Json::as_str).unwrap_or("").to_string();
    ui.add(egui::TextEdit::singleline(&mut t).desired_width(width));
    d.values.insert(key.into(), json!(t));
}

/// Draws the dialog; runs the filter on OK.
pub fn show(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool) {
    let get = |d: &Dialog, k: &str| d.values.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    let kind = get(d, "kind");
    let value_fields: Vec<String> = d
        .values
        .get("valueFields")
        .and_then(Json::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let value_field_combo = |ui: &mut egui::Ui, d: &mut Dialog| {
        let mut cur = get(d, "valueField");
        egui::ComboBox::from_id_salt("pf_value_field").selected_text(cur.clone()).width(150.0).show_ui(ui, |ui| {
            for f in &value_fields {
                ui.selectable_value(&mut cur, f.clone(), f.as_str());
            }
        });
        d.values.insert("valueField".into(), json!(cur));
    };
    match kind.as_str() {
        "top10" => {
            ui.label("Show");
            ui.horizontal(|ui| {
                combo(ui, d, "top", "pf_top", &[("Top", "top"), ("Bottom", "bottom")], 80.0);
                text(ui, d, "count", 50.0);
                combo(ui, d, "by", "pf_by", &[("Items", "items"), ("Percent", "percent"), ("Sum", "sum")], 80.0);
                ui.label("by");
                value_field_combo(ui, d);
            });
        }
        "value" => {
            ui.label("Show items for which");
            ui.horizontal(|ui| {
                value_field_combo(ui, d);
                combo(ui, d, "op", "pf_op", &VALUE_OPS, 170.0);
            });
        }
        "date" => {
            ui.label("Show items for which the date");
            combo(ui, d, "op", "pf_op", &DATE_OPS, 170.0);
        }
        _ => {
            ui.label("Show items for which the label");
            combo(ui, d, "op", "pf_op", &LABEL_OPS, 210.0);
        }
    }
    let op = get(d, "op");
    let between = matches!(op.as_str(), "between" | "notBetween");
    if kind != "top10" {
        ui.horizontal(|ui| {
            text(ui, d, "value", 140.0);
            if between {
                ui.label("and");
                text(ui, d, "value2", 140.0);
            }
        });
        if kind == "label" {
            ui.label(egui::RichText::new("Use ? to represent any single character.\nUse * to represent any series of characters.").small());
        }
    }
    let mut open = true;
    ok_cancel(ui, confirm, &mut open);
    if *confirm {
        let pivot = d.values.get("pivot").cloned().unwrap_or(Json::Null);
        let field = get(d, "field");
        let num = |k: &str| get(d, k).trim().parse::<f64>().ok();
        let (cmd, params) = match kind.as_str() {
            "top10" => {
                let Some(count) = num("count") else {
                    app.message = Some(("Top 10 Filter".into(), "Enter a number.".into()));
                    *confirm = false;
                    return;
                };
                (
                    "pivot.top10",
                    json!({"pivot": pivot, "field": field, "top": get(d, "top"), "count": count, "by": get(d, "by"), "valueField": get(d, "valueField")}),
                )
            }
            "value" => {
                let (Some(a), b) = (num("value"), num("value2")) else {
                    app.message = Some(("Value Filter".into(), "Enter a number.".into()));
                    *confirm = false;
                    return;
                };
                if between && b.is_none() {
                    app.message = Some(("Value Filter".into(), "Enter a number.".into()));
                    *confirm = false;
                    return;
                }
                (
                    "pivot.valueFilter",
                    json!({"pivot": pivot, "field": field, "op": op, "valueField": get(d, "valueField"), "value": a, "value2": b.unwrap_or(0.0)}),
                )
            }
            "date" => ("pivot.dateFilter", json!({"pivot": pivot, "field": field, "rule": op, "value": get(d, "value"), "value2": get(d, "value2")})),
            _ => ("pivot.labelFilter", json!({"pivot": pivot, "field": field, "op": op, "value": get(d, "value"), "value2": get(d, "value2")})),
        };
        if let Err(e) = app.run(cmd, params) {
            app.message = Some((d.title.clone(), crate::clean_error(&e)));
            *confirm = false;
        }
    }
    if !open {
        *confirm = true;
    }
}
