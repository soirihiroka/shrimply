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
source_time="$(git -C "$source_root" show -s --format=%ct "$3")"
source_archive="$(realpath -- "$2")"
test -f "$source_archive"
template="$source_root/dev.shrimply.Shrimply.yml"
grep -Fq '@SHRIMPLY_SOURCE@' "$template"
grep -Fq '@SHRIMPLY_GIT_HASH@' "$template"

# A local archive gives flatpak-builder a content-addressed cache entry for the submodule.
output_dir="$(dirname -- "$(realpath -m -- "$1")")"
temporary="$(mktemp "$output_dir/manifest.XXXXXX")"
ffmpeg_tar="$(mktemp "$output_dir/ffmpeg.XXXXXX")"
trap 'rm -f -- "$temporary" "$ffmpeg_tar"' EXIT
revision="$(git -C "$source_root" rev-parse :external/ffmpeg)"
git -C "$source_root/external/ffmpeg" archive --format=tar "$revision" > "$ffmpeg_tar"
ffmpeg_name="ffmpeg-$(sha256sum "$ffmpeg_tar" | cut -d' ' -f1).tar"
mv -- "$ffmpeg_tar" "$output_dir/$ffmpeg_name"

# Escape YAML single-quoted paths, then sed replacement characters.
source_archive="$(printf '%s' "$source_archive" | sed -e "s/'/''/g" -e 's/[\\&|]/\\&/g')"
source_root="$(printf '%s' "$source_root" | sed -e "s/'/''/g" -e 's/[\\&|]/\\&/g')"
sed \
    -e "s|@SHRIMPLY_SOURCE@|$source_archive|g" \
    -e "s|@SHRIMPLY_GIT_HASH@|$3|g" \
    -e "s|@SHRIMPLY_SOURCE_TIME@|$source_time|g" \
    -e "s|path: packaging/flatpak/\(.*\)|path: '$source_root/packaging/flatpak/\1'|" \
    -e "s|@SHRIMPLY_FFMPEG@|$ffmpeg_name|g" \
    "$template" > "$temporary"
if grep -Eq '@SHRIMPLY_(SOURCE|GIT_HASH|SOURCE_TIME|FFMPEG)@' "$temporary"; then
    echo "Flatpak manifest contains unresolved source metadata" >&2
    exit 1
fi
mv -- "$temporary" "$1"
