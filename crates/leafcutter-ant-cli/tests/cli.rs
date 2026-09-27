//! Runs the built `leafcutter` binary, which is what a user gets, rather
//! than calling functions in `main.rs`.

use std::ffi::OsString;
use std::process::{Command, Output, Stdio};

fn run<I, S>(args: I) -> Output
where
	I: IntoIterator<Item = S>,
	S: Into<OsString>,
{
	Command::new(env!("CARGO_BIN_EXE_leafcutter"))
		.args(args.into_iter().map(Into::into))
		.stdin(Stdio::null())
		.output()
		.expect("the leafcutter binary starts")
}

fn assert_usage_error(output: &Output) {
	assert_eq!(output.status.code(), Some(2));
	assert_eq!(output.stdout, b"");
	assert_eq!(output.stderr, b"usage: leafcutter [--version | --help]\n");
}

fn assert_help(output: &Output) {
	assert!(output.status.success());
	assert_eq!(output.stderr, b"");
	let stdout = String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8");
	assert!(
		stdout.starts_with("usage: leafcutter [--version | --help]\n\n"),
		"{stdout:?}"
	);
	assert!(
		stdout.contains("It cannot build a project yet"),
		"{stdout:?}"
	);
	assert!(stdout.ends_with("print this text\n"), "{stdout:?}");
}

#[test]
fn without_arguments_it_prints_the_help_and_succeeds() {
	assert_help(&run::<[&str; 0], _>([]));
}

#[test]
fn the_help_flags_print_the_help_and_succeed() {
	assert_help(&run(["--help"]));
	assert_help(&run(["-h"]));
}

#[test]
fn the_version_flag_prints_the_workspace_version() {
	let output = run(["--version"]);
	assert!(output.status.success());
	let expected = format!("leafcutter {}\n", env!("CARGO_PKG_VERSION"));
	assert_eq!(output.stdout, expected.as_bytes());
	assert_eq!(output.stderr, b"");
}

#[test]
fn a_second_argument_is_a_usage_error() {
	assert_usage_error(&run(["--version", "--help"]));
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
	assert_usage_error(&run(["--loud"]));
}

#[test]
fn a_command_that_does_not_exist_yet_is_a_usage_error() {
	assert_usage_error(&run(["build"]));
}

#[test]
fn an_argument_that_is_not_utf8_is_refused_instead_of_panicking() {
	#[cfg(unix)]
	let arg = {
		use std::os::unix::ffi::OsStringExt;
		OsString::from_vec(vec![0xff])
	};
	#[cfg(windows)]
	let arg = {
		use std::os::windows::ffi::OsStringExt;
		// A lone surrogate cannot be converted to UTF-8.
		OsString::from_wide(&[0xd800])
	};

	let output = run([arg]);
	assert_eq!(output.status.code(), Some(2));
	assert_eq!(output.stdout, b"");
	let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
	assert!(
		stderr.starts_with("leafcutter: argument is not valid UTF-8: "),
		"unexpected stderr: {stderr:?}"
	);
}
