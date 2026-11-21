#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ -z "$1" ]; then
    printf 'usage: %s PREFIX [--prefix FFMPEG_PREFIX | --wheel PYAV_WHEEL]\n' "$0" >&2
    exit 1
fi

source_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
worker=crates/media/visual/manim/manim-bridge/python
for file in "$worker/pyproject.toml" "$worker/uv.lock" external/manim/manimlib/__init__.py; do
    if [ ! -f "$source_root/$file" ]; then
        printf 'Missing Manim worker resource: %s\n' "$source_root/$file" >&2
        exit 1
    fi
done

archive=$(mktemp)
trap 'rm -f "$archive"' 0
tar -C "$source_root" \
    --exclude=.git --exclude=.venv --exclude=__pycache__ --exclude='*.pyc' \
    -cf "$archive" "$worker" external/manim
prefix=${1%/}
resources=$prefix/share/shrimply
mkdir -p -- "$resources"
rm -rf -- "$resources/$worker" "$resources/external/manim"
tar -C "$resources" -xf "$archive"
shift
if [ "$#" -gt 0 ]; then
    if [ "$1" = --prefix ]; then
        set -- "$@" --runtime-libs "$prefix/lib/shrimply/ffmpeg"
    fi
    python3 "$source_root/packaging/bundle-pyav.py" "$source_root/$worker" "$resources/$worker" "$@"
fi
