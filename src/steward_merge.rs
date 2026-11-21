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

/// Permit a stale-base refresh even when the ordinary merge candidate is
/// temporarily blocked by GitHub's BEHIND state or checks rerunning.
fn stale_base_candidate(listed: &Value, assessed: &Value) -> Result<(i64, String), &'static str> {
    if listed["mergeStateStatus"] != "BEHIND"
        || listed["isDraft"] != false
        || assessed["checks"] == "falhou"
        || listed["reviewDecision"] == "CHANGES_REQUESTED"
        || assessed["overlaps"]
            .as_array()
            .is_none_or(|x| !x.is_empty())
        || assessed["possible_duplicates"]
            .as_array()
            .is_none_or(|x| !x.is_empty())
    {
        return Err("PR não pode ser atualizada automaticamente");
    }
    let number = listed["number"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("número inválido")?;
    let head = listed["headRefOid"].as_str().ok_or("SHA ausente")?;
    if !valid_sha(head) {
        return Err("SHA inválido");
    }
    Ok((number, head.to_string()))
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

fn valid_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

/// An advanced base makes the previous CI proof stale. Updating is allowed
/// only for the owner's same-repository PR branch; GitHub reruns its gates on
/// the new head before the ordinary merge path can proceed.
fn can_update_branch(
    detail: &Value,
    repo: &str,
    base: &str,
    current_base: &str,
    login: &str,
) -> bool {
    let pr_base = detail["base"]["sha"].as_str().unwrap_or("");
    let head = detail["head"]["sha"].as_str().unwrap_or("");
    !login.is_empty()
        && detail["state"] == "open"
        && detail["draft"] == false
        && detail["user"]["login"] == login
        && detail["head"]["repo"]["full_name"] == repo
        && detail["base"]["repo"]["full_name"] == repo
        && detail["base"]["ref"] == base
        && detail["head"]["ref"] != base
        && valid_sha(pr_base)
        && valid_sha(current_base)
        && valid_sha(head)
        && pr_base != current_base
}

fn update_retry_deferred(previous: &Value, head: &str, base: &str, now: i64) -> bool {
    previous["head"] == head
        && previous["base"] == base
        && previous["at"]
            .as_i64()
            .is_some_and(|at| now.saturating_sub(at) < 5 * 60_000)
}

/// A API GitHub disponibiliza checks obrigatórios em `contexts` e/ou `checks`.
/// Ambos têm de indicar uma exigência concreta; listas vazias não autorizam merge.
fn has_required_status_checks(protection: &Value) -> bool {
    let required = &protection["required_status_checks"];
    required["contexts"].as_array().is_some_and(|contexts| {
        contexts
            .iter()
            .any(|x| x.as_str().is_some_and(|name| !name.trim().is_empty()))
    }) || required["checks"].as_array().is_some_and(|checks| {
        checks.iter().any(|x| {
            x["context"]
                .as_str()
                .is_some_and(|name| !name.trim().is_empty())
        })
    })
}

async fn merge_mode(
    repo: &str,
    base: &str,
    branch: &Value,
    local_enabled: bool,
) -> Result<MergeMode, String> {
    if branch["protected"] == true {
        let rules = ghj(
            &["api", &format!("repos/{repo}/branches/{base}/protection")],
            40,
        )
        .await?;
        return Ok(if has_required_status_checks(&rules) {
            MergeMode::Protected
        } else {
            MergeMode::Blocked
        });
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

// A reconciliação normal é sequencial. O guard protege também chamadas
// concorrentes ao Steward dentro do mesmo processo (e é libertado no cancelamento).
struct MergePermit {
    app: Ax,
    repo: String,
}

impl MergePermit {
    /// Acquires a merge permit only if a binary update has not begun draining.
    fn acquire(app: &Ax, repo: &str) -> Option<Self> {
        let _transition = app.update_transition.lock().ok()?;
        if app.drain.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        let mut active = app.merge_in_progress.lock().ok()?;
        if !active.insert(repo.to_string()) {
            return None;
        }
        Some(Self {
            app: app.clone(),
            repo: repo.to_string(),
        })
    }
}

impl Drop for MergePermit {
    fn drop(&mut self) {
        if let Ok(mut active) = self.app.merge_in_progress.lock() {
            active.remove(&self.repo);
        }
    }
}

struct ExpectedMerge<'a> {
    repo: &'a str,
    base: &'a str,
    base_sha: &'a str,
    head: &'a str,
    author: &'a str,
    mode: MergeMode,
}

fn pr_current(pr: &Value, expected: &ExpectedMerge<'_>) -> bool {
    pr["state"] == "open"
        && pr["draft"] == false
        && pr["head"]["sha"] == expected.head
        && pr["base"]["sha"] == expected.base_sha
        && pr["base"]["ref"] == expected.base
        && pr["base"]["repo"]["full_name"] == expected.repo
        && pr["user"]["login"] == expected.author
        && pr["mergeable"] == true
        && pr["mergeable_state"] == "clean"
        && (expected.mode != MergeMode::Local || pr["head"]["repo"]["full_name"] == expected.repo)
}

fn review_gate(reviews: &Value, expected: &ExpectedMerge<'_>) -> bool {
    match expected.mode {
        MergeMode::Blocked => false,
        MergeMode::Protected => reviewed(reviews, expected.author, expected.head),
        MergeMode::Local => no_outstanding_reviews(reviews, expected.head),
    }
}

fn check_gate(checks: &Value, statuses: &Value, mode: MergeMode) -> bool {
    match mode {
        MergeMode::Blocked => false,
        MergeMode::Protected => ci_verified(checks, statuses),
        MergeMode::Local => remote_checks_clean(checks, statuses),
    }
}

struct FinalReads {
    pr: Value,
    branch: Value,
    reviews: Value,
    threads_resolved: bool,
    checks: Value,
    statuses: Value,
}

// Avaliação do mesmo conjunto de leituras realmente usado antes do PUT.
// Erros da API não produzem permissão de merge.
fn final_gate(
    reads: Result<FinalReads, String>,
    expected: &ExpectedMerge<'_>,
) -> Result<bool, String> {
    let r = reads?;
    Ok(pr_current(&r.pr, expected)
        && r.branch["commit"]["sha"] == expected.base_sha
        && r.threads_resolved
        && review_gate(&r.reviews, expected)
        && check_gate(&r.checks, &r.statuses, expected.mode))
}

async fn try_one(a: &Ax, repo: &str, listed: &Value, assessed: &Value) -> Result<bool, String> {
    let local_enabled = a.db.n(
        "select local_merge from rp where full=? and on_=1 and auto_merge=1",
        pa![repo.to_string()],
    ) == 1;
    // A triagem local NÃO considera um check CodeRabbit prova de testes.
    let selected = match if local_enabled {
        local_candidate(listed, assessed)
    } else {
        candidate(listed, assessed)
    } {
        Ok(c) => (c, true),
        Err(_) => match stale_base_candidate(listed, assessed) {
            Ok(c) => (c, false),
            Err(_) => return Ok(false),
        },
    };
    let ((number, head), candidate_ready) = selected;
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
        || detail["base"]["repo"]["full_name"] != repo
    {
        return Ok(false);
    }
    let metadata = ghj(&["api", &format!("repos/{repo}")], 40).await?;
    if metadata["default_branch"] != base || metadata["archived"] == true {
        return Ok(false);
    }
    let branch = ghj(&["api", &format!("repos/{repo}/branches/{base}")], 40).await?;
    let current_base = branch["commit"]["sha"].as_str().unwrap_or("");
    if current_base.is_empty() {
        return Ok(false);
    }
    if current_base != base_sha {
        let update_key = format!("steward:update:{repo}#{number}");
        let previous = a.db.kg(&update_key);
        if update_retry_deferred(&previous, &head, current_base, crate::db::now()) {
            return Ok(false);
        }
        let login = crate::gh::login(a).await;
        if can_update_branch(&detail, repo, base, current_base, &login) {
            if a.hold("watcher")
                || a.db.n(
                    "select auto_merge from rp where full=? and on_=1",
                    pa![repo.to_string()],
                ) != 1
            {
                return Ok(false);
            }
            let latest = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 40).await?;
            if latest["head"]["sha"] != head
                || latest["base"]["sha"] == current_base
                || latest["base"]["sha"] != base_sha
                || latest["base"]["ref"] != base
                || !can_update_branch(&latest, repo, base, current_base, &login)
            {
                return Ok(false);
            }
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
                Ok(_) => {
                    a.db.ks(
                        &update_key,
                        &json!({
                            "status":"updated",
                            "head":head,
                            "base":current_base,
                            "at":crate::db::now(),
                            "message":"Branch atualizada no GitHub; CI e revisão têm de validar o novo HEAD"
                        }),
                    );
                    a.say(
                        "watcher",
                        &format!("{repo}#{number}: base avançou; branch atualizada e provas anteriores invalidadas"),
                        None,
                    );
                    a.steward_wake.notify_one();
                }
                Err(error) => {
                    a.db.ks(
                        &update_key,
                        &json!({
                            "status":"error",
                            "head":head,
                            "base":current_base,
                            "at":crate::db::now(),
                            "error":crate::sh::head(&error,500)
                        }),
                    );
                    a.say(
                        "watcher",
                        &format!("{repo}#{number}: não foi possível atualizar a branch para a base atual"),
                        None,
                    );
                }
            }
        } else {
            let reason = "a PR não é uma branch própria do repositório ou faltam permissões";
            a.db.ks(
                &update_key,
                &json!({
                    "status":"blocked",
                    "head":head,
                    "base":current_base,
                    "at":crate::db::now(),
                    "error":reason
                }),
            );
            a.say(
                "watcher",
                &format!("{repo}#{number}: base avançou; {reason}"),
                None,
            );
        }
        return Ok(false);
    }
    if !candidate_ready {
        return Ok(false);
    }
    if detail["mergeable"] != true || detail["mergeable_state"] != "clean" {
        return Ok(false);
    }
    let mode = merge_mode(repo, base, &branch, local_enabled).await?;
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
    if !check_gate(&checks, &statuses, mode) {
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
    let expected = ExpectedMerge {
        repo,
        base,
        base_sha,
        head: &head,
        author: detail["user"]["login"].as_str().unwrap_or(""),
        mode,
    };
    if !review_threads_resolved(repo, number).await? || !review_gate(&reviews, &expected) {
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
    // Ler novamente revisões, threads, checks, base e PR antes do PUT.
    // Uma resposta incompleta ou erro da API aborta o merge.
    let read: Result<FinalReads, String> = async {
        let reviews = ghj(
            &[
                "api",
                &format!("repos/{repo}/pulls/{number}/reviews?per_page=100"),
            ],
            40,
        )
        .await?;
        let threads_resolved = review_threads_resolved(repo, number).await?;
        let checks = ghj(
            &[
                "api",
                &format!("repos/{repo}/commits/{head}/check-runs?per_page=100"),
            ],
            40,
        )
        .await?;
        let statuses = ghj(&["api", &format!("repos/{repo}/commits/{head}/status")], 40).await?;
        let branch = ghj(&["api", &format!("repos/{repo}/branches/{base}")], 40).await?;
        let pr = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 40).await?;
        Ok(FinalReads {
            pr,
            branch,
            reviews,
            threads_resolved,
            checks,
            statuses,
        })
    }
    .await;
    if !final_gate(read, &expected)? {
        return Ok(false);
    }
    // O modo local NÃO garante atomicidade face a outros processos:
    // a API de merge só tem compare-and-swap para o HEAD da PR.
    // A base é reconfirmada imediatamente antes da escrita.
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
    let Some(_permit) = MergePermit::acquire(a, repo) else {
        return;
    };
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

    fn sample_expected() -> ExpectedMerge<'static> {
        ExpectedMerge {
            repo: "croquete1/repo-e2e",
            base: "main",
            base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            author: "croquete1",
            mode: MergeMode::Local,
        }
    }

    fn sample_final_reads() -> FinalReads {
        let e = sample_expected();
        FinalReads {
            pr: json!({
                "state":"open","draft":false,"mergeable":true,
                "mergeable_state":"clean","user":{"login":e.author},
                "head":{"sha":e.head,"repo":{"full_name":e.repo}},
                "base":{"ref":e.base,"sha":e.base_sha,"repo":{"full_name":e.repo}}
            }),
            branch: json!({"commit":{"sha":e.base_sha}}),
            reviews: json!([]),
            threads_resolved: true,
            checks: json!({"total_count":0,"check_runs":[]}),
            statuses: json!({"statuses":[]}),
        }
    }

    #[test]
    fn head_or_base_changed_during_review_never_authorizes_merge() {
        let e = sample_expected();
        assert!(final_gate(Ok(sample_final_reads()), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.pr["head"]["sha"] = json!("cccccccccccccccccccccccccccccccccccccccc");
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.pr["base"]["sha"] = json!("cccccccccccccccccccccccccccccccccccccccc");
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.branch["commit"]["sha"] = json!("cccccccccccccccccccccccccccccccccccccccc");
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.pr["mergeable_state"] = json!("dirty");
        assert!(!final_gate(Ok(snapshot), &e).unwrap());
    }

    #[test]
    fn late_negative_review_and_unresolved_thread_block_merge() {
        let e = sample_expected();
        let mut snapshot = sample_final_reads();
        snapshot.reviews = json!([{
            "state":"CHANGES_REQUESTED",
            "user":{"login":"maintainer"},
            "submitted_at":"2026-11-09T23:00:00Z",
            "commit_id":e.head
        }]);
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.threads_resolved = false;
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.pr["draft"] = json!(true);
        assert!(!final_gate(Ok(snapshot), &e).unwrap());
    }

    #[test]
    fn protected_mode_requires_current_human_approval_and_green_ci() {
        let mut e = sample_expected();
        e.mode = MergeMode::Protected;
        let mut snapshot = sample_final_reads();
        snapshot.checks = json!({
            "total_count":1,
            "check_runs":[{
                "name":"test-suite",
                "status":"completed",
                "conclusion":"success",
                "app":{"slug":"ci-service"}
            }]
        });
        let valid_checks = snapshot.checks.clone();
        snapshot.reviews = json!([{
            "state":"APPROVED",
            "commit_id":e.head,
            "user":{"login":"maintainer","type":"User"}
        }]);
        assert!(final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.checks = valid_checks.clone();
        snapshot.reviews = json!([{
            "state":"APPROVED",
            "commit_id":"cccccccccccccccccccccccccccccccccccccccc",
            "user":{"login":"maintainer","type":"User"}
        }]);
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.checks = valid_checks;
        snapshot.reviews = json!([{
            "state":"CHANGES_REQUESTED",
            "commit_id":e.head,
            "user":{"login":"maintainer","type":"User"}
        }]);
        assert!(!final_gate(Ok(snapshot), &e).unwrap());
    }

    #[test]
    fn branch_protection_supports_modern_required_checks_without_bypass() {
        assert!(has_required_status_checks(&json!({
            "required_status_checks":{"contexts":["unit-tests"]}
        })));
        assert!(has_required_status_checks(&json!({
            "required_status_checks":{"checks":[{"context":"CI / Tests","app_id":123}]}
        })));
        assert!(!has_required_status_checks(&json!({
            "required_status_checks":{"checks":[{"app_id":123}]}
        })));
        assert!(!has_required_status_checks(&json!({
            "required_status_checks":{"contexts":[],"checks":[]}
        })));
        assert!(!has_required_status_checks(&json!({})));
    }

    #[test]
    fn branch_update_requires_an_owned_open_same_repository_pr() {
        let old_base = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let current_base = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let head = "cccccccccccccccccccccccccccccccccccccccc";
        let pr = json!({
            "state":"open","draft":false,"user":{"login":"croquete1"},
            "head":{"sha":head,"ref":"fix/task","repo":{"full_name":"croquete1/repo"}},
            "base":{"sha":old_base,"ref":"main","repo":{"full_name":"croquete1/repo"}}
        });
        assert!(can_update_branch(
            &pr,
            "croquete1/repo",
            "main",
            current_base,
            "croquete1"
        ));

        let mut fork = pr.clone();
        fork["head"]["repo"]["full_name"] = json!("contributor/repo");
        assert!(!can_update_branch(
            &fork,
            "croquete1/repo",
            "main",
            current_base,
            "croquete1"
        ));

        let mut other_author = pr.clone();
        other_author["user"]["login"] = json!("contributor");
        assert!(!can_update_branch(
            &other_author,
            "croquete1/repo",
            "main",
            current_base,
            "croquete1"
        ));

        assert!(!can_update_branch(
            &pr,
            "croquete1/repo",
            "main",
            current_base,
            ""
        ));
        assert!(!can_update_branch(
            &pr,
            "croquete1/repo",
            "develop",
            current_base,
            "croquete1"
        ));
        let mut same_branch = pr.clone();
        same_branch["head"]["ref"] = json!("main");
        assert!(!can_update_branch(
            &same_branch,
            "croquete1/repo",
            "main",
            current_base,
            "croquete1"
        ));
        assert!(!can_update_branch(
            &pr,
            "croquete1/repo",
            "main",
            old_base,
            "croquete1"
        ));

        let mut draft = pr.clone();
        draft["draft"] = json!(true);
        assert!(!can_update_branch(
            &draft,
            "croquete1/repo",
            "main",
            current_base,
            "croquete1"
        ));
    }

    #[test]
    fn branch_update_backoff_is_scoped_to_head_and_base() {
        let previous = json!({"head":"h1","base":"b1","at":10_000});
        assert!(update_retry_deferred(&previous, "h1", "b1", 10_001));
        assert!(!update_retry_deferred(&previous, "h2", "b1", 10_001));
        assert!(!update_retry_deferred(&previous, "h1", "b2", 10_001));
        assert!(!update_retry_deferred(&previous, "h1", "b1", 310_000));
    }

    #[test]
    fn protected_branch_does_not_accept_lint_only_as_test_evidence() {
        let mut e = sample_expected();
        e.mode = MergeMode::Protected;
        let mut snapshot = sample_final_reads();
        snapshot.checks = json!({
            "total_count":1,
            "check_runs":[{
                "name":"lint",
                "status":"completed",
                "conclusion":"success"
            }]
        });
        snapshot.reviews = json!([{
            "state":"APPROVED",
            "commit_id":e.head,
            "user":{"login":"maintainer","type":"User"}
        }]);
        assert!(!final_gate(Ok(snapshot), &e).unwrap());
    }

    #[test]
    fn missing_or_failed_github_api_data_fails_closed() {
        let e = sample_expected();
        assert!(final_gate(Err("GitHub HTTP 503".to_string()), &e).is_err());
        assert!(final_gate(Err("GitHub timeout".to_string()), &e).is_err());

        let mut snapshot = sample_final_reads();
        snapshot.pr = json!({});
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.reviews = json!(null);
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.checks = json!(null);
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.branch = json!({});
        assert!(!final_gate(Ok(snapshot), &e).unwrap());
    }

    #[test]
    fn remote_ci_turning_red_before_merge_blocks_it() {
        let e = sample_expected();
        let mut snapshot = sample_final_reads();
        snapshot.checks = json!({
            "total_count":1,
            "check_runs":[{"status":"completed","conclusion":"failure"}]
        });
        assert!(!final_gate(Ok(snapshot), &e).unwrap());

        let mut snapshot = sample_final_reads();
        snapshot.statuses = json!({
            "state":"failure","statuses":[{"state":"failure"}]
        });
        assert!(!final_gate(Ok(snapshot), &e).unwrap());
    }

    #[test]
    fn concurrent_scans_share_a_per_repo_merge_permit() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            47301,
            "apollo-office".into(),
        ));
        let permit = MergePermit::acquire(&a, "croquete1/repo-e2e").unwrap();
        let other = a.clone();
        let denied = std::thread::spawn(move || {
            MergePermit::acquire(&other, "croquete1/repo-e2e").is_none()
        })
        .join()
        .unwrap();
        assert!(denied);
        assert!(MergePermit::acquire(&a, "croquete1/outro-repo").is_some());
        drop(permit);
        assert!(MergePermit::acquire(&a, "croquete1/repo-e2e").is_some());
    }

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
    fn behind_pr_can_refresh_only_when_content_has_no_known_blocker() {
        let mut pr = json!({
            "number":9,
            "headRefOid":"0123456789012345678901234567890123456789",
            "isDraft":false,
            "mergeStateStatus":"BEHIND",
            "reviewDecision":"APPROVED"
        });
        let mut assessment = json!({
            "overlaps":[],
            "possible_duplicates":[],
            "checks":"aprovado"
        });
        assert_eq!(
            stale_base_candidate(&pr, &assessment).unwrap(),
            (9, "0123456789012345678901234567890123456789".to_string())
        );

        pr["mergeStateStatus"] = json!("DIRTY");
        assert!(stale_base_candidate(&pr, &assessment).is_err());
        pr["mergeStateStatus"] = json!("BEHIND");
        assessment["overlaps"] = json!([10]);
        assert!(stale_base_candidate(&pr, &assessment).is_err());
        assessment["overlaps"] = json!([]);
        assessment["checks"] = json!("falhou");
        assert!(stale_base_candidate(&pr, &assessment).is_err());
        assessment["checks"] = json!("aprovado");
        pr["reviewDecision"] = json!("CHANGES_REQUESTED");
        assert!(stale_base_candidate(&pr, &assessment).is_err());
        pr["reviewDecision"] = json!("APPROVED");
        assessment["possible_duplicates"] = json!([11]);
        assert!(stale_base_candidate(&pr, &assessment).is_err());
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
        let wrong_head = "ffffffffffffffffffffffffffffffffffffffff";
        job["d"]["ci_evidence"]["candidate"] = json!(wrong_head);
        assert!(!local_proof(&job, sha, base, tree, ci));
        job["d"]["ci_evidence"]["candidate"] = json!(sha);
        job["d"]["ci_evidence"]
            .as_object_mut()
            .unwrap()
            .remove("candidate");
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
