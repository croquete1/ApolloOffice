"""Exercise the release promotion shell script without GitHub writes."""
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from scripts.release_manifest import EXPECTED_ASSETS, manifest


ROOT = Path(__file__).resolve().parents[1]
FAKE_GH = """#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shutil
import sys

args = sys.argv[1:]
mode = os.environ["FAKE_MODE"]
command = " ".join(args[:2]) if args[0] == "release" else args[0]
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write(command + "\\n")
if command == "api":
    if mode == "api-fail":
        sys.exit(1)
    print(json.dumps([{"tag_name": "v0.10.15", "draft": False}] if mode == "existing" else []))
elif command == "release create":
    assert "--draft" in args and "--verify-tag" in args
    if mode == "upload-fail":
        sys.exit(1)
    remote = Path(os.environ["FAKE_REMOTE"])
    remote.mkdir(exist_ok=True)
    for arg in args[3:]:
        item = Path(arg)
        if item.parent == Path(os.environ["FAKE_DIST"]) and item.is_file():
            if mode == "omitted" and item.name == "apollo-office-linux-x64":
                continue
            shutil.copyfile(item, remote / item.name)
elif command == "release view":
    print("false" if mode == "not-draft" else "true")
elif command == "release download":
    dst = Path(args[args.index("--dir") + 1])
    for item in Path(os.environ["FAKE_REMOTE"]).iterdir():
        shutil.copyfile(item, dst / item.name)
    if mode == "changed":
        (dst / "apollo-office-linux-x64").write_bytes(b"changed")
elif command == "release edit":
    assert "--draft=false" in args
else:
    sys.exit(99)
"""


class PublishReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.dist = self.root / "dist"
        self.dist.mkdir()
        for name in EXPECTED_ASSETS:
            (self.dist / name).write_bytes(name.encode())
        (self.dist / "latest.json").write_text(json.dumps(manifest(self.dist, "v0.10.15")))
        key = self.root / "key.pem"
        raw = self.root / "signature.raw"
        subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(key)], check=True)
        der = subprocess.check_output(["openssl", "pkey", "-in", str(key), "-pubout", "-outform", "DER"])
        self.public_key = der[-32:].hex()
        subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", str(key), "-rawin",
                        "-in", str(self.dist / "latest.json"), "-out", str(raw)], check=True)
        (self.dist / "latest.json.sig").write_bytes(base64.b64encode(raw.read_bytes()))
        gh = self.root / "gh"
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        self.log = self.root / "commands.log"

    def run_publish(self, mode, public_key=None):
        shutil.rmtree(self.root / "remote", ignore_errors=True)
        env = dict(os.environ, PATH=str(self.root) + os.pathsep + os.environ["PATH"],
                   FAKE_MODE=mode, FAKE_LOG=str(self.log), FAKE_DIST=str(self.dist),
                   FAKE_REMOTE=str(self.root / "remote"),
                   APOLLO_OFFICE_UPDATE_PUBKEY=self.public_key if public_key is None else public_key,
                   GITHUB_REF_NAME="v0.10.15", GITHUB_REPOSITORY="example/fixture")
        result = subprocess.run(["bash", str(ROOT / "scripts/publish_release.sh"), str(self.dist)],
                                env=env, capture_output=True, text=True, timeout=15)
        calls = self.log.read_text().splitlines() if self.log.exists() else []
        return result.returncode, calls

    def test_complete_draft_is_promoted(self):
        status, calls = self.run_publish("ok")
        self.assertEqual(status, 0)
        self.assertEqual(calls, ["api", "release create", "release view",
                                 "release download", "release edit"])

    def test_existing_release_and_api_failure_are_fail_closed(self):
        for mode in ("existing", "api-fail"):
            with self.subTest(mode=mode):
                self.log.unlink(missing_ok=True)
                status, calls = self.run_publish(mode)
                self.assertNotEqual(status, 0)
                self.assertEqual(calls, ["api"])

    def test_failed_upload_and_verification_never_promote(self):
        for mode in ("upload-fail", "not-draft", "changed"):
            with self.subTest(mode=mode):
                self.log.unlink(missing_ok=True)
                status, calls = self.run_publish(mode)
                self.assertNotEqual(status, 0)
                self.assertIn("release create", calls)
                self.assertNotIn("release edit", calls)

    def test_invalid_staging_never_calls_github(self):
        (self.dist / "apollo-office-linux-x64").unlink()
        status, calls = self.run_publish("ok")
        self.assertNotEqual(status, 0)
        self.assertEqual(calls, [])


    def test_missing_uploaded_asset_never_promotes(self):
        status, calls = self.run_publish("omitted")
        self.assertNotEqual(status, 0)
        self.assertIn("release download", calls)
        self.assertNotIn("release edit", calls)

    def test_untrusted_or_missing_key_never_calls_github(self):
        for public_key in ("", "00" * 32):
            with self.subTest(public_key=public_key):
                status, calls = self.run_publish("ok", public_key)
                self.assertNotEqual(status, 0)
                self.assertEqual(calls, [])

    def test_tampered_signature_never_calls_github(self):
        (self.dist / "latest.json.sig").write_bytes(base64.b64encode(bytes(64)))
        status, calls = self.run_publish("ok")
        self.assertNotEqual(status, 0)
        self.assertEqual(calls, [])
