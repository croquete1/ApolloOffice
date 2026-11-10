use rusqlite::{params_from_iter, types::ValueRef, Connection, ToSql};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>);

const SCH: &str = "
pragma journal_mode=wal;
pragma busy_timeout=5000;
create table if not exists kv(k text primary key, v text);
create table if not exists ses(id text primary key, exp int);
create table if not exists lnk(c text primary key, exp int);
create table if not exists rp(id integer primary key, full text unique, iss int default 1, hunt int default 0, ci text default '', setup text default '', rules text default '', mode text default 'ask', maxo int default 3, maxd int default 5, ai int default 1, on_ int default 1, t0 int);
create table if not exists hs(id text primary key, ssh text default '', dk int default 0, ln int default 1, ci int default 1, gb int default 50, raw int default 0, on_ int default 1, t0 int);
create table if not exists jobs(id integer primary key, repo text, num int, kind text default 'fix', title text, url text, st text, why text default '', d text default '{}', pr int, prio real default 0, t0 int, t1 int);
create index if not exists jobs_st on jobs(st);
create table if not exists ev(id integer primary key, t int, w text, m text, j int);
create table if not exists prs(repo text, num int, st text, title text, url text, upd text, merged text, d text default '{}', t0 int, primary key(repo, num));
create table if not exists tok(day text, w text, tin int default 0, tout int default 0, primary key(day, w));
create table if not exists seen(repo text, num int, upd text, primary key(repo, num));
create table if not exists cm(id text primary key, t int);
create table if not exists cost(id integer primary key, t int, day text, job int, repo text default '', w text, md text, sub int default 0, tin int default 0, tcw int default 0, tcr int default 0, tout int default 0, usd real);
create index if not exists cost_day on cost(day);
cre...[truncated]