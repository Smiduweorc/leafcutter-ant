//! Runs the built `app` binary, which is what a user gets, rather than calling
//! functions in `main.rs`.

use std::ffi::OsString;
use std::process::{Command, Output};

fn run<I, S>(args: I) -> Output
where
	I: IntoIterator<Item = S>,
	S: Into<OsString>,
{
	Command::new(env!("CARGO_BIN_EXE_app"))
		.args(args.into_iter().map(Into::into))
		.output()
		.expect("the app binary starts")
}

fn assert_usage_error(output: &Output) {
	assert_eq!(output.status.code(), Some(2));
	assert_eq!(output.stdout, b"");
	assert_eq!(output.stderr, b"usage: app [NAME | --version]\n");
}

#[test]
fn without_a_name_it_greets_the_world() {
	let output = run::<[&str; 0], _>([]);
	assert!(output.status.success());
	assert_eq!(output.stdout, b"hello, world\n");
	assert_eq!(output.stderr, b"");
}

#[test]
fn with_a_name_it_greets_that_name() {
	let output = run(["ants"]);
	assert!(output.status.success());
	assert_eq!(output.stdout, b"hello, ants\n");
}

#[test]
fn the_version_flag_prints_the_workspace_version() {
	let output = run(["--version"]);
	assert!(output.status.success());
	let expected = format!("app {}\n", env!("CARGO_PKG_VERSION"));
	assert_eq!(output.stdout, expected.as_bytes());
}

#[test]
fn a_second_argument_is_a_usage_error() {
	assert_usage_error(&run(["ants", "bees"]));
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
	assert_usage_error(&run(["--loud"]));
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
		stderr.starts_with("app: argument is not valid UTF-8: "),
		"unexpected stderr: {stderr:?}"
	);
}
