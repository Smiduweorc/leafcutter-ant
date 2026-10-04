//! The `leafcutter` command. It turns arguments into calls to the library
//! and prints the result; anything the program does belongs in the library.
//! `leafcutter build` takes the Deno CLI's `build` flags and sets Dash up the
//! way the Deno CLI does (`deno-dash-compiler/src/CLI.ts`).

mod local_cache;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;

use leafcutter_ant::console::Console;
use leafcutter_ant::fs::{FileSystem, NativeFileSystem};
use leafcutter_ant::json::{Indent, Value, parse_json5, stringify};
use leafcutter_ant::project::{DetectMatcher, FileTypes, PackTypes};
use leafcutter_ant::{Dash, DashOptions, HttpsImports, Mode, RequestJsonData, ScriptTimeLimit};

use local_cache::LocalCache;

const USAGE: &str = "usage: leafcutter build [--mode development|production] [--out <dir>] [--compilerConfig <file>] [--noCache]
       leafcutter [--version | --help]";

const HELP: &str = "usage: leafcutter build [--mode development|production] [--out <dir>] [--compilerConfig <file>] [--noCache]
       leafcutter [--version | --help]

leafcutter-ant is a Rust port of the Dash compiler for Minecraft Bedrock
add-ons. It runs the built-in plugins, generator scripts, custom commands
and extension plugins; molang and custom components are not supported yet.

build   Build the project in the working directory. The project config is
        dash-config.json when that file exists, else config.json.

  -m, --mode <mode>             development or production (default production)
  -o, --out <dir>               write the output into <dir>, as a com.mojang
                                folder; `preview` names Minecraft Preview's.
                                In development mode it defaults to
                                Minecraft's com.mojang folder where APPDATA
                                is set (Windows)
  -c, --compilerConfig <file>   take the plugin list from <file>
  -n, --noCache                 empty the ~/.dash cache of fetched data first

  --version   print the version
  -h, --help  print this text";

const PACK_DEFINITIONS: &str = "https://raw.githubusercontent.com/bridge-core/editor-packages/main/packages/minecraftBedrock/packDefinitions.json";
const FILE_DEFINITIONS: &str = "https://raw.githubusercontent.com/bridge-core/editor-packages/main/dist/minecraftBedrock/fileDefinitions.json";

/// What the arguments ask for.
#[derive(Debug, PartialEq, Eq)]
enum Command {
	Help,
	Version,
	Build(BuildArgs),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct BuildArgs {
	mode: Option<String>,
	out: Option<String>,
	compiler_config: Option<String>,
	no_cache: bool,
}

fn main() -> ExitCode {
	let command = match parse(std::env::args_os().skip(1).collect()) {
		Ok(command) => command,
		Err(message) => {
			eprintln!("{message}");
			return ExitCode::from(2);
		}
	};
	match command {
		Command::Help => write_line(&mut io::stdout().lock(), HELP),
		Command::Version => write_line(
			&mut io::stdout().lock(),
			&format!("leafcutter {}", env!("CARGO_PKG_VERSION")),
		),
		Command::Build(args) => match build(args) {
			Ok(()) => ExitCode::SUCCESS,
			Err(message) => {
				eprintln!("leafcutter: {message}");
				ExitCode::FAILURE
			}
		},
	}
}

/// The command these arguments ask for, or the error to report. Flags take
/// their value as the next argument or after `=`, as yargs reads them.
/// `std::env::args` would panic on an argument that is not UTF-8, so the raw
/// arguments come in and are refused here instead.
fn parse(args: Vec<OsString>) -> Result<Command, String> {
	let args = args
		.into_iter()
		.map(OsString::into_string)
		.collect::<Result<Vec<_>, _>>()
		.map_err(|arg| {
			format!(
				"leafcutter: argument is not valid UTF-8: {}",
				arg.to_string_lossy()
			)
		})?;
	match args.first().map(String::as_str) {
		None => return Ok(Command::Help),
		Some("--help" | "-h") if args.len() == 1 => return Ok(Command::Help),
		Some("--version") if args.len() == 1 => return Ok(Command::Version),
		Some("build") => {}
		_ => return Err(USAGE.to_owned()),
	}
	let mut build = BuildArgs::default();
	let mut rest = args[1..].iter();
	while let Some(arg) = rest.next() {
		let (flag, inline) = match arg.split_once('=') {
			Some((flag, value)) if flag.starts_with('-') => (flag, Some(value.to_owned())),
			_ => (arg.as_str(), None),
		};
		let mut value = |name: &str| -> Result<String, String> {
			inline
				.clone()
				.or_else(|| rest.next().cloned())
				.ok_or_else(|| format!("leafcutter: {name} needs a value\n{USAGE}"))
		};
		match flag {
			"-m" | "--mode" => {
				let mode = value("--mode")?;
				if mode != "development" && mode != "production" {
					return Err(format!(
						"leafcutter: --mode must be development or production, not {mode:?}\n{USAGE}"
					));
				}
				build.mode = Some(mode);
			}
			"-o" | "--out" => build.out = Some(value("--out")?),
			"-c" | "--compilerConfig" | "--compiler-config" => {
				build.compiler_config = Some(value("--compilerConfig")?)
			}
			"-n" | "--noCache" if inline.is_none() => build.no_cache = true,
			"-h" | "--help" => return Ok(Command::Help),
			_ => return Err(USAGE.to_owned()),
		}
	}
	Ok(Command::Build(build))
}

/// Prints log and info lines on stdout and warnings and errors on stderr, as
/// Deno's console does.
struct Terminal;

impl Console for Terminal {
	fn log(&self, message: &str) {
		let _ = writeln!(io::stdout().lock(), "{message}");
	}
	fn info(&self, message: &str) {
		let _ = writeln!(io::stdout().lock(), "{message}");
	}
	fn warn(&self, message: &str) {
		eprintln!("{message}");
	}
	fn error(&self, message: &str) {
		eprintln!("{message}");
	}
}

/// Minecraft's com.mojang folder under `APPDATA`, as the Deno CLI finds it.
fn com_mojang_folder(app: &str) -> Option<String> {
	let appdata = std::env::var_os("APPDATA")?;
	let folder = PathBuf::from(appdata)
		.join(app)
		.join("Users/Shared/games/com.mojang");
	Some(folder.to_string_lossy().into_owned())
}

/// The definitions from the cache, or fetched and cached when the cache has
/// none or holds something that does not parse.
fn definitions(cache: &LocalCache, name: &str, url: &str) -> Result<Value, String> {
	if let Some(cached) = cache.get(name)
		&& let Ok(value) = parse_json5(&cached)
	{
		return Ok(value);
	}
	let body = ureq::get(url)
		.call()
		.and_then(|mut response| response.body_mut().read_to_string())
		.map_err(|error| format!("cannot fetch {url}: {error}"))?;
	let value = parse_json5(&body).map_err(|error| format!("{url} is not JSON: {error}"))?;
	cache.save(name, &stringify(&value, Indent::None));
	Ok(value)
}

fn build(args: BuildArgs) -> Result<(), String> {
	let cache = LocalCache::new(
		std::env::var_os("HOME")
			.or_else(|| std::env::var_os("USERPROFILE"))
			.map(PathBuf::from),
	);
	let console = Rc::new(Terminal);
	if let Some(message) = cache.invalidate(args.no_cache) {
		console.log(message);
	}
	let mode = match args.mode.as_deref() {
		Some("development") => Mode::Development,
		_ => Mode::Production,
	};
	// CLI.ts verifyOptions.
	let out = match args.out {
		Some(out) if out == "preview" => com_mojang_folder("Minecraft Bedrock Preview"),
		None if mode == Mode::Development => com_mojang_folder("Minecraft Bedrock"),
		out => out,
	};
	let fs: Rc<dyn FileSystem> = Rc::new(NativeFileSystem::new());
	let output: Option<Rc<dyn FileSystem>> = out
		.filter(|out| !out.is_empty())
		.map(|out| Rc::new(NativeFileSystem::with_base(out)) as _);
	let config = if std::fs::read("dash-config.json").is_ok() {
		"./dash-config.json"
	} else {
		"./config.json"
	};
	let pack_types = PackTypes::new(definitions(
		&cache,
		"packDefinitions.json",
		PACK_DEFINITIONS,
	)?)
	.map_err(|error| format!("the pack definitions: {error}"))?;
	let file_types = FileTypes::new(
		definitions(&cache, "fileDefinitions.json", FILE_DEFINITIONS)?,
		DetectMatcher::Glob,
	)
	.map_err(|error| format!("the file definitions: {error}"))?;
	let cache = Rc::new(cache);
	let dash = Dash::new(
		fs,
		output,
		DashOptions {
			config: config.to_owned(),
			compiler_config: args.compiler_config,
			mode,
			console,
			verbose: true,
			pack_types,
			file_types,
			request_json_data: request_json_data(Rc::clone(&cache)),
			https_imports: HttpsImports::Fetch(Rc::new(|url: &str| {
				let fetched = fetch(url).map(String::into_bytes);
				Box::pin(std::future::ready(fetched))
			})),
			script_time_limit: ScriptTimeLimit::Unlimited,
		},
	);
	futures_executor::block_on(async {
		dash.setup().await?;
		dash.build().await
	})
	.map_err(|error| error.to_string())
}

/// The body at `url`, as text.
fn fetch(url: &str) -> Result<String, String> {
	ureq::get(url)
		.call()
		.and_then(|mut response| response.body_mut().read_to_string())
		.map_err(|error| format!("TypeError: cannot fetch {url}: {error}"))
}

/// The Deno CLI's `requestJsonData`: the cached copy when it parses, else
/// the file from bridge-core/editor-packages, which is then cached.
fn request_json_data(cache: Rc<LocalCache>) -> RequestJsonData {
	Rc::new(move |data_path: &str| {
		let result = (|| {
			if let Some(cached) = cache.get(data_path)
				&& let Ok(value) = parse_json5(&cached)
			{
				return Ok(value);
			}
			let url = data_path.replacen(
				"data/",
				"https://raw.githubusercontent.com/bridge-core/editor-packages/main/",
				1,
			);
			let value = parse_json5(&fetch(&url)?)
				.map_err(|error| format!("SyntaxError: {url} is not JSON: {error}"))?;
			cache.save(data_path, &stringify(&value, Indent::None));
			Ok(value)
		})();
		Box::pin(std::future::ready(result))
	})
}

/// Rust ignores SIGPIPE, so a reader that goes away early (`leafcutter --help | head -c0`)
/// shows up as a BrokenPipe error, which `println!` would turn into a panic.
/// The reader has taken what it wanted, so that case exits cleanly.
fn write_line(out: &mut impl Write, line: &str) -> ExitCode {
	match writeln!(out, "{line}").and_then(|()| out.flush()) {
		Ok(()) => ExitCode::SUCCESS,
		Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
		Err(err) => {
			eprintln!("leafcutter: cannot write the output: {err}");
			ExitCode::FAILURE
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A writer whose every write fails with `kind`.
	struct Failing(io::ErrorKind);

	impl Write for Failing {
		fn write(&mut self, _: &[u8]) -> io::Result<usize> {
			Err(self.0.into())
		}

		fn flush(&mut self) -> io::Result<()> {
			Ok(())
		}
	}

	fn args(list: &[&str]) -> Vec<OsString> {
		list.iter().map(OsString::from).collect()
	}

	#[test]
	fn the_line_is_written_with_a_trailing_newline() {
		let mut out = Vec::new();
		assert_eq!(write_line(&mut out, "line"), ExitCode::SUCCESS);
		assert_eq!(out, b"line\n");
	}

	#[test]
	fn a_reader_that_went_away_is_not_an_error() {
		let mut out = Failing(io::ErrorKind::BrokenPipe);
		assert_eq!(write_line(&mut out, "line"), ExitCode::SUCCESS);
	}

	#[test]
	fn any_other_write_failure_is_reported_as_a_failure() {
		let mut out = Failing(io::ErrorKind::StorageFull);
		assert_eq!(write_line(&mut out, "line"), ExitCode::FAILURE);
	}

	#[test]
	fn build_flags_take_values_after_a_space_or_an_equals_sign() {
		assert_eq!(
			parse(args(&[
				"build",
				"-m",
				"development",
				"--out=dist dir",
				"-c",
				"c.json",
				"-n"
			])),
			Ok(Command::Build(BuildArgs {
				mode: Some("development".to_owned()),
				out: Some("dist dir".to_owned()),
				compiler_config: Some("c.json".to_owned()),
				no_cache: true,
			}))
		);
		assert_eq!(
			parse(args(&[
				"build",
				"--mode=production",
				"--compiler-config",
				"x",
				"--noCache"
			])),
			Ok(Command::Build(BuildArgs {
				mode: Some("production".to_owned()),
				out: None,
				compiler_config: Some("x".to_owned()),
				no_cache: true,
			}))
		);
		assert_eq!(
			parse(args(&["build"])),
			Ok(Command::Build(BuildArgs::default()))
		);
	}

	#[test]
	fn a_missing_value_an_unknown_mode_or_an_unknown_flag_is_refused() {
		assert!(parse(args(&["build", "--out"])).is_err());
		assert!(parse(args(&["build", "--mode", "debug"])).is_err());
		assert!(parse(args(&["build", "--loud"])).is_err());
		assert!(parse(args(&["watch"])).is_err());
	}
}
