//! The Analysis ToolPak's tools as pure functions: input series in, a laid-out block of results
//! out ([`Out`]), positioned relative to the output's top-left cell. The engine resolves the
//! input and output ranges, converts [`Out`] into cells and charts, and writes it.
//!
//! Labels and layouts follow Excel's ToolPak (the statistics' names are standard terms);
//! numbers come from `gridcraft_functions::dist`, so they match the worksheet functions.

use gridcraft_core::CellError;

use crate::{Error, Result, input};

pub mod descriptive;
pub mod hypothesis;
pub mod regression;
pub mod series;

pub(crate) use descriptive::correl as descriptive_correl;

/// Most input values a tool accepts.
pub const MAX_VALUES: usize = 1_000_000;

/// A report cell's content.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Num(f64),
    Text(String),
    /// A formula (text with `=`), already in terms of the output and input positions.
    Formula(String),
    Error(CellError),
}

impl V {
    pub fn text(t: impl Into<String>) -> V {
        V::Text(t.into())
    }
    /// A number, or `#NUM!` when it isn't finite.
    pub fn num_or_err(x: f64) -> V {
        if x.is_finite() { V::Num(x) } else { V::Error(CellError::Num) }
    }
    /// A statistic that may be undefined (`#DIV/0!`, `#N/A`…), as the worksheet function shows it.
    pub fn stat(r: std::result::Result<f64, CellError>) -> V {
        match r {
            Ok(x) => V::num_or_err(x),
            Err(e) => V::Error(e),
        }
    }
}

/// How a report cell looks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub bold: bool,
    pub italic: bool,
    pub center: bool,
    /// Centred across this and the following blank cells with the same flag.
    pub across: bool,
    /// Thin border above / below.
    pub top: bool,
    pub bottom: bool,
    pub fmt: Option<&'static str>,
}

impl Look {
    pub const PLAIN: Look = Look { bold: false, italic: false, center: false, across: false, top: false, bottom: false, fmt: None };
    /// A heading in a results table: italic and centred.
    pub const HEAD: Look = Look { bold: false, italic: true, center: true, across: false, top: false, bottom: false, fmt: None };
    pub fn under(self) -> Look {
        Look { bottom: true, ..self }
    }
    pub fn over(self) -> Look {
        Look { top: true, ..self }
    }
    /// Centred across the cells to its right that share the flag (a heading over two columns).
    pub fn across(self) -> Look {
        Look { across: true, center: false, ..self }
    }
    pub fn fmt(self, f: &'static str) -> Look {
        Look { fmt: Some(f), ..self }
    }
}

/// Chart kinds the tools draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartKind {
    Column,
    Line,
    Scatter,
}

/// A block of cells for a chart series: inside the output, or input cells by reference text.
#[derive(Clone, Debug, PartialEq)]
pub enum Area {
    Out { row: u32, col: u32, rows: u32, cols: u32 },
    Ref(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChartSeries {
    pub name: Option<String>,
    pub x: Option<Area>,
    pub y: Area,
    /// Draw this series as a line (on a column chart: Pareto's cumulative %).
    pub line: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    pub kind: ChartKind,
    pub title: String,
    pub x_title: Option<String>,
    pub y_title: Option<String>,
    pub series: Vec<ChartSeries>,
    /// Top-left cell relative to the output origin, and size in points.
    pub row: u32,
    pub col: u32,
    pub width: f32,
    pub height: f32,
    pub legend: bool,
}

/// A tool's results.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Out {
    pub cells: Vec<(u32, u32, V, Look)>,
    pub charts: Vec<Chart>,
}

impl Out {
    pub fn put(&mut self, row: u32, col: u32, v: V, look: Look) {
        self.cells.push((row, col, v, look));
    }
    pub fn text(&mut self, row: u32, col: u32, t: &str, look: Look) {
        self.put(row, col, V::text(t), look);
    }
    pub fn num(&mut self, row: u32, col: u32, x: f64, look: Look) {
        self.put(row, col, V::num_or_err(x), look);
    }
    /// Rows and columns used (at least 1 × 1).
    pub fn extent(&self) -> (u32, u32) {
        let rows = self.cells.iter().map(|c| c.0 + 1).max().unwrap_or(1);
        let cols = self.cells.iter().map(|c| c.1 + 1).max().unwrap_or(1);
        (rows, cols)
    }
}

/// One input variable: a column of the input range (or a row, when grouped by rows). Blank
/// cells are `None`.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub label: String,
    pub values: Vec<Option<f64>>,
}

impl Series {
    pub fn new(label: impl Into<String>, values: Vec<Option<f64>>) -> Series {
        Series { label: label.into(), values }
    }
    /// The numbers, skipping blanks.
    pub fn numbers(&self) -> Vec<f64> {
        self.values.iter().flatten().copied().collect()
    }
    /// The numbers, with blanks refused (tools that pair values up).
    pub fn complete(&self) -> Result<Vec<f64>> {
        self.values.iter().map(|v| v.ok_or_else(|| input(format!("{} has blank cells; every input cell must hold a number.", self.label)))).collect()
    }
}

/// The confidence/significance level `alpha` (0 < alpha < 1).
pub fn check_alpha(alpha: f64) -> Result<f64> {
    if alpha > 0.0 && alpha < 1.0 { Ok(alpha) } else { Err(input("Alpha must be greater than 0 and less than 1.")) }
}

/// The default name of the `i`-th variable (`Column1` / `Row1`; ANOVA tables say `Column 1`).
pub fn default_label(by_rows: bool, i: usize, spaced: bool) -> String {
    let what = if by_rows { "Row" } else { "Column" };
    if spaced { format!("{what} {}", i + 1) } else { format!("{what}{}", i + 1) }
}

/// Refuses inputs that are too big to analyse.
pub fn check_size(series: &[Series]) -> Result<()> {
    let n: usize = series.iter().map(|s| s.values.len()).sum();
    if n > MAX_VALUES {
        return Err(Error::TooLarge(format!("The input has more than {MAX_VALUES} values.")));
    }
    if series.is_empty() || series.iter().all(|s| s.values.is_empty()) {
        return Err(input("The input range is empty."));
    }
    Ok(())
}
