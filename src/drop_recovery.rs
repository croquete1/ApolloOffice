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
    j.pr.is_some() || j.d["published_sha"].is_string() || j.d["ap"] == true || j.d["sq"] == true
}

fn attempts(j: &Job) -> Option<u64> {
    match j.d.as_object()?.get("recovery_attempts") {
        None => Some(0),
        Some(value) => value.as_u64(),
    }
}

fn eligible(j: &Job, automatic: bool) -> bool {
    if j.st != "drop" || j.kind != "fix" || !j.d.is_object() || published_or_approved(j) {
        return false;
    }
    if !automatic {
        return true;
    }
    category(&j.why) == "transitorio"
        && j.num > 0
        && attempts(j).is_some_and(|n| n < MAX_AUTO_ATTEMPTS)
        && !j.d["br"].is_string()
        && j.d["rs"] != true
        && j.d["rp"] != true
}

fn recovery_job(a: &Ax, id: i64) -> Option<(Job, i64)> {
    let rows =
        a.db.try_all(
            "select * from jobs where id=? and not exists(select 1 from kv where k=?)",
            pa![id, format!("submit_publication:{id}")],
        )
        .ok()?;
    let row = rows.into_iter().next()?;
    let updated = row["t1"].as_i64()?;
    row["d"].is_object().then(|| (Job::of(row), updated))
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

/// Atualiza a cache do HEAD remoto sem apagar marcadores CI válidos.
fn cache_default_head(a: &Ax, repo: &str, head: &Value) -> Result<(), String> {
    let sha = head["s"].as_str().ok_or("HEAD remoto sem SHA")?;
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("HEAD remoto com SHA inválido".into());
    }
    crate::gh::cs(a, &format!("head:{repo}"), head);
    Ok(())
}

async fn recovery_remote(args: Vec<String>, timeout: u64) -> Result<Value, String> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    ghj(&refs, timeout).await
}

/// Só recoloca uma issue após confirmar o estado do GitHub e o HEAD real.
pub async fn retry_manual(a: &Ax, id: i64) -> Result<bool, String> {
    retry_manual_with(a, id, recovery_remote).await
}

async fn retry_manual_with<F, Fut>(a: &Ax, id: i64, mut remote: F) -> Result<bool, String>
where
    F: FnMut(Vec<String>, u64) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let (job, updated) =
        recovery_job(a, id).ok_or("Trabalho inexistente ou metadados inválidos")?;
    if !eligible(&job, false) {
        return Err("Drop não elegível para recuperação segura".into());
    }
    if job.num > 0 {
        let issue_path = format!("repos/{}/issues/{}", job.repo, job.num);
        let issue = remote(vec!["api".into(), issue_path], 40).await?;
        if !open_issue(&issue, job.num) {
            return Err("Issue já não está aberta no GitHub".into());
        }
        let prs = remote(
            [
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
            ]
            .iter()
            .map(|arg| arg.to_string())
            .collect(),
            90,
        )
        .await?;
        let list = prs.as_array().ok_or("Resposta de PRs inválida")?;
        if list.len() >= 200 {
            return Err("Lista de PRs truncada; recuperação bloqueada".into());
        }
        if has_related_pr(list, job.num) {
            return Err("Já existe uma PR aberta para esta issue".into());
        }
    }
    // Evita que uma cache anterior bloqueie uma nova tentativa depois de a
    // branch principal mudar. Mantém as provas cired que ainda correspondem.
    let path = format!("repos/{}/commits/HEAD", job.repo);
    let head = remote(vec!["api".into(), path, "--jq".into(), "{s: .sha}".into()], 60).await?;
    cache_default_head(a, &job.repo, &head)?;
    retry_snapshot(a, &job, updated, false).map_err(|error| error.to_string())
}

/// Nova tentativa desde o início, com limpeza de provas e autorizações antigas.
/// A escrita condicional impede duas recuperações do mesmo drop em simultâneo.
#[cfg(test)]
fn retry(a: &Ax, id: i64, automatic: bool) -> Result<bool, &'static str> {
    let (job, updated) =
        recovery_job(a, id).ok_or("Trabalho inexistente ou metadados inválidos")?;
    retry_snapshot(a, &job, updated, automatic)
}

fn retry_snapshot(a: &Ax, j: &Job, updated: i64, automatic: bool) -> Result<bool, &'static str> {
    let id = j.id;
    if !eligible(j, automatic) {
        return Err("Drop não elegível para recuperação segura");
    }
    if (automatic && a.hold("watcher")) || a.drain.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Apollo Office em pausa ou manutenção");
    }
    let repo =
        a.db.one(
            "select on_,ci,auto_recover_drops from rp where full=?",
            pa![j.repo.clone()],
        )
        .ok_or("Repositório inexistente")?;
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
        history = history.split_off(history.len() - 7);
    }
    history.push(json!({"at":now(),"reason":j.why.clone(),"automatic":automatic}));
    data["drop_history"] = json!(history);
    let next_attempt = if automatic {
        let n = attempts(j).ok_or("Contador de recuperação inválido")? + 1;
        data["recovery_attempts"] = json!(n);
        Some(n)
    } else {
        None
    };
    if let Some(map) = data.as_object_mut() {
        for key in [
            "h",
            "rs",
            "rp",
            "br",
            "f",
            "f0",
            "sq",
            "known",
            "vt",
            "ap",
            "base",
            "pbase",
            "pt",
            "pb",
            "pb_footer",
            "df",
            "proof",
            "published_sha",
            "publish_tree",
            "ci_tree",
            "ci_evidence",
            "ci_verified_at",
            "failure",
            "last_review",
            "last_diagnosis",
            "fx",
            "ir",
            "ie",
            "acceptance",
            "acceptance_proof",
            "verification_feedback",
        ] {
            map.remove(key);
        }
    }
    let next_state = if !automatic && (j.why.starts_with("Guard") || j.why.starts_with("Vet")) {
        "vet"
    } else {
        "ready"
    };
    let message = if let Some(n) = next_attempt {
        format!("Recuperação automática segura ({n}/{MAX_AUTO_ATTEMPTS})")
    } else {
        "Recolocado na fila pelo proprietário".to_string()
    };
    // Lock the policy and the exact dropped job snapshot together. A newer
    // cancellation, proof, retry or repository change cannot reuse this decision.
    let changed = commit_retry(a, j, updated, automatic, next_state, &message, &data)
        .map_err(|_| "Falha ao guardar recuperação na base de dados")?;
    if !changed {
        return Ok(false);
    }
    a.say("watcher", &format!("{}: {message}", j.tag()), Some(id));
    a.sw.notify_one();
    Ok(true)
}

fn commit_retry(
    a: &Ax,
    job: &Job,
    updated: i64,
    automatic: bool,
    next_state: &str,
    message: &str,
    data: &Value,
) -> rusqlite::Result<bool> {
    let mut connection = a.db.0.lock().unwrap();
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "update jobs set st=?1,why=?2,d=?3,t1=?4
         where id=?5 and st='drop' and kind='fix' and pr is null
           and t1=?6 and why=?7
           and repo=?11 and num=?12 and t0=?13 and title=?14 and url=?15
           and not exists(select 1 from kv where k='submit_publication:' || jobs.id)
           and case when typeof(jobs.d)='text' and json_valid(jobs.d)
                    then json_type(jobs.d)='object' else 0 end
           and not exists(select fullkey,type,atom from json_tree(jobs.d)
             except select fullkey,type,atom from json_tree(?8))
           and not exists(select fullkey,type,atom from json_tree(?8)
             except select fullkey,type,atom from json_tree(jobs.d))
           and exists(select 1 from rp where full=jobs.repo and on_=1
             and trim(ci)<>'' and (?9=0 or auto_recover_drops=1))
           and (?9=0 or not exists(select 1 from kv where k=?10 and v<>'false'))
           and (jobs.num<=0 or not exists (
             select 1 from jobs as other
             where other.repo=jobs.repo and other.num=jobs.num
               and other.kind='fix' and other.id<>jobs.id
               and (other.st<>'drop' or other.pr is not null)
           ))",
        rusqlite::params![
            next_state,
            message,
            data.to_string(),
            now(),
            job.id,
            updated,
            job.why,
            job.d.to_string(),
            automatic,
            format!("kill:{}", job.id),
            job.repo,
            job.num,
            job.t0,
            job.title,
            job.url
        ],
    )?;
    if changed != 1 {
        return Ok(false);
    }
    // Only an explicit manual retry can clear a previous owner cancellation.
    if !automatic {
        tx.execute("delete from kv where k=?", [format!("kill:{}", job.id)])?;
    }
    tx.commit()?;
    crate::app::chg();
    Ok(true)
}

/// Corre apenas quando o proprietário ativa a opção no repositório.
/// Drops deliberados, com publicação, alterações de código ou sem provas
/// de segurança ficam intactos para análise manual.
pub async fn scan(a: &Ax, repo: &str, prs: &[Value]) {
    scan_with(a, repo, prs, recovery_remote).await;
}

async fn scan_with<F, Fut>(a: &Ax, repo: &str, prs: &[Value], mut remote: F)
where
    F: FnMut(Vec<String>, u64) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    if a.hold("watcher") || a.drain.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if a.db.n(
        "select count(*) from rp where full=? and on_=1 and auto_recover_drops=1",
        pa![repo.to_string()],
    ) != 1
    {
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
            let Some((job, updated)) = recovery_job(a, id) else {
                continue;
            };
            let age = now().saturating_sub(updated);
            if job.repo != repo
                || !eligible(&job, true)
                || !(RETRY_DELAY_MS..=MAX_AUTO_AGE_MS).contains(&age)
                || has_related_pr(prs, job.num)
            {
                continue;
            }
            // A issue tem de continuar aberta no GitHub. Erros de leitura
            // bloqueiam a recuperação, sem qualquer escrita ou requeue.
            let path = format!("repos/{repo}/issues/{}", job.num);
            let issue = match remote(vec!["api".into(), path], 40).await {
                Ok(value) => value,
                Err(_) => continue,
            };
            if !open_issue(&issue, job.num) {
                continue;
            }
            if retry_snapshot(a, &job, updated, true) == Ok(true) {
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
    let rows: Vec<Value> =
        a.db.all(
            "select *, case when typeof(d)='text' and json_valid(d) then json_type(d)='object' else 0 end as valid_metadata, exists(select 1 from kv where k='submit_publication:' || jobs.id) as publication_pending from jobs where repo=? and st='drop' order by t1 desc limit 25",
            pa![repo.to_string()],
        )
        .into_iter()
        .map(|row| {
            let valid_metadata = row["valid_metadata"] == 1;
            let publication_pending = row["publication_pending"] == 1;
            let j = Job::of(row);
            let cause = category(&j.why);
            let can_retry = valid_metadata && !publication_pending && eligible(&j, true);
            let used = if valid_metadata { attempts(&j) } else { None };
            json!({
                "id": j.id,
                "title": j.title,
                "reason": j.why,
                "category": cause,
                "safe_auto": can_retry,
                "attempts": used,
            })
        })
        .collect();
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
            db,
            std::env::temp_dir(),
            47300,
            "apollo-office".into(),
        ))
    }

    fn dropped(a: &Ax, reason: &str, data: Value) -> i64 {
        let id = a.db.jnew(
            "org/repo",
            42,
            "fix",
            "Corrigir problema",
            "",
            "drop",
            reason,
            data,
            None,
            0.0,
        );
        a.db.run(
            "update jobs set t1=? where id=?",
            pa![now() - RETRY_DELAY_MS - 1, id],
        );
        id
    }

    #[test]
    fn durable_publication_intent_blocks_drop_recovery() {
        let a = setup();
        let id = dropped(&a, "machine kept failing: timed out", json!({}));
        let key = format!("submit_publication:{id}");
        a.db.ks(&key, &json!({"number":42}));
        assert!(retry(&a, id, false).is_err());
        assert!(retry(&a, id, true).is_err());
        assert_eq!(report(&a, "org/repo")["jobs"][0]["safe_auto"], false);
        assert_eq!(a.db.kg(&key)["number"], 42);
        assert_eq!(a.db.jg(id).unwrap().st, "drop");
    }

    #[test]
    fn recovery_rejects_nonobject_metadata() {
        let a = setup();
        let id = dropped(&a, "machine kept failing: timed out", json!({}));
        for raw in ["null", "[]", "\"bad\"", "1", "{invalid"] {
            a.db.run("update jobs set d=? where id=?", pa![raw.to_string(), id]);
            assert!(recovery_job(&a, id).is_none());
            assert!(retry(&a, id, false).is_err());
            assert!(retry(&a, id, true).is_err());
            assert_eq!(a.db.jg(id).unwrap().st, "drop");
            assert_eq!(report(&a, "org/repo")["jobs"][0]["safe_auto"], false);
        }
    }

    #[test]
    fn recovery_trims_oversized_legacy_history() {
        let a = setup();
        let history: Vec<_> = (0..20).map(|i| json!({"at":i})).collect();
        let id = dropped(
            &a,
            "machine kept failing: timed out",
            json!({"drop_history":history}),
        );
        assert_eq!(retry(&a, id, true), Ok(true));
        let job = a.db.jg(id).unwrap();
        let history = job.d["drop_history"].as_array().unwrap();
        assert_eq!(history.len(), 8);
        assert_eq!(history[0]["at"], 13);
        assert_eq!(history[7]["automatic"], true);
    }

    #[test]
    fn malformed_retry_counter_never_resets_automatic_budget() {
        for counter in [json!(-1), json!(null), json!(0.5), json!("0")] {
            let a = setup();
            let id = dropped(
                &a,
                "machine kept failing: timed out",
                json!({"recovery_attempts":counter}),
            );
            assert!(retry(&a, id, true).is_err());
            assert_eq!(a.db.jg(id).unwrap().st, "drop");
            assert!(attempts(&a.db.jg(id).unwrap()).is_none());
        }
    }

    #[test]
    fn automatic_recovery_is_bounded_and_clears_stale_evidence() {
        let a = setup();
        let id = dropped(
            &a,
            "machine kept failing: timed out",
            json!({"h":"host","last_review":{"verdict":{"ok":true}},"ci_evidence":{"exit":0},"failure":{"stage":"ci","exit_code":1},"ir":5}),
        );
        assert_eq!(retry(&a, id, true), Ok(true));
        let j = a.db.jg(id).unwrap();
        assert_eq!(j.st, "ready");
        assert_eq!(j.d["recovery_attempts"], 1);
        assert!(j.d["last_review"].is_null());
        assert!(j.d["ci_evidence"].is_null());
        assert!(j.d["failure"].is_null());
        assert!(j.d["ir"].is_null());
        assert_eq!(j.d["drop_history"][0]["automatic"], true);
        assert_eq!(
            retry(&a, id, true),
            Err("Drop não elegível para recuperação segura")
        );
    }

    #[test]
    fn rejected_owner_or_guard_drops_are_never_automatically_retried() {
        let a = setup();
        for reason in [
            "dropped by the owner",
            "Guard rejected unsafe request",
            "Vet rejected",
        ] {
            let id = dropped(&a, reason, json!({}));
            assert_eq!(
                retry(&a, id, true),
                Err("Drop não elegível para recuperação segura")
            );
        }
    }

    #[test]
    fn manual_retry_does_not_reuse_approval_or_published_work() {
        let a = setup();
        for data in [
            json!({"ap":true}),
            json!({"published_sha":"abcdef"}),
            json!({"sq":true}),
        ] {
            let id = dropped(&a, "error: failed", data);
            assert_eq!(
                retry(&a, id, false),
                Err("Drop não elegível para recuperação segura")
            );
        }
        let id = dropped(&a, "Guard rejected", json!({"br":"scratch"}));
        assert_eq!(retry(&a, id, false), Ok(true));
        let j = a.db.jg(id).unwrap();
        assert_eq!(j.st, "vet");
        assert!(j.d["br"].is_null());
    }

    #[test]
    fn manual_recovery_queues_safely_while_office_is_paused() {
        let a = setup();
        a.lv.lock().unwrap().o.man = true;
        let manual = dropped(&a, "machine kept failing: timed out", json!({}));
        assert_eq!(retry(&a, manual, false), Ok(true));
        assert_eq!(a.db.jg(manual).unwrap().st, "ready");
        let automatic = dropped(&a, "machine kept failing: timed out", json!({}));
        assert_eq!(
            retry(&a, automatic, true),
            Err("Apollo Office em pausa ou manutenção")
        );
    }

    #[test]
    fn default_branch_refresh_keeps_markers_and_rejects_bad_sha() {
        let a = setup();
        a.db.ks("cired:org/repo", &json!({"sha":"a".repeat(40)}));
        let good = json!({"s":"a".repeat(40)});
        assert!(cache_default_head(&a, "org/repo", &good).is_ok());
        assert_eq!(
            crate::gh::cg(&a, "head:org/repo", 60_000),
            Some(good.clone())
        );
        assert_eq!(a.db.kg("cired:org/repo")["sha"], "a".repeat(40));
        assert!(cache_default_head(&a, "org/repo", &json!({"s":"invalid"})).is_err());
        assert_eq!(crate::gh::cg(&a, "head:org/repo", 60_000), Some(good));
    }

    #[test]
    fn no_auto_retry_without_explicit_opt_in_or_after_limit() {
        let a = setup();
        let id = dropped(
            &a,
            "machine kept failing: connection reset",
            json!({"recovery_attempts":2}),
        );
        assert_eq!(
            retry(&a, id, true),
            Err("Drop não elegível para recuperação segura")
        );
        let id = dropped(&a, "machine kept failing: connection reset", json!({}));
        a.db.run("update rp set auto_recover_drops=0", pa![]);
        assert_eq!(
            retry(&a, id, true),
            Err("Recuperação automática desativada")
        );
    }

    #[test]
    fn github_issue_validation_and_pr_duplicates_are_fail_closed() {
        assert!(open_issue(&json!({"number":42,"state":"open"}), 42));
        assert!(!open_issue(&json!({"number":42,"state":"closed"}), 42));
        assert!(!open_issue(&json!({"number":41,"state":"open"}), 42));
        assert!(!open_issue(
            &json!({"number":42,"state":"open","pull_request":{}}),
            42
        ));
        assert!(!open_issue(&json!({}), 42));
        let pr = json!({"body":"Fixes #42","number":123});
        assert!(has_related_pr(&[pr], 42));
        assert!(!has_related_pr(&[json!({"body":"Fixes #99"})], 42));
    }

    #[test]
    fn retry_commit_fences_policy_cancellation_and_newer_proof() {
        let a = setup();
        let id = dropped(&a, "machine kept failing: timed out", json!({}));
        let job = a.db.jg(id).unwrap();
        let updated = a.db.one("select t1 from jobs where id=?", pa![id]).unwrap()["t1"]
            .as_i64()
            .unwrap();
        let commit = || commit_retry(&a, &job, updated, true, "ready", "retry", &json!({}));
        let publication = format!("submit_publication:{id}");
        a.db.ks(&publication, &json!(null));
        assert!(!commit().unwrap());
        a.db.kd(&publication);
        a.db.run("update rp set auto_recover_drops=0", vec![]);
        assert!(!commit().unwrap());
        a.db.run("update rp set auto_recover_drops=1", vec![]);
        a.db.ks(&format!("kill:{id}"), &json!(true));
        assert!(!commit().unwrap());
        assert_eq!(a.db.kg(&format!("kill:{id}")), true);
        a.db.kd(&format!("kill:{id}"));
        a.db.run("update jobs set d=json_set(d,'$.ap',1) where id=?", pa![id]);
        assert!(!commit().unwrap());
        assert_eq!(a.db.jg(id).unwrap().st, "drop");
        a.db.run("update jobs set d='{}' where id=?", pa![id]);
        assert!(commit().unwrap());
        assert!(!commit().unwrap());
    }

    #[test]
    fn recovery_does_not_reuse_remote_evidence_for_changed_drop() {
        for automatic in [false, true] {
            let a = setup();
            let id = dropped(&a, "machine kept failing: timed out", json!({}));
            let (job, updated) = recovery_job(&a, id).unwrap();
            a.db.run("update jobs set num=43 where id=?", pa![id]);
            assert_eq!(retry_snapshot(&a, &job, updated, automatic), Ok(false));
            assert_eq!(a.db.jg(id).unwrap().st, "drop");
            a.db.run("update jobs set num=42 where id=?", pa![id]);
            a.db.run("update jobs set t1=t1+1 where id=?", pa![id]);
            assert_eq!(retry_snapshot(&a, &job, updated, automatic), Ok(false));
            a.db.run("update jobs set t1=? where id=?", pa![updated, id]);
            a.db.run(
                "update jobs set d='{\"recovery_attempts\":1}' where id=?",
                pa![id],
            );
            assert_eq!(retry_snapshot(&a, &job, updated, automatic), Ok(false));
            a.db.run("update jobs set d='{}' where id=?", pa![id]);
            assert_eq!(retry_snapshot(&a, &job, updated, automatic), Ok(true));
        }
    }

    #[tokio::test]
    async fn remote_entry_points_preserve_the_snapshot_during_io() {
        for automatic in [false, true] {
            for mutation in ["num", "timestamp", "metadata", "none"] {
                let a = setup();
                let id = dropped(&a, "machine kept failing: timed out", json!({}));
                let kill = format!("kill:{id}");
                if !automatic {
                    a.db.ks(&kill, &json!(true));
                }
                let mut calls = 0;
                let remote = |args: Vec<String>, _: u64| {
                    calls += 1;
                    let response = match args[0].as_str() {
                        "api" if args[1] == "repos/org/repo/issues/42" => {
                            match mutation {
                                "num" => {
                                    a.db.run("update jobs set num=43 where id=?", pa![id]);
                                }
                                "timestamp" => {
                                    a.db.run("update jobs set t1=t1+1 where id=?", pa![id]);
                                }
                                "metadata" => {
                                    a.db.run(
                                        "update jobs set d=json_set(d,'$.fixture',1) where id=?",
                                        pa![id],
                                    );
                                }
                                _ => {}
                            }
                            if automatic && mutation != "none" {
                                a.db.ks(&kill, &json!(true));
                            }
                            json!({"number":42,"state":"open"})
                        }
                        "pr" => json!([]),
                        "api" if args[1] == "repos/org/repo/commits/HEAD" => {
                            json!({"s":"a".repeat(40)})
                        }
                        _ => panic!("unexpected fixture request: {args:?}"),
                    };
                    std::future::ready(Ok(response))
                };
                if automatic {
                    scan_with(&a, "org/repo", &[], remote).await;
                    assert_eq!(calls, 1);
                } else {
                    assert_eq!(
                        retry_manual_with(&a, id, remote).await.unwrap(),
                        mutation == "none"
                    );
                    assert_eq!(calls, 3);
                }
                let job = a.db.jg(id).unwrap();
                if mutation == "none" {
                    assert_eq!(job.st, "ready");
                    assert_eq!(attempts(&job), Some(u64::from(automatic)));
                    assert!(a.db.kg(&kill).is_null());
                } else {
                    assert_eq!(job.st, "drop");
                    assert_eq!(attempts(&job), Some(0));
                    assert_eq!(a.db.kg(&kill), true);
                    assert!(job.d["drop_history"].is_null());
                }
            }
        }
    }

    #[test]
    fn recovery_waits_and_does_not_rerun_pending_duplicate_work() {
        let a = setup();
        let id = dropped(&a, "machine kept failing: connection reset", json!({}));
        a.db.run("update jobs set t1=? where id=?", pa![now(), id]);
        assert_eq!(retry(&a, id, true), Err("Aguarda intervalo de segurança"));
        a.db.run(
            "update jobs set t1=? where id=?",
            pa![now() - RETRY_DELAY_MS - 1, id],
        );
        a.db.jnew(
            "org/repo",
            42,
            "fix",
            "Outra tentativa",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        assert_eq!(
            retry(&a, id, true),
            Err("Já existe outro trabalho ativo para a mesma issue")
        );
    }
}
