#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 1 ]]; then
    echo "usage: source-archive.sh OUTPUT.tar" >&2
    exit 2
fi

# The Git index is the source manifest, including initialized submodules.
# Docker ignore patterns have different tar semantics: 'cache' also excludes
# legitimate Rust modules such as sanim-lang/src/cache/parsed.rs.
source_root="$(git rev-parse --show-toplevel)"
git -C "$source_root" ls-files --recurse-submodules -z \
    | tar -C "$source_root" --group=0 --no-recursion --null \
        --numeric-owner --owner=0 -cf "$1" -T -
