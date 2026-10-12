//! Equations (Insert › Symbols › Equation): floating shapes holding LaTeX source, laid out by
//! `gridcraft-math` and drawn from the cached layout.
//!
//! An equation is drawn to fit its frame, so resizing the frame scales it. Inserting sizes the
//! frame for the requested font size; editing keeps the current scale and resizes the frame to
//! the new content.

use gridcraft_math::{MathError, MathList};
use gridcraft_model::{Anchor, AnchorMode, Color, Shape, ShapeKind};
use serde_json::{Value as Json, json};

use super::{CommandSpec, EngineError, PROTECTED, Result, Session, bad, cell_param, cmd, edit, f64_param, has_doc, ok, str_param, u32_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "insert.equation",
            "Equation",
            ["Insert", "Symbols"],
            None,
            "{latex: \"x^2\", cell?|at?: \"B2\", size?: 11 (points), color?: hex|\"auto\"}",
            has_doc,
            insert_equation
        ),
        cmd!("equation.edit", "Edit Equation", [], None, "{id, latex}", has_doc, edit_equation),
    ]
}

/// Default equation font size, in points.
pub const DEFAULT_SIZE_PT: f64 = 11.0;
/// Space between an equation's frame and its content, in sheet pixels.
pub const PADDING: f32 = 4.0;
/// Sheet pixels per point.
const PX_PER_PT: f64 = 96.0 / 72.0;

/// The scale (sheet pixels per em) at which `list` fits a frame of `w` by `h` pixels.
pub fn fit_scale(list: &MathList, w: f32, h: f32) -> f32 {
    let (cw, ch) = (list.width.max(0.01), list.total_height().max(0.01));
    let s = ((w - 2.0 * PADDING) / cw).min((h - 2.0 * PADDING) / ch);
    if s.is_finite() { s.max(0.0) } else { 0.0 }
}

/// The frame size for `list` at `scale` pixels per em.
fn frame_for(list: &MathList, scale: f32) -> (f32, f32) {
    let w = list.width.max(0.5) * scale + 2.0 * PADDING;
    let h = list.total_height().max(1.0) * scale + 2.0 * PADDING;
    (w.clamp(8.0, 20_000.0), h.clamp(8.0, 20_000.0))
}

/// `latex` laid out, or a readable error with the position of the problem.
fn lay_out(cmd: &str, latex: &str) -> Result<MathList> {
    gridcraft_math::layout(latex).map_err(|e| bad(cmd, describe(latex, &e)))
}

/// "Error at character 5: Undefined control sequence: \foo".
pub fn describe(latex: &str, e: &MathError) -> String {
    match e.char_pos(latex) {
        Some(p) => format!("equation error at character {p}: {}", e.message),
        None => format!("equation error: {}", e.message),
    }
}

fn insert_equation(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "insert.equation";
    let latex = str_param(p, "latex").ok_or_else(|| bad(CMD, "`latex` must be the equation's LaTeX source, such as \"x^2 + y^2 = r^2\""))?;
    if latex.trim().is_empty() {
        return Err(bad(CMD, "`latex` is empty"));
    }
    let size = f64_param(p, "size").unwrap_or(DEFAULT_SIZE_PT);
    if !(1.0..=409.0).contains(&size) {
        return Err(bad(CMD, "`size` must be between 1 and 409 points"));
    }
    let color = match p.get("color") {
        None | Some(Json::Null) => Color::Auto,
        Some(c) => super::format::color_param(Some(c)).ok_or_else(|| bad(CMD, "`color` must be a hex colour such as \"#1F4E79\", or \"auto\""))?,
    };
    let list = lay_out(CMD, latex)?;
    let at = match cell_param(s, p, "cell")? {
        Some(c) => Some(c),
        None => cell_param(s, p, "at")?,
    };
    let d = s.doc()?;
    if d.wb.active().is_some_and(|sh| sh.is_protected()) {
        return Err(EngineError::Other(PROTECTED.into()));
    }
    let at = at.unwrap_or(d.selection.active);
    let id = d.wb.next_object_id();
    let (width, height) = frame_for(&list, (size * PX_PER_PT) as f32);
    let mut shape = Shape {
        id,
        kind: ShapeKind::Equation,
        anchor: Anchor { cell: at, dx: 0.0, dy: 0.0, width, height, mode: AnchorMode::MoveOnly },
        fill: Color::Auto,
        line: color,
        text: latex.to_string(),
        format: Default::default(),
        math: Default::default(),
    };
    shape.math.refresh(latex);
    let sheet = d.wb.active_sheet();
    edit(s, |cx| {
        cx.sheet_mut(sheet)?.shapes.push(shape);
        Ok(json!({"equation": id}))
    })
}

fn edit_equation(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "equation.edit";
    let id = u32_param(p, "id").ok_or_else(|| bad(CMD, "`id` must be an equation id"))?;
    let latex = str_param(p, "latex").ok_or_else(|| bad(CMD, "`latex` must be the equation's LaTeX source"))?;
    if latex.trim().is_empty() {
        return Err(bad(CMD, "`latex` is empty; delete the equation instead"));
    }
    let d = s.doc()?;
    let sheet = d.wb.active_sheet();
    let sh = d.wb.active().ok_or_else(|| bad(CMD, "no such equation"))?;
    let shape = sh.shapes.iter().find(|x| x.id == id && x.kind == ShapeKind::Equation).ok_or_else(|| bad(CMD, "no such equation"))?;
    if sh.is_protected() {
        return Err(EngineError::Other(PROTECTED.into()));
    }
    if shape.text == latex {
        return ok();
    }
    let list = lay_out(CMD, latex)?;
    // Keep the size the equation is drawn at now; without a layout, use the default size.
    let scale = match shape.math.get(&shape.text) {
        Some(Ok(old)) => fit_scale(old, shape.anchor.width, shape.anchor.height),
        _ => 0.0,
    };
    let scale = if scale > 0.1 { scale } else { (DEFAULT_SIZE_PT * PX_PER_PT) as f32 };
    let (width, height) = frame_for(&list, scale);
    edit(s, |cx| {
        let shape = cx.sheet_mut(sheet)?.shapes.iter_mut().find(|x| x.id == id).ok_or_else(|| bad(CMD, "no such equation"))?;
        shape.text = latex.to_string();
        shape.math.refresh(latex);
        shape.anchor.width = width;
        shape.anchor.height = height;
        Ok(json!({"equation": id}))
    })
}
