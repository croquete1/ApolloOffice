use rusqlite::{params_from_iter, types::ValueRef, Connection, ToSql};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>);

#[derive(Debug, PartialEq, Eq)]
pub enum HunterFinding {
    Queued,
    Duplicate,
    Revoked,
}

pub enum ProviderAdmission {
    Ready,
    Probe(String),
    Blocked,
}

/// Ownership of one persisted analysis attempt. Dropping it leaves the
/// retry deadline intact. A crash leaves the ticket for startup recovery.
pub struct ReviewAttempt {
    db: Db,
    repo: String,
    number: i64,
    ticket: String,
}

impl ReviewAttempt {
    fn matches(
        &self,
        tx: &rusqlite::Transaction<'_>,
        db: &Db,
        repo: &str,
        number: i64,
        updated: &str,
        head: &str,
    ) -> rusqlite::Result<bool> {
        if !Arc::ptr_eq(&self.db.0, &db.0) || self.repo != repo || self.number != number {
            return Ok(false);
        }
        tx.query_row(
            "select exists(select 1 from watcher_retry where repo=? and num=? and head=? and snapshot=? and analysis_ticket=?)",
            rusqlite::params![repo, number, head, updated, self.ticket],
            |row| row.get(0),
        )
    }

    fn release_ticket(&self) -> rusqlite::Result<()> {
        let mut c = self.db.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "update watcher_retry set analysis_ticket=null where repo=? and num=? and analysis_ticket=?",
            rusqlite::params![self.repo, self.number, self.ticket],
        )?;
        let retained: bool = tx.query_row(
            "select exists(select 1 from watcher_retry where repo=? and num=? and analysis_ticket=?)",
            rusqlite::params![self.repo, self.number, self.ticket],
            |row| row.get(0),
        )?;
        if retained {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()
    }

    pub(crate) fn release(mut self) -> rusqlite::Result<()> {
        self.release_ticket()?;
        self.ticket.clear();
        Ok(())
    }
}

impl Drop for ReviewAttempt {
    fn drop(&mut self) {
        if self.ticket.is_empty() {
            return;
        }
        if let Err(error) = self.release_ticket() {
            eprintln!("watcher attempt release failed; restart recovery required: {error}");
        }
    }
}

const SCH: &str = "
create table if not exists kv(k text primary key, v text);
create table if not exists ses(id text primary key, exp int);
create table if not exists lnk(c text primary key, exp int);
create table if not exists rp(id integer primary key, full text unique, iss int default 1, hunt int default 0, ci text default '', setup text default '', rules text default '', mode text default 'ask', merge int default 1, maxo int default 3, maxd int default 5, ai int default 1, on_ int default 1, t0 int, coderabbit_review int default 0, coderabbit_repair int default 0, coderabbit_epoch int default 0, coderabbit_review_since int default 0, coderabbit_free_confirmed int default 0);
create table if not exists hs(id text primary key, ssh text default '', dk int default 0, ln int default 1, ci int default 1, gb int default 50, raw int default 0, on_ int default 1, t0 int);
create table if not exists jobs(id integer primary key autoincrement, repo text, num int, kind text default 'fix', title text, url text, st text, why text default '', d text default '{}', pr int, prio real default 0, t0 int, t1 int);
create index if not exists jobs_st on jobs(st);
create table if not exists job_schedule(job integer primary key, manual_priority integer check(manual_priority between 0 and 3), retry_after integer not null default 0, retry_reason text not null default '', admitted_at integer not null default 0);
create table if not exists job_dependencies(job integer not null, prerequisite integer not null, primary key(job,prerequisite), check(job<>prerequisite));
create index if not exists job_dependencies_prerequisite on job_dependencies(prerequisite);
create index if not exists jobs_issue_latest on jobs(repo, num, kind, id desc);
create table if not exists ev(id integer primary key, t int, w text, m text, j int);
create table if not exists prs(repo text, num int, st text, title text, url text, upd text, merged text, d text default '{}', t0 int, primary key(repo, num));
create table if not exists github_issues(repo text not null, num int not null, title text not null, url text not null, author text default '', updated text default '', labels text default '[]', assignees text default '[]', seen_at int not null default 0, is_open int not null default 1, skip_reason text default '', primary key(repo, num));
create index if not exists github_issues_open on github_issues(repo,is_open,updated);
create table if not exists tok(day text, w text, tin int default 0, tout int default 0, primary key(day, w));
create table if not exists seen(repo text, num int, upd text, primary key(repo, num));
create table if not exists cm(id text primary key, t int);
create table if not exists watcher_inbox(repo text not null,num integer not null,head text not null,comment_id text not null,snapshot text not null,state text not null check(state in ('received','done','superseded')),received_at integer not null,updated_at integer not null,primary key(repo,num,head,comment_id));
create table if not exists watcher_retry(repo text not null,num integer not null,head text not null,snapshot text not null,attempts integer not null,next_attempt_at integer not null,analysis_ticket text,primary key(repo,num));
create index if not exists watcher_inbox_pending on watcher_inbox(repo,num,state,updated_at);
create table if not exists watcher_outbox(id text primary key, repo text not null, num int not null, author text not null, body text not null, state text not null check(state in ('pending','uncertain','sent','cancelled')), created int not null, updated int not null);
create index if not exists watcher_outbox_pending on watcher_outbox(state,updated);
create index if not exists watcher_outbox_pr on watcher_outbox(repo,num,created);
create table if not exists cost(id integer primary key, t int, day text, job int, repo text default '', w text, md text, sub int default 0, tin int default 0, tcw int default 0, tcr int default 0, tout int default 0, usd real);
create index if not exists cost_day on cost(day);
create index if not exists cost_job on cost(job);
create table if not exists fix_attempts(id integer primary key, job int not null, attempt int not null, model text not null, started int not null, finished int, elapsed_ms int, outcome text not null default 'running', gate text not null default '');
create index if not exists fix_attempts_started on fix_attempts(started);
create index if not exists fix_attempts_job on fix_attempts(job, attempt);
create table if not exists coderabbit_requests(repo text not null, number int not null, head text not null, status text not null, requested_at int not null, activation int not null, comment_id int, error text default '', primary key(repo,number,head));
create index if not exists coderabbit_requests_recent on coderabbit_requests(requested_at,status);
create table if not exists coderabbit_findings(repo text not null, number int not null, comment_id int not null, review_id int not null, head text not null, state text not null, severity text not null, body_hash text not null, activation int not null, job int, created_at int not null, updated_at int not null, guard_ticket text, primary key(repo,number,comment_id,head));
create index if not exists coderabbit_findings_pr on coderabbit_findings(repo,number,head,state);
create table if not exists coderabbit_cycles(repo text not null, number integer not null, cycles integer not null check(cycles>=0), primary key(repo,number));
";

const SCHEMA_VERSION: i64 = 9;

pub(crate) type CiMarkerRow = (String, String, String);

pub(crate) fn valid_git_sha(sha: &str) -> bool {
    sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

// Version 0 includes legacy installations with any subset of these columns.
// Inspect the schema rather than swallowing every ALTER TABLE error.
const LEGACY_COLUMNS: &[(&str, &str, &str)] = &[
    ("rp", "on_", "alter table rp add column on_ int default 0"),
    ("rp", "nm", "alter table rp add column nm int default 1"),
    ("hs", "no", "alter table hs add column no text default ''"),
    ("hs", "cpu", "alter table hs add column cpu int default 0"),
    ("rp", "bud", "alter table rp add column bud real default 0"),
    ("rp", "feat", "alter table rp add column feat int default 0"),
    ("rp", "skl", "alter table rp add column skl text default ''"),
    (
        "rp",
        "tests",
        "alter table rp add column tests text default ''",
    ),
    (
        "rp",
        "tcmd",
        "alter table rp add column tcmd text default ''",
    ),
    (
        "rp",
        "green",
        "alter table rp add column green text default ''",
    ),
    (
        "rp",
        "lang",
        "alter table rp add column lang text default 'en'",
    ),
    // Keep all existing opt-in permissions. Missing columns default to OFF.
    (
        "rp",
        "auto_merge",
        "alter table rp add column auto_merge int default 0",
    ),
    (
        "rp",
        "steward_repair",
        "alter table rp add column steward_repair int default 0",
    ),
    (
        "rp",
        "steward_activation",
        "alter table rp add column steward_activation text not null default ''",
    ),
    (
        "rp",
        "recovery_activation",
        "alter table rp add column recovery_activation text not null default ''",
    ),
    (
        "rp",
        "local_merge",
        "alter table rp add column local_merge int default 0",
    ),
    (
        "rp",
        "steward_close",
        "alter table rp add column steward_close int default 0",
    ),
    (
        "rp",
        "coderabbit_review",
        "alter table rp add column coderabbit_review int default 0",
    ),
    (
        "rp",
        "coderabbit_repair",
        "alter table rp add column coderabbit_repair int default 0",
    ),
    (
        "rp",
        "coderabbit_epoch",
        "alter table rp add column coderabbit_epoch int default 0",
    ),
    (
        "rp",
        "coderabbit_review_since",
        "alter table rp add column coderabbit_review_since int default 0",
    ),
    (
        "rp",
        "coderabbit_free_confirmed",
        "alter table rp add column coderabbit_free_confirmed int default 0",
    ),
    (
        "rp",
        "auto_recover_drops",
        "alter table rp add column auto_recover_drops int default 0",
    ),
    (
        "coderabbit_findings",
        "guard_ticket",
        "alter table coderabbit_findings add column guard_ticket text",
    ),
    (
        "seen",
        "why",
        "alter table seen add column why text default ''",
    ),
    (
        "watcher_retry",
        "analysis_ticket",
        "alter table watcher_retry add column analysis_ticket text",
    ),
    (
        "watcher_outbox",
        "source_head",
        "alter table watcher_outbox add column source_head text not null default ''",
    ),
];

fn init(c: &mut Connection) -> anyhow::Result<()> {
    c.execute_batch("pragma journal_mode=wal; pragma busy_timeout=5000;")?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    anyhow::ensure!(
        version <= SCHEMA_VERSION,
        "database schema {version} is newer than supported {SCHEMA_VERSION}"
    );
    tx.execute_batch(SCH)?;
    for &(table, column, sql) in LEGACY_COLUMNS {
        let exists: bool = tx.query_row(
            "select exists(select 1 from pragma_table_info(?) where name=?)",
            rusqlite::params![table, column],
            |r| r.get(0),
        )?;
        if !exists {
            tx.execute(sql, [])?;
        }
    }
    // SQLite may reuse a deleted maximum ROWID without AUTOINCREMENT.
    // Existing job references must never authorize a different future job.
    if version < 5 {
        let autoincrement: bool = tx.query_row(
            "select instr(upper(sql),'AUTOINCREMENT')>0 from sqlite_master where type='table' and name='jobs'",
            [],
            |r| r.get(0),
        )?;
        if !autoincrement {
            let columns: i64 =
                tx.query_row("select count(*) from pragma_table_info('jobs')", [], |r| {
                    r.get(0)
                })?;
            anyhow::ensure!(
                columns == 13,
                "unexpected legacy jobs schema; backup required"
            );
            tx.execute_batch(
                "create table jobs_v5(id integer primary key autoincrement, repo text, num int, kind text default 'fix', title text, url text, st text, why text default '', d text default '{}', pr int, prio real default 0, t0 int, t1 int);
                 insert into jobs_v5(id,repo,num,kind,title,url,st,why,d,pr,prio,t0,t1)
                   select id,repo,num,kind,title,url,st,why,d,pr,prio,t0,t1 from jobs;
                 drop table jobs;
                 alter table jobs_v5 rename to jobs;
                 create index jobs_st on jobs(st);
                 create index jobs_issue_latest on jobs(repo,num,kind,id desc);",
            )?;
        }
        // Include surviving historical references, even when the job is gone.
        tx.execute_batch(
            "insert into sqlite_sequence(name,seq)
               select 'jobs',0 where not exists(select 1 from sqlite_sequence where name='jobs');
             update sqlite_sequence set seq=max(seq,coalesce((select max(id) from jobs),0),
               coalesce((select max(job) from job_schedule),0),
               coalesce((select max(job) from job_dependencies),0),
               coalesce((select max(prerequisite) from job_dependencies),0),
               coalesce((select max(job) from coderabbit_findings),0),
               coalesce((select max(job) from fix_attempts),0),
               coalesce((select max(job) from cost),0),
               coalesce((select max(j) from ev),0))
               where name='jobs';",
        )?;
    }
    // A terminal/deleted tracked PR cannot retain executable review receipts.
    // Keep identifiers for audit; never write comment acknowledgements here.
    tx.execute_batch("
        drop trigger if exists watcher_inbox_pr_terminal;
        drop trigger if exists watcher_inbox_pr_deleted;
        create trigger if not exists watcher_inbox_pr_terminal_v2
        after update of st on prs when new.st in ('closed','merged') and old.st is not new.st
        begin
          update watcher_inbox set state='superseded',updated_at=cast(strftime('%s','now') as integer)*1000
            where repo=new.repo and num=new.num and state='received';
          delete from watcher_retry where repo=new.repo and num=new.num;
          select case when exists(select 1 from watcher_inbox where repo=new.repo and num=new.num and state='received')
            or exists(select 1 from watcher_retry where repo=new.repo and num=new.num)
            then raise(abort,'watcher PR retirement failed') end;
        end;
        create trigger if not exists watcher_inbox_pr_deleted_v2
        after delete on prs
        begin
          update watcher_inbox set state='superseded',updated_at=cast(strftime('%s','now') as integer)*1000
            where repo=old.repo and num=old.num and state='received';
          delete from watcher_retry where repo=old.repo and num=old.num;
          select case when exists(select 1 from watcher_inbox where repo=old.repo and num=old.num and state='received')
            or exists(select 1 from watcher_retry where repo=old.repo and num=old.num)
            then raise(abort,'watcher PR retirement failed') end;
        end;
        update watcher_inbox set state='superseded',updated_at=cast(strftime('%s','now') as integer)*1000
          where state='received' and not exists(select 1 from prs where prs.repo=watcher_inbox.repo and prs.num=watcher_inbox.num and prs.st='open');
        delete from watcher_retry where not exists(select 1 from prs where prs.repo=watcher_retry.repo and prs.num=watcher_retry.num and prs.st='open');
    ")?;
    let stale_watcher_receipts: bool = tx.query_row(
        "select exists(select 1 from watcher_inbox where state='received' and not exists(select 1 from prs where prs.repo=watcher_inbox.repo and prs.num=watcher_inbox.num and prs.st='open')) or exists(select 1 from watcher_retry where not exists(select 1 from prs where prs.repo=watcher_retry.repo and prs.num=watcher_retry.num and prs.st='open'))",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!stale_watcher_receipts, "watcher PR retirement failed");
    // Invalidate observation tickets and remove historical metadata on activation
    // changes or removal, atomically with the repository mutation. Exact prefix,
    // not LIKE: repository names may contain wildcard characters.
    tx.execute_batch("
        create trigger if not exists coderabbit_repo_activation
        after update of on_,full on rp
        when old.on_ is not new.on_ or old.full is not new.full
        begin
          delete from kv where substr(k,1,length('coderabbit:observation:' || old.full || '#')) = 'coderabbit:observation:' || old.full || '#';
        end;
        create trigger if not exists coderabbit_repo_removed
        after delete on rp
        begin
          delete from kv where substr(k,1,length('coderabbit:observation:' || old.full || '#')) = 'coderabbit:observation:' || old.full || '#';
        end;
    ")?;
    // Invalidate active analysis ownership on repository lifecycle changes,
    // without resetting the retry deadline or discarding receipt history.
    tx.execute_batch(
        "
        create trigger if not exists watcher_attempt_repo_changed
        after update of on_,full on rp
        when old.on_ is not new.on_ or old.full is not new.full
        begin
          update watcher_retry set analysis_ticket=null
            where repo in (old.full,new.full) and analysis_ticket is not null;
          select case when exists(select 1 from watcher_retry
            where repo in (old.full,new.full) and analysis_ticket is not null)
            then raise(abort,'watcher attempt revocation failed') end;
        end;
        create trigger if not exists watcher_attempt_repo_removed
        after delete on rp
        begin
          update watcher_retry set analysis_ticket=null
            where repo=old.full and analysis_ticket is not null;
          select case when exists(select 1 from watcher_retry
            where repo=old.full and analysis_ticket is not null)
            then raise(abort,'watcher attempt revocation failed') end;
        end;
        create trigger if not exists watcher_attempt_repo_inserted
        after insert on rp
        begin
          update watcher_retry set analysis_ticket=null
            where repo=new.full and analysis_ticket is not null;
          select case when exists(select 1 from watcher_retry
            where repo=new.full and analysis_ticket is not null)
            then raise(abort,'watcher attempt revocation failed') end;
        end;
    ",
    )?;
    // Cancel unsent replies when repository authorization is revoked. Never
    // discard uncertain deliveries: they may already have reached GitHub.
    tx.execute_batch("
        drop trigger if exists watcher_outbox_repo_removed;
        create trigger if not exists watcher_outbox_repo_removed_v2
        after delete on rp
        begin
          update watcher_outbox set state='cancelled',updated=cast(strftime('%s','now') as integer)*1000
            where repo=old.full and state='pending';
          select case when exists(select 1 from watcher_outbox where repo=old.full and state='pending')
            then raise(abort,'watcher reply revocation failed') end;
        end;
        drop trigger if exists watcher_outbox_repo_disabled;
        create trigger if not exists watcher_outbox_repo_disabled_v2
        after update of on_,full on rp
        when new.on_ is not 1 or old.full is not new.full
        begin
          update watcher_outbox set state='cancelled',updated=cast(strftime('%s','now') as integer)*1000
            where repo=old.full and state='pending';
          select case when exists(select 1 from watcher_outbox where repo=old.full and state='pending')
            then raise(abort,'watcher reply revocation failed') end;
        end;
        update watcher_outbox set state='cancelled',updated=cast(strftime('%s','now') as integer)*1000
          where state='pending' and not exists(select 1 from rp where full=watcher_outbox.repo and on_=1);
    ")?;
    // Terminal or deleted PRs cannot send previously queued replies. Keep
    // uncertain/sent deliveries for reconciliation; a reopen needs new work.
    tx.execute_batch("
        drop trigger if exists watcher_outbox_pr_terminal;
        create trigger if not exists watcher_outbox_pr_terminal_v2
        after update of st on prs when new.st is not 'open' and old.st is not new.st
        begin
          update watcher_outbox set state='cancelled',updated=cast(strftime('%s','now') as integer)*1000
            where repo=new.repo and num=new.num and state='pending';
          select case when exists(select 1 from watcher_outbox where repo=new.repo and num=new.num and state='pending')
            then raise(abort,'watcher reply revocation failed') end;
        end;
        drop trigger if exists watcher_outbox_pr_deleted;
        create trigger if not exists watcher_outbox_pr_deleted_v2
        after delete on prs
        begin
          update watcher_outbox set state='cancelled',updated=cast(strftime('%s','now') as integer)*1000
            where repo=old.repo and num=old.num and state='pending';
          select case when exists(select 1 from watcher_outbox where repo=old.repo and num=old.num and state='pending')
            then raise(abort,'watcher reply revocation failed') end;
        end;
        update watcher_outbox set state='cancelled',updated=cast(strftime('%s','now') as integer)*1000
          where state='pending' and not exists(select 1 from prs where repo=watcher_outbox.repo and num=watcher_outbox.num and st='open');
        drop trigger if exists watcher_outbox_pr_head_changed;
        create trigger if not exists watcher_outbox_pr_head_changed_v2
        after update of d on prs
        begin
          update watcher_outbox set state='cancelled',updated=cast(strftime('%s','now') as integer)*1000
            where repo=new.repo and num=new.num and state='pending' and
              (source_head='' or source_head is not case when json_valid(new.d) then json_extract(new.d,'$.watcher_head') end);
          select case when exists(select 1 from watcher_outbox where repo=new.repo and num=new.num and state='pending'
            and (source_head='' or source_head is not case when json_valid(new.d) then json_extract(new.d,'$.watcher_head') end))
            then raise(abort,'watcher reply revocation failed') end;
        end;
        update watcher_outbox set state='cancelled',updated=cast(strftime('%s','now') as integer)*1000
          where state='pending' and (source_head='' or not exists(
            select 1 from prs where repo=watcher_outbox.repo and num=watcher_outbox.num and st='open'
              and case when json_valid(d) then json_extract(d,'$.watcher_head') end=watcher_outbox.source_head));
    ")?;
    let stale_watcher_replies: bool = tx.query_row(
        "select exists(select 1 from watcher_outbox where state='pending' and (not exists(select 1 from rp where full=watcher_outbox.repo and on_=1) or source_head='' or not exists(select 1 from prs where repo=watcher_outbox.repo and num=watcher_outbox.num and st='open' and case when json_valid(d) then json_extract(d,'$.watcher_head') end=watcher_outbox.source_head)))",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!stale_watcher_replies, "watcher reply revocation failed");
    // A fresh opaque token also fences deletion/recreation of a repository.
    // Existing permissions are preserved, but old jobs have no valid token.
    tx.execute_batch("
        update rp set steward_activation=lower(hex(randomblob(16))) where steward_activation='';
        create trigger if not exists steward_activation_insert
        after insert on rp
        begin
          update rp set steward_activation=lower(hex(randomblob(16))) where id=new.id;
        end;
        create trigger if not exists steward_activation_change
        after update of on_,steward_repair,full on rp
        when old.on_ is not new.on_ or old.steward_repair is not new.steward_repair or old.full is not new.full
        begin
          update rp set steward_activation=lower(hex(randomblob(16))) where id=new.id;
        end;
    ")?;
    // Recovery authorization is a separate epoch: changing CI or automatic
    // recovery and then restoring it must not revive a pre-I/O snapshot.
    tx.execute_batch(
        "
        update rp set recovery_activation=lower(hex(randomblob(16))) where recovery_activation='';
        create trigger if not exists recovery_activation_insert
        after insert on rp
        begin
          update rp set recovery_activation=lower(hex(randomblob(16))) where id=new.id;
        end;
        create trigger if not exists recovery_activation_change
        after update of on_,steward_repair,full,ci,auto_recover_drops on rp
        when old.on_ is not new.on_ or old.steward_repair is not new.steward_repair
          or old.full is not new.full or old.ci is not new.ci
          or old.auto_recover_drops is not new.auto_recover_drops
        begin
          update rp set recovery_activation=lower(hex(randomblob(16))) where id=new.id;
        end;
    ",
    )?;
    // CI hold receipts are distinct even when a writer repeats the same
    // SHA, timestamp and raw JSON. Keep identity changes in the same transaction.
    tx.execute_batch(
        "
        create table if not exists ci_hold_epochs(k text primary key, token text not null);
        insert into ci_hold_epochs(k,token)
          select k,lower(hex(randomblob(16))) from kv where k glob 'cired:*'
          on conflict(k) do nothing;
        create trigger if not exists ci_hold_epoch_insert
        after insert on kv when new.k glob 'cired:*'
        begin
          insert into ci_hold_epochs(k,token) values(new.k,lower(hex(randomblob(16))))
            on conflict(k) do update set token=excluded.token;
        end;
        create trigger if not exists ci_hold_epoch_update
        after update of k,v on kv when old.k glob 'cired:*' or new.k glob 'cired:*'
        begin
          delete from ci_hold_epochs where k=old.k and old.k is not new.k;
          insert into ci_hold_epochs(k,token)
            select new.k,lower(hex(randomblob(16))) where new.k glob 'cired:*'
            on conflict(k) do update set token=excluded.token;
        end;
        create trigger if not exists ci_hold_epoch_delete
        after delete on kv when old.k glob 'cired:*'
        begin
          delete from ci_hold_epochs where k=old.k;
        end;
    ",
    )?;
    // Backfill recoverable history once; a cleared legacy job may only be
    // visible through its queued findings. Keep the larger observed count.
    if version < 4 {
        tx.execute_batch(
            "
            insert into coderabbit_cycles(repo,number,cycles)
            select repo,number,max(n) from (
                select repo,pr as number,count(*) as n from jobs
                  where kind='review' and pr>0 and json_valid(d)
                    and json_extract(d,'$.coderabbit_source')='coderabbit'
                  group by repo,pr
                union all
                select repo,number,count(distinct job) as n from coderabbit_findings
                  where job is not null group by repo,number
            ) group by repo,number
            on conflict(repo,number) do update set cycles=max(cycles,excluded.cycles);
        ",
        )?;
    }
    // Admission and the counter share the job insertion transaction.
    tx.execute_batch("
        create trigger if not exists coderabbit_cycle_admission
        after insert on jobs
        when new.kind='review' and new.pr>0 and
          case when json_valid(new.d) then json_extract(new.d,'$.coderabbit_source')='coderabbit' else 0 end
        begin
          insert into coderabbit_cycles(repo,number,cycles) values(new.repo,new.pr,1)
          on conflict(repo,number) do update set cycles=cycles+1;
        end;
    ")?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

pub type P = Vec<Box<dyn ToSql + Send>>;

#[macro_export]
macro_rules! pa {
    ($($x:expr),* $(,)?) => { { let v: $crate::db::P = vec![$(Box::new($x)),*]; v } };
}

fn cv(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into()),
        ValueRef::Blob(_) => Value::Null,
    }
}

impl Db {
    pub fn open(f: &Path) -> anyhow::Result<Db> {
        let mut c = Connection::open(f)?;
        init(&mut c)?;
        Ok(Db(Arc::new(Mutex::new(c))))
    }

    #[cfg(test)]
    pub fn mem() -> Db {
        let mut c = Connection::open_in_memory().unwrap();
        init(&mut c).unwrap();
        Db(Arc::new(Mutex::new(c)))
    }

    /// Strict read for safety decisions. No malformed JSON or failed row can
    /// turn a partial result into an apparently complete inventory.
    pub fn try_all(&self, s: &str, p: P) -> rusqlite::Result<Vec<Value>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare_cached(s)?;
        let names = st
            .column_names()
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>();
        let rows = st.query_map(params_from_iter(p.iter().map(|x| x.as_ref())), |r| {
            let mut row = Map::new();
            for (i, key) in names.iter().enumerate() {
                let raw = r.get_ref(i)?;
                let value = if key == "d" {
                    let ValueRef::Text(bytes) = raw else {
                        return Err(rusqlite::Error::InvalidColumnType(
                            i,
                            key.clone(),
                            raw.data_type(),
                        ));
                    };
                    serde_json::from_slice::<Value>(bytes).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            i,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?
                } else {
                    cv(raw)
                };
                row.insert(key.clone(), value);
            }
            Ok(Value::Object(row))
        })?;
        rows.collect()
    }

    pub fn try_n(&self, s: &str, p: P) -> rusqlite::Result<i64> {
        let c = self.0.lock().unwrap();
        c.query_row(s, params_from_iter(p.iter().map(|x| x.as_ref())), |r| {
            r.get(0)
        })
    }

    pub fn try_usd(&self, r: Option<&str>) -> rusqlite::Result<f64> {
        let c = self.0.lock().unwrap();
        match r {
            Some(repo) => c.query_row(
                "select coalesce(sum(usd),0.0) from cost where day=? and sub=0 and repo=?",
                rusqlite::params![day(), repo],
                |r| r.get(0),
            ),
            None => c.query_row(
                "select coalesce(sum(usd),0.0) from cost where day=? and sub=0",
                [day()],
                |r| r.get(0),
            ),
        }
    }

    pub fn all(&self, s: &str, p: P) -> Vec<Value> {
        let c = self.0.lock().unwrap();
        let mut st = match c.prepare_cached(s) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("sql {s}: {e}");
                return vec![];
            }
        };
        let n: Vec<String> = st.column_names().iter().map(|x| x.to_string()).collect();
        let r = st.query_map(params_from_iter(p.iter().map(|x| x.as_ref())), |r| {
            let mut m = Map::new();
            for (i, k) in n.iter().enumerate() {
                let mut v = cv(r.get_ref(i)?);
                if k == "d" {
                    if let Value::String(s) = &v {
                        v = serde_json::from_str(s).unwrap_or(json!({}));
                    }
                }
                m.insert(k.clone(), v);
            }
            Ok(Value::Object(m))
        });
        match r {
            Ok(it) => it.filter_map(|x| x.ok()).collect(),
            Err(e) => {
                eprintln!("sql {s}: {e}");
                vec![]
            }
        }
    }

    pub fn one(&self, s: &str, p: P) -> Option<Value> {
        self.all(s, p).into_iter().next()
    }

    pub fn n(&self, s: &str, p: P) -> i64 {
        self.one(s, p)
            .and_then(|v| v.as_object().and_then(|o| o.values().next().cloned()))
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    }

    pub fn run(&self, s: &str, p: P) -> i64 {
        let c = self.0.lock().unwrap();
        crate::app::chg();
        match c.execute(s, params_from_iter(p.iter().map(|x| x.as_ref()))) {
            Ok(_) => c.last_insert_rowid(),
            Err(e) => {
                eprintln!("sql {s}: {e}");
                -1
            }
        }
    }

    pub fn ch(&self, s: &str, p: P) -> usize {
        let c = self.0.lock().unwrap();
        crate::app::chg();
        c.execute(s, params_from_iter(p.iter().map(|x| x.as_ref())))
            .unwrap_or(0)
    }

    /// Apply a complete issue scan atomically. A failed write cannot close unseen issues.
    pub fn replace_issue_snapshot(
        &self,
        repo: &str,
        issues: &[Value],
        at: i64,
    ) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let enabled: bool = tx.query_row(
            "select exists(select 1 from rp where full=? and on_=1 and iss=1 and ci<>'')",
            [repo],
            |row| row.get(0),
        )?;
        if !enabled {
            return Err(rusqlite::Error::InvalidQuery);
        }
        for issue in issues {
            let number = issue["number"]
                .as_i64()
                .filter(|n| *n > 0)
                .ok_or(rusqlite::Error::InvalidQuery)?;
            let labels: Vec<&str> = issue["labels"]
                .as_array()
                .ok_or(rusqlite::Error::InvalidQuery)?
                .iter()
                .map(|x| x["name"].as_str().ok_or(rusqlite::Error::InvalidQuery))
                .collect::<Result<_, _>>()?;
            let assignees: Vec<&str> = issue["assignees"]
                .as_array()
                .ok_or(rusqlite::Error::InvalidQuery)?
                .iter()
                .map(|x| x["login"].as_str().ok_or(rusqlite::Error::InvalidQuery))
                .collect::<Result<_, _>>()?;
            tx.execute(
                "insert into github_issues(repo,num,title,url,author,updated,labels,assignees,seen_at,is_open,skip_reason) values(?,?,?,?,?,?,?,?,?,1,'') on conflict(repo,num) do update set title=excluded.title,url=excluded.url,author=excluded.author,labels=excluded.labels,assignees=excluded.assignees,seen_at=excluded.seen_at,is_open=1,skip_reason=case when github_issues.updated<>excluded.updated then '' else github_issues.skip_reason end,updated=excluded.updated",
                rusqlite::params![repo, number,
                    issue["title"].as_str().ok_or(rusqlite::Error::InvalidQuery)?,
                    issue["url"].as_str().ok_or(rusqlite::Error::InvalidQuery)?,
                    issue["author"]["login"].as_str().unwrap_or(""),
                    issue["updatedAt"].as_str().ok_or(rusqlite::Error::InvalidQuery)?,
                    json!(labels).to_string(), json!(assignees).to_string(), at],
            )?;
        }
        tx.execute(
            "update github_issues set is_open=0 where repo=? and seen_at<? and is_open=1",
            rusqlite::params![repo, at],
        )?;
        let scan = json!({"at":at,"count":issues.len(),"limit":null,"truncated":false,"error":null,"error_at":null});
        tx.execute(
            "insert into kv(k,v) values(?,?) on conflict(k) do update set v=excluded.v",
            rusqlite::params![format!("iscan:{repo}"), scan.to_string()],
        )?;
        tx.commit()?;
        crate::app::chg();
        Ok(())
    }

    /// Persist provider-local transient failures without storing error text or secrets.
    /// A late response from an older request cannot shorten a newer cooldown.
    pub fn provider_failure(&self, provider: &str, kind: &str, at: i64) -> rusqlite::Result<()> {
        if !matches!(kind, "quota" | "network") || crate::prov::pv(provider).is_none() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let key = format!("provider:cooldown:{provider}");
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous = {
            let mut stmt = tx.prepare("select v from kv where k=?")?;
            let mut rows = stmt.query([&key])?;
            match rows.next()? {
                Some(row) => serde_json::from_str::<Value>(&row.get::<_, String>(0)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                None => Value::Null,
            }
        };
        let last = previous["at"].as_i64().unwrap_or(0);
        if last > at {
            return Ok(());
        }
        let count = if last > 0 && at.saturating_sub(last) <= 1_800_000 && previous["kind"] == kind
        {
            previous["count"]
                .as_i64()
                .unwrap_or(0)
                .saturating_add(1)
                .clamp(1, 6)
        } else {
            1
        };
        let base = if kind == "quota" { 60_000 } else { 30_000 };
        let delay =
            (base * (1_i64 << (count - 1))).min(if kind == "quota" { 600_000 } else { 300_000 });
        let until = at
            .saturating_add(delay)
            .max(previous["until"].as_i64().unwrap_or(0));
        let value = json!({"kind":kind,"at":at,"count":count,"until":until});
        tx.execute(
            "insert into kv(k,v) values(?,?) on conflict(k) do update set v=excluded.v",
            rusqlite::params![key, value.to_string()],
        )?;
        tx.commit()?;
        crate::app::chg();
        Ok(())
    }

    /// After a cooldown, admit one recovery probe across all DB connections.
    /// The lease is bounded so an interrupted process cannot block recovery forever.
    pub fn provider_admission(
        &self,
        provider: &str,
        at: i64,
        lease_ms: i64,
    ) -> rusqlite::Result<ProviderAdmission> {
        if crate::prov::pv(provider).is_none() || !(1..=3_660_000).contains(&lease_ms) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let key = format!("provider:cooldown:{provider}");
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: Option<String> = {
            let mut stmt = tx.prepare("select v from kv where k=?")?;
            let mut rows = stmt.query([&key])?;
            rows.next()?.map(|row| row.get(0)).transpose()?
        };
        let Some(raw) = raw else {
            return Ok(ProviderAdmission::Ready);
        };
        let mut value: Value =
            serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let until = value["until"]
            .as_i64()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        if !matches!(value["kind"].as_str(), Some("quota" | "network")) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if until > at || value["probe_until"].as_i64().unwrap_or(0) > at {
            return Ok(ProviderAdmission::Blocked);
        }
        let ticket = hex::encode(rand::random::<[u8; 16]>());
        value["probe_ticket"] = json!(ticket);
        value["probe_until"] = json!(at.saturating_add(lease_ms));
        tx.execute(
            "update kv set v=? where k=?",
            rusqlite::params![value.to_string(), key],
        )?;
        tx.commit()?;
        Ok(ProviderAdmission::Probe(ticket))
    }

    /// A late or expired probe must not clear a newer failure or another probe.
    pub fn finish_provider_probe(
        &self,
        provider: &str,
        ticket: &str,
        at: i64,
        healthy: bool,
    ) -> rusqlite::Result<bool> {
        let key = format!("provider:cooldown:{provider}");
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = if healthy {
            tx.execute(
                "delete from kv where k=? and json_extract(v,'$.probe_ticket')=? and json_extract(v,'$.probe_until')>?",
                rusqlite::params![key, ticket, at],
            )?
        } else {
            tx.execute(
                "update kv set v=json_remove(json_set(v,'$.until',max(json_extract(v,'$.until'),?)),'$.probe_ticket','$.probe_until')
                 where k=? and json_extract(v,'$.probe_ticket')=? and json_extract(v,'$.probe_until')>?",
                rusqlite::params![at.saturating_add(30_000), key, ticket, at],
            )?
        };
        tx.commit()?;
        crate::app::chg();
        Ok(changed == 1)
    }

    pub fn provider_cooldown(&self, provider: &str, at: i64) -> rusqlite::Result<Option<Value>> {
        let rows = self.try_all(
            "select v from kv where k=?",
            crate::pa![format!("provider:cooldown:{provider}")],
        )?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let raw = row["v"].as_str().ok_or(rusqlite::Error::InvalidQuery)?;
        let value: Value = serde_json::from_str(raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let until = value["until"]
            .as_i64()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        if !matches!(value["kind"].as_str(), Some("quota" | "network")) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let probe_until = value["probe_until"].as_i64().unwrap_or(0);
        if probe_until > at && probe_until > until {
            let mut pending = value;
            pending["until"] = json!(probe_until);
            return Ok(Some(pending));
        }
        Ok((until > at).then_some(value))
    }

    pub fn kg(&self, k: &str) -> Value {
        self.one("select v from kv where k=?", crate::pa![k.to_string()])
            .and_then(|r| r["v"].as_str().and_then(|s| serde_json::from_str(s).ok()))
            .unwrap_or(Value::Null)
    }

    pub fn ks(&self, k: &str, v: &Value) {
        self.run(
            "insert into kv(k,v) values(?,?) on conflict(k) do update set v=excluded.v",
            crate::pa![k.to_string(), v.to_string()],
        );
    }

    pub fn kd(&self, k: &str) {
        self.run("delete from kv where k=?", crate::pa![k.to_string()]);
    }

    pub fn usd(&self, r: Option<&str>) -> f64 {
        let v = match r {
            Some(r) => self.one(
                "select coalesce(sum(usd),0) s from cost where day=? and sub=0 and repo=?",
                crate::pa![day(), r.to_string()],
            ),
            None => self.one(
                "select coalesce(sum(usd),0) s from cost where day=? and sub=0",
                crate::pa![day()],
            ),
        };
        v.and_then(|x| x["s"].as_f64()).unwrap_or(0.0)
    }

    /// Fail closed when a prerequisite has not completed and its PR has not merged.
    /// Missing/deleted prerequisites are blocked, not silently considered satisfied.
    pub fn schedule_eligible(&self, id: i64, repo: &str, at: i64) -> rusqlite::Result<bool> {
        let c = self.0.lock().unwrap();
        c.query_row(
            "select exists(select 1 from jobs j left join job_schedule s on s.job=j.id
             where j.id=? and j.repo=? and j.st in ('ready','submit')
             and coalesce(s.retry_after,0)<=?
             and not exists(select 1 from job_dependencies dep
               where dep.job=j.id and not exists(select 1 from jobs prior join prs p
                 on p.repo=prior.repo and p.num=prior.pr
                 where prior.id=dep.prerequisite and prior.st='done' and p.st='merged')))",
            rusqlite::params![id, repo, at],
            |r| r.get(0),
        )
    }

    /// Replace operator-defined scheduling constraints atomically. The caller
    /// holds update_transition and verifies that the job is not assigned.
    pub fn set_job_schedule(
        &self,
        id: i64,
        priority: Option<i64>,
        dependencies: &[i64],
    ) -> rusqlite::Result<()> {
        if priority.is_some_and(|p| !(0..=3).contains(&p))
            || dependencies.len() > 100
            || dependencies.iter().any(|d| *d <= 0 || *d == id)
            || dependencies
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != dependencies.len()
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let editable: bool = tx.query_row(
            "select exists(select 1 from jobs where id=? and st='ready' and kind='fix'
            and coalesce(json_extract(d,'$.rs'),0)=0 and coalesce(json_extract(d,'$.rp'),0)=0)",
            [id],
            |r| r.get(0),
        )?;
        if !editable {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.execute("delete from job_dependencies where job=?", [id])?;
        for dependency in dependencies {
            let exists: bool = tx.query_row("select exists(select 1 from jobs where id=? and kind in ('fix','review','rebase') and st<>'drop')",[dependency],|r|r.get(0))?;
            let cycle: bool = tx.query_row("with recursive ancestors(id) as (
                select ?1 union select d.prerequisite from job_dependencies d join ancestors a on d.job=a.id
            ) select exists(select 1 from ancestors where id=?2)",rusqlite::params![dependency,id],|r|r.get(0))?;
            if !exists || cycle {
                return Err(rusqlite::Error::InvalidQuery);
            }
            tx.execute(
                "insert into job_dependencies(job,prerequisite) values(?,?)",
                rusqlite::params![id, dependency],
            )?;
        }
        tx.execute("insert into job_schedule(job,manual_priority) values(?,?) on conflict(job) do update set manual_priority=excluded.manual_priority",
            rusqlite::params![id,priority])?;
        tx.commit()?;
        crate::app::chg();
        Ok(())
    }

    /// Keep the existing host-retry policy, but persist the next eligible time
    /// in the same transaction as the queue state (including approved submit).
    pub fn defer_host_retry(
        &self,
        job: &Job,
        retry_after: i64,
        reason: &str,
    ) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        let st = if job.st == "submit" {
            "submit"
        } else {
            "ready"
        };
        let changed = tx.execute("update jobs set st=?,d=?,t1=? where id=? and st not in ('done','drop','approve','park')",
            rusqlite::params![st,job.d.to_string(),now(),job.id])?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        tx.execute("insert into job_schedule(job,retry_after,retry_reason) values(?,?,?) on conflict(job) do update set retry_after=excluded.retry_after,retry_reason=excluded.retry_reason",
            rusqlite::params![job.id,retry_after,reason])?;
        tx.commit()?;
        crate::app::chg();
        Ok(())
    }

    pub fn jg(&self, id: i64) -> Option<Job> {
        self.one("select * from jobs where id=?", crate::pa![id])
            .map(Job::of)
    }

    pub fn jset(&self, id: i64, f: &[(&str, Value)]) {
        if f.is_empty() {
            return;
        }
        let mut p: P = vec![];
        let mut s = vec![];
        for (k, v) in f {
            s.push(format!("{k}=?"));
            match v {
                Value::Null => p.push(Box::new(rusqlite::types::Null)),
                Value::Number(n) if n.is_i64() => p.push(Box::new(n.as_i64().unwrap())),
                Value::Number(n) => p.push(Box::new(n.as_f64().unwrap_or(0.0))),
                Value::String(x) => p.push(Box::new(x.clone())),
                o => p.push(Box::new(o.to_string())),
            }
        }
        p.push(Box::new(now()));
        p.push(Box::new(id));
        self.run(
            &format!("update jobs set {}, t1=? where id=?", s.join(",")),
            p,
        );
    }

    pub fn jd(&self, j: &Job) {
        self.jset(j.id, &[("d", j.d.clone())]);
    }

    // INSERT OR IGNORE is idempotent only if the intended receipt exists.
    fn persist_comment_ack(
        tx: &rusqlite::Transaction<'_>,
        repo: &str,
        id: &str,
    ) -> rusqlite::Result<()> {
        let key = format!("{repo}:{id}");
        tx.execute(
            "insert or ignore into cm(id,t) values(?,?)",
            rusqlite::params![key, now()],
        )?;
        let exists: bool = tx.query_row(
            "select exists(select 1 from cm where id=?)",
            [&key],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }

    fn ack_comments_in_tx(
        tx: &rusqlite::Transaction<'_>,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
    ) -> rusqlite::Result<()> {
        for id in ids {
            Self::persist_comment_ack(tx, repo, id)?;
        }
        let changed = tx.execute("update prs set d=json_set(d, '$.seen', json('true'), '$.comment_seen_at', ?) where repo=? and num=?", rusqlite::params![updated, repo, number])?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }

    /// Store the authorized reply within the same transaction as its source
    /// acknowledgements. Reserve the existing 24-hour per-PR reply budget.
    fn queue_watcher_reply(
        tx: &rusqlite::Transaction<'_>,
        repo: &str,
        number: i64,
        head: &str,
        ids: &[String],
        reply: &Value,
    ) -> rusqlite::Result<()> {
        if reply.is_null() {
            return Ok(());
        }
        let author = reply["author"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let body = reply["body"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.chars().count() <= 1500)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        if ids.is_empty() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut source = ids.to_vec();
        source.sort();
        source.dedup();
        let key = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
            json!([repo, number, source]).to_string().as_bytes(),
        ));
        let marker = format!("<!-- apollo-office-reply:{key} -->");
        if body.contains("apollo-office-reply:") {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let body = format!("{body}\n\n{marker}");
        let at = now();
        let open: bool = tx.query_row(
            "select exists(select 1 from prs where repo=? and num=? and st='open')",
            rusqlite::params![repo, number],
            |r| r.get(0),
        )?;
        if !open {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let previous:Option<i64>=tx.query_row("select max(case when state='sent' then updated else created end) from watcher_outbox where repo=? and num=? and state<>'cancelled'",rusqlite::params![repo,number],|r|r.get(0))?;
        let legacy: Option<i64> = tx.query_row(
            "select max(case when json_valid(v) then cast(v as integer) end) from kv where k=?",
            [format!("wr:{repo}#{number}")],
            |r| r.get(0),
        )?;
        if previous
            .into_iter()
            .chain(legacy)
            .any(|t| at.saturating_sub(t) < 86_400_000)
        {
            return Ok(());
        }
        tx.execute("insert or ignore into watcher_outbox(id,repo,num,author,body,state,created,updated,source_head) select ?,?,?,?,?,'pending',?,?,? where exists(select 1 from rp where full=? and on_=1)",rusqlite::params![key,repo,number,author,body,at,at,head,repo])?;
        // A previous action with this key is a legitimate no-op, even if
        // cancelled. Never consume the batch if no durable action exists.
        let exists: bool = tx.query_row(
            "select exists(select 1 from watcher_outbox where id=? and repo=? and num=?)",
            rusqlite::params![key, repo, number],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }

    /// Check the exact observed open PR snapshot within the same transaction
    /// as acknowledgement. A newer local observation invalidates old analysis.
    fn review_snapshot_matches(
        tx: &rusqlite::Transaction<'_>,
        repo: &str,
        number: i64,
        updated: &str,
        head: &str,
    ) -> rusqlite::Result<bool> {
        if !valid_git_sha(head) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.query_row("select exists(select 1 from prs where repo=? and num=? and st='open' and upd=? and json_extract(d,'$.watcher_head')=? and exists(select 1 from rp where full=prs.repo and on_=1))",rusqlite::params![repo,number,updated,head],|r|r.get(0))
    }

    /// Record receipt before invoking Guard/Watcher. Only identifiers are stored;
    /// comment bodies are always fetched again and never treated as authorization.
    pub fn receive_review_batch(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        head: &str,
        ids: &[String],
    ) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !Self::review_snapshot_matches(&tx, repo, number, updated, head)? {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let unique = ids.iter().collect::<std::collections::HashSet<_>>();
        if unique.len() != ids.len() || ids.iter().any(|id| id.is_empty() || id.len() > 128) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let at = now();
        // Retain the evidence that a prior HEAD was observed without reusing it
        // as permission to act on the current HEAD.
        tx.execute("update watcher_inbox set state='superseded',updated_at=? where repo=? and num=? and head<>? and state='received'",rusqlite::params![at,repo,number,head])?;
        let stale: bool = tx.query_row(
            "select exists(select 1 from watcher_inbox where repo=? and num=? and head<>? and state='received')",
            rusqlite::params![repo, number, head],
            |row| row.get(0),
        )?;
        if stale {
            return Err(rusqlite::Error::InvalidQuery);
        }
        for id in ids {
            tx.execute("insert into watcher_inbox(repo,num,head,comment_id,snapshot,state,received_at,updated_at) values(?,?,?,?,?,'received',?,?) on conflict(repo,num,head,comment_id) do update set snapshot=excluded.snapshot,state='received',updated_at=excluded.updated_at where watcher_inbox.state in ('received','superseded')",rusqlite::params![repo,number,head,id,updated,at,at])?;
            // Preserve completed receipts on replay; otherwise require the
            // current snapshot to have been persisted before analysis.
            let exists: bool = tx.query_row(
                "select exists(select 1 from watcher_inbox where repo=? and num=? and head=? and comment_id=? and (state='done' or (state='received' and snapshot=?)))",
                rusqlite::params![repo, number, head, id, updated],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Reconcile previously received IDs only after a successful fresh comment
    /// read and remote snapshot recheck. Absence is NOT an acknowledgement;
    /// an ID seen again is eligible for a new receipt and authorization check.
    pub fn reconcile_review_receipts(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        head: &str,
        observed: &[String],
    ) -> rusqlite::Result<usize> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !Self::review_snapshot_matches(&tx, repo, number, updated, head)? {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let unique = observed.iter().collect::<std::collections::HashSet<_>>();
        if unique.len() != observed.len()
            || observed.iter().any(|id| id.is_empty() || id.len() > 128)
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let inventory = json!(observed).to_string();
        let changed=tx.execute("update watcher_inbox set state='superseded',updated_at=? where repo=? and num=? and head=? and state='received' and not exists(select 1 from json_each(?) where value=watcher_inbox.comment_id)",rusqlite::params![now(),repo,number,head,inventory])?;
        let stale: bool = tx.query_row(
            "select exists(select 1 from watcher_inbox where repo=? and num=? and head=? and state='received' and not exists(select 1 from json_each(?) where value=watcher_inbox.comment_id))",
            rusqlite::params![repo, number, head, inventory],
            |row| row.get(0),
        )?;
        if stale {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Self::clear_review_retry_if_complete(&tx, repo, number, head)?;
        tx.commit()?;
        Ok(changed)
    }

    /// Pending receipts override the normal timestamp/age polling shortcut.
    pub fn has_pending_review(&self, repo: &str, number: i64) -> rusqlite::Result<bool> {
        Ok(self.try_n("select exists(select 1 from watcher_inbox where repo=? and num=? and state='received')",crate::pa![repo.to_string(),number])?==1)
    }

    /// Reserve a retry window before any provider call. A process crash consumes
    /// an attempt instead of causing an immediate repeat. New snapshots reset the
    /// counter; repeated failures back off from ten minutes to one hour.
    pub fn claim_review_attempt(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        head: &str,
        at: i64,
    ) -> rusqlite::Result<Option<ReviewAttempt>> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !Self::review_snapshot_matches(&tx, repo, number, updated, head)? {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let changed=tx.execute("insert into watcher_retry(repo,num,head,snapshot,attempts,next_attempt_at)
            values(?,?,?,?,1,?+600000)
            on conflict(repo,num) do update set
              head=excluded.head,snapshot=excluded.snapshot,
              attempts=case when watcher_retry.head<>excluded.head or watcher_retry.snapshot<>excluded.snapshot then 1 else min(watcher_retry.attempts+1,4) end,
              next_attempt_at=excluded.next_attempt_at-600000+case when watcher_retry.head<>excluded.head or watcher_retry.snapshot<>excluded.snapshot then 600000 else min(3600000,600000*(1 << min(watcher_retry.attempts,3))) end
            where watcher_retry.analysis_ticket is null and (watcher_retry.head<>excluded.head or watcher_retry.snapshot<>excluded.snapshot or watcher_retry.next_attempt_at<=excluded.next_attempt_at-600000)",
            rusqlite::params![repo,number,head,updated,at])?;
        if changed != 1 {
            return Ok(None);
        }
        let ticket = hex::encode(rand::random::<[u8; 16]>());
        if tx.execute(
            "update watcher_retry set analysis_ticket=? where repo=? and num=? and analysis_ticket is null",
            rusqlite::params![ticket, repo, number],
        )? != 1
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()?;
        Ok(Some(ReviewAttempt {
            db: self.clone(),
            repo: repo.to_string(),
            number,
            ticket,
        }))
    }

    // Cleanup may legitimately be a no-op while receipts are pending.
    // If none remain, the matching retry row must actually be removed.
    fn clear_review_retry_if_complete(
        tx: &rusqlite::Transaction<'_>,
        repo: &str,
        number: i64,
        head: &str,
    ) -> rusqlite::Result<()> {
        let pending: bool = tx.query_row(
            "select exists(select 1 from watcher_inbox where repo=? and num=? and head=? and state='received')",
            rusqlite::params![repo, number, head],
            |row| row.get(0),
        )?;
        if pending {
            return Ok(());
        }
        tx.execute(
            "delete from watcher_retry where repo=? and num=? and head=?",
            rusqlite::params![repo, number, head],
        )?;
        let retained: bool = tx.query_row(
            "select exists(select 1 from watcher_retry where repo=? and num=? and head=?)",
            rusqlite::params![repo, number, head],
            |row| row.get(0),
        )?;
        if retained {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(())
    }

    fn finish_review_inbox(
        tx: &rusqlite::Transaction<'_>,
        repo: &str,
        number: i64,
        head: &str,
        ids: &[String],
    ) -> rusqlite::Result<()> {
        for id in ids {
            tx.execute("update watcher_inbox set state='done',updated_at=? where repo=? and num=? and head=? and comment_id=? and state='received'",rusqlite::params![now(),repo,number,head,id])?;
            // Legacy callers may have no receipt. An existing pending
            // receipt, however, must not survive a successful acknowledgement.
            let pending: bool = tx.query_row(
                "select exists(select 1 from watcher_inbox where repo=? and num=? and head=? and comment_id=? and state='received')",
                rusqlite::params![repo, number, head, id],
                |row| row.get(0),
            )?;
            if pending {
                return Err(rusqlite::Error::InvalidQuery);
            }
        }
        Self::clear_review_retry_if_complete(tx, repo, number, head)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn ack_pr_comments_with_reply(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        head: &str,
        ids: &[String],
        reply: &Value,
    ) -> rusqlite::Result<()> {
        self.ack_review_inner(repo, number, updated, head, (ids, None), reply)
    }

    pub(crate) fn ack_review_with_attempt(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        head: &str,
        batch: (&[String], Option<&ReviewAttempt>),
        reply: &Value,
    ) -> rusqlite::Result<()> {
        if batch.1.is_none() && (!batch.0.is_empty() || !reply.is_null()) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        self.ack_review_inner(repo, number, updated, head, batch, reply)
    }

    fn ack_review_inner(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        head: &str,
        (ids, attempt): (&[String], Option<&ReviewAttempt>),
        reply: &Value,
    ) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !Self::review_snapshot_matches(&tx, repo, number, updated, head)? {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let unique = ids.iter().collect::<std::collections::HashSet<_>>();
        if unique.len() != ids.len() || ids.iter().any(|id| id.trim().is_empty() || id.len() > 128)
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if let Some(attempt) = attempt {
            if !attempt.matches(&tx, self, repo, number, updated, head)? {
                return Err(rusqlite::Error::InvalidQuery);
            }
        } else if ids.is_empty() {
            // Empty observations must not consume a concurrent analysis or
            // advance the cursor past receipts admitted since the observation.
            let busy: bool = tx.query_row(
                "select exists(select 1 from watcher_retry where repo=? and num=? and analysis_ticket is not null) or exists(select 1 from watcher_inbox where repo=? and num=? and state='received')",
                rusqlite::params![repo, number, repo, number],
                |row| row.get(0),
            )?;
            if busy {
                return Err(rusqlite::Error::InvalidQuery);
            }
        }
        // Do not create an outbound action for a replayed or partially consumed batch.
        let mut consumed = false;
        for id in ids {
            let seen: bool = tx.query_row(
                "select exists(select 1 from cm where id=? or id=?)",
                rusqlite::params![format!("{repo}:{id}"), id],
                |r| r.get(0),
            )?;
            consumed |= seen;
        }
        if consumed && !reply.is_null() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Self::queue_watcher_reply(&tx, repo, number, head, ids, reply)?;
        Self::ack_comments_in_tx(&tx, repo, number, updated, ids)?;
        Self::finish_review_inbox(&tx, repo, number, head, ids)?;
        tx.commit()?;
        crate::app::chg();
        Ok(())
    }

    /// Public diagnostics expose only aggregate states, never reply content.
    pub fn watcher_outbox_counts(&self) -> rusqlite::Result<Value> {
        let rows = self.try_all(
            "select state,count(*) n from watcher_outbox group by state",
            vec![],
        )?;
        let mut result = json!({"pending":0,"uncertain":0,"sent":0,"cancelled":0});
        for row in rows {
            let state = row["state"].as_str().ok_or(rusqlite::Error::InvalidQuery)?;
            if !matches!(state, "pending" | "uncertain" | "sent" | "cancelled") {
                return Err(rusqlite::Error::InvalidQuery);
            }
            result[state] = row["n"].clone();
        }
        Ok(result)
    }

    /// Owner-only inventory via the authenticated local API. Include unresolved
    /// entries first and cap each page, without exposing reply bodies in state().
    pub fn watcher_outbox_page(&self, offset: i64) -> rusqlite::Result<Vec<Value>> {
        if !(0..=100_000).contains(&offset) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        self.try_all("select id,repo,num,author,body,state,created,updated from watcher_outbox order by case when state in ('pending','uncertain') then 0 else 1 end,created desc,id limit 51 offset ?",crate::pa![offset])
    }

    /// Persist the uncertain state BEFORE the external POST. A crash or
    /// ambiguous response must never automatically issue a second POST.
    pub fn claim_watcher_reply(&self, id: &str, author: &str) -> rusqlite::Result<bool> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let eligible: bool = tx.query_row(
            "select exists(select 1 from watcher_outbox where id=? and author=? and state='pending' and exists(select 1 from rp where full=watcher_outbox.repo and on_=1) and source_head<>'' and exists(select 1 from prs where repo=watcher_outbox.repo and num=watcher_outbox.num and st='open' and case when json_valid(d) then json_extract(d,'$.watcher_head') end=watcher_outbox.source_head))",
            rusqlite::params![id, author],
            |row| row.get(0),
        )?;
        if !eligible {
            return Ok(false);
        }
        let changed=tx.execute("update watcher_outbox set state='uncertain',updated=? where id=? and author=? and state='pending' and exists(select 1 from rp where full=watcher_outbox.repo and on_=1) and source_head<>'' and exists(select 1 from prs where repo=watcher_outbox.repo and num=watcher_outbox.num and st='open' and case when json_valid(d) then json_extract(d,'$.watcher_head') end=watcher_outbox.source_head)",rusqlite::params![now(),id,author])?;
        let reserved: bool = tx.query_row(
            "select exists(select 1 from watcher_outbox where id=? and author=? and state='uncertain')",
            rusqlite::params![id, author],
            |row| row.get(0),
        )?;
        if changed != 1 || !reserved {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()?;
        Ok(true)
    }

    // Cursor checkpoints are scheduling hints, never send authorizations.
    pub(crate) fn load_watcher_delivery_cursor(
        &self,
        author: &str,
        state: &str,
    ) -> rusqlite::Result<Option<(i64, String)>> {
        if !matches!(state, "pending" | "uncertain") || author.is_empty() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let key = format!("watcher:delivery-cursor:{state}:{author}");
        let rows = self.try_all("select v from kv where k=?", crate::pa![key])?;
        match rows.as_slice() {
            [] => Ok(None),
            [row] => serde_json::from_str(row["v"].as_str().ok_or(rusqlite::Error::InvalidQuery)?)
                .map(Some)
                .map_err(|_| rusqlite::Error::InvalidQuery),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }

    pub(crate) fn save_watcher_delivery_cursor(
        &self,
        author: &str,
        state: &str,
        updated: i64,
        id: &str,
    ) -> rusqlite::Result<()> {
        if !matches!(state, "pending" | "uncertain") || author.is_empty() || id.is_empty() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let key = format!("watcher:delivery-cursor:{state}:{author}");
        let value = json!([updated, id]).to_string();
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "insert into kv(k,v) values(?,?) on conflict(k) do update set v=excluded.v",
            rusqlite::params![key, value],
        )?;
        let persisted: bool = tx.query_row(
            "select exists(select 1 from kv where k=? and v=?)",
            rusqlite::params![key, value],
            |row| row.get(0),
        )?;
        if changed != 1 || !persisted {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()
    }

    // Rotate only the exact observed row. Do not resurrect a cancelled reply
    // or move a newer observation backwards.
    pub(crate) fn rotate_watcher_reply(
        &self,
        id: &str,
        author: &str,
        state: &str,
        previous: i64,
        at: i64,
    ) -> rusqlite::Result<bool> {
        if !matches!(state, "pending" | "uncertain") {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let eligible: bool = tx.query_row(
            "select exists(select 1 from watcher_outbox where id=? and author=? and state=? and updated=?)",
            rusqlite::params![id, author, state, previous],
            |row| row.get(0),
        )?;
        if !eligible {
            return Ok(false);
        }
        let at = at.max(previous);
        let changed = tx.execute(
            "update watcher_outbox set updated=? where id=? and author=? and state=? and updated=?",
            rusqlite::params![at, id, author, state, previous],
        )?;
        let persisted: bool = tx.query_row(
            "select exists(select 1 from watcher_outbox where id=? and author=? and state=? and updated=?)",
            rusqlite::params![id, author, state, at],
            |row| row.get(0),
        )?;
        if changed != 1 || !persisted {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn finish_watcher_reply(&self, id: &str, state: &str) -> rusqlite::Result<bool> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let source = match state {
            "sent" => "uncertain",
            "cancelled" => "pending",
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        let changed=match state{
            "sent"=>tx.execute("update watcher_outbox set state='sent',updated=? where id=? and state='uncertain'",rusqlite::params![now(),id])?,
            "cancelled"=>tx.execute("update watcher_outbox set state='cancelled',updated=? where id=? and state='pending'",rusqlite::params![now(),id])?,
            _=>return Err(rusqlite::Error::InvalidQuery)
        };
        let (still_eligible, persisted): (bool, bool) = tx.query_row(
            "select exists(select 1 from watcher_outbox where id=? and state=?),exists(select 1 from watcher_outbox where id=? and state=?)",
            rusqlite::params![id, source, id, state],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if still_eligible || (changed == 1 && !persisted) || changed > 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()?;
        if changed == 1 {
            crate::app::chg();
        }
        Ok(changed == 1)
    }

    /// Atomically acknowledge a successfully processed comment batch and its PR cursor.
    /// Admit a Watcher rebase only against the observed open PR, and serialize
    /// admission with review and mergecheck jobs in the same SQLite transaction.
    pub fn queue_watcher_rebase(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        head: &str,
        request: &Value,
    ) -> rusqlite::Result<bool> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !Self::review_snapshot_matches(&tx, repo, number, updated, head)? {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let active:bool=tx.query_row(
            "select exists(select 1 from jobs where repo=? and pr=? and ((kind in ('review','rebase','mergecheck') and st not in ('done','drop')) or (kind='rebase' and t1>?)))",
            rusqlite::params![repo,number,now()-86_400_000],|r|r.get(0))?;
        if active {
            return Ok(false);
        }
        let title = request["title"]
            .as_str()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let url = request["url"]
            .as_str()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let why = request["why"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let mut data = request["data"]
            .as_object()
            .cloned()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        for key in ["branch", "head"] {
            if !data
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
        }
        // The Steward's fresh read is not authorization if repository
        // activation or the tracked default branch changed before commit.
        let token = data
            .get("steward_rebase_activation")
            .and_then(Value::as_str)
            .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let base = data
            .get("steward_rebase_base_ref")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or(rusqlite::Error::InvalidQuery)?;
        if !data
            .get("steward_rebase_base_sha")
            .and_then(Value::as_str)
            .is_some_and(valid_git_sha)
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let current: bool = tx.query_row(
            "select exists(select 1 from rp join prs on prs.repo=rp.full
             where rp.full=?1 and rp.on_=1 and rp.steward_activation=?2
             and prs.num=?3 and prs.st='open'
             and json_extract(prs.d,'$.default_branch')=?4
             and json_extract(prs.d,'$.base')=?4)",
            rusqlite::params![repo, token, number, base],
            |row| row.get(0),
        )?;
        if !current {
            return Err(rusqlite::Error::InvalidQuery);
        }
        data.insert("watcher_observed_head".into(), json!(head));
        data.insert("watcher_observed_at".into(), json!(updated));
        let t = now();
        let encoded = json!(data).to_string();
        if tx.execute("insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1) values(?,?,'rebase',?,?,'ready',?,?,?,-1.0,?,?)",
            rusqlite::params![repo,number,title,url,why,encoded,number,t,t])? != 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let job = tx.last_insert_rowid();
        let persisted: bool = tx.query_row(
            "select exists(select 1 from jobs where id=?1 and repo=?2 and num=?3 and pr=?3 and kind='rebase' and st='ready' and title=?4 and url=?5 and why=?6 and d=?7 and prio=-1.0 and t0=?8 and t1=?8)",
            rusqlite::params![job, repo, number, title, url, why, encoded, t],
            |row| row.get(0),
        )?;
        if !persisted {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()?;
        crate::app::chg();
        Ok(true)
    }

    /// Queue a review and acknowledge its source comments in the same transaction.
    /// If another review is active, do not acknowledge the batch: retry it later.
    #[cfg(test)]
    pub fn queue_review_and_ack(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
        request: &Value,
    ) -> rusqlite::Result<bool> {
        self.queue_review_inner(repo, number, updated, ids, request, None)
    }

    pub(crate) fn queue_review_with_attempt(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
        request: &Value,
        attempt: &ReviewAttempt,
    ) -> rusqlite::Result<bool> {
        self.queue_review_inner(repo, number, updated, ids, request, Some(attempt))
    }

    fn queue_review_inner(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
        request: &Value,
        attempt: Option<&ReviewAttempt>,
    ) -> rusqlite::Result<bool> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let head = request["data"]["source_head"]
            .as_str()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let open = Self::review_snapshot_matches(&tx, repo, number, updated, head)?;
        if let Some(attempt) = attempt {
            if !attempt.matches(&tx, self, repo, number, updated, head)? {
                return Err(rusqlite::Error::InvalidQuery);
            }
        }
        let unique = ids.iter().collect::<std::collections::HashSet<_>>();
        if !open
            || ids.is_empty()
            || unique.len() != ids.len()
            || ids.iter().any(|id| id.trim().is_empty() || id.len() > 128)
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        for id in ids {
            let acknowledged: bool = tx.query_row(
                "select exists(select 1 from cm where id=? or id=?)",
                rusqlite::params![format!("{repo}:{id}"), id],
                |row| row.get(0),
            )?;
            if acknowledged {
                return Ok(false);
            }
        }
        let active: i64 = tx.query_row(
            "select count(*) from jobs where repo=? and pr=? and kind in ('review','rebase','mergecheck') and st not in ('done','drop')",
            rusqlite::params![repo, number], |row| row.get(0),
        )?;
        if active != 0 {
            return Ok(false);
        }
        let title = request["title"]
            .as_str()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let url = request["url"]
            .as_str()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let change = request["change"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let mut data = request["data"]
            .as_object()
            .cloned()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        data.insert("source_comments".into(), json!(ids));
        data.insert("source_updated_at".into(), json!(updated));
        let t = now();
        let encoded = json!(data).to_string();
        if tx.execute("insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1) values(?,?,'review',?,?,'ready',?,?,?,-1.0,?,?)",
            rusqlite::params![repo, number, title, url, change, encoded, number, t, t])? != 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let job = tx.last_insert_rowid();
        Self::queue_watcher_reply(&tx, repo, number, head, ids, &request["reply"])?;
        Self::ack_comments_in_tx(&tx, repo, number, updated, ids)?;
        Self::finish_review_inbox(&tx, repo, number, head, ids)?;
        let persisted: bool = tx.query_row(
            "select exists(select 1 from jobs where id=?1 and repo=?2 and num=?3 and pr=?3 and kind='review' and st='ready' and title=?4 and url=?5 and why=?6 and d=?7 and prio=-1.0 and t0=?8 and t1=?8)",
            rusqlite::params![job, repo, number, title, url, change, encoded, t],
            |row| row.get(0),
        )?;
        if !persisted {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()?;
        crate::app::chg();
        Ok(true)
    }

    /// Atomic follow-up admission and acknowledgement. A replay cannot create a
    /// second follow-up, and an already active follow-up leaves new input pending.
    // Late follow-ups must use the exact observed merged PR timestamp.
    const MERGED_SNAPSHOT: &'static str =
        "select exists(select 1 from prs where repo=? and num=? and st='merged' and upd=? and exists(select 1 from rp where full=prs.repo and on_=1))";

    fn ack_merged_in_tx(
        tx: &rusqlite::Transaction<'_>,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
        complete: bool,
    ) -> rusqlite::Result<()> {
        let matched: bool = tx.query_row(
            Self::MERGED_SNAPSHOT,
            rusqlite::params![repo, number, updated],
            |row| row.get(0),
        )?;
        let unique = ids.iter().collect::<std::collections::HashSet<_>>();
        if !matched || unique.len() != ids.len() || ids.iter().any(|id| id.trim().is_empty()) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if complete {
            Self::ack_comments_in_tx(tx, repo, number, updated, ids)?;
        } else {
            // Keep the old cursor so deferred input is observed again.
            for id in ids {
                Self::persist_comment_ack(tx, repo, id)?;
            }
        }
        Ok(())
    }

    pub fn ack_merged_comments(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
        complete: bool,
    ) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        Self::ack_merged_in_tx(&tx, repo, number, updated, ids, complete)?;
        tx.commit()?;
        crate::app::chg();
        Ok(())
    }

    pub fn queue_followup_and_ack(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
        request: &Value,
        complete: bool,
    ) -> rusqlite::Result<bool> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let merged: bool = tx.query_row(
            Self::MERGED_SNAPSHOT,
            rusqlite::params![repo, number, updated],
            |r| r.get(0),
        )?;
        if !merged
            || ids.is_empty()
            || ids.iter().any(|id| id.trim().is_empty())
            || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut seen = 0;
        for id in ids {
            let acknowledged: bool = tx.query_row(
                "select exists(select 1 from cm where id=? or id=?)",
                rusqlite::params![format!("{repo}:{id}"), id],
                |r| r.get(0),
            )?;
            if acknowledged {
                seen += 1;
            }
        }
        if seen == ids.len() {
            Self::ack_merged_in_tx(&tx, repo, number, updated, ids, complete)?;
            tx.commit()?;
            return Ok(true);
        }
        // The caller must re-read a partially processed batch instead of
        // associating an already-consumed comment with a new action.
        if seen != 0 {
            return Ok(false);
        }
        let active:bool=tx.query_row(
            "select exists(select 1 from jobs where repo=? and kind='fix' and json_extract(d,'$.after')=? and st not in ('done','drop'))",
            rusqlite::params![repo,number],|r|r.get(0))?;
        if active {
            return Ok(false);
        }
        let title = request["title"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let url = request["url"]
            .as_str()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let why = request["why"]
            .as_str()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let body = request["body"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let t = now();
        let encoded = json!({"title":title,"body":body,"after":number,"source_comments":ids}).to_string();
        if tx.execute("insert into jobs(repo,num,kind,title,url,st,why,d,prio,t0,t1) values(?,0,'fix',?,?,'vet',?,?,-1.0,?,?)",
            rusqlite::params![repo,title,url,why,encoded,t,t])? != 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let job = tx.last_insert_rowid();
        Self::ack_merged_in_tx(&tx, repo, number, updated, ids, complete)?;
        let persisted: bool = tx.query_row(
            "select exists(select 1 from jobs where id=?1 and repo=?2 and num=0 and pr is null and kind='fix' and st='vet' and title=?3 and url=?4 and why=?5 and d=?6 and prio=-1.0 and t0=?7 and t1=?7)",
            rusqlite::params![job, repo, title, url, why, encoded, t],
            |row| row.get(0),
        )?;
        if !persisted {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()?;
        crate::app::chg();
        Ok(true)
    }

    pub fn comment_acknowledged(&self, repo: &str, id: &str) -> rusqlite::Result<bool> {
        let c = self.0.lock().unwrap();
        let count: i64 = c.query_row(
            "select count(*) from cm where id=? or id=?",
            rusqlite::params![format!("{repo}:{id}"), id],
            |row| row.get(0),
        )?;
        Ok(count != 0)
    }

    /// Admit an exact internal Hunter duplicate at most once while the prior
    /// candidate is active. Different locations and subsequent regressions
    /// after a completed/dropped candidate remain eligible for human triage.
    /// The originating hunt must still be active and authorized. A revoked
    /// policy has its own outcome, distinct from duplicates and database errors.
    pub fn queue_hunter_find(
        &self,
        repo: &str,
        hunt_id: i64,
        data: &Value,
    ) -> rusqlite::Result<HunterFinding> {
        let title = data["title"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let location = data["hunt"]["where"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(rusqlite::Error::InvalidQuery)?;
        if !data["body"].as_str().is_some_and(|s| !s.trim().is_empty())
            || !data["hunt"]["proof"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let display = title.chars().take(200).collect::<String>();
        let t = now();
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let authorized: bool = tx.query_row(
            "select exists(select 1 from rp join jobs j on j.repo=rp.full
             where rp.full=? and rp.on_=1 and rp.hunt=1 and trim(rp.ci)<>''
             and j.id=? and j.kind='hunt' and j.st='hunt')",
            rusqlite::params![repo, hunt_id],
            |row| row.get(0),
        )?;
        if !authorized {
            return Ok(HunterFinding::Revoked);
        }
        let changed = tx.execute(
            "insert into jobs(repo,num,kind,title,url,st,why,d,prio,t0,t1)
             select ?,0,'fix',?,?,'vet','found by Hunter',?,0,?,?
             where not exists(select 1 from jobs where repo=? and num=0 and kind='fix'
               and st not in ('done','drop')
               and lower(trim(json_extract(d,'$.title')))=lower(?)
               and trim(json_extract(d,'$.hunt.where'))=?)",
            rusqlite::params![
                repo,
                display,
                format!("https://github.com/{repo}"),
                data.to_string(),
                t,
                t,
                repo,
                title,
                location
            ],
        )?;
        tx.commit()?;
        if changed != 0 {
            crate::app::chg();
        }
        Ok(if changed == 1 {
            HunterFinding::Queued
        } else {
            HunterFinding::Duplicate
        })
    }

    // Column order deliberately mirrors the jobs table.
    #[expect(
        clippy::too_many_arguments,
        reason = "Typed insert mirrors the existing jobs schema"
    )]
    pub fn jnew(
        &self,
        repo: &str,
        num: i64,
        kind: &str,
        title: &str,
        url: &str,
        st: &str,
        why: &str,
        d: Value,
        pr: Option<i64>,
        prio: f64,
    ) -> i64 {
        let t = now();
        self.run("insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1) values(?,?,?,?,?,?,?,?,?,?,?,?)", crate::pa![repo.to_string(), num, kind.to_string(), title.to_string(), url.to_string(), st.to_string(), why.to_string(), d.to_string(), pr, prio, t, t])
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: i64,
    pub repo: String,
    pub num: i64,
    pub kind: String,
    pub title: String,
    #[allow(dead_code)]
    pub url: String,
    pub st: String,
    pub why: String,
    pub d: Value,
    pub pr: Option<i64>,
    pub t0: i64,
}

impl Job {
    pub fn of(v: Value) -> Job {
        Job {
            id: v["id"].as_i64().unwrap_or(0),
            repo: v["repo"].as_str().unwrap_or("").into(),
            num: v["num"].as_i64().unwrap_or(0),
            kind: v["kind"].as_str().unwrap_or("fix").into(),
            title: v["title"].as_str().unwrap_or("").into(),
            url: v["url"].as_str().unwrap_or("").into(),
            st: v["st"].as_str().unwrap_or("").into(),
            why: v["why"].as_str().unwrap_or("").into(),
            d: if v["d"].is_object() {
                v["d"].clone()
            } else {
                json!({})
            },
            pr: v["pr"].as_i64(),
            t0: v["t0"].as_i64().unwrap_or(0),
        }
    }
    pub fn s(&self, k: &str) -> String {
        self.d[k].as_str().unwrap_or("").to_string()
    }
    pub fn h(&self) -> String {
        self.s("h")
    }
    pub fn hunt(&self) -> bool {
        self.d["hunt"].is_object()
    }
    pub fn ext(&self) -> bool {
        self.d["ext"].is_object()
    }
    pub fn feat(&self) -> bool {
        self.d["feat"].as_bool().unwrap_or(false)
    }
    pub fn opt(&self) -> bool {
        self.d["opt"].as_bool().unwrap_or(false)
    }
    pub fn own(&self) -> bool {
        self.hunt() || self.ext()
    }
    pub fn wh(&self) -> String {
        self.d["hunt"]["where"]
            .as_str()
            .or(self.d["ext"]["where"].as_str())
            .unwrap_or("")
            .to_string()
    }
    pub fn orig(&self) -> String {
        if self.hunt() {
            "Bug found by our own testing (there is no GitHub issue; do not reference one)".into()
        } else if self.ext() {
            format!("Bug report sent to us directly from {} (there is no GitHub issue; do not reference one)", self.d["ext"]["src"].as_str().unwrap_or("an external tool"))
        } else if self.opt() {
            format!(
                "Behaviour-preserving optimization request, issue #{}",
                self.num
            )
        } else if self.feat() {
            format!("Feature request, issue #{}", self.num)
        } else if self.opt() {
            format!(
                "Behaviour-preserving optimization request, issue #{}",
                self.num
            )
        } else {
            format!("Issue #{}", self.num)
        }
    }
    pub fn tag(&self) -> String {
        if self.ext() {
            format!("{} report {}", self.repo, self.id)
        } else if self.hunt() || self.num == 0 {
            format!("{} find {}", self.repo, self.id)
        } else {
            format!("{}#{}", self.repo, self.num)
        }
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn day() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn day0() -> i64 {
    let n = chrono::Local::now();
    n.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|d| d.and_local_timezone(chrono::Local).single())
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimization_jobs_remain_distinct_from_feature_requests() {
        let optimization = Job::of(json!({
            "id": 1, "repo": "o/r", "num": 14, "kind": "fix",
            "d": {"opt": true, "feat": false}
        }));
        assert!(optimization.opt());
        assert!(!optimization.feat());
        assert!(optimization
            .orig()
            .contains("Behaviour-preserving optimization"));

        let feature = Job::of(json!({
            "id": 2, "repo": "o/r", "num": 15, "kind": "fix",
            "d": {"opt": false, "feat": true}
        }));
        assert!(!feature.opt());
        assert!(feature.feat());
        assert!(feature.orig().contains("Feature request"));
    }

    #[test]
    fn pull_request_language_persists_with_english_as_default() {
        let db = Db::mem();
        db.run(
            "insert into rp(full) values(?)",
            crate::pa!["o/r".to_string()],
        );
        assert_eq!(
            db.one(
                "select lang from rp where full=?",
                crate::pa!["o/r".to_string()]
            )
            .unwrap()["lang"],
            "en"
        );
        db.run(
            "update rp set lang=? where full=?",
            crate::pa!["pt-PT".to_string(), "o/r".to_string()],
        );
        assert_eq!(
            db.one(
                "select lang from rp where full=?",
                crate::pa!["o/r".to_string()]
            )
            .unwrap()["lang"],
            "pt-PT"
        );
    }
}

#[cfg(test)]
mod comment_cursor_tests {
    use super::*;

    #[test]
    fn comment_ack_is_atomic_and_preserves_other_metadata() {
        let db = Db::mem();
        db.run("insert into rp(full) values('a/b')", vec![]);
        let ids = vec!["review-1".to_string()];
        assert!(db.ack_merged_comments("a/b", 1, "new", &ids, true).is_err());
        assert_eq!(db.n("select count(*) from cm", vec![]), 0);
        db.run(
            "insert into prs(repo,num,st,upd,d) values(?,?,'merged','new',?)",
            crate::pa![
                "a/b".to_string(),
                1_i64,
                r#"{"auto_merge":true}"#.to_string()
            ],
        );
        db.ack_merged_comments("a/b", 1, "new", &ids, true).unwrap();
        db.ack_merged_comments("a/b", 1, "new", &ids, true).unwrap();
        assert_eq!(db.n("select count(*) from cm", vec![]), 1);
        let row = db
            .one("select d from prs where repo='a/b' and num=1", vec![])
            .unwrap();
        assert_eq!(row["d"]["comment_seen_at"], "new");
        assert_eq!(row["d"]["seen"], true);
        assert_eq!(row["d"]["auto_merge"], true);
    }
    #[test]
    fn review_admission_and_ack_share_one_transaction_and_defer_active_work() {
        let db = Db::mem();
        db.run("insert into rp(full) values('a/b')", vec![]);
        let ids = vec!["review-1".to_string()];
        let request = json!({"title":"Fix","url":"https://github.com/a/b/pull/1","change":"Correct bug","data":{"source_head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}});
        assert!(db
            .queue_review_and_ack("a/b", 1, "new", &ids, &request)
            .is_err());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
        db.run(
            "insert into prs(repo,num,st,upd,d) values(?,?,'open','new',?)",
            crate::pa![
                "a/b".to_string(),
                1_i64,
                json!({"watcher_head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}).to_string()
            ],
        );
        assert!(db
            .queue_review_and_ack("a/b", 1, "new", &ids, &request)
            .unwrap());
        assert!(db.comment_acknowledged("a/b", "review-1").unwrap());
        let next = vec!["review-2".to_string()];
        db.run("update prs set upd='later'", vec![]);
        assert!(!db
            .queue_review_and_ack("a/b", 1, "later", &next, &request)
            .unwrap());
        assert!(!db.comment_acknowledged("a/b", "review-2").unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
        db.run("update jobs set st='done'", vec![]);
        assert!(db
            .queue_review_and_ack("a/b", 1, "later", &next, &request)
            .unwrap());
        assert!(db.comment_acknowledged("a/b", "review-2").unwrap());
        db.run("update jobs set st='done'", vec![]);
        assert!(!db
            .queue_review_and_ack("a/b", 1, "later", &next, &request)
            .unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 2);
    }

    #[test]
    fn review_rejects_partial_replay_and_other_active_pr_operations() {
        let db = Db::mem();
        db.run("insert into rp(full) values('a/b')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('a/b',1,'open','next',?)",
            crate::pa![
                json!({"watcher_head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}).to_string()
            ],
        );
        let request = json!({"title":"Fix","url":"https://github.com/a/b/pull/1","change":"Correct bug","data":{"source_head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}});
        db.ack_pr_comments_with_reply(
            "a/b",
            1,
            "next",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &["r1".into()],
            &Value::Null,
        )
        .unwrap();
        assert!(!db
            .queue_review_and_ack("a/b", 1, "next", &["r1".into(), "r2".into()], &request)
            .unwrap());
        assert!(!db.comment_acknowledged("a/b", "r2").unwrap());
        db.run(
            "insert into jobs(repo,num,pr,kind,st,d) values('a/b',1,1,'rebase','ready','{}')",
            vec![],
        );
        assert!(!db
            .queue_review_and_ack("a/b", 1, "next", &["r2".into()], &request)
            .unwrap());
        db.run("update jobs set st='done'", vec![]);
        assert!(db
            .queue_review_and_ack("a/b", 1, "next", &["r2".into()], &request)
            .unwrap());
        let job = db
            .one("select d from jobs where kind='review'", vec![])
            .unwrap();
        assert_eq!(job["d"]["source_comments"], json!(["r2"]));
        db.run("update prs set st='closed'", vec![]);
        assert!(db
            .queue_review_and_ack("a/b", 1, "last", &["r3".into()], &request)
            .is_err());
        assert!(!db.comment_acknowledged("a/b", "r3").unwrap());
    }
}

#[cfg(test)]
mod issue_snapshot_tests {
    use super::*;

    fn issue(number: i64) -> Value {
        json!({"number":number,"title":"Bug","url":"https://github.com/o/r/issues/1",
            "updatedAt":"2026-01-01T00:00:00Z","labels":[],"assignees":[]})
    }

    #[test]
    fn issue_snapshot_rejects_removed_or_disabled_repository() {
        let db = Db::mem();
        assert!(db.replace_issue_snapshot("o/r", &[issue(1)], 10).is_err());
        db.run("insert into rp(full,ci) values('o/r','test')", vec![]);
        db.replace_issue_snapshot("o/r", &[issue(1)], 20).unwrap();
        db.run("update rp set on_=0 where full='o/r'", vec![]);
        assert!(db.replace_issue_snapshot("o/r", &[issue(2)], 30).is_err());
        db.run("delete from rp where full='o/r'", vec![]);
        assert!(db.replace_issue_snapshot("o/r", &[issue(2)], 40).is_err());
        assert_eq!(
            db.n("select num from github_issues where is_open=1", vec![]),
            1
        );
        assert_eq!(db.kg("iscan:o/r")["at"], 20);
    }

    #[test]
    fn failed_snapshot_rolls_back_rows_closures_and_scan_metadata() {
        let db = Db::mem();
        db.run("insert into rp(full,ci) values('o/r','test')", vec![]);
        db.replace_issue_snapshot("o/r", &[issue(1)], 10).unwrap();
        assert!(db
            .replace_issue_snapshot("o/r", &[issue(2), json!({"number":3})], 20)
            .is_err());
        assert_eq!(
            db.n("select count(*) from github_issues where is_open=1", vec![]),
            1
        );
        assert_eq!(
            db.n("select num from github_issues where is_open=1", vec![]),
            1
        );
        assert_eq!(db.kg("iscan:o/r")["at"], 10);
        db.replace_issue_snapshot("o/r", &[issue(2)], 30).unwrap();
        assert_eq!(
            db.n("select num from github_issues where is_open=1", vec![]),
            2
        );
        assert_eq!(db.kg("iscan:o/r")["at"], 30);
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;
    fn job(db: &Db, n: i64) -> i64 {
        db.jnew(
            "o/r",
            n,
            "fix",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        )
    }
    #[test]
    fn prerequisites_require_merged_pr_and_cycles_roll_back() {
        let db = Db::mem();
        let first = job(&db, 1);
        let second = job(&db, 2);
        db.set_job_schedule(second, Some(0), &[first]).unwrap();
        assert!(!db.schedule_eligible(second, "o/r", now()).unwrap());
        assert!(db.set_job_schedule(first, None, &[second]).is_err());
        assert!(db.schedule_eligible(first, "o/r", now()).unwrap());
        db.jset(first, &[("st", json!("done")), ("pr", json!(10))]);
        assert!(!db.schedule_eligible(second, "o/r", now()).unwrap());
        db.run(
            "insert into prs(repo,num,st) values('o/r',10,'merged')",
            vec![],
        );
        assert!(db.schedule_eligible(second, "o/r", now()).unwrap());
        assert!(!db.schedule_eligible(second, "other/r", now()).unwrap());
        assert!(db.set_job_schedule(second, Some(7), &[]).is_err());
        assert_eq!(db.n("select count(*) from job_dependencies", vec![]), 1);
    }
    #[test]
    fn host_retry_persists_deadline_without_changing_approval_or_dependencies() {
        let db = Db::mem();
        let id = job(&db, 1);
        db.set_job_schedule(id, Some(1), &[]).unwrap();
        db.jset(
            id,
            &[
                ("st", json!("submit")),
                ("d", json!({"ap":true,"h":"local","ir":1})),
            ],
        );
        let j = db.jg(id).unwrap();
        db.defer_host_retry(&j, 1000, "host unavailable").unwrap();
        assert!(!db.schedule_eligible(id, "o/r", 999).unwrap());
        assert!(db.schedule_eligible(id, "o/r", 1000).unwrap());
        assert_eq!(db.jg(id).unwrap().st, "submit");
        assert_eq!(db.jg(id).unwrap().d["ap"], true);
        assert_eq!(db.n("select manual_priority from job_schedule", vec![]), 1);
        db.jset(id, &[("st", json!("drop"))]);
        assert!(db.defer_host_retry(&j, 2000, "late retry").is_err());
        assert!(!db.schedule_eligible(id, "o/r", 3000).unwrap());
    }
}

#[cfg(test)]
mod strict_db_tests {
    use super::*;
    #[test]
    fn legacy_migration_preserves_permissions_and_is_idempotent() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch("create table rp(id integer primary key, full text, auto_merge int default 0); insert into rp(full,auto_merge) values('o/r',1)").unwrap();
        init(&mut c).unwrap();
        init(&mut c).unwrap();
        let values: (i64, i64, i64, i64, i64, i64) = c
            .query_row(
                "select auto_merge,steward_repair,local_merge,coderabbit_review,coderabbit_repair,coderabbit_free_confirmed from rp where full='o/r'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap();
        assert_eq!(values, (1, 0, 0, 0, 0, 0));
        let enabled: i64 = c
            .query_row("select on_ from rp where full='o/r'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(enabled, 0);
        let version: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn job_identity_migration_preserves_rows_and_never_reuses_historical_ids() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(&SCH.replace(
            "jobs(id integer primary key autoincrement",
            "jobs(id integer primary key",
        ))
        .unwrap();
        c.execute_batch(
            "pragma user_version=4;
             insert into jobs(id,repo,st) values(7,'o/r','done');
             insert into job_schedule(job,manual_priority) values(99,0);
             insert into job_dependencies(job,prerequisite) values(7,101);",
        )
        .unwrap();
        init(&mut c).unwrap();
        init(&mut c).unwrap();
        let repo: String = c
            .query_row("select repo from jobs where id=7", [], |r| r.get(0))
            .unwrap();
        assert_eq!(repo, "o/r");
        c.execute("delete from jobs", []).unwrap();
        c.execute("insert into jobs(repo,st) values('new/r','ready')", [])
            .unwrap();
        let id = c.last_insert_rowid();
        assert!(id > 101);
        c.execute("delete from jobs", []).unwrap();
        c.execute("insert into jobs(repo,st) values('new/r','ready')", [])
            .unwrap();
        assert!(c.last_insert_rowid() > id);
    }

    #[test]
    fn startup_retirement_rolls_back_legacy_migration_on_partial_cleanup() {
        for fault in [
            "create trigger ignore_cleanup before update on watcher_inbox when new.state='superseded' begin select raise(ignore); end;",
            "create trigger ignore_cleanup before delete on watcher_retry begin select raise(ignore); end;",
            "create trigger ignore_cleanup before update on watcher_outbox when new.state='cancelled' begin select raise(ignore); end;",
        ] {
            let mut c = Connection::open_in_memory().unwrap();
            init(&mut c).unwrap();
            c.execute_batch(
                "drop trigger watcher_inbox_pr_terminal_v2;
                 drop trigger watcher_outbox_pr_terminal_v2;
                 create trigger watcher_inbox_pr_terminal after update of st on prs begin select 1; end;
                 create trigger watcher_outbox_pr_terminal after update of st on prs begin select 1; end;
                 insert into rp(full) values('o/r');
                 insert into prs(repo,num,st) values('o/r',1,'closed');
                 insert into watcher_inbox(repo,num,head,comment_id,snapshot,state,received_at,updated_at)
                    values('o/r',1,'head','r1','t','received',1,1);
                 insert into watcher_retry(repo,num,head,snapshot,attempts,next_attempt_at,analysis_ticket)
                    values('o/r',1,'head','t',2,100,'live');
                 insert into watcher_outbox(id,repo,num,author,body,state,created,updated)
                    values('a','o/r',1,'owner','reply','pending',1,1),
                          ('b','o/r',1,'owner','reply','uncertain',1,1);",
            )
            .unwrap();
            c.execute_batch(fault).unwrap();
            assert!(init(&mut c).is_err());
            fn count(c: &Connection, sql: &str) -> i64 {
                c.query_row(sql, [], |r| r.get(0)).unwrap()
            }
            assert_eq!(count(&c, "select count(*) from watcher_inbox where state='received' and updated_at=1"), 1);
            assert_eq!(count(&c, "select count(*) from watcher_retry where analysis_ticket='live' and next_attempt_at=100"), 1);
            assert_eq!(count(&c, "select count(*) from watcher_outbox where state='pending' and updated=1"), 1);
            assert_eq!(count(&c, "select count(*) from sqlite_master where type='trigger' and name in ('watcher_inbox_pr_terminal','watcher_outbox_pr_terminal')"), 2);
            assert_eq!(count(&c, "select count(*) from sqlite_master where type='trigger' and name in ('watcher_inbox_pr_terminal_v2','watcher_outbox_pr_terminal_v2')"), 0);
            c.execute_batch("drop trigger ignore_cleanup").unwrap();
            init(&mut c).unwrap();
            init(&mut c).unwrap();
            assert_eq!(count(&c, "select count(*) from watcher_inbox where state='superseded'"), 1);
            assert_eq!(count(&c, "select count(*) from watcher_retry"), 0);
            assert_eq!(count(&c, "select count(*) from watcher_outbox where state='cancelled'"), 1);
            assert_eq!(count(&c, "select count(*) from watcher_outbox where state='uncertain' and updated=1"), 1);
            assert_eq!(count(&c, "select count(*) from cm"), 0);
            assert_eq!(count(&c, "select count(*) from sqlite_master where type='trigger' and name in ('watcher_inbox_pr_terminal','watcher_outbox_pr_terminal')"), 0);
            assert_eq!(count(&c, "select count(*) from sqlite_master where type='trigger' and name in ('watcher_inbox_pr_terminal_v2','watcher_outbox_pr_terminal_v2')"), 2);
        }
    }

    #[test]
    fn migration_rolls_back_on_non_duplicate_failure_and_refuses_future_schema() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "create table rp(id integer primary key, full text); create view seen as select 1 x",
        )
        .unwrap();
        assert!(init(&mut c).is_err());
        let count: i64 = c
            .query_row(
                "select count(*) from pragma_table_info('rp') where name='nm'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        let version: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 0);
        c.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(init(&mut c).is_err());
    }

    #[test]
    fn steward_activation_persists_and_rotates_only_on_policy_changes() {
        let mut c = Connection::open_in_memory().unwrap();
        init(&mut c).unwrap();
        c.execute(
            "insert into rp(full,on_,steward_repair) values('o/r',1,1)",
            [],
        )
        .unwrap();
        fn token(c: &Connection) -> String {
            c.query_row("select steward_activation from rp", [], |r| r.get(0))
                .unwrap()
        }
        let first = token(&c);
        assert_eq!(first.len(), 32);
        init(&mut c).unwrap();
        assert_eq!(token(&c), first);
        c.execute("update rp set steward_repair=1,on_=1", [])
            .unwrap();
        assert_eq!(token(&c), first);
        c.execute("update rp set on_=0", []).unwrap();
        let disabled = token(&c);
        assert_ne!(disabled, first);
        c.execute("update rp set on_=1", []).unwrap();
        let enabled = token(&c);
        assert_ne!(enabled, disabled);
        assert_ne!(enabled, first);
        c.execute("update rp set full='o/renamed'", []).unwrap();
        let renamed = token(&c);
        assert_ne!(renamed, enabled);
        c.execute("delete from rp", []).unwrap();
        c.execute(
            "insert into rp(full,on_,steward_repair) values('o/r',1,1)",
            [],
        )
        .unwrap();
        assert_ne!(token(&c), first);
        assert_ne!(token(&c), renamed);
    }

    #[test]
    fn recovery_epoch_migrates_and_rotates_without_revoking_steward_receipts() {
        let mut c = Connection::open_in_memory().unwrap();
        init(&mut c).unwrap();
        c.execute_batch(
            "drop trigger recovery_activation_insert;
             drop trigger recovery_activation_change;
             alter table rp drop column recovery_activation;
             pragma user_version=7;
             insert into rp(full,on_,ci,auto_recover_drops,steward_repair)
             values('o/r',1,'cargo test',1,1);",
        )
        .unwrap();
        let steward: String = c
            .query_row("select steward_activation from rp", [], |r| r.get(0))
            .unwrap();
        init(&mut c).unwrap();
        fn recovery(c: &Connection) -> String {
            c.query_row("select recovery_activation from rp", [], |r| r.get(0))
                .unwrap()
        }
        let first = recovery(&c);
        assert_eq!(first.len(), 32);
        init(&mut c).unwrap();
        assert_eq!(recovery(&c), first);
        c.execute("update rp set ci='cargo test',auto_recover_drops=1", [])
            .unwrap();
        assert_eq!(recovery(&c), first);
        let mut last = first.clone();
        for change in [
            "update rp set ci='cargo test --locked'",
            "update rp set ci='cargo test'",
            "update rp set auto_recover_drops=0",
            "update rp set auto_recover_drops=1",
        ] {
            c.execute(change, []).unwrap();
            let token = recovery(&c);
            assert_ne!(token, last);
            assert_ne!(token, first);
            last = token;
        }
        let actual_steward: String = c
            .query_row("select steward_activation from rp", [], |r| r.get(0))
            .unwrap();
        assert_eq!(actual_steward, steward);
        c.execute("update rp set on_=0", []).unwrap();
        assert_ne!(recovery(&c), last);
        let disabled = recovery(&c);
        c.execute("update rp set on_=1", []).unwrap();
        assert_ne!(recovery(&c), disabled);
        c.execute("delete from rp", []).unwrap();
        c.execute(
            "insert into rp(full,on_,ci,auto_recover_drops) values('o/r',1,'cargo test',1)",
            [],
        )
        .unwrap();
        assert_ne!(recovery(&c), first);
    }

    #[test]
    fn ci_hold_epochs_backfill_and_rotate_even_for_identical_writes() {
        let mut c = Connection::open_in_memory().unwrap();
        init(&mut c).unwrap();
        c.execute_batch(
            "drop trigger ci_hold_epoch_insert;
             drop trigger ci_hold_epoch_update;
             drop trigger ci_hold_epoch_delete;
             drop table ci_hold_epochs;
             pragma user_version=8;
             insert into kv(k,v) values('cired:o/r','{}'),('unrelated','{}');",
        )
        .unwrap();
        init(&mut c).unwrap();
        fn token(c: &Connection) -> String {
            c.query_row(
                "select token from ci_hold_epochs where k='cired:o/r'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        }
        let first = token(&c);
        assert_eq!(first.len(), 32);
        init(&mut c).unwrap();
        assert_eq!(token(&c), first);
        c.execute("update kv set v=v where k='unrelated'", [])
            .unwrap();
        assert_eq!(token(&c), first);
        let tx = c.transaction().unwrap();
        tx.execute("update kv set v=v where k='cired:o/r'", [])
            .unwrap();
        assert_ne!(token(&tx), first);
        tx.rollback().unwrap();
        assert_eq!(token(&c), first);
        c.execute("update kv set v=v where k='cired:o/r'", [])
            .unwrap();
        let second = token(&c);
        assert_ne!(second, first);
        c.execute("delete from kv where k='cired:o/r'", []).unwrap();
        assert_eq!(
            c.query_row("select count(*) from ci_hold_epochs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        c.execute("insert into kv(k,v) values('cired:o/r','{}')", [])
            .unwrap();
        assert_ne!(token(&c), second);
        c.execute("update kv set k='cired:o/renamed' where k='cired:o/r'", [])
            .unwrap();
        assert!(c
            .query_row(
                "select token from ci_hold_epochs where k='cired:o/r'",
                [],
                |r| r.get::<_, String>(0)
            )
            .is_err());
        assert_eq!(
            c.query_row(
                "select length(token) from ci_hold_epochs where k='cired:o/renamed'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            32
        );
    }

    #[test]
    fn strict_reads_distinguish_zero_empty_and_failed_queries() {
        let db = Db::mem();
        assert_eq!(db.try_n("select count(*) from jobs", vec![]).unwrap(), 0);
        assert!(db.try_all("select * from jobs", vec![]).unwrap().is_empty());
        assert!(db.try_n("select count(*) from absent", vec![]).is_err());
        assert!(db.try_all("select * from absent", vec![]).is_err());
        db.run(
            "insert into jobs(repo,st,d) values('o/r','ready','not json')",
            vec![],
        );
        assert!(db.try_all("select * from jobs", vec![]).is_err());
        assert_eq!(db.try_usd(None).unwrap(), 0.0);
        db.run("drop table cost", vec![]);
        assert!(db.try_usd(None).is_err());
    }
}

#[cfg(test)]
mod late_followup_tests {
    use super::*;
    #[test]
    fn followup_commit_replay_and_pending_batch_are_atomic() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'merged','t1','{}')",
            vec![],
        );
        let request = json!({"title":"follow-up","body":"verified request","url":"https://example.test","why":"maintainer"});
        let first = vec!["i1".into()];
        let second = vec!["i2".into()];
        assert!(db
            .queue_followup_and_ack("o/r", 1, "t1", &first, &request, true)
            .unwrap());
        assert!(db
            .queue_followup_and_ack("o/r", 1, "t1", &first, &request, true)
            .unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
        db.run("update prs set upd='t2' where repo='o/r' and num=1", vec![]);
        assert!(db
            .queue_followup_and_ack("o/r", 1, "t1", &second, &request, true)
            .is_err());
        assert!(!db
            .queue_followup_and_ack("o/r", 1, "t2", &second, &request, true)
            .unwrap());
        assert!(!db.comment_acknowledged("o/r", "i2").unwrap());
        db.run("update jobs set st='done'", vec![]);
        assert!(db
            .queue_followup_and_ack("o/r", 1, "t2", &second, &request, true)
            .unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 2);
        assert!(db.comment_acknowledged("o/r", "i2").unwrap());
        assert!(db
            .queue_followup_and_ack("o/r", 2, "t3", &["i3".into()], &request, true)
            .is_err());
        assert!(!db.comment_acknowledged("o/r", "i3").unwrap());
    }
    #[test]
    fn revoked_repository_cannot_consume_merged_comments() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'merged','t','{}')",
            vec![],
        );
        let request = json!({"title":"follow-up","body":"verified request","url":"https://example.test","why":"maintainer"});
        let ids = vec!["i1".into()];
        for sql in ["update rp set on_=0", "delete from rp"] {
            db.run(sql, vec![]);
            assert!(db
                .queue_followup_and_ack("o/r", 1, "t", &ids, &request, true)
                .is_err());
            for complete in [false, true] {
                assert!(db
                    .ack_merged_comments("o/r", 1, "t", &ids, complete)
                    .is_err());
            }
            assert!(!db.comment_acknowledged("o/r", "i1").unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
        }
    }

    #[test]
    fn ignored_merged_ack_rolls_back_followup_and_preserves_cursor() {
        for complete in [false, true] {
            let db = Db::mem();
            db.run("insert into rp(full) values('o/r')", vec![]);
            db.run(
                "insert into prs(repo,num,st,upd,d) values('o/r',1,'merged','new',?)",
                crate::pa![json!({"comment_seen_at":"old"}).to_string()],
            );
            let request = json!({"title":"follow-up","body":"verified request","url":"https://example.test","why":"maintainer"});
            let ids = vec!["i1".into(), "i2".into()];
            db.0.lock()
                .unwrap()
                .execute_batch(
                    "create trigger ignore_ack before insert on cm
                     when new.id='o/r:i2' begin select raise(ignore); end;",
                )
                .unwrap();
            assert!(db
                .queue_followup_and_ack("o/r", 1, "new", &ids, &request, complete)
                .is_err());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
            assert_eq!(db.n("select count(*) from cm", vec![]), 0);
            assert_eq!(
                db.one("select d from prs where repo='o/r' and num=1", vec![])
                    .unwrap()["d"]["comment_seen_at"],
                "old"
            );
            db.run("drop trigger ignore_ack", vec![]);
            assert!(db
                .queue_followup_and_ack("o/r", 1, "new", &ids, &request, complete)
                .unwrap());
            assert!(db
                .queue_followup_and_ack("o/r", 1, "new", &ids, &request, complete)
                .unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
            assert_eq!(db.n("select count(*) from cm", vec![]), 2);
        }
    }

    #[test]
    fn followup_job_must_survive_acknowledgement_with_exact_payload() {
        for fault in [
            "create trigger mutate_job after insert on jobs when new.kind='fix' begin update jobs set d='{}' where id=new.id; end;",
            "create trigger mutate_job after insert on cm begin delete from jobs where kind='fix'; end;",
        ] {
            let db = Db::mem();
            db.run("insert into rp(full) values('o/r')", vec![]);
            db.run(
                "insert into prs(repo,num,st,upd,d) values('o/r',1,'merged','new',?)",
                crate::pa![json!({"comment_seen_at":"old"}).to_string()],
            );
            let request = json!({"title":"follow-up","body":"verified request","url":"https://example.test","why":"maintainer"});
            let ids = vec!["i1".into()];
            db.0.lock().unwrap().execute_batch(fault).unwrap();
            assert!(db
                .queue_followup_and_ack("o/r", 1, "new", &ids, &request, true)
                .is_err());
            assert!(!db.comment_acknowledged("o/r", "i1").unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
            assert_eq!(
                db.one("select d from prs where repo='o/r' and num=1", vec![])
                    .unwrap()["d"]["comment_seen_at"],
                "old"
            );
            db.run("drop trigger mutate_job", vec![]);
            assert!(db
                .queue_followup_and_ack("o/r", 1, "new", &ids, &request, true)
                .unwrap());
            assert!(db.comment_acknowledged("o/r", "i1").unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
        }
    }

    #[test]
    fn ignored_followup_insert_preserves_comments_and_cursor() {
        for complete in [false, true] {
            let db = Db::mem();
            db.run("insert into rp(full) values('o/r')", vec![]);
            db.run(
                "insert into prs(repo,num,st,upd,d) values('o/r',1,'merged','new',?)",
                crate::pa![json!({"comment_seen_at":"old"}).to_string()],
            );
            let request = json!({"title":"follow-up","body":"verified request","url":"https://example.test","why":"maintainer"});
            let ids = vec!["i1".into()];
            db.0.lock()
                .unwrap()
                .execute_batch(
                    "create trigger ignore_followup before insert on jobs
                     when new.kind='fix' begin select raise(ignore); end;",
                )
                .unwrap();
            assert!(db
                .queue_followup_and_ack("o/r", 1, "new", &ids, &request, complete)
                .is_err());
            assert!(!db.comment_acknowledged("o/r", "i1").unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
            assert_eq!(
                db.one("select d from prs where repo='o/r' and num=1", vec![])
                    .unwrap()["d"]["comment_seen_at"],
                "old"
            );
            db.run("drop trigger ignore_followup", vec![]);
            assert!(db
                .queue_followup_and_ack("o/r", 1, "new", &ids, &request, complete)
                .unwrap());
            assert!(db.comment_acknowledged("o/r", "i1").unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
            assert_eq!(
                db.one("select d from prs where repo='o/r' and num=1", vec![])
                    .unwrap()["d"]["comment_seen_at"],
                if complete { "new" } else { "old" }
            );
            assert!(db
                .queue_followup_and_ack("o/r", 1, "new", &ids, &request, complete)
                .unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
        }
    }

    #[test]
    fn partial_merged_ack_preserves_cursor_until_complete() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'merged','new',?)",
            crate::pa![json!({"comment_seen_at":"old"}).to_string()],
        );
        assert!(db
            .ack_merged_comments("o/r", 1, "old", &["i1".into()], false)
            .is_err());
        assert!(!db.comment_acknowledged("o/r", "i1").unwrap());
        db.ack_merged_comments("o/r", 1, "new", &["i1".into()], false)
            .unwrap();
        assert!(db.comment_acknowledged("o/r", "i1").unwrap());
        assert_eq!(
            db.one("select d from prs where repo='o/r' and num=1", vec![])
                .unwrap()["d"]["comment_seen_at"],
            "old"
        );
        db.ack_merged_comments("o/r", 1, "new", &["i2".into()], true)
            .unwrap();
        assert_eq!(
            db.one("select d from prs where repo='o/r' and num=1", vec![])
                .unwrap()["d"]["comment_seen_at"],
            "new"
        );
    }
}

#[cfg(test)]
mod provider_cooldown_tests {
    use super::*;
    #[test]
    fn cooldown_is_persistent_isolated_and_expires_without_extending_on_read() {
        let db = Db::mem();
        let at = 100_000;
        db.provider_failure("codex", "quota", at).unwrap();
        assert!(db.provider_cooldown("cc", at).unwrap().is_none());
        let first = db.provider_cooldown("codex", at).unwrap().unwrap();
        assert_eq!(first["until"], at + 60_000);
        assert_eq!(
            db.provider_cooldown("codex", at + 1).unwrap().unwrap(),
            first
        );
        assert!(db
            .provider_cooldown("codex", at + 60_000)
            .unwrap()
            .is_none());
        db.provider_failure("codex", "quota", at + 60_001).unwrap();
        let second = db.provider_cooldown("codex", at + 60_001).unwrap().unwrap();
        assert_eq!(second["count"], 2);
        assert_eq!(second["until"], at + 180_001);
        db.provider_failure("codex", "network", at).unwrap();
        assert_eq!(
            db.provider_cooldown("codex", at + 60_001).unwrap().unwrap(),
            second
        );
        assert!(db.provider_failure("unknown", "quota", at).is_err());
    }
    #[test]
    fn recovery_probe_is_exclusive_and_stale_results_cannot_clear_new_failure() {
        let db = Db::mem();
        let at = 100_000;
        db.provider_failure("codex", "network", at).unwrap();
        assert!(matches!(
            db.provider_admission("codex", at, 1000).unwrap(),
            ProviderAdmission::Blocked
        ));
        let ready = at + 30_000;
        let ProviderAdmission::Probe(first) = db.provider_admission("codex", ready, 1000).unwrap()
        else {
            panic!("expected recovery probe");
        };
        assert!(matches!(
            db.provider_admission("codex", ready + 1, 1000).unwrap(),
            ProviderAdmission::Blocked
        ));
        assert!(matches!(
            db.provider_admission("cc", ready, 1000).unwrap(),
            ProviderAdmission::Ready
        ));
        let ProviderAdmission::Probe(second) =
            db.provider_admission("codex", ready + 1000, 1000).unwrap()
        else {
            panic!("expected expired lease to recover");
        };
        assert!(!db
            .finish_provider_probe("codex", &first, ready + 1001, true)
            .unwrap());
        db.provider_failure("codex", "quota", ready + 1001).unwrap();
        assert!(!db
            .finish_provider_probe("codex", &second, ready + 1002, true)
            .unwrap());
        let next = ready + 61_001;
        let ProviderAdmission::Probe(third) = db.provider_admission("codex", next, 1000).unwrap()
        else {
            panic!("expected probe after new cooldown");
        };
        assert!(db
            .finish_provider_probe("codex", &third, next + 1, true)
            .unwrap());
        assert!(matches!(
            db.provider_admission("codex", next + 2, 1000).unwrap(),
            ProviderAdmission::Ready
        ));
    }

    #[test]
    fn corrupt_cooldown_is_not_treated_as_available() {
        let db = Db::mem();
        db.ks("provider:cooldown:cc", &json!({"kind":"quota"}));
        assert!(db.provider_cooldown("cc", 1000).is_err());
    }
}

#[cfg(test)]
mod hunter_duplicate_tests {
    use super::*;

    fn seed() -> Db {
        let db = Db::mem();
        db.run(
            "insert into rp(full,hunt,ci) values('o/r',1,'true'),('o/s',1,'true')",
            vec![],
        );
        db.run(
            "insert into jobs(id,repo,kind,st) values(100,'o/r','hunt','hunt'),(101,'o/s','hunt','hunt')",
            vec![],
        );
        db
    }

    #[test]
    fn exact_hunter_duplicate_is_atomic_and_scoped_to_repo_and_location() {
        let db = seed();
        let mut data = json!({"title":"Broken parser","body":"reproduction","hunt":{"where":"src/parser.rs","proof":"test failed for expected bug"}});
        assert_eq!(
            db.queue_hunter_find("o/r", 100, &data).unwrap(),
            HunterFinding::Queued
        );
        data["title"] = json!(" broken PARSER ");
        assert_eq!(
            db.queue_hunter_find("o/r", 100, &data).unwrap(),
            HunterFinding::Duplicate
        );
        assert_eq!(
            db.queue_hunter_find("o/s", 101, &data).unwrap(),
            HunterFinding::Queued
        );
        data["hunt"]["where"] = json!("src/other.rs");
        assert_eq!(
            db.queue_hunter_find("o/r", 100, &data).unwrap(),
            HunterFinding::Queued
        );
        db.run(
            "update jobs set st='done' where repo='o/r' and kind='fix'",
            vec![],
        );
        assert_eq!(
            db.queue_hunter_find("o/r", 100, &data).unwrap(),
            HunterFinding::Queued
        );
        data["hunt"]["where"] = json!("");
        assert!(db.queue_hunter_find("o/r", 100, &data).is_err());
        assert_eq!(
            db.n("select count(*) from jobs where kind='fix'", vec![]),
            4
        );
    }

    #[test]
    fn independent_connections_order_hunter_revocation_and_finding_writes() {
        let path = std::env::temp_dir().join(format!("apollo-hunt-race-{}", rand::random::<u64>()));
        std::fs::create_dir(&path).unwrap();
        let file = path.join("state.db");
        {
            let db = Db::open(&file).unwrap();
            db.run(
                "insert into rp(full,hunt,ci) values('o/r',1,'true')",
                vec![],
            );
            db.run(
                "insert into jobs(id,repo,kind,st) values(100,'o/r','hunt','hunt')",
                vec![],
            );
            let data = json!({"title":"Bug","body":"reproduction","hunt":{"where":"src/lib.rs","proof":"failed test"}});
            db.0.lock()
                .unwrap()
                .busy_timeout(std::time::Duration::ZERO)
                .unwrap();
            let mut other = Connection::open(&file).unwrap();
            // An independent writer owns the transaction before admission.
            let tx = other
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            tx.execute("update rp set hunt=0 where full='o/r'", [])
                .unwrap();
            let error = db.queue_hunter_find("o/r", 100, &data).unwrap_err();
            assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
            tx.commit().unwrap();
            assert_eq!(
                db.queue_hunter_find("o/r", 100, &data).unwrap(),
                HunterFinding::Revoked
            );
            assert_eq!(
                db.n("select count(*) from jobs where kind='fix'", vec![]),
                0
            );

            // Rollback must not revoke the committed authorization.
            other
                .execute("update rp set hunt=1 where full='o/r'", [])
                .unwrap();
            let tx = other
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            tx.execute("update jobs set st='drop' where id=100", [])
                .unwrap();
            let error = db.queue_hunter_find("o/r", 100, &data).unwrap_err();
            assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
            tx.rollback().unwrap();
            assert_eq!(
                db.queue_hunter_find("o/r", 100, &data).unwrap(),
                HunterFinding::Queued
            );

            // The opposite committed order preserves the earlier finding.
            other
                .execute("update jobs set st='drop' where id=100", [])
                .unwrap();
            assert_eq!(
                db.queue_hunter_find("o/r", 100, &data).unwrap(),
                HunterFinding::Revoked
            );
            assert_eq!(
                db.n("select count(*) from jobs where kind='fix'", vec![]),
                1
            );
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn hunter_find_requires_active_authorized_origin() {
        let db = seed();
        let data = json!({"title":"Broken parser","body":"reproduction","hunt":{"where":"src/parser.rs","proof":"test failed for expected bug"}});
        assert_eq!(
            db.queue_hunter_find("o/r", 101, &data).unwrap(),
            HunterFinding::Revoked
        );
        assert_eq!(
            db.queue_hunter_find("o/r", 999, &data).unwrap(),
            HunterFinding::Revoked
        );
        for (revoke, restore) in [
            (
                "update rp set on_=0 where full='o/r'",
                "update rp set on_=1 where full='o/r'",
            ),
            (
                "update rp set hunt=0 where full='o/r'",
                "update rp set hunt=1 where full='o/r'",
            ),
            (
                "update rp set ci=' ' where full='o/r'",
                "update rp set ci='true' where full='o/r'",
            ),
            (
                "update jobs set st='drop' where id=100",
                "update jobs set st='hunt' where id=100",
            ),
        ] {
            db.run(revoke, vec![]);
            assert_eq!(
                db.queue_hunter_find("o/r", 100, &data).unwrap(),
                HunterFinding::Revoked
            );
            db.run(restore, vec![]);
        }
        db.run("delete from rp where full='o/r'", vec![]);
        assert_eq!(
            db.queue_hunter_find("o/r", 100, &data).unwrap(),
            HunterFinding::Revoked
        );
        assert_eq!(
            db.n("select count(*) from jobs where kind='fix'", vec![]),
            0
        );
        assert_eq!(
            db.queue_hunter_find("o/s", 101, &data).unwrap(),
            HunterFinding::Queued
        );
    }
}

#[cfg(test)]
mod watcher_outbox_tests {
    use super::*;
    #[test]
    fn reply_is_atomic_with_review_and_never_claimed_twice() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![
                json!({"watcher_head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}).to_string()
            ],
        );
        let req = json!({"title":"Fix","url":"https://example.test","change":"Correct bug","data":{"source_head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"reply":{"author":"owner","body":"Acknowledged"}});
        assert!(db
            .queue_review_and_ack("o/r", 2, "t", &["r1".into()], &req)
            .is_err());
        assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 0);
        assert!(db
            .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &req)
            .unwrap());
        let row = db.one("select * from watcher_outbox", vec![]).unwrap();
        let id = row["id"].as_str().unwrap();
        assert!(row["body"]
            .as_str()
            .unwrap()
            .contains("apollo-office-reply:"));
        assert!(!db.claim_watcher_reply(id, "other").unwrap());
        assert!(db.claim_watcher_reply(id, "owner").unwrap());
        assert!(!db.claim_watcher_reply(id, "owner").unwrap());
        assert!(!db.finish_watcher_reply(id, "cancelled").unwrap());
        assert!(db.finish_watcher_reply(id, "sent").unwrap());
        assert!(!db.claim_watcher_reply(id, "owner").unwrap());
        db.run("update jobs set st='done'", vec![]);
        db.run("update prs set upd='next'", vec![]);
        assert!(db
            .queue_review_and_ack("o/r", 1, "next", &["r2".into()], &req)
            .unwrap());
        assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 1);
    }

    #[test]
    fn delivery_cursor_write_failures_preserve_previous_checkpoint() {
        let db = Db::mem();
        assert!(db
            .load_watcher_delivery_cursor("owner", "pending")
            .unwrap()
            .is_none());
        db.save_watcher_delivery_cursor("owner", "pending", 1, "a")
            .unwrap();
        db.run("create trigger ignore_cursor before update on kv when new.k='watcher:delivery-cursor:pending:owner' begin select raise(ignore); end", vec![]);
        assert!(db
            .save_watcher_delivery_cursor("owner", "pending", 2, "b")
            .is_err());
        assert_eq!(
            db.load_watcher_delivery_cursor("owner", "pending").unwrap(),
            Some((1, "a".into()))
        );
        db.run("drop trigger ignore_cursor", vec![]);
        db.save_watcher_delivery_cursor("owner", "pending", 0, "c")
            .unwrap();
        assert_eq!(
            db.load_watcher_delivery_cursor("owner", "pending").unwrap(),
            Some((0, "c".into()))
        );
        assert!(db
            .load_watcher_delivery_cursor("other", "pending")
            .unwrap()
            .is_none());
        assert!(db
            .load_watcher_delivery_cursor("owner", "uncertain")
            .unwrap()
            .is_none());
    }

    #[test]
    fn outbox_rotation_checks_exact_snapshot_and_persisted_timestamp() {
        for state in ["pending", "uncertain"] {
            let db = Db::mem();
            db.0.lock().unwrap().execute(
                "insert into watcher_outbox(id,repo,num,author,body,state,created,updated) values('a','o/r',1,'owner','reply',?,1,1)",
                [state],
            ).unwrap();
            for fault in [
                "create trigger ignore_rotation before update on watcher_outbox begin select raise(ignore); end;",
                "create trigger ignore_rotation after update on watcher_outbox when new.updated<>old.updated begin update watcher_outbox set updated=old.updated where id=new.id; end;",
            ] {
                db.0.lock().unwrap().execute_batch(fault).unwrap();
                assert!(db.rotate_watcher_reply("a", "owner", state, 1, 20).is_err());
                assert_eq!(db.one("select * from watcher_outbox", vec![]).unwrap()["updated"], 1);
                db.run("drop trigger ignore_rotation", vec![]);
            }
            assert!(!db.rotate_watcher_reply("a", "other", state, 1, 20).unwrap());
            assert!(db.rotate_watcher_reply("a", "owner", state, 1, 20).unwrap());
            assert!(!db.rotate_watcher_reply("a", "owner", state, 1, 30).unwrap());
            assert!(db
                .rotate_watcher_reply("a", "owner", state, 20, 10)
                .unwrap());
            let row = db.one("select * from watcher_outbox", vec![]).unwrap();
            assert_eq!(row["updated"], 20);
            assert_eq!(row["state"], state);
            let target = if state == "pending" {
                "cancelled"
            } else {
                "sent"
            };
            assert!(db.finish_watcher_reply("a", target).unwrap());
            assert!(!db
                .rotate_watcher_reply("a", "owner", state, 20, 40)
                .unwrap());
        }
    }

    #[test]
    fn reply_claim_requires_durable_uncertain_state_before_post() {
        let db = Db::mem();
        let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":head}).to_string()],
        );
        db.0.lock().unwrap().execute(
            "insert into watcher_outbox(id,repo,num,author,body,state,created,updated,source_head) values('a','o/r',1,'owner','reply','pending',1,1,?)",
            [head],
        ).unwrap();
        for fault in [
            "create trigger fail_claim before update on watcher_outbox when new.state='uncertain' begin select raise(ignore); end;",
            "create trigger fail_claim after update on watcher_outbox when new.state='uncertain' begin update watcher_outbox set state='pending' where id=new.id; end;",
        ] {
            db.0.lock().unwrap().execute_batch(fault).unwrap();
            assert!(!db.claim_watcher_reply("missing", "owner").unwrap());
            assert!(!db.claim_watcher_reply("a", "other").unwrap());
            assert!(db.claim_watcher_reply("a", "owner").is_err());
            let row = db.one("select * from watcher_outbox", vec![]).unwrap();
            assert_eq!(row["state"], "pending");
            assert_eq!(row["updated"], 1);
            db.run("drop trigger fail_claim", vec![]);
        }
        assert!(db.claim_watcher_reply("a", "owner").unwrap());
        assert_eq!(db.watcher_outbox_counts().unwrap()["uncertain"], 1);
        assert!(!db.claim_watcher_reply("a", "owner").unwrap());
        assert!(db.finish_watcher_reply("a", "sent").unwrap());
        assert!(!db.claim_watcher_reply("a", "owner").unwrap());
    }

    #[test]
    fn ignored_reply_finish_is_an_error_and_never_requeues_uncertain_delivery() {
        for (source, target) in [("uncertain", "sent"), ("pending", "cancelled")] {
            let db = Db::mem();
            {
                let c = db.0.lock().unwrap();
                c.execute(
                    "insert into watcher_outbox(id,repo,num,author,body,state,created,updated) values('a','o/r',1,'owner','reply',?,1,1)",
                    [source],
                )
                .unwrap();
                c.execute_batch(
                    "create trigger ignore_finish before update on watcher_outbox
                     begin select raise(ignore); end;",
                )
                .unwrap();
            }
            assert!(db.finish_watcher_reply("a", target).is_err());
            assert_eq!(db.watcher_outbox_counts().unwrap()[source], 1);
            assert_eq!(db.watcher_outbox_counts().unwrap()[target], 0);
            assert!(!db.finish_watcher_reply("missing", target).unwrap());
            db.run("drop trigger ignore_finish", vec![]);
            assert!(db.finish_watcher_reply("a", target).unwrap());
            assert!(!db.finish_watcher_reply("a", target).unwrap());
            assert_eq!(db.watcher_outbox_counts().unwrap()[target], 1);
            assert!(!db.claim_watcher_reply("a", "owner").unwrap());
        }
    }

    #[test]
    fn ignored_outbox_revocation_rolls_back_lifecycle_mutation() {
        let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        for mutation in [
            "update rp set on_=0 where full='o/r'",
            "update rp set full='o/other' where full='o/r'",
            "delete from rp where full='o/r'",
            "update prs set st='closed' where repo='o/r' and num=1",
            "delete from prs where repo='o/r' and num=1",
            "update prs set d='{}' where repo='o/r' and num=1",
        ] {
            let db = Db::mem();
            db.run("insert into rp(full) values('o/r')", vec![]);
            db.run(
                "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
                crate::pa![json!({"watcher_head":head}).to_string()],
            );
            {
                let c = db.0.lock().unwrap();
                for (id, state) in [("a", "pending"), ("b", "uncertain"), ("c", "sent")] {
                    c.execute(
                        "insert into watcher_outbox(id,repo,num,author,body,state,created,updated,source_head) values(?,'o/r',1,'owner','reply',?,1,1,?)",
                        rusqlite::params![id, state, head],
                    )
                    .unwrap();
                }
                c.execute_batch(
                    "create trigger ignore_outbox_revoke before update on watcher_outbox
                     when new.state='cancelled' begin select raise(ignore); end;",
                )
                .unwrap();
                assert!(c.execute(mutation, []).is_err());
                c.execute_batch("drop trigger ignore_outbox_revoke")
                    .unwrap();
            }
            assert_eq!(
                db.n("select count(*) from rp where full='o/r' and on_=1", vec![]),
                1
            );
            assert_eq!(db.n("select count(*) from prs where st='open' and json_extract(d,'$.watcher_head') is not null", vec![]), 1);
            assert_eq!(db.watcher_outbox_counts().unwrap()["pending"], 1);
            db.0.lock().unwrap().execute(mutation, []).unwrap();
            let counts = db.watcher_outbox_counts().unwrap();
            assert_eq!(counts["cancelled"], 1);
            assert_eq!(counts["uncertain"], 1);
            assert_eq!(counts["sent"], 1);
            assert!(!db.claim_watcher_reply("a", "owner").unwrap());
        }
    }

    #[test]
    fn repository_revocation_cancels_pending_but_preserves_uncertain_replies() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r'),('other/r')", vec![]);
        db.run("insert into watcher_outbox(id,repo,num,author,body,state,created,updated) values('a','o/r',1,'owner','reply','pending',1,1),('b','o/r',1,'owner','reply','uncertain',1,1),('c','other/r',1,'owner','reply','pending',1,1)",vec![]);
        db.run("update rp set on_=0 where full='o/r'", vec![]);
        assert!(!db.claim_watcher_reply("a", "owner").unwrap());
        assert_eq!(db.watcher_outbox_counts().unwrap()["cancelled"], 1);
        assert_eq!(db.watcher_outbox_counts().unwrap()["uncertain"], 1);
        db.run("delete from rp where full='other/r'", vec![]);
        assert!(!db.claim_watcher_reply("c", "owner").unwrap());
        db.run("insert into rp(full) values('other/r')", vec![]);
        assert!(!db.claim_watcher_reply("c", "owner").unwrap());
        assert_eq!(db.watcher_outbox_counts().unwrap()["cancelled"], 2);
        assert!(db.finish_watcher_reply("b", "sent").unwrap());
    }

    #[test]
    fn terminal_or_deleted_pr_cancels_only_pending_replies() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run("insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t','{}'),('o/r',2,'open','t','{}'),('o/r',3,'open','t','{}')",vec![]);
        db.run(
            "update prs set d=json_object('watcher_head',?)",
            crate::pa!["a".repeat(40)],
        );
        db.run("insert into watcher_outbox(id,repo,num,author,body,state,created,updated) values('a','o/r',1,'owner','reply','pending',1,1),('b','o/r',1,'owner','reply','uncertain',1,1),('c','o/r',2,'owner','reply','pending',1,1),('d','o/r',3,'owner','reply','pending',1,1),('e','o/r',99,'owner','reply','pending',1,1)",vec![]);
        db.run(
            "update watcher_outbox set source_head=?",
            crate::pa!["a".repeat(40)],
        );
        db.run("update prs set st='merged' where num=1", vec![]);
        assert!(!db.claim_watcher_reply("a", "owner").unwrap());
        db.run("update prs set st='open' where num=1", vec![]);
        assert!(!db.claim_watcher_reply("a", "owner").unwrap());
        db.run("delete from prs where num=2", vec![]);
        assert!(!db.claim_watcher_reply("c", "owner").unwrap());
        assert!(!db.claim_watcher_reply("e", "owner").unwrap());
        assert!(db.claim_watcher_reply("d", "owner").unwrap());
        assert_eq!(db.watcher_outbox_counts().unwrap()["cancelled"], 2);
        assert_eq!(db.watcher_outbox_counts().unwrap()["uncertain"], 2);
        assert!(db.finish_watcher_reply("b", "sent").unwrap());
    }

    #[test]
    fn changed_head_revokes_pending_reply_without_revival() {
        let db = Db::mem();
        let head = "a".repeat(40);
        let next = "b".repeat(40);
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":head}).to_string()],
        );
        let reply = json!({"author":"owner","body":"Thanks"});
        db.ack_pr_comments_with_reply("o/r", 1, "t", &head, &["r1".into()], &reply)
            .unwrap();
        let row = db.one("select * from watcher_outbox", vec![]).unwrap();
        let id = row["id"].as_str().unwrap();
        db.run("update prs set d=json_set(d,'$.seen',json('true'))", vec![]);
        assert_eq!(db.watcher_outbox_counts().unwrap()["pending"], 1);
        db.run(
            "update prs set d=?",
            crate::pa![json!({"watcher_head":next}).to_string()],
        );
        assert_eq!(db.watcher_outbox_counts().unwrap()["cancelled"], 1);
        db.run(
            "update prs set d=?",
            crate::pa![json!({"watcher_head":head}).to_string()],
        );
        assert!(!db.claim_watcher_reply(id, "owner").unwrap());
    }

    #[test]
    fn startup_cancels_legacy_unbound_pending_but_keeps_history() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":"a".repeat(40)}).to_string()],
        );
        db.run("insert into watcher_outbox(id,repo,num,author,body,state,created,updated) values('a','o/r',1,'owner','reply','pending',1,1),('b','o/r',1,'owner','reply','uncertain',1,1),('c','o/r',1,'owner','reply','sent',1,1)",vec![]);
        {
            let mut c = db.0.lock().unwrap();
            init(&mut c).unwrap();
        }
        let counts = db.watcher_outbox_counts().unwrap();
        assert_eq!(counts["cancelled"], 1);
        assert_eq!(counts["uncertain"], 1);
        assert_eq!(counts["sent"], 1);
    }

    #[test]
    fn revoked_repository_cannot_admit_or_acknowledge_review() {
        let db = Db::mem();
        let head = "a".repeat(40);
        let ids = vec!["r1".into()];
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":head}).to_string()],
        );
        db.receive_review_batch("o/r", 1, "t", &head, &ids).unwrap();
        let request = json!({"title":"Fix","url":"https://example.test","change":"Correct bug","data":{"source_head":head}});
        for sql in ["update rp set on_=0", "delete from rp"] {
            db.run(sql, vec![]);
            assert!(db
                .queue_review_and_ack("o/r", 1, "t", &ids, &request)
                .is_err());
            assert!(db
                .ack_pr_comments_with_reply("o/r", 1, "t", &head, &ids, &Value::Null)
                .is_err());
            assert!(!db.comment_acknowledged("o/r", "r1").unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
        }
    }

    #[test]
    fn stale_snapshot_cannot_acknowledge_or_enqueue_review() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        let head = "a".repeat(40);
        let newer = "b".repeat(40);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','new',?)",
            crate::pa![json!({"watcher_head":newer}).to_string()],
        );
        let request = json!({"title":"Fix","url":"https://example.test","change":"Correct bug","data":{"source_head":head},"reply":{"author":"owner","body":"Acknowledged"}});
        let ids = vec!["r1".into()];
        assert!(db
            .queue_review_and_ack("o/r", 1, "new", &ids, &request)
            .is_err());
        assert!(db
            .ack_pr_comments_with_reply("o/r", 1, "new", &head, &ids, &request["reply"])
            .is_err());
        assert!(!db.comment_acknowledged("o/r", "r1").unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
        assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 0);
    }

    #[test]
    fn owner_inventory_prioritizes_unresolved_and_cancel_cannot_race_claim() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t','{}')",
            vec![],
        );
        db.run(
            "update prs set d=json_object('watcher_head',?)",
            crate::pa!["a".repeat(40)],
        );
        for (id, state, at) in [("a", "sent", 3), ("b", "pending", 1), ("c", "uncertain", 2)] {
            db.run("insert into watcher_outbox(id,repo,num,author,body,state,created,updated) values(?,'o/r',1,'owner','reply',?,?,?)",crate::pa![id.to_string(),state.to_string(),at,at]);
        }
        db.run(
            "update watcher_outbox set source_head=?",
            crate::pa!["a".repeat(40)],
        );
        let rows = db.watcher_outbox_page(0).unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["c", "b", "a"]
        );
        assert!(db.watcher_outbox_page(-1).is_err());
        assert!(db.claim_watcher_reply("b", "owner").unwrap());
        assert!(!db.finish_watcher_reply("b", "cancelled").unwrap());
        assert_eq!(db.watcher_outbox_counts().unwrap()["uncertain"], 2);
    }

    #[test]
    fn failed_ack_rolls_back_reply_and_replay_does_not_queue() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t','bad json')",
            vec![],
        );
        let reply = json!({"author":"owner","body":"Thanks"});
        assert!(db
            .ack_pr_comments_with_reply(
                "o/r",
                1,
                "t",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &["r1".into()],
                &reply
            )
            .is_err());
        assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 0);
        assert!(!db.comment_acknowledged("o/r", "r1").unwrap());
        db.run(
            "update prs set d=?",
            crate::pa![
                json!({"watcher_head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}).to_string()
            ],
        );
        db.ack_pr_comments_with_reply(
            "o/r",
            1,
            "t",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &["r1".into()],
            &reply,
        )
        .unwrap();
        assert!(db
            .ack_pr_comments_with_reply(
                "o/r",
                1,
                "t",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &["r1".into()],
                &reply
            )
            .is_err());
        assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 1);
    }
}

#[cfg(test)]
mod watcher_inbox_tests {
    use super::*;
    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[test]
    fn receipt_survives_failed_ack_and_replay_is_idempotent() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":HEAD}).to_string()],
        );
        let ids = vec!["r1".to_string()];
        db.receive_review_batch("o/r", 1, "t", HEAD, &ids).unwrap();
        db.receive_review_batch("o/r", 1, "t", HEAD, &ids).unwrap();
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='received'",
                vec![]
            ),
            1
        );
        assert!(db
            .ack_pr_comments_with_reply("o/r", 1, "wrong", HEAD, &ids, &Value::Null)
            .is_err());
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='received'",
                vec![]
            ),
            1
        );
        db.ack_pr_comments_with_reply("o/r", 1, "t", HEAD, &ids, &Value::Null)
            .unwrap();
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='done'",
                vec![]
            ),
            1
        );
        db.receive_review_batch("o/r", 1, "t", HEAD, &ids).unwrap();
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='done'",
                vec![]
            ),
            1
        );
    }
    #[test]
    fn new_head_supersedes_only_unfinished_old_receipts() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        let next = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":HEAD}).to_string()],
        );
        let ids = vec!["r2".to_string()];
        db.receive_review_batch("o/r", 1, "t", HEAD, &ids).unwrap();
        db.run(
            "update prs set upd='t2',d=? where repo='o/r' and num=1",
            crate::pa![json!({"watcher_head":next}).to_string()],
        );
        db.receive_review_batch("o/r", 1, "t2", next, &ids).unwrap();
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='superseded'",
                vec![]
            ),
            1
        );
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='received'",
                vec![]
            ),
            1
        );
        assert!(db
            .ack_pr_comments_with_reply("o/r", 1, "t", HEAD, &ids, &Value::Null)
            .is_err());
        db.ack_pr_comments_with_reply("o/r", 1, "t2", next, &ids, &Value::Null)
            .unwrap();
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='done'",
                vec![]
            ),
            1
        );
    }
}

#[cfg(test)]
mod watcher_retry_tests {
    use super::*;

    #[test]
    fn analysis_ticket_is_exclusive_and_stale_release_cannot_clear_replacement() {
        let db = Db::mem();
        let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":head}).to_string()],
        );
        let first = db
            .claim_review_attempt("o/r", 1, "t", head, 0)
            .unwrap()
            .unwrap();
        assert!(db
            .claim_review_attempt("o/r", 1, "t", head, 600000)
            .unwrap()
            .is_none());
        let original = db.try_all("select * from watcher_retry", vec![]).unwrap();
        assert_eq!(original[0]["analysis_ticket"], first.ticket);
        assert_eq!(original[0]["next_attempt_at"], 600000);
        // Use production startup recovery, not a simulated ticket update.
        let app = std::sync::Arc::new(crate::app::App::new(
            db.clone(),
            std::env::temp_dir(),
            1,
            "apollo-office".into(),
        ));
        crate::pipe::recover(&app).unwrap();
        let recovered = db.try_all("select * from watcher_retry", vec![]).unwrap();
        assert!(recovered[0]["analysis_ticket"].is_null());
        assert_eq!(recovered[0]["next_attempt_at"], 600000);
        assert!(db
            .claim_review_attempt("o/r", 1, "t", head, 599999)
            .unwrap()
            .is_none());
        let second = db
            .claim_review_attempt("o/r", 1, "t", head, 600000)
            .unwrap()
            .unwrap();
        assert_ne!(first.ticket, second.ticket);
        drop(first);
        let active = db.try_all("select * from watcher_retry", vec![]).unwrap();
        assert_eq!(active[0]["analysis_ticket"], second.ticket);
        drop(second);
        let waiting = db.try_all("select * from watcher_retry", vec![]).unwrap();
        assert!(waiting[0]["analysis_ticket"].is_null());
        assert_eq!(waiting[0]["next_attempt_at"], 1800000);
    }

    #[test]
    fn ignored_analysis_release_is_reported_and_keeps_retry_deadline() {
        let db = Db::mem();
        let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":head}).to_string()],
        );
        let attempt = db
            .claim_review_attempt("o/r", 1, "t", head, 0)
            .unwrap()
            .unwrap();
        let before = db.try_all("select * from watcher_retry", vec![]).unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                "create trigger ignore_release before update on watcher_retry
                 when new.analysis_ticket is null begin select raise(ignore); end;",
            )
            .unwrap();
        assert!(attempt.release().is_err());
        assert_eq!(
            db.try_all("select * from watcher_retry", vec![]).unwrap(),
            before
        );
        assert!(db
            .claim_review_attempt("o/r", 1, "t", head, 600000)
            .unwrap()
            .is_none());
        db.run("drop trigger ignore_release", vec![]);
        let app = std::sync::Arc::new(crate::app::App::new(
            db.clone(),
            std::env::temp_dir(),
            1,
            "apollo-office".into(),
        ));
        crate::pipe::recover(&app).unwrap();
        assert!(db
            .claim_review_attempt("o/r", 1, "t", head, 599999)
            .unwrap()
            .is_none());
        let next = db
            .claim_review_attempt("o/r", 1, "t", head, 600000)
            .unwrap()
            .unwrap();
        next.release().unwrap();
        let rows = db.try_all("select * from watcher_retry", vec![]).unwrap();
        assert!(rows[0]["analysis_ticket"].is_null());
        assert_eq!(rows[0]["next_attempt_at"], 1800000);
    }

    #[test]
    fn legacy_retry_migration_preserves_deadline_and_does_not_clear_live_ticket() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(&SCH.replace("analysis_ticket text,", ""))
            .unwrap();
        c.execute_batch(
            "pragma user_version=6;
             insert into prs(repo,num,st) values('o/r',1,'open');
             insert into watcher_retry values('o/r',1,'head','snapshot',3,1234567);",
        )
        .unwrap();
        init(&mut c).unwrap();
        let row: (i64, i64, Option<String>) = c
            .query_row(
                "select attempts,next_attempt_at,analysis_ticket from watcher_retry",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, (3, 1234567, None));
        c.execute("update watcher_retry set analysis_ticket='live'", [])
            .unwrap();
        init(&mut c).unwrap();
        let ticket: String = c
            .query_row("select analysis_ticket from watcher_retry", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ticket, "live");
    }

    #[test]
    fn retry_window_is_persistent_and_snapshot_scoped() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":head}).to_string()],
        );
        let ids = vec!["r1".to_string()];
        db.receive_review_batch("o/r", 1, "t", head, &ids).unwrap();
        assert!(db.has_pending_review("o/r", 1).unwrap());
        assert!(db
            .claim_review_attempt("o/r", 1, "t", head, 0)
            .unwrap()
            .is_some());
        assert!(!db
            .claim_review_attempt("o/r", 1, "t", head, 599999)
            .unwrap()
            .is_some());
        assert!(db
            .claim_review_attempt("o/r", 1, "t", head, 600000)
            .unwrap()
            .is_some());
        assert!(!db
            .claim_review_attempt("o/r", 1, "t", head, 1799999)
            .unwrap()
            .is_some());
        assert!(db
            .claim_review_attempt("o/r", 1, "t", head, 1800000)
            .unwrap()
            .is_some());
        db.run("update prs set upd='t2' where repo='o/r' and num=1", vec![]);
        assert!(db
            .claim_review_attempt("o/r", 1, "t2", head, 1800001)
            .unwrap()
            .is_some());
        assert!(db
            .claim_review_attempt("o/r", 1, "t", head, 1800002)
            .is_err());
        db.ack_pr_comments_with_reply("o/r", 1, "t2", head, &ids, &Value::Null)
            .unwrap();
        assert!(!db.has_pending_review("o/r", 1).unwrap());
        assert_eq!(db.n("select count(*) from watcher_retry", vec![]), 0);
    }
}

#[cfg(test)]
mod watcher_receipt_lifecycle_tests {
    use super::*;
    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn setup() -> Db {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":HEAD}).to_string()],
        );
        db
    }
    #[test]
    fn terminal_pr_retires_only_pending_receipts_and_retry() {
        let db = setup();
        let done = vec!["r1".to_string()];
        db.receive_review_batch("o/r", 1, "t", HEAD, &done).unwrap();
        db.ack_pr_comments_with_reply("o/r", 1, "t", HEAD, &done, &Value::Null)
            .unwrap();
        db.receive_review_batch("o/r", 1, "t", HEAD, &["r2".into()])
            .unwrap();
        let _attempt = db.claim_review_attempt("o/r", 1, "t", HEAD, 0).unwrap();
        db.run(
            "update prs set st='closed' where repo='o/r' and num=1",
            vec![],
        );
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='done'",
                vec![]
            ),
            1
        );
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='superseded'",
                vec![]
            ),
            1
        );
        assert_eq!(db.n("select count(*) from watcher_retry", vec![]), 0);
        assert!(!db.comment_acknowledged("o/r", "r2").unwrap());
    }
    #[test]
    fn ignored_terminal_cleanup_rolls_back_pr_transition() {
        for mutation in [
            "update prs set st='closed' where repo='o/r' and num=1",
            "update prs set st='merged' where repo='o/r' and num=1",
            "delete from prs where repo='o/r' and num=1",
        ] {
            for fault in [
                "create trigger ignore_retirement before update on watcher_inbox when new.state='superseded' begin select raise(ignore); end;",
                "create trigger ignore_retirement before delete on watcher_retry begin select raise(ignore); end;",
            ] {
                let db = setup();
                db.receive_review_batch("o/r", 1, "t", HEAD, &["r1".into()])
                    .unwrap();
                let attempt = db
                    .claim_review_attempt("o/r", 1, "t", HEAD, 0)
                    .unwrap()
                    .unwrap();
                {
                    let c = db.0.lock().unwrap();
                    c.execute_batch(fault).unwrap();
                    assert!(c.execute(mutation, []).is_err());
                    c.execute_batch("drop trigger ignore_retirement").unwrap();
                }
                assert_eq!(db.n("select count(*) from prs where st='open'", vec![]), 1);
                assert!(db.has_pending_review("o/r", 1).unwrap());
                assert_eq!(db.n("select count(*) from watcher_retry", vec![]), 1);
                db.0.lock().unwrap().execute(mutation, []).unwrap();
                assert!(!db.has_pending_review("o/r", 1).unwrap());
                assert_eq!(db.n("select count(*) from watcher_retry", vec![]), 0);
                assert!(!db.comment_acknowledged("o/r", "r1").unwrap());
                attempt.release().unwrap();
            }
        }
    }
    #[test]
    fn absent_receipt_is_not_acknowledged_and_can_return() {
        let db = setup();
        let ids = vec!["r1".to_string(), "r2".to_string()];
        db.receive_review_batch("o/r", 1, "t", HEAD, &ids).unwrap();
        assert!(db
            .reconcile_review_receipts("o/r", 1, "wrong", HEAD, &[])
            .is_err());
        assert_eq!(
            db.reconcile_review_receipts("o/r", 1, "t", HEAD, &ids[..1])
                .unwrap(),
            1
        );
        assert!(!db.comment_acknowledged("o/r", "r2").unwrap());
        db.receive_review_batch("o/r", 1, "t", HEAD, &ids).unwrap();
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='received'",
                vec![]
            ),
            2
        );
        db.run("delete from prs where repo='o/r' and num=1", vec![]);
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='superseded'",
                vec![]
            ),
            2
        );
    }
}

#[cfg(test)]
mod watcher_rebase_admission_tests {
    use super::*;
    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[test]
    fn rebase_admission_respects_snapshot_and_other_pr_operations() {
        let db = Db::mem();
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![
                json!({"watcher_head":HEAD,"base":"main","default_branch":"main"}).to_string()
            ],
        );
        let token = db.one("select steward_activation from rp", vec![]).unwrap()
            ["steward_activation"]
            .clone();
        let mut request = json!({"title":"PR","url":"https://github.com/o/r/pull/1","why":"conflict","data":{"branch":"fix","head":"o/r","steward_rebase_base_ref":"main","steward_rebase_base_sha":"b".repeat(40),"steward_rebase_activation":token}});
        for keys in [
            vec!["steward_rebase_base_ref"],
            vec!["steward_rebase_base_sha"],
            vec!["steward_rebase_activation"],
            vec![
                "steward_rebase_base_ref",
                "steward_rebase_base_sha",
                "steward_rebase_activation",
            ],
        ] {
            let mut incomplete = request.clone();
            for key in keys {
                incomplete["data"].as_object_mut().unwrap().remove(key);
            }
            assert!(db
                .queue_watcher_rebase("o/r", 1, "t", HEAD, &incomplete)
                .is_err());
        }
        assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
        assert!(db
            .queue_watcher_rebase("o/r", 1, "wrong", HEAD, &request)
            .is_err());
        db.run("update rp set on_=0", vec![]);
        assert!(db
            .queue_watcher_rebase("o/r", 1, "t", HEAD, &request)
            .is_err());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
        db.run("update rp set on_=1", vec![]);
        assert!(db
            .queue_watcher_rebase("o/r", 1, "t", HEAD, &request)
            .is_err());
        request["data"]["steward_rebase_activation"] =
            db.one("select steward_activation from rp", vec![]).unwrap()["steward_activation"]
                .clone();
        let job = db.jnew(
            "o/r",
            1,
            "review",
            "PR",
            "url",
            "ready",
            "review",
            json!({}),
            Some(1),
            -1.0,
        );
        assert!(!db
            .queue_watcher_rebase("o/r", 1, "t", HEAD, &request)
            .unwrap());
        db.run("update jobs set st='done' where id=?", crate::pa![job]);
        assert!(db
            .queue_watcher_rebase("o/r", 1, "t", HEAD, &request)
            .unwrap());
        assert!(!db
            .queue_watcher_rebase("o/r", 1, "t", HEAD, &request)
            .unwrap());
        let row = db
            .one("select * from jobs where kind='rebase'", vec![])
            .unwrap();
        assert_eq!(row["d"]["watcher_observed_head"], HEAD);
        db.run("update jobs set st='done' where kind='rebase'", vec![]);
        assert!(!db
            .queue_watcher_rebase("o/r", 1, "t", HEAD, &request)
            .unwrap());
    }
}

#[cfg(test)]
mod watcher_storage_failure_tests {
    use super::*;

    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn seed(db: &Db) {
        db.run("insert into rp(full) values('o/r')", vec![]);
        db.run(
            "insert into prs(repo,num,st,upd,d) values('o/r',1,'open','t',?)",
            crate::pa![json!({"watcher_head":HEAD}).to_string()],
        );
        db.receive_review_batch("o/r", 1, "t", HEAD, &["r1".into()])
            .unwrap();
    }

    fn request() -> Value {
        json!({"title":"Fix","url":"https://github.com/o/r/pull/1","change":"Correct bug","data":{"source_head":HEAD},"reply":{"author":"maintainer","body":"Acknowledged"}})
    }

    fn assert_pending(db: &Db) {
        assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
        assert_eq!(db.n("select count(*) from cm", vec![]), 0);
        assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 0);
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='received'",
                vec![]
            ),
            1
        );
    }

    #[test]
    fn busy_writer_leaves_receipt_pending_and_allows_retry_after_restart() {
        let path = std::env::temp_dir().join(format!("apollo-db-busy-{}", rand::random::<u64>()));
        std::fs::create_dir(&path).unwrap();
        let file = path.join("state.db");
        {
            let db = Db::open(&file).unwrap();
            seed(&db);
            db.0.lock()
                .unwrap()
                .busy_timeout(std::time::Duration::ZERO)
                .unwrap();
            let mut other = Connection::open(&file).unwrap();
            let lock = other
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let error = db
                .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                .unwrap_err();
            assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
            lock.rollback().unwrap();
            assert_pending(&db);
        }
        {
            let db = Db::open(&file).unwrap();
            assert_pending(&db);
            assert!(db
                .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                .unwrap());
            assert!(db.comment_acknowledged("o/r", "r1").unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    fn rebase_request(db: &Db) -> Value {
        let token = db
            .one("select steward_activation from rp where full='o/r'", vec![])
            .unwrap()["steward_activation"]
            .clone();
        json!({"title":"PR","url":"https://github.com/o/r/pull/1","why":"conflict",
            "data":{"branch":"fix","head":"o/r","steward_rebase_base_ref":"main",
            "steward_rebase_base_sha":"b".repeat(40),"steward_rebase_activation":token}})
    }

    #[test]
    fn review_job_must_survive_acknowledgement_with_exact_payload() {
        for fault in [
            "create trigger mutate_job after insert on jobs when new.kind='review' begin update jobs set d='{}' where id=new.id; end;",
            "create trigger mutate_job after insert on cm begin delete from jobs where kind='review'; end;",
        ] {
            let db = Db::mem();
            seed(&db);
            let ids = vec!["r1".into()];
            db.0.lock().unwrap().execute_batch(fault).unwrap();
            assert!(db
                .queue_review_and_ack("o/r", 1, "t", &ids, &request())
                .is_err());
            assert_pending(&db);
            db.run("drop trigger mutate_job", vec![]);
            assert!(db
                .queue_review_and_ack("o/r", 1, "t", &ids, &request())
                .unwrap());
            assert!(db.comment_acknowledged("o/r", "r1").unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
        }
    }

    #[test]
    fn rebase_admission_rejects_mutated_or_deleted_job_after_insert() {
        let db = Db::mem();
        seed(&db);
        db.run(
            "update prs set d=json_set(d,'$.base','main','$.default_branch','main')",
            vec![],
        );
        let request = rebase_request(&db);
        for fault in [
            "create trigger mutate_rebase after insert on jobs when new.kind='rebase' begin update jobs set d='{}' where id=new.id; end;",
            "create trigger mutate_rebase after insert on jobs when new.kind='rebase' begin update jobs set st='drop' where id=new.id; end;",
            "create trigger mutate_rebase after insert on jobs when new.kind='rebase' begin delete from jobs where id=new.id; end;",
        ] {
            db.0.lock().unwrap().execute_batch(fault).unwrap();
            assert!(db
                .queue_watcher_rebase("o/r", 1, "t", HEAD, &request)
                .is_err());
            assert_pending(&db);
            db.0.lock().unwrap().execute_batch("drop trigger mutate_rebase").unwrap();
        }
        assert!(db
            .queue_watcher_rebase("o/r", 1, "t", HEAD, &request)
            .unwrap());
        let row = db
            .one("select * from jobs where kind='rebase'", vec![])
            .unwrap();
        assert_eq!(row["d"]["watcher_observed_head"], HEAD);
        assert_eq!(
            row["d"]["steward_rebase_activation"],
            request["data"]["steward_rebase_activation"]
        );
        assert!(!db
            .queue_watcher_rebase("o/r", 1, "t", HEAD, &request)
            .unwrap());
    }

    #[test]
    fn ignored_job_insert_never_acknowledges_review_or_reports_rebase_success() {
        let db = Db::mem();
        seed(&db);
        db.run(
            "update prs set d=json_set(d,'$.base','main','$.default_branch','main')",
            vec![],
        );
        let rebase = rebase_request(&db);
        db.0.lock()
            .unwrap()
            .execute_batch(
                "create trigger ignore_pr_job before insert on jobs
             when new.kind in ('review','rebase') begin select raise(ignore); end;",
            )
            .unwrap();
        assert!(db
            .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
            .is_err());
        assert!(db
            .queue_watcher_rebase("o/r", 1, "t", HEAD, &rebase)
            .is_err());
        assert_pending(&db);
        db.run("drop trigger ignore_pr_job", vec![]);
        assert!(db
            .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
            .unwrap());
        assert!(db.comment_acknowledged("o/r", "r1").unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
    }

    #[test]
    fn concurrent_steward_rebase_and_watcher_review_admit_only_one_operation() {
        let path = std::env::temp_dir().join(format!(
            "apollo-pr-operation-race-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        let file = path.join("state.db");
        {
            let db = Db::open(&file).unwrap();
            seed(&db);
            db.run(
                "update prs set d=json_set(d,'$.base','main','$.default_branch','main')",
                vec![],
            );
            let other = Db::open(&file).unwrap();
            let rebase = rebase_request(&db);
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let other_barrier = barrier.clone();
            let contender = std::thread::spawn(move || {
                other_barrier.wait();
                other
                    .queue_watcher_rebase("o/r", 1, "t", HEAD, &rebase)
                    .unwrap()
            });
            barrier.wait();
            let review_won = db
                .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                .unwrap();
            let rebase_won = contender.join().unwrap();
            assert_ne!(review_won, rebase_won);
            assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
            assert_eq!(db.comment_acknowledged("o/r", "r1").unwrap(), review_won);
            assert_eq!(
                db.n("select count(*) from watcher_outbox", vec![]),
                i64::from(review_won)
            );
            assert_eq!(
                db.n(
                    "select count(*) from watcher_inbox where state='received'",
                    vec![]
                ),
                i64::from(rebase_won)
            );
            if rebase_won {
                db.run("update jobs set st='done' where kind='rebase'", vec![]);
                assert!(db
                    .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                    .unwrap());
            }
            assert!(db.comment_acknowledged("o/r", "r1").unwrap());
            assert_eq!(
                db.n("select count(*) from jobs where st='ready'", vec![]),
                1
            );
            assert!(!db
                .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                .unwrap());
        }
        {
            let db = Db::open(&file).unwrap();
            assert!(db.comment_acknowledged("o/r", "r1").unwrap());
            assert_eq!(
                db.n("select count(*) from jobs where kind='review'", vec![]),
                1
            );
            assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 1);
            assert_eq!(
                db.n(
                    "select count(*) from watcher_inbox where state='received'",
                    vec![]
                ),
                0
            );
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn concurrent_connections_admit_exactly_one_review() {
        let path = std::env::temp_dir().join(format!("apollo-db-race-{}", rand::random::<u64>()));
        std::fs::create_dir(&path).unwrap();
        let file = path.join("state.db");
        {
            let db = Db::open(&file).unwrap();
            seed(&db);
            let other = Db::open(&file).unwrap();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let results = std::thread::scope(|scope| {
                let gate = barrier.clone();
                let first_db = &db;
                let first = scope.spawn(move || {
                    gate.wait();
                    first_db.queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                });
                let second = scope.spawn(|| {
                    barrier.wait();
                    other.queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                });
                [
                    first.join().unwrap().unwrap(),
                    second.join().unwrap().unwrap(),
                ]
            });
            assert_eq!(results.into_iter().filter(|value| *value).count(), 1);
            assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
            assert_eq!(db.n("select count(*) from cm", vec![]), 1);
            assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 1);
            assert_eq!(
                db.n(
                    "select count(*) from watcher_inbox where state='done'",
                    vec![]
                ),
                1
            );
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn full_database_does_not_consume_review_receipt() {
        let db = Db::mem();
        seed(&db);
        let pages: i64 =
            db.0.lock()
                .unwrap()
                .pragma_query_value(None, "page_count", |r| r.get(0))
                .unwrap();
        db.0.lock()
            .unwrap()
            .pragma_update(None, "max_page_count", pages)
            .unwrap();
        let mut req = request();
        req["change"] = json!("x".repeat(1_048_576));
        let error = db
            .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &req)
            .unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DiskFull)
        );
        assert_pending(&db);
    }

    #[test]
    fn ignored_watcher_effects_roll_back_job_and_ack() {
        for sql in [
            "create trigger ignore_effect before insert on watcher_outbox begin select raise(ignore); end;",
            "create trigger ignore_effect before update on watcher_inbox begin select raise(ignore); end;",
        ] {
            let db = Db::mem();
            seed(&db);
            db.0.lock().unwrap().execute_batch(sql).unwrap();
            assert!(db
                .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                .is_err());
            assert_pending(&db);
            db.run("drop trigger ignore_effect", vec![]);
            assert!(db
                .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                .unwrap());
            assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
            assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 1);
            assert!(db.comment_acknowledged("o/r", "r1").unwrap());
            assert!(!db.has_pending_review("o/r", 1).unwrap());
        }
    }

    #[test]
    fn ignored_retry_cleanup_rolls_back_receipt_transitions() {
        for reconcile in [false, true] {
            let db = Db::mem();
            seed(&db);
            assert!(db
                .claim_review_attempt("o/r", 1, "t", HEAD, 0)
                .unwrap()
                .is_some());
            db.0.lock()
                .unwrap()
                .execute_batch(
                    "create trigger ignore_retry before delete on watcher_retry
                     begin select raise(ignore); end;",
                )
                .unwrap();
            if reconcile {
                assert!(db
                    .reconcile_review_receipts("o/r", 1, "t", HEAD, &[])
                    .is_err());
            } else {
                assert!(db
                    .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                    .is_err());
            }
            assert_pending(&db);
            assert_eq!(db.n("select count(*) from watcher_retry", vec![]), 1);
            db.run("drop trigger ignore_retry", vec![]);
            if reconcile {
                assert_eq!(
                    db.reconcile_review_receipts("o/r", 1, "t", HEAD, &[])
                        .unwrap(),
                    1
                );
                assert!(!db.comment_acknowledged("o/r", "r1").unwrap());
            } else {
                assert!(db
                    .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
                    .unwrap());
                assert!(db.comment_acknowledged("o/r", "r1").unwrap());
            }
            assert!(!db.has_pending_review("o/r", 1).unwrap());
            assert_eq!(db.n("select count(*) from watcher_retry", vec![]), 0);
        }
    }

    #[test]
    fn ignored_retirement_blocks_new_head_receipt() {
        let db = Db::mem();
        seed(&db);
        let next = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        db.run(
            "update prs set upd='t2',d=? where repo='o/r' and num=1",
            crate::pa![json!({"watcher_head":next}).to_string()],
        );
        db.0.lock()
            .unwrap()
            .execute_batch(
                "create trigger ignore_retire before update on watcher_inbox
                 when new.state='superseded' begin select raise(ignore); end;",
            )
            .unwrap();
        let ids = vec!["r2".into()];
        assert!(db.receive_review_batch("o/r", 1, "t2", next, &ids).is_err());
        assert_pending(&db);
        assert_eq!(db.n("select count(*) from watcher_inbox", vec![]), 1);
        db.run("drop trigger ignore_retire", vec![]);
        db.receive_review_batch("o/r", 1, "t2", next, &ids).unwrap();
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='superseded'",
                vec![]
            ),
            1
        );
        assert_pending(&db);
        assert!(!db.comment_acknowledged("o/r", "r1").unwrap());
    }

    #[test]
    fn ignored_absence_retirement_rolls_back_other_retirements() {
        let db = Db::mem();
        seed(&db);
        db.receive_review_batch("o/r", 1, "t", HEAD, &["r2".into()])
            .unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                "create trigger ignore_retire before update on watcher_inbox
                 when new.state='superseded' and old.comment_id='r2'
                 begin select raise(ignore); end;",
            )
            .unwrap();
        assert!(db
            .reconcile_review_receipts("o/r", 1, "t", HEAD, &[])
            .is_err());
        assert_eq!(
            db.n(
                "select count(*) from watcher_inbox where state='received'",
                vec![]
            ),
            2
        );
        db.run("drop trigger ignore_retire", vec![]);
        assert_eq!(
            db.reconcile_review_receipts("o/r", 1, "t", HEAD, &[])
                .unwrap(),
            2
        );
        assert!(!db.has_pending_review("o/r", 1).unwrap());
        assert_eq!(db.n("select count(*) from cm", vec![]), 0);
        assert_eq!(
            db.reconcile_review_receipts("o/r", 1, "t", HEAD, &[])
                .unwrap(),
            0
        );
    }

    #[test]
    fn ignored_receipt_insert_rolls_back_entire_batch() {
        let db = Db::mem();
        seed(&db);
        db.0.lock()
            .unwrap()
            .execute_batch(
                "create trigger ignore_receipt before insert on watcher_inbox
                 when new.comment_id='r3' begin select raise(ignore); end;",
            )
            .unwrap();
        let ids = vec!["r2".into(), "r3".into()];
        assert!(db.receive_review_batch("o/r", 1, "t", HEAD, &ids).is_err());
        assert_pending(&db);
        assert_eq!(db.n("select count(*) from watcher_inbox", vec![]), 1);
        db.run("drop trigger ignore_receipt", vec![]);
        db.receive_review_batch("o/r", 1, "t", HEAD, &ids).unwrap();
        db.receive_review_batch("o/r", 1, "t", HEAD, &ids).unwrap();
        assert_eq!(db.n("select count(*) from watcher_inbox", vec![]), 3);
    }

    #[test]
    fn repository_lifecycle_revokes_attempt_without_resetting_retry() {
        for change in [
            "update rp set on_=0 where full='o/r'; update rp set on_=1 where full='o/r';",
            "delete from rp where full='o/r'; insert into rp(full) values('o/r');",
            "update rp set full='o/other' where full='o/r'; update rp set full='o/r' where full='o/other';",
        ] {
            let db = Db::mem();
            seed(&db);
            let ids = vec!["r1".into()];
            let old = db
                .claim_review_attempt("o/r", 1, "t", HEAD, 0)
                .unwrap()
                .unwrap();
            db.0.lock().unwrap().execute_batch(change).unwrap();
            let rows = db.try_all("select * from watcher_retry", vec![]).unwrap();
            assert!(rows[0]["analysis_ticket"].is_null());
            assert_eq!(rows[0]["next_attempt_at"], 600000);
            assert!(db
                .queue_review_with_attempt("o/r", 1, "t", &ids, &request(), &old)
                .is_err());
            assert_pending(&db);
            assert!(db
                .claim_review_attempt("o/r", 1, "t", HEAD, 599999)
                .unwrap()
                .is_none());
            let current = db
                .claim_review_attempt("o/r", 1, "t", HEAD, 600000)
                .unwrap()
                .unwrap();
            old.release().unwrap();
            db.run("update rp set on_=on_ where full='o/r'", vec![]);
            assert!(db
                .queue_review_with_attempt("o/r", 1, "t", &ids, &request(), &current)
                .unwrap());
            current.release().unwrap();
        }
    }

    #[test]
    fn ignored_repository_revocation_aborts_state_change() {
        let db = Db::mem();
        seed(&db);
        let attempt = db
            .claim_review_attempt("o/r", 1, "t", HEAD, 0)
            .unwrap()
            .unwrap();
        let c = db.0.lock().unwrap();
        c.execute_batch(
            "create trigger ignore_revoke before update on watcher_retry
             when new.analysis_ticket is null begin select raise(ignore); end;",
        )
        .unwrap();
        assert!(c
            .execute("update rp set on_=0 where full='o/r'", [])
            .is_err());
        let on: i64 = c
            .query_row("select on_ from rp where full='o/r'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(on, 1);
        c.execute_batch("drop trigger ignore_revoke").unwrap();
        drop(c);
        attempt.release().unwrap();
    }

    #[test]
    fn reopened_pr_rejects_old_analysis_even_with_same_snapshot() {
        for queue in [false, true] {
            let db = Db::mem();
            seed(&db);
            let ids = vec!["r1".into()];
            let old = db
                .claim_review_attempt("o/r", 1, "t", HEAD, 0)
                .unwrap()
                .unwrap();
            db.run("update prs set st='closed' where num=1", vec![]);
            db.run("update prs set st='open' where num=1", vec![]);
            db.receive_review_batch("o/r", 1, "t", HEAD, &ids).unwrap();
            let current = db
                .claim_review_attempt("o/r", 1, "t", HEAD, 0)
                .unwrap()
                .unwrap();
            let req = request();
            assert!(db
                .queue_review_with_attempt("o/r", 1, "t", &ids, &req, &old)
                .is_err());
            assert!(db
                .ack_review_with_attempt("o/r", 1, "t", HEAD, (&ids, Some(&old)), &req["reply"],)
                .is_err());
            assert_pending(&db);
            assert!(db
                .ack_review_with_attempt("o/r", 1, "t", HEAD, (&ids, None), &Value::Null)
                .is_err());
            assert!(db
                .ack_review_with_attempt("o/r", 1, "t", HEAD, (&[], None), &Value::Null)
                .is_err());
            old.release().unwrap();
            if queue {
                assert!(db
                    .queue_review_with_attempt("o/r", 1, "t", &ids, &req, &current)
                    .unwrap());
            } else {
                db.ack_review_with_attempt(
                    "o/r",
                    1,
                    "t",
                    HEAD,
                    (&ids, Some(&current)),
                    &req["reply"],
                )
                .unwrap();
            }
            assert!(db.comment_acknowledged("o/r", "r1").unwrap());
            assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 1);
            assert_eq!(db.n("select count(*) from jobs", vec![]), i64::from(queue));
            current.release().unwrap();
        }
    }

    #[test]
    fn ignored_review_ack_rolls_back_job_reply_and_receipt() {
        let db = Db::mem();
        seed(&db);
        db.0.lock()
            .unwrap()
            .execute_batch(
                "create trigger ignore_ack before insert on cm begin select raise(ignore); end;",
            )
            .unwrap();
        assert!(db
            .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
            .is_err());
        assert_pending(&db);
        db.run("drop trigger ignore_ack", vec![]);
        assert!(db
            .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
            .unwrap());
        assert!(db.comment_acknowledged("o/r", "r1").unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
        assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 1);
    }

    #[test]
    fn failure_after_job_insert_rolls_back_job_reply_and_ack() {
        let db = Db::mem();
        seed(&db);
        db.0.lock().unwrap().execute_batch(
            "create trigger fail_ack before insert on cm begin select raise(abort,'injected ack failure'); end;"
        ).unwrap();
        assert!(db
            .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
            .is_err());
        assert_pending(&db);
        db.0.lock()
            .unwrap()
            .execute_batch("drop trigger fail_ack")
            .unwrap();
        assert!(db
            .queue_review_and_ack("o/r", 1, "t", &["r1".into()], &request())
            .unwrap());
        assert_eq!(db.n("select count(*) from watcher_outbox", vec![]), 1);
        assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
    }
}
