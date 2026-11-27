"""Crash/reopen acceptance of Watcher SQLite statements, not Rust orchestration.

Uses the SQL embedded in production db.rs; never calls GitHub or a provider.
The subprocess exits without closing SQLite, including with a live transaction.
"""
from contextlib import closing
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / "src/db.rs").read_text(encoding="utf-8")
HEAD = "a" * 40
KEY = "fixture-reply"


def extract(prefix):
    matches = re.findall(r'"(' + re.escape(prefix) + r'[^"\\]*)"', SOURCE)
    if len(set(matches)) != 1:
        raise AssertionError(f"Expected unique production SQL: {prefix}")
    return matches[0]


def connect(path):
    db = sqlite3.connect(path, isolation_level=None, timeout=5)
    db.execute("PRAGMA journal_mode=WAL")
    return db


def seed(path):
    schema = re.search(r'const SCH: &str = "(.*?)";', SOURCE, re.S).group(1)
    with closing(connect(path)) as db:
        db.executescript(schema)
        if "source_head" not in {row[1] for row in db.execute("pragma table_info(watcher_outbox)")}:
            db.execute(extract("alter table watcher_outbox add column source_head"))
        db.execute("insert into rp(full) values('o/r')")
        db.execute("insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)", (json.dumps({"watcher_head": HEAD}),))
        db.execute(extract("insert into watcher_inbox(repo,num,head,comment_id,snapshot,state,received_at,updated_at)"), ("o/r", 1, HEAD, "r1", "t", 1, 1))
        db.execute(extract("insert into watcher_retry(repo,num,head,snapshot,attempts,next_attempt_at)"), ("o/r", 1, HEAD, "t", 1))


def enqueue(db, crash_at=None):
    def checkpoint(stage):
        if crash_at == stage:
            os._exit(73)

    db.execute("BEGIN IMMEDIATE")
    if db.execute(extract("select exists(select 1 from cm where id=? or id=?)"), ("o/r:r1", "r1")).fetchone()[0]:
        db.rollback()
        return False
    checkpoint("after-analysis")
    db.execute(extract("insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1) values(?,?,'review'"), ("o/r", 1, "Fix", "url", "change", "{}", 1, 2, 2))
    checkpoint("after-job")
    db.execute(extract("insert or ignore into watcher_outbox(id,repo,num,author,body,state,created,updated,source_head)"), (KEY, "o/r", 1, "owner", "reply", 2, 2, HEAD, "o/r"))
    checkpoint("after-enqueue")
    db.execute(extract("insert or ignore into cm(id,t) values(?,?)"), ("o/r:r1", 2))
    db.execute(extract("update prs set d=json_set(d, '$.seen', json('true'), '$.comment_seen_at', ?)"), ("t", "o/r", 1))
    checkpoint("after-ack")
    db.execute(extract("update watcher_inbox set state='done',updated_at=? where repo=? and num=? and head=? and comment_id=?"), (2, "o/r", 1, HEAD, "r1"))
    if not db.execute(extract("select exists(select 1 from watcher_inbox where repo=? and num=? and head=? and state='received')"), ("o/r", 1, HEAD)).fetchone()[0]:
        db.execute(extract("delete from watcher_retry where repo=? and num=? and head=?"), ("o/r", 1, HEAD))
        if db.execute(extract("select exists(select 1 from watcher_retry where repo=? and num=? and head=?)"), ("o/r", 1, HEAD)).fetchone()[0]:
            db.rollback()
            raise sqlite3.IntegrityError("Retry cleanup did not persist")
    checkpoint("before-commit")
    db.commit()
    checkpoint("after-commit")
    return True


def claim(db):
    return db.execute(extract("update watcher_outbox set state='uncertain',updated=? where id=? and author=?"), (3, KEY, "owner")).rowcount


def finish(db):
    return db.execute(extract("update watcher_outbox set state='sent',updated=? where id=? and state='uncertain'"), (4, KEY)).rowcount


def worker(path, stage):
    db = connect(path)
    if stage == "before-guard":
        os._exit(73)
    if stage == "after-claim":
        assert claim(db) == 1
        os._exit(73)
    if stage == "after-confirm":
        assert finish(db) == 1
        os._exit(73)
    enqueue(db, stage)
    raise AssertionError(f"Crash checkpoint not reached: {stage}")


class WatcherSqlCrashTests(unittest.TestCase):
    def make_db(self, directory):
        path = Path(directory) / "state.db"
        seed(path)
        return path

    def crash(self, path, stage):
        result = subprocess.run([sys.executable, __file__, "--crash", str(path), stage], capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 73, result.stdout + result.stderr)

    def state(self, db):
        return tuple(db.execute(sql).fetchone()[0] for sql in (
            "select count(*) from jobs", "select count(*) from cm",
            "select count(*) from watcher_outbox", "select state from watcher_inbox",
            "select count(*) from watcher_retry", "select json_extract(d,'$.comment_seen_at') from prs"))

    def test_uncommitted_changes_rollback_after_abrupt_exit(self):
        for stage in ("before-guard", "after-analysis", "after-job", "after-enqueue", "after-ack", "before-commit"):
            with self.subTest(stage=stage), tempfile.TemporaryDirectory() as directory:
                path = self.make_db(directory)
                self.crash(path, stage)
                with closing(connect(path)) as db:
                    self.assertEqual(self.state(db), (0, 0, 0, "received", 1, None))
                    self.assertTrue(enqueue(db))
                    self.assertEqual(self.state(db), (1, 1, 1, "done", 0, "t"))
                    self.assertFalse(enqueue(db))

    def test_ignored_retry_cleanup_rolls_back_entire_transaction(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self.make_db(directory)
            with closing(connect(path)) as db:
                db.execute("create trigger ignore_retry before delete on watcher_retry begin select raise(ignore); end")
                with self.assertRaises(sqlite3.IntegrityError):
                    enqueue(db)
                self.assertEqual(self.state(db), (0, 0, 0, "received", 1, None))
                db.execute("drop trigger ignore_retry")
                self.assertTrue(enqueue(db))
                self.assertEqual(self.state(db), (1, 1, 1, "done", 0, "t"))
                self.assertFalse(enqueue(db))

    def test_commit_survives_exit_and_replay_does_not_enqueue(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self.make_db(directory)
            self.crash(path, "after-commit")
            with closing(connect(path)) as db:
                self.assertEqual(self.state(db), (1, 1, 1, "done", 0, "t"))
                self.assertFalse(enqueue(db))
                self.assertEqual(self.state(db), (1, 1, 1, "done", 0, "t"))

    def test_uncertain_and_sent_replies_cannot_be_claimed_again(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self.make_db(directory)
            with closing(connect(path)) as db:
                self.assertTrue(enqueue(db))
            self.crash(path, "after-claim")
            with closing(connect(path)) as db:
                self.assertEqual(db.execute("select state from watcher_outbox").fetchone()[0], "uncertain")
                self.assertEqual(claim(db), 0)
            self.crash(path, "after-confirm")
            with closing(connect(path)) as db:
                self.assertEqual(db.execute("select state from watcher_outbox").fetchone()[0], "sent")
                self.assertEqual(claim(db), 0)
                self.assertEqual(finish(db), 0)


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "--crash":
        worker(sys.argv[2], sys.argv[3])
    else:
        unittest.main()
