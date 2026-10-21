//! A small code editor: monospace text with syntax colours, Tab to indent, Enter that keeps the
//! indentation, and a scroll area that follows the caret. It colours Python (for code in cells,
//! where `xl()` reads the sheet) and LaTeX (for equation source); reached as
//! `widgets::code_editor`. The tokenizer and the key edits are pure functions, tested below.

use std::ops::Range;

use egui::text::{CCursor, CCursorRange};
use egui::{FontFamily, FontId, Key, Modifiers, Response, TextEdit, Ui};

use crate::theme::{self, Syntax, Tokens};

/// The language a code editor colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Python,
    Latex,
    Plain,
}

/// A coloured span's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Number,
    Comment,
    Builtin,
}

/// Spaces one Tab inserts.
pub const INDENT: &str = "    ";

const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "match", "case", "nonlocal", "not", "or", "pass", "raise", "return",
    "try", "while", "with", "yield",
];

const PY_BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "bool",
    "dict",
    "enumerate",
    "filter",
    "float",
    "format",
    "int",
    "isinstance",
    "len",
    "list",
    "map",
    "max",
    "min",
    "print",
    "range",
    "reversed",
    "round",
    "set",
    "sorted",
    "str",
    "sum",
    "tuple",
    "type",
    "zip",
    "xl",
];

/// The coloured spans of `text` (byte ranges on char boundaries, in order, not overlapping).
/// Anything not covered is plain text. Never panics, whatever the input.
pub fn highlight(text: &str, lang: Lang) -> Vec<(Range<usize>, Token)> {
    match lang {
        Lang::Python => python(text),
        Lang::Latex => latex(text),
        Lang::Plain => Vec::new(),
    }
}

fn python(text: &str) -> Vec<(Range<usize>, Token)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |i: usize| chars.get(i).map(|(b, _)| *b).unwrap_or(text.len());
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&(start, c)) = chars.get(i) {
        if c == '#' {
            let mut j = i;
            while chars.get(j).is_some_and(|(_, c)| *c != '\n') {
                j += 1;
            }
            out.push((start..end_of(j), Token::Comment));
            i = j;
        } else if c == '"' || c == '\'' {
            let triple = chars.get(i + 1).is_some_and(|(_, d)| *d == c) && chars.get(i + 2).is_some_and(|(_, d)| *d == c);
            let mut j = if triple { i + 3 } else { i + 1 };
            loop {
                match chars.get(j) {
                    None => break,
                    Some((_, '\\')) => j += 2,
                    Some((_, '\n')) if !triple => break,
                    Some((_, d)) if *d == c => {
                        if !triple {
                            j += 1;
                            break;
                        }
                        if chars.get(j + 1).is_some_and(|(_, d)| *d == c) && chars.get(j + 2).is_some_and(|(_, d)| *d == c) {
                            j += 3;
                            break;
                        }
                        j += 1;
                    }
                    Some(_) => j += 1,
                }
            }
            let j = j.min(chars.len());
            out.push((start..end_of(j), Token::String));
            i = j;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|(_, d)| d.is_ascii_digit())) {
            let mut j = i + 1;
            while chars.get(j).is_some_and(|(_, d)| d.is_ascii_alphanumeric() || *d == '_' || *d == '.') {
                j += 1;
            }
            out.push((start..end_of(j), Token::Number));
            i = j;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i + 1;
            while chars.get(j).is_some_and(|(_, d)| d.is_alphanumeric() || *d == '_') {
                j += 1;
            }
            let word = text.get(start..end_of(j)).unwrap_or("");
            if PY_KEYWORDS.contains(&word) {
                out.push((start..end_of(j), Token::Keyword));
            } else if PY_BUILTINS.contains(&word) {
                out.push((start..end_of(j), Token::Builtin));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn latex(text: &str) -> Vec<(Range<usize>, Token)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |i: usize| chars.get(i).map(|(b, _)| *b).unwrap_or(text.len());
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&(start, c)) = chars.get(i) {
        match c {
            '%' => {
                let mut j = i;
                while chars.get(j).is_some_and(|(_, c)| *c != '\n') {
                    j += 1;
                }
                out.push((start..end_of(j), Token::Comment));
                i = j;
            }
            '\\' => {
                let mut j = i + 1;
                if chars.get(j).is_some_and(|(_, d)| d.is_ascii_alphabetic()) {
                    while chars.get(j).is_some_and(|(_, d)| d.is_ascii_alphabetic()) {
                        j += 1;
                    }
                } else if chars.get(j).is_some() {
                    j += 1; // a control symbol such as \{ or \%
                }
                out.push((start..end_of(j), Token::Keyword));
                i = j;
            }
            '^' | '_' | '&' => {
                out.push((start..end_of(i + 1), Token::Builtin));
                i += 1;
            }
            d if d.is_ascii_digit() => {
                let mut j = i + 1;
                while chars.get(j).is_some_and(|(_, d)| d.is_ascii_digit() || *d == '.') {
                    j += 1;
                }
                out.push((start..end_of(j), Token::Number));
                i = j;
            }
            _ => i += 1,
        }
    }
    out
}

fn color(s: &Syntax, token: Token) -> egui::Color32 {
    match token {
        Token::Keyword => s.keyword,
        Token::String => s.string,
        Token::Number => s.number,
        Token::Comment => s.comment,
        Token::Builtin => s.builtin,
    }
}

/// The code editor's font: the monospace face at 12.5 px.
pub fn font() -> FontId {
    FontId::new(12.5, FontFamily::Name(theme::MONO.into()))
}

/// A layout job for `text` with syntax colours.
pub fn layout_job(text: &str, lang: Lang, font: &FontId, plain: egui::Color32, syntax: &Syntax, wrap_width: f32) -> egui::text::LayoutJob {
    layout_job_marked(text, lang, font, plain, syntax, &[], wrap_width)
}

/// A layout job for `text` with syntax colours, where `marks` (byte ranges, in order, not
/// overlapping) take their own colour over the syntax colour: the reference colours of the
/// `xl("…")` literals in Python cells. Ranges that are out of bounds or off a char boundary are
/// ignored.
pub fn layout_job_marked(
    text: &str,
    lang: Lang,
    font: &FontId,
    plain: egui::Color32,
    syntax: &Syntax,
    marks: &[(Range<usize>, egui::Color32)],
    wrap_width: f32,
) -> egui::text::LayoutJob {
    let ok = |r: &Range<usize>| r.start <= r.end && r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end);
    let tokens: Vec<(Range<usize>, egui::Color32)> = highlight(text, lang).into_iter().map(|(r, k)| (r, color(syntax, k))).collect();
    let marks: Vec<&(Range<usize>, egui::Color32)> = marks.iter().filter(|(r, _)| ok(r)).collect();
    // Cut the text at every span boundary, then colour each piece: a mark wins over a token.
    let mut cuts: Vec<usize> = vec![0, text.len()];
    cuts.extend(tokens.iter().flat_map(|(r, _)| [r.start, r.end]));
    cuts.extend(marks.iter().flat_map(|(r, _)| [r.start, r.end]));
    cuts.sort_unstable();
    cuts.dedup();
    let mut job = egui::text::LayoutJob::default();
    let (mut ti, mut mi) = (0usize, 0usize);
    for pair in cuts.windows(2) {
        let [a, b] = pair else { continue };
        let Some(piece) = text.get(*a..*b) else { continue };
        while tokens.get(ti).is_some_and(|(r, _)| r.end <= *a) {
            ti += 1;
        }
        while marks.get(mi).is_some_and(|(r, _)| r.end <= *a) {
            mi += 1;
        }
        let inside = |r: &Range<usize>| r.start <= *a && *b <= r.end;
        let colour = match (marks.get(mi), tokens.get(ti)) {
            (Some((r, c)), _) if inside(r) => *c,
            (_, Some((r, c))) if inside(r) => *c,
            _ => plain,
        };
        job.append(piece, 0.0, egui::TextFormat::simple(font.clone(), colour));
    }
    job.wrap.max_width = wrap_width;
    job
}

/// Underlines the byte range `mark` of `job` in `color` (an error's position), splitting the
/// sections it crosses. An empty range marks the character before it (the end of the text, for
/// an error about missing input). A range past the end of the text is clamped to it.
pub fn underline(job: &mut egui::text::LayoutJob, mark: Range<usize>, color: egui::Color32) {
    let text = job.text.as_str();
    let mut mark = mark.start.min(text.len())..mark.end.min(text.len());
    if mark.is_empty() {
        let start = text.get(..mark.start).and_then(|t| t.char_indices().next_back()).map_or(0, |(i, _)| i);
        mark = start..mark.start.max(start + text.get(start..).and_then(|t| t.chars().next()).map_or(0, char::len_utf8));
    }
    let mut out = Vec::with_capacity(job.sections.len() + 2);
    for sec in job.sections.drain(..) {
        let r = sec.byte_range.start.0..sec.byte_range.end.0;
        let (a, b) = (mark.start.clamp(r.start, r.end), mark.end.clamp(r.start, r.end));
        for (piece, marked) in [(r.start..a, false), (a..b, true), (b..r.end, false)] {
            if piece.is_empty() {
                continue;
            }
            let mut s = sec.clone();
            s.byte_range = egui::text::ByteIndex(piece.start)..egui::text::ByteIndex(piece.end);
            if marked {
                s.format.underline = egui::Stroke::new(1.5, color);
                s.format.color = color;
            }
            if s.byte_range.start.0 != r.start {
                s.leading_space = 0.0;
            }
            out.push(s);
        }
    }
    job.sections = out;
}

fn byte_of(text: &str, char_index: usize) -> usize {
    text.char_indices().nth(char_index).map(|(b, _)| b).unwrap_or(text.len())
}

/// Start (char index) of the line holding char `at`.
fn line_start(text: &str, at: usize) -> usize {
    let b = byte_of(text, at);
    text.get(..b).and_then(|s| s.rfind('\n')).map(|nl| text.get(..nl + 1).map(|s| s.chars().count()).unwrap_or(0)).unwrap_or(0)
}

/// What a key did to the text: the new caret (and selection anchor), as char indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    pub anchor: usize,
    pub caret: usize,
}

/// Enter: a newline that keeps the current line's indentation (one level more after a Python `:`).
/// `sel` is the selection as char indices (anchor, caret); it is replaced.
pub fn newline(text: &mut String, sel: (usize, usize), lang: Lang) -> Edit {
    let count = text.chars().count();
    let (a, b) = (sel.0.min(sel.1).min(count), sel.0.max(sel.1).min(count));
    let ls = line_start(text, a);
    let line_before: String = text.chars().skip(ls).take(a - ls.min(a)).collect();
    let mut indent: String = line_before.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    if lang == Lang::Python && line_before.trim_end().ends_with(':') {
        indent.push_str(INDENT);
    }
    let insert = format!("\n{indent}");
    let (ba, bb) = (byte_of(text, a), byte_of(text, b));
    text.replace_range(ba..bb, &insert);
    let caret = a + insert.chars().count();
    Edit { anchor: caret, caret }
}

/// Tab (indent) or Shift+Tab (dedent). A selection over several lines indents or dedents each
/// line; otherwise Tab inserts four spaces at the caret.
pub fn indent(text: &mut String, sel: (usize, usize), dedent: bool) -> Edit {
    let count = text.chars().count();
    let (a, b) = (sel.0.min(sel.1).min(count), sel.0.max(sel.1).min(count));
    let multi = text.chars().skip(a).take(b - a).any(|c| c == '\n');
    if !dedent && !multi {
        let (ba, bb) = (byte_of(text, a), byte_of(text, b));
        text.replace_range(ba..bb, INDENT);
        let caret = a + INDENT.len();
        return Edit { anchor: caret, caret };
    }
    // Line starts (char indices) of every line the selection touches.
    let mut starts = vec![line_start(text, a)];
    for (i, c) in text.chars().enumerate().skip(a).take(b - a) {
        if c == '\n' && i < b {
            starts.push(i + 1);
        }
    }
    let (mut na, mut nb) = (a, b);
    // Work from the last line back so earlier indices stay valid.
    for &s in starts.iter().rev() {
        let bs = byte_of(text, s);
        let n = if dedent {
            let n = text.get(bs..).map(|rest| rest.chars().take(INDENT.len()).take_while(|c| *c == ' ').count()).unwrap_or(0);
            text.replace_range(bs..bs + n, "");
            n
        } else {
            text.insert_str(bs, INDENT);
            0
        };
        let shift = |p: usize| {
            if dedent {
                if p > s { p - n.min(p - s) } else { p }
            } else if p >= s {
                p + INDENT.len()
            } else {
                p
            }
        };
        na = shift(na);
        nb = shift(nb);
    }
    if sel.1 >= sel.0 { Edit { anchor: na, caret: nb } } else { Edit { anchor: nb, caret: na } }
}

/// The code editor. `max_lines` rows are shown before it scrolls; the scroll area follows the
/// caret.
pub fn code_editor(ui: &mut Ui, id: egui::Id, text: &mut String, lang: Lang, max_lines: usize) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = font();
    let row_h = ui.ctx().fonts_mut(|f| f.row_height(&font));
    let max_lines = max_lines.clamp(1, 200);
    // Keys the editor handles itself, before the TextEdit sees them.
    let focused = ui.memory(|m| m.has_focus(id));
    let mut edited = None;
    if focused {
        let (tab, shift_tab, enter) = ui.input_mut(|i| {
            (i.consume_key(Modifiers::NONE, Key::Tab), i.consume_key(Modifiers::SHIFT, Key::Tab), i.consume_key(Modifiers::NONE, Key::Enter))
        });
        if tab || shift_tab || enter {
            let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            let count = text.chars().count();
            let sel = state.cursor.char_range().map(|r| (r.secondary.index.into(), r.primary.index.into())).unwrap_or((count, count));
            let e = if enter { newline(text, sel, lang) } else { indent(text, sel, shift_tab) };
            state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(e.anchor), CCursor::new(e.caret))));
            state.store(ui.ctx(), id);
            edited = Some(e);
        }
    }
    let syntax = t.syntax;
    let plain = t.text;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let job = layout_job(buf.as_str(), lang, &self::font(), plain, &syntax, wrap_width);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let frame_h = row_h * max_lines as f32 + 8.0;
    let mut response = None;
    egui::Frame::new().fill(t.input_bg).stroke(egui::Stroke::new(1.0, t.input_border)).corner_radius(4.0).inner_margin(4.0).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(id.with("scroll")).max_height(frame_h - 8.0).auto_shrink([false, true]).show(ui, |ui| {
            let out = TextEdit::multiline(text)
                .id(id)
                .font(font.clone())
                .code_editor()
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .desired_rows(1)
                .lock_focus(true)
                .layouter(&mut layouter)
                .show(ui);
            if let Some(e) = edited {
                let caret = out.galley.pos_from_cursor(CCursor::new(e.caret)).translate(out.galley_pos.to_vec2());
                ui.scroll_to_rect(caret, None);
            }
            let mut r = out.response.response;
            if edited.is_some() {
                r.mark_changed();
            }
            response = Some(r);
        });
    });
    response.unwrap_or_else(|| ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()))
}

/// The width of a line-number gutter for `lines` lines in `font` (the widest number, padded).
pub fn gutter_width(ui: &Ui, lines: usize, font: &FontId) -> f32 {
    let digits = lines.max(1).to_string().len().max(2);
    let w = ui.fonts_mut(|f| f.glyph_width(font, '0'));
    w * digits as f32 + 12.0
}

/// Paints line numbers in a gutter whose right edge is at `right`, beside `galley` laid out at
/// `galley_pos`: one number per line of the text, on the first row of each wrapped line.
pub fn paint_line_numbers(painter: &egui::Painter, galley: &egui::Galley, galley_pos: egui::Pos2, right: f32, font: &FontId, color: egui::Color32) {
    let mut line = 1usize;
    let mut starts_line = true;
    for row in &galley.rows {
        if starts_line {
            let y = galley_pos.y + row.pos.y;
            painter.text(egui::pos2(right, y), egui::Align2::RIGHT_TOP, line.to_string(), font.clone(), color);
            line = line.saturating_add(1);
        }
        starts_line = row.ends_with_newline;
    }
}

/// The byte range of line `line` (1-based) of `text` without its indentation, for underlining an
/// error's line; `None` when the text has no such line.
pub fn line_range(text: &str, line: u32) -> Option<Range<usize>> {
    let index = usize::try_from(line).ok()?.checked_sub(1)?;
    let mut start = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        if i == index {
            let indent = l.len() - l.trim_start().len();
            let body = l.trim_end();
            let (a, b) = (start + indent, start + body.len().max(indent));
            return Some(a..b);
        }
        start = start.saturating_add(l.len() + 1);
    }
    None
}

/// The char index where line `line` (1-based) of `text` starts, clamped to the text.
pub fn line_start_char(text: &str, line: u32) -> usize {
    let want = usize::try_from(line).unwrap_or(usize::MAX).saturating_sub(1);
    if want == 0 {
        return 0;
    }
    let mut seen = 0usize;
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            seen += 1;
            if seen == want {
                return i + 1;
            }
        }
    }
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str, lang: Lang) -> Vec<(&str, Token)> {
        highlight(text, lang).into_iter().map(|(r, k)| (text.get(r).unwrap_or("?"), k)).collect()
    }

    #[test]
    fn python_tokens() {
        let s = spans("def f(x):\n    return len(x) + 1.5  # done\nxl(\"A1:B2\")", Lang::Python);
        assert_eq!(
            s,
            vec![
                ("def", Token::Keyword),
                ("return", Token::Keyword),
                ("len", Token::Builtin),
                ("1.5", Token::Number),
                ("# done", Token::Comment),
                ("xl", Token::Builtin),
                ("\"A1:B2\"", Token::String),
            ]
        );
        assert_eq!(spans("s = '''a\n'b'\nc'''", Lang::Python), vec![("'''a\n'b'\nc'''", Token::String)]);
        assert_eq!(spans("x = 'it\\'s'", Lang::Python), vec![("'it\\'s'", Token::String)]);
        assert_eq!(spans("hello", Lang::Plain), vec![]);
    }

    #[test]
    fn underline_splits_sections_at_the_mark() {
        let t = Tokens::light();
        let font = FontId::new(12.0, FontFamily::Monospace);
        let marked = |job: &egui::text::LayoutJob| -> Vec<String> {
            job.sections
                .iter()
                .filter(|s| s.format.underline.width > 0.0)
                .map(|s| job.text[s.byte_range.start.0..s.byte_range.end.0].to_string())
                .collect()
        };
        let mut job = layout_job("a + \\foo", Lang::Latex, &font, t.text, &t.syntax, 100.0);
        underline(&mut job, 4..8, t.danger);
        assert_eq!(marked(&job), vec!["\\foo"]);
        let total: usize = job.sections.iter().map(|s| s.byte_range.end.0 - s.byte_range.start.0).sum();
        assert_eq!(total, job.text.len(), "sections still cover the text");
        // An empty mark at the end underlines the last character; out-of-range marks are clamped.
        let mut job = layout_job("x^", Lang::Latex, &font, t.text, &t.syntax, 100.0);
        underline(&mut job, 2..2, t.danger);
        assert_eq!(marked(&job), vec!["^"]);
        let mut job = layout_job("\u{e9}", Lang::Latex, &font, t.text, &t.syntax, 100.0);
        underline(&mut job, 50..99, t.danger);
        assert!(job.sections.iter().any(|s| s.format.underline.width > 0.0));
    }

    #[test]
    fn latex_tokens() {
        let s = spans("\\frac{a^2}{3} % half\n\\{", Lang::Latex);
        assert_eq!(
            s,
            vec![
                ("\\frac", Token::Keyword),
                ("^", Token::Builtin),
                ("2", Token::Number),
                ("3", Token::Number),
                ("% half", Token::Comment),
                ("\\{", Token::Keyword),
            ]
        );
    }

    #[test]
    fn hostile_input_never_panics_and_spans_stay_on_char_boundaries() {
        let inputs = [
            "",
            "'",
            "\"",
            "'''",
            "\"\"\"x",
            "#",
            "\\",
            "%",
            "1",
            ".",
            "é'ü\\",
            "θ = 'π' # ∑",
            "\u{1F600}'\u{1F600}",
            "'\\",
            "\\\u{1F600}",
            "\u{200B}\n\t'''\n",
            "0x_é",
            "a.b.c",
            "____",
        ];
        for text in inputs {
            for lang in [Lang::Python, Lang::Latex, Lang::Plain] {
                let mut last = 0;
                for (r, _) in highlight(text, lang) {
                    assert!(r.start >= last && r.start <= r.end && r.end <= text.len(), "{text:?} {lang:?} {r:?}");
                    assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end), "{text:?} {lang:?} {r:?}");
                    last = r.end;
                }
                let job = layout_job(text, lang, &FontId::default(), egui::Color32::BLACK, &Tokens::light().syntax, f32::INFINITY);
                assert_eq!(job.text, text, "every character is laid out once");
                let marks = [
                    (0..1, egui::Color32::RED),
                    (1..text.len().saturating_add(3), egui::Color32::RED),
                    (std::ops::Range { start: 2, end: 1 }, egui::Color32::RED),
                ];
                let job = layout_job_marked(text, lang, &FontId::default(), egui::Color32::BLACK, &Tokens::light().syntax, &marks, f32::INFINITY);
                assert_eq!(job.text, text, "every character is laid out once, whatever the marks");
            }
        }
    }

    #[test]
    fn marks_take_their_colour_over_syntax() {
        let text = "a = xl(\"A1:B2\") # c";
        let syntax = Tokens::light().syntax;
        let start = text.find("A1").unwrap_or(0);
        let marks = [(start..start + 5, egui::Color32::RED)];
        let job = layout_job_marked(text, Lang::Python, &FontId::default(), egui::Color32::BLACK, &syntax, &marks, f32::INFINITY);
        let colour_at = |b: usize| job.sections.iter().find(|s| s.byte_range.start.0 <= b && b < s.byte_range.end.0).map(|s| s.format.color);
        assert_eq!(colour_at(start), Some(egui::Color32::RED));
        assert_eq!(colour_at(start - 1), Some(syntax.string), "the quote keeps the string colour");
        assert_eq!(colour_at(text.len() - 1), Some(syntax.comment));
        assert_eq!(colour_at(0), Some(egui::Color32::BLACK));
    }

    #[test]
    fn enter_keeps_indentation_and_indents_after_a_colon() {
        let mut s = "    x = 1".to_string();
        let e = newline(&mut s, (9, 9), Lang::Python);
        assert_eq!(s, "    x = 1\n    ");
        assert_eq!(e.caret, 14);
        let mut s = "if x:".to_string();
        let e = newline(&mut s, (5, 5), Lang::Python);
        assert_eq!(s, "if x:\n    ");
        assert_eq!(e.caret, 10);
        // Plain text keeps the indentation but adds none after a colon.
        let mut s = "  a:".to_string();
        newline(&mut s, (4, 4), Lang::Plain);
        assert_eq!(s, "  a:\n  ");
        // A selection is replaced; out-of-range carets are clamped.
        let mut s = "abcdef".to_string();
        let e = newline(&mut s, (4, 2), Lang::Plain);
        assert_eq!((s.as_str(), e.caret), ("ab\nef", 3));
        let mut s = "é".to_string();
        let e = newline(&mut s, (99, 99), Lang::Plain);
        assert_eq!((s.as_str(), e.caret), ("é\n", 2));
    }

    #[test]
    fn error_lines_map_to_ranges_and_carets() {
        let text = "x = 1\n    y = x +\nz";
        assert_eq!(line_range(text, 1), Some(0..5));
        assert_eq!(line_range(text, 2).and_then(|r| text.get(r)), Some("y = x +"));
        assert_eq!(line_range(text, 3), Some(18..19));
        assert_eq!(line_range(text, 4), None);
        assert_eq!(line_range(text, 0), None);
        assert_eq!(line_range("", 1), Some(0..0));
        assert_eq!(line_start_char(text, 1), 0);
        assert_eq!(line_start_char(text, 2), 6);
        assert_eq!(line_start_char(text, 3), 18);
        assert_eq!(line_start_char(text, 99), text.chars().count());
        assert_eq!(line_start_char("\u{e9}\nb", 2), 2);
        assert_eq!(line_start_char(text, u32::MAX), text.chars().count());
    }

    #[test]
    fn tab_inserts_spaces_and_shift_tab_dedents() {
        let mut s = "ab".to_string();
        let e = indent(&mut s, (1, 1), false);
        assert_eq!((s.as_str(), e.caret), ("a    b", 5));
        let mut s = "a\nb\nc".to_string();
        indent(&mut s, (0, 3), false);
        assert_eq!(s, "    a\n    b\nc");
        indent(&mut s, (2, 9), true);
        assert_eq!(s, "a\nb\nc");
        let mut s = "  x".to_string();
        let e = indent(&mut s, (3, 3), true);
        assert_eq!((s.as_str(), e.caret), ("x", 1));
        let mut s = String::new();
        indent(&mut s, (7, 3), true);
        assert_eq!(s, "");
    }
}
