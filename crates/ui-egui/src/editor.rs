//! In-cell / formula-bar editing state: point mode, reference colouring, function
//! autocomplete and argument hints.

use gridcraft_engine::core::{CellRef, RangeRef};

/// How many distinct reference colours a formula cycles through (`Tokens::ref_colors`).
pub const REF_COLOR_COUNT: usize = 8;

#[derive(Clone, Debug)]
pub struct EditState {
    pub sheet: usize,
    /// Syntax of this edit, captured when it starts (prevents a language switch reinterpreting it).
    pub locale: gridcraft_engine::formula::FormulaLocale,
    pub cell: CellRef,
    pub text: String,
    /// Started by typing (Enter mode: arrows commit) rather than F2 (Edit mode: arrows move the caret).
    pub enter_mode: bool,
    pub from_formula_bar: bool,
    /// Caret position (char index), mirrored from the text widget.
    pub caret: usize,
    /// The reference most recently inserted by point mode (byte range) — arrows/clicks replace it.
    pub point: Option<(usize, usize)>,
    /// Where point mode's keyboard cursor is.
    pub point_cell: Option<CellRef>,
    pub autocomplete: Vec<String>,
    pub ac_index: usize,
    pub request_focus: bool,
    /// The text changed programmatically: push `caret` into the widget on the next frame.
    pub sync_caret: bool,
    /// Column AutoComplete: the full entry the typed text completes to.
    pub completion: Option<String>,
    /// Move the caret to the end on the next frame.
    pub caret_to_end: bool,
    /// Python mode: the text is a Python cell's code, edited in the formula bar's code editor and
    /// committed with Ctrl+Enter (Cmd+Enter on macOS) through `python.set`.
    pub python: bool,
    /// Python mode's point mode, on when Python mode starts (F2 toggles it): clicking or dragging
    /// on the grid inserts `xl("A1:B5")`. A click on the grid never commits code.
    pub py_point: bool,
    /// The edit left Python mode on a Python cell: committing it empty turns the cell back into a
    /// normal cell (`python.clear`, which undo reverses).
    pub py_left: bool,
}

impl EditState {
    pub fn new(sheet: usize, cell: CellRef, text: String, enter_mode: bool, from_formula_bar: bool) -> EditState {
        let caret = text.chars().count();
        EditState {
            sheet,
            locale: Default::default(),
            cell,
            text,
            enter_mode,
            from_formula_bar,
            caret,
            point: None,
            point_cell: None,
            autocomplete: vec![],
            ac_index: 0,
            request_focus: true,
            sync_caret: true,
            caret_to_end: true,
            completion: None,
            python: false,
            py_point: false,
            py_left: false,
        }
    }

    /// An edit of a Python cell's code, in the formula bar.
    pub fn python(sheet: usize, cell: CellRef, code: String) -> EditState {
        EditState { python: true, py_point: true, ..EditState::new(sheet, cell, code, false, true) }
    }

    /// Whether the text is a bare `=PY` (or `=PY(`), which enters Python mode on Tab or Enter.
    pub fn is_py_trigger(&self) -> bool {
        !self.python && {
            let t = self.text.trim();
            t.eq_ignore_ascii_case("=PY") || t.eq_ignore_ascii_case("=PY(")
        }
    }

    /// Switches to Python mode. A bare `=PY` trigger becomes empty code; other text is kept as the
    /// code (so a shortcut pressed mid-edit loses nothing).
    pub fn enter_python(&mut self) {
        if self.is_py_trigger() {
            self.text.clear();
        }
        self.python = true;
        self.py_point = true;
        self.py_left = false;
        self.from_formula_bar = true;
        self.enter_mode = false;
        self.point = None;
        self.point_cell = None;
        self.autocomplete.clear();
        self.completion = None;
        self.caret = self.text.chars().count();
        self.request_focus = true;
        self.sync_caret = true;
    }

    /// Leaves Python mode: Python goes off for the cell and the input is cleared. `was_python`
    /// says whether the cell is a Python cell now (committing then converts it back).
    pub fn leave_python(&mut self, was_python: bool) {
        self.python = false;
        self.py_point = false;
        self.py_left = was_python;
        self.text.clear();
        self.caret = 0;
        self.point = None;
        self.point_cell = None;
        self.request_focus = true;
        self.sync_caret = true;
    }

    pub fn is_formula(&self) -> bool {
        self.text.starts_with('=')
            || (self.text.starts_with('+') && self.text.len() > 1)
            || (self.text.starts_with('-') && self.text.len() > 1 && !self.text[1..].starts_with(|c: char| c.is_ascii_digit()))
    }

    fn byte_caret(&self) -> usize {
        self.text.char_indices().nth(self.caret).map(|(i, _)| i).unwrap_or(self.text.len())
    }

    /// Point mode: the caret follows an operator, `(`, `,` or `=` in a formula, so clicking a
    /// cell or pressing an arrow inserts a reference.
    pub fn can_point(&self) -> bool {
        if self.python {
            return self.py_point;
        }
        if !self.is_formula() {
            return false;
        }
        if let Some((_, e)) = self.point
            && e == self.byte_caret()
        {
            return true;
        }
        let before = self.text.get(..self.byte_caret()).unwrap_or("");
        let last = before.trim_end().chars().last();
        matches!(last, Some('=' | '(' | ',' | '+' | '-' | '*' | '/' | '^' | '&' | '<' | '>' | ':' | ';' | '{'))
    }

    /// Inserts (or replaces the last pointed) reference text at the caret. In Python mode the
    /// reference goes in as an `xl("…")` call.
    pub fn insert_ref(&mut self, r: &str) {
        let wrapped;
        let r = if self.python {
            wrapped = format!("xl(\"{}\")", r.replace('"', "\\\""));
            wrapped.as_str()
        } else {
            r
        };
        let caret_b = self.byte_caret();
        let (start, end) = match self.point {
            Some((s, e)) if e == caret_b => (s, e),
            _ => (caret_b, caret_b),
        };
        let (Some(a), Some(b)) = (self.text.get(..start), self.text.get(end..)) else { return };
        self.text = format!("{a}{r}{b}");
        let new_end = start + r.len();
        self.point = Some((start, new_end));
        self.caret = self.text.get(..new_end).map(|s| s.chars().count()).unwrap_or(0);
        self.caret_to_end = false;
        self.sync_caret = true;
    }

    /// Updates function autocomplete for the word before the caret.
    pub fn update_autocomplete(&mut self, names: &[String]) {
        self.autocomplete.clear();
        if self.python || !self.is_formula() {
            return;
        }
        let before = self.text.get(..self.byte_caret()).unwrap_or("");
        let word: String =
            before.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_').collect::<String>().chars().rev().collect();
        if word.is_empty() || word.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return;
        }
        let u = word.to_ascii_uppercase();
        // Not after a column letter of a reference like A1 (digits follow) – fine either way.
        self.autocomplete = names.iter().filter(|n| n.starts_with(&u)).take(12).cloned().collect();
        if self.autocomplete.len() == 1 && self.autocomplete[0] == u {
            self.autocomplete.clear();
        }
        self.ac_index = self.ac_index.min(self.autocomplete.len().saturating_sub(1));
    }

    /// Accepts the highlighted autocomplete entry: replaces the word and adds `(`.
    pub fn accept_autocomplete(&mut self) -> bool {
        let Some(name) = self.autocomplete.get(self.ac_index).cloned() else { return false };
        let caret_b = self.byte_caret();
        let before = self.text.get(..caret_b).unwrap_or("");
        let wlen: usize = before.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_').map(char::len_utf8).sum();
        let start = caret_b - wlen;
        let (Some(a), Some(b)) = (self.text.get(..start), self.text.get(caret_b..)) else { return false };
        self.text = format!("{a}{name}({b}");
        self.caret = self.text.get(..start + name.len() + 1).map(|s| s.chars().count()).unwrap_or(0);
        self.autocomplete.clear();
        self.caret_to_end = false;
        self.sync_caret = true;
        true
    }

    /// The function call the caret is inside and the argument index (for the hint tooltip).
    pub fn current_function(&self) -> Option<(String, usize)> {
        if self.python || !self.is_formula() {
            return None;
        }
        let before = self.text.get(..self.byte_caret())?;
        let mut depth = 0i32;
        let mut arg = 0usize;
        let chars: Vec<char> = before.chars().collect();
        let mut in_str = false;
        let mut i = chars.len();
        while i > 0 {
            i -= 1;
            let c = chars[i];
            if c == '"' {
                in_str = !in_str;
                continue;
            }
            if in_str {
                continue;
            }
            match c {
                ')' | '}' | ']' => depth += 1,
                '{' | '[' => depth -= 1,
                '(' => {
                    if depth == 0 {
                        let name: String = chars[..i]
                            .iter()
                            .rev()
                            .take_while(|c| c.is_ascii_alphanumeric() || **c == '.' || **c == '_')
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect();
                        if name.is_empty() {
                            return None;
                        }
                        return Some((name.to_ascii_uppercase(), arg));
                    }
                    depth -= 1;
                }
                c if c == self.locale.list_separator() && depth == 0 => arg += 1,
                _ => {}
            }
        }
        None
    }
}

/// References in formula text with their byte spans and colour index (same reference text ⇒
/// same colour), for colouring the editor and outlining ranges on the grid.
pub fn formula_refs(text: &str) -> Vec<(usize, usize, RangeRef, Option<String>, usize)> {
    let body = text.strip_prefix('=').or_else(|| text.strip_prefix('+')).unwrap_or(text);
    let offset = text.len() - body.len();
    let Ok(toks) = gridcraft_engine::formula::lexer::tokenize(body) else { return vec![] };
    let mut out: Vec<(usize, usize, RangeRef, Option<String>, usize)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        use gridcraft_engine::formula::lexer::Tok;
        let (sheet, start, j) = match &toks[i].tok {
            Tok::Sheet(a, _) => (Some(a.clone()), toks[i].start, i + 1),
            _ => (None, toks[i].start, i),
        };
        if let Some(Tok::Word(w)) = toks.get(j).map(|t| &t.tok)
            && let Some(c) = CellRef::parse(w)
        {
            let mut end = toks[j].end;
            let mut range = RangeRef::cell(c);
            if let (Some(Tok::Colon | Tok::Trim(_)), Some(Tok::Word(w2))) = (toks.get(j + 1).map(|t| &t.tok), toks.get(j + 2).map(|t| &t.tok))
                && let Some(c2) = CellRef::parse(w2)
            {
                range = RangeRef::new(c, c2);
                end = toks[j + 2].end;
                i = j + 3;
            } else {
                i = j + 1;
            }
            let key = body.get(start..end).unwrap_or("").replace('$', "").to_ascii_uppercase();
            let idx = match seen.iter().position(|k| *k == key) {
                Some(k) => k,
                None => {
                    seen.push(key);
                    seen.len() - 1
                }
            };
            out.push((start + offset, end + offset, range, sheet, idx % REF_COLOR_COUNT));
            continue;
        }
        i += 1;
    }
    out
}

/// The references in Python code's `xl("…")` calls, like [`formula_refs`]: the literal's byte span
/// (between the quotes), the range, its sheet and a colour index (same reference, same colour).
/// Names and tables have no range here and are left out.
pub fn python_refs(code: &str) -> Vec<(usize, usize, RangeRef, Option<String>, usize)> {
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for call in gridcraft_engine::model::python::xl_calls(code) {
        let (Some(reference), Some(span)) = (call.reference, call.span) else { continue };
        let refs = formula_refs(&format!("={reference}"));
        // Only a literal that is one reference (a cell or a range, maybe on another sheet).
        let [(a, b, range, sheet, _)] = refs.as_slice() else { continue };
        if *a != 1 || *b != reference.len() + 1 || !code.is_char_boundary(span.start) || !code.is_char_boundary(span.end) {
            continue;
        }
        let key = reference.replace('$', "").to_ascii_uppercase();
        let idx = match seen.iter().position(|k| *k == key) {
            Some(k) => k,
            None => {
                seen.push(key);
                seen.len() - 1
            }
        };
        out.push((span.start, span.end, *range, sheet.clone(), idx % REF_COLOR_COUNT));
    }
    out
}

/// Excel-style AutoComplete: the one distinct text entry in the same column (contiguous data
/// above and below) that starts with `typed` (case-insensitive). `None` when ambiguous.
pub fn column_completion(sheet: &gridcraft_engine::model::Sheet, at: CellRef, typed: &str) -> Option<String> {
    if typed.is_empty() || typed.starts_with('=') || typed.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') {
        return None;
    }
    let lower = typed.to_lowercase();
    let mut found: Option<String> = None;
    let mut ambiguous = false;
    let mut scan = |row: u32| -> bool {
        match sheet.value(CellRef::new(row, at.col)) {
            gridcraft_engine::core::Value::Empty => false,
            gridcraft_engine::core::Value::Text(t) => {
                if t.to_lowercase().starts_with(&lower) && t.len() > typed.len() {
                    match &found {
                        Some(f) if !f.eq_ignore_ascii_case(&t) => ambiguous = true,
                        None => found = Some(t.to_string()),
                        _ => {}
                    }
                }
                true
            }
            _ => true,
        }
    };
    let mut r = at.row;
    let mut n = 0;
    while r > 0 && n < 5000 {
        r -= 1;
        n += 1;
        if !scan(r) {
            break;
        }
    }
    let mut r = at.row + 1;
    n = 0;
    while n < 5000 && scan(r) {
        r += 1;
        n += 1;
    }
    if ambiguous { None } else { found }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_mode() {
        let mut e = EditState::new(0, CellRef::new(0, 0), "=SUM(".into(), true, false);
        assert!(e.can_point());
        e.insert_ref("A1");
        assert_eq!(e.text, "=SUM(A1");
        e.insert_ref("A2");
        assert_eq!(e.text, "=SUM(A2");
        e.text.push(':');
        e.caret += 1;
        e.point = None;
        assert!(e.can_point());
        let e2 = EditState::new(0, CellRef::new(0, 0), "hello".into(), true, false);
        assert!(!e2.can_point());
    }

    #[test]
    fn autocomplete_and_hints() {
        let names: Vec<String> = ["SUM", "SUMIF", "SUMPRODUCT", "AVERAGE"].iter().map(|s| s.to_string()).collect();
        let mut e = EditState::new(0, CellRef::new(0, 0), "=su".into(), true, false);
        e.update_autocomplete(&names);
        assert_eq!(e.autocomplete.len(), 3);
        assert!(e.accept_autocomplete());
        assert_eq!(e.text, "=SUM(");
        let e = EditState::new(0, CellRef::new(0, 0), "=IF(A1>0,SUM(B1,".into(), true, false);
        assert_eq!(e.current_function(), Some(("SUM".into(), 1)));
    }

    #[test]
    fn python_mode_points_with_xl_calls() {
        let mut e = EditState::new(0, CellRef::new(0, 0), "=py".into(), true, false);
        assert!(e.is_py_trigger());
        e.enter_python();
        assert!(e.python && e.from_formula_bar && e.text.is_empty());
        assert!(e.can_point(), "Python mode points from the start");
        e.py_point = false;
        assert!(!e.can_point(), "until F2 turns it off");
        e.text = "df = ".into();
        e.caret = 5;
        e.py_point = true;
        assert!(e.can_point());
        e.insert_ref("A1");
        assert_eq!(e.text, "df = xl(\"A1\")");
        e.insert_ref("A1:B5");
        assert_eq!(e.text, "df = xl(\"A1:B5\")", "a drag replaces the pointed call");
        e.leave_python(true);
        assert!(!e.python && e.py_left && e.text.is_empty());
        let mut e = EditState::new(0, CellRef::new(0, 0), "=SUM(".into(), true, false);
        e.update_autocomplete(&["PY".to_string()]);
        assert!(!e.is_py_trigger());
    }

    #[test]
    fn python_reference_spans() {
        let code = "a = xl(\"A1:B5\")\n# xl(\"C1\")\nb = xl('Sheet2!C3') + xl(\"a1:b5\") + xl(\"Sales\") + xl(name)";
        let r = python_refs(code);
        assert_eq!(r.len(), 3, "{r:?}");
        assert_eq!(code.get(r[0].0..r[0].1), Some("A1:B5"));
        assert_eq!(r[1].3.as_deref(), Some("Sheet2"));
        assert_eq!(r[0].4, r[2].4, "the same range takes the same colour");
        for hostile in ["xl(\"", "xl('é", "xl(\"A1\"", "\u{1F600}xl(\"A1\")", "xl(\"A1:\")", "xl(\"=A1\")"] {
            for (a, b, ..) in python_refs(hostile) {
                assert!(hostile.get(a..b).is_some());
            }
        }
    }

    #[test]
    fn ref_spans() {
        let r = formula_refs("=A1+B2:C3*a1+Sheet2!D4");
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].4, r[2].4); // A1 and a1 share a colour
        assert_eq!(r[1].2.a1(), "B2:C3");
        assert_eq!(r[3].3.as_deref(), Some("Sheet2"));
    }
}
