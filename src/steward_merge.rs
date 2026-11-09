//! Merge conservador e opt-in. Por omissão nunca integra nada.
//! Cada operação é revalidada contra o GitHub no HEAD exato e sem bypass.
use crate::app::Ax;
use crate::pa;
use crate::sh::ghj;
use serde_json::{json, Value};

fn allowed_file(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    if p.starts_with(".github/")
        || p.starts_with("server/")
        || p.starts_with("database/")
        || p.starts_with("supabase/")
        || p.starts_with("migrations/")
        || p.starts_with("drizzle/")
        || p.starts_with("scripts/")
        || p.starts_with("infra/")
        || p.starts_with("deploy/")
    {
        return false;
    }
    ![
        "money",
        "monetary",
        "finance",
        "fiscal",
        "saft",
        "tax",
        "iva",
        "billing",
        "invoice",
        "treasury",
        "payment",
        "accounting",
        "permission",
        "rbac",
        "security",
        "auth",
        "credential",
        "idempotency",
        "migration",
        "secret",
        "policy-access",
    ]
    .iter()
    .any(|word| p.contains(word))
}

fn candidate(listed: &Value, assessed: &Value) -> Result<(i64, String), &'static str> {
    if assessed["state"] != "revisao_final_necessaria" {
        return Err("PR não passou a triagem ou pode colidir com outra PR");
    }
    if listed["isDraft"] != false {
        return Err("PR Draft");
    }
    if listed["mergeStateStatus"] != "CLEAN" {
        return Err("conflitos ou requisitos de merge não verificados");
    }
    if listed["reviewDecision"] != "APPROVED" {
        return Err("revisão independente não aprovada pelo GitHub");
    }
    let number = listed["number"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("número inválido")?;
    let head = listed["headRefOid"].as_str().ok_or("SHA ausente")?;
    if head.len() != 40 || !head.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("SHA inválido");
    }
    Ok((number, head.to_string()))
}

/// O modo local exige os mesmos bloqueios de conflito, Draft, duplicação e sensibilidade,
/// mas substitui os checks GitHub por provas locais do ApolloOffice do mesmo conteúdo.
fn local_candidate(listed: &Value, assessed: &Value) -> Result<(i64, String), &'static str> {
    if assessed["overlaps"]
        .as_array()
        .is_none_or(|x| !x.is_empty())
        || assessed["possible_duplicates"]
            .as_array()
            .is_none_or(|x| !x.is_empty())
        || assessed["checks"] == "falhou"
        || listed["reviewDecision"] == "CHANGES_REQUESTED"
    {
        return Err("conflitos, checks falhados, duplicação ou revisão com alterações pedidas");
    }
    if listed["isDraft"] != false || listed["mergeStateStatus"] != "CLEAN" {
        return Err("PR Draft ou sem merge limpo");
    }
    let n = listed["number"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("número inválido")?;
    let sha = listed["headRefOid"].as_str().ok_or("SHA ausente")?;
    if sha.len() != 40 || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("SHA inválido");
    }
    Ok((n, sha.to_string()))
}

/// Provas são recolhidas na CI e review do próprio ApolloOffice antes da publicação.
/// A árvore Git é estável durante squash, ao contrário do SHA do commit.
fn local_proof(job: &Value, head: &str, base: &str, tree: &str, ci: &str) -> bool {
    let d = &job["d"];
    let reviewed_by = d["last_review"]["model"].as_str().unwrap_or("");
    let author = d["author"].as_str().unwrap_or("");
    job["st"] == "done"
        && (job["kind"] == "fix" || job["kind"] == "review" || job["kind"] == "mergecheck")
        && d["published_sha"] == head
        && d["base"] == base
        && d["ci_tree"] == tree
        && d["publish_tree"] == tree
        && d["ci_evidence"]["exit"] == 0
        && d["ci_evidence"]["command"] == ci
        && d["ci_evidence"]["candidate"] == head
        && d["ci_verified_at"].as_i64().is_some_and(|v| v > 0)
        && d["last_review"]["t"]
            .as_i64()
            .is_some_and(|v| v >= d["ci_verified_at"].as_i64().unwrap_or(i64::MAX))
        && d["last_review"]["verdict"]["ok"] == true
        && !author.is_empty()
        && !reviewed_by.is_empty()
        && author != reviewed_by
}

fn remote_checks_clean(checks: &Value, status: &Value) -> bool {
    let Some(runs) = checks["check_runs"].as_array() else {
        return false;
    };
    let count = checks["total_count"].as_u64().unwrap_or(u64::MAX);
    if count > 100 || count != runs.len() as u64 {
        return false;
    }
    if !runs
        .iter()
        .all(|r| r["status"] == "completed" && r["conclusion"] == "success")
    {
        return false;
    }
    let Some(contexts) = status["statuses"].as_array() else {
        return false;
    };
    contexts.is_empty()
        || (status["state"] == "success" && contexts.iter().all(|r| r["state"] == "success"))
}

fn no_outstanding_reviews(reviews: &Value, head: &str) -> bool {
    let Some(rows) = reviews.as_array() else {
        return false;
    };
    if rows.len() >= 100 {
        return false;
    }
    rows.iter()
        .filter(|r| r["state"] == "CHANGES_REQUESTED")
        .all(|request| {
            let Some(author) = request["user"]["login"].as_str() else {
                return false;
            };
            let Some(at) = request["submitted_at"].as_str() else {
                return false;
            };
            rows.iter().any(|r| {
                r["user"]["login"] == author
                    && (r["state"] == "APPROVED" || r["state"] == "DISMISSED")
                    && r["commit_id"] == head
                    && r["submitted_at"].as_str().is_some_and(|t| t > at)
            })
        })
}

fn reviewed(reviews: &Value, author: &str, head: &str) -> bool {
    let Some(rows) = reviews.as_array() else {
        return false;
    };
    if rows.len() >= 100 {
        return false; // É necessário paginar para analisar a totalidade.
    }
    let mut approved = false;
    for r in rows {
        if r["state"] == "CHANGES_REQUESTED" {
            return false;
        }
        if r["state"] != "APPROVED" || r["commit_id"] != head {
            continue;
        }
        let Some(login) = r["user"]["login"].as_str() else {
            continue;
        };
        let t = r["user"]["type"].as_str().unwrap_or("");
        if !login.eq_ignore_ascii_case(author) && t != "Bot" && !login.ends_with("[bot]") {
            approved = true;
        }
    }
    approved
}

fn ci_verified(checks: &Value, status: &Value) -> bool {
    let Some(runs) = checks["check_runs"].as_array() else {
        return false;
    };
    let count = checks["total_count"].as_u64().unwrap_or(u64::MAX);
    if count == 0 || count != runs.len() as u64 || count > 100 {
        return false;
    }
    // Um check de revisão automatizada isolado não comprova testes.
    if !runs.iter().any(crate::steward::is_test_check) {
        return false;
    }
    if !runs
        .iter()
        .all(|r| r["status"] == "completed" && r["conclusion"] == "success")
    {
        return false;
    }
    let Some(contexts) = status["statuses"].as_array() else {
        return false;
    };
    contexts.is_empty()
        || (status["state"] == "success" && contexts.iter().all(|r| r["state"] == "success"))
}

async fn review_threads_resolved(repo: &str, number: i64) -> Result<bool, String> {
    let Some((owner, name)) = repo.split_once('/') else {
        return Ok(false);
    };
    let query = r#"query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){reviewThreads(first:100){nodes{isResolved}pageInfo{hasNextPage}}}}}"#;
    let data = ghj(
        &[
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-F",
            &format!("owner={owner}"),
            "-F",
            &format!("name={name}"),
            "-F",
            &format!("number={number}"),
        ],
        45,
    )
    .await?;
    if !data["errors"].is_null() {
        return Ok(false);
    }
    let threads = &data["data"]["repository"]["pullRequest"]["reviewThreads"];
    let Some(nodes) = threads["nodes"].as_array() else {
        return Ok(false);
    };
    if threads["pageInfo"]["hasNextPage"] != false {
        return Ok(false);
    }
    Ok(nodes.iter().all(|n| n["isResolved"] == true))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MergeMode {
    Blocked,
    Protected,
    Local,
}

async fn merge_mode(
    repo: &str,
    base: &str,
    sha: &str,
    local_enabled: bool,
) -> Result<MergeMode, String> {
    let branch = ghj(&["api", &format!("repos/{repo}/branches/{base}")], 40).await?;
    if branch["commit"]["sha"] != sha {
        return Ok(MergeMode::Blocked);
    }
    if branch["protected"] == true {
        let rules = ghj(
            &["api", &format!("repos/{repo}/branches/{base}/protection")],
            40,
        )
        .await?;
        let required = &rules["required_status_checks"]["contexts"];
        return Ok(
            if required
                .as_array()
                .is_some_and(|contexts| !contexts.is_empty())
            {
                MergeMode::Protected
            } else {
                MergeMode::Blocked
            },
        );
    }
    // Só aceitar a ausência EXPLÍCITA de proteção, e nunca um 403/valor omitido.
    Ok(if branch["protected"] == false && local_enabled {
        MergeMode::Local
    } else {
        MergeMode::Blocked
    })
}

/// Agenda apenas uma revalidação pendente por repositório.
/// Sem provas não há merge: o worker executará testes reais e Reviewer no HEAD.
fn queue_recheck(a: &Ax, repo: &str, number: i64, head: &str, base: &str, detail: &Value) -> bool {
    if a.hold("watcher") {
        return false;
    }
    let original = a.db.one(
        "select d from jobs where repo=? and pr=? and kind='fix' and st='done' order by id desc limit 1",
        pa![repo.to_string(), number],
    );
    let author = original
        .as_ref()
        .and_then(|x| x["d"]["author"].as_str())
        .unwrap_or("");
    let branch = detail["head"]["ref"].as_str().unwrap_or("");
    if author.is_empty() || branch.is_empty() {
        return false;
    }
    let data = json!({
        "head":repo,
        "branch":branch,
        "expected_head":head,
        "expected_base":base,
        "author":author,
        "body":detail["body"]
    });
    let t = crate::db::now();
    let inserted = a.db.ch(
        "insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1)
         select ?,?,'mergecheck',?,?,'ready','Revalidar CI e Reviewer antes de merge',?,?,-5.0,?,?
         where not exists (
             select 1 from jobs where repo=? and kind='mergecheck' and st not in ('done','drop')
         )
         and not exists (
             select 1 from jobs where repo=? and pr=? and kind='mergecheck'
               and json_extract(d,'$.expected_head')=?
               and json_extract(d,'$.expected_base')=?
         )",
        pa![
            repo.to_string(),
            number,
            detail["title"]
                .as_str()
                .unwrap_or("Revalidar PR")
                .to_string(),
            detail["html_url"].as_str().unwrap_or("").to_string(),
            data.to_string(),
            number,
            t,
            t,
            repo.to_string(),
            repo.to_string(),
            number,
            head.to_string(),
            base.to_string()
        ],
    ) == 1;
    if inserted {
        a.say(
            "watcher",
            &format!("{repo}#{number}: agendada revalidação gratuita com CI e Reviewer"),
            None,
        );
        a.sw.notify_one();
    }
    inserted
}

async fn try_one(a: &Ax, repo: &str, listed: &Value, assessed: &Value) -> Result<bool, String> {
    let local_enabled = a.db.n(
        "select local_merge from rp where full=? and on_=1 and auto_merge=1",
        pa![repo.to_string()],
    ) == 1;
    // A triagem local NÃO considera um check CodeRabbit prova de testes.
    let (number, head) = match if local_enabled {
        local_candidate(listed, assessed)
    } else {
        candidate(listed, assessed)
    } {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };
    if a.hold("watcher") {
        return Ok(false);
    }
    let detail = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 45).await?;
    let Some(base) = detail["base"]["ref"].as_str() else {
        return Ok(false);
    };
    let Some(base_sha) = detail["base"]["sha"].as_str() else {
        return Ok(false);
    };
    if detail["head"]["sha"] != head
        || detail["state"] != "open"
        || detail["draft"] != false
        || detail["mergeable"] != true
        || detail["mergeable_state"] != "clean"
        || detail["base"]["repo"]["full_name"] != repo
    {
        return Ok(false);
    }
    let metadata = ghj(&["api", &format!("repos/{repo}")], 40).await?;
    if metadata["default_branch"] != base || metadata["archived"] == true {
        return Ok(false);
    }
    let mode = merge_mode(repo, base, base_sha, local_enabled).await?;
    if mode == MergeMode::Blocked
        || (mode == MergeMode::Protected && candidate(listed, assessed).is_err())
    {
        return Ok(false);
    }
    if mode == MergeMode::Local {
        // Nunca usar o modo livre para integrar PRs de terceiros nem branches externas.
        let me = crate::gh::login(a).await;
        if me.is_empty()
            || detail["user"]["login"] != me
            || detail["head"]["repo"]["full_name"] != repo
        {
            return Ok(false);
        }
    }
    let changed = detail["changed_files"].as_u64().unwrap_or(u64::MAX);
    if changed == 0 || changed >= 100 {
        return Ok(false);
    }
    let files = ghj(
        &[
            "api",
            &format!("repos/{repo}/pulls/{number}/files?per_page=100"),
        ],
        45,
    )
    .await?;
    let Some(paths) = files.as_array() else {
        return Ok(false);
    };
    if paths.len() as u64 != changed
        || !paths
            .iter()
            .all(|p| p["filename"].as_str().is_some_and(allowed_file))
    {
        return Ok(false);
    }
    let checks = ghj(
        &[
            "api",
            &format!("repos/{repo}/commits/{head}/check-runs?per_page=100"),
        ],
        45,
    )
    .await?;
    let statuses = ghj(&["api", &format!("repos/{repo}/commits/{head}/status")], 45).await?;
    if (mode == MergeMode::Protected && !ci_verified(&checks, &statuses))
        || (mode == MergeMode::Local && !remote_checks_clean(&checks, &statuses))
    {
        return Ok(false);
    }
    let reviews = ghj(
        &[
            "api",
            &format!("repos/{repo}/pulls/{number}/reviews?per_page=100"),
        ],
        45,
    )
    .await?;
    if !review_threads_resolved(repo, number).await?
        || (mode == MergeMode::Protected
            && !reviewed(
                &reviews,
                detail["user"]["login"].as_str().unwrap_or(""),
                &head,
            ))
        || (mode == MergeMode::Local && !no_outstanding_reviews(&reviews, &head))
    {
        return Ok(false);
    }
    if mode == MergeMode::Local {
        // A árvore testada tem de já conter a main atual como ancestral.
        // Caso contrário, o update-branch cria um commit normal (sem force-push),
        // e a PR será novamente testada e revista no próximo ciclo.
        let ancestry = ghj(
            &["api", &format!("repos/{repo}/compare/{base_sha}...{head}")],
            45,
        )
        .await?;
        if ancestry["status"] != "ahead" && ancestry["status"] != "identical" {
            if ancestry["status"] == "diverged" {
                match ghj(
                    &[
                        "api",
                        "-X",
                        "PUT",
                        &format!("repos/{repo}/pulls/{number}/update-branch"),
                        "-f",
                        &format!("expected_head_sha={head}"),
                    ],
                    90,
                )
                .await
                {
                    Ok(_) => a.say(
                        "watcher",
                        &format!("{repo}#{number}: branch atualizada; CI terá de correr novamente"),
                        None,
                    ),
                    Err(error) => {
                        a.db.ks(
                            &format!("steward:update_error:{repo}#{number}"),
                            &json!({"head":head,"error":crate::sh::head(&error,500)}),
                        );
                    }
                }
            }
            return Ok(false);
        }

        let commit = ghj(&["api", &format!("repos/{repo}/git/commits/{head}")], 45).await?;
        let Some(tree) = commit["tree"]["sha"].as_str() else {
            return Ok(false);
        };
        let command = crate::pipe::rp(a, repo)["ci"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if command.is_empty() {
            return Ok(false);
        }
        let proven = a.db.all(
            "select kind,st,d from jobs where repo=? and pr=? and kind in ('fix','review','mergecheck') and st='done'",
            pa![repo.to_string(), number],
        );
        if !proven
            .iter()
            .any(|job| local_proof(job, &head, base_sha, tree, &command))
        {
            queue_recheck(a, repo, number, &head, base_sha, &detail);
            return Ok(false);
        }
    }

    // Reconfirmar a configuração e os SHAs imediatamente antes de um ato mutável.
    if a.hold("watcher")
        || a.db.n(
            "select auto_merge from rp where full=? and on_=1",
            pa![repo.to_string()],
        ) != 1
        || (mode == MergeMode::Local
            && a.db.n(
                "select local_merge from rp where full=? and on_=1",
                pa![repo.to_string()],
            ) != 1)
    {
        return Ok(false);
    }
    let fresh = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 40).await?;
    if fresh["head"]["sha"] != head
        || fresh["base"]["sha"] != base_sha
        || fresh["draft"] != false
        || fresh["mergeable"] != true
        || fresh["mergeable_state"] != "clean"
        || fresh["state"] != "open"
    {
        return Ok(false);
    }

    // O modo local NÃO protege a branch de pushes externos. Reduzir a janela:
    // base e HEAD são revalidados imediatamente antes do PUT, sem --admin/force.
    // O GitHub só oferece o campo sha para validar o HEAD da PR, não a base.
    let latest = ghj(&["api", &format!("repos/{repo}/branches/{base}")], 40).await?;
    if latest["commit"]["sha"] != base_sha {
        return Ok(false);
    }
    // sha=HEAD impede integrar um commit da PR que tenha sido alterado.
    let merged = ghj(
        &[
            "api",
            "-X",
            "PUT",
            &format!("repos/{repo}/pulls/{number}/merge"),
            "-f",
            "merge_method=squash",
            "-f",
            &format!("sha={head}"),
        ],
        90,
    )
    .await?;
    if merged["merged"] != true {
        return Err(format!("GitHub não confirmou o merge de {repo}#{number}"));
    }
    let confirm = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 40).await?;
    if confirm["merged"] != true {
        return Err(format!(
            "Merge de {repo}#{number} não confirmado por leitura"
        ));
    }
    a.db.run(
        "update prs set st='merged',merged=? where repo=? and num=?",
        pa![chrono::Utc::now().to_rfc3339(), repo.to_string(), number],
    );
    a.say(
        "submit",
        &format!("PR integrada e verificada: {repo}#{number}"),
        None,
    );
    a.note(
        "PR integrada",
        &format!("GitHub confirmou merge ordinário de {repo}#{number}; HEAD da PR {head}"),
        3,
    );
    Ok(true)
}

/// No máximo um merge por repositório e leitura, para reavaliar a nova main.
pub async fn run(a: &Ax, repo: &str, listed: &[Value], assessed: &[Value]) {
    if a.db.n(
        "select auto_merge from rp where full=? and on_=1",
        pa![repo.to_string()],
    ) != 1
    {
        return;
    }
    for (pr, status) in listed.iter().zip(assessed.iter()) {
        match try_one(a, repo, pr, status).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                a.db.ks(
                    &format!("steward:merge_error:{repo}"),
                    &json!({"pr":pr["number"],"reason":crate::sh::head(&error, 500)}),
                );
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revalidation_jobs_are_singleton_per_pr_and_head() {
        let db = crate::db::Db::mem();
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            47300,
            "apollo-office".into(),
        ));
        let repo = "croquete1/FixFlow-novo";
        let head = "0123456789012345678901234567890123456789";
        let base = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        a.db.jnew(
            repo,
            1,
            "fix",
            "Correção existente",
            "",
            "done",
            "",
            json!({"author":"codex:gpt-5.6-luna"}),
            Some(42),
            0.0,
        );
        let detail = json!({
            "title":"Corrigir problema",
            "html_url":"https://github.com/croquete1/FixFlow-novo/pull/42",
            "head":{"ref":"fix/repro"},
            "body":"Teste de regressão"
        });
        assert!(queue_recheck(&a, repo, 42, head, base, &detail));
        assert!(!queue_recheck(&a, repo, 42, head, base, &detail));
        assert_eq!(
            a.db.n(
                "select count(*) from jobs where repo=? and kind='mergecheck'",
                pa![repo.to_string()]
            ),
            1
        );
        let job =
            a.db.one("select * from jobs where kind='mergecheck'", pa![])
                .unwrap();
        assert_eq!(job["d"]["expected_head"], head);
        assert_eq!(job["d"]["expected_base"], base);
    }

    #[test]
    fn local_candidate_rejects_overlaps_failures_and_drafts() {
        let mut pr = json!({
            "number":9,"headRefOid":"0123456789012345678901234567890123456789",
            "isDraft":false,"mergeStateStatus":"CLEAN","reviewDecision":"APPROVED"
        });
        let mut assessment = json!({
            "state":"ci_pendente","overlaps":[],"possible_duplicates":[],"checks":"nao_verificado"
        });
        assert!(local_candidate(&pr, &assessment).is_ok());
        assessment["overlaps"] = json!([12]);
        assert!(local_candidate(&pr, &assessment).is_err());
        assessment["overlaps"] = json!([]);
        assessment["possible_duplicates"] = json!([16]);
        assert!(local_candidate(&pr, &assessment).is_err());
        assessment["possible_duplicates"] = json!([]);
        assessment["checks"] = json!("falhou");
        assert!(local_candidate(&pr, &assessment).is_err());
        assessment["checks"] = json!("nao_verificado");
        pr["reviewDecision"] = json!("CHANGES_REQUESTED");
        assert!(local_candidate(&pr, &assessment).is_err());
        pr["reviewDecision"] = json!("APPROVED");
        pr["isDraft"] = json!(true);
        assert!(local_candidate(&pr, &assessment).is_err());
    }

    #[test]
    fn local_proof_requires_same_published_sha_base_tree_and_independent_review() {
        let sha = "0123456789012345678901234567890123456789";
        let base = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let tree = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let ci = "npm test && npm run build";
        let mut job = json!({
            "st":"done","kind":"fix",
            "d":{
                "base":base,"published_sha":sha,"ci_tree":tree,"publish_tree":tree,
                "author":"codex:luna",
                "ci_evidence":{"exit":0,"command":ci,"candidate":sha},
                "ci_verified_at":200,
                "last_review":{"model":"codex:sol","t":201,"verdict":{"ok":true}}
            }
        });
        assert!(local_proof(&job, sha, base, tree, ci));
        job["d"]["ci_evidence"]["candidate"] =
            json!("ffffffffffffffffffffffffffffffffffffffff");
        assert!(!local_proof(&job, sha, base, tree, ci));
        job["d"]["ci_evidence"]["candidate"] = json!(sha);
        job["d"]["ci_evidence"].as_object_mut().unwrap().remove("candidate");
        assert!(!local_proof(&job, sha, base, tree, ci));
        job["d"]["ci_evidence"]["candidate"] = json!(sha);
        job["d"]["published_sha"] = json!("ffffffffffffffffffffffffffffffffffffffff");
        assert!(!local_proof(&job, sha, base, tree, ci));
        job["d"]["published_sha"] = json!(sha);
        job["d"]["publish_tree"] = json!("ffffffffffffffffffffffffffffffffffffffff");
        assert!(!local_proof(&job, sha, base, tree, ci));
        job["d"]["publish_tree"] = json!(tree);
        job["d"]["last_review"]["model"] = json!("codex:luna");
        assert!(!local_proof(&job, sha, base, tree, ci));
        job["d"]["last_review"]["model"] = json!("codex:sol");
        job["d"]["ci_evidence"]["exit"] = json!(1);
        assert!(!local_proof(&job, sha, base, tree, ci));
        job["d"]["ci_evidence"]["exit"] = json!(0);
        job["d"]["last_review"]["verdict"]["ok"] = json!(false);
        assert!(!local_proof(&job, sha, base, tree, ci));
        job["d"]["last_review"]["verdict"]["ok"] = json!(true);
        assert!(!local_proof(&job, sha, "different-base", tree, ci));
        assert!(!local_proof(&job, sha, base, tree, "different-ci"));
    }

    #[test]
    fn local_mode_requires_remote_checks_clear_and_no_change_requests() {
        let empty = json!({"total_count":0,"check_runs":[]});
        let statuses = json!({"statuses":[]});
        assert!(remote_checks_clean(&empty, &statuses));
        assert!(!remote_checks_clean(
            &json!({
                "total_count":1,"check_runs":[{
                    "name":"CodeRabbit","status":"completed","conclusion":"failure"
                }]
            }),
            &statuses
        ));
        assert!(!remote_checks_clean(
            &json!({
                "total_count":1,"check_runs":[{
                    "name":"CodeRabbit","status":"in_progress","conclusion":null
                }]
            }),
            &statuses
        ));
        let sha = "0123456789012345678901234567890123456789";
        assert!(no_outstanding_reviews(&json!([]), sha));
        assert!(!no_outstanding_reviews(
            &json!([{"state":"CHANGES_REQUESTED"}]),
            sha
        ));
        let requested = json!({"state":"CHANGES_REQUESTED","commit_id":sha,
            "user":{"login":"bot"},"submitted_at":"2026-11-09T13:00:00Z"});
        let approved = json!({"state":"APPROVED","commit_id":sha,
            "user":{"login":"bot"},"submitted_at":"2026-11-09T14:00:00Z"});
        assert!(no_outstanding_reviews(&json!([requested, approved]), sha));
    }

    #[test]
    fn never_merge_draft_conflict_or_missing_review() {
        let mut p = json!({
            "number": 9,"headRefOid":"0123456789012345678901234567890123456789",
            "isDraft":false,"mergeStateStatus":"CLEAN","reviewDecision":"APPROVED"
        });
        let assessed = json!({"state":"revisao_final_necessaria"});
        assert!(candidate(&p, &assessed).is_ok());
        p["isDraft"] = json!(true);
        assert!(candidate(&p, &assessed).is_err());
        p["isDraft"] = json!(false);
        p["mergeStateStatus"] = json!("DIRTY");
        assert!(candidate(&p, &assessed).is_err());
        p["mergeStateStatus"] = json!("CLEAN");
        p["reviewDecision"] = json!("REVIEW_REQUIRED");
        assert!(candidate(&p, &assessed).is_err());
    }

    #[test]
    fn sensitive_changes_are_not_eligible() {
        for path in [
            ".github/workflows/ci.yml",
            "shared/money.ts",
            "server/routes/billing.ts",
            "src/views/treasury/TreasuryForm.tsx",
            "supabase/schema.sql",
            "src/services/permissions.ts",
        ] {
            assert!(!allowed_file(path), "{path}");
        }
        assert!(allowed_file("src/components/Modal.tsx"));
    }

    #[test]
    fn requires_external_review_at_final_sha() {
        let head = "0123456789012345678901234567890123456789";
        let good = json!([{
            "state":"APPROVED","commit_id":head,
            "user":{"login":"reviewer","type":"User"}
        }]);
        assert!(reviewed(&good, "author", head));
        assert!(!reviewed(&good, "reviewer", head));
        assert!(!reviewed(
            &good,
            "author",
            "ffffffffffffffffffffffffffffffffffffffff"
        ));
        let changed = json!([{
            "state":"APPROVED","commit_id":head,
            "user":{"login":"reviewer","type":"User"}
        },{
            "state":"CHANGES_REQUESTED","commit_id":head,
            "user":{"login":"other","type":"User"}
        }]);
        assert!(!reviewed(&changed, "author", head));
    }

    #[test]
    fn bot_approval_is_not_independent_human_review() {
        let head = "0123456789012345678901234567890123456789";
        let bot = json!([{
            "state":"APPROVED","commit_id":head,
            "user":{"login":"coderabbitai[bot]","type":"Bot"}
        }]);
        assert!(!reviewed(&bot, "author", head));
    }

    #[test]
    fn missing_or_failed_checks_never_pass() {
        let status = json!({"statuses":[]});
        assert!(!ci_verified(
            &json!({"total_count":0,"check_runs":[]}),
            &status
        ));
        assert!(!ci_verified(
            &json!({"total_count":1,"check_runs":[{"status":"completed","conclusion":"failure"}]}),
            &status
        ));
        assert!(!ci_verified(
            &json!({"total_count":1,"check_runs":[{
                "name":"CodeRabbit","status":"completed","conclusion":"success",
                "app":{"slug":"coderabbitai"}
            }]}),
            &status
        ));
        assert!(ci_verified(
            &json!({"total_count":1,"check_runs":[{
                "name":"test-suite","status":"completed","conclusion":"success",
                "app":{"slug":"ci-service"}
            }]}),
            &status
        ));
    }
}
