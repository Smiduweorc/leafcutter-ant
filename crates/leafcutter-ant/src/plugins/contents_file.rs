//! `contentsFile` (`src/Plugins/BuiltIn/ContentsFile.ts`): meant to write a
//! `contents.json` into each pack listing its files. It is active only when
//! created for a full build.
//!
//! Its checks are inverted: `read` and `finalizeBuild` answer for every pack
//! file except `contents.json`, so every other file of a pack is written as
//! the pack's list, and `contents.json` itself, which an `include` hook adds
//! as a virtual file, is read as nothing and never written. The list holds
//! whatever path each `transformPath` call was given, the contents files
//! included, and it is never emptied, so every build in the same session
//! appends the whole pack again.

use std::cell::RefCell;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::json::{Array, Indent, Value, stringify};
use crate::plugin::BuildType;
use crate::plugin::{
	Context, Data, FileHandle, Finalized, Hook, Include, Options, PathChange, Plugin,
};

pub(crate) struct ContentsFile {
	/// `packContents`: each pack id in the config with the list of paths,
	/// which every file of the pack gets as its data.
	contents: IndexMap<String, Rc<RefCell<Value>>>,
	active: bool,
}

impl ContentsFile {
	pub(crate) fn new(cx: &Context, options: Options) -> Self {
		let contents = cx
			.project
			.available_packs()
			.into_iter()
			.map(|(id, _)| (id, Rc::new(RefCell::new(Value::Array(Array::new())))))
			.collect();
		ContentsFile {
			contents,
			active: options.get(cx, "buildType").is(BuildType::FullBuild.name()),
		}
	}

	/// `isContentsFile(filePath)`: the pack the path is in and that pack's
	/// `contents.json` path, for a path inside a known pack.
	fn pack_and_contents_path(cx: &Context, path: &str) -> Option<(String, String)> {
		let pack_id = cx.pack_types.id(&cx.project, path);
		if pack_id == "unknown" {
			return None;
		}
		let contents_path = cx
			.project
			.resolve_pack_path(Some(&pack_id), Some("contents.json"));
		Some((pack_id, contents_path))
	}
}

impl Plugin for ContentsFile {
	fn hooks(&self) -> &[Hook] {
		if self.active {
			&[
				Hook::Include,
				Hook::TransformPath,
				Hook::Read,
				Hook::FinalizeBuild,
			]
		} else {
			&[]
		}
	}

	fn include(&mut self, cx: &Context) -> Result<Option<Vec<Include>>, String> {
		Ok(Some(
			self.contents
				.keys()
				.map(|id| {
					Include::Entry(
						cx.project
							.resolve_pack_path(Some(id), Some("contents.json")),
						true,
					)
				})
				.collect(),
		))
	}

	fn read(
		&mut self,
		cx: &Context,
		path: &str,
		_file: FileHandle<'_>,
	) -> Result<Option<Data>, String> {
		let Some((pack_id, contents_path)) = Self::pack_and_contents_path(cx, path) else {
			return Ok(None);
		};
		if path == contents_path {
			return Ok(None);
		}
		Ok(self
			.contents
			.get(&pack_id)
			.map(|list| Data::Shared(Rc::clone(list))))
	}

	fn transform_path(&mut self, cx: &Context, path: &str) -> Result<PathChange, String> {
		if path.is_empty() {
			return Ok(PathChange::Keep);
		}
		let pack_id = cx.pack_types.id(&cx.project, path);
		if pack_id == "unknown" {
			return Ok(PathChange::Keep);
		}
		let Some(list) = self.contents.get(&pack_id) else {
			return Err(
				"TypeError: Cannot read properties of undefined (reading 'push')".to_owned(),
			);
		};
		if let Value::Array(items) = &mut *list.borrow_mut() {
			items.push(Value::String(path.to_owned()));
		}
		Ok(PathChange::Keep)
	}

	fn finalize_build(
		&mut self,
		cx: &Context,
		path: &str,
		_data: &Data,
	) -> Result<Finalized, String> {
		let Some((pack_id, contents_path)) = Self::pack_and_contents_path(cx, path) else {
			return Ok(Finalized::Undefined);
		};
		if path == contents_path {
			return Ok(Finalized::Undefined);
		}
		// JSON.stringify(undefined) is undefined, for a pack not in the list.
		Ok(match self.contents.get(&pack_id) {
			Some(list) => Finalized::Data(Data::Value(Value::String(stringify(
				&list.borrow(),
				Indent::None,
			)))),
			None => Finalized::Undefined,
		})
	}
}

#[cfg(test)]
mod tests {
	use futures_executor::block_on;

	use crate::testing::{MemoryFs, dash_with};

	#[test]
	fn every_build_in_a_session_appends_the_whole_pack_again() {
		// ContentsFile.ts never empties packContents, and a Dash keeps its
		// plugins from setup to setup.
		let fs = MemoryFs::with(&[("BP/a.json", "{}")]);
		let (mut dash, _) = dash_with(fs.clone(), r#"["contentsFile", "simpleRewrite"]"#);
		block_on(dash.build()).expect("the first build");
		assert_eq!(
			fs.text("builds/dev/Bridge BP/a.json").as_deref(),
			Some(r#"["BP/contents.json","BP/a.json"]"#)
		);
		block_on(dash.build()).expect("the second build");
		assert_eq!(
			fs.text("builds/dev/Bridge BP/a.json").as_deref(),
			Some(r#"["BP/contents.json","BP/a.json","BP/contents.json","BP/a.json"]"#)
		);
	}
}
