//! The pictures of Python cells in a file: linked floating pictures, and what hostile files do to
//! them. The pictures here are real PNGs, built in code.

use std::io::Read;
use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Anchor, Cell, Image, PyCode, Sheet, Workbook};

use super::make_zip;
use crate::{read_xlsx, write_xlsx};

fn at(cell: &str) -> CellRef {
    CellRef::parse(cell).unwrap()
}

/// A PNG of `width` x `height` pixels. Only its header is read here, so its data can be short.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend_from_slice(&width.to_be_bytes());
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    v
}

fn text(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

fn python_cell(sheet: &mut Sheet, cell: &str, py: PyCode, value: Value) {
    sheet.cells.set(at(cell), Cell { value, formula: None, style: Default::default(), py: Some(Arc::new(py)) });
}

fn linked_picture(id: u32, source: &str, alt: &str, data: &[u8]) -> Image {
    let anchor = Anchor { cell: at("B2"), width: 40.0, height: 30.0, ..Default::default() };
    Image { id, anchor, data: data.to_vec().into(), mime: "image/png".into(), alt: alt.into(), python_source: Some(source.into()) }
}

fn book(sheets: Vec<Sheet>) -> Workbook {
    Workbook { sheets: sheets.into_iter().map(Arc::new).collect(), ..Default::default() }
}

/// A one-sheet package whose drawing holds one picture with this description, on B2, and whose sheet
/// has this cell XML in its first row. Its code is one script, `1 + 1`.
fn package_with_picture(descr: &str, cells: &str) -> Vec<u8> {
    let workbook = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2023/09/relationships/Python" Target="python.xml"/></Relationships>"#;
    let python = r#"<python xmlns="http://schemas.microsoft.com/office/spreadsheetml/2023/python"><pythonScripts><pythonScript><code>1 + 1</code></pythonScript></pythonScripts></python>"#;
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData><row r="2">{cells}</row></sheetData><drawing r:id="rId1"/></worksheet>"#
    );
    let sheet_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#;
    let drawing = format!(
        r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:oneCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="952500" cy="952500"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="2" name="P" descr="{}"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rId1"/></xdr:blipFill><xdr:spPr/></xdr:pic><xdr:clientData/></xdr:oneCellAnchor></xdr:wsDr>"#,
        crate::xml::esc_attr(descr)
    );
    let drawing_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>"#;
    let picture = png(40, 30);
    make_zip(&[
        ("_rels/.rels", super::ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/python.xml", python.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/drawings/drawing1.xml", drawing.as_bytes()),
        ("xl/drawings/_rels/drawing1.xml.rels", drawing_rels.as_bytes()),
        ("xl/media/image1.png", picture.as_slice()),
    ])
}

/// The Python Object cell B2 of [`package_with_picture`], its code being its one script.
const OBJECT_CELL: &str = r#"<c r="B2" t="str"><f>_xlfn._xlws.PY(0,1)</f><v>Image</v></c>"#;

#[test]
fn a_linked_picture_keeps_its_source_through_a_file() {
    // A sheet name can hold every separator a marker might use.
    let reference = "'It''s a! test sheet'!$G$5";
    let mut source = Sheet::new("It's a! test sheet");
    python_cell(&mut source, "G5", PyCode { object: true, output_chosen: true, ..PyCode::new("1") }, Value::text("Image"));
    let mut other = Sheet::new("Other");
    let picture = png(40, 30);
    other.images.push(linked_picture(1, reference, "A \"plot\": <one>!", &picture));
    let bytes = write_xlsx(&book(vec![source, other])).unwrap();

    // Any other application sees a normal picture, with the marker in its description.
    let drawing = text(&bytes, "xl/drawings/drawing1.xml");
    assert!(drawing.contains("<xdr:pic>"), "{drawing}");
    assert!(drawing.contains(&format!("descr=\"gridcraft:python:{}:", reference.chars().count())), "{drawing}");

    let (back, report) = read_xlsx(&bytes).unwrap();
    assert!(!report.warnings.iter().any(|w| w.contains("picture")), "{:?}", report.warnings);
    let image = &back.sheets[1].images[0];
    assert_eq!(image.python_source.as_deref(), Some(reference));
    assert_eq!(image.alt, "A \"plot\": <one>!");
    assert_eq!(image.data.as_slice(), picture);

    // A second save says the same.
    let again = write_xlsx(&back).unwrap();
    assert_eq!(text(&again, "xl/drawings/drawing1.xml"), drawing);
}

#[test]
fn a_link_is_read_by_its_length_whatever_the_text_around_it_holds() {
    use crate::python_pictures::{linked_descr, read_link};
    for (reference, alt) in [
        ("Data!$G$5", ""),
        ("Data!$G$5", ":::!!!"),
        ("'a:b'!$A$1", "'x'!$B$2"),
        ("'a!b''c d'!$XFD$1048576", "\u{1F600} picture"),
        ("'\u{1F600}'!A1", "12:'x'!A1:y"),
    ] {
        let descr = linked_descr(reference, alt).unwrap();
        assert_eq!(read_link(&descr), Some((reference, alt)), "{descr}");
    }
    // What a file can't hold is saved as a plain picture.
    assert_eq!(linked_descr("#REF!", "x"), None);
    assert_eq!(linked_descr("Data", "x"), None);
    assert_eq!(linked_descr(&format!("'{}'!A1", "n".repeat(200)), "x"), None);
    // A character XML can't carry never reaches the file, and isn't counted.
    let descr = linked_descr("'a\u{1}b'!A1", "x").unwrap();
    assert_eq!(read_link(&descr), Some(("'ab'!A1", "x")));
}

#[test]
fn a_malformed_link_leaves_a_plain_picture_with_the_description_as_its_text() {
    for descr in [
        "gridcraft:python:",
        "gridcraft:python:9",
        "gridcraft:python::Data!A1:x",
        "gridcraft:python:+7:Data!B2:x",
        "gridcraft:python:-7:Data!B2:x",
        "gridcraft:python:007x:Data!B2:x",
        "gridcraft:python:99999999999999999999999:Data!B2:x",
        "gridcraft:python:999:Data!B2:x",
        "gridcraft:python:8:Data!B2",
        "gridcraft:python:3:Data!B2:x",
        "gridcraft:python:7:Data!B2x",
        "gridcraft:python:5:#REF!:x",
        "gridcraft:python:2:B2:x",
        "gridcraft:python:3:!B2:x",
        "gridcraft:python:9:Data!B2:C:x",
        "gridcraft:python:18:Data!ZZZZ999999999:x",
        "gridcraft:python:6:\u{1F600}!B2:x",
    ] {
        let bytes = package_with_picture(descr, OBJECT_CELL);
        let (back, _) = read_xlsx(&bytes).unwrap();
        let images = &back.sheets[0].images;
        assert_eq!(images.len(), 1, "{descr}");
        assert_eq!(images[0].python_source, None, "{descr}");
        assert_eq!(images[0].alt, descr);
    }
}

#[test]
fn opening_a_file_never_deletes_a_picture_whose_link_names_no_python_cell() {
    for (reference, cells) in [
        ("Data!$B$2", r#"<c r="B2"><v>5</v></c>"#),
        ("Data!$C$9", OBJECT_CELL),
        ("Nowhere!$B$2", OBJECT_CELL),
        ("'Da''ta'!$B$2", OBJECT_CELL),
        ("'\u{1F600}!$B$2", OBJECT_CELL),
    ] {
        let descr = format!("gridcraft:python:{}:{reference}:my text", reference.chars().count());
        let (back, report) = read_xlsx(&package_with_picture(&descr, cells)).unwrap();
        let image = &back.sheets[0].images[0];
        assert_eq!(image.python_source, None, "{reference}");
        assert_eq!(image.alt, "my text", "{reference}");
        assert_eq!(image.data.as_slice(), png(40, 30));
        assert!(report.warnings.iter().any(|w| w.contains("lost the link")), "{:?}", report.warnings);
    }
    // A Python cell keeps it, in whichever case the reference spells the sheet.
    let descr = "gridcraft:python:9:DATA!$B$2:my text";
    let (back, report) = read_xlsx(&package_with_picture(descr, OBJECT_CELL)).unwrap();
    assert_eq!(back.sheets[0].images[0].python_source.as_deref(), Some("DATA!$B$2"));
    assert!(!report.warnings.iter().any(|w| w.contains("lost the link")), "{:?}", report.warnings);
}

#[test]
fn a_picture_saved_with_a_link_a_file_cannot_hold_is_a_plain_picture() {
    let mut sheet = Sheet::new("Data");
    sheet.images.push(linked_picture(1, "#REF!", "my text", &png(40, 30)));
    let (back, _) = read_xlsx(&write_xlsx(&book(vec![sheet])).unwrap()).unwrap();
    assert_eq!(back.sheets[0].images[0].python_source, None);
    assert_eq!(back.sheets[0].images[0].alt, "my text");
}
