# leafcutter-ant

**leafcutter-ant is a Rust port of the Dash compiler that bridge. uses for
Minecraft Bedrock add-ons: the same project in, the same bytes out.**

It ports `@bridge-editor/dash-compiler` 0.13.0 ("TS Dash") and keeps its
interfaces: the plugin hooks, `config.json`, the `.bridge/.dash.<mode>.json`
cache, and every byte of output. TS Dash is the specification; where it does
something odd that a project can observe, leafcutter-ant does the same odd
thing and lists it in the quirk ledger below. The plan, milestones and open
questions are in `PRD-native-dash-compiler.md` next to this repository.

It cannot build a project yet. The port goes from the leaves up, and so far
holds the JSON layer the rest is built on: JSON values with JavaScript's
property order, a port of the json5 2.2.1 reader Dash bundles, and a writer
that matches V8's `JSON.stringify`. The library has two runtime
dependencies, `indexmap` and `ryu-js`, and does nothing until it is called.

## leafcutter-ant is not for you if you

- need to build a project today. Use TS Dash, through
  [deno-dash-compiler](https://github.com/bridge-core/deno-dash-compiler) or
  the bridge. editor.
- want features TS Dash does not have. Nothing new goes in until the output
  matches TS Dash on the whole parity corpus, and TS Dash's known quirks stay
  until then too.
- want to write compiler plugins in Rust. Plugins stay JavaScript modules, as
  in TS Dash.

## How it works

Parity is measured against the JavaScript that TS Dash runs, not described.
`tools/parity` pins that JavaScript (json5 2.2.1, and Node through
`.nvmrc`) and records golden vectors from it into
`crates/leafcutter-ant/tests/vectors/`:

| Vectors | Recorded from |
| --- | --- |
| `numbers.json` | V8's `JSON.stringify` of hand-picked and random f64 values |
| `stringify.json` | V8's `JSON.stringify` of random documents, compact and tab-indented |
| `json5.json` | json5 2.2.1's `parse` of edge cases, random json5 and damaged json5: results and error messages |

The Rust tests compare against every vector. CI records the vectors again and
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

`cargo doc --open` builds the API reference. The `leafcutter` binary prints
its version and help; the `build` command comes with the compiler pipeline.

## Quirk ledger

Behaviour kept from TS Dash because projects can observe it. Each row is
pinned by a test and stays until parity has shipped.

| Quirk | What TS Dash does | Pinned by |
| --- | --- | --- |
| `"__proto__"` keys vanish | json5 2.2.1 assigns `parent[key] = value`, which sets the prototype instead of adding a key | `a_proto_key_never_becomes_a_property`, `json5.json` |
| Index-like keys move to the front | JavaScript objects list `"0"` to `"4294967294"` first, ascending: `{"b":1,"1":2}` is written `{"1":2,"b":1}` | `index_keys_iterate_first_in_numeric_order_then_the_rest_in_insertion_order`, `stringify.json` |
| `NaN` and `Infinity` are written as `null` | json5 reads them; `JSON.stringify` writes `null` | `nan_and_the_infinities_are_written_as_null` |
| Error columns count UTF-16 units | json5 counts code units, and only `\n` starts a line | `the_position_is_the_column_after_the_offending_character`, `json5.json` |
| An invalid escaped key character is reported 5 columns back | json5 subtracts 5 from the column | `an_escaped_key_character_that_cannot_be_in_a_key_reports_five_columns_back` |

Where leafcutter-ant differs from TS Dash, and why:

| Difference | TS Dash | leafcutter-ant | Why |
| --- | --- | --- | --- |
| A `\u` escape that leaves a lone surrogate | keeps it, writes `"\udXXX"` | U+FFFD | Rust strings cannot hold a lone surrogate |
| Tab-indented output nested more than about 4,000 deep | V8 throws `RangeError` | writes it | the writer does not recurse |
| U+2028 or U+2029 inside a json5 string | json5 prints a warning to the console | reads it silently | a library does not print |
| A `"__proto__"` key in a file the Deno CLI reads through its own `FileSystem.readJson` | that path uses json5 2.2.3, which keeps the key | drops it | leafcutter-ant reads json5 as 2.2.1 everywhere, as the plugins inside Dash do |

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

`just setup` needs [mise](https://mise.jdx.dev) (for just, lefthook and
git-cliff) and [rustup](https://rustup.rs). Before just exists, run
`scripts/tasks.sh setup`.

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

[cc]: https://www.conventionalcommits.org

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
| `crates/leafcutter-ant-cli/` | The `leafcutter` binary: arguments in, library call, output out |
| `tools/parity/` | The pinned JavaScript that records the vectors and json5's tables |
| `Cargo.toml` | Workspace members, the shared version, and the lint levels |
| `rust-toolchain.toml` | The pinned Rust release, with rustfmt and clippy |
| `rustfmt.toml` | Tabs, and the edition rustfmt uses when the hook calls it directly |
| `mise.toml` | just, lefthook and git-cliff |
| `scripts/tasks.sh` | What every task means. Edit the Rust part to change behaviour. |
| `scripts/check-shared.sh` | The black-garden-ants comparison |
| `release.sh`, `cliff.toml`, `lefthook.yml`, `scripts/commit-msg.sh` | Shared release flow, changelog rules, hooks and commit check |
| `LICENSE` | Dash's MIT licence, which leafcutter-ant is released under |
| `NOTICE.md` | The licence notice for json5, whose parser is ported here |

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
`NOTICE.md` holds json5's notice for its ported parser.
