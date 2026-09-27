//! The library half of the workspace. Everything the program does lives here,
//! so the CLI and any later host (a Tauri app, a WASM build) stay thin.
//! Replace `greet` with the project's own API.

// A library does not own the terminal: it returns values and lets the host
// decide what to print.
#![deny(clippy::print_stdout, clippy::print_stderr)]

/// Returns the greeting for `name`. The name is used as given, so an empty
/// name still produces a greeting.
///
/// ```
/// assert_eq!(app::greet("world"), "hello, world");
/// ```
pub fn greet(name: &str) -> String {
	format!("hello, {name}")
}
