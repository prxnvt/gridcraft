//! The worker protocol: one JSON object per line, in both directions.
//!
//! The parent writes [`Request`] lines to the worker's standard input:
//!
//! - `{"type":"run","id":N,"code":"…","inputs":{"A1:B2":{"value":V}},"output":"object"}` runs one
//!   cell's code. `output` is there when the cell's return type is a Python object.
//!   `inputs` maps each literal `xl()` reference text in the code to its value, resolved by the
//!   parent before the request; there are no callbacks. An entry may carry `"headers":true`,
//!   which the worker accepts and does not use. With `"init":"person"` or `"init":"file"` the code is
//!   the workbook's initialization code ([`Init`]), and `value` in its reply lists what it skipped.
//! - `{"type":"reset","id":N}` clears the shared globals.
//!
//! The worker writes [`Reply`] lines to its standard output:
//!
//! - `{"type":"ready","selfTest":{"status":"notChecked"}}` once, when it starts;
//! - exactly one `{"type":"done","id":N,"ok":true,"value":V,"stdout":"…"}` or
//!   `{"type":"done","id":N,"ok":false,"error":{"message":"…","line":3,"kind":"denied.file",
//!   "subject":"/a/b"},"stdout":"…"}` per request (`stdout` is left out when nothing was printed;
//!   `kind` and `subject` when the worker has none). Either can carry `"peak":N`, the most resident
//!   memory the process has used so far, in bytes, which catches a spike the parent's watcher reads
//!   too late or not at all.
//!
//! A value `V` is `null` (empty), a number, a string, `true` or `false`, `{"error":"#N/A"}`, a date
//! or time as `{"date":"2024-01-15"}`, `{"dateTime":"2024-01-15T10:30:00.5"}` or
//! `{"time":"10:30:00"}`, `{"duration":1.5}` in days, `{"array":{"rows":R,"cols":C,"data":[…]}}` with
//! `R × C` scalars in row order, or, only as a result, `{"object":{"typeName":"…","preview":"…"}}`.
//! Dates carry no serial numbers, as only GridCraft knows the workbook's date system ([`Stamp`]).
//!
//! Everything the worker writes is untrusted: [`Reply::parse`] refuses a line that breaks the
//! shape or goes over any cap below, and the client kills the worker for it. `shim.py` holds the
//! same caps and keeps its own output within them.

use std::collections::BTreeMap;
use std::fmt;

use gridcraft_core::{Array, CellError, Value};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};

mod stamp;

pub use stamp::{Clock, Dated, Day, MAX_DATE_BYTES, Stamp};

/// Bytes in one line from the worker, without its newline. A 1,000,000-cell result of numbers
/// takes about 20 MB.
pub const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;
/// Bytes in one text value: enough for Excel's 32,767 characters in any script.
pub const MAX_TEXT_BYTES: usize = 128 * 1024;
/// Bytes in an error message.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024;
/// Bytes in an error's kind and in its subject.
pub const MAX_KIND_BYTES: usize = 64;
pub const MAX_SUBJECT_BYTES: usize = 1024;
/// Bytes in an object's type name.
pub const MAX_TYPE_NAME_BYTES: usize = 256;
/// Characters in an object's preview (its `repr`, cut).
pub const MAX_PREVIEW_CHARS: usize = 255;
/// Bytes of printed output kept per run.
pub const MAX_STDOUT_BYTES: usize = 1024 * 1024;
/// What the worker puts at the end of printed output it cut at [`MAX_STDOUT_BYTES`] (`shim.py`
/// writes the same text).
pub const OUTPUT_CUT: &str = "\n[output cut off]\n";
/// Cells in one array.
pub const MAX_ARRAY_CELLS: usize = 1_000_000;
/// The largest `peak` a reply may report, in bytes.
pub const MAX_PEAK_BYTES: u64 = 1 << 60;
/// Bytes in the self-test's detail.
pub const MAX_SELF_TEST_BYTES: usize = 4 * 1024;
/// Bytes in an error value's text (`#GETTING_DATA` is the longest).
const MAX_ERROR_TEXT_BYTES: usize = 32;

/// One `xl()` reference's value, as the parent resolved it.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    /// A scalar for one cell, an array for a range. [`Value::Empty`] where a stamp goes.
    pub value: Value,
    /// `xl(ref, headers=True)`: carried in the request; the worker does not read it.
    pub headers: Option<bool>,
    /// The cells that hold dates and times, by their place in row order, in ascending order.
    pub stamps: Vec<(u32, Stamp)>,
}

impl Input {
    pub fn new(value: Value) -> Input {
        Input { value, headers: None, stamps: Vec::new() }
    }
}

/// Whose initialization code a request runs, which decides what a failing line means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Init {
    /// A person wrote or changed it: any error is the code's failure. Only lines that import or use
    /// `excel`, Excel's own module that no other Python has, are skipped.
    Person,
    /// The application that wrote the file put it there. Such code imports packages and sets options
    /// (Excel's default does), so what it can't have doesn't stop the cells: a line that fails to
    /// import, what needs that line, and a setting on a module that fails, are skipped and reported.
    /// Any other error is the code's failure.
    File,
}

impl Init {
    fn id(self) -> &'static str {
        match self {
            Init::Person => "person",
            Init::File => "file",
        }
    }

    fn parse(id: &str) -> Option<Init> {
        [Init::Person, Init::File].into_iter().find(|i| i.id() == id)
    }
}

/// Parent to worker.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Run one cell's code in the shared globals. With `object`, a result with no single-cell form
    /// comes back as a Python object, not as cells to spill. With `init`, the code is initialization
    /// code rather than a cell's: `xl()` isn't there, and the reply's value is empty or a row of
    /// `module, line` for each package it skipped.
    Run { id: u64, code: String, inputs: BTreeMap<String, Input>, object: bool, init: Option<Init> },
    /// Clear the shared globals.
    Reset { id: u64 },
}

impl Request {
    pub fn id(&self) -> u64 {
        match self {
            Request::Run { id, .. } | Request::Reset { id } => *id,
        }
    }

    /// The request as one line, with its newline.
    pub fn to_line(&self) -> Vec<u8> {
        let mut out = match serde_json::to_vec(&WireRequest(self)) {
            Ok(v) => v,
            // Serializing these types can't fail: every map key is a string, and numbers that
            // JSON can't hold are written as #NUM!.
            Err(_) => b"{}".to_vec(),
        };
        out.push(b'\n');
        out
    }

    /// Reads a request line (the worker's side, and the fake worker in tests).
    pub fn parse(line: &[u8]) -> Result<Request, String> {
        let raw: RawRequest = serde_json::from_slice(line).map_err(|e| e.to_string())?;
        match raw.kind {
            RequestKind::Run => {
                let input = |v: RawInput| Input { value: v.value.0.value, headers: v.headers, stamps: v.value.0.stamps };
                let inputs = raw.inputs.unwrap_or_default().into_iter().map(|(k, v)| (k, input(v))).collect();
                let init = raw.init.as_deref().and_then(Init::parse);
                Ok(Request::Run { id: raw.id, code: raw.code.unwrap_or_default(), inputs, object: raw.output.as_deref() == Some("object"), init })
            }
            RequestKind::Reset => Ok(Request::Reset { id: raw.id }),
        }
    }
}

/// A Python result.
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    /// A value cells can hold: a scalar, or an array to spill.
    Value(Value),
    /// The same with dates, times or durations in it.
    Dated(Dated),
    /// A Python object with no cell form.
    Object {
        /// Python's name for its type (`dict`, `function`…), at most [`MAX_TYPE_NAME_BYTES`].
        type_name: String,
        /// Its `repr`, at most [`MAX_PREVIEW_CHARS`] characters.
        preview: String,
    },
}

/// The error a cell's code raised.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PyError {
    /// `ExceptionType: message`, at most [`MAX_MESSAGE_BYTES`].
    pub message: String,
    /// The line in the cell's code, when the error happened in it.
    pub line: Option<u32>,
    /// What kind of failure the worker takes this for, an id of the diagnostics catalog. Untrusted:
    /// [`crate::diagnostics::for_error`] maps one it doesn't know to a plain Python error.
    pub kind: Option<String>,
    /// What it was about: a module's name, the file tried.
    pub subject: Option<String>,
}

/// The worker's check of its own confinement, reported in `ready`. A worker that makes no such
/// check reports [`SelfTest::NotChecked`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelfTest {
    NotChecked,
    Passed,
    Failed(String),
}

/// Worker to parent.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// The worker started.
    Ready { self_test: SelfTest },
    /// Request `id` finished.
    Done {
        id: u64,
        result: Result<Output, PyError>,
        stdout: String,
        /// The most resident memory the worker has used so far, in bytes, when it can tell. Untrusted:
        /// too high only stops the worker that says so, and the parent's watcher stays for too low.
        peak: Option<u64>,
    },
}

impl Reply {
    /// Reads one line from the worker, refusing anything malformed or over a cap.
    pub fn parse(line: &[u8]) -> Result<Reply, String> {
        if line.len() > MAX_LINE_BYTES {
            return Err(format!("a line of more than {MAX_LINE_BYTES} bytes"));
        }
        let raw: RawReply = serde_json::from_slice(line).map_err(|e| e.to_string())?;
        raw.validate()
    }

    /// The reply as one line, with its newline (the fake worker in tests writes these).
    pub fn to_line(&self) -> Vec<u8> {
        let mut out = match serde_json::to_vec(&WireReply(self)) {
            Ok(v) => v,
            Err(_) => b"{}".to_vec(),
        };
        out.push(b'\n');
        out
    }
}

// ---- Reading -------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum ReplyKind {
    Ready,
    Done,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawReply {
    #[serde(rename = "type")]
    kind: ReplyKind,
    id: Option<u64>,
    ok: Option<bool>,
    // `"value": null` is an empty result, not a missing one.
    #[serde(default, deserialize_with = "present")]
    value: Option<ResultValue>,
    error: Option<RawError>,
    stdout: Option<Capped<MAX_STDOUT_BYTES>>,
    self_test: Option<RawSelfTest>,
    peak: Option<u64>,
}

impl RawReply {
    fn validate(self) -> Result<Reply, String> {
        match self.kind {
            ReplyKind::Ready => {
                if self.id.is_some()
                    || self.ok.is_some()
                    || self.value.is_some()
                    || self.error.is_some()
                    || self.stdout.is_some()
                    || self.peak.is_some()
                {
                    return Err("ready carries only selfTest".into());
                }
                let st = self.self_test.ok_or("ready without selfTest")?;
                let self_test = match (st.status, st.detail) {
                    (SelfTestStatus::NotChecked, None) => SelfTest::NotChecked,
                    (SelfTestStatus::Passed, None) => SelfTest::Passed,
                    (SelfTestStatus::Failed, detail) => SelfTest::Failed(detail.map(|d| d.0).unwrap_or_default()),
                    _ => return Err("selfTest detail only goes with a failure".into()),
                };
                Ok(Reply::Ready { self_test })
            }
            ReplyKind::Done => {
                if self.self_test.is_some() {
                    return Err("done carries no selfTest".into());
                }
                let id = self.id.ok_or("done without id")?;
                if self.peak.is_some_and(|p| p > MAX_PEAK_BYTES) {
                    return Err(format!("a peak of more than {MAX_PEAK_BYTES} bytes"));
                }
                let result = match (self.ok, self.value, self.error) {
                    (Some(true), Some(v), None) => Ok(v.0),
                    (Some(false), None, Some(e)) => {
                        Err(PyError { message: e.message.0, line: e.line, kind: e.kind.map(|k| k.0), subject: e.subject.map(|k| k.0) })
                    }
                    _ => return Err("done needs ok with a value, or not ok with an error".into()),
                };
                Ok(Reply::Done { id, result, stdout: self.stdout.map(|s| s.0).unwrap_or_default(), peak: self.peak })
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawError {
    message: Capped<MAX_MESSAGE_BYTES>,
    line: Option<u32>,
    kind: Option<Capped<MAX_KIND_BYTES>>,
    subject: Option<Capped<MAX_SUBJECT_BYTES>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum SelfTestStatus {
    NotChecked,
    Passed,
    Failed,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSelfTest {
    status: SelfTestStatus,
    detail: Option<Capped<MAX_SELF_TEST_BYTES>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum RequestKind {
    Run,
    Reset,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRequest {
    #[serde(rename = "type")]
    kind: RequestKind,
    id: u64,
    code: Option<String>,
    inputs: Option<BTreeMap<String, RawInput>>,
    output: Option<String>,
    init: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawInput {
    value: CellValue,
    headers: Option<bool>,
}

/// A string of at most `N` bytes; a longer one is refused, not cut.
struct Capped<const N: usize>(String);

impl<'de, const N: usize> Deserialize<'de> for Capped<N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() > N {
            return Err(de::Error::custom(format!("a string of {} bytes, over the cap of {N}", s.len())));
        }
        Ok(Capped(s))
    }
}

/// A field that is there, even as `null`.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<ResultValue>, D::Error> {
    ResultValue::deserialize(d).map(Some)
}

/// A result: a cell value or an object.
struct ResultValue(Output);
/// A value cells can hold (no object).
struct CellValue(Dated);

/// A value as it is read.
enum Parsed {
    Value(Value),
    Stamp(Stamp),
    /// An array with stamps in it.
    Dated(Dated),
    Object {
        type_name: String,
        preview: String,
    },
}

impl Parsed {
    fn into_output(self) -> Output {
        match self {
            Parsed::Value(v) => Output::Value(v),
            Parsed::Stamp(s) => Output::Dated(Dated { value: Value::Empty, stamps: vec![(0, s)] }),
            Parsed::Dated(d) => Output::Dated(d),
            Parsed::Object { type_name, preview } => Output::Object { type_name, preview },
        }
    }
}

impl<'de> Deserialize<'de> for ResultValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ValueVisitor { place: Place::Result }).map(|p| ResultValue(p.into_output()))
    }
}

impl<'de> Deserialize<'de> for CellValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match d.deserialize_any(ValueVisitor { place: Place::Cell })?.into_output() {
            Output::Value(value) => Ok(CellValue(Dated { value, stamps: Vec::new() })),
            Output::Dated(dated) => Ok(CellValue(dated)),
            Output::Object { .. } => Err(de::Error::custom("an object where a cell value goes")),
        }
    }
}

/// Where a value sits, which decides what it may be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// A run's result: anything, an object included.
    Result,
    /// An input: a scalar or an array.
    Cell,
    /// An element of an array: a scalar only.
    Element,
}

struct ValueVisitor {
    place: Place,
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "camelCase")]
enum ValueKey {
    Error,
    Array,
    Object,
    Date,
    DateTime,
    Time,
    Duration,
}

/// A date or time's text as a value: one that names no date is `#VALUE!`, not a broken line.
fn stamp_or_error(stamp: Option<Stamp>) -> Parsed {
    stamp.map_or(Parsed::Value(Value::Error(CellError::Value)), Parsed::Stamp)
}

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Parsed;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("null, a number, a string, a boolean, or an error, array or object")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Parsed, E> {
        Ok(Parsed::Value(Value::Empty))
    }
    fn visit_none<E: de::Error>(self) -> Result<Parsed, E> {
        Ok(Parsed::Value(Value::Empty))
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Parsed, E> {
        Ok(Parsed::Value(Value::Bool(v)))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Parsed, E> {
        Ok(Parsed::Value(Value::Number(v as f64)))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Parsed, E> {
        Ok(Parsed::Value(Value::Number(v as f64)))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Parsed, E> {
        if !v.is_finite() {
            return Err(E::custom("a number that isn't finite"));
        }
        Ok(Parsed::Value(Value::Number(v)))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Parsed, E> {
        if v.len() > MAX_TEXT_BYTES {
            return Err(E::custom(format!("a text of {} bytes, over the cap of {MAX_TEXT_BYTES}", v.len())));
        }
        Ok(Parsed::Value(Value::text(v)))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Parsed, A::Error> {
        let key: ValueKey = map.next_key()?.ok_or_else(|| de::Error::custom("an empty object"))?;
        let out = match key {
            ValueKey::Error => {
                let text: Capped<MAX_ERROR_TEXT_BYTES> = map.next_value()?;
                let e = CellError::ALL.iter().find(|e| e.as_str() == text.0).copied();
                Parsed::Value(Value::Error(e.ok_or_else(|| de::Error::custom("an unknown error value"))?))
            }
            ValueKey::Date => stamp_or_error(stamp::Day::parse(&map.next_value::<Capped<MAX_DATE_BYTES>>()?.0).map(Stamp::Date)),
            ValueKey::DateTime => stamp_or_error(stamp::parse_date_time(&map.next_value::<Capped<MAX_DATE_BYTES>>()?.0)),
            ValueKey::Time => stamp_or_error(stamp::Clock::parse(&map.next_value::<Capped<MAX_DATE_BYTES>>()?.0).map(Stamp::Time)),
            ValueKey::Duration => Parsed::Stamp(Stamp::Duration(map.next_value()?)),
            ValueKey::Array if self.place != Place::Element => map.next_value_seed(ArraySeed)?,
            ValueKey::Object if self.place == Place::Result => {
                let o: RawObject = map.next_value()?;
                if o.preview.0.chars().count() > MAX_PREVIEW_CHARS {
                    return Err(de::Error::custom(format!("a preview over {MAX_PREVIEW_CHARS} characters")));
                }
                Parsed::Object { type_name: o.type_name.0, preview: o.preview.0 }
            }
            ValueKey::Array => return Err(de::Error::custom("an array inside an array")),
            ValueKey::Object => return Err(de::Error::custom("an object where a cell value goes")),
        };
        if map.next_key::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom("a value object with more than one key"));
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawObject {
    type_name: Capped<MAX_TYPE_NAME_BYTES>,
    // A preview of 255 characters is at most 1,020 bytes.
    preview: Capped<{ MAX_PREVIEW_CHARS * 4 }>,
}

/// `{"rows":R,"cols":C,"data":[…]}`, with the shape checked against the data.
struct ArraySeed;

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "camelCase")]
enum ArrayKey {
    Rows,
    Cols,
    Data,
}

impl<'de> DeserializeSeed<'de> for ArraySeed {
    type Value = Parsed;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Parsed, D::Error> {
        d.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for ArraySeed {
    type Value = Parsed;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an array with rows, cols and data")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Parsed, A::Error> {
        let (mut rows, mut cols, mut data): (Option<u64>, Option<u64>, Option<RawData>) = (None, None, None);
        while let Some(key) = map.next_key::<ArrayKey>()? {
            match key {
                ArrayKey::Rows if rows.is_none() => rows = Some(map.next_value()?),
                ArrayKey::Cols if cols.is_none() => cols = Some(map.next_value()?),
                ArrayKey::Data if data.is_none() => {
                    // With the shape already read, the data may not run past it.
                    let expected = rows.zip(cols).and_then(|(r, c)| r.checked_mul(c)).and_then(|n| usize::try_from(n).ok());
                    data = Some(map.next_value_seed(DataSeed { expected })?);
                }
                _ => return Err(de::Error::custom("a repeated array field")),
            }
        }
        let (Some(rows), Some(cols), Some((data, stamps))) = (rows, cols, data) else {
            return Err(de::Error::custom("an array without rows, cols and data"));
        };
        let (Ok(r), Ok(c)) = (usize::try_from(rows), usize::try_from(cols)) else {
            return Err(de::Error::custom("an array shape too large"));
        };
        if r.checked_mul(c).is_none_or(|n| n > MAX_ARRAY_CELLS) {
            return Err(de::Error::custom(format!("an array of more than {MAX_ARRAY_CELLS} cells")));
        }
        // Array::new refuses an empty shape and one that doesn't match the data.
        let value = Array::new(r, c, data).map(Value::from).ok_or_else(|| de::Error::custom("an array whose shape doesn't match its data"))?;
        Ok(if stamps.is_empty() { Parsed::Value(value) } else { Parsed::Dated(Dated { value, stamps }) })
    }
}

struct DataSeed {
    expected: Option<usize>,
}

/// An array's data, and the stamps among it.
type RawData = (Vec<Value>, Vec<(u32, Stamp)>);

impl<'de> DeserializeSeed<'de> for DataSeed {
    type Value = RawData;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<RawData, D::Error> {
        d.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for DataSeed {
    type Value = RawData;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a list of scalar values")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<RawData, A::Error> {
        let limit = self.expected.unwrap_or(MAX_ARRAY_CELLS).min(MAX_ARRAY_CELLS);
        // The size hint comes from the input: trust it only up to a small first allocation.
        let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(limit).min(4096));
        let mut stamps = Vec::new();
        while let Some(element) = seq.next_element_seed(ElementSeed)? {
            if out.len() >= limit {
                return Err(de::Error::custom(format!("array data longer than its shape, or over {MAX_ARRAY_CELLS} cells")));
            }
            match element {
                Parsed::Value(v) => out.push(v),
                Parsed::Stamp(s) => {
                    // The limit keeps this within a u32.
                    stamps.push((u32::try_from(out.len()).map_err(de::Error::custom)?, s));
                    out.push(Value::Empty);
                }
                Parsed::Dated(_) | Parsed::Object { .. } => return Err(de::Error::custom("an array or object inside an array")),
            }
        }
        Ok((out, stamps))
    }
}

struct ElementSeed;

impl<'de> DeserializeSeed<'de> for ElementSeed {
    type Value = Parsed;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Parsed, D::Error> {
        d.deserialize_any(ValueVisitor { place: Place::Element })
    }
}

// ---- Writing -------------------------------------------------------------------------------

/// A cell value in its wire form.
struct WireValue<'a> {
    value: &'a Value,
    /// Inside an array, where a nested array has no form.
    element: bool,
}

impl Serialize for WireValue<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.value {
            Value::Empty => s.serialize_unit(),
            Value::Number(n) if n.is_finite() => s.serialize_f64(*n),
            Value::Number(_) => error_value(s, CellError::Num),
            Value::Text(t) => s.serialize_str(t),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Error(e) => error_value(s, *e),
            Value::Array(_) if self.element => error_value(s, CellError::Value),
            Value::Array(a) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("array", &WireArray(a))?;
                m.end()
            }
        }
    }
}

fn error_value<S: Serializer>(s: S, e: CellError) -> Result<S::Ok, S::Error> {
    let mut m = s.serialize_map(Some(1))?;
    m.serialize_entry("error", e.as_str())?;
    m.end()
}

struct WireArray<'a>(&'a Array);

impl Serialize for WireArray<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        WireStampedArray { array: self.0, stamps: &[] }.serialize(s)
    }
}

/// An array with the stamps among its data.
struct WireStampedArray<'a> {
    array: &'a Array,
    stamps: &'a [(u32, Stamp)],
}

impl Serialize for WireStampedArray<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(3))?;
        m.serialize_entry("rows", &self.array.rows)?;
        m.serialize_entry("cols", &self.array.cols)?;
        m.serialize_entry("data", &WireData { data: &self.array.data, stamps: self.stamps })?;
        m.end()
    }
}

struct WireData<'a> {
    data: &'a [Value],
    stamps: &'a [(u32, Stamp)],
}

impl Serialize for WireData<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.data.len()))?;
        let mut stamps = self.stamps.iter().peekable();
        for (i, v) in self.data.iter().enumerate() {
            match stamps.next_if(|(at, _)| *at as usize == i) {
                Some((_, stamp)) => seq.serialize_element(stamp)?,
                None => seq.serialize_element(&WireValue { value: v, element: true })?,
            }
        }
        seq.end()
    }
}

/// A value with its stamps.
struct WireDated<'a> {
    value: &'a Value,
    stamps: &'a [(u32, Stamp)],
}

impl Serialize for WireDated<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match (self.value, self.stamps.first()) {
            (Value::Array(a), _) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("array", &WireStampedArray { array: a, stamps: self.stamps })?;
                m.end()
            }
            (_, Some((_, stamp))) => stamp.serialize(s),
            (value, None) => WireValue { value, element: false }.serialize(s),
        }
    }
}

struct WireInputs<'a>(&'a BTreeMap<String, Input>);

impl Serialize for WireInputs<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (k, input) in self.0 {
            m.serialize_entry(k, &WireInput(input))?;
        }
        m.end()
    }
}

struct WireInput<'a>(&'a Input);

impl Serialize for WireInput<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("value", &WireDated { value: &self.0.value, stamps: &self.0.stamps })?;
        if let Some(h) = self.0.headers {
            m.serialize_entry("headers", &h)?;
        }
        m.end()
    }
}

struct WireRequest<'a>(&'a Request);

impl Serialize for WireRequest<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        match self.0 {
            Request::Run { id, code, inputs, object, init } => {
                m.serialize_entry("type", "run")?;
                m.serialize_entry("id", id)?;
                m.serialize_entry("code", code)?;
                m.serialize_entry("inputs", &WireInputs(inputs))?;
                if *object {
                    m.serialize_entry("output", "object")?;
                }
                if let Some(init) = init {
                    m.serialize_entry("init", init.id())?;
                }
            }
            Request::Reset { id } => {
                m.serialize_entry("type", "reset")?;
                m.serialize_entry("id", id)?;
            }
        }
        m.end()
    }
}

struct WireReply<'a>(&'a Reply);

impl Serialize for WireReply<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        match self.0 {
            Reply::Ready { self_test } => {
                m.serialize_entry("type", "ready")?;
                m.serialize_entry("selfTest", &WireSelfTest(self_test))?;
            }
            Reply::Done { id, result, stdout, peak } => {
                m.serialize_entry("type", "done")?;
                m.serialize_entry("id", id)?;
                match result {
                    Ok(out) => {
                        m.serialize_entry("ok", &true)?;
                        m.serialize_entry("value", &WireOutput(out))?;
                    }
                    Err(e) => {
                        m.serialize_entry("ok", &false)?;
                        m.serialize_entry("error", &WireError(e))?;
                    }
                }
                if !stdout.is_empty() {
                    m.serialize_entry("stdout", stdout)?;
                }
                if let Some(peak) = peak {
                    m.serialize_entry("peak", peak)?;
                }
            }
        }
        m.end()
    }
}

struct WireOutput<'a>(&'a Output);

impl Serialize for WireOutput<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Output::Value(v) => WireValue { value: v, element: false }.serialize(s),
            Output::Dated(d) => WireDated { value: &d.value, stamps: &d.stamps }.serialize(s),
            Output::Object { type_name, preview } => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("object", &serde_json::json!({"typeName": type_name, "preview": preview}))?;
                m.end()
            }
        }
    }
}

struct WireError<'a>(&'a PyError);

impl Serialize for WireError<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("message", &self.0.message)?;
        if let Some(line) = self.0.line {
            m.serialize_entry("line", &line)?;
        }
        if let Some(kind) = &self.0.kind {
            m.serialize_entry("kind", kind)?;
        }
        if let Some(subject) = &self.0.subject {
            m.serialize_entry("subject", subject)?;
        }
        m.end()
    }
}

struct WireSelfTest<'a>(&'a SelfTest);

impl Serialize for WireSelfTest<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        match self.0 {
            SelfTest::NotChecked => m.serialize_entry("status", "notChecked")?,
            SelfTest::Passed => m.serialize_entry("status", "passed")?,
            SelfTest::Failed(detail) => {
                m.serialize_entry("status", "failed")?;
                m.serialize_entry("detail", detail)?;
            }
        }
        m.end()
    }
}

#[cfg(test)]
mod tests;
