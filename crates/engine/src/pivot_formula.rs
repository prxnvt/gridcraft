//! Formulas of calculated PivotTable fields and items.
//!
//! A formula names fields (or, for calculated items, items) bare (`Sales/Units`) or quoted with
//! single quotes (`'Unit Price'*2`). Names become placeholders before parsing, so a field called
//! `Q1` isn't read as a cell, and are bound to numbers when the formula is evaluated.

use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_formula::ast::Expr;
use gridcraft_model::Workbook;

/// Deepest chain of calculated fields that refer to calculated fields.
pub const MAX_DEPTH: usize = 32;
/// Longest formula accepted.
pub const MAX_LEN: usize = 8192;

const PREFIX: &str = "__PVF";

/// A parsed formula and the names it uses, in placeholder order.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub expr: Expr,
    /// Indexes into the `names` the formula was prepared against.
    pub refs: Vec<usize>,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// Parses `formula` (with or without `=`) where `names` are the names it may use.
/// `item_refs` also accepts `Field[Item]` and resolves it through `qualified`.
pub fn prepare(formula: &str, names: &[String], qualified: Option<&dyn Fn(&str, &str) -> Option<usize>>) -> Result<Prepared, String> {
    if formula.len() > MAX_LEN {
        return Err("The formula is too long.".into());
    }
    let body = formula.trim().strip_prefix('=').unwrap_or(formula.trim());
    if body.trim().is_empty() {
        return Err("Enter a formula.".into());
    }
    let find = |n: &str| names.iter().position(|x| x.eq_ignore_ascii_case(n.trim()));
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len() + 16);
    let mut refs: Vec<usize> = Vec::new();
    let mut place = |idx: usize, out: &mut String| {
        let at = match refs.iter().position(|r| *r == idx) {
            Some(p) => p,
            None => {
                refs.push(idx);
                refs.len() - 1
            }
        };
        out.push_str(&format!("{PREFIX}{at}"));
    };
    // A quoted or bare name starting at `i`: (name, index after it).
    let read_quoted = |i: usize| -> Result<(String, usize), String> {
        let mut j = i + 1;
        let mut name = String::new();
        loop {
            match chars.get(j) {
                None => return Err("A quoted name isn't closed.".into()),
                Some('\'') if chars.get(j + 1) == Some(&'\'') => {
                    name.push('\'');
                    j += 2;
                }
                Some('\'') => return Ok((name, j + 1)),
                Some(c) => {
                    name.push(*c);
                    j += 1;
                }
            }
        }
    };
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c == '"' {
            // A text literal, copied as is.
            out.push(c);
            i += 1;
            while let Some(&d) = chars.get(i) {
                out.push(d);
                i += 1;
                if d == '"' {
                    if chars.get(i) == Some(&'"') {
                        out.push('"');
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            // A number, with its exponent.
            while let Some(&d) = chars.get(i) {
                if d.is_ascii_digit() || d == '.' {
                    out.push(d);
                    i += 1;
                } else if (d == 'e' || d == 'E') && chars.get(i + 1).is_some_and(|x| x.is_ascii_digit() || *x == '+' || *x == '-') {
                    out.push(d);
                    i += 1;
                    if let Some(&s) = chars.get(i).filter(|s| **s == '+' || **s == '-') {
                        out.push(s);
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            continue;
        }
        let (name, next, quoted) = if c == '\'' {
            let (n, j) = read_quoted(i)?;
            (n, j, true)
        } else if is_ident(c) {
            let mut j = i;
            while chars.get(j).is_some_and(|x| is_ident(*x)) {
                j += 1;
            }
            (chars.get(i..j).map(|s| s.iter().collect::<String>()).unwrap_or_default(), j, false)
        } else {
            out.push(c);
            i += 1;
            continue;
        };
        let mut j = next;
        while chars.get(j).is_some_and(|x| x.is_whitespace()) {
            j += 1;
        }
        // Field[Item]
        if chars.get(j) == Some(&'[')
            && let Some(q) = qualified
        {
            let close = (j + 1..chars.len()).find(|k| chars.get(*k) == Some(&']')).ok_or("A [ isn't closed.")?;
            let mut item: String = chars.get(j + 1..close).map(|s| s.iter().collect()).unwrap_or_default();
            let t = item.trim();
            if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
                item = t.get(1..t.len() - 1).unwrap_or("").replace("''", "'");
            }
            let idx = q(&name, item.trim()).ok_or_else(|| format!("There's no item \"{}\" in \"{name}\".", item.trim()))?;
            place(idx, &mut out);
            i = close + 1;
            continue;
        }
        if !quoted && chars.get(j) == Some(&'(') {
            // A function.
            out.push_str(&name);
            i = next;
            continue;
        }
        match find(&name) {
            Some(idx) => place(idx, &mut out),
            None if !quoted && (name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE")) => out.push_str(&name),
            None => return Err(format!("\"{name}\" isn't a field or item this formula can use.")),
        }
        i = next;
    }
    let expr = gridcraft_formula::parse(&out).map_err(|_| "There's a problem with this formula.".to_string())?;
    let mut bad = false;
    expr.walk(&mut |e| {
        if matches!(e, Expr::Ref(_) | Expr::Struct(_)) || matches!(e, Expr::Name(n) if !n.starts_with(PREFIX)) {
            bad = true;
        }
    });
    if bad {
        return Err("PivotTable formulas can't refer to cells, ranges or defined names.".into());
    }
    Ok(Prepared { expr, refs })
}

fn bind(e: &Expr, vals: &[Value]) -> Expr {
    let lit = |v: &Value| match v {
        Value::Number(n) => Expr::Number(*n),
        Value::Error(err) => Expr::Error(*err),
        Value::Bool(b) => Expr::Bool(*b),
        Value::Text(t) => Expr::Text(t.clone()),
        _ => Expr::Number(0.0),
    };
    match e {
        Expr::Name(n) => match n.strip_prefix(PREFIX).and_then(|i| i.parse::<usize>().ok()).and_then(|i| vals.get(i)) {
            Some(v) => lit(v),
            None => Expr::Error(CellError::Name),
        },
        Expr::Array(rows) => Expr::Array(rows.iter().map(|r| r.iter().map(|x| bind(x, vals)).collect()).collect()),
        Expr::Unary(op, x) => Expr::Unary(*op, Box::new(bind(x, vals))),
        Expr::Paren(x) => Expr::Paren(Box::new(bind(x, vals))),
        Expr::Binary(op, a, b) => Expr::Binary(*op, Box::new(bind(a, vals)), Box::new(bind(b, vals))),
        Expr::Call(f, args) => Expr::Call(f.clone(), args.iter().map(|x| bind(x, vals)).collect()),
        Expr::Invoke(c, args) => Expr::Invoke(Box::new(bind(c, vals)), args.iter().map(|x| bind(x, vals)).collect()),
        other => other.clone(),
    }
}

/// Evaluates a prepared formula with `vals[i]` standing for its `i`th name.
pub fn eval(wb: &Workbook, sheet: usize, at: CellRef, p: &Prepared, vals: &[Value]) -> Value {
    let bound = bind(&p.expr, vals);
    match gridcraft_calc::evaluate_expr(wb, sheet, at, &bound) {
        Value::Array(a) => a.data.first().cloned().unwrap_or(Value::Error(CellError::Value)),
        Value::Empty => Value::Number(0.0),
        v => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &str, names: &[&str], vals: &[f64]) -> Result<Value, String> {
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        let p = prepare(f, &names, None)?;
        let vals: Vec<Value> = p.refs.iter().map(|i| Value::Number(vals[*i])).collect();
        let wb = Workbook::new();
        Ok(eval(&wb, 0, CellRef::default(), &p, &vals))
    }

    #[test]
    fn names_numbers_and_functions() {
        assert_eq!(run("=Sales/Units", &["Units", "Sales"], &[4.0, 10.0]), Ok(Value::Number(2.5)));
        // Q1 is a field here, not a cell.
        assert_eq!(run("=Q1*2", &["Q1"], &[3.0]), Ok(Value::Number(6.0)));
        assert_eq!(run("='Unit Price'*1.5E1", &["Unit Price"], &[2.0]), Ok(Value::Number(30.0)));
        assert_eq!(run("=IF(sales>5,ROUND(sales/3,1),0)", &["Sales"], &[10.0]), Ok(Value::Number(3.3)));
        assert_eq!(run("=Sales/0", &["Sales"], &[1.0]), Ok(Value::Error(CellError::Div0)));
        assert_eq!(run("=LEN(\"Sales\")+Sales", &["Sales"], &[1.0]), Ok(Value::Number(6.0)));
        assert!(run("=A1+Sales", &["Sales"], &[1.0]).is_err(), "A1 isn't a field");
        assert!(run("=Bogus*2", &["Sales"], &[1.0]).is_err());
        assert!(run("='Sales", &["Sales"], &[1.0]).is_err());
        assert!(run("", &["Sales"], &[1.0]).is_err());
        assert!(run("=Sales+", &["Sales"], &[1.0]).is_err());
    }

    #[test]
    fn qualified_items() {
        let names: Vec<String> = vec!["East".into(), "West".into()];
        let q = |f: &str, i: &str| if f.eq_ignore_ascii_case("Region") { names.iter().position(|n| n.eq_ignore_ascii_case(i)) } else { None };
        let p = prepare("=Region[East]+Region['West']*2", &names, Some(&q)).unwrap();
        let vals: Vec<Value> = p.refs.iter().map(|i| Value::Number([1.0, 10.0][*i])).collect();
        assert_eq!(eval(&Workbook::new(), 0, CellRef::default(), &p, &vals), Value::Number(21.0));
        assert!(prepare("=Region[North]", &names, Some(&q)).is_err());
    }
}
