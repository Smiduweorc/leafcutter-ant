//! The `app` command. It turns arguments into a call to the library and prints
//! the result; anything the program does belongs in the library.

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

const USAGE: &str = "usage: app [NAME | --version]";

fn main() -> ExitCode {
	match output_for(std::env::args_os().skip(1).collect()) {
		Ok(line) => write_line(&mut io::stdout().lock(), &line),
		Err(message) => {
			eprintln!("{message}");
			ExitCode::from(2)
		}
	}
}

/// What the command prints for these arguments, or the error to report.
/// `std::env::args` would panic on an argument that is not UTF-8, so the raw
/// arguments come in and are refused here instead.
fn output_for(args: Vec<OsString>) -> Result<String, String> {
	let args = args
		.into_iter()
		.map(OsString::into_string)
		.collect::<Result<Vec<_>, _>>()
		.map_err(|arg| {
			format!(
				"app: argument is not valid UTF-8: {}",
				arg.to_string_lossy()
			)
		})?;
	match args.as_slice() {
		[] => Ok(app::greet("world")),
		[flag] if flag == "--version" => Ok(format!("app {}", env!("CARGO_PKG_VERSION"))),
		[name] if !name.starts_with('-') => Ok(app::greet(name)),
		_ => Err(USAGE.to_owned()),
	}
}

/// Rust ignores SIGPIPE, so a reader that goes away early (`app | head -c0`)
/// shows up as a BrokenPipe error, which `println!` would turn into a panic.
/// The reader has taken what it wanted, so that case exits cleanly.
fn write_line(out: &mut impl Write, line: &str) -> ExitCode {
	match writeln!(out, "{line}").and_then(|()| out.flush()) {
		Ok(()) => ExitCode::SUCCESS,
		Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
		Err(err) => {
			eprintln!("app: cannot write the output: {err}");
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

	#[test]
	fn the_line_is_written_with_a_trailing_newline() {
		let mut out = Vec::new();
		assert_eq!(write_line(&mut out, "hello, world"), ExitCode::SUCCESS);
		assert_eq!(out, b"hello, world\n");
	}

	#[test]
	fn a_reader_that_went_away_is_not_an_error() {
		let mut out = Failing(io::ErrorKind::BrokenPipe);
		assert_eq!(write_line(&mut out, "hello, world"), ExitCode::SUCCESS);
	}

	#[test]
	fn any_other_write_failure_is_reported_as_a_failure() {
		let mut out = Failing(io::ErrorKind::StorageFull);
		assert_eq!(write_line(&mut out, "hello, world"), ExitCode::FAILURE);
	}
}
