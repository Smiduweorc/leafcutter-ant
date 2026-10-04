//! JSON as TS Dash holds it: values with JavaScript's property order, read
//! with json5 2.2.1 and written with `JSON.stringify`.
//!
//! Nothing here recurses over a value. json5 accepts any nesting depth and
//! V8's compact `JSON.stringify` writes it, so parsing, writing and dropping
//! are all iterative and a deeply nested file cannot overflow the stack.

mod json5;
mod json5_unicode;
mod stringify;

use std::collections::BTreeMap;
use std::collections::btree_map;
use std::fmt;
use std::mem;
use std::ops::{Deref, DerefMut};

pub(crate) use json5::parse_json_text;
pub use json5::{Json5Error, Json5ErrorKind, parse_json5};
pub(crate) use stringify::stringify_replacing;
pub use stringify::{Indent, stringify};

use stringify::{NonFinite, Root};

/// A JSON value. Numbers are `f64`, as in JavaScript, so `NaN` and the
/// infinities that json5 can produce are representable;
/// [`stringify()`] writes them as `null`, as `JSON.stringify` does.
pub enum Value {
	/// `null`.
	Null,
	/// `true` or `false`.
	Bool(bool),
	/// A JavaScript number.
	Number(f64),
	/// A string.
	String(String),
	/// An array.
	Array(Array),
	/// An object.
	Object(Object),
}

/// The elements of a JSON array. It dereferences to a slice for reading and
/// editing in place.
#[derive(Default)]
pub struct Array(Vec<Value>);

impl Array {
	/// An empty array.
	pub fn new() -> Self {
		Self::default()
	}

	/// Appends `value`, as `Array.prototype.push` does.
	pub fn push(&mut self, value: Value) {
		self.0.push(value);
	}
}

impl From<Vec<Value>> for Array {
	fn from(values: Vec<Value>) -> Self {
		Self(values)
	}
}

impl Deref for Array {
	type Target = [Value];

	fn deref(&self) -> &[Value] {
		&self.0
	}
}

impl DerefMut for Array {
	fn deref_mut(&mut self) -> &mut [Value] {
		&mut self.0
	}
}

/// A JSON object whose keys iterate in the order JavaScript gives them:
/// keys that are array indices (the canonical decimal form of an integer from
/// 0 to 2^32 - 2) first, in ascending numeric order, then every other key in
/// the order it was first inserted. `JSON.stringify` writes keys in that
/// order, so `{"b": 1, "1": 2}` comes out as `{"1":2,"b":1}`.
///
/// An `Object` has no prototype: `"__proto__"` is an ordinary key here. The
/// json5 reader applies json5's own handling of that key before inserting.
#[derive(Default)]
pub struct Object {
	indices: BTreeMap<u32, (String, Value)>,
	named: indexmap::IndexMap<String, Value>,
}

impl Object {
	/// An empty object.
	pub fn new() -> Self {
		Self::default()
	}

	/// The number of keys.
	pub fn len(&self) -> usize {
		self.indices.len() + self.named.len()
	}

	/// Whether the object has no keys.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// The value stored under `key`.
	pub fn get(&self, key: &str) -> Option<&Value> {
		match array_index(key) {
			Some(index) => self.indices.get(&index).map(|(_, value)| value),
			None => self.named.get(key),
		}
	}

	/// Stores `value` under `key`, as `object[key] = value` does in
	/// JavaScript: a new key takes its place in the order described on
	/// [`Object`], and an existing key keeps its place and gets the new value.
	/// Returns the value it replaced.
	pub fn insert(&mut self, key: String, value: Value) -> Option<Value> {
		match array_index(&key) {
			Some(index) => self.indices.insert(index, (key, value)).map(|(_, old)| old),
			None => self.named.insert(key, value),
		}
	}

	/// Removes `key`, as `delete object[key]` does, keeping the order of the
	/// other keys. Returns the value it held.
	pub fn remove(&mut self, key: &str) -> Option<Value> {
		match array_index(key) {
			Some(index) => self.indices.remove(&index).map(|(_, value)| value),
			None => self.named.shift_remove(key),
		}
	}

	/// The keys and values, in JavaScript's order.
	pub fn iter(&self) -> Iter<'_> {
		Iter {
			indices: self.indices.values(),
			named: self.named.iter(),
		}
	}

	fn take_values(&mut self) -> impl Iterator<Item = Value> {
		let indices = mem::take(&mut self.indices).into_values();
		let named = mem::take(&mut self.named).into_values();
		indices.map(|(_, value)| value).chain(named)
	}
}

impl<'a> IntoIterator for &'a Object {
	type Item = (&'a str, &'a Value);
	type IntoIter = Iter<'a>;

	fn into_iter(self) -> Iter<'a> {
		self.iter()
	}
}

/// The keys and values of an [`Object`], in JavaScript's order.
pub struct Iter<'a> {
	indices: btree_map::Values<'a, u32, (String, Value)>,
	named: indexmap::map::Iter<'a, String, Value>,
}

impl<'a> Iterator for Iter<'a> {
	type Item = (&'a str, &'a Value);

	fn next(&mut self) -> Option<Self::Item> {
		match self.indices.next() {
			Some((key, value)) => Some((key, value)),
			None => self.named.next().map(|(key, value)| (key.as_str(), value)),
		}
	}
}

/// The index `key` names if it is an array index in the sense of ECMAScript's
/// CanonicalNumericIndexString: `"0"`, `"7"` and `"4294967294"` are indices;
/// `"01"`, `"-1"`, `"1.0"`, `"+1"` and `"4294967295"` are ordinary keys.
fn array_index(key: &str) -> Option<u32> {
	let bytes = key.as_bytes();
	let canonical = match bytes {
		[] => false,
		[b'0'] => true,
		[first, ..] => *first != b'0' && bytes.len() <= 10 && bytes.iter().all(u8::is_ascii_digit),
	};
	if !canonical {
		return None;
	}
	key.parse::<u32>().ok().filter(|&index| index != u32::MAX)
}

// Debug shows the value as JSON, one line or (with `{:#?}`) tab-indented, but
// with `NaN` and the infinities spelled out rather than written as `null`.
// Like everything else here it does not recurse, which a derived Debug would.
fn debug(root: Root<'_>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	let indent = if f.alternate() {
		Indent::Tab
	} else {
		Indent::None
	};
	f.write_str(&stringify::write(root, indent, NonFinite::Name, &|_| None))
}

impl fmt::Debug for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		debug(Root::Value(self), f)
	}
}

impl fmt::Debug for Array {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		debug(Root::Array(self), f)
	}
}

impl fmt::Debug for Object {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		debug(Root::Object(self), f)
	}
}

// A derived clone would recurse once per level of nesting. This one keeps the
// containers it is still filling on a stack of its own.
impl Clone for Value {
	fn clone(&self) -> Self {
		/// A container being copied: the rest of its source, the copy so far,
		/// and for an object the key the child being copied will go under.
		enum Filling<'a> {
			Array(std::slice::Iter<'a, Value>, Vec<Value>),
			Object(Iter<'a>, Object, String),
		}
		fn start(value: &Value) -> Result<Filling<'_>, Value> {
			match value {
				Value::Array(array) => Ok(Filling::Array(
					array.iter(),
					Vec::with_capacity(array.len()),
				)),
				Value::Object(object) => {
					Ok(Filling::Object(object.iter(), Object::new(), String::new()))
				}
				Value::Null => Err(Value::Null),
				Value::Bool(b) => Err(Value::Bool(*b)),
				Value::Number(n) => Err(Value::Number(*n)),
				Value::String(s) => Err(Value::String(s.clone())),
			}
		}
		let mut stack = match start(self) {
			Ok(filling) => vec![filling],
			Err(scalar) => return scalar,
		};
		loop {
			let top = stack
				.last_mut()
				.expect("the stack holds the container being filled");
			let next = match top {
				Filling::Array(items, _) => items.next(),
				Filling::Object(entries, _, key) => entries.next().map(|(k, v)| {
					*key = k.to_owned();
					v
				}),
			};
			let done = match next {
				Some(child) => match start(child) {
					Ok(filling) => {
						stack.push(filling);
						continue;
					}
					Err(scalar) => scalar,
				},
				None => match stack.pop().expect("the stack is not empty") {
					Filling::Array(_, items) => Value::Array(Array(items)),
					Filling::Object(_, object, _) => Value::Object(object),
				},
			};
			match stack.last_mut() {
				None => return done,
				Some(Filling::Array(_, items)) => items.push(done),
				Some(Filling::Object(_, object, key)) => {
					object.insert(std::mem::take(key), done);
				}
			}
		}
	}
}

// A derived drop would recurse once per level of nesting. Each container
// instead moves its children into one flat list and drops them from there, so
// the nested containers it reaches are already empty when they are dropped.
impl Drop for Array {
	fn drop(&mut self) {
		drop_flat(mem::take(&mut self.0));
	}
}

impl Drop for Object {
	fn drop(&mut self) {
		let values = self.take_values().collect();
		drop_flat(values);
	}
}

fn drop_flat(mut pending: Vec<Value>) {
	while let Some(value) = pending.pop() {
		match value {
			Value::Array(mut array) => pending.append(&mut array.0),
			Value::Object(mut object) => pending.extend(object.take_values()),
			_ => {}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn keys(object: &Object) -> Vec<&str> {
		object.iter().map(|(key, _)| key).collect()
	}

	fn number(value: Option<&Value>) -> Option<f64> {
		match value {
			Some(Value::Number(n)) => Some(*n),
			_ => None,
		}
	}

	#[test]
	fn array_indices_are_the_canonical_integers_below_two_to_the_32_minus_one() {
		assert_eq!(array_index("0"), Some(0));
		assert_eq!(array_index("7"), Some(7));
		assert_eq!(array_index("10"), Some(10));
		assert_eq!(array_index("4294967294"), Some(4_294_967_294));
	}

	#[test]
	fn keys_that_only_look_numeric_are_not_array_indices() {
		for key in [
			"",
			"01",
			"00",
			"-1",
			"-0",
			"+1",
			"1.0",
			"1e3",
			" 1",
			"1 ",
			"4294967295",
			"4294967296",
			"99999999999",
			"\u{0661}",
		] {
			assert_eq!(array_index(key), None, "{key:?}");
		}
	}

	#[test]
	fn index_keys_iterate_first_in_numeric_order_then_the_rest_in_insertion_order() {
		let mut object = Object::new();
		for key in ["b", "10", "a", "2", "01", "4294967295", "0"] {
			object.insert(key.to_owned(), Value::Null);
		}
		assert_eq!(
			keys(&object),
			["0", "2", "10", "b", "a", "01", "4294967295"]
		);
	}

	#[test]
	fn assigning_an_existing_key_keeps_its_place_and_returns_the_old_value() {
		let mut object = Object::new();
		object.insert("a".to_owned(), Value::Number(1.0));
		object.insert("b".to_owned(), Value::Number(2.0));
		object.insert("5".to_owned(), Value::Number(3.0));
		let old = object.insert("a".to_owned(), Value::Number(4.0));
		let old_index = object.insert("5".to_owned(), Value::Number(5.0));
		assert_eq!(number(old.as_ref()), Some(1.0));
		assert_eq!(number(old_index.as_ref()), Some(3.0));
		assert_eq!(keys(&object), ["5", "a", "b"]);
		assert_eq!(number(object.get("a")), Some(4.0));
		assert_eq!(number(object.get("5")), Some(5.0));
		assert_eq!(object.len(), 3);
	}

	#[test]
	fn removing_a_key_keeps_the_order_of_the_others() {
		let mut object = Object::new();
		for key in ["b", "1", "a", "0", "c"] {
			object.insert(key.to_owned(), Value::Null);
		}
		assert!(object.remove("a").is_some());
		assert!(object.remove("0").is_some());
		assert!(object.remove("missing").is_none());
		assert_eq!(keys(&object), ["1", "b", "c"]);
	}

	#[test]
	fn a_borrowed_object_iterates_in_javascript_order() {
		let mut object = Object::new();
		object.insert("b".to_owned(), Value::Number(1.0));
		object.insert("3".to_owned(), Value::Number(2.0));
		let mut seen = Vec::new();
		for (key, value) in &object {
			seen.push((key, number(Some(value))));
		}
		assert_eq!(seen, [("3", Some(2.0)), ("b", Some(1.0))]);
	}

	#[test]
	fn array_elements_can_be_replaced_in_place() {
		let mut array = Array::from(vec![Value::Null, Value::Bool(true)]);
		array[0] = Value::Number(7.0);
		array.push(Value::Null);
		assert_eq!(array.len(), 3);
		assert_eq!(number(array.first()), Some(7.0));
	}

	#[test]
	fn a_missing_key_has_no_value() {
		let mut object = Object::new();
		assert!(object.is_empty());
		object.insert("1".to_owned(), Value::Null);
		assert!(object.get("01").is_none());
		assert!(object.get("2").is_none());
		assert!(object.get("x").is_none());
		assert!(!object.is_empty());
	}

	#[test]
	fn debug_shows_json_with_non_finite_numbers_spelled_out() {
		let mut object = Object::new();
		object.insert("n".to_owned(), Value::Number(f64::NAN));
		object.insert("0".to_owned(), Value::Number(f64::NEG_INFINITY));
		let mut array = Array::new();
		array.push(Value::Number(f64::INFINITY));
		array.push(Value::Number(-0.0));
		object.insert("a".to_owned(), Value::Array(array));
		let value = Value::Object(object);
		assert_eq!(
			format!("{value:?}"),
			r#"{"0":-Infinity,"n":NaN,"a":[Infinity,0]}"#
		);
		assert_eq!(
			format!("{value:#?}"),
			"{\n\t\"0\": -Infinity,\n\t\"n\": NaN,\n\t\"a\": [\n\t\tInfinity,\n\t\t0\n\t]\n}"
		);
		let Value::Object(object) = value else {
			unreachable!()
		};
		assert_eq!(
			format!("{object:?}"),
			r#"{"0":-Infinity,"n":NaN,"a":[Infinity,0]}"#
		);
		assert_eq!(format!("{:?}", Array::new()), "[]");
		assert_eq!(format!("{:?}", Object::new()), "{}");
	}

	#[test]
	fn a_clone_keeps_key_order_and_every_value() {
		let value =
			crate::json::parse_json5("{b: [1, {c: null, '2': true}], '1': 'x', a: {}, d: []}")
				.expect("json5");
		assert_eq!(
			format!("{:?}", value.clone()),
			r#"{"1":"x","b":[1,{"2":true,"c":null}],"a":{},"d":[]}"#
		);
		assert_eq!(format!("{:?}", Value::Number(f64::NAN).clone()), "NaN");
	}

	#[test]
	fn a_million_levels_of_nesting_clone_without_overflowing_the_stack() {
		let mut value = Value::Null;
		for level in 0..1_000_000 {
			value = if level % 2 == 0 {
				Value::Array(Array::from(vec![value]))
			} else {
				let mut object = Object::new();
				object.insert("k".to_owned(), value);
				Value::Object(object)
			};
		}
		let copy = value.clone();
		drop(value);
		let mut depth = 0;
		let mut at = &copy;
		loop {
			at = match at {
				Value::Array(items) => &items[0],
				Value::Object(object) => object.get("k").expect("the key survives"),
				_ => break,
			};
			depth += 1;
		}
		assert_eq!(depth, 1_000_000);
	}

	#[test]
	fn a_million_levels_of_nesting_drop_without_overflowing_the_stack() {
		// Test threads have a 2 MiB stack; a recursive drop overflows it long
		// before this depth.
		let mut value = Value::Null;
		for level in 0..1_000_000 {
			value = if level % 2 == 0 {
				Value::Array(Array::from(vec![value]))
			} else {
				let mut object = Object::new();
				object.insert("k".to_owned(), value);
				Value::Object(object)
			};
		}
		drop(value);
	}
}
