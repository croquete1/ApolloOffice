//! Opt-in deployment policy: free author, subscription reviewer, bounded repair.
use crate::app::{App, Ax, R, X};
use serde_json::{json, Value};

pub const FREE: [&str; 2] = [
    "nvidia:nvidia/nemotron-3-super-120b-a12b",
    "mistral:ministral-14b-latest",
];
pub const OPENCODE: [&str; 2] = [
    "opencode:opencode/beam-free",
    "opencode:opencode/deepseek-v4.1-flash-free",
];
pub fn is_opencode(md: &str) -> bool {
    OPENCODE.contains(&md)
}
pub const REVIEWERS: [&str; 2] = ["codex:", "cc:sonnet"];
pub fn enabled(a: &App) -> bool {
    std::env::var("APOLLO_OFFICE_FREE_FIRST").as_deref() == Ok("1")
        || a.set()["free_first"].as_bool() == Some(true)
}
pub fn is_free(md: &str) -> bool {
    FREE.contains(&md) || is_opencode(md) || md == "groq:openai/gpt-oss-120b"
}
pub fn plan(a: &Ax) -> Vec<String> {
    let settings = a.set();
    let extra = settings["opencode_authors"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|v| v.as_str())
                .filter(|md| is_opencode(md))
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut result = extra;
    if let Ok(models) = std::env::var("APOLLO_OFFICE_OPENCODE_AUTHORS") {
        result.extend(
            models
                .split(',')
                .map(str::trim)
                .filter(|m| is_opencode(m))
                .map(str::to_string),
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    result.retain(|m| seen.insert(m.clone()));
    result.extend(FREE.iter().chain(REVIEWERS.iter()).map(|s| s.to_string()));
    result
}

pub fn plan_for(a: &Ax, role: &str) -> Vec<String> {
    if matches!(role, "fixer" | "repro") {
        plan(a)
    } else {
        FREE.iter()
            .chain(REVIEWERS.iter())
            .map(|s| s.to_string())
            .collect()
    }
}

pub fn validate(a: &Ax, md: &str) -> R<()> {
    if !enabled(a) {
        return Ok(());
    }
    if !is_free(md) && !REVIEWERS.contains(&md) {
        return Err(X::E("route denied by free-first policy".into()));
    }
    if is_free(md) {
        let (id, model) = crate::prov::parse(md);
        if matches!(id, "nvidia" | "mistral" | "groq") {
            if let Ok(path) = std::env::var("APOLLO_OFFICE_FREE_APPROVALS") {
                let data = std::fs::read_to_string(path)
                    .map_err(|_| X::E("free-tier approval missing".into()))?;
                let approvals: Value = serde_json::from_str(&data)
                    .map_err(|_| X::E("free-tier approval malformed".into()))?;
                let approved = approvals["providers"]
                    .as_array()
                    .map(|rows| {
                        rows.iter().any(|r| {
                            r["provider"] == id
                                && r["model"] == model
                                && r["base_url"] == crate::prov::base(a, id)
                                && r["user_approved_free_only"] == true
                                && r["billing_disabled_verified"] == true
                                && r["tool_calling_verified"] == true
                                && r["expires_at"].as_f64().unwrap_or(0.0)
                                    > crate::db::now() as f64 / 1000.0
                        })
                    })
                    .unwrap_or(false);
                if !approved {
                    return Err(X::E(
                        "free-tier approval expired or route not approved; paid inference denied"
                            .into(),
                    ));
                }
            } else if !cfg!(test) {
                return Err(X::E("free-tier approval path missing".into()));
            }
        }
        if crate::prov::pv(id).map(|p| p.base) != Some(crate::prov::base(a, id).as_str()) {
            return Err(X::E("free route endpoint override denied".into()));
        }
    }
    Ok(())
}

// Adaptive routing is deliberately limited to the ordinary Fixer. The staged
// RED/GREEN pilot and the free-first authorization policy retain their plans.
fn independent_model(author: &str, reviewer: &str) -> bool {
    let (ap, am) = crate::prov::parse(author);
    let (rp, rm) = crate::prov::parse(reviewer);
    if ap != rp {
        return true;
    }
    if am.is_empty() || rm.is_empty() || am == rm || am == "default" || rm == "default" {
        return false;
    }
    if ap == "cc" {
        for family in ["sonnet", "opus", "haiku"] {
            if (am == family && rm.contains(family)) || (rm == family && am.contains(family)) {
                return false;
            }
        }
    }
    true
}

fn subscription_available(a: &Ax, model: &str) -> bool {
    if a.provider_limit(model).is_some() {
        return false;
    }
    match crate::prov::parse(model).0 {
        "cc" => true,
        "codex" => {
            let usage = a.codex_usage.lock().unwrap_or_else(|e| e.into_inner());
            !crate::codex_usage::hunt_reserve_reached(&usage, crate::db::now(), 0.0)
        }
        _ => false,
    }
}

fn choose_history<'a>(preferred: &'a str, candidates: &[&'a str], rows: &[Value]) -> &'a str {
    let score = |model: &str| -> Option<f64> {
        let r = rows.iter().find(|r| r["model"] == model)?;
        let n = r["attempts"].as_i64()?;
        let accepted = r["accepted"].as_i64()?;
        if n < 5 || accepted < 0 || accepted > n {
            return None;
        }
        Some(accepted as f64 / n as f64)
    };
    let Some(base) = score(preferred) else {
        return preferred;
    };
    let mut best = preferred;
    let mut best_score = base;
    for &model in candidates {
        if let Some(s) = score(model) {
            if s >= base + 0.15 && s > best_score {
                best = model;
                best_score = s;
            }
        }
    }
    best
}

/// Select only explicit subscription author routes, never a new paid API or
/// a local model. The existing independent review and verification gates remain.
pub fn adaptive_fixer(
    a: &Ax,
    job: &crate::db::Job,
    preferred: &str,
    complex: bool,
    round: u64,
) -> R<Option<Value>> {
    let settings = a.set();
    if enabled(a) || settings["adaptive_routing"] != true {
        return Ok(None);
    }
    let raw = a.db.kg("set");
    let reviewer = a.md("reviewer");
    let mut candidates = Vec::new();
    for role in ["fixer", "big"] {
        if role == "big" && settings["big"] != true {
            continue;
        }
        let Some(model) = raw["md"][role].as_str().filter(|m| !m.trim().is_empty()) else {
            continue;
        };
        let provider = crate::prov::parse(model).0;
        if !matches!(provider, "cc" | "codex")
            || model.starts_with("ollama:")
            || !independent_model(model, &reviewer)
        {
            continue;
        }
        if !candidates.contains(&model) && subscription_available(a, model) {
            candidates.push(model);
        }
    }
    if candidates.is_empty() {
        return Err(X::Lim("Adaptive Fixer: no available explicitly configured subscription model independent of the reviewer".into()));
    }
    let baseline = candidates
        .iter()
        .copied()
        .find(|m| *m == preferred)
        .unwrap_or(candidates[0]);
    let since = crate::db::now() - 90 * 86_400_000;
    let rows = a.db.try_all(
        "select f.model,count(*) attempts,sum(f.outcome='accepted') accepted
         from fix_attempts f join jobs j on j.id=f.job
         where j.repo=? and j.kind='fix' and f.attempt=1 and f.finished is not null
         and f.started>=? and f.outcome in ('accepted','rejected','failed')
         and coalesce(json_extract(j.d,'$.cx'),'')<>'complex'
         group by f.model",
        crate::pa![job.repo.clone(), since],
    );
    let samples = rows.as_ref().ok().map(Vec::as_slice).unwrap_or(&[]);
    let previous = job.d["author"].as_str().unwrap_or("");
    let (selected, reason) = if round > 0 {
        if let Some(alternative) = candidates.iter().copied().find(|m| *m != previous) {
            (alternative, "retry_alternative")
        } else {
            (baseline, "only_available_route")
        }
    } else if baseline != preferred {
        (baseline, "preferred_unavailable_or_not_independent")
    } else if complex {
        (baseline, "complex_configured_model")
    } else {
        let best = choose_history(baseline, &candidates, samples);
        if best != baseline {
            (best, "historical_first_pass")
        } else if rows.is_err() {
            (baseline, "history_unavailable")
        } else {
            (baseline, "configured_baseline")
        }
    };
    Ok(Some(
        json!({"model":selected,"preferred":preferred,"reason":reason,
        "samples":samples.iter().filter(|r|candidates.iter().any(|m|r["model"]==*m)).collect::<Vec<_>>(),
        "reviewer":reviewer,"at":crate::db::now()}),
    ))
}

// Independent records avoid overwriting a pipeline Job snapshot during async work.
pub fn event(a: &Ax, job: Option<i64>, role: &str, attempt: usize, md: &str, feedback: &str) {
    if !enabled(a) {
        return;
    }
    let key = format!(
        "routing:{}:{role}:{attempt}:{}",
        job.unwrap_or(0),
        hex::encode(rand::random::<[u8; 16]>())
    );
    a.db.ks(
        &key,
        &json!({"job":job,"role":role,"attempt":attempt + 1,"model":md,
        "t":crate::db::now(),"feedback":crate::sh::head(feedback,6000),
        "subscription_repair": role != "reviewer" && REVIEWERS.contains(&md)}),
    );
    a.say(
        role,
        &format!(
            "attempt {}: {md}{}",
            attempt + 1,
            if !feedback.is_empty() {
                "; prior diagnostics handed over"
            } else {
                ""
            }
        ),
        job,
    );
}

pub fn lesson(a: &Ax, job: i64, reviewer: &str, finding: &Value, evidence: &str, diff: &str) {
    if !enabled(a) || finding["ok"] != false {
        return;
    }
    use sha2::{Digest, Sha256};
    let diff_sha = hex::encode(Sha256::digest(diff.as_bytes()));
    let record = json!({"status":"pending_review","objective_verified":false,"job":job,
        "reviewer":reviewer,"finding":finding,"evidence":evidence,"diff_sha256":diff_sha,
        "t":crate::db::now(),"origin":"apollo_subscription_review","promotion":"manual_review_required"});
    let id = hex::encode(Sha256::digest(record.to_string().as_bytes()));
    a.db.ks(&format!("lesson:{job}:{id}"), &record);
}

/// Normalize provenance before a chat model summarizes it. A route's feedback
/// belongs to the preceding attempt, never to the model receiving that feedback.
pub fn evidence_summary(rows: &[Value], detail: &Value, state: &str) -> Value {
    let mut attempts = Vec::new();
    let mut author = Value::Null;
    let accepted = matches!(state, "approve" | "done")
        && detail["last_review"]["verdict"]["ok"] == true
        && detail["proof"]["b_exit"] == 0;
    for role in ["guard", "vet", "repro", "fixer", "reviewer"] {
        let mut group = rows
            .iter()
            .filter(|r| r["role"] == role)
            .collect::<Vec<_>>();
        group.sort_by_key(|r| {
            (
                r["t"].as_i64().unwrap_or(0),
                r["attempt"].as_u64().unwrap_or(0),
            )
        });
        for (i, row) in group.iter().enumerate() {
            let failure = group
                .get(i + 1)
                .and_then(|r| r["feedback"].as_str())
                .filter(|s| {
                    !s.is_empty() && *s != "read-only independent review; author repairs first"
                });
            let outcome = if failure.is_some() {
                "failed_or_rejected"
            } else if role == "fixer" && i + 1 == group.len() && accepted {
                author = row["model"].clone();
                "accepted_candidate"
            } else {
                "last_recorded_attempt"
            };
            attempts.push(
                json!({"role":role,"attempt":row["attempt"],"model":row["model"],
                "outcome":outcome,"failure_diagnostic":failure}),
            );
        }
    }
    json!({"accepted_code_author":author,"reviewer":detail["last_review"]["model"],
        "attempts":attempts,"regression_test_names":detail["f"]["tests"],
        "regression_test_count":detail["f"]["tests"].as_array().map(Vec::len),
        "independent_check_count":detail["acceptance_proof"]["checks"].as_array().map(Vec::len),
        "ci_evidence":detail["ci_evidence"],
        "inventory_scope":"requirements.tools is the worker system inventory, not the active project test runtime",
        "missing_evidence":"null means not recorded; do not infer a runtime or test total from inventory, diff examples or named acceptance checks"})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> Ax {
        let db = crate::db::Db::mem();
        db.ks(
            "set",
            &json!({"free_first":true,"md":{"big":"openai:paid","fixer":"cc:sonnet"}}),
        );
        std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "apollo-office".into(),
        ))
    }
    #[test]
    fn reviewer_failure_diagnostic_is_evidence_not_generic_instructions() {
        let rows = vec![
            json!({"role":"reviewer","attempt":1,"model":"codex:","feedback":""}),
            json!({"role":"reviewer","attempt":2,"model":"cc:sonnet","feedback":"Reviewer codex: failed: request timed out"}),
        ];
        let report = evidence_summary(&rows, &json!({}), "review");
        assert_eq!(
            report["attempts"][0]["failure_diagnostic"],
            "Reviewer codex: failed: request timed out"
        );
        assert_eq!(report["attempts"][0]["outcome"], "failed_or_rejected");
        let mut legacy = rows;
        legacy[1]["feedback"] = json!("read-only independent review; author repairs first");
        assert!(
            evidence_summary(&legacy, &json!({}), "review")["attempts"][0]["failure_diagnostic"]
                .is_null()
        );
    }

    #[test]
    fn repeated_attempts_keep_separate_provenance_records() {
        let a = app();
        event(&a, Some(42), "reviewer", 0, "codex:", "first");
        event(&a, Some(42), "reviewer", 0, "codex:", "second");
        assert_eq!(
            a.db.n(
                "select count(*) from kv where k like 'routing:42:reviewer:0:%'",
                crate::pa![]
            ),
            2
        );
    }

    #[test]
    fn author_free_even_for_complex_work_and_paid_config() {
        let a = app();
        assert_eq!(a.md("fixer"), FREE[0]);
        assert_eq!(a.md("big"), FREE[0]);
        let p = plan(&a);
        assert_eq!(&p[..2], &FREE);
        assert_eq!(&p[2..], &REVIEWERS);
        assert!(validate(&a, "openai:gpt-5.5").is_err());
        assert!(validate(&a, "anthropic:claude-sonnet-5-5").is_err());
        for md in p {
            assert!(validate(&a, &md).is_ok());
        }
        a.db.ks(
            "set",
            &json!({"free_first":true,"prov":{"nvidia":{"base":"https://attacker.invalid"}}}),
        );
        assert!(validate(&a, FREE[0]).is_err());
    }
    #[test]
    fn opencode_is_opt_in_and_keeps_existing_authors_and_reviewers() {
        let a = app();
        assert_eq!(plan(&a)[0], FREE[0]);
        a.db.ks(
            "set",
            &json!({"free_first":true, "opencode_authors":[OPENCODE[0], "opencode:paid"]}),
        );
        let p = plan(&a);
        assert_eq!(p[0], OPENCODE[0]);
        assert_eq!(&p[1..3], &FREE);
        assert_eq!(&p[3..], &REVIEWERS);
        assert_eq!(a.md("fixer"), OPENCODE[0]);
        assert_eq!(a.md("guard"), FREE[0]);
        assert_eq!(plan_for(&a, "guard")[0], FREE[0]);
        assert_eq!(plan_for(&a, "repro")[0], OPENCODE[0]);
        assert!(validate(&a, OPENCODE[0]).is_ok());
        assert!(validate(&a, "opencode:opencode/paid-model").is_err());
        a.db.ks(
            "set",
            &json!({"free_first":true,"prov":{"opencode":{"base":"https://external.invalid"}}}),
        );
        assert!(validate(&a, OPENCODE[0]).is_err());
    }
    #[test]
    fn reporting_attributes_previous_failure_and_keeps_test_counts_distinct() {
        let rows = vec![
            json!({"role":"fixer","attempt":1,"model":OPENCODE[0],"feedback":""}),
            json!({"role":"fixer","attempt":2,"model":FREE[0],"feedback":"step limit"}),
            json!({"role":"fixer","attempt":3,"model":"codex:","feedback":"HTTP 404"}),
        ];
        let detail = json!({"last_review":{"model":"codex:","verdict":{"ok":true}},"proof":{"b_exit":0},
            "f":{"tests":["one_regression_with_many_cases"]},"acceptance_proof":{"checks":["offset","window"]}});
        let report = evidence_summary(&rows, &detail, "approve");
        assert_eq!(report["accepted_code_author"], "codex:");
        assert_eq!(report["attempts"][0]["failure_diagnostic"], "step limit");
        assert_eq!(report["attempts"][1]["failure_diagnostic"], "HTTP 404");
        assert!(report["attempts"][2]["failure_diagnostic"].is_null());
        assert_eq!(report["regression_test_count"], 1);
        assert_eq!(report["independent_check_count"], 2);
        assert!(report["ci_evidence"].is_null());
        assert!(evidence_summary(&rows, &detail, "fix")["accepted_code_author"].is_null());
    }

    #[test]
    fn rejection_stages_attributed_lesson_only() {
        let a = app();
        lesson(&a, 12, "codex:", &json!({"ok":true}), "tests pass", "diff");
        assert_eq!(
            a.db.n(
                "select count(*) from kv where k like 'lesson:%'",
                crate::pa![]
            ),
            0
        );
        lesson(
            &a,
            12,
            "codex:",
            &json!({"ok":false,"issues":["fix assertion"]}),
            "real test exit 1",
            "diff",
        );
        let row =
            a.db.one("select v from kv where k like 'lesson:%'", crate::pa![])
                .unwrap();
        let v: Value = serde_json::from_str(row["v"].as_str().unwrap()).unwrap();
        assert_eq!(v["status"], "pending_review");
        assert_eq!(v["objective_verified"], false);
        assert_eq!(v["reviewer"], "codex:");
        assert_eq!(v["diff_sha256"].as_str().unwrap().len(), 64);
    }
}

#[cfg(test)]
mod adaptive_tests {
    use super::*;
    #[test]
    fn requires_material_improvement_and_enough_finished_samples() {
        let mut rows = vec![
            json!({"model":"codex:","attempts":10,"accepted":5}),
            json!({"model":"cc:sonnet","attempts":4,"accepted":4}),
        ];
        let routes = ["codex:", "cc:sonnet"];
        assert_eq!(choose_history("codex:", &routes, &rows), "codex:");
        rows[1]["attempts"] = json!(10);
        rows[1]["accepted"] = json!(6);
        assert_eq!(choose_history("codex:", &routes, &rows), "codex:");
        rows[1]["accepted"] = json!(8);
        assert_eq!(choose_history("codex:", &routes, &rows), "cc:sonnet");
        assert!(!independent_model("cc:sonnet", "claude-sonnet-5-5"));
        assert!(!independent_model("codex:", "codex:gpt-6"));
        assert!(independent_model("codex:", "cc:sonnet"));
    }

    #[test]
    fn only_explicit_available_subscription_routes_and_no_qwen() {
        let db = crate::db::Db::mem();
        let a = std::sync::Arc::new(App::new(
            db,
            std::env::temp_dir(),
            1,
            "apollo-office".into(),
        ));
        let job = crate::db::Job::of(json!({"id":1,"repo":"o/r","kind":"fix","d":{}}));
        assert!(adaptive_fixer(&a, &job, "codex:", false, 0)
            .unwrap()
            .is_none());
        a.db.ks("set",&json!({"adaptive_routing":true,"md":{"fixer":"codex:","big":"ollama:qwen3:8b","reviewer":"cc:sonnet"}}));
        let result = adaptive_fixer(&a, &job, "codex:", false, 0)
            .unwrap()
            .unwrap();
        assert_eq!(result["model"], "codex:");
        a.db.ks("set",&json!({"adaptive_routing":true,"md":{"fixer":"cc:sonnet","big":"ollama:qwen3:8b","reviewer":"cc:sonnet"}}));
        assert!(matches!(
            adaptive_fixer(&a, &job, "cc:sonnet", false, 0),
            Err(X::Lim(_))
        ));
        a.db.ks("set",&json!({"adaptive_routing":true,"md":{"fixer":"cc:sonnet","big":"codex:","reviewer":"cc:haiku"}}));
        a.rl(&json!({"status":"rejected","resetsAt":crate::db::now()/1000+3600}));
        assert_eq!(
            adaptive_fixer(&a, &job, "cc:sonnet", false, 0)
                .unwrap()
                .unwrap()["model"],
            "codex:"
        );
    }
}
