#!/usr/bin/env bash
#
# Cuts a release: runs the flavor's preflight checks, bumps any version-bearing
# manifest, regenerates CHANGELOG.md, commits, and creates an annotated tag
# whose message is the changelog for the new version.
#
# Usage:
#   ./release.sh v1.2.3      explicit version
#   ./release.sh patch       bump patch from the latest tag
#   ./release.sh minor
#   ./release.sh major
#
# Env:
#   PUSH=1            push the branch and tag when done (default: 0, local only)
#   RELEASE_BRANCH    branch releases must be cut from (default: master)
#
# This file is identical in every template. Everything language-specific lives
# behind scripts/tasks.sh, so it never needs editing per project.

set -euo pipefail

RELEASE_BRANCH="${RELEASE_BRANCH:-master}"
PUSH="${PUSH:-0}"

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# git-cliff may come from PATH (mise) or from node_modules (npm stack).
if command -v git-cliff >/dev/null 2>&1; then
	cliff() { git-cliff "$@"; }
elif [ -x node_modules/.bin/git-cliff ]; then
	cliff() { node_modules/.bin/git-cliff "$@"; }
else
	die "git-cliff not found. Run the 'setup' task first."
fi

arg="${1:-}"
[ -n "$arg" ] || die "usage: ./release.sh <vX.Y.Z|major|minor|patch>"

latest="$(git describe --tags --abbrev=0 2>/dev/null || echo v0.0.0)"

case "$arg" in
	major | minor | patch)
		base="${latest#v}"
		base="${base%%-*}"
		IFS='.' read -r major minor patch <<<"$base"
		case "$arg" in
			major) major=$((major + 1)); minor=0; patch=0 ;;
			minor) minor=$((minor + 1)); patch=0 ;;
			patch) patch=$((patch + 1)) ;;
		esac
		tag="v${major}.${minor}.${patch}"
		;;
	v*)
		tag="$arg"
		;;
	*)
		die "version must be vX.Y.Z or one of: major, minor, patch"
		;;
esac

if ! [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
	die "tag must be of the form v[X.Y.Z] (got '$tag')"
fi

version="${tag#v}"

branch="$(git rev-parse --abbrev-ref HEAD)"
[ "$branch" = "$RELEASE_BRANCH" ] || die "must release from '$RELEASE_BRANCH' (on '$branch')"

# Refuse to release from a dirty tree so the release commit holds only the
# changelog and the version bump.
[ -z "$(git status --porcelain)" ] || die "working tree is dirty; commit or stash first"

! git rev-parse "$tag" >/dev/null 2>&1 || die "tag '$tag' already exists"

echo "==> Releasing $tag (previous: $latest)"

echo "==> Preflight"
scripts/tasks.sh preflight

# The flavor bumps whatever manifest it has, if any, and prints the paths it
# touched, one per line, so they can be staged. Go prints nothing: the tag is
# the version.
bumped_paths="$(scripts/tasks.sh bump-version "$version")"
files=(CHANGELOG.md)
if [ -n "$bumped_paths" ]; then
	mapfile -t bumped <<<"$bumped_paths"
	echo "==> Bumped to $version:"
	printf '      %s\n' "${bumped[@]}"
	files+=("${bumped[@]}")
fi

echo "==> Updating CHANGELOG.md"
cliff --config cliff.toml --tag "$tag" --output CHANGELOG.md

git add -- "${files[@]}"
git commit -q -m "chore(release): prepare for $tag"

# The release commit itself is skipped by the cliff.toml commit parsers, so
# --unreleased yields exactly the commits going into this tag.
export GIT_CLIFF_TEMPLATE="\
	{% for group, commits in commits | group_by(attribute=\"group\") %}
	{{ group | upper_first }}\
	{% for commit in commits %}
		- {% if commit.breaking %}(breaking) {% endif %}{{ commit.message | upper_first }} ({{ commit.id | truncate(length=7, end=\"\") }})\
	{% endfor %}
	{% endfor %}"
changelog="$(cliff --config cliff.toml --unreleased --strip all)"

echo "==> Tagging $tag"
git tag -a "$tag" -m "Release $tag" -m "$changelog"

if [ "$PUSH" = "1" ]; then
	echo "==> Pushing $branch and $tag"
	git push origin "$branch"
	git push origin "$tag"
	echo "==> Done."
else
	echo "==> Done (local only). Push with:"
	echo "      git push origin $branch && git push origin $tag"
fi
