"""Exercise the manual fork publisher's trust gate without publishing anything."""
import os
import platform
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(platform.system() == "Linux" and platform.machine() == "x86_64" and shutil.which("openssl"), "Linux x64 publisher requires OpenSSL")
class ManualReleaseTrustTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.key = self.make_key("ED25519", "signing.pem")
        self.trusted = self.raw_public(self.key)
        self.calls = self.root / "external.calls"
        for name in ("git", "gh", "cargo", "node"):
            command = self.root / name
            command.write_text('#!/bin/bash\nprintf "%s\\n" "$0" >> "$TEST_CALLS"\nexit 99\n')
            command.chmod(0o755)

    def make_key(self, algorithm, name):
        key = self.root / name
        subprocess.run(["openssl", "genpkey", "-algorithm", algorithm, "-out", str(key)], check=True, capture_output=True)
        key.chmod(0o600)
        return key

    def raw_public(self, key):
        return subprocess.check_output(["openssl", "pkey", "-in", str(key), "-pubout", "-outform", "DER"])[-32:].hex()

    def run_gate(self, trusted=None, key=None, repo=True):
        if repo:
            (self.root / "Cargo.toml").write_text("[package]\nversion = \"0.10.15\"\n")
            (self.root / "src").mkdir(exist_ok=True)
            (self.root / "src/upd.rs").touch()
        env = dict(os.environ, PATH=str(self.root) + os.pathsep + os.environ["PATH"],
                   APOLLO_SIGNING_KEY_FILE=str(key or self.key), TEST_CALLS=str(self.calls))
        env.pop("APOLLO_OFFICE_UPDATE_PUBKEY", None)
        if trusted is not None:
            env["APOLLO_OFFICE_UPDATE_PUBKEY"] = trusted
        result = subprocess.run(["bash", str(ROOT / "scripts/publish-signed-linux.sh")], cwd=self.root,
                                env=env, capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.calls.exists(), "trust gate must precede Git/build/publication commands")
        return result.stdout + result.stderr

    def test_missing_trusted_key_rejected(self):
        self.assertIn("APOLLO_OFFICE_UPDATE_PUBKEY", self.run_gate())

    def test_malformed_trusted_key_rejected(self):
        self.assertIn("APOLLO_OFFICE_UPDATE_PUBKEY", self.run_gate("not-a-key"))

    def test_different_ed25519_key_rejected(self):
        other = self.make_key("ED25519", "other.pem")
        self.assertIn("não corresponde", self.run_gate(self.raw_public(other)))

    def test_wrong_algorithm_rejected(self):
        other = self.make_key("X25519", "exchange.pem")
        self.assertIn("Ed25519", self.run_gate(self.raw_public(other), other))

    def test_matching_key_reaches_next_guard(self):
        for trusted in (self.trusted, self.trusted.upper()):
            with self.subTest(trusted=trusted):
                self.assertIn("Execute na raiz do fork", self.run_gate(trusted, repo=False))


class UpstreamWorkflowContractTests(unittest.TestCase):
    def test_upstream_jobs_are_restricted_to_upstream_repository(self):
        # Static contract, not execution of GitHub's expression evaluator.
        source = (ROOT / ".github/workflows/release.yml").read_text()
        jobs = source.split("\njobs:\n", 1)[1]
        for job in ("build", "sign", "publish"):
            with self.subTest(job=job):
                match = re.search(rf"^  {job}:\n((?:^    .*\n|^\n)*)", jobs, re.MULTILINE)
                self.assertIsNotNone(match)
                self.assertIn("    if: github.repository == 'Silvertree2010/ApolloOffice'\n", match.group(1))


if __name__ == "__main__":
    unittest.main()
