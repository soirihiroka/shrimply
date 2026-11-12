"""Regression coverage for formatting a path dependency's foreign workspace."""

import importlib.util
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "format_workspace", Path(__file__).resolve().parents[1] / "format_workspace.py"
)
formatter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(formatter)


class WorkspaceFormattingTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.manifest = self.root / "Cargo.toml"
        self.manifest.write_text('[workspace]\nmembers=["crates/*"]\nexclude=["external/vtracer"]\n')
        self.package("crates/app", "app")
        self.package("crates/core", "core")
        foreign = self.package("external/vtracer/bench", "vtracer-bench")
        (self.root / "external/vtracer/Cargo.toml").write_text('[workspace]\nmembers=["bench"]\n')
        (self.root / ".gitmodules").write_text('[submodule "external/vtracer"]\npath=external/vtracer\n')
        # A dependency belongs to a separate workspace; it must never become a format target.
        with (self.root / "crates/app/Cargo.toml").open("a") as source:
            source.write('[dependencies]\nvtracer-bench={path="../../external/vtracer/bench"}\n')
        self.foreign = foreign / "src/lib.rs"
        self.foreign.write_text("pub fn untouched( ){ }\n")

    def package(self, path, name):
        directory = self.root / path
        (directory / "src").mkdir(parents=True)
        (directory / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2024"\n')
        (directory / "src/lib.rs").write_text("pub fn local() {}\n")
        return directory

    def test_local_dependencies_are_not_selected(self):
        self.assertEqual(formatter.package_flags(self.manifest), ["--package", "app", "--package", "core"])

    def test_accidentally_declared_submodule_is_rejected(self):
        self.manifest.write_text('[workspace]\nmembers=["crates/*","external/vtracer/bench"]\n')
        with self.assertRaisesRegex(ValueError, "Git submodule"):
            formatter.package_flags(self.manifest)

    def test_quoted_submodule_paths_are_rejected(self):
        self.manifest.write_text('[workspace]\nmembers=["crates/*","external/vtracer/bench"]\n')
        (self.root / ".gitmodules").write_text('[submodule "external/vtracer"]\npath="external/vtracer"\n')
        with self.assertRaisesRegex(ValueError, "Git submodule"):
            formatter.package_flags(self.manifest)

    def test_workspace_excludes_are_honored(self):
        self.manifest.write_text('[workspace]\nmembers=["crates/*"]\nexclude=["crates/core"]\n')
        self.assertEqual(formatter.package_flags(self.manifest), ["--package", "app"])

    def test_symlink_into_a_submodule_is_rejected(self):
        try:
            (self.root / "crates/foreign").symlink_to(self.root / "external/vtracer/bench", target_is_directory=True)
        except OSError as error:
            self.skipTest(f"symlink creation unavailable: {error}")
        with self.assertRaisesRegex(ValueError, "Git submodule"):
            formatter.package_flags(self.manifest)

    def test_make_format_targets_use_the_workspace_guard(self):
        repository = Path(__file__).resolve().parents[2]
        for workspace in (repository, repository / "sanim"):
            for target in ("fmt", "fmt-check"):
                command = subprocess.run(
                    ["make", "--no-print-directory", "-n", target, "RUSTC=true"],
                    cwd=workspace, check=True, capture_output=True, text=True,
                ).stdout
                self.assertIn("format_workspace.py", command)
                self.assertNotIn("cargo fmt", command)

    def test_runner_explicitly_selects_packages_without_all(self):
        before = self.foreign.read_bytes()
        for mode in [[], ["--check"]]:
            with patch("sys.argv", ["format_workspace.py", "--manifest", str(self.manifest), *mode, "--", "rustup", "run", "nightly", "cargo"]), patch.object(formatter, "package_flags", return_value=["--package", "app", "--package", "core"]), patch.object(formatter.subprocess, "run") as run:
                run.return_value.returncode = 0
                self.assertEqual(formatter.main(), 0)
                command = run.call_args.args[0]
                self.assertEqual(command[:5], ["rustup", "run", "nightly", "cargo", "fmt"])
                self.assertNotIn("--all", command)
                self.assertNotIn("vtracer-bench", command)
                self.assertIn("--package", command)
                self.assertEqual("--check" in command, bool(mode))
        self.assertEqual(before, self.foreign.read_bytes())


if __name__ == "__main__":
    unittest.main()
