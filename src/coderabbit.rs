//! Read-only CodeRabbit evidence. Never grants review/merge authority.
use crate::sh::{apip_unique, ghj};
use serde_json::{json, Value};

const BOT_ID: u64 = 136622811;
const COVERAGE: &str = "<!-- final_review_risk_coverage:";

fn valid_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(crate) fn valid_repo(repo: &str) -> bool {
    let parts: Vec<_> = repo.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|s| {
            !s.is_empty()
                && *s != "."
                && *s != ".."
                && s.len() <= 100
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
}

fn classify(rows: &[Value], head: &str) -> Value {
    let mut latest = None;
    for row in rows.iter().filter(|r| {
        r["user"]["id"].as_u64() == Some(BOT_ID)
            && r["user"]["login"] == "coderabbitai[bot]"
            && r["user"]["type"] == "Bot"
    }) {
        let body = row["body"].as_str().unwrap_or("");
        if !body.contains("<!-- coderabbit-ai-generated:v1 -->")
            || !body.contains("<!-- This is an auto-generated comment: summarize by coderabbit.ai -->")
        {
            continue;
        }
        let Some(at) = row["updated_at"].as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) else {
            return json!({"state":"unknown","head":head});
        };
        let Some(id) = row["id"].as_u64().filter(|n| *n > 0) else {
            return json!({"state":"unknown","head":head});
        };
        if latest.as_ref().is_none_or(|(old, old_id, _)| (&at, id) > (old, *old_id)) {
            latest = Some((at, id, body));
        }
    }
    let Some((at, id, body)) = latest else {
        return json!({"state":"not_observed","head":head});
    };
    let mut result = json!({"state":"unknown","head":head,"comment_id":id,"observed_at":at.to_rfc3339()});
    let markers: Vec<_> = body.match_indices(COVERAGE).collect();
    if markers.len() != 1 {
        return result;
    }
    let Some((raw, _)) = body[markers[0].0 + COVERAGE.len()..].split_once("-->") else {
        return result;
    };
    let Ok(coverage) = serde_json::from_str::<Value>(raw.trim()) else {
        return result;
    };
    let source = coverage["sourceCommitId"].as_str().unwrap_or("");
    let covered = coverage["coveredCommitId"].as_str().unwrap_or("");
    if coverage["kind"] != "reviewed" || !valid_sha(source) || !valid_sha(covered) {
        return result;
    }
    result["covered_head"] = json!(covered);
    result["state"] = json!(if source == head && covered == head {
        "observed_current"
    } else {
        "stale"
    });
    result
}

fn matches_pr(pr: &Value, repo: &str, number: i64, head: &str) -> bool {
    pr["number"].as_i64() == Some(number)
        && pr["state"] == "open"
        && pr["base"]["repo"]["full_name"] == repo
        && pr["head"]["sha"] == head
        && pr["updated_at"].as_str().is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
}

pub(crate) async fn observe(repo: &str, number: i64, head: &str) -> Result<Value, String> {
    if !valid_repo(repo) || number <= 0 || !valid_sha(head) {
        return Err("Invalid review observation scope".into());
    }
    // The only remote operations are GitHub reads. Never call CodeRabbit's API or
    // request a review, and refuse private/unknown repositories.
    let repository = ghj(&["api", &format!("repos/{repo}")], 45).await?;
    if repository["private"] != false || repository["visibility"] != "public"
        || repository["full_name"] != repo {
        return Err("Only verified public repositories are eligible".into());
    }
    let endpoint = format!("repos/{repo}/pulls/{number}");
    let before = ghj(&["api", &endpoint], 45).await?;
    if !matches_pr(&before, repo, number, head) {
        return Err("PR scope or HEAD changed; refresh Steward".into());
    }
    let rows = apip_unique(&format!("repos/{repo}/issues/{number}/comments?per_page=100")).await?;
    let after = ghj(&["api", &endpoint], 45).await?;
    if !matches_pr(&after, repo, number, head) || before["updated_at"] != after["updated_at"] {
        return Err("PR changed during observation; refresh Steward".into());
    }
    Ok(classify(&rows, head))
}

#[cfg(test)]
mod tests {
    use super::*;
    const HEAD: &str = "090dcdb4817ad2f8f9c500efc13b371288850131";
    const OLD: &str = "f0eee616dc04f75faed4525c6c98f0eeae10538f";
    fn row(sha: &str) -> Value {
        json!({"id":42,"updated_at":"2026-11-24T12:22:00Z",
            "user":{"id":BOT_ID,"login":"coderabbitai[bot]","type":"Bot"},
            "body":format!("<!-- coderabbit-ai-generated:v1 -->\n<!-- This is an auto-generated comment: summarize by coderabbit.ai -->\n{COVERAGE}{} -->",
                json!({"sourceCommitId":sha,"coveredCommitId":sha,"kind":"reviewed"}))})
    }
    #[test]
    fn current_and_stale_are_distinct() {
        assert_eq!(classify(&[row(HEAD)], HEAD)["state"], "observed_current");
        assert_eq!(classify(&[row(OLD)], HEAD)["state"], "stale");
        assert_eq!(classify(&[], HEAD)["state"], "not_observed");
    }
    #[test]
    fn spoof_and_malformed_evidence_cannot_establish_review() {
        let mut spoof = row(HEAD);
        spoof["user"]["id"] = json!(1);
        assert_eq!(classify(&[spoof], HEAD)["state"], "not_observed");
        let mut malformed = row(HEAD);
        malformed["body"] = json!(format!("{}\n{COVERAGE}bad -->", malformed["body"].as_str().unwrap()));
        assert_eq!(classify(&[malformed], HEAD)["state"], "unknown");
    }
    #[test]
    fn newer_skipped_summary_supersedes_old_coverage() {
        let mut skipped = row(HEAD);
        skipped["id"] = json!(43);
        skipped["updated_at"] = json!("2026-11-24T12:23:00Z");
        skipped["body"] = json!("<!-- coderabbit-ai-generated:v1 -->\n<!-- This is an auto-generated comment: summarize by coderabbit.ai -->\nReview skipped");
        assert_eq!(classify(&[row(HEAD), skipped], HEAD)["state"], "unknown");
    }
    #[test]
    fn repository_and_pr_scope_are_strict() {
        assert!(valid_repo("croquete1/ApolloOffice"));
        assert!(!valid_repo("../repo"));
        assert!(!valid_repo("owner/repo?private=true"));
        let pr = json!({"number":67,"state":"open","base":{"repo":{"full_name":"croquete1/ApolloOffice"}},"head":{"sha":HEAD},"updated_at":"2026-11-24T12:22:00Z"});
        assert!(matches_pr(&pr,"croquete1/ApolloOffice",67,HEAD));
        assert!(!matches_pr(&pr,"croquete1/ApolloOffice",67,OLD));
    }
}
