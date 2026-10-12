//! Equations in drawings: Office Math Markup (ECMA-376 Part 1 §22.1) read back into LaTeX for the
//! common subset, the inverse of `gridcraft_math::to_omml`. A construct outside the subset makes
//! the whole equation unreadable as maths (`None`), and the caller keeps it as a text box.

use crate::xml::El;

/// Deepest OMML nesting converted.
const MAX_DEPTH: usize = 64;

/// The LaTeX for an `m:oMath` (or `m:oMathPara`) element, if every part is in the subset and the
/// result lays out.
pub fn to_latex(e: &El) -> Option<String> {
    let mut out = String::new();
    match e.name.as_str() {
        "oMathPara" => {
            let maths: Vec<&El> = e.kids("oMath").collect();
            for (i, m) in maths.iter().enumerate() {
                if i > 0 {
                    out.push_str(" \\\\ ");
                }
                seq(m, &mut out, 0)?;
            }
        }
        "oMath" => seq(e, &mut out, 0)?,
        _ => return None,
    }
    let out = out.trim().to_string();
    (!out.is_empty() && gridcraft_math::check(&out).is_ok()).then_some(out)
}

/// The first `m:oMath`/`m:oMathPara` under `e` (an `a14:m` element or a paragraph), searched to a
/// bounded depth.
pub fn find_math(e: &El, depth: usize) -> Option<&El> {
    if depth > 8 {
        return None;
    }
    if e.name == "oMathPara" || e.name == "oMath" {
        return Some(e);
    }
    e.children.iter().find_map(|c| find_math(c, depth + 1))
}

/// Children in order, skipping property elements.
fn seq(e: &El, out: &mut String, depth: usize) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    for c in &e.children {
        node(c, out, depth + 1)?;
    }
    Some(())
}

/// `{…}` around a child's contents (empty group when the child is missing).
fn group(e: Option<&El>, out: &mut String, depth: usize) -> Option<()> {
    out.push('{');
    if let Some(e) = e {
        seq(e, out, depth)?;
    }
    out.push('}');
    Some(())
}

fn val<'a>(props: Option<&'a El>, name: &str) -> Option<&'a str> {
    props.and_then(|p| p.child(name)).and_then(|c| c.attr("val"))
}

fn on(props: Option<&El>, name: &str) -> bool {
    props.and_then(|p| p.child(name)).is_some_and(|c| c.attr("val").is_none_or(|v| v == "1" || v == "on" || v == "true"))
}

/// LaTeX for one character of a math run.
fn char_latex(c: char) -> Option<&'static str> {
    Some(match c {
        'α' => "\\alpha ",
        'β' => "\\beta ",
        'γ' => "\\gamma ",
        'δ' => "\\delta ",
        'ε' => "\\varepsilon ",
        'ϵ' => "\\epsilon ",
        'ζ' => "\\zeta ",
        'η' => "\\eta ",
        'θ' => "\\theta ",
        'ϑ' => "\\vartheta ",
        'ι' => "\\iota ",
        'κ' => "\\kappa ",
        'λ' => "\\lambda ",
        'μ' => "\\mu ",
        'ν' => "\\nu ",
        'ξ' => "\\xi ",
        'π' => "\\pi ",
        'ρ' => "\\rho ",
        'σ' => "\\sigma ",
        'τ' => "\\tau ",
        'υ' => "\\upsilon ",
        'φ' => "\\varphi ",
        'ϕ' => "\\phi ",
        'χ' => "\\chi ",
        'ψ' => "\\psi ",
        'ω' => "\\omega ",
        'Γ' => "\\Gamma ",
        'Δ' => "\\Delta ",
        'Θ' => "\\Theta ",
        'Λ' => "\\Lambda ",
        'Ξ' => "\\Xi ",
        'Π' => "\\Pi ",
        'Σ' => "\\Sigma ",
        'Υ' => "\\Upsilon ",
        'Φ' => "\\Phi ",
        'Ψ' => "\\Psi ",
        'Ω' => "\\Omega ",
        '±' => "\\pm ",
        '∓' => "\\mp ",
        '×' => "\\times ",
        '÷' => "\\div ",
        '⋅' | '·' => "\\cdot ",
        '−' => "-",
        '≤' => "\\le ",
        '≥' => "\\ge ",
        '≠' => "\\ne ",
        '≈' => "\\approx ",
        '≡' => "\\equiv ",
        '∼' => "\\sim ",
        '∝' => "\\propto ",
        '∞' => "\\infty ",
        '∂' => "\\partial ",
        '∇' => "\\nabla ",
        '∀' => "\\forall ",
        '∃' => "\\exists ",
        '∈' => "\\in ",
        '∉' => "\\notin ",
        '⊂' => "\\subset ",
        '⊆' => "\\subseteq ",
        '⊃' => "\\supset ",
        '∪' => "\\cup ",
        '∩' => "\\cap ",
        '∅' => "\\emptyset ",
        '→' => "\\to ",
        '←' => "\\leftarrow ",
        '⇒' => "\\Rightarrow ",
        '⇔' => "\\Leftrightarrow ",
        '…' => "\\ldots ",
        '⋯' => "\\cdots ",
        '∘' => "\\circ ",
        '{' => "\\{",
        '}' => "\\}",
        '%' => "\\%",
        '#' => "\\#",
        '&' => "\\&",
        '$' => "\\$",
        '_' => "\\_",
        '\\' => "\\backslash ",
        '~' => "\\sim ",
        '^' => "\\hat{}",
        _ => return None,
    })
}

fn run_text(r: &El) -> String {
    r.kids("t").map(|t| t.text.as_str()).collect()
}

fn escape_text(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '{' | '}' | '%' | '#' | '&' | '$' | '_' => {
                o.push('\\');
                o.push(c);
            }
            '\\' => o.push_str("\\textbackslash "),
            '^' | '~' => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

/// Known operator names written upright in a run (`sin`, `log`, `lim`).
const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "log", "ln", "lg", "exp", "lim", "max", "min",
    "sup", "inf", "det", "gcd", "deg", "dim", "ker", "arg", "Pr", "liminf", "limsup",
];

fn run(r: &El, out: &mut String) -> Option<()> {
    let text = run_text(r);
    if text.is_empty() {
        return Some(());
    }
    let rpr = r.child("rPr");
    if on(rpr, "nor") {
        out.push_str("\\text{");
        out.push_str(&escape_text(&text));
        out.push('}');
        return Some(());
    }
    let plain = val(rpr, "sty") == Some("p");
    if plain && FUNCTIONS.contains(&text.trim()) {
        out.push('\\');
        out.push_str(text.trim());
        out.push(' ');
        return Some(());
    }
    let wrap = match (val(rpr, "scr"), val(rpr, "sty")) {
        (Some("double-struck"), _) => Some("\\mathbb{"),
        (Some("script"), _) => Some("\\mathcal{"),
        (Some("fraktur"), _) => Some("\\mathfrak{"),
        (Some("sans-serif"), _) => Some("\\mathsf{"),
        (Some("monospace"), _) => Some("\\mathtt{"),
        (_, Some("b")) => Some("\\mathbf{"),
        (_, Some("bi")) => Some("\\boldsymbol{"),
        (_, Some("p")) if text.chars().any(|c| c.is_alphabetic()) => Some("\\mathrm{"),
        _ => None,
    };
    if let Some(w) = wrap {
        out.push_str(w);
    }
    for c in text.chars() {
        match char_latex(c) {
            Some(l) => out.push_str(l),
            None => out.push(c),
        }
    }
    if wrap.is_some() {
        out.push('}');
    }
    Some(())
}

/// A delimiter character as a `\left`/`\right` argument.
fn delim(c: Option<&str>, default: &str) -> String {
    let c = c.unwrap_or(default);
    match c {
        "" => ".".into(),
        "{" => "\\{".into(),
        "}" => "\\}".into(),
        "⟨" | "〈" => "\\langle".into(),
        "⟩" | "〉" => "\\rangle".into(),
        "⌊" => "\\lfloor".into(),
        "⌋" => "\\rfloor".into(),
        "⌈" => "\\lceil".into(),
        "⌉" => "\\rceil".into(),
        "‖" => "\\|".into(),
        "∣" => "|".into(),
        other => other.into(),
    }
}

fn nary_op(chr: &str) -> Option<&'static str> {
    Some(match chr {
        "∑" => "\\sum",
        "∏" => "\\prod",
        "∐" => "\\coprod",
        "∫" => "\\int",
        "∬" => "\\iint",
        "∭" => "\\iiint",
        "∮" => "\\oint",
        "⋃" => "\\bigcup",
        "⋂" => "\\bigcap",
        "⋁" => "\\bigvee",
        "⋀" => "\\bigwedge",
        "⨁" => "\\bigoplus",
        "⨂" => "\\bigotimes",
        _ => return None,
    })
}

fn accent(chr: &str) -> Option<&'static str> {
    Some(match chr {
        "\u{0302}" | "^" => "\\hat",
        "\u{030C}" => "\\check",
        "\u{0303}" | "~" => "\\tilde",
        "\u{0301}" => "\\acute",
        "\u{0300}" => "\\grave",
        "\u{0307}" => "\\dot",
        "\u{0308}" => "\\ddot",
        "\u{0306}" => "\\breve",
        "\u{0305}" | "\u{0304}" | "¯" => "\\bar",
        "\u{20D7}" | "→" => "\\vec",
        "\u{20D6}" => "\\overleftarrow",
        "\u{030A}" => "\\mathring",
        _ => return None,
    })
}

fn node(e: &El, out: &mut String, depth: usize) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    match e.name.as_str() {
        // Properties and run-level DrawingML are not content.
        n if n.ends_with("Pr") => {}
        "r" => run(e, out)?,
        "f" => fraction(e, out, depth)?,
        "rad" => radical(e, out, depth)?,
        "sSup" | "sSub" | "sSubSup" => scripts(e, out, depth)?,
        "nary" => nary(e, out, depth)?,
        "d" => delimiters(e, out, depth)?,
        "m" => matrix(e, out, depth)?,
        "eqArr" => equation_array(e, out, depth)?,
        "acc" => {
            out.push_str(accent(val(e.child("accPr"), "chr").unwrap_or("\u{0302}"))?);
            group(e.child("e"), out, depth)?;
        }
        "bar" => {
            out.push_str(if val(e.child("barPr"), "pos") == Some("top") { "\\overline" } else { "\\underline" });
            group(e.child("e"), out, depth)?;
        }
        "groupChr" => group_char(e, out, depth)?,
        "limLow" | "limUpp" => limit(e, out, depth)?,
        "func" => {
            seq(e.child("fName")?, out, depth)?;
            out.push(' ');
            group(e.child("e"), out, depth)?;
        }
        "borderBox" => {
            out.push_str("\\boxed");
            group(e.child("e"), out, depth)?;
        }
        "phant" => {
            out.push_str("\\phantom");
            group(e.child("e"), out, depth)?;
        }
        "box" => group(e.child("e"), out, depth)?,
        _ => return None,
    }
    Some(())
}

/// `m:f`: a fraction, or a binomial-style stack without a bar.
fn fraction(e: &El, out: &mut String, depth: usize) -> Option<()> {
    if val(e.child("fPr"), "type") == Some("noBar") {
        out.push_str("\\genfrac{}{}{0pt}{}");
    } else {
        out.push_str("\\frac");
    }
    group(e.child("num"), out, depth)?;
    group(e.child("den"), out, depth)
}

/// `m:rad`: a square root, or a root with a degree.
fn radical(e: &El, out: &mut String, depth: usize) -> Option<()> {
    let hide = on(e.child("radPr"), "degHide");
    match e.child("deg").filter(|d| !hide && !d.children.is_empty()) {
        Some(d) => {
            out.push_str("\\sqrt[");
            seq(d, out, depth)?;
            out.push(']');
        }
        None => out.push_str("\\sqrt"),
    }
    group(e.child("e"), out, depth)
}

/// `m:sSup`, `m:sSub`, `m:sSubSup`: a base with scripts.
fn scripts(e: &El, out: &mut String, depth: usize) -> Option<()> {
    group(e.child("e"), out, depth)?;
    if let Some(sub) = e.child("sub") {
        out.push('_');
        group(Some(sub), out, depth)?;
    }
    if let Some(sup) = e.child("sup") {
        out.push('^');
        group(Some(sup), out, depth)?;
    }
    Some(())
}

/// `m:nary`: a large operator with its limits and operand.
fn nary(e: &El, out: &mut String, depth: usize) -> Option<()> {
    let p = e.child("naryPr");
    let op = nary_op(val(p, "chr").unwrap_or("∫"))?;
    out.push_str(op);
    if val(p, "limLoc") == Some("undOvr") && op.contains("int") {
        out.push_str("\\limits");
    }
    if !on(p, "subHide") {
        out.push('_');
        group(e.child("sub"), out, depth)?;
    }
    if !on(p, "supHide") {
        out.push('^');
        group(e.child("sup"), out, depth)?;
    }
    out.push(' ');
    group(e.child("e"), out, depth)
}

/// `m:d`: delimiters around one or more parts, with a separator between them.
fn delimiters(e: &El, out: &mut String, depth: usize) -> Option<()> {
    let p = e.child("dPr");
    out.push_str("\\left");
    out.push_str(&delim(val(p, "begChr"), "("));
    out.push(' ');
    let sep = delim(val(p, "sepChr"), "|");
    for (i, part) in e.kids("e").enumerate() {
        if i > 0 {
            out.push_str(" \\middle");
            out.push_str(&sep);
            out.push(' ');
        }
        seq(part, out, depth)?;
    }
    out.push_str(" \\right");
    out.push_str(&delim(val(p, "endChr"), ")"));
    Some(())
}

/// `m:m`: a matrix, row by row.
fn matrix(e: &El, out: &mut String, depth: usize) -> Option<()> {
    out.push_str("\\begin{matrix}");
    for (i, row) in e.kids("mr").enumerate() {
        if i > 0 {
            out.push_str(" \\\\ ");
        }
        for (k, cell) in row.kids("e").enumerate() {
            if k > 0 {
                out.push_str(" & ");
            }
            seq(cell, out, depth)?;
        }
    }
    out.push_str("\\end{matrix}");
    Some(())
}

/// `m:eqArr`: equations stacked one per row.
fn equation_array(e: &El, out: &mut String, depth: usize) -> Option<()> {
    out.push_str("\\begin{gathered}");
    for (i, row) in e.kids("e").enumerate() {
        if i > 0 {
            out.push_str(" \\\\ ");
        }
        seq(row, out, depth)?;
    }
    out.push_str("\\end{gathered}");
    Some(())
}

/// `m:groupChr`: a brace over or under its content.
fn group_char(e: &El, out: &mut String, depth: usize) -> Option<()> {
    let p = e.child("groupChrPr");
    let top = val(p, "pos") == Some("top");
    out.push_str(match val(p, "chr").unwrap_or("\u{23DF}") {
        "\u{23DE}" => "\\overbrace",
        "\u{23DF}" => "\\underbrace",
        _ if top => "\\overbrace",
        _ => "\\underbrace",
    });
    group(e.child("e"), out, depth)
}

/// `m:limLow`, `m:limUpp`: a limit under or over its base.
fn limit(e: &El, out: &mut String, depth: usize) -> Option<()> {
    let low = e.name == "limLow";
    // A function name with a limit (lim, max) takes it as a subscript with \limits.
    let base_is_fn = e.child("e").is_some_and(|b| {
        let t: String = b.kids("r").map(run_text).collect();
        b.children.iter().all(|c| c.name == "r") && FUNCTIONS.contains(&t.trim())
    });
    if base_is_fn {
        seq(e.child("e")?, out, depth)?;
        out.push_str(if low { "\\limits_" } else { "\\limits^" });
        group(e.child("lim"), out, depth)?;
    } else {
        out.push_str(if low { "\\underset" } else { "\\overset" });
        group(e.child("lim"), out, depth)?;
        group(e.child("e"), out, depth)?;
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(xml: &str) -> El {
        crate::xml::parse(format!("<m:oMath xmlns:m=\"m\" xmlns:a=\"a\">{xml}</m:oMath>").as_bytes()).unwrap()
    }

    fn latex(xml: &str) -> Option<String> {
        to_latex(&el(xml))
    }

    #[test]
    fn the_subset_reads_back() {
        assert_eq!(latex("<m:r><m:t>x+1</m:t></m:r>").as_deref(), Some("x+1"));
        assert_eq!(latex("<m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>").as_deref(), Some("\\frac{a}{b}"));
        assert_eq!(latex("<m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>").as_deref(), Some("{x}^{2}"));
        assert_eq!(
            latex("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e><m:r><m:t>π</m:t></m:r></m:e></m:rad>").as_deref(),
            Some("\\sqrt{\\pi }")
        );
        let sum = latex(
            "<m:nary><m:naryPr><m:chr m:val=\"∑\"/><m:limLoc m:val=\"undOvr\"/></m:naryPr><m:sub><m:r><m:t>i=1</m:t></m:r></m:sub><m:sup><m:r><m:t>n</m:t></m:r></m:sup><m:e><m:r><m:t>i</m:t></m:r></m:e></m:nary>",
        );
        assert_eq!(sum.as_deref(), Some("\\sum_{i=1}^{n} {i}"));
        // An integral without a character, with hidden limits.
        assert_eq!(latex("<m:nary><m:naryPr><m:subHide m:val=\"1\"/><m:supHide m:val=\"1\"/></m:naryPr><m:sub/><m:sup/><m:e><m:r><m:t>f</m:t></m:r></m:e></m:nary>").as_deref(), Some("\\int {f}"));
        let m = latex(
            "<m:d><m:dPr><m:begChr m:val=\"[\"/><m:endChr m:val=\"]\"/></m:dPr><m:e><m:m><m:mr><m:e><m:r><m:t>1</m:t></m:r></m:e><m:e><m:r><m:t>0</m:t></m:r></m:e></m:mr><m:mr><m:e><m:r><m:t>0</m:t></m:r></m:e><m:e><m:r><m:t>1</m:t></m:r></m:e></m:mr></m:m></m:e></m:d>",
        );
        assert_eq!(m.as_deref(), Some("\\left[ \\begin{matrix}1 & 0 \\\\ 0 & 1\\end{matrix} \\right]"));
        assert_eq!(latex("<m:r><m:rPr><m:sty m:val=\"p\"/></m:rPr><m:t>sin</m:t></m:r><m:r><m:t>x</m:t></m:r>").as_deref(), Some("\\sin x"));
        assert_eq!(latex("<m:r><m:rPr><m:scr m:val=\"double-struck\"/></m:rPr><m:t>R</m:t></m:r>").as_deref(), Some("\\mathbb{R}"));
        assert_eq!(latex("<m:r><m:rPr><m:nor/></m:rPr><m:t>if {x}</m:t></m:r>").as_deref(), Some("\\text{if \\{x\\}}"));
        assert_eq!(
            latex("<m:acc><m:accPr><m:chr m:val=\"\u{0302}\"/></m:accPr><m:e><m:r><m:t>x</m:t></m:r></m:e></m:acc>").as_deref(),
            Some("\\hat{x}")
        );
    }

    #[test]
    fn what_is_outside_the_subset_is_none() {
        assert_eq!(latex("<m:sPre><m:sub/><m:sup/><m:e/></m:sPre>"), None);
        assert_eq!(latex(""), None, "an empty equation is not maths");
        assert_eq!(latex("<m:nary><m:naryPr><m:chr m:val=\"☃\"/></m:naryPr><m:e/></m:nary>"), None);
        // Deep nesting stops instead of recursing without bound.
        let deep = format!("{}<m:r><m:t>x</m:t></m:r>{}", "<m:box><m:e>".repeat(100), "</m:e></m:box>".repeat(100));
        assert_eq!(latex(&deep), None);
    }

    #[test]
    fn writing_then_reading_keeps_the_maths() {
        for (_, src) in gridcraft_math::GALLERY {
            let omml = gridcraft_math::to_omml(src, 11.0).unwrap();
            let back =
                to_latex(&crate::xml::parse(format!("<x xmlns:m=\"m\" xmlns:a=\"a\">{omml}</x>").as_bytes()).unwrap().children[0].clone()).unwrap();
            let (a, b) = (gridcraft_math::layout(src).unwrap(), gridcraft_math::layout(&back).unwrap());
            let glyphs = |l: &gridcraft_math::MathList| -> String {
                l.items
                    .iter()
                    .filter_map(|i| if let gridcraft_math::Item::Glyph(g) = i { Some(g.ch) } else { None })
                    .filter(|c| !c.is_whitespace())
                    .collect()
            };
            assert_eq!(glyphs(&a), glyphs(&b), "{src} came back as {back}");
        }
    }
}
