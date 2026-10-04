//! The files of a build and what Dash knows about each: `DashFile` and
//! `IncludedFiles` (`src/Core/DashFile.ts`, `src/Core/IncludedFiles.ts`),
//! including the `.bridge/.dash.<mode>.json` cache they are saved to.

use std::collections::HashMap;

use indexmap::{IndexMap, IndexSet};

use crate::glob::{Globs, is_glob};
use crate::json::{Array, Object, Value};
use crate::plugin::{Data, Hook};

/// An index into [`IncludedFiles`].
pub(crate) type FileId = usize;

/// An alias as a JavaScript `Map` or `Set` key compares it: strings, numbers,
/// booleans and `null` by value (with `NaN` equal to itself and `-0` to `0`),
/// and each object or array by identity, which a fresh alias never shares.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AliasKey {
	String(String),
	Number(u64),
	Bool(bool),
	Null,
	Object(u64),
}

/// One alias: its key and the value the cache file writes.
pub(crate) struct Alias {
	pub(crate) key: AliasKey,
	pub(crate) value: Value,
}

pub(crate) struct DashFile {
	pub(crate) path: String,
	pub(crate) is_virtual: bool,
	pub(crate) output_path: Option<String>,
	pub(crate) is_done: bool,
	pub(crate) data: Option<Data>,
	pub(crate) required_files: IndexSet<String>,
	pub(crate) aliases: Vec<Alias>,
	pub(crate) update_files: IndexSet<FileId>,
	pub(crate) metadata: IndexMap<String, Value>,
	pub(crate) ignored_by: Vec<String>,
	/// For each hook, the plugins that do not ignore this file
	/// (`myImplementedHooks`); `None` until the ignore hooks have run.
	pub(crate) hooks: Option<[Vec<usize>; 12]>,
	/// The file's bytes, read when some `read` hook applies to it; `Err` when
	/// the read failed, which the file handle reports as `null`.
	pub(crate) content: Option<Result<Vec<u8>, ()>>,
}

impl DashFile {
	fn new(path: String, is_virtual: bool) -> Self {
		DashFile {
			output_path: Some(path.clone()),
			path,
			is_virtual,
			is_done: false,
			data: None,
			required_files: IndexSet::new(),
			aliases: Vec::new(),
			update_files: IndexSet::new(),
			metadata: IndexMap::new(),
			ignored_by: Vec::new(),
			hooks: None,
			content: None,
		}
	}

	/// The plugins that implement `hook` and do not ignore this file.
	pub(crate) fn plugins_for(&self, hook: Hook) -> Option<&[usize]> {
		self.hooks
			.as_ref()
			.map(|hooks| hooks[hook.index()].as_slice())
	}

	/// `reset()`, after a build.
	pub(crate) fn reset(&mut self) {
		self.is_done = false;
		self.data = None;
		self.hooks = None;
		self.content = None;
	}
}

/// Every file of the build, in the order they were added, with their aliases.
#[derive(Default)]
pub(crate) struct IncludedFiles {
	arena: Vec<DashFile>,
	files: IndexMap<String, FileId>,
	aliases: IndexMap<AliasKey, FileId>,
	query_cache: HashMap<String, Vec<FileId>>,
	next_object: u64,
}

impl IncludedFiles {
	pub(crate) fn file(&self, id: FileId) -> &DashFile {
		&self.arena[id]
	}

	pub(crate) fn file_mut(&mut self, id: FileId) -> &mut DashFile {
		&mut self.arena[id]
	}

	/// The file with this id, when there is one: an id a script kept from an
	/// earlier build may be gone.
	pub(crate) fn checked(&self, id: FileId) -> Option<&DashFile> {
		self.arena.get(id)
	}

	pub(crate) fn checked_mut(&mut self, id: FileId) -> Option<&mut DashFile> {
		self.arena.get_mut(id)
	}

	/// Every alias, in the order the alias map holds them
	/// (`getAliasesWhere`).
	pub(crate) fn alias_values(&self) -> Vec<Value> {
		self.aliases
			.iter()
			.map(|(key, &id)| match key {
				AliasKey::String(s) => Value::String(s.clone()),
				AliasKey::Number(bits) => Value::Number(f64::from_bits(*bits)),
				AliasKey::Bool(b) => Value::Bool(*b),
				AliasKey::Null => Value::Null,
				AliasKey::Object(_) => self.arena[id]
					.aliases
					.iter()
					.find(|alias| alias.key == *key)
					.map_or(Value::Null, |alias| alias.value.clone()),
			})
			.collect()
	}

	/// `all()`: every file, in insertion order.
	pub(crate) fn all(&self) -> Vec<FileId> {
		self.files.values().copied().collect()
	}

	/// `get(fileId)`: an alias first, then a path.
	pub(crate) fn get(&self, id: &str) -> Option<FileId> {
		self.aliases
			.get(&AliasKey::String(id.to_owned()))
			.or_else(|| self.files.get(id))
			.copied()
	}

	/// `query(query)`: an alias, a path, or every file whose path matches the
	/// query when is-glob calls it a glob. Glob results are cached until the
	/// next build loads the file list (`loadAll`).
	pub(crate) fn query(&mut self, globs: &Globs, query: &str) -> Vec<FileId> {
		if let Some(id) = self.get(query) {
			return vec![id];
		}
		if !is_glob(query) {
			return Vec::new();
		}
		if let Some(cached) = self.query_cache.get(query) {
			return cached.clone();
		}
		// isMatch throwing propagates out of query in TS Dash, into the
		// caller's hook; here a pattern picomatch refuses matches nothing.
		let matched: Vec<FileId> = self
			.all()
			.into_iter()
			.filter(|&id| globs.is_match(&self.arena[id].path, query).unwrap_or(false))
			.collect();
		self.query_cache.insert(query.to_owned(), matched.clone());
		matched
	}

	pub(crate) fn clear_query_cache(&mut self) {
		self.query_cache.clear();
	}

	/// `addOne(filePath, isVirtual)`: a new file, replacing any file already
	/// at the path.
	pub(crate) fn add_one(&mut self, path: String, is_virtual: bool) -> FileId {
		let id = self.arena.len();
		self.arena.push(DashFile::new(path.clone(), is_virtual));
		self.files.insert(path, id);
		id
	}

	/// `add(filePaths)`: files for paths not yet included, keeping those that
	/// are.
	pub(crate) fn add(
		&mut self,
		paths: impl IntoIterator<Item = String>,
		is_virtual: bool,
	) -> Vec<FileId> {
		paths
			.into_iter()
			.map(|path| match self.files.get(&path) {
				Some(&id) => id,
				None => self.add_one(path, is_virtual),
			})
			.collect()
	}

	/// A key for an alias value, as a `Map` would compare it.
	pub(crate) fn alias_key(&mut self, value: &Value) -> AliasKey {
		match value {
			Value::String(s) => AliasKey::String(s.clone()),
			Value::Number(n) if n.is_nan() => AliasKey::Number(f64::NAN.to_bits()),
			Value::Number(n) => AliasKey::Number((n + 0.0).to_bits()),
			Value::Bool(b) => AliasKey::Bool(*b),
			Value::Null => AliasKey::Null,
			Value::Array(_) | Value::Object(_) => {
				self.next_object += 1;
				AliasKey::Object(self.next_object)
			}
		}
	}

	/// `file.setAliases(aliases)`: each alias points at the file from now on,
	/// taking over an alias another file registered (the alias keeps its
	/// place in the map, as a `Map` key does), and the file's aliases become
	/// these. Aliases the file had before stay in the map.
	pub(crate) fn set_aliases(&mut self, id: FileId, values: Vec<Value>) {
		let mut aliases: Vec<Alias> = Vec::new();
		for value in values {
			let key = self.alias_key(&value);
			if !aliases.iter().any(|alias| alias.key == key) {
				aliases.push(Alias { key, value });
			}
		}
		for alias in &aliases {
			self.aliases.insert(alias.key.clone(), id);
		}
		self.arena[id].aliases = aliases;
	}

	/// `removeAll()`.
	pub(crate) fn remove_all(&mut self) {
		*self = IncludedFiles {
			next_object: self.next_object,
			..IncludedFiles::default()
		};
	}

	/// `resetAll()`.
	pub(crate) fn reset_all(&mut self) {
		for id in self.all() {
			self.arena[id].reset();
		}
	}

	/// The cache file's content: `JSON.stringify` of `serialize()` for every
	/// file. `metadata` is left out when the file has none, as
	/// `JSON.stringify` leaves out `undefined`.
	pub(crate) fn serialize(&self) -> Value {
		let mut files = Array::new();
		for id in self.all() {
			let file = &self.arena[id];
			let mut object = Object::new();
			object.insert("isVirtual".to_owned(), Value::Bool(file.is_virtual));
			object.insert("filePath".to_owned(), Value::String(file.path.clone()));
			let aliases: Vec<Value> = file
				.aliases
				.iter()
				.map(|alias| alias.value.clone())
				.collect();
			object.insert("aliases".to_owned(), Value::Array(aliases.into()));
			let required: Vec<Value> = file
				.required_files
				.iter()
				.cloned()
				.map(Value::String)
				.collect();
			object.insert("requiredFiles".to_owned(), Value::Array(required.into()));
			let updates: Vec<Value> = file
				.update_files
				.iter()
				.map(|&update| Value::String(self.arena[update].path.clone()))
				.collect();
			object.insert("updateFiles".to_owned(), Value::Array(updates.into()));
			if !file.metadata.is_empty() {
				let mut metadata = Object::new();
				for (key, value) in &file.metadata {
					metadata.insert(key.clone(), value.clone());
				}
				object.insert("metadata".to_owned(), Value::Object(metadata));
			}
			files.push(Value::Object(object));
		}
		Value::Array(files)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::json::{Indent, parse_json5, stringify};

	#[test]
	fn a_path_added_twice_is_one_file_and_add_one_replaces() {
		let mut files = IncludedFiles::default();
		let first = files.add(["a".to_owned(), "b".to_owned(), "a".to_owned()], false);
		assert_eq!(first, [0, 1, 0]);
		let virtual_a = files.add_one("a".to_owned(), true);
		assert_eq!(files.all(), [virtual_a, 1]);
		assert!(
			files
				.file(files.get("a").expect("a is included"))
				.is_virtual
		);
	}

	#[test]
	fn an_alias_taken_over_keeps_its_place_and_points_at_the_last_file() {
		let mut files = IncludedFiles::default();
		let ids = files.add(["x.json".to_owned(), "y.json".to_owned()], false);
		files.set_aliases(
			ids[0],
			vec![
				Value::String("ns:a".to_owned()),
				Value::String("ns:b".to_owned()),
			],
		);
		files.set_aliases(ids[1], vec![Value::String("ns:a".to_owned())]);
		assert_eq!(files.get("ns:a"), Some(ids[1]));
		assert_eq!(files.get("ns:b"), Some(ids[0]));
		let order: Vec<&AliasKey> = files.aliases.keys().collect();
		assert_eq!(
			order,
			[
				&AliasKey::String("ns:a".to_owned()),
				&AliasKey::String("ns:b".to_owned())
			]
		);
	}

	#[test]
	fn aliases_compare_as_map_keys_do() {
		let mut files = IncludedFiles::default();
		let id = files.add(["x".to_owned()], false)[0];
		let values =
			parse_json5("['a', 'a', 1, 1.0, -0, 0, '1', true, null, [], []]").expect("json5");
		let Value::Array(values) = values else {
			unreachable!()
		};
		files.set_aliases(id, values.iter().cloned().collect());
		assert_eq!(
			stringify(&files.serialize(), Indent::None),
			r#"[{"isVirtual":false,"filePath":"x","aliases":["a",1,0,"1",true,null,[],[]],"requiredFiles":[],"updateFiles":[]}]"#
		);
		assert_eq!(files.get("1"), Some(id));
	}

	#[test]
	fn a_glob_query_finds_matching_paths_and_a_plain_one_needs_an_exact_path() {
		let globs = Globs::default();
		let mut files = IncludedFiles::default();
		files.add(
			[
				"BP/a.json".to_owned(),
				"BP/sub/b.json".to_owned(),
				"RP/c.json".to_owned(),
			],
			false,
		);
		assert_eq!(files.query(&globs, "BP/**/*.json"), [0, 1]);
		assert_eq!(files.query(&globs, "BP/a.json"), [0]);
		assert_eq!(files.query(&globs, "BP/b.json"), Vec::<FileId>::new());
		assert_eq!(files.query(&globs, "BP/*.json"), [0]);
	}

	#[test]
	fn the_cache_lists_update_files_by_path_and_metadata_only_when_there_is_some() {
		let mut files = IncludedFiles::default();
		let ids = files.add(["BP/a.json".to_owned(), "BP/b.json".to_owned()], false);
		files.add_one("BP/contents.json".to_owned(), true);
		files
			.file_mut(ids[0])
			.required_files
			.insert("BP/b.json".to_owned());
		files.file_mut(ids[1]).update_files.insert(ids[0]);
		files.file_mut(ids[1]).metadata.insert(
			"generatedFiles".to_owned(),
			parse_json5("['BP/x.json']").expect("json5"),
		);
		assert_eq!(
			stringify(&files.serialize(), Indent::None),
			concat!(
				r#"[{"isVirtual":false,"filePath":"BP/a.json","aliases":[],"requiredFiles":["BP/b.json"],"updateFiles":[]},"#,
				r#"{"isVirtual":false,"filePath":"BP/b.json","aliases":[],"requiredFiles":[],"updateFiles":["BP/a.json"],"metadata":{"generatedFiles":["BP/x.json"]}},"#,
				r#"{"isVirtual":true,"filePath":"BP/contents.json","aliases":[],"requiredFiles":[],"updateFiles":[]}]"#
			)
		);
	}
}
