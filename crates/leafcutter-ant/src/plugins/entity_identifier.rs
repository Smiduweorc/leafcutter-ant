//! `entityIdentifierAlias` (`src/Plugins/BuiltIn/EntityIdentifier.ts`): gives
//! each entity file its identifier as an alias, so other files can require
//! it by `namespace:name`.
//!
//! The plugin reads nothing itself; it sees an entity's JSON only when an
//! earlier plugin read the file.

use crate::js::{self, Prop};
use crate::json::Value;
use crate::plugin::{Context, Data, Hook, HookFuture, Plugin, ready};

pub(crate) struct EntityIdentifierAlias;

fn is_entity(cx: &Context, path: &str) -> Result<bool, String> {
	Ok(cx
		.file_types
		.id(&cx.project(), path)
		.map_err(|e| e.to_string())?
		== "entity")
}

impl EntityIdentifierAlias {
	fn ignore(&self, cx: &Context, path: &str) -> Result<bool, String> {
		Ok(!is_entity(cx, path)?)
	}

	/// `fileContent?.["minecraft:entity"]?.description?.identifier`, when it
	/// is truthy, whatever its type.
	fn register_aliases(
		&self,
		cx: &Context,
		path: &str,
		data: &Data,
	) -> Result<Option<Vec<Value>>, String> {
		if !is_entity(cx, path)? {
			return Ok(None);
		}
		let Data::Value(content) = data else {
			// The only shared data is the contents file's list, an array,
			// which has no "minecraft:entity".
			return Ok(None);
		};
		let identifier = ["minecraft:entity", "description", "identifier"]
			.iter()
			.try_fold(content, |at, key| match js::get(at, key) {
				Prop::Value(Value::Null) | Prop::Undefined | Prop::Inherited(_) => None,
				Prop::Value(next) => Some(next),
			});
		Ok(identifier
			.filter(|identifier| js::truthy(identifier))
			.map(|identifier| vec![identifier.clone()]))
	}
}

impl Plugin for EntityIdentifierAlias {
	fn hooks(&self) -> &[Hook] {
		&[Hook::Ignore, Hook::RegisterAliases]
	}

	fn ignore<'a>(&'a self, cx: &'a Context, path: &'a str) -> HookFuture<'a, bool> {
		ready(self.ignore(cx, path))
	}

	fn register_aliases<'a>(
		&'a self,
		cx: &'a Context,
		path: &'a str,
		data: &'a mut Data,
	) -> HookFuture<'a, Option<Vec<Value>>> {
		ready(self.register_aliases(cx, path, data))
	}
}

#[cfg(test)]
mod tests {
	use futures_executor::block_on;

	use crate::json::{Indent, stringify};
	use crate::plugin::Data;
	use crate::testing::{MemoryFs, dash_with, plugin_vectors, value};

	/// plugins.json, recorded by tools/parity/plugins.mjs from TS Dash's own
	/// plugin: `ignore` and `registerAliases` for entity and other paths, with
	/// identifiers of every JSON type.
	#[test]
	fn ignore_and_aliases_match_ts_dash() {
		let (dash, _) = dash_with(MemoryFs::with(&[]), r#"["entityIdentifierAlias"]"#);
		let vectors = plugin_vectors("entityIdentifierAlias");
		assert!(vectors.len() > 150);
		let mut failures = Vec::new();
		for vector in &vectors {
			let path = vector[0].as_str().expect("a path");
			let mut data = Data::Value(value(&vector[1]));
			let (cx, plugin) = dash.first_plugin();
			let ignored = block_on(plugin.ignore(cx, path)).expect("no error");
			let aliases =
				match block_on(plugin.register_aliases(cx, path, &mut data)).expect("no error") {
					None => "\"<undefined>\"".to_owned(),
					Some(aliases) => {
						stringify(&crate::json::Value::Array(aliases.into()), Indent::None)
					}
				};
			let aliases_json: serde_json::Value =
				serde_json::from_str(&aliases).expect("our output is JSON");
			if serde_json::Value::Bool(ignored) != vector[2] || aliases_json != vector[3] {
				failures.push(format!(
					"{path} {}: expected {} {}, got {ignored} {aliases}",
					vector[1], vector[2], vector[3]
				));
			}
		}
		assert!(
			failures.is_empty(),
			"{} differ:\n{}",
			failures.len(),
			failures.join("\n")
		);
	}
}
