//! The built-in plugins (`src/Plugins/BuiltIn/`), by the names a plugin list
//! uses for them.

mod contents_file;
mod entity_identifier;
mod float_fix;
mod format_version;
mod rewrite_for_packaging;
mod simple_rewrite;
mod typescript;

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
		"contentsFile" => BuiltIn::Plugin(Box::new(contents_file::ContentsFile::new(cx, options))),
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
		"typeScript" => BuiltIn::Plugin(Box::new(typescript::TypeScript::new(cx, options))),
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
