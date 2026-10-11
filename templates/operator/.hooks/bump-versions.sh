#!/bin/sh
# cog.toml pre_bump_hooks entry — keeps Cargo.toml's [workspace.package]
# version and the Helm chart's in lockstep with the tag cocogitto is about
# to create. cog stages nothing by itself here, so the edited files are
# added explicitly to land inside cog's bump commit.
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

# The chart's own version and appVersion, never its dependencies'.
for chart in deploy/charts/*/Chart.yaml; do
    sed -i \
        -e "s/^version: .*/version: ${VERSION}/" \
        -e "s/^appVersion: .*/appVersion: \"${VERSION}\"/" \
        "$chart"
    git add "$chart"
done
