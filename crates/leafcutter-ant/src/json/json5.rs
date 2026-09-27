//! A port of json5 2.2.1's `lib/parse.js`, the version TS Dash 0.13.0 bundles.
//!
//! The lexer keeps json5's states and reads characters in the same order,
//! because Dash writes json5's error messages into output files (see
//! `Components/Plugins.ts`), so the line and column in them are output too.
//! Columns count UTF-16 code units, and only `\n` starts a new line.

use std::fmt;

use super::json5_unicode::{ID_CONTINUE, ID_START, SPACE_SEPARATOR};
use super::{Array, Object, Value};

/// Reads `source` as json5 2.2.1 does, including its quirks (the quirk
/// ledger in the README lists them):
///
/// - A `"__proto__"` key is dropped. json5 2.2.1 assigns it, which sets the
///   object's prototype instead of creating a property.
/// - A repeated key keeps the position of its first occurrence and the value
///   of its last.
/// - A `\u` escape that leaves a lone surrogate becomes U+FFFD, because a Rust
///   string cannot hold a lone surrogate.
///
/// Parsing is iterative, so any nesting depth is accepted, as json5 accepts
/// it.
pub fn parse_json5(source: &str) -> Result<Value, Json5Error> {
	Parser::new(source).parse()
}

/// Why json5 refused its input, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Json5Error {
	kind: Json5ErrorKind,
	line: usize,
	column: usize,
}

/// The three refusals json5 has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Json5ErrorKind {
	/// A character that cannot appear where it did.
	InvalidCharacter(char),
	/// The input ended inside a value, a string, a comment or an escape.
	InvalidEndOfInput,
	/// A `\u` escape in an unquoted key named a character a key cannot hold.
	InvalidIdentifierCharacter,
}

impl Json5Error {
	/// What was wrong.
	pub fn kind(&self) -> Json5ErrorKind {
		self.kind
	}

	/// The line json5 reports, starting at 1.
	pub fn line(&self) -> usize {
		self.line
	}

	/// The column json5 reports, in UTF-16 code units. It is the column after
	/// the offending character, and for
	/// [`Json5ErrorKind::InvalidIdentifierCharacter`] it is 5 less than that,
	/// as json5 computes it.
	pub fn column(&self) -> usize {
		self.column
	}
}

/// The message is json5's own, character for character, since Dash can write
/// it into an output file.
impl fmt::Display for Json5Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let (line, column) = (self.line, self.column);
		match &self.kind {
			Json5ErrorKind::InvalidCharacter(c) => write!(
				f,
				"JSON5: invalid character '{}' at {line}:{column}",
				FormatChar(*c)
			),
			Json5ErrorKind::InvalidEndOfInput => {
				write!(f, "JSON5: invalid end of input at {line}:{column}")
			}
			Json5ErrorKind::InvalidIdentifierCharacter => {
				write!(f, "JSON5: invalid identifier character at {line}:{column}")
			}
		}
	}
}

impl std::error::Error for Json5Error {}

/// json5's `formatChar`.
struct FormatChar(char);

impl fmt::Display for FormatChar {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let replacement = match self.0 {
			'\'' => "\\'",
			'"' => "\\\"",
			'\\' => "\\\\",
			'\u{8}' => "\\b",
			'\u{c}' => "\\f",
			'\n' => "\\n",
			'\r' => "\\r",
			'\t' => "\\t",
			'\u{b}' => "\\v",
			'\0' => "\\0",
			'\u{2028}' => "\\u2028",
			'\u{2029}' => "\\u2029",
			c if c < ' ' => return write!(f, "\\x{:02x}", u32::from(c)),
			c => return write!(f, "{c}"),
		};
		f.write_str(replacement)
	}
}

/// json5's `parseState`, which also picks the lexer state a token starts in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParseState {
	Start,
	BeforePropertyName,
	AfterPropertyName,
	BeforePropertyValue,
	AfterPropertyValue,
	BeforeArrayValue,
	AfterArrayValue,
	End,
}

/// json5's `lexState`.
#[derive(Clone, Copy)]
enum LexState {
	Default,
	Comment,
	MultiLineComment,
	MultiLineCommentAsterisk,
	SingleLineComment,
	Value,
	IdentifierNameStartEscape,
	IdentifierName,
	IdentifierNameEscape,
	Sign,
	Zero,
	DecimalInteger,
	DecimalPointLeading,
	DecimalPoint,
	DecimalFraction,
	DecimalExponent,
	DecimalExponentSign,
	DecimalExponentInteger,
	Hexadecimal,
	HexadecimalInteger,
	String,
}

enum Token {
	Punctuator(char),
	Null,
	Bool(bool),
	Number(f64),
	String(String),
	Identifier(String),
	Eof,
}

/// An array or object that has been opened and not yet closed, with the key
/// it will be stored under in its parent.
struct Open {
	key: Option<String>,
	container: Container,
}

enum Container {
	Array(Array),
	Object(Object),
}

struct Parser<'a> {
	source: &'a str,
	pos: usize,
	line: usize,
	column: usize,
	parse_state: ParseState,
	stack: Vec<Open>,
	key: Option<String>,
	root: Option<Value>,
	buffer: Text,
	double_quote: bool,
	negative: bool,
}

type Step<T> = Result<Option<T>, Json5Error>;

impl<'a> Parser<'a> {
	fn new(source: &'a str) -> Self {
		Self {
			source,
			pos: 0,
			line: 1,
			column: 0,
			parse_state: ParseState::Start,
			stack: Vec::new(),
			key: None,
			root: None,
			buffer: Text::default(),
			double_quote: false,
			negative: false,
		}
	}

	fn parse(mut self) -> Result<Value, Json5Error> {
		loop {
			let token = self.lex()?;
			let eof = matches!(token, Token::Eof);
			self.parse_token(token)?;
			if eof {
				break;
			}
		}
		// The loop ends only on the eof token, which every state but `End`
		// refuses, and `End` is reached only once the root is stored.
		Ok(self
			.root
			.take()
			.expect("json5 reaches the end state only after storing the root"))
	}

	fn peek(&self) -> Option<char> {
		self.source[self.pos..].chars().next()
	}

	fn read(&mut self) -> Option<char> {
		let c = self.peek();
		match c {
			Some('\n') => {
				self.line += 1;
				self.column = 0;
			}
			Some(c) => self.column += c.len_utf16(),
			None => self.column += 1,
		}
		if let Some(c) = c {
			self.pos += c.len_utf8();
		}
		c
	}

	/// json5's `invalidChar(read())`.
	fn invalid_char(&mut self) -> Json5Error {
		let kind = match self.read() {
			Some(c) => Json5ErrorKind::InvalidCharacter(c),
			None => Json5ErrorKind::InvalidEndOfInput,
		};
		self.error(kind)
	}

	fn invalid_eof(&self) -> Json5Error {
		self.error(Json5ErrorKind::InvalidEndOfInput)
	}

	fn invalid_identifier(&mut self) -> Json5Error {
		self.column -= 5;
		self.error(Json5ErrorKind::InvalidIdentifierCharacter)
	}

	fn error(&self, kind: Json5ErrorKind) -> Json5Error {
		Json5Error {
			kind,
			line: self.line,
			column: self.column,
		}
	}

	fn lex(&mut self) -> Result<Token, Json5Error> {
		let mut state = LexState::Default;
		self.buffer = Text::default();
		self.double_quote = false;
		self.negative = false;
		loop {
			let c = self.peek();
			if let Some(token) = self.lex_step(&mut state, c)? {
				return Ok(token);
			}
		}
	}

	fn number(&mut self, value: f64) -> Step<Token> {
		Ok(Some(Token::Number(if self.negative {
			-value
		} else {
			value
		})))
	}

	fn buffer_number(&mut self) -> Step<Token> {
		let text = self.buffer.take();
		let value = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
			Some(digits) => hex_to_f64(digits),
			// The lexer only lets through digits, one '.', and an exponent,
			// all of which Rust's float grammar accepts, correctly rounded,
			// as JavaScript's Number() does.
			None => text.parse::<f64>().unwrap_or(f64::NAN),
		};
		self.number(value)
	}

	/// One call of json5's `lexStates[lexState]()`.
	fn lex_step(&mut self, state: &mut LexState, c: Option<char>) -> Step<Token> {
		match *state {
			LexState::Default => match c {
				Some(
					'\t' | '\u{b}' | '\u{c}' | ' ' | '\u{a0}' | '\u{feff}' | '\n' | '\r'
					| '\u{2028}' | '\u{2029}',
				) => {
					self.read();
					Ok(None)
				}
				Some('/') => {
					self.read();
					*state = LexState::Comment;
					Ok(None)
				}
				None => {
					self.read();
					Ok(Some(Token::Eof))
				}
				Some(c) if in_table(SPACE_SEPARATOR, c) => {
					self.read();
					Ok(None)
				}
				// json5 calls the handler named after the parse state without
				// switching to it; every such handler switches or returns.
				Some(_) => self.lex_parse_state(state, self.parse_state, c),
			},
			LexState::Comment => match c {
				Some('*') => {
					self.read();
					*state = LexState::MultiLineComment;
					Ok(None)
				}
				Some('/') => {
					self.read();
					*state = LexState::SingleLineComment;
					Ok(None)
				}
				_ => Err(self.invalid_char()),
			},
			LexState::MultiLineComment => match c {
				Some('*') => {
					self.read();
					*state = LexState::MultiLineCommentAsterisk;
					Ok(None)
				}
				None => Err(self.invalid_char()),
				Some(_) => {
					self.read();
					Ok(None)
				}
			},
			LexState::MultiLineCommentAsterisk => match c {
				Some('*') => {
					self.read();
					Ok(None)
				}
				Some('/') => {
					self.read();
					*state = LexState::Default;
					Ok(None)
				}
				None => Err(self.invalid_char()),
				Some(_) => {
					self.read();
					*state = LexState::MultiLineComment;
					Ok(None)
				}
			},
			LexState::SingleLineComment => match c {
				Some('\n' | '\r' | '\u{2028}' | '\u{2029}') => {
					self.read();
					*state = LexState::Default;
					Ok(None)
				}
				None => {
					self.read();
					Ok(Some(Token::Eof))
				}
				Some(_) => {
					self.read();
					Ok(None)
				}
			},
			LexState::Value => match c {
				Some(p @ ('{' | '[')) => {
					self.read();
					Ok(Some(Token::Punctuator(p)))
				}
				Some('n') => {
					self.read();
					self.literal("ull")?;
					Ok(Some(Token::Null))
				}
				Some('t') => {
					self.read();
					self.literal("rue")?;
					Ok(Some(Token::Bool(true)))
				}
				Some('f') => {
					self.read();
					self.literal("alse")?;
					Ok(Some(Token::Bool(false)))
				}
				Some('-' | '+') => {
					if self.read() == Some('-') {
						self.negative = true;
					}
					*state = LexState::Sign;
					Ok(None)
				}
				Some('.') => {
					self.buffer_read_start();
					*state = LexState::DecimalPointLeading;
					Ok(None)
				}
				Some('0') => {
					self.buffer_read_start();
					*state = LexState::Zero;
					Ok(None)
				}
				Some('1'..='9') => {
					self.buffer_read_start();
					*state = LexState::DecimalInteger;
					Ok(None)
				}
				Some('I') => {
					self.read();
					self.literal("nfinity")?;
					Ok(Some(Token::Number(f64::INFINITY)))
				}
				Some('N') => {
					self.read();
					self.literal("aN")?;
					Ok(Some(Token::Number(f64::NAN)))
				}
				Some(q @ ('"' | '\'')) => {
					self.read();
					self.double_quote = q == '"';
					self.buffer = Text::default();
					*state = LexState::String;
					Ok(None)
				}
				_ => Err(self.invalid_char()),
			},
			LexState::IdentifierNameStartEscape => {
				if c != Some('u') {
					return Err(self.invalid_char());
				}
				self.read();
				let unit = self.unicode_escape()?;
				match char::from_u32(unit.into()) {
					Some(u @ ('$' | '_')) => self.buffer.push(u),
					Some(u) if is_id_start(u) => self.buffer.push(u),
					_ => return Err(self.invalid_identifier()),
				}
				*state = LexState::IdentifierName;
				Ok(None)
			}
			LexState::IdentifierName => match c {
				Some(u @ ('$' | '_' | '\u{200c}' | '\u{200d}')) => {
					self.read();
					self.buffer.push(u);
					Ok(None)
				}
				Some('\\') => {
					self.read();
					*state = LexState::IdentifierNameEscape;
					Ok(None)
				}
				Some(u) if is_id_continue(u) => {
					self.read();
					self.buffer.push(u);
					Ok(None)
				}
				_ => Ok(Some(Token::Identifier(self.buffer.take()))),
			},
			LexState::IdentifierNameEscape => {
				if c != Some('u') {
					return Err(self.invalid_char());
				}
				self.read();
				let unit = self.unicode_escape()?;
				match char::from_u32(unit.into()) {
					Some(u)
						if matches!(u, '$' | '_' | '\u{200c}' | '\u{200d}')
							|| is_id_continue(u) =>
					{
						self.buffer.push(u);
					}
					_ => return Err(self.invalid_identifier()),
				}
				*state = LexState::IdentifierName;
				Ok(None)
			}
			LexState::Sign => match c {
				Some('.') => {
					self.buffer_read_start();
					*state = LexState::DecimalPointLeading;
					Ok(None)
				}
				Some('0') => {
					self.buffer_read_start();
					*state = LexState::Zero;
					Ok(None)
				}
				Some('1'..='9') => {
					self.buffer_read_start();
					*state = LexState::DecimalInteger;
					Ok(None)
				}
				Some('I') => {
					self.read();
					self.literal("nfinity")?;
					self.number(f64::INFINITY)
				}
				Some('N') => {
					self.read();
					self.literal("aN")?;
					Ok(Some(Token::Number(f64::NAN)))
				}
				_ => Err(self.invalid_char()),
			},
			LexState::Zero => match c {
				Some('.') => self.buffer_read_to(state, LexState::DecimalPoint),
				Some('e' | 'E') => self.buffer_read_to(state, LexState::DecimalExponent),
				Some('x' | 'X') => self.buffer_read_to(state, LexState::Hexadecimal),
				_ => self.number(0.0),
			},
			LexState::DecimalInteger => match c {
				Some('.') => self.buffer_read_to(state, LexState::DecimalPoint),
				Some('e' | 'E') => self.buffer_read_to(state, LexState::DecimalExponent),
				Some('0'..='9') => self.buffer_read(),
				_ => self.buffer_number(),
			},
			LexState::DecimalPointLeading => match c {
				Some('0'..='9') => self.buffer_read_to(state, LexState::DecimalFraction),
				_ => Err(self.invalid_char()),
			},
			LexState::DecimalPoint => match c {
				Some('e' | 'E') => self.buffer_read_to(state, LexState::DecimalExponent),
				Some('0'..='9') => self.buffer_read_to(state, LexState::DecimalFraction),
				_ => self.buffer_number(),
			},
			LexState::DecimalFraction => match c {
				Some('e' | 'E') => self.buffer_read_to(state, LexState::DecimalExponent),
				Some('0'..='9') => self.buffer_read(),
				_ => self.buffer_number(),
			},
			LexState::DecimalExponent => match c {
				Some('+' | '-') => self.buffer_read_to(state, LexState::DecimalExponentSign),
				Some('0'..='9') => self.buffer_read_to(state, LexState::DecimalExponentInteger),
				_ => Err(self.invalid_char()),
			},
			LexState::DecimalExponentSign => match c {
				Some('0'..='9') => self.buffer_read_to(state, LexState::DecimalExponentInteger),
				_ => Err(self.invalid_char()),
			},
			LexState::DecimalExponentInteger => match c {
				Some('0'..='9') => self.buffer_read(),
				_ => self.buffer_number(),
			},
			LexState::Hexadecimal => match c {
				Some(h) if h.is_ascii_hexdigit() => {
					self.buffer_read_to(state, LexState::HexadecimalInteger)
				}
				_ => Err(self.invalid_char()),
			},
			LexState::HexadecimalInteger => match c {
				Some(h) if h.is_ascii_hexdigit() => self.buffer_read(),
				_ => self.buffer_number(),
			},
			LexState::String => match c {
				Some('\\') => {
					self.read();
					self.escape()?;
					Ok(None)
				}
				Some('"') if self.double_quote => {
					self.read();
					Ok(Some(Token::String(self.buffer.take())))
				}
				Some('\'') if !self.double_quote => {
					self.read();
					Ok(Some(Token::String(self.buffer.take())))
				}
				Some('\n' | '\r') | None => Err(self.invalid_char()),
				// json5 also prints a console warning for U+2028 and U+2029
				// here; a library does not print, so that warning is dropped.
				Some(c) => {
					self.read();
					self.buffer.push(c);
					Ok(None)
				}
			},
		}
	}

	/// The lexer states named after parse states.
	fn lex_parse_state(
		&mut self,
		state: &mut LexState,
		parse_state: ParseState,
		c: Option<char>,
	) -> Step<Token> {
		match parse_state {
			ParseState::Start => match c {
				Some(p @ ('{' | '[')) => {
					self.read();
					Ok(Some(Token::Punctuator(p)))
				}
				_ => {
					*state = LexState::Value;
					Ok(None)
				}
			},
			ParseState::BeforePropertyName => match c {
				Some(u @ ('$' | '_')) => {
					self.read();
					self.buffer = Text::default();
					self.buffer.push(u);
					*state = LexState::IdentifierName;
					Ok(None)
				}
				Some('\\') => {
					self.read();
					*state = LexState::IdentifierNameStartEscape;
					Ok(None)
				}
				Some('}') => {
					self.read();
					Ok(Some(Token::Punctuator('}')))
				}
				Some(q @ ('"' | '\'')) => {
					self.read();
					self.double_quote = q == '"';
					*state = LexState::String;
					Ok(None)
				}
				Some(u) if is_id_start(u) => {
					self.read();
					self.buffer.push(u);
					*state = LexState::IdentifierName;
					Ok(None)
				}
				_ => Err(self.invalid_char()),
			},
			ParseState::AfterPropertyName => match c {
				Some(':') => {
					self.read();
					Ok(Some(Token::Punctuator(':')))
				}
				_ => Err(self.invalid_char()),
			},
			ParseState::BeforePropertyValue => {
				*state = LexState::Value;
				Ok(None)
			}
			ParseState::AfterPropertyValue => match c {
				Some(p @ (',' | '}')) => {
					self.read();
					Ok(Some(Token::Punctuator(p)))
				}
				_ => Err(self.invalid_char()),
			},
			ParseState::BeforeArrayValue => match c {
				Some(']') => {
					self.read();
					Ok(Some(Token::Punctuator(']')))
				}
				_ => {
					*state = LexState::Value;
					Ok(None)
				}
			},
			ParseState::AfterArrayValue => match c {
				Some(p @ (',' | ']')) => {
					self.read();
					Ok(Some(Token::Punctuator(p)))
				}
				_ => Err(self.invalid_char()),
			},
			ParseState::End => Err(self.invalid_char()),
		}
	}

	/// `buffer = read()`: a number starts a fresh buffer.
	fn buffer_read_start(&mut self) {
		self.buffer = Text::default();
		if let Some(c) = self.read() {
			self.buffer.push(c);
		}
	}

	/// `buffer += read()`.
	fn buffer_read(&mut self) -> Step<Token> {
		if let Some(c) = self.read() {
			self.buffer.push(c);
		}
		Ok(None)
	}

	fn buffer_read_to(&mut self, state: &mut LexState, next: LexState) -> Step<Token> {
		*state = next;
		self.buffer_read()
	}

	fn literal(&mut self, rest: &str) -> Result<(), Json5Error> {
		for expected in rest.chars() {
			if self.peek() != Some(expected) {
				return Err(self.invalid_char());
			}
			self.read();
		}
		Ok(())
	}

	/// json5's `escape()`, after the backslash, appending to the string.
	fn escape(&mut self) -> Result<(), Json5Error> {
		let c = self.peek();
		let unit = match c {
			Some('b') => '\u{8}',
			Some('f') => '\u{c}',
			Some('n') => '\n',
			Some('r') => '\r',
			Some('t') => '\t',
			Some('v') => '\u{b}',
			Some('0') => {
				self.read();
				if self.peek().is_some_and(|d| d.is_ascii_digit()) {
					return Err(self.invalid_char());
				}
				self.buffer.push('\0');
				return Ok(());
			}
			Some('x') => {
				self.read();
				let value = self.hex_escape()?;
				self.buffer.push(value);
				return Ok(());
			}
			Some('u') => {
				self.read();
				let unit = self.unicode_escape()?;
				self.buffer.push_unit(unit);
				return Ok(());
			}
			Some('\n' | '\u{2028}' | '\u{2029}') => {
				self.read();
				return Ok(());
			}
			Some('\r') => {
				self.read();
				if self.peek() == Some('\n') {
					self.read();
				}
				return Ok(());
			}
			Some('1'..='9') | None => return Err(self.invalid_char()),
			Some(other) => other,
		};
		self.read();
		self.buffer.push(unit);
		Ok(())
	}

	fn hex_escape(&mut self) -> Result<char, Json5Error> {
		let mut value: u8 = 0;
		for _ in 0..2 {
			value = (value << 4) | self.hex_digit()? as u8;
		}
		Ok(char::from(value))
	}

	fn unicode_escape(&mut self) -> Result<u16, Json5Error> {
		let mut value: u16 = 0;
		for _ in 0..4 {
			value = (value << 4) | self.hex_digit()? as u16;
		}
		Ok(value)
	}

	fn hex_digit(&mut self) -> Result<u32, Json5Error> {
		match self.peek().and_then(|c| c.to_digit(16)) {
			Some(digit) => {
				self.read();
				Ok(digit)
			}
			None => Err(self.invalid_char()),
		}
	}

	/// One call of json5's `parseStates[parseState]()`.
	fn parse_token(&mut self, token: Token) -> Result<(), Json5Error> {
		match self.parse_state {
			ParseState::Start => {
				if matches!(token, Token::Eof) {
					return Err(self.invalid_eof());
				}
				self.push(token);
			}
			ParseState::BeforePropertyName => match token {
				Token::Identifier(key) | Token::String(key) => {
					self.key = Some(key);
					self.parse_state = ParseState::AfterPropertyName;
				}
				Token::Punctuator(_) => self.pop(),
				Token::Eof => return Err(self.invalid_eof()),
				Token::Null | Token::Bool(_) | Token::Number(_) => {}
			},
			ParseState::AfterPropertyName => {
				if matches!(token, Token::Eof) {
					return Err(self.invalid_eof());
				}
				self.parse_state = ParseState::BeforePropertyValue;
			}
			ParseState::BeforePropertyValue => {
				if matches!(token, Token::Eof) {
					return Err(self.invalid_eof());
				}
				self.push(token);
			}
			ParseState::BeforeArrayValue => match token {
				Token::Eof => return Err(self.invalid_eof()),
				Token::Punctuator(']') => self.pop(),
				token => self.push(token),
			},
			ParseState::AfterPropertyValue => match token {
				Token::Eof => return Err(self.invalid_eof()),
				Token::Punctuator(',') => self.parse_state = ParseState::BeforePropertyName,
				Token::Punctuator('}') => self.pop(),
				_ => {}
			},
			ParseState::AfterArrayValue => match token {
				Token::Eof => return Err(self.invalid_eof()),
				Token::Punctuator(',') => self.parse_state = ParseState::BeforeArrayValue,
				Token::Punctuator(']') => self.pop(),
				_ => {}
			},
			ParseState::End => {}
		}
		Ok(())
	}

	fn push(&mut self, token: Token) {
		let value = match token {
			Token::Punctuator('{') => {
				let key = self.key.take();
				self.stack.push(Open {
					key,
					container: Container::Object(Object::new()),
				});
				self.parse_state = ParseState::BeforePropertyName;
				return;
			}
			Token::Punctuator('[') => {
				let key = self.key.take();
				self.stack.push(Open {
					key,
					container: Container::Array(Array::new()),
				});
				self.parse_state = ParseState::BeforeArrayValue;
				return;
			}
			Token::Null => Value::Null,
			Token::Bool(b) => Value::Bool(b),
			Token::Number(n) => Value::Number(n),
			Token::String(s) => Value::String(s),
			Token::Punctuator(_) | Token::Identifier(_) | Token::Eof => {
				unreachable!("the lexer hands push() only values and opening brackets")
			}
		};
		let key = self.key.take();
		self.attach(key, value);
	}

	fn pop(&mut self) {
		if let Some(open) = self.stack.pop() {
			let value = match open.container {
				Container::Array(array) => Value::Array(array),
				Container::Object(object) => Value::Object(object),
			};
			self.attach(open.key, value);
		}
	}

	/// Stores a finished value in the innermost open container, or as the
	/// root, and moves to the state after a value. json5 stores a container in
	/// its parent when it opens rather than when it closes; nothing else
	/// reaches the parent in between, so the key order is the same.
	fn attach(&mut self, key: Option<String>, value: Value) {
		match self.stack.last_mut() {
			None => {
				self.root = Some(value);
				self.parse_state = ParseState::End;
			}
			Some(Open {
				container: Container::Array(array),
				..
			}) => {
				array.push(value);
				self.parse_state = ParseState::AfterArrayValue;
			}
			Some(Open {
				container: Container::Object(object),
				..
			}) => {
				let key = key.unwrap_or_default();
				// `parent["__proto__"] = value` sets the prototype (or does
				// nothing, for a primitive); it never creates a property.
				if key != "__proto__" {
					object.insert(key, value);
				}
				self.parse_state = ParseState::AfterPropertyValue;
			}
		}
	}
}

/// A string being built from source characters and `\u` escapes. Escapes are
/// UTF-16 code units, so a surrogate pair written as two escapes is joined
/// here; a surrogate left alone becomes U+FFFD.
#[derive(Default)]
struct Text {
	text: String,
	high_surrogate: bool,
	pending: u16,
}

impl Text {
	fn push(&mut self, c: char) {
		self.flush();
		self.text.push(c);
	}

	fn push_unit(&mut self, unit: u16) {
		match unit {
			0xd800..=0xdbff => {
				self.flush();
				self.high_surrogate = true;
				self.pending = unit;
			}
			0xdc00..=0xdfff if self.high_surrogate => {
				let high = u32::from(self.pending - 0xd800);
				let low = u32::from(unit - 0xdc00);
				self.high_surrogate = false;
				let c = char::from_u32(0x10000 + (high << 10) + low);
				self.text.push(c.unwrap_or(char::REPLACEMENT_CHARACTER));
			}
			unit => match char::from_u32(unit.into()) {
				Some(c) => self.push(c),
				None => self.push(char::REPLACEMENT_CHARACTER),
			},
		}
	}

	fn flush(&mut self) {
		if self.high_surrogate {
			self.high_surrogate = false;
			self.text.push(char::REPLACEMENT_CHARACTER);
		}
	}

	fn take(&mut self) -> String {
		self.flush();
		std::mem::take(&mut self.text)
	}
}

fn in_table(table: &[(u32, u32)], c: char) -> bool {
	let c = u32::from(c);
	table
		.binary_search_by(|&(start, end)| {
			if end < c {
				std::cmp::Ordering::Less
			} else if start > c {
				std::cmp::Ordering::Greater
			} else {
				std::cmp::Ordering::Equal
			}
		})
		.is_ok()
}

fn is_id_start(c: char) -> bool {
	in_table(ID_START, c)
}

fn is_id_continue(c: char) -> bool {
	in_table(ID_CONTINUE, c)
}

/// `Number("0x" + digits)`: the integer the hex digits spell, rounded to the
/// nearest double with ties to even, and infinity from 2^1024 up.
fn hex_to_f64(digits: &str) -> f64 {
	// The leading 64 significant bits, how many significant bits there are in
	// all, and whether any bit after the leading 64 is set.
	let mut top: u64 = 0;
	let mut significant: u64 = 0;
	let mut sticky = false;
	for digit in digits.chars().filter_map(|c| c.to_digit(16)) {
		for shift in (0..4).rev() {
			let bit = u64::from((digit >> shift) & 1);
			if significant == 0 && bit == 0 {
				continue;
			}
			if significant < 64 {
				top = (top << 1) | bit;
			} else {
				sticky |= bit == 1;
			}
			significant += 1;
		}
	}
	if significant <= 64 {
		// u64 to f64 rounds to nearest, ties to even.
		return top as f64;
	}
	// `top as f64` drops 11 bits. If they are exactly one half, the bits
	// beyond `top` decide the direction, so a set sticky bit moves the lowest
	// bit up to break the tie; any other rounding is unaffected by it.
	let rounded = (top | u64::from(sticky)) as f64;
	let scale = significant - 64;
	if scale > 1023 {
		return f64::INFINITY;
	}
	// A power of two scales exactly, overflowing to infinity past the largest
	// double.
	rounded * f64::from_bits((scale + 1023) << 52)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::json::{Indent, stringify};

	fn parsed(source: &str) -> String {
		match parse_json5(source) {
			Ok(value) => stringify(&value, Indent::None),
			Err(error) => panic!("{source:?} did not parse: {error}"),
		}
	}

	fn refusal(source: &str) -> String {
		match parse_json5(source) {
			Ok(value) => panic!("{source:?} parsed as {}", stringify(&value, Indent::None)),
			Err(error) => error.to_string(),
		}
	}

	#[test]
	fn hex_integers_up_to_64_bits_round_to_the_nearest_even_double() {
		assert_eq!(hex_to_f64(""), 0.0);
		assert_eq!(hex_to_f64("000"), 0.0);
		assert_eq!(hex_to_f64("1f"), 31.0);
		assert_eq!(hex_to_f64("1fffffffffffff"), 9_007_199_254_740_991.0);
		// 2^53 + 1 and 2^53 + 3 are ties; the even neighbours are 2^53 and 2^53 + 4.
		assert_eq!(hex_to_f64("20000000000001"), 9_007_199_254_740_992.0);
		assert_eq!(hex_to_f64("20000000000003"), 9_007_199_254_740_996.0);
		assert_eq!(hex_to_f64("ffffffffffffffff"), 18_446_744_073_709_551_616.0);
	}

	#[test]
	fn hex_integers_past_64_bits_use_every_digit_to_break_ties() {
		// 2^64 + 2^11 is halfway between 2^64 and 2^64 + 2^12: even wins.
		assert_eq!(
			hex_to_f64("10000000000000800"),
			18_446_744_073_709_551_616.0
		);
		// One more makes it closer to 2^64 + 2^12.
		assert_eq!(
			hex_to_f64("10000000000000801"),
			18_446_744_073_709_555_712.0
		);
		let far_below = format!("10000000000000800{}1", "0".repeat(40));
		assert_eq!(hex_to_f64(&far_below), 2f64.powi(228) + 2f64.powi(176));
	}

	#[test]
	fn hex_integers_of_2_to_the_1024_or_more_are_infinite() {
		assert_eq!(
			hex_to_f64(&format!("8{}", "0".repeat(255))),
			2f64.powi(1023)
		);
		assert_eq!(hex_to_f64(&format!("1{}", "0".repeat(256))), f64::INFINITY);
		// 2^1024 - 1 rounds up to 2^1024, which no double holds.
		assert_eq!(hex_to_f64(&"f".repeat(256)), f64::INFINITY);
		assert_eq!(hex_to_f64(&"f".repeat(5000)), f64::INFINITY);
	}

	#[test]
	fn format_char_escapes_what_json5_escapes() {
		let cases = [
			('\'', "\\'"),
			('"', "\\\""),
			('\\', "\\\\"),
			('\u{8}', "\\b"),
			('\u{b}', "\\v"),
			('\0', "\\0"),
			('\u{1}', "\\x01"),
			('\u{1f}', "\\x1f"),
			('\u{2028}', "\\u2028"),
			('\u{7f}', "\u{7f}"),
			('x', "x"),
			('\u{1f41c}', "\u{1f41c}"),
		];
		for (c, expected) in cases {
			assert_eq!(FormatChar(c).to_string(), expected, "{c:?}");
		}
	}

	#[test]
	fn a_proto_key_never_becomes_a_property() {
		assert_eq!(parsed("{\"__proto__\":{\"x\":1},\"y\":2}"), "{\"y\":2}");
		assert_eq!(
			parsed("{__proto__:1,'__proto__':[],\"__proto__\":null}"),
			"{}"
		);
	}

	#[test]
	fn a_repeated_key_keeps_its_first_place_and_its_last_value() {
		assert_eq!(parsed("{b:1,a:2,b:{c:3}}"), "{\"b\":{\"c\":3},\"a\":2}");
	}

	#[test]
	fn surrogate_escapes_join_into_one_character() {
		assert_eq!(parsed("'\\uD83D\\uDC1C'"), "\"\u{1f41c}\"");
	}

	#[test]
	fn a_lone_surrogate_escape_becomes_the_replacement_character() {
		assert_eq!(parsed("'\\uD83D'"), "\"\u{fffd}\"");
		assert_eq!(parsed("'\\uDC1C'"), "\"\u{fffd}\"");
		assert_eq!(
			parsed("'\\uD83Dx\\uD83D\\uD83D\\uDC1C'"),
			"\"\u{fffd}x\u{fffd}\u{1f41c}\""
		);
	}

	#[test]
	fn an_error_reports_its_kind_line_and_column() {
		let error = parse_json5("[\n\n  1 x]").expect_err("x is not allowed");
		assert_eq!(error.kind(), Json5ErrorKind::InvalidCharacter('x'));
		assert_eq!((error.line(), error.column()), (3, 5));
		let error = parse_json5("[1").expect_err("the array is not closed");
		assert_eq!(error.kind(), Json5ErrorKind::InvalidEndOfInput);
		assert_eq!((error.line(), error.column()), (1, 3));
		let error = parse_json5("{a\\u002d:1}").expect_err("- cannot be in a key");
		assert_eq!(error.kind(), Json5ErrorKind::InvalidIdentifierCharacter);
		assert_eq!((error.line(), error.column()), (1, 3));
	}

	#[test]
	fn an_empty_input_ends_early_at_column_one() {
		assert_eq!(refusal(""), "JSON5: invalid end of input at 1:1");
		assert_eq!(refusal("  // c"), "JSON5: invalid end of input at 1:7");
	}

	#[test]
	fn the_position_is_the_column_after_the_offending_character() {
		assert_eq!(refusal("[1,]x"), "JSON5: invalid character 'x' at 1:5");
		assert_eq!(refusal("{\n  a 1}"), "JSON5: invalid character '1' at 2:5");
		// A carriage return does not start a line; U+1F41C is two columns.
		assert_eq!(
			refusal("\r\u{1f41c}"),
			"JSON5: invalid character '\u{1f41c}' at 1:3"
		);
	}

	#[test]
	fn an_escaped_key_character_that_cannot_be_in_a_key_reports_five_columns_back() {
		// The escape ends at column 8; json5 subtracts 5.
		assert_eq!(
			refusal("{a\\u002d:1}"),
			"JSON5: invalid identifier character at 1:3"
		);
	}

	#[test]
	fn a_million_levels_of_nesting_parse_without_overflowing_the_stack() {
		let depth = 1_000_000;
		let source = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
		let value = parse_json5(&source).expect("deep nesting parses");
		assert_eq!(stringify(&value, Indent::None), source);
	}
}
