//! Builds every project under tests/corpus the four ways
//! tools/parity/corpus.mjs builds it with TS Dash 0.13.0, and compares what
//! each build wrote, removed and output with `<project>.expected.json`.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;

use futures_executor::block_on;
use leafcutter_ant::console::Console;
use leafcutter_ant::fs::{FileSystem, NativeFileSystem};
use leafcutter_ant::json::parse_json5;
use leafcutter_ant::project::{DetectMatcher, FileTypes, PackTypes};
use leafcutter_ant::{Dash, DashOptions, HttpsImports, Mode, ScriptTimeLimit};

struct Quiet;

impl Console for Quiet {
	fn log(&self, _: &str) {}
	fn info(&self, _: &str) {}
	fn warn(&self, _: &str) {}
	fn error(&self, _: &str) {}
}

fn manifest_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn definitions(name: &str) -> leafcutter_ant::json::Value {
	let text = std::fs::read_to_string(manifest_dir().join("tests/data").join(name))
		.expect("definitions are readable");
	parse_json5(&text).expect("definitions are JSON")
}

/// The data the custom commands plugin asks for, vendored from the
/// editor-packages commit the definitions come from; corpus.mjs answers the
/// same way.
fn request_json_data(
	path: &str,
) -> Pin<Box<dyn Future<Output = Result<leafcutter_ant::json::Value, String>>>> {
	let result = match path {
		"data/packages/minecraftBedrock/location/validCommand.json" => {
			Ok(definitions("validCommand.json"))
		}
		other => Err(format!("Error: no data at {other}")),
	};
	Box::pin(std::future::ready(result))
}

/// Scripts' `https://` imports: corpus.json lists what each URL serves, and
/// any other URL fails, as corpus.mjs's `fetch` does.
fn https_imports(served: &serde_json::Value) -> HttpsImports {
	let served = served.clone();
	HttpsImports::Fetch(Rc::new(move |url: &str| {
		let body = match served.get(url).and_then(serde_json::Value::as_str) {
			Some(body) => Ok(body.as_bytes().to_vec()),
			None => Err("TypeError: fetch failed".to_owned()),
		};
		Box::pin(std::future::ready(body))
	}))
}

/// Every file under `dir`, by path relative to it with `/` separators.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
	let mut files = BTreeMap::new();
	let mut pending = vec![PathBuf::new()];
	while let Some(sub) = pending.pop() {
		for entry in std::fs::read_dir(dir.join(&sub)).expect("the directory is readable") {
			let entry = entry.expect("the entry is readable");
			let rel = sub.join(entry.file_name());
			if entry.file_type().expect("the entry has a type").is_dir() {
				pending.push(rel);
			} else {
				let key = rel.to_string_lossy().replace('\\', "/");
				files.insert(
					key,
					std::fs::read(dir.join(&rel)).expect("the file is readable"),
				);
			}
		}
	}
	files
}

fn copy_dir(from: &Path, to: &Path) {
	for (rel, bytes) in snapshot(from) {
		let target = to.join(&rel);
		std::fs::create_dir_all(target.parent().expect("a file has a parent")).expect("created");
		std::fs::write(target, bytes).expect("written");
	}
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
	let mut out = String::new();
	for chunk in bytes.chunks(3) {
		let n = chunk
			.iter()
			.enumerate()
			.fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
		for i in 0..4 {
			if i <= chunk.len() {
				out.push(char::from(BASE64[(n >> (18 - 6 * i) & 63) as usize]));
			} else {
				out.push('=');
			}
		}
	}
	out
}

/// The file as corpus.mjs records it: the text, or base64 of what is not
/// UTF-8.
fn encode(bytes: &[u8]) -> serde_json::Value {
	match std::str::from_utf8(bytes) {
		Ok(text) => serde_json::Value::String(text.to_owned()),
		Err(_) => serde_json::json!({ "base64": base64(bytes) }),
	}
}

fn build(project: &Path, mode: Mode, separate_output: bool) -> serde_json::Value {
	let work = std::env::temp_dir().join(format!(
		"leafcutter-corpus-{}-{}-{:?}-{separate_output}",
		std::process::id(),
		project.file_name().expect("a name").to_string_lossy(),
		mode
	));
	let _ = std::fs::remove_dir_all(&work);
	let root = work.join("project");
	let out = work.join("out");
	copy_dir(project, &root);
	std::fs::create_dir_all(&out).expect("created");
	let before = snapshot(&root);

	let settings: serde_json::Value = std::fs::read_to_string(project.join("corpus.json"))
		.map(|text| serde_json::from_str(&text).expect("corpus.json is JSON"))
		.unwrap_or(serde_json::Value::Null);
	let fs: Rc<dyn FileSystem> = Rc::new(NativeFileSystem::with_base(&root));
	let output: Option<Rc<dyn FileSystem>> =
		separate_output.then(|| Rc::new(NativeFileSystem::with_base(&out)) as _);
	let config = if root.join("dash-config.json").exists() {
		"./dash-config.json"
	} else {
		"./config.json"
	};
	let options = DashOptions {
		config: config.to_owned(),
		compiler_config: settings["compilerConfig"].as_str().map(str::to_owned),
		mode,
		console: Rc::new(Quiet),
		verbose: false,
		pack_types: PackTypes::new(definitions("packDefinitions.json")).expect("pack definitions"),
		file_types: FileTypes::new(definitions("fileDefinitions.json"), DetectMatcher::Glob)
			.expect("file definitions"),
		request_json_data: Rc::new(request_json_data),
		https_imports: https_imports(&settings["https"]),
		script_time_limit: ScriptTimeLimit::Unlimited,
	};
	let dash = Dash::new(fs, output, options);
	block_on(dash.setup()).expect("setup succeeds");
	block_on(dash.build()).expect("the build succeeds");

	let after = snapshot(&root);
	let written: serde_json::Map<String, serde_json::Value> = after
		.iter()
		.filter(|(path, bytes)| before.get(*path) != Some(bytes))
		.map(|(path, bytes)| (path.clone(), encode(bytes)))
		.collect();
	let removed: Vec<serde_json::Value> = before
		.keys()
		.filter(|path| !after.contains_key(*path))
		.map(|path| path.clone().into())
		.collect();
	let output: serde_json::Map<String, serde_json::Value> = snapshot(&out)
		.iter()
		.map(|(path, bytes)| (path.clone(), encode(bytes)))
		.collect();
	std::fs::remove_dir_all(&work).expect("the work directory is removed");
	serde_json::json!({ "written": written, "removed": removed, "output": output })
}

/// Sorts the `removed` list, which corpus.mjs writes in its own walk order.
fn normalized(mut result: serde_json::Value) -> serde_json::Value {
	if let Some(removed) = result["removed"].as_array_mut() {
		removed.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
	}
	result
}

#[test]
fn every_corpus_project_builds_as_ts_dash_builds_it() {
	let corpus = manifest_dir().join("tests/corpus");
	let mut projects: Vec<PathBuf> = std::fs::read_dir(&corpus)
		.expect("the corpus exists")
		.map(|entry| entry.expect("readable").path())
		.filter(|path| path.is_dir())
		.collect();
	projects.sort();
	assert!(!projects.is_empty());
	let mut failures = Vec::new();
	for project in &projects {
		let name = project
			.file_name()
			.expect("a name")
			.to_string_lossy()
			.into_owned();
		let expected_path = corpus.join(format!("{name}.expected.json"));
		let expected: serde_json::Value = serde_json::from_str(
			&std::fs::read_to_string(&expected_path).expect("recorded by corpus.mjs"),
		)
		.expect("JSON");
		for (variant, mode, separate) in [
			("production", Mode::Production, false),
			("development", Mode::Development, false),
			("production-out", Mode::Production, true),
			("development-out", Mode::Development, true),
		] {
			let actual = normalized(build(project, mode, separate));
			let wanted = normalized(expected[variant].clone());
			if actual != wanted {
				failures.push(format!(
					"{name} {variant}:\n  expected {}\n  got      {}",
					serde_json::to_string(&wanted).expect("serializable"),
					serde_json::to_string(&actual).expect("serializable")
				));
			}
		}
	}
	assert!(
		failures.is_empty(),
		"{} of {} builds differ:\n{}",
		failures.len(),
		projects.len() * 4,
		failures.join("\n")
	);
}
