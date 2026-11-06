//! The pictures of Python cells in a drawing part. How Excel stores a Python image isn't documented,
//! so they are ordinary pictures whose descriptions say what they are, as an equation's does
//! (`gridcraft:latex:`). Any other application sees a normal picture.
//!
//! A floating picture linked to a Python cell has the description
//! `gridcraft:python:<n>:<reference>:<alt>`. A sheet name can hold any separator, so `<n>`, the number
//! of characters of the reference, is what ends it; characters and not bytes, so that a script holding
//! the description as text can slice it. A picture whose description doesn't read this way is a plain
//! picture, with the description as its alternative text.

use std::collections::HashMap;

use gridcraft_core::CellRef;
use gridcraft_core::addr::unquote_sheet;
use gridcraft_model::Workbook;

use crate::xml;

const LINK: &str = "gridcraft:python:";
/// Longest reference a marker holds: a quoted sheet name of 31 characters, each doubled, and the
/// largest cell come to 77. A crafted file can claim any length.
const MAX_REFERENCE_CHARS: usize = 128;
/// Digits of the longest length.
const MAX_LENGTH_DIGITS: usize = 3;

/// The description of a floating picture linked to `reference`. `None` for a reference a file can't
/// hold, such as `#REF!`: the picture is then saved as a plain picture.
pub fn linked_descr(reference: &str, alt: &str) -> Option<String> {
    // What XML can't carry never reaches the file, so it isn't counted either. A sheet name loses the
    // same characters, and the reference still names the sheet when it is read.
    let reference: String = reference.chars().filter(|c| xml::is_xml_char(*c)).collect();
    parse_reference(&reference)?;
    let n = reference.chars().count();
    (n <= MAX_REFERENCE_CHARS).then(|| format!("{LINK}{n}:{reference}:{alt}"))
}

/// The reference and alternative text of a linked picture's description. `None` for any other
/// description, and for a marker that is malformed.
pub fn read_link(descr: &str) -> Option<(&str, &str)> {
    let rest = descr.strip_prefix(LINK)?;
    let (digits, rest) = rest.split_once(':')?;
    if digits.is_empty() || digits.len() > MAX_LENGTH_DIGITS || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: usize = digits.parse().ok()?;
    if n > MAX_REFERENCE_CHARS {
        return None;
    }
    let (end, _) = rest.char_indices().nth(n)?;
    let reference = rest.get(..end)?;
    let alt = rest.get(end..)?.strip_prefix(':')?;
    parse_reference(reference)?;
    Some((reference, alt))
}

/// The sheet and cell of a reference such as `'My Sheet'!$G$5`. The last `!` ends the sheet's name,
/// as a cell holds none and a name can.
fn parse_reference(reference: &str) -> Option<(String, CellRef)> {
    let (sheet, cell) = reference.rsplit_once('!')?;
    let sheet = unquote_sheet(sheet);
    if sheet.is_empty() {
        return None;
    }
    Some((sheet, CellRef::parse(&cell.replace('$', ""))?))
}

/// Forgets the link of every picture whose source isn't a Python cell of this workbook, which takes
/// every sheet to be read: opening a file never deletes a picture, and the engine removes a linked
/// picture whose source is gone. Code too long to run counts as no Python cell, as the engine drops
/// it. Returns how many links went.
pub fn settle_links(wb: &mut Workbook) -> usize {
    if !wb.sheets.iter().any(|s| s.images.iter().any(|o| o.python_source.is_some())) {
        return 0;
    }
    let sheets: HashMap<String, usize> = wb.sheets.iter().enumerate().map(|(i, s)| (s.name.to_ascii_lowercase(), i)).collect();
    let names_python_cell = |wb: &Workbook, reference: &str| {
        let Some((sheet, at)) = parse_reference(reference) else { return false };
        let py = sheets.get(&sheet.to_ascii_lowercase()).and_then(|i| wb.sheets.get(*i)).and_then(|s| s.cell(at)).and_then(|c| c.py.as_deref());
        py.is_some_and(|py| py.fits())
    };
    let mut dropped = 0;
    for i in 0..wb.sheets.len() {
        let Some(sheet) = wb.sheets.get(i) else { continue };
        let kept: Vec<bool> = sheet.images.iter().map(|o| o.python_source.as_deref().is_none_or(|r| names_python_cell(wb, r))).collect();
        if kept.iter().all(|k| *k) {
            continue;
        }
        let Some(sheet) = wb.sheet_mut(i) else { continue };
        for (picture, kept) in sheet.images.iter_mut().zip(kept) {
            if !kept {
                picture.python_source = None;
                dropped += 1;
            }
        }
    }
    dropped
}
