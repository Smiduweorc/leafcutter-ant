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

/// Scripts stringify their own output (a generator script may return
/// `JSON.stringify(data)`), so the engine that runs them has to write numbers
/// and documents as V8 does. QuickJS does, on every vector.
#[test]
fn quickjs_writes_numbers_and_documents_as_v8_writes_them() {
	let runtime = rquickjs::Runtime::new().expect("a QuickJS runtime");
	let context = rquickjs::Context::full(&runtime).expect("a QuickJS context");
	let numbers = vectors("numbers.json");
	let documents = vectors("stringify.json");
	let mut failures = Vec::new();
	context.with(|ctx| {
		let number: rquickjs::Function = ctx
			.eval("(n) => [JSON.stringify(n), String(n)]")
			.expect("the number probe compiles");
		for case in &numbers {
			let n = f64::from_bits(u64::from_str_radix(&case[0], 16).expect("hex bits"));
			let [json, string]: [String; 2] = number
				.call::<_, Vec<String>>((n,))
				.expect("the probe runs")
				.try_into()
				.expect("two strings");
			// JSON.stringify writes the non-finite numbers as null, and String
			// writes them by name.
			let expected_string = match n {
				n if n.is_nan() => "NaN",
				f64::INFINITY => "Infinity",
				f64::NEG_INFINITY => "-Infinity",
				_ => &case[1],
			};
			if json != case[1] || string != expected_string {
				failures.push(format!("{}: V8 {}, QuickJS {json} and {string}", case[0], case[1]));
			}
		}
		let document: rquickjs::Function = ctx
			.eval("(s) => { const v = JSON.parse(s); return [JSON.stringify(v), JSON.stringify(v, null, '\\t')] }")
			.expect("the document probe compiles");
		for case in &documents {
			let written: Vec<String> = document.call((case[0].clone(),)).expect("the probe runs");
			if written[0] != case[1] || written[1] != case[2] {
				failures.push(format!("{}\n  V8      {}\n  QuickJS {}", case[0], case[1], written[0]));
			}
		}
	});
	assert!(numbers.len() > 6000 && documents.len() > 1500);
	assert!(
		failures.is_empty(),
		"{} of {} differ:\n{}",
		failures.len(),
		numbers.len() + documents.len(),
		failures.join("\n")
	);
}
