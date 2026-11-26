"""Run the real Watcher inbox against isolated fake GitHub and model CLIs."""
from contextlib import closing
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

from watcher_process_acceptance import HEAD, db_connection, fake_gh, isolated_network, running


def github():
    root = Path(os.environ['WATCHER_FIXTURE'])
    args = sys.argv[2:]
    if args[:2] == ['api', '--paginate']:
        endpoint = args[-1]
        if endpoint == 'repos/o/r/issues/1/comments?per_page=100':
            print((root / 'remote.json').read_text())
            return 0
        if endpoint in ('repos/o/r/pulls/1/reviews?per_page=100', 'repos/o/r/pulls/1/comments?per_page=100'):
            print('[]')
            return 0
    return fake_gh()


def provider():
    root = Path(os.environ['WATCHER_FIXTURE'])
    stage = (root / 'stage').read_text()
    args = sys.argv[2:]
    prompt = Path(args[args.index('--system-prompt-file') + 1]).read_text()
    message = sys.stdin.read()
    if prompt.startswith('You are Guard'):
        role = 'guard'
        if 'i201' not in message:
            return 98
        result = {'i201': {'inj': False, 'why': 'fixture'}}
    elif prompt.startswith('You are Watcher'):
        role = 'watcher'
        if 'Fixture requested change' not in message:
            return 98
        result = {'act': True, 'change': 'Correct fixture bug', 'draft': '', 'reply': ''}
    else:
        return 99
    (root / f'{stage}-{role}').touch()
    if stage == 'before-guard' and role == 'guard':
        time.sleep(60)  # parent kills the process group at this checkpoint
    if stage == 'provider-failure' and role == 'watcher':
        print('fixture provider failed', file=sys.stderr)
        return 1
    terminal = {'type': 'result', 'subtype': 'success', 'is_error': False, 'result': json.dumps(result), 'usage': {}}
    if role == 'watcher' and stage == 'failed-terminal':
        terminal['is_error'] = True
        terminal['subtype'] = 'error_max_turns'
    print(json.dumps(terminal), flush=True)
    if role == 'watcher' and stage == 'duplicate-terminal':
        print(json.dumps(terminal), flush=True)
    return 1 if role == 'watcher' and stage == 'failed-exit' else 0


def query(root, sql, args=()):
    with closing(db_connection(root)) as db:
        return db.execute(sql, args).fetchall()


def pending(root):
    return query(root, "select state from watcher_inbox where repo='o/r' and num=1 and head=? and comment_id='i201'", (HEAD,)) == [('received',)]


def run_stage(binary, root, env, stage, predicate):
    (root / 'stage').write_text(stage)
    with closing(db_connection(root)) as db, db:
        db.execute('update watcher_retry set next_attempt_at=0')
        db.execute('delete from ev')
    with running(binary, root, env) as process:
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise AssertionError(f'Process exited during {stage}: {process.returncode}')
            if predicate():
                break
            time.sleep(0.1)
        else:
            raise AssertionError(f'{stage} timed out: {query(root, "select w,m from ev order by id desc limit 8")}')
    print(f'F02 inbox restart checkpoint: {stage}', flush=True)


def event(root, text):
    return any(text in row[0] for row in query(root, "select m from ev where w='watcher'"))


def scenario(source_binary):
    with tempfile.TemporaryDirectory(prefix='apollo-inbox-') as directory:
        root = Path(directory)
        binary = root / 'apollo-office'
        shutil.copy2(source_binary, binary)
        (root / 'home').mkdir()
        for name, mode in [('gh', '--fake-gh'), ('claude', '--fake-provider')]:
            cli = root / name
            cli.write_text(f'#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(Path(__file__).resolve()))} {mode} "$@"\n')
            cli.chmod(0o700)
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(root / 'home'), 'WATCHER_FIXTURE': str(root),
               'APOLLO_OFFICE_GH': str(root / 'gh'), 'APOLLO_OFFICE_CLAUDE': str(root / 'claude'),
               'APOLLO_OFFICE_CODEX': '/bin/false', 'APOLLO_OFFICE_OPENCODE': '/bin/false', 'APOLLO_OFFICE_FAST': '1'}
        timestamp = datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
        pr = {'state': 'OPEN', 'mergedAt': None, 'updatedAt': timestamp, 'title': 'Fixture', 'url': 'https://github.com/o/r/pull/1',
              'mergeable': 'MERGEABLE', 'headRefName': 'fix', 'headRepositoryOwner': {'login': 'tester'}, 'headRepository': {'name': 'r'},
              'baseRefName': 'main', 'reviewDecision': 'APPROVED', 'headRefOid': HEAD, 'body': 'Fixture', 'files': [{'path': 'fixture.txt'}], 'changedFiles': 1}
        (root / 'fixture.json').write_text(json.dumps({'accepted': False, 'pr': pr}))
        (root / 'remote.json').write_text(json.dumps([{'id': 201, 'user': {'login': 'maintainer', 'id': 2}, 'author_association': 'MEMBER', 'body': 'Fixture requested change', 'created_at': timestamp, 'updated_at': timestamp}]))
        subprocess.run([str(binary), '--data', str(root / 'data'), 'status'], env=env, cwd=root, check=True, capture_output=True, timeout=20)
        with closing(db_connection(root)) as db, db:
            db.execute("insert into kv(k,v) values('hold',?)", (json.dumps({role: True for role in ('scout', 'vet', 'hunter', 'repro', 'fixer', 'ci', 'reviewer', 'submit')}),))
            db.execute('update hs set on_=0')
            db.execute("insert into rp(full,on_,hunt,auto_merge) values('o/r',1,0,0)")
            db.execute("insert into prs(repo,num,st,upd,d) values('o/r',1,'open',?,?)", (timestamp, json.dumps({'seen': True, 'comment_seen_at': 'previous', 'watcher_head': HEAD})))

        def assert_pending():
            assert pending(root)
            assert query(root, 'select count(*) from cm') == [(0,)]
            assert query(root, 'select count(*) from watcher_outbox') == [(0,)]
            assert query(root, 'select count(*) from watcher_retry') == [(1,)]

        run_stage(binary, root, env, 'before-guard', lambda: pending(root) and (root / 'before-guard-guard').exists())
        assert_pending()
        run_stage(binary, root, env, 'provider-failure', lambda: (root / 'provider-failure-watcher').exists() and event(root, 'processing deferred'))
        assert_pending()
        # A valid JSON body cannot authorize work from a failed/ambiguous CLI run.
        def failed_verdict(stage):
            assert query(root, 'select count(*) from jobs') == [(0,)], f'{stage}: failed verdict admitted a job'
            assert query(root, 'select count(*) from cm') == [(0,)], f'{stage}: failed verdict acknowledged a comment'
            return (root / f'{stage}-watcher').exists() and event(root, 'claude did not complete successfully')

        for stage in ('failed-terminal', 'failed-exit', 'duplicate-terminal'):
            run_stage(binary, root, env, stage, lambda: failed_verdict(stage))
            assert_pending()
        with closing(db_connection(root)) as db, db:
            db.execute("insert into jobs(repo,num,pr,kind,st,title,url,why,d,t0,t1) values('o/r',1,1,'review','approve','Active fixture','url','fixture','{}',1,1)")
        run_stage(binary, root, env, 'active-job', lambda: event(root, 'new review deferred until active review finishes'))
        assert_pending()
        assert query(root, 'select count(*) from jobs') == [(1,)]
        with closing(db_connection(root)) as db, db:
            db.execute('delete from jobs')
            db.execute("create trigger fixture_ack_failure before insert on cm begin select raise(abort,'fixture ack failure'); end")
        run_stage(binary, root, env, 'enqueue-failure', lambda: event(root, 'fixture ack failure'))
        assert_pending()
        assert query(root, 'select count(*) from jobs') == [(0,)]
        with closing(db_connection(root)) as db, db:
            db.execute('drop trigger fixture_ack_failure')
        run_stage(binary, root, env, 'recovery', lambda: query(root, 'select state from watcher_inbox') == [('done',)])
        assert query(root, 'select id from cm') == [('o/r:i201',)]
        assert query(root, 'select count(*) from watcher_retry') == [(0,)]
        assert query(root, "select kind, json_extract(d,'$.source_head'), json_extract(d,'$.source_comments') from jobs") == [('review', HEAD, '["i201"]')]
        assert query(root, 'select count(*) from watcher_outbox') == [(0,)]
        assert not list(root.glob('post-*'))
        # Force a fresh inventory after restart, without changing the receipt or acknowledgement.
        with closing(db_connection(root)) as db, db:
            db.execute("delete from kv where k='ws:o/r#1'")
            db.execute("update prs set d=json_set(d,'$.comment_seen_at','previous') where repo='o/r' and num=1")
        run_stage(binary, root, env, 'replay', lambda: bool(query(root, "select v from kv where k='ws:o/r#1'")))
        assert query(root, 'select count(*) from jobs') == [(1,)]
        assert query(root, 'select state from watcher_inbox') == [('done',)]
        assert query(root, 'select count(*) from cm') == [(1,)]
        assert not list(root.glob('replay-*'))
        assert not list(root.glob('post-*'))


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--fake-gh':
        raise SystemExit(github())
    if len(sys.argv) > 1 and sys.argv[1] == '--fake-provider':
        raise SystemExit(provider())
    isolated_network()
    if len(sys.argv) != 2:
        raise SystemExit('usage: watcher_inbox_acceptance.py /path/to/apollo-office')
    scenario(Path(sys.argv[1]).resolve(strict=True))
