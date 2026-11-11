//! Circuit breaker for repeated, deterministic Hunter setup failures.
//! A failed Hunter never makes other repository work ineligible.
use crate::app::Ax;
use crate::db::{now, Job};
use regex::Regex;
use serde_json::{json, Value};

const LIMIT: i64 = 3;
const COOLDOWN_MS: i64 = 30 * 60 * 1000;

/// Identifica o estado persistente das falhas de pesquisa por repositório.
fn key(repo: &str) -> String {
    format!("hunt-fail:{repo}")
}

/// Devolve as falhas acumuladas e a suspensão efetiva da pesquisa.
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

/// Indica se o repositório não pode iniciar um novo Hunter.
pub fn blocked(a: &Ax, repo: &str) -> bool {
    status(a, repo)["blocked"] == true
}

/// Lista suspensões ativas para o painel autenticado.
pub fn blocked_repos(a: &Ax) -> Vec<Value> {
    a.db.all(
        "select full from rp where on_=1 and hunt=1 order by full",
        crate::pa![],
    )
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

/// Regista uma falha técnica, suspendendo novos Hunts após o limiar.
pub fn fail(a: &Ax, repo: &str, stage: &str, code: i32) {
    // The same gate is used for Hunter creation and final job admission.
    let (failures, until, was_blocked) = {
        let _transition = a.update_transition.lock().unwrap();
        let before = status(a, repo);
        let failures = before["failures"].as_i64().unwrap_or(0).saturating_add(1);
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
        (failures, until, before["blocked"] == true)
    };
    if failures == LIMIT || (until > 0 && !was_blocked) {
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

/// Limpa o contador quando uma preparação real termina com sucesso.
pub fn success(a: &Ax, repo: &str) {
    let had_failures = {
        let _transition = a.update_transition.lock().unwrap();
        let failed = status(a, repo)["failures"].as_i64().unwrap_or(0) > 0;
        if failed {
            a.db.kd(&key(repo));
        }
        failed
    };
    if had_failures {
        a.say(
            "hunter",
            &format!("{repo}: preparação validada; contador de falhas limpo"),
            None,
        );
    }
}

/// Reposição explícita do bloqueio após corrigir o ambiente.
pub fn reset(a: &Ax, repo: &str) {
    {
        let _transition = a.update_transition.lock().unwrap();
        a.db.kd(&key(repo));
    }
    a.say(
        "hunter",
        &format!("{repo}: bloqueio de pesquisas reposto manualmente"),
        None,
    );
}

/// Redact common credentials before saving the bounded diagnostic tail.
pub(crate) fn safe_tail(output: &str) -> String {
    static PATTERNS: std::sync::LazyLock<Vec<Regex>> = std::sync::LazyLock::new(|| {
        [
            // The entire DOCKER_AUTH_CONFIG assignment can be a JSON object
            // containing unrecognised registry auth formats. Redact its full
            // line before running generic token patterns.
            r#"(?im)(?:"|')?\bDOCKER_AUTH_CONFIG(?:"|')?[ \t]*[:=][ \t]*[^\r\n]*"#,
            // Docker credentials can also be printed across several JSON lines.
            r#"(?i)(?:"|')?(?:auth|identitytoken)(?:"|')?[ \t]*[:=][ \t]*(?:"[^"\r\n]*"|'[^'\r\n]*'|[^\s,;}]+)"#,
            // Include the complete Basic/Bearer header value, never just its prefix.
            r#"(?im)(?:"|')?\bauthorization(?:"|')?[ \t]*[:=][ \t]*[^\r\n]*"#,
            // Strip URL userinfo, including a password containing an extra @.
            r"(?i)\b[a-z][a-z0-9+.-]*://[^\s/]*@",
            // Consume all quoted values rather than only the first word.
            r#"(?i)(?:"|')?(?:api[_-]?key|token|password|secret)(?:"|')?[ \t]*[:=][ \t]*(?:"[^"\r\n]*"|'[^'\r\n]*'|[^\s,;]+)"#,
            r"(?i)\bbearer[ \t]+[^\s,;]+",
            r"(?i)(?:github_pat_[A-Za-z0-9_]+|gh[pousr]_[A-Za-z0-9_]+|sk-[A-Za-z0-9_-]{12,})",
        ]
        .iter()
        .map(|pattern| Regex::new(pattern).expect("constant credential pattern"))
        .collect()
    });
    let mut clean = output.to_string();
    for pattern in PATTERNS.iter() {
        clean = pattern
            .replace_all(&clean, "[credencial ocultada]")
            .into_owned();
    }
    crate::sh::last(&clean, 8_000)
}

/// Record exhausted host retries without incrementing repository setup failures.
pub fn host_diagnostic(job: &mut Job, host: &str, output: &str) {
    job.d["failure"] = json!({
        "stage": "host",
        "exit_code": Value::Null,
        "host": host,
        "at": now(),
        "output": safe_tail(output),
    });
}

/// Persist stage, exit code, worker and a detailed tail before the job is dropped.
pub fn diagnostic(a: &Ax, job: &mut Job, stage: &str, code: i32, output: &str) -> String {
    let detail = safe_tail(output);
    let host = job.h();
    job.d["failure"] = json!({
        "stage": stage,
        "exit_code": code,
        "host": host,
        "at": now(),
        "output": detail.clone(),
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

    #[tokio::test]
    async fn breaker_is_isolated_by_repository_and_clears_after_success() {
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

    #[tokio::test]
    async fn host_failure_diagnostic_does_not_trip_setup_breaker() {
        let a = app();
        let id = a.db.jnew(
            "example/one",
            0,
            "hunt",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let mut job = a.db.jg(id).unwrap();
        host_diagnostic(&mut job, "worker1", "docker: No such container");
        assert_eq!(job.d["failure"]["stage"], "host");
        assert_eq!(job.d["failure"]["host"], "worker1");
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

    #[test]
    fn docker_auth_config_is_redacted_from_setup_failure_diagnostics() {
        let output = concat!(
            "DOCKER_AUTH_CONFIG={\"auths\":{\"registry.example\":{\"auth\":\"YWxpY2U6c2VjcmV0\"}}}\n",
            "export DOCKER_AUTH_CONFIG='{\"auths\": {\"mirror.internal\": {\"auth\": \"more-secret\"}}}'\n",
            "\"DOCKER_AUTH_CONFIG\": \"{\\\"auths\\\":{\\\"mirror\\\":{\\\"auth\\\":\\\"quoted-secret\\\"}}}\"\n",
            "debug registry credentials:\n",
            "{\n  \"auth\": \"multiline-secret\",\n  \"identitytoken\": \"multiline-token\"\n}\n",
            "setup failed\n"
        );
        let tail = safe_tail(output);
        for secret in [
            "YWxpY2U6c2VjcmV0",
            "more-secret",
            "quoted-secret",
            "multiline-secret",
            "multiline-token",
        ] {
            assert!(!tail.contains(secret), "credential leaked: {secret}");
        }
        assert!(tail.contains("setup failed"));
        assert!(tail.contains("[credencial ocultada]"));
    }

    #[test]
    fn complete_authorization_urls_and_quoted_secrets_are_redacted() {
        let output = concat!(
            "Authorization: Basic dXNlcjpwYXNz\n",
            "\"Authorization\": \"Basic Zm9vOmJhcg==\"\n",
            "authorization: Bearer abcdef012345 trailing-value\n",
            "remote https://alice:my-password@github.example/repo.git\n",
            "password='long password with spaces'\n",
            "token=\"long token with spaces\"\n",
            "github_pat_abcdefgh123456\n",
        );
        let tail = safe_tail(output);
        for secret in [
            "dXNlcjpwYXNz",
            "Zm9vOmJhcg==",
            "abcdef012345",
            "trailing-value",
            "alice",
            "my-password",
            "long password with spaces",
            "long token with spaces",
            "github_pat_abcdefgh123456",
        ] {
            assert!(
                !tail.contains(secret),
                "credential fragment leaked: {secret}"
            );
        }
        assert!(tail.contains("github.example/repo.git"));
    }
}
