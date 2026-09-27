//! Ports of the two pathe versions in TS Dash's dependency tree. The functions
//! at the top level are pathe 2.0.2, which Dash imports; [`v1`] holds the two
//! functions of pathe 1.1.2, which mc-project-core imports, that return
//! something else. Only the functions Dash and mc-project-core call are here.
//!
//! JavaScript strings are UTF-16 and these are UTF-8, but pathe only ever
//! looks for `/`, `\`, `.` and `:`, which are ASCII, so indexing bytes finds
//! the same positions that indexing code units does.

/// pathe's `normalizeWindowsPath`: backslashes become slashes, and a leading
/// drive letter followed by a slash is upper-cased.
fn normalize_windows_path(input: &str) -> String {
	let mut path = input.replace('\\', "/");
	let bytes = path.as_bytes();
	if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/' {
		path[..1].make_ascii_uppercase();
	}
	path
}

/// `/^[/\\](?![/\\])|^[/\\]{2}(?!\.)|^[A-Za-z]:[/\\]/`
pub(crate) fn is_absolute(path: &str) -> bool {
	let is_sep = |b: Option<&u8>| matches!(b, Some(b'/' | b'\\'));
	let bytes = path.as_bytes();
	match bytes {
		[a, b, c, ..] if a.is_ascii_alphabetic() && *b == b':' && is_sep(Some(c)) => true,
		_ if !is_sep(bytes.first()) => false,
		_ if !is_sep(bytes.get(1)) => true,
		_ => bytes.get(2) != Some(&b'.'),
	}
}

/// `/^[A-Za-z]:$/`
fn is_drive_letter(path: &str) -> bool {
	matches!(path.as_bytes(), [letter, b':'] if letter.is_ascii_alphabetic())
}

pub(crate) fn normalize(path: &str) -> String {
	if path.is_empty() {
		return ".".to_owned();
	}
	let path = normalize_windows_path(path);
	let is_unc = path.starts_with("//");
	let is_path_absolute = is_absolute(&path);
	let trailing_separator = path.ends_with('/');
	let mut result = normalize_string(&path, !is_path_absolute);
	if result.is_empty() {
		return if is_path_absolute {
			"/"
		} else if trailing_separator {
			"./"
		} else {
			"."
		}
		.to_owned();
	}
	if trailing_separator {
		result.push('/');
	}
	if is_drive_letter(&result) {
		result.push('/');
	}
	if is_unc {
		return if is_path_absolute {
			format!("//{result}")
		} else {
			format!("//./{result}")
		};
	}
	if is_path_absolute && !is_absolute(&result) {
		format!("/{result}")
	} else {
		result
	}
}

/// pathe's `normalizeString`: resolves `.` and `..` segments and drops empty
/// ones. `..` that climbs above the start is kept only when
/// `allow_above_root` is set.
fn normalize_string(path: &str, allow_above_root: bool) -> String {
	let bytes = path.as_bytes();
	let mut res = String::new();
	let mut last_segment_length = 0;
	let mut last_slash: isize = -1;
	let mut dots: i32 = 0;
	let mut char = 0u8;
	let mut index = 0usize;
	while index <= bytes.len() {
		if index < bytes.len() {
			char = bytes[index];
		} else if char == b'/' {
			break;
		} else {
			char = b'/';
		}
		if char == b'/' {
			let at = index as isize;
			if last_slash == at - 1 || dots == 1 {
				// An empty segment or ".": nothing to add.
			} else if dots == 2 {
				let climbed = res.len() >= 2 && last_segment_length == 2 && res.ends_with("..");
				if !climbed {
					if res.len() > 2 {
						match res.rfind('/') {
							None => {
								res.clear();
								last_segment_length = 0;
							}
							Some(last_slash_index) => {
								res.truncate(last_slash_index);
								last_segment_length =
									res.len() - res.rfind('/').map_or(0, |i| i + 1);
							}
						}
						last_slash = at;
						dots = 0;
						index += 1;
						continue;
					} else if !res.is_empty() {
						res.clear();
						last_segment_length = 0;
						last_slash = at;
						dots = 0;
						index += 1;
						continue;
					}
				}
				if allow_above_root {
					res.push_str(if res.is_empty() { ".." } else { "/.." });
					last_segment_length = 2;
				}
			} else {
				let segment = &path[(last_slash + 1) as usize..index];
				if !res.is_empty() {
					res.push('/');
				}
				res.push_str(segment);
				last_segment_length = (at - last_slash - 1) as usize;
			}
			last_slash = at;
			dots = 0;
		} else if char == b'.' && dots != -1 {
			dots += 1;
		} else {
			dots = -1;
		}
		index += 1;
	}
	res
}

/// pathe 2.0.2's `join`: skips empty segments, joins the rest with exactly one
/// slash between neighbours, then normalizes.
pub(crate) fn join(segments: &[&str]) -> String {
	let mut path = String::new();
	for seg in segments.iter().filter(|seg| !seg.is_empty()) {
		if path.is_empty() {
			path.push_str(seg);
			continue;
		}
		match (path.ends_with('/'), seg.starts_with('/')) {
			(true, true) => path.push_str(&seg[1..]),
			(false, false) => {
				path.push('/');
				path.push_str(seg);
			}
			_ => path.push_str(seg),
		}
	}
	normalize(&path)
}

/// pathe's `resolve`, with the working directory fixed at `/`: pathe uses
/// `process.cwd()` where a `process` exists and `/` where it does not, as in
/// the editor's web worker. Relative inputs resolve to the same relative
/// result either way unless they climb above the working directory.
pub(crate) fn resolve(paths: &[&str]) -> String {
	let mut resolved = String::new();
	let mut resolved_absolute = false;
	for path in paths
		.iter()
		.rev()
		.map(|p| normalize_windows_path(p))
		.chain(["/".to_owned()])
	{
		if path.is_empty() {
			continue;
		}
		resolved = format!("{path}/{resolved}");
		resolved_absolute = is_absolute(&path);
		if resolved_absolute {
			break;
		}
	}
	let resolved = normalize_string(&resolved, !resolved_absolute);
	if resolved_absolute && !is_absolute(&resolved) {
		return format!("/{resolved}");
	}
	if resolved.is_empty() {
		".".to_owned()
	} else {
		resolved
	}
}

pub(crate) fn relative(from: &str, to: &str) -> String {
	// `/^\/([A-Za-z]:)?$/` replaced by the drive letter it captured.
	fn strip_root_folder(path: String) -> String {
		match path.as_bytes() {
			[b'/'] => String::new(),
			[b'/', letter, b':'] if letter.is_ascii_alphabetic() => path[1..].to_owned(),
			_ => path,
		}
	}
	let from = strip_root_folder(resolve(&[from]));
	let to = strip_root_folder(resolve(&[to]));
	let mut from: Vec<&str> = from.split('/').collect();
	let to_parts: Vec<&str> = to.split('/').collect();
	let is_drive = |part: &str| part.as_bytes().get(1) == Some(&b':');
	if is_drive(to_parts[0]) && is_drive(from[0]) && from[0] != to_parts[0] {
		return to_parts.join("/");
	}
	let common = from
		.iter()
		.zip(&to_parts)
		.take_while(|(a, b)| a == b)
		.count();
	from.drain(..common);
	let mut parts: Vec<&str> = from.iter().map(|_| "..").collect();
	parts.extend(&to_parts[common..]);
	parts.join("/")
}

pub(crate) fn dirname(path: &str) -> String {
	let normalized = normalize_windows_path(path);
	let trimmed = normalized.strip_suffix('/').unwrap_or(&normalized);
	let mut segments: Vec<String> = trimmed.split('/').map(str::to_owned).collect();
	segments.pop();
	if segments.len() == 1 && is_drive_letter(&segments[0]) {
		segments[0].push('/');
	}
	let joined = segments.join("/");
	if !joined.is_empty() {
		joined
	} else if is_absolute(path) {
		"/".to_owned()
	} else {
		".".to_owned()
	}
}

/// pathe 2.0.2's `basename`: the last non-empty segment, without `extension`
/// if it ends with it.
pub(crate) fn basename(path: &str, extension: Option<&str>) -> String {
	let normalized = normalize_windows_path(path);
	let last = normalized
		.rsplit('/')
		.find(|segment| !segment.is_empty())
		.unwrap_or("");
	match extension {
		Some(extension) if !extension.is_empty() && last.ends_with(extension) => {
			last[..last.len() - extension.len()].to_owned()
		}
		_ => last.to_owned(),
	}
}

/// The group pathe 1.1.2's `/.(\.[^./]+)$/` captures. The regular expression
/// can only end at the last `.`, so this finds that dot and checks the rest
/// of the pattern around it: the `.` before the group matches any character
/// but a line terminator, and after the dot come one or more characters
/// with no `/`.
fn extension_match(path: &str) -> String {
	let Some(dot) = path.rfind('.') else {
		return String::new();
	};
	let rest = &path[dot + 1..];
	let group_matches = !rest.is_empty() && !rest.contains('/');
	let preceded = path[..dot]
		.chars()
		.next_back()
		.is_some_and(|c| !matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'));
	if group_matches && preceded {
		path[dot..].to_owned()
	} else {
		String::new()
	}
}

/// pathe 1.1.2, where it differs from 2.0.2.
pub(crate) mod v1 {
	use super::{extension_match, normalize, normalize_windows_path};

	/// pathe 1.1.2's `join`: the non-empty arguments joined with `/`, runs of
	/// slashes collapsed, then normalized; `"."` when there is nothing to join.
	pub(crate) fn join(segments: &[&str]) -> String {
		let joined: Vec<&str> = segments.iter().copied().filter(|s| !s.is_empty()).collect();
		if joined.is_empty() {
			return ".".to_owned();
		}
		let mut collapsed = String::new();
		for c in joined.join("/").chars() {
			if c != '/' || !collapsed.ends_with('/') {
				collapsed.push(c);
			}
		}
		normalize(&collapsed)
	}

	/// pathe 1.1.2's `extname`: `/.(\.[^./]+)$/`, so a trailing dot is no
	/// extension and `".."` gets no special case.
	pub(crate) fn extname(path: &str) -> String {
		extension_match(&normalize_windows_path(path))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// paths.json, recorded by tools/parity/paths.mjs from pathe 2.0.2 and
	/// 1.1.2 with the working directory set to `/`.
	#[test]
	fn every_function_returns_what_pathe_returns() {
		let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/paths.json");
		let text = std::fs::read_to_string(path).expect("paths.json is readable");
		let vectors: Vec<(String, String, Vec<String>, serde_json::Value)> =
			serde_json::from_str(&text).expect("paths.json is well formed");
		assert!(vectors.len() > 5000);
		let mut failures = Vec::new();
		for (version, function, args, expected) in &vectors {
			let a: Vec<&str> = args.iter().map(String::as_str).collect();
			let actual: serde_json::Value = match (version.as_str(), function.as_str()) {
				("2.0.2", "normalize") => normalize(a[0]).into(),
				("2.0.2", "dirname") => dirname(a[0]).into(),
				("2.0.2", "basename") => basename(a[0], a.get(1).copied()).into(),
				("2.0.2", "isAbsolute") => is_absolute(a[0]).into(),
				("2.0.2", "resolve") => resolve(&a).into(),
				("2.0.2", "relative") => relative(a[0], a[1]).into(),
				("2.0.2", "join") => join(&a).into(),
				("1.1.2", "join") => v1::join(&a).into(),
				("1.1.2", "extname") => v1::extname(a[0]).into(),
				other => panic!("no port of {other:?}"),
			};
			if &actual != expected {
				failures.push(format!(
					"{version} {function}{args:?}: expected {expected}, got {actual}"
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
	fn pathe_1_joins_and_reads_extensions_its_own_way() {
		assert_eq!(join(&["//a"]), "//a");
		assert_eq!(v1::join(&["//a"]), "/a");
		assert_eq!(join(&[]), ".");
		assert_eq!(v1::join(&[]), ".");
		assert_eq!(v1::extname("file."), "");
		assert_eq!(v1::extname("a/.."), "");
		assert_eq!(v1::extname(".."), "");
	}

	#[test]
	fn a_dot_after_a_line_break_starts_no_extension() {
		assert_eq!(v1::extname("a\n.txt"), "");
		assert_eq!(v1::extname("a\u{2028}.txt"), "");
		assert_eq!(v1::extname("a .txt"), ".txt");
	}
}
