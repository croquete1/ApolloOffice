"""Actual daemon restart acceptance for Steward's durable branch-update observation.

Seed a committed attempt, not a remote PUT. Run the release binary in an
isolated non-root network namespace. The fake CLI supplies remote reads and
records any forbidden merge/update attempt before returning.
"""
from contextlib import closing
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

from watcher_process_acceptance import db_connection, isolated_network, running

OLD = "a" * 40
BASE = "b" * 40
HEAD = "c" * 40
KEY = "steward:update:o/r#1"
ERROR = "steward:merge_error:o/r"


def fake_gh():
    root = Path(os.environ["STEWARD_FIXTURE"])
    args = sys.argv[2:]
    stage = (root / "stage").read_text()
    fixture = json.loads((root / "fixture.json").read_text())

    def record(kind):
        (root / f"{stage}-{kind}-{uuid.uuid4().hex}").touch()

    def out(value):
        print(json.dumps(value))
        return 0

    if args[:2] == ["api", "rate_limit"]:
        print(5000)
        return 0
    if args[:2] == ["api", "user"]:
        if "--jq" in args:
            print("tester")
            return 0
        return out({"login": "tester", "id": 1, "name": "Tester"})
    if args == ["api", "repos/o/r"]:
        return out({"full_name": "o/r", "default_branch": "main", "permissions": {"push": True}})
    if any(arg.endswith(("/update-branch", "/merge")) for arg in args):
        record("mutation")
        return 98
    if args == ["api", "repos/o/r/pulls/1"]:
        return out(fixture["rest"])
    if len(args) == 2 and args[0] == "api" and args[1].startswith("repos/o/r/pulls?state=open&"):
        return out([fixture["rest"]])
    if args[:4] == ["pr", "list", "-R", "o/r"]:
        return out([fixture["detail"]])
    if args[:5] == ["pr", "view", "1", "-R", "o/r"]:
        return out(fixture["detail"])
    if args == ["api", f"repos/o/r/compare/{OLD}...{HEAD}"]:
        record("old-compare")
        if stage == "failure":
            print("fixture: GitHub unavailable", file=sys.stderr)
            return 1
        return out({"status": "ahead", "base_commit": {"sha": OLD},
                    "merge_base_commit": {"sha": OLD}, "ahead_by": 1, "behind_by": 0})
    if args == ["api", f"repos/o/r/compare/{BASE}...{HEAD}"]:
        record("base-compare")
        return out({"status": "ahead", "base_commit": {"sha": BASE},
                    "merge_base_commit": {"sha": BASE}, "ahead_by": 1, "behind_by": 0})
    print("unsupported fixture CLI call", file=sys.stderr)
    return 99


def read_value(root, key):
    with closing(db_connection(root)) as db:
        row = db.execute("select v from kv where k=?", (key,)).fetchone()
    return json.loads(row[0]) if row else None


def wait_for(root, process, predicate, label):
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f"Apollo exited during {label}: {process.returncode}")
        if predicate():
            return
        time.sleep(0.1)
    raise AssertionError(f"Timeout during {label}: record={read_value(root, KEY)}, "
                         f"merge_error={read_value(root, ERROR)}, "
                         f"scan_error={read_value(root, 'steward:error:o/r')}")


def scenario(source_binary):
    with tempfile.TemporaryDirectory(prefix="apollo-steward-") as directory:
        root = Path(directory)
        binary = root / "apollo-office"
        shutil.copy2(source_binary, binary)
        home = root / "home"
        home.mkdir()
        gh = root / "gh"
        gh.write_text(f"#!{sys.executable}\nimport runpy,sys\nsys.path.insert(0, {str(Path(__file__).resolve().parent)!r})\nsys.argv.insert(1, '--fake-gh')\nrunpy.run_path({str(Path(__file__).resolve())!r}, run_name='__main__')\n")
        gh.chmod(0o700)
        env = {"PATH": "/usr/bin:/bin", "HOME": str(home), "STEWARD_FIXTURE": str(root),
               "APOLLO_OFFICE_GH": str(gh), "APOLLO_OFFICE_FAST": "1",
               "APOLLO_OFFICE_CLAUDE": "/bin/false", "APOLLO_OFFICE_CODEX": "/bin/false",
               "APOLLO_OFFICE_OPENCODE": "/bin/false"}
        timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        rest = {"number": 1, "state": "open", "draft": False, "updated_at": timestamp,
                "head": {"sha": HEAD, "ref": "fix", "repo": {"full_name": "o/r"}},
                "base": {"sha": BASE, "ref": "main", "repo": {"full_name": "o/r"}}}
        # Reconciliation runs before candidate selection. Deliberately keep
        # merge ineligible, even if the observed ancestry is valid.
        detail = {"number": 1, "title": "Fixture", "body": "", "url": "https://github.com/o/r/pull/1",
                  "isDraft": False, "headRefOid": HEAD, "baseRefName": "main",
                  "baseRefOid": BASE, "createdAt": timestamp, "updatedAt": timestamp,
                  "mergeStateStatus": "BLOCKED", "reviewDecision": "REVIEW_REQUIRED",
                  "statusCheckRollup": [], "files": []}
        (root / "fixture.json").write_text(json.dumps({"rest": rest, "detail": detail}))
        (root / "stage").write_text("failure")
        subprocess.run([str(binary), "--data", str(root / "data"), "status"],
                       env=env, cwd=root, check=True, capture_output=True, timeout=20)
        with closing(db_connection(root)) as db, db:
            roles = ("scout", "guard", "vet", "hunter", "repro", "fixer", "ci", "reviewer", "submit")
            db.execute("insert into kv(k,v) values('hold',?)", (json.dumps(dict.fromkeys(roles, True)),))
            db.execute("update hs set on_=0")
            db.execute("insert into rp(full,on_,hunt,iss,auto_merge,local_merge) values('o/r',1,0,0,1,0)")
            activation = db.execute("select merge_activation from rp where full='o/r'").fetchone()[0]
            assert len(activation) == 32
            previous = {"head": OLD, "base": BASE, "at": 100, "status": "uncertain",
                        "merge_activation": activation, "error": "fixture: ambiguous prior attempt"}
            db.execute("insert into kv(k,v) values(?,?)", (KEY, json.dumps(previous, separators=(",", ":"), sort_keys=True)))
        with running(binary, root, env) as process:
            wait_for(root, process, lambda: read_value(root, ERROR) is not None and
                     any(root.glob("failure-old-compare-*")), "failed reconciliation")
        assert read_value(root, KEY) == previous
        assert not list(root.glob("failure-base-compare-*"))

        (root / "stage").write_text("recover")
        with running(binary, root, env) as process:
            wait_for(root, process, lambda: read_value(root, KEY).get("status") == "observed",
                     "restart recovery")
        observed = read_value(root, KEY)
        assert observed["observed_head"] == HEAD
        assert observed["head"] == OLD and observed["base"] == BASE
        assert observed["merge_activation"] == activation and observed["at"] == 100
        assert len(list(root.glob("recover-old-compare-*"))) == 1
        assert len(list(root.glob("recover-base-compare-*"))) == 1

        (root / "stage").write_text("replay")
        before = read_value(root, "steward:o/r")["at"]
        with running(binary, root, env) as process:
            # Two distinct complete inventories: the second report proves
            # the previous scan (including reconciliation) returned.
            wait_for(root, process, lambda: read_value(root, "steward:o/r")["at"] > before,
                     "first replay scan")
            first = read_value(root, "steward:o/r")["at"]
            wait_for(root, process, lambda: read_value(root, "steward:o/r")["at"] > first,
                     "second replay scan")
        assert read_value(root, KEY) == observed
        assert not list(root.glob("replay-*-compare-*"))
        assert not list(root.glob("*-mutation-*"))
        print("Steward daemon restart: uncertain preserved, observed after recovery, replay without comparisons or mutation", flush=True)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--fake-gh":
        raise SystemExit(fake_gh())
    isolated_network()
    if len(sys.argv) != 2:
        raise SystemExit("usage: steward_process_acceptance.py /path/to/apollo-office")
    scenario(Path(sys.argv[1]).resolve(strict=True))
