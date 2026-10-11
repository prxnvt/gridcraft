//! The PivotTable Fields task pane: shown on the right when the active cell is in a pivot
//! report. Ticking a field adds it (text → Rows, numbers → Values). Fields drag from the list
//! into an area, between areas and within one to reorder; dragging one out of the areas removes
//! it. Each area field also has a menu. Everything runs `pivot.*` commands.

use egui::{DragAndDrop, Id, Rect, Sense, Ui, vec2};
use serde_json::{Value as Json, json};

use crate::theme::{self, Tokens};
use crate::{SheetApp, dnd};

/// Whether the active cell is inside a pivot report (so the pane should show).
pub fn active_pivot(app: &SheetApp) -> Option<String> {
    let d = app.session.active()?;
    let sh = d.wb.active()?;
    let a = d.selection.active;
    sh.pivots.iter().find(|p| p.last_range.is_some_and(|r| r.contains(a)) || p.anchor == a).map(|p| p.name.clone())
}

/// A field being dragged: its name (a value field's name in Values) and the area it came from
/// (`None` = the field list).
#[derive(Clone, Debug)]
struct Drag {
    field: String,
    from: Option<&'static str>,
    /// Its place in that area.
    index: usize,
}

const AREAS: [(&str, &str); 4] = [("filters", "Filters"), ("columns", "Columns"), ("rows", "Rows"), ("values", "Values")];

/// What dropping `d` at `pos` does: the command and its params, if any.
fn drop_action(pivot: &str, d: &Drag, target: Option<(&'static str, usize)>, areas: &[(&'static str, Vec<String>)]) -> Option<(&'static str, Json)> {
    let in_area = |key: &str| areas.iter().find(|(k, _)| *k == key).is_some_and(|(_, l)| l.iter().any(|f| f.eq_ignore_ascii_case(&d.field)));
    match (d.from, target) {
        // From the list: a field already in Rows, Columns or Filters moves; Values can take a field twice.
        (None, Some((to, pos))) => {
            if to != "values"
                && let Some(from) = ["rows", "columns", "filters"].into_iter().find(|a| in_area(a))
            {
                return Some(("pivot.moveField", json!({"pivot": pivot, "field": d.field, "from": from, "to": to, "position": pos})));
            }
            Some(("pivot.addField", json!({"pivot": pivot, "field": d.field, "area": to, "position": pos})))
        }
        (Some(from), Some((to, pos))) => {
            // Within an area the field leaves its old place first.
            let pos = if from == to && d.index < pos { pos - 1 } else { pos };
            if from == to && pos == d.index {
                return None;
            }
            Some(("pivot.moveField", json!({"pivot": pivot, "field": d.field, "from": from, "to": to, "position": pos})))
        }
        (Some(from), None) => Some(("pivot.removeField", json!({"pivot": pivot, "field": d.field, "area": from}))),
        (None, None) => None,
    }
}

pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let Some(name) = active_pivot(app) else { return };
    let t = Tokens::get(ui.ctx());
    let info = match app.session.run("pivot.fields", json!({"pivot": name})) {
        Ok(v) => v,
        Err(_) => return,
    };
    egui::Panel::right(app.widget_id("pivot_pane"))
        .default_size(280.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(10))
        .show(ui, |ui| {
            ui.label(egui::RichText::new("PivotTable Fields").font(theme::ui_bold(15.0)));
            ui.label(egui::RichText::new("Choose fields to add to report:").small().color(t.text_dim));
            ui.add_space(4.0);
            let fields = info["fields"].as_array().cloned().unwrap_or_default();
            egui::ScrollArea::vertical().max_height(220.0).id_salt("pv_fields").show(ui, |ui| {
                for f in &fields {
                    let fname = f["name"].as_str().unwrap_or("").to_string();
                    let areas: Vec<String> =
                        f["areas"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                    let mut on = !areas.is_empty();
                    let numeric = f["numeric"].as_bool().unwrap_or(false);
                    let handle = Id::new(("pv_drag_list", fname.as_str()));
                    dnd::drag_under(ui, handle, Drag { field: fname.clone(), from: None, index: 0 });
                    let r = ui.checkbox(&mut on, egui::RichText::new(&fname).font(theme::ui_font(13.0)));
                    ui.ctx().data_mut(|d| d.insert_temp(handle, r.rect));
                    if r.changed() {
                        let res = if on {
                            app.run("pivot.addField", json!({"pivot": name, "field": fname, "area": if numeric { "values" } else { "rows" }}))
                        } else {
                            app.run("pivot.removeField", json!({"pivot": name, "field": fname}))
                        };
                        if let Err(e) = res {
                            app.message = Some(("PivotTable".into(), crate::clean_error(&e)));
                        }
                    }
                    let samples: Vec<String> =
                        f["samples"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                    r.on_hover_text(samples.join(", "));
                }
            });
            ui.separator();
            ui.label(egui::RichText::new("Drag fields between the areas below:").small().color(t.text_dim));
            let area = |key: &str| -> Vec<String> {
                info["pivot"][key]
                    .as_array()
                    .or(info[key].as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x["name"].as_str().or(x["field"].as_str()).or(x["sourceCol"].as_str()).or(x.as_str()).map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let lists: Vec<(&'static str, Vec<String>)> = AREAS.iter().map(|(k, _)| (*k, area(k))).collect();
            let dragging = DragAndDrop::payload::<Drag>(ui.ctx());
            let pointer = ui.ctx().pointer_latest_pos();
            // (area, its box, its fields' rects)
            let mut zones: Vec<(&'static str, Rect, Vec<Rect>)> = Vec::new();
            ui.columns(2, |cols| {
                for (i, (key, label)) in AREAS.iter().enumerate() {
                    let ui = &mut cols[i % 2];
                    ui.label(egui::RichText::new(*label).strong());
                    let mut chips: Vec<Rect> = Vec::new();
                    let frame = egui::Frame::NONE.fill(t.window).corner_radius(4.0).inner_margin(4.0).show(ui, |ui| {
                        ui.set_min_size(vec2(ui.available_width(), 60.0));
                        for (fi, f) in area(key).into_iter().enumerate() {
                            let b = ui.add(
                                egui::Button::new(egui::RichText::new(format!("{f} ▾")).font(theme::ui_font(12.0)).color(t.text))
                                    .fill(t.input_bg)
                                    .stroke(egui::Stroke::new(1.0, t.input_border))
                                    .min_size(vec2(ui.available_width(), 20.0))
                                    .sense(Sense::click_and_drag()),
                            );
                            chips.push(b.rect);
                            dnd::drag_source(ui, &b, Drag { field: f.clone(), from: Some(key), index: fi });
                            egui::Popup::menu(&b).show(|ui| {
                                for (to, l) in [
                                    ("rows", "Move to Row Labels"),
                                    ("columns", "Move to Column Labels"),
                                    ("filters", "Move to Report Filter"),
                                    ("values", "Move to Values"),
                                ] {
                                    if to != *key && ui.button(l).clicked() {
                                        app.run_or_alert("pivot.moveField", json!({"pivot": name, "field": f, "from": key, "to": to}));
                                    }
                                }
                                if *key == "values" {
                                    ui.separator();
                                    for (func, l) in [("sum", "Sum"), ("count", "Count"), ("average", "Average"), ("max", "Max"), ("min", "Min")] {
                                        if ui.button(format!("Summarize by {l}")).clicked() {
                                            app.run_or_alert("pivot.valueSettings", json!({"pivot": name, "field": f, "func": func}));
                                        }
                                    }
                                    for (sa, l) in [
                                        ("normal", "No Calculation"),
                                        ("percentOfGrandTotal", "% of Grand Total"),
                                        ("percentOfColumnTotal", "% of Column Total"),
                                        ("percentOfRowTotal", "% of Row Total"),
                                        ("percentOfParentRowTotal", "% of Parent Row Total"),
                                        ("percentOfParentColumnTotal", "% of Parent Column Total"),
                                        ("runningTotal", "Running Total In"),
                                        ("percentRunningTotal", "% Running Total In"),
                                        ("rankAscending", "Rank Smallest to Largest"),
                                        ("rankDescending", "Rank Largest to Smallest"),
                                        ("index", "Index"),
                                    ] {
                                        if ui.button(format!("Show as {l}")).clicked() {
                                            app.run_or_alert("pivot.valueSettings", json!({"pivot": name, "field": f, "showAs": sa}));
                                        }
                                    }
                                } else {
                                    ui.separator();
                                    for (o, l) in [("asc", "Sort A to Z"), ("desc", "Sort Z to A")] {
                                        if ui.button(l).clicked() {
                                            app.run_or_alert("pivot.sort", json!({"pivot": name, "field": f, "order": o}));
                                        }
                                    }
                                    for (g, l) in [
                                        ("years", "Group by Years"),
                                        ("quarters", "Group by Quarters"),
                                        ("months", "Group by Months"),
                                        ("none", "Ungroup"),
                                    ] {
                                        if ui.button(l).clicked() {
                                            app.run_or_alert("pivot.group", json!({"pivot": name, "field": f, "by": g}));
                                        }
                                    }
                                }
                                ui.separator();
                                if ui.button("Remove Field").clicked() {
                                    app.run_or_alert("pivot.removeField", json!({"pivot": name, "field": f, "area": key}));
                                }
                            });
                        }
                    });
                    zones.push((key, frame.response.rect, chips));
                }
            });
            // While dragging: the area under the pointer lights up and a line shows where the field goes.
            if let (Some(_), Some(p)) = (&dragging, pointer)
                && let Some((_, zone, chips)) = zones.iter().find(|(_, z, _)| z.contains(p))
            {
                dnd::drop_marker(ui, *zone, chips, p.y);
            }
            if let Some(d) = &dragging
                && let Some(p) = pointer
            {
                dnd::ghost(ui, &d.field, p);
                if ui.input(|i| i.pointer.any_released()) {
                    let _ = DragAndDrop::take_payload::<Drag>(ui.ctx());
                    let target = zones.iter().find(|(_, z, _)| z.contains(p)).map(|(k, _, chips)| (*k, dnd::insert_index(chips, p.y)));
                    if let Some((cmd, params)) = drop_action(&name, d, target, &lists) {
                        app.run_or_alert(cmd, params);
                    }
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Refresh").clicked() {
                    app.run_or_alert("pivot.refresh", json!({"pivot": name}));
                }
                egui::ComboBox::from_id_salt("pv_layout").selected_text("Report Layout").show_ui(ui, |ui| {
                    for (l, v) in [("Compact", "compact"), ("Outline", "outline"), ("Tabular", "tabular")] {
                        if ui.button(l).clicked() {
                            app.run_or_alert("pivot.layout", json!({"pivot": name, "layout": v}));
                        }
                    }
                });
            });
            let _: Option<Json> = None;
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn areas() -> Vec<(&'static str, Vec<String>)> {
        vec![
            ("filters", vec![]),
            ("columns", vec!["Product".into()]),
            ("rows", vec!["Region".into(), "Date".into()]),
            ("values", vec!["Sum of Sales".into()]),
        ]
    }

    #[test]
    fn drops() {
        let d = |field: &str, from: Option<&'static str>, index: usize| Drag { field: field.into(), from, index };
        // From the list into an area, at a place.
        let (c, p) = drop_action("P", &d("Units", None, 0), Some(("rows", 1)), &areas()).unwrap();
        assert_eq!((c, p["area"].as_str(), p["position"].as_u64()), ("pivot.addField", Some("rows"), Some(1)));
        // A list field already on an axis moves.
        let (c, p) = drop_action("P", &d("Product", None, 0), Some(("rows", 0)), &areas()).unwrap();
        assert_eq!((c, p["from"].as_str()), ("pivot.moveField", Some("columns")));
        // Values can take a field twice.
        assert_eq!(drop_action("P", &d("Region", None, 0), Some(("values", 1)), &areas()).unwrap().0, "pivot.addField");
        // Reordering down accounts for the field leaving its place; a drop where it was does nothing.
        let (_, p) = drop_action("P", &d("Region", Some("rows"), 0), Some(("rows", 2)), &areas()).unwrap();
        assert_eq!(p["position"].as_u64(), Some(1));
        assert!(drop_action("P", &d("Region", Some("rows"), 0), Some(("rows", 1)), &areas()).is_none());
        // Between areas, and out of them.
        let (c, p) = drop_action("P", &d("Sum of Sales", Some("values"), 0), Some(("columns", 0)), &areas()).unwrap();
        assert_eq!((c, p["to"].as_str()), ("pivot.moveField", Some("columns")));
        let (c, p) = drop_action("P", &d("Date", Some("rows"), 1), None, &areas()).unwrap();
        assert_eq!((c, p["area"].as_str()), ("pivot.removeField", Some("rows")));
        assert!(drop_action("P", &d("Units", None, 0), None, &areas()).is_none());
        assert_eq!(
            dnd::insert_index(&[Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 20.0)), Rect::from_min_size(pos2(0.0, 22.0), vec2(10.0, 20.0))], 25.0),
            1
        );
    }
}
