"""Verified source snapshots for native dependency builds."""
import hashlib
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request


FFMPEG = Path(__file__).resolve().parents[1] / "external/ffmpeg"


def source(cache, name, url, digest):
    import fcntl

    archive = cache / url.rsplit("/", 1)[1]
    directory = cache / name
    with (cache / f".{name}.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if not archive.exists():
            with tempfile.TemporaryDirectory(dir=cache) as staging:
                temporary = Path(staging) / archive.name
                with urllib.request.urlopen(url, timeout=60) as response, temporary.open("wb") as output:
                    shutil.copyfileobj(response, output)
                verify_archive(temporary, digest)
                temporary.replace(archive)
        else:
            verify_archive(archive, digest)
        if not directory.is_dir():
            with tempfile.TemporaryDirectory(prefix="extract-", dir=cache) as staging:
                with tarfile.open(archive) as compressed:
                    compressed.extractall(staging, filter="data")
                (Path(staging) / name).rename(directory)
    return directory


def verify_archive(archive, digest):
    with archive.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != digest:
            raise RuntimeError(f"checksum mismatch for {archive.name}")


def ffmpeg_revision():
    repo = FFMPEG.parents[1]
    setup = "Run git submodule update --init external/ffmpeg."
    if not (FFMPEG / "configure").is_file():
        raise RuntimeError(f"Missing FFmpeg source in {FFMPEG}. {setup}")
    if (FFMPEG / ".git").exists():
        revision = subprocess.check_output(["git", "-C", FFMPEG, "rev-parse", "HEAD"], text=True).strip()
        if (repo / ".git").exists():
            pinned = subprocess.check_output(["git", "-C", repo, "rev-parse", ":external/ffmpeg"], text=True).strip()
            if revision != pinned:
                raise RuntimeError(f"FFmpeg checkout {revision} differs from the project gitlink {pinned}. {setup}")
        return revision
    if (repo / ".git").exists():
        raise RuntimeError(f"FFmpeg submodule is not initialized. {setup}")

    # Exported source archives contain the submodule files without a Git database.
    identity = hashlib.sha256()
    for file in sorted(FFMPEG.rglob("*")):
        if file.is_file():
            identity.update(file.relative_to(FFMPEG).as_posix().encode() + b"\0")
            with file.open("rb") as stream:
                identity.update(hashlib.file_digest(stream, "sha256").digest())
    return identity.hexdigest()


def ffmpeg_source(cache, *patches):
    import fcntl

    revision = ffmpeg_revision()
    identity = hashlib.sha256(revision.encode())
    for patch in patches:
        identity.update(hashlib.sha256(patch.read_bytes()).digest())
    directory = cache / f"ffmpeg-{identity.hexdigest()}"
    with (cache / ".ffmpeg.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if not directory.is_dir():
            with tempfile.TemporaryDirectory(prefix="ffmpeg-", dir=cache) as staging:
                extracted = Path(staging) / "source"
                if (FFMPEG / ".git").exists():
                    extracted.mkdir()
                    with tempfile.TemporaryFile() as archive:
                        subprocess.run(["git", "-C", FFMPEG, "archive", revision], stdout=archive, check=True)
                        archive.seek(0)
                        with tarfile.open(fileobj=archive) as tar:
                            tar.extractall(extracted, filter="data")
                else:
                    shutil.copytree(FFMPEG, extracted)
                # version.sh otherwise finds the enclosing editor checkout's tag.
                shutil.copyfile(extracted / "RELEASE", extracted / "FF_VERSION")
                for patch in patches:
                    subprocess.run(["patch", "-F", "0", "-s", "-f", "-p1", "-i", str(patch.resolve())],
                                   cwd=extracted, check=True)
                extracted.rename(directory)
    return directory
