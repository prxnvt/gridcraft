//! Equations in xlsx: Office Math with a plain-text fallback, the LaTeX source in `descr`, and
//! Office Math alone (as other writers save it) read back into LaTeX.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Arc;

use gridcraft_core::CellRef;
use gridcraft_model::*;

use crate::{read_xlsx, write_xlsx};

fn equation(id: u32, cell: &str, latex: &str, ink: Color) -> Shape {
    Shape {
        id,
        kind: ShapeKind::Equation,
        anchor: Anchor { cell: CellRef::parse(cell).unwrap(), width: 200.0, height: 60.0, mode: AnchorMode::MoveOnly, ..Default::default() },
        fill: Color::Auto,
        line: ink,
        text: latex.into(),
        math: Default::default(),
    }
}

fn book(shapes: Vec<Shape>) -> Workbook {
    let mut wb = Workbook::new();
    let mut sh = (*wb.sheets[0]).clone();
    for mut s in shapes {
        s.math.refresh(&s.text.clone());
        sh.shapes.push(s);
    }
    wb.sheets[0] = Arc::new(sh);
    wb
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut m = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        m.insert(f.name().to_string(), b);
    }
    m
}

fn rezip(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in parts {
        out.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

fn drawing(parts: &BTreeMap<String, Vec<u8>>) -> String {
    String::from_utf8(parts.iter().find(|(k, _)| k.starts_with("xl/drawings/drawing")).unwrap().1.clone()).unwrap()
}

fn shapes(wb: &Workbook) -> Vec<Shape> {
    wb.sheets[0].shapes.clone()
}

#[test]
fn equations_write_office_math_with_a_text_fallback_and_read_back() {
    let quad = "x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}";
    let cd = "\\begin{CD} A @>f>> B \\end{CD}";
    let wb = book(vec![
        equation(1, "B2", quad, Color::Rgb(0x1F4E79)),
        equation(2, "B8", cd, Color::Auto),
        equation(3, "B12", "a < b \\text{ \"q\" }\nc", Color::Auto),
    ]);
    let bytes = write_xlsx(&wb).unwrap();
    let parts = unzip(&bytes);
    let d = drawing(&parts);
    // The quadratic formula: a choice with Office Math and a fallback with the source as text.
    assert!(d.contains("<mc:Choice xmlns:a14=\"http://schemas.microsoft.com/office/drawing/2010/main\" Requires=\"a14\">"));
    assert!(d.contains("<a14:m><m:oMathPara xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">"));
    assert!(d.contains("<m:f><m:num>") && d.contains("<m:rad>"));
    assert!(d.contains("descr=\"gridcraft:latex:x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}\""));
    assert!(d.contains("<a:t>x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}</a:t>"), "the fallback shows the source");
    assert!(d.contains("<a:srgbClr val=\"1F4E79\"/>"), "the ink colour is kept");
    // A commutative diagram is outside the Office Math subset: text only, no choice.
    assert_eq!(d.matches("<mc:AlternateContent").count(), 2, "{d}");
    assert!(d.contains("descr=\"gridcraft:latex:\\begin{CD} A @&gt;f&gt;&gt; B \\end{CD}\""));
    // Read back: the source wins, exactly, including quotes and a newline.
    let (back, _) = read_xlsx(&bytes).unwrap();
    let s = shapes(&back);
    assert_eq!(s.len(), 3);
    assert!(s.iter().all(|x| x.kind == ShapeKind::Equation));
    assert_eq!(s[0].text, quad);
    assert_eq!(s[0].line, Color::Rgb(0x1F4E79));
    assert_eq!(s[1].text, cd);
    assert_eq!(s[2].text, "a < b \\text{ \"q\" }\nc");
    assert_eq!(s[0].anchor.mode, AnchorMode::MoveOnly);
    assert!((s[0].anchor.width - 200.0).abs() < 1.0);
}

#[test]
fn office_math_without_a_source_is_converted_to_latex() {
    let quad = "x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}";
    let wb = book(vec![equation(1, "B2", quad, Color::Auto), equation(2, "B8", "\\sum_{k=0}^{n} \\binom{n}{k} \\le \\infty", Color::Auto)]);
    let mut parts = unzip(&write_xlsx(&wb).unwrap());
    // Strip GridCraft's source, as a file saved by another application would be.
    let name = parts.keys().find(|k| k.starts_with("xl/drawings/drawing")).unwrap().clone();
    let mut d = drawing(&parts);
    while let Some(a) = d.find(" descr=\"gridcraft:latex:") {
        let b = d[a + 8..].find('"').unwrap() + a + 9;
        d.replace_range(a..b, "");
    }
    assert!(!d.contains("gridcraft:latex"));
    parts.insert(name, d.into_bytes());
    let (back, _) = read_xlsx(&rezip(&parts)).unwrap();
    let s = shapes(&back);
    assert_eq!(s.len(), 2);
    for (x, src) in s.iter().zip([quad, "\\sum_{k=0}^{n} \\binom{n}{k} \\le \\infty"]) {
        assert_eq!(x.kind, ShapeKind::Equation, "{}", x.text);
        // The conversion lays out to the same glyphs as the original.
        let glyphs = |t: &str| -> String {
            gridcraft_math::layout(t)
                .unwrap()
                .items
                .iter()
                .filter_map(|i| if let gridcraft_math::Item::Glyph(g) = i { Some(g.ch) } else { None })
                .collect()
        };
        assert_eq!(glyphs(&x.text), glyphs(src), "{} from {src}", x.text);
    }
}

#[test]
fn a_text_box_stays_a_text_box_and_hostile_sources_survive() {
    let mut wb = book(vec![equation(1, "B2", &"{".repeat(40), Color::Auto), equation(2, "B6", "\u{0}\u{1F600} & ]]> <!-- -->", Color::Auto)]);
    let mut sh = (*wb.sheets[0]).clone();
    sh.shapes.push(Shape {
        id: 3,
        kind: ShapeKind::TextBox,
        anchor: Anchor { cell: CellRef::parse("D2").unwrap(), width: 80.0, height: 30.0, ..Default::default() },
        fill: Color::Auto,
        line: Color::Auto,
        text: "plain".into(),
        math: Default::default(),
    });
    wb.sheets[0] = Arc::new(sh);
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    let s = shapes(&back);
    assert_eq!(s.len(), 3);
    assert_eq!(s[0].text, "{".repeat(40));
    assert_eq!(s[0].kind, ShapeKind::Equation);
    assert_eq!(s[1].text, "\u{1F600} & ]]> <!-- -->", "XML-illegal characters are dropped, the rest kept");
    assert_eq!(s[2].kind, ShapeKind::TextBox);
    assert_eq!(s[2].text, "plain");
}
