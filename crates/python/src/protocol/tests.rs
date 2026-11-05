use super::*;

fn array(rows: usize, cols: usize, data: Vec<Value>) -> Value {
    Array::new(rows, cols, data).map(Value::from).unwrap()
}

fn done_line(value_json: &str) -> String {
    format!(r#"{{"type":"done","id":1,"ok":true,"value":{value_json}}}"#)
}

fn value_of(line: &str) -> Result<Output, String> {
    match Reply::parse(line.as_bytes())? {
        Reply::Done { result: Ok(out), .. } => Ok(out),
        other => Err(format!("not a value: {other:?}")),
    }
}

#[test]
fn requests_and_replies_round_trip() {
    let mut inputs = BTreeMap::new();
    inputs.insert("A1".to_string(), Input::new(Value::Number(2.5)));
    inputs.insert(
        "Sheet 2!B1:C2".to_string(),
        Input {
            value: array(2, 2, vec![Value::text("é\"\n"), Value::Bool(true), Value::Empty, Value::Error(CellError::Connect)]),
            headers: Some(true),
        },
    );
    let run = Request::Run { id: 7, object: false, code: "x = xl(\"A1\")\nx * 2".into(), inputs };
    let line = run.to_line();
    assert!(line.ends_with(b"\n") && !line[..line.len() - 1].contains(&b'\n'), "one line");
    assert_eq!(Request::parse(&line[..line.len() - 1]).unwrap(), run);
    let reset = Request::Reset { id: 8 };
    assert_eq!(Request::parse(reset.to_line().trim_ascii_end()).unwrap(), reset);
    assert_eq!(String::from_utf8(reset.to_line()).unwrap(), "{\"type\":\"reset\",\"id\":8}\n");

    for reply in [
        Reply::Ready { self_test: SelfTest::NotChecked },
        Reply::Ready { self_test: SelfTest::Failed("socket opened".into()) },
        Reply::Done { id: 3, result: Ok(Output::Value(array(1, 2, vec![Value::Number(1.0), Value::text("b")]))), stdout: "hi\n".into(), peak: None },
        Reply::Done {
            id: 4,
            result: Ok(Output::Object { type_name: "dict".into(), preview: "{'a': 1}".into() }),
            stdout: String::new(),
            peak: Some(123),
        },
        Reply::Done {
            id: 5,
            result: Err(PyError { message: "NameError: name 'y' is not defined".into(), line: Some(2), ..PyError::default() }),
            stdout: String::new(),
            peak: Some(456),
        },
        Reply::Done {
            id: 6,
            result: Err(PyError { message: "SyntaxError".into(), line: None, ..PyError::default() }),
            stdout: String::new(),
            peak: None,
        },
        Reply::Done { id: 9, result: Ok(Output::Value(Value::Empty)), stdout: String::new(), peak: None },
    ] {
        let line = reply.to_line();
        assert_eq!(Reply::parse(line.trim_ascii_end()).unwrap(), reply);
    }
}

#[test]
fn what_the_shim_writes_parses() {
    assert_eq!(Reply::parse(br#"{"type":"ready","selfTest":{"status":"notChecked"}}"#).unwrap(), Reply::Ready { self_test: SelfTest::NotChecked });
    assert_eq!(value_of(&done_line("3")).unwrap(), Output::Value(Value::Number(3.0)));
    assert_eq!(value_of(&done_line("-1.5e300")).unwrap(), Output::Value(Value::Number(-1.5e300)));
    assert_eq!(value_of(&done_line("null")).unwrap(), Output::Value(Value::Empty));
    assert_eq!(value_of(&done_line(r##"{"error":"#NUM!"}"##)).unwrap(), Output::Value(Value::Error(CellError::Num)));
    assert_eq!(
        value_of(&done_line(r##"{"array":{"rows":2,"cols":1,"data":[true,{"error":"#N/A"}]}}"##)).unwrap(),
        Output::Value(array(2, 1, vec![Value::Bool(true), Value::Error(CellError::NA)]))
    );
    // The shape may come after the data.
    assert_eq!(
        value_of(&done_line(r#"{"array":{"data":[1,2],"cols":2,"rows":1}}"#)).unwrap(),
        Output::Value(array(1, 2, vec![1.0.into(), 2.0.into()]))
    );
}

#[test]
fn array_shapes_are_checked_against_the_data() {
    for bad in [
        r#"{"array":{"rows":2,"cols":2,"data":[1,2,3]}}"#,
        r#"{"array":{"rows":1,"cols":2,"data":[1,2,3]}}"#,
        r#"{"array":{"data":[1,2,3],"rows":2,"cols":2}}"#,
        r#"{"array":{"rows":0,"cols":0,"data":[]}}"#,
        r#"{"array":{"rows":0,"cols":5,"data":[]}}"#,
        r#"{"array":{"rows":1,"cols":1}}"#,
        r#"{"array":{"rows":1,"data":[1]}}"#,
        r#"{"array":{"rows":1,"cols":1,"data":[1],"rows":1}}"#,
        r#"{"array":{"rows":1,"cols":1,"data":[1],"extra":1}}"#,
        r#"{"array":{"rows":-1,"cols":1,"data":[1]}}"#,
        r#"{"array":{"rows":1.5,"cols":1,"data":[1]}}"#,
        // A product that wraps around in 64 bits.
        r#"{"array":{"rows":4294967296,"cols":4294967296,"data":[1]}}"#,
        r#"{"array":{"rows":18446744073709551615,"cols":2,"data":[1]}}"#,
        // Nesting: arrays and objects are not elements.
        r#"{"array":{"rows":1,"cols":1,"data":[{"array":{"rows":1,"cols":1,"data":[1]}}]}}"#,
        r#"{"array":{"rows":1,"cols":1,"data":[{"object":{"typeName":"x","preview":"x"}}]}}"#,
        r#"{"array":{"rows":1,"cols":1,"data":[[1]]}}"#,
        r#"{"array":{"rows":1,"cols":1,"data":{"0":1}}}"#,
    ] {
        assert!(value_of(&done_line(bad)).is_err(), "{bad}");
    }
}

#[test]
fn arrays_over_a_million_cells_are_refused() {
    let cells = |n: usize| vec!["0"; n].join(",");
    let at_cap = done_line(&format!(r#"{{"array":{{"rows":1000,"cols":1000,"data":[{}]}}}}"#, cells(MAX_ARRAY_CELLS)));
    match value_of(&at_cap).unwrap() {
        Output::Value(Value::Array(a)) => assert_eq!(a.data.len(), MAX_ARRAY_CELLS),
        other => panic!("{other:?}"),
    }
    // One more cell, with a shape that matches it, is over the cap.
    let over = done_line(&format!(r#"{{"array":{{"rows":1000001,"cols":1,"data":[{}]}}}}"#, cells(MAX_ARRAY_CELLS + 1)));
    assert!(value_of(&over).unwrap_err().contains("1000000 cells"));
    // Data that runs past a million is stopped while it is read, whatever the shape says.
    let long = done_line(&format!(r#"{{"array":{{"data":[{}],"rows":1,"cols":1}}}}"#, cells(MAX_ARRAY_CELLS + 1)));
    assert!(value_of(&long).is_err());
}

#[test]
fn every_field_has_a_cap() {
    let text = |n: usize| format!("\"{}\"", "x".repeat(n));
    assert!(value_of(&done_line(&text(MAX_TEXT_BYTES))).is_ok());
    assert!(value_of(&done_line(&text(MAX_TEXT_BYTES + 1))).is_err());
    let in_array = format!(r#"{{"array":{{"rows":1,"cols":1,"data":[{}]}}}}"#, text(MAX_TEXT_BYTES + 1));
    assert!(value_of(&done_line(&in_array)).is_err());

    let error = |n: usize| format!(r#"{{"type":"done","id":1,"ok":false,"error":{{"message":"{}","line":1}}}}"#, "e".repeat(n));
    assert!(Reply::parse(error(MAX_MESSAGE_BYTES).as_bytes()).is_ok());
    assert!(Reply::parse(error(MAX_MESSAGE_BYTES + 1).as_bytes()).is_err());

    let object = |name: usize, preview: &str| format!(r#"{{"object":{{"typeName":"{}","preview":"{preview}"}}}}"#, "t".repeat(name));
    assert!(value_of(&done_line(&object(MAX_TYPE_NAME_BYTES, "p"))).is_ok());
    assert!(value_of(&done_line(&object(MAX_TYPE_NAME_BYTES + 1, "p"))).is_err());
    // The preview counts characters: 255 four-byte characters fit, 256 one-byte ones don't.
    assert!(value_of(&done_line(&object(1, &"\u{1F600}".repeat(MAX_PREVIEW_CHARS)))).is_ok());
    assert!(value_of(&done_line(&object(1, &"p".repeat(MAX_PREVIEW_CHARS + 1)))).is_err());

    let stdout = |n: usize| format!(r#"{{"type":"done","id":1,"ok":true,"value":null,"stdout":"{}"}}"#, "o".repeat(n));
    assert!(Reply::parse(stdout(MAX_STDOUT_BYTES).as_bytes()).is_ok());
    assert!(Reply::parse(stdout(MAX_STDOUT_BYTES + 1).as_bytes()).is_err());

    let detail = |n: usize| format!(r#"{{"type":"ready","selfTest":{{"status":"failed","detail":"{}"}}}}"#, "d".repeat(n));
    assert!(Reply::parse(detail(MAX_SELF_TEST_BYTES).as_bytes()).is_ok());
    assert!(Reply::parse(detail(MAX_SELF_TEST_BYTES + 1).as_bytes()).is_err());

    // The line itself.
    let mut long = done_line("null").into_bytes();
    long.resize(MAX_LINE_BYTES + 1, b' ');
    assert!(Reply::parse(&long).unwrap_err().contains("more than"));
}

#[test]
fn malformed_replies_are_refused() {
    for bad in [
        "",
        "not json",
        "[]",
        "{}",
        r#"{"type":"done"}"#,
        r#"{"type":"done","id":1}"#,
        r#"{"type":"done","id":1,"ok":true}"#,
        r#"{"type":"done","id":1,"ok":false}"#,
        r#"{"type":"done","id":1,"ok":true,"value":1,"error":{"message":"x"}}"#,
        r#"{"type":"done","id":1,"ok":false,"value":1,"error":{"message":"x"}}"#,
        r#"{"type":"done","id":1,"ok":true,"value":1,"id":2}"#,
        r#"{"type":"done","id":-1,"ok":true,"value":1}"#,
        r#"{"type":"done","id":1,"ok":true,"value":1,"extend":600000}"#,
        r#"{"type":"done","id":1,"ok":false,"error":{"message":"x","line":-1}}"#,
        r#"{"type":"done","id":1,"ok":false,"error":{"message":"x","line":4294967296}}"#,
        r#"{"type":"done","id":1,"ok":true,"value":1,"selfTest":{"status":"passed"}}"#,
        r#"{"type":"extend","id":1,"ms":600000}"#,
        r#"{"type":"xl","id":1,"reference":"A1"}"#,
        r#"{"type":"ready"}"#,
        r#"{"type":"ready","id":1,"selfTest":{"status":"notChecked"}}"#,
        r#"{"type":"ready","selfTest":{"status":"passed","detail":"x"}}"#,
        r#"{"type":"ready","selfTest":{"status":"maybe"}}"#,
        r#"{"type":"done","id":1,"ok":true,"value":1} trailing"#,
    ] {
        assert!(Reply::parse(bad.as_bytes()).is_err(), "{bad}");
    }
    for bad_value in [
        r##"{"error":"#NOPE!"}"##,
        r##"{"error":"#CIRC!"}"##,
        r##"{"error":1}"##,
        r##"{"error":"#N/A","array":{}}"##,
        r#"{}"#,
        r#"{"number":1}"#,
        r#"[1,2]"#,
        r#"{"object":{"typeName":"x"}}"#,
        r#"{"object":{"typeName":"x","preview":"y","extra":1}}"#,
    ] {
        assert!(value_of(&done_line(bad_value)).is_err(), "{bad_value}");
    }
    // Not UTF-8.
    assert!(Reply::parse(b"{\"type\":\"done\",\"id\":1,\"ok\":true,\"value\":\"\xff\"}").is_err());
    // A lone surrogate escape isn't text.
    assert!(value_of(&done_line(r#""\ud800""#)).is_err());
}

#[test]
fn inputs_never_carry_objects_and_odd_numbers_become_errors() {
    let bad = br#"{"type":"run","id":1,"code":"","inputs":{"A1":{"value":{"object":{"typeName":"x","preview":"y"}}}}}"#;
    assert!(Request::parse(bad).is_err());
    // A number JSON can't hold is sent as #NUM!, and an array inside an array as #VALUE!.
    let inner = array(1, 1, vec![Value::Number(1.0)]);
    let mut inputs = BTreeMap::new();
    inputs.insert("A1".to_string(), Input::new(Value::Number(f64::NAN)));
    inputs.insert("B1:B2".to_string(), Input::new(array(2, 1, vec![inner, Value::Number(f64::INFINITY)])));
    let line = Request::Run { id: 1, code: String::new(), inputs, object: true }.to_line();
    let Request::Run { inputs, object: true, .. } = Request::parse(line.trim_ascii_end()).unwrap() else { panic!() };
    assert_eq!(inputs["A1"].value, Value::Error(CellError::Num));
    assert_eq!(inputs["B1:B2"].value, array(2, 1, vec![Value::Error(CellError::Value), Value::Error(CellError::Num)]));
}

/// The shim keeps its output within the same caps, so a change on one side must be made on both.
#[test]
fn the_shim_has_the_same_caps() {
    let shim = include_str!("../shim.py");
    for (name, value) in [
        ("MAX_LINE_BYTES", MAX_LINE_BYTES),
        ("MAX_TEXT_BYTES", MAX_TEXT_BYTES),
        ("MAX_MESSAGE_BYTES", MAX_MESSAGE_BYTES),
        ("MAX_TYPE_NAME_BYTES", MAX_TYPE_NAME_BYTES),
        ("MAX_PREVIEW_CHARS", MAX_PREVIEW_CHARS),
        ("MAX_STDOUT_BYTES", MAX_STDOUT_BYTES),
        ("MAX_ARRAY_CELLS", MAX_ARRAY_CELLS),
    ] {
        assert!(shim.contains(&format!("    {name} = {value}\n")), "{name} = {value} in shim.py");
    }
    // And knows every error value.
    for e in CellError::ALL {
        assert!(shim.contains(&format!("\"{}\"", e.as_str())), "{e} in shim.py");
    }
}

#[test]
fn a_peak_is_an_optional_capped_number_on_an_answer() {
    let peak = |line: &str| match Reply::parse(line.as_bytes()) {
        Ok(Reply::Done { peak, .. }) => Ok(peak),
        Ok(other) => panic!("{other:?}"),
        Err(e) => Err(e),
    };
    assert_eq!(peak(r#"{"type":"done","id":1,"ok":true,"value":1}"#), Ok(None));
    assert_eq!(peak(r#"{"type":"done","id":1,"ok":true,"value":1,"peak":1048576}"#), Ok(Some(1_048_576)));
    assert_eq!(peak(&format!(r#"{{"type":"done","id":1,"ok":true,"value":1,"peak":{MAX_PEAK_BYTES}}}"#)), Ok(Some(MAX_PEAK_BYTES)));
    for bad in [
        format!(r#"{{"type":"done","id":1,"ok":true,"value":1,"peak":{}}}"#, MAX_PEAK_BYTES + 1),
        r#"{"type":"done","id":1,"ok":true,"value":1,"peak":-1}"#.to_string(),
        r#"{"type":"done","id":1,"ok":true,"value":1,"peak":1.5}"#.to_string(),
        r#"{"type":"done","id":1,"ok":true,"value":1,"peak":"big"}"#.to_string(),
        r#"{"type":"ready","selfTest":{"status":"notChecked"},"peak":1}"#.to_string(),
    ] {
        assert!(Reply::parse(bad.as_bytes()).is_err(), "{bad}");
    }
}
