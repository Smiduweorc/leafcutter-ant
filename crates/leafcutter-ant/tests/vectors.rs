//! Compares the crate, through its public API, with golden vectors that
//! tools/parity/vectors.mjs records from V8 and json5 2.2.1, the JavaScript
//! TS Dash 0.13.0 runs.

use leafcutter_ant::json::{Array, Indent, Object, Value, parse_json5, stringify};

fn vectors(name: &str) -> Vec<Vec<String>> {
	let path = format!("{}/tests/vectors/{name}", env!("CARGO_MANIFEST_DIR"));
	let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
	serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Builds a value by inserting keys in the order the JSON text lists them,
/// which is what JSON.parse does.
fn from_serde(value: serde_json::Value) -> Value {
	match value {
		serde_json::Value::Null => Value::Null,
		serde_json::Value::Bool(b) => Value::Bool(b),
		serde_json::Value::Number(n) => {
			Value::Number(n.as_f64().expect("every JSON number has an f64 value"))
		}
		serde_json::Value::String(s) => Value::String(s),
		serde_json::Value::Array(items) => Value::Array(Array::from(
			items.into_iter().map(from_serde).collect::<Vec<_>>(),
		)),
		serde_json::Value::Object(entries) => {
			let mut object = Object::new();
			for (key, value) in entries {
				object.insert(key, from_serde(value));
			}
			Value::Object(object)
		}
	}
}

#[test]
fn numbers_are_written_as_v8_writes_them() {
	let cases = vectors("numbers.json");
	assert!(cases.len() > 6000);
	let mut failures = Vec::new();
	for case in &cases {
		let [bits, expected] = case.as_slice() else {
			panic!("bad vector {case:?}")
		};
		let n = f64::from_bits(u64::from_str_radix(bits, 16).expect("hex bits"));
		let actual = stringify(&Value::Number(n), Indent::None);
		if &actual != expected {
			failures.push(format!("{bits}: expected {expected}, got {actual}"));
		}
	}
	assert!(
		failures.is_empty(),
		"{} of {} differ:\n{}",
		failures.len(),
		cases.len(),
		failures.join("\n")
	);
}

#[test]
fn documents_are_written_as_v8_writes_them_in_both_layouts() {
	let cases = vectors("stringify.json");
	assert!(cases.len() > 1500);
	let mut failures = Vec::new();
	for case in &cases {
		let [source, compact, tab] = case.as_slice() else {
			panic!("bad vector {case:?}")
		};
		let value = from_serde(serde_json::from_str(source).expect("the source is JSON"));
		let actual_compact = stringify(&value, Indent::None);
		let actual_tab = stringify(&value, Indent::Tab);
		if &actual_compact != compact || &actual_tab != tab {
			failures.push(format!(
				"{source}\n  expected {compact}\n  got      {actual_compact}"
			));
		}
	}
	assert!(
		failures.is_empty(),
		"{} of {} differ:\n{}",
		failures.len(),
		cases.len(),
		failures.join("\n")
	);
}

#[test]
fn json5_reads_and_refuses_what_json5_2_2_1_does() {
	let cases = vectors("json5.json");
	assert!(cases.len() > 3000);
	let mut failures = Vec::new();
	for case in &cases {
		let [source, outcome, expected] = case.as_slice() else {
			panic!("bad vector {case:?}")
		};
		let actual = match parse_json5(source) {
			Ok(value) => ("ok", stringify(&value, Indent::None)),
			Err(error) => ("error", error.to_string()),
		};
		if actual.0 != outcome || &actual.1 != expected {
			failures.push(format!(
				"{source:?}\n  expected {outcome} {expected}\n  got      {} {}",
				actual.0, actual.1
			));
		}
	}
	assert!(
		failures.is_empty(),
		"{} of {} differ:\n{}",
		failures.len(),
		cases.len(),
		failures.join("\n")
	);
}
