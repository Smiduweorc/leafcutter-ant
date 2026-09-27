//! The Deno CLI's local data cache (`src/LocalCache.ts`): `~/.dash`, where the
//! pack and file definitions fetched from bridge-core/editor-packages are
//! kept, and emptied once it is a day old or when `--noCache` asks.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How old the cache may get before a build empties it: 24 hours, as in the
/// Deno CLI.
const MAX_AGE_MS: f64 = 1000.0 * 60.0 * 60.0 * 24.0;

pub struct LocalCache {
	dir: Option<PathBuf>,
}

fn now_ms() -> f64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0.0, |d| d.as_millis() as f64)
}

/// `parseInt(text)`: leading whitespace, an optional sign, then decimal
/// digits; `None` (NaN) when there are no digits.
fn parse_int(text: &str) -> Option<f64> {
	let text = text.trim_start();
	let (sign, digits) = match text.strip_prefix('-') {
		Some(rest) => (-1.0, rest),
		None => (1.0, text.strip_prefix('+').unwrap_or(text)),
	};
	let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
	if digits.is_empty() {
		return None;
	}
	digits.parse::<f64>().ok().map(|n| sign * n)
}

/// Removes everything inside `dir` and keeps `dir`, as `fs.emptyDir` does.
fn empty_dir(dir: &Path) -> std::io::Result<()> {
	for entry in std::fs::read_dir(dir)? {
		let path = entry?.path();
		if path.is_dir() {
			std::fs::remove_dir_all(&path)?;
		} else {
			std::fs::remove_file(&path)?;
		}
	}
	Ok(())
}

impl LocalCache {
	/// The cache under `home`, which the CLI takes from `HOME` or
	/// `USERPROFILE`; without a home there is no cache and everything is
	/// fetched.
	pub fn new(home: Option<PathBuf>) -> Self {
		let dir = home.map(|home| home.join(".dash"));
		if let Some(dir) = &dir {
			// `fs.ensureDir`; a cache that cannot be created behaves as empty.
			let _ = std::fs::create_dir_all(dir);
		}
		LocalCache { dir }
	}

	/// `tryInvalidateLocalData(force)`: empties the cache when forced or when
	/// its `.timestamp` is more than a day old. A timestamp that is not a
	/// number never expires, as `parseInt` gives NaN and every comparison
	/// with NaN is false. Failures are ignored, as in the Deno CLI.
	pub fn invalidate(&self, force: bool) -> Option<&'static str> {
		let dir = self.dir.as_ref()?;
		if force {
			let _ = empty_dir(dir);
			return Some("Invalidating local cache!");
		}
		let timestamp = dir.join(".timestamp");
		let time = match std::fs::read_to_string(&timestamp) {
			Ok(text) => parse_int(&text),
			Err(_) if !timestamp.exists() => Some(0.0),
			Err(_) => return None,
		};
		match time {
			Some(time) if now_ms() - time > MAX_AGE_MS => {
				let _ = empty_dir(dir);
				Some("Invalidating local cache of remote data!")
			}
			_ => None,
		}
	}

	/// `getLocalData(path)`.
	pub fn get(&self, path: &str) -> Option<String> {
		std::fs::read_to_string(self.dir.as_ref()?.join(path)).ok()
	}

	/// `saveLocalData(path, content)`: writes the file and, when there is
	/// none yet, the `.timestamp` the expiry counts from.
	pub fn save(&self, path: &str, content: &str) {
		let Some(dir) = &self.dir else {
			return;
		};
		let full = dir.join(path);
		if let Some(parent) = full.parent() {
			let _ = std::fs::create_dir_all(parent);
		}
		let _ = std::fs::write(&full, content);
		let timestamp = dir.join(".timestamp");
		if !timestamp.exists() {
			let _ = std::fs::write(timestamp, format!("{}", now_ms() as u64));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	struct Home(PathBuf);

	impl Home {
		fn new(name: &str) -> Self {
			let path = std::env::temp_dir()
				.join(format!("leafcutter-cache-{name}-{}", std::process::id()));
			let _ = std::fs::remove_dir_all(&path);
			Home(path)
		}
	}

	impl Drop for Home {
		fn drop(&mut self) {
			let _ = std::fs::remove_dir_all(&self.0);
		}
	}

	#[test]
	fn parse_int_reads_a_leading_integer_as_javascript_does() {
		assert_eq!(parse_int("123"), Some(123.0));
		assert_eq!(parse_int("  -45xyz"), Some(-45.0));
		assert_eq!(parse_int("+7"), Some(7.0));
		assert_eq!(parse_int("1.9"), Some(1.0));
		assert_eq!(parse_int("abc"), None);
		assert_eq!(parse_int(""), None);
	}

	#[test]
	fn saving_writes_the_file_and_a_timestamp_once() {
		let home = Home::new("save");
		let cache = LocalCache::new(Some(home.0.clone()));
		cache.save("data/x.json", "{}");
		let first = std::fs::read_to_string(home.0.join(".dash/.timestamp")).expect("a timestamp");
		std::fs::write(home.0.join(".dash/.timestamp"), "1").expect("written");
		cache.save("y.json", "[]");
		assert_eq!(cache.get("data/x.json").as_deref(), Some("{}"));
		assert_eq!(
			std::fs::read_to_string(home.0.join(".dash/.timestamp")).expect("read"),
			"1"
		);
		assert!(parse_int(&first).is_some_and(|t| t > 1.0e12));
	}

	#[test]
	fn a_cache_older_than_a_day_is_emptied_and_a_fresh_one_kept() {
		let home = Home::new("expiry");
		let cache = LocalCache::new(Some(home.0.clone()));
		cache.save("a.json", "1");
		assert_eq!(cache.invalidate(false), None);
		assert_eq!(cache.get("a.json").as_deref(), Some("1"));
		let old = now_ms() - MAX_AGE_MS - 1000.0;
		std::fs::write(home.0.join(".dash/.timestamp"), format!("{}", old as u64))
			.expect("written");
		assert_eq!(
			cache.invalidate(false),
			Some("Invalidating local cache of remote data!")
		);
		assert_eq!(cache.get("a.json"), None);
		assert!(home.0.join(".dash").is_dir(), "the directory itself stays");
	}

	#[test]
	fn a_timestamp_that_is_not_a_number_never_expires() {
		let home = Home::new("nan");
		let cache = LocalCache::new(Some(home.0.clone()));
		cache.save("a.json", "1");
		std::fs::write(home.0.join(".dash/.timestamp"), "yesterday").expect("written");
		assert_eq!(cache.invalidate(false), None);
		assert_eq!(cache.get("a.json").as_deref(), Some("1"));
	}

	#[test]
	fn no_cache_empties_it_whatever_its_age() {
		let home = Home::new("force");
		let cache = LocalCache::new(Some(home.0.clone()));
		cache.save("a.json", "1");
		assert_eq!(cache.invalidate(true), Some("Invalidating local cache!"));
		assert_eq!(cache.get("a.json"), None);
	}

	#[test]
	fn without_a_home_nothing_is_cached() {
		let cache = LocalCache::new(None);
		cache.save("a.json", "1");
		assert_eq!(cache.get("a.json"), None);
		assert_eq!(cache.invalidate(true), None);
	}
}
