//! A port of the parts of `@bridge-editor/mc-project-core` 0.5.0 Dash uses:
//! the project config's pack paths, and pack and file type detection. The
//! definitions come from bridge-core/editor-packages, which the host fetches.
//! mc-project-core imports pathe 1.1.2, so paths here go through that
//! version's `join` and `extname`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use crate::glob::{Globs, PicomatchError};
use crate::js::{self, Prop};
use crate::json::Value;
use crate::pathe::v1;

/// A project config (`config.json`) and the directory it sits in.
pub(crate) struct ProjectConfig {
	base_path: String,
	data: Value,
	/// `data.packs` when it is not null. A string is kept as an array of its
	/// characters, which is what indexing it gives.
	packs: Option<Value>,
}

/// mc-project-core's `defaultPackPaths`, a plain object, so the names of
/// `Object.prototype`'s properties find those.
fn default_pack_path(pack_id: &str) -> Prop<'static> {
	static DEFAULTS: LazyLock<[(&str, Value); 4]> = LazyLock::new(|| {
		[
			("behaviorPack", "./BP"),
			("resourcePack", "./RP"),
			("skinPack", "./SP"),
			("worldTemplate", "./WT"),
		]
		.map(|(id, path)| (id, Value::String(path.to_owned())))
	});
	match DEFAULTS.iter().find(|(id, _)| *id == pack_id) {
		Some((_, path)) => Prop::Value(path),
		None => js::Inherited::lookup(pack_id).map_or(Prop::Undefined, Prop::Inherited),
	}
}

impl ProjectConfig {
	/// The config at `base_path`, which Dash takes from pathe's `dirname` and
	/// so is never empty.
	pub(crate) fn new(base_path: String, data: Value) -> Self {
		let packs = match js::own(&data, "packs") {
			Prop::Value(Value::Null) | Prop::Undefined | Prop::Inherited(_) => None,
			Prop::Value(Value::String(s)) => {
				let units: Vec<u16> = s.encode_utf16().collect();
				let chars = units
					.iter()
					.map(|&unit| Value::String(String::from_utf16_lossy(&[unit])));
				Some(Value::Array(chars.collect::<Vec<_>>().into()))
			}
			Prop::Value(packs) => Some(packs.clone()),
		};
		ProjectConfig {
			base_path,
			data,
			packs,
		}
	}

	/// The config as it was read.
	pub(crate) fn data(&self) -> &Value {
		&self.data
	}

	/// `getRelativePackRoot`: `packs[packId] ?? defaultPackPaths[packId]`.
	fn relative_pack_root(&self, pack_id: &str) -> Prop<'_> {
		let own = self
			.packs
			.as_ref()
			.map_or(Prop::Undefined, |packs| js::own(packs, pack_id));
		own.or_else(|| default_pack_path(pack_id))
	}

	/// `resolvePackPath(packId, filePath)`. An empty id or path counts as
	/// missing, as it does in JavaScript.
	pub(crate) fn resolve_pack_path(
		&self,
		pack_id: Option<&str>,
		file_path: Option<&str>,
	) -> String {
		let pack_id = pack_id.filter(|id| !id.is_empty());
		let file_path = file_path.filter(|path| !path.is_empty());
		match (pack_id, file_path) {
			(None, None) => self.base_path.clone(),
			(None, Some(file_path)) => v1::join(&[&self.base_path, file_path]),
			(Some(pack_id), None) => {
				let root = self.relative_pack_root(pack_id);
				if root.has_positive_length() {
					v1::join(&[&self.base_path, &root.to_js_string()])
				} else {
					v1::join(&[&self.base_path])
				}
			}
			(Some(pack_id), Some(file_path)) => {
				let root = self.relative_pack_root(pack_id).to_js_string();
				v1::join(&[&self.base_path, &format!("{root}/{file_path}")])
			}
		}
	}

	/// `getAvailablePacks()`: each pack id in the config, in JavaScript's key
	/// order, with its resolved root.
	pub(crate) fn available_packs(&self) -> Vec<(String, String)> {
		let keys = self.packs.as_ref().map(js::for_in_keys).unwrap_or_default();
		keys.into_iter()
			.map(|id| {
				let path = self.resolve_pack_path(Some(&id), None);
				(id, path)
			})
			.collect()
	}

	/// `getAvailablePackPaths()`.
	pub(crate) fn available_pack_paths(&self) -> Vec<String> {
		self.available_packs()
			.into_iter()
			.map(|(_, path)| path)
			.collect()
	}
}

/// Definitions a host handed in that are not what editor-packages publishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionsError(String);

impl fmt::Display for DefinitionsError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

impl std::error::Error for DefinitionsError {}

/// Each definition must be an object with a string `id`; mc-project-core
/// would take anything and fail later, somewhere else.
fn definitions(value: Value, what: &str) -> Result<Vec<(String, Value)>, DefinitionsError> {
	let Value::Array(items) = value else {
		return Err(DefinitionsError(format!("{what} must be an array")));
	};
	let mut out = Vec::with_capacity(items.len());
	for (i, item) in items.iter().enumerate() {
		let id = match item {
			Value::Object(object) => match object.get("id") {
				Some(Value::String(id)) => id.clone(),
				_ => {
					return Err(DefinitionsError(format!(
						"{what}[{i}] has no string \"id\""
					)));
				}
			},
			_ => return Err(DefinitionsError(format!("{what}[{i}] is not an object"))),
		};
		out.push((id, item.clone()));
	}
	Ok(out)
}

/// The pack types a project can have (`packDefinitions.json` from
/// editor-packages): behavior pack, resource pack, skin pack, world template.
pub struct PackTypes {
	definitions: Vec<(String, Value)>,
}

impl PackTypes {
	/// Pack types from the parsed `packDefinitions.json`.
	pub fn new(definitions: Value) -> Result<Self, DefinitionsError> {
		Ok(PackTypes {
			definitions: self::definitions(definitions, "pack definitions")?,
		})
	}

	/// `get(filePath)`: the first pack in the config whose root the path
	/// starts with decides, and a pack id without a definition gives no pack
	/// even when a later pack would match.
	pub(crate) fn get(&self, project: &ProjectConfig, path: &str) -> Option<(&str, &Value)> {
		let (id, _) = project
			.available_packs()
			.into_iter()
			.find(|(_, root)| path.starts_with(root.as_str()))?;
		self.definitions
			.iter()
			.find(|(definition_id, _)| *definition_id == id)
			.map(|(id, definition)| (id.as_str(), definition))
	}

	/// `getId(filePath)`: the pack's id, or `"unknown"`.
	pub(crate) fn id(&self, project: &ProjectConfig, path: &str) -> String {
		self.get(project, path)
			.map_or("unknown", |(id, _)| id)
			.to_owned()
	}
}

/// How file definitions that detect files by glob (`detect.matcher`, without
/// a `detect.scope`) are matched. Hosts differ here, so there is no default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectMatcher {
	/// With picomatch, as the Deno CLI sets up mc-project-core.
	Glob,
	/// Never, as the bridge. editor sets it up (`isMatch` is `() => false`),
	/// so a definition that only has matchers detects nothing.
	Never,
}

/// A file definition's `detect`, read once.
struct Detect {
	pack_types: Vec<Value>,
	file_extensions: Option<Value>,
	scope: Option<Vec<Value>>,
	matcher: Option<Vec<Value>>,
}

struct FileDefinition {
	id: String,
	raw: Value,
	detect: Detect,
}

/// Why detecting a file's type threw in mc-project-core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FileTypeError {
	/// A definition the search reached has neither `scope` nor `matcher`.
	/// mc-project-core logs the definition and throws `Invalid file
	/// definition, no "detect" properties`.
	NoDetect(String),
	/// A value in the definition is not the type the JavaScript calls a
	/// method on, such as a `fileExtensions` that is a number.
	NotAFunction(String),
	/// picomatch threw on a matcher.
	Glob(PicomatchError),
}

impl fmt::Display for FileTypeError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NoDetect(_) => {
				f.write_str("Error: Invalid file definition, no \"detect\" properties")
			}
			Self::NotAFunction(what) => write!(f, "TypeError: {what} is not a function"),
			Self::Glob(error) => write!(f, "Error: {error}"),
		}
	}
}

/// The file types a project's files can have (`fileDefinitions.json` from
/// editor-packages), detected by extension, by path prefix (`scope`) or by
/// glob (`matcher`, with `!` for exclusions).
pub struct FileTypes {
	definitions: Vec<FileDefinition>,
	matcher: DetectMatcher,
	globs: Globs,
	/// Results per path, as the Deno CLI's `FileTypeImpl` caches them; a
	/// detection that threw is not cached.
	cache: RefCell<HashMap<String, Option<usize>>>,
}

/// `add: "pre"` sorts first and `add: "post"` last; the sort is stable.
fn add_rank(definition: &Value) -> u8 {
	match js::own(definition, "add") {
		Prop::Value(Value::String(add)) if add == "pre" => 0,
		Prop::Value(Value::String(add)) if add == "post" => 2,
		_ => 1,
	}
}

/// `Array.isArray(v) ? v : [v]`.
fn listify(value: Prop<'_>) -> Vec<Value> {
	match value {
		Prop::Value(Value::Array(items)) => items.iter().cloned().collect(),
		Prop::Value(value) => vec![value.clone()],
		Prop::Undefined | Prop::Inherited(_) => vec![Value::Null],
	}
}

fn is_truthy(value: Prop<'_>) -> bool {
	match value {
		Prop::Undefined => false,
		Prop::Value(value) => js::truthy(value),
		Prop::Inherited(_) => true,
	}
}

impl FileTypes {
	/// File types from the parsed `fileDefinitions.json`, sorted as
	/// mc-project-core sorts them.
	pub fn new(definitions: Value, matcher: DetectMatcher) -> Result<Self, DefinitionsError> {
		let mut definitions: Vec<FileDefinition> =
			self::definitions(definitions, "file definitions")?
				.into_iter()
				.map(|(id, raw)| {
					let detect = js::own(&raw, "detect");
					let field = |name: &str| match detect {
						Prop::Value(detect) => js::own(detect, name),
						_ => Prop::Undefined,
					};
					let pack_types = match field("packType") {
						Prop::Undefined => Vec::new(),
						other => listify(other),
					};
					let file_extensions = match field("fileExtensions") {
						Prop::Value(value) => Some(value.clone()),
						_ => None,
					};
					let scope = is_truthy(field("scope")).then(|| listify(field("scope")));
					let matcher = is_truthy(field("matcher")).then(|| listify(field("matcher")));
					let detect = Detect {
						pack_types,
						file_extensions,
						scope,
						matcher,
					};
					FileDefinition { id, raw, detect }
				})
				.collect();
		definitions.sort_by_key(|definition| add_rank(&definition.raw));
		Ok(FileTypes {
			definitions,
			matcher,
			globs: Globs::default(),
			cache: RefCell::default(),
		})
	}

	/// Every definition, in detection order (`fileType.all`).
	pub(crate) fn all(&self) -> impl Iterator<Item = (&str, &Value)> {
		self.definitions.iter().map(|d| (d.id.as_str(), &d.raw))
	}

	/// `getId(filePath)`: the id of the first definition that detects the
	/// path, or `"unknown"`.
	pub(crate) fn id(&self, project: &ProjectConfig, path: &str) -> Result<String, FileTypeError> {
		Ok(self
			.get(project, path)?
			.map_or("unknown", |(id, _)| id)
			.to_owned())
	}

	/// `get(filePath)`.
	pub(crate) fn get(
		&self,
		project: &ProjectConfig,
		path: &str,
	) -> Result<Option<(&str, &Value)>, FileTypeError> {
		if let Some(found) = self.cache.borrow().get(path) {
			return Ok(found.map(|i| (self.definitions[i].id.as_str(), &self.definitions[i].raw)));
		}
		let found = self.detect(project, path)?;
		self.cache.borrow_mut().insert(path.to_owned(), found);
		Ok(found.map(|i| (self.definitions[i].id.as_str(), &self.definitions[i].raw)))
	}

	fn detect(&self, project: &ProjectConfig, path: &str) -> Result<Option<usize>, FileTypeError> {
		let extension = if path.is_empty() {
			String::new()
		} else {
			v1::extname(path)
		};
		if extension.is_empty() {
			return Ok(None);
		}
		for (index, definition) in self.definitions.iter().enumerate() {
			let detect = &definition.detect;
			if let Some(extensions) = &detect.file_extensions
				&& js::truthy(extensions)
				&& !includes(extensions, &extension)?
			{
				continue;
			}
			if let Some(scope) = &detect.scope {
				let prefixes = prefix_matchers(project, &detect.pack_types, scope.iter().map(Some));
				if prefixes
					.iter()
					.any(|prefix| path.starts_with(prefix.as_str()))
				{
					return Ok(Some(index));
				}
			} else if let Some(matcher) = &detect.matcher {
				let mut include = Vec::new();
				let mut exclude = Vec::new();
				for m in matcher {
					let Value::String(m) = m else {
						return Err(FileTypeError::NotAFunction("m.startsWith".to_owned()));
					};
					match m.strip_prefix('!') {
						Some(excluded) => exclude.push(excluded.to_owned()),
						None => include.push(m.clone()),
					}
				}
				let as_values =
					|list: Vec<String>| list.into_iter().map(Value::String).collect::<Vec<_>>();
				let (include, exclude) = (as_values(include), as_values(exclude));
				let must_match_any =
					prefix_matchers(project, &detect.pack_types, include.iter().map(Some));
				let must_not_match =
					prefix_matchers(project, &detect.pack_types, exclude.iter().map(Some));
				if self.is_match(path, &must_match_any)? && !self.is_match(path, &must_not_match)? {
					return Ok(Some(index));
				}
			} else {
				return Err(FileTypeError::NoDetect(definition.id.clone()));
			}
		}
		Ok(None)
	}

	fn is_match(&self, path: &str, patterns: &[String]) -> Result<bool, FileTypeError> {
		match self.matcher {
			DetectMatcher::Never => Ok(false),
			DetectMatcher::Glob => self
				.globs
				.is_match_any(path, patterns.iter().map(String::as_str))
				.map_err(FileTypeError::Glob),
		}
	}
}

/// `fileExtensions.includes(extension)`: element equality for an array, a
/// substring test for a string, and a TypeError for anything else.
fn includes(extensions: &Value, extension: &str) -> Result<bool, FileTypeError> {
	match extensions {
		Value::Array(items) => Ok(items
			.iter()
			.any(|item| matches!(item, Value::String(s) if s == extension))),
		Value::String(s) => Ok(s.contains(extension)),
		_ => Err(FileTypeError::NotAFunction(
			"fileExtensions.includes".to_owned(),
		)),
	}
}

/// `prefixMatchers(packTypes, matchers)`: each matcher resolved inside each
/// pack type's root, or inside the project when the definition names no pack
/// type.
fn prefix_matchers<'a>(
	project: &ProjectConfig,
	pack_types: &[Value],
	matchers: impl Iterator<Item = Option<&'a Value>> + Clone,
) -> Vec<String> {
	// A falsy value counts as missing in resolvePackPath; anything else goes
	// through a template literal.
	let key = |value: Option<&Value>| match value {
		Some(value) if js::truthy(value) => Some(js::to_js_string(value)),
		_ => None,
	};
	if pack_types.is_empty() {
		return matchers
			.map(|m| project.resolve_pack_path(None, key(m).as_deref()))
			.collect();
	}
	let mut prefixed = Vec::new();
	for pack_type in pack_types {
		for m in matchers.clone() {
			prefixed.push(
				project.resolve_pack_path(key(Some(pack_type)).as_deref(), key(m).as_deref()),
			);
		}
	}
	prefixed
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::json::parse_json5;

	fn read(name: &str) -> String {
		let path = format!("{}/tests/{name}", env!("CARGO_MANIFEST_DIR"));
		std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
	}

	fn definitions(name: &str) -> Value {
		parse_json5(&read(&format!("data/{name}"))).expect("the definitions are JSON")
	}

	fn text(value: &serde_json::Value) -> Option<&str> {
		value.as_str()
	}

	/// project.json, recorded by tools/parity/project.mjs from
	/// mc-project-core 0.5.0 with the vendored definitions.
	#[test]
	fn pack_paths_and_file_and_pack_types_match_mc_project_core() {
		let vectors: Vec<serde_json::Value> =
			serde_json::from_str(&read("vectors/project.json")).expect("JSON");
		assert!(vectors.len() > 15);
		let packs = PackTypes::new(definitions("packDefinitions.json")).expect("pack definitions");
		let mut failures = Vec::new();
		let mut checked = 0;
		for vector in &vectors {
			let base_path = text(&vector["basePath"]).expect("a base path").to_owned();
			let data = parse_json5(&vector["data"].to_string()).expect("the config is JSON");
			let project = ProjectConfig::new(base_path.clone(), data);
			let (glob_types, never_types) = match vector.get("definitions") {
				Some(custom) => {
					let custom = parse_json5(&custom.to_string()).expect("JSON");
					let again = parse_json5(&vector["definitions"].to_string()).expect("JSON");
					(
						FileTypes::new(custom, DetectMatcher::Glob).expect("definitions"),
						FileTypes::new(again, DetectMatcher::Never).expect("definitions"),
					)
				}
				None => (
					FileTypes::new(definitions("fileDefinitions.json"), DetectMatcher::Glob)
						.expect("definitions"),
					FileTypes::new(definitions("fileDefinitions.json"), DetectMatcher::Never)
						.expect("definitions"),
				),
			};
			let expected_packs: Vec<(String, String)> = vector["packs"]
				.as_object()
				.expect("an object")
				.iter()
				.map(|(k, v)| (k.clone(), v.as_str().expect("a path").to_owned()))
				.collect();
			if project.available_packs() != expected_packs {
				failures.push(format!(
					"{base_path} packs: expected {expected_packs:?}, got {:?}",
					project.available_packs()
				));
			}
			let expected_paths: Vec<&str> = vector["packPaths"]
				.as_array()
				.expect("an array")
				.iter()
				.filter_map(text)
				.collect();
			if project.available_pack_paths() != expected_paths {
				failures.push(format!(
					"{base_path} pack paths: expected {expected_paths:?}"
				));
			}
			for case in vector["resolve"].as_array().expect("an array") {
				checked += 1;
				let actual = project.resolve_pack_path(text(&case[0]), text(&case[1]));
				if Some(actual.as_str()) != text(&case[2]) {
					failures.push(format!(
						"{base_path} resolvePackPath({}, {}): expected {}, got {actual}",
						case[0], case[1], case[2]
					));
				}
			}
			for case in vector["files"].as_array().expect("an array") {
				checked += 1;
				let path = text(&case[0]).expect("a path");
				let outcome = |result: Result<String, FileTypeError>| {
					result.unwrap_or_else(|_| "throws".to_owned())
				};
				let glob_id = outcome(glob_types.id(&project, path));
				let never_id = outcome(never_types.id(&project, path));
				let pack_id = packs.id(&project, path);
				if Some(glob_id.as_str()) != text(&case[1]) {
					failures.push(format!(
						"{base_path} file type (glob) of {path:?}: expected {}, got {glob_id}",
						case[1]
					));
				}
				if text(&case[2]) != Some("skip") && Some(never_id.as_str()) != text(&case[2]) {
					failures.push(format!(
						"{base_path} file type (never) of {path:?}: expected {}, got {never_id}",
						case[2]
					));
				}
				if Some(pack_id.as_str()) != text(&case[3]) {
					failures.push(format!(
						"{base_path} pack type of {path:?}: expected {}, got {pack_id}",
						case[3]
					));
				}
			}
		}
		assert!(
			failures.is_empty(),
			"{} of {checked} differ:\n{}",
			failures.len(),
			failures.join("\n")
		);
	}

	#[test]
	fn definitions_that_are_not_objects_with_string_ids_are_refused() {
		for (source, message) in [
			("{}", "file definitions must be an array"),
			("[1]", "file definitions[0] is not an object"),
			("[{id: 1}]", "file definitions[0] has no string \"id\""),
			("[{}]", "file definitions[0] has no string \"id\""),
		] {
			let error =
				FileTypes::new(parse_json5(source).expect("json5"), DetectMatcher::Glob).err();
			assert_eq!(
				error.map(|e| e.to_string()),
				Some(message.to_owned()),
				"{source}"
			);
		}
		assert!(PackTypes::new(parse_json5("[null]").expect("json5")).is_err());
	}

	#[test]
	fn a_detection_that_threw_is_not_cached() {
		let definitions = parse_json5("[{id: 'broken'}]").expect("json5");
		let file_types = FileTypes::new(definitions, DetectMatcher::Glob).expect("definitions");
		let project = ProjectConfig::new(".".to_owned(), Value::Null);
		for _ in 0..2 {
			assert_eq!(
				file_types.id(&project, "x.json"),
				Err(FileTypeError::NoDetect("broken".to_owned()))
			);
		}
		assert_eq!(file_types.id(&project, "x"), Ok("unknown".to_owned()));
	}
}
