//! Glob matching as Dash does it: common-utils' `isMatch` over its vendored
//! picomatch, and is-glob 4.0.3 to decide whether a query is a glob at all.
//!
//! picomatch builds the source of a JavaScript regular expression; `regress`
//! compiles it with ECMAScript semantics, outside unicode mode, and matches it
//! against the path's UTF-16 code units, which is what `RegExp#exec` sees.

mod picomatch;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub(crate) use picomatch::PicomatchError;

/// A compiled glob: the pattern, for picomatch's exact-string shortcut, and
/// its regular expression, which is `None` when V8 would have refused the
/// source and picomatch fell back to `/$^/`, which matches nothing.
struct Compiled {
	pattern: String,
	regex: Option<regress::Regex>,
}

/// Compiles each glob once. TS Dash compiles it again on every call; the
/// result is the same, only slower.
#[derive(Default)]
pub(crate) struct Globs {
	cache: RefCell<HashMap<String, Rc<Result<Compiled, PicomatchError>>>>,
}

impl Globs {
	/// `isMatch(path, pattern)`: whether picomatch's regular expression for
	/// `pattern` matches `path`. An error is what picomatch would throw.
	pub(crate) fn is_match(&self, path: &str, pattern: &str) -> Result<bool, PicomatchError> {
		let compiled = self.compiled(pattern);
		let compiled = match compiled.as_ref() {
			Ok(compiled) => compiled,
			Err(error) => return Err(error.clone()),
		};
		// picomatch.test: an empty path never matches, and a path equal to the
		// pattern matches without the regular expression.
		if path.is_empty() {
			return Ok(false);
		}
		if path == compiled.pattern {
			return Ok(true);
		}
		let Some(regex) = &compiled.regex else {
			return Ok(false);
		};
		let units: Vec<u16> = path.encode_utf16().collect();
		Ok(regex.find_from_ucs2(&units, 0).next().is_some())
	}

	/// `isMatch(path, patterns)`: whether any pattern matches, trying them in
	/// order and stopping at the first match or the first error.
	pub(crate) fn is_match_any<'a>(
		&self,
		path: &str,
		patterns: impl IntoIterator<Item = &'a str>,
	) -> Result<bool, PicomatchError> {
		for pattern in patterns {
			if self.is_match(path, pattern)? {
				return Ok(true);
			}
		}
		Ok(false)
	}

	fn compiled(&self, pattern: &str) -> Rc<Result<Compiled, PicomatchError>> {
		if let Some(compiled) = self.cache.borrow().get(pattern) {
			return Rc::clone(compiled);
		}
		let compiled = Rc::new(picomatch::regex_source(pattern).map(|source| Compiled {
			pattern: pattern.to_owned(),
			regex: compile(&source),
		}));
		self.cache
			.borrow_mut()
			.insert(pattern.to_owned(), Rc::clone(&compiled));
		compiled
	}
}

/// `new RegExp(source)`, with the source as UTF-16 code units: outside unicode
/// mode each code unit, surrogates included, is one pattern character.
fn compile(source: &[u16]) -> Option<regress::Regex> {
	regress::Regex::from_unicode(
		source.iter().map(|&unit| u32::from(unit)),
		regress::Flags::default(),
	)
	.ok()
}

/// Whether `new RegExp(source)` would succeed.
fn compiles(source: &[u16]) -> bool {
	compile(source).is_some()
}

/// is-glob 4.0.3's `isGlob(str)` in its default, strict mode.
pub(crate) fn is_glob(s: &str) -> bool {
	if s.is_empty() {
		return false;
	}
	let chars: Vec<char> = s.chars().collect();
	is_extglob(&chars) || strict_check(&chars)
}

fn is_line_terminator(c: char) -> bool {
	matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// is-extglob 2.1.1: repeatedly finds `/(\\).|([@?!+*]\(.*\))/` and answers
/// true at the first match of the second branch; each match of the first
/// branch (an escape) cuts the string after it.
fn is_extglob(chars: &[char]) -> bool {
	let mut rest = chars;
	'search: loop {
		for i in 0..rest.len() {
			if rest[i] == '\\' && rest.get(i + 1).is_some_and(|&c| !is_line_terminator(c)) {
				rest = &rest[i + 2..];
				continue 'search;
			}
			if "@?!+*".contains(rest[i]) && rest.get(i + 1) == Some(&'(') {
				// `\(.*\)`: a `)` later on the same line.
				let closes = rest[i + 2..]
					.iter()
					.take_while(|&&c| !is_line_terminator(c))
					.any(|&c| c == ')');
				if closes {
					return true;
				}
			}
		}
		return false;
	}
}

/// is-glob's `strictCheck`.
fn strict_check(s: &[char]) -> bool {
	if s[0] == '!' {
		return true;
	}
	let at = |i: isize| -> Option<char> { usize::try_from(i).ok().and_then(|i| s.get(i).copied()) };
	let index_of = |c: char, from: isize| -> isize {
		let from = from.max(0) as usize;
		s.get(from..)
			.and_then(|rest| rest.iter().position(|&x| x == c))
			.map_or(-1, |p| (p + from) as isize)
	};
	let mut index: isize = 0;
	let mut pipe_index: isize = -2;
	let mut close_square_index: isize = -2;
	let mut close_curly_index: isize = -2;
	let mut close_paren_index: isize = -2;
	let mut back_slash_index: isize = -2;
	while (index as usize) < s.len() {
		let current = at(index);
		if current == Some('*') {
			return true;
		}
		if at(index + 1) == Some('?') && current.is_some_and(|c| "].+)".contains(c)) {
			return true;
		}
		if close_square_index != -1 && current == Some('[') && at(index + 1) != Some(']') {
			if close_square_index < index {
				close_square_index = index_of(']', index);
			}
			if close_square_index > index {
				if back_slash_index == -1 || back_slash_index > close_square_index {
					return true;
				}
				back_slash_index = index_of('\\', index);
				if back_slash_index == -1 || back_slash_index > close_square_index {
					return true;
				}
			}
		}
		if close_curly_index != -1 && current == Some('{') && at(index + 1) != Some('}') {
			close_curly_index = index_of('}', index);
			if close_curly_index > index {
				back_slash_index = index_of('\\', index);
				if back_slash_index == -1 || back_slash_index > close_curly_index {
					return true;
				}
			}
		}
		if close_paren_index != -1
			&& current == Some('(')
			&& at(index + 1) == Some('?')
			&& at(index + 2).is_some_and(|c| ":!=".contains(c))
			&& at(index + 3) != Some(')')
		{
			close_paren_index = index_of(')', index);
			if close_paren_index > index {
				back_slash_index = index_of('\\', index);
				if back_slash_index == -1 || back_slash_index > close_paren_index {
					return true;
				}
			}
		}
		if pipe_index != -1 && current == Some('(') && at(index + 1) != Some('|') {
			if pipe_index < index {
				pipe_index = index_of('|', index);
			}
			if pipe_index != -1 && at(pipe_index + 1) != Some(')') {
				close_paren_index = index_of(')', pipe_index);
				if close_paren_index > pipe_index {
					back_slash_index = index_of('\\', pipe_index);
					if back_slash_index == -1 || back_slash_index > close_paren_index {
						return true;
					}
				}
			}
		}
		if current == Some('\\') {
			let open = at(index + 1);
			index += 2;
			let close = match open {
				Some('{') => Some('}'),
				Some('(') => Some(')'),
				Some('[') => Some(']'),
				_ => None,
			};
			if let Some(close) = close {
				let n = index_of(close, index);
				if n != -1 {
					index = n + 1;
				}
			}
			if at(index) == Some('!') {
				return true;
			}
		} else {
			index += 1;
		}
	}
	false
}

#[cfg(test)]
mod tests {
	use super::*;

	fn vectors(name: &str) -> Vec<serde_json::Value> {
		let path = format!("{}/tests/vectors/{name}", env!("CARGO_MANIFEST_DIR"));
		let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
		serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
	}

	/// The source as globs.mjs writes it: code units outside printable ASCII
	/// as `\uXXXX`.
	fn escape(units: &[u16]) -> String {
		units
			.iter()
			.map(|&unit| match u8::try_from(unit) {
				Ok(b @ 0x20..=0x7e) => char::from(b).to_string(),
				_ => format!("\\u{unit:04x}"),
			})
			.collect()
	}

	fn text(value: &serde_json::Value) -> &str {
		value.as_str().expect("a string in the vector")
	}

	/// globs.json, recorded by tools/parity/globs.mjs from the picomatch
	/// common-utils bundles: the regex source, whether V8 compiled it, and
	/// isMatch on paths meant to match and paths meant not to.
	#[test]
	fn picomatch_builds_the_same_regex_and_matches_the_same_paths() {
		let cases = vectors("globs.json");
		assert!(cases.len() > 1500);
		let globs = Globs::default();
		let mut failures = Vec::new();
		let mut checked = 0;
		for case in &cases {
			let pattern = text(&case[0]);
			let source = case[1].as_str().map(str::to_owned);
			let compiled = &case[2].as_bool().expect("a boolean");
			let results = case[3].as_array().expect("an array of results");
			let actual_source = picomatch::regex_source(pattern)
				.ok()
				.map(|units| escape(&units));
			if actual_source != source {
				failures.push(format!(
					"{pattern:?}: source\n  expected {source:?}\n  got      {actual_source:?}"
				));
			}
			let actual_compiled = picomatch::regex_source(pattern).is_ok_and(|s| compiles(&s));
			if actual_compiled != *compiled {
				failures.push(format!(
					"{pattern:?}: V8 compiled it: {compiled}, regress: {actual_compiled}"
				));
			}
			for result in results {
				let (path, expected) = (text(&result[0]), &result[1]);
				checked += 1;
				let actual = match globs.is_match(path, pattern) {
					Ok(matched) => serde_json::Value::Bool(matched),
					Err(_) => serde_json::Value::String("throws".to_owned()),
				};
				if &actual != expected {
					failures.push(format!(
						"isMatch({path:?}, {pattern:?}): expected {expected}, got {actual}"
					));
				}
			}
		}
		assert!(
			failures.is_empty(),
			"{} failures over {} patterns and {checked} paths:\n{}",
			failures.len(),
			cases.len(),
			failures.join("\n")
		);
	}

	#[test]
	fn is_glob_agrees_with_is_glob_4_0_3() {
		let cases = vectors("is-glob.json");
		assert!(cases.len() > 3000);
		let failures: Vec<String> = cases
			.iter()
			.filter(|case| Some(is_glob(text(&case[0]))) != case[1].as_bool())
			.map(|case| format!("{:?}: expected {}", case[0], case[1]))
			.collect();
		assert!(
			failures.is_empty(),
			"{} differ:\n{}",
			failures.len(),
			failures.join("\n")
		);
	}

	#[test]
	fn an_empty_pattern_throws_and_an_empty_path_never_matches() {
		let globs = Globs::default();
		assert_eq!(globs.is_match("a", ""), Err(PicomatchError::EmptyPattern));
		assert_eq!(globs.is_match("", "*"), Ok(false));
		assert_eq!(globs.is_match("", "**"), Ok(false));
	}

	#[test]
	fn a_path_equal_to_the_pattern_matches_even_when_the_regex_would_not() {
		let globs = Globs::default();
		// "a[b" escapes the lone bracket; "(?<n>" throws before any matching.
		assert_eq!(globs.is_match("a[b", "a[b"), Ok(true));
		assert_eq!(globs.is_match("[abc]", "[abc]"), Ok(true));
		assert_eq!(globs.is_match("b", "[abc]"), Ok(true));
		assert_eq!(
			globs.is_match("(?<n>a)", "(?<n>a)"),
			Err(PicomatchError::Lookbehind)
		);
	}

	#[test]
	fn patterns_named_like_object_prototype_properties_throw() {
		let globs = Globs::default();
		for name in ["constructor", "toString", "__proto__", "hasOwnProperty"] {
			assert_eq!(
				globs.is_match(name, name),
				Err(PicomatchError::InheritedReplacement)
			);
		}
	}

	#[test]
	fn is_match_any_stops_at_the_first_match() {
		let globs = Globs::default();
		assert_eq!(globs.is_match_any("BP/a.json", ["RP/*", "BP/*"]), Ok(true));
		assert_eq!(globs.is_match_any("BP/a.json", ["BP/*", ""]), Ok(true));
		assert_eq!(
			globs.is_match_any("BP/a.json", ["", "BP/*"]),
			Err(PicomatchError::EmptyPattern)
		);
		assert_eq!(globs.is_match_any("BP/a.json", []), Ok(false));
	}
}
