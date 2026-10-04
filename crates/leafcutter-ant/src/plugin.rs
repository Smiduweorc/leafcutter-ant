//! The plugin host: the hooks a compiler plugin can implement, how Dash
//! combines their results (`src/Plugins/AllPlugins.ts`), and what happens
//! when one fails (`src/Plugins/Plugin.ts`).

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use crate::console::Logger;
use crate::dash::RequestJsonData;
use crate::fs::FileSystem;
use crate::glob::Globs;
use crate::js;
use crate::js::engine::{Engine, Handle};
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
	/// An object, function, symbol or BigInt a script made, which stays in
	/// the engine so that every script that sees it gets the same one.
	Js(Handle),
}

impl Data {
	/// `data === null || data === undefined`, for a value that is present.
	pub(crate) fn is_null(&self) -> bool {
		match self {
			Data::Value(value) => matches!(value, Value::Null),
			Data::Shared(value) => matches!(*value.borrow(), Value::Null),
			Data::Js(_) => false,
		}
	}

	/// The bytes TS Dash writes for this data (`FileTransformer.transformFile`):
	/// a string as it is, anything else through `JSON.stringify`, and for a
	/// script's value what `isWritableData` and the file system make of it.
	/// `Err` is the TypeError `JSON.stringify` throws, which stops TS Dash's
	/// build.
	pub(crate) fn output(&self, cx: &Context) -> Result<Output, String> {
		Ok(match self {
			Data::Value(Value::String(s)) => Output::Bytes(s.clone().into_bytes()),
			Data::Value(value) => Output::Bytes(stringify(value, Indent::None).into_bytes()),
			Data::Shared(value) => match &*value.borrow() {
				Value::String(s) => Output::Bytes(s.clone().into_bytes()),
				value => Output::Bytes(stringify(value, Indent::None).into_bytes()),
			},
			Data::Js(handle) => cx.engine.output(handle)?,
		})
	}

	/// The data as JSON, the way `JSON.stringify` sees a script's value;
	/// `None` for a value it writes as `undefined`.
	pub(crate) fn json(&self, cx: &Context) -> Result<Option<Cow<'_, Value>>, String> {
		Ok(match self {
			Data::Value(value) => Some(Cow::Borrowed(value)),
			Data::Shared(value) => Some(Cow::Owned(value.borrow().clone())),
			Data::Js(handle) => cx.engine.to_value(handle)?.map(Cow::Owned),
		})
	}

	/// The data as JSON to change in place. A script's value is replaced by
	/// its JSON first, so a script that kept the object no longer sees the
	/// changes (a recorded difference from TS Dash).
	pub(crate) fn json_mut(&mut self, cx: &Context) -> Result<Option<&mut Value>, String> {
		if let Data::Js(handle) = self {
			match cx.engine.to_value(handle)? {
				Some(value) => *self = Data::Value(value),
				None => return Ok(None),
			}
		}
		Ok(match self {
			Data::Value(value) => Some(value),
			Data::Shared(value) => {
				let owned = value.borrow().clone();
				*self = Data::Value(owned);
				match self {
					Data::Value(value) => Some(value),
					_ => None,
				}
			}
			Data::Js(_) => None,
		})
	}

	/// A second reference to the data, for `dependencies`: a script's object
	/// and a shared value stay one value, JSON is copied.
	pub(crate) fn share(&self) -> Data {
		match self {
			Data::Value(value) => Data::Value(value.clone()),
			Data::Shared(value) => Data::Shared(Rc::clone(value)),
			Data::Js(handle) => Data::Js(handle.clone()),
		}
	}
}

/// What writing a file's data comes to.
pub(crate) enum Output {
	/// These bytes are written.
	Bytes(Vec<u8>),
	/// Nothing is written: `JSON.stringify` gave `undefined`, or the data is
	/// something the file system refuses (a Blob, an ArrayBuffer), which TS
	/// Dash's write fails on without a message.
	Nothing,
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
	/// `getFile()` resolves to the file with these bytes. A script gets the
	/// handle object kept in the cell, one per file, so every `getFile()`
	/// returns the same promise, as TS Dash's cached read does.
	File(&'a [u8], &'a RefCell<Option<Handle>>),
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
	Path(String),
	Entry(String, bool),
}

/// What a hook threw, as the message the console shows.
pub(crate) type HookError = String;

/// What a hook returns: every hook may wait, as a JavaScript plugin's hooks
/// may return promises.
pub(crate) type HookFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, HookError>> + 'a>>;

/// A hook result that is ready at once, for the hooks of the built-ins that
/// do not wait.
pub(crate) fn ready<'a, T: 'a>(result: Result<T, HookError>) -> HookFuture<'a, T> {
	Box::pin(std::future::ready(result))
}

/// The files a file requires, with their data, by path and by alias, which
/// the `transform` hook gets as `dependencies`. Later entries win a name, as
/// in `Object.fromEntries`; `None` is a file without data (`undefined`).
pub(crate) type Dependencies = Vec<(String, Option<Data>)>;

/// A compiler plugin. Each method is a hook; a plugin lists the hooks it has
/// in [`Plugin::hooks`], and only those are called. The combination rule of
/// each hook is on the method that runs it in [`Plugins`].
///
/// Hooks take `&self`: a hook may run while another hook of the same plugin
/// is waiting (a plugin that compiles more files from `buildEnd` sees its own
/// hooks run for them), so a plugin keeps its state in cells and never holds
/// a borrow across an `await`.
///
/// Hooks that get the file's data get it mutably: a Rust plugin may change it
/// in place, and a JavaScript plugin replaces it with the object it was
/// handed, so that the next plugin sees the same object, as in TS Dash.
pub(crate) trait Plugin {
	fn hooks(&self) -> &[Hook];

	fn build_start<'a>(&'a self, _cx: &'a Context) -> HookFuture<'a, ()> {
		ready(Ok(()))
	}

	fn build_end<'a>(&'a self, _cx: &'a Context) -> HookFuture<'a, ()> {
		ready(Ok(()))
	}

	fn include<'a>(&'a self, _cx: &'a Context) -> HookFuture<'a, Option<Vec<Include>>> {
		ready(Ok(None))
	}

	fn ignore<'a>(&'a self, _cx: &'a Context, _path: &'a str) -> HookFuture<'a, bool> {
		ready(Ok(false))
	}

	fn transform_path<'a>(
		&'a self,
		_cx: &'a Context,
		_path: &'a str,
	) -> HookFuture<'a, PathChange> {
		ready(Ok(PathChange::Keep))
	}

	fn read<'a>(
		&'a self,
		_cx: &'a Context,
		_path: &'a str,
		_file: FileHandle<'a>,
	) -> HookFuture<'a, Option<Data>> {
		ready(Ok(None))
	}

	/// Gets the data so far, which it may change in place; returns new data
	/// to replace it, or `None` (`undefined`) to keep it.
	fn load<'a>(
		&'a self,
		_cx: &'a Context,
		_path: &'a str,
		_data: &'a mut Data,
	) -> HookFuture<'a, Option<Data>> {
		ready(Ok(None))
	}

	fn register_aliases<'a>(
		&'a self,
		_cx: &'a Context,
		_path: &'a str,
		_data: &'a mut Data,
	) -> HookFuture<'a, Option<Vec<Value>>> {
		ready(Ok(None))
	}

	fn require<'a>(
		&'a self,
		_cx: &'a Context,
		_path: &'a str,
		_data: Option<&'a mut Data>,
	) -> HookFuture<'a, Option<Vec<String>>> {
		ready(Ok(None))
	}

	/// As [`Plugin::load`].
	fn transform<'a>(
		&'a self,
		_cx: &'a Context,
		_path: &'a str,
		_data: &'a mut Data,
		_dependencies: &'a Dependencies,
	) -> HookFuture<'a, Option<Data>> {
		ready(Ok(None))
	}

	fn finalize_build<'a>(
		&'a self,
		_cx: &'a Context,
		_path: &'a str,
		_data: &'a mut Data,
	) -> HookFuture<'a, Finalized> {
		ready(Ok(Finalized::Undefined))
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
/// (`getPluginContext`). JavaScript plugins get it as an object built from
/// this (`js::plugin`).
pub(crate) struct Context {
	/// The input file system, counted by the engine (`engine::Counted`).
	pub(crate) fs: Rc<dyn FileSystem>,
	/// The output file system; the same `Rc` as `fs` when the host gave none.
	pub(crate) output_fs: Rc<dyn FileSystem>,
	/// `fileSystem !== outputFileSystem`: whether the host gave a separate
	/// output file system, as the Deno CLI does for `--out`.
	pub(crate) has_com_mojang_directory: bool,
	pub(crate) logger: Rc<Logger>,
	project: RefCell<Rc<ProjectConfig>>,
	pub(crate) project_root: String,
	pub(crate) pack_types: PackTypes,
	pub(crate) file_types: FileTypes,
	pub(crate) globs: Globs,
	pub(crate) mode: Mode,
	pub(crate) build_type: Cell<BuildType>,
	pub(crate) request_json_data: RequestJsonData,
	/// Last, so that it is dropped after everything that holds a handle
	/// into it.
	pub(crate) engine: Engine,
}

impl Context {
	#[expect(
		clippy::too_many_arguments,
		reason = "one argument per part of the context, called once"
	)]
	pub(crate) fn new(
		fs: Rc<dyn FileSystem>,
		output_fs: Rc<dyn FileSystem>,
		has_com_mojang_directory: bool,
		logger: Rc<Logger>,
		project_root: String,
		(pack_types, file_types): (PackTypes, FileTypes),
		mode: Mode,
		request_json_data: RequestJsonData,
		engine: Engine,
	) -> Self {
		Context {
			fs,
			output_fs,
			has_com_mojang_directory,
			logger,
			project: RefCell::new(Rc::new(ProjectConfig::new(
				project_root.clone(),
				Value::Object(Object::new()),
			))),
			project_root,
			pack_types,
			file_types,
			globs: Globs::default(),
			mode,
			build_type: Cell::new(BuildType::FullBuild),
			request_json_data,
			engine,
		}
	}

	/// The project config as the last `setup` read it.
	pub(crate) fn project(&self) -> Rc<ProjectConfig> {
		Rc::clone(&self.project.borrow())
	}

	pub(crate) fn set_project(&self, project: ProjectConfig) {
		*self.project.borrow_mut() = Rc::new(project);
	}
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
	entries: Vec<(String, Rc<dyn Plugin>)>,
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
	pub(crate) fn add(&mut self, id: String, plugin: Rc<dyn Plugin>) {
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
	pub(crate) fn first(&self) -> Rc<dyn Plugin> {
		Rc::clone(&self.entries[0].1)
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
	pub(crate) async fn build_start(&self, cx: &Context) {
		self.run_all(cx, Hook::BuildStart).await;
	}

	/// `runBuildEndHooks`: as `buildStart`.
	pub(crate) async fn build_end(&self, cx: &Context) {
		self.run_all(cx, Hook::BuildEnd).await;
	}

	async fn run_all(&self, cx: &Context, hook: Hook) {
		let futures = self.by_hook[hook.index()].iter().map(|&index| {
			let (id, plugin) = &self.entries[index];
			let future = match hook {
				Hook::BuildStart => plugin.build_start(cx),
				_ => plugin.build_end(cx),
			};
			async move { (id, future.await) }
		});
		for (id, result) in poll_all(futures.collect()).await {
			if let Err(error) = result {
				Self::report(cx, id, hook, None, &error);
			}
		}
	}

	/// `runIncludeHooks`: each plugin in turn; the arrays they return are
	/// concatenated, and anything that is not an array is skipped.
	pub(crate) async fn include(&self, cx: &Context) -> Vec<Include> {
		let mut included = Vec::new();
		for &index in &self.by_hook[Hook::Include.index()] {
			let (id, plugin) = &self.entries[index];
			match plugin.include(cx).await {
				Ok(Some(entries)) => included.extend(entries),
				Ok(None) => {}
				Err(error) => Self::report(cx, id, Hook::Include, None, &error),
			}
		}
		included
	}

	/// `runIgnoreHooks`: every plugin is asked; the ids of those that say
	/// yes are returned.
	pub(crate) async fn ignore(&self, cx: &Context, path: &str) -> Vec<String> {
		let mut ignored_by = Vec::new();
		for &index in &self.by_hook[Hook::Ignore.index()] {
			let (id, plugin) = &self.entries[index];
			match plugin.ignore(cx, path).await {
				Ok(true) => ignored_by.push(id.clone()),
				Ok(false) => {}
				Err(error) => Self::report(cx, id, Hook::Ignore, Some(path), &error),
			}
		}
		ignored_by
	}

	/// `runTransformPathHooks`: chained over every plugin (none is skipped
	/// for ignoring the file); `null` ends the chain with no output path.
	pub(crate) async fn transform_path(&self, cx: &Context, path: &str) -> Option<String> {
		let mut current = path.to_owned();
		for &index in &self.by_hook[Hook::TransformPath.index()] {
			let (id, plugin) = &self.entries[index];
			match plugin.transform_path(cx, &current).await {
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
	pub(crate) async fn read(
		&self,
		cx: &Context,
		members: &[usize],
		path: &str,
		file: FileHandle<'_>,
	) -> Option<Data> {
		for &index in members {
			let (id, plugin) = &self.entries[index];
			match plugin.read(cx, path, file).await {
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
	pub(crate) async fn load(
		&self,
		cx: &Context,
		members: &[usize],
		path: &str,
		data: Data,
	) -> Data {
		self.chain(cx, members, path, data, None).await
	}

	/// `runTransformHooks`, then `file.data = result ?? file.data`: as
	/// `load`, with the file's dependencies.
	pub(crate) async fn transform(
		&self,
		cx: &Context,
		members: &[usize],
		path: &str,
		data: Data,
		dependencies: &Dependencies,
	) -> Data {
		self.chain(cx, members, path, data, Some(dependencies))
			.await
	}

	async fn chain(
		&self,
		cx: &Context,
		members: &[usize],
		path: &str,
		mut original: Data,
		dependencies: Option<&Dependencies>,
	) -> Data {
		// `None` while the chain still holds the data it started with, which
		// the plugins may change in place.
		let mut replaced: Option<Data> = None;
		for &index in members {
			let (id, plugin) = &self.entries[index];
			let current = replaced.as_mut().unwrap_or(&mut original);
			let (hook, result) = match dependencies {
				None => (Hook::Load, plugin.load(cx, path, current).await),
				Some(dependencies) => (
					Hook::Transform,
					plugin.transform(cx, path, current, dependencies).await,
				),
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
	pub(crate) async fn register_aliases(
		&self,
		cx: &Context,
		members: &[usize],
		path: &str,
		data: &mut Data,
	) -> Vec<Value> {
		let mut aliases = Vec::new();
		for &index in members {
			let (id, plugin) = &self.entries[index];
			match plugin.register_aliases(cx, path, data).await {
				Ok(Some(values)) => aliases.extend(values),
				Ok(None) => {}
				Err(error) => Self::report(cx, id, Hook::RegisterAliases, Some(path), &error),
			}
		}
		aliases
	}

	/// `runRequireHooks`: the union of every result, as `registerAliases`.
	pub(crate) async fn require(
		&self,
		cx: &Context,
		members: &[usize],
		path: &str,
		mut data: Option<&mut Data>,
	) -> Vec<String> {
		let mut required = Vec::new();
		for &index in members {
			let (id, plugin) = &self.entries[index];
			match plugin.require(cx, path, data.as_deref_mut()).await {
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
	pub(crate) async fn finalize_build(
		&self,
		cx: &Context,
		members: &[usize],
		path: &str,
		data: &mut Data,
	) -> Finalized {
		for &index in members {
			let (id, plugin) = &self.entries[index];
			match plugin.finalize_build(cx, path, data).await {
				Ok(Finalized::Undefined) => {}
				Ok(finalized) => return finalized,
				Err(error) => Self::report(cx, id, Hook::FinalizeBuild, Some(path), &error),
			}
		}
		Finalized::Undefined
	}
}

/// Gives the other futures the executor polls a turn, as an `await` does in
/// JavaScript: the files TS Dash loads at once step through their hooks
/// side by side, one await at a time.
pub(crate) async fn yield_now() {
	let mut yielded = false;
	std::future::poll_fn(|cx| {
		if yielded {
			std::task::Poll::Ready(())
		} else {
			yielded = true;
			cx.waker().wake_by_ref();
			std::task::Poll::Pending
		}
	})
	.await;
}

/// Runs the futures together and returns their outputs in order. Every
/// unfinished future is polled on every wake: a future waiting on the engine
/// can be freed by work another one does, which no waker announces.
pub(crate) async fn poll_all<F: Future>(futures: Vec<F>) -> Vec<F::Output> {
	let mut futures: Vec<Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
	let mut outputs: Vec<Option<F::Output>> = futures.iter().map(|_| None).collect();
	std::future::poll_fn(|cx| {
		let mut pending = false;
		for (future, output) in futures.iter_mut().zip(outputs.iter_mut()) {
			if output.is_some() {
				continue;
			}
			match future.as_mut().poll(cx) {
				std::task::Poll::Ready(value) => *output = Some(value),
				std::task::Poll::Pending => pending = true,
			}
		}
		if pending {
			std::task::Poll::Pending
		} else {
			std::task::Poll::Ready(())
		}
	})
	.await;
	outputs.into_iter().flatten().collect()
}
