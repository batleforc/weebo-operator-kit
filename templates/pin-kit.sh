#!/bin/sh
# Pins every reference an operator repository makes to weebo-operator-kit
# onto one release:
#   - Cargo.toml: the kit crates' `tag = "vX.Y.Z"`;
#   - Taskfile.yaml: the remote includes of tasks/ (URL tag and `checksum:`,
#     the sha256 Task verifies), and its `KIT_TAG:` variable (the pin of a
#     repository without a kit crate);
#   - .github/workflows/: the kit's reusable workflows (`@vX.Y.Z`);
#   - deploy/charts/*/Chart.yaml: the weebo-operator-lib dependency version
#     (Chart.lock follows with `helm dependency update`);
#   - docs/package.json: the @weebo/operator-docs release tarball
#     (package-lock.json follows with `npm install --package-lock-only`).
#
# Usage: pin-kit.sh <repo dir> <vX.Y.Z> [kit checkout]
# The checksums come from the kit checkout's tasks/ when given (it must be
# at that tag, or be the tree about to be tagged), else from the tag's URLs.
# Run by `task kit:sync` (templates/kit.yaml) in an operator, and by the
# kit's .hooks/bump-versions.sh on templates/operator.
set -eu

dir="$1"
tag="$2"
kit_dir="${3:-}"
version="${tag#v}"
repo=batleforc/weebo-operator-kit
raw="https://raw.githubusercontent.com/$repo"

if [ -f "$dir/Cargo.toml" ]; then
    sed -i -E "s|(git = \"https://github.com/$repo\", tag = \")[^\"]*\"|\1$tag\"|" "$dir/Cargo.toml"
fi

for f in "$dir"/.github/workflows/*.yml "$dir"/.github/workflows/*.yaml; do
    [ -f "$f" ] || continue
    sed -i -E "s|($repo/\.github/workflows/[A-Za-z0-9_.-]+@)[^ ]+|\1$tag|" "$f"
done

for f in "$dir"/deploy/charts/*/Chart.yaml; do
    [ -f "$f" ] || continue
    awk -v v="$version" '
        /^ *- name: weebo-operator-lib *$/ { lib = 1 }
        lib && /^ +version: / { sub(/version: .*/, "version: " v); lib = 0 }
        { print }
    ' "$f" > "$f.tmp"
    mv "$f.tmp" "$f"
done

pkg="$dir/docs/package.json"
if [ -f "$pkg" ]; then
    sed -i -E "s|(\"@weebo/operator-docs\": \"https://github.com/$repo/releases/download/)[^/]+/weebo-operator-docs-[^\"]+\.tgz\"|\1$tag/weebo-operator-docs-$version.tgz\"|" "$pkg"
fi

tf="$dir/Taskfile.yaml"
if [ -f "$tf" ]; then
    sed -i -E "s|($raw/)[^/]+(/tasks/)|\1$tag\2|" "$tf"
    sed -i -E "s|^( +KIT_TAG: *\"?)v[0-9]+\.[0-9]+\.[0-9]+|\1$tag|" "$tf"
    # name=sha256 of every tasks/ file the Taskfile includes.
    sums=""
    names="$(sed -n -E "s|.*$raw/[^/]+/tasks/([A-Za-z0-9_-]+)\.yaml.*|\1|p" "$tf" | sort -u)"
    for name in $names; do
        if [ -n "$kit_dir" ]; then
            sum="$(sha256sum "$kit_dir/tasks/$name.yaml" | cut -d' ' -f1)"
        else
            sum="$(curl -fsSL "$raw/$tag/tasks/$name.yaml" | sha256sum | cut -d' ' -f1)"
        fi
        sums="$sums $name=$sum"
    done
    # The `checksum:` line following each include's `taskfile:` line.
    awk -v sums="$sums" -v raw="$raw" '
        BEGIN { n = split(sums, kv, " "); for (i = 1; i <= n; i++) { split(kv[i], p, "="); sum[p[1]] = p[2] } }
        index($0, raw "/") && /\/tasks\/[A-Za-z0-9_-]+\.yaml/ {
            name = $0; sub(/.*\/tasks\//, "", name); sub(/\.yaml.*/, "", name); pending = name
        }
        pending != "" && /^ +checksum: / { sub(/checksum: .*/, "checksum: " sum[pending]); pending = "" }
        { print }
    ' "$tf" > "$tf.tmp"
    mv "$tf.tmp" "$tf"
fi

echo "$dir pinned on weebo-operator-kit $tag"
