//! Plugins written in JavaScript: extension plugins, and the built-ins that
//! run user scripts (`generatorScripts`, `customCommands`), which run as
//! the JavaScript TS Dash ships (`layer/dash.js`). Each is a [`JsPlugin`]
//! whose hooks call into the engine; the plugin context they get is built in
//! the layer on the functions [`compiler_functions`] gives it.
//!
//! A hook gets a file's data as the engine's value. JSON data is handed over
//! as a new object, which then becomes the file's data, so that the next
//! hook sees the object the script changed, as in TS Dash.

use std::rc::Rc;

use indexmap::IndexMap;
use rquickjs::{Array, Ctx, Exception, Function, Object, Persistent, TypedArray, Value as JsValue};

use crate::dash::Compiler;
use crate::files::FileId;
use crate::json::Value;
use crate::pathe;
use crate::plugin::{
	Context, Data, Dependencies, FileHandle, Finalized, Hook, HookFuture, Include, PathChange,
	Plugin,
};
use crate::project::FileTypeError;

use super::bridge;
use super::engine::{Engine, Handle, Host, Queue, Settle, caught};

/// A plugin object a factory returned, and the hooks it had then.
pub(crate) struct JsPlugin {
	object: Handle,
	hooks: Vec<Hook>,
}

/// Hands the extensions' plugins to the layer: id to module path, in
/// manifest order.
pub(crate) fn set_extensions(cx: &Context, plugins: &IndexMap<String, String>) {
	cx.engine.with(|ctx, layer| {
		let result = (|| -> rquickjs::Result<()> {
			let entries = Array::new(ctx.clone())?;
			for (i, (id, path)) in plugins.iter().enumerate() {
				let entry = Array::new(ctx.clone())?;
				entry.set(0, id.as_str())?;
				entry.set(1, path.as_str())?;
				entries.set(i, entry)?;
			}
			let dash: Object = layer.get("dash")?;
			let set: Function = dash.get("setExtensions")?;
			set.call::<_, ()>((entries,))
		})();
		// Fails only when memory runs out.
		drop(result);
	});
}

/// `if (plugins[pluginId])`: whether the id names an extension plugin.
/// Properties every object inherits count, as they do in TS Dash.
pub(crate) fn is_extension(cx: &Context, id: &str) -> bool {
	cx.engine.with(|ctx, layer| {
		Engine::call_dash(&ctx, &layer, "isExtension", (id,))
			.ok()
			.and_then(|value| value.as_bool())
			.unwrap_or(false)
	})
}

/// Evaluates an extension plugin's module; its default export when that is
/// a function. Failures are reported on the console, as TS Dash reports
/// them.
pub(crate) async fn evaluate(cx: &Context, id: &str) -> Option<Handle> {
	let promise = cx.engine.with(|ctx, layer| {
		Engine::call_dash(&ctx, &layer, "evaluatePlugin", (id,))
			.map(|value| Persistent::save(&ctx, value))
	});
	let factory = match promise {
		Ok(promise) => cx.engine.settle(promise).await,
		Err(error) => Err(error),
	};
	match factory {
		Ok(factory) => cx.engine.with(|ctx, _| {
			let value = factory.clone().restore(&ctx).ok()?;
			value.is_function().then_some(factory)
		}),
		Err(error) => {
			cx.logger
				.console()
				.error(&format!("Failed to execute plugin {id}: {error}"));
			None
		}
	}
}

/// The factory of a built-in plugin that runs as JavaScript.
pub(crate) fn built_in(cx: &Context, name: &str) -> Handle {
	cx.engine.with(|ctx, layer| {
		let factory = layer
			.get::<_, Object>("dash")
			.and_then(|dash| dash.get::<_, Object>("builtIn"))
			.and_then(|built_in| built_in.get::<_, JsValue>(name))
			.expect("the layer defines every JavaScript built-in");
		Persistent::save(&ctx, factory)
	})
}

/// `addPlugin`: calls the factory with the plugin context and waits for the
/// plugin it returns. A factory that throws, or returns something that is
/// not an object, adds no plugin; TS Dash leaves that rejection unhandled,
/// here it is reported.
pub(crate) async fn create(
	cx: &Context,
	id: &str,
	factory: Handle,
	options: Option<&Value>,
) -> Option<JsPlugin> {
	let promise = cx.engine.with(|ctx, layer| {
		let factory = factory.restore(&ctx).map_err(|e| caught(&ctx, e))?;
		let options = match options {
			Some(options) => bridge::to_js(&ctx, options).map_err(|e| caught(&ctx, e))?,
			None => JsValue::new_undefined(ctx.clone()),
		};
		Engine::call_dash(&ctx, &layer, "createPlugin", (id, factory, options))
			.map(|value| Persistent::save(&ctx, value))
	});
	let created = match promise {
		Ok(promise) => cx.engine.settle(promise).await,
		Err(error) => Err(error),
	};
	let created = created.and_then(|created| {
		cx.engine.with(|ctx, _| {
			let pair: Array = created
				.restore(&ctx)
				.and_then(|value| value.get())
				.map_err(|e| caught(&ctx, e))?;
			let object: JsValue = pair.get(0).map_err(|e| caught(&ctx, e))?;
			let hooks: Vec<String> = pair.get(1).map_err(|e| caught(&ctx, e))?;
			let hooks = Hook::ALL
				.into_iter()
				.filter(|hook| hooks.iter().any(|name| name == hook.name()))
				.collect();
			Ok(JsPlugin {
				object: Persistent::save(&ctx, object),
				hooks,
			})
		})
	});
	match created {
		Ok(plugin) => Some(plugin),
		Err(error) => {
			cx.logger
				.console()
				.error(&format!("Failed to create plugin {id}: {error}"));
			None
		}
	}
}

/// File data as an argument: JSON objects and arrays become the engine's
/// objects, which then are the file's data.
fn data_arg<'js>(ctx: &Ctx<'js>, data: &mut Data) -> rquickjs::Result<JsValue<'js>> {
	let value = bridge::data_to_js(ctx, data)?;
	if value.is_object() && !matches!(data, Data::Js(_)) {
		*data = Data::Js(Persistent::save(ctx, value.clone()));
	}
	Ok(value)
}

fn string<'js>(ctx: &Ctx<'js>, s: &str) -> rquickjs::Result<JsValue<'js>> {
	rquickjs::String::from_str(ctx.clone(), s).map(rquickjs::String::into_value)
}

/// `String(value)`.
fn js_string<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<String> {
	if let Some(s) = value.as_string() {
		return bridge::string(ctx, s.clone());
	}
	let string: Function = ctx.globals().get("String")?;
	let s: rquickjs::String = string.call((value,))?;
	bridge::string(ctx, s)
}

impl JsPlugin {
	/// Calls `hook` with the arguments `args` builds and waits for its
	/// result, which `convert` reads.
	async fn call<T>(
		&self,
		cx: &Context,
		hook: Hook,
		args: impl for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<Vec<JsValue<'js>>>,
		convert: impl for<'js> FnOnce(&Ctx<'js>, &Object<'js>, JsValue<'js>) -> Result<T, String>,
	) -> Result<T, String> {
		let promise = cx.engine.with(|ctx, layer| {
			let args = args(&ctx).map_err(|e| caught(&ctx, e))?;
			let list = Array::new(ctx.clone()).map_err(|e| caught(&ctx, e))?;
			for (i, arg) in args.into_iter().enumerate() {
				list.set(i, arg).map_err(|e| caught(&ctx, e))?;
			}
			let object = self
				.object
				.clone()
				.restore(&ctx)
				.map_err(|e| caught(&ctx, e))?;
			Engine::call_dash(&ctx, &layer, "callHook", (object, hook.name(), list))
				.map(|value| Persistent::save(&ctx, value))
		})?;
		let result = cx.engine.settle(promise).await?;
		cx.engine.with(|ctx, layer| {
			let value = result.restore(&ctx).map_err(|e| caught(&ctx, e))?;
			convert(&ctx, &layer, value)
		})
	}
}

/// A hook result as file data.
fn to_data<'js>(
	ctx: &Ctx<'js>,
	_: &Object<'js>,
	value: JsValue<'js>,
) -> Result<Option<Data>, String> {
	bridge::to_data(ctx, value).map_err(|e| caught(ctx, e))
}

/// The elements of an array result, or the result itself (`registerAliases`
/// and `require`); nothing for `null` and `undefined`.
fn elements<'js>(value: JsValue<'js>) -> rquickjs::Result<Vec<JsValue<'js>>> {
	if value.is_null() || value.is_undefined() {
		return Ok(Vec::new());
	}
	match value.as_array() {
		Some(array) => array.iter().collect(),
		None => Ok(vec![value]),
	}
}

impl Plugin for JsPlugin {
	fn hooks(&self) -> &[Hook] {
		&self.hooks
	}

	fn build_start<'a>(&'a self, cx: &'a Context) -> HookFuture<'a, ()> {
		Box::pin(self.call(cx, Hook::BuildStart, |_| Ok(Vec::new()), |_, _, _| Ok(())))
	}

	fn build_end<'a>(&'a self, cx: &'a Context) -> HookFuture<'a, ()> {
		Box::pin(self.call(cx, Hook::BuildEnd, |_| Ok(Vec::new()), |_, _, _| Ok(())))
	}

	fn include<'a>(&'a self, cx: &'a Context) -> HookFuture<'a, Option<Vec<Include>>> {
		Box::pin(self.call(
			cx,
			Hook::Include,
			|_| Ok(Vec::new()),
			|ctx, _, value| {
				let Some(array) = value.as_array() else {
					return Ok(None);
				};
				let mut included = Vec::new();
				for item in array.iter::<JsValue>() {
					let item = item.map_err(|e| caught(ctx, e))?;
					if item.is_string() {
						included.push(Include::Path(
							js_string(ctx, item).map_err(|e| caught(ctx, e))?,
						));
					} else if let Some(entry) = item.as_object() {
						// `addOne(includedFile[0], includedFile[1].isVirtual)`.
						let path: JsValue = entry.get(0).map_err(|e| caught(ctx, e))?;
						let options: JsValue = entry.get(1).map_err(|e| caught(ctx, e))?;
						let is_virtual = match options.as_object() {
							Some(options) => {
								let flag: JsValue =
									options.get("isVirtual").map_err(|e| caught(ctx, e))?;
								truthy(ctx, flag)
							}
							None => false,
						};
						included.push(Include::Entry(
							js_string(ctx, path).map_err(|e| caught(ctx, e))?,
							is_virtual,
						));
					}
				}
				Ok(Some(included))
			},
		))
	}

	fn ignore<'a>(&'a self, cx: &'a Context, path: &'a str) -> HookFuture<'a, bool> {
		Box::pin(self.call(
			cx,
			Hook::Ignore,
			|ctx| Ok(vec![string(ctx, path)?]),
			|ctx, _, value| Ok(truthy(ctx, value)),
		))
	}

	fn transform_path<'a>(&'a self, cx: &'a Context, path: &'a str) -> HookFuture<'a, PathChange> {
		Box::pin(self.call(
			cx,
			Hook::TransformPath,
			|ctx| Ok(vec![string(ctx, path)?]),
			|ctx, _, value| {
				Ok(if value.is_null() {
					PathChange::Omit
				} else if value.is_undefined() {
					PathChange::Keep
				} else {
					PathChange::To(js_string(ctx, value).map_err(|e| caught(ctx, e))?)
				})
			},
		))
	}

	fn read<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		file: FileHandle<'a>,
	) -> HookFuture<'a, Option<Data>> {
		Box::pin(self.call(
			cx,
			Hook::Read,
			move |ctx| {
				let handle = match file {
					FileHandle::None => JsValue::new_undefined(ctx.clone()),
					FileHandle::Unreadable => file_handle(ctx, &cx.engine.queue, path, None)?,
					FileHandle::File(bytes, cell) => {
						let existing = cell.borrow().clone();
						match existing {
							Some(handle) => handle.restore(ctx)?,
							None => {
								let handle = file_handle(ctx, &cx.engine.queue, path, Some(bytes))?;
								*cell.borrow_mut() = Some(Persistent::save(ctx, handle.clone()));
								handle
							}
						}
					}
				};
				Ok(vec![string(ctx, path)?, handle])
			},
			to_data,
		))
	}

	fn load<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		data: &'a mut Data,
	) -> HookFuture<'a, Option<Data>> {
		Box::pin(self.call(
			cx,
			Hook::Load,
			move |ctx| Ok(vec![string(ctx, path)?, data_arg(ctx, data)?]),
			to_data,
		))
	}

	fn register_aliases<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		data: &'a mut Data,
	) -> HookFuture<'a, Option<Vec<Value>>> {
		Box::pin(self.call(
			cx,
			Hook::RegisterAliases,
			move |ctx| Ok(vec![string(ctx, path)?, data_arg(ctx, data)?]),
			|ctx, layer, value| {
				let mut aliases = Vec::new();
				for item in elements(value).map_err(|e| caught(ctx, e))? {
					aliases.push(bridge::to_value(ctx, layer, item)?.unwrap_or(Value::Null));
				}
				Ok(Some(aliases))
			},
		))
	}

	fn require<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		data: Option<&'a mut Data>,
	) -> HookFuture<'a, Option<Vec<String>>> {
		Box::pin(self.call(
			cx,
			Hook::Require,
			move |ctx| {
				let data = match data {
					Some(data) => data_arg(ctx, data)?,
					None => JsValue::new_undefined(ctx.clone()),
				};
				Ok(vec![string(ctx, path)?, data])
			},
			|ctx, _, value| {
				let mut required = Vec::new();
				for item in elements(value).map_err(|e| caught(ctx, e))? {
					required.push(js_string(ctx, item).map_err(|e| caught(ctx, e))?);
				}
				Ok(Some(required))
			},
		))
	}

	fn transform<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		data: &'a mut Data,
		dependencies: &'a Dependencies,
	) -> HookFuture<'a, Option<Data>> {
		Box::pin(self.call(
			cx,
			Hook::Transform,
			move |ctx| {
				let entries = Array::new(ctx.clone())?;
				for (i, (name, data)) in dependencies.iter().enumerate() {
					let entry = Array::new(ctx.clone())?;
					entry.set(0, name.as_str())?;
					let value = match data {
						Some(data) => bridge::data_to_js(ctx, data)?,
						None => JsValue::new_undefined(ctx.clone()),
					};
					entry.set(1, value)?;
					entries.set(i, entry)?;
				}
				let object: Function = ctx
					.globals()
					.get::<_, Object>("Object")?
					.get("fromEntries")?;
				let dependencies: JsValue = object.call((entries,))?;
				Ok(vec![string(ctx, path)?, data_arg(ctx, data)?, dependencies])
			},
			to_data,
		))
	}

	fn finalize_build<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		data: &'a mut Data,
	) -> HookFuture<'a, Finalized> {
		Box::pin(self.call(
			cx,
			Hook::FinalizeBuild,
			move |ctx| Ok(vec![string(ctx, path)?, data_arg(ctx, data)?]),
			|ctx, layer, value| {
				Ok(match to_data(ctx, layer, value)? {
					None => Finalized::Undefined,
					Some(data) => Finalized::Data(data),
				})
			},
		))
	}
}

/// JavaScript truthiness of a value.
fn truthy<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> bool {
	ctx.globals()
		.get::<_, Function>("Boolean")
		.and_then(|boolean| boolean.call::<_, bool>((value,)))
		.unwrap_or(false)
}

/// The file handle of a read hook: `getFile()` resolves to the file, or to
/// `null` when Dash could not read it.
fn file_handle<'js>(
	ctx: &Ctx<'js>,
	queue: &Queue,
	path: &str,
	bytes: Option<&[u8]>,
) -> rquickjs::Result<JsValue<'js>> {
	let layer = queue.layer(ctx)?;
	let bytes = match bytes {
		Some(bytes) => TypedArray::<u8>::new(ctx.clone(), bytes.to_vec())?.into_value(),
		None => JsValue::new_null(ctx.clone()),
	};
	let dash: Object = layer.get("dash")?;
	let make: Function = dash.get("fileHandle")?;
	make.call((path, bytes))
}

/// A path argument: a string, or `None` for anything else, which no file has.
fn path_arg<'js>(ctx: &Ctx<'js>, value: &JsValue<'js>) -> Option<String> {
	value
		.as_string()
		.and_then(|s| bridge::string(ctx, s.clone()).ok())
}

fn compiler(host: &Host) -> rquickjs::Result<Rc<Compiler>> {
	host.compiler.upgrade().ok_or(rquickjs::Error::Unknown)
}

fn value_settle(value: Value) -> Settle {
	Box::new(move |ctx| bridge::to_js(&ctx, &value))
}

/// The functions of `host` the plugin context in `layer/dash.js` is built
/// on: everything a plugin reaches of the compiler.
pub(crate) fn compiler_functions<'js>(
	ctx: &Ctx<'js>,
	object: &Object<'js>,
	host: &Rc<Host>,
	queue: &Rc<Queue>,
) -> rquickjs::Result<()> {
	object.set("mode", host.mode.name())?;
	object.set("projectRoot", host.project_root.as_str())?;
	object.set("separateOutput", host.separate_output)?;
	{
		let host = Rc::clone(host);
		object.set(
			"fileDefinitions",
			Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
				bridge::to_js(&ctx, &host.file_definitions)
			})?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"packDefinitions",
			Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
				bridge::to_js(&ctx, &host.pack_definitions)
			})?,
		)?;
	}
	object.set(
		"normalize",
		Function::new(ctx.clone(), |path: String| pathe::normalize(&path))?,
	)?;
	{
		let host = Rc::clone(host);
		object.set(
			"isMatch",
			Function::new(
				ctx.clone(),
				move |ctx: Ctx<'js>, path: JsValue<'js>, pattern: JsValue<'js>| {
					let compiler = compiler(&host)?;
					let Some(pattern) = path_arg(&ctx, &pattern) else {
						return Err(Exception::throw_type(
							&ctx,
							"Expected pattern to be a non-empty string",
						));
					};
					let Some(path) = path_arg(&ctx, &path) else {
						return Err(Exception::throw_type(&ctx, "Expected input to be a string"));
					};
					compiler
						.cx
						.globs
						.is_match(&path, &pattern)
						.map_err(|error| Exception::throw_type(&ctx, &error.to_string()))
				},
			)?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"findFileType",
			Function::new(
				ctx.clone(),
				move |ctx: Ctx<'js>,
				      path: JsValue<'js>,
				      search: JsValue<'js>,
				      check: bool|
				      -> rquickjs::Result<Array<'js>> {
					let pair = |kind: &str, index: i64| -> rquickjs::Result<Array<'js>> {
						let array = Array::new(ctx.clone())?;
						array.set(0, kind)?;
						array.set(1, index)?;
						Ok(array)
					};
					let compiler = compiler(&host)?;
					// `filePath ? extname(filePath) : null`: a falsy path has
					// no type, and extname throws on anything but a string.
					let path = match path_arg(&ctx, &path) {
						Some(path) => path,
						None if !truthy(&ctx, path) => return pair("none", -1),
						None => {
							return Err(Exception::throw_type(
								&ctx,
								"input.replace is not a function",
							));
						}
					};
					let search = path_arg(&ctx, &search);
					let project = compiler.cx.project();
					match compiler
						.cx
						.file_types
						.find(&project, &path, search.as_deref(), check)
					{
						Ok(Some(index)) => pair("found", index as i64),
						Ok(None) => pair("none", -1),
						Err(FileTypeError::NoDetect(id)) => {
							let index = compiler
								.cx
								.file_types
								.all()
								.position(|(definition, _)| definition == id)
								.map_or(-1, |i| i as i64);
							pair("noDetect", index)
						}
						Err(FileTypeError::NotAFunction(what)) => Err(Exception::throw_type(
							&ctx,
							&format!("{what} is not a function"),
						)),
						Err(FileTypeError::Glob(error)) => {
							Err(Exception::throw_message(&ctx, &error.to_string()))
						}
					}
				},
			)?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"buildType",
			Function::new(ctx.clone(), move || -> rquickjs::Result<&'static str> {
				Ok(compiler(&host)?.cx.build_type.get().name())
			})?,
		)?;
	}
	{
		let (host, queue) = (Rc::clone(host), Rc::clone(queue));
		object.set(
			"requestJsonData",
			Function::new(ctx.clone(), move |ctx: Ctx<'js>, path: JsValue<'js>| {
				let path = js_string(&ctx, path)?;
				let compiler = compiler(&host)?;
				let request = Rc::clone(&compiler.cx.request_json_data);
				queue.spawn(&ctx, async move {
					let value = request(&path).await?;
					Ok(value_settle(value))
				})
			})?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"findFile",
			Function::new(ctx.clone(), move |ctx: Ctx<'js>, path: JsValue<'js>| {
				let compiler = compiler(&host)?;
				let id = path_arg(&ctx, &path).and_then(|path| compiler.files.borrow().get(&path));
				Ok::<_, rquickjs::Error>(id.map_or(-1, |id| id as i64))
			})?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"getAliases",
			Function::new(ctx.clone(), move |ctx: Ctx<'js>, path: JsValue<'js>| {
				let compiler = compiler(&host)?;
				let files = compiler.files.borrow();
				let aliases: Vec<Value> = path_arg(&ctx, &path)
					.and_then(|path| files.get(&path))
					.map(|id| {
						files
							.file(id)
							.aliases
							.iter()
							.map(|a| a.value.clone())
							.collect()
					})
					.unwrap_or_default();
				bridge::to_js(&ctx, &Value::Array(aliases.into()))
			})?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"allAliases",
			Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
				let compiler = compiler(&host)?;
				let aliases = compiler.files.borrow().alias_values();
				bridge::to_js(&ctx, &Value::Array(aliases.into()))
			})?,
		)?;
	}
	metadata_functions(ctx, object, host, queue)?;
	{
		let (host, queue) = (Rc::clone(host), Rc::clone(queue));
		object.set(
			"getOutputPath",
			Function::new(ctx.clone(), move |ctx: Ctx<'js>, path: JsValue<'js>| {
				let path = js_string(&ctx, path)?;
				let compiler = compiler(&host)?;
				queue.spawn(&ctx, async move {
					let output = compiler.output_path(&path).await;
					let settle: Settle = Box::new(move |ctx| match output {
						Some(output) => string(&ctx, &output),
						None => Ok(JsValue::new_undefined(ctx)),
					});
					Ok(settle)
				})
			})?,
		)?;
	}
	{
		let (host, queue) = (Rc::clone(host), Rc::clone(queue));
		object.set(
			"unlinkOutputFiles",
			Function::new(ctx.clone(), move |ctx: Ctx<'js>, paths: JsValue<'js>| {
				let paths = string_list(&ctx, paths)?;
				let compiler = compiler(&host)?;
				queue.spawn(&ctx, async move {
					compiler.unlink_outputs(paths).await?;
					let settle: Settle = Box::new(|ctx| Ok(JsValue::new_undefined(ctx)));
					Ok(settle)
				})
			})?,
		)?;
	}
	{
		let (host, queue) = (Rc::clone(host), Rc::clone(queue));
		object.set(
			"compileFiles",
			Function::new(
				ctx.clone(),
				move |ctx: Ctx<'js>, paths: JsValue<'js>, is_virtual: bool| {
					let paths = string_list(&ctx, paths)?;
					let compiler = compiler(&host)?;
					queue.spawn(&ctx, async move {
						compiler
							.compile_additional(paths, is_virtual)
							.await
							.map_err(|error| format!("Error: {error}"))?;
						let settle: Settle = Box::new(|ctx| Ok(JsValue::new_undefined(ctx)));
						Ok(settle)
					})
				},
			)?,
		)?;
	}
	Ok(())
}

/// The elements of an iterable of paths, each through `String(path)`.
fn string_list<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<Vec<String>> {
	let array_from: Function = ctx.globals().get::<_, Object>("Array")?.get("from")?;
	let array: Array = array_from.call((value,))?;
	array
		.iter::<JsValue>()
		.map(|item| item.and_then(|item| js_string(ctx, item)))
		.collect()
}

/// `getMetadata`, `setMetadata`, `deleteMetadata`, `setRequiredFiles` and
/// `addRequiredFile` on a file found by `findFile`.
fn metadata_functions<'js>(
	ctx: &Ctx<'js>,
	object: &Object<'js>,
	host: &Rc<Host>,
	queue: &Rc<Queue>,
) -> rquickjs::Result<()> {
	{
		let host = Rc::clone(host);
		object.set(
			"getMetadata",
			Function::new(
				ctx.clone(),
				move |ctx: Ctx<'js>, file: FileId, key: JsValue<'js>| {
					let key = js_string(&ctx, key)?;
					let compiler = compiler(&host)?;
					let files = compiler.files.borrow();
					match files.checked(file).and_then(|f| f.metadata.get(&key)) {
						Some(value) => bridge::to_js(&ctx, value),
						None => Ok(JsValue::new_undefined(ctx)),
					}
				},
			)?,
		)?;
	}
	{
		let (host, queue) = (Rc::clone(host), Rc::clone(queue));
		object.set(
			"setMetadata",
			Function::new(
				ctx.clone(),
				move |ctx: Ctx<'js>, file: FileId, key: JsValue<'js>, value: JsValue<'js>| {
					let key = js_string(&ctx, key)?;
					let compiler = compiler(&host)?;
					let layer = queue.layer(&ctx)?;
					let value = bridge::to_value(&ctx, &layer, value)
						.map_err(|message| Exception::throw_message(&ctx, &message))?;
					let mut files = compiler.files.borrow_mut();
					let Some(found) = files.checked_mut(file) else {
						return Ok(());
					};
					let metadata = &mut found.metadata;
					match value {
						Some(value) => {
							metadata.insert(key, value);
						}
						None => {
							metadata.shift_remove(&key);
						}
					}
					Ok::<_, rquickjs::Error>(())
				},
			)?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"deleteMetadata",
			Function::new(
				ctx.clone(),
				move |ctx: Ctx<'js>, file: FileId, key: JsValue<'js>| {
					let key = js_string(&ctx, key)?;
					let compiler = compiler(&host)?;
					if let Some(found) = compiler.files.borrow_mut().checked_mut(file) {
						found.metadata.shift_remove(&key);
					}
					Ok::<_, rquickjs::Error>(())
				},
			)?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"setRequiredFiles",
			Function::new(
				ctx.clone(),
				move |ctx: Ctx<'js>, file: FileId, paths: JsValue<'js>| {
					let paths = string_list(&ctx, paths)?;
					let compiler = compiler(&host)?;
					if let Some(found) = compiler.files.borrow_mut().checked_mut(file) {
						found.required_files = paths.into_iter().collect();
					}
					Ok::<_, rquickjs::Error>(())
				},
			)?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"addRequiredFile",
			Function::new(
				ctx.clone(),
				move |ctx: Ctx<'js>, file: FileId, path: JsValue<'js>| {
					let path = js_string(&ctx, path)?;
					let compiler = compiler(&host)?;
					if let Some(found) = compiler.files.borrow_mut().checked_mut(file) {
						found.required_files.insert(path);
					}
					Ok::<_, rquickjs::Error>(())
				},
			)?,
		)?;
	}
	Ok(())
}
