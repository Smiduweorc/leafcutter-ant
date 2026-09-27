//! Where leafcutter-ant reads a project and writes its output: TS Dash's
//! `FileSystem` (`src/FileSystem/FileSystem.ts`), as an async trait the host
//! implements, and [`NativeFileSystem`] for a local disk.
//!
//! The futures are boxed and not `Send`. A host drives them on whatever
//! executor it has: the CLI blocks on them, the desktop app can run them on a
//! single-threaded runtime, and a browser host can back them with the async
//! file APIs of a web page, which are single-threaded too. leafcutter-ant
//! picks no executor.

use std::cell::Cell;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::UNIX_EPOCH;

use crate::json::{Indent, Value, parse_json5, stringify};
use crate::pathe;

/// What a [`FileSystem`] method returns.
pub type FsFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, FsError>> + 'a>>;

/// A failed file system operation: the path it was about and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsError {
	path: String,
	message: String,
}

impl FsError {
	/// An error about `path`, with a message for the console.
	pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
		FsError {
			path: path.into(),
			message: message.into(),
		}
	}

	/// The path the operation was about.
	pub fn path(&self) -> &str {
		&self.path
	}

	fn io(path: &str, error: &io::Error) -> Self {
		FsError::new(path, error.to_string())
	}
}

impl fmt::Display for FsError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}: {}", self.path, self.message)
	}
}

impl std::error::Error for FsError {}

/// Whether a directory entry is a directory or anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
	/// A file. The Deno CLI reports a symbolic link as a file too, whatever
	/// it points to.
	File,
	/// A directory.
	Directory,
}

/// One entry of [`FileSystem::readdir`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
	/// The entry's name, without its directory.
	pub name: String,
	/// What it is.
	pub kind: EntryKind,
}

/// A place to read a project from or write its output to. Paths use `/`, and
/// a relative path is relative to wherever the implementation is rooted.
pub trait FileSystem {
	/// The file's bytes.
	fn read_file<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<u8>>;

	/// Replaces the file with `content`, creating the directories it needs.
	fn write_file<'a>(&'a self, path: &'a str, content: &'a [u8]) -> FsFuture<'a, ()>;

	/// Removes a file, or a directory with everything in it. Removing a path
	/// that does not exist is an error.
	fn unlink<'a>(&'a self, path: &'a str) -> FsFuture<'a, ()>;

	/// The entries of a directory, in the order the implementation lists
	/// them. Dash processes files in that order, so it decides the order of
	/// every list Dash writes.
	fn readdir<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<DirEntry>>;

	/// Creates a directory and its missing parents.
	fn mkdir<'a>(&'a self, path: &'a str) -> FsFuture<'a, ()>;

	/// The file's modification time in milliseconds since the Unix epoch.
	fn last_modified<'a>(&'a self, path: &'a str) -> FsFuture<'a, f64>;
}

/// `FileSystem.allFiles`: every file under `path`, depth first, each
/// directory's entries in [`FileSystem::readdir`] order, joined to `path`
/// with pathe 2.0.2's `join`.
pub(crate) async fn all_files(fs: &dyn FileSystem, path: &str) -> Result<Vec<String>, FsError> {
	let mut files = Vec::new();
	// Directories being walked: each one's path and its entries still to
	// visit, reversed so the next one pops off the end.
	let mut open = vec![(path.to_owned(), reversed(fs.readdir(path).await?))];
	while let Some((dir, entries)) = open.last_mut() {
		let Some(entry) = entries.pop() else {
			open.pop();
			continue;
		};
		let child = pathe::join(&[dir, &entry.name]);
		match entry.kind {
			EntryKind::File => files.push(child),
			EntryKind::Directory => {
				let entries = reversed(fs.readdir(&child).await?);
				open.push((child, entries));
			}
		}
	}
	Ok(files)
}

fn reversed(mut entries: Vec<DirEntry>) -> Vec<DirEntry> {
	entries.reverse();
	entries
}

/// `FileSystem.copyFile(from, to, outputFs)`.
pub(crate) async fn copy_file(
	from_fs: &dyn FileSystem,
	from: &str,
	to_fs: &dyn FileSystem,
	to: &str,
) -> Result<(), FsError> {
	let content = from_fs.read_file(from).await?;
	to_fs.write_file(to, &content).await
}

/// `Blob.text()`: UTF-8 with a leading byte order mark dropped and invalid
/// sequences replaced by U+FFFD, as the WHATWG decoder does.
pub(crate) fn text(bytes: &[u8]) -> String {
	let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
	String::from_utf8_lossy(bytes).into_owned()
}

/// `FileSystem.readJson`: the file read as json5, or `Invalid JSON: <path>`
/// when it does not parse. A file that cannot be read fails with the read
/// error.
pub(crate) async fn read_json(fs: &dyn FileSystem, path: &str) -> Result<Value, FsError> {
	let bytes = fs.read_file(path).await?;
	parse_json5(&text(&bytes)).map_err(|_| FsError::new(path, format!("Invalid JSON: {path}")))
}

/// `FileSystem.writeJson`: `JSON.stringify(value, null, "\t")`.
pub(crate) async fn write_json(
	fs: &dyn FileSystem,
	path: &str,
	value: &Value,
) -> Result<(), FsError> {
	fs.write_file(path, stringify(value, Indent::Tab).as_bytes())
		.await
}

/// The local disk, like the Deno CLI's `DenoFileSystem`: relative paths are
/// taken from a base directory, or from the working directory when there is
/// none, and absolute paths are used as they are.
///
/// It lists directories sorted by name (byte order of the UTF-8 name), where
/// `DenoFileSystem` gives whatever order the operating system returns. Writes
/// go to a temporary file in the same directory, which is then renamed over
/// the target, so a build that stops halfway leaves no truncated file. The
/// operations block the thread that polls them.
pub struct NativeFileSystem {
	base: Option<PathBuf>,
	temporaries: Cell<u64>,
}

impl NativeFileSystem {
	/// A file system rooted at the working directory.
	pub fn new() -> Self {
		NativeFileSystem {
			base: None,
			temporaries: Cell::new(0),
		}
	}

	/// A file system rooted at `base`, as `new DenoFileSystem(base)` is.
	pub fn with_base(base: impl Into<PathBuf>) -> Self {
		NativeFileSystem {
			base: Some(base.into()),
			temporaries: Cell::new(0),
		}
	}

	fn resolve(&self, path: &str) -> PathBuf {
		match &self.base {
			Some(base) if !Path::new(path).is_absolute() => base.join(path),
			_ => PathBuf::from(path),
		}
	}

	fn write(&self, path: &str, content: &[u8]) -> Result<(), FsError> {
		let target = self.resolve(path);
		let dir = target
			.parent()
			.filter(|dir| !dir.as_os_str().is_empty())
			.unwrap_or(Path::new("."));
		std::fs::create_dir_all(dir).map_err(|e| FsError::io(path, &e))?;
		let name = target
			.file_name()
			.map(|name| name.to_string_lossy().into_owned())
			.unwrap_or_default();
		let n = self.temporaries.get();
		self.temporaries.set(n + 1);
		let temporary = dir.join(format!(".{name}.{}-{n}.leafcutter-tmp", std::process::id()));
		let written =
			std::fs::write(&temporary, content).and_then(|()| std::fs::rename(&temporary, &target));
		if let Err(error) = written {
			// The temporary may or may not exist by now; the write error is
			// the one worth reporting.
			let _ = std::fs::remove_file(&temporary);
			return Err(FsError::io(path, &error));
		}
		Ok(())
	}
}

impl Default for NativeFileSystem {
	fn default() -> Self {
		Self::new()
	}
}

impl FileSystem for NativeFileSystem {
	fn read_file<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<u8>> {
		Box::pin(
			async move { std::fs::read(self.resolve(path)).map_err(|e| FsError::io(path, &e)) },
		)
	}

	fn write_file<'a>(&'a self, path: &'a str, content: &'a [u8]) -> FsFuture<'a, ()> {
		Box::pin(async move { self.write(path, content) })
	}

	fn unlink<'a>(&'a self, path: &'a str) -> FsFuture<'a, ()> {
		Box::pin(async move {
			let target = self.resolve(path);
			let metadata = std::fs::symlink_metadata(&target).map_err(|e| FsError::io(path, &e))?;
			let removed = if metadata.is_dir() {
				std::fs::remove_dir_all(&target)
			} else {
				std::fs::remove_file(&target)
			};
			removed.map_err(|e| FsError::io(path, &e))
		})
	}

	fn readdir<'a>(&'a self, path: &'a str) -> FsFuture<'a, Vec<DirEntry>> {
		Box::pin(async move {
			let mut entries = Vec::new();
			for entry in std::fs::read_dir(self.resolve(path)).map_err(|e| FsError::io(path, &e))? {
				let entry = entry.map_err(|e| FsError::io(path, &e))?;
				let is_dir = entry
					.file_type()
					.map_err(|e| FsError::io(path, &e))?
					.is_dir();
				entries.push((
					entry.file_name(),
					DirEntry {
						name: entry.file_name().to_string_lossy().into_owned(),
						kind: if is_dir {
							EntryKind::Directory
						} else {
							EntryKind::File
						},
					},
				));
			}
			entries.sort_by(|(a, _), (b, _)| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));
			Ok(entries.into_iter().map(|(_, entry)| entry).collect())
		})
	}

	fn mkdir<'a>(&'a self, path: &'a str) -> FsFuture<'a, ()> {
		Box::pin(async move {
			std::fs::create_dir_all(self.resolve(path)).map_err(|e| FsError::io(path, &e))
		})
	}

	fn last_modified<'a>(&'a self, path: &'a str) -> FsFuture<'a, f64> {
		Box::pin(async move {
			let metadata =
				std::fs::metadata(self.resolve(path)).map_err(|e| FsError::io(path, &e))?;
			let millis = metadata
				.modified()
				.ok()
				.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
				.map_or(0.0, |duration| duration.as_millis() as f64);
			Ok(millis)
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use futures_executor::block_on;

	/// A fresh directory under the system temporary directory, removed when
	/// dropped.
	struct Scratch(PathBuf);

	impl Scratch {
		fn new(name: &str) -> Self {
			let path =
				std::env::temp_dir().join(format!("leafcutter-fs-{name}-{}", std::process::id()));
			let _ = std::fs::remove_dir_all(&path);
			std::fs::create_dir_all(&path).expect("the scratch directory can be created");
			Scratch(path)
		}

		fn fs(&self) -> NativeFileSystem {
			NativeFileSystem::with_base(&self.0)
		}
	}

	impl Drop for Scratch {
		fn drop(&mut self) {
			let _ = std::fs::remove_dir_all(&self.0);
		}
	}

	#[test]
	fn a_write_creates_the_directories_and_leaves_no_temporary_file() {
		let scratch = Scratch::new("write");
		let fs = scratch.fs();
		block_on(fs.write_file("a/b/c.txt", b"one")).expect("the write succeeds");
		block_on(fs.write_file("a/b/c.txt", b"two")).expect("the rewrite succeeds");
		assert_eq!(block_on(fs.read_file("a/b/c.txt")), Ok(b"two".to_vec()));
		let names: Vec<String> = block_on(fs.readdir("a/b"))
			.expect("listed")
			.into_iter()
			.map(|e| e.name)
			.collect();
		assert_eq!(names, ["c.txt"]);
	}

	#[test]
	fn directories_are_listed_sorted_by_name_with_their_kind() {
		let scratch = Scratch::new("readdir");
		let fs = scratch.fs();
		for path in ["b.json", "a/x", "B.json", "\u{e9}.json", "_"] {
			block_on(fs.write_file(path, b"")).expect("written");
		}
		let entries = block_on(fs.readdir("")).expect("listed");
		let listed: Vec<(&str, EntryKind)> =
			entries.iter().map(|e| (e.name.as_str(), e.kind)).collect();
		assert_eq!(
			listed,
			[
				("B.json", EntryKind::File),
				("_", EntryKind::File),
				("a", EntryKind::Directory),
				("b.json", EntryKind::File),
				("\u{e9}.json", EntryKind::File),
			]
		);
	}

	#[test]
	fn all_files_walks_depth_first_in_listing_order() {
		let scratch = Scratch::new("all-files");
		let fs = scratch.fs();
		for path in [
			"BP/b.json",
			"BP/a/z.json",
			"BP/a/y/x.json",
			"BP/c/w.json",
			"RP/r.json",
		] {
			block_on(fs.write_file(path, b"{}")).expect("written");
		}
		assert_eq!(
			block_on(all_files(&fs, "./BP")),
			Ok(vec![
				"BP/a/y/x.json".to_owned(),
				"BP/a/z.json".to_owned(),
				"BP/b.json".to_owned(),
				"BP/c/w.json".to_owned(),
			])
		);
		assert!(block_on(all_files(&fs, "missing")).is_err());
	}

	#[test]
	fn unlink_removes_files_and_whole_directories_and_refuses_what_is_missing() {
		let scratch = Scratch::new("unlink");
		let fs = scratch.fs();
		block_on(fs.write_file("d/e/f.txt", b"x")).expect("written");
		block_on(fs.write_file("g.txt", b"x")).expect("written");
		block_on(fs.unlink("d")).expect("the directory goes");
		block_on(fs.unlink("g.txt")).expect("the file goes");
		assert!(block_on(fs.readdir("")).expect("listed").is_empty());
		let error = block_on(fs.unlink("d")).expect_err("nothing is left to remove");
		assert_eq!(error.path(), "d");
	}

	#[test]
	fn json_is_read_as_json5_and_written_tab_indented() {
		let scratch = Scratch::new("json");
		let fs = scratch.fs();
		block_on(fs.write_file("in.json", b"\xef\xbb\xbf{b: 1, '0': [true], // note\n}"))
			.expect("written");
		let value = block_on(read_json(&fs, "in.json")).expect("json5 reads it");
		block_on(write_json(&fs, "out.json", &value)).expect("written");
		assert_eq!(
			block_on(fs.read_file("out.json")),
			Ok(b"{\n\t\"0\": [\n\t\ttrue\n\t],\n\t\"b\": 1\n}".to_vec())
		);
		block_on(fs.write_file("bad.json", b"{a:")).expect("written");
		let error = block_on(read_json(&fs, "bad.json")).expect_err("not json5");
		assert_eq!(error.to_string(), "bad.json: Invalid JSON: bad.json");
		assert!(block_on(read_json(&fs, "missing.json")).is_err());
	}

	#[test]
	fn copying_between_file_systems_keeps_the_bytes() {
		let input = Scratch::new("copy-in");
		let output = Scratch::new("copy-out");
		block_on(input.fs().write_file("x.bin", &[0, 159, 146, 150])).expect("written");
		block_on(copy_file(&input.fs(), "x.bin", &output.fs(), "deep/x.bin")).expect("copied");
		assert_eq!(
			block_on(output.fs().read_file("deep/x.bin")),
			Ok(vec![0, 159, 146, 150])
		);
	}

	#[test]
	fn text_drops_one_byte_order_mark_and_replaces_invalid_bytes() {
		assert_eq!(text(b"\xef\xbb\xbfa"), "a");
		assert_eq!(text(b"\xef\xbb\xbf\xef\xbb\xbfa"), "\u{feff}a");
		assert_eq!(text(b"a\xffb\xe2\x82"), "a\u{fffd}b\u{fffd}");
	}

	#[test]
	fn an_absolute_path_ignores_the_base() {
		let scratch = Scratch::new("absolute");
		let other = Scratch::new("absolute-other");
		let path = other.0.join("x.txt");
		let path = path.to_str().expect("the scratch path is UTF-8");
		block_on(scratch.fs().write_file(path, b"x")).expect("written");
		assert_eq!(std::fs::read(other.0.join("x.txt")).expect("read"), b"x");
		assert!(block_on(scratch.fs().last_modified(path)).expect("stat") > 0.0);
	}
}
