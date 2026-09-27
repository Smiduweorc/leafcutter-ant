//! The pipeline and plugin host, driven through `Dash` with an in-memory file
//! system and small plugins defined here. Each test cites the part of TS
//! Dash it pins.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use futures_executor::block_on;

use super::*;
use crate::console::tests::Recorder;
use crate::fs::{DirEntry, EntryKind, FsFuture};
use crate::json::{Indent, parse_json5, stringify};
use crate::plugin::{Data, PathChange, Plugin};

/// Files in memory, keyed by normalized path.
#[derive(Default)]
struct MemoryFs {
	files: RefCell<BTreeMap<String, Vec<u8>>>,
	reads: RefCell<Vec<String>>,
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
	fn with(files: &[(&str, &str)]) -> Rc<Self> {
		let fs = MemoryFs::default();
		for (path, content) in files {
			fs.files
				.borrow_mut()
				.insert(key(path), content.as_bytes().to_vec());
		}
		Rc::new(fs)
	}

	fn text(&self, path: &str) -> Option<String> {
		self.files
			.borrow()
			.get(&key(path))
			.map(|bytes| String::from_utf8(bytes.clone()).expect("UTF-8"))
	}

	fn paths(&self) -> Vec<String> {
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

const PACKS: &str = r#"[{"id": "behaviorPack", "defaultPackPath": "BP"}, {"id": "resourcePack", "defaultPackPath": "RP"}]"#;

fn dash_with(fs: Rc<MemoryFs>, mode: Mode) -> (Dash, Rc<Recorder>) {
	let recorder = Rc::new(Recorder::default());
	let options = DashOptions {
		config: "./config.json".to_owned(),
		compiler_config: None,
		mode,
		console: recorder.clone(),
		verbose: false,
		pack_types: PackTypes::new(parse_json5(PACKS).expect("json")).expect("pack definitions"),
		file_types: FileTypes::new(
			parse_json5(r#"[{"id": "entity", "detect": {"packType": "behaviorPack", "scope": "entities/", "fileExtensions": [".json"]}}]"#)
				.expect("json"),
			crate::project::DetectMatcher::Glob,
		)
		.expect("file definitions"),
	};
	(Dash::new(fs, None, options), recorder)
}

const CONFIG: &str = r#"{"packs": {"behaviorPack": "./BP"}, "compiler": {"plugins": []}}"#;

type IgnoreFn = dyn Fn(&str) -> bool;
type PathFn = dyn Fn(&str) -> Result<PathChange, String>;
type ReadFn = dyn Fn(&str, FileHandle<'_>) -> Result<Option<Data>, String>;
type ChainFn = dyn Fn(&str, &mut Data) -> Option<Data>;
type AliasFn = dyn Fn(&str) -> Option<Vec<Value>>;
type RequireFn = dyn Fn(&str) -> Option<Vec<String>>;
type FinalizeFn = dyn Fn(&str, &Data) -> Finalized;

/// A plugin assembled from closures, one per hook it implements.
#[derive(Default)]
struct TestPlugin {
	hooks: Vec<Hook>,
	ignore: Option<Box<IgnoreFn>>,
	transform_path: Option<Box<PathFn>>,
	read: Option<Box<ReadFn>>,
	load: Option<Box<ChainFn>>,
	aliases: Option<Box<AliasFn>>,
	require: Option<Box<RequireFn>>,
	transform: Option<Box<ChainFn>>,
	finalize: Option<Box<FinalizeFn>>,
	include: Option<Vec<(String, Option<bool>)>>,
	log: Option<Rc<RefCell<Vec<String>>>>,
	name: &'static str,
}

impl TestPlugin {
	fn record(&self, event: String) {
		if let Some(log) = &self.log {
			log.borrow_mut().push(format!("{} {event}", self.name));
		}
	}
}

impl Plugin for TestPlugin {
	fn hooks(&self) -> &[Hook] {
		&self.hooks
	}

	fn include(&mut self, _cx: &Context) -> Result<Option<Vec<Include>>, String> {
		Ok(self.include.as_ref().map(|entries| {
			entries
				.iter()
				.map(|(path, is_virtual)| match is_virtual {
					Some(is_virtual) => Include::Entry(path.clone(), *is_virtual),
					None => Include::Path(path.clone()),
				})
				.collect()
		}))
	}

	fn ignore(&mut self, _cx: &Context, path: &str) -> Result<bool, String> {
		Ok(self.ignore.as_ref().is_some_and(|f| f(path)))
	}

	fn transform_path(&mut self, _cx: &Context, path: &str) -> Result<PathChange, String> {
		self.record(format!("transformPath {path}"));
		self.transform_path
			.as_ref()
			.map_or(Ok(PathChange::Keep), |f| f(path))
	}

	fn read(
		&mut self,
		_cx: &Context,
		path: &str,
		file: FileHandle<'_>,
	) -> Result<Option<Data>, String> {
		self.record(format!("read {path}"));
		self.read.as_ref().map_or(Ok(None), |f| f(path, file))
	}

	fn load(&mut self, _cx: &Context, path: &str, data: &mut Data) -> Result<Option<Data>, String> {
		self.record(format!("load {path}"));
		Ok(self.load.as_ref().and_then(|f| f(path, data)))
	}

	fn register_aliases(
		&mut self,
		_cx: &Context,
		path: &str,
		_data: &Data,
	) -> Result<Option<Vec<Value>>, String> {
		Ok(self.aliases.as_ref().and_then(|f| f(path)))
	}

	fn require(
		&mut self,
		_cx: &Context,
		path: &str,
		_data: Option<&Data>,
	) -> Result<Option<Vec<String>>, String> {
		Ok(self.require.as_ref().and_then(|f| f(path)))
	}

	fn transform(
		&mut self,
		_cx: &Context,
		path: &str,
		data: &mut Data,
	) -> Result<Option<Data>, String> {
		self.record(format!("transform {path}"));
		Ok(self.transform.as_ref().and_then(|f| f(path, data)))
	}

	fn finalize_build(
		&mut self,
		_cx: &Context,
		path: &str,
		data: &Data,
	) -> Result<Finalized, String> {
		Ok(self
			.finalize
			.as_ref()
			.map_or(Finalized::Undefined, |f| f(path, data)))
	}
}

fn text(s: &str) -> Data {
	Data::Value(Value::String(s.to_owned()))
}

fn json(source: &str) -> Data {
	Data::Value(parse_json5(source).expect("json5"))
}

/// Reads every file as its text and moves it from `BP/` to `out/`.
fn reader() -> TestPlugin {
	TestPlugin {
		hooks: vec![Hook::TransformPath, Hook::Read],
		transform_path: Some(Box::new(|path| {
			Ok(PathChange::To(path.replacen("BP/", "out/", 1)))
		})),
		read: Some(Box::new(|_, file| match file {
			FileHandle::File(bytes) => Ok(Some(text(&String::from_utf8_lossy(bytes)))),
			_ => Ok(None),
		})),
		..TestPlugin::default()
	}
}

fn build(dash: &mut Dash) {
	block_on(dash.setup()).expect("setup succeeds");
	block_on(dash.build()).expect("the build succeeds");
}

fn setup_with(dash: &mut Dash, plugins: Vec<(&str, TestPlugin)>) {
	block_on(dash.setup()).expect("setup succeeds");
	for (id, plugin) in plugins {
		dash.plugins.add(id.to_owned(), Box::new(plugin));
	}
}

#[test]
fn without_a_compiler_plugin_array_nothing_is_built() {
	for config in [
		r#"{"packs": {"behaviorPack": "./BP"}}"#,
		r#"{"compiler": {"plugins": {}}}"#,
		r#"{"compiler": 5}"#,
	] {
		let fs = MemoryFs::with(&[("config.json", config), ("BP/a.json", "{}")]);
		let (mut dash, recorder) = dash_with(fs.clone(), Mode::Development);
		build(&mut dash);
		assert_eq!(fs.paths(), ["BP/a.json", "config.json"], "{config}");
		assert_eq!(*recorder.0.borrow(), ["log: Starting compilation..."]);
	}
}

#[test]
fn a_null_compiler_stops_the_build_as_the_type_error_does_in_ts_dash() {
	// Dash.ts isCompilerActivated reads `config.compiler.plugins` after only
	// checking `!== undefined`.
	let fs = MemoryFs::with(&[("config.json", r#"{"compiler": null}"#)]);
	let (mut dash, _) = dash_with(fs, Mode::Development);
	block_on(dash.setup()).expect("setup succeeds");
	assert!(matches!(
		block_on(dash.build()),
		Err(DashError::CompilerNull)
	));
}

#[test]
fn an_unreadable_config_is_reported_and_treated_as_empty() {
	let fs = MemoryFs::with(&[("config.json", "{packs:")]);
	let (mut dash, recorder) = dash_with(fs, Mode::Development);
	build(&mut dash);
	assert_eq!(
		*recorder.0.borrow(),
		[
			"error: Failed to load project config: ./config.json: Invalid JSON: ./config.json",
			"log: Starting compilation..."
		]
	);
}

#[test]
fn plugin_list_entries_name_builtins_and_unknown_or_javascript_plugins_are_reported() {
	let config = r#"{"compiler": {"plugins": ["nope", ["moLang", {}], [5], {"0": "customCommands"}, "constructor", "fromExtension"]}}"#;
	let fs = MemoryFs::with(&[
		("config.json", config),
		(
			".bridge/extensions/ext/manifest.json",
			r#"{"compiler": {"plugins": {"fromExtension": "plugin.js"}}}"#,
		),
	]);
	let (mut dash, recorder) = dash_with(fs, Mode::Development);
	block_on(dash.setup()).expect("setup succeeds");
	assert_eq!(
		*recorder.0.borrow(),
		[
			"error: Unknown compiler plugin: nope",
			"error: The built-in plugin moLang needs a JavaScript runtime, which leafcutter-ant does not have yet",
			"error: Unknown compiler plugin: 5",
			"error: The built-in plugin customCommands needs a JavaScript runtime, which leafcutter-ant does not have yet",
			"error: Failed to execute plugin constructor: leafcutter-ant cannot run JavaScript plugins yet",
			"error: Failed to execute plugin fromExtension: leafcutter-ant cannot run JavaScript plugins yet",
		]
	);
}

#[test]
fn a_plugin_list_ts_dash_cannot_read_fails_setup() {
	// AllPlugins.ts loadPlugins: `usedPlugin[0]` on null and `.plugins` on a
	// null compiler config throw.
	for config in [
		r#"{"compiler": {"plugins": [null]}}"#,
		r#"{"compiler": {"plugins": {"length": 1}}}"#,
	] {
		let fs = MemoryFs::with(&[("config.json", config)]);
		let (mut dash, _) = dash_with(fs, Mode::Development);
		assert!(
			matches!(block_on(dash.setup()), Err(DashError::PluginList(_))),
			"{config}"
		);
	}
	let fs = MemoryFs::with(&[("config.json", CONFIG), ("compiler.json", "null")]);
	let (mut dash, _) = dash_with(fs.clone(), Mode::Development);
	dash.compiler_config = Some("compiler.json".to_owned());
	assert!(matches!(
		block_on(dash.setup()),
		Err(DashError::PluginList(_))
	));
	dash.compiler_config = Some("missing.json".to_owned());
	assert!(matches!(
		block_on(dash.setup()),
		Err(DashError::CompilerConfig(_))
	));
}

#[test]
fn files_are_read_transformed_and_written_to_their_output_path() {
	let fs = MemoryFs::with(&[
		("config.json", CONFIG),
		("BP/b.txt", "bee"),
		("BP/a/x.txt", "ex"),
	]);
	let (mut dash, _) = dash_with(fs.clone(), Mode::Production);
	let upper = TestPlugin {
		hooks: vec![Hook::Transform],
		transform: Some(Box::new(|_, data| match data {
			Data::Value(Value::String(s)) => Some(text(&s.to_uppercase())),
			_ => None,
		})),
		..TestPlugin::default()
	};
	setup_with(&mut dash, vec![("reader", reader()), ("upper", upper)]);
	block_on(dash.build()).expect("built");
	assert_eq!(fs.text("out/b.txt").as_deref(), Some("BEE"));
	assert_eq!(fs.text("out/a/x.txt").as_deref(), Some("EX"));
	assert!(
		fs.text(".bridge/.dash.production.json").is_none(),
		"no cache in production"
	);
}

#[test]
fn a_file_no_read_hook_reads_is_copied_when_its_path_changes() {
	// DashFile.ts processAfterLoad: nothing read, a different output path,
	// not virtual: copy.
	let fs = MemoryFs::with(&[
		("config.json", CONFIG),
		("BP/icon.png", "\u{1}png"),
		("BP/same.txt", "s"),
	]);
	let (mut dash, _) = dash_with(fs.clone(), Mode::Production);
	let mover = TestPlugin {
		hooks: vec![Hook::TransformPath],
		transform_path: Some(Box::new(|path| {
			Ok(if path.ends_with("same.txt") {
				PathChange::Keep
			} else {
				PathChange::To(path.replacen("BP/", "out/", 1))
			})
		})),
		..TestPlugin::default()
	};
	setup_with(&mut dash, vec![("mover", mover)]);
	block_on(dash.build()).expect("built");
	assert_eq!(fs.text("out/icon.png").as_deref(), Some("\u{1}png"));
	assert_eq!(
		fs.paths(),
		["BP/icon.png", "BP/same.txt", "config.json", "out/icon.png"]
	);
	assert!(
		fs.reads.borrow().iter().all(|path| path != "BP/same.txt"),
		"no read hook, so no read"
	);
}

#[test]
fn transform_path_null_ends_the_chain_and_drops_the_output() {
	let log = Rc::new(RefCell::new(Vec::new()));
	let fs = MemoryFs::with(&[
		("config.json", CONFIG),
		("BP/x.d.ts", "declare"),
		("BP/y.ts", "code"),
	]);
	let (mut dash, _) = dash_with(fs.clone(), Mode::Production);
	let omit = TestPlugin {
		hooks: vec![Hook::TransformPath],
		transform_path: Some(Box::new(|path| {
			Ok(if path.ends_with(".d.ts") {
				PathChange::Omit
			} else {
				PathChange::Keep
			})
		})),
		log: Some(log.clone()),
		name: "omit",
		..TestPlugin::default()
	};
	let second = TestPlugin {
		log: Some(log.clone()),
		name: "second",
		..reader()
	};
	setup_with(&mut dash, vec![("omit", omit), ("second", second)]);
	block_on(dash.build()).expect("built");
	assert_eq!(
		fs.paths(),
		["BP/x.d.ts", "BP/y.ts", "config.json", "out/y.ts"]
	);
	let log = log.borrow();
	assert_eq!(
		log[..3],
		[
			"omit transformPath BP/x.d.ts",
			"omit transformPath BP/y.ts",
			"second transformPath BP/y.ts"
		]
	);
}

#[test]
fn the_first_read_that_is_not_null_wins_and_null_means_nothing_was_read() {
	let fs = MemoryFs::with(&[
		("config.json", CONFIG),
		("BP/a.txt", "a"),
		("BP/b.txt", "b"),
	]);
	let (mut dash, _) = dash_with(fs.clone(), Mode::Production);
	let nulls = TestPlugin {
		hooks: vec![Hook::Read],
		read: Some(Box::new(|path, _| {
			Ok(if path.ends_with("a.txt") {
				Some(json("null"))
			} else {
				Some(text("first"))
			})
		})),
		..TestPlugin::default()
	};
	setup_with(&mut dash, vec![("nulls", nulls), ("reader", reader())]);
	block_on(dash.build()).expect("built");
	assert_eq!(
		fs.text("out/a.txt").as_deref(),
		Some("a"),
		"null falls through to the next reader"
	);
	assert_eq!(fs.text("out/b.txt").as_deref(), Some("first"));
}

#[test]
fn load_and_transform_chains_fall_back_to_the_start_when_they_end_in_null() {
	// AllPlugins.ts runLoadHooks and LoadFiles.ts `?? file.data`.
	let fs = MemoryFs::with(&[("config.json", CONFIG), ("BP/a.txt", "start")]);
	let (mut dash, _) = dash_with(fs.clone(), Mode::Production);
	let replace_then_null = TestPlugin {
		hooks: vec![Hook::Load, Hook::Transform],
		load: Some(Box::new(|_, _| Some(text("replaced")))),
		transform: Some(Box::new(|_, data| {
			// Changes the data in place, then returns null: the in-place
			// change stays, the null does not.
			if let Data::Value(Value::String(s)) = data {
				s.push_str(" changed");
			}
			Some(json("null"))
		})),
		..TestPlugin::default()
	};
	let null_load = TestPlugin {
		hooks: vec![Hook::Load],
		load: Some(Box::new(|_, _| Some(json("null")))),
		..TestPlugin::default()
	};
	setup_with(
		&mut dash,
		vec![
			("reader", reader()),
			("a", replace_then_null),
			("b", null_load),
		],
	);
	block_on(dash.build()).expect("built");
	assert_eq!(fs.text("out/a.txt").as_deref(), Some("start changed"));
}

#[test]
fn finalize_build_takes_the_first_answer_that_is_not_undefined_and_null_omits_the_file() {
	let fs = MemoryFs::with(&[
		("config.json", CONFIG),
		("BP/a.txt", "a"),
		("BP/b.txt", "b"),
		("BP/c.txt", "c"),
	]);
	let (mut dash, _) = dash_with(fs.clone(), Mode::Production);
	let to_json = TestPlugin {
		hooks: vec![Hook::Transform, Hook::FinalizeBuild],
		transform: Some(Box::new(|path, _| {
			path.ends_with("c.txt")
				.then(|| json("{b: 1, '0': [1.0, 'x']}"))
		})),
		finalize: Some(Box::new(|path, _| {
			if path.ends_with("a.txt") {
				Finalized::Data(json("null"))
			} else if path.ends_with("b.txt") {
				Finalized::Current
			} else {
				Finalized::Undefined
			}
		})),
		..TestPlugin::default()
	};
	let last = TestPlugin {
		hooks: vec![Hook::Ignore, Hook::FinalizeBuild],
		ignore: Some(Box::new(|path| !path.ends_with("c.txt"))),
		finalize: Some(Box::new(|_, _| Finalized::Data(text("never")))),
		..TestPlugin::default()
	};
	setup_with(
		&mut dash,
		vec![("reader", reader()), ("json", to_json), ("last", last)],
	);
	block_on(dash.build()).expect("built");
	assert_eq!(fs.text("out/a.txt"), None, "null omits the file");
	assert_eq!(fs.text("out/b.txt").as_deref(), Some("b"));
	assert_eq!(
		fs.text("out/c.txt").as_deref(),
		Some("never"),
		"only c is left to the last plugin"
	);
}

#[test]
fn data_that_is_not_a_string_is_written_as_compact_json() {
	let fs = MemoryFs::with(&[("config.json", CONFIG), ("BP/a.json", "{}")]);
	let (mut dash, _) = dash_with(fs.clone(), Mode::Production);
	let parse = TestPlugin {
		hooks: vec![Hook::Load],
		load: Some(Box::new(|_, _| {
			Some(json("{b: 1.50, '1': [true, null], a: 'q\"'}"))
		})),
		..TestPlugin::default()
	};
	setup_with(&mut dash, vec![("reader", reader()), ("parse", parse)]);
	block_on(dash.build()).expect("built");
	assert_eq!(
		fs.text("out/a.json").as_deref(),
		Some(r#"{"1":[true,null],"b":1.5,"a":"q\""}"#)
	);
}

#[test]
fn a_plugin_that_ignores_a_file_is_left_out_of_its_per_file_hooks_by_id() {
	// DashFile.ts createImplementedHooksMap filters by plugin id, so a second
	// instance with the same id is left out as well.
	let log = Rc::new(RefCell::new(Vec::new()));
	let fs = MemoryFs::with(&[("config.json", CONFIG), ("BP/a.txt", "a")]);
	let (mut dash, _) = dash_with(fs.clone(), Mode::Production);
	let ignorer = |name: &'static str| TestPlugin {
		hooks: vec![Hook::Ignore, Hook::TransformPath, Hook::Transform],
		ignore: Some(Box::new(move |_| name == "first")),
		log: Some(log.clone()),
		name,
		..TestPlugin::default()
	};
	setup_with(
		&mut dash,
		vec![
			("reader", reader()),
			("same", ignorer("first")),
			("same", ignorer("second")),
		],
	);
	block_on(dash.build()).expect("built");
	assert_eq!(
		*log.borrow(),
		[
			"first transformPath out/a.txt",
			"second transformPath out/a.txt"
		]
	);
}

#[test]
fn a_hook_that_throws_is_reported_with_the_plugin_hook_and_file_and_the_build_goes_on() {
	let fs = MemoryFs::with(&[("config.json", CONFIG), ("BP/a.txt", "a")]);
	let (mut dash, recorder) = dash_with(fs.clone(), Mode::Production);
	let thrower = TestPlugin {
		hooks: vec![Hook::TransformPath, Hook::Read],
		transform_path: Some(Box::new(|_| Err("TypeError: nope".to_owned()))),
		read: Some(Box::new(|_, _| Err("Error: unreadable".to_owned()))),
		..TestPlugin::default()
	};
	setup_with(&mut dash, vec![("thrower", thrower), ("reader", reader())]);
	block_on(dash.build()).expect("built");
	assert_eq!(fs.text("out/a.txt").as_deref(), Some("a"));
	let lines = recorder.0.borrow();
	assert!(lines.contains(&"error: The plugin \"thrower\" threw an error while running the \"transformPath\" hook for \"BP/a.txt\": TypeError: nope".to_owned()), "{lines:?}");
	assert!(lines.contains(&"error: The plugin \"thrower\" threw an error while running the \"read\" hook for \"BP/a.txt\": Error: unreadable".to_owned()), "{lines:?}");
}

#[test]
fn included_virtual_files_come_first_and_included_paths_after_the_packs() {
	// IncludedFiles.ts loadAll: `[path, { isVirtual }]` entries are added on
	// the spot, plain paths join the pack files.
	let log = Rc::new(RefCell::new(Vec::new()));
	let fs = MemoryFs::with(&[
		(
			"config.json",
			r#"{"packs": {"resourcePack": "./RP", "behaviorPack": "./BP", "skinPack": "./nowhere"}, "compiler": {"plugins": []}}"#,
		),
		("BP/b.txt", "b"),
		("RP/r.txt", "r"),
		("extra/e.txt", "e"),
	]);
	let (mut dash, recorder) = dash_with(fs.clone(), Mode::Development);
	let includer = TestPlugin {
		hooks: vec![Hook::Include, Hook::TransformPath],
		include: Some(vec![
			("extra/e.txt".to_owned(), None),
			("BP/virtual.json".to_owned(), Some(true)),
			("BP/b.txt".to_owned(), None),
		]),
		log: Some(log.clone()),
		name: "p",
		..TestPlugin::default()
	};
	setup_with(&mut dash, vec![("includer", includer)]);
	block_on(dash.build()).expect("built");
	assert_eq!(
		*log.borrow(),
		[
			"p transformPath BP/virtual.json",
			"p transformPath RP/r.txt",
			"p transformPath BP/b.txt",
			"p transformPath extra/e.txt"
		]
	);
	assert!(
		recorder
			.0
			.borrow()
			.contains(&"warn: nowhere: not a directory".to_owned()),
		"{:?}",
		recorder.0.borrow()
	);
}

#[test]
fn the_cache_file_lists_every_file_with_aliases_requirements_and_update_files() {
	// Core/DashFile.ts serialize, IncludedFiles.ts save, ResolveFileOrder.ts
	// addUpdateFile; written through the input file system, tab-indented.
	let fs = MemoryFs::with(&[
		("config.json", CONFIG),
		("BP/a.txt", "a"),
		("BP/b.txt", "b"),
		("BP/c.txt", "c"),
	]);
	let (mut dash, recorder) = dash_with(fs.clone(), Mode::Development);
	let deps = TestPlugin {
		hooks: vec![Hook::RegisterAliases, Hook::Require],
		aliases: Some(Box::new(|path| {
			path.ends_with("b.txt")
				.then(|| vec![Value::String("ns:b".to_owned()), Value::Number(2.0)])
		})),
		require: Some(Box::new(|path| {
			if path.ends_with("a.txt") {
				Some(vec![
					"ns:b".to_owned(),
					"BP/c.txt".to_owned(),
					"BP/*.nothing".to_owned(),
					"BP/missing.txt".to_owned(),
				])
			} else if path.ends_with("c.txt") {
				Some(vec!["BP/a.txt".to_owned()])
			} else {
				None
			}
		})),
		..TestPlugin::default()
	};
	setup_with(&mut dash, vec![("reader", reader()), ("deps", deps)]);
	block_on(dash.build()).expect("built");
	let cache = fs
		.text(".bridge/.dash.development.json")
		.expect("the cache is written");
	let expected = parse_json5(
		r#"[
		{isVirtual: false, filePath: "BP/a.txt", aliases: [], requiredFiles: ["ns:b", "BP/c.txt", "BP/*.nothing", "BP/missing.txt"], updateFiles: ["BP/c.txt"]},
		{isVirtual: false, filePath: "BP/b.txt", aliases: ["ns:b", 2], requiredFiles: [], updateFiles: ["BP/a.txt"]},
		{isVirtual: false, filePath: "BP/c.txt", aliases: [], requiredFiles: ["BP/a.txt"], updateFiles: ["BP/a.txt"]},
	]"#,
	)
	.expect("json5");
	assert_eq!(cache, stringify(&expected, Indent::Tab));
	let errors: Vec<String> = recorder
		.0
		.borrow()
		.iter()
		.filter(|l| l.starts_with("error"))
		.cloned()
		.collect();
	assert_eq!(
		errors,
		[
			"error: Circular dependency detected: BP/a.txt is required by BP/c.txt but also depends on this file."
		]
	);
}

/// An output file system that refuses every write.
struct RefusingFs;

impl FileSystem for RefusingFs {
	fn read_file<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<u8>> {
		Box::pin(async move { Err(FsError::new(path, "refused")) })
	}
	fn write_file<'a>(&'a self, path: &'a str, _content: &'a [u8]) -> FsFuture<'a, ()> {
		Box::pin(async move { Err(FsError::new(path, "refused")) })
	}
	fn unlink<'a>(&'a self, path: &'a str) -> FsFuture<'a, ()> {
		Box::pin(async move { Err(FsError::new(path, "refused")) })
	}
	fn readdir<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<DirEntry>> {
		Box::pin(async move { Err(FsError::new(path, "refused")) })
	}
	fn mkdir<'a>(&'a self, path: &'a str) -> FsFuture<'a, ()> {
		Box::pin(async move { Err(FsError::new(path, "refused")) })
	}
	fn last_modified<'a>(&'a self, path: &'a str) -> FsFuture<'a, f64> {
		Box::pin(async move { Err(FsError::new(path, "refused")) })
	}
}

#[test]
fn failed_writes_and_copies_are_silent_as_in_ts_dash() {
	// LoadFiles.ts awaitAllFilesCopied and TransformFiles.ts run settle their
	// promises and look at none of the results.
	let fs = MemoryFs::with(&[
		("config.json", CONFIG),
		("BP/a.txt", "a"),
		("BP/b.png", "b"),
	]);
	let (dash, recorder) = dash_with(fs.clone(), Mode::Production);
	let options = DashOptions {
		config: "./config.json".to_owned(),
		compiler_config: None,
		mode: Mode::Production,
		console: recorder.clone(),
		verbose: false,
		pack_types: dash.cx.pack_types,
		file_types: dash.cx.file_types,
	};
	let mut dash = Dash::new(fs.clone(), Some(Rc::new(RefusingFs)), options);
	let copy_png = TestPlugin {
		read: Some(Box::new(|path, file| match file {
			FileHandle::File(bytes) if path.ends_with(".txt") => {
				Ok(Some(text(&String::from_utf8_lossy(bytes))))
			}
			_ => Ok(None),
		})),
		..reader()
	};
	setup_with(&mut dash, vec![("reader", copy_png)]);
	block_on(dash.build()).expect("the build succeeds");
	let lines = recorder.0.borrow();
	assert_eq!(lines.len(), 2, "{lines:?}");
	assert_eq!(lines[0], "log: Starting compilation...");
	assert!(
		lines[1].starts_with("log: Dash compiled 2 files in "),
		"{lines:?}"
	);
}
