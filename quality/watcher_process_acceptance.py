"""Exercise the actual Watcher process across an ambiguous reply and restart.

Run the built Linux binary in a disposable network namespace without credentials.
The fake GitHub CLI models the external side effect, not the Watcher logic.
"""
from contextlib import closing, contextmanager
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import uuid

HEAD = "a" * 40
BODY = "Fixture reply\n\n<!-- apollo-office-reply:fixture -->"


def fake_gh():
    root = Path(os.environ["WATCHER_FIXTURE"])
    args = sys.argv[2:]
    fixture = json.loads((root / "fixture.json").read_text())
    if args[:2] == ["api", "rate_limit"]:
        print(5000)
        return 0
    if args[:2] == ["api", "user"]:
        print(json.dumps({"login": "tester", "id": 1, "name": "Tester"}))
        return 0
    if args == ["api", "repos/o/r"]:
        print(json.dumps({"full_name": "o/r", "default_branch": "main", "permissions": {"push": True}}))
        return 0
    if args[:2] == ["pr", "view"] and args[2:5] == ["1", "-R", "o/r"]:
        print(json.dumps(fixture["pr"]))
        return 0
    if args == ["pr", "comment", "1", "-R", "o/r", "--body-file", "-"]:
        body = sys.stdin.read()
        # Count even unexpected payloads: a failed local claim must never POST.
        if body != BODY:
            (root / ("post-" + uuid.uuid4().hex)).touch()
            return 98
        # Every POST attempt has its own record; no fixture-side deduplication.
        if fixture["accepted"]:
            temporary = root / (uuid.uuid4().hex + ".tmp")
            temporary.write_text(json.dumps([{"id": 101, "user": {"login": "tester"}, "body": body}]))
            temporary.replace(root / "remote.json")
        (root / ("post-" + uuid.uuid4().hex)).touch()
        # The caller cannot determine whether the external effect occurred.
        print("fixture: connection lost after POST", file=sys.stderr)
        return 1
    if args[:2] == ["api", "--paginate"] and args[-1] == "repos/o/r/issues/1/comments?per_page=100":
        print((root / "remote.json").read_text())
        return 0
    print("unsupported fixture CLI call", file=sys.stderr)
    return 99


def isolated_network():
    interfaces = {line.split(":", 1)[0].strip() for line in Path("/proc/net/dev").read_text().splitlines()[2:]}
    if interfaces != {"lo"} or os.geteuid() == 0:
        raise RuntimeError("Requires a non-root process in a loopback-only network namespace")


def db_connection(root):
    return sqlite3.connect(root / "data/office.db", timeout=5)


def read_state(root):
    with closing(db_connection(root)) as db:
        return db.execute("select state,updated from watcher_outbox where id='fixture'").fetchone()


def wait_for(root, process, predicate, label):
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f"Apollo exited during {label}: {process.returncode}")
        row = read_state(root)
        if predicate(row):
            return row
        time.sleep(0.1)
    raise AssertionError(f"Timeout during {label}; last outbox state: {read_state(root)}")


@contextmanager
def running(binary, root, env):
    with (root / "process.log").open("ab") as log:
        process = subprocess.Popen([str(binary), "--data", str(root / "data"), "--port", "0", "--quiet"], env=env, cwd=root, stdout=log, stderr=log, start_new_session=True)
        try:
            yield process
        finally:
            # Abrupt process termination, no graceful SQLite shutdown.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=10)


def scenario(source_binary, accepted, cursor_restart=False):
    with tempfile.TemporaryDirectory(prefix="apollo-watcher-") as directory:
        root = Path(directory)
        binary = root / "apollo-office"
        shutil.copy2(source_binary, binary)
        home = root / "home"
        home.mkdir()
        gh = root / "gh"
        gh.write_text(f"#!{sys.executable}\nimport runpy,sys\nsys.argv.insert(1, '--fake-gh')\nrunpy.run_path({str(Path(__file__).resolve())!r}, run_name='__main__')\n")
        gh.chmod(0o700)
        env = {"PATH": "/usr/bin:/bin", "HOME": str(home), "WATCHER_FIXTURE": str(root),
               "APOLLO_OFFICE_GH": str(gh), "APOLLO_OFFICE_FAST": "1", "APOLLO_OFFICE_CLAUDE": "/bin/false",
               "APOLLO_OFFICE_CODEX": "/bin/false", "APOLLO_OFFICE_OPENCODE": "/bin/false"}
        timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        pr = {"number": 1, "baseRefOid": "b" * 40, "state": "OPEN", "mergedAt": None, "updatedAt": timestamp, "title": "Fixture", "url": "https://github.com/o/r/pull/1",
              "mergeable": "MERGEABLE", "headRefName": "fix", "headRepositoryOwner": {"login": "tester"}, "headRepository": {"name": "r"},
              "baseRefName": "main", "reviewDecision": "APPROVED", "headRefOid": HEAD}
        (root / "fixture.json").write_text(json.dumps({"accepted": accepted, "pr": pr}))
        (root / "remote.json").write_text("[]")
        subprocess.run([str(binary), "--data", str(root / "data"), "status"], env=env, cwd=root, check=True, capture_output=True, timeout=20)
        now = int(time.time() * 1000)
        with closing(db_connection(root)) as db, db:
            db.execute("insert into kv(k,v) values('hold',?)", (json.dumps({role: True for role in ("scout", "guard", "vet", "hunter", "repro", "fixer", "ci", "reviewer", "submit")}),))
            db.execute("update hs set on_=0")
            db.execute("insert into rp(full,on_,hunt,auto_merge) values('o/r',1,0,0)")
            db.execute("insert into prs(repo,num,st,upd,d) values('o/r',1,'open',?,?)", (timestamp, json.dumps({"seen": True, "comment_seen_at": timestamp, "watcher_head": HEAD})))
            db.execute("insert into watcher_outbox(id,repo,num,author,body,state,created,updated,source_head) values('blocked','o/r',1,'tester','must not post','pending',?,?,?)", (now, now - 1, HEAD))
            db.execute("create trigger blocked_claim before update on watcher_outbox when old.id='blocked' and new.state='uncertain' begin select raise(ignore); end")
            if cursor_restart:
                db.executemany("insert into watcher_outbox(id,repo,num,author,body,state,created,updated,source_head) values(?,'o/r',1,'tester','must not post','pending',?,?,?)",
                               [(f"blocked-{i:02}", now, now - 1, HEAD) for i in range(1, 20)])
                db.execute("create trigger blocked_rotation before update on watcher_outbox when old.id like 'blocked%' begin select raise(ignore); end")
        if cursor_restart:
            key = "watcher:delivery-cursor:pending:tester"
            def checkpoint_ready(_):
                with closing(db_connection(root)) as db:
                    row = db.execute("select v from kv where k=?", (key,)).fetchone()
                return row is not None and json.loads(row[0]) == [now - 1, "blocked-19"]
            with running(binary, root, env) as process:
                wait_for(root, process, checkpoint_ready, "first batch checkpoint")
            assert not list(root.glob("post-*"))
            with closing(db_connection(root)) as db, db:
                assert db.execute("select count(*) from watcher_outbox where state='pending' and updated=?", (now - 1,)).fetchone() == (20,)
                db.execute("create table cursor_acceptance_audit (v text not null)")
                db.execute("create trigger audit_cursor after update on kv when new.k='watcher:delivery-cursor:pending:tester' begin insert into cursor_acceptance_audit(v) values(new.v); end")
        with closing(db_connection(root)) as db, db:
            db.execute("insert into watcher_outbox(id,repo,num,author,body,state,created,updated,source_head) values('fixture','o/r',1,'tester',?,'pending',?,?,?)", (BODY, now, now, HEAD))
        with running(binary, root, env) as process:
            wait_for(root, process, lambda row: row[0] == "uncertain" and any(root.glob("post-*")) and (not accepted or json.loads((root / "remote.json").read_text())), "ambiguous POST")
        assert len(list(root.glob("post-*"))) == 1
        if cursor_restart:
            with closing(db_connection(root)) as db:
                first = db.execute("select v from cursor_acceptance_audit order by rowid limit 1").fetchone()
                assert first is not None and json.loads(first[0]) == [now, "fixture"], first
                assert db.execute("select count(*) from watcher_outbox where id like 'blocked%' and state='pending'").fetchone() == (20,)
        # Make the existing 10-minute reconciliation window due; do not alter state.
        with closing(db_connection(root)) as db, db:
            assert db.execute("update watcher_outbox set updated=0 where id='fixture' and state='uncertain'").rowcount == 1
        with running(binary, root, env) as process:
            wanted = "sent" if accepted else "uncertain"
            wait_for(root, process, lambda row: row[0] == wanted and row[1] > 0, "restart reconciliation")
        assert read_state(root)[0] == wanted
        with closing(db_connection(root)) as db:
            assert db.execute("select state from watcher_outbox where id='blocked'").fetchone() == ("pending",)
        assert len(list(root.glob("post-*"))) == 1
        print(f"Watcher restart: accepted={accepted}, cursor_restart={cursor_restart}, final={wanted}, POST attempts=1", flush=True)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--fake-gh":
        raise SystemExit(fake_gh())
    isolated_network()
    if len(sys.argv) != 2:
        raise SystemExit("usage: watcher_process_acceptance.py /path/to/apollo-office")
    for accepted in (True, False):
        scenario(Path(sys.argv[1]).resolve(strict=True), accepted)
    scenario(Path(sys.argv[1]).resolve(strict=True), True, cursor_restart=True)
