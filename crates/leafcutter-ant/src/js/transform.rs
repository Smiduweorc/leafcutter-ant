//! What js-runtime 0.4.5 does to a module's source before it runs it
//! (`Runtime.transformSource` and `Transform/main.ts`): swc compiles it to
//! ES2020, the minifier prints it again, and the printed code is parsed once
//! more so that every top-level `import` and `export` can be rewritten into
//! `await ___require(...)` and `___module.<name> = ...`, the form the loader
//! runs inside an async function.
//!
//! The rewrite is a port of the JavaScript, faults included, because the
//! loader then runs whatever it produced:
//!
//! - Positions are counted from the first statement, not from the start of
//!   the code, so a comment the minifier keeps in front of it (`/*! ... */`)
//!   shifts every edit and the result is broken code.
//! - `export { a }` without `as` throws (`exported` is null in swc's tree),
//!   as does `export * as ns from ...`.
//! - `export * from ...` is left as it is, which does not parse inside a
//!   function.
//! - A destructuring export (`export const { a } = ...`) is exported as
//!   `___module.undefined = undefined`.
//! - `import d, * as ns from ...` binds `d` and leaves `ns` undeclared.
//! - A module with no statements throws, as `body[0].span` does.

use swc_common::Spanned;
use swc_ecma_ast::{
	Decl, DefaultDecl, EsVersion, ImportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Pat,
};
use swc_ecma_parser::{EsSyntax, Syntax, TsSyntax};

use super::magic_string::MagicString;
use crate::wasm_web;

/// `transformSource(filePath, fileContent)`: `path` decides the syntax (a
/// path ending in `.js` is JavaScript, anything else TypeScript) and
/// `basename` is path-browserify's `basename(path)`, which swc gets as the
/// file name. An error is the message the loader would reject with.
#[cfg_attr(
	not(test),
	expect(
		dead_code,
		reason = "the script loader, which comes with the engine, calls it"
	)
)]
pub(crate) fn transform_source(path: &str, basename: &str, source: &str) -> Result<String, String> {
	let is_js = path.ends_with(".js");
	let syntax_name = if is_js { "ecmascript" } else { "typescript" };
	let compiled = wasm_web::transform(
		source,
		serde_json::json!({
			"filename": basename,
			"jsc": {
				"parser": { "syntax": syntax_name, "topLevelAwait": true },
				"preserveAllComments": false,
				"target": "es2020"
			},
			"isModule": true
		}),
	)?;
	let minified = wasm_web::minify(
		&compiled,
		serde_json::json!({
			"compress": false,
			"mangle": false,
			"format": { "beautify": true },
			"module": true
		}),
	)?;
	let syntax = if is_js {
		Syntax::Es(EsSyntax::default())
	} else {
		Syntax::Typescript(TsSyntax::default())
	};
	let edits = wasm_web::parse_module(&minified, syntax, EsVersion::Es2022, |module, _| {
		plan(&module.body)
	})??;
	apply(&minified, edits)
}

/// One step of the rewrite, with positions as swc reports them.
enum Edit {
	/// `overwrite(start, end, text)`, where `text` may embed `slice`s of the
	/// code, taken before the overwrite.
	Overwrite {
		start: u32,
		end: u32,
		parts: Vec<Part>,
	},
}

enum Part {
	Text(String),
	Slice(u32, u32),
}

/// Walks the module's statements as `transform()` walks `body`, and lists
/// the edits with the exports to append. The offset every position is taken
/// relative to is the first statement's start.
fn plan(body: &[ModuleItem]) -> Result<(u32, Vec<Edit>, String), String> {
	let Some(first) = body.first() else {
		return Err("TypeError: Cannot read properties of undefined (reading 'span')".to_owned());
	};
	let offset = first.span().lo.0;
	let mut edits = Vec::new();
	let mut append = String::new();
	let overwrite = |start: u32, end: u32, parts: Vec<Part>| Edit::Overwrite { start, end, parts };
	for item in body {
		let ModuleItem::ModuleDecl(decl) = item else {
			continue;
		};
		match decl {
			ModuleDecl::ExportDefaultDecl(node) => {
				let inner = match &node.decl {
					DefaultDecl::Class(class) => class.class.span,
					DefaultDecl::Fn(function) => function.function.span,
					DefaultDecl::TsInterfaceDecl(interface) => interface.span,
				};
				edits.push(overwrite(
					node.span.lo.0,
					node.span.hi.0,
					vec![
						Part::Text("\n___module.__default__ = ".to_owned()),
						Part::Slice(inner.lo.0, inner.hi.0),
						Part::Text(";".to_owned()),
					],
				));
			}
			ModuleDecl::ExportDefaultExpr(node) => {
				let inner = node.expr.span();
				edits.push(overwrite(
					node.span.lo.0,
					node.span.hi.0,
					vec![
						Part::Text("\n___module.__default__ = ".to_owned()),
						Part::Slice(inner.lo.0, inner.hi.0),
						Part::Text(";".to_owned()),
					],
				));
			}
			ModuleDecl::ExportDecl(node) => {
				let inner = node.decl.span();
				edits.push(overwrite(
					node.span.lo.0,
					node.span.hi.0,
					vec![Part::Slice(inner.lo.0, inner.hi.0)],
				));
				match &node.decl {
					Decl::Class(class) => {
						let name = &class.ident.sym;
						append.push_str(&format!("\n___module.{name} = {name};"));
					}
					Decl::Fn(function) => {
						let name = &function.ident.sym;
						append.push_str(&format!("\n___module.{name} = {name};"));
					}
					Decl::Var(var) => {
						for declarator in &var.decls {
							// `decl.id.value`: only a plain binding has a
							// `value`; a pattern gives undefined.
							let name = match &declarator.name {
								Pat::Ident(binding) => binding.id.sym.to_string(),
								_ => "undefined".to_owned(),
							};
							append.push_str(&format!("\n___module.{name} = {name};"));
						}
					}
					_ => {}
				}
			}
			ModuleDecl::ExportNamed(node) => {
				let mut text = String::new();
				for specifier in &node.specifiers {
					let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else {
						// A namespace or default specifier has no `exported`
						// in the destructuring, so reading `.value` throws.
						return Err(
							"TypeError: Cannot read properties of undefined (reading 'value')"
								.to_owned(),
						);
					};
					let Some(exported) = &named.exported else {
						return Err(
							"TypeError: Cannot read properties of null (reading 'value')"
								.to_owned(),
						);
					};
					let orig = export_name(&named.orig);
					match export_name(exported) {
						"default" => text.push_str(&format!("\n___module.__default__ = {orig};")),
						name => text.push_str(&format!("\n___module.{name} = {orig};")),
					}
				}
				edits.push(overwrite(
					node.span.lo.0,
					node.span.hi.0,
					vec![Part::Text(text)],
				));
			}
			ModuleDecl::Import(node) => {
				let raw = node.src.raw.as_deref().unwrap_or("null");
				let text = match node.specifiers.as_slice() {
					[ImportSpecifier::Namespace(namespace)] => {
						format!("\nconst {} = await ___require({raw});", namespace.local.sym)
					}
					specifiers => {
						let mut imports = Vec::new();
						for specifier in specifiers {
							match specifier {
								ImportSpecifier::Default(default) => {
									imports.push(format!("__default__: {}", default.local.sym));
								}
								ImportSpecifier::Named(named) => {
									let imported = match &named.imported {
										Some(imported) => export_name(imported),
										None => &named.local.sym,
									};
									imports.push(format!("{imported}: {}", named.local.sym));
								}
								ImportSpecifier::Namespace(_) => {}
							}
						}
						if imports.is_empty() {
							format!("\nawait ___require({raw});")
						} else {
							format!(
								"\nconst {{{}}} = await ___require({raw});",
								imports.join(", ")
							)
						}
					}
				};
				edits.push(overwrite(
					node.span.lo.0,
					node.span.hi.0,
					vec![Part::Text(text)],
				));
			}
			_ => {}
		}
	}
	Ok((offset, edits, append))
}

/// `.value` of an identifier or a string literal.
fn export_name(name: &ModuleExportName) -> &str {
	match name {
		ModuleExportName::Ident(ident) => &ident.sym,
		ModuleExportName::Str(string) => &string.value,
	}
}

/// Runs the edits through magic-string, over the code's UTF-16 units, with
/// every position less the offset, and appends the exports.
fn apply(code: &str, (offset, edits, append): (u32, Vec<Edit>, String)) -> Result<String, String> {
	let mut output = MagicString::new(code.encode_utf16().collect());
	// Positions before the first statement cannot occur: every node starts
	// at or after it.
	let at = |position: u32| (position - offset) as usize;
	for edit in edits {
		let Edit::Overwrite { start, end, parts } = edit;
		let mut text: Vec<u16> = Vec::new();
		for part in parts {
			match part {
				Part::Text(s) => text.extend(s.encode_utf16()),
				Part::Slice(from, to) => text.extend(output.slice(at(from), at(to))?),
			}
		}
		output.overwrite(at(start), at(end), text)?;
	}
	let mut units = output.into_units();
	units.extend(append.encode_utf16());
	Ok(String::from_utf16_lossy(&units))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// runtime.json, recorded by tools/parity/runtime.mjs from js-runtime
	/// 0.4.5's `transformSource` on @swc/wasm-web 1.6.5.
	#[test]
	fn modules_are_rewritten_as_js_runtime_rewrites_them() {
		let vectors = crate::testing::vectors("runtime.json");
		let vectors = vectors.as_array().expect("an array");
		assert!(vectors.len() >= 38);
		let mut failures = Vec::new();
		for vector in vectors {
			let path = vector[0].as_str().expect("a path");
			let source = vector[1].as_str().expect("a source");
			let basename = path.rsplit('/').next().unwrap_or(path);
			let actual = transform_source(path, basename, source).ok();
			if actual.as_deref() != vector[2].as_str() {
				failures.push(format!(
					"{path}:\n  expected {}\n  got      {actual:?}",
					vector[2]
				));
			}
		}
		assert!(
			failures.is_empty(),
			"{} of {} differ:\n{}",
			failures.len(),
			vectors.len(),
			failures.join("\n")
		);
	}

	#[test]
	fn an_empty_module_and_an_unaliased_export_list_throw_as_in_js_runtime() {
		assert_eq!(
			transform_source("a.js", "a.js", "").expect_err("no statements"),
			"TypeError: Cannot read properties of undefined (reading 'span')"
		);
		assert_eq!(
			transform_source("a.js", "a.js", "const a = 1\nexport { a }").expect_err("no as"),
			"TypeError: Cannot read properties of null (reading 'value')"
		);
	}
}
