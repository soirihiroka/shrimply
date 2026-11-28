#!/usr/bin/env python3
"""Archive rust-skia's compiled libraries and bindings for a CI cache."""
import os
from pathlib import Path
import sys
import tarfile


def main(build_directory: Path, archive: Path) -> None:
    bindings = [
        *build_directory.glob("skia-bindings/*/out/skia/bindings.rs"),
        *build_directory.glob("skia-bindings-*/out/skia/bindings.rs"),
    ]
    if len(bindings) != 1:
        print("::notice::Expected one Skia bindings directory; skipping the binary cache.")
        return

    build = bindings[0].parent
    libraries = sorted(
        path for path in build.iterdir()
        if path.is_file() and path.suffix in {".a", ".lib"}
    )
    if not libraries:
        print("::notice::No Skia libraries were found; skipping the binary cache.")
        return

    files = [*libraries, bindings[0]]
    if (icu := build / "icudtl.dat").is_file():
        files.append(icu)
    archive.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "w:gz", compresslevel=3, dereference=True) as bundle:
        for path in files:
            bundle.add(path, arcname=f"skia-binaries/{path.name}")
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        print("ready=true", file=output)


if __name__ == "__main__":
    build_directory, archive = map(Path, sys.argv[1:])
    main(build_directory, archive)
