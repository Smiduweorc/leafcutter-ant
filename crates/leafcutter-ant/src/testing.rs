//! What the unit tests share: an in-memory file system, and a compiler set
//! up with one built-in plugin and the vendored definitions.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::fs::{DirEntry, EntryKind, FileSystem, FsError, FsFuture};
use crate::pathe;

/// Files in memory, keyed by normalized path.
#[derive(Default)]
pub(crate) struct MemoryFs {
	pub(crate) files: RefCell<BTreeMap<String, Vec<u8>>>,
	/// Every path read, in order.
	pub(crate) reads: RefCell<Vec<String>>,
}

fn key(path: &str) -> String {
	let normalized = pathe::normalize(path);
	normalized
		.strip_prefix("./")
		.unwrap_or(&normalized)
		.trim_end_matches('/')
		.to_owned()
}

impl MemoryFs {
	pub(crate) fn with(files: &[(&str, &str)]) -> Rc<Self> {
		let fs = MemoryFs::default();
		for (path, content) in files {
			fs.files
				.borrow_mut()
				.insert(key(path), content.as_bytes().to_vec());
		}
		Rc::new(fs)
	}

	pub(crate) fn text(&self, path: &str) -> Option<String> {
		self.files
			.borrow()
			.get(&key(path))
			.map(|bytes| String::from_utf8(bytes.clone()).expect("UTF-8"))
	}

	pub(crate) fn paths(&self) -> Vec<String> {
		self.files.borrow().keys().cloned().collect()
	}
}

impl FileSystem for MemoryFs {
	fn read_file<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<u8>> {
		Box::pin(async move {
			self.reads.borrow_mut().push(path.to_owned());
			self.files
				.borrow()
				.get(&key(path))
				.cloned()
				.ok_or_else(|| FsError::new(path, "not found"))
		})
	}

	fn write_file<'a>(&'a self, path: &'a str, content: &'a [u8]) -> FsFuture<'a, ()> {
		Box::pin(async move {
			self.files.borrow_mut().insert(key(path), content.to_vec());
			Ok(())
		})
	}

	fn unlink<'a>(&'a self, path: &'a str) -> FsFuture<'a, ()> {
		Box::pin(async move {
			let prefix = format!("{}/", key(path));
			let mut files = self.files.borrow_mut();
			let before = files.len();
			files.retain(|k, _| *k != key(path) && !k.starts_with(&prefix));
			if files.len() == before {
				return Err(FsError::new(path, "not found"));
			}
			Ok(())
		})
	}

	fn readdir<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<DirEntry>> {
		Box::pin(async move {
			let dir = key(path);
			let prefix = if dir.is_empty() || dir == "." {
				String::new()
			} else {
				format!("{dir}/")
			};
			let mut entries: Vec<DirEntry> = Vec::new();
			for k in self.files.borrow().keys() {
				let Some(rest) = k.strip_prefix(&prefix) else {
					continue;
				};
				let (name, kind) = match rest.split_once('/') {
					Some((name, _)) => (name, EntryKind::Directory),
					None => (rest, EntryKind::File),
				};
				if !entries.iter().any(|e| e.name == name) {
					entries.push(DirEntry {
						name: name.to_owned(),
						kind,
					});
				}
			}
			if entries.is_empty() {
				return Err(FsError::new(path, "not a directory"));
			}
			Ok(entries)
		})
	}

	fn mkdir<'a>(&'a self, _path: &'a str) -> FsFuture<'a, ()> {
		Box::pin(async { Ok(()) })
	}

	fn last_modified<'a>(&'a self, _path: &'a str) -> FsFuture<'a, f64> {
		Box::pin(async { Ok(0.0) })
	}
}

fn vendored(name: &str) -> crate::json::Value {
	let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
	let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
	crate::json::parse_json5(&text).expect("the definitions are JSON")
}

/// A development compiler for a project with a behavior and a resource pack
/// whose plugin list is `plugins`, set up and ready to build, and the console
/// it reports to.
pub(crate) fn dash_with(
	fs: Rc<MemoryFs>,
	plugins: &str,
) -> (crate::Dash, Rc<crate::console::tests::Recorder>) {
	let config = format!(
		r#"{{"packs": {{"behaviorPack": "./BP", "resourcePack": "./RP"}}, "compiler": {{"plugins": {plugins}}}}}"#
	);
	fs.files
		.borrow_mut()
		.insert("config.json".to_owned(), config.into_bytes());
	let recorder = Rc::new(crate::console::tests::Recorder::default());
	let options = crate::DashOptions {
		config: "./config.json".to_owned(),
		compiler_config: None,
		request_json_data: no_request_json_data(),
		https_imports: crate::HttpsImports::Refused,
		script_time_limit: crate::ScriptTimeLimit::Unlimited,
		mode: crate::Mode::Development,
		console: recorder.clone(),
		verbose: false,
		pack_types: crate::project::PackTypes::new(vendored("packDefinitions.json"))
			.expect("pack definitions"),
		file_types: crate::project::FileTypes::new(
			vendored("fileDefinitions.json"),
			crate::project::DetectMatcher::Glob,
		)
		.expect("file definitions"),
	};
	let dash = crate::Dash::new(fs, None, options);
	futures_executor::block_on(dash.setup()).expect("setup succeeds");
	(dash, recorder)
}

/// A `requestJsonData` that has no data.
pub(crate) fn no_request_json_data() -> crate::RequestJsonData {
	Rc::new(|path: &str| {
		let message = format!("Error: no data at {path}");
		Box::pin(std::future::ready(Err(message)))
	})
}

/// A vector file as JSON.
pub(crate) fn vectors(name: &str) -> serde_json::Value {
	let path = format!("{}/tests/vectors/{name}", env!("CARGO_MANIFEST_DIR"));
	let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
	serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The recorded answers of one plugin in plugins.json.
pub(crate) fn plugin_vectors(plugin: &str) -> Vec<serde_json::Value> {
	vectors("plugins.json")
		.as_array()
		.expect("an array")
		.iter()
		.find(|entry| entry[0] == plugin)
		.and_then(|entry| entry[1].as_array().cloned())
		.unwrap_or_else(|| panic!("no vectors for {plugin}"))
}

/// A serde value as one of ours, keys in the order the JSON lists them.
pub(crate) fn value(json: &serde_json::Value) -> crate::json::Value {
	crate::json::parse_json5(&json.to_string()).expect("JSON is json5")
}
