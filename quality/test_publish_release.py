"""Exercise the release promotion shell script without GitHub writes."""
import base64
import json
import os
from pathlib import Path
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
elif command == "release view":
    print("false" if mode == "not-draft" else "true")
elif command == "release download":
    dst = Path(args[args.index("--dir") + 1])
    for item in Path(os.environ["FAKE_DIST"]).iterdir():
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
        (self.dist / "latest.json.sig").write_bytes(base64.b64encode(bytes(64)))
        gh = self.root / "gh"
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        self.log = self.root / "commands.log"

    def run_publish(self, mode):
        env = dict(os.environ, PATH=str(self.root) + os.pathsep + os.environ["PATH"],
                   FAKE_MODE=mode, FAKE_LOG=str(self.log), FAKE_DIST=str(self.dist),
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
