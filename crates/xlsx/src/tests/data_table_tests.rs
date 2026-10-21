//! What-If data tables: `<f t="dataTable">` on the first result cell, cached results in all of them.

use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::{Cell, DataTable, Formula, Workbook};

use super::minimal;
use crate::package::Package;
use crate::xml::El;
use crate::{read_xlsx, write_xlsx};

fn at(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}

fn table(range: &str, row_input: Option<&str>, col_input: Option<&str>) -> DataTable {
    DataTable { range: RangeRef::parse(range).unwrap(), row_input: row_input.map(at), col_input: col_input.map(at) }
}

fn cell<'a>(sheet: &'a El, address: &str) -> &'a El {
    sheet.child("sheetData").unwrap().kids("row").flat_map(|r| r.kids("c")).find(|c| c.attr("r") == Some(address)).unwrap()
}

fn attrs(f: &El) -> Vec<(String, String)> {
    ["t", "ref", "dt2D", "dtr", "r1", "r2"].iter().filter_map(|k| f.attr(k).map(|v| (k.to_string(), v.to_string()))).collect()
}

#[test]
fn data_tables_round_trip() {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    for (a, v) in [("B1", 0.05), ("B2", 1000.0), ("D3", 100.0), ("D4", 200.0), ("E3", 105.0), ("E4", 210.0)] {
        sh.cells.set(at(a), Cell::value(Value::Number(v)));
    }
    sh.cells.set(at("E2"), Cell::formula(Formula::new("B2*(1+B1)")));
    // One-variable down a column, two-variable, and one-variable across a row (text result).
    sh.data_tables.push(table("D2:E4", None, Some("B2")));
    sh.data_tables.push(table("G1:I3", Some("B1"), Some("B2")));
    sh.cells.set(at("L2"), Cell::value(Value::text("low")));
    sh.data_tables.push(table("K1:M2", Some("B1"), None));
    let bytes = write_xlsx(&wb).unwrap();

    let mut pkg = Package::open(&bytes).unwrap();
    let xml = pkg.read_xml("xl/worksheets/sheet1.xml").unwrap().unwrap();
    let f = |a: &str| attrs(cell(&xml, a).child("f").unwrap());
    let pairs = |v: &[(&str, &str)]| v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
    assert_eq!(f("E3"), pairs(&[("t", "dataTable"), ("ref", "E3:E4"), ("dt2D", "0"), ("dtr", "0"), ("r1", "B2")]));
    assert_eq!(f("H2"), pairs(&[("t", "dataTable"), ("ref", "H2:I3"), ("dt2D", "1"), ("dtr", "0"), ("r1", "B1"), ("r2", "B2")]));
    assert_eq!(f("L2"), pairs(&[("t", "dataTable"), ("ref", "L2:M2"), ("dt2D", "0"), ("dtr", "1"), ("r1", "B1")]));
    // Cached results: on the formula cell and as values in the others.
    assert_eq!(cell(&xml, "E3").child("v").unwrap().text, "105");
    assert_eq!(cell(&xml, "E4").child("v").unwrap().text, "210");
    assert!(cell(&xml, "E4").child("f").is_none());
    assert_eq!(cell(&xml, "L2").attr("t"), Some("str"));

    let (back, report) = read_xlsx(&bytes).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let sh = back.sheet(0).unwrap();
    // Read in cell order.
    let sorted = |mut v: Vec<DataTable>| {
        v.sort_by_key(|t| (t.range.start.row, t.range.start.col));
        v
    };
    assert_eq!(sorted(sh.data_tables.clone()), sorted(wb.sheet(0).unwrap().data_tables.clone()));
    assert_eq!(sh.value(at("E3")), Value::Number(105.0));
    assert_eq!(sh.value(at("E4")), Value::Number(210.0));
    assert_eq!(sh.value(at("L2")), Value::text("low"));
    assert!(sh.cell(at("E3")).unwrap().formula.is_none());
}

#[test]
fn damaged_data_tables_keep_their_values() {
    let rows = [
        // A valid table.
        r#"<row r="3"><c r="E3"><f t="dataTable" ref="E3:E4" dt2D="0" dtr="0" r1="B2"/><v>1</v></c></row>"#,
        r#"<row r="4"><c r="E4"><v>2</v></c></row>"#,
        // Overlaps the first one.
        r#"<row r="5"><c r="E5"><f t="dataTable" ref="E5:E6" dt2D="0" dtr="0" r1="B2"/><v>3</v></c></row>"#,
        // Deleted input, input inside the table, a two-variable table with one input, no room for edges.
        r#"<row r="10"><c r="H10"><f t="dataTable" ref="H10:H11" dt2D="0" dtr="0" r1="B2" del1="1"/><v>4</v></c><c r="K10"><f t="dataTable" ref="K10:K11" dt2D="0" dtr="0" r1="J11"/><v>5</v></c><c r="N10"><f t="dataTable" ref="N10:O11" dt2D="1" r1="B1"/><v>6</v></c></row>"#,
        r#"<row r="1"><c r="A1"><f t="dataTable" ref="A1:A2" dt2D="0" dtr="0" r1="B2"/><v>7</v></c></row>"#,
        // Too many result cells, a `ref` that doesn't start at the cell, no `ref` at all.
        r#"<row r="20"><c r="B20"><f t="dataTable" ref="B20:XFD1048576" dt2D="0" dtr="0" r1="B2"/><v>8</v></c><c r="Q20"><f t="dataTable" ref="R20:R21" r1="B2"/><v>9</v></c><c r="T20"><f t="dataTable" r1="B2"/><v>10</v></c></row>"#,
    ];
    let bytes = minimal(&format!("<sheetData>{}</sheetData>", rows.join("")), &[], "", "");
    let (wb, report) = read_xlsx(&bytes).unwrap();
    let sh = wb.sheet(0).unwrap();
    assert_eq!(sh.data_tables, vec![table("D2:E4", None, Some("B2"))]);
    assert!(!report.warnings.is_empty());
    for (a, v) in [("E5", 3.0), ("H10", 4.0), ("K10", 5.0), ("N10", 6.0), ("A1", 7.0), ("B20", 8.0), ("Q20", 9.0), ("T20", 10.0)] {
        assert_eq!(sh.value(at(a)), Value::Number(v), "{a}");
    }
}
