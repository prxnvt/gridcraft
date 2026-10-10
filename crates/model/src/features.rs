//! Sheet features beyond cells: tables, conditional formatting, data validation, comments,
//! hyperlinks, charts, images, autofilter, print settings.

use gridcraft_core::{CellRef, RangeRef};
use serde::{Deserialize, Serialize};

use crate::style::{Color, Style};

// ---------------------------------------------------------------- tables

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TotalsFn {
    #[default]
    None,
    Average,
    Count,
    CountNums,
    Max,
    Min,
    Sum,
    StdDev,
    Var,
    Custom,
}

impl TotalsFn {
    /// SUBTOTAL function number.
    pub fn subtotal_code(&self) -> Option<u32> {
        Some(match self {
            TotalsFn::Average => 101,
            TotalsFn::Count => 103,
            TotalsFn::CountNums => 102,
            TotalsFn::Max => 104,
            TotalsFn::Min => 105,
            TotalsFn::Sum => 109,
            TotalsFn::StdDev => 107,
            TotalsFn::Var => 110,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableColumn {
    pub name: String,
    pub totals: TotalsFn,
    /// Totals row label (first column) or custom formula.
    pub totals_label: Option<String>,
    /// Calculated column formula.
    pub formula: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub id: u32,
    pub name: String,
    /// Whole table including header and totals rows.
    pub range: RangeRef,
    pub header_row: bool,
    pub totals_row: bool,
    pub columns: Vec<TableColumn>,
    pub style: String,
    pub banded_rows: bool,
    pub banded_cols: bool,
    pub first_col: bool,
    pub last_col: bool,
    pub filter_button: bool,
}

impl Table {
    pub fn data_range(&self) -> Option<RangeRef> {
        let r0 = self.range.start.row + self.header_row as u32;
        let r1 = self.range.end.row.checked_sub(self.totals_row as u32)?;
        if r1 < r0 {
            return None;
        }
        Some(RangeRef::new(CellRef::new(r0, self.range.start.col), CellRef::new(r1, self.range.end.col)))
    }
    pub fn header_range(&self) -> Option<RangeRef> {
        self.header_row.then(|| RangeRef::new(self.range.start, CellRef::new(self.range.start.row, self.range.end.col)))
    }
    pub fn totals_range(&self) -> Option<RangeRef> {
        self.totals_row.then(|| RangeRef::new(CellRef::new(self.range.end.row, self.range.start.col), self.range.end))
    }
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name.eq_ignore_ascii_case(name))
    }
}

// ---------------------------------------------------------------- conditional formatting

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CfOperator {
    Between,
    NotBetween,
    Equal,
    NotEqual,
    Greater,
    Less,
    GreaterOrEqual,
    LessOrEqual,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CfValueKind {
    Min,
    Max,
    Number(f64),
    Percent(f64),
    Percentile(f64),
    Formula(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CfRule {
    /// Cell value compared with one or two formulas.
    CellIs {
        op: CfOperator,
        a: String,
        b: Option<String>,
        style: Box<Style>,
    },
    /// Custom formula (relative to the top-left cell of the range).
    Expression {
        formula: String,
        style: Box<Style>,
    },
    ContainsText {
        text: String,
        style: Box<Style>,
    },
    NotContainsText {
        text: String,
        style: Box<Style>,
    },
    BeginsWith {
        text: String,
        style: Box<Style>,
    },
    EndsWith {
        text: String,
        style: Box<Style>,
    },
    Blanks {
        style: Box<Style>,
    },
    NoBlanks {
        style: Box<Style>,
    },
    Errors {
        style: Box<Style>,
    },
    NoErrors {
        style: Box<Style>,
    },
    Duplicate {
        style: Box<Style>,
    },
    Unique {
        style: Box<Style>,
    },
    /// Top/bottom N or N percent.
    Top10 {
        bottom: bool,
        percent: bool,
        rank: u32,
        style: Box<Style>,
    },
    AboveAverage {
        below: bool,
        equal: bool,
        std_dev: u8,
        style: Box<Style>,
    },
    /// `yesterday`, `today`, `tomorrow`, `last7Days`, `lastWeek`, `thisWeek`, `nextWeek`, `lastMonth`, `thisMonth`, `nextMonth`.
    TimePeriod {
        period: String,
        style: Box<Style>,
    },
    ColorScale {
        stops: Vec<(CfValueKind, Color)>,
    },
    DataBar {
        min: CfValueKind,
        max: CfValueKind,
        color: Color,
        gradient: bool,
        show_value: bool,
    },
    /// Icon set name (`3Arrows`, `3TrafficLights1`, `4Rating`, `5Quarters`…).
    IconSet {
        set: String,
        thresholds: Vec<CfValueKind>,
        reverse: bool,
        show_value: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CondFormat {
    pub ranges: Vec<RangeRef>,
    pub rule: CfRule,
    pub priority: u32,
    pub stop_if_true: bool,
}

// ---------------------------------------------------------------- data validation

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationKind {
    #[default]
    Any,
    Whole,
    Decimal,
    List,
    Date,
    Time,
    TextLength,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorStyle {
    #[default]
    Stop,
    Warning,
    Information,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Validation {
    pub ranges: Vec<RangeRef>,
    pub kind: ValidationKind,
    pub op: CfOperator,
    /// Formula 1 (list source: `"a,b,c"` literal list or `=$A$1:$A$5`).
    pub f1: String,
    pub f2: Option<String>,
    pub allow_blank: bool,
    pub in_cell_dropdown: bool,
    pub input_title: String,
    pub input_message: String,
    pub show_input: bool,
    pub error_title: String,
    pub error_message: String,
    pub error_style: ErrorStyle,
    pub show_error: bool,
}

impl Default for Validation {
    fn default() -> Self {
        Validation {
            ranges: vec![],
            kind: ValidationKind::Any,
            op: CfOperator::Between,
            f1: String::new(),
            f2: None,
            allow_blank: true,
            in_cell_dropdown: true,
            input_title: String::new(),
            input_message: String::new(),
            show_input: true,
            error_title: String::new(),
            error_message: String::new(),
            error_style: ErrorStyle::Stop,
            show_error: true,
        }
    }
}

// ---------------------------------------------------------------- comments, links

/// A note (classic comment) or a threaded comment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub author: String,
    pub text: String,
    /// Replies of a threaded comment (author, text).
    pub replies: Vec<(String, String)>,
    pub threaded: bool,
    pub resolved: bool,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hyperlink {
    /// URL, `mailto:`, or an in-workbook location like `Sheet2!A1`.
    pub target: String,
    pub tooltip: Option<String>,
}

// ---------------------------------------------------------------- drawing objects

/// How a floating object is pinned to the grid when its cells move or resize. Mirrors Excel's
/// Format ▸ Size and Properties ▸ Properties, stored in the file as the anchor's `editAs`:
/// `twoCell` (absent `editAs`), `oneCell` and `absolute`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnchorMode {
    /// Move and size with cells (Excel's default; `<xdr:twoCellAnchor>`).
    #[default]
    MoveAndSize,
    /// Move but don't size with cells (`<xdr:twoCellAnchor editAs="oneCell">` / `<xdr:oneCellAnchor>`).
    MoveOnly,
    /// Don't move or size with cells (`editAs="absolute"` / `<xdr:absoluteAnchor>`).
    Absolute,
}

/// Position of a floating object: top-left cell plus offset, size in points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub cell: CellRef,
    pub dx: f32,
    pub dy: f32,
    pub width: f32,
    pub height: f32,
    /// How the object follows its anchor cell when rows/columns move (sort, insert, delete).
    #[serde(default)]
    pub mode: AnchorMode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChartKind {
    #[default]
    ColumnClustered,
    ColumnStacked,
    ColumnStacked100,
    BarClustered,
    BarStacked,
    BarStacked100,
    Line,
    LineMarkers,
    LineStacked,
    Pie,
    Doughnut,
    Area,
    AreaStacked,
    Scatter,
    ScatterLines,
    Bubble,
    Radar,
    Histogram,
    Waterfall,
    Funnel,
    Treemap,
    Sunburst,
    BoxWhisker,
    Stock,
    Combo,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegendPos {
    None,
    #[default]
    Bottom,
    Top,
    Left,
    Right,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// Formula text for the series name (`Sheet1!$B$1`) or a literal.
    pub name: Option<String>,
    pub categories: Option<String>,
    pub values: String,
    /// X values for scatter / bubble sizes.
    pub bubble_sizes: Option<String>,
    pub color: Option<Color>,
    /// Combo charts: this series drawn as a line on the secondary axis.
    pub secondary: bool,
    pub kind: Option<ChartKind>,
    /// Draw line segments as a smoothed (spline) curve.
    #[serde(default)]
    pub smooth: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    pub id: u32,
    pub kind: ChartKind,
    pub anchor: Anchor,
    pub title: Option<String>,
    pub series: Vec<Series>,
    pub legend: LegendPos,
    pub data_labels: bool,
    pub gridlines: bool,
    pub style: u32,
    pub x_title: Option<String>,
    pub y_title: Option<String>,
    /// Source range when created from a selection (for Switch Row/Column).
    pub source: Option<String>,
    pub by_rows: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub id: u32,
    pub anchor: Anchor,
    /// Encoded bytes (PNG/JPEG), base64 in JSON.
    #[serde(with = "crate::b64")]
    pub data: Vec<u8>,
    pub mime: String,
    pub alt: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    #[default]
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Triangle,
    Line,
    Arrow,
    TextBox,
    /// An icon from GridCraft's own icon set; `Shape::text` holds the icon name.
    Icon,
    /// A freehand ink stroke; `Shape::text` holds the points as `x,y;x,y;…` relative to the
    /// anchor (sheet points), `Shape::line` the ink colour and `fill` unused.
    Ink,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: u32,
    pub kind: ShapeKind,
    pub anchor: Anchor,
    pub fill: Color,
    pub line: Color,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SparklineKind {
    #[default]
    Line,
    Column,
    WinLoss,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sparkline {
    pub cell: CellRef,
    pub source: String,
    pub kind: SparklineKind,
    pub color: Color,
    pub markers: bool,
}

// ---------------------------------------------------------------- filter & sort

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FilterCriterion {
    /// Show rows whose display text is one of these (case-insensitive); `blanks` includes empty.
    Values {
        values: Vec<String>,
        blanks: bool,
    },
    /// Custom: one or two comparisons (`op` like `>=`, `=*abc*`), joined by and/or.
    Custom {
        a: (String, String),
        b: Option<(String, String)>,
        and: bool,
    },
    Top10 {
        bottom: bool,
        percent: bool,
        count: u32,
    },
    AboveAverage(bool),
    FillColor(Color),
    FontColor(Color),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoFilter {
    pub range: RangeRef,
    /// Column offset within `range` → criterion.
    pub criteria: Vec<(u32, FilterCriterion)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrintSettings {
    pub orientation: Orientation,
    /// Paper size name (`Letter`, `A4`, `Legal`…).
    pub paper: String,
    /// Margins in inches: left, right, top, bottom, header, footer.
    pub margins: [f32; 6],
    pub print_area: Option<RangeRef>,
    pub title_rows: Option<(u32, u32)>,
    pub title_cols: Option<(u32, u32)>,
    /// Percent scale, or fit to pages (wide, tall).
    pub scale: u16,
    pub fit_to: Option<(u16, u16)>,
    pub gridlines: bool,
    pub headings: bool,
    pub center_h: bool,
    pub center_v: bool,
    pub header: String,
    pub footer: String,
    pub row_breaks: Vec<u32>,
    pub col_breaks: Vec<u32>,
}

impl Default for PrintSettings {
    fn default() -> Self {
        PrintSettings {
            orientation: Orientation::Portrait,
            paper: "Letter".into(),
            margins: [0.7, 0.7, 0.75, 0.75, 0.3, 0.3],
            print_area: None,
            title_rows: None,
            title_cols: None,
            scale: 100,
            fit_to: None,
            gridlines: false,
            headings: false,
            center_h: false,
            center_v: false,
            header: String::new(),
            footer: String::new(),
            row_breaks: vec![],
            col_breaks: vec![],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SheetProtection {
    /// Hash of the password (our own salted FNV; never the password itself).
    pub password_hash: Option<u64>,
    pub select_locked: bool,
    pub select_unlocked: bool,
    pub format_cells: bool,
    pub format_columns: bool,
    pub format_rows: bool,
    pub insert_columns: bool,
    pub insert_rows: bool,
    pub delete_columns: bool,
    pub delete_rows: bool,
    pub sort: bool,
    pub autofilter: bool,
}

/// Simple non-cryptographic password hash for sheet protection (like Excel's, protection is a
/// convenience, not security).
pub fn password_hash(pw: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325 ^ 0x5348_4545_5443;
    for b in pw.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Outline grouping info for a row or column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LineInfo {
    /// Size in points; `None` = default (rows auto-fit to content).
    pub size: Option<f32>,
    pub hidden: bool,
    pub outline: u8,
    pub collapsed: bool,
    pub style: Option<crate::style::StyleId>,
    /// The size was set by the user (rows): automatic heights follow the content.
    #[serde(default)]
    pub custom: bool,
}

// ---------------------------------------------------------------- data tables

/// A What-If data table (`{=TABLE(row_input, column_input)}`). The range's top row and left
/// column are its edges: input values along one or both, formulas along the other (or, for a
/// two-variable table, one formula in the top-left corner). The rest is the body, whose cells
/// hold the formulas' results with each input value substituted. Input cells are on the same
/// sheet, outside the range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataTable {
    pub range: RangeRef,
    /// Receives the values across the top row.
    pub row_input: Option<CellRef>,
    /// Receives the values down the left column.
    pub col_input: Option<CellRef>,
}

/// One substitution of a data table: set each `(input, value_cell)` input to the value in
/// `value_cell`, then copy each `(formula, out)` formula result into `out`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableTrial {
    pub inputs: Vec<(CellRef, CellRef)>,
    pub outputs: Vec<(CellRef, CellRef)>,
}

impl DataTable {
    /// The results area (the range without its top row and left column).
    pub fn body(&self) -> Option<RangeRef> {
        let r = self.range;
        (r.height() >= 2 && r.width() >= 2).then(|| RangeRef::new(CellRef::new(r.start.row + 1, r.start.col + 1), r.end))
    }

    /// Whether `c` is on the top row or left column (the cells the results depend on).
    pub fn is_edge(&self, c: CellRef) -> bool {
        self.range.contains(c) && (c.row == self.range.start.row || c.col == self.range.start.col)
    }

    /// The edge cells: top row and left column.
    pub fn edges(&self) -> Vec<CellRef> {
        let r = self.range;
        let mut v: Vec<CellRef> = (r.start.col..=r.end.col).map(|c| CellRef::new(r.start.row, c)).collect();
        v.extend((r.start.row + 1..=r.end.row).map(|row| CellRef::new(row, r.start.col)));
        v
    }

    /// The `{=TABLE(…)}` text shown for body cells (absolute input references, blank when absent).
    pub fn formula_text(&self) -> String {
        let a = |c: Option<CellRef>| c.map(|c| format!("${}${}", gridcraft_core::col_to_letters(c.col), c.row + 1)).unwrap_or_default();
        format!("TABLE({},{})", a(self.row_input), a(self.col_input))
    }

    /// The trials that fill the body, grouped so each substitution is computed once.
    pub fn trials(&self) -> Vec<TableTrial> {
        let Some(body) = self.body() else { return vec![] };
        let (r0, c0) = (self.range.start.row, self.range.start.col);
        match (self.row_input, self.col_input) {
            // Values down the left column, formulas across the top row.
            (None, Some(ci)) => (body.start.row..=body.end.row)
                .map(|row| TableTrial {
                    inputs: vec![(ci, CellRef::new(row, c0))],
                    outputs: (body.start.col..=body.end.col).map(|col| (CellRef::new(r0, col), CellRef::new(row, col))).collect(),
                })
                .collect(),
            // Values across the top row, formulas down the left column.
            (Some(ri), None) => (body.start.col..=body.end.col)
                .map(|col| TableTrial {
                    inputs: vec![(ri, CellRef::new(r0, col))],
                    outputs: (body.start.row..=body.end.row).map(|row| (CellRef::new(row, c0), CellRef::new(row, col))).collect(),
                })
                .collect(),
            // One formula in the corner; every body cell is its own trial.
            (Some(ri), Some(ci)) => body
                .iter()
                .map(|c| TableTrial {
                    inputs: vec![(ri, CellRef::new(r0, c.col)), (ci, CellRef::new(c.row, c0))],
                    outputs: vec![(CellRef::new(r0, c0), c)],
                })
                .collect(),
            (None, None) => vec![],
        }
    }
}

// ---------------------------------------------------------------- pivot tables

/// How a PivotTable lays out its row fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PivotLayout {
    /// All row fields in one column, inner items indented ("Row Labels" header).
    #[default]
    Compact,
    /// One column per row field; each item on its own row.
    Outline,
    /// One column per row field; inner items start on their parent's row, totals below.
    Tabular,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PivotSort {
    #[default]
    Asc,
    Desc,
    /// Source order (first appearance).
    None,
}

/// Grouping of a date field (values are date serials).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PivotDateGroup {
    #[default]
    None,
    Years,
    Quarters,
    Months,
    Days,
    Hours,
    Minutes,
    Seconds,
}

/// Grouping of a number field into ranges (Group › By).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PivotNumberGroup {
    pub start: f64,
    pub end: f64,
    pub by: f64,
}

/// One manual group: a name and the items in it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PivotNamedGroup {
    pub name: String,
    pub items: Vec<String>,
}

/// A field made by grouping selected items of `base` (Group Selection): its items are the
/// group names, and the ungrouped items as they are. Named like Excel's ("Region2").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PivotManualGroup {
    pub name: String,
    pub base: String,
    pub groups: Vec<PivotNamedGroup>,
}

/// A row or column field's subtotals (Field Settings › Subtotals).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PivotSubtotals {
    /// One subtotal per value field with its own function (when the PivotTable shows subtotals).
    #[default]
    Automatic,
    None,
    /// One subtotal line per function.
    Custom(Vec<PivotFunc>),
}

/// A field placed in the Rows or Columns area.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PivotField {
    /// Source column header name.
    pub source_col: String,
    pub sort: PivotSort,
    pub date_group: PivotDateGroup,
    /// Item labels whose detail is hidden (collapsed).
    pub collapsed_items: Vec<String>,
    /// Custom name shown in headers (Field Settings › Custom Name).
    pub caption: Option<String>,
    pub subtotals: PivotSubtotals,
    /// Repeat the item labels on every row (outline and tabular forms).
    pub repeat_labels: bool,
    /// Insert a blank line after each item.
    pub blank_line: bool,
    /// Show items that have no data under every parent.
    pub show_empty: bool,
    /// Insert a page break after each item (printing).
    pub page_break: bool,
    /// Days grouping: number of days per group (0 or 1 = each day).
    pub group_days: u32,
    /// Grouping bounds (Starting at / Ending at); values outside fall into "<start" / ">end".
    pub group_start: Option<f64>,
    pub group_end: Option<f64>,
    pub number_group: Option<PivotNumberGroup>,
}

impl PivotField {
    /// The name shown in headers.
    pub fn shown_name(&self) -> &str {
        self.caption.as_deref().filter(|c| !c.is_empty()).unwrap_or(&self.source_col)
    }
}

/// PivotTable Options (and the Design tab's style options).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PivotOptions {
    /// Layout & Format.
    pub merge_labels: bool,
    /// Compact form indent, in characters.
    pub indent: u8,
    /// Report filters: over, then down (else down, then over).
    pub filters_over_then_down: bool,
    /// Report filter fields per column (row); 0 = all in one.
    pub filters_per_column: u32,
    pub error_text: Option<String>,
    pub empty_text: Option<String>,
    pub autofit: bool,
    pub preserve_formatting: bool,
    /// Totals & Filters.
    pub multiple_filters: bool,
    pub custom_list_sort: bool,
    /// Display.
    pub expand_buttons: bool,
    pub contextual_tooltips: bool,
    pub classic_layout: bool,
    pub show_values_row: bool,
    pub empty_rows: bool,
    pub empty_columns: bool,
    pub field_list_az: bool,
    pub row_header_caption: Option<String>,
    pub column_header_caption: Option<String>,
    /// Printing.
    pub print_buttons: bool,
    pub repeat_row_labels: bool,
    pub print_titles: bool,
    /// Data.
    pub save_data: bool,
    pub enable_drilldown: bool,
    pub refresh_on_open: bool,
    /// Alt Text.
    pub alt_title: String,
    pub alt_description: String,
    /// Design › PivotTable Style Options.
    pub row_headers: bool,
    pub column_headers: bool,
    pub banded_rows: bool,
    pub banded_columns: bool,
}

impl Default for PivotOptions {
    fn default() -> Self {
        PivotOptions {
            merge_labels: false,
            indent: 1,
            filters_over_then_down: false,
            filters_per_column: 0,
            error_text: None,
            empty_text: None,
            autofit: true,
            preserve_formatting: true,
            multiple_filters: false,
            custom_list_sort: true,
            expand_buttons: true,
            contextual_tooltips: true,
            classic_layout: false,
            show_values_row: true,
            empty_rows: false,
            empty_columns: false,
            field_list_az: false,
            row_header_caption: None,
            column_header_caption: None,
            print_buttons: false,
            repeat_row_labels: true,
            print_titles: false,
            save_data: true,
            enable_drilldown: true,
            refresh_on_open: false,
            alt_title: String::new(),
            alt_description: String::new(),
            row_headers: true,
            column_headers: true,
            banded_rows: false,
            banded_columns: false,
        }
    }
}

/// Summarize-values-by function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PivotFunc {
    #[default]
    Sum,
    Count,
    Average,
    Max,
    Min,
    Product,
    CountNumbers,
    StdDev,
    StdDevP,
    Var,
    VarP,
}

/// Show-values-as transform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PivotShowAs {
    #[default]
    Normal,
    PercentOfGrandTotal,
    PercentOfColumnTotal,
    PercentOfRowTotal,
    /// Running total in the base field (the first row field when none is given).
    RunningTotal,
    /// Rank largest to smallest among the base field's items.
    Rank,
    /// As a percentage of the base field's base item.
    PercentOf,
    /// As a percentage of the parent row (the row one level up).
    PercentOfParentRowTotal,
    /// As a percentage of the parent column.
    PercentOfParentColumnTotal,
    /// As a percentage of the total of the base field's item that contains it.
    PercentOfParentTotal,
    /// Minus the base field's base item.
    DifferenceFrom,
    /// (value − base item) / base item.
    PercentDifferenceFrom,
    /// Running total in the base field, as a percentage of the field's total.
    PercentRunningTotal,
    /// Rank smallest to largest among the base field's items.
    RankAscending,
    /// (value × grand total) / (row total × column total).
    Index,
}

impl PivotShowAs {
    /// Whether the transform needs a base field.
    pub fn needs_base_field(self) -> bool {
        matches!(
            self,
            PivotShowAs::RunningTotal
                | PivotShowAs::PercentRunningTotal
                | PivotShowAs::Rank
                | PivotShowAs::RankAscending
                | PivotShowAs::PercentOf
                | PivotShowAs::PercentOfParentTotal
                | PivotShowAs::DifferenceFrom
                | PivotShowAs::PercentDifferenceFrom
        )
    }
    /// Whether it also needs a base item.
    pub fn needs_base_item(self) -> bool {
        matches!(self, PivotShowAs::PercentOf | PivotShowAs::DifferenceFrom | PivotShowAs::PercentDifferenceFrom)
    }
    /// Whether the result is a percentage.
    pub fn is_percent(self) -> bool {
        matches!(
            self,
            PivotShowAs::PercentOfGrandTotal
                | PivotShowAs::PercentOfColumnTotal
                | PivotShowAs::PercentOfRowTotal
                | PivotShowAs::PercentOf
                | PivotShowAs::PercentOfParentRowTotal
                | PivotShowAs::PercentOfParentColumnTotal
                | PivotShowAs::PercentOfParentTotal
                | PivotShowAs::PercentDifferenceFrom
                | PivotShowAs::PercentRunningTotal
        )
    }
}

/// A base item that means the previous / next item of the base field.
pub const BASE_PREVIOUS: &str = "(previous)";
pub const BASE_NEXT: &str = "(next)";

/// A field in the Values area.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PivotValue {
    pub source_col: String,
    pub func: PivotFunc,
    /// Caption, e.g. "Sum of Sales".
    pub name: String,
    pub show_as: PivotShowAs,
    pub number_format: Option<String>,
    /// Show Values As base field (source column name).
    pub base_field: Option<String>,
    /// Show Values As base item: an item label, [`BASE_PREVIOUS`] or [`BASE_NEXT`].
    pub base_item: Option<String>,
}

/// A report filter (or, for a field also in Rows/Columns, its hidden-items filter).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PivotFilter {
    pub source_col: String,
    /// Item labels shown; `None` = all.
    pub selected: Option<Vec<String>>,
}

/// A PivotTable: its definition plus where its output was last written. The output itself is
/// static cell values and styles on the sheet, rewritten on every refresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PivotTable {
    pub id: u32,
    pub name: String,
    /// Source: range formula text (`Sheet1!$A$1:$E$200`), a table name or a defined name.
    pub source: String,
    /// Top-left of the output body on the pivot's sheet.
    pub anchor: CellRef,
    pub rows: Vec<PivotField>,
    pub columns: Vec<PivotField>,
    pub values: Vec<PivotValue>,
    pub filters: Vec<PivotFilter>,
    pub layout: PivotLayout,
    /// Grand total for each row (the "Grand Total" column), like Excel's "On for Rows".
    pub grand_totals_rows: bool,
    /// Grand total for each column (the "Grand Total" row), like Excel's "On for Columns".
    pub grand_totals_cols: bool,
    pub subtotals: bool,
    /// Subtotals at the top of each group (Compact/Outline); otherwise below it.
    pub subtotals_top: bool,
    pub style: String,
    pub show_headers: bool,
    /// Area written by the last refresh.
    pub last_range: Option<RangeRef>,
    pub options: PivotOptions,
    /// Fields made by grouping selected items.
    pub manual_groups: Vec<PivotManualGroup>,
}

impl Default for PivotTable {
    fn default() -> Self {
        PivotTable {
            id: 0,
            name: "PivotTable1".into(),
            source: String::new(),
            anchor: CellRef::default(),
            rows: vec![],
            columns: vec![],
            values: vec![],
            filters: vec![],
            layout: PivotLayout::Compact,
            grand_totals_rows: true,
            grand_totals_cols: true,
            subtotals: true,
            subtotals_top: true,
            style: "PivotStyleLight16".into(),
            show_headers: true,
            last_range: None,
            options: PivotOptions::default(),
            manual_groups: vec![],
        }
    }
}
