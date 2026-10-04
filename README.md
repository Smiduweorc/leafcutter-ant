# leafcutter-ant

![logo of a leafcutter ant](./assets/logo.png)

**leafcutter-ant is a Rust port of the Dash compiler that bridge. uses for
Minecraft Bedrock add-ons: the same project in, the same bytes out.**

It ports `@bridge-editor/dash-compiler` 0.13.0 ("TS Dash") and keeps its
interfaces: the plugin hooks, `config.json`, the `.bridge/.dash.<mode>.json`
cache, and every byte of output. TS Dash is the specification; where it does
something odd that a project can observe, leafcutter-ant does the same odd
thing and lists it in the quirk ledger below. The plan, milestones and open
questions are in `PRD-native-dash-compiler.md` next to this repository.

It builds projects whose plugins are the built-ins `simpleRewrite`,
`rewriteForPackaging`, `entityIdentifierAlias`, `formatVersionCorrection`,
`floatPropertyTruncationFix`, `contentsFile`, `typeScript`,
`generatorScripts` and `customCommands`, and compiler plugins from
extensions; scripts run in an embedded JavaScript engine (QuickJS). The built-ins that still need porting (`moLang` and the custom
components) get an error on the console when a plugin list names them, and
the build goes on without them. Hot updates and `watch` come after that.

The library's runtime dependencies are `indexmap`, `ryu-js`, `regress`,
`futures-util`, `swc`, `swc_common`, `serde_json` and `rquickjs`, each with
its reason in `Cargo.toml`, and it does nothing until it is called. `Cargo.lock` holds swc
and the crates around it at the versions swc 1.6.5 was released with; do not
let `cargo update` move them.

## leafcutter-ant is not for you if you

- need Molang functions or custom components today, or a watch mode. Use
  TS Dash, through
  [deno-dash-compiler](https://github.com/bridge-core/deno-dash-compiler) or
  the bridge. editor.
- want features TS Dash does not have. Nothing new goes in until the output
  matches TS Dash on the whole parity corpus, and TS Dash's known quirks stay
  until then too.
- want to write compiler plugins in Rust. Plugins stay JavaScript modules, as
  in TS Dash.

## How it works

Parity is measured against the JavaScript that TS Dash runs, not described.
`tools/parity` pins that JavaScript (TS Dash 0.13.0 and every package it
loads, at the versions TS Dash's lockfile resolves, and Node through
`.nvmrc`) and records golden vectors from it into
`crates/leafcutter-ant/tests/vectors/`:

| Vectors | Recorded from |
| --- | --- |
| `numbers.json` | V8's `JSON.stringify` of hand-picked and random f64 values |
| `stringify.json` | V8's `JSON.stringify` of random documents, compact and tab-indented |
| `json5.json` | json5 2.2.1's `parse` of edge cases, random json5 and damaged json5: results and error messages |
| `paths.json` | the functions of pathe 2.0.2 (which Dash imports) and pathe 1.1.2 (which mc-project-core imports) that each calls, on edge cases and random paths |
| `typescript.json` | @swc/wasm-web 1.6.5 on the sources in `tools/parity/typescript/`, with the options Dash's `typeScript` plugin passes, with and without inline source maps |
| `runtime.json` | js-runtime 0.4.5's `transformSource`, the rewrite that turns a script's `import` and `export` statements into calls its module loader understands, on the sources in `tools/parity/runtime/` and on the two modules Dash hands the loader as source text |
| `globs.json` | the picomatch that common-utils vendors: the regex source it builds for every file definition matcher, the float fix's globs and random globs, whether V8 compiles that source, and `isMatch` on paths built to match and paths that should not |
| `is-glob.json` | is-glob 4.0.3 on the same globs and on random strings |
| `plugins.json` | TS Dash's own `entityIdentifierAlias` and `floatPropertyTruncationFix`, taken out of a real setup and called hook by hook, for input the non-JavaScript corpus cannot give them because no built-in there reads the files they look at |
| `project.json` | mc-project-core 0.5.0 with the vendored definitions: pack roots and pack and file type detection for regular and malformed project configs, with picomatch as the matcher (as the Deno CLI sets it up) and with a matcher that never matches (as the editor does) |

The Rust tests compare against every vector. `tools/parity/corpus.mjs` also
builds each project under `crates/leafcutter-ant/tests/corpus/` with TS Dash
itself, four ways (production and development, each with and without a
separate output file system, as the Deno CLI's `--out` gives one), and
records what every build wrote, removed and output in
`<project>.expected.json`; `tests/corpus.rs` builds the same projects with
leafcutter-ant and compares. Both answer `requestJsonData` from the vendored
`validCommand.json`, and serve a script's `https://` imports from the bodies
the project's `corpus.json` lists. CI records the vectors and the corpus again and
fails if the committed copies differ, so the files cannot drift from what the
JavaScript does. To record them yourself:

```sh
cd tools/parity
npm ci
npm run vectors
```

`src/json/json5_unicode.rs` comes from the same run: json5 carries its own
Unicode 10 tables for unquoted keys, and they are copied out of json5 rather
than taken from a newer Unicode release.

The host owns the disk and the terminal. leafcutter-ant reads a project and
writes its output through the `fs::FileSystem` trait, a port of TS Dash's
`FileSystem`, and reports through a `console::Console` the host supplies. The
trait is async: its methods return boxed futures that are not `Send`, and
the library picks no executor, so the CLI can block on them, the desktop app
can run them on a single-threaded runtime, and a browser host can back them
with a page's async file APIs. `fs::NativeFileSystem` is the local disk. It
lists directories sorted by name, writes through a temporary file that is
renamed over the target, and blocks the thread that polls it.

## What scripts may touch

Extension plugins, generator scripts and custom commands are JavaScript
modules that run in QuickJS, through rquickjs, in the same process as the
compiler. The engine is a runtime with no walls of its own: a script can do
anything its JavaScript can reach, so what it can reach is kept to this list.

- **Globals**: ECMAScript's own (`Object`, `JSON`, `Promise`, `Map`,
  `RegExp`, typed arrays and the rest); the few web globals QuickJS adds
  (`performance`, `queueMicrotask`, `atob`, `btoa`, `DOMException`);
  `console` (`log`, `info`, `warn`, `error`, `time`, `timeEnd`, all going to
  the host's `Console`); and `Blob` and `File` (`text`, `arrayBuffer`,
  `bytes`, `slice`, `size`, `type`, `name`, `lastModified`). There is no
  `fetch`, no timers, no `process`, `Deno` or `require`, and no file or
  network access of the engine's own. A test pins this list
  (`scripts_see_exactly_the_globals_the_readme_lists`).
- **Modules**, resolved as js-runtime 0.4.5 resolves them: the modules Dash
  registers (`@bridge/compiler` with the build `mode`; `@bridge/generate`,
  `pathe` and `path-browserify` while generator scripts run), relative paths
  with `.ts` then `.js` appended, `.json` files read with json5, and
  `https://` URLs, which only the host fetches (`DashOptions::https_imports`:
  the CLI fetches them, a host can refuse them all). Every path goes through
  the host's file system.
- **The plugin context** (`TCompilerPluginFactory.ts`): `options`,
  `console`, `fileSystem` and `outputFileSystem` (the host's file systems,
  rooted where the host roots them), `projectConfig`, `projectRoot`,
  `packType`, `fileType`, `targetVersion`, `requestJsonData` (the host's
  callback), `getAliases`, `getAliasesWhere`, `getFileMetadata`,
  `addFileDependencies`, `getOutputPath`, `unlinkOutputFiles`,
  `hasComMojangDirectory`, `compileFiles`, `jsonStringifyWithFloatFix` and
  `jsRuntime`, Dash's script runtime.
- **Time**: unlimited by default, as in TS Dash; a host can stop a script
  that runs too long without returning (`DashOptions::script_time_limit`),
  and the hook that ran it fails with an error.

A file's data crosses into the engine as the objects `JSON.parse` would
make, and comes back the way `JSON.stringify` writes it, without recursion
in either direction. An object a script returns stays in the engine, so the
next script to see the file gets that same object, as in TS Dash.

## Quick start

```sh
git clone https://github.com/Smiduweorc/leafcutter-ant
cd leafcutter-ant
scripts/tasks.sh test
```

The library, as a dependent crate sees it (this block runs as a test):

```rust
use leafcutter_ant::json::{Indent, parse_json5, stringify};

let value = parse_json5("{b: 1, '1': .5, a: [0x10, +Infinity], // note\n}")?;
assert_eq!(stringify(&value, Indent::None), r#"{"1":0.5,"b":1,"a":[16,null]}"#);
assert_eq!(
	stringify(&value, Indent::Tab),
	"{\n\t\"1\": 0.5,\n\t\"b\": 1,\n\t\"a\": [\n\t\t16,\n\t\tnull\n\t]\n}"
);

let error = parse_json5("{a: 1,,}").unwrap_err();
assert_eq!(error.to_string(), "JSON5: invalid character ',' at 1:7");
# Ok::<(), leafcutter_ant::json::Json5Error>(())
```

`cargo doc --open` builds the API reference.

To build a project, run `leafcutter build` in its folder, with the Deno
CLI's flags:

```sh
cargo build --release
cd path/to/project
path/to/leafcutter-ant/target/release/leafcutter build --mode development --out ./out
```

`--mode` is `production` unless given, `--out` writes into a separate folder
the way a `com.mojang` folder is written (`preview` names Minecraft
Preview's, and in development mode the default is Minecraft's own where
`APPDATA` is set), `--compilerConfig` takes the plugin list from another
file, and `--noCache` empties the cache first. The project config is
`dash-config.json` when that file exists, else `config.json`. The pack and
file definitions come from bridge-core/editor-packages and are kept in
`~/.dash`, which a build empties once it is more than a day old, as the Deno
CLI does. The exit code is 0 for a build, 1 when the build could not run,
and 2 for arguments it does not understand.

## Quirk ledger

Behaviour kept from TS Dash because projects can observe it, bugs included.
Each row is pinned by a test that cites the TS source. None is fixed before
parity has shipped; after that, each fix lands as its own commit on a
separate branch, and this table is that branch's checklist.

| Quirk | What TS Dash does | Pinned by |
| --- | --- | --- |
| `"__proto__"` keys vanish | json5 2.2.1 assigns `parent[key] = value`, which sets the prototype instead of adding a key | `a_proto_key_never_becomes_a_property`, `json5.json` |
| Index-like keys move to the front | JavaScript objects list `"0"` to `"4294967294"` first, ascending: `{"b":1,"1":2}` is written `{"1":2,"b":1}` | `index_keys_iterate_first_in_numeric_order_then_the_rest_in_insertion_order`, `stringify.json` |
| `NaN` and `Infinity` are written as `null` | json5 reads them; `JSON.stringify` writes `null` | `nan_and_the_infinities_are_written_as_null` |
| Error columns count UTF-16 units | json5 counts code units, and only `\n` starts a line | `the_position_is_the_column_after_the_offending_character`, `json5.json` |
| An invalid escaped key character is reported 5 columns back | json5 subtracts 5 from the column | `an_escaped_key_character_that_cannot_be_in_a_key_reports_five_columns_back` |
| A project config whose `compiler` is `null` stops the build | `isCompilerActivated` checks `compiler !== undefined`, then reads `compiler.plugins` and throws (`Dash.ts`) | `a_null_compiler_stops_the_build_as_the_type_error_does_in_ts_dash` |
| A plugin list entry named like a property of `Object.prototype`, such as `constructor`, fails as an extension plugin | the extension plugin map is a plain object, so `plugins["constructor"]` finds the inherited function and Dash tries to run it as a module (`AllPlugins.ts`) | `plugin_list_entries_name_builtins_extensions_and_unknown_plugins` |
| A plugin that ignores a file drops every plugin with the same id from that file's hooks | `createImplementedHooksMap` filters by plugin id, and a plugin listed twice has one id (`DashFile.ts`) | `a_plugin_that_ignores_a_file_is_left_out_of_its_per_file_hooks_by_id` |
| A `load` or `transform` chain that ends in `null` keeps the data it started with, even after a plugin replaced it | the chain's result goes through `?? file.data` (`LoadFiles.ts`, `TransformFiles.ts`) | `load_and_transform_chains_fall_back_to_the_start_when_they_end_in_null` |
| Virtual files an `include` hook adds come before every pack file, in processing and in the cache file | `loadAll` adds `[path, { isVirtual }]` entries at once and the pack files afterwards (`IncludedFiles.ts`) | `included_virtual_files_come_first_and_included_paths_after_the_packs` |
| A failed copy or write leaves the file out of the output with no message | `Promise.allSettled` over the copies and writes, results unread (`LoadFiles.ts`, `TransformFiles.ts`) | `failed_writes_and_copies_are_silent_as_in_ts_dash` |
| A plugin option named `mode` or `buildType` replaces the live value, for that plugin only | the options object is `{ get mode(), get buildType(), ...pluginOpts }`, so the spread overwrites the getters (`AllPlugins.ts`) | `a_plugin_option_named_mode_replaces_the_build_mode_for_that_plugin` |
| `simpleRewrite` clears the default pack folder before a build, never the one `packNameSuffix` names, so output under a suffix piles up | `buildStart` unlinks `<packName> <defaultPackPath>` (`SimpleRewrite.ts`) | `a_full_build_clears_the_default_pack_folder_but_not_a_suffixed_one` |
| A block, item or fog file that json5 cannot read, or that reads as `null`, is copied as it is | `formatVersionCorrection`'s `read` catches the error, logs it on the global console and returns nothing (`FormatVersionCorrection.ts`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`format-version`) |
| A `format_version` named like a property of `Object.prototype` drops the key, or turns it into `{}` for `__proto__` | `formatVersionMap[version]` looks in a plain object and finds the inherited function, which `JSON.stringify` leaves out (`FormatVersionCorrection.ts`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`format-version`) |
| `floatPropertyTruncationFix` fixes the first float property of a `player.json` and misses the rest | its `JSON.stringify` replacer pushes the key of every object it enters and never pops it, so later paths carry every key visited before (`FloatPropertyTruncationFix.ts`) | `finalize_build_and_its_console_lines_match_ts_dash` |
| A float that prints with an exponent, or as `NaN` or `Infinity`, is written as the marker string `"$___dash___floatPropertyTruncationFix___..."` | the marker is replaced by a regular expression that accepts only digits, `.` and `-` | `finalize_build_and_its_console_lines_match_ts_dash` |
| That marker, written by the project itself anywhere in a `player.json`, is replaced too | the replacement runs over the whole output | `a_marker_is_replaced_only_when_digits_dots_and_minus_signs_follow` |
| Every entity whose path ends in `player.json`, `myplayer.json` included, is written tab-indented; other entity files are handed on unchanged, which keeps any later `finalizeBuild` plugin from seeing them | `finalizeBuild` tests `endsWith("player.json")` and returns `fileContent` for other entities | `finalize_build_and_its_console_lines_match_ts_dash` |
| The float fix logs the path of every value in a `player.json`, and every glob test on a number | `console.log` inside the replacer | `finalize_build_and_its_console_lines_match_ts_dash` |
| `contentsFile` writes every file of a pack as the pack's file list and never writes `contents.json` | `read` and `finalizeBuild` return early for `contents.json` instead of for every other file (`ContentsFile.ts`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`contents-file`, `contents-then-rewrite`, `float-fix-then-contents`) |
| The list holds the path each `transformPath` call was given, `contents.json` included; a file an earlier plugin already moved out of the pack is left out | `transformPath` pushes its argument and looks up the pack by it | `every_corpus_project_builds_as_ts_dash_builds_it` (`rewrite-then-contents`) |
| Every build in a session appends the whole pack to the list again | the list is filled in `transformPath` and never emptied, and plugins live from setup to setup | `every_build_in_a_session_appends_the_whole_pack_again` |
| A `.ts` file swc refuses is written to its `.js` path as the TypeScript source | the `load` hook throws, the host catches it, the data stays the source text, and `finalizeBuild` returns any string (`TypeScript.ts`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`typescript`) |
| A generator script that throws, does not compile, imports a module that cannot be found, or has a falsy default export writes its own source to its output path | `load` returns `null`, the chain's `?? file.data` keeps the source `read` returned, and `finalizeBuild` writes any string (`GeneratorScripts/Plugin.ts`, `LoadFiles.ts`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`generator-scripts`) |
| An `import` that names a module with its `.js` or `.ts` extension is not found | js-runtime appends `.ts`, then `.js`, to every name it looks up (`Runtime.ts` `require`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`generator-scripts`) |
| Templates read with `useTemplate` are written to the output too, `omitTemplate` or not | `currentTemplates` is never filled, so templates are neither unlinked nor required, and `omitUsedTemplates` is only read in `ignore`, which ran for every file before the first script (`GeneratorScripts/Plugin.ts`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`generator-scripts`) |
| A script whose default export is a function writes nothing, and its metadata still lists the path as generated | `finalizeBuild` returns the function, `JSON.stringify` gives `undefined`, and the failed write is not reported | `every_corpus_project_builds_as_ts_dash_builds_it` (`generator-scripts`) |
| A typed array a script exports is written as JSON with index keys, `{"0":123,"1":125}`, not as bytes | `finalizeBuild` stringifies every object | `every_corpus_project_builds_as_ts_dash_builds_it` (`generator-scripts`) |
| Scripts share each module by path, and a module keeps the variables of the script that evaluated it: `useTemplate` resolves against the folder of the first script that imported `@bridge/generate`, except in scripts that started importing it before that one finished | the loader caches a module once its code has run, with the `env` it ran with (`Runtime.ts` `eval`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`generator-scripts`, whose scripts in different folders all start together and each get their own copy) |
| Custom commands written in TypeScript are never loaded, so their names stay in the output as they were written | `read` returns a command file's text only when its path ends in `.js` (`Commands/Plugin.ts`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`custom-commands`) |
| A command location whose last segment is a `*{regex}` key finds nothing | `setObjectAt` treats only a plain `*` specially in the last segment and reads any other as a literal key (common-utils `setObjectAt`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`custom-commands`) |
| A command template that returns neither a string nor an array leaves a `# Failed to process command ...` line in `.mcfunction` output; command lists in JSON drop it | `process` turns the error into a comment, and only `.mcfunction` files keep comments (`Commands/Command.ts`, `transformCommands.ts`) | `every_corpus_project_builds_as_ts_dash_builds_it` (`custom-commands`) |
| A `~/.dash/.timestamp` that is not a number keeps the cache forever | `parseInt` gives NaN, and `now - NaN > day` is false (`LocalCache.ts`) | `a_timestamp_that_is_not_a_number_never_expires` |
| A required file that does not exist is skipped without a message | `resolveSingle` reports an undefined dependency only for an entry `query` never returns (`ResolveFileOrder.ts`) | `the_cache_file_lists_every_file_with_aliases_requirements_and_update_files` |

Where leafcutter-ant differs from TS Dash, and why:

| Difference | TS Dash | leafcutter-ant | Why |
| --- | --- | --- | --- |
| A `\u` escape that leaves a lone surrogate, or a config whose `packs` is a string split between the halves of a surrogate pair | keeps it, writes `"\udXXX"` | U+FFFD | Rust strings cannot hold a lone surrogate |
| Tab-indented output nested more than about 4,000 deep | V8 throws `RangeError` | writes it | the writer does not recurse |
| The console lines of `floatPropertyTruncationFix` and the parse errors of `formatVersionCorrection` | go to the global `console` | go to the host's `Console` | a library does not print |
| `leafcutter build` start-up | checks GitHub for a newer release and downloads the swc WebAssembly module | neither | the release check is a network call nobody asked for, and swc is compiled in |
| Cached definitions in `~/.dash` | read with `JSON.parse` | read with the json5 reader | the cache is written by the tool itself as plain JSON, which both read alike |
| U+2028 or U+2029 inside a json5 string | json5 prints a warning to the console | reads it silently | a library does not print |
| A `"__proto__"` key in a file the Deno CLI reads through its own `FileSystem.readJson` | that path uses json5 2.2.3, which keeps the key | drops it | leafcutter-ant reads json5 as 2.2.1 everywhere, as the plugins inside Dash do |
| File or pack definitions that are not an array of objects with a string `id` | takes them and fails later, where a plugin asks for a file type | refused when the host builds `FileTypes` or `PackTypes` | the definitions come from the host, which can report them at startup |
| A plugin reading a key an object lacks, where the object has a `"__proto__"` key | json5 2.2.1 makes the `"__proto__"` value the object's prototype, so the lookup finds the key there | the key is dropped and no prototype is kept, so the lookup finds nothing | JSON values here have no prototypes; modelling them is an open question (pinned by `a_proto_key_gives_the_file_no_inherited_format_version`) |
| The order the hooks of different files run in | each file's hooks run as its read and every `await` before them finish, interleaved with the other files' | every file steps through its hooks at once, side by side: one step per hook and per settled promise, in build order within a step, and a script's promise jobs run first in, first out, as in one event loop | output must not depend on timing, yet scripts must interleave as they do in TS Dash: two generator scripts that both import `@bridge/generate` each get their own copy of it only because neither has finished importing it when the other starts. Where two files register one alias, the one that gets there later in this order owns it |
| Two files with the same output path | both writes run at once and either may land last | the later file in build order wins | the same |
| A copied file and a written file with the same output path | the copy starts first and usually lands first | copies finish before any write starts, so the written file wins | the same |
| Two extensions that declare the same compiler plugin id | the manifest read last wins | the manifest listed last wins | the same |
| The order of a directory listing | the operating system's, through Deno's `readDir` | sorted by the bytes of each name | the listing order becomes the order of contents lists and of the cache file, and must not change from one machine to the next |
| Error messages from the engine | V8's | QuickJS's, which match V8's for most `TypeError` and `SyntaxError` texts but not all; they reach only the console | a different engine |
| A script's `console.log` of something that is not a string | the host console formats it (Deno's inspector in the CLI) | strings as they are, errors as `String(error)`, functions as `[Function: name]`, anything else as its JSON, or `String(value)` where it has none | the host's `Console` takes text |
| A plugin factory that throws, or returns something that is not an object | the rejection of `addPlugin` goes unhandled, which ends the Deno CLI | `Failed to create plugin <id>: <error>` on the console, and the build goes on without the plugin | a library does not end the process |
| A factory that waits (an `async` factory) | its hooks are registered when it finishes, possibly after other plugins' hooks or after the build started | every factory is waited for in plugin list order before the build starts | plugin order must not depend on timing |
| An `include` entry that is neither a path nor a `[path, options]` pair | `loadAll` throws and the build stops | skipped | |
| A hook whose promise never settles | the build waits forever | once nothing can settle it any more, the hook fails with `Error: the promise can never settle: nothing it waits for is still running` and the build goes on | a build should end |
| A script that runs without returning | runs forever | the same, unless the host sets `script_time_limit`, which stops it with an error the hook reports | |
| A script's object that a Rust built-in changes in place (`formatVersionCorrection`'s `transform`) | the built-in changes the script's object, which a script that kept it sees | the built-in replaces it with a JSON copy it changes | the built-ins work on JSON |
| `getFileMetadata(path).set(key, value)` | keeps `value` itself | keeps what `JSON.stringify` makes of it; `undefined` removes the key | the cache file stores metadata as JSON anyway |
| A non-string alias, required file or path a hook returns | kept as it is, and written to the cache file as JSON | aliases are kept as JSON values; required files and paths go through `String(value)` | the file model holds strings |
| The modules `@molang/expressions`, `@molang/core` and `molang` | registered in Dash's script runtime | missing until the Molang port (M3) | not ported yet |
| `projectConfig`, `fileType` and `packType` in the plugin context | mc-project-core's objects | the same classes for `projectConfig` (`get`, `resolvePackPath`, `getRelativePackRoot`, `getAbsolutePackRoot`, `getAvailablePacks`, `getAvailablePackPaths`) and `packType` (`all`, `get`, `getId`, `getFromId`, `addExtensionPackType`); `fileType` has `all`, `get`, `getId`, `getIds` and `isJsonFile`, with detection run by the compiler's own port | Dash never calls the rest; `fileType` shares the compiler's cache so scripts and built-ins see one detection |
| `File` and `Blob` | the host's (Deno's, a browser's) | a shim with `text`, `arrayBuffer`, `bytes`, `slice`, `size`, `type`, `name` and `lastModified`, and no `stream` | the engine has no web APIs |
| pathe's `resolve` and `relative` on a path that climbs above the working directory | resolved against `process.cwd()` in the Deno CLI and against `/` in the editor | resolved against `/` | the result then depends on no process state; relative paths that stay below the working directory give the same answer either way |

## Tasks

| Task | What it does |
| --- | --- |
| `just setup` | Install the toolchain, dependencies and git hooks |
| `just fmt` | Format sources in place |
| `just lint` | `cargo fmt --check`, clippy with warnings as errors, and the API docs build; changes nothing |
| `just test` | Unit, integration and doc tests for the whole workspace |
| `just build` | Release build of every crate |
| `just changelog` | Regenerate `CHANGELOG.md` from the commit history |
| `just release v1.2.3` | Cut a release (or `patch`, `minor`, `major`) |
| `just preflight` | `lint` + `test`, as run before a release |
| `just hooks` | (Re)install the git hooks |
| `just clean` | `cargo clean` |
| `just help` | List the tasks |

Every one of these is a thin wrapper around `scripts/tasks.sh <task>`. That
file is the single definition of what each task means; `just`, the git hooks
and CI all call into it, so they cannot drift apart. Every cargo call passes
`--locked`, so a `Cargo.lock` that is out of date fails the task instead of
being rewritten. Recording the golden vectors needs Node, so it is the
`vectors` script in `tools/parity/package.json` instead.

`just setup` needs [mise](https://mise.jdx.dev) (for just, lefthook,
git-cliff and commitlint-rs) and [rustup](https://rustup.rs). Before just
exists, run `scripts/tasks.sh setup`. commitlint-rs has no release binaries
for the pinned version, so the first `mise install` builds it with cargo.

## Releasing

```sh
just release v1.2.3          # or: patch | minor | major
PUSH=1 just release patch    # also push the branch and the tag
```

`release.sh` refuses to run off `master` or on a dirty tree, runs `preflight`,
sets the version in `[workspace.package]` of `Cargo.toml` (every crate
inherits it) and refreshes `Cargo.lock`, regenerates `CHANGELOG.md`, commits
as `chore(release): prepare for <tag>`, and creates an annotated tag whose
message is that release's changelog. Publishing is manual; every crate has
`publish = false`.

## Commits

Commit messages follow [Conventional Commits][cc]. `scripts/commit-msg.sh`
checks them in the commit-msg hook, and git-cliff builds `CHANGELOG.md` from
them. Dependabot's bumps are `chore(deps): ...`, which the changelog skips.

Because `.commitlintrc.yml` exists, the hook hands the message to
[commitlint-rs][clrs] (pinned in `mise.toml`) with the rules in that file, and
still refuses a subject line over 100 characters, which commitlint-rs has no
rule for. Without commitlint-rs on `PATH` it falls back to its own regex,
which accepts the same types.

[cc]: https://www.conventionalcommits.org
[clrs]: https://github.com/KeisukeYamashita/commitlint-rs

## Files shared with black-garden-ants

`release.sh`, `cliff.toml`, `lefthook.yml`, `scripts/commit-msg.sh`,
`.editorconfig`, the issue templates, and the head and tail of
`scripts/tasks.sh` are copies of black-garden-ants' `src/core`.
`scripts/check-shared.sh` compares them byte for byte, and the
**Shared files** workflow runs it on every push and every Monday. To change
one of them, change it in black-garden-ants first, then copy it here.

## Layout

| Path | Purpose |
| --- | --- |
| `crates/leafcutter-ant/` | The library: everything the compiler does |
| `crates/leafcutter-ant/tests/vectors/` | Golden vectors recorded by `tools/parity` |
| `crates/leafcutter-ant/tests/corpus/` | Small projects for the parity harness, each next to the output TS Dash gave for it |
| `crates/leafcutter-ant/tests/data/` | `fileDefinitions.json`, `packDefinitions.json` and `validCommand.json` from bridge-core/editor-packages at commit `10e360dc`, the data TS Dash fetches at run time |
| `crates/leafcutter-ant/src/js/layer/` | The JavaScript the engine runs before any script: js-runtime's module loader, the `File` shim and console, and the parts of TS Dash that handle scripts' values, copied from the published packages |
| `crates/leafcutter-ant-cli/` | The `leafcutter` binary: arguments in, library call, output out, and the `~/.dash` cache |
| `assets/logo.png` | The logo, drawn by grml |
| `tools/parity/` | The pinned JavaScript that records the vectors, json5's tables and the corpus output |
| `Cargo.toml` | Workspace members, the shared version, and the lint levels |
| `rust-toolchain.toml` | The pinned Rust release, with rustfmt and clippy |
| `rustfmt.toml` | Tabs, and the edition rustfmt uses when the hook calls it directly |
| `mise.toml` | just, lefthook, git-cliff and commitlint-rs |
| `.commitlintrc.yml` | The commitlint-rs rules the commit-msg hook applies |
| `scripts/tasks.sh` | What every task means. Edit the Rust part to change behaviour. |
| `scripts/check-shared.sh` | The black-garden-ants comparison |
| `release.sh`, `cliff.toml`, `lefthook.yml`, `scripts/commit-msg.sh` | Shared release flow, changelog rules, hooks and commit check |
| `LICENSE` | Dash's MIT licence, which leafcutter-ant is released under |
| `NOTICE.md` | The licence notices for the code ported here and the vendored test data |

## Lints

`unsafe_code` is forbidden in every crate, and missing docs, `dbg!`, `todo!`
and `unimplemented!` warn, which `just lint` turns into errors. The library
also denies printing to stdout or stderr: it returns values and the host
decides what to show.

## Known quirks

- The Rust release is pinned in two places that must agree: `channel` in
  `rust-toolchain.toml` and `rust-version` in `Cargo.toml`. Raise them
  together by hand.
- The edition is in both `Cargo.toml` and `rustfmt.toml`, because the
  pre-commit hook runs rustfmt on staged files without Cargo.
- rustfmt also formats the out-of-line modules a staged file declares, so the
  hook can change a file you did not stage. It stages only what was already
  staged.
- Dependabot does not watch `tools/parity`: its pins must stay the versions
  TS Dash 0.13.0 resolves.

## Licence

MIT: Dash's licence, in `LICENSE`, since leafcutter-ant is a port of it.
`NOTICE.md` holds the notices for the code ported from other MIT projects.
