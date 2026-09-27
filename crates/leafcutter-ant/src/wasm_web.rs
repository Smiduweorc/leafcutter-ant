//! swc, the compiler TS Dash runs as `@swc/wasm-web` 1.6.5, built here from
//! the same release: `transformSync`, `minifySync` and `parseSync` as the
//! browser build runs them, each on a compiler of its own.
//!
//! swc panics on a few inputs. In the browser build a panic is thrown as an
//! error, which the caller catches, so here it is caught as one too; the
//! process's panic hook still sees it.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use swc_common::errors::{ColorConfig, Handler};
use swc_common::{FileName, GLOBALS, Globals, SourceFile, SourceMap};
use swc_ecma_ast::{EsVersion, Module};
use swc_ecma_parser::Syntax;

/// Runs `run` with a fresh compiler, source map and error handler, and turns
/// swc's error or panic into the message the browser build throws.
fn with_compiler<T>(
	run: impl FnOnce(&swc::Compiler, &Arc<SourceMap>, &Handler) -> Result<T, anyhow::Error>,
) -> Result<T, String> {
	let attempt = || {
		let cm: Arc<SourceMap> = Arc::default();
		let compiler = swc::Compiler::new(Arc::clone(&cm));
		GLOBALS.set(&Globals::new(), || {
			swc::try_with_handler(
				Arc::clone(&cm),
				swc::HandlerOpts {
					color: ColorConfig::Never,
					skip_filename: false,
				},
				|handler| run(&compiler, &cm, handler),
			)
			.map_err(|error| format!("{error:?}"))
		})
	};
	catch_unwind(AssertUnwindSafe(attempt))
		.unwrap_or_else(|_| Err("RuntimeError: unreachable".to_owned()))
}

fn source_file(cm: &SourceMap, filename: &str, source: &str) -> Arc<SourceFile> {
	let name = if filename.is_empty() {
		FileName::Anon
	} else {
		FileName::Real(filename.into())
	};
	cm.new_source_file(name, source.to_owned())
}

/// `transformSync(source, options).code`. `options` is the JSON the caller
/// hands wasm-web; `swcrc` stays off as it is in the browser build, and the
/// working directory, which nothing reads, is fixed so swc does not ask the
/// process for one.
pub(crate) fn transform(source: &str, mut options: serde_json::Value) -> Result<String, String> {
	options["swcrc"] = false.into();
	options["cwd"] = "/".into();
	let filename = options["filename"].as_str().unwrap_or_default().to_owned();
	let options: swc::config::Options =
		serde_json::from_value(options).map_err(|error| error.to_string())?;
	with_compiler(|compiler, cm, handler| {
		let file = source_file(cm, &filename, source);
		compiler
			.process_js_file(file, handler, &options)
			.map(|output| output.code)
	})
}

/// `minifySync(source, options).code`, on a file with no name, as the
/// browser build reads it.
pub(crate) fn minify(source: &str, options: serde_json::Value) -> Result<String, String> {
	let options: swc::config::JsMinifyOptions =
		serde_json::from_value(options).map_err(|error| error.to_string())?;
	with_compiler(|compiler, cm, handler| {
		let file = source_file(cm, "", source);
		compiler
			.minify(file, handler, &options)
			.map(|output| output.code)
	})
}

/// `parseSync(source, { syntax, target })` of a module, handing the tree to
/// `inspect` with the position of the file's first byte, which the spans in
/// it count from.
pub(crate) fn parse_module<T>(
	source: &str,
	syntax: Syntax,
	target: EsVersion,
	inspect: impl FnOnce(&Module, u32) -> T,
) -> Result<T, String> {
	with_compiler(|compiler, cm, handler| {
		let file = source_file(cm, "", source);
		let start = file.start_pos.0;
		let program = compiler.parse_js(
			file,
			handler,
			target,
			syntax,
			swc::config::IsModule::Bool(true),
			None,
		)?;
		match program.module() {
			Some(module) => Ok(inspect(&module, start)),
			None => Err(anyhow::anyhow!("not a module")),
		}
	})
}
