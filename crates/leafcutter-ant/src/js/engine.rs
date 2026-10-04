//! The engine user scripts and extension plugins run in: QuickJS, through
//! rquickjs, with the JavaScript half in `layer/runtime.js`.
//!
//! Scripts await host work (file reads, fetched modules, requested data, and
//! the compiler itself for `compileFiles`). Each such call returns a promise
//! and queues an operation: a Rust future and the functions that settle the
//! promise. Nothing polls those futures on its own. [`Engine::settle`], which
//! the compiler awaits whenever it waits for a promise, runs the engine's
//! jobs and polls the queued operations until the promise settles, so the
//! host's executor drives everything and the library picks none.
//!
//! What a script can reach is written down in the README ("What scripts may
//! touch"): ECMAScript's globals, `console`, `File` and `Blob`, the values
//! Dash hands it, and host work only through those values. The engine has no
//! timers, no `fetch` and no process or file system access of its own.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Poll, Wake, Waker};
use std::time::{Duration, Instant};

use rquickjs::function::Opt;
use rquickjs::{
	Context, Ctx, Exception, Function, Object, Persistent, Promise, Runtime, TypedArray,
	Value as JsValue,
};

use crate::console::Logger;
use crate::dash::Compiler;
use crate::fs::{self, EntryKind, FileSystem, FsError, FsFuture};
use crate::json::{Value, parse_json5};
use crate::plugin::{Mode, Output};

use super::bridge;
use super::transform::transform_source;

/// A JavaScript value kept alive outside a context scope.
pub(crate) type Handle = Persistent<JsValue<'static>>;

/// Turns the outcome of host work into the value a promise resolves with,
/// inside a context scope.
pub(crate) type Settle = Box<dyn for<'js> FnOnce(Ctx<'js>) -> rquickjs::Result<JsValue<'js>>>;

/// Host work a script is waiting for. `Err` rejects the promise with an
/// `Error` carrying the message.
pub(crate) type OpFuture = Pin<Box<dyn Future<Output = Result<Settle, String>>>>;

struct Op {
	future: OpFuture,
	resolve: Persistent<Function<'static>>,
	reject: Persistent<Function<'static>>,
}

/// What the engine keeps between polls: queued operations by creation order,
/// and the count of host futures the compiler itself is awaiting, which the
/// file systems it is given report through [`Counted`].
#[derive(Default)]
pub(crate) struct Queue {
	ops: RefCell<BTreeMap<u64, Op>>,
	next_op: Cell<u64>,
	in_flight: Cell<usize>,
	/// Bumped each time the whole build is found waiting on promises that
	/// nothing can settle any more; a settle started before a bump fails.
	stuck: Cell<u64>,
	/// What `layer/runtime.js` returned, for host functions that call back
	/// into it.
	layer: RefCell<Option<Persistent<Object<'static>>>>,
}

impl Queue {
	/// The layer object, inside a context scope.
	pub(crate) fn layer<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
		let layer = self
			.layer
			.borrow()
			.clone()
			.ok_or(rquickjs::Error::Unknown)?;
		layer.restore(ctx)
	}

	/// Queues `future` behind a new promise, which it settles.
	pub(crate) fn spawn<'js>(
		&self,
		ctx: &Ctx<'js>,
		future: impl Future<Output = Result<Settle, String>> + 'static,
	) -> rquickjs::Result<Promise<'js>> {
		let (promise, resolve, reject) = ctx.promise()?;
		let id = self.next_op.get();
		self.next_op.set(id + 1);
		self.ops.borrow_mut().insert(
			id,
			Op {
				future: Box::pin(future),
				resolve: Persistent::save(ctx, resolve),
				reject: Persistent::save(ctx, reject),
			},
		);
		Ok(promise)
	}
}

/// How long a script may run each time the compiler hands it control, before
/// the engine stops it with an error that cannot be caught.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptTimeLimit {
	/// No limit, as in TS Dash: a script that never returns stops the build.
	Unlimited,
	/// Stop a script that runs longer than this without returning.
	After(Duration),
}

/// A host's fetch: the response body at a URL, or why there is none.
pub type Fetch = Rc<dyn Fn(&str) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, String>>>>>;

/// How a script's `import` of an `https://` URL is fetched.
#[derive(Clone)]
pub enum HttpsImports {
	/// Every such import fails, as if the network were down.
	Refused,
	/// The host fetches the URL and returns the response body.
	Fetch(Fetch),
}

/// What the host-facing functions of the engine need.
pub(crate) struct Host {
	/// The input file system, and the output one, which may be the same.
	pub(crate) file_systems: [Rc<dyn FileSystem>; 2],
	pub(crate) logger: Rc<Logger>,
	pub(crate) https_imports: HttpsImports,
	/// The compiler the plugin context reaches into.
	pub(crate) compiler: Weak<Compiler>,
	pub(crate) mode: Mode,
	pub(crate) project_root: String,
	/// Whether the host gave an output file system of its own.
	pub(crate) separate_output: bool,
	/// The file definitions in detection order, and the pack definitions.
	pub(crate) file_definitions: Value,
	pub(crate) pack_definitions: Value,
}

pub(crate) struct Engine {
	pub(crate) queue: Rc<Queue>,
	layer: Persistent<Object<'static>>,
	deadline: Rc<Cell<Option<Instant>>>,
	limit: ScriptTimeLimit,
	context: Context,
	runtime: Runtime,
}

impl Drop for Engine {
	fn drop(&mut self) {
		// Queued operations hold handles, which must go before the runtime
		// that owns what they point at.
		self.queue.ops.borrow_mut().clear();
		self.queue.layer.borrow_mut().take();
	}
}

const PATH_BROWSERIFY: &str = include_str!("layer/path-browserify.js");
const PATHE: &str = include_str!("layer/pathe.js");
const RUNTIME: &str = include_str!("layer/runtime.js");
const DASH: &str = include_str!("layer/dash.js");

impl Engine {
	pub(crate) fn new(host: Host, limit: ScriptTimeLimit) -> Result<Engine, String> {
		let runtime = Runtime::new().map_err(|e| e.to_string())?;
		let deadline: Rc<Cell<Option<Instant>>> = Rc::default();
		{
			let deadline = Rc::clone(&deadline);
			runtime.set_interrupt_handler(Some(Box::new(move || {
				deadline.get().is_some_and(|at| Instant::now() > at)
			})));
		}
		let context = Context::full(&runtime).map_err(|e| e.to_string())?;
		let queue = Rc::new(Queue::default());
		let host = Rc::new(host);
		let layer = context.with(|ctx| -> rquickjs::Result<_> {
			let path_browserify: JsValue = ctx.eval(format!(
				"(() => {{ const module = {{ exports: {{}} }};\n{PATH_BROWSERIFY}\nreturn module.exports }})()"
			))?;
			let pathe: JsValue = ctx.eval(format!("(() => {{\n{PATHE}\n}})()"))?;
			let setup: Function = ctx.eval(RUNTIME)?;
			let host_object = host_functions(&ctx, &host, &queue)?;
			let layer: Object =
				setup.call((host_object.clone(), path_browserify, pathe.clone()))?;
			*queue.layer.borrow_mut() = Some(Persistent::save(&ctx, layer.clone()));
			let dash_setup: Function = ctx.eval(DASH)?;
			let dash: Object = dash_setup.call((host_object, layer.clone(), pathe))?;
			layer.set("dash", dash)?;
			Ok(Persistent::save(&ctx, layer))
		});
		let layer = layer.map_err(|e| e.to_string())?;
		Ok(Engine {
			queue,
			layer,
			deadline,
			limit,
			context,
			runtime,
		})
	}

	/// Runs `f` in the engine's context, with the script time limit counting
	/// from now.
	pub(crate) fn with<R>(&self, f: impl for<'js> FnOnce(Ctx<'js>, Object<'js>) -> R) -> R {
		self.deadline.set(match self.limit {
			ScriptTimeLimit::Unlimited => None,
			ScriptTimeLimit::After(limit) => Some(Instant::now() + limit),
		});
		let result = self.context.with(|ctx| {
			let layer = self
				.layer
				.clone()
				.restore(&ctx)
				.expect("the layer was saved from this runtime");
			f(ctx, layer)
		});
		self.deadline.set(None);
		result
	}

	/// Runs the engine's pending jobs; whether there were any.
	fn run_jobs(ctx: &Ctx<'_>) -> bool {
		let mut ran = false;
		while ctx.execute_pending_job() {
			ran = true;
		}
		ran
	}

	/// Polls every queued operation once, settling those that finished, and
	/// says whether any did. An operation is out of the queue while it is
	/// polled, so an operation that awaits the engine itself (the compiler
	/// running for `compileFiles`) is not polled again from inside.
	fn poll_ops(&self, cx: &mut std::task::Context<'_>) -> bool {
		let ids: Vec<u64> = self.queue.ops.borrow().keys().copied().collect();
		let mut settled = false;
		for id in ids {
			let Some(mut op) = self.queue.ops.borrow_mut().remove(&id) else {
				continue;
			};
			match op.future.as_mut().poll(cx) {
				Poll::Pending => {
					self.queue.ops.borrow_mut().insert(id, op);
				}
				Poll::Ready(outcome) => {
					settled = true;
					self.with(|ctx, _| {
						let resolved = outcome.and_then(|settle| {
							settle(ctx.clone()).map_err(|error| caught(&ctx, error))
						});
						let call = match resolved {
							Ok(value) => op
								.resolve
								.restore(&ctx)
								.and_then(|resolve| resolve.call::<_, ()>((value,))),
							Err(message) => {
								let error = Exception::from_message(ctx.clone(), &message)
									.map(Exception::into_value);
								op.reject
									.restore(&ctx)
									.and_then(|reject| reject.call::<_, ()>((error?,)))
							}
						};
						// Settling a promise only queues jobs; it cannot throw.
						drop(call);
					});
				}
			}
		}
		settled
	}

	/// Waits for `value` as `await` does, driving the engine meanwhile, and
	/// returns what it resolved to, or the rejection as the text
	/// `String(reason)` gives.
	pub(crate) async fn settle(&self, value: Handle) -> Result<Handle, String> {
		let promise = self.with(|ctx, layer| -> Result<Handle, String> {
			let settle: Function = layer.get("settle").map_err(|e| caught(&ctx, e))?;
			let value = value.restore(&ctx).map_err(|e| caught(&ctx, e))?;
			let promise: JsValue = settle.call((value,)).map_err(|e| caught(&ctx, e))?;
			Ok(Persistent::save(&ctx, promise))
		})?;
		let started = self.queue.stuck.get();
		// The first poll only queues the call: other futures the executor
		// polls next start their calls too before any job runs, so that
		// scripts of different files interleave as promises do in one event
		// loop, instead of each running to its end before the next starts.
		let mut queued = false;
		poll_fn(|cx| {
			if !queued {
				queued = true;
				cx.waker().wake_by_ref();
				return Poll::Pending;
			}
			let mut progressed = false;
			loop {
				let outcome = self.with(|ctx, layer| {
					let outcome = || {
						let promise: Promise = promise
							.clone()
							.restore(&ctx)
							.ok()
							.and_then(JsValue::into_promise)
							.expect("the layer's settle returns a promise of this runtime");
						match promise.result::<JsValue>() {
							None => None,
							Some(Ok(value)) => Some(Ok(Persistent::save(&ctx, value))),
							Some(Err(_)) => Some(Err(describe(&ctx, &layer, ctx.catch()))),
						}
					};
					// A promise that settled while another future ran the
					// jobs returns at once, so that its caller goes on to its
					// next call before the jobs that call queues run.
					if let Some(settled) = outcome() {
						return Some(settled);
					}
					progressed |= Self::run_jobs(&ctx);
					outcome()
				});
				if let Some(outcome) = outcome {
					return Poll::Ready(outcome);
				}
				if self.poll_ops(cx) {
					progressed = true;
					continue;
				}
				if self.queue.stuck.get() > started {
					return Poll::Ready(Err(
						"Error: the promise can never settle: nothing it waits for is still running"
							.to_owned(),
					));
				}
				if progressed {
					// Another future waiting on the engine may be able to go
					// on now; have the executor poll again.
					cx.waker().wake_by_ref();
				}
				return Poll::Pending;
			}
		})
		.await
	}

	/// Settles every queued operation, for work a script started without
	/// waiting for it, as an event loop would before the build returns.
	pub(crate) async fn drain(&self) {
		poll_fn(|cx| {
			loop {
				let ran = self.with(|ctx, _| Self::run_jobs(&ctx));
				let settled = self.poll_ops(cx);
				if self.queue.ops.borrow().is_empty() && !self.runtime.is_job_pending() {
					return Poll::Ready(());
				}
				if !ran && !settled {
					return Poll::Pending;
				}
			}
		})
		.await;
	}

	/// Whether the engine can do nothing more until a host future finishes:
	/// no jobs, no queued operations, no host future in flight.
	fn idle(&self) -> bool {
		!self.runtime.is_job_pending()
			&& self.queue.ops.borrow().is_empty()
			&& self.queue.in_flight.get() == 0
	}

	/// Runs a compiler entry point. When it waits and nothing at all can wake
	/// it (every awaited promise waits on something that will never happen,
	/// such as a hook returning `new Promise(() => {})`), each settle that is
	/// waiting fails instead, so the build reports the hook and goes on
	/// rather than waiting forever.
	pub(crate) async fn run<T>(&self, future: impl Future<Output = T>) -> T {
		let mut future = pin!(future);
		poll_fn(|cx| {
			let mut bumped = false;
			loop {
				let flag = Arc::new(Flag {
					woken: AtomicBool::new(false),
					outer: cx.waker().clone(),
				});
				let waker = Waker::from(Arc::clone(&flag));
				let mut inner = std::task::Context::from_waker(&waker);
				if let Poll::Ready(value) = future.as_mut().poll(&mut inner) {
					return Poll::Ready(value);
				}
				if flag.woken.load(Ordering::Relaxed) || bumped || !self.idle() {
					return Poll::Pending;
				}
				self.queue.stuck.set(self.queue.stuck.get() + 1);
				bumped = true;
			}
		})
		.await
	}

	/// Calls a function of the layer's Dash half (`layer/dash.js`).
	pub(crate) fn call_dash<'js, A: rquickjs::function::IntoArgs<'js>>(
		ctx: &Ctx<'js>,
		layer: &Object<'js>,
		name: &str,
		args: A,
	) -> Result<JsValue<'js>, String> {
		let dash: Object = layer.get("dash").map_err(|e| caught(ctx, e))?;
		let function: Function = dash.get(name).map_err(|e| caught(ctx, e))?;
		function.call(args).map_err(|e| caught(ctx, e))
	}

	/// `projectConfig.data`, after `setup` read the config.
	pub(crate) fn reset_project(&self, data: &Value) {
		self.with(|ctx, layer| {
			let result = bridge::to_js(&ctx, data)
				.map_err(|e| caught(&ctx, e))
				.and_then(|data| Self::call_dash(&ctx, &layer, "setProjectData", (data,)));
			// Building a value or setting a property fails only when memory
			// runs out.
			drop(result);
		});
	}

	/// `clearCache()` on Dash's runtime (`dash.jsRuntime`), as every build
	/// starts.
	pub(crate) fn clear_cache(&self) {
		self.clear_runtime_cache("jsRuntime");
	}

	/// `clearCache()` on the runtime extension plugins are evaluated in, as
	/// `loadPlugins` starts.
	pub(crate) fn clear_plugin_cache(&self) {
		self.clear_runtime_cache("pluginRuntime");
	}

	fn clear_runtime_cache(&self, name: &str) {
		self.with(|ctx, layer| {
			let runtime = layer
				.get::<_, Object>("dash")
				.and_then(|dash| dash.get::<_, JsValue>(name));
			if let Ok(runtime) = runtime {
				drop(call_method(&ctx, &runtime, "clearCache", Vec::new()));
			}
		});
	}

	/// What TS Dash writes for a script's value (`layer.output`).
	pub(crate) fn output(&self, handle: &Handle) -> Result<Output, String> {
		self.with(|ctx, layer| {
			let value = handle.clone().restore(&ctx).map_err(|e| caught(&ctx, e))?;
			let output: Function = layer.get("output").map_err(|e| caught(&ctx, e))?;
			let pair: rquickjs::Array = output.call((value,)).map_err(|e| caught(&ctx, e))?;
			let kind: String = pair.get(0).map_err(|e| caught(&ctx, e))?;
			let text: rquickjs::String = pair.get(1).map_err(|e| caught(&ctx, e))?;
			let text = bridge::string(&ctx, text).map_err(|e| caught(&ctx, e))?;
			Ok(match kind.as_str() {
				"text" => Output::Bytes(text.into_bytes()),
				"bytes" => Output::Bytes(bridge::latin1(&text)),
				_ => Output::Nothing,
			})
		})
	}

	/// What `JSON.stringify` makes of a script's value, as JSON.
	pub(crate) fn to_value(&self, handle: &Handle) -> Result<Option<Value>, String> {
		self.with(|ctx, layer| {
			let value = handle.clone().restore(&ctx).map_err(|e| caught(&ctx, e))?;
			bridge::to_value(&ctx, &layer, value)
		})
	}
}

/// Records a wake of the task, and passes it on.
struct Flag {
	woken: AtomicBool,
	outer: Waker,
}

impl Wake for Flag {
	fn wake(self: Arc<Self>) {
		self.wake_by_ref();
	}

	fn wake_by_ref(self: &Arc<Self>) {
		self.woken.store(true, Ordering::Relaxed);
		self.outer.wake_by_ref();
	}
}

/// The text of a thrown value: `String(value)`, as a template literal or the
/// console shows an error.
fn describe<'js>(ctx: &Ctx<'js>, layer: &Object<'js>, value: JsValue<'js>) -> String {
	let text = layer
		.get::<_, Function>("describe")
		.and_then(|describe| describe.call::<_, rquickjs::String>((value,)))
		.and_then(|s| bridge::string(ctx, s));
	text.unwrap_or_else(|_| "an exception that has no text".to_owned())
}

/// The text of what a call into the engine threw.
pub(crate) fn caught(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
	match error {
		rquickjs::Error::Exception => {
			let value = ctx.catch();
			if value.is_uncatchable_error() {
				return "InternalError: interrupted: the script ran past its time limit".to_owned();
			}
			match ctx
				.globals()
				.get::<_, Function>("String")
				.and_then(|string| string.call::<_, rquickjs::String>((value,)))
				.and_then(|s| bridge::string(ctx, s))
			{
				Ok(text) => text,
				Err(_) => "an exception that has no text".to_owned(),
			}
		}
		other => format!("Error: {other}"),
	}
}

/// The `host` object `layer/runtime.js` gets: the only way its code reaches
/// Rust.
fn host_functions<'js>(
	ctx: &Ctx<'js>,
	host: &Rc<Host>,
	queue: &Rc<Queue>,
) -> rquickjs::Result<Object<'js>> {
	let object = Object::new(ctx.clone())?;
	object.set(
		"encode",
		Function::new(ctx.clone(), |ctx: Ctx<'js>, text: String| {
			TypedArray::<u8>::new(ctx, text.into_bytes())
		})?,
	)?;
	object.set(
		"decode",
		Function::new(ctx.clone(), |bytes: String| {
			fs::text(&bridge::latin1(&bytes))
		})?,
	)?;
	object.set(
		"parseJson5",
		Function::new(
			ctx.clone(),
			|ctx: Ctx<'js>, text: String| match parse_json5(&text) {
				Ok(value) => bridge::to_js(&ctx, &value),
				Err(error) => Err(Exception::throw_syntax(&ctx, &error.to_string())),
			},
		)?,
	)?;
	object.set(
		"transform",
		Function::new(
			ctx.clone(),
			|ctx: Ctx<'js>, path: String, basename: String, source: String| {
				transform_source(&path, &basename, &source)
					.map_err(|message| Exception::throw_message(&ctx, &message))
			},
		)?,
	)?;
	{
		let host = Rc::clone(host);
		object.set(
			"log",
			Function::new(ctx.clone(), move |level: String, text: String| {
				let console = host.logger.console();
				match level.as_str() {
					"error" => console.error(&text),
					"warn" => console.warn(&text),
					"info" => console.info(&text),
					_ => console.log(&text),
				}
			})?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"time",
			Function::new(ctx.clone(), move |name: String| host.logger.time(&name))?,
		)?;
	}
	{
		let host = Rc::clone(host);
		object.set(
			"timeEnd",
			Function::new(ctx.clone(), move |name: String| host.logger.time_end(&name))?,
		)?;
	}
	{
		let (host, queue) = (Rc::clone(host), Rc::clone(queue));
		object.set(
			"fetch",
			Function::new(ctx.clone(), move |ctx: Ctx<'js>, url: String| {
				let fetch = host.https_imports.clone();
				queue.spawn(&ctx, async move {
					let bytes = match fetch {
						HttpsImports::Refused => {
							return Err("TypeError: Failed to fetch: https imports are turned off"
								.to_owned());
						}
						HttpsImports::Fetch(fetch) => fetch(&url).await?,
					};
					Ok(bytes_settle(bytes))
				})
			})?,
		)?;
	}
	file_system_functions(ctx, &object, host, queue)?;
	super::plugin::compiler_functions(ctx, &object, host, queue)?;
	Ok(object)
}

fn bytes_settle(bytes: Vec<u8>) -> Settle {
	Box::new(move |ctx| TypedArray::<u8>::new(ctx, bytes).map(TypedArray::into_value))
}

fn undefined_settle() -> Settle {
	Box::new(|ctx| Ok(JsValue::new_undefined(ctx)))
}

/// `readFile`, `writeFile`, `unlink`, `readdir`, `mkdir` and `lastModified`
/// on the input (0) or output (1) file system, each returning a promise.
fn file_system_functions<'js>(
	ctx: &Ctx<'js>,
	object: &Object<'js>,
	host: &Rc<Host>,
	queue: &Rc<Queue>,
) -> rquickjs::Result<()> {
	type Start = fn(Rc<dyn FileSystem>, String, Option<Vec<u8>>) -> OpFuture;
	// The layer hands written bytes over as Latin-1 text, one character per
	// byte.
	let operations: [(&str, Start); 6] = [
		("readFile", |fs, path, _| {
			Box::pin(async move { Ok(bytes_settle(fs.read_file(&path).await.map_err(fs_error)?)) })
		}),
		("writeFile", |fs, path, bytes| {
			Box::pin(async move {
				let bytes = bytes.unwrap_or_default();
				fs.write_file(&path, &bytes).await.map_err(fs_error)?;
				Ok(undefined_settle())
			})
		}),
		("unlink", |fs, path, _| {
			Box::pin(async move {
				fs.unlink(&path).await.map_err(fs_error)?;
				Ok(undefined_settle())
			})
		}),
		("readdir", |fs, path, _| {
			Box::pin(async move {
				let entries = fs.readdir(&path).await.map_err(fs_error)?;
				let settle: Settle = Box::new(move |ctx| {
					let list = rquickjs::Array::new(ctx.clone())?;
					for (i, entry) in entries.into_iter().enumerate() {
						let item = Object::new(ctx.clone())?;
						item.set("name", entry.name)?;
						item.set(
							"kind",
							match entry.kind {
								EntryKind::Directory => "directory",
								EntryKind::File => "file",
							},
						)?;
						list.set(i, item)?;
					}
					Ok(list.into_value())
				});
				Ok(settle)
			})
		}),
		("mkdir", |fs, path, _| {
			Box::pin(async move {
				fs.mkdir(&path).await.map_err(fs_error)?;
				Ok(undefined_settle())
			})
		}),
		("lastModified", |fs, path, _| {
			Box::pin(async move {
				let time = fs.last_modified(&path).await.map_err(fs_error)?;
				let settle: Settle = Box::new(move |ctx| Ok(JsValue::new_number(ctx, time)));
				Ok(settle)
			})
		}),
	];
	for (name, start) in operations {
		let (host, queue) = (Rc::clone(host), Rc::clone(queue));
		let writes = name == "writeFile";
		object.set(
			name,
			Function::new(
				ctx.clone(),
				move |ctx: Ctx<'js>, id: usize, path: String, content: Opt<String>| {
					let fs = Rc::clone(&host.file_systems[id.min(1)]);
					let bytes = content
						.0
						.filter(|_| writes)
						.map(|text| bridge::latin1(&text));
					queue.spawn(&ctx, start(fs, path, bytes))
				},
			)?,
		)?;
	}
	Ok(())
}

fn fs_error(error: FsError) -> String {
	format!("Error: {error}")
}

/// A file system whose futures the engine counts while they are pending, so
/// that it can tell a build waiting on the host from one that can never go
/// on. The compiler reads and writes through these.
pub(crate) struct Counted {
	inner: Rc<dyn FileSystem>,
	queue: Rc<Queue>,
}

impl Counted {
	pub(crate) fn new(inner: Rc<dyn FileSystem>, queue: Rc<Queue>) -> Self {
		Counted { inner, queue }
	}

	fn count<'a, T: 'a>(&'a self, future: FsFuture<'a, T>) -> FsFuture<'a, T> {
		Box::pin(InFlight::new(&self.queue, future))
	}
}

/// A host future the engine counts as in flight until it finishes or is
/// dropped.
pub(crate) struct InFlight<'a, F: ?Sized> {
	queue: &'a Queue,
	future: Pin<Box<F>>,
	done: bool,
}

impl<'a, F: Future + ?Sized> InFlight<'a, F> {
	pub(crate) fn new(queue: &'a Queue, future: Pin<Box<F>>) -> Self {
		queue.in_flight.set(queue.in_flight.get() + 1);
		InFlight {
			queue,
			future,
			done: false,
		}
	}
}

impl<F: Future + ?Sized> Future for InFlight<'_, F> {
	type Output = F::Output;

	fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<F::Output> {
		let outcome = self.future.as_mut().poll(cx);
		if outcome.is_ready() && !self.done {
			self.done = true;
			self.queue.in_flight.set(self.queue.in_flight.get() - 1);
		}
		outcome
	}
}

impl<F: ?Sized> Drop for InFlight<'_, F> {
	fn drop(&mut self) {
		if !self.done {
			self.queue.in_flight.set(self.queue.in_flight.get() - 1);
		}
	}
}

impl FileSystem for Counted {
	fn read_file<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<u8>> {
		self.count(self.inner.read_file(path))
	}

	fn write_file<'a>(&'a self, path: &'a str, content: &'a [u8]) -> FsFuture<'a, ()> {
		self.count(self.inner.write_file(path, content))
	}

	fn unlink<'a>(&'a self, path: &'a str) -> FsFuture<'a, ()> {
		self.count(self.inner.unlink(path))
	}

	fn readdir<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<fs::DirEntry>> {
		self.count(self.inner.readdir(path))
	}

	fn mkdir<'a>(&'a self, path: &'a str) -> FsFuture<'a, ()> {
		self.count(self.inner.mkdir(path))
	}

	fn last_modified<'a>(&'a self, path: &'a str) -> FsFuture<'a, f64> {
		self.count(self.inner.last_modified(path))
	}
}

/// A method call `object[name](...args)` whose result is kept as a handle.
pub(crate) fn call_method<'js>(
	ctx: &Ctx<'js>,
	object: &JsValue<'js>,
	name: &str,
	args: Vec<JsValue<'js>>,
) -> Result<JsValue<'js>, String> {
	let target = object
		.as_object()
		.ok_or_else(|| format!("TypeError: cannot call {name} on a non-object"))?;
	let function: Function = target.get(name).map_err(|e| caught(ctx, e))?;
	let mut call_args = rquickjs::function::Args::new(ctx.clone(), args.len());
	call_args.this(object.clone()).map_err(|e| caught(ctx, e))?;
	call_args.push_args(args).map_err(|e| caught(ctx, e))?;
	function.call_arg(call_args).map_err(|e| caught(ctx, e))
}
