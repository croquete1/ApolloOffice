//! Diagnóstico e recuperação limitada de trabalhos abandonados.
//! Nunca reutiliza aprovações ou provas de uma tentativa anterior.
use crate::app::Ax;
use crate::db::{now, Job};
use crate::pa;
use crate::sh::ghj;
use serde_json::{json, Value};

const MAX_AUTO_ATTEMPTS: u64 = 2;
const RETRY_DELAY_MS: i64 = 5 * 60 * 1000;
const MAX_AUTO_AGE_MS: i64 = 48 * 60 * 60 * 1000;

fn category(reason: &str) -> &'static str {
    let s = reason.to_ascii_lowercase();
    if s.contains("dropped by the owner") || s.contains("repository removed") {
        "deliberado"
    } else if s.starts_with("guard") || s.starts_with("vet") {
        "triagem"
    } else if s.starts_with("machine kept failing") || crate::app::netish(reason) {
        "transitorio"
    } else {
        "investigar"
    }
}

fn published_or_approved(j: &Job) -> bool {
    j.pr.is_some()
        || j.d["published_sha"].is_string()
        || j.d["ap"] == true
        || j.d["sq"] == true
}

fn attempts(j: &Job) -> u64 {
    j.d["recovery_attempts"].as_u64().unwrap_or(0)
}

fn eligible(j: &Job, automatic: bool) -> bool {
    if j.st != "drop" || j.kind != "fix" || published_or_approved(j) {
        return false;
    }
    if !automatic {
        return true;
    }
    category(&j.why) == "transitorio"
        && j.num > 0
        && attempts(j) < MAX_AUTO_ATTEMPTS
        && !j.d["br"].is_string()
        && j.d["rs"] != true
        && j.d["rp"] != true
}

fn open_issue(issue: &Value, number: i64) -> bool {
    number > 0
        && issue["number"] == number
        && issue["state"] == "open"
        && issue["pull_request"].is_null()
}

fn has_related_pr(prs: &[Value], number: i64) -> bool {
    let issue = number.to_string();
    prs.iter()
        .any(|pr| crate::steward::issue_refs(pr).contains(&issue))
}

pub async fn retry_manual(a: &Ax, id: i64) -> Result<bool, String> {
    let job = a.db.jg(id).ok_or("Trabalho inexistente")?;
    if !eligible(&job, false) {
        return Err("Drop não elegível para recuperação segura".into());
    }
    if job.num > 0 {
        let issue_path = format!("repos/{}/issues/{}", job.repo, job.num);
        let issue = ghj(&["api", &issue_path], 40).await?;
        if !open_issue(&issue, job.num) {
            return Err("Issue já não está aberta no GitHub".into());
        }
        let prs = ghj(
            &[
                "pr",
                "list",
                "-R",
                &job.repo,
                "--state",
                "open",
                "--limit",
                "200",
                "--json",
                "number,title,body",
            ],
            90,
        )
        .await?;
        let list = prs
            .as_array()
            .ok_or("Resposta de PRs inválida")?;
        if list.len() >= 200 {
            return Err("Lista de PRs truncada; recuperação bloqueada".into());
        }
        if has_related_pr(list, job.num) {
            return Err("Já existe uma PR aberta para esta issue".into());
        }
    }
    retry(a, id, false).map_err(str::to_string)
}

/// Nova tentativa desde o início, com limpeza de provas e autorizações antigas.
/// A escrita condicional impede duas recuperações do mesmo drop em simultâneo.
pub fn retry(a: &Ax, id: i64, automatic: bool) -> Result<bool, &'static str> {
    let j = a.db.jg(id).ok_or("Trabalho inexistente")?;
    if !eligible(&j, automatic) {
        return Err("Drop não elegível para recuperação segura");
    }
    if a.hold("watcher") || a.drain.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Apollo Office em pausa ou manutenção");
    }
    let repo = a.db.one(
        "select on_,ci,auto_recover_drops from rp where full=?",
        pa![j.repo.clone()],
    ).ok_or("Repositório inexistente")?;
    if repo["on_"] != 1 || repo["ci"].as_str().unwrap_or("").trim().is_empty() {
        return Err("Repositório inativo ou sem comando de testes");
    }
    if automatic && repo["auto_recover_drops"] != 1 {
        return Err("Recuperação automática desativada");
    }
    if j.num > 0
        && a.db.n(
            "select count(*) from jobs where repo=? and num=? and kind='fix' and id<>? and (st<>'drop' or pr is not null)",
            pa![j.repo.clone(), j.num, id],
        ) != 0
    {
        return Err("Já existe outro trabalho ativo para a mesma issue");
    }
    let updated = a.db.one("select t1 from jobs where id=?", pa![id])
        .and_then(|v| v["t1"].as_i64())
        .ok_or("Data de estado em falta")?;
    let age = now().saturating_sub(updated);
    if automatic && age < RETRY_DELAY_MS {
        return Err("Aguarda intervalo de segurança");
    }
    if automatic && age > MAX_AUTO_AGE_MS {
        return Err("Drop antigo; requer avaliação manual");
    }
    let mut data = j.d.clone();
    let mut history = data["drop_history"].as_array().cloned().unwrap_or_default();
    if history.len() >= 8 {
        history.remove(0);
    }
    history.push(json!({"at":now(),"reason":j.why.clone(),"automatic":automatic}));
    data["drop_history"] = json!(history);
    if automatic {
        data["recovery_attempts"] = json!(attempts(&j) + 1);
    }
    if let Some(map) = data.as_object_mut() {
        for key in [
            "h", "rs", "rp", "br", "f", "f0", "sq", "known", "vt", "ap",
            "base", "pbase", "pt", "pb", "pb_footer", "df", "proof",
            "published_sha", "publish_tree", "ci_tree", "ci_evidence",
            "ci_verified_at", "last_review", "last_diagnosis", "fx",
            "ir", "ie", "acceptance", "acceptance_proof", "verification_feedback",
        ] {
            map.remove(key);
        }
    }
    let next_state = if !automatic
        && (j.why.starts_with("Guard") || j.why.starts_with("Vet"))
    {
        "vet"
    } else {
        "ready"
    };
    let message = if automatic {
        format!("Recuperação automática segura ({}/{MAX_AUTO_ATTEMPTS})", attempts(&j) + 1)
    } else {
        "Recolocado na fila pelo proprietário".to_string()
    };
    let changed = a.db.ch(
        "update jobs set st=?,why=?,d=?,t1=? where id=? and st='drop' and pr is null",
        pa![next_state.to_string(), message.clone(), data.to_string(), now(), id],
    );
    if changed != 1 {
        return Ok(false);
    }
    a.db.kd(&format!("kill:{id}"));
    a.say("watcher", &format!("{}: {message}", j.tag()), Some(id));
    a.sw.notify_one();
    Ok(true)
}

/// Corre apenas quando o proprietário ativa a opção no repositório.
/// Drops deliberados, com publicação, alterações de código ou sem provas
/// de segurança ficam intactos para análise manual.
pub async fn scan(a: &Ax, repo: &str, prs: &[Value]) {
    if a.hold("watcher") || a.drain.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if a.db.n(
        "select count(*) from rp where full=? and on_=1 and auto_recover_drops=1",
        pa![repo.to_string()],
    ) != 1 {
        return;
    }
    let dropped = a.db.all(
        "select id,t1 from jobs where repo=? and st='drop' and kind='fix' order by t1 desc limit 100",
        pa![repo.to_string()],
    );
    let mut restarted = 0_u8;
    for item in dropped {
        if restarted >= 2 {
            break;
        }
        if a.hold("watcher") {
            break;
        }
        if let Some(id) = item["id"].as_i64() {
            let Some(job) = a.db.jg(id) else {
                continue;
            };
            let age = now().saturating_sub(item["t1"].as_i64().unwrap_or(0));
            if !eligible(&job, true)
                || !(RETRY_DELAY_MS..=MAX_AUTO_AGE_MS).contains(&age)
                || has_related_pr(prs, job.num)
            {
                continue;
            }
            // A issue tem de continuar aberta no GitHub. Erros de leitura
            // bloqueiam a recuperação, sem qualquer escrita ou requeue.
            let path = format!("repos/{repo}/issues/{}", job.num);
            let issue = match ghj(&["api", &path], 40).await {
                Ok(value) => value,
                Err(_) => continue,
            };
            if !open_issue(&issue, job.num) {
                continue;
            }
            if retry(a, id, true) == Ok(true) {
                restarted += 1;
            }
        }
    }
}

pub fn report(a: &Ax, repo: &str) -> Value {
    let enabled = a.db.n(
        "select count(*) from rp where full=? and on_=1 and auto_recover_drops=1",
        pa![repo.to_string()],
    ) == 1;
    let total = a.db.n(
        "select count(*) from jobs where repo=? and st='drop'",
        pa![repo.to_string()],
    );
    let rows: Vec<Value> = a.db.all(
        "select * from jobs where repo=? and st='drop' order by t1 desc limit 25",
        pa![repo.to_string()],
    ).into_iter().map(|row| {
        let j = Job::of(row);
        let cause = category(&j.why);
        let can_retry = eligible(&j, true);
        let used = attempts(&j);
        json!({
            "id": j.id,
            "title": j.title,
            "reason": j.why,
            "category": cause,
            "safe_auto": can_retry,
            "attempts": used,
        })
    }).collect();
    json!({"enabled":enabled,"total":total,"jobs":rows})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::db::Db;
    use std::sync::Arc;

    fn setup() -> Ax {
        let db = Db::mem();
        db.run(
            "insert into rp(full,ci,on_,auto_recover_drops) values(?,?,1,1)",
            pa!["org/repo".to_string(), "cargo test".to_string()],
        );
        Arc::new(App::new(
            db, std::env::temp_dir(), 47300, "apollo-office".into(),
        ))
    }

    fn dropped(a: &Ax, reason: &str, data: Value) -> i64 {
        let id = a.db.jnew(
            "org/repo", 42, "fix", "Corrigir problema", "", "drop", reason, data, None, 0.0,
        );
        a.db.run(
            "update jobs set t1=? where id=?",
            pa![now() - RETRY_DELAY_MS - 1, id],
        );
        id
    }

    #[test]
    fn automatic_recovery_is_bounded_and_clears_stale_evidence() {
        let a = setup();
        let id = dropped(
            &a, "machine kept failing: timed out",
            json!({"h":"host","last_review":{"verdict":{"ok":true}},"ci_evidence":{"exit":0},"ir":5}),
        );
        assert_eq!(retry(&a, id, true), Ok(true));
        let j = a.db.jg(id).unwrap();
        assert_eq!(j.st, "ready");
        assert_eq!(j.d["recovery_attempts"], 1);
        assert!(j.d["last_review"].is_null());
        assert!(j.d["ci_evidence"].is_null());
        assert!(j.d["ir"].is_null());
        assert_eq!(j.d["drop_history"][0]["automatic"], true);
        assert_eq!(retry(&a, id, true), Err("Drop não elegível para recuperação segura"));
    }

    #[test]
    fn rejected_owner_or_guard_drops_are_never_automatically_retried() {
        let a = setup();
        for reason in ["dropped by the owner", "Guard rejected unsafe request", "Vet rejected"] {
            let id = dropped(&a, reason, json!({}));
            assert_eq!(retry(&a, id, true), Err("Drop não elegível para recuperação segura"));
        }
    }

    #[test]
    fn manual_retry_does_not_reuse_approval_or_published_work() {
        let a = setup();
        for data in [json!({"ap":true}),json!({"published_sha":"abcdef"}),json!({"sq":true})] {
            let id = dropped(&a, "error: failed", data);
            assert_eq!(retry(&a, id, false), Err("Drop não elegível para recuperação segura"));
        }
        let id = dropped(&a, "Guard rejected", json!({"br":"scratch"}));
        assert_eq!(retry(&a, id, false), Ok(true));
        let j = a.db.jg(id).unwrap();
        assert_eq!(j.st, "vet");
        assert!(j.d["br"].is_null());
    }

    #[test]
    fn no_auto_retry_without_explicit_opt_in_or_after_limit() {
        let a = setup();
        let id = dropped(&a, "machine kept failing: connection reset", json!({"recovery_attempts":2}));
        assert_eq!(retry(&a, id, true), Err("Drop não elegível para recuperação segura"));
        let id = dropped(&a, "machine kept failing: connection reset", json!({}));
        a.db.run("update rp set auto_recover_drops=0", pa![]);
        assert_eq!(retry(&a, id, true), Err("Recuperação automática desativada"));
    }

    #[test]
    fn github_issue_validation_and_pr_duplicates_are_fail_closed() {
        assert!(open_issue(&json!({"number":42,"state":"open"}), 42));
        assert!(!open_issue(&json!({"number":42,"state":"closed"}), 42));
        assert!(!open_issue(&json!({"number":41,"state":"open"}), 42));
        assert!(!open_issue(&json!({"number":42,"state":"open","pull_request":{}}), 42));
        assert!(!open_issue(&json!({}), 42));
        let pr = json!({"body":"Fixes #42","number":123});
        assert!(has_related_pr(&[pr], 42));
        assert!(!has_related_pr(&[json!({"body":"Fixes #99"})], 42));
    }

    #[test]
    fn recovery_waits_and_does_not_rerun_pending_duplicate_work() {
        let a = setup();
        let id = dropped(&a, "machine kept failing: connection reset", json!({}));
        a.db.run("update jobs set t1=? where id=?", pa![now(),id]);
        assert_eq!(retry(&a, id, true), Err("Aguarda intervalo de segurança"));
        a.db.run("update jobs set t1=? where id=?", pa![now()-RETRY_DELAY_MS-1,id]);
        a.db.jnew("org/repo",42,"fix","Outra tentativa","","ready","",json!({}),None,0.0);
        assert_eq!(retry(&a, id, true), Err("Já existe outro trabalho ativo para a mesma issue"));
    }
}
