//! Equations: painting a laid-out [`MathList`] with egui.
//!
//! Painting only ever reads the layout cached on the shape (`Shape::math`); it never lays an
//! equation out, so nothing here can reach the layout engine. Glyphs are drawn with the system's
//! maths fonts (`theme::MATH*`); a glyph no font has becomes a `[?]` box the size of its TeX box.

use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{Color32, FontFamily, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use gridcraft_engine::math::{Face, Item, MathList, PathCmd};
use gridcraft_engine::model::{Shape, Workbook};

use crate::theme::{self, Tokens};

/// Most points one outline may have; longer ones are stroked instead of filled.
const MAX_FILL_POINTS: usize = 4000;

fn family(face: Face) -> FontFamily {
    FontFamily::Name(
        match face {
            Face::Roman | Face::Large(_) => theme::MATH,
            Face::Italic => theme::MATH_ITALIC,
            Face::Bold | Face::BoldItalic => theme::MATH_BOLD,
            Face::Sans => theme::CELL,
            Face::SansBold => theme::CELL_BOLD,
            Face::SansItalic => theme::CELL_ITALIC,
            Face::Mono => theme::MONO,
        }
        .into(),
    )
}

/// Draws an equation shape in its frame `r` (screen pixels, zoom `z`).
pub fn paint_shape(p: &Painter, r: Rect, sp: &Shape, wb: &Workbook, z: f32) {
    let t = Tokens::get(p.ctx());
    let ink = sp.line.resolve(&wb.theme).map(theme::color32).unwrap_or(t.cell_text);
    let p = p.with_clip_rect(r.intersect(p.clip_rect()));
    match sp.math.get(&sp.text) {
        Some(Ok(list)) => {
            paint_list(&p, r, list, ink, gridcraft_engine::cmd::equation::PADDING * z);
        }
        // No layout (a source that doesn't parse, from a file): show the source in a dashed
        // frame so it can be found and fixed.
        other => {
            let font = FontId::new(13.0 * z, FontFamily::Name(theme::MONO.into()));
            let g = p.layout(sp.text.clone(), font, t.cell_text, (r.width() - 8.0 * z).max(1.0));
            p.galley(r.min + vec2(4.0, 4.0) * z, g, t.cell_text);
            if matches!(other, Some(Err(_))) {
                let dash = egui::Shape::dashed_line(
                    &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()],
                    Stroke::new(1.0, t.sheet_control_border),
                    4.0,
                    3.0,
                );
                p.extend(dash);
            }
        }
    }
}

/// The scale (screen pixels per em) at which `list` fits `r` inside `pad`.
pub fn fit(list: &MathList, r: Rect, pad: f32) -> f32 {
    let (w, h) = (list.width.max(0.01), list.total_height().max(0.01));
    let s = ((r.width() - 2.0 * pad) / w).min((r.height() - 2.0 * pad) / h);
    if s.is_finite() { s.clamp(0.0, 4000.0) } else { 0.0 }
}

/// Draws `list` centred in `r` at the largest scale that fits inside `pad`. Returns the scale.
pub fn paint_list(p: &Painter, r: Rect, list: &MathList, ink: Color32, pad: f32) -> f32 {
    let s = fit(list, r, pad);
    paint_at(p, r.center() - vec2(list.width, list.total_height()) * s / 2.0, list, ink, s);
    s
}

/// Draws `list` with its box's top-left at `o`, at `s` screen pixels per em.
pub fn paint_at(p: &Painter, o: Pos2, list: &MathList, ink: Color32, s: f32) {
    if s < 0.05 {
        return;
    }
    let col = |c: Option<[u8; 3]>| c.map_or(ink, |[r, g, b]| Color32::from_rgb(r, g, b));
    for item in &list.items {
        match item {
            Item::Glyph(g) => {
                let color = col(g.color);
                let (x, baseline) = (o.x + g.x * s, o.y + g.y * s);
                let large = matches!(g.face, Face::Large(_));
                // A large operator or delimiter is drawn at the height of its TeX box.
                let px = if large { (g.height + g.depth) * s } else { g.size * s }.clamp(0.0, 4000.0);
                if px < 1.0 {
                    continue;
                }
                let font = FontId::new(px, family(g.face));
                let ch = if renders(p, &font, g.ch) { Some(g.ch) } else { g.alt.filter(|a| renders(p, &font, *a)) };
                let Some(ch) = ch else {
                    unknown_box(
                        p,
                        Rect::from_min_max(pos2(x, baseline - g.height * s), pos2(x + g.width.max(0.3) * s, baseline + g.depth * s)),
                        color,
                    );
                    continue;
                };
                let galley = p.layout_no_wrap(ch.to_string(), font, color);
                let ascent = galley.rows.first().and_then(|row| row.glyphs.first().map(|gl| row.pos.y + gl.pos.y)).unwrap_or(px * 0.8);
                let (gx, gy) = if large {
                    // Centre the glyph on its box: maths fonts centre these on the maths axis,
                    // about a quarter em above the baseline.
                    let mid = o.y + (g.y - (g.height - g.depth) / 2.0) * s;
                    (x + (g.width * s - galley.size().x) / 2.0, mid + 0.25 * px - ascent)
                } else {
                    (x, baseline - ascent)
                };
                p.galley(pos2(gx, gy), galley, color);
            }
            Item::Rule(rule) => {
                let h = if rule.height > 0.0 { (rule.height * s).max(1.0) } else { 0.0 };
                let w = if rule.width > 0.0 { (rule.width * s).max(1.0) } else { 0.0 };
                if w > 0.0 && h > 0.0 {
                    p.rect_filled(Rect::from_min_size(pos2(o.x + rule.x * s, o.y + rule.y * s), vec2(w, h)), 0.0, col(rule.color));
                }
            }
            Item::Path(path) => {
                let at = pos2(o.x + path.x * s, o.y + path.y * s);
                let color = col(path.color);
                for sub in flatten(&path.commands, at, s) {
                    if path.fill && sub.len() >= 3 && sub.len() <= MAX_FILL_POINTS {
                        p.add(fill_polygon(&sub, color));
                    } else if sub.len() >= 2 {
                        p.add(PathShape::line(sub, PathStroke::new(1.0, color)));
                    }
                }
            }
        }
    }
}

/// Whether some face in `font`'s family draws `c` (rather than the replacement glyph).
///
/// `Fonts::has_glyph` alone reports a false negative for every character of the face that also
/// supplies the replacement glyph (a maths font usually has `◻`), so a "no" is double-checked by
/// comparing the glyph's atlas slot with the replacement glyph's.
pub fn renders(p: &Painter, font: &FontId, c: char) -> bool {
    if p.ctx().fonts_mut(|f| f.has_glyph(font, c)) {
        return true;
    }
    let uv = |c: char| {
        let g = p.layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE);
        g.rows.first().and_then(|r| r.glyphs.first().map(|gl| gl.uv_rect))
    };
    let Some(mine) = uv(c) else { return false };
    if mine.is_nothing() {
        return false;
    }
    ['\u{25FB}', '?'].iter().all(|r| *r == c || uv(*r) != Some(mine))
}

/// A glyph no font has: a box the size of its TeX box with a question mark.
fn unknown_box(p: &Painter, r: Rect, color: Color32) {
    if r.width() < 4.0 || r.height() < 4.0 {
        p.rect_filled(r, 0.0, color.gamma_multiply(0.4));
        return;
    }
    p.rect_stroke(r.shrink(0.5), 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
    let size = (r.height() * 0.6).min(r.width() * 0.9).max(4.0);
    p.text(r.center(), egui::Align2::CENTER_CENTER, "?", FontId::new(size, FontFamily::Name(theme::CELL.into())), color);
}

/// The outline as screen-space polygons, one per subpath, with curves as short lines.
fn flatten(cmds: &[PathCmd], at: Pos2, s: f32) -> Vec<Vec<Pos2>> {
    let pt = |x: f32, y: f32| pos2(at.x + x * s, at.y + y * s);
    let mut out: Vec<Vec<Pos2>> = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    for c in cmds {
        match *c {
            PathCmd::MoveTo(x, y) => {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                cur.push(pt(x, y));
            }
            PathCmd::LineTo(x, y) => cur.push(pt(x, y)),
            PathCmd::QuadTo(x1, y1, x, y) => {
                let p0 = cur.last().copied().unwrap_or(pt(x1, y1));
                let (c1, p1) = (pt(x1, y1), pt(x, y));
                for k in 1..=8 {
                    let t = k as f32 / 8.0;
                    let u = 1.0 - t;
                    cur.push(pos2(u * u * p0.x + 2.0 * u * t * c1.x + t * t * p1.x, u * u * p0.y + 2.0 * u * t * c1.y + t * t * p1.y));
                }
            }
            PathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                let p0 = cur.last().copied().unwrap_or(pt(x1, y1));
                let (c1, c2, p1) = (pt(x1, y1), pt(x2, y2), pt(x, y));
                for k in 1..=10 {
                    let t = k as f32 / 10.0;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    cur.push(pos2(a * p0.x + b * c1.x + c * c2.x + d * p1.x, a * p0.y + b * c1.y + c * c2.y + d * p1.y));
                }
            }
            PathCmd::Close => {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
            }
        }
    }
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// A filled simple polygon (convex or not), triangulated by ear clipping.
fn fill_polygon(pts: &[Pos2], color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    let mut pts: Vec<Pos2> = pts.to_vec();
    pts.dedup_by(|a, b| (*a - *b).length_sq() < 1e-6);
    if pts.len() > 2 && pts.first().zip(pts.last()).is_some_and(|(a, b)| (*a - *b).length_sq() < 1e-6) {
        pts.pop();
    }
    for q in &pts {
        mesh.colored_vertex(*q, color);
    }
    for [a, b, c] in triangulate(&pts) {
        mesh.add_triangle(a as u32, b as u32, c as u32);
    }
    mesh
}

fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

/// Ear clipping of a simple polygon. A degenerate polygon that has no ear left is finished as a
/// fan, so the result is always a valid (if imperfect) set of triangles.
fn triangulate(pts: &[Pos2]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return vec![];
    }
    let area: f32 =
        (0..n).map(|i| cross(Pos2::ZERO, pts.get(i).copied().unwrap_or_default(), pts.get((i + 1) % n).copied().unwrap_or_default())).sum();
    let ccw = area > 0.0;
    let at = |i: usize| pts.get(i).copied().unwrap_or_default();
    let mut idx: Vec<usize> = (0..n).collect();
    let mut tris = Vec::with_capacity(n);
    let mut guard = 0usize;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (ia, ib, ic) =
                (idx.get((k + m - 1) % m).copied().unwrap_or(0), idx.get(k).copied().unwrap_or(0), idx.get((k + 1) % m).copied().unwrap_or(0));
            let (a, b, c) = (at(ia), at(ib), at(ic));
            let turn = cross(a, b, c);
            if (turn > 0.0) != ccw || turn.abs() < 1e-9 {
                continue;
            }
            let inside = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let q = at(j);
                let (d1, d2, d3) = (cross(a, b, q), cross(b, c, q), cross(c, a, q));
                if ccw { d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0 } else { d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0 }
            });
            if inside {
                continue;
            }
            tris.push([ia, ib, ic]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    // Whatever is left (a triangle, or a degenerate remainder) as a fan.
    if let Some((&first, rest)) = idx.split_first() {
        for w in rest.windows(2) {
            if let [b, c] = *w {
                tris.push([first, b, c]);
            }
        }
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ear_clipping_covers_a_concave_polygon() {
        // An L shape: area 3 unit squares.
        let l = [pos2(0.0, 0.0), pos2(2.0, 0.0), pos2(2.0, 1.0), pos2(1.0, 1.0), pos2(1.0, 2.0), pos2(0.0, 2.0)];
        for pts in [l.to_vec(), l.iter().rev().copied().collect::<Vec<_>>()] {
            let t = triangulate(&pts);
            assert_eq!(t.len(), 4);
            let area: f32 = t.iter().map(|[a, b, c]| cross(pts[*a], pts[*b], pts[*c]).abs() / 2.0).sum();
            assert!((area - 3.0).abs() < 1e-4, "{area}");
        }
        assert!(triangulate(&[pos2(0.0, 0.0), pos2(1.0, 1.0)]).is_empty());
        // Collinear points still come back as (degenerate) triangles rather than looping.
        assert!(triangulate(&[pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(2.0, 0.0), pos2(3.0, 0.0)]).len() <= 2);
    }

    #[test]
    fn curves_are_flattened_per_subpath() {
        let cmds = [
            PathCmd::MoveTo(0.0, 0.0),
            PathCmd::QuadTo(1.0, 1.0, 2.0, 0.0),
            PathCmd::Close,
            PathCmd::MoveTo(5.0, 5.0),
            PathCmd::CubicTo(6.0, 6.0, 7.0, 6.0, 8.0, 5.0),
        ];
        let subs = flatten(&cmds, Pos2::ZERO, 10.0);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].len(), 9);
        assert_eq!(subs[1].len(), 11);
        assert_eq!(subs[1].last().copied(), Some(pos2(80.0, 50.0)));
    }
}
