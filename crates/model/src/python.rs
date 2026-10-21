//! Python cells: the code a cell holds, its last result, and the text forms around it (the
//! `PY(…)` formula files store it as, and the `xl("…")` references inside it).

use gridcraft_core::{CellError, Value};
use serde::{Deserialize, Serialize};

/// Characters of Python code one cell may hold, so its `PY(…)` formula stays under Excel's
/// 8,192-character formula limit.
pub const MAX_CODE_CHARS: usize = 7_000;

/// Characters in one string literal of a `PY(…)` formula (Excel's limit for a literal).
const CHUNK_CHARS: usize = 255;

/// A Python cell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PyCode {
    pub code: String,
    /// Show the result as a Python object (true) or convert it to cell values (false).
    pub object: bool,
    /// The last result, when the code ran without an error.
    pub output: Option<PyOutput>,
    /// The cell's inputs changed while calculation was manual: its value is out of date until
    /// the workbook is calculated.
    pub stale: bool,
}

/// A Python result kept on the cell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PyOutput {
    /// The result as cells (an array spills), or its preview as text when it has no cell form.
    pub value: Value,
    /// Python's name for the result's type.
    pub type_name: String,
    /// The result's `repr`, shortened.
    pub preview: String,
    /// Whether `value` is the result converted to cells.
    pub convertible: bool,
}

impl PyCode {
    pub fn new(code: &str) -> PyCode {
        PyCode { code: normalize_code(code), ..PyCode::default() }
    }

    /// What the cell shows for its result: cell values (an array spills from the cell), or, for a
    /// Python object, its type name. `None` before the code has run.
    pub fn result_value(&self) -> Option<Value> {
        let out = self.output.as_ref()?;
        let scalar = !matches!(out.value, Value::Array(_)) && out.convertible;
        Some(if self.object && !scalar { Value::text(out.type_name.as_str()) } else { out.value.clone() })
    }

    /// The formula a file stores the cell as (without `=`): `PY("…"&"…",0)`, with the code in
    /// string literals of at most 255 characters and 1 for a Python object result, 0 for values.
    pub fn to_formula(&self) -> String {
        let chunks = chunk(&self.code, CHUNK_CHARS);
        let lits: Vec<String> = chunks.iter().map(|c| format!("\"{}\"", c.replace('"', "\"\""))).collect();
        let lits = if lits.is_empty() { "\"\"".to_string() } else { lits.join("&") };
        format!("PY({lits},{})", u8::from(self.object))
    }

    /// Reads a `PY(…)` formula (with or without `=` and an `_xlfn.` prefix): string literals joined
    /// by `&`, optionally followed by `,0` or `,1`. `None` for anything else.
    pub fn from_formula(text: &str) -> Option<PyCode> {
        let body = text.trim().trim_start_matches('=').trim_start();
        let body = body.strip_prefix("_xlfn.").unwrap_or(body);
        let rest = body.get(..3).filter(|h| h.eq_ignore_ascii_case("PY(")).and_then(|_| body.get(3..))?;
        let mut chars = rest.chars().peekable();
        let mut code = String::new();
        loop {
            skip_space(&mut chars);
            if chars.next()? != '"' {
                return None;
            }
            loop {
                match chars.next()? {
                    '"' if chars.peek() == Some(&'"') => {
                        chars.next();
                        code.push('"');
                    }
                    '"' => break,
                    c => code.push(c),
                }
            }
            skip_space(&mut chars);
            match chars.next()? {
                '&' => continue,
                ',' => {
                    skip_space(&mut chars);
                    let object = match chars.next()? {
                        '0' => false,
                        '1' => true,
                        _ => return None,
                    };
                    skip_space(&mut chars);
                    return (chars.next()? == ')' && chars.all(char::is_whitespace)).then(|| PyCode { object, ..PyCode::new(&code) });
                }
                ')' => return chars.all(char::is_whitespace).then(|| PyCode::new(&code)),
                _ => return None,
            }
        }
    }
}

fn skip_space(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
}

/// Line breaks as `\n`.
pub fn normalize_code(code: &str) -> String {
    code.replace("\r\n", "\n").replace('\r', "\n")
}

/// `s` in pieces of at most `n` characters.
fn chunk(s: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(n.max(1)).map(|c| c.iter().collect()).collect()
}

/// Whether `text` (a formula, with or without `=`) is a `PY(` call: such formulas are Python cells,
/// set with `python.set`.
pub fn is_py_formula(text: &str) -> bool {
    let body = text.trim().trim_start_matches('=').trim_start();
    let body = body.strip_prefix("_xlfn.").unwrap_or(body);
    body.get(..3).is_some_and(|h| h.eq_ignore_ascii_case("PY(")) || body.eq_ignore_ascii_case("PY")
}

/// An `xl()` call in Python code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XlCall {
    /// The reference text, when the first argument is a plain string literal.
    pub reference: Option<String>,
    /// Byte range of the literal's contents (between the quotes) in the code.
    pub span: Option<std::ops::Range<usize>>,
}

/// The `xl(…)` calls in `code`, skipping comments and other string literals. A call whose first
/// argument isn't a plain string literal has no reference: the cells it reads are only known when
/// it runs.
pub fn xl_calls(code: &str) -> Vec<XlCall> {
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'#' => {
                // A comment runs to the end of the line.
                while bytes.get(i).is_some_and(|c| *c != b'\n') {
                    i += 1;
                }
            }
            b'"' | b'\'' => i = skip_string(bytes, i),
            b'x' if bytes.get(i + 1) == Some(&b'l') && (i == 0 || bytes.get(i - 1).is_some_and(|p| !is_ident(*p) && *p != b'.')) => {
                let mut j = i + 2;
                if bytes.get(j).is_some_and(|c| is_ident(*c)) {
                    i = j;
                    continue;
                }
                while bytes.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
                    j += 1;
                }
                if bytes.get(j) != Some(&b'(') {
                    i = j;
                    continue;
                }
                j += 1;
                while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                    j += 1;
                }
                let call = match bytes.get(j) {
                    Some(&q @ (b'"' | b'\'')) => {
                        let start = j + 1;
                        let end = skip_string(bytes, j);
                        // A plain literal: closed on its own line, no escapes, not part of an
                        // expression (`xl("A" + n)`), and not a triple-quoted string.
                        let close = end.saturating_sub(1);
                        let mut k = end;
                        while bytes.get(k).is_some_and(|c| *c == b' ' || *c == b'\t') {
                            k += 1;
                        }
                        let plain = close > start
                            && bytes.get(close) == Some(&q)
                            && bytes.get(start) != Some(&q)
                            && matches!(bytes.get(k), Some(b')' | b','))
                            && code.get(start..close).is_some_and(|t| !t.contains(['\\', '\n']));
                        i = end;
                        match code.get(start..close).filter(|_| plain) {
                            Some(t) => XlCall { reference: Some(t.to_string()), span: Some(start..close) },
                            None => XlCall { reference: None, span: None },
                        }
                    }
                    _ => {
                        i = j;
                        XlCall { reference: None, span: None }
                    }
                };
                out.push(call);
            }
            _ => i += 1,
        }
    }
    out
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// The index just past the string literal starting at `at` (single, double or triple quoted).
fn skip_string(bytes: &[u8], at: usize) -> usize {
    let Some(&q) = bytes.get(at) else { return at.saturating_add(1) };
    let triple = bytes.get(at + 1) == Some(&q) && bytes.get(at + 2) == Some(&q);
    let mut i = if triple { at + 3 } else { at + 1 };
    while let Some(&c) = bytes.get(i) {
        if c == b'\\' {
            i += 2;
            continue;
        }
        if c == q {
            if !triple {
                return i + 1;
            }
            if bytes.get(i + 1) == Some(&q) && bytes.get(i + 2) == Some(&q) {
                return i + 3;
            }
        } else if c == b'\n' && !triple {
            return i;
        }
        i += 1;
    }
    i
}

/// Rewrites the reference of every `xl("…")` literal in `code` with `f`; `None` from `f` leaves
/// it. Returns the new code when anything changed.
pub fn rewrite_xl(code: &str, mut f: impl FnMut(&str) -> Option<String>) -> Option<String> {
    let mut out = String::with_capacity(code.len());
    let mut last = 0usize;
    let mut changed = false;
    for call in xl_calls(code) {
        let (Some(reference), Some(span)) = (call.reference, call.span) else { continue };
        let Some(new) = f(&reference).filter(|n| *n != reference) else { continue };
        // A reference with the literal's quote in it (a sheet name) would end the string early.
        let quote = code.get(span.start.saturating_sub(1)..span.start).unwrap_or("\"");
        if new.contains(quote) || new.contains('\\') || new.contains('\n') {
            continue;
        }
        out.push_str(code.get(last..span.start).unwrap_or_default());
        out.push_str(&new);
        last = span.end;
        changed = true;
    }
    if !changed {
        return None;
    }
    out.push_str(code.get(last..).unwrap_or_default());
    Some(out)
}

/// The value a Python cell shows when it can't run: `#BLOCKED!`.
pub const BLOCKED: Value = Value::Error(CellError::Blocked);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_round_trip_with_chunks_and_quotes() {
        let code = format!("s = \"quoted\"\n{}\nlen(s)", "x".repeat(600));
        let py = PyCode { object: true, ..PyCode::new(&code) };
        let f = py.to_formula();
        assert!(f.starts_with("PY(\"s = \"\"quoted\"\"\n"));
        assert!(f.ends_with(",1)"));
        assert_eq!(f.matches("\"&\"").count(), 2, "{f}");
        let back = PyCode::from_formula(&format!("={f}")).unwrap();
        assert_eq!(back.code, code);
        assert!(back.object);
        let long = PyCode::new(&"y".repeat(MAX_CODE_CHARS));
        let f = long.to_formula();
        assert!(f.chars().count() < 8_192, "{}", f.chars().count());
        assert_eq!(PyCode::from_formula(&f).unwrap().code, long.code);
        assert_eq!(PyCode::from_formula("_xlfn.PY(\"1+1\")").unwrap().code, "1+1");
        assert_eq!(PyCode::from_formula(" py( \"a\" & \"b\" , 0 ) ").unwrap().code, "ab");
        assert_eq!(PyCode::new("a\r\nb").code, "a\nb");
        for bad in ["PY(A1)", "PY(\"a\",2)", "PY(\"a\"", "PY(\"a\")+1", "SUM(1)", "PY(\"a\"&B1)", "PY(", ""] {
            assert!(PyCode::from_formula(bad).is_none(), "{bad}");
        }
        assert!(is_py_formula("=PY(\"1\")") && is_py_formula("=py") && !is_py_formula("=PYTHON(1)"));
    }

    #[test]
    fn result_values() {
        let arr = gridcraft_core::Array::new(2, 1, vec![1.0.into(), 2.0.into()]).map(Value::from).unwrap();
        let out = PyOutput { value: arr.clone(), type_name: "list".into(), preview: "[1, 2]".into(), convertible: true };
        let mut py = PyCode { output: Some(out), ..PyCode::new("[1, 2]") };
        assert_eq!(py.result_value(), Some(arr));
        py.object = true;
        assert_eq!(py.result_value(), Some(Value::text("list")));
        py.output = Some(PyOutput { value: Value::Number(3.0), type_name: "int".into(), preview: "3".into(), convertible: true });
        assert_eq!(py.result_value(), Some(Value::Number(3.0)), "scalars show as values in object mode too");
        py.output = None;
        assert_eq!(py.result_value(), None);
    }

    #[test]
    fn finds_xl_literals() {
        let code = "a = xl(\"A1:B2\", headers=True)\n# xl('C1') in a comment\ns = \"xl('D1')\"\nb = xl( 'Sheet2!E5' )\nc = xl(name)\nd = df.xl('F1')\nxlx('G1')\ne = xl(\"A\" + \"1\")";
        let calls = xl_calls(code);
        let refs: Vec<Option<&str>> = calls.iter().map(|c| c.reference.as_deref()).collect();
        assert_eq!(refs, vec![Some("A1:B2"), Some("Sheet2!E5"), None, None]);
        let new = rewrite_xl(code, |r| (r == "A1:B2").then(|| "A2:B3".to_string())).unwrap();
        assert!(new.starts_with("a = xl(\"A2:B3\", headers=True)"));
        assert_eq!(new.len(), code.len());
        assert_eq!(rewrite_xl(code, |_| None), None);
        // A replacement can't break out of its string.
        assert_eq!(rewrite_xl("xl('A1')", |_| Some("'Q'!A1".into())), None);
        assert_eq!(rewrite_xl("xl(\"A1\")", |_| Some("'Q'!A1".into())), Some("xl(\"'Q'!A1\")".into()));
        // Unclosed and odd input doesn't loop or panic.
        for odd in ["xl(", "xl(\"", "xl('''A1''')", "x", "xl", "\"unterminated", "xl  (  \"A1\"  )", "é xl('A1')"] {
            let _ = xl_calls(odd);
            let _ = rewrite_xl(odd, |_| Some("B2".into()));
        }
        assert_eq!(xl_calls("xl  (  \"A1\"  )")[0].reference.as_deref(), Some("A1"));
        assert_eq!(xl_calls("xl('''A1''')")[0].reference, None);
    }
}
