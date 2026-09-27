//! `typeScript` (`src/Plugins/BuiltIn/TypeScript.ts`): compiles `.ts` files to
//! `.js` with swc, the compiler TS Dash runs as `@swc/wasm-web` 1.6.5, built
//! here from the same swc release. Declaration files (`.d.ts`) are left out
//! of the output.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use swc_common::errors::ColorConfig;
use swc_common::{FileName, GLOBALS, Globals, SourceMap};

use crate::fs;
use crate::json::Value;
use crate::pathe;
use crate::plugin::{Context, Data, FileHandle, Finalized, Hook, Options, PathChange, Plugin};

pub(crate) struct TypeScript {
	inline_source_map: bool,
}

impl TypeScript {
	pub(crate) fn new(cx: &Context, options: Options) -> Self {
		TypeScript {
			inline_source_map: options.get(cx, "inlineSourceMap").truthy(),
		}
	}
}

/// `transformSync(source, options).code` with the options the plugin passes
/// (`filename` is the file's base name, `swcrc` stays off as it is in the
/// browser build, and the working directory, which nothing reads, is fixed
/// so swc does not ask the process for one). An error is swc's message.
///
/// swc panics on a few inputs; in the browser build a panic is thrown as an
/// error, which the plugin host catches, so here it is caught as one. The
/// process's panic hook still sees it.
pub(crate) fn transform(
	source: &str,
	filename: &str,
	inline_source_map: bool,
) -> Result<String, String> {
	let mut options = serde_json::json!({
		"filename": filename,
		"swcrc": false,
		"cwd": "/",
		"jsc": {
			"parser": { "syntax": "typescript" },
			"preserveAllComments": false,
			"target": "es2020",
			"transform": { "useDefineForClassFields": false }
		}
	});
	if inline_source_map {
		options["sourceMaps"] = "inline".into();
	}
	let options: swc::config::Options =
		serde_json::from_value(options).map_err(|e| e.to_string())?;
	let run = || {
		let cm: Arc<SourceMap> = Arc::default();
		let compiler = swc::Compiler::new(Arc::clone(&cm));
		GLOBALS.set(&Globals::new(), || {
			swc::try_with_handler(
				Arc::clone(&cm),
				swc::HandlerOpts {
					color: ColorConfig::Never,
					skip_filename: false,
				},
				|handler| {
					let name = if filename.is_empty() {
						FileName::Anon
					} else {
						FileName::Real(filename.into())
					};
					let file = cm.new_source_file(name, source.to_owned());
					compiler.process_js_file(file, handler, &options)
				},
			)
			.map(|output| output.code)
			.map_err(|error| format!("{error:?}"))
		})
	};
	catch_unwind(AssertUnwindSafe(run))
		.unwrap_or_else(|_| Err("RuntimeError: unreachable".to_owned()))
}

impl Plugin for TypeScript {
	fn hooks(&self) -> &[Hook] {
		&[
			Hook::Ignore,
			Hook::TransformPath,
			Hook::Read,
			Hook::Load,
			Hook::FinalizeBuild,
		]
	}

	fn ignore(&mut self, _cx: &Context, path: &str) -> Result<bool, String> {
		Ok(!path.ends_with(".ts"))
	}

	fn transform_path(&mut self, _cx: &Context, path: &str) -> Result<PathChange, String> {
		if !path.ends_with(".ts") {
			return Ok(PathChange::Keep);
		}
		if path.ends_with(".d.ts") {
			return Ok(PathChange::Omit);
		}
		Ok(PathChange::To(format!("{}.js", &path[..path.len() - 3])))
	}

	fn read(
		&mut self,
		_cx: &Context,
		path: &str,
		file: FileHandle<'_>,
	) -> Result<Option<Data>, String> {
		match file {
			FileHandle::File(bytes) if path.ends_with(".ts") => {
				Ok(Some(Data::Value(Value::String(fs::text(bytes)))))
			}
			_ => Ok(None),
		}
	}

	/// A file swc refuses keeps its TypeScript source, and `finalizeBuild`
	/// then writes that source to the `.js` path.
	fn load(&mut self, _cx: &Context, path: &str, data: &mut Data) -> Result<Option<Data>, String> {
		let Data::Value(Value::String(source)) = data else {
			return Ok(None);
		};
		if !path.ends_with(".ts") {
			return Ok(None);
		}
		let code = transform(source, &pathe::basename(path, None), self.inline_source_map)?;
		Ok(Some(Data::Value(Value::String(code))))
	}

	fn finalize_build(
		&mut self,
		_cx: &Context,
		path: &str,
		data: &Data,
	) -> Result<Finalized, String> {
		match data {
			Data::Value(Value::String(_)) if path.ends_with(".ts") => Ok(Finalized::Current),
			_ => Ok(Finalized::Undefined),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// typescript.json, recorded by tools/parity/typescript.mjs from
	/// @swc/wasm-web 1.6.5 with the plugin's options.
	#[test]
	fn the_output_matches_swc_wasm_web_1_6_5() {
		let vectors = crate::testing::vectors("typescript.json");
		let vectors = vectors.as_array().expect("an array");
		assert!(vectors.len() >= 40);
		let mut failures = Vec::new();
		for vector in vectors {
			let (name, source) = (
				vector[0].as_str().expect("a name"),
				vector[1].as_str().expect("a source"),
			);
			let inline = vector[2].as_bool().expect("a flag");
			let actual = transform(source, name, inline).ok();
			if actual.as_deref() != vector[3].as_str() {
				failures.push(format!(
					"{name} inline={inline}:\n  expected {}\n  got      {actual:?}",
					vector[3]
				));
			}
		}
		assert!(
			failures.is_empty(),
			"{} of {} differ:\n{}",
			failures.len(),
			vectors.len(),
			failures.join("\n")
		);
	}
}
