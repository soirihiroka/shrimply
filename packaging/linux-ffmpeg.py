"""Build the desktop FFmpeg libraries and command-line tools from the project submodule."""
import fcntl
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from native_sources import ffmpeg_revision, ffmpeg_source


def main():
    cache = Path(sys.argv[1]).resolve()
    cache.mkdir(parents=True, exist_ok=True)
    options = [
        "--enable-shared", "--disable-static", "--disable-doc", "--disable-debug", "--disable-ffplay",
        "--enable-ffnvcodec", "--enable-nvdec", "--enable-nvenc", "--enable-vaapi", "--enable-libdrm",
        "--enable-vulkan", "--enable-gnutls", "--enable-libopus", "--enable-libmp3lame",
        "--enable-libvorbis", "--enable-libdav1d", "--enable-gpl", "--enable-libx264", "--enable-libx265",
        "--enable-nonfree", "--enable-libfdk-aac",
    ]
    build_env = {k: os.environ.get(k) for k in ("CC", "CFLAGS", "CPPFLAGS", "LDFLAGS", "PKG_CONFIG_PATH")}
    key = hashlib.sha256(repr((ffmpeg_revision(), options, build_env, str(cache))).encode()).hexdigest()
    prefix = cache / key
    with (cache / ".build.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if not (prefix / ".complete").is_file():
            source = ffmpeg_source(cache)
            with tempfile.TemporaryDirectory(dir=cache) as build:
                subprocess.run([source / "configure", f"--prefix={prefix}", *options], cwd=build, check=True)
                subprocess.run(["make", f"-j{len(os.sched_getaffinity(0))}", "install"], cwd=build, check=True)
            (prefix / ".complete").touch()
        link = cache / ".current"
        link.unlink(missing_ok=True)
        link.symlink_to(prefix.name)
        link.replace(cache / "install")


if __name__ == "__main__":
    main()
