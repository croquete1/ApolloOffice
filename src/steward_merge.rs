//! Merge conservador e opt-in. Por omissão nunca integra nada.
//! Cada operação é revalidada contra o GitHub no HEAD exato e sem bypass.
use crate::app::Ax;
use crate::pa;
use crate::sh::{apip_counted, apip_unique, ghj};
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

fn safe_changed_file(file: &Value) -> bool {
    file["filename"].as_str().is_some_and(allowed_file)
        && if file["status"] == "renamed"
            || file["status"] == "copied"
            || !file["previous_filename"].is_null()
        {
            file["previous_filename"].as_str().is_some_and(allowed_file)
        } else {
            true
        }
}

async fn all_checks(repo: &str, head: &str) -> Result<(Value, Value), String> {
    let checks = apip_counted(
        &format!("repos/{repo}/commits/{head}/check-runs?per_page=100"),
        "check_runs",
        None,
    )
    .await?;
    let statuses = apip_counted(
        &format!("repos/{repo}/commits/{head}/status?per_page=100"),
        "statuses",
        Some(head),
    )
    .await?;
    Ok((checks, statuses))
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
fn valid_merge_activation(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn mergecheck_proof_current(job: &Value, activation: &str) -> bool {
    job["kind"] != "mergecheck"
        || (valid_merge_activation(activation) && job["d"]["merge_activation"] == activation)
}

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

// Read policy and proof from a single SQLite snapshot. The caller repeats
// this check after remote preflight, immediately before requesting merge.
fn current_local_proof(
    db: &crate::db::Db,
    repo: &str,
    number: i64,
    activation: &str,
    head: &str,
    base: &str,
    tree: &str,
) -> rusqlite::Result<bool> {
    use rusqlite::OptionalExtension;
    let mut c = db.0.lock().unwrap();
    let tx = c.transaction()?;
    let command: Option<String> = tx
        .query_row(
            "select trim(ci) from rp where full=? and on_=1 and auto_merge=1 and local_merge=1 and merge_activation=?",
            rusqlite::params![repo, activation],
            |row| row.get(0),
        )
        .optional()?;
    let Some(command) = command.filter(|s| !s.is_empty()) else {
        return Ok(false);
    };
    if !valid_merge_activation(activation) || !valid_sha(tree) {
        return Ok(false);
    }
    let found = {
        let mut stmt = tx.prepare(
            "select kind,st,d from jobs where repo=? and pr=? and kind in ('fix','review','mergecheck') and st='done'",
        )?;
        let mut rows = stmt.query(rusqlite::params![repo, number])?;
        let mut found = false;
        while let Some(row) = rows.next()? {
            let kind: String = row.get(0)?;
            let state: String = row.get(1)?;
            let raw: String = row.get(2)?;
            let data: Value =
                serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let job = json!({"kind":kind,"st":state,"d":data});
            if mergecheck_proof_current(&job, activation)
                && local_proof(&job, head, base, tree, &command)
            {
                found = true;
            }
        }
        found
    };
    tx.commit()?;
    Ok(found)
}

fn remote_checks_clean(checks: &Value, status: &Value) -> bool {
    let Some(runs) = checks["check_runs"].as_array() else {
        return false;
    };
    let count = checks["total_count"].as_u64().unwrap_or(u64::MAX);
    if count != runs.len() as u64 {
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
    if count == 0 || count != runs.len() as u64 {
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

// Missing, repeated or unresolved threads never authorize a merge.
#[derive(Default)]
struct ThreadScan {
    ids: std::collections::BTreeSet<String>,
    cursors: std::collections::BTreeSet<String>,
    total: Option<u64>,
}

impl ThreadScan {
    fn accept(&mut self, threads: &Value) -> Result<Option<String>, &'static str> {
        let total = threads["totalCount"]
            .as_u64()
            .ok_or("missing thread count")?;
        if self.total.is_some_and(|previous| previous != total) {
            return Err("thread count changed during pagination");
        }
        self.total = Some(total);
        let nodes = threads["nodes"]
            .as_array()
            .ok_or("missing review threads")?;
        for node in nodes {
            let id = node["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("missing thread id")?;
            if node["isResolved"] != true || !self.ids.insert(id.to_string()) {
                return Err("unresolved, invalid or repeated review thread");
            }
        }
        match threads["pageInfo"]["hasNextPage"].as_bool() {
            Some(false) if self.ids.len() as u64 == total => Ok(None),
            Some(true) if !nodes.is_empty() && (self.ids.len() as u64) < total => {
                let cursor = threads["pageInfo"]["endCursor"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("missing thread cursor")?;
                if !self.cursors.insert(cursor.to_string()) {
                    return Err("repeated thread cursor");
                }
                Ok(Some(cursor.to_string()))
            }
            _ => Err("incomplete thread inventory"),
        }
    }
}

async fn review_threads_resolved(repo: &str, number: i64) -> Result<bool, String> {
    let Some((owner, name)) = repo.split_once('/') else {
        return Ok(false);
    };
    let query = r#"query($owner:String!,$name:String!,$number:Int!,$cursor:String){repository(owner:$owner,name:$name){pullRequest(number:$number){reviewThreads(first:100,after:$cursor){totalCount nodes{id isResolved} pageInfo{hasNextPage endCursor}}}}}"#;
    let query_arg = format!("query={query}");
    let owner_arg = format!("owner={owner}");
    let name_arg = format!("name={name}");
    let number_arg = format!("number={number}");
    let mut scan = ThreadScan::default();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = vec![
            "api",
            "graphql",
            "-f",
            &query_arg,
            "-F",
            &owner_arg,
            "-F",
            &name_arg,
            "-F",
            &number_arg,
        ];
        let cursor_arg = cursor.as_ref().map(|value| format!("cursor={value}"));
        if let Some(value) = cursor_arg.as_deref() {
            args.extend(["-f", value]);
        }
        let data = ghj(&args, 45).await?;
        if !data["errors"].is_null() {
            return Ok(false);
        }
        match scan.accept(&data["data"]["repository"]["pullRequest"]["reviewThreads"]) {
            Ok(Some(next)) => cursor = Some(next),
            Ok(None) => return Ok(true),
            Err(_) => return Ok(false),
        }
    }
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
fn owned_update_target(detail: &Value, repo: &str, base: &str, login: &str) -> bool {
    let pr_base = detail["base"]["sha"].as_str().unwrap_or("");
    let head = detail["head"]["sha"].as_str().unwrap_or("");
    !login.is_empty()
        && detail["state"] == "open"
        && detail["draft"] == false
        && detail["user"]["login"] == login
        && detail["head"]["repo"]["full_name"] == repo
        && detail["base"]["repo"]["full_name"] == repo
        && detail["base"]["ref"] == base
        && detail["head"]["ref"]
            .as_str()
            .is_some_and(|branch| !branch.is_empty() && branch != base)
        && valid_sha(pr_base)
        && valid_sha(head)
}

fn can_update_branch(
    detail: &Value,
    repo: &str,
    base: &str,
    current_base: &str,
    login: &str,
) -> bool {
    owned_update_target(detail, repo, base, login)
        && valid_sha(current_base)
        && detail["base"]["sha"] != current_base
}

fn update_policy_allows(
    a: &Ax,
    repo: &str,
    number: i64,
    local: bool,
    activation: &str,
) -> Result<bool, String> {
    if a.hold("watcher") || a.drain.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(false);
    }
    a.db.try_n(
        "select count(*) from rp where full=? and on_=1 and auto_merge=1 and merge_activation=?
         and (?=0 or local_merge=1)
         and not exists(select 1 from jobs where repo=rp.full and pr=?
           and kind in ('review','rebase','mergecheck') and st not in ('done','drop'))",
        pa![
            repo.to_string(),
            activation.to_string(),
            local as i64,
            number
        ],
    )
    .map(|count| count == 1)
    .map_err(|error| format!("branch update policy unavailable: {error}"))
}

// Both update paths reserve the exact attempt before remote I/O. A storage
// error, duplicate reservation or revoked policy never authorizes a PUT.
fn reserve_branch_update(
    db: &crate::db::Db,
    repo: &str,
    number: i64,
    activation: &str,
    local: bool,
    target: (&str, &str),
    at: i64,
) -> rusqlite::Result<bool> {
    use rusqlite::OptionalExtension;
    let (head, base) = target;
    if !valid_merge_activation(activation) || !valid_sha(head) || !valid_sha(base) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut c = db.0.lock().unwrap();
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let policy = "select exists(select 1 from rp where full=?1 and on_=1 and auto_merge=1 and merge_activation=?2
        and (?3=0 or local_merge=1)
        and not exists(select 1 from jobs where repo=rp.full and pr=?4
          and kind in ('review','rebase','mergecheck') and st not in ('done','drop')))";
    let authorized: bool = tx.query_row(
        policy,
        rusqlite::params![repo, activation, local as i64, number],
        |row| row.get(0),
    )?;
    if !authorized {
        return Ok(false);
    }
    let key = format!("steward:update:{repo}#{number}");
    let previous: Option<String> = tx
        .query_row("select v from kv where k=?", [&key], |row| row.get(0))
        .optional()?;
    if let Some(previous) = previous {
        let previous: Value =
            serde_json::from_str(&previous).map_err(|_| rusqlite::Error::InvalidQuery)?;
        if update_retry_deferred(&previous, head, base, at) {
            return Ok(false);
        }
    }
    let data = branch_update_reservation(activation, target, at).to_string();
    let changed = tx.execute(
        "insert into kv(k,v) values(?,?) on conflict(k) do update set v=excluded.v",
        rusqlite::params![key, data],
    )?;
    let persisted: bool = tx.query_row(
        "select exists(select 1 from kv where k=? and v=?)",
        rusqlite::params![key, data],
        |row| row.get(0),
    )?;
    let authorized: bool = tx.query_row(
        policy,
        rusqlite::params![repo, activation, local as i64, number],
        |row| row.get(0),
    )?;
    if changed != 1 || !persisted || !authorized {
        return Err(rusqlite::Error::InvalidQuery);
    }
    tx.commit()?;
    Ok(true)
}

fn branch_update_reservation(activation: &str, target: (&str, &str), at: i64) -> Value {
    json!({"head":target.0,"base":target.1,"at":at,"status":"attempted","merge_activation":activation})
}

// The update endpoint accepts a request asynchronously. A successful HTTP
// response is not proof of a new HEAD; an error can be an ambiguous outcome.
fn record_branch_update_outcome(
    db: &crate::db::Db,
    repo: &str,
    number: i64,
    reservation: &Value,
    error: Option<&str>,
) -> rusqlite::Result<bool> {
    let mut outcome = reservation.clone();
    outcome["status"] = json!(if error.is_some() {
        "uncertain"
    } else {
        "accepted"
    });
    if let Some(error) = error {
        outcome["error"] = json!(crate::sh::head(error, 500));
    }
    replace_branch_update_record(db, repo, number, reservation, &outcome)
}

fn replace_branch_update_record(
    db: &crate::db::Db,
    repo: &str,
    number: i64,
    reservation: &Value,
    outcome: &Value,
) -> rusqlite::Result<bool> {
    let mut c = db.0.lock().unwrap();
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let key = format!("steward:update:{repo}#{number}");
    let previous = reservation.to_string();
    let eligible: bool = tx.query_row(
        "select exists(select 1 from kv where k=? and v=?)",
        rusqlite::params![key, previous],
        |row| row.get(0),
    )?;
    if !eligible {
        return Ok(false);
    }
    let data = outcome.to_string();
    let changed = tx.execute(
        "update kv set v=? where k=? and v=?",
        rusqlite::params![data, key, previous],
    )?;
    let persisted: bool = tx.query_row(
        "select exists(select 1 from kv where k=? and v=?)",
        rusqlite::params![key, data],
        |row| row.get(0),
    )?;
    if changed != 1 || !persisted {
        return Err(rusqlite::Error::InvalidQuery);
    }
    tx.commit()?;
    Ok(true)
}

async fn submit_reserved_branch_update(
    a: &Ax,
    repo: &str,
    number: i64,
    reservation: &Value,
) -> Result<(), String> {
    let head = reservation["head"]
        .as_str()
        .ok_or("missing reserved HEAD")?;
    let response = ghj(
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
    .await;
    let error = response.as_ref().err().map(String::as_str);
    if !record_branch_update_outcome(&a.db, repo, number, reservation, error)
        .map_err(|error| format!("branch update outcome unavailable: {error}"))?
    {
        return Err("branch update reservation replaced before outcome recording".into());
    }
    a.say(
        "watcher",
        &format!(
            "{repo}#{number}: {}",
            if error.is_some() {
                "resultado da atualização incerto; aguarda nova observação do GitHub"
            } else {
                "pedido de atualização aceite; aguarda novo HEAD e validação"
            }
        ),
        None,
    );
    a.steward_wake.notify_one();
    Ok(())
}

// Only an observed descendant containing BOTH reserved commits is evidence
// of integration. This does not attribute the change to our HTTP request.
fn observed_branch_update(
    previous: &Value,
    new_head: &str,
    old_comparison: &Value,
    base_comparison: &Value,
    at: i64,
) -> Option<Value> {
    let old_head = previous["head"].as_str()?;
    let base = previous["base"].as_str()?;
    if !matches!(
        previous["status"].as_str(),
        Some("attempted" | "accepted" | "uncertain")
    ) || !valid_sha(old_head)
        || !valid_sha(base)
        || !valid_sha(new_head)
        || old_head == new_head
        || old_comparison["status"] != "ahead"
        || old_comparison["merge_base_commit"]["sha"] != old_head
        || !matches!(
            base_comparison["status"].as_str(),
            Some("ahead" | "identical")
        )
        || base_comparison["merge_base_commit"]["sha"] != base
    {
        return None;
    }
    let mut observed = previous.clone();
    observed["status"] = json!("observed");
    observed["observed_head"] = json!(new_head);
    observed["observed_at"] = json!(at);
    Some(observed)
}

fn load_branch_update(
    db: &crate::db::Db,
    repo: &str,
    number: i64,
) -> rusqlite::Result<Option<Value>> {
    use rusqlite::OptionalExtension;
    let c = db.0.lock().unwrap();
    let raw: Option<String> = c
        .query_row(
            "select v from kv where k=?",
            [format!("steward:update:{repo}#{number}")],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|value| serde_json::from_str(&value).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()
}

async fn reconcile_branch_update(a: &Ax, repo: &str, number: i64) -> Result<(), String> {
    let Some(previous) = load_branch_update(&a.db, repo, number)
        .map_err(|error| format!("branch update reconciliation unavailable: {error}"))?
    else {
        return Ok(());
    };
    let previous = &previous;
    if !matches!(
        previous["status"].as_str(),
        Some("attempted" | "accepted" | "uncertain")
    ) {
        return Ok(());
    }
    let old = previous["head"]
        .as_str()
        .filter(|sha| valid_sha(sha))
        .ok_or("invalid reserved HEAD")?;
    let base = previous["base"]
        .as_str()
        .filter(|sha| valid_sha(sha))
        .ok_or("invalid reserved base")?;
    let detail = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 40).await?;
    let Some(new_head) = detail["head"]["sha"].as_str().filter(|sha| valid_sha(sha)) else {
        return Ok(());
    };
    if detail["number"] != number
        || detail["state"] != "open"
        || detail["draft"] != false
        || detail["head"]["repo"]["full_name"] != repo
        || detail["base"]["repo"]["full_name"] != repo
        || new_head == old
    {
        return Ok(());
    }
    let old_comparison = ghj(
        &["api", &format!("repos/{repo}/compare/{old}...{new_head}")],
        40,
    )
    .await?;
    let base_comparison = ghj(
        &["api", &format!("repos/{repo}/compare/{base}...{new_head}")],
        40,
    )
    .await?;
    let Some(observed) = observed_branch_update(
        previous,
        new_head,
        &old_comparison,
        &base_comparison,
        crate::db::now(),
    ) else {
        return Ok(());
    };
    if replace_branch_update_record(&a.db, repo, number, previous, &observed)
        .map_err(|error| format!("branch update observation unavailable: {error}"))?
    {
        a.say("watcher", &format!("{repo}#{number}: novo HEAD observado inclui os dois commits reservados; validação obrigatória"), None);
    }
    Ok(())
}

// Both update paths require a fresh owned PR and the exact observed target.
// The GitHub PUT still carries expected_head_sha; it cannot fence a base race.
fn same_update_target(before: &Value, after: &Value, repo: &str, base: &str, login: &str) -> bool {
    owned_update_target(before, repo, base, login)
        && owned_update_target(after, repo, base, login)
        && before["head"]["sha"] == after["head"]["sha"]
        && before["head"]["ref"] == after["head"]["ref"]
        && before["base"]["sha"] == after["base"]["sha"]
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
    if a.hold("watcher") || !crate::db::valid_git_sha(head) || !crate::db::valid_git_sha(base) {
        return false;
    }
    let inserted = match admit_recheck(&a.db, repo, number, head, base, detail) {
        Ok(inserted) => inserted,
        Err(error) => {
            a.say(
                "watcher",
                &format!("{repo}#{number}: recheck admission failed: {error}"),
                None,
            );
            false
        }
    };
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

// Keep the original fix evidence and admission predicates in the same write
// transaction. A trigger must not silently remove or rewrite the new job.
fn admit_recheck(
    db: &crate::db::Db,
    repo: &str,
    number: i64,
    head: &str,
    base: &str,
    detail: &Value,
) -> rusqlite::Result<bool> {
    use rusqlite::OptionalExtension;
    let mut c = db.0.lock().unwrap();
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let author: Option<String> = tx
        .query_row(
            "select json_extract(d,'$.author') from jobs where repo=? and pr=? and kind='fix' and st='done' order by id desc limit 1",
            rusqlite::params![repo, number],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let branch = detail["head"]["ref"].as_str().unwrap_or("");
    let Some(author) = author.filter(|s| !s.trim().is_empty()) else {
        return Ok(false);
    };
    if branch.trim().is_empty()
        || !crate::db::valid_git_sha(head)
        || !crate::db::valid_git_sha(base)
    {
        return Ok(false);
    }
    let activation: Option<String> = tx
        .query_row(
            "select merge_activation from rp where full=? and on_=1 and auto_merge=1 and local_merge=1",
            [repo],
            |row| row.get(0),
        )
        .optional()?;
    let Some(activation) = activation.filter(|s| valid_merge_activation(s)) else {
        return Ok(false);
    };
    let data = json!({
        "head":repo,
        "branch":branch,
        "expected_head":head,
        "expected_base":base,
        "merge_activation":activation,
        "author":author,
        "body":detail["body"]
    })
    .to_string();
    let title = detail["title"].as_str().unwrap_or("Revalidar PR");
    let url = detail["html_url"].as_str().unwrap_or("");
    let t = crate::db::now();
    let eligible: bool = tx.query_row(
        "select exists (
             select 1 from rp where full=?1 and on_=1 and auto_merge=1 and local_merge=1
         )
         and not exists (
             select 1 from jobs where repo=?1 and pr=?2 and kind in ('review','rebase','mergecheck')
               and st not in ('done','drop')
         )
         and not exists (
             select 1 from jobs where repo=?1 and kind='mergecheck' and st not in ('done','drop')
         )
         and not exists (
             select 1 from jobs where repo=?1 and pr=?2 and kind='mergecheck'
               and json_extract(d,'$.expected_head')=?3
               and json_extract(d,'$.expected_base')=?4
               and json_extract(d,'$.merge_activation')=?5
         )",
        rusqlite::params![repo, number, head, base, activation],
        |row| row.get(0),
    )?;
    if !eligible {
        return Ok(false);
    }
    let changed = tx.execute(
        "insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1)
         select ?,?,'mergecheck',?,?,'ready','Revalidar CI e Reviewer antes de merge',?,?,-5.0,?,?
         where exists (
             select 1 from rp where full=? and on_=1 and auto_merge=1 and local_merge=1
         )
         and not exists (
             select 1 from jobs where repo=? and pr=? and kind in ('review','rebase','mergecheck')
               and st not in ('done','drop')
         )
         and not exists (
             select 1 from jobs where repo=? and kind='mergecheck' and st not in ('done','drop')
         )
         and not exists (
             select 1 from jobs where repo=? and pr=? and kind='mergecheck'
               and json_extract(d,'$.expected_head')=?
               and json_extract(d,'$.expected_base')=?
               and json_extract(d,'$.merge_activation')=?
         )",
        rusqlite::params![
            repo, number, title, url, data, number, t, t, repo, repo, number, repo, repo, number,
            head, base, activation
        ],
    )?;
    let job = tx.last_insert_rowid();
    let persisted: bool = tx.query_row(
        "select exists(select 1 from jobs where id=?1 and repo=?2 and num=?3 and pr=?3 and kind='mergecheck' and st='ready' and title=?4 and url=?5 and d=?6 and why='Revalidar CI e Reviewer antes de merge' and prio=-5.0 and t0=?7 and t1=?7)
         and exists(select 1 from rp where full=?2 and on_=1 and auto_merge=1 and local_merge=1 and merge_activation=?8)",
        rusqlite::params![job, repo, number, title, url, data, t, activation],
        |row| row.get(0),
    )?;
    if changed != 1 || !persisted {
        return Err(rusqlite::Error::InvalidQuery);
    }
    tx.commit()?;
    crate::app::chg();
    Ok(true)
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
        if app.drain.load(std::sync::atomic::Ordering::SeqCst) || app.hold("watcher") {
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
    let policy =
        a.db.try_all(
            "select local_merge,merge_activation from rp where full=? and on_=1 and auto_merge=1",
            pa![repo.to_string()],
        )
        .map_err(|error| format!("merge policy unavailable: {error}"))?;
    let Some(policy) = policy.first() else {
        return Ok(false);
    };
    let Some(activation) = policy["merge_activation"]
        .as_str()
        .filter(|s| valid_merge_activation(s))
    else {
        return Ok(false);
    };
    let local_enabled = policy["local_merge"] == 1;
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
            let latest = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 40).await?;
            let fresh_base = ghj(&["api", &format!("repos/{repo}/branches/{base}")], 40).await?;
            if !same_update_target(&detail, &latest, repo, base, &login)
                || !can_update_branch(&latest, repo, base, current_base, &login)
                || fresh_base["commit"]["sha"] != current_base
                || !update_policy_allows(a, repo, number, false, activation)?
            {
                return Ok(false);
            }
            let at = crate::db::now();
            if !reserve_branch_update(
                &a.db,
                repo,
                number,
                activation,
                false,
                (&head, current_base),
                at,
            )
            .map_err(|error| format!("branch update reservation unavailable: {error}"))?
                || !update_policy_allows(a, repo, number, false, activation)?
            {
                return Ok(false);
            }
            let reservation = branch_update_reservation(activation, (&head, current_base), at);
            submit_reserved_branch_update(a, repo, number, &reservation).await?;
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
    // Preserve the existing conservative change-size limit; it is independent of pagination.
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
    if paths.len() as u64 != changed || !paths.iter().all(safe_changed_file) {
        return Ok(false);
    }
    let (checks, statuses) = all_checks(repo, &head).await?;
    if !check_gate(&checks, &statuses, mode) {
        return Ok(false);
    }
    let reviews =
        json!(apip_unique(&format!("repos/{repo}/pulls/{number}/reviews?per_page=100")).await?);
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
    let mut local_evidence = None;
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
                let update_key = format!("steward:update:{repo}#{number}");
                if update_retry_deferred(&a.db.kg(&update_key), &head, base_sha, crate::db::now()) {
                    return Ok(false);
                }
                let login = crate::gh::login(a).await;
                let latest = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 40).await?;
                let fresh_base =
                    ghj(&["api", &format!("repos/{repo}/branches/{base}")], 40).await?;
                if !same_update_target(&detail, &latest, repo, base, &login)
                    || fresh_base["commit"]["sha"] != base_sha
                    || !update_policy_allows(a, repo, number, true, activation)?
                {
                    return Ok(false);
                }
                // Reserve the backoff before an external attempt, including an
                // ambiguous response, rather than retrying on every scan.
                let at = crate::db::now();
                if !reserve_branch_update(
                    &a.db,
                    repo,
                    number,
                    activation,
                    true,
                    (&head, base_sha),
                    at,
                )
                .map_err(|error| format!("branch update reservation unavailable: {error}"))?
                    || !update_policy_allows(a, repo, number, true, activation)?
                {
                    return Ok(false);
                }
                let reservation = branch_update_reservation(activation, (&head, base_sha), at);
                submit_reserved_branch_update(a, repo, number, &reservation).await?;
            }
            return Ok(false);
        }

        let commit = ghj(&["api", &format!("repos/{repo}/git/commits/{head}")], 45).await?;
        let Some(tree) = commit["tree"]["sha"].as_str() else {
            return Ok(false);
        };
        if !current_local_proof(&a.db, repo, number, activation, &head, base_sha, tree)
            .map_err(|error| format!("local merge proof database unavailable: {error}"))?
        {
            queue_recheck(a, repo, number, &head, base_sha, &detail);
            return Ok(false);
        }
        local_evidence = Some((activation.to_string(), tree.to_string()));
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
        let reviews =
            json!(apip_unique(&format!("repos/{repo}/pulls/{number}/reviews?per_page=100")).await?);
        let threads_resolved = review_threads_resolved(repo, number).await?;
        let (checks, statuses) = all_checks(repo, &head).await?;
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
    if let Some((activation, tree)) = local_evidence {
        if !current_local_proof(&a.db, repo, number, &activation, &head, base_sha, &tree)
            .map_err(|error| format!("final local merge proof unavailable: {error}"))?
        {
            return Ok(false);
        }
    }
    if !update_policy_allows(a, repo, number, mode == MergeMode::Local, activation)? {
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
    // A removal or policy change may have occurred before acquiring the permit.
    if !matches!(
        a.db.try_n(
            "select auto_merge from rp where full=? and on_=1",
            pa![repo.to_string()],
        ),
        Ok(1)
    ) {
        return;
    }
    for (pr, status) in listed.iter().zip(assessed.iter()) {
        if let Some(number) = pr["number"].as_i64().filter(|n| *n > 0) {
            if let Err(error) = reconcile_branch_update(a, repo, number).await {
                a.db.ks(
                    &format!("steward:merge_error:{repo}"),
                    &json!({"pr":number,"reason":crate::sh::head(&error, 500)}),
                );
                continue;
            }
        }
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
    fn branch_update_rejects_changed_target_and_revoked_policy() {
        let repo = "o/r";
        let head = "a".repeat(40);
        let base = "b".repeat(40);
        let pr = json!({
            "state":"open","draft":false,"user":{"login":"owner"},
            "head":{"sha":head,"ref":"fix/task","repo":{"full_name":repo}},
            "base":{"sha":base,"ref":"main","repo":{"full_name":repo}}
        });
        assert!(same_update_target(&pr, &pr, repo, "main", "owner"));
        for (path, value) in [
            ("/head/sha", json!("c".repeat(40))),
            ("/base/sha", json!("c".repeat(40))),
            ("/head/ref", json!("fix/other")),
            ("/head/ref", Value::Null),
            ("/head/repo/full_name", json!("other/r")),
            ("/draft", json!(true)),
            ("/state", json!("closed")),
        ] {
            let mut changed = pr.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert!(!same_update_target(&pr, &changed, repo, "main", "owner"));
        }
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            47301,
            "apollo-office".into(),
        ));
        assert!(!update_policy_allows(&a, repo, 1, false, "").unwrap());
        a.db.run(
            "insert into rp(full,on_,auto_merge,local_merge) values('o/r',1,1,0)",
            pa![],
        );
        let policy = a.db.one("select merge_activation from rp", pa![]).unwrap();
        let activation = policy["merge_activation"].as_str().unwrap();
        assert!(update_policy_allows(&a, repo, 1, false, activation).unwrap());
        assert!(!update_policy_allows(&a, repo, 1, true, activation).unwrap());
        a.db.run("update rp set local_merge=1", pa![]);
        assert!(!update_policy_allows(&a, repo, 1, false, activation).unwrap());
        let policy = a.db.one("select merge_activation from rp", pa![]).unwrap();
        let activation = policy["merge_activation"].as_str().unwrap();
        assert!(update_policy_allows(&a, repo, 1, true, activation).unwrap());
        a.db.run(
            "insert into jobs(repo,pr,kind,st) values('o/r',1,'review','ready')",
            pa![],
        );
        assert!(!update_policy_allows(&a, repo, 1, true, activation).unwrap());
        assert!(update_policy_allows(&a, repo, 2, true, activation).unwrap());
        a.db.run("update jobs set st='done'", pa![]);
        a.db.run("update rp set auto_merge=0", pa![]);
        assert!(!update_policy_allows(&a, repo, 1, true, activation).unwrap());
    }

    #[test]
    fn branch_update_reservation_is_atomic_and_epoch_bound() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into rp(full,on_,auto_merge,local_merge) values('o/r',1,1,1)",
            pa![],
        );
        let policy = db.one("select merge_activation from rp", pa![]).unwrap();
        let activation = policy["merge_activation"].as_str().unwrap();
        let head = "a".repeat(40);
        let base = "b".repeat(40);
        let reserve =
            |at| reserve_branch_update(&db, "o/r", 1, activation, true, (&head, &base), at);
        for fault in [
            "create trigger fail_reserve before insert on kv when new.k glob 'steward:update:*' begin select raise(ignore); end;",
            "create trigger fail_reserve after insert on kv when new.k glob 'steward:update:*' begin update kv set v='{}' where k=new.k; end;",
            "create trigger fail_reserve after insert on kv when new.k glob 'steward:update:*' begin update rp set auto_merge=0; end;",
        ] {
            db.0.lock().unwrap().execute_batch(fault).unwrap();
            assert!(reserve(100).is_err());
            assert_eq!(db.n("select count(*) from kv where k glob 'steward:update:*'", pa![]), 0);
            assert_eq!(db.n("select auto_merge from rp", pa![]), 1);
            db.run("drop trigger fail_reserve", pa![]);
        }
        assert!(reserve(100).unwrap());
        assert!(!reserve(101).unwrap());
        assert!(reserve(300_100).unwrap());
        db.run("update rp set auto_merge=0", pa![]);
        db.run("update rp set auto_merge=1", pa![]);
        assert!(!reserve(600_100).unwrap());
    }

    #[test]
    fn branch_update_outcome_preserves_reservation_and_rejects_stale_writes() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into rp(full,on_,auto_merge,local_merge) values('o/r',1,1,1)",
            pa![],
        );
        let policy = db.one("select merge_activation from rp", pa![]).unwrap();
        let activation = policy["merge_activation"].as_str().unwrap();
        let head = "a".repeat(40);
        let base = "b".repeat(40);
        let target = (head.as_str(), base.as_str());
        let key = "steward:update:o/r#1";
        assert!(reserve_branch_update(&db, "o/r", 1, activation, true, target, 100).unwrap());
        let first = branch_update_reservation(activation, target, 100);
        db.run("create trigger ignore_outcome before update on kv when new.k glob 'steward:update:*' begin select raise(ignore); end;", pa![]);
        assert!(record_branch_update_outcome(&db, "o/r", 1, &first, None).is_err());
        assert_eq!(db.kg(key), first);
        db.run("drop trigger ignore_outcome", pa![]);
        assert!(record_branch_update_outcome(&db, "o/r", 1, &first, Some("timeout")).unwrap());
        let uncertain = db.kg(key);
        assert_eq!(uncertain["status"], "uncertain");
        assert_eq!(uncertain["at"], 100);
        assert!(!reserve_branch_update(&db, "o/r", 1, activation, true, target, 101).unwrap());
        assert!(reserve_branch_update(&db, "o/r", 1, activation, true, target, 300_100).unwrap());
        let second = branch_update_reservation(activation, target, 300_100);
        assert!(!record_branch_update_outcome(&db, "o/r", 1, &first, None).unwrap());
        assert_eq!(db.kg(key), second);
        assert!(record_branch_update_outcome(&db, "o/r", 1, &second, None).unwrap());
        let accepted = db.kg(key);
        assert_eq!(accepted["status"], "accepted");
        assert_eq!(accepted["at"], 300_100);
        assert_eq!(accepted["merge_activation"], activation);
    }

    #[test]
    fn observed_update_requires_both_ancestors_and_exact_reservation() {
        let db = crate::db::Db::mem();
        let old = "a".repeat(40);
        let base = "b".repeat(40);
        let head = "c".repeat(40);
        let previous = branch_update_reservation(&"d".repeat(32), (&old, &base), 100);
        let key = "steward:update:o/r#1";
        let old_compare = json!({"status":"ahead","merge_base_commit":{"sha":old}});
        let base_compare = json!({"status":"ahead","merge_base_commit":{"sha":base}});
        assert!(
            observed_branch_update(&previous, &old, &old_compare, &base_compare, 200).is_none()
        );
        assert!(observed_branch_update(
            &previous,
            &head,
            &json!({"status":"ahead"}),
            &base_compare,
            200
        )
        .is_none());
        assert!(observed_branch_update(
            &previous,
            &head,
            &old_compare,
            &json!({"status":"diverged","merge_base_commit":{"sha":base}}),
            200
        )
        .is_none());
        let observed =
            observed_branch_update(&previous, &head, &old_compare, &base_compare, 200).unwrap();
        assert_eq!(observed["status"], "observed");
        assert_eq!(observed["observed_head"], head);
        assert_eq!(observed["at"], 100);
        assert!(load_branch_update(&db, "o/r", 1).unwrap().is_none());
        db.ks(key, &previous);
        assert_eq!(
            load_branch_update(&db, "o/r", 1).unwrap(),
            Some(previous.clone())
        );
        db.run("create trigger ignore_observation before update on kv when new.k glob 'steward:update:*' begin select raise(ignore); end;", pa![]);
        assert!(replace_branch_update_record(&db, "o/r", 1, &previous, &observed).is_err());
        assert_eq!(db.kg(key), previous);
        db.run("drop trigger ignore_observation", pa![]);
        assert!(replace_branch_update_record(&db, "o/r", 1, &previous, &observed).unwrap());
        assert_eq!(db.kg(key), observed);
        assert!(!replace_branch_update_record(&db, "o/r", 1, &previous, &observed).unwrap());
        assert!(
            observed_branch_update(&observed, &head, &old_compare, &base_compare, 300).is_none()
        );
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
        a.lv.lock().unwrap().o.man = true;
        assert!(MergePermit::acquire(&a, "croquete1/repo-e2e").is_none());
        a.lv.lock().unwrap().o.man = false;
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
        assert!(!queue_recheck(&a, repo, 42, head, base, &detail));
        a.db.run(
            "insert into rp(full,on_,auto_merge,local_merge) values(?,1,1,0)",
            pa![repo.to_string()],
        );
        assert!(!queue_recheck(&a, repo, 42, head, base, &detail));
        a.db.run("update rp set local_merge=1", pa![]);
        assert!(!queue_recheck(&a, repo, 42, "invalid", base, &detail));
        assert!(!queue_recheck(&a, repo, 42, head, "invalid", &detail));
        for kind in ["review", "rebase"] {
            a.db.run(
                "insert into jobs(repo,pr,kind,st) values(?,42,?,'ready')",
                pa![repo.to_string(), kind.to_string()],
            );
            assert!(!queue_recheck(&a, repo, 42, head, base, &detail));
            a.db.run(
                "update jobs set st='done' where repo=? and pr=42 and kind=?",
                pa![repo.to_string(), kind.to_string()],
            );
        }
        a.db.run("update rp set auto_merge=0", pa![]);
        assert!(!queue_recheck(&a, repo, 42, head, base, &detail));
        a.db.run("update rp set auto_merge=1,on_=0", pa![]);
        assert!(!queue_recheck(&a, repo, 42, head, base, &detail));
        a.db.run("update rp set on_=1", pa![]);
        for fault in [
            "create trigger fail_recheck before insert on jobs when new.kind='mergecheck' begin select raise(ignore); end;",
            "create trigger fail_recheck after insert on jobs when new.kind='mergecheck' begin update jobs set d='{}' where id=new.id; end;",
            "create trigger fail_recheck after insert on jobs when new.kind='mergecheck' begin delete from jobs where id=new.id; end;",
            "create trigger fail_recheck after insert on jobs when new.kind='mergecheck' begin update rp set local_merge=0; end;",
        ] {
            a.db.0.lock().unwrap().execute_batch(fault).unwrap();
            assert!(admit_recheck(&a.db, repo, 42, head, base, &detail).is_err());
            assert_eq!(
                a.db.n("select count(*) from jobs where kind='mergecheck'", pa![]),
                0
            );
            assert_eq!(a.db.n("select local_merge from rp", pa![]), 1);
            a.db.run("drop trigger fail_recheck", pa![]);
        }
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
        let first = job["d"]["merge_activation"].as_str().unwrap();
        assert!(mergecheck_proof_current(&job, first));
        a.db.run("update jobs set st='done' where kind='mergecheck'", pa![]);
        a.db.run("update rp set local_merge=0", pa![]);
        a.db.run("update rp set local_merge=1", pa![]);
        let policy = a.db.one("select merge_activation from rp", pa![]).unwrap();
        let current = policy["merge_activation"].as_str().unwrap();
        assert_ne!(current, first);
        assert!(!mergecheck_proof_current(&job, current));
        assert!(queue_recheck(&a, repo, 42, head, base, &detail));
        assert!(!queue_recheck(&a, repo, 42, head, base, &detail));
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
    fn final_local_proof_rejects_removed_or_revoked_evidence() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into rp(full,on_,auto_merge,local_merge,ci) values('o/r',1,1,1,'cargo test')",
            pa![],
        );
        let policy = db.one("select merge_activation from rp", pa![]).unwrap();
        let activation = policy["merge_activation"].as_str().unwrap();
        let head = "a".repeat(40);
        let base = "b".repeat(40);
        let tree = "c".repeat(40);
        let data = json!({
            "merge_activation":activation,"base":base,"published_sha":head,
            "ci_tree":tree,"publish_tree":tree,"author":"author",
            "ci_evidence":{"exit":0,"command":"cargo test","candidate":head},
            "ci_verified_at":100,"last_review":{"model":"reviewer","t":101,"verdict":{"ok":true}}
        });
        let id = db.jnew(
            "o/r",
            1,
            "mergecheck",
            "PR",
            "url",
            "done",
            "",
            data.clone(),
            Some(1),
            -5.0,
        );
        let check = || current_local_proof(&db, "o/r", 1, activation, &head, &base, &tree);
        assert!(check().unwrap());
        db.run("update jobs set st='drop' where id=?", pa![id]);
        assert!(!check().unwrap());
        db.run("update jobs set st='done',d='{}' where id=?", pa![id]);
        assert!(!check().unwrap());
        db.run("update jobs set d=? where id=?", pa![data.to_string(), id]);
        assert!(check().unwrap());
        db.run("update rp set local_merge=0", pa![]);
        db.run("update rp set local_merge=1", pa![]);
        assert!(!check().unwrap());
        let policy = db.one("select merge_activation from rp", pa![]).unwrap();
        let current = policy["merge_activation"].as_str().unwrap();
        assert!(!current_local_proof(&db, "o/r", 1, current, &head, &base, &tree).unwrap());
        db.run("delete from jobs where id=?", pa![id]);
        assert!(!current_local_proof(&db, "o/r", 1, current, &head, &base, &tree).unwrap());
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

#[cfg(test)]
mod thread_pagination_tests {
    use super::*;

    fn page(start: u64, end: u64, total: u64, next: bool, cursor: &str) -> Value {
        json!({"totalCount":total,"nodes":(start..end).map(|n| json!({"id":n.to_string(),"isResolved":true})).collect::<Vec<_>>(),
            "pageInfo":{"hasNextPage":next,"endCursor":cursor}})
    }

    #[test]
    fn resolved_threads_can_span_multiple_pages() {
        let mut scan = ThreadScan::default();
        assert_eq!(
            scan.accept(&page(0, 100, 101, true, "next")),
            Ok(Some("next".into()))
        );
        assert_eq!(scan.accept(&page(100, 101, 101, false, "end")), Ok(None));
    }

    #[test]
    fn unresolved_duplicate_or_incomplete_threads_block_merge() {
        let mut scan = ThreadScan::default();
        scan.accept(&page(0, 100, 101, true, "next")).unwrap();
        let mut unresolved = page(100, 101, 101, false, "end");
        unresolved["nodes"][0]["isResolved"] = json!(false);
        assert!(scan.accept(&unresolved).is_err());
        assert!(scan.accept(&page(99, 100, 101, false, "end")).is_err());
        assert!(ThreadScan::default()
            .accept(&page(0, 1, 2, false, "end"))
            .is_err());
        assert!(ThreadScan::default().accept(&json!({})).is_err());
    }
}

#[cfg(test)]
mod complete_inventory_tests {
    use super::*;
    #[test]
    fn checks_and_reviews_beyond_first_page_still_enforce_every_gate() {
        let head = "a".repeat(40);
        let mut checks = json!({"total_count":101,"check_runs":(1..=101).map(|id|json!({"id":id,"name":"test-suite","app":{"slug":"ci-service"},"status":"completed","conclusion":"success"})).collect::<Vec<_>>()});
        let status = json!({"statuses":[]});
        assert!(ci_verified(&checks, &status));
        checks["check_runs"][100]["conclusion"] = json!("failure");
        assert!(!ci_verified(&checks, &status));
        let mut reviews=json!((1..=101).map(|id|json!({"id":id,"state":"APPROVED","commit_id":head,"user":{"login":"maintainer","type":"User"}})).collect::<Vec<_>>());
        assert!(reviewed(&reviews, "author", &head));
        reviews[100]["state"] = json!("CHANGES_REQUESTED");
        assert!(!reviewed(&reviews, "author", &head));
    }
    #[test]
    fn renamed_sensitive_file_still_blocks_merge() {
        assert!(!safe_changed_file(
            &json!({"filename":"src/ui.ts","previous_filename":"src/auth.ts","status":"renamed"})
        ));
        assert!(!safe_changed_file(
            &json!({"filename":"src/ui.ts","status":"renamed"})
        ));
        assert!(safe_changed_file(
            &json!({"filename":"src/ui.ts","previous_filename":"src/view.ts","status":"renamed"})
        ));
    }
}
