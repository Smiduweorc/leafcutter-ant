#!/usr/bin/env sh
#
# Checks that the files this repo shares with black-garden-ants still match
# its src/core byte for byte, so the release flow, changelog rules, hooks and
# commit check cannot drift the way copied templates have before.
# .github/workflows/shared.yml runs it. To change one of these files, change
# it in black-garden-ants first, then copy it here.
#
# scripts/tasks.sh is black-garden-ants' tasks.head.sh, then this repo's Rust
# tasks, then tasks.tail.sh with the runner filled in, so only its head and
# tail are compared.
#
# Usage: scripts/check-shared.sh [ref]     ref defaults to master
#
# No `set -e`: every check runs, and the exit status is the verdict.

set -u

cd "$(dirname "$0")/.." || exit 1

ref="${1:-master}"
base="https://raw.githubusercontent.com/Smiduweorc/black-garden-ants/$ref/src/core"

files="
.editorconfig
.github/ISSUE_TEMPLATE/bug-report.yml
.github/ISSUE_TEMPLATE/config.yml
.github/ISSUE_TEMPLATE/feature-request.yml
cliff.toml
lefthook.yml
release.sh
scripts/commit-msg.sh
"

tmp="$(mktemp -d)" || exit 1
trap 'rm -rf "$tmp"' EXIT

fail=0
ok() { printf '  ok    %s\n' "$*"; }
bad() { printf '  FAIL  %s\n' "$*"; fail=1; }

# Each file is a few kilobytes; the timeout only stops a hung connection from
# holding the job until GitHub's six-hour limit.
fetch() {
	curl -fsSL --retry 3 --max-time 30 -o "$2" "$base/$1" || {
		printf 'error: could not fetch %s\n' "$base/$1" >&2
		exit 2
	}
}

echo "Comparing with black-garden-ants@$ref"

for f in $files; do
	fetch "$f" "$tmp/expected"
	if [ ! -f "$f" ]; then
		bad "$f is missing here"
	elif cmp -s "$tmp/expected" "$f"; then
		ok "$f"
	else
		bad "$f differs"
	fi
done

fetch scripts/tasks.head.sh "$tmp/head"
fetch scripts/tasks.tail.sh "$tmp/tail.raw"
# build.sh in black-garden-ants fills these in for the just runner.
sed -e 's|<RUNNER>|just|g' -e 's|<RELEASE_EXAMPLE>|just release v1.2.3|g' \
	"$tmp/tail.raw" >"$tmp/tail"

head -c "$(wc -c <"$tmp/head")" scripts/tasks.sh >"$tmp/our-head"
tail -c "$(wc -c <"$tmp/tail")" scripts/tasks.sh >"$tmp/our-tail"
if cmp -s "$tmp/head" "$tmp/our-head"; then
	ok "scripts/tasks.sh head"
else
	bad "scripts/tasks.sh head differs from tasks.head.sh"
fi
if cmp -s "$tmp/tail" "$tmp/our-tail"; then
	ok "scripts/tasks.sh tail"
else
	bad "scripts/tasks.sh tail differs from tasks.tail.sh"
fi

if [ "$fail" = 0 ]; then
	echo "All shared files match."
else
	echo "Shared files have drifted. Change them in black-garden-ants first, then copy them here." >&2
fi
exit "$fail"
