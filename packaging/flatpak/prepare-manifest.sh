#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 3 ]]; then
    echo "usage: $0 OUTPUT SOURCE_ARCHIVE GIT_HASH" >&2
    exit 1
fi
if [[ ! $3 =~ ^[[:xdigit:]]{40}$ ]]; then
    echo "Flatpak build Git hash must be a full commit SHA" >&2
    exit 1
fi

source_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source_archive="$(realpath -- "$2")"
test -f "$source_archive"
template="$source_root/dev.shrimply.Shrimply.yml"
grep -Fq '@SHRIMPLY_SOURCE@' "$template"
grep -Fq '@SHRIMPLY_GIT_HASH@' "$template"

# Escape YAML single-quoted paths, then sed replacement characters.
source_archive="$(printf '%s' "$source_archive" | sed -e "s/'/''/g" -e 's/[\\&|]/\\&/g')"
source_root="$(printf '%s' "$source_root" | sed -e "s/'/''/g" -e 's/[\\&|]/\\&/g')"
temporary="$(mktemp "$1.XXXXXX")"
trap 'rm -f -- "$temporary"' EXIT
sed \
    -e "s|@SHRIMPLY_SOURCE@|$source_archive|g" \
    -e "s|@SHRIMPLY_GIT_HASH@|$3|g" \
    -e "s|path: packaging/flatpak/\(.*\)|path: '$source_root/packaging/flatpak/\1'|" \
    "$template" > "$temporary"
if grep -Eq '@SHRIMPLY_(SOURCE|GIT_HASH)@' "$temporary"; then
    echo "Flatpak manifest contains unresolved source metadata" >&2
    exit 1
fi
mv -- "$temporary" "$1"
