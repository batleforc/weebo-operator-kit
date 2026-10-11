#!/bin/sh
# cog.toml pre_bump_hooks entry — keeps every version in the repo in
# lockstep with the tag cocogitto is about to create: Cargo.toml's
# [workspace.package] version (the crates inherit it),
# packages/operator-docs' package.json (its release tarball is named after
# it), the library chart's version, and the operator template's pins on
# the kit (crates, tasks/ checksums, workflows, library chart). cog stages nothing by itself here, so the edited files are added
# explicitly to land inside cog's bump commit.
set -e

VERSION="$1"
if [ -z "$VERSION" ]; then
    echo "usage: bump-versions.sh <new-version>" >&2
    exit 1
fi

# Only the version line inside [workspace.package] — dependency entries
# elsewhere in Cargo.toml must be left untouched.
awk -v ver="$VERSION" '
    /^\[workspace\.package\]/ { in_pkg = 1 }
    /^\[/ && !/^\[workspace\.package\]/ { in_pkg = 0 }
    in_pkg && /^version = / { print "version = \"" ver "\""; next }
    { print }
' Cargo.toml > Cargo.toml.tmp
mv Cargo.toml.tmp Cargo.toml
# Workspace crates inherit that version; refresh their Cargo.lock entries
# (`--workspace` touches nothing else). Offline when the local registry
# cache can resolve the graph; a clean CI runner needs the index.
cargo update --workspace --offline 2>/dev/null || cargo update --workspace
git add Cargo.toml Cargo.lock

# package.json + its package-lock.json, nothing else (no git tag from npm).
(cd packages/operator-docs && npm version "$VERSION" --no-git-tag-version --allow-same-version >/dev/null)
git add packages/operator-docs/package.json packages/operator-docs/package-lock.json

# The library chart, published under that version.
sed -i -E "s/^version: .*/version: $VERSION/" charts/weebo-operator-lib/Chart.yaml
git add charts/weebo-operator-lib/Chart.yaml

# A new operator starts on the release it was generated from; the tasks/
# checksums are this tree's, the one being tagged.
./templates/pin-kit.sh templates/operator "v$VERSION" .
git add templates/operator
