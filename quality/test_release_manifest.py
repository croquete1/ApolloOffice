import base64
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from scripts.release_manifest import EXPECTED_ASSETS, manifest, verify_staged


# Independent contract: changing the production constant must not change fixtures.
REQUIRED_BINARIES = frozenset({
    "apollo-office-windows-x64.exe",
    "apollo-office-windows-arm64.exe",
    "apollo-office-linux-x64",
    "apollo-office-linux-arm64",
    "apollo-office-macos-arm64",
    "apollo-office-macos-x64",
})


class ReleaseManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in REQUIRED_BINARIES:
            (self.root / name).write_bytes(name.encode())

    def test_inventory_contract(self):
        self.assertEqual(EXPECTED_ASSETS, REQUIRED_BINARIES)

    def test_complete_inventory_and_hashes(self):
        result = manifest(self.root, "v0.10.15")
        self.assertEqual(result["v"], "0.10.15")
        self.assertEqual(result["files"], {
            name: hashlib.sha256(name.encode()).hexdigest()
            for name in REQUIRED_BINARIES
        })

    def test_missing_asset_rejected(self):
        (self.root / "apollo-office-linux-x64").unlink()
        with self.assertRaisesRegex(ValueError, "missing"):
            manifest(self.root, "v0.10.15")

    def test_unexpected_asset_rejected(self):
        (self.root / "apollo-office-unexpected").write_bytes(b"unexpected")
        with self.assertRaisesRegex(ValueError, "unexpected"):
            manifest(self.root, "v0.10.15")

    def test_empty_asset_rejected(self):
        (self.root / "apollo-office-linux-x64").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "invalid release asset"):
            manifest(self.root, "v0.10.15")

    def test_invalid_tag_rejected(self):
        with self.assertRaisesRegex(ValueError, "invalid release tag"):
            manifest(self.root, "0.10.15")

    def test_directory_rejected(self):
        path = self.root / "apollo-office-linux-x64"
        path.unlink()
        path.mkdir()
        with self.assertRaisesRegex(ValueError, "invalid release asset"):
            manifest(self.root, "v0.10.15")

    def stage_metadata(self):
        (self.root / "latest.json").write_text(json.dumps(manifest(self.root, "v0.10.15")))
        (self.root / "latest.json.sig").write_bytes(base64.b64encode(bytes(64)))

    def test_staged_release_matches_manifest(self):
        self.stage_metadata()
        verify_staged(self.root, "v0.10.15")

    def test_staged_changed_binary_rejected(self):
        self.stage_metadata()
        (self.root / "apollo-office-linux-x64").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "manifest does not match"):
            verify_staged(self.root, "v0.10.15")

    def test_staged_invalid_signature_rejected(self):
        self.stage_metadata()
        (self.root / "latest.json.sig").write_bytes(base64.b64encode(b"short"))
        with self.assertRaisesRegex(ValueError, "signature length"):
            verify_staged(self.root, "v0.10.15")

    def test_staged_unexpected_metadata_rejected(self):
        self.stage_metadata()
        (self.root / "extra").write_bytes(b"extra")
        with self.assertRaisesRegex(ValueError, "invalid staged release inventory"):
            verify_staged(self.root, "v0.10.15")
