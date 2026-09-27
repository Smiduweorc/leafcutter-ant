//! The built-in plugins (`src/Plugins/BuiltIn/`), by the names a plugin list
//! uses for them.

use crate::plugin::{Context, Options, Plugin};

/// What a plugin list entry names.
pub(crate) enum BuiltIn {
	#[expect(
		dead_code,
		reason = "the built-in plugins land one per commit after the host"
	)]
	Plugin(Box<dyn Plugin>),
	/// A built-in plugin that runs user JavaScript, which leafcutter-ant
	/// cannot do yet.
	NeedsJavaScript,
	Unknown,
}

/// `builtInPlugins[id]`, created with its context and options.
pub(crate) fn create(id: &str, _cx: &Context, _options: Options) -> BuiltIn {
	match id {
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
