"""Build the editor's VideoToolbox-enabled FFmpeg with hardware decoding required."""
import fcntl
import hashlib
import os
from pathlib import Path
import platform
import shlex
import subprocess
import sys
import tempfile

from native_sources import ffmpeg_revision, ffmpeg_source


def output(*command):
    return subprocess.check_output(command, text=True).strip()


def main():
    root = Path(sys.argv[1]).resolve()
    prefix = root / "install"
    patch = Path(__file__).resolve().parent / "macos/ffmpeg-hardware.patch"
    # Preserve the Homebrew build's codec and filter choices without carrying
    # its build machine's paths into the editor's dylibs.
    installed = Path(output("brew", "--prefix", "ffmpeg")) / "bin/ffmpeg"
    flags = [flag for flag in shlex.split(output(str(installed), "-hide_banner", "-buildconf"))
             if flag.startswith(("--enable-", "--disable-"))]
    deps = output("brew", "deps", "--installed", "ffmpeg").splitlines()
    brew = Path(output("brew", "--prefix"))
    paths = [brew / "opt" / name for name in deps]
    env = os.environ.copy()
    for key, values in {
        "PKG_CONFIG_PATH": [str(path / "lib/pkgconfig") for path in paths],
        "CPPFLAGS": [f"-I{path}/include" for path in paths],
        "LDFLAGS": [f"-L{path}/lib" for path in paths],
    }.items():
        separator = os.pathsep if key == "PKG_CONFIG_PATH" else " "
        env[key] = separator.join(filter(None, [*values, env.get(key)]))
    key = hashlib.sha256(patch.read_bytes() + Path(__file__).read_bytes() +
                         repr((ffmpeg_revision(), platform.machine(), str(prefix), flags,
                               env.get("MACOSX_DEPLOYMENT_TARGET"), output("xcrun", "--show-sdk-version"),
                               output("brew", "list", "--versions", "ffmpeg", *deps))).encode()).hexdigest()
    root.mkdir(parents=True, exist_ok=True)
    with (root / ".lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        stamp = root / "installed.sha256"
        if stamp.is_file() and stamp.read_text() == key and all((prefix / file).is_file() for file in ("lib/libavcodec.dylib", "bin/ffmpeg")):
            return
        source = ffmpeg_source(root, patch)
        with tempfile.TemporaryDirectory(dir=root) as build:
            subprocess.run([source / "configure", *flags, f"--prefix={prefix}", "--enable-videotoolbox",
                            "--enable-shared", "--disable-static", "--enable-ffmpeg", "--disable-ffplay", "--disable-ffprobe", "--disable-doc"],
                           cwd=build, env=env, check=True)
            subprocess.run(["make", f"-j{os.cpu_count() or 1}", "install"], cwd=build, env=env, check=True)
        stamp.write_text(key)


if __name__ == "__main__":
    main()
