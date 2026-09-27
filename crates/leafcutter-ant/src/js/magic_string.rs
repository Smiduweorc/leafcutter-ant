//! The part of magic-string 0.26.7 that js-runtime's import/export rewrite
//! uses: `overwrite`, `slice` and `toString`, over UTF-16 code units as
//! JavaScript strings index them.
//!
//! It is ported rather than replaced by plain string splicing because the
//! rewrite can hand it ranges that overlap or run past earlier edits (a kept
//! `/*! ... */` comment shifts every position after it), and what
//! magic-string does then, throwing or quietly producing broken code, is what
//! the module loader goes on to run.

use std::collections::HashMap;

struct Chunk {
	start: usize,
	end: usize,
	original: Vec<u16>,
	intro: Vec<u16>,
	outro: Vec<u16>,
	content: Vec<u16>,
	edited: bool,
	previous: Option<usize>,
	next: Option<usize>,
}

impl Chunk {
	fn new(start: usize, end: usize, content: Vec<u16>) -> Self {
		Chunk {
			start,
			end,
			original: content.clone(),
			intro: Vec::new(),
			outro: Vec::new(),
			content,
			edited: false,
			previous: None,
			next: None,
		}
	}

	fn contains(&self, index: usize) -> bool {
		self.start < index && index < self.end
	}

	/// `edit(content, storeName, contentOnly)`, which the overwrite path
	/// calls with `contentOnly` false: the intro and outro go too.
	fn edit(&mut self, content: Vec<u16>) {
		self.content = content;
		self.intro.clear();
		self.outro.clear();
		self.edited = true;
	}
}

pub(crate) struct MagicString {
	original_len: usize,
	chunks: Vec<Chunk>,
	first_chunk: usize,
	last_chunk: usize,
	last_searched_chunk: usize,
	by_start: HashMap<usize, usize>,
	by_end: HashMap<usize, usize>,
}

impl MagicString {
	pub(crate) fn new(original: Vec<u16>) -> Self {
		let len = original.len();
		MagicString {
			original_len: len,
			chunks: vec![Chunk::new(0, len, original)],
			first_chunk: 0,
			last_chunk: 0,
			last_searched_chunk: 0,
			by_start: HashMap::from([(0, 0)]),
			by_end: HashMap::from([(len, 0)]),
		}
	}

	/// `overwrite(start, end, content)`.
	pub(crate) fn overwrite(
		&mut self,
		start: usize,
		end: usize,
		content: Vec<u16>,
	) -> Result<(), String> {
		if end > self.original_len {
			return Err("end is out of bounds".to_owned());
		}
		if start == end {
			return Err(
				"Cannot overwrite a zero-length range - use appendLeft or prependRight instead"
					.to_owned(),
			);
		}
		self.split(start)?;
		self.split(end)?;
		let first = self.by_start.get(&start).copied();
		let last = self.by_end.get(&end).copied();
		match first {
			Some(first) => {
				let mut chunk = first;
				while Some(chunk) != last {
					let next = self.chunks[chunk].next;
					if next != self.by_start.get(&self.chunks[chunk].end).copied() {
						return Err("Cannot overwrite across a split point".to_owned());
					}
					let Some(next) = next else {
						return Err(
							"TypeError: Cannot read properties of undefined (reading 'edit')"
								.to_owned(),
						);
					};
					chunk = next;
					self.chunks[chunk].edit(Vec::new());
				}
				self.chunks[first].edit(content);
			}
			None => {
				// "must be inserting at the end"
				let Some(last) = last else {
					return Err(
						"TypeError: Cannot set properties of undefined (setting 'next')".to_owned(),
					);
				};
				let mut chunk = Chunk::new(start, end, Vec::new());
				chunk.edit(content);
				chunk.previous = Some(last);
				let index = self.chunks.len();
				self.chunks.push(chunk);
				self.chunks[last].next = Some(index);
			}
		}
		Ok(())
	}

	/// `slice(start, end)`: the text between two original positions, with
	/// the edits made inside the range.
	pub(crate) fn slice(&self, start: usize, end: usize) -> Result<Vec<u16>, String> {
		let mut result = Vec::new();
		let mut chunk = Some(self.first_chunk);
		while let Some(index) = chunk {
			let c = &self.chunks[index];
			if !(c.start > start || c.end <= start) {
				break;
			}
			if c.start < end && c.end >= end {
				return Ok(result);
			}
			chunk = c.next;
		}
		if let Some(index) = chunk {
			let c = &self.chunks[index];
			if c.edited && c.start != start {
				return Err(format!(
					"Cannot use replaced character {start} as slice start anchor."
				));
			}
		}
		let start_chunk = chunk;
		while let Some(index) = chunk {
			let c = &self.chunks[index];
			if !c.intro.is_empty() && (start_chunk != chunk || c.start == start) {
				result.extend_from_slice(&c.intro);
			}
			let contains_end = c.start < end && c.end >= end;
			if contains_end && c.edited && c.end != end {
				return Err(format!(
					"Cannot use replaced character {end} as slice end anchor."
				));
			}
			let slice_start = if start_chunk == chunk {
				start - c.start
			} else {
				0
			};
			let slice_end = if contains_end {
				(c.content.len() + end).saturating_sub(c.end)
			} else {
				c.content.len()
			};
			result.extend_from_slice(js_slice(&c.content, slice_start, slice_end));
			if !c.outro.is_empty() && (!contains_end || c.end == end) {
				result.extend_from_slice(&c.outro);
			}
			if contains_end {
				break;
			}
			chunk = c.next;
		}
		Ok(result)
	}

	fn split(&mut self, index: usize) -> Result<(), String> {
		if self.by_start.contains_key(&index) || self.by_end.contains_key(&index) {
			return Ok(());
		}
		let mut chunk = Some(self.last_searched_chunk);
		let search_forward = index > self.chunks[self.last_searched_chunk].end;
		while let Some(current) = chunk {
			if self.chunks[current].contains(index) {
				return self.split_chunk(current, index);
			}
			chunk = if search_forward {
				self.by_start.get(&self.chunks[current].end).copied()
			} else {
				self.by_end.get(&self.chunks[current].start).copied()
			};
		}
		Ok(())
	}

	fn split_chunk(&mut self, index: usize, at: usize) -> Result<(), String> {
		if self.chunks[index].edited && !self.chunks[index].content.is_empty() {
			return Err(format!(
				"Cannot split a chunk that has already been edited (\"{}\")",
				String::from_utf16_lossy(&self.chunks[index].original)
			));
		}
		let new_index = self.chunks.len();
		let chunk = &mut self.chunks[index];
		let slice_index = at - chunk.start;
		let original_after = chunk
			.original
			.split_off(slice_index.min(chunk.original.len()));
		let mut new_chunk = Chunk::new(at, chunk.end, original_after);
		new_chunk.outro = std::mem::take(&mut chunk.outro);
		chunk.end = at;
		if chunk.edited {
			new_chunk.edit(Vec::new());
			chunk.content.clear();
		} else {
			chunk.content = chunk.original.clone();
		}
		new_chunk.next = chunk.next;
		new_chunk.previous = Some(index);
		chunk.next = Some(new_index);
		if let Some(next) = new_chunk.next {
			self.chunks[next].previous = Some(new_index);
		}
		let new_end = new_chunk.end;
		self.chunks.push(new_chunk);
		self.by_end.insert(at, index);
		self.by_start.insert(at, new_index);
		self.by_end.insert(new_end, new_index);
		if index == self.last_chunk {
			self.last_chunk = new_index;
		}
		self.last_searched_chunk = index;
		Ok(())
	}

	/// `toString()`.
	pub(crate) fn into_units(self) -> Vec<u16> {
		let mut out = Vec::new();
		let mut chunk = Some(self.first_chunk);
		while let Some(index) = chunk {
			let c = &self.chunks[index];
			out.extend_from_slice(&c.intro);
			out.extend_from_slice(&c.content);
			out.extend_from_slice(&c.outro);
			chunk = c.next;
		}
		out
	}
}

/// `String.prototype.slice(start, end)` for non-negative bounds: clamped to
/// the length, and empty when they cross.
fn js_slice(units: &[u16], start: usize, end: usize) -> &[u16] {
	let end = end.min(units.len());
	let start = start.min(end);
	&units[start..end]
}

#[cfg(test)]
mod tests {
	use super::*;

	fn units(s: &str) -> Vec<u16> {
		s.encode_utf16().collect()
	}

	fn text(units: Vec<u16>) -> String {
		String::from_utf16_lossy(&units)
	}

	#[test]
	fn disjoint_overwrites_replace_their_ranges() {
		let mut s = MagicString::new(units("abcdefgh"));
		s.overwrite(1, 3, units("XY")).expect("in range");
		s.overwrite(5, 8, units("")).expect("in range");
		assert_eq!(text(s.into_units()), "aXYde");
	}

	#[test]
	fn a_slice_sees_the_edits_inside_its_range() {
		let mut s = MagicString::new(units("0123456789"));
		s.overwrite(2, 4, units("ab")).expect("in range");
		assert_eq!(text(s.slice(1, 6).expect("anchors are fine")), "1ab45");
		assert_eq!(text(s.slice(4, 6).expect("anchors are fine")), "45");
		assert_eq!(
			s.slice(3, 6).expect_err("3 is inside an edit"),
			"Cannot use replaced character 3 as slice start anchor."
		);
		assert_eq!(
			s.slice(0, 3).expect_err("3 is inside an edit"),
			"Cannot use replaced character 3 as slice end anchor."
		);
	}

	#[test]
	fn overlapping_and_out_of_range_overwrites_fail_as_in_magic_string() {
		let mut s = MagicString::new(units("0123456789"));
		s.overwrite(2, 6, units("x")).expect("in range");
		assert_eq!(
			s.overwrite(4, 8, units("y"))
				.expect_err("4 splits an edited chunk"),
			"Cannot split a chunk that has already been edited (\"2345\")"
		);
		assert_eq!(
			s.overwrite(8, 11, units("z")).expect_err("past the end"),
			"end is out of bounds"
		);
		assert!(s.overwrite(7, 7, units("z")).is_err());
		// An overwrite that covers an earlier one replaces it whole.
		s.overwrite(1, 7, units("W"))
			.expect("chunk boundaries line up");
		assert_eq!(text(s.into_units()), "0W789");
	}
}
