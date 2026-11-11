//! Circuit breaker for repeated, deterministic Hunter setup failures.
//! A failed Hunter never makes other repository work ineligible.
use crate::app::Ax;
use crate::db::{now, Job};
use regex::Regex;
use serde_json::{json, Value};

const LIMIT: i64 = 3;
const COOLDOWN_MS: i64 = 30 * 60 * 1000;

fn key(repo: &str) -> String {
    format!("hunt-fail:{repo}")
}

pub fn status(a: &Ax, repo: &str) -> Value {
    let v = a.db.kg(&key(repo));
    let until = v["until"].as_i64().unwrap_or(0);
    json!({
        "blocked": until > now(),
        "failures": v["failures"].as_i64().unwrap_or(0),
        "until": until,
        "stage": v["stage"],
        "exit_code": v["exit_code"],
    })
}

pub fn blocked(a: &Ax, repo: &str) -> bool {
    status(a, repo)["blocked"] == true
}

pub fn blocked_repos(a: &Ax) -> Vec<Value> {
    a.db
        .all("select full from rp where on_=1 and hunt=1 order by full", crate::pa![])
        .into_iter()
        .filter_map(|v| {
            let repo = v["full"].as_str()?;
            let h = status(a, repo);
            if h["blocked"] == true {
                Some(json!({"repo": repo, "health": h}))
            } else {
                None
            }
        })
        .collect()
}

pub fn fail(a: &Ax, repo: &str, stage: &str, code: i32) {
    let before = status(a, repo);
    let failures = before["failures"]
        .as_i64()
        .unwrap_or(0)
        .saturating_add(1);
    let until = if failures >= LIMIT {
        now().saturating_add(COOLDOWN_MS)
    } else {
        0
    };
    a.db.ks(
        &key(repo),
        &json!({
            "failures": failures,
            "until": until,
            "stage": stage,
            "exit_code": code,
        }),
    );
    if failures == LIMIT || (until > 0 && before["blocked"] != true) {
        a.say(
            "hunter",
            &format!(
                "{repo}: pesquisa suspensa durante 30 minutos após {failures} falhas consecutivas de {stage}"
            ),
            None,
        );
        a.note(
            "Hunter suspenso",
            &format!("{repo}: {failures} falhas consecutivas de {stage}; corrigir o ambiente antes de reativar"),
            3,
        );
    }
}

pub fn success(a: &Ax, repo: &str) {
    let before = status(a, repo);
    if before["failures"].as_i64().unwrap_or(0) > 0 {
        a.db.kd(&key(repo));
        a.say("hunter", &format!("{repo}: preparação validada; contador de falhas limpo"), None);
    }
}

pub fn reset(a: &Ax, repo: &str) {
    a.db.kd(&key(repo));
    a.say(
        "hunter",
        &format!("{repo}: bloqueio de pesquisas reposto manualmente"),
        None,
    );
}

/// Restrict logs shown to the signed-in operator to a bounded, redacted tail.
fn safe_tail(output: &str) -> String {
    let credentials = Regex::new(
        r"(?i)(authorization\s*[:=]\s*(?:bearer\s+)?\S+|github_pat_[A-Za-z0-9_]+|gh[pousr]_[A-Za-z0-9_]+|sk-[A-Za-z0-9_-]{12,}|bearer\s+\S+|(?:api[_-]?key|token|password|secret)\s*[:=]\s*\S+)",
    )
    .expect("constant credential pattern");
    let clean = credentials.replace_all(output, "[credencial ocultada]");
    crate::sh::last(&clean, 8_000)
}

/// Persist stage, exit code, worker and a detailed tail before the job is dropped.
pub fn diagnostic(
    a: &Ax,
    job: &mut Job,
    stage: &str,
    code: i32,
    output: &str,
) -> String {
    let detail = safe_tail(output);
    let host = job.h();
    job.d["failure"] = json!({
        "stage": stage,
        "exit_code": code,
        "host": host,
        "at": now(),
        "output": detail,
    });
    a.db.jd(job);
    if job.kind == "hunt" {
        fail(a, &job.repo, stage, code);
    }
    format!(
        "{stage} command failed (exit {code}): {}",
        crate::sh::last(detail.trim(), 300)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::db::Db;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn app() -> Ax {
        Arc::new(App::new(
            Db::mem(),
            PathBuf::from("/tmp/apollo-hunt-health-tests"),
            0,
            PathBuf::from("/tmp/apollo-hunt-health-tests/apollo-office"),
        ))
    }

    #[test]
    fn breaker_is_isolated_by_repository_and_clears_after_success() {
        let a = app();
        for _ in 0..LIMIT {
            fail(&a, "example/one", "setup", 1);
        }
        assert!(blocked(&a, "example/one"));
        assert!(!blocked(&a, "example/two"));
        assert_eq!(status(&a, "example/one")["failures"], LIMIT);
        success(&a, "example/one");
        assert!(!blocked(&a, "example/one"));
        assert_eq!(status(&a, "example/one")["failures"], 0);
    }

    #[test]
    fn credential_values_are_redacted_from_diagnostics() {
        let output = "npm error token=verysecret\nAuthorization: Bearer abcdef012345\nsetup failed";
        let tail = safe_tail(output);
        assert!(!tail.contains("verysecret"));
        assert!(!tail.contains("abcdef012345"));
        assert!(tail.contains("setup failed"));
    }
}
