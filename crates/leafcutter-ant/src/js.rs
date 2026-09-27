//! The few JavaScript conversions TS Dash applies to project data: truthiness,
//! `String(value)` for template literals, and property lookup, including the
//! properties every plain object inherits from `Object.prototype`, which a
//! lookup in a plain object finds when the key is, say, `"constructor"`.

use crate::json::Value;

/// A value a property lookup can produce: a JSON value, `undefined`, or one
/// of `Object.prototype`'s properties.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Prop<'a> {
	Undefined,
	Value(&'a Value),
	Inherited(Inherited),
}

/// A property of `Object.prototype`: `__proto__` is the prototype object, and
/// the rest are functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Inherited(&'static str);

/// `Object.getOwnPropertyNames(Object.prototype)` in V8, with each function's
/// `length`.
const OBJECT_PROTOTYPE: [(&str, u32); 12] = [
	("constructor", 1),
	("__defineGetter__", 2),
	("__defineSetter__", 2),
	("hasOwnProperty", 1),
	("__lookupGetter__", 1),
	("__lookupSetter__", 1),
	("isPrototypeOf", 1),
	("propertyIsEnumerable", 1),
	("toString", 0),
	("valueOf", 0),
	("__proto__", 0),
	("toLocaleString", 0),
];

impl Inherited {
	/// The inherited property named `key`, if there is one.
	pub(crate) fn lookup(key: &str) -> Option<Self> {
		OBJECT_PROTOTYPE
			.iter()
			.find(|(name, _)| *name == key)
			.map(|(name, _)| Inherited(name))
	}

	/// `String(Object.prototype[name])`.
	fn to_js_string(self) -> String {
		match self.0 {
			"__proto__" => "[object Object]".to_owned(),
			"constructor" => "function Object() { [native code] }".to_owned(),
			name => format!("function {name}() {{ [native code] }}"),
		}
	}

	/// The function's `length`; `None` for the prototype object, which has no
	/// `length`.
	fn length(self) -> Option<u32> {
		if self.0 == "__proto__" {
			return None;
		}
		OBJECT_PROTOTYPE
			.iter()
			.find(|(name, _)| *name == self.0)
			.map(|(_, length)| *length)
	}
}

impl<'a> Prop<'a> {
	/// `value ?? fallback`.
	pub(crate) fn or_else(self, fallback: impl FnOnce() -> Prop<'a>) -> Prop<'a> {
		match self {
			Prop::Undefined | Prop::Value(Value::Null) => fallback(),
			other => other,
		}
	}

	/// `String(value)`, as a template literal converts it.
	pub(crate) fn to_js_string(self) -> String {
		match self {
			Prop::Undefined => "undefined".to_owned(),
			Prop::Value(value) => to_js_string(value),
			Prop::Inherited(inherited) => inherited.to_js_string(),
		}
	}

	/// `value && value.length > 0`, pathe 1.1.2's test for a segment worth
	/// joining.
	pub(crate) fn has_positive_length(self) -> bool {
		match self {
			Prop::Undefined => false,
			Prop::Inherited(inherited) => inherited.length().is_some_and(|length| length > 0),
			Prop::Value(value) => match value {
				Value::String(s) => !s.is_empty(),
				Value::Array(items) => !items.is_empty(),
				Value::Object(object) => match object.get("length") {
					Some(length) => greater_than_zero(length),
					None => false,
				},
				_ => false,
			},
		}
	}
}

/// `length > 0` for an object's own `length` property: JavaScript's
/// relational comparison, which converts the value to a number first.
fn greater_than_zero(length: &Value) -> bool {
	match length {
		Value::Number(n) => *n > 0.0,
		Value::Bool(b) => *b,
		Value::Null => false,
		other => string_to_number(&to_js_string(other)).is_some_and(|n| n > 0.0),
	}
}

/// ECMAScript's StringToNumber, with `None` for `NaN`: surrounding
/// whitespace is ignored, the empty string is zero, `0x`, `0o` and `0b`
/// prefixes read an unsigned integer, and anything else must be a decimal
/// literal or a signed `Infinity`.
fn string_to_number(s: &str) -> Option<f64> {
	let is_js_space = |c: char| {
		matches!(
			c,
			'\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
				..='\u{200a}'
					| '\u{2028}' | '\u{2029}'
					| '\u{202f}' | '\u{205f}'
					| '\u{3000}' | '\u{feff}'
		)
	};
	let s = s.trim_matches(is_js_space);
	if s.is_empty() {
		return Some(0.0);
	}
	for (prefix, radix) in [
		("0x", 16),
		("0X", 16),
		("0o", 8),
		("0O", 8),
		("0b", 2),
		("0B", 2),
	] {
		if let Some(digits) = s.strip_prefix(prefix) {
			if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
				return None;
			}
			return Some(digits.chars().fold(0.0, |n, c| {
				n * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
			}));
		}
	}
	let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
	if unsigned == "Infinity" {
		return Some(if s.starts_with('-') {
			f64::NEG_INFINITY
		} else {
			f64::INFINITY
		});
	}
	// Rust also reads "inf" and "nan", which JavaScript does not.
	if !unsigned
		.bytes()
		.all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
	{
		return None;
	}
	s.parse::<f64>().ok()
}

/// JavaScript truthiness.
pub(crate) fn truthy(value: &Value) -> bool {
	match value {
		Value::Null => false,
		Value::Bool(b) => *b,
		Value::Number(n) => *n != 0.0 && !n.is_nan(),
		Value::String(s) => !s.is_empty(),
		Value::Array(_) | Value::Object(_) => true,
	}
}

/// `String(value)`: numbers as `Number#toString` writes them, arrays joined
/// with commas (`null` elements empty), objects as `[object Object]`. Nested
/// arrays are joined without recursion.
pub(crate) fn to_js_string(value: &Value) -> String {
	let mut out = String::new();
	// Each entry is a value still to write, and whether a comma goes first.
	let mut pending: Vec<(&Value, bool)> = vec![(value, false)];
	let mut top_level = true;
	while let Some((value, comma)) = pending.pop() {
		if comma {
			out.push(',');
		}
		match value {
			Value::Null if top_level => out.push_str("null"),
			Value::Null => {}
			Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
			Value::Number(n) => out.push_str(ryu_js::Buffer::new().format(*n)),
			Value::String(s) => out.push_str(s),
			Value::Object(_) => out.push_str("[object Object]"),
			Value::Array(items) => {
				for (i, item) in items.iter().enumerate().rev() {
					pending.push((item, i > 0));
				}
			}
		}
		top_level = false;
	}
	out
}

/// `object[key]` on a JSON value: own properties of objects, indices of
/// arrays and strings, and `undefined` for everything else. The caller
/// decides whether the key can reach an inherited property.
pub(crate) fn own<'a>(value: &'a Value, key: &str) -> Prop<'a> {
	match value {
		Value::Object(object) => object.get(key).map_or(Prop::Undefined, Prop::Value),
		Value::Array(items) => index(key)
			.and_then(|i| items.get(i))
			.map_or(Prop::Undefined, Prop::Value),
		_ => Prop::Undefined,
	}
}

/// `object[key]` on a plain object parsed from JSON, which inherits from
/// `Object.prototype` when the key is not its own.
pub(crate) fn get<'a>(value: &'a Value, key: &str) -> Prop<'a> {
	match (value, own(value, key)) {
		(Value::Object(_), Prop::Undefined) => {
			Inherited::lookup(key).map_or(Prop::Undefined, Prop::Inherited)
		}
		(_, found) => found,
	}
}

/// The keys `for (key in value)` visits for a JSON value: an object's keys in
/// JavaScript order, an array's indices, a string's code unit indices, and
/// none for anything else.
pub(crate) fn for_in_keys(value: &Value) -> Vec<String> {
	match value {
		Value::Object(object) => object.iter().map(|(key, _)| key.to_owned()).collect(),
		Value::Array(items) => (0..items.len()).map(|i| i.to_string()).collect(),
		Value::String(s) => (0..s.encode_utf16().count())
			.map(|i| i.to_string())
			.collect(),
		_ => Vec::new(),
	}
}

fn index(key: &str) -> Option<usize> {
	let canonical =
		key == "0" || (!key.starts_with('0') && key.bytes().all(|b| b.is_ascii_digit()));
	if canonical && !key.is_empty() {
		key.parse().ok()
	} else {
		None
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::json::parse_json5;

	fn value(source: &str) -> Value {
		parse_json5(source).expect("valid json5")
	}

	#[test]
	fn string_conversion_follows_javascript() {
		for (source, expected) in [
			("null", "null"),
			("true", "true"),
			("1.5", "1.5"),
			("1e21", "1e+21"),
			("-0", "0"),
			("'x'", "x"),
			("[1, [2, [3, null]], null, 'a']", "1,2,3,,,a"),
			("[]", ""),
			("[null]", ""),
			("[{}]", "[object Object]"),
			("{}", "[object Object]"),
		] {
			assert_eq!(to_js_string(&value(source)), expected, "{source}");
		}
	}

	#[test]
	fn truthiness_follows_javascript() {
		for (source, expected) in [
			("null", false),
			("false", false),
			("0", false),
			("-0", false),
			("NaN", false),
			("''", false),
			("'0'", true),
			("[]", true),
			("{}", true),
			("-1", true),
		] {
			assert_eq!(truthy(&value(source)), expected, "{source}");
		}
	}

	#[test]
	fn inherited_properties_are_found_on_objects_only() {
		let object = value("{a: 1}");
		assert!(matches!(get(&object, "constructor"), Prop::Inherited(_)));
		assert_eq!(
			get(&object, "constructor").to_js_string(),
			"function Object() { [native code] }"
		);
		assert_eq!(
			get(&object, "toString").to_js_string(),
			"function toString() { [native code] }"
		);
		assert_eq!(get(&object, "__proto__").to_js_string(), "[object Object]");
		assert!(matches!(get(&value("[1]"), "constructor"), Prop::Undefined));
		assert!(matches!(get(&object, "b"), Prop::Undefined));
		assert!(get(&object, "constructor").has_positive_length());
		assert!(!get(&object, "toString").has_positive_length());
		assert!(!get(&object, "__proto__").has_positive_length());
	}

	#[test]
	fn for_in_visits_object_keys_array_indices_and_string_units() {
		assert_eq!(for_in_keys(&value("{b: 1, '1': 2}")), ["1", "b"]);
		assert_eq!(for_in_keys(&value("[5, 6]")), ["0", "1"]);
		assert_eq!(for_in_keys(&value("'ab'")), ["0", "1"]);
		assert!(for_in_keys(&value("5")).is_empty());
	}
}
