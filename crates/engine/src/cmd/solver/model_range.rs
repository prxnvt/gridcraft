//! Load/Save Model and Reset All: the model written on cells of its sheet, read back from them,
//! and removed from the sheet.

use super::model::{Rhs, model_formulas, parse_model, parse_model_formulas, save_model, saved_model};
use super::{
    Cell, CellRef, DefinedName, EngineError, Json, LOAD_CMD, MAX_CONSTRAINTS, RangeRef, Result, SAVE_CMD, Session, bad, edit, json, split_sheet,
    str_param,
};
use crate::cmd::cell_param_on;

/// `data.solverSaveModel`: writes the model on cells of its sheet, in one step.
pub(super) fn save_model_cells(s: &mut Session, p: &Json) -> Result<Json> {
    let d = s.doc()?;
    let params = match p.get("model") {
        Some(m @ Json::Object(_)) => m.clone(),
        Some(Json::Null) | None => {
            saved_model(&d.wb, d.wb.active_sheet()).ok_or_else(|| EngineError::Other("This sheet has no saved Solver model.".into()))?
        }
        Some(_) => return Err(bad(SAVE_CMD, "`model` must be data.solver params")),
    };
    let model = parse_model(s, &params)?;
    let to = cell_param_on(s, p, "to", model.sheet)?.ok_or_else(|| bad(SAVE_CMD, "missing `to` (the first cell of the saved model)"))?;
    let rows = model_formulas(&model)?;
    let count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    let last = to
        .row
        .checked_add(count.saturating_sub(1))
        .filter(|r| *r < gridcraft_core::MAX_ROWS)
        .ok_or_else(|| EngineError::Other("The saved model doesn't fit below that cell.".into()))?;
    let range = RangeRef::new(to, CellRef::new(last, to.col));
    // The model's own cells are never overwritten.
    let mut used = vec![model.objective];
    used.extend(&model.vars);
    for c in &model.cons {
        used.extend(&c.lhs);
        if let Rhs::Cells(r) = &c.rhs {
            used.extend(r);
        }
    }
    if used.iter().any(|c| range.contains(*c)) {
        return Err(EngineError::Other(format!("{} overlaps the model's cells; choose another place to save it.", range.a1())));
    }
    let sheet = model.sheet;
    edit(s, |cx| {
        let sh = cx.sheet_mut(sheet)?;
        let cells: Vec<CellRef> = range.iter().collect();
        for (c, f) in cells.iter().zip(&rows) {
            let mut cell = Cell::formula(gridcraft_model::Formula::new(f));
            cell.style = sh.style_id(*c);
            sh.set_cell(*c, cell);
        }
        for c in cells {
            cx.touch(sheet, c);
        }
        Ok(())
    })?;
    Ok(json!({"range": range.a1()}))
}

/// `data.solverLoadModel`: makes a model written by Save Model the active sheet's model.
pub(super) fn load_model_cells(s: &mut Session, p: &Json) -> Result<Json> {
    let d = s.doc()?;
    let active = d.wb.active_sheet();
    let text = str_param(p, "from").ok_or_else(|| bad(LOAD_CMD, "missing `from` (the saved model's range)"))?;
    let (sh, body) = split_sheet(text);
    if sh.is_some_and(|n| d.wb.sheet_index(&n) != Some(active)) {
        return Err(EngineError::Other("Load Model reads a model saved on the active sheet.".into()));
    }
    let r = RangeRef::parse(body.trim()).ok_or_else(|| bad(LOAD_CMD, format!("not a range: `{text}`")))?;
    let cap = MAX_CONSTRAINTS + 3;
    let sheet = d.wb.sheet(active).ok_or(EngineError::NoDocument)?;
    let formula = |c: CellRef| sheet.cell(c).and_then(|x| x.formula.as_ref()).map(|f| f.text.clone());
    let rows: Vec<String> = if r.is_single() {
        (0..cap as u32).map_while(|k| r.start.row.checked_add(k).and_then(|row| formula(CellRef::new(row, r.start.col)))).collect()
    } else {
        if r.count() > cap as u64 {
            return Err(EngineError::Other(format!("The range is too large for a saved model (at most {cap} cells).")));
        }
        r.iter().filter_map(formula).collect()
    };
    let params = parse_model_formulas(&rows)?;
    let model = parse_model(s, &params)?;
    if model.sheet != active {
        return Err(EngineError::Other("Load Model reads a model of the active sheet.".into()));
    }
    edit(s, |cx| {
        save_model(&mut cx.wb, &model);
        Ok(())
    })?;
    Ok(json!({"model": saved_model(&s.doc()?.wb, active)}))
}

/// Whether a defined name belongs to the saved model of `sheet`.
fn is_model_name(n: &DefinedName, sheet: usize) -> bool {
    n.scope == Some(sheet) && n.name.to_ascii_lowercase().starts_with("solver_")
}

/// `data.solverReset`: Reset All removes the active sheet's model. A model the sheet reads from
/// workbook-level names (which other sheets may read too) is left in place and masked by an
/// empty sheet-level `solver_opt`, so no other sheet's model changes.
pub(super) fn reset_model(s: &mut Session, _p: &Json) -> Result<Json> {
    let d = s.doc()?;
    let active = d.wb.active_sheet();
    let removed = d.wb.names.iter().filter(|n| is_model_name(n, active)).count();
    let mut after = d.wb.names.clone();
    after.retain(|n| !is_model_name(n, active));
    let masked = {
        let mut probe = (*d.wb).clone();
        probe.names = after;
        saved_model(&probe, active).is_some()
    };
    if removed > 0 || masked {
        edit(s, |cx| {
            cx.wb.names.retain(|n| !is_model_name(n, active));
            if masked {
                cx.wb.names.push(DefinedName {
                    name: "solver_opt".into(),
                    scope: Some(active),
                    formula: "\"\"".into(),
                    comment: String::new(),
                    hidden: true,
                });
            }
            Ok(())
        })?;
    }
    Ok(json!({"removed": removed, "masked": masked}))
}
