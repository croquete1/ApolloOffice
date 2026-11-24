use rusqlite::{params_from_iter, types::ValueRef, Connection, ToSql};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>);

const SCH: &str = "
create table if not exists kv(k text primary key, v text);
create table if not exists ses(id text primary key, exp int);
create table if not exists lnk(c text primary key, exp int);
create table if not exists rp(id integer primary key, full text unique, iss int default 1, hunt int default 0, ci text default '', setup text default '', rules text default '', mode text default 'ask', merge int default 1, maxo int default 3, maxd int default 5, ai int default 1, on_ int default 1, t0 int);
create table if not exists hs(id text primary key, ssh text default '', dk int default 0, ln int default 1, ci int default 1, gb int default 50, raw int default 0, on_ int default 1, t0 int);
create table if not exists jobs(id integer primary key, repo text, num int, kind text default 'fix', title text, url text, st text, why text default '', d text default '{}', pr int, prio real default 0, t0 int, t1 int);
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
create table if not exists cost(id integer primary key, t int, day text, job int, repo text default '', w text, md text, sub int default 0, tin int default 0, tcw int default 0, tcr int default 0, tout int default 0, usd real);
create index if not exists cost_day on cost(day);
create index if not exists cost_job on cost(job);
create table if not exists fix_attempts(id integer primary key, job int not null, attempt int not null, model text not null, started int not null, finished int, elapsed_ms int, outcome text not null default 'running', gate text not null default '');
create index if not exists fix_attempts_started on fix_attempts(started);
create index if not exists fix_attempts_job on fix_attempts(job, attempt);
";

const SCHEMA_VERSION: i64 = 1;

// Version 0 includes legacy installations with any subset of these columns.
// Inspect the schema rather than swallowing every ALTER TABLE error.
const LEGACY_COLUMNS: &[(&str, &str, &str)] = &[
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
        "auto_recover_drops",
        "alter table rp add column auto_recover_drops int default 0",
    ),
    (
        "seen",
        "why",
        "alter table seen add column why text default ''",
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
        let tx = c.transaction()?;
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

    fn ack_comments_in_tx(
        tx: &rusqlite::Transaction<'_>,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
    ) -> rusqlite::Result<()> {
        for id in ids {
            tx.execute(
                "insert or ignore into cm(id,t) values(?,?)",
                rusqlite::params![format!("{repo}:{id}"), now()],
            )?;
        }
        let changed = tx.execute("update prs set d=json_set(d, '$.seen', json('true'), '$.comment_seen_at', ?) where repo=? and num=?", rusqlite::params![updated, repo, number])?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }

    /// Atomically acknowledge a successfully processed comment batch and its PR cursor.
    pub fn ack_pr_comments(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
    ) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        Self::ack_comments_in_tx(&tx, repo, number, updated, ids)?;
        tx.commit()?;
        crate::app::chg();
        Ok(())
    }

    /// Queue a review and acknowledge its source comments in the same transaction.
    /// If another review is active, do not acknowledge the batch: retry it later.
    pub fn queue_review_and_ack(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
        request: &Value,
    ) -> rusqlite::Result<bool> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let active: i64 = tx.query_row(
            "select count(*) from jobs where repo=? and pr=? and kind='review' and st not in ('done','drop')",
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
        let data = request["data"]
            .as_object()
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let t = now();
        tx.execute("insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1) values(?,?,'review',?,?,'ready',?,?,?,-1.0,?,?)",
            rusqlite::params![repo, number, title, url, change, json!(data).to_string(), number, t, t])?;
        Self::ack_comments_in_tx(&tx, repo, number, updated, ids)?;
        tx.commit()?;
        crate::app::chg();
        Ok(true)
    }

    /// Atomic follow-up admission and acknowledgement. A replay cannot create a
    /// second follow-up, and an already active follow-up leaves new input pending.
    pub fn queue_followup_and_ack(
        &self,
        repo: &str,
        number: i64,
        updated: &str,
        ids: &[String],
        request: &Value,
    ) -> rusqlite::Result<bool> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let merged: bool = tx.query_row(
            "select exists(select 1 from prs where repo=? and num=? and st='merged')",
            rusqlite::params![repo, number],
            |r| r.get(0),
        )?;
        if !merged || ids.is_empty() || ids.iter().any(|id| id.is_empty()) {
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
            Self::ack_comments_in_tx(&tx, repo, number, updated, ids)?;
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
        let data = json!({"title":title,"body":body,"after":number,"source_comments":ids});
        tx.execute("insert into jobs(repo,num,kind,title,url,st,why,d,prio,t0,t1) values(?,0,'fix',?,?,'vet',?,?,-1.0,?,?)",
            rusqlite::params![repo,title,url,why,data.to_string(),t,t])?;
        Self::ack_comments_in_tx(&tx, repo, number, updated, ids)?;
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
        let ids = vec!["review-1".to_string()];
        assert!(db.ack_pr_comments("a/b", 1, "new", &ids).is_err());
        assert_eq!(db.n("select count(*) from cm", vec![]), 0);
        db.run(
            "insert into prs(repo,num,d) values(?,?,?)",
            crate::pa![
                "a/b".to_string(),
                1_i64,
                r#"{"auto_merge":true}"#.to_string()
            ],
        );
        db.ack_pr_comments("a/b", 1, "new", &ids).unwrap();
        db.ack_pr_comments("a/b", 1, "new", &ids).unwrap();
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
        let ids = vec!["review-1".to_string()];
        let request = json!({"title":"Fix","url":"https://github.com/a/b/pull/1","change":"Correct bug","data":{}});
        assert!(db
            .queue_review_and_ack("a/b", 1, "new", &ids, &request)
            .is_err());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
        db.run(
            "insert into prs(repo,num,d) values(?,?,?)",
            crate::pa!["a/b".to_string(), 1_i64, "{}".to_string()],
        );
        assert!(db
            .queue_review_and_ack("a/b", 1, "new", &ids, &request)
            .unwrap());
        assert!(db.comment_acknowledged("a/b", "review-1").unwrap());
        let next = vec!["review-2".to_string()];
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
    fn failed_snapshot_rolls_back_rows_closures_and_scan_metadata() {
        let db = Db::mem();
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
        let values: (i64, i64, i64) = c
            .query_row(
                "select auto_merge,steward_repair,local_merge from rp where full='o/r'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(values, (1, 0, 0));
        let version: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
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
        db.run(
            "insert into prs(repo,num,st,d) values('o/r',1,'merged','{}')",
            vec![],
        );
        let request = json!({"title":"follow-up","body":"verified request","url":"https://example.test","why":"maintainer"});
        let first = vec!["i1".into()];
        let second = vec!["i2".into()];
        assert!(db
            .queue_followup_and_ack("o/r", 1, "t1", &first, &request)
            .unwrap());
        assert!(db
            .queue_followup_and_ack("o/r", 1, "t1", &first, &request)
            .unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
        assert!(!db
            .queue_followup_and_ack("o/r", 1, "t2", &second, &request)
            .unwrap());
        assert!(!db.comment_acknowledged("o/r", "i2").unwrap());
        db.run("update jobs set st='done'", vec![]);
        assert!(db
            .queue_followup_and_ack("o/r", 1, "t2", &second, &request)
            .unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 2);
        assert!(db.comment_acknowledged("o/r", "i2").unwrap());
        assert!(db
            .queue_followup_and_ack("o/r", 2, "t3", &["i3".into()], &request)
            .is_err());
        assert!(!db.comment_acknowledged("o/r", "i3").unwrap());
    }
}
