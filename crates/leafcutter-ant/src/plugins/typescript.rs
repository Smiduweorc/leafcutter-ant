//! `typeScript` (`src/Plugins/BuiltIn/TypeScript.ts`): compiles `.ts` files to
//! `.js` with swc, the compiler TS Dash runs as `@swc/wasm-web` 1.6.5, built
//! here from the same swc release. Declaration files (`.d.ts`) are left out
//! of the output.

use crate::fs;
use crate::json::Value;
use crate::pathe;
use crate::plugin::{
	Context, Data, FileHandle, Finalized, Hook, HookFuture, Options, PathChange, Plugin, ready,
};
use crate::wasm_web;

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

/// `transformSync(source, options).code` with the options the plugin passes;
/// `filename` is the file's base name. An error is swc's message.
pub(crate) fn transform(
	source: &str,
	filename: &str,
	inline_source_map: bool,
) -> Result<String, String> {
	let mut options = serde_json::json!({
		"filename": filename,
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
	wasm_web::transform(source, options)
}

impl TypeScript {
	fn ignore(&self, _cx: &Context, path: &str) -> Result<bool, String> {
		Ok(!path.ends_with(".ts"))
	}

	fn transform_path(&self, _cx: &Context, path: &str) -> Result<PathChange, String> {
		if !path.ends_with(".ts") {
			return Ok(PathChange::Keep);
		}
		if path.ends_with(".d.ts") {
			return Ok(PathChange::Omit);
		}
		Ok(PathChange::To(format!("{}.js", &path[..path.len() - 3])))
	}

	fn read(
		&self,
		_cx: &Context,
		path: &str,
		file: FileHandle<'_>,
	) -> Result<Option<Data>, String> {
		match file {
			FileHandle::File(bytes, _) if path.ends_with(".ts") => {
				Ok(Some(Data::Value(Value::String(fs::text(bytes)))))
			}
			_ => Ok(None),
		}
	}

	/// A file swc refuses keeps its TypeScript source, and `finalizeBuild`
	/// then writes that source to the `.js` path.
	fn load(&self, _cx: &Context, path: &str, data: &mut Data) -> Result<Option<Data>, String> {
		let Data::Value(Value::String(source)) = data else {
			return Ok(None);
		};
		if !path.ends_with(".ts") {
			return Ok(None);
		}
		let code = transform(source, &pathe::basename(path, None), self.inline_source_map)?;
		Ok(Some(Data::Value(Value::String(code))))
	}

	fn finalize_build(&self, _cx: &Context, path: &str, data: &Data) -> Result<Finalized, String> {
		match data {
			Data::Value(Value::String(_)) if path.ends_with(".ts") => Ok(Finalized::Current),
			_ => Ok(Finalized::Undefined),
		}
	}
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

	fn ignore<'a>(&'a self, cx: &'a Context, path: &'a str) -> HookFuture<'a, bool> {
		ready(self.ignore(cx, path))
	}

	fn transform_path<'a>(&'a self, cx: &'a Context, path: &'a str) -> HookFuture<'a, PathChange> {
		ready(self.transform_path(cx, path))
	}

	fn read<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		file: FileHandle<'a>,
	) -> HookFuture<'a, Option<Data>> {
		ready(self.read(cx, path, file))
	}

	fn load<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		data: &'a mut Data,
	) -> HookFuture<'a, Option<Data>> {
		ready(self.load(cx, path, data))
	}

	fn finalize_build<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		data: &'a mut Data,
	) -> HookFuture<'a, Finalized> {
		ready(self.finalize_build(cx, path, data))
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
