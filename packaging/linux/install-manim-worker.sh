#!/bin/sh
set -eu

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    printf 'usage: %s PREFIX\n' "$0" >&2
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
resources=${1%/}/share/shrimply
mkdir -p -- "$resources"
rm -rf -- "$resources/$worker" "$resources/external/manim"
tar -C "$resources" -xf "$archive"
