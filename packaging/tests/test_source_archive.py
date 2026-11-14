"""Release archives must contain every tracked source, including cache modules."""

from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

ARCHIVER = Path(__file__).resolve().parents[1] / "flatpak/source-archive.sh"


class SourceArchiveTests(unittest.TestCase):
    def test_tracked_modules_survive_docker_ignore_patterns(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            tracked = {
                ".dockerignore": b"cache\ntarget\n**/*.tmp\n",
                "sanim/crates/sanim-lang/src/cache.rs": b"mod parsed;\n",
                "sanim/crates/sanim-lang/src/cache/parsed.rs": b"// source, not generated cache\n",
                "nested/target/source.rs": b"// tracked source\n",
                "space and\nnewline.rs": b"// unusual tracked filename\n",
                "--leading.rs": b"// not a tar option\n",
            }
            for name, content in tracked.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
            subprocess.run(["git", "add", "--all"], cwd=root, check=True)
            (root / "generated.tmp").write_text("untracked build output")
            archive = root / "source.tar"
            subprocess.run(["bash", str(ARCHIVER), str(archive)], cwd=root, check=True)
            with tarfile.open(archive) as files:
                self.assertEqual(set(files.getnames()), set(tracked))
                for name, content in tracked.items():
                    member = files.getmember(name)
                    self.assertEqual(files.extractfile(member).read(), content)
                    self.assertEqual((member.uid, member.gid), (0, 0))

    def test_archive_includes_initialized_submodule_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("app", "dependency"):
                subprocess.run(["git", "init", "-q", str(root / name)], check=True)
            dependency = root / "dependency"
            (dependency / "cache").mkdir()
            (dependency / "cache/lib.rs").write_text("// dependency source\n")
            subprocess.run(["git", "add", "--all"], cwd=dependency, check=True)
            subprocess.run(["git", "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "source"], cwd=dependency, check=True)
            app = root / "app"
            subprocess.run(["git", "-c", "protocol.file.allow=always", "submodule", "add", "-q", str(dependency), "external/dependency"], cwd=app, check=True)
            archive = root / "source.tar"
            subprocess.run(["bash", str(ARCHIVER), str(archive)], cwd=app, check=True)
            with tarfile.open(archive) as files:
                self.assertIn("external/dependency/cache/lib.rs", files.getnames())
                self.assertNotIn("external/dependency/.git", files.getnames())


if __name__ == "__main__":
    unittest.main()
