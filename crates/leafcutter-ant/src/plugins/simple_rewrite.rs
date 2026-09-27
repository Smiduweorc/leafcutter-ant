//! `simpleRewrite` (`src/Plugins/BuiltIn/SimpleRewrite.ts`): moves each pack
//! file into `builds/dev` or `builds/dist`, or into the development pack
//! folders of a `com.mojang` directory when the host gave Dash a separate
//! output file system.

use crate::js::{self, Prop};
use crate::pathe;
use crate::plugin::{Context, Hook, HookFuture, OptionValue, Options, PathChange, Plugin};

/// The four pack types the plugin rewrites, with their development folders
/// in a `com.mojang` directory.
const FOLDERS: [(&str, &str); 4] = [
	("behaviorPack", "development_behavior_packs"),
	("resourcePack", "development_resource_packs"),
	("skinPack", "skin_packs"),
	("worldTemplate", "minecraftWorlds"),
];

pub(crate) struct SimpleRewrite {
	options: Options,
	build_name: String,
	pack_name: String,
	has_com_mojang_directory: bool,
}

impl SimpleRewrite {
	/// The factory's defaults: `buildName` from the mode at creation, and
	/// `rewriteToComMojang: false` (or any other falsy value) turning the
	/// `com.mojang` layout off.
	pub(crate) fn new(cx: &Context, options: Options) -> Self {
		let build_name = match options.get(cx, "buildName") {
			value if value.truthy() => value.to_js_string(),
			_ if options.get(cx, "mode").is("development") => "dev".to_owned(),
			_ => "dist".to_owned(),
		};
		let pack_name = match options.get(cx, "packName") {
			value if value.truthy() => value.to_js_string(),
			_ => "Bridge".to_owned(),
		};
		let rewrite_to_com_mojang = match options.get(cx, "rewriteToComMojang") {
			value if value.is_nullish() => true,
			value => value.truthy(),
		};
		SimpleRewrite {
			has_com_mojang_directory: cx.has_com_mojang_directory && rewrite_to_com_mojang,
			options,
			build_name,
			pack_name,
		}
	}

	/// `pathPrefix(pack)`: the pack's development folder when there is a
	/// `com.mojang` directory and the mode is development, else the project's
	/// build folder.
	fn path_prefix(&self, cx: &Context, pack: &str) -> String {
		if self.has_com_mojang_directory && self.options.get(cx, "mode").is("development") {
			FOLDERS
				.iter()
				.find(|(id, _)| *id == pack)
				.map_or("undefined", |(_, folder)| folder)
				.to_owned()
		} else {
			format!("{}/builds/{}", cx.project_root, self.build_name)
		}
	}

	fn path_prefix_with_pack(&self, cx: &Context, pack: &str, suffix: &str) -> String {
		format!("{}/{} {suffix}", self.path_prefix(cx, pack), self.pack_name)
	}
}

/// `pack.defaultPackPath` in a template literal.
fn default_pack_path(definition: &crate::json::Value) -> String {
	js::own(definition, "defaultPackPath").to_js_string()
}

impl Plugin for SimpleRewrite {
	fn hooks(&self) -> &[Hook] {
		&[Hook::BuildStart, Hook::TransformPath]
	}

	/// Removes the previous output before a full build, and before every
	/// production build. Only the default pack names are removed, so output
	/// written under a `packNameSuffix` stays behind.
	fn build_start<'a>(&'a mut self, cx: &'a Context) -> HookFuture<'a> {
		Box::pin(async move {
			let production = self.options.get(cx, "mode").is("production");
			let full_build = self.options.get(cx, "buildType").is("fullBuild");
			if !production && !full_build {
				return Ok(());
			}
			if self.has_com_mojang_directory {
				for (pack_id, _) in FOLDERS {
					let Some(pack) = cx.pack_types.by_id(pack_id) else {
						continue;
					};
					let path = self.path_prefix_with_pack(cx, pack_id, &default_pack_path(pack));
					// A missing folder is fine: `.catch(() => {})`.
					let _ = cx.output_fs.unlink(&path).await;
				}
			} else {
				let _ = cx.output_fs.unlink(&self.path_prefix(cx, "BP")).await;
			}
			Ok(())
		})
	}

	fn transform_path(&mut self, cx: &Context, path: &str) -> Result<PathChange, String> {
		if path.is_empty() {
			return Ok(PathChange::Keep);
		}
		// Game tests are left where they are in production builds, which keeps
		// them out of the output: a file whose path does not change is not
		// written.
		if path.contains("BP/scripts/gametests/") && self.options.get(cx, "mode").is("production") {
			return Ok(PathChange::Keep);
		}
		let Some((pack_id, pack)) = cx.pack_types.get(&cx.project, path) else {
			return Ok(PathChange::Keep);
		};
		if !FOLDERS.iter().any(|(id, _)| *id == pack_id) {
			return Ok(PathChange::Keep);
		}
		let pack_root = cx.project.resolve_pack_path(Some(pack_id), None);
		let relative = pathe::relative(&pack_root, path);
		let suffix = match self.options.get(cx, "packNameSuffix") {
			OptionValue::Value(suffixes) => match js::get(suffixes, pack_id) {
				Prop::Value(crate::json::Value::Null) | Prop::Undefined => None,
				found => Some(found.to_js_string()),
			},
			_ => None,
		}
		.unwrap_or_else(|| default_pack_path(pack));
		let prefix = self.path_prefix_with_pack(cx, pack_id, &suffix);
		Ok(PathChange::To(pathe::join(&[&prefix, &relative])))
	}
}

#[cfg(test)]
mod tests {
	use std::rc::Rc;

	use futures_executor::block_on;

	use crate::console::tests::Recorder;
	use crate::json::parse_json5;
	use crate::project::{DetectMatcher, FileTypes, PackTypes};
	use crate::testing::MemoryFs;
	use crate::{Dash, DashOptions, Mode};

	fn build(input: Rc<MemoryFs>, output: Rc<MemoryFs>, mode: Mode) {
		let options = DashOptions {
			config: "./config.json".to_owned(),
			compiler_config: None,
			mode,
			console: Rc::new(Recorder::default()),
			verbose: false,
			pack_types: PackTypes::new(
				parse_json5(
					"[{id: 'behaviorPack', defaultPackPath: 'BP'}, {id: 'resourcePack', defaultPackPath: 'RP'}]",
				)
				.expect("json5"),
			)
			.expect("pack definitions"),
			file_types: FileTypes::new(parse_json5("[]").expect("json5"), DetectMatcher::Glob)
				.expect("definitions"),
		};
		let mut dash = Dash::new(input, Some(output), options);
		block_on(dash.setup()).expect("setup");
		block_on(dash.build()).expect("build");
	}

	#[test]
	fn a_full_build_clears_the_default_pack_folder_but_not_a_suffixed_one() {
		// SimpleRewrite.ts buildStart unlinks `<packName> <defaultPackPath>`
		// whatever packNameSuffix says, so output under a suffix piles up.
		let config = r#"{"packs": {"behaviorPack": "./BP"}, "compiler": {"plugins": [["simpleRewrite", {"packNameSuffix": {"behaviorPack": "BPx"}}]]}}"#;
		let input = MemoryFs::with(&[("config.json", config), ("BP/manifest.json", "{}")]);
		let output = MemoryFs::with(&[
			("development_behavior_packs/Bridge BP/old.json", "old"),
			("development_behavior_packs/Bridge BPx/old.json", "old"),
		]);
		build(input, output.clone(), Mode::Development);
		assert_eq!(
			output.paths(),
			[
				"development_behavior_packs/Bridge BPx/manifest.json",
				"development_behavior_packs/Bridge BPx/old.json"
			]
		);
	}

	#[test]
	fn a_plugin_option_named_mode_replaces_the_build_mode_for_that_plugin() {
		// AllPlugins.ts getPluginContext spreads the options after the `mode`
		// getter, so a `mode` option takes its place: a development build with
		// a com.mojang folder writes production paths, `builds/dist` included.
		let config = r#"{"packs": {"behaviorPack": "./BP"}, "compiler": {"plugins": [["simpleRewrite", {"mode": "production"}]]}}"#;
		let input = MemoryFs::with(&[("config.json", config), ("BP/manifest.json", "{}")]);
		let output = MemoryFs::with(&[]);
		build(input, output.clone(), Mode::Development);
		assert_eq!(output.paths(), ["builds/dist/Bridge BP/manifest.json"]);
	}
}
