//! The compiler: TS Dash's `Dash` class (`src/Dash.ts`) and the pipeline under
//! it (`src/Core/LoadFiles.ts`, `ResolveFileOrder.ts`, `TransformFiles.ts`).
//!
//! TS Dash runs the hooks of different files interleaved, as their reads
//! finish, and its output could depend on which read finished first. Here
//! every phase visits the files in the order they were included: first the
//! `ignore` and `transformPath` hooks of every file, then the reads, which
//! run concurrently, then `read`, `load` and `registerAliases` file by file,
//! then `require` for every file once all aliases exist.
//!
//! Plugins reach back into the compiler while their hooks run (a script's
//! `compileFiles` compiles more files from inside `buildEnd`), so the
//! compiler is shared: its state sits in cells, and no borrow of it is held
//! while a hook runs.

use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::time::Instant;

use futures_util::future::join_all;
use indexmap::{IndexMap, IndexSet};

use crate::console::{Console, Logger, Progress};
use crate::files::{AliasKey, FileId, IncludedFiles};
use crate::fs::{self, FileSystem, FsError};
use crate::js::engine::{Counted, Engine, Host, HttpsImports, ScriptTimeLimit};
use crate::js::{self, Prop};
use crate::json::{Object, Value};
use crate::pathe;
use crate::plugin::{
	BuildType, Context, Data, Dependencies, FileHandle, Finalized, Hook, Include, Mode, Options,
	Output, Plugin, Plugins, is_nullish, poll_all, yield_now,
};
use crate::plugins::{self, BuiltIn};
use crate::project::{FileTypes, PackTypes, ProjectConfig};

/// The host's `requestJsonData`: the JSON at a path such as
/// `data/packages/minecraftBedrock/location/validCommand.json`, which the
/// Deno CLI fetches from bridge-core/editor-packages and caches. `Err` is the
/// message of what the host threw.
pub type RequestJsonData = Rc<dyn Fn(&str) -> Pin<Box<dyn Future<Output = Result<Value, String>>>>>;

/// What a host sets a compiler up with: TS Dash's `IDashOptions`.
pub struct DashOptions {
	/// The project config's path, as the host found it; the Deno CLI passes
	/// `./dash-config.json` when that file exists and `./config.json`
	/// otherwise. The project root is its directory.
	pub config: String,
	/// A compiler config to take the plugin list from instead of the
	/// project config's `compiler` (the Deno CLI's `--compilerConfig`).
	pub compiler_config: Option<String>,
	/// The build mode.
	pub mode: Mode,
	/// Where messages go.
	pub console: Rc<dyn Console>,
	/// Whether to report how long each phase took.
	pub verbose: bool,
	/// The pack definitions.
	pub pack_types: PackTypes,
	/// The file definitions.
	pub file_types: FileTypes,
	/// Data the custom commands plugin asks for.
	pub request_json_data: RequestJsonData,
	/// How a script's `import` of an `https://` URL is fetched.
	pub https_imports: HttpsImports,
	/// How long a script may run without returning.
	pub script_time_limit: ScriptTimeLimit,
}

/// Why setting up or building stopped. Everything else a plugin or a file
/// can get wrong is reported on the console and the build carries on, as in
/// TS Dash.
#[derive(Debug)]
pub enum DashError {
	/// The compiler config named in [`DashOptions::compiler_config`] could
	/// not be read.
	CompilerConfig(FsError),
	/// The plugin list is a value JavaScript cannot read a plugin from, so
	/// TS Dash's `loadPlugins` throws a TypeError.
	PluginList(String),
	/// The project config's `compiler` is `null`, which TS Dash's build
	/// reads `plugins` from and throws.
	CompilerNull,
	/// The cache file could not be written.
	CacheFile(FsError),
	/// A script's data for a file could not be written: `JSON.stringify`
	/// threw (a cycle, a BigInt, a throwing `toJSON`), which stops TS Dash's
	/// build.
	Output(String, String),
}

impl fmt::Display for DashError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			DashError::CompilerConfig(error) => {
				write!(f, "cannot read the compiler config: {error}")
			}
			DashError::PluginList(message) => write!(f, "cannot read the plugin list: {message}"),
			DashError::CompilerNull => f.write_str("the project config's \"compiler\" is null"),
			DashError::CacheFile(error) => write!(f, "cannot write the cache file: {error}"),
			DashError::Output(path, message) => {
				write!(f, "cannot write the data of {path}: {message}")
			}
		}
	}
}

impl std::error::Error for DashError {}

/// A compiler for one project.
pub struct Dash {
	compiler: Rc<Compiler>,
}

/// The compiler's state, shared with the plugin context.
pub(crate) struct Compiler {
	pub(crate) config_path: String,
	pub(crate) compiler_config: RefCell<Option<String>>,
	pub(crate) plugins: RefCell<Rc<Plugins>>,
	pub(crate) files: RefCell<IncludedFiles>,
	pub(crate) progress: Progress,
	/// Copies started while loading files, which `compileIncludedFiles`
	/// waits for at its end (`awaitAllFilesCopied`): output path to source.
	copies: RefCell<IndexMap<String, String>>,
	/// Last, so that everything holding engine handles goes first.
	pub(crate) cx: Context,
}

impl Dash {
	/// A compiler reading the project from `fs` and writing its output to
	/// `output_fs`, or to `fs` when there is none. Plugins can tell the two
	/// cases apart: a separate output file system means the output goes to a
	/// `com.mojang` folder.
	///
	/// # Panics
	///
	/// When the script engine cannot be created, which happens only when
	/// memory runs out.
	pub fn new(
		fs: Rc<dyn FileSystem>,
		output_fs: Option<Rc<dyn FileSystem>>,
		options: DashOptions,
	) -> Self {
		let has_com_mojang_directory = output_fs.is_some();
		let output_fs = output_fs.unwrap_or_else(|| Rc::clone(&fs));
		let project_root = pathe::dirname(&options.config);
		let logger = Rc::new(Logger::new(options.console, options.verbose));
		let file_definitions = Value::Array(
			options
				.file_types
				.all()
				.map(|(_, definition)| definition.clone())
				.collect::<Vec<_>>()
				.into(),
		);
		let pack_definitions =
			Value::Array(options.pack_types.all().cloned().collect::<Vec<_>>().into());
		let compiler = Rc::new_cyclic(|weak: &Weak<Compiler>| {
			let engine = Engine::new(
				Host {
					file_systems: [Rc::clone(&fs), Rc::clone(&output_fs)],
					logger: Rc::clone(&logger),
					https_imports: options.https_imports,
					compiler: weak.clone(),
					mode: options.mode,
					project_root: project_root.clone(),
					separate_output: has_com_mojang_directory,
					file_definitions,
					pack_definitions,
				},
				options.script_time_limit,
			)
			.expect("the script engine starts unless memory runs out");
			let counted_fs: Rc<dyn FileSystem> =
				Rc::new(Counted::new(Rc::clone(&fs), Rc::clone(&engine.queue)));
			let counted_output: Rc<dyn FileSystem> = if has_com_mojang_directory {
				Rc::new(Counted::new(output_fs, Rc::clone(&engine.queue)))
			} else {
				Rc::clone(&counted_fs)
			};
			Compiler {
				config_path: options.config,
				compiler_config: RefCell::new(options.compiler_config),
				plugins: RefCell::default(),
				files: RefCell::default(),
				progress: Progress::new(),
				copies: RefCell::default(),
				cx: Context::new(
					counted_fs,
					counted_output,
					has_com_mojang_directory,
					logger,
					project_root,
					(options.pack_types, options.file_types),
					options.mode,
					options.request_json_data,
					engine,
				),
			}
		});
		Dash { compiler }
	}

	#[cfg(test)]
	pub(crate) fn compiler(&self) -> &Rc<Compiler> {
		&self.compiler
	}

	/// The context and the first plugin, for tests that call a built-in's
	/// hooks directly.
	#[cfg(test)]
	pub(crate) fn first_plugin(&self) -> (&Context, Rc<dyn Plugin>) {
		(&self.compiler.cx, self.compiler.plugins.borrow().first())
	}

	/// The build's progress, for a progress bar.
	pub fn progress(&self) -> &Progress {
		&self.compiler.progress
	}

	/// `setup()`: reads the project config and loads the plugins. A config
	/// that cannot be read is reported and treated as `{}`.
	pub async fn setup(&self) -> Result<(), DashError> {
		let compiler = &self.compiler;
		compiler.cx.engine.run(compiler.setup()).await
	}

	/// `build()`: a full build of every file in the project's packs and every
	/// file an `include` hook adds. The cache file is written in development
	/// mode only.
	pub async fn build(&self) -> Result<(), DashError> {
		let compiler = &self.compiler;
		let result = compiler.cx.engine.run(compiler.build()).await;
		compiler.cx.engine.drain().await;
		result
	}
}

impl Compiler {
	async fn setup(&self) -> Result<(), DashError> {
		match fs::read_json(&*self.cx.fs, &self.config_path).await {
			Ok(data) => self
				.cx
				.set_project(ProjectConfig::new(self.cx.project_root.clone(), data)),
			Err(error) => self
				.cx
				.logger
				.console()
				.error(&format!("Failed to load project config: {error}")),
		}
		self.cx.engine.reset_project(self.cx.project().data());
		self.load_plugins().await
	}

	fn cache_path(&self) -> String {
		pathe::join(&[
			&self.cx.project_root,
			&format!(".bridge/.dash.{}.json", self.cx.mode.name()),
		])
	}

	/// `isCompilerActivated`: the config has a `compiler` whose `plugins` is
	/// an array. A `compiler` of `null` makes TS Dash throw.
	pub(crate) fn is_compiler_activated(&self) -> Result<bool, DashError> {
		let project = self.cx.project();
		match js::own(project.data(), "compiler") {
			Prop::Undefined => Ok(false),
			Prop::Value(Value::Null) => Err(DashError::CompilerNull),
			Prop::Value(compiler) => Ok(matches!(
				js::get(compiler, "plugins"),
				Prop::Value(Value::Array(_))
			)),
			Prop::Inherited(_) => Ok(false),
		}
	}

	/// `loadPlugins()`: finds the plugins extensions provide, evaluates
	/// their modules, then creates each plugin the plugin list names, in list
	/// order.
	async fn load_plugins(&self) -> Result<(), DashError> {
		*self.plugins.borrow_mut() = Rc::default();
		self.cx.engine.clear_plugin_cache();
		let extension_plugins = self.extension_plugins().await;
		let compiler_config = self.compiler_config.borrow().clone();
		let list = match compiler_config.as_deref().filter(|path| !path.is_empty()) {
			Some(path) => fs::read_json(&*self.cx.fs, path)
				.await
				.map_err(DashError::CompilerConfig)?,
			None => match js::own(self.cx.project().data(), "compiler") {
				Prop::Value(Value::Null) | Prop::Undefined | Prop::Inherited(_) => {
					Value::Object(Object::new())
				}
				Prop::Value(compiler) => compiler.clone(),
			},
		};
		let used = match &list {
			Value::Null => {
				return Err(DashError::PluginList(
					"Cannot read properties of null (reading 'plugins')".to_owned(),
				));
			}
			other => match js::get(other, "plugins") {
				Prop::Value(Value::Null) | Prop::Undefined => Vec::new(),
				Prop::Value(plugins) => plugin_entries(plugins)?,
				Prop::Inherited(_) => Vec::new(),
			},
		};
		let mut entries = Vec::new();
		for entry in used {
			let (id, options) = match &entry {
				Value::String(id) => (id.clone(), None),
				Value::Null => {
					return Err(DashError::PluginList(
						"Cannot read properties of null (reading '0')".to_owned(),
					));
				}
				other => (
					js::own(other, "0").to_js_string(),
					match js::own(other, "1") {
						Prop::Value(options) => Some(options.clone()),
						_ => None,
					},
				),
			};
			entries.push((id, options));
		}

		// TS Dash's loop: an id the extensions' plugin object has (inherited
		// properties included) is an extension plugin, whose module starts
		// evaluating; otherwise a built-in, or an unknown id, reported at
		// once.
		js::plugin::set_extensions(&self.cx, &extension_plugins);
		let mut kinds = Vec::new();
		for (id, _) in &entries {
			kinds.push(if js::plugin::is_extension(&self.cx, id) {
				Kind::Extension
			} else {
				match plugins::status(id) {
					plugins::Status::Unknown => {
						self.cx
							.logger
							.console()
							.error(&format!("Unknown compiler plugin: {id}"));
						Kind::Skip
					}
					plugins::Status::NotPorted => {
						self.cx.logger.console().error(&format!(
							"The built-in plugin {id} is not ported to leafcutter-ant yet"
						));
						Kind::Skip
					}
					plugins::Status::Ported => Kind::BuiltIn,
				}
			});
		}
		// The extension modules evaluate together, as TS Dash's promises
		// run, and report their failures as they settle.
		let evaluations = entries
			.iter()
			.zip(&kinds)
			.map(|((id, _), kind)| async move {
				match kind {
					Kind::Extension => js::plugin::evaluate(&self.cx, id).await,
					_ => None,
				}
			});
		let factories = poll_all(evaluations.collect()).await;
		for (((id, options), kind), factory) in entries.into_iter().zip(kinds).zip(factories) {
			let plugin: Rc<dyn Plugin> = match kind {
				Kind::Skip => continue,
				Kind::Extension => {
					let Some(factory) = factory else { continue };
					match js::plugin::create(&self.cx, &id, factory, options.as_ref()).await {
						Some(plugin) => Rc::new(plugin),
						None => continue,
					}
				}
				Kind::BuiltIn => {
					match plugins::create(&id, &self.cx, Options::new(options.as_ref())) {
						BuiltIn::Plugin(plugin) => Rc::from(plugin),
						BuiltIn::JavaScript(name) => {
							let factory = js::plugin::built_in(&self.cx, name);
							match js::plugin::create(&self.cx, &id, factory, options.as_ref()).await
							{
								Some(plugin) => Rc::new(plugin),
								None => continue,
							}
						}
						BuiltIn::Unknown => continue,
					}
				}
			};
			let mut plugins = self.plugins.borrow_mut();
			Rc::get_mut(&mut plugins)
				.expect("nothing holds the plugin list while it is loaded")
				.add(id, plugin);
		}
		Ok(())
	}

	/// The compiler plugins that extensions in `<project>/.bridge/extensions`
	/// and `extensions` declare in their manifests, id to module path. The
	/// manifests are read in listing order and a later one wins an id; TS
	/// Dash reads them concurrently and the last read to finish wins.
	async fn extension_plugins(&self) -> IndexMap<String, String> {
		let mut extensions = Vec::new();
		let project_extensions = pathe::join(&[&self.cx.project_root, ".bridge/extensions"]);
		for root in [project_extensions.as_str(), "extensions"] {
			if let Ok(entries) = self.cx.fs.readdir(root).await {
				for entry in entries
					.into_iter()
					.filter(|e| e.kind == fs::EntryKind::Directory)
				{
					extensions.push(pathe::join(&[root, &entry.name]));
				}
			}
		}
		let mut plugins = IndexMap::new();
		for extension in extensions {
			let Ok(manifest) =
				fs::read_json(&*self.cx.fs, &pathe::join(&[&extension, "manifest.json"])).await
			else {
				continue;
			};
			let declared = match js::get(&manifest, "compiler") {
				Prop::Value(compiler) => js::get(compiler, "plugins"),
				_ => Prop::Undefined,
			};
			let Prop::Value(declared) = declared else {
				continue;
			};
			if !js::truthy(declared) {
				continue;
			}
			for (id, path) in js::for_in_entries(declared) {
				plugins.insert(id, pathe::join(&[&extension, &js::to_js_string(&path)]));
			}
		}
		plugins
	}

	async fn build(&self) -> Result<(), DashError> {
		self.cx.logger.console().log("Starting compilation...");
		if !self.is_compiler_activated()? {
			return Ok(());
		}
		self.cx.engine.clear_cache();
		self.cx.build_type.set(BuildType::FullBuild);
		self.files.borrow_mut().remove_all();
		let started = Instant::now();
		self.progress.set_total(7);
		let plugins = self.plugins();

		self.cx.logger.time("[HOOK] Build start");
		plugins.build_start(&self.cx).await;
		self.cx.logger.time_end("[HOOK] Build start");
		self.progress.advance();

		self.load_all().await;
		self.progress.advance();

		let all = self.files.borrow().all();
		self.compile(&all).await?;

		self.cx.logger.time("[HOOK] Build end");
		plugins.build_end(&self.cx).await;
		self.cx.logger.time_end("[HOOK] Build end");
		self.progress.advance();

		if self.cx.mode == Mode::Development {
			self.save_cache().await?;
		}
		self.files.borrow_mut().reset_all();
		self.progress.advance();

		self.cx.logger.console().log(&format!(
			"Dash compiled {} files in {}ms!",
			self.files.borrow().all().len(),
			started.elapsed().as_millis()
		));
		Ok(())
	}

	fn plugins(&self) -> Rc<Plugins> {
		Rc::clone(&self.plugins.borrow())
	}

	async fn save_cache(&self) -> Result<(), DashError> {
		let cache = self.files.borrow().serialize();
		fs::write_json(&*self.cx.fs, &self.cache_path(), &cache)
			.await
			.map_err(DashError::CacheFile)
	}

	/// `IncludedFiles.loadAll()`: every file under each pack root, pack by
	/// pack in config order, then what the `include` hooks add. Files an
	/// `include` hook marks virtual come first; the rest keep their order,
	/// and a path listed twice is included once.
	async fn load_all(&self) {
		self.cx.logger.time("Load all files");
		self.files.borrow_mut().clear_query_cache();
		let mut paths: IndexSet<String> = IndexSet::new();
		for pack_path in self.cx.project().available_pack_paths() {
			match fs::all_files(&*self.cx.fs, &pack_path).await {
				Ok(files) => paths.extend(files),
				Err(error) => self.cx.logger.console().warn(&error.to_string()),
			}
		}
		for include in self.plugins().include(&self.cx).await {
			match include {
				Include::Path(path) => {
					paths.insert(path);
				}
				Include::Entry(path, is_virtual) => {
					self.files.borrow_mut().add_one(path, is_virtual);
				}
			}
		}
		self.files.borrow_mut().add(paths, false);
		self.cx.logger.time_end("Load all files");
	}

	/// `compileIncludedFiles(files)`.
	async fn compile(&self, ids: &[FileId]) -> Result<(), DashError> {
		self.cx.logger.time("Loading files...");
		self.load_files(ids, true).await;
		self.cx.logger.time_end("Loading files...");
		self.progress.advance();

		self.cx.logger.time("Resolving file order...");
		let order = self.resolve_order(ids);
		self.cx.logger.time_end("Resolving file order...");
		self.progress.advance();

		self.cx.logger.time("Transforming files...");
		let result = self.transform_files(&order).await;
		self.cx.logger.time_end("Transforming files...");
		self.progress.advance();

		self.await_copies().await;
		result
	}

	/// `compileAdditionalFiles(filePaths, virtual)`, which a plugin's
	/// `compileFiles` calls: the paths become files (those already included
	/// stay as they are), are reset and compiled.
	pub(crate) async fn compile_additional(
		&self,
		paths: Vec<String>,
		is_virtual: bool,
	) -> Result<(), DashError> {
		let ids = self.files.borrow_mut().add(paths, is_virtual);
		self.progress.add_to_total(3);
		{
			let mut files = self.files.borrow_mut();
			for &id in &ids {
				files.file_mut(id).reset();
			}
		}
		self.compile(&ids).await
	}

	/// `awaitAllFilesCopied`. Copy errors vanish, as they do in TS Dash's
	/// `Promise.allSettled`. Two files copied to one path: the last in build
	/// order is the one kept.
	async fn await_copies(&self) {
		let copies = std::mem::take(&mut *self.copies.borrow_mut());
		let (from_fs, to_fs) = (&*self.cx.fs, &*self.cx.output_fs);
		join_all(
			copies
				.iter()
				.map(|(to, from)| fs::copy_file(from_fs, from, to_fs, to)),
		)
		.await;
	}

	/// The plugins of `hook` that do not ignore the file.
	fn members(&self, id: FileId, hook: Hook) -> Vec<usize> {
		self.files
			.borrow()
			.file(id)
			.plugins_for(hook)
			.unwrap_or_default()
			.to_vec()
	}

	/// `LoadFiles.run(files, writeFiles)`: every file that is not done is
	/// loaded at once, as TS Dash's promises run; then, once every alias
	/// exists, the `require` hooks of every file run, again at once.
	async fn load_files(&self, ids: &[FileId], write: bool) {
		let pending: Vec<FileId> = {
			let files = self.files.borrow();
			ids.iter()
				.copied()
				.filter(|&id| !files.file(id).is_done)
				.collect()
		};
		let plugins = self.plugins();
		let loads = pending
			.iter()
			.map(|&id| self.load_file(&plugins, id, write));
		poll_all(loads.collect()).await;
		let requires = ids.iter().map(|&id| self.require_file(&plugins, id));
		poll_all(requires.collect()).await;
	}

	/// `loadFile(file, writeFiles)`: the `ignore` hooks, then the file's
	/// read starts (`createImplementedHooksMap`), the `transformPath` hooks,
	/// the `read` hooks, and for a file something read: the `load` and
	/// `registerAliases` hooks. A file nothing read is done, and is copied
	/// when its output path differs.
	async fn load_file(&self, plugins: &Plugins, id: FileId, write: bool) {
		let cx = &self.cx;
		let path = self.files.borrow().file(id).path.clone();
		let ignored_by = plugins.ignore(cx, &path).await;
		yield_now().await;
		let reads = {
			let mut files = self.files.borrow_mut();
			let file = files.file_mut(id);
			for plugin in ignored_by {
				if !file.ignored_by.contains(&plugin) {
					file.ignored_by.push(plugin);
				}
			}
			let hooks = Hook::ALL.map(|hook| {
				plugins
					.implementing(hook)
					.iter()
					.copied()
					.filter(|&index| !file.ignored_by.iter().any(|id| id == plugins.id(index)))
					.collect::<Vec<_>>()
			});
			let reads = !hooks[Hook::Read.index()].is_empty();
			file.hooks = Some(hooks);
			reads
		};
		let output = plugins.transform_path(cx, &path).await;
		yield_now().await;
		let content = if reads {
			Some(cx.fs.read_file(&path).await.map_err(|_| ()))
		} else {
			None
		};
		let is_virtual = self.files.borrow().file(id).is_virtual;

		let cell = RefCell::new(None);
		let handle = match (&content, is_virtual) {
			(_, true) => FileHandle::None,
			(Some(Ok(bytes)), false) => FileHandle::File(bytes, &cell),
			(_, false) => FileHandle::Unreadable,
		};
		let members = self.members(id, Hook::Read);
		let data = plugins.read(cx, &members, &path, handle).await;
		drop(cell);
		yield_now().await;

		{
			let mut files = self.files.borrow_mut();
			let file = files.file_mut(id);
			file.output_path = output.clone();
			if is_nullish(&data) {
				file.is_done = true;
				if let Some(output) = output
					&& output != path
					&& !file.is_virtual
					&& write
				{
					self.copies.borrow_mut().insert(output, path);
				}
				return;
			}
		}
		let data = data.expect("present, as it is not nullish");
		let load_members = self.members(id, Hook::Load);
		let alias_members = self.members(id, Hook::RegisterAliases);
		let mut data = plugins.load(cx, &load_members, &path, data).await;
		yield_now().await;
		let aliases = plugins
			.register_aliases(cx, &alias_members, &path, &mut data)
			.await;
		let mut files = self.files.borrow_mut();
		files.file_mut(id).data = Some(data);
		files.set_aliases(id, aliases);
	}

	/// `runRequireHooks(file)` and `setRequiredFiles`, for a file whose hooks
	/// are known.
	async fn require_file(&self, plugins: &Plugins, id: FileId) {
		let (members, path, mut data) = {
			let mut files = self.files.borrow_mut();
			let file = files.file_mut(id);
			let Some(members) = file.plugins_for(Hook::Require).map(<[usize]>::to_vec) else {
				return;
			};
			(members, file.path.clone(), file.data.take())
		};
		let required = plugins
			.require(&self.cx, &members, &path, data.as_mut())
			.await;
		let mut files = self.files.borrow_mut();
		let file = files.file_mut(id);
		file.data = data;
		file.required_files = required.into_iter().collect();
	}

	/// `ResolveFileOrder.run(files)`: each file after the files it requires,
	/// depth first. Every dependency learns which files require it (their
	/// update files). A cycle is reported and broken where it closes.
	fn resolve_order(&self, ids: &[FileId]) -> Vec<FileId> {
		let mut resolved: IndexSet<FileId> = IndexSet::new();
		for &id in ids {
			if self.files.borrow().file(id).is_done || resolved.contains(&id) {
				continue;
			}
			self.resolve_single(id, &mut resolved);
		}
		resolved.into_iter().collect()
	}

	/// `resolveSingle`, with the recursion kept on a stack of its own.
	fn resolve_single(&self, root: FileId, resolved: &mut IndexSet<FileId>) {
		/// A file whose dependencies are being resolved: its required
		/// queries still to look up, and the files the current one matched.
		struct Frame {
			file: FileId,
			queries: std::vec::IntoIter<String>,
			matched: std::vec::IntoIter<FileId>,
		}
		let frame = |files: &IncludedFiles, file: FileId| Frame {
			file,
			queries: files
				.file(file)
				.required_files
				.iter()
				.cloned()
				.collect::<Vec<_>>()
				.into_iter(),
			matched: Vec::new().into_iter(),
		};
		let mut files = self.files.borrow_mut();
		let mut unresolved: IndexSet<FileId> = IndexSet::new();
		unresolved.insert(root);
		let mut stack = vec![frame(&files, root)];
		while let Some(top) = stack.last_mut() {
			let file = top.file;
			let Some(dependency) = top.matched.next() else {
				match top.queries.next() {
					Some(query) => top.matched = files.query(&self.cx.globs, &query).into_iter(),
					None => {
						resolved.insert(file);
						unresolved.shift_remove(&file);
						stack.pop();
					}
				}
				continue;
			};
			files.file_mut(dependency).update_files.insert(file);
			if resolved.contains(&dependency) {
				continue;
			}
			if unresolved.contains(&dependency) {
				self.cx.logger.console().error(&format!(
					"Circular dependency detected: {} is required by {} but also depends on this file.",
					files.file(dependency).path,
					files.file(file).path
				));
				continue;
			}
			unresolved.insert(dependency);
			stack.push(frame(&files, dependency));
		}
	}

	/// The `dependencies` a file's `transform` hooks get
	/// (`runTransformHooks`): for every file its required queries find, the
	/// file's path and each of its aliases, with its data.
	fn dependencies(&self, id: FileId) -> Dependencies {
		let mut files = self.files.borrow_mut();
		let queries: Vec<String> = files.file(id).required_files.iter().cloned().collect();
		let mut dependencies = Vec::new();
		for query in queries {
			for found in files.query(&self.cx.globs, &query) {
				let file = files.file(found);
				let data = file.data.as_ref().map(Data::share);
				dependencies.push((file.path.clone(), data.as_ref().map(Data::share)));
				for alias in &file.aliases {
					let name = match &alias.key {
						AliasKey::String(s) => s.clone(),
						_ => js::to_js_string(&alias.value),
					};
					dependencies.push((name, data.as_ref().map(Data::share)));
				}
			}
		}
		dependencies
	}

	/// `FileTransformer.run(order)`: transforms and finalizes the files one
	/// by one in dependency order, then writes the outputs concurrently.
	/// Data that is not a string is written as compact JSON.
	async fn transform_files(&self, order: &[FileId]) -> Result<(), DashError> {
		let cx = &self.cx;
		let plugins = self.plugins();
		let mut writes: IndexMap<String, Vec<u8>> = IndexMap::new();
		for &id in order {
			if self.files.borrow().file(id).is_done {
				continue;
			}
			let dependencies = self.dependencies(id);
			let transform_members = self.members(id, Hook::Transform);
			let finalize_members = self.members(id, Hook::FinalizeBuild);
			let (path, data) = {
				let mut files = self.files.borrow_mut();
				let file = files.file_mut(id);
				(file.path.clone(), file.data.take())
			};
			let Some(data) = data else {
				// A file that is not done always has data.
				continue;
			};
			let mut data = plugins
				.transform(cx, &transform_members, &path, data, &dependencies)
				.await;
			drop(dependencies);
			let finalized = plugins
				.finalize_build(cx, &finalize_members, &path, &mut data)
				.await;
			let written = match &finalized {
				Finalized::Undefined | Finalized::Current => {
					(!data.is_null()).then(|| data.output(cx))
				}
				Finalized::Data(finalized) => (!finalized.is_null()).then(|| finalized.output(cx)),
			};
			let output_path = {
				let mut files = self.files.borrow_mut();
				let file = files.file_mut(id);
				file.data = Some(data);
				file.is_done = true;
				file.output_path.clone()
			};
			let written = match written.transpose() {
				Ok(written) => written,
				Err(message) => {
					// TS Dash's build rejects here, before anything of this
					// round is written.
					return Err(DashError::Output(path, message));
				}
			};
			if let (Some(Output::Bytes(bytes)), Some(output)) = (written, output_path)
				&& output != path
			{
				// Two files written to one path: TS Dash writes both at once
				// and either may land last; here the later file in build
				// order wins.
				writes.insert(output, bytes);
			}
		}
		let out = &*cx.output_fs;
		join_all(
			writes
				.iter()
				.map(|(path, bytes)| out.write_file(path, bytes)),
		)
		.await;
		Ok(())
	}

	/// `getCompilerOutputPath(filePath)`: a known file's output path when it
	/// differs from its path (`null` becomes `undefined`), else the result of
	/// the `transformPath` hooks for the path.
	pub(crate) async fn output_path(&self, path: &str) -> Option<String> {
		if !matches!(self.is_compiler_activated(), Ok(true)) {
			return None;
		}
		let known = {
			let files = self.files.borrow();
			files.get(path).map(|id| files.file(id).output_path.clone())
		};
		if let Some(output) = known
			&& output.as_deref() != Some(path)
		{
			return output;
		}
		self.plugins()
			.transform_path(&self.cx, path)
			.await
			.filter(|output| !output.is_empty())
	}

	/// `unlinkMultiple(paths, false, true)`, which a plugin's
	/// `unlinkOutputFiles` calls: each path's output is removed; the first
	/// error is returned after every path was tried.
	pub(crate) async fn unlink_outputs(&self, paths: Vec<String>) -> Result<(), String> {
		if !matches!(self.is_compiler_activated(), Ok(true)) || paths.is_empty() {
			return Ok(());
		}
		let mut first_error = None;
		for path in paths {
			let Some(output) = self.output_path(&path).await else {
				continue;
			};
			if output == path {
				continue;
			}
			if let Err(error) = self.cx.output_fs.unlink(&output).await
				&& first_error.is_none()
			{
				first_error = Some(format!("Error: {error}"));
			}
		}
		first_error.map_or(Ok(()), Err)
	}
}

/// What a plugin list entry turned out to name.
enum Kind {
	Extension,
	BuiltIn,
	Skip,
}

/// The entries of a plugin list, as TS Dash's `for (i < usedPlugins.length)`
/// loop reads them: an array's elements, a string's characters, or an
/// object's `"0"`, `"1"` and so on up to its `length`, where a missing entry
/// makes the JavaScript throw.
fn plugin_entries(plugins: &Value) -> Result<Vec<Value>, DashError> {
	match plugins {
		Value::Array(items) => Ok(items.iter().cloned().collect()),
		Value::String(s) => Ok(s
			.encode_utf16()
			.map(|unit| Value::String(String::from_utf16_lossy(&[unit])))
			.collect()),
		Value::Object(object) => {
			let length = object.get("length").map_or(f64::NAN, js::to_number);
			let mut entries = Vec::new();
			let mut i = 0usize;
			while (i as f64) < length {
				match object.get(&i.to_string()) {
					Some(Value::Null) | None => {
						return Err(DashError::PluginList(format!(
							"Cannot read properties of {} (reading '0')",
							if object.get(&i.to_string()).is_some() {
								"null"
							} else {
								"undefined"
							}
						)));
					}
					Some(entry) => entries.push(entry.clone()),
				}
				i += 1;
			}
			Ok(entries)
		}
		_ => Ok(Vec::new()),
	}
}

#[cfg(test)]
mod tests;
