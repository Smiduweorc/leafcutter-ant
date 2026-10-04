#!/usr/bin/env sh
#
# Validates a commit message against Conventional Commits, so git-cliff can
# build a changelog from the history. Invoked by the commit-msg git hook.
#
# If commitlint is set up, it's used instead of the hand-rolled regex below:
#
#   - commitlint (Node): a commitlint.config.* or .commitlintrc.{json,js} is
#     present and node_modules/.bin/commitlint is installed.
#   - commitlint-rs: a .commitlintrc, .commitlintrc.json, .commitlintrc.yaml or
#     .commitlintrc.yml is present and the `commitlint` on PATH is
#     commitlint-rs (a Rust project pins it through mise). This script then
#     adds the @commitlint/config-conventional checks commitlint-rs has no
#     rules for. commitlint-rs 0.2 fails to parse a bare .commitlintrc, so use
#     one of the others.
#
# This file is otherwise identical across every template, so a project picks
# up either one just by adding it; nothing here needs to change to notice.
#
# Usage: scripts/commit-msg.sh <path-to-commit-msg-file>

set -eu

file="${1:-}"
[ -n "$file" ] || { echo "usage: scripts/commit-msg.sh <file>" >&2; exit 2; }

if [ -x node_modules/.bin/commitlint ]; then
	for cfg in commitlint.config.js commitlint.config.mjs commitlint.config.cjs \
		commitlint.config.ts .commitlintrc.json .commitlintrc.js; do
		if [ -f "$cfg" ]; then
			exec node_modules/.bin/commitlint --edit "$file"
		fi
	done
fi

# The subject is the first non-blank, non-comment line: git appends its own
# commented help text to the message file.
subject="$(grep -v '^#' "$file" | sed '/^[[:space:]]*$/d' | head -n 1)"

# git writes these itself: merges, reverts, and the fixup!/squash!/amend!
# commits that `git rebase --autosquash` folds away. commitlint skips them too.
case "$subject" in
	Merge\ * | Revert\ * | Reapply\ * | fixup!* | squash!* | amend!*) exit 0 ;;
esac

# commitlint-rs and Node's commitlint both install a binary named commitlint;
# only commitlint-rs prints its package name in --version.
use_commitlint_rs=false
if command -v commitlint >/dev/null 2>&1 &&
	commitlint --version 2>/dev/null | grep -q '^commitlint-rs '; then
	for cfg in .commitlintrc .commitlintrc.json .commitlintrc.yaml .commitlintrc.yml; do
		if [ -f "$cfg" ]; then use_commitlint_rs=true; fi
	done
fi

if [ "$use_commitlint_rs" = true ]; then
	# commitlint-rs takes the first line of its input as the subject, so it
	# gets the message without git's comments and leading blank lines, and
	# without the diff `git commit -v` puts below the scissors line.
	msg="$(sed '/^# -* >8 -*$/,$d' "$file" | grep -v '^#' | sed '/[^[:space:]]/,$!d')"
	printf '%s\n' "$msg" | commitlint

	# @commitlint/config-conventional rules that commitlint-rs cannot express.
	# The subject length is capped below, on both paths.
	case "$subject" in
		[[:space:]]* | *[[:space:]])
			echo "Commit subject must not start or end with whitespace." >&2
			exit 1
			;;
	esac
	# commitlint exempts lines with a URL in them, and so does this.
	long="$(printf '%s\n' "$msg" | awk 'NR > 1 && length($0) > 100 && !/https?:\/\/[^[:space:]]/')"
	if [ -n "$long" ]; then
		printf 'Commit body and footer lines must be at most 100 characters:\n%s\n' "$long" >&2
		exit 1
	fi
	# A warning in commitlint as well, so it does not stop the commit.
	case "$(printf '%s\n' "$msg" | sed -n 2p)" in
		*[![:space:]]*) echo "warning: leave a blank line between the subject and the body." >&2 ;;
	esac
else
	types='feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert'

	if ! printf '%s\n' "$subject" | grep -qE "^($types)(\([a-z0-9._/ -]+\))?!?: .+"; then
		cat >&2 <<'USAGE'
Commit message must be a Conventional Commit.

    <type>[optional scope][!]: <description>

Types: feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert
A "!" before the colon marks a breaking change.

Examples:
    feat: add the widget
    fix(parser): handle an empty input
    refactor!: drop the legacy entry point
USAGE
		printf '\nGot: %s\n' "$subject" >&2
		exit 1
	fi
fi

# commitlint-rs has no rule for the length of the whole subject line, so the
# cap applies to both paths.
if [ "${#subject}" -gt 100 ]; then
	printf 'Commit subject is %s characters; keep it under 100.\n' "${#subject}" >&2
	exit 1
fi
