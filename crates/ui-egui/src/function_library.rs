//! The Formulas tab's Function Library: the function categories, their menus, the AutoSum menu
//! (shared with Home › Editing), and the Recently Used list, which follows the formulas entered
//! from any source (the ribbon, the control channel, MCP or a script) by reading the session's
//! journal.

use std::sync::OnceLock;

use egui::{Ui, vec2};
use gridcraft_engine::formula::lexer::{Tok, tokenize};
use gridcraft_engine::formula::normalize_function_name;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::SheetApp;

/// How many functions Recently Used keeps.
pub const RECENT_MAX: usize = 10;
/// The Insert Function category that lists [`crate::UiState::recent_functions`].
pub const MOST_RECENTLY_USED: &str = "Most Recently Used";
/// The Insert Function category that lists every function.
pub const ALL: &str = "All";

/// The function categories in the order the Insert Function dialog lists them: (the engine's
/// category id, the label people see).
pub const CATEGORIES: [(&str, &str); 14] = [
    ("Financial", "Financial"),
    ("DateTime", "Date & Time"),
    ("MathTrig", "Math & Trig"),
    ("Statistical", "Statistical"),
    ("Lookup", "Lookup & Reference"),
    ("Database", "Database"),
    ("Text", "Text"),
    ("Logical", "Logical"),
    ("Information", "Information"),
    ("Engineering", "Engineering"),
    ("Cube", "Cube"),
    ("Compatibility", "Compatibility"),
    ("Web", "Web"),
    ("AddIn", "Add-in and Automation"),
];

/// The categories under More Functions, each a cascading submenu.
pub const MORE_FUNCTIONS: [&str; 6] = ["Statistical", "Engineering", "Cube", "Information", "Compatibility", "Web"];

/// The label of an engine category id (the id itself if it is unknown).
pub fn category_label(id: &str) -> &str {
    CATEGORIES.iter().find(|(c, _)| *c == id).map(|(_, l)| *l).unwrap_or(id)
}

/// The engine category id for a category given by id or label, in any case.
pub fn category_id(name: &str) -> Option<&'static str> {
    CATEGORIES.iter().find(|(c, l)| c.eq_ignore_ascii_case(name) || l.eq_ignore_ascii_case(name)).map(|(c, _)| *c)
}

/// What Recently Used lists before any function has been used.
pub fn default_recent() -> Vec<String> {
    ["SUM", "AVERAGE", "IF", "COUNT", "MAX", "MIN", "SUMIF", "XLOOKUP", "PMT", "TODAY"].map(String::from).to_vec()
}

/// One function of the library.
#[derive(Clone, Debug)]
pub struct FnEntry {
    pub name: String,
    pub category: String,
    pub description: String,
}

/// Every function, sorted by name (built once; the list is static).
pub fn all() -> &'static [FnEntry] {
    static LIST: OnceLock<Vec<FnEntry>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut list: Vec<FnEntry> = gridcraft_engine::cmd::formulas::function_list()
            .iter()
            .filter_map(|f| {
                Some(FnEntry {
                    name: f["name"].as_str()?.to_string(),
                    category: f["category"].as_str().unwrap_or("").to_string(),
                    description: f["description"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list.dedup_by(|a, b| a.name == b.name);
        list
    })
}

/// The function named `name` (upper case), if the library knows it.
pub fn find(name: &str) -> Option<&'static FnEntry> {
    let list = all();
    list.binary_search_by(|f| f.name.as_str().cmp(name)).ok().and_then(|i| list.get(i))
}

/// The functions of one engine category, in name order.
pub fn in_category(id: &str) -> impl Iterator<Item = &'static FnEntry> + '_ {
    all().iter().filter(move |f| f.category == id)
}

/// The longest formula text read for Recently Used (Excel's formula length limit).
const FORMULA_MAX: usize = 8_192;

/// The functions a formula (`=…`) calls, outermost first, known functions only.
pub fn functions_in_formula(text: &str) -> Vec<String> {
    let Some(body) = text.trim_start().strip_prefix('=') else { return vec![] };
    // A formula longer than Excel allows was not typed by a person; it is not worth the scan.
    if body.len() > FORMULA_MAX {
        return vec![];
    }
    let mut out: Vec<String> = Vec::new();
    for t in tokenize(body).unwrap_or_default() {
        if let Tok::Func(name) = t.tok {
            let name = normalize_function_name(&name);
            if find(&name).is_some() && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Moves `names` (most recent first) to the front of `recent`, keeping at most [`RECENT_MAX`].
pub fn push_recent(recent: &mut Vec<String>, names: &[String]) {
    let mut next: Vec<String> = Vec::with_capacity(RECENT_MAX);
    for n in names.iter().chain(recent.iter()) {
        if next.len() == RECENT_MAX {
            break;
        }
        if !next.contains(n) {
            next.push(n.clone());
        }
    }
    *recent = next;
}

/// Reads a saved Recently Used list: known function names only, at most [`RECENT_MAX`]. A value
/// that is not a list gives the default list; a hand-edited or hostile `ui.json` loses this list,
/// never the rest of the preferences.
pub fn lenient_recent<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let raw = Json::deserialize(d)?;
    let Some(list) = raw.as_array() else { return Ok(default_recent()) };
    let mut out = Vec::new();
    for name in list.iter().filter_map(Json::as_str) {
        let name = name.to_ascii_uppercase();
        if out.len() < RECENT_MAX && find(&name).is_some() && !out.contains(&name) {
            out.push(name);
        }
    }
    Ok(out)
}

/// The formula texts a journaled command entered, in the order of its params.
fn journal_formulas<'a>(id: &str, params: &'a Json, out: &mut Vec<&'a str>) {
    if !(id.starts_with("cell.") || id.starts_with("range.")) {
        return;
    }
    fn walk<'a>(v: &'a Json, out: &mut Vec<&'a str>, depth: usize) {
        match v {
            Json::String(s) if s.starts_with('=') => out.push(s),
            // A range's values are rows of cells; anything deeper is not a cell value.
            Json::Array(a) if depth < 2 => a.iter().for_each(|x| walk(x, out, depth + 1)),
            _ => {}
        }
    }
    for key in ["input", "value", "values", "formula"] {
        if let Some(v) = params.get(key) {
            walk(v, out, 0);
        }
    }
}

/// The most formula texts read in one frame: a large paste needs only its last few functions.
const SCAN_BUDGET: usize = 2_000;

/// Reads the journal entries added since the last call and moves the functions their formulas
/// call to the front of Recently Used.
///
/// The journal also holds nested commands (AutoSum's own `cell.set`), which only repeat a
/// function already counted. When the journal passes [`gridcraft_engine::JOURNAL_LIMIT`] entries
/// it drops its oldest [`gridcraft_engine::JOURNAL_TRIM`], which this allows for.
pub fn harvest(app: &mut SheetApp) {
    let len = app.session.journal.len();
    if len < app.journal_seen {
        app.journal_seen = app.journal_seen.saturating_sub(gridcraft_engine::JOURNAL_TRIM).min(len);
    }
    if len == app.journal_seen {
        return;
    }
    // Newest first: the last entries name the most recent functions.
    let mut names: Vec<String> = Vec::new();
    let mut scanned = 0;
    'entries: for (id, params) in app.session.journal.get(app.journal_seen..).unwrap_or_default().iter().rev() {
        let mut formulas = Vec::new();
        journal_formulas(id, params, &mut formulas);
        for f in formulas.iter().rev() {
            for n in functions_in_formula(f) {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
            scanned += 1;
            if names.len() >= RECENT_MAX || scanned >= SCAN_BUDGET {
                break 'entries;
            }
        }
    }
    app.journal_seen = len;
    if !names.is_empty() {
        push_recent(&mut app.ui.recent_functions, &names);
    }
}

/// The name a function shows under in the interface language.
fn display_name<'a>(app: &SheetApp, name: &'a str) -> &'a str {
    let locale = app.ui.language.formula_locale();
    app.session.active().map(|d| crate::formula_locale::completion_name(name, locale, &d.wb, d.wb.active_sheet)).unwrap_or(name)
}

/// One menu row per function, with its description on hover; a click starts the formula.
fn function_items<'a>(app: &mut SheetApp, ui: &mut Ui, names: impl Iterator<Item = &'a str>) {
    let locale = app.ui.language.formula_locale();
    ui.set_max_height(420.0);
    egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
        for n in names {
            let Some(entry) = find(n) else { continue };
            let name = display_name(app, &entry.name);
            let desc = crate::formula_locale::description(&entry.name, locale).unwrap_or_default();
            let r = ui.add(egui::Button::new(name).frame(false).min_size(vec2(180.0, 20.0)));
            if r.on_hover_text(desc).clicked() {
                app.run_or_alert("formulas.insertFunction", json!({"name": entry.name}));
                ui.close();
            }
        }
    });
}

/// The "Insert Function…" row that ends every category menu; it opens the dialog on `category`.
fn insert_function_item(app: &mut SheetApp, ui: &mut Ui, category: &str) {
    ui.separator();
    let label = app.ui.language.tr("Insert Function…");
    if ui.add(egui::Button::new(label).frame(false).min_size(vec2(180.0, 22.0))).clicked() {
        app.open_dialog("insertFunction", json!({"category": category}));
        ui.close();
    }
}

/// The menu of one category: its functions, then Insert Function….
pub fn category_menu(app: &mut SheetApp, ui: &mut Ui, id: &str) {
    let names: Vec<&str> = in_category(id).map(|f| f.name.as_str()).collect();
    function_items(app, ui, names.into_iter());
    insert_function_item(app, ui, category_label(id));
}

/// Recently Used: the most recent functions, newest first, then Insert Function….
pub fn recent_menu(app: &mut SheetApp, ui: &mut Ui) {
    let recent = app.ui.recent_functions.clone();
    function_items(app, ui, recent.iter().map(String::as_str));
    insert_function_item(app, ui, MOST_RECENTLY_USED);
}

/// More Functions: a cascading submenu per remaining category.
pub fn more_menu(app: &mut SheetApp, ui: &mut Ui) {
    for id in MORE_FUNCTIONS {
        let label = app.ui.language.tr(category_label(id)).to_string();
        ui.menu_button(label, |ui| category_menu(app, ui, id));
    }
}

/// The AutoSum menu, shared by Home › Editing and Formulas › Function Library: a function, or
/// the Insert Function dialog.
pub fn auto_sum_menu(app: &mut SheetApp, ui: &mut Ui) {
    for (label, f) in [("Sum", "SUM"), ("Average", "AVERAGE"), ("Count Numbers", "COUNT"), ("Max", "MAX"), ("Min", "MIN")] {
        let label = app.ui.language.tr(label);
        if ui.add(egui::Button::new(label).frame(false).min_size(vec2(160.0, 22.0))).clicked() {
            app.run_or_alert("formulas.autoSum", json!({"function": f}));
            ui.close();
        }
    }
    ui.separator();
    let label = app.ui.language.tr("More Functions…");
    if ui.add(egui::Button::new(label).frame(false).min_size(vec2(160.0, 22.0))).clicked() {
        app.open_dialog("insertFunction", json!({}));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_cover_every_function() {
        for f in all() {
            assert!(category_id(&f.category).is_some(), "{} is in an unknown category {}", f.name, f.category);
        }
        assert_eq!(category_id("date & time"), Some("DateTime"));
        assert_eq!(category_id("Lookup"), Some("Lookup"));
        assert_eq!(category_id("nope"), None);
        for c in MORE_FUNCTIONS {
            assert!(category_id(c).is_some());
        }
        assert!(all().windows(2).all(|w| w[0].name < w[1].name), "sorted by name, without duplicates");
        assert_eq!(find("SUM").map(|f| f.category.as_str()), Some("MathTrig"));
        assert!(find("NOT_A_FUNCTION").is_none());
    }

    #[test]
    fn formulas_name_their_functions_outermost_first() {
        assert_eq!(functions_in_formula("=IF(SUM(A1:A3)>2,_xlfn.XLOOKUP(1,B:B,C:C),sum(1))"), ["IF", "SUM", "XLOOKUP"]);
        assert!(functions_in_formula("=MYLAMBDA(1)+A1").is_empty(), "only known functions");
        assert!(functions_in_formula("SUM(1)").is_empty(), "not a formula");
        assert!(functions_in_formula("=\"SUM(\"").is_empty(), "inside a string");
        assert!(functions_in_formula(&format!("=SUM({})", "1+".repeat(9000))).is_empty(), "too long");
        assert!(functions_in_formula("=((((").is_empty());
        assert!(functions_in_formula("=").is_empty());
    }

    #[test]
    fn journal_entries_give_their_formulas() {
        let entries = [
            ("range.setValues", json!({"values": [["=SUM(1)", 2], ["x", "=PMT(1,2,3)"]]})),
            ("cell.set", json!({"input": "=NPV(1,2)"})),
            ("home.bold", json!({"input": "=IRR(1)"})),
            ("range.setValues", json!({"values": [[["=FV(1,2,3)"]]]})),
        ];
        let mut out = Vec::new();
        for (id, params) in &entries {
            journal_formulas(id, params, &mut out);
        }
        assert_eq!(out, ["=SUM(1)", "=PMT(1,2,3)", "=NPV(1,2)"]);
    }

    #[test]
    fn recent_keeps_ten_distinct_newest_first() {
        let mut r = default_recent();
        assert!(r.iter().all(|n| find(n).is_some()), "the default list names known functions");
        push_recent(&mut r, &["PMT".into(), "NPV".into()]);
        assert_eq!(r.len(), RECENT_MAX);
        assert_eq!(&r[..3], ["PMT", "NPV", "SUM"]);
        assert_eq!(r.iter().filter(|n| *n == "PMT").count(), 1);
        let many: Vec<String> = all().iter().take(25).map(|f| f.name.clone()).collect();
        push_recent(&mut r, &many);
        assert_eq!(r, many[..RECENT_MAX]);
    }
}
