//! The compiler: TS Dash's `Dash` class (`src/Dash.ts`) and the pipeline under
//! it (`src/Core/LoadFiles.ts`, `ResolveFileOrder.ts`, `TransformFiles.ts`).
//!
//! TS Dash runs the hooks of different files interleaved, as their reads
//! finish, and its output could depend on which read finished first. Here
//! every phase visits the files in the order they were included: first the
//! `ignore` and `transformPath` hooks of every file, then the reads, which
//! run concurrently, then `read`, `load` and `registerAliases` file by file,
//! then `require` for every file once all aliases exist.

use std::fmt;
use std::rc::Rc;
use std::time::Instant;

use futures_util::future::join_all;
use indexmap::{IndexMap, IndexSet};

use crate::console::{Console, Logger, Progress};
use crate::files::{FileId, IncludedFiles};
use crate::fs::{self, FileSystem, FsError};
use crate::glob::Globs;
use crate::js::{self, Prop};
use crate::json::{Object, Value};
use crate::pathe;
use crate::plugin::{
	BuildType, Context, FileHandle, Finalized, Hook, Include, Mode, Options, Plugins, is_nullish,
};
use crate::plugins::{self, BuiltIn};
use crate::project::{FileTypes, PackTypes, ProjectConfig};

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
		}
	}
}

impl std::error::Error for DashError {}

/// A compiler for one project.
pub struct Dash {
	cx: Context,
	config_path: String,
	compiler_config: Option<String>,
	plugins: Plugins,
	files: IncludedFiles,
	progress: Progress,
}

impl Dash {
	/// A compiler reading the project from `fs` and writing its output to
	/// `output_fs`, or to `fs` when there is none. Plugins can tell the two
	/// cases apart: a separate output file system means the output goes to a
	/// `com.mojang` folder.
	pub fn new(
		fs: Rc<dyn FileSystem>,
		output_fs: Option<Rc<dyn FileSystem>>,
		options: DashOptions,
	) -> Self {
		let output_fs = output_fs.unwrap_or_else(|| Rc::clone(&fs));
		let project_root = pathe::dirname(&options.config);
		let cx = Context {
			has_com_mojang_directory: !Rc::ptr_eq(&fs, &output_fs),
			fs,
			output_fs,
			logger: Logger::new(options.console, options.verbose),
			project: ProjectConfig::new(project_root.clone(), Value::Object(Object::new())),
			project_root,
			pack_types: options.pack_types,
			file_types: options.file_types,
			globs: Globs::default(),
			mode: options.mode,
			build_type: std::cell::Cell::new(BuildType::FullBuild),
		};
		Dash {
			cx,
			config_path: options.config,
			compiler_config: options.compiler_config,
			plugins: Plugins::default(),
			files: IncludedFiles::default(),
			progress: Progress::new(),
		}
	}

	/// The build's progress, for a progress bar.
	pub fn progress(&self) -> &Progress {
		&self.progress
	}

	/// `setup()`: reads the project config and loads the plugins. A config
	/// that cannot be read is reported and treated as `{}`.
	pub async fn setup(&mut self) -> Result<(), DashError> {
		match fs::read_json(&*self.cx.fs, &self.config_path).await {
			Ok(data) => self.cx.project = ProjectConfig::new(self.cx.project_root.clone(), data),
			Err(error) => self
				.cx
				.logger
				.console()
				.error(&format!("Failed to load project config: {error}")),
		}
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
	fn is_compiler_activated(&self) -> Result<bool, DashError> {
		match js::own(self.cx.project.data(), "compiler") {
			Prop::Undefined => Ok(false),
			Prop::Value(Value::Null) => Err(DashError::CompilerNull),
			Prop::Value(compiler) => Ok(matches!(
				js::get(compiler, "plugins"),
				Prop::Value(Value::Array(_))
			)),
			Prop::Inherited(_) => Ok(false),
		}
	}

	/// `loadPlugins()`: finds the plugins extensions provide, then creates
	/// each plugin the plugin list names, in list order.
	async fn load_plugins(&mut self) -> Result<(), DashError> {
		self.plugins.clear();
		let extension_plugins = self.extension_plugins().await;
		let list = match self
			.compiler_config
			.as_deref()
			.filter(|path| !path.is_empty())
		{
			Some(path) => fs::read_json(&*self.cx.fs, path)
				.await
				.map_err(DashError::CompilerConfig)?,
			None => match js::own(self.cx.project.data(), "compiler") {
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
						Prop::Value(options) => Some(options),
						_ => None,
					},
				),
			};
			if extension_plugins.contains_key(&id) || js::Inherited::lookup(&id).is_some() {
				self.cx.logger.console().error(&format!(
					"Failed to execute plugin {id}: leafcutter-ant cannot run JavaScript plugins yet"
				));
				continue;
			}
			match plugins::create(&id, &self.cx, Options::new(options)) {
				BuiltIn::Plugin(plugin) => self.plugins.add(id, plugin),
				BuiltIn::NeedsJavaScript => self.cx.logger.console().error(&format!(
					"The built-in plugin {id} needs a JavaScript runtime, which leafcutter-ant does not have yet"
				)),
				BuiltIn::Unknown => self
					.cx
					.logger
					.console()
					.error(&format!("Unknown compiler plugin: {id}")),
			}
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

	/// `build()`: a full build of every file in the project's packs and every
	/// file an `include` hook adds. The cache file is written in development
	/// mode only.
	pub async fn build(&mut self) -> Result<(), DashError> {
		self.cx.logger.console().log("Starting compilation...");
		if !self.is_compiler_activated()? {
			return Ok(());
		}
		self.cx.build_type.set(BuildType::FullBuild);
		self.files.remove_all();
		let started = Instant::now();
		self.progress.set_total(7);

		self.cx.logger.time("[HOOK] Build start");
		self.plugins.build_start(&self.cx).await;
		self.cx.logger.time_end("[HOOK] Build start");
		self.progress.advance();

		self.load_all().await;
		self.progress.advance();

		let all = self.files.all();
		self.compile(&all).await;

		self.cx.logger.time("[HOOK] Build end");
		self.plugins.build_end(&self.cx).await;
		self.cx.logger.time_end("[HOOK] Build end");
		self.progress.advance();

		if self.cx.mode == Mode::Development {
			self.save_cache().await?;
		}
		self.files.reset_all();
		self.progress.advance();

		self.cx.logger.console().log(&format!(
			"Dash compiled {} files in {}ms!",
			self.files.all().len(),
			started.elapsed().as_millis()
		));
		Ok(())
	}

	async fn save_cache(&self) -> Result<(), DashError> {
		fs::write_json(&*self.cx.fs, &self.cache_path(), &self.files.serialize())
			.await
			.map_err(DashError::CacheFile)
	}

	/// `IncludedFiles.loadAll()`: every file under each pack root, pack by
	/// pack in config order, then what the `include` hooks add. Files an
	/// `include` hook marks virtual come first; the rest keep their order,
	/// and a path listed twice is included once.
	async fn load_all(&mut self) {
		self.cx.logger.time("Load all files");
		self.files.clear_query_cache();
		let mut paths: IndexSet<String> = IndexSet::new();
		for pack_path in self.cx.project.available_pack_paths() {
			match fs::all_files(&*self.cx.fs, &pack_path).await {
				Ok(files) => paths.extend(files),
				Err(error) => self.cx.logger.console().warn(&error.to_string()),
			}
		}
		for include in self.plugins.include(&self.cx) {
			match include {
				Include::Path(path) => {
					paths.insert(path);
				}
				Include::Entry(path, is_virtual) => {
					self.files.add_one(path, is_virtual);
				}
			}
		}
		self.files.add(paths, false);
		self.cx.logger.time_end("Load all files");
	}

	/// `compileIncludedFiles(files)`.
	async fn compile(&mut self, ids: &[FileId]) {
		self.cx.logger.time("Loading files...");
		self.load_files(ids, true).await;
		self.cx.logger.time_end("Loading files...");
		self.progress.advance();

		self.cx.logger.time("Resolving file order...");
		let order = self.resolve_order(ids);
		self.cx.logger.time_end("Resolving file order...");
		self.progress.advance();

		self.cx.logger.time("Transforming files...");
		self.transform_files(&order).await;
		self.cx.logger.time_end("Transforming files...");
		self.progress.advance();
	}

	/// `LoadFiles.run(files, writeFiles)`.
	async fn load_files(&mut self, ids: &[FileId], write: bool) {
		let Dash {
			cx, plugins, files, ..
		} = self;
		let pending: Vec<FileId> = ids
			.iter()
			.copied()
			.filter(|&id| !files.file(id).is_done)
			.collect();

		let mut outputs = Vec::with_capacity(pending.len());
		for &id in &pending {
			let path = files.file(id).path.clone();
			let ignored_by = plugins.ignore(cx, &path);
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
			file.hooks = Some(hooks);
			outputs.push(plugins.transform_path(cx, &path));
		}

		// createImplementedHooksMap starts a read for every file some read
		// hook applies to, virtual files included.
		let to_read: Vec<FileId> = pending
			.iter()
			.copied()
			.filter(|&id| {
				files
					.file(id)
					.plugins_for(Hook::Read)
					.is_some_and(|members| !members.is_empty())
			})
			.collect();
		let reads = join_all(
			to_read
				.iter()
				.map(|&id| cx.fs.read_file(&files.file(id).path)),
		)
		.await;
		for (&id, content) in to_read.iter().zip(reads) {
			files.file_mut(id).content = Some(content.map_err(|_| ()));
		}

		let mut copies: IndexMap<String, String> = IndexMap::new();
		for (&id, output) in pending.iter().zip(outputs) {
			let file = files.file(id);
			let path = file.path.clone();
			let handle = match (&file.content, file.is_virtual) {
				(_, true) => FileHandle::None,
				(Some(Ok(bytes)), false) => FileHandle::File(bytes),
				(_, false) => FileHandle::Unreadable,
			};
			let members = file.plugins_for(Hook::Read).unwrap_or_default().to_vec();
			let data = plugins.read(cx, &members, &path, handle);

			let file = files.file_mut(id);
			file.content = None;
			file.output_path = output.clone();
			if is_nullish(&data) {
				file.is_done = true;
				if let Some(output) = output
					&& output != path
					&& !file.is_virtual
					&& write
				{
					copies.insert(output, path);
				}
				continue;
			}
			let data = data.expect("present, as it is not nullish");
			let load_members = file.plugins_for(Hook::Load).unwrap_or_default().to_vec();
			let alias_members = file
				.plugins_for(Hook::RegisterAliases)
				.unwrap_or_default()
				.to_vec();
			let data = plugins.load(cx, &load_members, &path, data);
			let aliases = plugins.register_aliases(cx, &alias_members, &path, &data);
			files.file_mut(id).data = Some(data);
			files.set_aliases(id, aliases);
		}

		// Copy errors vanish, as they do in TS Dash's Promise.allSettled. Two
		// files copied to one path: the last in build order is the one kept.
		let (cx_fs, cx_out) = (&*cx.fs, &*cx.output_fs);
		join_all(
			copies
				.iter()
				.map(|(to, from)| fs::copy_file(cx_fs, from, cx_out, to)),
		)
		.await;

		for &id in ids {
			let file = files.file(id);
			let Some(members) = file.plugins_for(Hook::Require).map(<[usize]>::to_vec) else {
				continue;
			};
			let path = file.path.clone();
			let required = plugins.require(cx, &members, &path, file.data.as_ref());
			files.file_mut(id).required_files = required.into_iter().collect();
		}
	}

	/// `ResolveFileOrder.run(files)`: each file after the files it requires,
	/// depth first. Every dependency learns which files require it (their
	/// update files). A cycle is reported and broken where it closes.
	fn resolve_order(&mut self, ids: &[FileId]) -> Vec<FileId> {
		let mut resolved: IndexSet<FileId> = IndexSet::new();
		for &id in ids {
			if self.files.file(id).is_done || resolved.contains(&id) {
				continue;
			}
			self.resolve_single(id, &mut resolved);
		}
		resolved.into_iter().collect()
	}

	/// `resolveSingle`, with the recursion kept on a stack of its own.
	fn resolve_single(&mut self, root: FileId, resolved: &mut IndexSet<FileId>) {
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
		let mut unresolved: IndexSet<FileId> = IndexSet::new();
		unresolved.insert(root);
		let mut stack = vec![frame(&self.files, root)];
		while let Some(top) = stack.last_mut() {
			let file = top.file;
			let Some(dependency) = top.matched.next() else {
				match top.queries.next() {
					Some(query) => {
						top.matched = self.files.query(&self.cx.globs, &query).into_iter()
					}
					None => {
						resolved.insert(file);
						unresolved.shift_remove(&file);
						stack.pop();
					}
				}
				continue;
			};
			self.files.file_mut(dependency).update_files.insert(file);
			if resolved.contains(&dependency) {
				continue;
			}
			if unresolved.contains(&dependency) {
				self.cx.logger.console().error(&format!(
					"Circular dependency detected: {} is required by {} but also depends on this file.",
					self.files.file(dependency).path,
					self.files.file(file).path
				));
				continue;
			}
			unresolved.insert(dependency);
			stack.push(frame(&self.files, dependency));
		}
	}

	/// `FileTransformer.run(order)`: transforms and finalizes the files one
	/// by one in dependency order, then writes the outputs concurrently.
	/// Data that is not a string is written as compact JSON.
	async fn transform_files(&mut self, order: &[FileId]) {
		let Dash {
			cx, plugins, files, ..
		} = self;
		let mut writes: IndexMap<String, Vec<u8>> = IndexMap::new();
		for &id in order {
			let file = files.file_mut(id);
			if file.is_done {
				continue;
			}
			let path = file.path.clone();
			let transform_members = file
				.plugins_for(Hook::Transform)
				.unwrap_or_default()
				.to_vec();
			let finalize_members = file
				.plugins_for(Hook::FinalizeBuild)
				.unwrap_or_default()
				.to_vec();
			let Some(data) = file.data.take() else {
				// A file that is not done always has data.
				continue;
			};
			let data = plugins.transform(cx, &transform_members, &path, data);
			let written = match plugins.finalize_build(cx, &finalize_members, &path, &data) {
				Finalized::Undefined | Finalized::Current => {
					(!data.is_null()).then(|| data.to_output())
				}
				Finalized::Data(finalized) => (!finalized.is_null()).then(|| finalized.to_output()),
			};
			let file = files.file_mut(id);
			file.data = Some(data);
			file.is_done = true;
			if let (Some(bytes), Some(output)) = (written, &file.output_path)
				&& *output != path
			{
				// Two files written to one path: TS Dash writes both at once
				// and either may land last; here the later file in build
				// order wins.
				writes.insert(output.clone(), bytes);
			}
		}
		let out = &*cx.output_fs;
		join_all(
			writes
				.iter()
				.map(|(path, bytes)| out.write_file(path, bytes)),
		)
		.await;
	}
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
