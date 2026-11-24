//! Reconciliação periódica e diagnóstico consultivo das PRs.
//! O merge é opt-in por repositório, sujeito aos gates conservadores definidos
//! em steward_merge; a triagem nunca fecha PRs nem altera branches.
use crate::app::{sl, Ax};
use crate::db::now;
use crate::pa;
use crate::sh::ghj;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

// Use the paginated REST inventory as the completeness authority. The CLI
// batch is only an enrichment optimization, never proof that all PRs were read.
const DETAIL_FIELDS: &str = "number,title,body,url,isDraft,headRefOid,baseRefName,baseRefOid,createdAt,updatedAt,mergeStateStatus,reviewDecision,statusCheckRollup,files";
const DETAIL_BATCH: usize = 1000;
type Inventory = BTreeMap<i64, (String, String, String)>;

fn inventory_row(row: &Value) -> Result<(i64, (String, String, String)), String> {
    let number = row["number"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("PR without a valid number")?;
    if row["state"] != "open" {
        return Err(format!("PR #{number} changed state during inventory"));
    }
    let sha = |v: &Value| -> Result<String, String> {
        v.as_str()
            .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(String::from)
            .ok_or_else(|| format!("PR #{number} missing valid SHA"))
    };
    let updated = row["updated_at"]
        .as_str()
        .filter(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
        .ok_or("PR without valid updated_at")?;
    Ok((
        number,
        (
            sha(&row["head"]["sha"])?,
            sha(&row["base"]["sha"])?,
            updated.to_string(),
        ),
    ))
}

fn append_inventory(inventory: &mut Inventory, rows: &[Value]) -> Result<(), String> {
    for row in rows {
        let (number, identity) = inventory_row(row)?;
        if inventory.insert(number, identity).is_some() {
            return Err(format!("Duplicate PR #{number} in paginated inventory"));
        }
    }
    Ok(())
}

async fn inventory(repo: &str) -> Result<Inventory, String> {
    let mut all = Inventory::new();
    let mut page = 1_u64;
    loop {
        let endpoint = format!(
            "repos/{repo}/pulls?state=open&sort=created&direction=asc&per_page=100&page={page}"
        );
        let data = ghj(&["api", &endpoint], 90).await?;
        let rows = data.as_array().ok_or("Invalid paginated PR inventory")?;
        if rows.len() > 100 {
            return Err("Invalid PR page size".into());
        }
        append_inventory(&mut all, rows)?;
        if rows.len() < 100 {
            return Ok(all);
        }
        page = page.checked_add(1).ok_or("PR page counter overflow")?;
    }
}

fn detail_matches(row: &Value, number: i64, identity: &(String, String, String)) -> bool {
    row["number"] == number
        && row["headRefOid"] == identity.0
        && row["baseRefOid"] == identity.1
        && row["updatedAt"] == identity.2
        && row["title"].is_string()
        && row["url"].is_string()
        && row["isDraft"].is_boolean()
        && row["baseRefName"].is_string()
        && row["statusCheckRollup"].is_array()
        && row["files"].is_array()
}

async fn complete_open_prs(repo: &str) -> Result<Vec<Value>, String> {
    let before = inventory(repo).await?;
    let mut details = BTreeMap::new();
    if !before.is_empty() {
        let limit = before.len().min(DETAIL_BATCH).to_string();
        let batch = ghj(
            &[
                "pr",
                "list",
                "-R",
                repo,
                "--state",
                "open",
                "--limit",
                &limit,
                "--json",
                DETAIL_FIELDS,
            ],
            120,
        )
        .await?;
        let rows = batch.as_array().ok_or("Invalid PR details batch")?;
        for row in rows {
            let number = row["number"].as_i64().ok_or("PR details missing number")?;
            let identity = before
                .get(&number)
                .ok_or("PR inventory changed during enrichment")?;
            if !detail_matches(row, number, identity)
                || details.insert(number, row.clone()).is_some()
            {
                return Err(format!(
                    "PR #{number} has inconsistent or duplicate details"
                ));
            }
        }
        // Beyond the batch limit, obtain the remaining details individually.
        // Any failure aborts the entire scan; no automatic actions use a partial set.
        for (number, identity) in &before {
            if details.contains_key(number) {
                continue;
            }
            let row = ghj(
                &[
                    "pr",
                    "view",
                    &number.to_string(),
                    "-R",
                    repo,
                    "--json",
                    DETAIL_FIELDS,
                ],
                60,
            )
            .await?;
            if !detail_matches(&row, *number, identity) {
                return Err(format!("PR #{number} changed or has incomplete details"));
            }
            details.insert(*number, row);
        }
    }
    if inventory(repo).await? != before {
        return Err("PR inventory changed while scanning; retry before taking actions".into());
    }
    Ok(details.into_values().collect())
}

pub(crate) const STATES: [&str; 8] = [
    "draft",
    "conflito",
    "duplicado_possivel",
    "sobreposicao",
    "ci_falhou",
    "ci_pendente",
    "revisao_pendente",
    "revisao_final_necessaria",
];

/// Counts the complete scan by the same state used to prioritize individual PRs.
pub(crate) fn state_counts(report: &[Value]) -> Value {
    let mut counts = json!({
        "draft": 0,
        "conflito": 0,
        "duplicado_possivel": 0,
        "sobreposicao": 0,
        "ci_falhou": 0,
        "ci_pendente": 0,
        "revisao_pendente": 0,
        "revisao_final_necessaria": 0
    });
    for pr in report {
        if let Some(state) = pr["state"].as_str().filter(|state| STATES.contains(state)) {
            counts[state] = json!(counts[state].as_i64().unwrap_or(0) + 1);
        }
    }
    counts
}

/// Reviews automáticas não constituem provas de testes.
fn is_review_bot(check: &Value) -> bool {
    let name = check["name"]
        .as_str()
        .or_else(|| check["context"].as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let app = check["app"]["slug"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    name.contains("coderabbit") || app.contains("coderabbit")
}

/// Exigir um nome que identifique testes de forma explícita.
/// Um check isolado de lint, build ou análise estática não certifica execução de testes.
pub(crate) fn is_test_check(check: &Value) -> bool {
    if is_review_bot(check) {
        return false;
    }
    let name = check["name"]
        .as_str()
        .or_else(|| check["context"].as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| {
            matches!(
                part,
                "test"
                    | "tests"
                    | "testing"
                    | "unit"
                    | "integration"
                    | "e2e"
                    | "smoke"
                    | "pytest"
                    | "jest"
                    | "vitest"
                    | "mocha"
                    | "playwright"
            )
        })
}

fn work_files(pr: &Value) -> BTreeSet<String> {
    pr["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["path"].as_str())
        .filter(|p| {
            !p.ends_with(".test.ts")
                && !p.ends_with(".test.tsx")
                && !p.ends_with(".spec.ts")
                && !p.ends_with(".spec.tsx")
                && !p.starts_with("docs/")
                && !p.starts_with(".github/")
                && !p.ends_with(".md")
        })
        .map(str::to_string)
        .collect()
}

pub(crate) fn issue_refs(pr: &Value) -> BTreeSet<String> {
    let source = format!(
        "{}\n{}",
        pr["title"].as_str().unwrap_or(""),
        pr["body"].as_str().unwrap_or("")
    );
    let re = regex::Regex::new(r"(?i)\b(?:fix(?:e[sd])?|close[sd]?|resolve[sd]?)\s*:?\s+#(\d+)\b")
        .unwrap();
    re.captures_iter(&source)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn check_state(pr: &Value) -> &'static str {
    let Some(checks) = pr["statusCheckRollup"].as_array() else {
        return "nao_verificado";
    };
    let checks: Vec<_> = checks.iter().filter(|c| !is_review_bot(c)).collect();
    if checks.is_empty() {
        return "nao_verificado";
    }
    if checks.iter().any(|c| {
        c["conclusion"] == "FAILURE"
            || c["conclusion"] == "CANCELLED"
            || c["conclusion"] == "TIMED_OUT"
            || c["state"] == "FAILURE"
            || c["state"] == "ERROR"
    }) {
        return "falhou";
    }
    if !checks.iter().any(|c| is_test_check(c)) {
        return "nao_verificado";
    }
    let success = |c: &&Value| c["conclusion"] == "SUCCESS" || c["state"] == "SUCCESS";
    let neutral = |c: &&Value| c["conclusion"] == "SKIPPED" || c["conclusion"] == "NEUTRAL";
    // Os checks opcionais ignorados não bloqueiam, mas é obrigatório
    // pelo menos um teste realmente executado com sucesso.
    if checks.iter().any(|c| is_test_check(c) && success(c))
        && checks.iter().all(|c| success(c) || neutral(c))
    {
        return "aprovado";
    }
    "pendente"
}

pub fn assess(prs: &[Value]) -> Vec<Value> {
    let paths: Vec<_> = prs.iter().map(work_files).collect();
    let issues: Vec<_> = prs.iter().map(issue_refs).collect();
    prs.iter()
        .enumerate()
        .map(|(i, p)| {
            let number = p["number"].as_i64().unwrap_or(0);
            let mut overlaps = BTreeSet::new();
            let mut duplicates = BTreeSet::new();
            for (j, other) in prs.iter().enumerate() {
                if i == j {
                    continue;
                }
                let n = other["number"].as_i64().unwrap_or(0);
                if n <= 0 {
                    continue;
                }
                let same_issue = !issues[i].is_empty() && !issues[i].is_disjoint(&issues[j]);
                if same_issue {
                    duplicates.insert(n);
                }
                if !paths[i].is_empty() && !paths[i].is_disjoint(&paths[j]) {
                    overlaps.insert(n);
                }
            }
            let checks = check_state(p);
            let draft = p["isDraft"].as_bool() != Some(false);
            let merge_state = p["mergeStateStatus"].as_str().unwrap_or("");
            let review = p["reviewDecision"].as_str().unwrap_or("");
            // A classificação é informativa: nunca equivale a autorização de merge.
            let state = if draft {
                "draft"
            } else if merge_state == "DIRTY" {
                "conflito"
            } else if !duplicates.is_empty() {
                "duplicado_possivel"
            } else if !overlaps.is_empty() {
                "sobreposicao"
            } else if checks == "falhou" {
                "ci_falhou"
            } else if checks != "aprovado" {
                "ci_pendente"
            } else if review != "APPROVED" {
                "revisao_pendente"
            } else {
                "revisao_final_necessaria"
            };
            let (priority, action) = match state {
                "revisao_pendente" if review == "CHANGES_REQUESTED" => (
                    0,
                    "Analisar alterações solicitadas; encaminhar apenas PRs próprias",
                ),
                "ci_falhou" => (1, "Investigar a primeira falha de CI"),
                "conflito" => (2, "Resolver conflitos e repetir validação"),
                "duplicado_possivel" => (3, "Comparar as PRs da mesma issue"),
                "sobreposicao" => (4, "Rever a sobreposição de ficheiros"),
                "revisao_final_necessaria" => (5, "Revalidar gates antes de integrar"),
                "ci_pendente" => (6, "Aguardar ou investigar testes pendentes"),
                "revisao_pendente" => (7, "Solicitar revisão independente"),
                "draft" => (8, "Concluir alterações e testes da Draft"),
                _ => (9, "Investigar estado da PR"),
            };
            json!({
                "priority": priority,
                "next_action": action,
                "number": number,
                "title": p["title"],
                "url": p["url"],
                "head": p["headRefOid"],
                "head_sha": p["headRefOid"],
                "base": p["baseRefName"],
                "base_sha": p["baseRefOid"],
                "created_at": p["createdAt"],
                "updated_at": p["updatedAt"],
                "draft": draft,
                "checks": checks,
                "review": review,
                "merge_state": merge_state,
                "state": state,
                "overlaps": overlaps.into_iter().collect::<Vec<_>>(),
                "possible_duplicates": duplicates.into_iter().collect::<Vec<_>>(),
                "files": paths[i].iter().cloned().collect::<Vec<_>>()
            })
        })
        .collect()
}

pub async fn scan(a: &Ax, repo: &str) -> Result<(), String> {
    let data = complete_open_prs(repo).await?;
    let list = data.as_slice();
    let report = assess(list);
    // Só recuperar falhas transitórias depois de obter PRs atuais do GitHub.
    crate::drop_recovery::scan(a, repo, list).await;
    let merge_enabled = a.db.n(
        "select auto_merge from rp where full=? and on_=1",
        pa![repo.to_string()],
    ) == 1;
    let repair_enabled = a.db.n(
        "select steward_repair from rp where full=? and on_=1",
        pa![repo.to_string()],
    ) == 1;
    let coderabbit_request_enabled = a.db.n(
        "select coderabbit_review from rp where full=? and on_=1",
        pa![repo.to_string()],
    ) == 1;
    let coderabbit_repair_enabled = a.db.n(
        "select coderabbit_repair from rp where full=? and on_=1",
        pa![repo.to_string()],
    ) == 1;
    let local_merge_enabled = a.db.n(
        "select local_merge from rp where full=? and on_=1 and auto_merge=1",
        pa![repo.to_string()],
    ) == 1;
    let summary = json!({
        "review_requested": report.iter().filter(|p| p["review"] == "CHANGES_REQUESTED").count(),
        "ci_failed": report.iter().filter(|p| p["checks"] == "falhou").count(),
        "total": report.len(),
        "draft": report.iter().filter(|p| p["state"] == "draft").count(),
        "overlap": report.iter().filter(|p| p["state"] == "sobreposicao" || p["state"] == "duplicado_possivel").count(),
        "blocked": report.iter().filter(|p| p["state"] != "revisao_final_necessaria").count(),
        "states": state_counts(&report)
    });
    let key = format!("steward:{repo}");
    let prev = a.db.kg(&key);
    a.db.ks(
        &key,
        &json!({
            "at": now(),
            "repo": repo,
            "summary": summary,
            "prs": report.clone(),
            "read_only": !merge_enabled,
            "merge_enabled": merge_enabled,
            "local_merge_enabled": local_merge_enabled,
            "repair_enabled": repair_enabled,
            "coderabbit_request_enabled": coderabbit_request_enabled,
            "coderabbit_repair_enabled": coderabbit_repair_enabled
        }),
    );
    // A fresh complete scan supersedes any previously cached transient error.
    a.db.kd(&format!("steward:error:{repo}"));
    if repair_enabled {
        crate::steward_repair::run(a, repo, list, &report).await;
    }
    if coderabbit_request_enabled {
        crate::coderabbit::request_reviews(a, repo, list).await;
    }
    if coderabbit_repair_enabled {
        crate::coderabbit::route_findings(a, repo, list, &report).await;
    }
    if merge_enabled {
        crate::steward_merge::run(a, repo, list, &report).await;
    }
    if prev["summary"] != summary {
        a.say(
            "watcher",
            &format!(
                "{repo}: {} PRs abertas, {} com sobreposições; revisão independente necessária",
                summary["total"], summary["overlap"]
            ),
            None,
        );
    }
    Ok(())
}

// A revalidação aprovada deve antecipar a próxima reconciliação, sem dispensar gates.
async fn wait_for_next_scan(a: &Ax) {
    tokio::select! {
        _ = sl(300_000) => {},
        _ = a.steward_wake.notified() => {},
    }
}

pub async fn run(a: Ax) {
    loop {
        if !a.hold("watcher") && !a.drain.load(std::sync::atomic::Ordering::Relaxed) {
            let repos =
                a.db.all("select full from rp where on_=1 order by full", pa![]);
            for row in repos {
                if a.hold("watcher") {
                    break;
                }
                let Some(repo) = row["full"].as_str() else {
                    continue;
                };
                if let Err(err) = scan(&a, repo).await {
                    a.db.ks(
                        &format!("steward:error:{repo}"),
                        &json!({"at": now(), "error": crate::sh::head(&err, 500)}),
                    );
                }
            }
        }
        wait_for_next_scan(&a).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn validacao_concluida_acorda_steward_sem_esperar_cinco_minutos() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            47300,
            "apollo-office".into(),
        ));
        // Notify guarda a notificação recebida entre a validação e a espera.
        a.steward_wake.notify_one();
        tokio::time::timeout(
            std::time::Duration::from_millis(500),
            wait_for_next_scan(&a),
        )
        .await
        .expect("a revalidação deve antecipar a leitura");
    }

    fn sample(n: i64, path: &str, issue: &str) -> Value {
        json!({
            "number":n,
            "title":format!("Correção {n}"),
            "body":format!("Fixes #{issue}"),
            "url":format!("https://github.com/org/repo/pull/{n}"),
            "isDraft":false,
            "headRefOid":"abc",
            "baseRefName":"main",
            "mergeStateStatus":"CLEAN",
            "reviewDecision":"APPROVED",
            "statusCheckRollup":[{"name":"test-suite","conclusion":"SUCCESS"}],
            "files":[{"path":path}]
        })
    }

    #[test]
    fn inventory_accepts_more_than_one_thousand_prs_and_rejects_duplicates() {
        let rows: Vec<Value> = (1..=1001)
            .map(|n| {
                json!({
                    "number":n,"state":"open","head":{"sha":"a".repeat(40)},
                    "base":{"sha":"b".repeat(40)},"updated_at":"2026-01-01T00:00:00Z"
                })
            })
            .collect();
        let mut map = Inventory::new();
        for page in rows.chunks(100) {
            append_inventory(&mut map, page).unwrap();
        }
        assert_eq!(map.len(), 1001);
        assert!(append_inventory(&mut map, &rows[..1]).is_err());
        assert!(inventory_row(&json!({"number":2,"state":"open"})).is_err());
    }

    #[test]
    fn enrichment_must_match_inventory_head_base_and_timestamp() {
        let identity = (
            "a".repeat(40),
            "b".repeat(40),
            "2026-01-01T00:00:00Z".to_string(),
        );
        let mut row = sample(1, "src/a.rs", "1");
        row["headRefOid"] = json!(identity.0);
        row["baseRefOid"] = json!(identity.1);
        row["updatedAt"] = json!(identity.2);
        assert!(detail_matches(&row, 1, &identity));
        row["baseRefOid"] = json!("c".repeat(40));
        assert!(!detail_matches(&row, 1, &identity));
    }

    #[test]
    fn state_summary_counts_every_pr_state_and_ignores_unknown_values() {
        let rows = [
            json!({"state":"draft"}),
            json!({"state":"ci_falhou"}),
            json!({"state":"ci_falhou"}),
            json!({"state":"revisao_final_necessaria"}),
            json!({"state":"future_state"}),
        ];
        assert_eq!(
            state_counts(&rows),
            json!({
                "draft":1,
                "conflito":0,
                "duplicado_possivel":0,
                "sobreposicao":0,
                "ci_falhou":2,
                "ci_pendente":0,
                "revisao_pendente":0,
                "revisao_final_necessaria":1
            })
        );
    }

    #[test]
    fn detects_all_github_issue_closing_keywords_without_false_issue_number() {
        for keyword in [
            "Fix", "Fixes", "Fixed", "Close", "Closes", "Closed", "Resolve", "Resolves", "Resolved",
        ] {
            let p = json!({"title":"Correção","body":format!("{keyword} #42")});
            assert_eq!(issue_refs(&p), BTreeSet::from(["42".to_string()]));
        }
        let p = json!({"title":"Correção","body":"Resolved: #42\nFixed #142"});
        assert_eq!(
            issue_refs(&p),
            BTreeSet::from(["42".to_string(), "142".to_string()])
        );
        assert!(!issue_refs(&json!({"body":"Referência à issue #42"})).contains("42"));
    }

    #[test]
    fn skipped_optional_checks_do_not_block_successful_tests() {
        let mut pr = sample(44, "src/abc.rs", "44");
        pr["statusCheckRollup"] = json!([
            {"name":"test-suite","conclusion":"SUCCESS"},
            {"name":"lint","conclusion":"SKIPPED"},
            {"name":"docs","conclusion":"NEUTRAL"}
        ]);
        assert_eq!(assess(&[pr.clone()])[0]["checks"], "aprovado");
        pr["statusCheckRollup"][0]["conclusion"] = json!("SKIPPED");
        assert_eq!(assess(&[pr.clone()])[0]["checks"], "pendente");
        pr["statusCheckRollup"][0]["conclusion"] = json!("FAILURE");
        assert_eq!(assess(&[pr])[0]["checks"], "falhou");
    }

    #[test]
    fn same_issue_marks_possible_duplicate_without_closing() {
        let rows = assess(&[
            sample(671, "shared/policy.ts", "42"),
            sample(673, "shared/policy.ts", "42"),
        ]);
        assert_eq!(rows[0]["state"], "duplicado_possivel");
        assert_eq!(rows[0]["possible_duplicates"], json!([673]));
        assert_eq!(rows[1]["possible_duplicates"], json!([671]));
    }

    #[test]
    fn overlapping_files_are_only_a_warning_not_a_merge_permission() {
        let rows = assess(&[
            sample(671, "shared/policy.ts", "42"),
            sample(673, "shared/policy.ts", "43"),
        ]);
        assert_eq!(rows[0]["state"], "sobreposicao");
        assert_eq!(rows[0]["overlaps"], json!([673]));
        assert_eq!(rows[0]["possible_duplicates"], json!([]));
    }

    #[test]
    fn draft_and_missing_checks_are_never_classified_as_ready() {
        let mut draft = sample(1, "src/a.ts", "1");
        draft["isDraft"] = json!(true);
        let mut missing = sample(2, "src/b.ts", "2");
        missing["statusCheckRollup"] = json!([]);
        let rows = assess(&[draft, missing]);
        assert_eq!(rows[0]["state"], "draft");
        assert_eq!(rows[1]["state"], "ci_pendente");
    }

    #[test]
    fn coderabbit_only_does_not_validate_ci() {
        let mut pr = sample(12, "src/a.ts", "12");
        pr["statusCheckRollup"] = json!([{
            "name":"CodeRabbit","conclusion":"SUCCESS",
            "app":{"slug":"coderabbitai"}
        }]);
        let rows = assess(&[pr.clone()]);
        assert_eq!(rows[0]["checks"], "nao_verificado");
        assert_eq!(rows[0]["state"], "ci_pendente");
        pr["statusCheckRollup"] = json!([
            {"name":"CodeRabbit","conclusion":"SUCCESS","app":{"slug":"coderabbitai"}},
            {"name":"test-suite","conclusion":"SUCCESS","app":{"slug":"ci-service"}}
        ]);
        let rows = assess(&[pr]);
        assert_eq!(rows[0]["checks"], "aprovado");
        assert_eq!(rows[0]["state"], "revisao_final_necessaria");
    }

    #[test]
    fn static_analysis_or_build_alone_cannot_prove_tests() {
        let mut pr = sample(14, "src/c.ts", "14");
        pr["statusCheckRollup"] = json!([
            {"name":"lint","conclusion":"SUCCESS"},
            {"name":"CodeQL","conclusion":"SUCCESS"},
            {"name":"build","conclusion":"SUCCESS"}
        ]);
        assert_eq!(assess(&[pr])[0]["checks"], "nao_verificado");
    }

    #[test]
    fn failing_lint_blocks_success_even_when_tests_pass() {
        let mut pr = sample(15, "src/c.ts", "15");
        pr["statusCheckRollup"] = json!([
            {"name":"test-suite","conclusion":"SUCCESS"},
            {"name":"lint","conclusion":"FAILURE"}
        ]);
        assert_eq!(assess(&[pr])[0]["checks"], "falhou");
    }

    #[test]
    fn nameless_statuses_do_not_claim_ci_success() {
        let mut pr = sample(13, "src/b.ts", "13");
        pr["statusCheckRollup"] = json!([{"conclusion":"SUCCESS"}]);
        assert_eq!(assess(&[pr])[0]["checks"], "nao_verificado");
    }

    #[test]
    fn even_approved_prs_require_independent_final_review() {
        let rows = assess(&[sample(1, "src/a.ts", "1")]);
        assert_eq!(rows[0]["state"], "revisao_final_necessaria");
    }
}
