//! leafcutter-ant, a Rust port of the Dash compiler (`@bridge-editor/dash-compiler`
//! 0.13.0) for Minecraft Bedrock add-ons. The goal is byte-identical output
//! with TS Dash on the same project; the port proceeds from the leaves up,
//! and so far holds the JSON layer every later part is built on.

// A library does not own the terminal: it returns values and lets the host
// decide what to print.
#![deny(clippy::print_stdout, clippy::print_stderr)]

// The next two are reached from the project model and the pipeline, which
// arrive in later commits; each `expect` turns into a warning once every item
// in its module has a caller.
#[cfg_attr(not(test), expect(dead_code))]
mod glob;
pub mod json;
#[cfg_attr(not(test), expect(dead_code))]
mod pathe;

// Runs the Rust examples in the README as doctests, so the README cannot show
// code that no longer compiles or no longer holds.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct ReadmeExamples;
