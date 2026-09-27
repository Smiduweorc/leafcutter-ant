//! Runs the built `leafcutter` binary, which is what a user gets, rather
//! than calling functions in `main.rs`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const USAGE: &[u8] = b"usage: leafcutter build [--mode development|production] [--out <dir>] [--compilerConfig <file>] [--noCache]
       leafcutter [--version | --help]\n";

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
	assert_eq!(output.stderr, USAGE);
}

fn assert_help(output: &Output) {
	assert!(output.status.success());
	assert_eq!(output.stderr, b"");
	let stdout = String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8");
	assert!(stdout.starts_with("usage: leafcutter build "), "{stdout:?}");
	assert!(stdout.contains("-n, --noCache"), "{stdout:?}");
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
	assert_help(&run(["build", "--help"]));
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
fn an_unknown_flag_or_command_is_a_usage_error() {
	assert_usage_error(&run(["--loud"]));
	assert_usage_error(&run(["watch"]));
	assert_usage_error(&run(["build", "--loud"]));
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

/// A project and a home directory whose `~/.dash` already holds the
/// definitions, fresh, so a build needs no network.
struct Workspace(PathBuf);

impl Workspace {
	fn new(name: &str, config: &str) -> Self {
		let root =
			std::env::temp_dir().join(format!("leafcutter-cli-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../leafcutter-ant/tests/data");
		let dash = root.join("home/.dash");
		std::fs::create_dir_all(&dash).expect("created");
		for name in ["packDefinitions.json", "fileDefinitions.json"] {
			std::fs::copy(data.join(name), dash.join(name)).expect("copied");
		}
		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.expect("after 1970");
		std::fs::write(dash.join(".timestamp"), now.as_millis().to_string()).expect("written");
		let project = root.join("project");
		std::fs::create_dir_all(project.join("BP/entities")).expect("created");
		std::fs::write(project.join("config.json"), config).expect("written");
		std::fs::write(project.join("BP/entities/a.json"), "{}").expect("written");
		Workspace(root)
	}

	fn build(&self, args: &[&str]) -> Output {
		Command::new(env!("CARGO_BIN_EXE_leafcutter"))
			.arg("build")
			.args(args)
			.current_dir(self.0.join("project"))
			.env("HOME", self.0.join("home"))
			.env_remove("USERPROFILE")
			.env_remove("APPDATA")
			.stdin(Stdio::null())
			.output()
			.expect("the leafcutter binary starts")
	}

	fn project(&self, path: &str) -> PathBuf {
		self.0.join("project").join(path)
	}
}

impl Drop for Workspace {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.0);
	}
}

const CONFIG: &str =
	r#"{"packs": {"behaviorPack": "./BP"}, "compiler": {"plugins": ["simpleRewrite"]}}"#;

#[test]
fn build_writes_a_production_build_by_default() {
	let workspace = Workspace::new("production", CONFIG);
	let output = workspace.build(&[]);
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	assert_eq!(
		std::fs::read(workspace.project("builds/dist/Bridge BP/entities/a.json")).expect("built"),
		b"{}"
	);
	assert!(!workspace.project(".bridge/.dash.production.json").exists());
	let stdout = String::from_utf8(output.stdout).expect("UTF-8");
	assert!(
		stdout.starts_with("Starting compilation...\n"),
		"{stdout:?}"
	);
	assert!(stdout.contains("Dash compiled 1 files in "), "{stdout:?}");
}

#[test]
fn a_development_build_keeps_the_cache_and_out_writes_elsewhere() {
	let workspace = Workspace::new("development", CONFIG);
	let out = workspace.0.join("com.mojang");
	let output = workspace.build(&[
		"--mode",
		"development",
		"--out",
		out.to_str().expect("UTF-8"),
	]);
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	assert_eq!(
		std::fs::read(out.join("development_behavior_packs/Bridge BP/entities/a.json"))
			.expect("built"),
		b"{}"
	);
	assert!(workspace.project(".bridge/.dash.development.json").exists());
}

#[test]
fn dash_config_json_wins_over_config_json() {
	let workspace = Workspace::new("dash-config", r#"{"packs": {"behaviorPack": "./BP"}}"#);
	std::fs::write(workspace.project("dash-config.json"), CONFIG).expect("written");
	let output = workspace.build(&[]);
	assert!(output.status.success());
	assert!(
		workspace
			.project("builds/dist/Bridge BP/entities/a.json")
			.exists()
	);
}

#[test]
fn a_compiler_config_that_cannot_be_read_fails_the_build() {
	let workspace = Workspace::new("compiler-config", CONFIG);
	let output = workspace.build(&["-c", "missing.json"]);
	assert_eq!(output.status.code(), Some(1));
	let stderr = String::from_utf8(output.stderr).expect("UTF-8");
	assert!(
		stderr.starts_with("leafcutter: cannot read the compiler config: missing.json: "),
		"{stderr:?}"
	);
}
