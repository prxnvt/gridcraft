//! Design tokens and fonts.
//!
//! Colours are our own, tuned to read like a modern desktop spreadsheet. Fonts come from the
//! operating system at runtime (nothing bundled): the UI uses the platform UI font, cells use the
//! best available match for the workbook font (Calibri/Carlito/Aptos/Arial…) with egui's built-in
//! fonts as the last resort.

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub window: Color32,
    pub ribbon: Color32,
    pub ribbon_border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_disabled: Color32,
    pub accent: Color32,
    pub accent_dark: Color32,
    pub accent_soft: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub separator: Color32,
    pub grid_bg: Color32,
    pub gridline: Color32,
    pub header_bg: Color32,
    pub header_text: Color32,
    pub header_line: Color32,
    pub header_sel_bg: Color32,
    pub header_sel_text: Color32,
    pub header_all_bg: Color32,
    pub sel_fill: Color32,
    pub sel_border: Color32,
    pub input_bg: Color32,
    pub input_border: Color32,
    pub tab_bar: Color32,
    pub tab_active: Color32,
    pub status_bar: Color32,
    pub cell_text: Color32,
    pub menu_bg: Color32,
    pub shadow: Color32,
    pub danger: Color32,
    /// Text and glyphs drawn on an `accent` fill (primary buttons, switched-on toggles).
    pub on_accent: Color32,
    /// The category palette, indexed by [`Cat`]: icon accents and Function Library categories.
    pub cat: [Color32; 8],
    /// The page/sheet colour inside icons (documents, cells, clipboards) and its outline.
    pub paper: Color32,
    pub paper_line: Color32,
    /// Reference colours for formulas edited in the window-coloured formula bar.
    pub ref_colors: [Color32; 8],
    /// Reference colours on the sheet (outlines and the in-cell editor). The sheet stays white.
    pub sheet_ref_colors: [Color32; 8],
    /// Code colours in the window-coloured formula bar and code editors.
    pub syntax: Syntax,
    /// Code colours on the sheet (in-cell editor).
    pub sheet_syntax: Syntax,
    /// The PY badge fill and its text.
    pub badge_py: Color32,
    pub on_badge: Color32,
    /// The macro recorder's dot.
    pub recording: Color32,
    /// A code breakpoint marker.
    pub breakpoint: Color32,
    /// Text of a stale (not yet recalculated) cell, drawn on the sheet.
    pub stale_text: Color32,
    /// The message box warning disc and its glyph.
    pub warning: Color32,
    pub on_warning: Color32,
    /// Scroll thumbs in the window chrome (sheet tab bar).
    pub scrollbar: Color32,
    pub scrollbar_hover: Color32,
    /// Controls drawn on the sheet (filter buttons, form controls, object handles). They follow
    /// the sheet, so they are light in both themes.
    pub sheet_control_bg: Color32,
    pub sheet_control_border: Color32,
    pub sheet_control_text: Color32,
    pub sheet_track: Color32,
    pub sheet_thumb: Color32,
    /// A sheet button under the pointer, and pressed.
    pub sheet_control_hover: Color32,
    pub sheet_control_pressed: Color32,
    /// The filled part of a slider's track (from the minimum to the thumb).
    pub sheet_fill: Color32,
    /// The ring around the form control that takes the arrow keys.
    pub sheet_focus: Color32,
    /// Macro overlays on the sheet (Check and debugging): the inputs, the cells the macro writes,
    /// the cells it would write over, and the current step's target. Sheet colours.
    pub sheet_macro_input: Color32,
    pub sheet_macro_footprint: Color32,
    pub sheet_macro_conflict: Color32,
    pub sheet_macro_target: Color32,
}

/// The eight category colours, in the order of [`Tokens::cat`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cat {
    Blue,
    Green,
    Orange,
    Red,
    Purple,
    Teal,
    Amber,
    Slate,
}

impl Cat {
    pub const ALL: [Cat; 8] = [Cat::Blue, Cat::Green, Cat::Orange, Cat::Red, Cat::Purple, Cat::Teal, Cat::Amber, Cat::Slate];
}

/// Code colours for one background; plain text uses the surface's own text colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Syntax {
    pub keyword: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub builtin: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Colours for text on the white sheet, the same in both themes (the sheet does not follow the theme).
const SHEET_REF_COLORS: [Color32; 8] =
    [rgb(0x2E6FD8), rgb(0xD03C3C), rgb(0x7A4FC9), rgb(0x23803D), rgb(0xB5531A), rgb(0x16798A), rgb(0xB83B8E), rgb(0x77681A)];
const SHEET_MACRO_INPUT: Color32 = rgb(0x2E6FD8);
const SHEET_MACRO_FOOTPRINT: Color32 = rgb(0x23803D);
const SHEET_MACRO_CONFLICT: Color32 = rgb(0xC42B1C);
const SHEET_MACRO_TARGET: Color32 = rgb(0x6B3FC0);
const LIGHT_SYNTAX: Syntax =
    Syntax { keyword: rgb(0x0B57B3), string: rgb(0xA3361B), number: rgb(0x157A5A), comment: rgb(0x6B7280), builtin: rgb(0x7B3FB5) };
const DARK_SYNTAX: Syntax =
    Syntax { keyword: rgb(0x6FAEFF), string: rgb(0xF09A7A), number: rgb(0x7FD1A8), comment: rgb(0x9AA3AE), builtin: rgb(0xC69BF2) };
const SHEET_CONTROL_BG: Color32 = rgb(0xF3F3F3);
const SHEET_CONTROL_BORDER: Color32 = rgb(0x8A8A8A);
const SHEET_CONTROL_TEXT: Color32 = rgb(0x1F1F1F);
const SHEET_TRACK: Color32 = rgb(0xD6D6D6);
const SHEET_THUMB: Color32 = rgb(0x6B6B6B);
const SHEET_CONTROL_HOVER: Color32 = rgb(0xE4E4E4);
const SHEET_CONTROL_PRESSED: Color32 = rgb(0xCFCFCF);
const SHEET_FILL: Color32 = rgb(0x107C41);
const SHEET_FOCUS: Color32 = rgb(0x107C41);
const STALE_TEXT: Color32 = rgb(0x767676);

impl Tokens {
    pub fn light() -> Tokens {
        Tokens {
            dark: false,
            window: Color32::from_rgb(0xF3, 0xF3, 0xF3),
            ribbon: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            ribbon_border: Color32::from_rgb(0xE3, 0xE3, 0xE3),
            text: Color32::from_rgb(0x24, 0x24, 0x24),
            text_dim: Color32::from_rgb(0x61, 0x61, 0x61),
            text_disabled: Color32::from_rgb(0xB0, 0xB0, 0xB0),
            accent: Color32::from_rgb(0x10, 0x7C, 0x41),
            accent_dark: Color32::from_rgb(0x0B, 0x5E, 0x31),
            accent_soft: Color32::from_rgb(0xDF, 0xF1, 0xE6),
            hover: Color32::from_rgb(0xEB, 0xEB, 0xEB),
            pressed: Color32::from_rgb(0xDD, 0xDD, 0xDD),
            separator: Color32::from_rgb(0xE0, 0xE0, 0xE0),
            grid_bg: Color32::WHITE,
            gridline: Color32::from_rgb(0xE1, 0xE1, 0xE1),
            header_bg: Color32::from_rgb(0xF8, 0xF8, 0xF8),
            header_text: Color32::from_rgb(0x42, 0x42, 0x42),
            header_line: Color32::from_rgb(0xD6, 0xD6, 0xD6),
            header_sel_bg: Color32::from_rgb(0xE1, 0xE1, 0xE1),
            header_sel_text: Color32::from_rgb(0x0B, 0x5E, 0x31),
            header_all_bg: Color32::from_rgb(0xC8, 0xE6, 0xD3),
            sel_fill: Color32::from_rgba_unmultiplied(0x10, 0x7C, 0x41, 0x1C),
            sel_border: Color32::from_rgb(0x10, 0x7C, 0x41),
            input_bg: Color32::WHITE,
            input_border: Color32::from_rgb(0xD1, 0xD1, 0xD1),
            tab_bar: Color32::from_rgb(0xF3, 0xF3, 0xF3),
            tab_active: Color32::WHITE,
            status_bar: Color32::from_rgb(0xF3, 0xF3, 0xF3),
            cell_text: Color32::BLACK,
            menu_bg: Color32::WHITE,
            shadow: Color32::from_black_alpha(40),
            danger: Color32::from_rgb(0xC4, 0x2B, 0x1C),
            on_accent: Color32::WHITE,
            cat: [rgb(0x2F6FCF), rgb(0x1E8A4C), rgb(0xC8601A), rgb(0xC8372D), rgb(0x8350C8), rgb(0x13808A), rgb(0x9C6C00), rgb(0x5A6B7D)],
            paper: Color32::WHITE,
            paper_line: rgb(0x5F5F5F),
            ref_colors: SHEET_REF_COLORS,
            sheet_ref_colors: SHEET_REF_COLORS,
            syntax: LIGHT_SYNTAX,
            sheet_syntax: LIGHT_SYNTAX,
            badge_py: rgb(0x1E7A45),
            on_badge: Color32::WHITE,
            recording: rgb(0xC42B1C),
            breakpoint: rgb(0xD13438),
            stale_text: STALE_TEXT,
            warning: rgb(0xF2C32E),
            on_warning: rgb(0x1F1F1F),
            scrollbar: Color32::from_gray(190),
            scrollbar_hover: Color32::from_gray(150),
            sheet_control_bg: SHEET_CONTROL_BG,
            sheet_control_border: SHEET_CONTROL_BORDER,
            sheet_control_text: SHEET_CONTROL_TEXT,
            sheet_track: SHEET_TRACK,
            sheet_thumb: SHEET_THUMB,
            sheet_control_hover: SHEET_CONTROL_HOVER,
            sheet_control_pressed: SHEET_CONTROL_PRESSED,
            sheet_fill: SHEET_FILL,
            sheet_focus: SHEET_FOCUS,
            sheet_macro_input: SHEET_MACRO_INPUT,
            sheet_macro_footprint: SHEET_MACRO_FOOTPRINT,
            sheet_macro_conflict: SHEET_MACRO_CONFLICT,
            sheet_macro_target: SHEET_MACRO_TARGET,
        }
    }

    pub fn dark() -> Tokens {
        Tokens {
            dark: true,
            window: Color32::from_rgb(0x29, 0x29, 0x29),
            ribbon: Color32::from_rgb(0x33, 0x33, 0x33),
            ribbon_border: Color32::from_rgb(0x40, 0x40, 0x40),
            text: Color32::from_rgb(0xEE, 0xEE, 0xEE),
            text_dim: Color32::from_rgb(0xB0, 0xB0, 0xB0),
            text_disabled: Color32::from_rgb(0x70, 0x70, 0x70),
            accent: Color32::from_rgb(0x5C, 0xC5, 0x87),
            accent_dark: Color32::from_rgb(0x3F, 0xA8, 0x6C),
            accent_soft: Color32::from_rgb(0x23, 0x45, 0x31),
            hover: Color32::from_rgb(0x42, 0x42, 0x42),
            pressed: Color32::from_rgb(0x4D, 0x4D, 0x4D),
            separator: Color32::from_rgb(0x48, 0x48, 0x48),
            grid_bg: Color32::WHITE,
            gridline: Color32::from_rgb(0xE1, 0xE1, 0xE1),
            header_bg: Color32::from_rgb(0x30, 0x30, 0x30),
            header_text: Color32::from_rgb(0xCC, 0xCC, 0xCC),
            header_line: Color32::from_rgb(0x45, 0x45, 0x45),
            header_sel_bg: Color32::from_rgb(0x45, 0x45, 0x45),
            header_sel_text: Color32::from_rgb(0x6F, 0xD3, 0x9A),
            header_all_bg: Color32::from_rgb(0x2E, 0x55, 0x3E),
            sel_fill: Color32::from_rgba_unmultiplied(0x10, 0x7C, 0x41, 0x24),
            sel_border: Color32::from_rgb(0x10, 0x7C, 0x41),
            input_bg: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            input_border: Color32::from_rgb(0x50, 0x50, 0x50),
            tab_bar: Color32::from_rgb(0x29, 0x29, 0x29),
            tab_active: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            status_bar: Color32::from_rgb(0x29, 0x29, 0x29),
            cell_text: Color32::BLACK,
            menu_bg: Color32::from_rgb(0x30, 0x30, 0x30),
            shadow: Color32::from_black_alpha(90),
            danger: Color32::from_rgb(0xFF, 0x95, 0x85),
            on_accent: rgb(0x0F2A1A),
            cat: [rgb(0x6EA8F7), rgb(0x5CC587), rgb(0xF0965A), rgb(0xF07B6E), rgb(0xB394EE), rgb(0x4CC7CF), rgb(0xF2C14E), rgb(0xA9B6C4)],
            paper: rgb(0x454545),
            paper_line: rgb(0xC8C8C8),
            ref_colors: [rgb(0x6FA3F5), rgb(0xF27C7C), rgb(0xB394EE), rgb(0x6CCB85), rgb(0xF0A061), rgb(0x5CC9DA), rgb(0xE77FC4), rgb(0xD4C25A)],
            sheet_ref_colors: SHEET_REF_COLORS,
            syntax: DARK_SYNTAX,
            sheet_syntax: LIGHT_SYNTAX,
            badge_py: rgb(0x1F6B40),
            on_badge: Color32::WHITE,
            recording: rgb(0xF1705F),
            breakpoint: rgb(0xF1707A),
            stale_text: STALE_TEXT,
            warning: rgb(0xF2C32E),
            on_warning: rgb(0x1F1F1F),
            scrollbar: Color32::from_gray(0x5A),
            scrollbar_hover: Color32::from_gray(0x7A),
            sheet_control_bg: SHEET_CONTROL_BG,
            sheet_control_border: SHEET_CONTROL_BORDER,
            sheet_control_text: SHEET_CONTROL_TEXT,
            sheet_track: SHEET_TRACK,
            sheet_thumb: SHEET_THUMB,
            sheet_control_hover: SHEET_CONTROL_HOVER,
            sheet_control_pressed: SHEET_CONTROL_PRESSED,
            sheet_fill: SHEET_FILL,
            sheet_focus: SHEET_FOCUS,
            sheet_macro_input: SHEET_MACRO_INPUT,
            sheet_macro_footprint: SHEET_MACRO_FOOTPRINT,
            sheet_macro_conflict: SHEET_MACRO_CONFLICT,
            sheet_macro_target: SHEET_MACRO_TARGET,
        }
    }

    pub fn get(ctx: &egui::Context) -> Tokens {
        if ctx.global_style().visuals.dark_mode { Tokens::dark() } else { Tokens::light() }
    }

    /// One colour of the category palette.
    pub fn cat(&self, c: Cat) -> Color32 {
        self.cat.get(c as usize).copied().unwrap_or(self.text)
    }

    /// The reference colour for a formula's `index`-th reference, on the bar or on the sheet.
    pub fn ref_color(&self, index: usize, on_sheet: bool) -> Color32 {
        let set = if on_sheet { &self.sheet_ref_colors } else { &self.ref_colors };
        set.get(index % set.len()).copied().unwrap_or(self.text)
    }
}

/// WCAG 2 relative luminance of an opaque colour.
pub fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG 2 contrast ratio between two opaque colours (1 to 21).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

pub const UI: &str = "ui";
pub const UI_BOLD: &str = "ui-bold";
pub const CELL: &str = "cell";
pub const CELL_BOLD: &str = "cell-bold";
pub const CELL_ITALIC: &str = "cell-italic";
pub const CELL_BOLD_ITALIC: &str = "cell-bold-italic";
pub const SERIF: &str = "serif";
pub const MONO: &str = "mono";
/// Equation faces: a maths font (upright letters, operators, Greek, double-struck and script
/// letters), its italic for variables, and a bold. System fonts only; nothing is bundled.
pub const MATH: &str = "math";
pub const MATH_ITALIC: &str = "math-italic";
pub const MATH_BOLD: &str = "math-bold";

pub fn ui_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(UI.into()))
}
pub fn ui_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(UI_BOLD.into()))
}

/// The font family to use for a cell font name and weight.
pub fn cell_family(name: &str, bold: bool, italic: bool) -> FontFamily {
    let lower = name.to_ascii_lowercase();
    if lower.contains("courier") || lower.contains("mono") || lower.contains("consolas") {
        return FontFamily::Name(MONO.into());
    }
    if lower.contains("times")
        || lower.contains("georgia")
        || lower.contains("cambria")
        || lower.contains("serif") && !lower.contains("sans")
        || lower.contains("garamond")
    {
        return FontFamily::Name(SERIF.into());
    }
    FontFamily::Name(
        match (bold, italic) {
            (true, true) => CELL_BOLD_ITALIC,
            (true, false) => CELL_BOLD,
            (false, true) => CELL_ITALIC,
            (false, false) => CELL,
        }
        .into(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn try_load(paths: &[(&str, u32)]) -> Option<FontData> {
    for (p, index) in paths {
        if let Ok(bytes) = std::fs::read(p) {
            let mut fd = FontData::from_owned(bytes);
            fd.index = *index;
            return Some(fd);
        }
    }
    None
}

#[cfg(target_arch = "wasm32")]
fn try_load(_paths: &[(&str, u32)]) -> Option<FontData> {
    None
}

fn home_fonts(file: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Library/Fonts/{file}")
}

/// Candidate font files per role, in preference order.
fn candidates(role: &str) -> Vec<(String, u32)> {
    let mac_sys = "/System/Library/Fonts";
    let mac_sup = "/System/Library/Fonts/Supplemental";
    let win = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let lin = ["/usr/share/fonts/truetype", "/usr/share/fonts", "/usr/local/share/fonts", "/usr/share/fonts/TTF"];
    let mut v: Vec<(String, u32)> = Vec::new();
    let mut add = |s: String, i: u32| v.push((s, i));
    match role {
        "ui" => {
            add(format!("{mac_sys}/SFNS.ttf"), 0);
            add(format!("{mac_sys}/HelveticaNeue.ttc"), 0);
            add(format!("{win}\\segoeui.ttf"), 0);
            for d in lin {
                add(format!("{d}/cantarell/Cantarell-Regular.otf"), 0);
                add(format!("{d}/noto/NotoSans-Regular.ttf"), 0);
                add(format!("{d}/dejavu/DejaVuSans.ttf"), 0);
                add(format!("{d}/liberation/LiberationSans-Regular.ttf"), 0);
            }
        }
        "ui-bold" => {
            add(format!("{mac_sys}/HelveticaNeue.ttc"), 1);
            add(format!("{win}\\segoeuib.ttf"), 0);
            for d in lin {
                add(format!("{d}/noto/NotoSans-Bold.ttf"), 0);
                add(format!("{d}/dejavu/DejaVuSans-Bold.ttf"), 0);
                add(format!("{d}/liberation/LiberationSans-Bold.ttf"), 0);
            }
        }
        "cell" | "cell-bold" | "cell-italic" | "cell-bold-italic" => {
            let (carlito, calibri_win, calibri_mac, arial_mac, arial_win, lib, dejavu) = match role {
                "cell" => {
                    ("Carlito-Regular.ttf", "calibri.ttf", "Calibri.ttf", "Arial.ttf", "arial.ttf", "LiberationSans-Regular.ttf", "DejaVuSans.ttf")
                }
                "cell-bold" => (
                    "Carlito-Bold.ttf",
                    "calibrib.ttf",
                    "Calibri Bold.ttf",
                    "Arial Bold.ttf",
                    "arialbd.ttf",
                    "LiberationSans-Bold.ttf",
                    "DejaVuSans-Bold.ttf",
                ),
                "cell-italic" => (
                    "Carlito-Italic.ttf",
                    "calibrii.ttf",
                    "Calibri Italic.ttf",
                    "Arial Italic.ttf",
                    "ariali.ttf",
                    "LiberationSans-Italic.ttf",
                    "DejaVuSans-Oblique.ttf",
                ),
                _ => (
                    "Carlito-BoldItalic.ttf",
                    "calibriz.ttf",
                    "Calibri Bold Italic.ttf",
                    "Arial Bold Italic.ttf",
                    "arialbi.ttf",
                    "LiberationSans-BoldItalic.ttf",
                    "DejaVuSans-BoldOblique.ttf",
                ),
            };
            // Calibri if the user has it installed system-wide (never from an Office bundle).
            add(format!("/Library/Fonts/{calibri_mac}"), 0);
            add(home_fonts(calibri_mac), 0);
            add(format!("{win}\\{calibri_win}"), 0);
            for d in lin {
                add(format!("{d}/crosextra/{carlito}"), 0);
                add(format!("{d}/carlito/{carlito}"), 0);
            }
            add(home_fonts(carlito), 0);
            add(format!("{mac_sup}/{arial_mac}"), 0);
            add(format!("{win}\\{arial_win}"), 0);
            for d in lin {
                add(format!("{d}/liberation/{lib}"), 0);
                add(format!("{d}/dejavu/{dejavu}"), 0);
            }
        }
        "serif" => {
            add(format!("{mac_sup}/Times New Roman.ttf"), 0);
            add(format!("{win}\\times.ttf"), 0);
            for d in lin {
                add(format!("{d}/liberation/LiberationSerif-Regular.ttf"), 0);
                add(format!("{d}/dejavu/DejaVuSerif.ttf"), 0);
            }
        }
        "math" => {
            add(format!("{mac_sup}/STIXTwoMath.otf"), 0);
            add(format!("{win}\\cambria.ttc"), 1);
            for d in lin {
                add(format!("{d}/opentype/stix/STIXTwoMath-Regular.otf"), 0);
                add(format!("{d}/stix/STIXTwoMath-Regular.otf"), 0);
                add(format!("{d}/opentype/lmodern/latinmodern-math.otf"), 0);
                add(format!("{d}/dejavu/DejaVuSerif.ttf"), 0);
                add(format!("{d}/liberation/LiberationSerif-Regular.ttf"), 0);
            }
            add(format!("{mac_sup}/Times New Roman.ttf"), 0);
            add(format!("{win}\\times.ttf"), 0);
        }
        "math-italic" => {
            add(format!("{mac_sup}/STIXTwoText-Italic.ttf"), 0);
            add(format!("{mac_sup}/Times New Roman Italic.ttf"), 0);
            add(format!("{win}\\cambriai.ttf"), 0);
            add(format!("{win}\\timesi.ttf"), 0);
            for d in lin {
                add(format!("{d}/opentype/stix/STIXTwoText-Italic.otf"), 0);
                add(format!("{d}/liberation/LiberationSerif-Italic.ttf"), 0);
                add(format!("{d}/dejavu/DejaVuSerif-Italic.ttf"), 0);
            }
        }
        "math-bold" => {
            add(format!("{mac_sup}/Times New Roman Bold.ttf"), 0);
            add(format!("{win}\\cambriab.ttf"), 0);
            add(format!("{win}\\timesbd.ttf"), 0);
            for d in lin {
                add(format!("{d}/opentype/stix/STIXTwoText-Bold.otf"), 0);
                add(format!("{d}/liberation/LiberationSerif-Bold.ttf"), 0);
                add(format!("{d}/dejavu/DejaVuSerif-Bold.ttf"), 0);
            }
        }
        "mono" => {
            add(format!("{mac_sys}/SFNSMono.ttf"), 0);
            add(format!("{mac_sys}/Menlo.ttc"), 0);
            add(format!("{win}\\consola.ttf"), 0);
            for d in lin {
                add(format!("{d}/dejavu/DejaVuSansMono.ttf"), 0);
            }
        }
        _ => {}
    }
    v
}

/// Builds the font set from system fonts (with egui's defaults as fallback), with the default
/// (non-Japanese) Han order. Never reads the host locale, so tests and snapshots are stable.
pub fn font_definitions() -> FontDefinitions {
    font_definitions_for_language(crate::i18n::Language::En)
}

/// Which Han face leads the fallback chain. Both Han faces cover kana and most kanji, so this only
/// decides glyph shapes (and which face catches a glyph the other lacks); switching between two
/// languages with the same order needs no font rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HanOrder {
    /// Japanese glyph shapes first (the interface is Japanese).
    Japanese,
    /// Simplified Chinese first: the widest Han coverage, for every other interface language.
    Chinese,
}

impl HanOrder {
    pub fn of(language: crate::i18n::Language) -> Self {
        if language == crate::i18n::Language::Ja { Self::Japanese } else { Self::Chinese }
    }
}

/// Builds the font set with the Han fallback faces ordered for the interface language.
pub fn font_definitions_for_language(language: crate::i18n::Language) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let base_prop: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let base_mono: Vec<String> = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    // The non-Latin fallback faces have to sit at the end of each chain: a Latin font owns Latin,
    // and epaint picks the first face that has the glyph, so they only catch their own scripts.
    let fallback = load_fallback(&mut fonts, HanOrder::of(language));
    for role in [UI, UI_BOLD, CELL, CELL_BOLD, CELL_ITALIC, CELL_BOLD_ITALIC, SERIF, MONO, MATH, MATH_ITALIC, MATH_BOLD] {
        let paths = candidates(role);
        let refs: Vec<(&str, u32)> = paths.iter().map(|(p, i)| (p.as_str(), *i)).collect();
        let mut chain: Vec<String> = Vec::new();
        if let Some(fd) = try_load(&refs) {
            let key = format!("sys-{role}");
            fonts.font_data.insert(key.clone(), Arc::new(fd));
            chain.push(key);
        } else if role == UI_BOLD {
            chain.push(format!("sys-{UI}"));
        } else if role.starts_with("cell-") {
            chain.push(format!("sys-{CELL}"));
        }
        // Equation faces fall back to the maths font (for the symbols a text face lacks), then
        // to the serif face.
        if role == MATH_ITALIC || role == MATH_BOLD {
            chain.push(format!("sys-{MATH}"));
        }
        if role.starts_with("math") {
            chain.push(format!("sys-{SERIF}"));
        }
        chain.extend(if role == MONO { base_mono.clone() } else { base_prop.clone() });
        chain.extend(fallback.iter().cloned());
        chain.retain(|k| fonts.font_data.contains_key(k));
        fonts.families.insert(FontFamily::Name(role.into()), chain);
    }
    // Default proportional text in widgets uses the UI font.
    if let Some(ui) = fonts.families.get(&FontFamily::Name(UI.into())).cloned() {
        fonts.families.insert(FontFamily::Proportional, ui);
    }
    fonts
}

/// Loads the system non-Latin fallback faces (one file per script family) and returns their keys so
/// every family can carry them as fallback. egui's bundled fonts have no coverage beyond Latin, so
/// without this a Thai, Hebrew, Japanese or Chinese workbook shows tofu boxes even though the text
/// was read correctly. With a face installed those scripts get real glyphs instead of tofu; nothing
/// is bundled, and a machine without the face keeps the old behaviour.
#[cfg(not(target_arch = "wasm32"))]
fn load_fallback(fonts: &mut FontDefinitions, han: HanOrder) -> Vec<String> {
    let mut keys = Vec::new();
    // Files already loaded as a fallback face. A file serves at most one key: the region-merged
    // Noto Sans CJK .ttc found for Han also carries Hangul (and Tahoma carries both Thai and
    // Hebrew), so loading it again for a later script would only duplicate tens of MB in memory.
    let mut loaded: Vec<String> = Vec::new();
    for (key, cands) in fallback_faces(han) {
        for (path, index) in &cands {
            if loaded.contains(path) {
                break; // a face loaded for an earlier script already covers this one
            }
            if let Some(fd) = try_load(&[(path.as_str(), *index)]) {
                fonts.font_data.insert(key.to_string(), Arc::new(fd));
                keys.push(key.to_string());
                loaded.push(path.clone());
                break;
            }
        }
    }
    keys
}

#[cfg(target_arch = "wasm32")]
fn load_fallback(_fonts: &mut FontDefinitions, _han: HanOrder) -> Vec<String> {
    Vec::new()
}

/// Every fallback face we look for, in chain order: `(key, candidate files)`. One file per script
/// family is enough — epaint picks the first face with the glyph, and a script's glyphs live in one
/// file. Order only matters between faces that both cover a glyph (Japanese vs Chinese kanji), so
/// the interface language reorders the two Han entries and nothing else.
#[cfg(not(target_arch = "wasm32"))]
fn fallback_faces(han: HanOrder) -> Vec<(&'static str, Vec<(String, u32)>)> {
    let ja = ("sys-cjk-han-ja", cjk_han_japanese());
    let zh = ("sys-cjk-han-zh", cjk_han_simplified());
    let (first, second) = match han {
        HanOrder::Japanese => (ja, zh),
        HanOrder::Chinese => (zh, ja),
    };
    vec![first, second, ("sys-cjk-hangul", cjk_hangul()), ("sys-thai", thai()), ("sys-hebrew", hebrew())]
}

/// Linux directories that hold CJK fonts across distributions: Debian/Ubuntu (`opentype/noto`,
/// `truetype/wqy`), Fedora (`google-noto-cjk`), Arch (`noto-cjk`, `wenquanyi`), plus local and
/// flat layouts.
#[cfg(not(target_arch = "wasm32"))]
const LINUX_CJK_DIRS: [&str; 10] = [
    "/usr/share/fonts/opentype/noto",
    "/usr/share/fonts/google-noto-cjk",
    "/usr/share/fonts/noto-cjk",
    "/usr/share/fonts/truetype/noto",
    "/usr/share/fonts/noto",
    "/usr/share/fonts/truetype",
    "/usr/share/fonts/TTF",
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    "/usr/local/share/fonts/noto-cjk",
];

/// Japanese Han + Kana faces (Japanese kanji glyph shapes).
///
/// The region-merged `NotoSansCJK-Regular.ttc` holds one face per locale (JP 0, KR 1, SC 2, TC 3,
/// HK 4) with the same glyph coverage, so the index only picks glyph shapes. Whichever Han list
/// comes first loads it; [`load_fallback`] does not load the same file again for the other.
#[cfg(not(target_arch = "wasm32"))]
fn cjk_han_japanese() -> Vec<(String, u32)> {
    let mac = "/System/Library/Fonts";
    let win = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let mut v: Vec<(String, u32)> = Vec::new();
    v.push((format!("{win}\\YuGothR.ttc"), 0)); // Yu Gothic
    v.push((format!("{win}\\msgothic.ttc"), 0)); // MS Gothic
    v.push((format!("{mac}/ヒラギノ角ゴシック W3.ttc"), 0)); // Hiragino Sans
    for d in LINUX_CJK_DIRS {
        v.push((format!("{d}/NotoSansCJKjp-Regular.otf"), 0));
        v.push((format!("{d}/NotoSansCJK-Regular.ttc"), 0)); // JP face
    }
    v
}

/// Simplified Chinese Han faces. Loaded apart from the Japanese ones so a glyph one family lacks
/// (simplified-only hanzi in a Japanese font) still falls through to a real glyph.
#[cfg(not(target_arch = "wasm32"))]
fn cjk_han_simplified() -> Vec<(String, u32)> {
    let mac = "/System/Library/Fonts";
    let win = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let mut v: Vec<(String, u32)> = Vec::new();
    v.push((format!("{win}\\msyh.ttc"), 0)); // Microsoft YaHei
    v.push((format!("{win}\\simsun.ttc"), 0)); // SimSun
    v.push((format!("{win}\\simhei.ttf"), 0)); // SimHei
    v.push((format!("{mac}/Hiragino Sans GB.ttc"), 0)); // ships on every recent macOS
    v.push((format!("{mac}/PingFang.ttc"), 0)); // older macOS only; now a downloadable asset
    for d in LINUX_CJK_DIRS {
        v.push((format!("{d}/NotoSansCJKsc-Regular.otf"), 0));
        v.push((format!("{d}/NotoSansCJK-Regular.ttc"), 2)); // SC face
    }
    for d in LINUX_CJK_DIRS {
        v.push((format!("{d}/wqy/wqy-microhei.ttc"), 0));
        v.push((format!("{d}/wenquanyi/wqy-microhei.ttc"), 0));
        v.push((format!("{d}/wqy-microhei.ttc"), 0));
    }
    v
}

/// Hangul faces (Korean). The Han faces above that come from the region-merged Noto Sans CJK file
/// already carry Hangul; [`load_fallback`] then skips this list rather than load the file twice.
#[cfg(not(target_arch = "wasm32"))]
fn cjk_hangul() -> Vec<(String, u32)> {
    let mac = "/System/Library/Fonts";
    let win = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let mut v: Vec<(String, u32)> = Vec::new();
    v.push((format!("{win}\\malgun.ttf"), 0)); // Malgun Gothic
    v.push((format!("{mac}/AppleSDGothicNeo.ttc"), 0));
    for d in LINUX_CJK_DIRS {
        v.push((format!("{d}/NotoSansCJKkr-Regular.otf"), 0));
        v.push((format!("{d}/NotoSansCJK-Regular.ttc"), 1)); // KR face
    }
    v
}

/// Thai faces. Windows ships Leelawadee UI and Tahoma/Microsoft Sans Serif (both carry Thai);
/// macOS has Thonburi; Linux usually has Noto Sans Thai or Garuda.
#[cfg(not(target_arch = "wasm32"))]
fn thai() -> Vec<(String, u32)> {
    let mac_sys = "/System/Library/Fonts";
    let mac_sup = "/System/Library/Fonts/Supplemental";
    let win = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let lin = ["/usr/share/fonts/truetype/noto", "/usr/share/fonts/opentype/noto", "/usr/share/fonts/truetype", "/usr/share/fonts"];
    let mut v: Vec<(String, u32)> = Vec::new();
    v.push((format!("{win}\\LeelawUI.ttf"), 0)); // Leelawadee UI
    v.push((format!("{win}\\tahoma.ttf"), 0)); // Tahoma
    v.push((format!("{win}\\micross.ttf"), 0)); // Microsoft Sans Serif
    v.push((format!("{mac_sup}/Thonburi.ttc"), 0));
    v.push((format!("{mac_sys}/SukhumvitSet.ttc"), 0));
    for d in lin {
        v.push((format!("{d}/NotoSansThai-Regular.ttf"), 0));
        v.push((format!("{d}/tlwg/Garuda.ttf"), 0));
        v.push((format!("{d}/garuda/Garuda.ttf"), 0));
    }
    v
}

/// Hebrew faces. Windows has David and FrankRuehl (and Tahoma/Courier New carry Hebrew); macOS has
/// Arial Hebrew and David; Linux usually has Noto Sans Hebrew or DejaVu Sans.
#[cfg(not(target_arch = "wasm32"))]
fn hebrew() -> Vec<(String, u32)> {
    let mac_sys = "/System/Library/Fonts";
    let mac_sup = "/System/Library/Fonts/Supplemental";
    let win = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let lin = ["/usr/share/fonts/truetype/noto", "/usr/share/fonts/opentype/noto", "/usr/share/fonts/truetype", "/usr/share/fonts"];
    let mut v: Vec<(String, u32)> = Vec::new();
    v.push((format!("{win}\\david.ttf"), 0)); // David
    v.push((format!("{win}\\frank.ttf"), 0)); // FrankRuehl
    v.push((format!("{win}\\tahoma.ttf"), 0)); // Tahoma
    v.push((format!("{mac_sup}/Arial Hebrew.ttf"), 0));
    v.push((format!("{mac_sup}/David.ttf"), 0));
    v.push((format!("{mac_sys}/Lucida Grande.ttc"), 0));
    for d in lin {
        v.push((format!("{d}/NotoSansHebrew-Regular.ttf"), 0));
        v.push((format!("{d}/dejavu/DejaVuSans.ttf"), 0));
        v.push((format!("{d}/freefont/FreeSans.ttf"), 0));
    }
    v
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    // Install both palettes before System can switch between the style slots.
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let dark = theme == egui::Theme::Dark;
        let t = if dark { Tokens::dark() } else { Tokens::light() };
        let mut v = if dark { Visuals::dark() } else { Visuals::light() };
        v.panel_fill = t.window;
        v.window_fill = t.menu_bg;
        v.extreme_bg_color = t.input_bg;
        v.selection.bg_fill = t.accent_soft;
        v.selection.stroke = Stroke::new(1.0, t.accent);
        v.hyperlink_color = t.accent;
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
        v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        v.widgets.hovered.weak_bg_fill = t.hover;
        v.widgets.active.weak_bg_fill = t.pressed;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.input_border);
        v.window_corner_radius = egui::CornerRadius::same(10);
        v.menu_corner_radius = egui::CornerRadius::same(8);
        v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: t.shadow };
        ctx.set_visuals_of(theme, v);
    }
    ctx.options_mut(|o| o.fallback_theme = egui::Theme::Light);
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 3.0);
        s.spacing.interact_size.y = 22.0;
        s.text_styles.insert(egui::TextStyle::Body, ui_font(13.0));
        s.text_styles.insert(egui::TextStyle::Button, ui_font(13.0));
        s.text_styles.insert(egui::TextStyle::Small, ui_font(11.0));
        s.text_styles.insert(egui::TextStyle::Heading, ui_bold(18.0));
        s.text_styles.insert(egui::TextStyle::Monospace, FontId::new(12.0, FontFamily::Name(MONO.into())));
    });
}

/// Model colour → egui colour.
pub fn color32(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod contrast_tests {
    use super::*;

    /// Body text needs 4.5:1 (WCAG AA); glyphs, icons and other graphics need 3:1.
    const TEXT: f32 = 4.5;
    const GLYPH: f32 = 3.0;

    /// Every foreground/background pair the chrome draws: ribbon, popups, dialogs, panes, formula
    /// bar, status bar, plus the colours that sit on the white sheet.
    fn pairs(t: &Tokens) -> Vec<(String, Color32, Color32, f32)> {
        let mut v: Vec<(String, Color32, Color32, f32)> = Vec::new();
        let mut add = |name: &str, fg: Color32, bg: Color32, min: f32| v.push((name.to_string(), fg, bg, min));
        let surfaces = [
            ("window", t.window),
            ("ribbon", t.ribbon),
            ("menu", t.menu_bg),
            ("input", t.input_bg),
            ("status bar", t.status_bar),
            ("tab bar", t.tab_bar),
            ("active tab", t.tab_active),
            ("hover", t.hover),
        ];
        for (name, bg) in surfaces {
            add(&format!("text on {name}"), t.text, bg, TEXT);
            add(&format!("dim text on {name}"), t.text_dim, bg, TEXT);
            add(&format!("accent glyph on {name}"), t.accent, bg, GLYPH);
            add(&format!("danger on {name}"), t.danger, bg, TEXT);
        }
        add("text on pressed", t.text, t.pressed, TEXT);
        add("accent text (File, contextual tabs) on window", t.accent, t.window, TEXT);
        for (name, bg) in [("ribbon", t.ribbon), ("menu", t.menu_bg), ("hover", t.hover), ("pressed", t.pressed)] {
            for c in Cat::ALL {
                add(&format!("{c:?} icon accent on {name}"), t.cat(c), bg, GLYPH);
            }
        }
        add("icon outline on paper", t.paper_line, t.paper, GLYPH);
        add("icon stroke on paper", t.text, t.paper, GLYPH);
        add("on_accent on accent", t.on_accent, t.accent, TEXT);
        add("PY badge text", t.on_badge, t.badge_py, TEXT);
        add("warning glyph", t.on_warning, t.warning, TEXT);
        add("recording dot on ribbon", t.recording, t.ribbon, GLYPH);
        add("recording dot on status bar", t.recording, t.status_bar, GLYPH);
        add("breakpoint on code editor", t.breakpoint, t.input_bg, GLYPH);
        add("header text", t.header_text, t.header_bg, TEXT);
        add("selected header text", t.header_sel_text, t.header_sel_bg, TEXT);
        for (i, c) in t.ref_colors.iter().enumerate() {
            add(&format!("formula bar reference {i}"), *c, t.input_bg, TEXT);
        }
        for (name, s, bg) in [("formula bar", t.syntax, t.input_bg), ("sheet", t.sheet_syntax, t.grid_bg)] {
            add(&format!("{name} keyword"), s.keyword, bg, TEXT);
            add(&format!("{name} string"), s.string, bg, TEXT);
            add(&format!("{name} number"), s.number, bg, TEXT);
            add(&format!("{name} comment"), s.comment, bg, TEXT);
            add(&format!("{name} builtin"), s.builtin, bg, TEXT);
        }
        // On the sheet.
        add("cell text", t.cell_text, t.grid_bg, TEXT);
        add("stale cell text", t.stale_text, t.grid_bg, TEXT);
        for (i, c) in t.sheet_ref_colors.iter().enumerate() {
            add(&format!("sheet reference {i}"), *c, t.grid_bg, TEXT);
        }
        add("selection border on sheet", t.sel_border, t.grid_bg, GLYPH);
        add("sheet control text", t.sheet_control_text, t.sheet_control_bg, TEXT);
        add("sheet control border on sheet", t.sheet_control_border, t.grid_bg, GLYPH);
        add("sheet thumb on track", t.sheet_thumb, t.sheet_track, GLYPH);
        add("sheet control text on hover", t.sheet_control_text, t.sheet_control_hover, TEXT);
        add("sheet control text when pressed", t.sheet_control_text, t.sheet_control_pressed, TEXT);
        add("slider fill on sheet", t.sheet_fill, t.grid_bg, GLYPH);
        add("slider fill on track", t.sheet_fill, t.sheet_track, GLYPH);
        add("sheet control arrows when pressed", t.sheet_control_text, t.sheet_control_pressed, GLYPH);
        add("control focus ring on sheet", t.sheet_focus, t.grid_bg, GLYPH);
        for (name, c) in [
            ("macro input", t.sheet_macro_input),
            ("macro footprint", t.sheet_macro_footprint),
            ("macro conflict", t.sheet_macro_conflict),
            ("macro target", t.sheet_macro_target),
        ] {
            add(&format!("{name} outline on sheet"), c, t.grid_bg, GLYPH);
        }
        // The macro step editor: breakpoint dots in the gutter, error markers, the current step.
        add("breakpoint on window", t.breakpoint, t.window, GLYPH);
        add("breakpoint on current step", t.breakpoint, t.accent_soft, GLYPH);
        add("text on current step", t.text, t.accent_soft, TEXT);
        add("current step arrow", t.accent, t.accent_soft, GLYPH);
        add("error marker on window", t.danger, t.window, GLYPH);
        v
    }

    #[test]
    fn token_pairs_meet_wcag_contrast_in_both_themes() {
        let mut failures = Vec::new();
        for t in [Tokens::light(), Tokens::dark()] {
            for (name, fg, bg, min) in pairs(&t) {
                let ratio = contrast(fg, bg);
                if ratio < min {
                    failures.push(format!("{} theme: {name} is {ratio:.2}:1, needs {min}:1", if t.dark { "dark" } else { "light" }));
                }
            }
        }
        assert!(failures.is_empty(), "contrast failures:\n{}", failures.join("\n"));
    }

    #[test]
    fn contrast_matches_wcag_reference_values() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.001);
        // #767676 on white is the classic 4.54:1 grey.
        assert!((contrast(rgb(0x767676), Color32::WHITE) - 4.54).abs() < 0.01);
    }

    #[test]
    fn sheet_tokens_follow_the_sheet_not_the_theme() {
        let (l, d) = (Tokens::light(), Tokens::dark());
        assert_eq!(l.grid_bg, d.grid_bg);
        assert_eq!(l.sheet_ref_colors, d.sheet_ref_colors);
        assert_eq!(l.sheet_syntax, d.sheet_syntax);
        assert_eq!(
            (l.sheet_control_bg, l.sheet_control_border, l.sheet_control_text, l.sheet_track, l.sheet_thumb),
            (d.sheet_control_bg, d.sheet_control_border, d.sheet_control_text, d.sheet_track, d.sheet_thumb)
        );
        assert_eq!(
            (l.sheet_control_hover, l.sheet_control_pressed, l.sheet_fill, l.sheet_focus),
            (d.sheet_control_hover, d.sheet_control_pressed, d.sheet_fill, d.sheet_focus)
        );
        assert_eq!(
            (l.sheet_macro_input, l.sheet_macro_footprint, l.sheet_macro_conflict, l.sheet_macro_target),
            (d.sheet_macro_input, d.sheet_macro_footprint, d.sheet_macro_conflict, d.sheet_macro_target)
        );
        for c in Cat::ALL {
            assert_ne!(l.cat(c), d.cat(c), "{c:?} has a dark-mode value of its own");
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn fallback_faces_are_appended_to_every_family() {
        // Structural: whatever loaded, the non-Latin fallback keys must sit at the end of each chain
        // (after the Latin/default fonts) so Latin still wins for Latin text, in the language's order.
        for han in [HanOrder::Chinese, HanOrder::Japanese] {
            let language = if han == HanOrder::Japanese { Language::Ja } else { Language::Zh };
            let fonts = font_definitions_for_language(language);
            let fb_keys: Vec<String> =
                fallback_faces(han).into_iter().map(|(k, _)| k.to_string()).filter(|k| fonts.font_data.contains_key(k)).collect();
            if fb_keys.is_empty() {
                return; // no fallback face on this machine (minimal Linux)
            }
            for role in [UI, UI_BOLD, CELL, CELL_BOLD, CELL_ITALIC, CELL_BOLD_ITALIC, SERIF, MONO, MATH, MATH_ITALIC, MATH_BOLD] {
                let chain = fonts.families.get(&FontFamily::Name(role.into())).map(Vec::as_slice).unwrap_or_default();
                assert!(chain.len() >= fb_keys.len(), "{role} chain too short for its fallback");
                let tail = &chain[chain.len() - fb_keys.len()..];
                assert_eq!(tail, fb_keys.as_slice(), "{role} must end its chain with the fallback faces");
            }
        }
    }

    #[test]
    fn language_reorders_only_the_han_faces() {
        let keys = |han| fallback_faces(han).into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(keys(HanOrder::Japanese), ["sys-cjk-han-ja", "sys-cjk-han-zh", "sys-cjk-hangul", "sys-thai", "sys-hebrew"]);
        assert_eq!(keys(HanOrder::Chinese), ["sys-cjk-han-zh", "sys-cjk-han-ja", "sys-cjk-hangul", "sys-thai", "sys-hebrew"]);
        assert_eq!(HanOrder::of(Language::Ja), HanOrder::Japanese);
        for l in [Language::En, Language::Zh, Language::Ko, Language::Ru, Language::PtBr] {
            assert_eq!(HanOrder::of(l), HanOrder::Chinese, "{l:?} shares the default order (no font rebuild)");
        }
    }

    #[test]
    fn fallback_faces_load_each_file_once() {
        // The region-merged Noto Sans CJK .ttc serves Japanese, Chinese and Hangul; it must be loaded
        // once, not once per script (tens of MB each).
        for language in [Language::En, Language::Ja] {
            let fonts = font_definitions_for_language(language);
            let present: Vec<&Arc<FontData>> =
                fallback_faces(HanOrder::of(language)).into_iter().filter_map(|(k, _)| fonts.font_data.get(k)).collect();
            for (i, a) in present.iter().enumerate() {
                for b in present.iter().skip(i + 1) {
                    assert!(a.font != b.font, "the same font file is loaded for two fallback scripts");
                }
            }
        }
    }

    #[test]
    fn fallback_covers_its_script() {
        // Real render check, per script: the proportional family (what cells and the UI use) must
        // resolve a sample string to a real glyph, not the tofu replacement char. Skips a script
        // whose face is absent (a minimal machine) and fails if a face is present but not wired in.
        let cases: [(&str, Vec<(String, u32)>, &str); 6] = [
            ("sys-cjk-han-ja", cjk_han_japanese(), "カテゴリ"),
            ("sys-cjk-han-zh", cjk_han_simplified(), "页面布局 客户端配置"),
            ("sys-cjk-hangul", cjk_hangul(), "한글"),
            ("sys-thai", thai(), "ทดสอบ"),
            ("sys-hebrew", hebrew(), "עברית"),
            ("ui", candidates(UI), "Русский"),
        ];
        for language in [Language::En, Language::Ja] {
            let ctx = egui::Context::default();
            ctx.set_fonts(font_definitions_for_language(language));
            let mut out = ctx.run_ui(egui::RawInput::default(), |_ui| {});
            out.textures_delta.clear(); // nothing consumes the atlas in a headless test
            // `has_glyphs` is a false negative when the face that owns the script is also the face
            // that draws the replacement glyph (DejaVu Sans as the UI font owns Hebrew), so compare
            // the laid out glyphs with the tofu glyph itself instead.
            for family in [FontFamily::Proportional, FontFamily::Name(CELL.into())] {
                let font = FontId::new(14.0, family);
                let glyphs = |text: &str| {
                    let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::BLACK));
                    galley.rows.iter().flat_map(|r| r.glyphs.iter().filter(|g| g.chr != ' ').map(|g| g.uv_rect)).collect::<Vec<_>>()
                };
                let tofu = glyphs("\u{25FB}");
                for (key, cands, sample) in &cases {
                    if !cands.iter().any(|(p, _)| std::path::Path::new(p).exists()) {
                        continue; // no face for this script on this machine; nothing to assert
                    }
                    let drawn = glyphs(sample);
                    assert!(
                        !drawn.is_empty() && drawn.iter().all(|g| !tofu.contains(g)),
                        "{key} is present but {sample:?} still renders as tofu ({language:?}, {:?})",
                        font.family
                    );
                }
            }
        }
    }
}
