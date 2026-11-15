#!/usr/bin/env python3
"""Build the already-locked L1 solver with Android's NDK C++ runtime."""
import argparse
import fcntl
import hashlib
from pathlib import Path
import runpy
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ndk", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--abi", choices=("arm64-v8a", "x86_64"), default="arm64-v8a")
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    repository = Path(__file__).resolve().parent.parent
    native = runpy.run_path(str(repository / "packaging/android-native.py"))
    with (repository / "Cargo.lock").open("rb") as lock:
        dependency = next(package for package in tomllib.load(lock)["package"] if package["name"] == "highs-sys")
    root = args.cache.resolve() / args.abi / args.profile
    root.mkdir(parents=True, exist_ok=True)
    with (root / ".lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        prefix = root / "install"
        stamp = root / "highs.sha256"
        identity = hashlib.sha256(Path(__file__).read_bytes() + dependency["checksum"].encode() + (args.ndk / "source.properties").read_bytes()).hexdigest()
        if stamp.exists() and stamp.read_text() == identity and (prefix / "lib/libhighs.a").is_file():
            print(f"Android L1 solver is current: {prefix}", flush=True)
            return
        sources = args.cache.resolve() / "sources"
        sources.mkdir(exist_ok=True)
        name = f"highs-sys-{dependency['version']}"
        directory = native["source"](sources, name, f"https://static.crates.io/crates/highs-sys/{name}.crate", dependency["checksum"])
        native["cmake_build"](args, root, "highs", directory / "HiGHS", [
            "-DBUILD_CXX_EXE=OFF", "-DBUILD_EXAMPLES=OFF", "-DBUILD_TESTING=OFF",
            "-DFAST_BUILD=ON", "-DZLIB=OFF", "-DCMAKE_INTERPROCEDURAL_OPTIMIZATION=OFF",
        ])
        # highs-sys discovers this instead of using its Linux libstdc++ fallback.
        (prefix / "lib/pkgconfig/highs.pc").write_text(f"prefix={prefix}\nlibdir=${{prefix}}/lib\nincludedir=${{prefix}}/include/highs\nName: highs\nDescription: HiGHS linear programming solver\nVersion: {dependency['version']}\nLibs: -L${{libdir}} -lhighs\nLibs.private: -lc++_shared -lm\nCflags: -I${{includedir}}\n")
        stamp.write_text(identity)


if __name__ == "__main__":
    main()
