use crate::app::{Ax, WS};
use crate::db::now;
use crate::pa;
use axum::body::{to_bytes, Body};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, State};
use axum::http::{HeaderMap, Method, Request, Response, StatusCode};
use axum::response::IntoResponse;
use rust_embed::RustEmbed;
use serde_json::{json, Value};

#[derive(RustEmbed)]
#[folder = "ui/"]
struct Ui;

// Keep the authenticated session so the owner can configure the paused office.
fn wipe_office_data(db: &crate::db::Db) -> rusqlite::Result<()> {
    let mut c = db.0.lock().unwrap();
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "delete from job_dependencies;
         delete from job_schedule;
         delete from watcher_inbox;
         delete from watcher_retry;
         delete from watcher_outbox;
         delete from github_issues;
         delete from jobs;
         delete from ev;
         delete from prs;
         delete from cost;
         delete from fix_attempts;
         delete from coderabbit_requests;
         delete from coderabbit_findings;
         delete from coderabbit_cycles;
         delete from tok;
         delete from seen;
         delete from cm;
         delete from rp;
         delete from hs;
         delete from lnk;
         delete from kv;
         insert into kv(k,v) values('man','true');",
    )?;
    tx.commit()?;
    crate::app::chg();
    Ok(())
}

// Preserve prerequisite evidence required by jobs that may still execute.
fn clear_job_history(db: &crate::db::Db) -> rusqlite::Result<()> {
    let mut c = db.0.lock().unwrap();
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "delete from jobs where st in ('done','drop') and not exists(
           select 1 from job_dependencies dep join jobs dependent on dependent.id=dep.job
           where dep.prerequisite=jobs.id and dependent.st not in ('done','drop'));
         delete from job_schedule where job not in (select id from jobs);
         delete from job_dependencies where job not in (select id from jobs);
         delete from fix_attempts where job not in (select id from jobs);
         delete from kv where k glob 'lesson_exposure:*' and not exists(
           select 1 from fix_attempts f where kv.k=printf('lesson_exposure:%lld:%lld:%lld',f.id,f.job,f.started));
         delete from ev where j is null or j not in (select id from jobs);",
    )?;
    tx.commit()?;
    crate::app::chg();
    Ok(())
}

fn remove_repository(db: &crate::db::Db, repo: &str) -> rusqlite::Result<()> {
    let mut c = db.0.lock().unwrap();
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("delete from rp where full=?", [repo])?;
    tx.execute(
        "update jobs set st='drop', why='repository removed' where repo=? and st in ('vet','ready','approve','park')",
        [repo],
    )?;
    tx.commit()?;
    crate::app::chg();
    Ok(())
}

fn save_repo_settings(db: &crate::db::Db, repo: &str, b: &Value) -> rusqlite::Result<()> {
    let mut c = db.0.lock().unwrap();
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let exists: bool = tx.query_row(
        "select exists(select 1 from rp where full=?)",
        [repo],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    for (k, kind) in [
        ("iss", 0),
        ("hunt", 0),
        ("feat", 0),
        ("nm", 0),
        ("on_", 0),
        ("merge", 0),
        ("auto_merge", 0),
        ("local_merge", 0),
        ("steward_repair", 0),
        ("steward_close", 0),
        ("coderabbit_free_confirmed", 0),
        ("coderabbit_review", 0),
        ("coderabbit_repair", 0),
        ("auto_recover_drops", 0),
        ("maxo", 1),
        ("maxd", 1),
        ("bud", 3),
        ("ci", 2),
        ("setup", 2),
        ("rules", 2),
        ("mode", 2),
        ("lang", 2),
        ("skl", 2),
    ] {
        let v = &b[k];
        if v.is_null() {
            continue;
        }
        match kind {
            3 => {
                let x = v.as_f64().unwrap_or(0.0).clamp(0.0, 100_000.0);
                tx.execute(
                    &format!("update rp set {k}=? where full=?"),
                    rusqlite::params![x, repo],
                )?;
            }
            0 => {
                let x = v
                    .as_bool()
                    .map(|x| x as i64)
                    .or(v.as_i64())
                    .unwrap_or(0)
                    .clamp(0, 1);
                if k == "on_" {
                    tx.execute(
                        "update rp set coderabbit_epoch=coderabbit_epoch + case when on_<>?1 then 1 else 0 end,coderabbit_review_since=case when on_<>?1 and ?1=1 and coderabbit_review=1 and coderabbit_free_confirmed=1 then ?2 else coderabbit_review_since end,on_=?1 where full=?3",
                        rusqlite::params![x, now(), repo],
                    )?;
                } else if k == "coderabbit_review" {
                    let at = now();
                    tx.execute(
                        "update rp set coderabbit_epoch=coderabbit_epoch + case when coderabbit_review<>? then 1 else 0 end,coderabbit_review_since=case when coderabbit_review<>? and ?=1 and on_=1 and coderabbit_free_confirmed=1 then ? else coderabbit_review_since end,coderabbit_review=? where full=?",
                        rusqlite::params![x, x, x, at, x, repo],
                    )?;
                } else if k == "coderabbit_free_confirmed" {
                    let at = now();
                    tx.execute(
                        "update rp set coderabbit_epoch=coderabbit_epoch + case when coderabbit_free_confirmed<>? then 1 else 0 end,coderabbit_review_since=case when coderabbit_free_confirmed<>? and ?=1 and on_=1 and coderabbit_review=1 then ? else coderabbit_review_since end,coderabbit_free_confirmed=? where full=?",
                        rusqlite::params![x, x, x, at, x, repo],
                    )?;
                } else if k == "coderabbit_repair" {
                    tx.execute(
                        "update rp set coderabbit_epoch=coderabbit_epoch + case when coderabbit_repair<>? then 1 else 0 end,coderabbit_repair=? where full=?",
                        rusqlite::params![x, x, repo],
                    )?;
                } else {
                    tx.execute(
                        &format!("update rp set {k}=? where full=?"),
                        rusqlite::params![x, repo],
                    )?;
                }
            }
            1 => {
                let x = v.as_i64().unwrap_or(3).clamp(0, 10000);
                tx.execute(
                    &format!("update rp set {k}=? where full=?"),
                    rusqlite::params![x, repo],
                )?;
            }
            _ => {
                let mut s = v.as_str().unwrap_or("").trim().to_string();
                if k == "mode" && s != "auto" {
                    s = "ask".into();
                }
                if k == "lang" {
                    s = pr_lang(&s).into();
                }
                s = crate::sh::head(&s, 4000);
                tx.execute(
                    &format!("update rp set {k}=? where full=?"),
                    rusqlite::params![s, repo],
                )?;
            }
        }
    }
    tx.execute(
        "delete from kv where k=? or k like ?",
        rusqlite::params![format!("cired:{repo}"), format!("cired:{repo}@%")],
    )?;
    tx.commit()?;
    crate::app::chg();
    Ok(())
}

fn work_class(job: &Value) -> &'static str {
    match job["kind"].as_str().unwrap_or("") {
        "hunt" => "discovered_bug",
        "mergecheck" => "pr_recheck",
        "review" => "pr_review",
        "rebase" => "branch_update",
        "fix" if job["d"]["opt"] == true => "optimization",
        "fix" if job["d"]["feat"] == true => "feature",
        "fix" if job["d"]["ext"]["src"].is_string() => "reported_bug",
        "fix" if job["num"].as_i64().unwrap_or(0) == 0 => "discovered_bug",
        "fix" => "issue_bug",
        _ => "other",
    }
}

fn rs(c: StatusCode, ct: &str, b: impl Into<Body>) -> Response<Body> {
    Response::builder()
        .status(c)
        .header("content-type", ct)
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff")
        .header("x-frame-options", "DENY")
        .header("referrer-policy", "same-origin")
        .header("content-security-policy", "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self' ws://127.0.0.1:* ws://localhost:*; frame-ancestors 'none'; form-action 'self'; base-uri 'none'")
        .body(b.into())
        .unwrap()
}

fn js(v: Value) -> Response<Body> {
    rs(StatusCode::OK, "application/json", v.to_string())
}

fn er(c: StatusCode, m: &str) -> Response<Body> {
    rs(c, "application/json", json!({"error": m}).to_string())
}

// Parse OAuth query values as application/x-www-form-urlencoded, including
// percent-escapes and plus signs, without accepting a similarly named key.
fn query_value(query: &str, name: &str) -> String {
    reqwest::Url::parse(&format!("http://localhost/?{query}"))
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        })
        .unwrap_or_default()
}

fn ck(h: &HeaderMap, n: &str) -> Option<String> {
    h.get_all("cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .map(|s| s.trim())
        .find_map(|s| s.strip_prefix(&format!("{n}=")).map(String::from))
}

fn host_ok(a: &Ax, h: &HeaderMap) -> bool {
    let hs = h.get("host").and_then(|v| v.to_str().ok()).unwrap_or("");
    hs == format!("127.0.0.1:{}", a.port)
        || hs == format!("localhost:{}", a.port)
        || hs == format!("[::1]:{}", a.port)
}

fn origin_ok(a: &Ax, h: &HeaderMap) -> bool {
    match h.get("origin").and_then(|v| v.to_str().ok()) {
        None => h
            .get("sec-fetch-site")
            .and_then(|v| v.to_str().ok())
            .map(|s| s == "same-origin" || s == "none")
            .unwrap_or(true),
        Some(o) => {
            o == format!("http://127.0.0.1:{}", a.port)
                || o == format!("http://localhost:{}", a.port)
                || o == format!("http://[::1]:{}", a.port)
        }
    }
}

fn authed(a: &Ax, h: &HeaderMap) -> bool {
    ck(h, "ao")
        .map(|s| {
            a.db.n(
                "select count(*) from ses where id=? and exp>?",
                pa![s, now()],
            ) > 0
        })
        .unwrap_or(false)
}

pub fn link_new(a: &crate::db::Db) -> String {
    let c = hex::encode(rand::random::<[u8; 16]>());
    a.run("delete from lnk where exp<?", pa![now()]);
    a.run(
        "insert into lnk(c,exp) values(?,?)",
        pa![c.clone(), now() + 600_000],
    );
    c
}

const LP: &str = "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>ApolloOffice</title><style>body{margin:0;min-height:100vh;display:grid;place-items:center;background:#0b0c0e;color:#ecebe6;font:14px/1.5 ui-monospace,Menlo,Consolas,monospace}div{max-width:420px;padding:24px}button{font:inherit;color:#0b0c0e;background:#ecebe6;border:0;border-radius:6px;padding:8px 16px;cursor:pointer}code{color:#4fb3e0}</style><div>";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(b: &str) -> Response<Body> {
    rs(
        StatusCode::OK,
        "text/html; charset=utf-8",
        format!("{LP}{b}</div>"),
    )
}

const ISSUE_STATE_CTE: &str = r#"
with issue_state as (
    select i.repo,i.num,i.title,i.url,i.author,i.updated,i.labels,
        case
            when j.st='drop' then 'blocked'
            when j.st='done' and p.st='open' then 'pr_open'
            when j.st='done' and p.st='merged' then 'integrated'
            when j.st='done' then 'done'
            when j.id is not null then 'queued'
            when coalesce(i.skip_reason,'')<>'' then 'filtered'
            when s.upd=i.updated then 'reviewed'
            else 'waiting'
        end status,
        case
            when j.id is not null then coalesce(j.why,'')
            when coalesce(i.skip_reason,'')<>'' then i.skip_reason
            when s.upd=i.updated then coalesce(nullif(s.why,''),'Scout já avaliou esta versão; uma nova atualização volta a ser triada.')
            else ''
        end reason,
        coalesce(j.st,'') job_stage,
        coalesce(j.id,0) job_id,
        coalesce(p.url,'') pr_url,
        i.seen_at
    from github_issues i
    left join jobs j on j.id=(select id from jobs where repo=i.repo and num=i.num and kind='fix' order by id desc limit 1)
    left join prs p on p.repo=j.repo and p.num=j.pr
    left join seen s on s.repo=i.repo and s.num=i.num
    where i.is_open=1 and exists(select 1 from rp where full=i.repo and on_=1 and iss=1)
)
"#;

fn issue_backlog(a: &Ax) -> (Vec<Value>, Vec<Value>) {
    let counts = a.db.all(
        &format!("{ISSUE_STATE_CTE} select repo,status,count(*) n from issue_state group by repo,status order by repo,status"),
        pa![],
    );
    let repos = a.db.all(
        "select full,ci from rp where on_=1 and iss=1 order by full",
        pa![],
    );
    let mut summary = Vec::with_capacity(repos.len());
    for row in repos {
        let repo = row["full"].as_str().unwrap_or("");
        let mut item = json!({
            "repo": repo,
            "intake_ready": !row["ci"].as_str().unwrap_or("").trim().is_empty(),
            "open": 0,
            "waiting": 0,
            "reviewed": 0,
            "filtered": 0,
            "queued": 0,
            "blocked": 0,
            "pr_open": 0,
            "integrated": 0,
            "done": 0,
            "scan": a.db.kg(&format!("iscan:{repo}")),
        });
        for count in counts.iter().filter(|x| x["repo"].as_str() == Some(repo)) {
            let status = count["status"].as_str().unwrap_or("");
            let n = count["n"].as_i64().unwrap_or(0);
            item["open"] = json!(item["open"].as_i64().unwrap_or(0) + n);
            if item.get(status).is_some() {
                item[status] = json!(n);
            }
        }
        summary.push(item);
    }
    let rows = a.db.all(
        &format!("{ISSUE_STATE_CTE} select repo,num,title,url,author,updated,labels,status,reason,job_stage,job_id,pr_url,seen_at from issue_state order by case status when 'waiting' then 0 when 'blocked' then 1 when 'queued' then 2 when 'pr_open' then 3 when 'reviewed' then 4 when 'filtered' then 5 else 6 end, updated asc limit 80"),
        pa![],
    );
    (rows, summary)
}

fn pr_steward_state(a: &Ax) -> Value {
    let repos =
        a.db.all("select full from rp where on_=1 order by full", pa![]);
    let mut summaries = Vec::with_capacity(repos.len());
    let mut prs = Vec::new();
    let mut total = 0_i64;
    let mut review_requested = 0_i64;
    let mut ci_failed = 0_i64;
    let mut draft = 0_i64;
    let mut overlap = 0_i64;
    let mut blocked = 0_i64;
    let mut final_check = 0_i64;
    let mut states = crate::steward::state_counts(&[]);
    for row in repos {
        let repo = row["full"].as_str().unwrap_or("");
        let report = a.db.kg(&format!("steward:{repo}"));
        let summary = &report["summary"];
        let report_prs = report["prs"].as_array().cloned().unwrap_or_default();
        let report_total = summary["total"].as_i64().unwrap_or(report_prs.len() as i64);
        let report_blocked = summary["blocked"].as_i64().unwrap_or(report_total);
        let report_states = if summary["states"].is_object() {
            summary["states"].clone()
        } else {
            crate::steward::state_counts(&report_prs)
        };
        total += report_total;
        review_requested += summary["review_requested"].as_i64().unwrap_or(0);
        ci_failed += summary["ci_failed"].as_i64().unwrap_or(0);
        draft += summary["draft"].as_i64().unwrap_or(0);
        overlap += summary["overlap"].as_i64().unwrap_or(0);
        blocked += report_blocked;
        final_check += (report_total - report_blocked).max(0);
        for state in crate::steward::STATES {
            states[state] = json!(
                states[state].as_i64().unwrap_or(0) + report_states[state].as_i64().unwrap_or(0)
            );
        }
        summaries.push(json!({
            "repo": repo,
            "at": report["at"],
            "total": report_total,
            "review_requested": summary["review_requested"],
            "ci_failed": summary["ci_failed"],
            "draft": summary["draft"],
            "overlap": summary["overlap"],
            "blocked": report_blocked,
            "final_check": (report_total - report_blocked).max(0),
            "states": report_states,
            "error": a.db.kg(&format!("steward:error:{repo}")),
        }));
        for pr in report_prs {
            prs.push(json!({
                "repo": repo,
                "num": pr["number"],
                "title": pr["title"],
                "url": pr["url"],
                "state": pr["state"],
                "priority": pr["priority"],
                "next_action": pr["next_action"],
            }));
        }
    }
    prs.sort_by_key(|pr| pr["priority"].as_i64().unwrap_or(9));
    prs.truncate(40);
    json!({
        "total": total,
        "review_requested": review_requested,
        "ci_failed": ci_failed,
        "draft": draft,
        "overlap": overlap,
        "blocked": blocked,
        "final_check": final_check,
        "states": states,
        "repos": summaries,
        "prs": prs,
    })
}

const WORK_CLASS_SQL: &str = "case when kind='hunt' then 'discovered_bug' when kind='mergecheck' then 'pr_recheck' when kind='review' then 'pr_review' when kind='rebase' then 'branch_update' when kind='fix' and coalesce(json_extract(d,'$.opt'),0)=1 then 'optimization' when kind='fix' and coalesce(json_extract(d,'$.feat'),0)=1 then 'feature' when kind='fix' and json_type(d,'$.ext.src') is not null then 'reported_bug' when kind='fix' and num=0 then 'discovered_bug' when kind='fix' then 'issue_bug' else 'other' end";

fn state(a: &Ax) -> Value {
    let d = crate::db::day();
    let mut tk = json!({});
    for t in
        a.db.all("select w, tin+tout n from tok where day=?", pa![d.clone()])
    {
        tk[t["w"].as_str().unwrap_or("")] = t["n"].clone();
    }
    let (w, o, fl, ln, cil, hst) = {
        let l = a.lv.lock().unwrap();
        (
            json!(l.w),
            json!(l.o),
            json!(l
                .fl
                .iter()
                .filter(|f| now() - f.t < 8000)
                .collect::<Vec<_>>()),
            json!(l
                .ln
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<std::collections::HashMap<_, _>>()),
            json!(l
                .cil
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<std::collections::HashMap<_, _>>()),
            json!(l.hst),
        )
    };
    let mut cs = json!({});
    for t in a.db.all("select w, sum(case when sub=0 then usd end) a, sum(case when sub=1 then usd end) v from cost where day=? group by w", pa![d.clone()]) {
        cs[t["w"].as_str().unwrap_or("")] = json!([t["a"], t["v"]]);
    }
    let mut wv = w;
    for x in WS {
        wv[*x]["tk"] = tk[*x].clone();
        wv[*x]["cs"] = cs[*x].clone();
        if wv[*x]["md"].as_str().map(|s| s.is_empty()).unwrap_or(true)
            && *x != "ci"
            && *x != "submit"
        {
            wv[*x]["md"] = json!(a.md(x));
        }
    }
    // A queued submission is not running merely because its state is 'submit'.
    // Snapshot IDs before DB reads to avoid reversing the lock order.
    let admitted_ids: Vec<i64> = a.lk.lock().unwrap().iter().copied().collect();
    let active_submits: Vec<i64> = admitted_ids
        .into_iter()
        .filter(|id| a.db.jg(*id).is_some_and(|job| job.st == "submit"))
        .collect();
    for x in a.db.all("select st, count(*) n, max(id) j from jobs where st in ('hunt','repro','fix','ci','review','gate','submit') group by st", pa![]) {
        let w = match x["st"].as_str().unwrap_or("") {
            "hunt" => "hunter",
            "fix" => "fixer",
            "review" => "reviewer",
            "gate" => "submit",
            s => s,
        };
        let is_submit = x["st"] == "submit";
        let count = if is_submit {
            active_submits.len() as i64
        } else {
            x["n"].as_i64().unwrap_or(0)
        };
        if wv[w].is_object() && count > 0 {
            wv[w]["n"] = json!(wv[w]["n"].as_i64().unwrap_or(0) + count);
            if wv[w]["st"] != "work" {
                wv[w]["st"] = json!("work");
                wv[w]["job"] = if is_submit {
                    json!(active_submits.iter().max())
                } else {
                    x["j"].clone()
                };
            }
        }
    }
    // Distinguish an available agent from one that is deliberately paused,
    // waiting for admission or blocked by an environment failure.
    let queued = a.db.all(
        "select id, st from jobs where st in ('ready','submit')",
        pa![],
    );
    // An admitted job can remain in 'ready' or 'submit' during execution.
    let admitted = a.lk.lock().unwrap();
    let waiting = queued
        .iter()
        .filter(|job| !admitted.contains(&job["id"].as_i64().unwrap_or(0)))
        .count() as i64;
    std::mem::drop(admitted);
    let paused = o["man"] == true || o["pause"].as_str().is_some_and(|reason| !reason.is_empty());
    let blocked_hunts = crate::hunt_health::blocked_repos(a);
    // Diagnostics: a host may be ready or still running `ensure`; a repository
    // may have hunting or issue intake switched off. These were previously
    // invisible, leaving the office looking idle for no stated reason.
    let (ens_ids, ing_ids) = crate::pipe::ensure_state();
    let hosts_diag: Vec<Value> =
        a.db.all(
            "select id, dk, on_ from hs order by id='local' desc, id",
            pa![],
        )
        .into_iter()
        .map(|h| {
            let id = h["id"].as_str().unwrap_or("").to_string();
            let up = a.db.kg(&format!("up:{id}")) == json!(true) || ens_ids.contains(&id);
            json!({
                "id": id,
                "dk": h["dk"].as_i64().unwrap_or(0) == 1,
                "on": h["on_"].as_i64().unwrap_or(0) == 1,
                "up": up,
                "preparing": ing_ids.contains(&id),
            })
        })
        .collect();
    let hunt_off: Vec<Value> = a.db.all(
        "select full from rp where on_=1 and (hunt=0 or ci='')",
        pa![],
    );
    let iss_off: Vec<Value> = a.db.all("select full from rp where on_=1 and iss=0", pa![]);
    let hunt_enabled = a.db.n(
        "select count(*) from rp where on_=1 and hunt=1 and ci<>''",
        pa![],
    );
    let preparing_hosts: Vec<&str> = hosts_diag
        .iter()
        .filter(|h| h["preparing"] == true)
        .filter_map(|h| h["id"].as_str())
        .collect();
    for role in WS {
        let idle = wv[*role]["st"] != "work";
        let reason = if !idle {
            ""
        } else if wv[*role]["hold"] == true {
            "Agente pausado"
        } else if paused {
            "Escritório em pausa"
        } else if !preparing_hosts.is_empty() {
            "A preparar o ambiente de build; a aguardar que a máquina fique pronta"
        } else if *role == "hunter" && hunt_enabled == 0 {
            "Caça desligada: nenhum repositório com caça ativa"
        } else if wv[*role]["st"] == "sleep" {
            "Em espera; consultar o último evento do agente"
        } else if *role == "hunter" && !blocked_hunts.is_empty() {
            "Existem pesquisas suspensas por falhas de preparação"
        } else if waiting > 0 {
            "Sem tarefa atribuída; existem trabalhos na fila"
        } else {
            "Sem tarefa atribuída"
        };
        wv[*role]["idle_reason"] = json!(reason);
    }
    let lm = a.db.kg("lm");
    let q = a.db.all(&format!("select id,repo,num,kind,title,url,st,why,pr,t0,t1,{WORK_CLASS_SQL} work_class,json_extract(d,'$.h') h,json_extract(d,'$.staged_red.stage') red_stage,json_extract(d,'$.staged_red.test_commit') red_commit,json_extract(d,'$.staged_green.commit') green_commit from jobs where st not in ('done','drop') order by st in ('hunt','repro','fix','ci','review','gate','submit') desc, st='approve' desc, st='vet', id limit 60"), pa![]);
    let dn = a.db.all(&format!("select id,repo,num,kind,title,url,st,why,pr,t1,{WORK_CLASS_SQL} work_class from jobs where st in ('done','drop') order by t1 desc limit 25"), pa![]);
    let (issue_backlog, issue_summary) = issue_backlog(a);
    let pr_steward = pr_steward_state(a);
    let mut pr_counts = json!({"open": 0, "merged": 0, "closed": 0});
    for row in a.db.all("select st,count(*) n from prs group by st", pa![]) {
        let status = row["st"].as_str().unwrap_or("");
        if pr_counts.get(status).is_some() {
            pr_counts[status] = row["n"].clone();
        }
    }
    json!({
        "t": now(), "v": crate::app::V,
        "codex_usage": a.codex_usage.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        "me": a.db.kg("name"),
        "o": o, "w": wv, "fl": fl, "ln": ln, "cil": cil, "hst": hst,
        "delivery": {
            "published_fixes": a.db.n("select count(distinct p.repo || '#' || p.num) from jobs j join prs p on p.repo=j.repo and p.num=j.pr where j.kind='fix' and j.st='done' and j.pr>0", pa![]),
            "open_fixes": a.db.n("select count(distinct p.repo || '#' || p.num) from jobs j join prs p on p.repo=j.repo and p.num=j.pr where j.kind='fix' and j.st='done' and p.st='open'", pa![]),
            "merged_fixes": a.db.n("select count(distinct p.repo || '#' || p.num) from jobs j join prs p on p.repo=j.repo and p.num=j.pr where j.kind='fix' and j.st='done' and p.st='merged' and coalesce(json_extract(p.d,'$.default_branch'),'')<>'' and json_extract(p.d,'$.base')=json_extract(p.d,'$.default_branch')", pa![])
        },
        "ops": {"waiting": waiting, "blocked_hunts": blocked_hunts, "paused": paused, "resources": crate::resources::status(a), "watcher_outbox": a.db.watcher_outbox_counts().ok()},
        "diag": {"hosts": hosts_diag, "hunt_off": hunt_off, "iss_off": iss_off, "hunt_enabled": hunt_enabled},
        "hs": a.db.all("select id, ln, dk, on_ from hs order by id='local' desc, id", pa![]),
        "rp": a.db.all("select full repo,on_ from rp order by full", pa![]),
        "q": q,
        "dn": dn,
        "issue_backlog": issue_backlog,
        "issue_summary": issue_summary,
        "pr": a.db.all("select repo,num,st,title,url,upd,merged from prs order by t0 desc limit 40", pa![]),
        "pr_counts": pr_counts,
        "pr_steward": pr_steward,
        "mg": a.db.all("select substr(merged,1,10) d, count(*) n from prs where merged is not null and merged >= date('now','-30 day') group by 1 order by 1", pa![]),
        "ev": a.db.all("select t,w,m,j from ev order by id desc limit 60", pa![]),
        "n": Value::Object(a.db.all("select st, count(*) n from jobs group by st", pa![]).into_iter().map(|x| (x["st"].as_str().unwrap_or("").to_string(), x["n"].clone())).collect()),
        "byRepo": a.db.all("select repo,st,count(*) n from jobs group by repo,st order by repo,st", pa![]),
        "done24": a.db.n("select count(*) from jobs where st='done' and t1>=?", pa![now() - 86_400_000]),
        "done24ByRepo": a.db.all("select repo,count(*) n from jobs where st='done' and t1>=? group by repo", pa![now() - 86_400_000]),
        "prod24": a.db.all("select cast((?1 - t1)/3600000 as int) g, count(*) n from jobs where st='done' and t1 between ?2 and ?1 group by g", pa![now(), now() - 86_400_000]),
        "tdd": crate::pipe::staged_ui_config(a),
        "lm": if now() - lm["t"].as_i64().unwrap_or(0) < 180_000 { lm } else { Value::Null },
        "nr": a.db.n("select count(*) from rp", pa![]),
        "star": crate::star::show(a),
    })
}

async fn checks(a: &Ax) -> Value {
    use crate::sh::{run, tool, Rq};
    let v = |p: &str, x: &[&str]| {
        let p = p.to_string();
        let x: Vec<String> = x.iter().map(|s| s.to_string()).collect();
        async move {
            run(
                &p,
                &x,
                Rq {
                    t: 30,
                    ..Default::default()
                },
            )
            .await
        }
    };
    let (g, h, c, d, b, codex) = tokio::join!(
        v(&tool("git"), &["--version"]),
        v(&tool("gh"), &["api", "user", "--jq", ".login"]),
        v(&tool("claude"), &["--version"]),
        v(&tool("docker"), &["info", "--format", "{{.ServerVersion}}"]),
        v(&crate::sh::bash(), &["-c", "echo ok"]),
        v(&tool("codex"), &["--version"])
    );
    let _ = a;
    json!({
        "git": {"ok": g.c == 0, "m": if g.c == 0 { g.o.trim().to_string() } else { "Install git: https://git-scm.com".into() }},
        "gh": {"ok": h.c == 0, "m": if h.c == 0 { format!("logged in as {}", h.o.trim()) } else { "Install the GitHub CLI and run: gh auth login".into() }},
        "claude": {"ok": c.c == 0, "m": if c.c == 0 { c.o.trim().to_string() } else { "Install Claude Code and log in: https://claude.com/claude-code".into() }},
        "codex": {"ok": codex.c == 0, "m": if codex.c == 0 { codex.o.trim().to_string() } else { "Install the Codex CLI and log in with codex login".into() }},
        "docker": {"ok": d.c == 0, "m": if d.c == 0 { format!("Docker {}", d.o.trim()) } else { "Docker is not running (optional, recommended)".into() }},
        "bash": {"ok": b.c == 0, "m": if b.c == 0 { "bash ok".to_string() } else { "bash not found (on Windows it comes with Git for Windows)".to_string() }},
    })
}

fn cfg(a: &Ax) -> Value {
    let rs: Vec<Value> =
        a.db.all("select * from rp order by full", pa![])
            .into_iter()
            .map(|mut r| {
                let f = r["full"].as_str().unwrap_or("").to_string();
                r["st"] = rst(a, &f);
                r
            })
            .collect();
    json!({"repos": rs, "hosts": a.db.all("select * from hs order by id='local' desc, id", pa![]), "set": a.set(), "data": a.dd.display().to_string(), "auto": crate::sys::autostart_on(&a.dd), "prov": crate::prov::status(a), "google": crate::goog::linked(a)["email"], "upd": crate::upd::info(a), "asset": crate::upd::asset(), "api": {"on": a.db.kg("api")["h"].is_string(), "t0": a.db.kg("api")["t0"], "last": a.db.kg("api")["last"], "port": a.port}, "hook": {"on": a.db.kg("hook")["h"].is_string(), "t0": a.db.kg("hook")["t0"]}})
}

fn rst(a: &Ax, r: &str) -> Value {
    let t = now() - 14 * 86_400_000;
    let o = a.db.one("select sum(p.st='merged') mg, sum(p.st='open') op, sum(p.st='open' and p.t0<?) old from prs p where p.repo=? and p.t0>?", pa![now() - 7 * 86_400_000, r.to_string(), now() - 90 * 86_400_000]).unwrap_or(json!({}));
    let m14 = a.db.n("select count(*) from prs where repo=? and st='merged' and merged>strftime('%Y-%m-%dT%H:%M:%SZ','now','-14 days')", pa![r.to_string()]);
    let is = a.db.n(
        "select count(*) from jobs where repo=? and num>0 and kind='fix' and t0>?",
        pa![r.to_string(), t],
    );
    let ik = a.db.n("select count(*) from jobs j join prs p on p.repo=j.repo and p.num=j.pr where j.repo=? and j.num>0 and j.kind='fix' and j.t0>?", pa![r.to_string(), t]);
    let q = a.db.n(
        "select count(*) from jobs where repo=? and st in ('vet','ready')",
        pa![r.to_string()],
    );
    let usd =
        a.db.one(
            "select sum(usd) u from cost where repo=? and t>?",
            pa![r.to_string(), t],
        )
        .map(|x| x["u"].as_f64().unwrap_or(0.0))
        .unwrap_or(0.0);
    json!({"m14": m14, "op": o["op"], "old": o["old"], "is": is, "ik": ik, "q": q, "usd": usd, "hy": crate::pipe::hy(a, r), "iv": crate::wk::ivl(a, r) / 60_000, "red": !a.db.kg(&format!("cired:{r}")).is_null(), "hunt_health": crate::hunt_health::status(a, r)})
}

fn costs(a: &Ax) -> Value {
    let t30 = now() - 30 * 86_400_000;
    let rows = a.db.all("select c.repo, c.w, c.md, count(distinct c.job) jobs, count(distinct case when j.pr is not null then c.job end) prs, count(distinct case when p.st='merged' then c.job end) mg, count(distinct case when p.st='closed' then c.job end) cl, sum(c.usd) usd, max(c.sub) sub from cost c join jobs j on j.id=c.job left join prs p on p.repo=j.repo and p.num=j.pr where c.job is not null and c.t>=? group by 1,2,3 order by 1,2,4 desc", pa![now() - 90 * 86_400_000]);
    let mut st = vec![];
    for r in &rows {
        let mut x = r.clone();
        x["nm"] = json!(crate::prov::name(r["md"].as_str().unwrap_or("")));
        st.push(x);
    }
    let attempts = a.db.all(
        "select a.model, count(*) attempts, sum(case when a.outcome='accepted' then 1 else 0 end) accepted, sum(case when a.attempt=1 and a.outcome='accepted' then 1 else 0 end) first_pass, sum(case when a.attempt=1 and exists(select 1 from fix_attempts b where b.job=a.job and b.attempt>1 and b.model<>a.model) then 1 else 0 end) escalated, sum(case when a.gate='verification' and a.outcome in ('rejected','failed') then 1 else 0 end) verification_failures, sum(case when a.gate='ci' and a.outcome in ('rejected','failed') then 1 else 0 end) ci_failures, sum(case when a.gate='review' and a.outcome='rejected' then 1 else 0 end) review_rejections, avg(a.elapsed_ms) avg_ms from fix_attempts a where a.started>=? group by a.model order by attempts desc, a.model",
        pa![now() - 90 * 86_400_000],
    );
    let mut fm = vec![];
    for r in &attempts {
        let mut x = r.clone();
        x["nm"] = json!(crate::prov::name(r["model"].as_str().unwrap_or("")));
        fm.push(x);
    }
    json!({
        "today": a.db.one("select sum(case when sub=0 then usd end) usd, sum(case when sub=1 then usd end) val, sum(case when usd is null then 1 else 0 end) unk from cost where day=?", pa![crate::db::day()]),
        "days": a.db.all("select day, sum(case when sub=0 then usd end) usd, sum(case when sub=1 then usd end) val from cost where day>=date('now','localtime','-13 day') group by day order by day desc", pa![]),
        "repos": a.db.all("select repo, sum(case when sub=0 then usd end) usd, sum(case when sub=1 then usd end) val, count(distinct job) jobs from cost where t>=? and repo<>'' group by repo order by 2 desc", pa![t30]),
        "stats": st,
        "fixer_metrics": fm,
        "first_pass_by_repo": a.db.try_all("select j.repo,f.model,count(*) samples,sum(f.outcome='accepted') accepted,avg(f.elapsed_ms) avg_ms from fix_attempts f join jobs j on j.id=f.job where j.kind='fix' and f.attempt=1 and f.finished is not null and f.outcome in ('accepted','rejected','failed') and f.started>=? group by j.repo,f.model order by j.repo,f.model",pa![now()-90*86_400_000]).ok(),
        "lesson_exposure_metrics": a.db.try_all("select j.repo,f.model,
        case json_type(k.v,'$.included') when 'true' then 'included' when 'false' then 'not_included' else 'unknown' end exposure,
        count(*) samples,sum(f.outcome='accepted') accepted,
        sum(f.gate='verification' and f.outcome in ('failed','rejected')) verification_failures,
        sum(f.gate='review' and f.outcome='rejected') review_rejections
        from fix_attempts f join jobs j on j.id=f.job
        left join kv k on k.k=printf('lesson_exposure:%lld:%lld:%lld',f.id,f.job,f.started)
        and json_valid(k.v)
        where j.kind='fix' and f.attempt=1 and f.finished is not null
        and f.outcome in ('accepted','failed','rejected') and f.started>=?
        group by j.repo,f.model,exposure order by j.repo,f.model,exposure",pa![now()-90*86_400_000]).ok(),
        "hints": hints(&rows),
        "budday": a.set()["budday"],
        "px": a.set()["px"],
    })
}

pub fn hints(rows: &[Value]) -> Vec<String> {
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
    let mut o = vec![];
    for x in rows {
        for y in rows {
            if x["repo"] != y["repo"]
                || x["w"] != y["w"]
                || x["md"] == y["md"]
                || f(x, "jobs") < 5.0
                || f(y, "jobs") < 5.0
            {
                continue;
            }
            let (cx, cy) = (f(x, "usd") / f(x, "jobs"), f(y, "usd") / f(y, "jobs"));
            let (rx, ry) = (f(x, "mg") / f(x, "jobs"), f(y, "mg") / f(y, "jobs"));
            if cy > 0.0 && cx >= cy * 2.0 && ry + 0.05 >= rx {
                o.push(format!(
                    "{} on {}: {} gets as many merges per job as {} at {:.0}× lower cost",
                    x["w"].as_str().unwrap_or(""),
                    x["repo"].as_str().unwrap_or(""),
                    crate::prov::name(y["md"].as_str().unwrap_or("")),
                    crate::prov::name(x["md"].as_str().unwrap_or("")),
                    cx / cy
                ));
            }
        }
    }
    o
}

fn sets(a: &Ax, b: &Value) {
    let mut s = a.db.kg("set");
    if !s.is_object() {
        s = json!({});
    }
    if let Some(m) = b["md"].as_object() {
        let valid_model = regex::Regex::new(r"^[A-Za-z0-9._:/\[\]-]{0,120}$").unwrap();
        for (k, v) in m {
            if let Some(x) = v.as_str() {
                if valid_model.is_match(x.trim()) {
                    s["md"][k] = json!(x.trim());
                }
            }
        }
    }
    if let Some(x) = b["supervised_lessons"].as_bool() {
        s["supervised_lessons"] = json!(x);
    }
    if let Some(x) = b["adaptive_routing"].as_bool() {
        s["adaptive_routing"] = json!(x);
    }
    if let Some(x) = b["adaptive_support_routing"].as_bool() {
        s["adaptive_support_routing"] = json!(x);
    }
    if let Some(x) = b["big"].as_bool() {
        s["big"] = json!(x);
    }
    if let Some(x) = b["resource_profile"]
        .as_str()
        .filter(|p| crate::resources::PROFILES.contains(p))
    {
        s["resource_profile"] = json!(x);
    }
    if let Some(x) = b["codex_hunt_reserve_percent"]
        .as_f64()
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
    {
        s["codex_hunt_reserve_percent"] = json!(x);
    }
    if let Some(x) = b["hunters"].as_i64() {
        s["hunters"] = json!(x.clamp(0, 12));
    }
    if let Some(x) = b["updates"].as_bool() {
        s["updates"] = json!(x && crate::upd::configured());
    }
    if let Some(x) = b["cckc"].as_str() {
        if regex::Regex::new(r"^[A-Za-z0-9._ -]{0,100}$")
            .unwrap()
            .is_match(x.trim())
        {
            s["prov"]["cc"]["kc"] = json!(x.trim());
        }
    }
    if let Some(x) = b["octx"].as_i64() {
        s["prov"]["ollama"]["ctx"] = json!(x.clamp(2048, 1_048_576));
    }
    if let Some(x) = b["ollama"].as_str() {
        let x = x.trim();
        if x.is_empty() || x.starts_with("http://") || x.starts_with("https://") {
            s["prov"]["ollama"]["base"] = json!(x);
        }
    }
    if let Some(x) = b["tokday"].as_i64() {
        s["tokday"] = json!(x.max(0));
    }
    if let Some(x) = b["budday"].as_f64() {
        s["budday"] = json!(x.clamp(0.0, 100_000.0));
    }
    if let Some(o) = b["px"].as_object() {
        let valid_price_key = regex::Regex::new(r"^[a-z0-9._:/-]{1,100}$").unwrap();
        let mut px = json!({});
        for (k, v) in o.iter().take(100) {
            let k = k.trim().to_lowercase();
            let x: Vec<f64> = v
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(|x| x.as_f64())
                        .filter(|x| x.is_finite() && *x >= 0.0 && *x < 10_000.0)
                        .collect()
                })
                .unwrap_or_default();
            if valid_price_key.is_match(&k) && (x.len() == 2 || x.len() == 4) {
                px[k] = json!(x);
            }
        }
        s["px"] = px;
    }
    for k in ["ntfy", "discord"] {
        if let Some(x) = b[k].as_str() {
            let x = x.trim();
            if x.is_empty() || x.starts_with("https://") || k == "ntfy" && x.starts_with("http://")
            {
                s[k] = json!(x);
            }
        }
    }
    a.db.ks("set", &s);
    a.tick();
    if let Some(w) = b["wh"].as_object() {
        let re = regex::Regex::new(r"^([01]\d|2[0-3]):[0-5]\d$").unwrap();
        let (f, t) = (
            w.get("from").and_then(|x| x.as_str()).unwrap_or(""),
            w.get("to").and_then(|x| x.as_str()).unwrap_or(""),
        );
        if re.is_match(f) && re.is_match(t) {
            let mut s = a.db.kg("set");
            s["wh"] = json!({"on": w.get("on").and_then(|x| x.as_bool()).unwrap_or(false), "from": f, "to": t});
            a.db.ks("set", &s);
            a.tick();
        }
    }
}

fn pr_lang(value: &str) -> &'static str {
    if value == "pt-PT" {
        "pt-PT"
    } else {
        "en"
    }
}

const RPF: &[&str] = &[
    "iss",
    "hunt",
    "feat",
    "nm",
    "on_",
    "merge",
    "auto_merge",
    "local_merge",
    "steward_repair",
    "steward_close",
    "coderabbit_free_confirmed",
    "coderabbit_review",
    "coderabbit_repair",
    "auto_recover_drops",
    "maxo",
    "maxd",
    "bud",
    "ci",
    "setup",
    "rules",
    "mode",
    "lang",
    "skl",
];
const HSF: &[&str] = &["id", "ssh", "dk", "ln", "ci", "gb", "raw", "no", "cpu"];

fn export(a: &Ax) -> Value {
    let mut s = a.db.kg("set");
    if let Some(o) = s.as_object_mut() {
        o.remove("ntfy");
        o.remove("discord");
    }
    let pick = |r: &Value, ks: &[&str]| {
        Value::Object(ks.iter().map(|k| (k.to_string(), r[*k].clone())).collect())
    };
    json!({
        "apollo_office": crate::app::V,
        "t": now(),
        "set": s,
        "repos": a.db.all("select * from rp order by full", pa![]).iter().map(|r| { let mut x = pick(r, RPF); x["full"] = r["full"].clone(); x }).collect::<Vec<_>>(),
        "hosts": a.db.all("select * from hs order by id", pa![]).iter().map(|r| pick(r, HSF)).collect::<Vec<_>>(),
    })
}

fn import(a: &Ax, b: &Value) -> Result<String, String> {
    if !b["apollo_office"].is_string() {
        return Err("this is not an ApolloOffice export".into());
    }
    let s = &b["set"];
    let mut x = json!({});
    for k in ["md", "hunters", "tokday", "budday", "updates", "px", "wh"] {
        if !s[k].is_null() {
            x[k] = s[k].clone();
        }
    }
    if let Some(v) = s["prov"]["ollama"]["base"].as_str() {
        x["ollama"] = json!(v);
    }
    if let Some(v) = s["prov"]["ollama"]["ctx"].as_i64() {
        x["octx"] = json!(v);
    }
    if let Some(v) = s["prov"]["cc"]["kc"].as_str() {
        x["cckc"] = json!(v);
    }
    sets(a, &x);
    let mut nr = 0;
    for r in b["repos"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(500)
    {
        let Some(f) = rid(r["full"].as_str().unwrap_or("")) else {
            continue;
        };
        if a.db
            .n("select count(*) from rp where full=?", pa![f.clone()])
            > 0
        {
            continue;
        }
        a.db.run(
            "insert into rp(full,t0,coderabbit_epoch) values(?,?,?)",
            pa![f.clone(), now(), now()],
        );
        for k in RPF {
            // External review requests, autonomous repair and automatic closing
            // must be enabled explicitly on each installation after importing a backup.
            if matches!(
                *k,
                "coderabbit_free_confirmed"
                    | "coderabbit_review"
                    | "coderabbit_repair"
                    | "steward_close"
            ) {
                continue;
            }
            let v = &r[*k];
            // Impedir que valores numéricos contaminem o idioma persistido.
            if *k == "lang" && !v.is_string() {
                continue;
            }
            match v {
                Value::Number(n) if n.is_i64() && *k != "bud" => {
                    a.db.run(
                        &format!("update rp set {k}=? where full=?"),
                        pa![n.as_i64().unwrap().clamp(0, 10000), f.clone()],
                    );
                }
                Value::Number(n) => {
                    a.db.run(
                        &format!("update rp set {k}=? where full=?"),
                        pa![n.as_f64().unwrap_or(0.0).clamp(0.0, 100_000.0), f.clone()],
                    );
                }
                Value::String(t) if ["ci", "setup", "rules", "mode", "lang"].contains(k) => {
                    let t = if *k == "mode" && t != "auto" {
                        "ask".to_string()
                    } else if *k == "lang" {
                        pr_lang(t).to_string()
                    } else {
                        crate::sh::head(t.trim(), 4000)
                    };
                    a.db.run(
                        &format!("update rp set {k}=? where full=?"),
                        pa![t, f.clone()],
                    );
                }
                _ => {}
            }
        }
        nr += 1;
    }
    let mut nh = 0;
    let idr = regex::Regex::new(r"^[A-Za-z0-9._-]{1,40}$").unwrap();
    let shr = regex::Regex::new(r"^[A-Za-z0-9._@-]{0,100}$").unwrap();
    for h in b["hosts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(50)
    {
        let id = h["id"].as_str().unwrap_or("");
        let ssh = if id == "local" {
            ""
        } else {
            h["ssh"].as_str().unwrap_or("")
        };
        if !idr.is_match(id)
            || !shr.is_match(ssh)
            || a.db
                .n("select count(*) from hs where id=?", pa![id.to_string()])
                > 0
        {
            continue;
        }
        let i = |k: &str, d: i64, lo: i64, hi: i64| h[k].as_i64().unwrap_or(d).clamp(lo, hi);
        a.db.run(
            "insert into hs(id,ssh,dk,ln,ci,gb,raw,on_,t0,no,cpu) values(?,?,?,?,?,?,?,0,?,?,?)",
            pa![
                id.to_string(),
                ssh.to_string(),
                i("dk", 0, 0, 1),
                i("ln", 1, 1, 8),
                i("ci", 1, 1, 8),
                i("gb", 50, 5, 10000),
                i("raw", 0, 0, 1),
                now(),
                crate::sh::head(h["no"].as_str().unwrap_or(""), 2000),
                i("cpu", 0, 0, 256)
            ],
        );
        nh += 1;
    }
    a.sw.notify_one();
    Ok(format!("Imported settings, {nr} new repositories and {nh} new machines. Machines come in switched off: check them under Machines, then switch them on."))
}

async fn health(a: &Ax, f: bool) -> Value {
    if let Some(v) = crate::gh::cg(a, "health", 300_000).filter(|_| !f) {
        return v;
    }
    let ck = checks(a).await;
    let s = a.set();
    let mut us: Vec<&str> = vec![];
    if let Some(m) = s["md"].as_object() {
        for v in m.values() {
            let p = crate::prov::parse(v.as_str().unwrap_or("")).0;
            if !us.contains(&p) {
                us.push(p);
            }
        }
    }
    let ps = crate::prov::status(a);
    let mut ai = vec![];
    for p in &us {
        let st = ps
            .as_array()
            .and_then(|x| x.iter().find(|y| y["id"] == *p))
            .cloned()
            .unwrap_or(json!({}));
        let nm = st["nm"].as_str().unwrap_or(p).to_string();
        let (ok, m) = match *p {
            "cc" => (
                ck["claude"]["ok"].as_bool().unwrap_or(false),
                if ck["claude"]["ok"] == true {
                    "Claude Code found".to_string()
                } else {
                    "Claude Code is not installed or not on PATH. Install it and log in with claude.".to_string()
                },
            ),
            "codex" => {
                let r = crate::sh::run(
                    &crate::sh::tool("codex"),
                    &["--version".to_string()],
                    crate::sh::Rq {
                        t: 20,
                        ..Default::default()
                    },
                )
                .await;
                (
                    r.c == 0,
                    if r.c == 0 {
                        "Codex CLI found".to_string()
                    } else {
                        "Codex CLI is not installed. Install it and log in with codex login."
                            .to_string()
                    },
                )
            }
            "ollama" => (true, "local, checked when used".to_string()),
            _ => (
                st["has"] == true,
                if st["has"] == true {
                    "API key saved".to_string()
                } else {
                    format!("No API key for {nm}. Add it under Settings → Providers.")
                },
            ),
        };
        ai.push(json!({"k": nm, "ok": ok, "m": m}));
    }
    let net = a.lv.lock().unwrap().o.net.clone();
    let hs = a.db.all("select id, dk from hs where on_=1", pa![]);
    let ms: Vec<Value> = hs.iter().map(|h| { let id = h["id"].as_str().unwrap_or(""); let ok = *net.get(id).unwrap_or(&true); json!({"k": id, "ok": ok, "m": if ok { "reachable".to_string() } else if id == "local" { "this computer reports a problem, see the log".to_string() } else { format!("{id} is not reachable over ssh. Check that it is on and that ssh {id} works.") }}) }).collect();
    let dk = hs.iter().any(|h| h["dk"] == 1 && h["id"] == "local");
    let v = json!({
        "gh": {"ok": ck["gh"]["ok"], "m": ck["gh"]["m"]},
        "ai": {"ok": ai.iter().all(|x| x["ok"] == true), "m": ai},
        "docker": {"ok": !dk || ck["docker"]["ok"] == true, "m": if dk { ck["docker"]["m"].clone() } else { json!("not used on this computer") }},
        "hosts": {"ok": !ms.is_empty() && ms.iter().all(|x| x["ok"] == true), "m": if ms.is_empty() { json!([{"k": "none", "ok": false, "m": "No machine is switched on. Add one under Machines."}]) } else { json!(ms) }},
        "t": now(),
    });
    crate::gh::cs(a, "health", &v);
    v
}

fn th(t: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(t.as_bytes()))
}

fn tok_ok(a: &Ax, h: &HeaderMap) -> bool {
    let t = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("")
        .trim()
        .to_string();
    let k = a.db.kg("api");
    if t.len() < 20 || k["h"].as_str() != Some(&th(&t)) {
        return false;
    }
    let mut k = k;
    k["last"] = json!(now());
    a.db.ks("api", &k);
    true
}

fn cut(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let h = crate::sh::head(s, n / 5);
    let mut i = s.len() - (n - n / 5);
    while !s.is_char_boundary(i) {
        i += 1;
    }
    format!("{h}\n[...]\n{}", &s[i..])
}

fn rep(a: &Ax, b: &Value) -> Result<Value, (StatusCode, String)> {
    let bad = |m: &str| Err((StatusCode::BAD_REQUEST, m.to_string()));
    let Some(r) = rid(b["repo"].as_str().unwrap_or("")) else {
        return bad("repo must be owner/name");
    };
    if a.db.n(
        "select count(*) from rp where full=? and on_=1",
        pa![r.clone()],
    ) == 0
    {
        return bad("this repository is not added to the office or is switched off");
    }
    if a.db.n(
        "select count(*) from jobs where json_extract(d,'$.api')=1 and st not in ('done','drop')",
        pa![],
    ) >= 20
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "20 reports are already open, try again later".into(),
        ));
    }
    let src: String = b["src"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " ._-".contains(*c))
        .take(40)
        .collect::<String>()
        .trim()
        .to_string();
    if let Some(n) = b["issue"].as_i64() {
        if n <= 0 {
            return bad("issue must be a positive number");
        }
        if let Some(x) = a.db.one(
            "select id from jobs where repo=? and num=? and kind='fix' and st<>'drop'",
            pa![r.clone(), n],
        ) {
            return Ok(json!({"id": x["id"], "new": false}));
        }
        let id = a.db.jnew(
            &r,
            n,
            "fix",
            &format!("Issue #{n}"),
            &format!("https://github.com/{r}/issues/{n}"),
            "vet",
            &format!(
                "sent through the API{}",
                if src.is_empty() {
                    String::new()
                } else {
                    format!(" by {src}")
                }
            ),
            json!({"api": 1, "gi": true}),
            None,
            -0.7,
        );
        a.flow("scout", "vet", &format!("{r}#{n}"));
        a.say(
            "scout",
            &format!("{r}#{n} sent through the API → Vet"),
            Some(id),
        );
        return Ok(json!({"id": id, "new": true}));
    }
    let ti: String = b["title"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect();
    let bd = b["body"].as_str().unwrap_or("").trim().to_string();
    if ti.is_empty() || bd.is_empty() {
        return bad("title and body are required (or issue)");
    }
    let fp: String = b["fp"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect();
    if !fp.is_empty() {
        if let Some(x) = a.db.one("select id, d from jobs where repo=? and json_extract(d,'$.ext.fp')=? and st not in ('done','drop')", pa![r.clone(), fp.clone()]) {
            let id = x["id"].as_i64().unwrap_or(0);
            if let Some(mut j) = a.db.jg(id) {
                j.d["ext"]["n"] = json!(j.d["ext"]["n"].as_i64().unwrap_or(1) + 1);
                a.db.jd(&j);
            }
            return Ok(json!({"id": id, "new": false}));
        }
    }
    let wh: String = b["where"]
        .as_str()
        .unwrap_or("")
        .trim()
        .chars()
        .take(300)
        .collect();
    let lg = b["logs"].as_str().unwrap_or("").trim();
    let mut body = cut(&bd, 20000);
    if !wh.is_empty() {
        body.push_str(&format!("\n\nWhere: {wh}"));
    }
    if !lg.is_empty() {
        body.push_str(&format!("\n\nLogs:\n```\n{}\n```", cut(lg, 20000)));
    }
    let id = a.db.jnew(&r, 0, "fix", &ti, &format!("https://github.com/{r}"), "vet", &format!("reported{}", if src.is_empty() { String::new() } else { format!(" by {src}") }), json!({"api": 1, "ext": {"src": if src.is_empty() { "an external tool".to_string() } else { src.clone() }, "where": wh, "fp": fp}, "title": ti, "body": body}), None, -0.7);
    a.flow("scout", "vet", &r);
    a.say(
        "scout",
        &format!(
            "{r} report {id} received{}: {ti}",
            if src.is_empty() {
                String::new()
            } else {
                format!(" from {src}")
            }
        ),
        Some(id),
    );
    Ok(json!({"id": id, "new": true}))
}

// Report tokens may submit reports but do not grant private source/review access.
fn repst(a: &Ax, id: i64) -> Option<Value> {
    let mut j=a.db.one("select id,repo,num,st,why,pr,t0,t1,json_extract(d,'$.acceptance_proof.ok') acceptance_ok from jobs where id=? and json_extract(d,'$.api')=1",pa![id])?;
    j["routing_models"] = json!(a
        .db
        .all(
            "select distinct json_extract(v,'$.model') model from kv where k like ? limit 32",
            pa![format!("routing:{id}:%")]
        )
        .iter()
        .filter_map(|r| r["model"].as_str().map(str::to_string))
        .collect::<Vec<_>>());
    Some(j)
}

fn repdetail(a: &Ax, id: i64) -> Option<Value> {
    let j = a.db.one(
        "select id,repo,num,st,why,pr,t0,t1 from jobs where id=? and json_extract(d,'$.api')=1",
        pa![id],
    )?;
    let pu = j["pr"].as_i64().map(|n| {
        format!(
            "https://github.com/{}/pull/{n}",
            j["repo"].as_str().unwrap_or("")
        )
    });
    let detail = a.db.jg(id).map(|j| j.d).unwrap_or(Value::Null);
    let lessons =
        a.db.all(
            "select v from kv where k like ?",
            pa![format!("lesson:{id}:%")],
        )
        .iter()
        .filter_map(|r| serde_json::from_str::<Value>(r["v"].as_str()?).ok())
        .collect::<Vec<_>>();
    let routing =
        a.db.all(
            "select v from kv where k like ?",
            pa![format!("routing:{id}:%")],
        )
        .iter()
        .filter_map(|r| serde_json::from_str::<Value>(r["v"].as_str()?).ok())
        .collect::<Vec<_>>();
    let evidence =
        crate::routing::evidence_summary(&routing, &detail, j["st"].as_str().unwrap_or(""));
    Some(
        json!({"evidence_summary":evidence,"requirements":{"base":detail["preflight"]["base"],"tools":detail["preflight"]["tools"],"min_sdk":detail["preflight"]["min_sdk"],"unresolved_dependencies":detail["preflight"]["unresolved_dependencies"]},"acceptance":detail["acceptance_proof"],"diff":detail["df"],"proof":detail["proof"],"result":detail["f"],"review":detail["last_review"],"lessons":lessons,"routing":routing,"id": j["id"], "repo": j["repo"], "issue": if j["num"].as_i64().unwrap_or(0) > 0 { j["num"].clone() } else { Value::Null }, "st": j["st"], "why": j["why"], "pr": pu, "t0": j["t0"], "t1": j["t1"]}),
    )
}

pub fn trk(v: &Value) -> Option<Value> {
    let s = |x: &Value| x.as_str().unwrap_or("").trim().to_string();
    if let Some(at) = v["attachments"].as_array().and_then(|x| x.first()) {
        let ti = s(&at["title"]);
        if ti.is_empty() {
            return None;
        }
        let mut b = format!("Error tracker alert from GlitchTip: {ti}");
        if !s(&at["text"]).is_empty() {
            b.push_str(&format!("\n\nCulprit: {}", s(&at["text"])));
        }
        for f in at["fields"].as_array().cloned().unwrap_or_default() {
            b.push_str(&format!("\n{}: {}", s(&f["title"]), s(&f["value"])));
        }
        let ln = s(&at["title_link"]);
        if !ln.is_empty() {
            b.push_str(&format!("\nLink: {ln}"));
        }
        return Some(
            json!({"title": ti, "body": b, "where": s(&at["text"]), "src": "GlitchTip", "fp": format!("gt:{ln}")}),
        );
    }
    let d = &v["data"];
    let e = if d["event"].is_object() {
        &d["event"]
    } else {
        &d["error"]
    };
    let i = &d["issue"];
    let ti = [s(&e["title"]), s(&i["title"])]
        .into_iter()
        .find(|x| !x.is_empty())?;
    let wh = [s(&e["culprit"]), s(&i["culprit"]), s(&e["location"])]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or_default();
    let mut b = format!("Error tracker alert from Sentry: {ti}");
    if !wh.is_empty() {
        b.push_str(&format!("\n\nCulprit: {wh}"));
    }
    let msg = [s(&e["message"]), s(&e["logentry"]["formatted"])]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or_default();
    if !msg.is_empty() && msg != ti {
        b.push_str(&format!("\nMessage: {msg}"));
    }
    for (k, n) in [
        ("release", "Release"),
        ("environment", "Environment"),
        ("platform", "Platform"),
    ] {
        if !s(&e[k]).is_empty() {
            b.push_str(&format!("\n{n}: {}", s(&e[k])));
        }
    }
    let mut st = String::new();
    for x in e["exception"]["values"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        st.push_str(&format!("\n{}: {}", s(&x["type"]), s(&x["value"])));
        let fr = x["stacktrace"]["frames"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let ap: Vec<&Value> = fr
            .iter()
            .filter(|f| f["in_app"].as_bool().unwrap_or(false))
            .collect();
        let fs: Vec<&Value> = if ap.is_empty() {
            fr.iter().collect()
        } else {
            ap
        };
        for f in fs.iter().rev().take(15) {
            st.push_str(&format!(
                "\n  at {} ({}:{})",
                s(&f["function"]),
                [s(&f["filename"]), s(&f["abs_path"]), s(&f["module"])]
                    .into_iter()
                    .find(|x| !x.is_empty())
                    .unwrap_or_default(),
                f["lineno"]
                    .as_i64()
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            ));
        }
    }
    if st.is_empty() && i["metadata"].is_object() {
        let m = &i["metadata"];
        st = format!(
            "\n{}: {} in {} ({})",
            s(&m["type"]),
            s(&m["value"]),
            s(&m["function"]),
            s(&m["filename"])
        );
    }
    let ln = [s(&e["web_url"]), s(&i["web_url"]), s(&i["permalink"])]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or_default();
    if !ln.is_empty() {
        b.push_str(&format!("\nLink: {ln}"));
    }
    let id = [
        s(&i["id"]),
        s(&e["issue_id"]),
        s(&e["groupID"]),
        e["issue_id"]
            .as_i64()
            .map(|n| n.to_string())
            .unwrap_or_default(),
        i["id"].as_i64().map(|n| n.to_string()).unwrap_or_default(),
    ]
    .into_iter()
    .find(|x| !x.is_empty())
    .unwrap_or_else(|| ti.clone());
    Some(
        json!({"title": ti, "body": b, "where": wh, "logs": st.trim(), "src": "Sentry", "fp": format!("sentry:{id}")}),
    )
}

async fn hook(a: &Ax, q: &str, hd: &HeaderMap, b: Body) -> Response<Body> {
    let qv = |k: &str| {
        q.split('&')
            .find_map(|x| x.strip_prefix(&format!("{k}=")))
            .unwrap_or("")
            .to_string()
    };
    let k = a.db.kg("hook");
    let hk = hd
        .get("x-hook-key")
        .or(hd.get("authorization"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim_start_matches("Bearer ").trim().to_string())
        .unwrap_or_default();
    let t = if hk.is_empty() { qv("key") } else { hk };
    if t.len() < 20 || k["h"].as_str() != Some(&th(&t)) {
        return er(
            StatusCode::UNAUTHORIZED,
            "missing or wrong key (Settings → Error trackers)",
        );
    }
    let Ok(by) = to_bytes(b, 1 << 20).await else {
        return er(StatusCode::PAYLOAD_TOO_LARGE, "body larger than 1 MB");
    };
    let Ok(v) = serde_json::from_slice::<Value>(&by) else {
        return er(StatusCode::BAD_REQUEST, "body must be JSON");
    };
    if v["action"]
        .as_str()
        .is_some_and(|x| !matches!(x, "created" | "triggered"))
    {
        return js(json!({"ok": true, "skip": "action"}));
    }
    let Some(mut r) = trk(&v) else {
        return js(json!({"ok": true, "skip": "no error in payload"}));
    };
    r["repo"] = json!(qv("repo").replace("%2F", "/").replace("%2f", "/"));
    match rep(a, &r) {
        Ok(x) => js(x),
        Err((c, e)) => er(c, &e),
    }
}

async fn v1(a: &Ax, m: &Method, p: &str, q: &str, hd: &HeaderMap, b: Body) -> Response<Body> {
    if m == Method::POST && p == "/v1/hook" {
        return hook(a, q, hd, b).await;
    }
    if !tok_ok(a, hd) {
        return er(
            StatusCode::UNAUTHORIZED,
            "missing or wrong API token (Settings → API)",
        );
    }
    let qv = |k: &str| {
        q.split('&')
            .find_map(|x| x.strip_prefix(&format!("{k}=")))
            .unwrap_or("")
            .to_string()
    };
    match (m.as_str(), p) {
        ("POST", "/v1/report") => {
            let Ok(by) = to_bytes(b, 1 << 20).await else {
                return er(StatusCode::PAYLOAD_TOO_LARGE, "body larger than 1 MB");
            };
            let Ok(v) = serde_json::from_slice::<Value>(&by) else {
                return er(StatusCode::BAD_REQUEST, "body must be JSON");
            };
            match rep(a, &v) {
                Ok(r) => js(r),
                Err((c, e)) => er(c, &e),
            }
        }
        ("GET", "/v1/report") => match repst(a, qv("id").parse().unwrap_or(0)) {
            Some(v) => js(v),
            None => er(StatusCode::NOT_FOUND, "no report with this id"),
        },
        ("GET", "/v1/reports") => js(json!(a
            .db
            .all(
                "select id from jobs where json_extract(d,'$.api')=1 order by id desc limit 50",
                pa![]
            )
            .iter()
            .filter_map(|x| repst(a, x["id"].as_i64().unwrap_or(0)))
            .collect::<Vec<_>>())),
        ("GET", "/v1/repos") => js(json!(a
            .db
            .all("select full from rp where on_=1 order by full", pa![])
            .iter()
            .filter_map(|x| x["full"].as_str().map(String::from))
            .collect::<Vec<_>>())),
        _ => er(StatusCode::NOT_FOUND, "unknown endpoint"),
    }
}

async fn ws(a: Ax, mut s: WebSocket) {
    let mut last = String::new();
    loop {
        let n = crate::app::CHG.notified();
        tokio::pin!(n);
        n.as_mut().enable();
        let mut v = state(&a);
        v["t"] = json!(0);
        let k = v.to_string();
        if k != last {
            v["t"] = json!(now());
            if s.send(Message::Text(v.to_string().into())).await.is_err() {
                return;
            }
            last = k;
        }
        tokio::select! {
            _ = &mut n => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
            m = s.recv() => if !matches!(m, Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Binary(_)))) { return },
            _ = a.quit.notified() => return,
        }
    }
}

fn rid(s: &str) -> Option<String> {
    let s = s
        .trim()
        .trim_start_matches("https://github.com/")
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let re = regex::Regex::new(r"^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$").unwrap();
    if re.is_match(s) {
        Some(s.to_string())
    } else {
        None
    }
}

#[cfg(test)]
fn branch_update_matches_pr(update: &Value, head: &Value, base: &Value) -> bool {
    let status = update["status"].as_str().unwrap_or("");
    if matches!(status, "error" | "blocked") {
        update["head"] == *head
    } else {
        update["head"] == *head || update["base"] == *base
    }
}

fn steward_report(a: &Ax, repo: &str) -> Value {
    let mut report = a.db.kg(&format!("steward:{repo}"));
    if !report.is_object() {
        return report;
    }
    let jobs = a.db.all(
        "select id,st,t0,t1,why,d,pr from jobs where repo=? and kind='mergecheck' and (st not in ('done','drop') or t0>=?) order by id desc",
        pa![repo.to_string(), now() - 90 * 86_400_000],
    );
    let at = now();
    let active: Vec<_> = jobs
        .iter()
        .filter(|j| !matches!(j["st"].as_str(), Some("done" | "drop")))
        .collect();
    if let Some(prs) = report["prs"].as_array_mut() {
        for pr in prs {
            let number = pr["number"].as_i64().unwrap_or(0);
            pr["coderabbit_previous"] = crate::coderabbit::previous(&a.db, repo, number);
            pr["coderabbit_request"] = crate::coderabbit::request_history(&a.db, repo, number);
            pr["coderabbit_findings"] = crate::coderabbit::finding_status(&a.db, repo, number);
            let latest = jobs.iter().find(|j| j["pr"].as_i64() == Some(number));
            pr["recheck"] = latest.map(|job| {
                let expected_head = job["d"]["expected_head"].as_str().unwrap_or("");
                let expected_base = job["d"]["expected_base"].as_str().unwrap_or("");
                let started_at = job["t0"].as_i64().unwrap_or(at);
                let age_ms = if matches!(job["st"].as_str(), Some("done" | "drop")) {
                    job["t1"]
                        .as_i64()
                        .filter(|updated| *updated > 0)
                        .map(|updated| updated.saturating_sub(started_at))
                        .unwrap_or_else(|| at.saturating_sub(started_at))
                } else {
                    at.saturating_sub(started_at)
                };
                json!({
                    "id":job["id"], "state":job["st"], "started_at":job["t0"], "updated_at":job["t1"],
                    "age_ms":age_ms, "why":job["why"],
                    "expected_head":expected_head, "expected_base":expected_base,
                    "matches_head":expected_head == pr["head_sha"].as_str().unwrap_or(""),
                    "matches_base":expected_base == pr["base_sha"].as_str().unwrap_or("")
                })
            }).unwrap_or(Value::Null);
            let update = a.db.kg(&format!("steward:update:{repo}#{number}"));
            pr["branch_update"] =
                if update["head"] == pr["head_sha"] || update["base"] == pr["base_sha"] {
                    update
                } else {
                    Value::Null
                };
        }
    }
    report["queue"] = json!({
        "pending_rechecks":active.len(),
        "oldest_age_ms":active.iter().map(|j| at.saturating_sub(j["t0"].as_i64().unwrap_or(at))).max().unwrap_or(0)
    });
    report
}

/// Routes authenticated application commands, including safe manual updates.
async fn api(a: &Ax, m: &Method, p: &str, q: &str, b: Value) -> Response<Body> {
    let qv = |k: &str| {
        q.split('&')
            .find_map(|x| x.strip_prefix(&format!("{k}=")))
            .unwrap_or("")
            .to_string()
    };
    match (m.as_str(), p) {
        ("GET", "/api/state") => js(state(a)),
        ("GET", "/api/cfg") => js(cfg(a)),
        ("GET", "/api/check") => js(checks(a).await),
        ("GET", "/api/failures") => js(json!({
            "jobs": a.db.all(
                "select id,repo,kind,title,why,t1, json_extract(d,'$.failure.stage') stage, json_extract(d,'$.failure.exit_code') exit_code from jobs where st='drop' and coalesce(why,'') not in ('dropped by the owner','repository removed') order by t1 desc limit 80",
                pa![],
            ),
            "blocked": crate::hunt_health::blocked_repos(a),
        })),
        ("GET", "/api/report") => match repdetail(a, qv("id").parse().unwrap_or(0)) {
            Some(value) => js(value),
            None => er(StatusCode::NOT_FOUND, "no report with this id"),
        },
        ("GET", "/api/job") => {
            let id: i64 = qv("id").parse().unwrap_or(0);
            match a.db.one("select * from jobs where id=?", pa![id]) {
                Some(j) => js(
                    json!({"id": j["id"], "repo": j["repo"], "num": j["num"], "kind": j["kind"], "work_class": work_class(&j), "title": j["title"], "url": j["url"], "st": j["st"], "why": j["why"], "schedule": a.db.one("select manual_priority,retry_after,retry_reason from job_schedule where job=?",pa![id]), "adaptive_route": j["d"]["adaptive_route"], "schedule_editable": j["st"]=="ready" && j["kind"]=="fix" && j["d"]["rs"]!=true && j["d"]["rp"]!=true && !a.lk.lock().unwrap().contains(&id), "dependencies": a.db.all("select d.prerequisite,j.st,p.st pr_state from job_dependencies d left join jobs j on j.id=d.prerequisite left join prs p on p.repo=j.repo and p.num=j.pr where d.job=? order by d.prerequisite",pa![id]), "failure": j["d"]["failure"], "pt": j["d"]["pt"], "pb": j["d"]["pb"], "df": j["d"]["df"], "proof": j["d"]["proof"], "known": j["d"]["known"], "h": j["d"]["h"], "ext": j["d"]["ext"]["src"], "cost": a.db.one("select sum(case when sub=0 then usd end) usd, sum(case when sub=1 then usd end) val, sum(tin+tcw+tcr) ti, sum(tout) tou, count(*) n from cost where job=?", pa![id]), "ev": a.db.all("select t,w,m from ev where j=? order by id desc limit 80", pa![id])}),
                ),
                None => er(StatusCode::NOT_FOUND, "no such job"),
            }
        }
        ("POST", "/api/act") => {
            let id = b["id"].as_i64().unwrap_or(0);
            match b["a"].as_str().unwrap_or("") {
                "schedule" => {
                    let priority = if b["priority"].is_null() {
                        None
                    } else {
                        match b["priority"].as_i64().filter(|p| (0..=3).contains(p)) {
                            Some(p) => Some(p),
                            None => {
                                return er(StatusCode::BAD_REQUEST, "priority must be 0–3 or null")
                            }
                        }
                    };
                    let Some(dependencies) = b["dependencies"].as_array().and_then(|rows| {
                        rows.iter().map(Value::as_i64).collect::<Option<Vec<_>>>()
                    }) else {
                        return er(
                            StatusCode::BAD_REQUEST,
                            "dependencies must be an array of job ids",
                        );
                    };
                    let result = {
                        let _transition = a.update_transition.lock().unwrap();
                        if a.lk.lock().unwrap().contains(&id)
                            || a.drain.load(std::sync::atomic::Ordering::SeqCst)
                        {
                            return er(
                                StatusCode::CONFLICT,
                                "job already assigned or office draining",
                            );
                        }
                        a.db.set_job_schedule(id, priority, &dependencies)
                    };
                    if let Err(error) = result {
                        eprintln!("job schedule {id}: {error}");
                        return er(
                            StatusCode::CONFLICT,
                            "invalid job, dependencies, cycle, or scheduling database unavailable",
                        );
                    }
                    a.sw.notify_one();
                }
                "reset-hunt" => {
                    let repo = b["repo"].as_str().unwrap_or("");
                    if a.db.n(
                        "select count(*) from rp where full=?",
                        pa![repo.to_string()],
                    ) != 1
                    {
                        return er(StatusCode::BAD_REQUEST, "unknown repository");
                    }
                    crate::hunt_health::reset(a, repo);
                }
                "pause" => {
                    let on = b["on"].as_bool().unwrap_or(false);
                    let _transition = a.update_transition.lock().unwrap();
                    if a.db.run(
                        "insert into kv(k,v) values('man',?) on conflict(k) do update set v=excluded.v",
                        pa![json!(on).to_string()],
                    ) < 0 {
                        return er(StatusCode::CONFLICT, "office pause was not saved");
                    }
                    a.lv.lock().unwrap().o.man = on;
                    a.say(
                        "scout",
                        if on {
                            "office paused"
                        } else {
                            "office resumed"
                        },
                        None,
                    );
                }
                "hold" => {
                    let w = b["w"].as_str().unwrap_or("").to_string();
                    let on = b["on"].as_bool().unwrap_or(false);
                    if let Some(x) = a.lv.lock().unwrap().w.get_mut(&w) {
                        x.hold = on;
                    }
                    let mut h = a.db.kg("hold");
                    if !h.is_object() {
                        h = json!({});
                    }
                    h[&w] = json!(on);
                    a.db.ks("hold", &h);
                }
                "drop" => {
                    if let Some(j) = a.db.jg(id) {
                        if ["ready", "vet", "approve"].contains(&j.st.as_str()) {
                            a.db.jset(
                                id,
                                &[
                                    ("st", json!("drop")),
                                    ("why", json!("dropped by the owner")),
                                ],
                            );
                        } else if !["done", "drop"].contains(&j.st.as_str()) {
                            a.db.ks(&format!("kill:{id}"), &json!(true));
                        }
                    }
                }
                "approve" => {
                    if let Some(mut j) = a.db.jg(id) {
                        if j.st != "approve" {
                            return er(StatusCode::CONFLICT, "job is not waiting for approval");
                        }
                        if let Some(t) = b["pt"].as_str().filter(|s| !s.trim().is_empty()) {
                            j.d["pt"] = json!(t.trim());
                        }
                        if let Some(t) = b["pb"].as_str().filter(|s| !s.trim().is_empty()) {
                            j.d["pb"] = json!(t.trim());
                        }
                        j.d["ap"] = json!(true);
                        a.db.jset(
                            id,
                            &[
                                ("st", json!("submit")),
                                ("why", json!("approved, submitting")),
                                ("d", j.d),
                            ],
                        );
                    }
                }
                "requeue" => match crate::drop_recovery::retry_manual(a, id).await {
                    Ok(true) => {}
                    Ok(false) => {
                        return er(StatusCode::CONFLICT, "Job alterado durante a recuperação");
                    }
                    Err(reason) => return er(StatusCode::CONFLICT, &reason),
                },
                _ => return er(StatusCode::BAD_REQUEST, "unknown action"),
            }
            js(state(a))
        }
        ("POST", "/api/repo") => {
            let op = b["op"].as_str().unwrap_or("");
            let Some(r) = rid(b["full"].as_str().unwrap_or("")) else {
                return er(StatusCode::BAD_REQUEST, "use owner/name");
            };
            match op {
                "add" => {
                    let mt = match crate::gh::meta(a, &r).await {
                        Ok(v) => v,
                        Err(e) => {
                            return er(StatusCode::BAD_REQUEST, &format!("cannot read {r}: {e}"))
                        }
                    };
                    if mt["arch"].as_bool().unwrap_or(false) {
                        return er(StatusCode::BAD_REQUEST, "repository is archived");
                    }
                    let r = mt["full"].as_str().unwrap_or(&r).to_string();
                    let (c, s) = crate::gh::detect(&r).await;
                    if a.db.run(
                        "insert or ignore into rp(full,ci,setup,mode,t0,coderabbit_epoch) values(?,?,?,?,?,?)",
                        pa![r.clone(), c.clone(), s.clone(), "ask".to_string(), now(), now()],
                    ) < 0 {
                        return er(StatusCode::CONFLICT, "repository was not added");
                    }
                    a.sw.notify_one();
                    js(json!({"full": r, "ci": c, "setup": s, "cfg": cfg(a)}))
                }
                "set" => {
                    if save_repo_settings(&a.db, &r, &b).is_err() {
                        return er(StatusCode::CONFLICT, "repository settings were not saved");
                    }
                    a.sw.notify_one();
                    a.steward_wake.notify_one();
                    js(cfg(a))
                }
                "detect" => {
                    let (c, s) = crate::gh::detect(&r).await;
                    js(json!({"ci": c, "setup": s}))
                }
                "del" => {
                    if remove_repository(&a.db, &r).is_err() {
                        return er(StatusCode::CONFLICT, "repository was not removed");
                    }
                    js(cfg(a))
                }
                _ => er(StatusCode::BAD_REQUEST, "unknown op"),
            }
        }
        ("POST", "/api/host") => {
            let op = b["op"].as_str().unwrap_or("");
            let id = b["id"].as_str().unwrap_or("").trim().to_string();
            if !regex::Regex::new(r"^[A-Za-z0-9._-]{1,40}$")
                .unwrap()
                .is_match(&id)
            {
                return er(StatusCode::BAD_REQUEST, "name: letters, digits, . _ - only");
            }
            match op {
                "add" | "set" => {
                    let ssh = if id == "local" {
                        String::new()
                    } else {
                        b["ssh"].as_str().unwrap_or(&id).trim().to_string()
                    };
                    if !ssh.is_empty()
                        && !regex::Regex::new(r"^[A-Za-z0-9._@-]{1,100}$")
                            .unwrap()
                            .is_match(&ssh)
                    {
                        return er(
                            StatusCode::BAD_REQUEST,
                            "ssh: use a host alias from ~/.ssh/config or user@host",
                        );
                    }
                    let dk = b["dk"].as_bool().unwrap_or(false);
                    if !dk && !b["raw"].as_bool().unwrap_or(false) {
                        return er(StatusCode::BAD_REQUEST, "without Docker the workers run repository code directly on this machine; confirm that first");
                    }
                    let no = crate::sh::head(b["no"].as_str().unwrap_or("").trim(), 2000);
                    {
                        let _transition = a.update_transition.lock().unwrap();
                        if a.drain.load(std::sync::atomic::Ordering::SeqCst)
                            || !a.act.lock().unwrap().is_empty()
                            || !a.cache_maintenance.lock().unwrap().is_empty()
                        {
                            return er(StatusCode::CONFLICT,"wait until active workers and maintenance finish before changing machines");
                        }
                        if a.db.run("insert into hs(id,ssh,dk,ln,ci,gb,raw,on_,t0,no,cpu) values(?,?,?,?,?,?,?,?,?,?,?) on conflict(id) do update set ssh=excluded.ssh, dk=excluded.dk, ln=excluded.ln, ci=excluded.ci, gb=excluded.gb, raw=excluded.raw, on_=excluded.on_, no=excluded.no, cpu=excluded.cpu", pa![id.clone(), ssh, dk as i64, b["ln"].as_i64().unwrap_or(1).clamp(1, 8), b["ci"].as_i64().unwrap_or(1).clamp(1, 8), b["gb"].as_i64().unwrap_or(50).clamp(5, 10000), (!dk) as i64, b["on"].as_bool().unwrap_or(true) as i64, now(), no, b["cpu"].as_i64().unwrap_or(0).clamp(0, 256)]) < 0 { return er(StatusCode::INTERNAL_SERVER_ERROR,"could not save machine"); }
                        a.db.kd(&format!("up:{id}"));
                    }
                    match crate::sh::ensure(a, &id).await {
                        Ok(()) => {
                            if dk {
                                a.db.ks(&format!("up:{id}"), &json!(true));
                            }
                            a.lv.lock().unwrap().o.net.insert(id.clone(), true);
                            js(json!({"ok": true, "cfg": cfg(a)}))
                        }
                        Err(m) => js(json!({"ok": false, "m": m, "cfg": cfg(a)})),
                    }
                }
                "del" => {
                    {
                        let _transition = a.update_transition.lock().unwrap();
                        if a.drain.load(std::sync::atomic::Ordering::SeqCst)
                            || !a.act.lock().unwrap().is_empty()
                            || !a.cache_maintenance.lock().unwrap().is_empty()
                        {
                            return er(StatusCode::CONFLICT,"wait until active workers and maintenance finish before removing machines");
                        }
                        if a.db.run("delete from hs where id=?", pa![id.clone()]) < 0 {
                            return er(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "could not remove machine",
                            );
                        }
                    }
                    js(cfg(a))
                }
                _ => er(StatusCode::BAD_REQUEST, "unknown op"),
            }
        }
        ("POST", "/api/set") => {
            sets(a, &b);
            if b["test"].as_bool().unwrap_or(false) {
                crate::ntf::send(a, "ApolloOffice", "Test notification", 3).await;
            }
            if let Some(x) = b["auto"].as_bool() {
                let _ = crate::sys::autostart(x, &a.me, &a.dd, a.port);
            }
            js(cfg(a))
        }
        ("POST", "/api/prov") => {
            let id = b["id"].as_str().unwrap_or("");
            let Some(p) = crate::prov::pv(id) else {
                return er(StatusCode::BAD_REQUEST, "unknown provider");
            };
            if let Some(k) = b["key"].as_str() {
                if !p.key && id != "cc" {
                    return er(StatusCode::BAD_REQUEST, "this provider has no API key");
                }
                if let Err(e) = crate::vault::put(a, id, k.trim()) {
                    return er(StatusCode::INTERNAL_SERVER_ERROR, &e);
                }
            }
            js(cfg(a))
        }
        ("GET", "/api/models") => match crate::prov::models(a, &qv("p")).await {
            Ok(v) => js(json!({"ms": v})),
            Err(e) => js(json!({"ms": [], "error": e})),
        },
        ("POST", "/api/test") => {
            let md = b["md"].as_str().unwrap_or("").to_string();
            match crate::prov::test(a, &md).await {
                Ok(t) => js(json!({"ok": true, "m": t})),
                Err(e) => js(json!({"ok": false, "m": crate::wk::errm(&e)})),
            }
        }
        ("POST", "/api/steward/coderabbit") => {
            let repo = b["repo"].as_str().unwrap_or("");
            let number = b["number"].as_i64().unwrap_or(0);
            let head = b["head"].as_str().unwrap_or("");
            if !crate::coderabbit::valid_scope(repo, number, head) {
                return er(StatusCode::BAD_REQUEST, "invalid observation scope");
            }
            // Limit manual reads to PRs actually present in the current Steward snapshot.
            let report = a.db.kg(&format!("steward:{repo}"));
            if !report["prs"].as_array().is_some_and(|rows| {
                rows.iter()
                    .any(|pr| pr["number"].as_i64() == Some(number) && pr["head_sha"] == head)
            }) {
                return er(StatusCode::CONFLICT, "refresh Steward before observing");
            }
            let Ok(ticket) = crate::coderabbit::reserve(&a.db, repo, number) else {
                return er(StatusCode::CONFLICT, "observation unavailable");
            };
            match crate::coderabbit::observe(repo, number, head).await {
                Ok(value) => match crate::coderabbit::save(&a.db, repo, number, &ticket, &value) {
                    Ok(true) => js(value),
                    _ => js(json!({"state":"unavailable","head":head})),
                },
                Err(error) if error == crate::coderabbit::RATE_LIMITED => {
                    js(json!({"state":"rate_limited","head":head}))
                }
                Err(error) if error == crate::coderabbit::OBSERVATION_BUSY => {
                    js(json!({"state":"busy","head":head}))
                }
                Err(_) => js(json!({"state":"unavailable","head":head})),
            }
        }
        ("GET", "/api/steward") => js(json!({
            "repos": a.db.all("select full from rp where on_=1 order by full", pa![])
                .iter()
                .map(|row| {
                    let repo = row["full"].as_str().unwrap_or("");
                    json!({
                        "repo": repo,
                        "report": steward_report(a, repo),
                        "error": a.db.kg(&format!("steward:error:{repo}")),
                        "merge_error": a.db.kg(&format!("steward:merge_error:{repo}")),
                        "repair_error": a.db.kg(&format!("steward:repair_error:{repo}")),
                        "close_error": a.db.kg(&format!("steward:close_error:{repo}")),
                        "drops": crate::drop_recovery::report(a, repo)
                    })
                })
                .collect::<Vec<Value>>()
        })),
        ("GET", "/api/watcher-outbox") => {
            let offset = if qv("offset").is_empty() {
                0
            } else {
                match qv("offset").parse::<i64>() {
                    Ok(n) if (0..=100_000).contains(&n) => n,
                    _ => return er(StatusCode::BAD_REQUEST, "invalid page offset"),
                }
            };
            match (
                a.db.watcher_outbox_page(offset),
                a.db.watcher_outbox_counts(),
            ) {
                (Ok(mut rows), Ok(counts)) => {
                    let more = rows.len() > 50;
                    rows.truncate(50);
                    js(json!({"rows":rows,"counts":counts,"offset":offset,"more":more}))
                }
                _ => er(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "outbox inventory unavailable",
                ),
            }
        }
        ("POST", "/api/watcher-outbox") => {
            let Some(id) = b["id"]
                .as_str()
                .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
            else {
                return er(StatusCode::BAD_REQUEST, "invalid reply identifier");
            };
            match b["action"].as_str() {
                Some("cancel") => {
                    // CAS refuses cancellation once a worker has claimed a send.
                    match a.db.finish_watcher_reply(id, "cancelled") {
                        Ok(true) => {
                            a.say("watcher", "owner cancelled a pending reply", None);
                            js(json!({"ok":true,"state":"cancelled"}))
                        }
                        Ok(false) => er(StatusCode::CONFLICT, "reply is no longer pending"),
                        Err(_) => er(StatusCode::INTERNAL_SERVER_ERROR, "could not cancel reply"),
                    }
                }
                Some("verify") => {
                    let rows=match a.db.try_all("select repo,num,author,body from watcher_outbox where id=? and state='uncertain'",pa![id.to_string()]){
                        Ok(rows)=>rows,
                        Err(_)=>return er(StatusCode::INTERNAL_SERVER_ERROR,"outbox inventory unavailable")
                    };
                    let Some(row) = rows.first() else {
                        return er(StatusCode::CONFLICT, "reply is no longer uncertain");
                    };
                    let (Some(repo), Some(number), Some(author), Some(body)) = (
                        row["repo"].as_str(),
                        row["num"].as_i64(),
                        row["author"].as_str(),
                        row["body"].as_str(),
                    ) else {
                        return er(StatusCode::INTERNAL_SERVER_ERROR, "invalid stored reply");
                    };
                    let comments = match crate::sh::apip_unique(&format!(
                        "repos/{repo}/issues/{number}/comments?per_page=100"
                    ))
                    .await
                    {
                        Ok(rows) => rows,
                        Err(_) => {
                            return er(
                                StatusCode::BAD_GATEWAY,
                                "could not obtain complete GitHub comments",
                            )
                        }
                    };
                    if !crate::wk::reply_confirmed(&comments, author, body) {
                        return js(json!({"ok":true,"state":"uncertain","confirmed":false}));
                    }
                    match a.db.finish_watcher_reply(id, "sent") {
                        Ok(true) => {
                            a.say("watcher", "owner verified reply delivery on GitHub", None);
                            js(json!({"ok":true,"state":"sent","confirmed":true}))
                        }
                        Ok(false) => er(
                            StatusCode::CONFLICT,
                            "reply state changed; refresh inventory",
                        ),
                        Err(_) => er(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "could not persist confirmation",
                        ),
                    }
                }
                _ => er(StatusCode::BAD_REQUEST, "unsupported outbox action"),
            }
        }
        ("GET", "/api/lessons") => match crate::routing::lessons(a) {
            Ok(rows) => js(json!({"lessons":rows})),
            Err(error) => {
                eprintln!("lesson inventory: {error}");
                er(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "lesson inventory unavailable",
                )
            }
        },
        ("POST", "/api/lessons") => match crate::routing::review_lesson(
            a,
            b["key"].as_str().unwrap_or(""),
            b["action"].as_str().unwrap_or(""),
            b["note"].as_str().unwrap_or(""),
            b["days"].as_i64().unwrap_or(30),
        ) {
            Ok(()) => js(json!({"ok":true})),
            Err(error) => er(StatusCode::CONFLICT, &error),
        },
        ("GET", "/api/cost") => js(costs(a)),
        ("GET", "/api/health") => js(health(a, qv("f") == "1").await),
        ("GET", "/api/export") => js(export(a)),
        ("POST", "/api/import") => match import(a, &b) {
            Ok(m) => js(json!({"ok": true, "m": m, "cfg": cfg(a)})),
            Err(e) => er(StatusCode::BAD_REQUEST, &e),
        },
        ("POST", "/api/danger") => {
            let op = b["op"].as_str().unwrap_or("");
            match op {
                "logout" => {
                    a.db.run("delete from ses", pa![]);
                    a.say("scout", "signed out everywhere", None);
                }
                "history" => {
                    if clear_job_history(&a.db).is_err() {
                        return er(StatusCode::CONFLICT, "job history was not cleared");
                    }
                    a.say(
                        "scout",
                        "job history cleared; active prerequisites retained",
                        None,
                    );
                }
                "reset" => {
                    {
                        let _transition = a.update_transition.lock().unwrap();
                        if a.drain.load(std::sync::atomic::Ordering::SeqCst) {
                            return er(StatusCode::CONFLICT, "update is in progress");
                        }
                        if a.db.run("delete from kv where k in ('set','hold','man')", pa![]) < 0 {
                            return er(StatusCode::CONFLICT, "settings were not reset");
                        }
                        let mut l = a.lv.lock().unwrap();
                        l.o.man = false;
                        for w in l.w.values_mut() {
                            w.hold = false;
                        }
                    }
                    a.tick();
                    a.say("scout", "settings reset to defaults", None);
                }
                "wipe" => {
                    if b["sure"].as_str() != Some("delete") {
                        return er(StatusCode::BAD_REQUEST, "type delete to confirm");
                    }
                    let _transition = a.update_transition.lock().unwrap();
                    if a.drain.load(std::sync::atomic::Ordering::SeqCst)
                        || !crate::upd::manually_paused(a)
                        || !crate::upd::idle(a)
                        || !a.cache_maintenance.lock().unwrap().is_empty()
                    {
                        return er(
                            StatusCode::CONFLICT,
                            "pause the office and wait for active work before deleting data",
                        );
                    }
                    if wipe_office_data(&a.db).is_err() {
                        return er(
                            StatusCode::CONFLICT,
                            "data deletion failed; office remains paused",
                        );
                    }
                    *a.vk.lock().unwrap() = None;
                    a.say("scout", "all data deleted, the office is paused", None);
                }
                _ => return er(StatusCode::BAD_REQUEST, "unknown op"),
            }
            js(json!({"ok": true}))
        }
        ("POST", "/api/hooktok") => match b["op"].as_str().unwrap_or("") {
            "new" => {
                let t = hex::encode(rand::random::<[u8; 24]>());
                a.db.ks("hook", &json!({"h": th(&t), "t0": now()}));
                a.say("scout", "new error tracker link created", None);
                js(json!({"ok": true, "tok": t}))
            }
            "off" => {
                a.db.kd("hook");
                a.say("scout", "error tracker link revoked", None);
                js(json!({"ok": true}))
            }
            _ => er(StatusCode::BAD_REQUEST, "op"),
        },
        ("POST", "/api/apitok") => match b["op"].as_str().unwrap_or("") {
            "new" => {
                let t = format!("ao_{}", hex::encode(rand::random::<[u8; 24]>()));
                a.db.ks("api", &json!({"h": th(&t), "t0": now()}));
                a.say("scout", "new API token created", None);
                js(json!({"ok": true, "tok": t}))
            }
            "off" => {
                a.db.kd("api");
                a.say("scout", "API token revoked", None);
                js(json!({"ok": true}))
            }
            _ => er(StatusCode::BAD_REQUEST, "op"),
        },
        ("POST", "/api/google") => match b["op"].as_str().unwrap_or("") {
            "link" => {
                if !crate::goog::available() {
                    er(
                        StatusCode::CONFLICT,
                        "Configure APOLLO_OFFICE_GOOGLE_SECRET before linking Google",
                    )
                } else {
                    js(json!({"url": crate::goog::start(a, "link")}))
                }
            }
            "unlink" => {
                a.db.kd("google");
                js(cfg(a))
            }
            _ => er(StatusCode::BAD_REQUEST, "unknown op"),
        },
        ("POST", "/api/star") => match b["op"].as_str().unwrap_or("") {
            "star" => match crate::star::star(a).await {
                Ok(()) => js(state(a)),
                Err(e) => er(StatusCode::BAD_GATEWAY, &e),
            },
            "later" => {
                crate::star::later(a);
                js(state(a))
            }
            _ => er(StatusCode::BAD_REQUEST, "unknown op"),
        },
        ("POST", "/api/update") => match b["op"].as_str().unwrap_or("") {
            "check-main" => match crate::upd::check_main().await {
                Ok(v) => js(json!({"ok": true, "r": v})),
                Err(e) => js(json!({"ok": false, "m": e})),
            },
            "check" => match crate::upd::check(a).await {
                Ok(v) => js(json!({"ok": true, "r": v, "cfg": cfg(a)})),
                Err(e) => js(json!({"ok": false, "m": e, "cfg": cfg(a)})),
            },
            "apply" => {
                if !crate::upd::ready(a) {
                    return er(StatusCode::CONFLICT, "no trusted update downloaded");
                }
                if !crate::upd::manually_paused(a) {
                    return er(
                        StatusCode::CONFLICT,
                        "Select Pause office before installing an update",
                    );
                }
                if !crate::upd::idle(a) {
                    return er(
                        StatusCode::CONFLICT,
                        "Wait for all running jobs and merges to finish",
                    );
                }
                if !crate::upd::apply(a, true) {
                    return er(StatusCode::CONFLICT, "Update interrupted by a running task");
                }
                js(json!({"ok": true}))
            }
            _ => er(StatusCode::BAD_REQUEST, "unknown op"),
        },
        _ => er(StatusCode::NOT_FOUND, "not found"),
    }
}

pub async fn h(State(a): State<Ax>, req: Request<Body>) -> Response<Body> {
    let (mut pt, b) = req.into_parts();
    if pt.uri.path() == "/api/ws" {
        if !host_ok(&a, &pt.headers) || !authed(&a, &pt.headers) {
            return er(StatusCode::UNAUTHORIZED, "login");
        }
        if !origin_ok(&a, &pt.headers) {
            return er(StatusCode::FORBIDDEN, "forbidden");
        }
        return match WebSocketUpgrade::from_request_parts(&mut pt, &a).await {
            Ok(u) => {
                let a2 = a.clone();
                u.on_upgrade(move |s| ws(a2, s)).into_response()
            }
            Err(e) => e.into_response(),
        };
    }
    if pt.uri.path().starts_with("/v1/") {
        let (m, p, q) = (
            pt.method.clone(),
            pt.uri.path().to_string(),
            pt.uri.query().unwrap_or("").to_string(),
        );
        return v1(&a, &m, &p, &q, &pt.headers, b).await;
    }
    let m = pt.method.clone();
    let p = pt.uri.path().to_string();
    let q = pt.uri.query().unwrap_or("").to_string();
    let hd = pt.headers;
    if p == "/int/mcp" && m == Method::POST {
        let t = hd
            .get("x-run")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let Ok(by) = to_bytes(b, 16 << 20).await else {
            return er(StatusCode::BAD_REQUEST, "body");
        };
        let v: Value = serde_json::from_slice(&by).unwrap_or(Value::Null);
        return js(crate::mcp::handle(&a, &t, &v).await);
    }
    if !host_ok(&a, &hd) {
        return er(StatusCode::MISDIRECTED_REQUEST, "use http://127.0.0.1");
    }
    if p == "/auth/google" && m == Method::GET {
        if !crate::goog::available() {
            return page("Google sign-in is disabled until APOLLO_OFFICE_GOOGLE_SECRET is configured. Use <code>apollo-office login-link</code> instead.");
        }
        if crate::goog::linked(&a)["sub"].is_null() {
            return page("No Google account is connected yet. Log in with <code>apollo-office login-link</code>, then connect Google in Settings.");
        }
        return Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header("location", crate::goog::start(&a, "login"))
            .header("cache-control", "no-store")
            .body(Body::empty())
            .unwrap();
    }
    if p == "/auth/google/cb" && m == Method::GET {
        let qv = |k: &str| query_value(&q, k);
        if !qv("error").is_empty() {
            return page("Google sign-in was cancelled. <a href='/'>Back</a>");
        }
        return match crate::goog::back(&a, &qv("code"), &qv("state")).await {
            Ok((md, sub, em)) if md == "link" => {
                crate::goog::link(&a, &sub, &em);
                a.say(
                    "scout",
                    &format!("Google account {em} connected for sign-in"),
                    None,
                );
                rs(StatusCode::OK, "text/html; charset=utf-8", format!("{LP}<meta http-equiv=refresh content='1;url=/'><p>Connected <b>{}</b>. You can now sign in with this Google account.</p></div>", esc(&em)))
            }
            Ok((_, sub, em)) => {
                if !crate::goog::ok(&a, &sub) {
                    return page(&format!("<b>{}</b> is not the Google account connected to this ApolloOffice. <a href='/'>Back</a>", esc(&em)));
                }
                let s = hex::encode(rand::random::<[u8; 32]>());
                a.db.run("delete from ses where exp<?", pa![now()]);
                a.db.run(
                    "insert into ses(id,exp) values(?,?)",
                    pa![s.clone(), now() + 30 * 86_400_000],
                );
                Response::builder().status(StatusCode::OK).header("content-type", "text/html; charset=utf-8").header("cache-control", "no-store").header("set-cookie", format!("ao={s}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}", 30 * 86400)).body(Body::from(format!("{LP}<meta http-equiv=refresh content='0;url=/'><p>Signed in as {}.</p></div>", esc(&em)))).unwrap()
            }
            Err(e) => page(&format!("{} <a href='/'>Back</a>", esc(&e))),
        };
    }
    if p == "/auth/link" {
        let c = q
            .split('&')
            .find_map(|x| x.strip_prefix("c="))
            .unwrap_or("")
            .to_string();
        if !c.chars().all(|x| x.is_ascii_hexdigit()) || c.len() != 32 {
            return page("This login link is not valid. Run <code>apollo-office login-link</code> for a new one.");
        }
        if m == Method::GET {
            return page(&format!("<p>Log in to ApolloOffice on this computer?</p><form method=post action='/auth/link?c={c}'><button>Log in</button></form>"));
        }
        if m == Method::POST && origin_ok(&a, &hd) {
            if a.db
                .ch("delete from lnk where c=? and exp>?", pa![c, now()])
                == 1
            {
                let s = hex::encode(rand::random::<[u8; 32]>());
                a.db.run("delete from ses where exp<?", pa![now()]);
                a.db.run(
                    "insert into ses(id,exp) values(?,?)",
                    pa![s.clone(), now() + 30 * 86_400_000],
                );
                return Response::builder()
                    .status(StatusCode::SEE_OTHER)
                    .header("location", "/")
                    .header(
                        "set-cookie",
                        format!(
                            "ao={s}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
                            30 * 86400
                        ),
                    )
                    .body(Body::empty())
                    .unwrap();
            }
            return page("This login link was already used or has expired. Run <code>apollo-office login-link</code> for a new one.");
        }
        return er(StatusCode::FORBIDDEN, "forbidden");
    }
    if p.starts_with("/api/") {
        if !authed(&a, &hd) {
            return er(StatusCode::UNAUTHORIZED, "login");
        }
        if m != Method::GET && (!origin_ok(&a, &hd) || hd.get("x-ao").is_none()) {
            return er(StatusCode::FORBIDDEN, "forbidden");
        }
        let b: Value = if m == Method::POST {
            match to_bytes(b, 1 << 20).await {
                Ok(by) => serde_json::from_slice(&by).unwrap_or(json!({})),
                Err(_) => return er(StatusCode::BAD_REQUEST, "body"),
            }
        } else {
            Value::Null
        };
        return api(&a, &m, &p, &q, b).await;
    }
    if p == "/api/wait" {
        return er(StatusCode::NOT_FOUND, "");
    }
    let f = if p == "/" {
        "index.html".to_string()
    } else {
        p.trim_start_matches('/').to_string()
    };
    if f == "index.html" && !authed(&a, &hd) {
        let g = if crate::goog::linked(&a)["sub"].is_string() {
            "<p><a href='/auth/google'><button>Sign in with Google</button></a></p><p>or, in a terminal, run</p>"
        } else {
            "<p>Not logged in. In a terminal, run</p>"
        };
        return page(&format!("<p><b>ApolloOffice</b></p>{g}<p><code>apollo-office login-link</code></p><p>and open the link it prints.</p>"));
    }
    match Ui::get(&f) {
        Some(x) => rs(
            StatusCode::OK,
            mime_guess::from_path(&f).first_or_octet_stream().as_ref(),
            x.data.into_owned(),
        ),
        None => er(StatusCode::NOT_FOUND, "not found"),
    }
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn steward_state_reports_full_totals_even_when_pr_preview_is_limited() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into rp(full,on_) values('o/large',1),('o/legacy',1)",
            pa![],
        );
        let large_prs: Vec<Value> = (1..=60)
            .map(|number| json!({"number":number,"state":"draft","priority":8}))
            .collect();
        db.ks(
            "steward:o/large",
            &json!({
                "summary":{
                    "total":112,"blocked":108,"review_requested":1,"ci_failed":2,
                    "draft":4,"overlap":3,
                    "states":{"draft":4,"conflito":1,"duplicado_possivel":2,"sobreposicao":1,
                        "ci_falhou":2,"ci_pendente":80,"revisao_pendente":18,"revisao_final_necessaria":4}
                },
                "prs":large_prs
            }),
        );
        db.ks(
            "steward:o/legacy",
            &json!({"summary":{"total":2},"prs":[{"state":"conflito"},{"state":"draft"}]}),
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));

        let state = pr_steward_state(&a);
        assert_eq!(state["total"], 114);
        assert_eq!(state["blocked"], 110);
        assert_eq!(state["states"]["draft"], 5);
        assert_eq!(state["states"]["ci_pendente"], 80);
        assert_eq!(state["states"]["conflito"], 2);
        assert_eq!(state["prs"].as_array().unwrap().len(), 40);
        assert_eq!(state["repos"][0]["total"], 112);
        assert_eq!(state["repos"][0]["states"]["revisao_pendente"], 18);
    }

    #[test]
    fn state_exposes_staged_route_and_only_checkpoint_summaries() {
        let db = crate::db::Db::mem();
        db.ks(
            "set",
            &json!({
                "staged_tdd": true,
                "staged_repos": ["o/r"],
                "md": {"fixer": "ollama:qwen3:8b", "reviewer": "cc:opus"}
            }),
        );
        db.run(
            "insert into rp(full,on_) values(?,1)",
            pa!["o/r".to_string()],
        );
        db.jnew("o/r", 1, "fix", "bug", "", "fix", "", json!({
            "staged_red": {"stage": "committed", "test_commit": "red-sha", "test_cmd": "private command"},
            "staged_green": {"commit": "green-sha", "js": {"private": "result"}}
        }), None, 0.0);
        db.run(
            "insert into jobs(id,repo,num,kind,title,url,st,t1) values(?,?,?,?,?,?,?,?)",
            pa![
                99,
                "o/r".to_string(),
                2,
                "fix".to_string(),
                "recent done".to_string(),
                "".to_string(),
                "done".to_string(),
                now()
            ],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));

        let snapshot = state(&a);
        assert_eq!(snapshot["tdd"]["active"], true);
        assert_eq!(snapshot["tdd"]["primary"], "ollama:qwen3:8b");
        assert_eq!(snapshot["tdd"]["reviewer"], "cc:opus");
        assert_eq!(snapshot["q"][0]["red_stage"], "committed");
        assert_eq!(snapshot["q"][0]["red_commit"], "red-sha");
        assert_eq!(snapshot["q"][0]["green_commit"], "green-sha");
        assert_eq!(snapshot["rp"][0]["repo"], "o/r");
        assert_eq!(snapshot["done24"], 1);
        assert_eq!(snapshot["done24ByRepo"][0]["n"], 1);
        assert_eq!(snapshot["byRepo"][0]["repo"], "o/r");
        assert!(!snapshot.to_string().contains("private command"));
        assert!(!snapshot.to_string().contains("private result"));
    }

    #[test]
    fn history_cleanup_preserves_active_dependencies_and_rolls_back() {
        let db = crate::db::Db::mem();
        db.0.lock().unwrap().execute_batch(
            "insert into jobs(id,repo,st,pr) values(1,'o/r','done',1),(2,'o/r','ready',null),(3,'o/r','done',3);
             insert into prs(repo,num,st) values('o/r',1,'merged');
             insert into job_dependencies(job,prerequisite) values(2,1);
             insert into job_schedule(job) values(1),(2),(3);
             create trigger fail_history before delete on job_schedule begin select raise(abort,'injected'); end;",
        ).unwrap();
        assert!(clear_job_history(&db).is_err());
        assert_eq!(db.n("select count(*) from jobs", pa![]), 3);
        db.0.lock()
            .unwrap()
            .execute_batch("drop trigger fail_history")
            .unwrap();
        clear_job_history(&db).unwrap();
        assert_eq!(db.n("select count(*) from jobs", pa![]), 2);
        assert_eq!(db.n("select count(*) from job_schedule", pa![]), 2);
        assert!(db.schedule_eligible(2, "o/r", now()).unwrap());
        db.run("update jobs set st='done' where id=2", pa![]);
        clear_job_history(&db).unwrap();
        assert_eq!(db.n("select count(*) from jobs", pa![]), 0);
        assert_eq!(db.n("select count(*) from job_dependencies", pa![]), 0);
        assert_eq!(db.n("select count(*) from job_schedule", pa![]), 0);
    }

    #[test]
    fn repository_removal_is_atomic_and_cancels_parked_jobs() {
        let db = crate::db::Db::mem();
        db.0.lock().unwrap().execute_batch(
            "insert into rp(full) values('o/r'),('other/r');
             insert into jobs(repo,st) values('o/r','ready'),('o/r','park'),('other/r','ready');
             create trigger fail_drop before update of st on jobs begin select raise(abort,'injected'); end;",
        ).unwrap();
        assert!(remove_repository(&db, "o/r").is_err());
        assert_eq!(db.n("select count(*) from rp", pa![]), 2);
        assert_eq!(db.n("select count(*) from jobs where st='drop'", pa![]), 0);
        db.0.lock()
            .unwrap()
            .execute_batch("drop trigger fail_drop")
            .unwrap();
        remove_repository(&db, "o/r").unwrap();
        assert_eq!(db.n("select count(*) from rp where full='o/r'", pa![]), 0);
        assert_eq!(db.n("select count(*) from jobs where st='drop'", pa![]), 2);
        assert_eq!(
            db.n(
                "select count(*) from jobs where repo='other/r' and st='ready'",
                pa![]
            ),
            1
        );
    }

    #[tokio::test]
    async fn settings_reset_preserves_pause_on_failure_and_during_drain() {
        let db = crate::db::Db::mem();
        db.ks("man", &json!(true));
        db.ks("hold", &json!({"fixer":true}));
        db.ks("set", &json!({"hunters":1}));
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let request = || api(&a, &Method::POST, "/api/danger", "", json!({"op":"reset"}));
        a.drain.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(request().await.status(), StatusCode::CONFLICT);
        a.drain.store(false, std::sync::atomic::Ordering::SeqCst);
        a.db.0.lock().unwrap().execute_batch(
            "create trigger fail_reset before delete on kv when old.k='hold' begin select raise(abort,'injected'); end;",
        ).unwrap();
        assert_eq!(request().await.status(), StatusCode::CONFLICT);
        assert!(a.lv.lock().unwrap().o.man);
        assert_eq!(a.db.kg("man"), json!(true));
        assert_eq!(a.db.kg("hold"), json!({"fixer":true}));
        assert_eq!(a.db.kg("set"), json!({"hunters":1}));
        a.db
            .0
            .lock()
            .unwrap()
            .execute_batch("drop trigger fail_reset")
            .unwrap();
        assert_eq!(request().await.status(), StatusCode::OK);
        assert!(!a.lv.lock().unwrap().o.man);
        assert!(!a.lv.lock().unwrap().w.values().any(|w| w.hold));
        assert_eq!(
            a.db.n(
                "select count(*) from kv where k in ('set','hold','man')",
                pa![]
            ),
            0
        );
    }

    #[tokio::test]
    async fn wipe_requires_pause_and_idle_workers() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full) values('o/r')", pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let request = || {
            api(
                &a,
                &Method::POST,
                "/api/danger",
                "",
                json!({"op":"wipe","sure":"delete"}),
            )
        };
        assert_eq!(request().await.status(), StatusCode::CONFLICT);
        a.lv.lock().unwrap().o.man = true;
        a.act.lock().unwrap().insert("o/r local".into());
        assert_eq!(request().await.status(), StatusCode::CONFLICT);
        assert_eq!(a.db.n("select count(*) from rp", pa![]), 1);
        a.act.lock().unwrap().clear();
        assert_eq!(request().await.status(), StatusCode::OK);
        assert_eq!(a.db.n("select count(*) from rp", pa![]), 0);
        assert_eq!(a.db.kg("man"), json!(true));
    }

    #[test]
    fn full_wipe_rolls_back_and_does_not_restore_old_coderabbit_history() {
        let db = crate::db::Db::mem();
        db.0.lock().unwrap().execute_batch(
            "insert into rp(full) values('o/r');
             insert into jobs(id,repo,kind,pr,d) values(7,'o/r','review',1,'{\"coderabbit_source\":\"coderabbit\"}');
             insert into coderabbit_requests(repo,number,head,status,requested_at,activation) values('o/r',1,'old','sent',1,1);
             insert into coderabbit_findings(repo,number,comment_id,review_id,head,state,severity,body_hash,activation,job,created_at,updated_at) values('o/r',1,1,1,'old','queued','high','hash',1,7,1,1);
             insert into job_schedule(job) values(7);
             insert into watcher_outbox(id,repo,num,author,body,state,created,updated) values('old','o/r',1,'owner','old','pending',1,1);
             create trigger fail_wipe before delete on coderabbit_requests begin select raise(abort,'injected'); end;",
        ).unwrap();
        assert!(wipe_office_data(&db).is_err());
        assert_eq!(db.n("select count(*) from jobs", pa![]), 1);
        assert_eq!(db.n("select count(*) from watcher_outbox", pa![]), 1);
        assert_eq!(db.n("select count(*) from coderabbit_cycles", pa![]), 1);
        db.0.lock()
            .unwrap()
            .execute_batch("drop trigger fail_wipe")
            .unwrap();
        wipe_office_data(&db).unwrap();
        db.run("insert into rp(full) values('o/r')", pa![]);
        for table in [
            "jobs",
            "job_schedule",
            "watcher_outbox",
            "coderabbit_requests",
            "coderabbit_findings",
            "coderabbit_cycles",
        ] {
            assert_eq!(db.n(&format!("select count(*) from {table}"), pa![]), 0);
        }
        assert_eq!(db.kg("man"), json!(true));
    }

    #[tokio::test]
    async fn coderabbit_route_reports_busy_without_remote_request() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full) values('o/r')", pa![]);
        let head = "a".repeat(40);
        db.ks(
            "steward:o/r",
            &json!({"prs":[{"number":1,"head_sha":head}]}),
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let _permits = crate::coderabbit::OBSERVATIONS
            .acquire_many(2)
            .await
            .unwrap();
        let response = api(
            &a,
            &Method::POST,
            "/api/steward/coderabbit",
            "",
            json!({"repo":"o/r","number":1,"head":head}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["state"], "busy");
        assert_eq!(value["head"], head);
    }

    #[test]
    fn repository_reactivation_starts_new_review_eligibility_window() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into rp(full,on_,coderabbit_review,coderabbit_free_confirmed,coderabbit_review_since) values('o/r',1,1,1,100)",
            pa![],
        );
        save_repo_settings(&db, "o/r", &json!({"on_":false})).unwrap();
        let row = db.one("select * from rp where full='o/r'", pa![]).unwrap();
        assert_eq!(row["coderabbit_review_since"], 100);
        let before = now();
        save_repo_settings(&db, "o/r", &json!({"on_":true})).unwrap();
        let row = db.one("select * from rp where full='o/r'", pa![]).unwrap();
        assert!(row["coderabbit_review_since"].as_i64().unwrap() >= before);
        save_repo_settings(&db, "o/r", &json!({"on_":true})).unwrap();
        let unchanged = db.one("select * from rp where full='o/r'", pa![]).unwrap();
        assert_eq!(
            row["coderabbit_review_since"],
            unchanged["coderabbit_review_since"]
        );
        assert_eq!(row["coderabbit_epoch"], unchanged["coderabbit_epoch"]);
    }

    #[test]
    fn repo_settings_rollback_preserves_activation_and_confirmation() {
        let db = crate::db::Db::mem();
        db.0.lock().unwrap().execute(
            "insert into rp(full,coderabbit_review,coderabbit_free_confirmed,coderabbit_epoch,coderabbit_review_since) values('o/r',1,1,7,100)",
            [],
        ).unwrap();
        db.0.lock().unwrap().execute_batch(
            "create trigger fail_review before update of coderabbit_review on rp begin select raise(abort,'injected'); end;",
        ).unwrap();
        let changes = json!({"coderabbit_free_confirmed":false,"coderabbit_review":false});
        assert!(save_repo_settings(&db, "o/r", &changes).is_err());
        let state = db.one("select * from rp where full='o/r'", pa![]).unwrap();
        assert_eq!(state["coderabbit_free_confirmed"], 1);
        assert_eq!(state["coderabbit_review"], 1);
        assert_eq!(state["coderabbit_epoch"], 7);
        assert_eq!(state["coderabbit_review_since"], 100);
        db.0.lock()
            .unwrap()
            .execute_batch("drop trigger fail_review")
            .unwrap();
        save_repo_settings(&db, "o/r", &changes).unwrap();
        let state = db.one("select * from rp where full='o/r'", pa![]).unwrap();
        assert_eq!(state["coderabbit_free_confirmed"], 0);
        assert_eq!(state["coderabbit_review"], 0);
        assert_eq!(state["coderabbit_epoch"], 9);
    }

    #[test]
    fn repository_language_and_steward_close_are_saved() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full) values('o/r')", pa![]);
        save_repo_settings(&db, "o/r", &json!({"lang":"pt-PT","steward_close":true})).unwrap();
        let state = db.one("select * from rp where full='o/r'", pa![]).unwrap();
        assert_eq!(state["lang"], "pt-PT");
        assert_eq!(state["steward_close"], 1);
        save_repo_settings(&db, "o/r", &json!({"lang":"invalid","steward_close":false})).unwrap();
        let state = db.one("select * from rp where full='o/r'", pa![]).unwrap();
        assert_eq!(state["lang"], "en");
        assert_eq!(state["steward_close"], 0);
    }

    #[test]
    fn repository_auto_merge_setting_is_configurable_and_exported() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into rp(full) values(?)",
            pa!["owner/repo".to_string()],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db.clone(),
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        assert_eq!(cfg(&a)["repos"][0]["merge"], 1);
        assert_eq!(export(&a)["repos"][0]["merge"], 1);
        db.run(
            "update rp set merge=0 where full=?",
            pa!["owner/repo".to_string()],
        );
        assert_eq!(cfg(&a)["repos"][0]["merge"], 0);
        assert_eq!(export(&a)["repos"][0]["merge"], 0);
    }

    #[test]
    fn report_token_summary_does_not_expose_private_patch_or_review() {
        let db = crate::db::Db::mem();
        let id=db.jnew("o/private",0,"fix","bug","","approve","",json!({"api":1,"df":"PRIVATE_PATCH","proof":{"secret":"PRIVATE_PROOF"},"acceptance_proof":{"ok":true},"last_review":{"verdict":{"ok":true}}}),None,0.0);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let public = repst(&a, id).unwrap();
        assert_eq!(public["st"], "approve");
        assert!(!public.to_string().contains("PRIVATE_"));
        for field in [
            "diff",
            "proof",
            "result",
            "review",
            "lessons",
            "routing",
            "evidence_summary",
        ] {
            assert!(public.get(field).is_none());
        }
        assert_eq!(repdetail(&a, id).unwrap()["diff"], "PRIVATE_PATCH");
    }

    #[test]
    fn public_queue_labels_bug_optimization_and_feature_origins_without_job_data() {
        let db = crate::db::Db::mem();
        let cases = [
            ("fix", 1, json!({}), "issue_bug"),
            ("fix", 0, json!({}), "discovered_bug"),
            ("fix", 2, json!({"opt":true}), "optimization"),
            ("fix", 3, json!({"feat":true}), "feature"),
            (
                "fix",
                0,
                json!({"ext":{"src":"error-tracker"}}),
                "reported_bug",
            ),
            ("hunt", 0, json!({}), "discovered_bug"),
            ("mergecheck", 2, json!({}), "pr_recheck"),
        ];
        let mut ids = Vec::new();
        for (kind, number, details, expected) in cases {
            let id = db.jnew(
                "owner/repo",
                number,
                kind,
                "example work",
                "",
                "ready",
                "",
                details,
                None,
                0.0,
            );
            ids.push((id, expected));
        }
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let public = state(&a);
        for (id, expected) in ids {
            let job = public["q"]
                .as_array()
                .unwrap()
                .iter()
                .find(|job| job["id"] == id)
                .unwrap();
            assert_eq!(job["work_class"], expected);
            assert!(job.get("d").is_none());
            assert!(job.get("ext").is_none());
        }
    }

    #[test]
    fn steward_report_exposes_head_base_and_recheck_diagnostics() {
        let db = crate::db::Db::mem();
        let repo = "owner/repo";
        db.jnew(
            repo,
            7,
            "mergecheck",
            "Revalidar PR",
            "",
            "ready",
            "aguarda worker",
            json!({
                "expected_head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "expected_base":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }),
            Some(42),
            -5.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        a.db.ks(
            &format!("steward:{repo}"),
            &json!({"prs":[{
                "number":42,
                "head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "base_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }]}),
        );
        a.db.ks(
            &format!("steward:update:{repo}#42"),
            &json!({"status":"updated","head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","base":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","at":now()}),
        );
        let finished_at = now() - 30_000;
        let done_id = a.db.jnew(
            repo,
            2,
            "mergecheck",
            "Completed recheck",
            "",
            "done",
            "",
            json!({"expected_head":"cccccccccccccccccccccccccccccccccccccccc","expected_base":"dddddddddddddddddddddddddddddddddddddddd"}),
            Some(43),
            -5.0,
        );
        a.db.run(
            "update jobs set t0=?,t1=? where id=?",
            pa![finished_at - 25_000, finished_at, done_id],
        );
        let drop_id = a.db.jnew(
            repo,
            3,
            "mergecheck",
            "Dropped recheck",
            "",
            "drop",
            "",
            json!({"expected_head":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","expected_base":"ffffffffffffffffffffffffffffffffffffffff"}),
            Some(44),
            -5.0,
        );
        a.db.run(
            "update jobs set t0=?,t1=? where id=?",
            pa![finished_at - 40_000, finished_at, drop_id],
        );
        a.db.ks(
            &format!("steward:{repo}"),
            &json!({"prs":[
                {"number":42,"head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","base_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
                {"number":43,"head_sha":"cccccccccccccccccccccccccccccccccccccccc","base_sha":"dddddddddddddddddddddddddddddddddddddddd"},
                {"number":44,"head_sha":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","base_sha":"ffffffffffffffffffffffffffffffffffffffff"}
            ]}),
        );
        let report = steward_report(&a, repo);
        assert_eq!(report["queue"]["pending_rechecks"], 1);
        assert_eq!(report["prs"][0]["recheck"]["state"], "ready");
        assert_eq!(report["prs"][0]["recheck"]["matches_head"], true);
        assert_eq!(report["prs"][0]["recheck"]["matches_base"], true);
        assert_eq!(report["prs"][0]["branch_update"]["status"], "updated");
        assert_eq!(report["prs"][1]["recheck"]["age_ms"], 25_000);
        assert_eq!(report["prs"][2]["recheck"]["age_ms"], 40_000);
    }

    #[test]
    fn stale_branch_update_errors_do_not_follow_a_new_pr_head() {
        let head = json!("new-head");
        let base = json!("same-base");
        let error = json!({"status":"error","head":"old-head","base":"same-base"});
        let blocked = json!({"status":"blocked","head":"old-head","base":"same-base"});
        let updated = json!({"status":"updated","head":"old-head","base":"same-base"});
        assert!(!branch_update_matches_pr(&error, &head, &base));
        assert!(!branch_update_matches_pr(&blocked, &head, &base));
        assert!(branch_update_matches_pr(&updated, &head, &base));
    }

    #[test]
    fn trackers() {
        let s = json!({"action": "triggered", "data": {"event": {"title": "TypeError: x is undefined", "culprit": "app/main.js in run", "issue_id": "42", "release": "1.2.0", "exception": {"values": [{"type": "TypeError", "value": "x is undefined", "stacktrace": {"frames": [{"function": "lib", "filename": "node_modules/a.js", "lineno": 1, "in_app": false}, {"function": "run", "filename": "app/main.js", "lineno": 7, "in_app": true}]}}]}, "web_url": "https://sentry.io/i/42"}}});
        let r = trk(&s).unwrap();
        assert_eq!(r["fp"], "sentry:42");
        assert_eq!(r["where"], "app/main.js in run");
        assert!(r["logs"]
            .as_str()
            .unwrap()
            .contains("at run (app/main.js:7)"));
        assert!(!r["logs"].as_str().unwrap().contains("node_modules"));
        assert!(r["body"].as_str().unwrap().contains("Release: 1.2.0"));
        let i = json!({"action": "created", "data": {"issue": {"id": "9", "title": "ZeroDivisionError", "culprit": "calc.py", "permalink": "https://x/9", "metadata": {"type": "ZeroDivisionError", "value": "division by zero", "function": "div", "filename": "calc.py"}}}});
        let r = trk(&i).unwrap();
        assert_eq!(r["fp"], "sentry:9");
        assert!(r["logs"].as_str().unwrap().contains("division by zero"));
        let g = json!({"text": "GlitchTip Alert", "attachments": [{"title": "KeyError: 'id'", "title_link": "https://gt/issues/5", "text": "views.get", "fields": [{"title": "Project", "value": "web"}]}]});
        let r = trk(&g).unwrap();
        assert_eq!(r["src"], "GlitchTip");
        assert_eq!(r["fp"], "gt:https://gt/issues/5");
        assert!(trk(&json!({"data": {}})).is_none());
    }

    #[test]
    fn hint() {
        let r = |md: &str, jobs: i64, mg: i64, usd: f64| json!({"repo": "o/r", "w": "fixer", "md": md, "jobs": jobs, "mg": mg, "usd": usd});
        let h = hints(&[
            r("anthropic:claude-opus-5-5", 6, 3, 6.0),
            r("anthropic:claude-sonnet-5-5", 6, 3, 1.5),
        ]);
        assert_eq!(h.len(), 1);
        assert!(
            h[0].contains("Claude Sonnet 5.5") && h[0].contains("4×"),
            "{}",
            h[0]
        );
        assert!(hints(&[
            r("anthropic:claude-opus-5-5", 6, 5, 6.0),
            r("anthropic:claude-sonnet-5-5", 6, 2, 1.5)
        ])
        .is_empty());
        assert!(hints(&[
            r("anthropic:claude-opus-5-5", 6, 3, 6.0),
            r("anthropic:claude-sonnet-5-5", 4, 3, 1.0)
        ])
        .is_empty());
    }

    #[test]
    fn dedup() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full,on_) values('o/r',1)", pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let b = json!({"repo": "o/r", "title": "t", "body": "b", "fp": "sentry:1"});
        let x = rep(&a, &b).unwrap();
        let y = rep(&a, &b).unwrap();
        assert_eq!(x["id"], y["id"]);
        assert_eq!(y["new"], false);
        assert_eq!(a.db.jg(x["id"].as_i64().unwrap()).unwrap().d["ext"]["n"], 2);
        let z = rep(
            &a,
            &json!({"repo": "o/r", "title": "t", "body": "b", "fp": "sentry:2"}),
        )
        .unwrap();
        assert_ne!(x["id"], z["id"]);
    }

    #[test]
    fn import_skips_non_string_pull_request_language() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            47302,
            "apollo-office".into(),
        ));
        import(
            &a,
            &json!({
                "apollo_office": "0.10.0",
                "set": {},
                "repos": [{"full": "owner/repo", "lang": 1}],
                "hosts": []
            }),
        )
        .unwrap();
        let row =
            a.db.one(
                "select lang from rp where full=?",
                pa!["owner/repo".to_string()],
            )
            .unwrap();
        assert_eq!(row["lang"], "en");
    }

    #[test]
    fn pull_request_language_has_a_strict_fallback() {
        assert_eq!(pr_lang("pt-PT"), "pt-PT");
        for invalid in ["", "pt", "pt-BR", "en-US", "../pt-PT"] {
            assert_eq!(pr_lang(invalid), "en");
        }
    }

    #[test]
    fn lesson_metrics_distinguish_missing_exposure_and_completed_first_attempts() {
        let db = crate::db::Db::mem();
        let at = now();
        for (id, outcome, gate) in [
            (1, "accepted", ""),
            (2, "rejected", "review"),
            (3, "failed", "verification"),
            (4, "accepted", ""),
        ] {
            let job = db.jnew(
                "o/r",
                id,
                "fix",
                "test",
                "",
                "done",
                "",
                json!({}),
                None,
                0.0,
            );
            db.run(
                "insert into fix_attempts(id,job,attempt,model,started,finished,outcome,gate) values(?,?,?,?,?,?,?,?)",
                pa![id,job,if id==4 {2} else {1}, "cc:sonnet".to_string(),at,at+1,outcome.to_string(),gate.to_string()],
            );
            if id != 3 {
                db.ks(
                    &format!("lesson_exposure:{id}:{job}:{at}"),
                    &json!({"included":id==1}),
                );
            }
        }
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            47304,
            "apollo-office".into(),
        ));
        let report = costs(&a);
        let rows = report["lesson_exposure_metrics"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["exposure"], "included");
        assert_eq!(rows[0]["accepted"], 1);
        assert_eq!(rows[1]["exposure"], "not_included");
        assert_eq!(rows[1]["review_rejections"], 1);
        assert_eq!(rows[2]["exposure"], "unknown");
        assert_eq!(rows[2]["verification_failures"], 1);
        assert!(rows.iter().all(|r| r["samples"] == 1));
    }

    #[test]
    fn only_prs_merged_into_the_default_branch_count_as_integrated_fixes() {
        let db = crate::db::Db::mem();
        let make_fix = |num| {
            db.jnew(
                "o/r",
                num,
                "fix",
                "correction",
                "",
                "done",
                "",
                json!({}),
                None,
                0.0,
            )
        };
        let open = make_fix(42);
        let release_branch = make_fix(43);
        let main = make_fix(44);
        db.jset(open, &[("pr", json!(200))]);
        db.jset(release_branch, &[("pr", json!(201))]);
        db.jset(main, &[("pr", json!(202))]);
        db.run(
            "insert into prs(repo,num,st,title,t0,d) values('o/r',200,'open','Open PR',1,'{\"base\":\"main\",\"default_branch\":\"main\"}'),('o/r',201,'merged','Release PR',1,'{\"base\":\"release/1.0\",\"default_branch\":\"main\"}'),('o/r',202,'merged','Main PR',1,'{\"base\":\"main\",\"default_branch\":\"main\"}')",
            pa![],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            47303,
            "apollo-office".into(),
        ));
        let observed = state(&a);
        assert_eq!(observed["delivery"]["published_fixes"], 3);
        assert_eq!(observed["delivery"]["open_fixes"], 1);
        assert_eq!(observed["delivery"]["merged_fixes"], 1);
    }

    #[test]
    fn google_callback_decodes_encoded_authorization_code() {
        assert_eq!(query_value("code=a%2Fb%2Bc%3D&state=123", "code"), "a/b+c=");
        assert_eq!(query_value("code=a+b&state=x%252Fy", "code"), "a b");
        assert_eq!(query_value("code=a&code=b&state=x", "code"), "a");
        assert_eq!(query_value("error=access_denied", "code"), "");
        assert_eq!(
            query_value("other_code=malicious&code=safe", "code"),
            "safe"
        );
        assert_eq!(query_value("state=s%2Bt%3D", "state"), "s+t=");
    }
}
