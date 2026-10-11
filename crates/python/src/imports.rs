//! What the built-in Python (Monty) can't run, found before any of a cell runs: imports of modules
//! it doesn't have. A cell that needs them is refused whole with a message pointing to CPython,
//! rather than half run.
//!
//! The scan is a small tokenizer, not a parser: it blanks strings and comments, joins lines inside
//! brackets and after a backslash, splits statements on `;` and `:`, and reads `import a.b, c` and
//! `from a.b import c` statements. It never fails: odd input just finds nothing.

/// The modules Monty 1.1 provides.
pub const MONTY_MODULES: &[&str] = &[
    "__future__",
    "asyncio",
    "base64",
    "binascii",
    "collections",
    "copy",
    "dataclasses",
    "datetime",
    "functools",
    "itertools",
    "json",
    "math",
    "os",
    "pathlib",
    "random",
    "re",
    "sys",
    "time",
    "typing",
    "unicodedata",
];

/// Characters of code scanned (Python code is capped at 7,000 per cell; this bounds hostile input
/// from elsewhere).
const MAX_SCAN: usize = 1_000_000;

/// The message for a cell that imports `module`, which the built-in Python doesn't have.
pub fn needs_cpython(module: &str) -> String {
    format!("This cell needs CPython: `{module}` isn't available in the built-in Python. Install Python 3.11 or newer and set it in Python Options.")
}

/// The message for a cell that uses something the built-in Python doesn't support, from the
/// message Monty's compiler gave.
pub fn unsupported(monty_message: &str) -> String {
    let m = monty_message.trim().trim_end_matches('.');
    let detail = match m.split_once("does not yet support ") {
        Some((_, rest)) => rest.to_string(),
        None => {
            let base = m.strip_suffix(" are not supported").or_else(|| m.strip_suffix(" is not supported")).unwrap_or(m);
            let mut chars = base.chars();
            chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default()
        }
    };
    format!("This cell needs CPython: the built-in Python doesn't support {detail}. Install Python 3.11 or newer and set it in Python Options.")
}

/// The first module `code` imports that the built-in Python doesn't have, with its line.
pub fn missing_module(code: &str) -> Option<(String, u32)> {
    imports(code).into_iter().find(|(m, _)| !MONTY_MODULES.contains(&m.as_str()))
}

/// The top-level modules `code` imports (`a` for `import a.b` and `from a.b import c`), with the
/// line of each statement. Relative imports are left out.
pub fn imports(code: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (stmt, line) in statements(code) {
        let words: Vec<&str> = stmt.split_whitespace().collect();
        match words.first().copied() {
            Some("import") => {
                let rest = stmt.trim_start().get("import".len()..).unwrap_or_default();
                for part in rest.split(',') {
                    let name = part.split_whitespace().next().unwrap_or_default();
                    push(&mut out, name, line);
                }
            }
            Some("from") => {
                if let Some(name) = words.get(1)
                    && !name.starts_with('.')
                    && words.get(2) == Some(&"import")
                {
                    push(&mut out, name, line);
                }
            }
            _ => {}
        }
    }
    out
}

fn push(out: &mut Vec<(String, u32)>, dotted: &str, line: u32) {
    let top = dotted.trim_matches(|c: char| c == '(' || c == ')').split('.').next().unwrap_or_default();
    if !top.is_empty() && top.chars().all(|c| c.is_alphanumeric() || c == '_') {
        out.push((top.to_string(), line));
    }
}

/// Statements with strings and comments blanked, each with the line it starts on.
fn statements(code: &str) -> Vec<(String, u32)> {
    let chars: Vec<char> = code.chars().take(MAX_SCAN).collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut line, mut start, mut depth) = (1u32, 1u32, 0usize);
    let mut i = 0;
    let flush = |cur: &mut String, start: u32, out: &mut Vec<(String, u32)>| {
        if !cur.trim().is_empty() {
            out.push((std::mem::take(cur), start));
        }
        cur.clear();
    };
    while let Some(&c) = chars.get(i) {
        match c {
            '#' => {
                while chars.get(i).is_some_and(|&c| c != '\n') {
                    i += 1;
                }
                continue;
            }
            '\'' | '"' => {
                let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
                i += if triple { 3 } else { 1 };
                cur.push_str(" \"\" ");
                while let Some(&d) = chars.get(i) {
                    if d == '\\' {
                        if chars.get(i + 1) == Some(&'\n') {
                            line = line.saturating_add(1);
                        }
                        i += 2;
                        continue;
                    }
                    if d == '\n' {
                        if !triple {
                            // An unclosed string ends at the line's end.
                            break;
                        }
                        line = line.saturating_add(1);
                    }
                    if d == c && (!triple || (chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c))) {
                        i += if triple { 3 } else { 1 };
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            '\\' if chars.get(i + 1) == Some(&'\n') => {
                line = line.saturating_add(1);
                cur.push(' ');
                i += 2;
                continue;
            }
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '\n' => {
                line = line.saturating_add(1);
                if depth == 0 {
                    flush(&mut cur, start, &mut out);
                    start = line;
                } else {
                    cur.push(' ');
                }
                i += 1;
                continue;
            }
            ';' | ':' if depth == 0 => {
                flush(&mut cur, start, &mut out);
                start = line;
                i += 1;
                continue;
            }
            _ => {}
        }
        if cur.trim().is_empty() {
            start = line;
        }
        cur.push(c);
        i += 1;
    }
    flush(&mut cur, start, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(code: &str) -> Vec<(String, u32)> {
        imports(code)
    }

    #[test]
    fn imports_are_found_in_any_form() {
        let code = "import pandas as pd, numpy\nfrom scipy.stats import norm\nimport os.path\nif True: import statistics\nx = 1; import re\nfrom . import local";
        let all = mods(code);
        let found: Vec<(&str, u32)> = all.iter().map(|(m, l)| (m.as_str(), *l)).collect();
        assert_eq!(found, [("pandas", 1), ("numpy", 1), ("scipy", 2), ("os", 3), ("statistics", 4), ("re", 5)]);
        assert_eq!(missing_module(code), Some(("pandas".into(), 1)));
        assert_eq!(missing_module("import math, json\nfrom collections import Counter"), None);
        // Multi-line statements count from their first line.
        assert_eq!(mods("x = (1,\n 2)\nfrom typing import (\n  List,\n)\nimport \\\n  numpy"), [("typing".into(), 3), ("numpy".into(), 6)]);
    }

    #[test]
    fn strings_and_comments_are_not_code() {
        assert!(mods("s = 'import pandas'\n# import numpy\nt = \"\"\"\nimport scipy\n\"\"\"\nu = 'a\\'b import x'").is_empty());
        assert_eq!(mods("s = '''x\n'''\nimport pandas"), [("pandas".into(), 3)]);
        assert!(mods("d = {'import': 1}\nprint('from x import y')").is_empty());
        assert!(mods("imported = 1\nfromage = 2\nimportlib_x = 3").is_empty());
    }

    #[test]
    fn hostile_code_never_breaks_the_scan() {
        for code in ["'", "'''", "\\", "(((((", ")))", "import", "from", "from x", "import ,,,", "\"\\", "#", "\u{0}import\u{0}x"] {
            let _ = imports(code);
        }
        let deep = "(".repeat(100_000) + "\nimport pandas";
        assert!(imports(&deep).is_empty(), "inside an unclosed bracket nothing is a statement");
        let long = "import a\n".repeat(200_000);
        assert!(!imports(&long).is_empty());
    }

    #[test]
    fn messages() {
        assert_eq!(
            needs_cpython("pandas"),
            "This cell needs CPython: `pandas` isn't available in the built-in Python. Install Python 3.11 or newer and set it in Python Options."
        );
    }
}
