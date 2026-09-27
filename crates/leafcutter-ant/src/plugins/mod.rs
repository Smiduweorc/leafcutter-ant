//! The built-in plugins (`src/Plugins/BuiltIn/`), by the names a plugin list
//! uses for them.

mod entity_identifier;
mod float_fix;
mod format_version;
mod rewrite_for_packaging;
mod simple_rewrite;

use crate::plugin::{Context, Options, Plugin};

/// What a plugin list entry names.
pub(crate) enum BuiltIn {
	Plugin(Box<dyn Plugin>),
	/// A built-in plugin that runs user JavaScript, which leafcutter-ant
	/// cannot do yet.
	NeedsJavaScript,
	Unknown,
}

/// `builtInPlugins[id]`, created with its context and options.
pub(crate) fn create(id: &str, cx: &Context, options: Options) -> BuiltIn {
	match id {
		"entityIdentifierAlias" => {
			BuiltIn::Plugin(Box::new(entity_identifier::EntityIdentifierAlias))
		}
		"floatPropertyTruncationFix" => {
			BuiltIn::Plugin(Box::new(float_fix::FloatPropertyTruncationFix))
		}
		"formatVersionCorrection" => {
			BuiltIn::Plugin(Box::new(format_version::FormatVersionCorrection::new(cx)))
		}
		"rewriteForPackaging" => BuiltIn::Plugin(Box::new(
			rewrite_for_packaging::RewriteForPackaging::new(cx, options),
		)),
		"simpleRewrite" => {
			BuiltIn::Plugin(Box::new(simple_rewrite::SimpleRewrite::new(cx, options)))
		}
		"moLang"
		| "molang"
		| "customEntityComponents"
		| "customItemComponents"
		| "customBlockComponents"
		| "customCommands"
		| "generatorScripts" => BuiltIn::NeedsJavaScript,
		_ => BuiltIn::Unknown,
	}
}
