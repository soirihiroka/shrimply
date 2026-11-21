"""Install the project's PyAV into a packaged Manim worker."""
import argparse
import os
from pathlib import Path
import subprocess
import tomllib
from zipfile import ZipFile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("project", type=Path)
    parser.add_argument("--runtime-libs", type=Path, help="private library directory in a Linux installation")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--prefix", type=Path, help="build against this FFmpeg installation")
    group.add_argument("--wheel", type=Path, help="install an already-built wheel offline")
    args = parser.parse_args()

    lock = tomllib.loads((args.source / "uv.lock").read_text())
    av = next(p for p in lock["package"] if p["name"] == "av")
    if args.wheel:
        if not args.wheel.name.startswith(f"av-{av['version']}-"):
            parser.error(f"{args.wheel.name} does not match locked PyAV {av['version']}")
        with ZipFile(args.wheel) as wheel:
            wheel.extractall(args.project)
    else:
        prefix = args.prefix.resolve()
        if not (prefix / "include/libavcodec/avcodec.h").is_file():
            parser.error(f"Missing FFmpeg development headers in {prefix}")
        env = os.environ.copy()
        for key, directory in (("PKG_CONFIG_PATH", "lib/pkgconfig"), ("INCLUDE", "include"), ("LIB", "lib")):
            env[key] = os.pathsep.join(filter(None, (str(prefix / directory), env.get(key))))
        url = av["source"]["url"] + "#" + av["sdist"]["hash"].replace(":", "=", 1)
        subprocess.run(["uv", "pip", "install", "--python", "3.14", "--target", args.project,
                        "--no-deps", "--link-mode", "copy", "--no-cache", url], env=env, check=True)

    if args.runtime_libs:
        for extension in (args.project / "av").rglob("*.so"):
            relative = os.path.relpath(args.runtime_libs, extension.parent)
            subprocess.run(["patchelf", "--add-rpath", f"$ORIGIN/{relative}", extension], check=True)

    # The source tree installs PyAV through uv. The packaged worker already
    # contains it, and can use the exact same dependency lock completely offline.
    manifest = args.project / "pyproject.toml"
    source = 'default-groups = ["dev", "pyav"]'
    contents = manifest.read_text()
    if source not in contents:
        raise ValueError(f"Missing the PyAV dependency group in {manifest}")
    manifest.write_text(contents.replace(source, 'default-groups = ["dev"]'))


if __name__ == "__main__":
    main()
