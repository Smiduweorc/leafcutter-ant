//! `formatVersionCorrection` (`src/Plugins/BuiltIn/FormatVersionCorrection.ts`):
//! rewrites `format_version` through the `formatVersionMap` a file definition
//! carries (blocks, items and fogs in the vendored definitions), and writes
//! those files as compact JSON.

use std::collections::{HashMap, HashSet};

use crate::fs;
use crate::js::{self, Prop};
use crate::json::{Indent, Object, Value, parse_json5, stringify};
use crate::plugin::{Context, Data, FileHandle, Finalized, Hook, Plugin};

pub(crate) struct FormatVersionCorrection {
	/// The ids of the file types that have a `formatVersionMap`.
	to_transform: HashSet<String>,
	/// `needsTransformationCache`, which is read only for a `true` result;
	/// a `false` one is stored and then computed again on every call. That
	/// costs time, not output.
	cache: HashMap<String, bool>,
}

impl FormatVersionCorrection {
	pub(crate) fn new(cx: &Context) -> Self {
		let to_transform = cx
			.file_types
			.all()
			.filter(
				|(_, definition)| match js::own(definition, "formatVersionMap") {
					Prop::Value(map) => js::truthy(map),
					_ => false,
				},
			)
			.map(|(id, _)| id.to_owned())
			.collect();
		FormatVersionCorrection {
			to_transform,
			cache: HashMap::new(),
		}
	}

	fn needs_transformation(&mut self, cx: &Context, path: &str) -> Result<bool, String> {
		if path.is_empty() {
			return Ok(false);
		}
		if self.cache.get(path) == Some(&true) {
			return Ok(true);
		}
		let id = cx
			.file_types
			.id(&cx.project, path)
			.map_err(|e| e.to_string())?;
		let needs = self.to_transform.contains(&id);
		self.cache.insert(path.to_owned(), needs);
		Ok(needs)
	}
}

/// `object[key] = value` where the value may be an inherited property. A
/// function has no JSON form, and `JSON.stringify` leaves such a property
/// out, so the key is removed here; `Object.prototype` (`__proto__`) has no
/// enumerable properties and is written as `{}`.
fn assign(object: &mut Object, key: &str, value: Prop<'_>) {
	match value {
		Prop::Value(value) => {
			object.insert(key.to_owned(), value.clone());
		}
		Prop::Inherited(inherited)
			if inherited == js::Inherited::lookup("__proto__").expect("listed") =>
		{
			object.insert(key.to_owned(), Value::Object(Object::new()));
		}
		Prop::Inherited(_) => {
			object.remove(key);
		}
		Prop::Undefined => {}
	}
}

impl Plugin for FormatVersionCorrection {
	fn hooks(&self) -> &[Hook] {
		&[
			Hook::Ignore,
			Hook::Read,
			Hook::Load,
			Hook::Transform,
			Hook::FinalizeBuild,
		]
	}

	fn ignore(&mut self, cx: &Context, path: &str) -> Result<bool, String> {
		Ok(!self.needs_transformation(cx, path)?)
	}

	/// The file as json5. A file that does not parse is reported and read as
	/// nothing, so it is copied as it is.
	fn read(
		&mut self,
		cx: &Context,
		path: &str,
		file: FileHandle<'_>,
	) -> Result<Option<Data>, String> {
		if matches!(file, FileHandle::None) || !self.needs_transformation(cx, path)? {
			return Ok(None);
		}
		let FileHandle::File(bytes) = file else {
			return Ok(None);
		};
		match parse_json5(&fs::text(bytes)) {
			Ok(value) => Ok(Some(Data::Value(value))),
			Err(error) => {
				// TS Dash hands the error to the global console.
				cx.logger.console().error(&format!("SyntaxError: {error}"));
				Ok(None)
			}
		}
	}

	fn load(&mut self, cx: &Context, path: &str, _data: &mut Data) -> Result<Option<Data>, String> {
		self.needs_transformation(cx, path)?;
		Ok(None)
	}

	/// `format_version` looked up in the map, which is a plain object: a
	/// version named like a property of `Object.prototype` finds that
	/// property.
	fn transform(
		&mut self,
		cx: &Context,
		path: &str,
		data: &mut Data,
	) -> Result<Option<Data>, String> {
		if !self.needs_transformation(cx, path)? {
			return Ok(None);
		}
		let definition = cx
			.file_types
			.get(&cx.project, path)
			.map_err(|e| e.to_string())?;
		let Some(map) =
			definition.and_then(
				|(_, definition)| match js::own(definition, "formatVersionMap") {
					Prop::Value(map) if js::truthy(map) => Some(map),
					_ => None,
				},
			)
		else {
			return Ok(None);
		};
		// The contents file's shared list is an array, which has no
		// format_version.
		let Data::Value(Value::Object(content)) = data else {
			return Ok(None);
		};
		let Some(version) = content
			.get("format_version")
			.filter(|version| js::truthy(version))
		else {
			return Ok(None);
		};
		let mapped = js::get(map, &js::to_js_string(version));
		let replace = match mapped {
			Prop::Value(value) => js::truthy(value),
			Prop::Inherited(_) => true,
			Prop::Undefined => false,
		};
		if replace {
			assign(content, "format_version", mapped);
		}
		Ok(None)
	}

	fn finalize_build(
		&mut self,
		cx: &Context,
		path: &str,
		data: &Data,
	) -> Result<Finalized, String> {
		if !self.needs_transformation(cx, path)? {
			return Ok(Finalized::Undefined);
		}
		let json = match data {
			Data::Value(value) => stringify(value, Indent::None),
			Data::Shared(value) => stringify(&value.borrow(), Indent::None),
		};
		Ok(Finalized::Data(Data::Value(Value::String(json))))
	}
}

#[cfg(test)]
mod tests {
	use crate::json::{Value, parse_json5};
	use crate::plugin::{Data, Finalized};
	use crate::testing::{MemoryFs, dash_with};

	#[test]
	fn a_proto_key_gives_the_file_no_inherited_format_version() {
		// json5 2.2.1 makes the value of "__proto__" the object's prototype,
		// so in TS Dash `fileContent.format_version` finds "1.18.30" there and
		// the output gains `"format_version":"1.18.0"`. leafcutter-ant drops
		// the key and models no prototype: a recorded difference.
		let mut dash = dash_with(MemoryFs::with(&[]), r#"["formatVersionCorrection"]"#);
		let (cx, plugin) = dash.first_plugin();
		let source = r#"{"__proto__": {"format_version": "1.18.30"}, "k": 1}"#;
		let mut data = Data::Value(parse_json5(source).expect("json5"));
		plugin
			.transform(cx, "BP/blocks/a.json", &mut data)
			.expect("no error");
		let Finalized::Data(Data::Value(Value::String(json))) = plugin
			.finalize_build(cx, "BP/blocks/a.json", &data)
			.expect("no error")
		else {
			panic!("finalizeBuild gives a string");
		};
		assert_eq!(json, r#"{"k":1}"#);
	}
}
