//! A port of the picomatch copy vendored in `@bridge-editor/common-utils`
//! 0.3.3 (`src/glob/picomatch.js`), as Dash calls it: `picomatch(pattern)`
//! with no options. In that bundle `path.sep` is `/`, so Windows mode is off,
//! and the shimmed `process.version` is `""`, so picomatch believes the engine
//! has no lookbehind and throws on `(?<`.
//!
//! picomatch turns a glob into the source of a JavaScript regular expression.
//! The port builds the same source, code unit for code unit, and the caller
//! compiles it with an ECMAScript regex engine. Everything works on UTF-16
//! code units because picomatch does: its fast path escapes each half of a
//! surrogate pair separately.

use std::fmt;

type Units = Vec<u16>;

fn u(s: &str) -> Units {
	s.encode_utf16().collect()
}

const DOT_LITERAL: &str = "\\.";
const SLASH_LITERAL: &str = "\\/";
const ONE_CHAR: &str = "(?=.)";
const QMARK: &str = "[^/]";
const NO_DOT: &str = "(?!\\.)";
const NO_DOT_SLASH: &str = "(?!\\.{0,1}(?:\\/|$))";
const QMARK_NO_DOT: &str = "[^.\\/]";
const STAR: &str = "[^/]*?";
/// `globstar({})`: any run of characters that never starts a segment with a
/// dot.
const GLOBSTAR: &str = "(?:(?:(?!(?:^|\\/)\\.).)*?)";
const MAX_LENGTH: usize = 1024 * 64;

/// Why picomatch threw instead of building a regular expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PicomatchError {
	/// `picomatch("")`: "Expected pattern to be a non-empty string".
	EmptyPattern,
	/// A pattern longer than 65,536 UTF-16 code units.
	TooLong(usize),
	/// `(?<` inside a group: "Node.js v10 or higher is required for regex
	/// lookbehinds", because the bundle's `process.version` is empty.
	Lookbehind,
	/// The pattern is the name of a property of `Object.prototype`, such as
	/// `constructor`. picomatch looks the pattern up in a plain object of
	/// replacements, gets the inherited function back, and fails calling a
	/// string method on it.
	InheritedReplacement,
}

impl fmt::Display for PicomatchError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::EmptyPattern => f.write_str("Expected pattern to be a non-empty string"),
			Self::TooLong(len) => write!(
				f,
				"Input length: {len}, exceeds maximum allowed length: {MAX_LENGTH}"
			),
			Self::Lookbehind => {
				f.write_str("Node.js v10 or higher is required for regex lookbehinds")
			}
			Self::InheritedReplacement => f.write_str("output.startsWith is not a function"),
		}
	}
}

/// The properties every plain JavaScript object inherits, which a lookup like
/// `REPLACEMENTS[input]` finds.
const OBJECT_PROTOTYPE: [&str; 12] = [
	"constructor",
	"__defineGetter__",
	"__defineSetter__",
	"hasOwnProperty",
	"__lookupGetter__",
	"__lookupSetter__",
	"isPrototypeOf",
	"propertyIsEnumerable",
	"toString",
	"valueOf",
	"__proto__",
	"toLocaleString",
];

/// What `String(Object.prototype[name])` gives, for the inherited names.
fn inherited_to_string(name: &str) -> Option<String> {
	match name {
		"__proto__" => Some("[object Object]".to_owned()),
		"constructor" => Some("function Object() { [native code] }".to_owned()),
		_ if OBJECT_PROTOTYPE.contains(&name) => {
			Some(format!("function {name}() {{ [native code] }}"))
		}
		_ => None,
	}
}

/// The regex source `picomatch.makeRe(pattern)` hands to `new RegExp`, anchors
/// and negation included.
pub(crate) fn regex_source(pattern: &str) -> Result<Units, PicomatchError> {
	if pattern.is_empty() {
		return Err(PicomatchError::EmptyPattern);
	}
	let input = u(pattern);
	let (output, negated) = match fastpaths(&input)? {
		Some(output) => (output, false),
		None => {
			let state = parse(&input)?;
			(state.output, state.negated)
		}
	};
	let mut source = u("^(?:");
	source.extend(output);
	source.extend(u(")$"));
	if negated {
		let mut negated = u("^(?!");
		negated.extend(source);
		negated.extend(u(").*$"));
		source = negated;
	}
	Ok(source)
}

fn check_length(input: &[u16]) -> Result<(), PicomatchError> {
	if input.len() > MAX_LENGTH {
		return Err(PicomatchError::TooLong(input.len()));
	}
	Ok(())
}

/// `REPLACEMENTS[input] || input`.
fn replacement(input: &[u16]) -> Result<Units, PicomatchError> {
	let text = String::from_utf16_lossy(input);
	match text.as_str() {
		"***" => Ok(u("*")),
		"**/**" | "**/**/**" => Ok(u("**")),
		_ if OBJECT_PROTOTYPE.contains(&text.as_str()) => Err(PicomatchError::InheritedReplacement),
		_ => Ok(input.to_vec()),
	}
}

/// `utils.removePrefix`: drops a leading `./`.
fn remove_prefix(input: &[u16]) -> &[u16] {
	input.strip_prefix(&u("./")[..]).unwrap_or(input)
}

/// `parse.fastpaths`, which picomatch tries first for patterns starting with
/// `.` or `*`. `None` means no shortcut applies.
fn fastpaths(input: &[u16]) -> Result<Option<Units>, PicomatchError> {
	if input.first() != Some(&(b'.' as u16)) && input.first() != Some(&(b'*' as u16)) {
		return Ok(None);
	}
	check_length(input)?;
	// Every replacement key starts with `*`, so the inherited names cannot
	// reach this lookup.
	let input = replacement(input)?;
	let source = create_fastpath(remove_prefix(&input));
	Ok(source.map(|mut source| {
		source.extend(u(SLASH_LITERAL));
		source.push(b'?' as u16);
		source
	}))
}

fn create_fastpath(input: &[u16]) -> Option<Units> {
	let s = |parts: &[&str]| -> Option<Units> { Some(u(&parts.concat())) };
	match String::from_utf16_lossy(input).as_str() {
		"*" => s(&[NO_DOT, ONE_CHAR, STAR]),
		".*" => s(&[DOT_LITERAL, ONE_CHAR, STAR]),
		"*.*" => s(&[NO_DOT, STAR, DOT_LITERAL, ONE_CHAR, STAR]),
		"*/*" => s(&[NO_DOT, STAR, SLASH_LITERAL, ONE_CHAR, NO_DOT, STAR]),
		"**" => s(&[NO_DOT, GLOBSTAR]),
		"**/*" => s(&[
			"(?:",
			NO_DOT,
			GLOBSTAR,
			SLASH_LITERAL,
			")?",
			NO_DOT,
			ONE_CHAR,
			STAR,
		]),
		"**/*.*" => s(&[
			"(?:",
			NO_DOT,
			GLOBSTAR,
			SLASH_LITERAL,
			")?",
			NO_DOT,
			STAR,
			DOT_LITERAL,
			ONE_CHAR,
			STAR,
		]),
		"**/.*" => s(&[
			"(?:",
			NO_DOT,
			GLOBSTAR,
			SLASH_LITERAL,
			")?",
			DOT_LITERAL,
			ONE_CHAR,
			STAR,
		]),
		_ => {
			// `/^(.*?)\.(\w+)$/`: the part after the last dot must be word
			// characters, and `.` stops at line terminators.
			let dot = input.iter().rposition(|&c| c == b'.' as u16)?;
			let word = &input[dot + 1..];
			if word.is_empty()
				|| !word.iter().all(|&c| is_word(c))
				|| input[..dot].iter().any(|&c| is_line_terminator(c))
			{
				return None;
			}
			let mut source = create_fastpath(&input[..dot])?;
			source.extend(u(DOT_LITERAL));
			source.extend_from_slice(word);
			Some(source)
		}
	}
}

/// `\w` outside unicode mode.
fn is_word(c: u16) -> bool {
	u8::try_from(c).is_ok_and(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn is_line_terminator(c: u16) -> bool {
	matches!(c, 0x0a | 0x0d | 0x2028 | 0x2029)
}

/// `utils.escapeRegex`.
fn escape_regex(input: &[u16]) -> Units {
	let mut out = Units::with_capacity(input.len());
	for &c in input {
		if is_regex_char(c) {
			out.push(b'\\' as u16);
		}
		out.push(c);
	}
	out
}

/// `/[-*+?.^${}(|)[\]]/`
fn is_regex_char(c: u16) -> bool {
	u8::try_from(c).is_ok_and(|b| b"-*+?.^${}(|)[]".contains(&b))
}

/// `utils.escapeLast`: escapes the last `char` not already preceded by a
/// backslash.
fn escape_last(input: &[u16], char: u8, last_index: Option<usize>) -> Units {
	let end = last_index.map_or(input.len(), |i| (i + 1).min(input.len()));
	let Some(index) = input[..end].iter().rposition(|&c| c == char as u16) else {
		return input.to_vec();
	};
	if index > 0 && input[index - 1] == b'\\' as u16 {
		return escape_last(input, char, Some(index - 1));
	}
	let mut out = input[..index].to_vec();
	out.push(b'\\' as u16);
	out.extend_from_slice(&input[index..]);
	out
}

/// JavaScript's `string.slice(0, -n)`: `-0` is `0`, so a zero `n` empties the
/// string.
fn slice_off(s: &mut Units, n: usize) {
	if n == 0 || n > s.len() {
		s.clear();
	} else {
		s.truncate(s.len() - n);
	}
}

fn contains(haystack: &[u16], needle: u8) -> bool {
	haystack.contains(&(needle as u16))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
	Bos,
	Text,
	Paren,
	Bracket,
	Brace,
	Comma,
	Slash,
	Dot,
	Dots,
	Qmark,
	Star,
	Globstar,
	Plus,
	At,
	Negate,
	MaybeSlash,
}

struct Token {
	kind: Kind,
	value: Units,
	/// `undefined` in picomatch when `None`; an empty output is kept apart
	/// because `token.output != null` and `token.output || token.value` treat
	/// the two differently.
	output: Option<Units>,
	prev: usize,
	extglob: bool,
	posix: bool,
	star: bool,
	comma: bool,
	dots: bool,
	output_index: usize,
	tokens_index: usize,
}

impl Token {
	fn new(kind: Kind, value: Units, output: Option<Units>) -> Self {
		Token {
			kind,
			value,
			output,
			prev: 0,
			extglob: false,
			posix: false,
			star: false,
			comma: false,
			dots: false,
			output_index: 0,
			tokens_index: 0,
		}
	}

	fn output_or_value(&self) -> &Units {
		self.output.as_ref().unwrap_or(&self.value)
	}
}

/// An open extglob such as `!(`, with the parts `extglobOpen` records.
struct Extglob {
	kind: Kind,
	close: &'static str,
	inner: Units,
	parens: isize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Counter {
	Parens,
	Brackets,
	Braces,
}

struct ParseState {
	output: Units,
	negated: bool,
}

struct Parser<'a> {
	input: &'a [u16],
	/// Every token ever created; `tokens` and `prev` index into it.
	arena: Vec<Token>,
	/// `state.tokens`, in order.
	tokens: Vec<usize>,
	prev: usize,
	index: isize,
	start: isize,
	output: Units,
	negated: bool,
	backtrack: bool,
	// Signed, as in picomatch: a `)` with no `(` open takes `parens` to -1,
	// and later decisions read the negative count.
	brackets: isize,
	braces: isize,
	parens: isize,
	quotes: usize,
	extglobs: Vec<Extglob>,
	brace_stack: Vec<usize>,
	stack: Vec<Counter>,
}

const BOS: usize = 0;

/// picomatch's `parse` with no options.
fn parse(raw: &[u16]) -> Result<ParseState, PicomatchError> {
	// `REPLACEMENTS[input] || input` runs before the length check.
	let replaced = replacement(raw)?;
	check_length(&replaced)?;
	let input = remove_prefix(&replaced);
	let mut parser = Parser {
		input,
		arena: vec![Token::new(Kind::Bos, Units::new(), Some(Units::new()))],
		tokens: vec![BOS],
		prev: BOS,
		index: -1,
		start: 0,
		output: Units::new(),
		negated: false,
		backtrack: false,
		brackets: 0,
		braces: 0,
		parens: 0,
		quotes: 0,
		extglobs: Vec::new(),
		brace_stack: Vec::new(),
		stack: Vec::new(),
	};
	parser.run()?;
	Ok(ParseState {
		output: parser.output,
		negated: parser.negated,
	})
}

impl<'a> Parser<'a> {
	fn len(&self) -> isize {
		self.input.len() as isize
	}

	fn eos(&self) -> bool {
		self.index == self.len() - 1
	}

	fn peek(&self, n: isize) -> Option<u16> {
		let at = self.index + n;
		if at < 0 {
			return None;
		}
		self.input.get(at as usize).copied()
	}

	fn peek_is(&self, n: isize, c: u8) -> bool {
		self.peek(n) == Some(c as u16)
	}

	fn advance(&mut self) -> Units {
		self.index += 1;
		self.input
			.get(self.index as usize)
			.map(|&c| vec![c])
			.unwrap_or_default()
	}

	fn remaining(&self) -> &'a [u16] {
		let from = (self.index + 1).max(0) as usize;
		self.input.get(from..).unwrap_or(&[])
	}

	fn append(&mut self, value: &Units, output: Option<&Units>) {
		self.output.extend_from_slice(output.unwrap_or(value));
	}

	fn increment(&mut self, counter: Counter) {
		*self.counter(counter) += 1;
		self.stack.push(counter);
	}

	fn decrement(&mut self, counter: Counter) {
		*self.counter(counter) -= 1;
		self.stack.pop();
	}

	fn counter(&mut self, counter: Counter) -> &mut isize {
		match counter {
			Counter::Parens => &mut self.parens,
			Counter::Brackets => &mut self.brackets,
			Counter::Braces => &mut self.braces,
		}
	}

	fn push(&mut self, mut tok: Token) {
		let prev = self.prev;
		if self.arena[prev].kind == Kind::Globstar {
			let is_brace = self.braces > 0 && matches!(tok.kind, Kind::Comma | Kind::Brace);
			let is_extglob = tok.extglob || (!self.extglobs.is_empty() && tok.kind == Kind::Paren);
			if tok.kind != Kind::Slash && tok.kind != Kind::Paren && !is_brace && !is_extglob {
				let len = self.arena[prev].output_or_value().len();
				slice_off(&mut self.output, len);
				let prev_token = &mut self.arena[prev];
				prev_token.kind = Kind::Star;
				prev_token.value = u("*");
				prev_token.output = Some(u(STAR));
				self.output.extend(u(STAR));
			}
		}
		if let Some(extglob) = self.extglobs.last_mut()
			&& tok.kind != Kind::Paren
		{
			extglob.inner.extend_from_slice(&tok.value);
		}
		let has_content =
			!tok.value.is_empty() || tok.output.as_ref().is_some_and(|o| !o.is_empty());
		if has_content {
			let value = tok.value.clone();
			self.append(&value, tok.output.as_ref());
		}
		if self.arena[prev].kind == Kind::Text && tok.kind == Kind::Text {
			let prev_token = &mut self.arena[prev];
			prev_token.value.extend_from_slice(&tok.value);
			let mut output = prev_token.output.take().unwrap_or_default();
			output.extend_from_slice(&tok.value);
			prev_token.output = Some(output);
			return;
		}
		tok.prev = prev;
		self.arena.push(tok);
		let index = self.arena.len() - 1;
		self.tokens.push(index);
		self.prev = index;
	}

	fn extglob_open(&mut self, kind: Kind, value: Units) {
		let close = match kind {
			Kind::Negate => "))[^/]*?)",
			Kind::Qmark => ")?",
			Kind::Plus => ")+",
			Kind::Star => ")*",
			_ => ")",
		};
		let open = if kind == Kind::Negate {
			"(?:(?!(?:"
		} else {
			"(?:"
		};
		let extglob = Extglob {
			kind,
			close,
			inner: Units::new(),
			parens: self.parens,
		};
		self.increment(Counter::Parens);
		let output = if self.output.is_empty() {
			u(ONE_CHAR)
		} else {
			Units::new()
		};
		self.push(Token::new(kind, value, Some(output)));
		let paren_value = self.advance();
		let mut paren = Token::new(Kind::Paren, paren_value, Some(u(open)));
		paren.extglob = true;
		self.push(paren);
		self.extglobs.push(extglob);
	}

	fn extglob_close(&mut self, extglob: Extglob, value: Units) {
		let mut output = u(extglob.close);
		if extglob.kind == Kind::Negate {
			let mut extglob_star = u(STAR);
			if extglob.inner.len() > 1 && contains(&extglob.inner, b'/') {
				extglob_star = u(GLOBSTAR);
			}
			let rest = self.remaining();
			let only_parens = !rest.is_empty() && rest.iter().all(|&c| c == b')' as u16);
			if extglob_star != u(STAR) || self.eos() || only_parens {
				output = u(")$))");
				output.extend_from_slice(&extglob_star);
			}
			let rest = self.remaining();
			// `/^\.[^\\/.]+$/`
			let dotted_extension = rest.len() > 1
				&& rest[0] == b'.' as u16
				&& rest[1..]
					.iter()
					.all(|&c| c != b'\\' as u16 && c != b'/' as u16 && c != b'.' as u16);
			if contains(&extglob.inner, b'*') && dotted_extension {
				output = u(")");
				output.extend_from_slice(rest);
				output.push(b')' as u16);
				output.extend_from_slice(&extglob_star);
				output.push(b')' as u16);
			}
		}
		let mut paren = Token::new(Kind::Paren, value, Some(output));
		paren.extglob = true;
		self.push(paren);
		self.decrement(Counter::Parens);
	}

	fn run(&mut self) -> Result<(), PicomatchError> {
		if let Some(output) = self.fast_path() {
			self.output = output;
			return Ok(());
		}
		while !self.eos() {
			let mut value = self.advance();
			let c = value[0];
			if c == 0 {
				continue;
			}
			if c == b'\\' as u16 {
				let next = self.peek(1);
				if next == Some(b'/' as u16) {
					continue;
				}
				if next == Some(b'.' as u16) || next == Some(b';' as u16) {
					continue;
				}
				if next.is_none() {
					value.push(b'\\' as u16);
					self.push(Token::new(Kind::Text, value, None));
					continue;
				}
				let slashes = self
					.remaining()
					.iter()
					.take_while(|&&c| c == b'\\' as u16)
					.count();
				if slashes > 2 {
					self.index += slashes as isize;
					if slashes % 2 != 0 {
						value.push(b'\\' as u16);
					}
				}
				let next = self.advance();
				value.extend(next);
				if self.brackets == 0 {
					self.push(Token::new(Kind::Text, value, None));
					continue;
				}
			}
			let prev = self.prev;
			let prev_value_is = |p: &Self, s: &str| p.arena[prev].value == u(s);
			if self.brackets > 0
				&& (value != u("]") || prev_value_is(self, "[") || prev_value_is(self, "[^"))
			{
				if value == u(":") {
					let prev_value = self.arena[prev].value.clone();
					let inner = &prev_value[1.min(prev_value.len())..];
					if contains(inner, b'[') {
						self.arena[prev].posix = true;
						if contains(inner, b':') {
							let idx = prev_value
								.iter()
								.rposition(|&c| c == b'[' as u16)
								.unwrap_or(0);
							let pre = &prev_value[..idx];
							let rest =
								String::from_utf16_lossy(prev_value.get(idx + 2..).unwrap_or(&[]));
							if let Some(posix) = posix_class(&rest) {
								let mut new_value = pre.to_vec();
								new_value.extend(u(&posix));
								self.arena[prev].value = new_value;
								self.backtrack = true;
								self.advance();
								if self.arena[BOS].output.as_ref().is_none_or(Vec::is_empty)
									&& self.tokens.iter().position(|&t| t == prev) == Some(1)
								{
									self.arena[BOS].output = Some(u(ONE_CHAR));
								}
								continue;
							}
						}
					}
				}
				if (value == u("[") && !self.peek_is(1, b':'))
					|| (value == u("-") && self.peek_is(1, b']'))
				{
					value.insert(0, b'\\' as u16);
				}
				if value == u("]") && (prev_value_is(self, "[") || prev_value_is(self, "[^")) {
					value.insert(0, b'\\' as u16);
				}
				self.arena[prev].value.extend_from_slice(&value);
				self.append(&value, None);
				continue;
			}
			if self.quotes == 1 && value != u("\"") {
				let escaped = escape_regex(&value);
				self.arena[prev].value.extend_from_slice(&escaped);
				self.append(&escaped, None);
				continue;
			}
			let c = value[0];
			if value == u("\"") {
				self.quotes = if self.quotes == 1 { 0 } else { 1 };
				continue;
			}
			if c == b'(' as u16 {
				self.increment(Counter::Parens);
				self.push(Token::new(Kind::Paren, value, None));
				continue;
			}
			if c == b')' as u16 {
				let parens = self.parens;
				if let Some(extglob) = self.extglobs.pop_if(|extglob| parens == extglob.parens + 1)
				{
					self.extglob_close(extglob, value);
					continue;
				}
				let output = if self.parens != 0 { u(")") } else { u("\\)") };
				self.push(Token::new(Kind::Paren, value, Some(output)));
				self.decrement(Counter::Parens);
				continue;
			}
			if c == b'[' as u16 {
				if !contains(self.remaining(), b']') {
					value = u("\\[");
				} else {
					self.increment(Counter::Brackets);
				}
				self.push(Token::new(Kind::Bracket, value, None));
				continue;
			}
			if c == b']' as u16 {
				let prev_token = &self.arena[prev];
				if prev_token.kind == Kind::Bracket && prev_token.value.len() == 1 {
					self.push(Token::new(Kind::Text, value, Some(u("\\]"))));
					continue;
				}
				if self.brackets == 0 {
					self.push(Token::new(Kind::Text, value, Some(u("\\]"))));
					continue;
				}
				self.decrement(Counter::Brackets);
				let prev_value =
					self.arena[prev].value[1.min(self.arena[prev].value.len())..].to_vec();
				if !self.arena[prev].posix
					&& prev_value.first() == Some(&(b'^' as u16))
					&& !contains(&prev_value, b'/')
				{
					value.insert(0, b'/' as u16);
				}
				self.arena[prev].value.extend_from_slice(&value);
				self.append(&value, None);
				if prev_value.iter().any(|&c| is_regex_char(c)) {
					continue;
				}
				let escaped = escape_regex(&self.arena[prev].value);
				let len = self.arena[prev].value.len();
				slice_off(&mut self.output, len);
				let mut new_value = u("(?:");
				new_value.extend_from_slice(&escaped);
				new_value.push(b'|' as u16);
				new_value.extend_from_slice(&self.arena[prev].value);
				new_value.push(b')' as u16);
				self.output.extend_from_slice(&new_value);
				self.arena[prev].value = new_value;
				continue;
			}
			if c == b'{' as u16 {
				self.increment(Counter::Braces);
				let mut open = Token::new(Kind::Brace, value, Some(u("(")));
				open.output_index = self.output.len();
				open.tokens_index = self.tokens.len();
				self.push(open);
				let index = self.arena.len() - 1;
				self.brace_stack.push(index);
				continue;
			}
			if c == b'}' as u16 {
				let Some(&brace) = self.brace_stack.last() else {
					self.push(Token::new(Kind::Text, value.clone(), Some(value)));
					continue;
				};
				let mut output = u(")");
				if self.arena[brace].dots {
					let mut range: Vec<Units> = Vec::new();
					while let Some(t) = self.tokens.pop() {
						if self.arena[t].kind == Kind::Brace {
							break;
						}
						if self.arena[t].kind != Kind::Dots {
							range.insert(0, self.arena[t].value.clone());
						}
					}
					output = expand_range(range);
					self.backtrack = true;
				}
				if !self.arena[brace].comma && !self.arena[brace].dots {
					let mut out = self.output
						[..self.arena[brace].output_index.min(self.output.len())]
						.to_vec();
					let toks: Vec<usize> = self
						.tokens
						.get(self.arena[brace].tokens_index..)
						.unwrap_or(&[])
						.to_vec();
					self.arena[brace].value = u("\\{");
					self.arena[brace].output = Some(u("\\{"));
					value = u("\\}");
					output = u("\\}");
					for t in toks {
						let token = &self.arena[t];
						let text = match &token.output {
							Some(o) if !o.is_empty() => o,
							_ => &token.value,
						};
						out.extend_from_slice(text);
					}
					self.output = out;
				}
				self.push(Token::new(Kind::Brace, value, Some(output)));
				self.decrement(Counter::Braces);
				self.brace_stack.pop();
				continue;
			}
			if c == b'|' as u16 {
				self.push(Token::new(Kind::Text, value, None));
				continue;
			}
			if c == b',' as u16 {
				let mut output = value.clone();
				if let Some(&brace) = self.brace_stack.last()
					&& self.stack.last() == Some(&Counter::Braces)
				{
					self.arena[brace].comma = true;
					output = u("|");
				}
				self.push(Token::new(Kind::Comma, value, Some(output)));
				continue;
			}
			if c == b'/' as u16 {
				if self.arena[prev].kind == Kind::Dot && self.index == self.start + 1 {
					self.start = self.index + 1;
					self.output.clear();
					self.tokens.pop();
					self.prev = BOS;
					continue;
				}
				self.push(Token::new(Kind::Slash, value, Some(u(SLASH_LITERAL))));
				continue;
			}
			if c == b'.' as u16 {
				if self.braces > 0 && self.arena[prev].kind == Kind::Dot {
					if self.arena[prev].value == u(".") {
						self.arena[prev].output = Some(u(DOT_LITERAL));
					}
					let brace = *self.brace_stack.last().expect("a brace is open");
					let prev_token = &mut self.arena[prev];
					prev_token.kind = Kind::Dots;
					prev_token
						.output
						.get_or_insert_with(Units::new)
						.extend_from_slice(&value);
					prev_token.value.extend_from_slice(&value);
					self.arena[brace].dots = true;
					continue;
				}
				if self.braces + self.parens == 0
					&& self.arena[prev].kind != Kind::Bos
					&& self.arena[prev].kind != Kind::Slash
				{
					self.push(Token::new(Kind::Text, value, Some(u(DOT_LITERAL))));
					continue;
				}
				self.push(Token::new(Kind::Dot, value, Some(u(DOT_LITERAL))));
				continue;
			}
			if c == b'?' as u16 {
				let is_group = self.arena[prev].value == u("(");
				if !is_group && self.peek_is(1, b'(') && !self.peek_is(2, b'?') {
					self.extglob_open(Kind::Qmark, value);
					continue;
				}
				if self.arena[prev].kind == Kind::Paren {
					let next = self.peek(1);
					if next == Some(b'<' as u16) {
						return Err(PicomatchError::Lookbehind);
					}
					let mut output = value.clone();
					let next_is_group_char =
						next.is_some_and(|c| u8::try_from(c).is_ok_and(|b| b"!=<:".contains(&b)));
					if self.arena[prev].value == u("(") && !next_is_group_char {
						output.insert(0, b'\\' as u16);
					}
					self.push(Token::new(Kind::Text, value, Some(output)));
					continue;
				}
				if matches!(self.arena[prev].kind, Kind::Slash | Kind::Bos) {
					self.push(Token::new(Kind::Qmark, value, Some(u(QMARK_NO_DOT))));
					continue;
				}
				self.push(Token::new(Kind::Qmark, value, Some(u(QMARK))));
				continue;
			}
			if c == b'!' as u16 {
				if self.peek_is(1, b'(') {
					let third_is_group_char = self
						.peek(3)
						.is_some_and(|c| u8::try_from(c).is_ok_and(|b| b"!=<:".contains(&b)));
					if !self.peek_is(2, b'?') || !third_is_group_char {
						self.extglob_open(Kind::Negate, value);
						continue;
					}
				}
				if self.index == 0 {
					self.negate();
					continue;
				}
			}
			if c == b'+' as u16 {
				if self.peek_is(1, b'(') && !self.peek_is(2, b'?') {
					self.extglob_open(Kind::Plus, value);
					continue;
				}
				if self.arena[prev].value == u("(") {
					self.push(Token::new(Kind::Plus, value, Some(u("\\+"))));
					continue;
				}
				if matches!(
					self.arena[prev].kind,
					Kind::Bracket | Kind::Paren | Kind::Brace
				) || self.parens > 0
				{
					self.push(Token::new(Kind::Plus, value, None));
					continue;
				}
				self.push(Token::new(Kind::Plus, u("\\+"), None));
				continue;
			}
			if c == b'@' as u16 {
				if self.peek_is(1, b'(') && !self.peek_is(2, b'?') {
					let mut at = Token::new(Kind::At, value, Some(Units::new()));
					at.extglob = true;
					self.push(at);
					continue;
				}
				self.push(Token::new(Kind::Text, value, None));
				continue;
			}
			if c != b'*' as u16 {
				if c == b'$' as u16 || c == b'^' as u16 {
					value.insert(0, b'\\' as u16);
				}
				let run = self
					.remaining()
					.iter()
					.take_while(|&&c| !is_non_special_stop(c))
					.count();
				if run > 0 {
					value.extend_from_slice(&self.remaining()[..run]);
					self.index += run as isize;
				}
				self.push(Token::new(Kind::Text, value, None));
				continue;
			}
			if self.arena[prev].kind == Kind::Globstar || self.arena[prev].star {
				let prev_token = &mut self.arena[prev];
				prev_token.kind = Kind::Star;
				prev_token.star = true;
				prev_token.value.extend_from_slice(&value);
				prev_token.output = Some(u(STAR));
				self.backtrack = true;
				continue;
			}
			let mut rest = self.remaining().to_vec();
			// `/^\([^?]/`
			if rest.len() > 1 && rest[0] == b'(' as u16 && rest[1] != b'?' as u16 {
				self.extglob_open(Kind::Star, value);
				continue;
			}
			if self.arena[prev].kind == Kind::Star {
				let prior = self.arena[prev].prev;
				let before = self.arena[prior].prev;
				let prior_kind = self.arena[prior].kind;
				let is_start = matches!(prior_kind, Kind::Slash | Kind::Bos);
				let after_star =
					prior != BOS && matches!(self.arena[before].kind, Kind::Star | Kind::Globstar);
				let is_brace = self.braces > 0 && matches!(prior_kind, Kind::Comma | Kind::Brace);
				let is_extglob = !self.extglobs.is_empty() && prior_kind == Kind::Paren;
				if !is_start && prior_kind != Kind::Paren && !is_brace && !is_extglob {
					self.push(Token::new(Kind::Star, value, Some(Units::new())));
					continue;
				}
				while rest.starts_with(&u("/**")) {
					let after = self.input.get((self.index + 4) as usize).copied();
					if after.is_some_and(|c| c != b'/' as u16) {
						break;
					}
					rest.drain(..3);
					self.index += 3;
				}
				if prior_kind == Kind::Bos && self.eos() {
					let prev_token = &mut self.arena[prev];
					prev_token.kind = Kind::Globstar;
					prev_token.value.extend_from_slice(&value);
					prev_token.output = Some(u(GLOBSTAR));
					self.output = u(GLOBSTAR);
					continue;
				}
				let prior_prev_is_bos = self.arena[self.arena[prior].prev].kind == Kind::Bos;
				if prior_kind == Kind::Slash && !prior_prev_is_bos && !after_star && self.eos() {
					let cut = self.arena[prior].output_or_value().len()
						+ self.arena[prev].output_or_value().len();
					slice_off(&mut self.output, cut);
					let mut prior_output = u("(?:");
					prior_output.extend_from_slice(self.arena[prior].output_or_value());
					self.arena[prior].output = Some(prior_output.clone());
					let prev_output = u(&format!("{GLOBSTAR}|$)"));
					let prev_token = &mut self.arena[prev];
					prev_token.kind = Kind::Globstar;
					prev_token.output = Some(prev_output.clone());
					prev_token.value.extend_from_slice(&value);
					self.output.extend(prior_output);
					self.output.extend(prev_output);
					continue;
				}
				if prior_kind == Kind::Slash
					&& !prior_prev_is_bos
					&& rest.first() == Some(&(b'/' as u16))
				{
					let end = if rest.len() > 1 { "|$" } else { "" };
					let cut = self.arena[prior].output_or_value().len()
						+ self.arena[prev].output_or_value().len();
					slice_off(&mut self.output, cut);
					let mut prior_output = u("(?:");
					prior_output.extend_from_slice(self.arena[prior].output_or_value());
					self.arena[prior].output = Some(prior_output.clone());
					let prev_output =
						u(&format!("{GLOBSTAR}{SLASH_LITERAL}|{SLASH_LITERAL}{end})"));
					let prev_token = &mut self.arena[prev];
					prev_token.kind = Kind::Globstar;
					prev_token.output = Some(prev_output.clone());
					prev_token.value.extend_from_slice(&value);
					self.output.extend(prior_output);
					self.output.extend(prev_output);
					self.advance();
					self.push(Token::new(Kind::Slash, u("/"), Some(Units::new())));
					continue;
				}
				if prior_kind == Kind::Bos && rest.first() == Some(&(b'/' as u16)) {
					let prev_output =
						u(&format!("(?:^|{SLASH_LITERAL}|{GLOBSTAR}{SLASH_LITERAL})"));
					let prev_token = &mut self.arena[prev];
					prev_token.kind = Kind::Globstar;
					prev_token.value.extend_from_slice(&value);
					prev_token.output = Some(prev_output.clone());
					self.output = prev_output;
					self.advance();
					self.push(Token::new(Kind::Slash, u("/"), Some(Units::new())));
					continue;
				}
				let len = self.arena[prev].output_or_value().len();
				slice_off(&mut self.output, len);
				let prev_token = &mut self.arena[prev];
				prev_token.kind = Kind::Globstar;
				prev_token.output = Some(u(GLOBSTAR));
				prev_token.value.extend_from_slice(&value);
				self.output.extend(u(GLOBSTAR));
				continue;
			}
			let token = Token::new(Kind::Star, value, Some(u(STAR)));
			let prev_kind = self.arena[prev].kind;
			if self.index == self.start || prev_kind == Kind::Slash || prev_kind == Kind::Dot {
				let guard = if prev_kind == Kind::Dot {
					NO_DOT_SLASH
				} else {
					NO_DOT
				};
				self.output.extend(u(guard));
				self.arena[prev]
					.output
					.get_or_insert_with(Units::new)
					.extend(u(guard));
				if !self.peek_is(1, b'*') {
					self.output.extend(u(ONE_CHAR));
					self.arena[prev]
						.output
						.get_or_insert_with(Units::new)
						.extend(u(ONE_CHAR));
				}
			}
			self.push(token);
		}
		while self.brackets > 0 {
			self.output = escape_last(&self.output, b'[', None);
			self.decrement(Counter::Brackets);
		}
		while self.parens > 0 {
			self.output = escape_last(&self.output, b'(', None);
			self.decrement(Counter::Parens);
		}
		while self.braces > 0 {
			self.output = escape_last(&self.output, b'{', None);
			self.decrement(Counter::Braces);
		}
		if matches!(self.arena[self.prev].kind, Kind::Star | Kind::Bracket) {
			self.push(Token::new(
				Kind::MaybeSlash,
				Units::new(),
				Some(u(&format!("{SLASH_LITERAL}?"))),
			));
		}
		if self.backtrack {
			let mut output = Units::new();
			for &t in &self.tokens {
				output.extend_from_slice(self.arena[t].output_or_value());
			}
			self.output = output;
		}
		Ok(())
	}

	/// `negate()`: a run of leading `!` negates the pattern when it is odd.
	fn negate(&mut self) {
		let mut count = 1;
		while self.peek_is(1, b'!') && (!self.peek_is(2, b'(') || self.peek_is(3, b'?')) {
			self.advance();
			self.start += 1;
			count += 1;
		}
		if count % 2 == 0 {
			return;
		}
		self.negated = true;
		self.start += 1;
	}

	/// The fast path at the top of `parse`, for patterns with no slash,
	/// bracket, brace, parenthesis or quote that start with neither `*` nor
	/// `!`. Its output is wrapped in anchors here, and wrapped again by
	/// `compileRe`.
	fn fast_path(&self) -> Option<Units> {
		let input = self.input;
		let starts_special =
			matches!(input.first(), Some(&c) if c == b'*' as u16 || c == b'!' as u16);
		let has_special = input
			.iter()
			.any(|&c| u8::try_from(c).is_ok_and(|b| b"/()[]{}\"".contains(&b)));
		if starts_special || has_special {
			return None;
		}
		let mut backslashes = false;
		let mut output = Units::new();
		let mut i = 0;
		// `input.replace(/(\\?)((\W)(\3*))/g, ...)`
		while i < input.len() {
			let bs = b'\\' as u16;
			let (esc, first_at) =
				if input[i] == bs && input.get(i + 1).is_some_and(|&c| !is_word(c)) {
					(true, i + 1)
				} else if !is_word(input[i]) {
					(false, i)
				} else {
					output.push(input[i]);
					i += 1;
					continue;
				};
			let first = input[first_at];
			let run = input[first_at + 1..]
				.iter()
				.take_while(|&&c| c == first)
				.count();
			let end = first_at + 1 + run;
			let whole = &input[i..end];
			let index = i;
			if first == bs {
				backslashes = true;
				output.extend_from_slice(whole);
			} else if first == b'?' as u16 {
				if esc {
					output.push(bs);
					output.push(first);
					for _ in 0..run {
						output.extend(u(QMARK));
					}
				} else if index == 0 {
					output.extend(u(QMARK_NO_DOT));
					for _ in 0..run {
						output.extend(u(QMARK));
					}
				} else {
					for _ in 0..=run {
						output.extend(u(QMARK));
					}
				}
			} else if first == b'.' as u16 {
				for _ in 0..=run {
					output.extend(u(DOT_LITERAL));
				}
			} else if first == b'*' as u16 {
				if esc {
					output.push(bs);
					output.push(first);
					if run > 0 {
						output.extend(u(STAR));
					}
				} else {
					output.extend(u(STAR));
				}
			} else if esc {
				output.extend_from_slice(whole);
			} else {
				output.push(bs);
				output.extend_from_slice(whole);
			}
			i = end;
		}
		if backslashes {
			// `/\\+/g`: an even run becomes two backslashes, an odd run one.
			let mut collapsed = Units::new();
			let mut i = 0;
			while i < output.len() {
				if output[i] != b'\\' as u16 {
					collapsed.push(output[i]);
					i += 1;
					continue;
				}
				let run = output[i..]
					.iter()
					.take_while(|&&c| c == b'\\' as u16)
					.count();
				collapsed.push(b'\\' as u16);
				if run % 2 == 0 {
					collapsed.push(b'\\' as u16);
				}
				i += run;
			}
			output = collapsed;
		}
		let mut wrapped = u("^(?:");
		wrapped.extend(output);
		wrapped.extend(u(")$"));
		Some(wrapped)
	}
}

/// The characters `REGEX_NON_SPECIAL_CHARS` (`/^[^@![\].,$*+?^{}()|\\/]+/`)
/// stops at.
fn is_non_special_stop(c: u16) -> bool {
	u8::try_from(c).is_ok_and(|b| b"@![].,$*+?^{}()|\\/".contains(&b))
}

/// `POSIX_REGEX_SOURCE[name]`, a plain object, so the inherited names find
/// functions and objects whose string forms land in the output.
fn posix_class(name: &str) -> Option<String> {
	let class = match name {
		"alnum" => "a-zA-Z0-9",
		"alpha" => "a-zA-Z",
		"ascii" => "\\x00-\\x7F",
		"blank" => " \\t",
		"cntrl" => "\\x00-\\x1F\\x7F",
		"digit" => "0-9",
		"graph" => "\\x21-\\x7E",
		"lower" => "a-z",
		"print" => "\\x20-\\x7E ",
		"punct" => "\\-!\"#$%&'()\\*+,./:;<=>?@[\\]^_`{|}~",
		"space" => " \\t\\r\\n\\v\\f",
		"upper" => "A-Z",
		"word" => "A-Za-z0-9_",
		"xdigit" => "A-Fa-f0-9",
		_ => return inherited_to_string(name),
	};
	Some(class.to_owned())
}

/// `expandRange` for a brace range such as `{a..c}`: `[a-c]` when that is a
/// valid character class, and the escaped ends joined by `..` when it is not.
fn expand_range(mut args: Vec<Units>) -> Units {
	args.sort();
	let mut value = u("[");
	for (i, arg) in args.iter().enumerate() {
		if i > 0 {
			value.push(b'-' as u16);
		}
		value.extend_from_slice(arg);
	}
	value.push(b']' as u16);
	if super::compiles(&value) {
		return value;
	}
	let mut out = Units::new();
	for (i, arg) in args.iter().enumerate() {
		if i > 0 {
			out.extend(u(".."));
		}
		out.extend(escape_regex(arg));
	}
	out
}
