//! `floatPropertyTruncationFix` (`src/Plugins/BuiltIn/FloatPropertyTruncationFix.ts`):
//! writes `player.json` entity files tab-indented, with the numbers of float
//! properties printed with a decimal point (`1` as `1.0`), because Minecraft
//! reads a float property written as `1` as an integer.
//!
//! TS Dash does this with a `JSON.stringify` replacer that is kept here
//! exactly as it behaves, bugs included: it pushes the key of every object
//! and array it enters and never pops it, so after the first nested object
//! the path it matches against is every key visited so far joined with `/`;
//! it logs every primitive's path and every glob test on the console; and it
//! marks a number with a string it replaces afterwards with a regular
//! expression that only accepts digits, `.` and `-`, so `1e+21.0` or
//! `NaN.0` stays in the output as a marker string.

use std::collections::HashMap;

use crate::js::{self, Prop};
use crate::json::{Indent, Value, stringify_replacing};
use crate::plugin::{Context, Data, Finalized, Hook, Plugin};

const MARKER: &str =
	"$___dash___floatPropertyTruncationFix___THIS IS AUTO GENERATED AND I HATE IT___";

/// The plugin's two matchers: a property's `value` when the object holding it
/// has `type: "float"`, and each element of a property's `range` when the
/// object two steps up does.
const MATCHERS: [(&str, usize); 2] = [
	("minecraft:entity/description/properties/*/value", 1),
	("minecraft:entity/description/properties/*/range/*", 2),
];

pub(crate) struct FloatPropertyTruncationFix;

/// `traversedObjects[traversedObjects.length - back].type !== "float"`.
fn is_float(holders: &[&Value], back: usize) -> bool {
	let Some(holder) = holders.len().checked_sub(back).map(|i| holders[i]) else {
		return false;
	};
	matches!(js::get(holder, "type"), Prop::Value(Value::String(t)) if t == "float")
}

/// The replacer's state: the keys and holders it pushed, and the numbers it
/// replaced, by address, with their marker strings.
#[derive(Default)]
struct Replacer<'a> {
	keys: Vec<String>,
	holders: Vec<&'a Value>,
	replaced: HashMap<usize, String>,
}

impl<'a> Replacer<'a> {
	/// The replacer's call for a string, number or boolean: log the path,
	/// pop, and for a number try the matchers in order, logging each test,
	/// until one applies.
	fn primitive(&mut self, cx: &Context, entry: Option<(String, &'a Value)>, value: &Value) {
		if let Some((key, holder)) = entry {
			self.keys.push(key);
			self.holders.push(holder);
		}
		let path = self.keys.join("/");
		let traversed = self.holders.clone();
		cx.logger.console().log(&path);
		self.keys.pop();
		self.holders.pop();
		let Value::Number(n) = value else {
			return;
		};
		for (glob, back) in MATCHERS {
			let matched = cx.globs.is_match(&path, glob).unwrap_or(false);
			cx.logger.console().log(&format!("{path} {glob} {matched}"));
			if matched && is_float(&traversed, back) {
				let text = ryu_js::Buffer::new().format(*n).to_owned();
				let text = if text.contains('.') {
					text
				} else {
					format!("{text}.0")
				};
				self.replaced
					.insert(address(value), format!("{MARKER}{text}"));
				return;
			}
		}
	}
}

fn address(value: &Value) -> usize {
	std::ptr::from_ref(value) as usize
}

/// A container being walked, with its children still to visit.
enum Walk<'a> {
	Array(&'a Value, std::iter::Enumerate<std::slice::Iter<'a, Value>>),
	Object(&'a Value, crate::json::Iter<'a>),
}

fn walk(value: &Value) -> Option<Walk<'_>> {
	match value {
		Value::Array(items) => Some(Walk::Array(value, items.iter().enumerate())),
		Value::Object(object) => Some(Walk::Object(value, object.iter())),
		_ => None,
	}
}

/// `jsonStringifyWithFloatFix(json, matchers)` with the plugin's matchers and
/// a tab indent. The replacer sees values in the order `JSON.stringify` does:
/// depth first, each object's keys in JavaScript order.
fn json_stringify_with_float_fix(cx: &Context, json: &Value) -> String {
	let mut replacer = Replacer::default();
	let mut stack: Vec<Walk<'_>> = Vec::new();
	match walk(json) {
		Some(container) => stack.push(container),
		// `typeof null === "object"`, so a null root is not a primitive.
		None if matches!(json, Value::Null) => {}
		None => replacer.primitive(cx, None, json),
	}
	while let Some(top) = stack.last_mut() {
		let next = match top {
			Walk::Array(holder, items) => items
				.next()
				.map(|(i, child)| (*holder, i.to_string(), child)),
			Walk::Object(holder, entries) => entries
				.next()
				.map(|(key, child)| (*holder, key.to_owned(), child)),
		};
		let Some((holder, key, child)) = next else {
			stack.pop();
			continue;
		};
		match child {
			Value::Array(_) | Value::Object(_) | Value::Null => {
				// An object, array or null: pushed and never popped.
				replacer.keys.push(key);
				replacer.holders.push(holder);
				stack.extend(walk(child));
			}
			primitive => replacer.primitive(cx, Some((key, holder)), primitive),
		}
	}
	let replaced = replacer.replaced;
	let output = stringify_replacing(json, Indent::Tab, &|value| {
		replaced.get(&address(value)).cloned()
	});
	replace_markers(&output)
}

/// `.replaceAll(/"\$___dash___floatPropertyTruncationFix___THIS IS AUTO
/// GENERATED AND I HATE IT___([0-9]|\.|-)+"/g, value => value.substring(80,
/// value.length - 1))`: every quoted marker followed only by digits, dots and
/// minus signs becomes those characters, unquoted, wherever it is in the
/// output, strings the project wrote itself included.
fn replace_markers(output: &str) -> String {
	let needle = format!("\"{MARKER}");
	let mut result = String::with_capacity(output.len());
	let mut rest = output;
	while let Some(at) = rest.find(&needle) {
		let after = &rest[at + needle.len()..];
		let digits = after
			.bytes()
			.take_while(|b| b.is_ascii_digit() || *b == b'.' || *b == b'-')
			.count();
		if digits > 0 && after.as_bytes().get(digits) == Some(&b'"') {
			result.push_str(&rest[..at]);
			result.push_str(&after[..digits]);
			rest = &after[digits + 1..];
		} else {
			// No match here; the next attempt starts one character on.
			result.push_str(&rest[..at + 1]);
			rest = &rest[at + 1..];
		}
	}
	result.push_str(rest);
	result
}

impl Plugin for FloatPropertyTruncationFix {
	fn hooks(&self) -> &[Hook] {
		&[Hook::FinalizeBuild]
	}

	/// Entity files only: `player.json` (any path ending in it) is written
	/// with the fix, other entities are handed on as they are, and every other
	/// file is left to the next plugin.
	fn finalize_build(
		&mut self,
		cx: &Context,
		path: &str,
		data: &Data,
	) -> Result<Finalized, String> {
		if cx
			.file_types
			.id(&cx.project, path)
			.map_err(|e| e.to_string())?
			!= "entity"
		{
			return Ok(Finalized::Undefined);
		}
		if !path.ends_with("player.json") {
			return Ok(Finalized::Current);
		}
		let json = match data {
			Data::Value(Value::String(_)) => return Ok(Finalized::Current),
			Data::Value(value) => json_stringify_with_float_fix(cx, value),
			Data::Shared(value) => json_stringify_with_float_fix(cx, &value.borrow()),
		};
		Ok(Finalized::Data(Data::Value(Value::String(json))))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::testing::{MemoryFs, dash_with, plugin_vectors, value};

	/// plugins.json, recorded by tools/parity/plugins.mjs from TS Dash's own
	/// plugin: the result of `finalizeBuild` and every line it logged, for
	/// entity documents and paths in and out of the entities folder.
	#[test]
	fn finalize_build_and_its_console_lines_match_ts_dash() {
		let (mut dash, recorder) =
			dash_with(MemoryFs::with(&[]), r#"["floatPropertyTruncationFix"]"#);
		let vectors = plugin_vectors("floatPropertyTruncationFix");
		assert!(vectors.len() > 150);
		let mut failures = Vec::new();
		for vector in &vectors {
			let path = vector[0].as_str().expect("a path");
			recorder.0.borrow_mut().clear();
			let data = Data::Value(value(&vector[1]));
			let (cx, plugin) = dash.first_plugin();
			let result = match plugin.finalize_build(cx, path, &data).expect("no error") {
				Finalized::Undefined => serde_json::json!("<undefined>"),
				Finalized::Current => serde_json::json!("<fileContent>"),
				Finalized::Data(Data::Value(Value::String(s))) => serde_json::Value::String(s),
				Finalized::Data(_) => panic!("finalizeBuild gives a string"),
			};
			let logged: Vec<serde_json::Value> = recorder
				.0
				.borrow()
				.iter()
				.map(|line| {
					serde_json::Value::String(
						line.strip_prefix("log: ").expect("a log line").to_owned(),
					)
				})
				.collect();
			if result != vector[2] || serde_json::Value::Array(logged.clone()) != vector[3] {
				failures.push(format!(
					"{path} {}:\n  expected {} {}\n  got      {result} {}",
					vector[1],
					vector[2],
					vector[3],
					serde_json::Value::Array(logged)
				));
			}
		}
		assert!(
			failures.is_empty(),
			"{} of {} differ:\n{}",
			failures.len(),
			vectors.len(),
			failures.join("\n")
		);
	}

	#[test]
	fn a_marker_is_replaced_only_when_digits_dots_and_minus_signs_follow() {
		let m = MARKER;
		assert_eq!(
			replace_markers(&format!("[\"{m}1.0\", \"{m}-2.5\"]")),
			"[1.0, -2.5]"
		);
		assert_eq!(
			replace_markers(&format!("\"{m}1e+21.0\"")),
			format!("\"{m}1e+21.0\"")
		);
		assert_eq!(replace_markers(&format!("\"{m}\"")), format!("\"{m}\""));
		assert_eq!(
			replace_markers(&format!("\"{m}\"{m}3\"")),
			format!("\"{m}3")
		);
	}
}
