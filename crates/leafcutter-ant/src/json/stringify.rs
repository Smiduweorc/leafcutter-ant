use std::fmt::Write as _;

use super::{Array, Iter, Object, Value};

/// The two layouts Dash writes: `JSON.stringify(value)` and
/// `JSON.stringify(value, null, "\t")`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
	/// Everything on one line, with no spaces.
	None,
	/// One element or key per line, indented with tabs, with a space after
	/// each colon. Empty arrays and objects stay `[]` and `{}`.
	Tab,
}

/// Writes `value` byte for byte as V8's `JSON.stringify` does: keys in
/// JavaScript's order, numbers as `Number#toString` formats them (`1`, not
/// `1.0`; `1e+21`; `-0` as `0`), `NaN` and the infinities as `null`, and only
/// `"`, `\` and the control characters escaped.
pub fn stringify(value: &Value, indent: Indent) -> String {
	write(Root::Value(value), indent, NonFinite::Null, &|_| None)
}

/// [`stringify()`] with a replacer that can turn a scalar into a string, as a
/// `JSON.stringify` replacer function that only replaces primitives does.
/// `replace` is called with each scalar, which it can recognise by address.
pub(crate) fn stringify_replacing(
	value: &Value,
	indent: Indent,
	replace: &dyn Fn(&Value) -> Option<String>,
) -> String {
	write(Root::Value(value), indent, NonFinite::Null, replace)
}

/// What is written for `NaN` and the infinities.
#[derive(Clone, Copy)]
pub(super) enum NonFinite {
	/// `null`, as `JSON.stringify` writes them.
	Null,
	/// `NaN`, `Infinity` and `-Infinity`, as `String(n)` writes them.
	Name,
}

/// What [`write`] starts from.
pub(super) enum Root<'a> {
	Value(&'a Value),
	Array(&'a Array),
	Object(&'a Object),
}

pub(super) fn write(
	root: Root<'_>,
	indent: Indent,
	non_finite: NonFinite,
	replace: &dyn Fn(&Value) -> Option<String>,
) -> String {
	let mut out = String::new();
	let mut open: Vec<Open<'_>> = Vec::new();
	open.extend(match root {
		Root::Value(value) => write_value(&mut out, value, non_finite, replace),
		Root::Array(array) => open_array(&mut out, array),
		Root::Object(object) => open_object(&mut out, object),
	});
	let mut next = None;
	loop {
		if let Some(value) = next.take()
			&& let Some(container) = write_value(&mut out, value, non_finite, replace)
		{
			open.push(container);
		}
		let depth = open.len();
		let Some(top) = open.last_mut() else {
			return out;
		};
		let (child, first) = match top {
			Open::Array { items, first } => (items.next().map(|item| (None, item)), first),
			Open::Object { entries, first } => (entries.next().map(|(k, v)| (Some(k), v)), first),
		};
		match child {
			Some((key, item)) => {
				if !*first {
					out.push(',');
				}
				*first = false;
				newline(&mut out, indent, depth);
				if let Some(key) = key {
					quote(&mut out, key);
					out.push(':');
					if indent == Indent::Tab {
						out.push(' ');
					}
				}
				next = Some(item);
			}
			None => {
				let close = match top {
					Open::Array { .. } => ']',
					Open::Object { .. } => '}',
				};
				open.pop();
				newline(&mut out, indent, depth - 1);
				out.push(close);
			}
		}
	}
}

/// A non-empty array or object whose elements are being written.
enum Open<'a> {
	Array {
		items: std::slice::Iter<'a, Value>,
		first: bool,
	},
	Object {
		entries: Iter<'a>,
		first: bool,
	},
}

/// Writes a scalar or an empty container whole, or writes the opening bracket
/// of a non-empty container and returns it to be filled.
fn write_value<'a>(
	out: &mut String,
	value: &'a Value,
	non_finite: NonFinite,
	replace: &dyn Fn(&Value) -> Option<String>,
) -> Option<Open<'a>> {
	if !matches!(value, Value::Array(_) | Value::Object(_))
		&& let Some(replacement) = replace(value)
	{
		quote(out, &replacement);
		return None;
	}
	match value {
		Value::Null => out.push_str("null"),
		Value::Bool(true) => out.push_str("true"),
		Value::Bool(false) => out.push_str("false"),
		Value::Number(n) => match (n.is_finite(), non_finite) {
			(true, _) => out.push_str(ryu_js::Buffer::new().format_finite(*n)),
			(false, NonFinite::Null) => out.push_str("null"),
			(false, NonFinite::Name) => out.push_str(ryu_js::Buffer::new().format(*n)),
		},
		Value::String(s) => quote(out, s),
		Value::Array(array) => return open_array(out, array),
		Value::Object(object) => return open_object(out, object),
	}
	None
}

fn open_array<'a>(out: &mut String, array: &'a Array) -> Option<Open<'a>> {
	if array.is_empty() {
		out.push_str("[]");
		return None;
	}
	out.push('[');
	Some(Open::Array {
		items: array.iter(),
		first: true,
	})
}

fn open_object<'a>(out: &mut String, object: &'a Object) -> Option<Open<'a>> {
	if object.is_empty() {
		out.push_str("{}");
		return None;
	}
	out.push('{');
	Some(Open::Object {
		entries: object.iter(),
		first: true,
	})
}

fn newline(out: &mut String, indent: Indent, depth: usize) {
	if indent == Indent::Tab {
		out.push('\n');
		out.extend(std::iter::repeat_n('\t', depth));
	}
}

/// ECMAScript's QuoteJSONString. Lone surrogates, which it writes as
/// `\udXXX`, cannot occur in a Rust string.
fn quote(out: &mut String, s: &str) {
	out.push('"');
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\u{8}' => out.push_str("\\b"),
			'\u{c}' => out.push_str("\\f"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			// Writing to a String cannot fail.
			c if c < ' ' => {
				let _ = write!(out, "\\u{:04x}", u32::from(c));
			}
			c => out.push(c),
		}
	}
	out.push('"');
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::json::{Array, Object};

	fn object(entries: Vec<(&str, Value)>) -> Value {
		let mut object = Object::new();
		for (key, value) in entries {
			object.insert(key.to_owned(), value);
		}
		Value::Object(object)
	}

	fn array(values: Vec<Value>) -> Value {
		Value::Array(Array::from(values))
	}

	fn compact(value: &Value) -> String {
		stringify(value, Indent::None)
	}

	#[test]
	fn nan_and_the_infinities_are_written_as_null() {
		for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
			assert_eq!(compact(&Value::Number(n)), "null");
		}
	}

	#[test]
	fn numbers_use_the_javascript_format() {
		let cases = [
			(-0.0, "0"),
			(1.0, "1"),
			(1.5, "1.5"),
			(-2.0, "-2"),
			(1e21, "1e+21"),
			(1e20, "100000000000000000000"),
			(1e-7, "1e-7"),
			(1e-6, "0.000001"),
			(0.1 + 0.2, "0.30000000000000004"),
		];
		for (n, expected) in cases {
			assert_eq!(compact(&Value::Number(n)), expected, "{n:e}");
		}
	}

	#[test]
	fn only_quotes_backslashes_and_control_characters_are_escaped() {
		let s = "\"\\/\u{8}\u{c}\n\r\t\u{0}\u{1f}\u{7f}\u{2028}\u{e9}\u{1f41c}";
		assert_eq!(
			compact(&Value::String(s.to_owned())),
			"\"\\\"\\\\/\\b\\f\\n\\r\\t\\u0000\\u001f\u{7f}\u{2028}\u{e9}\u{1f41c}\""
		);
	}

	#[test]
	fn keys_are_escaped_like_strings() {
		let value = object(vec![("a\"\n", Value::Null)]);
		assert_eq!(compact(&value), "{\"a\\\"\\n\":null}");
	}

	#[test]
	fn compact_output_has_no_whitespace() {
		let value = object(vec![
			(
				"b",
				array(vec![Value::Bool(true), Value::Bool(false), Value::Null]),
			),
			("1", object(vec![])),
			("a", array(vec![])),
		]);
		assert_eq!(
			compact(&value),
			"{\"1\":{},\"b\":[true,false,null],\"a\":[]}"
		);
	}

	#[test]
	fn tab_output_puts_each_element_on_its_own_line() {
		let value = object(vec![
			(
				"b",
				array(vec![Value::Number(1.0), array(vec![Value::Null])]),
			),
			("1", object(vec![])),
			("a", array(vec![])),
		]);
		assert_eq!(
			stringify(&value, Indent::Tab),
			"{\n\t\"1\": {},\n\t\"b\": [\n\t\t1,\n\t\t[\n\t\t\tnull\n\t\t]\n\t],\n\t\"a\": []\n}"
		);
	}

	#[test]
	fn a_scalar_is_written_alone_in_both_layouts() {
		let value = Value::String("x".to_owned());
		assert_eq!(stringify(&value, Indent::None), "\"x\"");
		assert_eq!(stringify(&value, Indent::Tab), "\"x\"");
	}

	#[test]
	fn a_million_levels_of_nesting_are_written_without_overflowing_the_stack() {
		let depth = 1_000_000;
		let mut value = Value::Null;
		for _ in 0..depth {
			value = array(vec![value]);
		}
		let expected = format!("{}null{}", "[".repeat(depth), "]".repeat(depth));
		assert!(compact(&value) == expected);
	}
}
