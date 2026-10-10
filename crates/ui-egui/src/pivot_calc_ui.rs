//! Insert Calculated Field and Insert Calculated Item dialogs.

use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;

/// A field or item name as a formula uses it: bare when it can be, else in single quotes.
pub fn quote_name(n: &str) -> String {
    let bare = n.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && n.chars().all(|c| c.is_alphanumeric() || c == '_');
    if bare { n.to_string() } else { format!("'{}'", n.replace('\'', "''")) }
}

/// The PivotTable the active cell is in: (id, row and column fields).
pub fn active(app: &SheetApp) -> Option<(u32, Vec<String>)> {
    let d = app.session.active()?;
    let sh = d.wb.active()?;
    let a = d.selection.active;
    let pt = sh.pivots.iter().find(|p| p.last_range.is_some_and(|r| r.contains(a)))?;
    Some((pt.id, pt.rows.iter().chain(pt.columns.iter()).map(|f| f.source_col.clone()).collect()))
}

pub fn open_field(app: &SheetApp) -> Option<Dialog> {
    let (pivot, _) = active(app)?;
    Some(Dialog::custom("pivotCalcField", "Insert Calculated Field", json!({"pivot": pivot, "name": "", "formula": "= 0", "pick": 0})))
}

pub fn open_item(app: &SheetApp) -> Option<Dialog> {
    let (pivot, axis) = active(app)?;
    // The field of the active cell's label column, else the first row or column field.
    let field = axis.first().cloned()?;
    Some(Dialog::custom(
        "pivotCalcItem",
        &format!("Insert Calculated Item in \"{field}\""),
        json!({"pivot": pivot, "field": field, "name": "", "formula": "= 0", "pick": null, "pickItem": 0}),
    ))
}

fn get(d: &Dialog, k: &str) -> String {
    d.values.get(k).and_then(Json::as_str).unwrap_or("").to_string()
}

fn insert_into_formula(d: &mut Dialog, name: &str) {
    let mut f = get(d, "formula");
    if f.trim() == "= 0" || f.trim() == "=0" {
        f = "= ".into();
    }
    if !f.ends_with(' ') && !f.ends_with('=') && !f.ends_with('(') {
        f.push(' ');
    }
    f.push_str(&quote_name(name));
    d.values.insert("formula".into(), json!(f));
}

/// The list box: returns the picked index and whether it was double-clicked.
fn list(ui: &mut egui::Ui, salt: &str, items: &[String], pick: usize) -> (usize, bool) {
    let mut pick = pick;
    let mut double = false;
    egui::Frame::NONE.stroke(egui::Stroke::new(1.0, crate::theme::Tokens::get(ui.ctx()).input_border)).inner_margin(4).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(salt).max_height(150.0).min_scrolled_height(150.0).show(ui, |ui| {
            ui.set_min_width(200.0);
            for (i, n) in items.iter().enumerate() {
                let r = ui.selectable_label(i == pick, n.as_str());
                if r.clicked() {
                    pick = i;
                }
                if r.double_clicked() {
                    pick = i;
                    double = true;
                }
            }
        });
    });
    (pick, double)
}

/// Name and Formula rows; returns whether the name is an existing entry.
fn name_formula(ui: &mut egui::Ui, d: &mut Dialog, existing: &[(String, String)]) -> bool {
    egui::Grid::new("pv_calc_nf").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("Name:");
        ui.horizontal(|ui| {
            let mut name = get(d, "name");
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(220.0));
            egui::ComboBox::from_id_salt("pv_calc_names").selected_text("").width(24.0).show_ui(ui, |ui| {
                for (n, f) in existing {
                    if ui.selectable_label(false, n.as_str()).clicked() {
                        name = n.clone();
                        d.values.insert("formula".into(), json!(f));
                    }
                }
            });
            d.values.insert("name".into(), json!(name));
        });
        ui.end_row();
        ui.label("Formula:");
        let mut f = get(d, "formula");
        ui.add(egui::TextEdit::singleline(&mut f).desired_width(260.0));
        d.values.insert("formula".into(), json!(f));
        ui.end_row();
    });
    let name = get(d, "name");
    existing.iter().any(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
}

/// Runs a command, showing its error; true when it worked.
fn run(app: &mut SheetApp, title: &str, cmd: &str, p: Json) -> bool {
    match app.run(cmd, p) {
        Ok(_) => true,
        Err(e) => {
            app.message = Some((title.to_string(), crate::clean_error(&e)));
            false
        }
    }
}

/// The buttons along the bottom: OK and Close.
fn ok_close(ui: &mut egui::Ui) -> (bool, bool) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut ok = false;
    let mut close = false;
    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(" Close ").clicked() {
            close = true;
        }
        if ui.add(egui::Button::new(egui::RichText::new("   OK   ").color(egui::Color32::WHITE)).fill(t.accent)).clicked() {
            ok = true;
        }
    });
    (ok, close)
}

pub fn field_dialog(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool) {
    let pivot = d.values.get("pivot").cloned().unwrap_or(Json::Null);
    let info = app.session.run("pivot.fields", json!({"pivot": pivot})).unwrap_or(Json::Null);
    let fields: Vec<String> =
        info["fields"].as_array().map(|a| a.iter().filter_map(|f| f["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    let existing: Vec<(String, String)> = info["calculatedFields"]
        .as_array()
        .map(|a| a.iter().map(|f| (f["name"].as_str().unwrap_or("").to_string(), f["formula"].as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    let modify = name_formula(ui, d, &existing);
    let name = get(d, "name").trim().to_string();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label("Fields:");
            let pick = d.values.get("pick").and_then(Json::as_u64).unwrap_or(0) as usize;
            let (pick, double) = list(ui, "pv_calc_fields", &fields, pick);
            d.values.insert("pick".into(), json!(pick));
            if (ui.button("Insert Field").clicked() || double)
                && let Some(f) = fields.get(pick)
            {
                insert_into_formula(d, f);
            }
        });
        ui.vertical(|ui| {
            ui.add_space(20.0);
            if ui.add_enabled(!name.is_empty(), egui::Button::new(if modify { "Modify" } else { "  Add  " })).clicked()
                && run(app, &d.title, "pivot.calculatedField", json!({"pivot": pivot, "name": name, "formula": get(d, "formula")}))
            {
                d.values.insert("name".into(), json!(""));
                d.values.insert("formula".into(), json!("= 0"));
            }
            if ui.add_enabled(modify, egui::Button::new("Delete")).clicked()
                && run(app, &d.title, "pivot.deleteCalculatedField", json!({"pivot": pivot, "name": name}))
            {
                d.values.insert("name".into(), json!(""));
                d.values.insert("formula".into(), json!("= 0"));
            }
        });
    });
    let (ok, close) = ok_close(ui);
    // OK adds or changes the named field first, when one is named.
    if ok && (name.is_empty() || run(app, &d.title, "pivot.calculatedField", json!({"pivot": pivot, "name": name, "formula": get(d, "formula")}))) {
        *confirm = true;
    }
    if close {
        *confirm = true;
    }
}

pub fn item_dialog(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool) {
    let pivot = d.values.get("pivot").cloned().unwrap_or(Json::Null);
    let field = get(d, "field");
    let info = app.session.run("pivot.fields", json!({"pivot": pivot})).unwrap_or(Json::Null);
    let fields: Vec<String> = info["fields"]
        .as_array()
        .map(|a| a.iter().filter(|f| f["calculated"] != true).filter_map(|f| f["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let existing: Vec<(String, String)> = info["calculatedItems"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|c| c["field"].as_str().is_some_and(|f| f.eq_ignore_ascii_case(&field)))
                .map(|c| (c["name"].as_str().unwrap_or("").to_string(), c["formula"].as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default();
    let modify = name_formula(ui, d, &existing);
    let name = get(d, "name").trim().to_string();
    ui.add_space(6.0);
    // The list starts on the item's own field.
    let pick = d
        .values
        .get("pick")
        .and_then(Json::as_u64)
        .map(|p| p as usize)
        .unwrap_or_else(|| fields.iter().position(|f| f.eq_ignore_ascii_case(&field)).unwrap_or(0));
    let shown_field = fields.get(pick).cloned().unwrap_or_default();
    let items: Vec<String> = app
        .session
        .run("pivot.items", json!({"pivot": pivot, "field": shown_field}))
        .ok()
        .and_then(|r| r["items"].as_array().map(|a| a.iter().filter_map(|i| i["name"].as_str().map(str::to_string)).collect()))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label("Fields:");
            let (p, double) = list(ui, "pv_ci_fields", &fields, pick);
            d.values.insert("pick".into(), json!(p));
            if (ui.button("Insert Field").clicked() || double)
                && let Some(f) = fields.get(p)
            {
                insert_into_formula(d, f);
            }
        });
        ui.vertical(|ui| {
            ui.label("Items:");
            let ip = d.values.get("pickItem").and_then(Json::as_u64).unwrap_or(0) as usize;
            let (ip, double) = list(ui, "pv_ci_items", &items, ip);
            d.values.insert("pickItem".into(), json!(ip));
            if (ui.button("Insert Item").clicked() || double)
                && let Some(i) = items.get(ip)
            {
                // Items of another field need their field's name.
                if shown_field.eq_ignore_ascii_case(&field) {
                    insert_into_formula(d, i);
                } else {
                    let mut f = get(d, "formula");
                    f.push_str(&format!(" {}[{}]", quote_name(&shown_field), quote_name(i)));
                    d.values.insert("formula".into(), json!(f));
                }
            }
        });
        ui.vertical(|ui| {
            ui.add_space(20.0);
            if ui.add_enabled(!name.is_empty(), egui::Button::new(if modify { "Modify" } else { "  Add  " })).clicked()
                && run(app, &d.title, "pivot.calculatedItem", json!({"pivot": pivot, "field": field, "name": name, "formula": get(d, "formula")}))
            {
                d.values.insert("name".into(), json!(""));
                d.values.insert("formula".into(), json!("= 0"));
            }
            if ui.add_enabled(modify, egui::Button::new("Delete")).clicked()
                && run(app, &d.title, "pivot.deleteCalculatedItem", json!({"pivot": pivot, "field": field, "name": name}))
            {
                d.values.insert("name".into(), json!(""));
                d.values.insert("formula".into(), json!("= 0"));
            }
        });
    });
    let (ok, close) = ok_close(ui);
    if ok
        && (name.is_empty()
            || run(app, &d.title, "pivot.calculatedItem", json!({"pivot": pivot, "field": field, "name": name, "formula": get(d, "formula")})))
    {
        *confirm = true;
    }
    if close {
        *confirm = true;
    }
}

#[cfg(test)]
mod tests {
    use super::quote_name;

    #[test]
    fn quoting() {
        assert_eq!(quote_name("Sales"), "Sales");
        assert_eq!(quote_name("Unit Price"), "'Unit Price'");
        assert_eq!(quote_name("Bob's"), "'Bob''s'");
        assert_eq!(quote_name("2026"), "'2026'");
    }
}
