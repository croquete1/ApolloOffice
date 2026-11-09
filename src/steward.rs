//! Reconciliação periódica e diagnóstico consultivo das PRs.
//! O merge é opt-in por repositório, sujeito aos gates conservadores definidos
//! em steward_merge; a triagem nunca fecha PRs nem altera branches.
use crate::app::{sl, Ax};
use crate::db::now;
use crate::pa;
use crate::sh::ghj;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const PAGE_LIMIT: &str = "200";

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

fn issue_refs(pr: &Value) -> BTreeSet<String> {
    let source = format!(
        "{}\n{}",
        pr["title"].as_str().unwrap_or(""),
        pr["body"].as_str().unwrap_or("")
    );
    let re = regex::Regex::new(r"(?i)\b(?:fixes|closes|resolves)\s+#(\d+)\b").unwrap();
    re.captures_iter(&source)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn check_state(pr: &Value) -> &'static str {
    let Some(checks) = pr["statusCheckRollup"].as_array() else {
        return "nao_verificado";
    };
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
    if checks.iter().all(|c| c["conclusion"] == "SUCCESS" || c["state"] == "SUCCESS") {
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
            json!({
                "number": number,
                "title": p["title"],
                "url": p["url"],
                "head": p["headRefOid"],
                "base": p["baseRefName"],
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
    let data = ghj(
        &[
            "pr",
            "list",
            "-R",
            repo,
            "--state",
            "open",
            "--limit",
            PAGE_LIMIT,
            "--json",
            "number,title,body,url,isDraft,headRefOid,baseRefName,mergeStateStatus,reviewDecision,statusCheckRollup,files",
        ],
        90,
    )
    .await?;
    let list = data.as_array().ok_or("Resposta de PRs inválida")?;
    if list.len() >= PAGE_LIMIT.parse::<usize>().unwrap() {
        a.db.ks(
            &format!("steward:{repo}"),
            &json!({"at":now(),"error":"Limite de resultados atingido; análise incompleta e sem ações permitidas.","prs":[]}),
        );
        return Err(format!("{repo}: lista de PRs truncada"));
    }
    let report = assess(list);
    let merge_enabled = a.db.n(
        "select auto_merge from rp where full=? and on_=1",
        pa![repo.to_string()],
    ) == 1;
    let summary = json!({
        "total": report.len(),
        "draft": report.iter().filter(|p| p["state"] == "draft").count(),
        "overlap": report.iter().filter(|p| p["state"] == "sobreposicao" || p["state"] == "duplicado_possivel").count(),
        "blocked": report.iter().filter(|p| p["state"] != "revisao_final_necessaria").count()
    });
    let key = format!("steward:{repo}");
    let prev = a.db.kg(&key);
    a.db.ks(&key, &json!({
        "at": now(),
        "repo": repo,
        "summary": summary,
        "prs": report.clone(),
        "read_only": !merge_enabled,
        "merge_enabled": merge_enabled
    }));
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

pub async fn run(a: Ax) {
    loop {
        if !a.hold("watcher") && !a.drain.load(std::sync::atomic::Ordering::Relaxed) {
            let repos = a.db.all(
                "select full from rp where on_=1 order by full",
                pa![],
            );
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
        sl(300_000).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "statusCheckRollup":[{"conclusion":"SUCCESS"}],
            "files":[{"path":path}]
        })
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
    fn even_approved_prs_require_independent_final_review() {
        let rows = assess(&[sample(1, "src/a.ts", "1")]);
        assert_eq!(rows[0]["state"], "revisao_final_necessaria");
    }
}
