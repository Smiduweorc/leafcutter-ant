# rustCrateTemplate

**rustCrateTemplate is where a Smiduweorc Rust project starts: a Cargo
workspace driven by the same tasks, hooks and release flow as
[black-garden-ants][bga].**

It gives you a library crate that holds the program, a thin CLI crate that
calls it, strict lints, tabs, and CI on Linux, macOS and Windows. It has no
runtime dependencies and adds none. It deliberately leaves out Deno FFI, WASM
and any other host glue: the project that needs one adds it as its own crate.
Publishing is manual; every crate starts with `publish = false`.

[bga]: https://github.com/Smiduweorc/black-garden-ants

## This template is not for you if you

- want a JavaScript library with a native hot path. That is
  [nodeaddons](https://github.com/Smiduweorc/nodeaddons) (C++ through N-API).
- want a TypeScript or Go project. black-garden-ants has those.
- need a crate published on every tag. Releases here cut a tag and a
  changelog; nothing is uploaded anywhere.

## Getting started

1. Create a repository from this one: **Use this template** on GitHub, or
   `gh repo create <name> --template Smiduweorc/rustCrateTemplate --clone`.
2. Install [mise](https://mise.jdx.dev) and
   [rustup](https://rustup.rs), then run `just setup`. That installs just,
   lefthook and git-cliff from `mise.toml`, the Rust toolchain from
   `rust-toolchain.toml`, the crates, and the git hooks. Before just exists,
   run `scripts/tasks.sh setup` instead.
3. Rename `app`. Rename the directories `crates/app` and `crates/app-cli`,
   then run `rg -w app crates` for the rest: the `name` fields and the
   `[[bin]]` name in the two `Cargo.toml` files, the `app = { path = "../app" }`
   dependency, the `app::` paths in the code, tests and doc test, and the
   `usage: app` and `app:` strings in `main.rs` and `tests/cli.rs`. Also change
   `CARGO_BIN_EXE_app` in `tests/cli.rs`, which that search misses because it
   is part of a longer name. `Cargo.lock` updates itself on the next build.
4. Replace `greet` with the real API, keeping the program in the library crate
   and the CLI to argument parsing and printing.

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
being rewritten.

## Releasing

```sh
just release v1.2.3          # or: patch | minor | major
PUSH=1 just release patch    # also push the branch and the tag
```

`release.sh` refuses to run off `master` or on a dirty tree, runs `preflight`,
sets the version in `[workspace.package]` of `Cargo.toml` (every crate
inherits it) and refreshes `Cargo.lock`, regenerates `CHANGELOG.md`, commits
as `chore(release): prepare for <tag>`, and creates an annotated tag whose
message is that release's changelog. The binary reports the new version with
`--version`.

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
**Shared files** workflow runs it on every push and every Monday, so a change
on either side shows up as a failed check. To change one of them, change it in
black-garden-ants first, then copy it here. A project made from this template
can keep the check or delete `scripts/check-shared.sh` and
`.github/workflows/shared.yml` if it means to diverge.

## Layout

| Path | Purpose |
| --- | --- |
| `crates/app/` | The library: everything the program does |
| `crates/app-cli/` | The `app` binary: arguments in, library call, output out |
| `Cargo.toml` | Workspace members, the shared version, and the lint levels |
| `rust-toolchain.toml` | The pinned Rust release, with rustfmt and clippy |
| `rustfmt.toml` | Tabs, and the edition rustfmt uses when the hook calls it directly |
| `mise.toml` | just, lefthook and git-cliff |
| `scripts/tasks.sh` | What every task means. Edit the Rust part to change behaviour. |
| `scripts/check-shared.sh` | The black-garden-ants comparison |
| `release.sh`, `cliff.toml`, `lefthook.yml`, `scripts/commit-msg.sh` | Shared release flow, changelog rules, hooks and commit check |

## Lints

`unsafe_code` is forbidden in every crate, and missing docs, `dbg!`, `todo!`
and `unimplemented!` warn, which `just lint` turns into errors. The library
also denies printing to stdout or stderr: it returns values and the host
decides what to show. A crate that needs `unsafe`, such as an FFI crate, sets
its own `[lints]` table instead of `workspace = true`.

## Known quirks

- The Rust release is pinned in two places that must agree: `channel` in
  `rust-toolchain.toml` and `rust-version` in `Cargo.toml`. The Dependabot
  config here does not touch either; raise them together by hand.
- The edition is in both `Cargo.toml` and `rustfmt.toml`, because the
  pre-commit hook runs rustfmt on staged files without Cargo.
- rustfmt also formats the out-of-line modules a staged file declares, so the
  hook can change a file you did not stage. It stages only what was already
  staged.
- CI does not cache `target/`. The template has no dependencies to cache; add
  caching when a project's build is slow enough to need it.

## Licence

ISC, as set in `Cargo.toml`.
