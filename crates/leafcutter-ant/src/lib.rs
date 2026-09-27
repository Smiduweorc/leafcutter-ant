//! leafcutter-ant, a Rust port of the Dash compiler (`@bridge-editor/dash-compiler`
//! 0.13.0) for Minecraft Bedrock add-ons. The goal is byte-identical output
//! with TS Dash on the same project. [`Dash`] builds a project read through
//! a [`fs::FileSystem`] the host supplies.

// A library does not own the terminal: it returns values and lets the host
// decide what to print.
#![deny(clippy::print_stdout, clippy::print_stderr)]

pub mod console;
mod dash;
mod files;
pub mod fs;
// The built-in plugins, which land one per commit after the pipeline, are
// what calls the rest of these; each `expect` turns into a warning once every
// item in its module has a caller.
#[cfg_attr(not(test), expect(dead_code))]
mod glob;
mod js;
pub mod json;
#[cfg_attr(not(test), expect(dead_code))]
mod pathe;
#[expect(dead_code)]
mod plugin;
mod plugins;
#[expect(dead_code)]
pub mod project;

pub use dash::{Dash, DashError, DashOptions};
pub use plugin::{BuildType, Mode};

// Runs the Rust examples in the README as doctests, so the README cannot show
// code that no longer compiles or no longer holds.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct ReadmeExamples;
