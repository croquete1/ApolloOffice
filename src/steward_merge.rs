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

async fn branch_protection_verified(repo: &str, base: &str, sha: &str) -> Result<bool, String> {
    let branch = ghj(&["api", &format!("repos/{repo}/branches/{base}")], 40).await?;
    // O modo sem proteção fica bloqueado: não presumir ausência de regras num 403.
    if branch["protected"] != true || branch["commit"]["sha"] != sha {
        return Ok(false);
    }
    let rules = ghj(
        &["api", &format!("repos/{repo}/branches/{base}/protection")],
        40,
    )
    .await?;
    // Exigir pelo menos um check obrigatório publicado pela proteção.
    let required = &rules["required_status_checks"]["contexts"];
    Ok(required
        .as_array()
        .is_some_and(|contexts| !contexts.is_empty()))
}

async fn try_one(a: &Ax, repo: &str, listed: &Value, assessed: &Value) -> Result<bool, String> {
    let (number, head) = match candidate(listed, assessed) {
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
    if !branch_protection_verified(repo, base, base_sha).await? {
        return Ok(false);
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
    if !ci_verified(&checks, &statuses) {
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
    if !reviewed(
        &reviews,
        detail["user"]["login"].as_str().unwrap_or(""),
        &head,
    ) || !review_threads_resolved(repo, number).await?
    {
        return Ok(false);
    }

    // Reconfirmar a configuração e os SHAs imediatamente antes de um ato mutável.
    if a.hold("watcher")
        || a.db.n(
            "select auto_merge from rp where full=? and on_=1",
            pa![repo.to_string()],
        ) != 1
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

    // PUT /merge do GitHub respeita a proteção, sem --admin nem force.
    // sha=HEAD impede integrar um commit que foi alterado entretanto.
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
        assert!(ci_verified(
            &json!({"total_count":1,"check_runs":[{"status":"completed","conclusion":"success"}]}),
            &status
        ));
    }
}
