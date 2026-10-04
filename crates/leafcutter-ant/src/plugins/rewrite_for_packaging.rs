//! `rewriteForPackaging` (`src/Plugins/BuiltIn/RewriteForPackaging.ts`): lays
//! the packs out in `builds/dist` the way an `.mcaddon`, `.mctemplate` or
//! `.mcworld` archive holds them.

use crate::pathe;
use crate::plugin::{Context, Hook, HookFuture, Options, PathChange, Plugin, ready};

pub(crate) struct RewriteForPackaging {
	options: Options,
	pack_name: String,
}

impl RewriteForPackaging {
	pub(crate) fn new(cx: &Context, options: Options) -> Self {
		let pack_name = match options.get(cx, "packName") {
			value if value.truthy() => value.to_js_string(),
			_ => "bridge project".to_owned(),
		};
		RewriteForPackaging { options, pack_name }
	}

	fn mcaddon(cx: &Context, path: &str) -> PathChange {
		let pack_id = cx.pack_types.id(&cx.project(), path);
		let relative = pathe::relative(&cx.project_root, path);
		match pack_id.as_str() {
			"behaviorPack" | "resourcePack" | "skinPack" => PathChange::To(pathe::join(&[
				&cx.project_root,
				"builds/dist",
				&pack_id,
				&relevant_file_path(&relative),
			])),
			_ => PathChange::Keep,
		}
	}

	fn mctemplate(&self, cx: &Context, path: &str) -> PathChange {
		let pack_id = cx.pack_types.id(&cx.project(), path);
		let relative = relevant_file_path(&pathe::relative(&cx.project_root, path));
		match pack_id.as_str() {
			"worldTemplate" => {
				PathChange::To(pathe::join(&[&cx.project_root, "builds/dist", &relative]))
			}
			"behaviorPack" | "resourcePack" => {
				let folder = if pack_id == "behaviorPack" {
					"behavior_packs"
				} else {
					"resource_packs"
				};
				PathChange::To(pathe::join(&[
					&cx.project_root,
					"builds/dist",
					folder,
					&self.pack_name,
					&relative,
				]))
			}
			_ => PathChange::Keep,
		}
	}
}

/// `relevantFilePath`: the path without its first segment and without `.`
/// and `..` segments, split on either slash.
fn relevant_file_path(path: &str) -> String {
	let parts: Vec<&str> = path
		.split(['/', '\\'])
		.filter(|part| *part != ".." && *part != ".")
		.collect();
	parts.get(1..).unwrap_or(&[]).join("/")
}

impl RewriteForPackaging {
	fn build_start<'a>(&'a self, cx: &'a Context) -> HookFuture<'a, ()> {
		Box::pin(async move {
			// A missing folder is fine: `.catch(() => {})`.
			let _ = cx
				.output_fs
				.unlink(&format!("{}/builds/dist", cx.project_root))
				.await;
			Ok(())
		})
	}

	/// A `format` other than the three it knows logs an error for every file
	/// and leaves the path alone.
	fn transform_path(&self, cx: &Context, path: &str) -> Result<PathChange, String> {
		if path.is_empty() {
			return Ok(PathChange::Keep);
		}
		let format = self.options.get(cx, "format");
		if format.is("mcaddon") {
			Ok(Self::mcaddon(cx, path))
		} else if format.is("mcworld") {
			// .mcworld is .mctemplate without the world manifest.
			match cx
				.file_types
				.id(&cx.project(), path)
				.map_err(|e| e.to_string())?
				.as_str()
			{
				"worldManifest" => Ok(PathChange::Omit),
				_ => Ok(self.mctemplate(cx, path)),
			}
		} else if format.is("mctemplate") {
			Ok(self.mctemplate(cx, path))
		} else {
			cx.logger.console().error(&format!(
				"Unknown packaging format: {}",
				format.to_js_string()
			));
			Ok(PathChange::Keep)
		}
	}
}

impl Plugin for RewriteForPackaging {
	fn hooks(&self) -> &[Hook] {
		&[Hook::BuildStart, Hook::TransformPath]
	}

	fn build_start<'a>(&'a self, cx: &'a Context) -> HookFuture<'a, ()> {
		self.build_start(cx)
	}

	fn transform_path<'a>(&'a self, cx: &'a Context, path: &'a str) -> HookFuture<'a, PathChange> {
		ready(self.transform_path(cx, path))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_relevant_path_drops_the_first_segment_and_dot_segments() {
		assert_eq!(relevant_file_path("BP/entities/a.json"), "entities/a.json");
		assert_eq!(
			relevant_file_path("../../BP/manifest.json"),
			"manifest.json"
		);
		assert_eq!(relevant_file_path("behavPack\\x\\y.json"), "x/y.json");
		assert_eq!(relevant_file_path("./BP/./a/../b"), "a/b");
		assert_eq!(relevant_file_path("single"), "");
		assert_eq!(relevant_file_path(""), "");
	}
}
