"""Exercise real prebuilt installs and updates without touching editor state."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SOURCE = Path(__file__).resolve().parents[1]


class InstallTests(unittest.TestCase):
    def install(self, directory):
        return subprocess.run(
            [sys.executable, str(SOURCE / "install.py"), "--directory", str(directory)],
            capture_output=True,
            text=True,
        )

    def test_update_replaces_complete_adapter_and_preserves_zed_owned_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            extensions = Path(temporary)
            installed = extensions / "installed"
            unrelated = installed / "other"
            unrelated.mkdir(parents=True)
            (unrelated / "sentinel").write_text("keep")
            index = extensions / "index.json"
            index.write_text("Zed owns this index")
            first = self.install(installed)
            self.assertEqual(first.returncode, 0, first.stderr)
            target = installed / "sanim"
            self.assertTrue((target / "languages/sanim/config.toml").is_file())
            (target / "extension.wasm").write_bytes(b"old version")
            second = self.install(installed)
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertEqual((target / "extension.wasm").read_bytes(), (SOURCE / "extension.wasm").read_bytes())
            self.assertEqual((unrelated / "sentinel").read_text(), "keep")
            self.assertEqual(index.read_text(), "Zed owns this index")
            self.assertEqual(list((extensions / "staging").iterdir()), [])

    def test_unrelated_extension_at_destination_is_rejected_without_modification(self):
        with tempfile.TemporaryDirectory() as temporary:
            installed = Path(temporary) / "installed"
            target = installed / "sanim"
            target.mkdir(parents=True)
            (target / "extension.toml").write_text('id = "someone-else"\n')
            (target / "extension.wasm").write_bytes(b"keep")
            result = self.install(installed)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("refusing to replace unrelated extension", result.stderr)
            self.assertEqual((target / "extension.wasm").read_bytes(), b"keep")
