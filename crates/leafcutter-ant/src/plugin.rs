//! The plugin host: the hooks a compiler plugin can implement, how Dash
//! combines their results (`src/Plugins/AllPlugins.ts`), and what happens
//! when one fails (`src/Plugins/Plugin.ts`).

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use crate::console::Logger;
use crate::fs::FileSystem;
use crate::glob::Globs;
use crate::js;
use crate::json::{Indent, Object, Value, stringify};
use crate::project::{FileTypes, PackTypes, ProjectConfig};

/// The hooks, in the order `availableHooks` lists them, which is the order a
/// plugin's hooks are registered in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hook {
	BuildStart,
	BuildEnd,
	Include,
	Ignore,
	TransformPath,
	Read,
	Load,
	RegisterAliases,
	Require,
	Transform,
	FinalizeBuild,
	BeforeFileUnlinked,
}

impl Hook {
	pub(crate) const ALL: [Hook; 12] = [
		Hook::BuildStart,
		Hook::BuildEnd,
		Hook::Include,
		Hook::Ignore,
		Hook::TransformPath,
		Hook::Read,
		Hook::Load,
		Hook::RegisterAliases,
		Hook::Require,
		Hook::Transform,
		Hook::FinalizeBuild,
		Hook::BeforeFileUnlinked,
	];

	/// The hook's name as TS Dash prints it in error messages.
	pub(crate) fn name(self) -> &'static str {
		match self {
			Hook::BuildStart => "buildStart",
			Hook::BuildEnd => "buildEnd",
			Hook::Include => "include",
			Hook::Ignore => "ignore",
			Hook::TransformPath => "transformPath",
			Hook::Read => "read",
			Hook::Load => "load",
			Hook::RegisterAliases => "registerAliases",
			Hook::Require => "require",
			Hook::Transform => "transform",
			Hook::FinalizeBuild => "finalizeBuild",
			Hook::BeforeFileUnlinked => "beforeFileUnlinked",
		}
	}

	pub(crate) fn index(self) -> usize {
		Hook::ALL
			.iter()
			.position(|hook| *hook == self)
			.expect("every hook is listed")
	}
}

/// A file's data as it passes between hooks: a JavaScript value. `undefined`
/// is not a `Data`; call sites use `Option<Data>` for it, and JSON `null` is
/// `Data::Value(Value::Null)`.
pub(crate) enum Data {
	/// A value that belongs to this file alone.
	Value(Value),
	/// A value several files hold at once, as JavaScript shares one array
	/// between them; the contents file plugin hands every file of a pack the
	/// same list and keeps adding to it.
	Shared(Rc<RefCell<Value>>),
}

impl Data {
	/// `data === null || data === undefined`, for a value that is present.
	pub(crate) fn is_null(&self) -> bool {
		match self {
			Data::Value(value) => matches!(value, Value::Null),
			Data::Shared(value) => matches!(*value.borrow(), Value::Null),
		}
	}

	/// The bytes TS Dash writes for this data: a string as it is, anything
	/// else through `JSON.stringify` (`isWritableData` and
	/// `FileTransformer.transformFile`).
	pub(crate) fn to_output(&self) -> Vec<u8> {
		match self {
			Data::Value(Value::String(s)) => s.clone().into_bytes(),
			Data::Value(value) => stringify(value, Indent::None).into_bytes(),
			Data::Shared(value) => match &*value.borrow() {
				Value::String(s) => s.clone().into_bytes(),
				value => stringify(value, Indent::None).into_bytes(),
			},
		}
	}
}

/// `undefined` or `null` counts as missing wherever TS Dash writes `x ??` or
/// `x === null || x === undefined`.
pub(crate) fn is_nullish(data: &Option<Data>) -> bool {
	data.as_ref().is_none_or(Data::is_null)
}

/// What a `read` hook gets as its second argument.
#[derive(Clone, Copy)]
pub(crate) enum FileHandle<'a> {
	/// Virtual files have no handle.
	None,
	/// `getFile()` resolves to `null`: the file could not be read.
	Unreadable,
	/// `getFile()` resolves to the file with these bytes.
	File(&'a [u8]),
}

/// What `transformPath` returns.
pub(crate) enum PathChange {
	/// `undefined`: leave the path as it is.
	Keep,
	/// `null`: the file gets no output path, and later plugins are not asked.
	Omit,
	/// A new path.
	To(String),
}

/// What `finalizeBuild` returns.
pub(crate) enum Finalized {
	/// `undefined`: ask the next plugin.
	Undefined,
	/// The file's data itself, unchanged (`return fileContent`).
	Current,
	/// New data, `null` included, which omits the file.
	Data(Data),
}

/// An entry an `include` hook returns: a path, or `[path, { isVirtual }]`.
pub(crate) enum Include {
	/// Only JavaScript plugins return plain paths; the built-ins return
	/// entries.
	#[cfg_attr(
		not(test),
		expect(
			dead_code,
			reason = "JavaScript plugins, which arrive with the embedded runtime, return plain paths"
		)
	)]
	Path(String),
	Entry(String, bool),
}

/// What a hook threw, as the message the console shows.
pub(crate) type HookError = String;

pub(crate) type HookFuture<'a> = Pin<Box<dyn Future<Output = Result<(), HookError>> + 'a>>;

/// A compiler plugin. Each method is a hook; a plugin lists the hooks it has
/// in [`Plugin::hooks`], and only those are called. The combination rule of
/// each hook is on the method that runs it in [`Plugins`].
pub(crate) trait Plugin {
	fn hooks(&self) -> &[Hook];

	fn build_start<'a>(&'a mut self, _cx: &'a Context) -> HookFuture<'a> {
		Box::pin(async { Ok(()) })
	}

	fn build_end<'a>(&'a mut self, _cx: &'a Context) -> HookFuture<'a> {
		Box::pin(async { Ok(()) })
	}

	fn include(&mut self, _cx: &Context) -> Result<Option<Vec<Include>>, HookError> {
		Ok(None)
	}

	fn ignore(&mut self, _cx: &Context, _path: &str) -> Result<bool, HookError> {
		Ok(false)
	}

	fn transform_path(&mut self, _cx: &Context, _path: &str) -> Result<PathChange, HookError> {
		Ok(PathChange::Keep)
	}

	fn read(
		&mut self,
		_cx: &Context,
		_path: &str,
		_file: FileHandle<'_>,
	) -> Result<Option<Data>, HookError> {
		Ok(None)
	}

	/// Gets the data so far, which it may change in place; returns new data
	/// to replace it, or `None` (`undefined`) to keep it.
	fn load(
		&mut self,
		_cx: &Context,
		_path: &str,
		_data: &mut Data,
	) -> Result<Option<Data>, HookError> {
		Ok(None)
	}

	fn register_aliases(
		&mut self,
		_cx: &Context,
		_path: &str,
		_data: &Data,
	) -> Result<Option<Vec<Value>>, HookError> {
		Ok(None)
	}

	fn require(
		&mut self,
		_cx: &Context,
		_path: &str,
		_data: Option<&Data>,
	) -> Result<Option<Vec<String>>, HookError> {
		Ok(None)
	}

	/// As [`Plugin::load`].
	fn transform(
		&mut self,
		_cx: &Context,
		_path: &str,
		_data: &mut Data,
	) -> Result<Option<Data>, HookError> {
		Ok(None)
	}

	fn finalize_build(
		&mut self,
		_cx: &Context,
		_path: &str,
		_data: &Data,
	) -> Result<Finalized, HookError> {
		Ok(Finalized::Undefined)
	}
}

/// Which build is running, as plugins see it in `options.buildType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildType {
	/// `build()`.
	FullBuild,
	/// `updateFiles()`.
	HotUpdate,
	/// `compileFile()`.
	FileRequest,
}

impl BuildType {
	pub(crate) fn name(self) -> &'static str {
		match self {
			BuildType::FullBuild => "fullBuild",
			BuildType::HotUpdate => "hotUpdate",
			BuildType::FileRequest => "fileRequest",
		}
	}
}

/// `development` or `production`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
	/// Builds into the development folders and keeps the incremental cache.
	Development,
	/// Builds a distributable copy.
	Production,
}

impl Mode {
	pub(crate) fn name(self) -> &'static str {
		match self {
			Mode::Development => "development",
			Mode::Production => "production",
		}
	}
}

/// What every plugin gets from Dash: TS Dash's plugin context
/// (`getPluginContext`), minus what only JavaScript plugins use.
pub(crate) struct Context {
	pub(crate) fs: Rc<dyn FileSystem>,
	pub(crate) output_fs: Rc<dyn FileSystem>,
	/// `fileSystem !== outputFileSystem`: whether the host gave a separate
	/// output file system, as the Deno CLI does for `--out`.
	pub(crate) has_com_mojang_directory: bool,
	pub(crate) logger: Logger,
	pub(crate) project: ProjectConfig,
	pub(crate) project_root: String,
	pub(crate) pack_types: PackTypes,
	pub(crate) file_types: FileTypes,
	pub(crate) globs: Globs,
	pub(crate) mode: Mode,
	pub(crate) build_type: Cell<BuildType>,
}

/// A plugin's `options`: `{ get mode(), get buildType(), ...pluginOpts }`. The
/// two getters read the current mode and build type, unless the user's
/// options have a key of the same name, which the spread puts in their place.
pub(crate) struct Options {
	user: Object,
}

/// The value an options lookup gives.
pub(crate) enum OptionValue<'a> {
	Undefined,
	Value(&'a Value),
	/// A getter's current value.
	Text(&'static str),
}

impl OptionValue<'_> {
	pub(crate) fn truthy(&self) -> bool {
		match self {
			OptionValue::Undefined => false,
			OptionValue::Value(value) => js::truthy(value),
			OptionValue::Text(text) => !text.is_empty(),
		}
	}

	/// `String(value)`.
	pub(crate) fn to_js_string(&self) -> String {
		match self {
			OptionValue::Undefined => "undefined".to_owned(),
			OptionValue::Value(value) => js::to_js_string(value),
			OptionValue::Text(text) => (*text).to_owned(),
		}
	}

	/// `value === text`.
	pub(crate) fn is(&self, text: &str) -> bool {
		match self {
			OptionValue::Undefined => false,
			OptionValue::Value(value) => matches!(value, Value::String(s) if s == text),
			OptionValue::Text(own) => *own == text,
		}
	}

	/// `value ?? fallback` is the fallback.
	pub(crate) fn is_nullish(&self) -> bool {
		matches!(
			self,
			OptionValue::Undefined | OptionValue::Value(Value::Null)
		)
	}
}

impl Options {
	/// `{ ...pluginOpts }`: an object's own keys, an array's indices or a
	/// string's characters; nothing for other values.
	pub(crate) fn new(plugin_options: Option<&Value>) -> Self {
		let mut user = Object::new();
		match plugin_options {
			Some(Value::Object(object)) => {
				for (key, value) in object {
					user.insert(key.to_owned(), value.clone());
				}
			}
			Some(Value::Array(items)) => {
				for (i, item) in items.iter().enumerate() {
					user.insert(i.to_string(), item.clone());
				}
			}
			Some(Value::String(s)) => {
				for (i, unit) in s.encode_utf16().enumerate() {
					user.insert(
						i.to_string(),
						Value::String(String::from_utf16_lossy(&[unit])),
					);
				}
			}
			_ => {}
		}
		Options { user }
	}

	pub(crate) fn get(&self, cx: &Context, key: &str) -> OptionValue<'_> {
		if let Some(value) = self.user.get(key) {
			return OptionValue::Value(value);
		}
		match key {
			"mode" => OptionValue::Text(cx.mode.name()),
			"buildType" => OptionValue::Text(cx.build_type.get().name()),
			_ => OptionValue::Undefined,
		}
	}
}

/// The loaded plugins, in config order, and for each hook the plugins that
/// implement it (`implementedHooks`).
#[derive(Default)]
pub(crate) struct Plugins {
	entries: Vec<(String, Box<dyn Plugin>)>,
	by_hook: [Vec<usize>; 12],
}

/// The error line TS Dash prints when a hook throws, without the error.
pub(crate) fn hook_error(plugin_id: &str, hook: Hook, path: Option<&str>) -> String {
	match path {
		Some(path) => format!(
			"The plugin \"{plugin_id}\" threw an error while running the \"{}\" hook for \"{path}\":",
			hook.name()
		),
		None => format!(
			"The plugin \"{plugin_id}\" threw an error while running the \"{}\" hook:",
			hook.name()
		),
	}
}

impl Plugins {
	pub(crate) fn clear(&mut self) {
		*self = Plugins::default();
	}

	pub(crate) fn add(&mut self, id: String, plugin: Box<dyn Plugin>) {
		let index = self.entries.len();
		for hook in Hook::ALL {
			if plugin.hooks().contains(&hook) {
				self.by_hook[hook.index()].push(index);
			}
		}
		self.entries.push((id, plugin));
	}

	/// The plugins implementing `hook`, as indices, in registration order.
	pub(crate) fn implementing(&self, hook: Hook) -> &[usize] {
		&self.by_hook[hook.index()]
	}

	/// The first plugin, for tests that call a built-in's hooks directly.
	#[cfg(test)]
	pub(crate) fn first(&mut self) -> &mut dyn Plugin {
		&mut *self.entries[0].1
	}

	pub(crate) fn id(&self, index: usize) -> &str {
		&self.entries[index].0
	}

	fn report(cx: &Context, plugin_id: &str, hook: Hook, path: Option<&str>, error: &str) {
		cx.logger
			.console()
			.error(&format!("{} {error}", hook_error(plugin_id, hook, path)));
	}

	/// `runBuildStartHooks`: every plugin at once (`Promise.all`); a plugin
	/// that throws is reported and the others carry on.
	pub(crate) async fn build_start(&mut self, cx: &Context) {
		self.run_all(cx, Hook::BuildStart).await;
	}

	/// `runBuildEndHooks`: as `buildStart`.
	pub(crate) async fn build_end(&mut self, cx: &Context) {
		self.run_all(cx, Hook::BuildEnd).await;
	}

	async fn run_all(&mut self, cx: &Context, hook: Hook) {
		let members = self.by_hook[hook.index()].clone();
		let mut futures = Vec::new();
		for (index, (id, plugin)) in self.entries.iter_mut().enumerate() {
			if !members.contains(&index) {
				continue;
			}
			let id = id.as_str();
			let future = match hook {
				Hook::BuildStart => plugin.build_start(cx),
				_ => plugin.build_end(cx),
			};
			futures.push(async move { (id, future.await) });
		}
		for (id, result) in futures_util::future::join_all(futures).await {
			if let Err(error) = result {
				Self::report(cx, id, hook, None, &error);
			}
		}
	}

	/// `runIncludeHooks`: each plugin in turn; the arrays they return are
	/// concatenated, and anything that is not an array is skipped.
	pub(crate) fn include(&mut self, cx: &Context) -> Vec<Include> {
		let mut included = Vec::new();
		for index in self.by_hook[Hook::Include.index()].clone() {
			let (id, plugin) = &mut self.entries[index];
			match plugin.include(cx) {
				Ok(Some(entries)) => included.extend(entries),
				Ok(None) => {}
				Err(error) => Self::report(cx, id, Hook::Include, None, &error),
			}
		}
		included
	}

	/// `runIgnoreHooks`: every plugin is asked; the ids of those that say
	/// yes are returned.
	pub(crate) fn ignore(&mut self, cx: &Context, path: &str) -> Vec<String> {
		let mut ignored_by = Vec::new();
		for index in self.by_hook[Hook::Ignore.index()].clone() {
			let (id, plugin) = &mut self.entries[index];
			match plugin.ignore(cx, path) {
				Ok(true) => ignored_by.push(id.clone()),
				Ok(false) => {}
				Err(error) => Self::report(cx, id, Hook::Ignore, Some(path), &error),
			}
		}
		ignored_by
	}

	/// `runTransformPathHooks`: chained over every plugin (none is skipped
	/// for ignoring the file); `null` ends the chain with no output path.
	pub(crate) fn transform_path(&mut self, cx: &Context, path: &str) -> Option<String> {
		let mut current = path.to_owned();
		for index in self.by_hook[Hook::TransformPath.index()].clone() {
			let (id, plugin) = &mut self.entries[index];
			match plugin.transform_path(cx, &current) {
				Ok(PathChange::Keep) => {}
				Ok(PathChange::Omit) => return None,
				Ok(PathChange::To(new_path)) => current = new_path,
				Err(error) => Self::report(cx, id, Hook::TransformPath, Some(&current), &error),
			}
		}
		Some(current)
	}

	/// `runReadHooks`: the first result that is neither `null` nor
	/// `undefined` wins.
	pub(crate) fn read(
		&mut self,
		cx: &Context,
		members: &[usize],
		path: &str,
		file: FileHandle<'_>,
	) -> Option<Data> {
		for &index in members {
			let (id, plugin) = &mut self.entries[index];
			match plugin.read(cx, path, file) {
				Ok(data) if !is_nullish(&data) => return data,
				Ok(_) => {}
				Err(error) => Self::report(cx, id, Hook::Read, Some(path), &error),
			}
		}
		None
	}

	/// `runLoadHooks` followed by `setReadData(result ?? file.data)`: each
	/// plugin gets the result of the one before, `undefined` keeps it, and a
	/// chain that ends in `null` falls back to the data it started from.
	pub(crate) fn load(&mut self, cx: &Context, members: &[usize], path: &str, data: Data) -> Data {
		self.chain(cx, members, path, data, Hook::Load)
	}

	/// `runTransformHooks`, then `file.data = result ?? file.data`: as
	/// `load`.
	pub(crate) fn transform(
		&mut self,
		cx: &Context,
		members: &[usize],
		path: &str,
		data: Data,
	) -> Data {
		self.chain(cx, members, path, data, Hook::Transform)
	}

	fn chain(
		&mut self,
		cx: &Context,
		members: &[usize],
		path: &str,
		mut original: Data,
		hook: Hook,
	) -> Data {
		// `None` while the chain still holds the data it started with, which
		// the plugins may change in place.
		let mut replaced: Option<Data> = None;
		for &index in members {
			let (id, plugin) = &mut self.entries[index];
			let current = replaced.as_mut().unwrap_or(&mut original);
			let result = match hook {
				Hook::Load => plugin.load(cx, path, current),
				_ => plugin.transform(cx, path, current),
			};
			match result {
				Ok(Some(data)) => replaced = Some(data),
				Ok(None) => {}
				Err(error) => Self::report(cx, id, hook, Some(path), &error),
			}
		}
		match replaced {
			Some(data) if !data.is_null() => data,
			_ => original,
		}
	}

	/// `runRegisterAliasesHooks`: the union of every result, an array
	/// contributing each element and anything else itself; `null` and
	/// `undefined` contribute nothing.
	pub(crate) fn register_aliases(
		&mut self,
		cx: &Context,
		members: &[usize],
		path: &str,
		data: &Data,
	) -> Vec<Value> {
		let mut aliases = Vec::new();
		for &index in members {
			let (id, plugin) = &mut self.entries[index];
			match plugin.register_aliases(cx, path, data) {
				Ok(Some(values)) => aliases.extend(values),
				Ok(None) => {}
				Err(error) => Self::report(cx, id, Hook::RegisterAliases, Some(path), &error),
			}
		}
		aliases
	}

	/// `runRequireHooks`: the union of every result, as `registerAliases`.
	pub(crate) fn require(
		&mut self,
		cx: &Context,
		members: &[usize],
		path: &str,
		data: Option<&Data>,
	) -> Vec<String> {
		let mut required = Vec::new();
		for &index in members {
			let (id, plugin) = &mut self.entries[index];
			match plugin.require(cx, path, data) {
				Ok(Some(paths)) => {
					for path in paths {
						if !required.contains(&path) {
							required.push(path);
						}
					}
				}
				Ok(None) => {}
				Err(error) => Self::report(cx, id, Hook::Require, Some(path), &error),
			}
		}
		required
	}

	/// `runFinalizeBuildHooks`: the first result that is not `undefined`
	/// wins, `null` included.
	pub(crate) fn finalize_build(
		&mut self,
		cx: &Context,
		members: &[usize],
		path: &str,
		data: &Data,
	) -> Finalized {
		for &index in members {
			let (id, plugin) = &mut self.entries[index];
			match plugin.finalize_build(cx, path, data) {
				Ok(Finalized::Undefined) => {}
				Ok(finalized) => return finalized,
				Err(error) => Self::report(cx, id, Hook::FinalizeBuild, Some(path), &error),
			}
		}
		Finalized::Undefined
	}
}
