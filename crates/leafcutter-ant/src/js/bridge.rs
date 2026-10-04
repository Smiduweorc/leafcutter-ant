//! Values crossing between the compiler and the engine.
//!
//! JSON values go in as the objects `JSON.parse` would make of them, built
//! without recursion. Script values come out as file data: `undefined` is
//! nothing, the other primitives become JSON values, and objects, functions,
//! symbols and BigInts stay in the engine as handles, so the next script to
//! see the data gets the same object, as in TS Dash. Where the compiler needs
//! JSON from a handle, it gets what `JSON.stringify` makes of it (`toJSON`,
//! boxed primitives unwrapped, `undefined` and functions left out of objects
//! and written as `null` in arrays, cycles and BigInts refused), computed
//! without recursion too.
//!
//! Strings are UTF-16 in the engine. A lone surrogate coming out of it
//! becomes U+FFFD, as it does in json5's strings.

use rquickjs::function::This;
use rquickjs::{Ctx, Function, Object, Persistent, Value as JsValue};

use crate::json::{Value, parse_json_text};
use crate::plugin::Data;

use super::engine::caught;

/// A JavaScript string as a Rust string.
pub(crate) fn string<'js>(ctx: &Ctx<'js>, s: rquickjs::String<'js>) -> rquickjs::Result<String> {
	if let Ok(text) = s.to_string() {
		return Ok(text);
	}
	let well_formed: Function = ctx
		.globals()
		.get::<_, Object>("String")?
		.get::<_, Object>("prototype")?
		.get("toWellFormed")?;
	well_formed
		.call::<_, rquickjs::String>((This(s),))?
		.to_string()
}

/// Bytes the layer handed over as a string of Latin-1 characters, one per
/// byte (a Uint8Array cannot be read without unsafe code).
pub(crate) fn latin1(text: &str) -> Vec<u8> {
	text.chars().map(|c| c as u8).collect()
}

/// A container being filled: the engine's copy and what is left of the
/// source.
enum Open<'a, 'js> {
	Array(
		rquickjs::Array<'js>,
		std::iter::Enumerate<std::slice::Iter<'a, Value>>,
	),
	Object(Object<'js>, crate::json::Iter<'a>),
}

/// A value as the engine's value; a container comes back empty, with what
/// is left to fill it with.
fn open<'a, 'js>(
	ctx: &Ctx<'js>,
	value: &'a Value,
) -> rquickjs::Result<(JsValue<'js>, Option<Open<'a, 'js>>)> {
	Ok(match value {
		Value::Null => (JsValue::new_null(ctx.clone()), None),
		Value::Bool(b) => (JsValue::new_bool(ctx.clone(), *b), None),
		Value::Number(n) => (JsValue::new_number(ctx.clone(), *n), None),
		Value::String(s) => (
			rquickjs::String::from_str(ctx.clone(), s)?.into_value(),
			None,
		),
		Value::Array(items) => {
			let array = rquickjs::Array::new(ctx.clone())?;
			(
				array.clone().into_value(),
				Some(Open::Array(array, items.iter().enumerate())),
			)
		}
		Value::Object(object) => {
			let target = Object::new(ctx.clone())?;
			(
				target.clone().into_value(),
				Some(Open::Object(target, object.iter())),
			)
		}
	})
}

/// A JSON value as the engine's object, array or primitive.
pub(crate) fn to_js<'js>(ctx: &Ctx<'js>, value: &Value) -> rquickjs::Result<JsValue<'js>> {
	let (root, first) = open(ctx, value)?;
	let mut stack: Vec<Open<'_, 'js>> = first.into_iter().collect();
	while let Some(top) = stack.last_mut() {
		match top {
			Open::Array(array, items) => {
				let Some((i, item)) = items.next() else {
					stack.pop();
					continue;
				};
				let (js, child) = open(ctx, item)?;
				array.set(i, js)?;
				stack.extend(child);
			}
			Open::Object(object, entries) => {
				let Some((key, item)) = entries.next() else {
					stack.pop();
					continue;
				};
				let (js, child) = open(ctx, item)?;
				if key == "__proto__" {
					// An own property, as JSON.parse makes it; assigning it
					// would set the prototype instead.
					let define: Function = ctx
						.globals()
						.get::<_, Object>("Object")?
						.get("defineProperty")?;
					let descriptor = Object::new(ctx.clone())?;
					descriptor.set("value", js)?;
					descriptor.set("writable", true)?;
					descriptor.set("enumerable", true)?;
					descriptor.set("configurable", true)?;
					define.call::<_, JsValue>((object.clone(), key, descriptor))?;
				} else {
					object.set(key, js)?;
				}
				stack.extend(child);
			}
		}
	}
	Ok(root)
}

/// A script's value as file data: `None` for `undefined`.
pub(crate) fn to_data<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<Option<Data>> {
	if value.is_undefined() {
		return Ok(None);
	}
	if value.is_null() {
		return Ok(Some(Data::Value(Value::Null)));
	}
	if let Some(b) = value.as_bool() {
		return Ok(Some(Data::Value(Value::Bool(b))));
	}
	if let Some(n) = value.as_number() {
		return Ok(Some(Data::Value(Value::Number(n))));
	}
	if let Some(s) = value.as_string() {
		return Ok(Some(Data::Value(Value::String(string(ctx, s.clone())?))));
	}
	Ok(Some(Data::Js(Persistent::save(ctx, value))))
}

/// File data as the value a script gets.
pub(crate) fn data_to_js<'js>(ctx: &Ctx<'js>, data: &Data) -> rquickjs::Result<JsValue<'js>> {
	match data {
		Data::Value(value) => to_js(ctx, value),
		Data::Shared(value) => to_js(ctx, &value.borrow()),
		Data::Js(handle) => handle.clone().restore(ctx),
	}
}

/// What `JSON.stringify(value)` makes of a script's value, read back as
/// JSON: `None` where it gives `undefined`, and the text of the TypeError it
/// throws for a cycle or a BigInt.
pub(crate) fn to_value<'js>(
	ctx: &Ctx<'js>,
	layer: &Object<'js>,
	value: JsValue<'js>,
) -> Result<Option<Value>, String> {
	let text = json_text(ctx, layer, value)?;
	text.map(|text| {
		parse_json_text(&text).map_err(|error| format!("SyntaxError: {error} in written JSON"))
	})
	.transpose()
}

/// `JSON.stringify(value)`, compact.
pub(crate) fn json_text<'js>(
	ctx: &Ctx<'js>,
	layer: &Object<'js>,
	value: JsValue<'js>,
) -> Result<Option<String>, String> {
	let to_json: Function = layer.get("toJsonText").map_err(|e| caught(ctx, e))?;
	let text: JsValue = to_json.call((value,)).map_err(|e| caught(ctx, e))?;
	match text.as_string() {
		Some(s) => string(ctx, s.clone()).map(Some).map_err(|e| caught(ctx, e)),
		None => Ok(None),
	}
}

#[cfg(test)]
mod tests {
	use rquickjs::{Function, Object, Persistent, Value as JsValue};

	use super::*;
	use crate::json::{Indent, parse_json5, stringify};
	use crate::plugin::Output;
	use crate::testing::{MemoryFs, dash_with, vectors};

	/// Evaluates `source` in a fresh compiler's engine and hands the value to
	/// `f` with the layer.
	fn with_value<R>(
		source: &str,
		f: impl for<'js> FnOnce(&Ctx<'js>, &Object<'js>, JsValue<'js>) -> R,
	) -> R {
		let (dash, _) = dash_with(MemoryFs::with(&[]), "[]");
		dash.compiler().cx.engine.with(|ctx, layer| {
			let value: JsValue = ctx.eval(source).expect("the source runs");
			f(&ctx, &layer, value)
		})
	}

	fn json(source: &str) -> Value {
		parse_json5(source).expect("json5")
	}

	/// A conversion's result with the JSON as compact text, to compare.
	fn text(result: Result<Option<Value>, String>) -> Result<Option<String>, String> {
		result.map(|value| value.map(|value| stringify(&value, Indent::None)))
	}

	fn compact(source: &str) -> Result<Option<String>, String> {
		Ok(Some(stringify(&json(source), Indent::None)))
	}

	#[test]
	fn quickjs_writes_numbers_and_documents_as_v8_does() {
		// User scripts call JSON.stringify and Number#toString themselves,
		// so the engine's own must match the V8 vectors.
		let (dash, _) = dash_with(MemoryFs::with(&[]), "[]");
		let numbers = vectors("numbers.json");
		let documents = vectors("stringify.json");
		let failures = dash.compiler().cx.engine.with(|ctx, _| {
			let stringify: Function = ctx
				.eval("(v, i) => JSON.stringify(JSON.parse(v), null, i)")
				.expect("compiles");
			let to_string: Function = ctx.eval("(n) => String(n)").expect("compiles");
			let mut failures = Vec::new();
			for case in numbers.as_array().expect("an array") {
				let bits = case[0].as_str().expect("hex bits");
				let n = f64::from_bits(u64::from_str_radix(bits, 16).expect("hex"));
				let text: String = to_string.call((n,)).expect("String(n)");
				// JSON.stringify writes non-finite numbers as null, String as
				// their names.
				let expected = case[1].as_str().expect("text");
				if n.is_finite() && text != expected {
					failures.push(format!("{bits}: V8 {expected}, QuickJS {text}"));
				}
			}
			for case in documents.as_array().expect("an array") {
				let source = case[0].as_str().expect("source");
				for (indent, expected) in [("", &case[1]), ("\t", &case[2])] {
					let text: String = stringify.call((source, indent)).expect("stringify");
					if text != expected.as_str().expect("text") {
						failures.push(format!("{source}: V8 {expected}, QuickJS {text}"));
					}
				}
			}
			failures
		});
		assert!(failures.is_empty(), "{}", failures.join("\n"));
	}

	#[test]
	fn script_values_become_json_as_json_stringify_writes_them() {
		let value = with_value(
			"({ u: undefined, f() {}, s: Symbol('x'), list: [undefined, () => 1, Symbol('y'), NaN, -0, Infinity], n: new Number(3), t: new String('t'), b: Object(false), j: { toJSON(key) { return 'key ' + key } }, d: new Date(0), m: new Map([[1, 2]]), nested: { a: [{ b: null }] } })",
			to_value,
		);
		assert_eq!(
			text(value),
			compact(
				r#"{"list": [null, null, null, null, 0, null], "n": 3, "t": "t", "b": false, "j": "key j", "d": "1970-01-01T00:00:00.000Z", "m": {}, "nested": {"a": [{"b": null}]}}"#
			)
		);
		for source in ["undefined", "(() => 1)", "Symbol('z')"] {
			assert_eq!(text(with_value(source, to_value)), Ok(None), "{source}");
		}
	}

	#[test]
	fn cycles_and_bigints_are_the_type_errors_json_stringify_throws() {
		let cycle = with_value("const a = { b: {} }; a.b.a = a; a", to_value);
		assert_eq!(
			text(cycle),
			Err("TypeError: Converting circular structure to JSON".to_owned())
		);
		let bigint = with_value("({ n: 10n })", to_value);
		assert_eq!(
			text(bigint),
			Err("TypeError: Do not know how to serialize a BigInt".to_owned())
		);
		// The same object twice is no cycle.
		let shared = with_value("const s = [1]; [s, s]", to_value);
		assert_eq!(text(shared), compact("[[1], [1]]"));
	}

	#[test]
	fn values_nested_a_hundred_thousand_deep_cross_without_recursion() {
		let depth = 100_000;
		let deep = with_value(
			&format!(
				"let v = 1; for (let i = 0; i < {depth}; i++) v = i % 2 ? [v] : {{ k: v }}; v"
			),
			|ctx, layer, value| {
				let back = to_value(ctx, layer, value)
					.expect("converted")
					.expect("a value");
				let again = to_js(ctx, &back).expect("built");
				to_value(ctx, layer, again)
			},
		);
		let mut value = deep.expect("converted").expect("a value");
		let mut levels = 0;
		loop {
			value = match value {
				Value::Array(mut items) => std::mem::replace(&mut items[0], Value::Null),
				Value::Object(mut object) => object.remove("k").expect("k"),
				_ => break,
			};
			levels += 1;
		}
		assert_eq!(levels, depth);
		assert!(matches!(value, Value::Number(n) if n == 1.0));
	}

	#[test]
	fn a_proto_key_stays_an_own_property_both_ways() {
		let mut object = crate::json::Object::new();
		object.insert("__proto__".to_owned(), json(r#"{"a": 1}"#));
		object.insert("b".to_owned(), Value::Bool(true));
		let with_proto = Value::Object(object);
		let (dash, _) = dash_with(MemoryFs::with(&[]), "[]");
		let (own, back) = dash.compiler().cx.engine.with(|ctx, layer| {
			let js = to_js(&ctx, &with_proto).expect("built");
			let own: Function = ctx
				.eval(
					"(o) => Object.keys(o).join() + ' ' + (Object.getPrototypeOf(o) === Object.prototype)",
				)
				.expect("compiles");
			let own: String = own.call((js.clone(),)).expect("called");
			(own, to_value(&ctx, &layer, js))
		});
		assert_eq!(own, "__proto__,b true");
		assert_eq!(text(back), Ok(Some(stringify(&with_proto, Indent::None))));
	}

	#[test]
	fn lone_surrogates_come_out_as_replacement_characters() {
		let result = with_value("'a\\uD800b'", |ctx, layer, value| {
			let data = to_data(ctx, value.clone()).expect("converted");
			let json = to_value(ctx, layer, value);
			(
				matches!(data, Some(Data::Value(Value::String(s))) if s == "a\u{fffd}b"),
				json,
			)
		});
		assert!(result.0);
		// JSON.stringify escapes a lone surrogate, which reads back as U+FFFD.
		assert_eq!(text(result.1), Ok(Some("\"a\u{fffd}b\"".to_owned())));
	}

	#[test]
	fn written_data_is_text_bytes_or_nothing_as_ts_dash_writes_it() {
		let (dash, _) = dash_with(MemoryFs::with(&[]), "[]");
		let engine = &dash.compiler().cx.engine;
		let cases = [
			(
				"new Uint8Array([104, 105, 0, 255])",
				Some(vec![104, 105, 0, 255]),
			),
			(
				"new Uint16Array([0x6968]).subarray(0)",
				Some(vec![0x68, 0x69]),
			),
			("'caf\\u00e9'", Some("caf\u{e9}".as_bytes().to_vec())),
			("({ a: [1, 'x'] })", Some(br#"{"a":[1,"x"]}"#.to_vec())),
			("new Blob(['x'])", None),
			("new File(['x'], 'x.txt')", None),
			("new ArrayBuffer(2)", None),
			("(() => 1)", None),
		];
		for (source, expected) in cases {
			let handle = engine.with(|ctx, _| {
				let value: JsValue = ctx.eval(source).expect("runs");
				Persistent::save(&ctx, value)
			});
			let output = engine.output(&handle).expect("no TypeError");
			let actual = match output {
				Output::Bytes(bytes) => Some(bytes),
				Output::Nothing => None,
			};
			assert_eq!(actual, expected, "{source}");
		}
	}

	#[test]
	fn json_values_cross_into_the_engine_as_json_parse_makes_them() {
		let source = r#"{"z": 1, "a": [true, null, "s", 2.5, {"1": 1, "0": 0}], "e": {}}"#;
		let (dash, _) = dash_with(MemoryFs::with(&[]), "[]");
		let text = dash.compiler().cx.engine.with(|ctx, _| {
			let js = to_js(&ctx, &json(source)).expect("built");
			let stringify: Function = ctx.eval("(v) => JSON.stringify(v)").expect("compiles");
			stringify.call::<_, String>((js,)).expect("called")
		});
		assert_eq!(text, stringify(&json(source), Indent::None));
	}
}
