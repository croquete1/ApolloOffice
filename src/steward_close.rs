//! Fecha apenas PRs próprias que foram claramente substituídas por uma PR já integrada.
//! A ação é opt-in por repositório e revalida o estado no GitHub imediatamente antes de fechar.
use crate::app::Ax;
use crate::pa;
use crate::sh::{gh, ghj};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn author(pr: &Value) -> &str {
    pr["author"]["login"].as_str().unwrap_or("")
}

fn issue_refs(pr: &Value) -> BTreeSet<String> {
    crate::steward::issue_refs(pr)
}

fn valid_merged_replacement(
    pr: &Value,
    number: i64,
    repo: &str,
    login: &str,
    base: &str,
    issue: &str,
) -> bool {
    pr["number"].as_i64() == Some(number)
        && pr["state"] == "closed"
        && pr["merged_at"].is_string()
        && pr["user"]["login"]
            .as_str()
            .is_some_and(|user| user.eq_ignore_ascii_case(login))
        && pr["base"]["ref"] == base
        && pr["base"]["repo"]["full_name"] == repo
        && issue_refs(pr).contains(issue)
}

fn valid_open_target(
    pr: &Value,
    number: i64,
    repo: &str,
    login: &str,
    base: &str,
    issues: &BTreeSet<String>,
) -> bool {
    pr["number"].as_i64() == Some(number)
        && pr["state"] == "open"
        && pr["merged_at"].is_null()
        && pr["user"]["login"]
            .as_str()
            .is_some_and(|user| user.eq_ignore_ascii_case(login))
        && pr["head"]["repo"]["owner"]["login"]
            .as_str()
            .is_some_and(|owner| owner.eq_ignore_ascii_case(login))
        && pr["base"]["ref"] == base
        && pr["base"]["repo"]["full_name"] == repo
        && issue_refs(pr).eq(issues)
}

/// Fecha no máximo uma PR por ciclo, para poder reconciliar novamente após cada alteração.
pub async fn run(a: &Ax, repo: &str, open: &[Value]) -> Result<(), String> {
    if a.hold("watcher")
        || a.db.n(
            "select steward_close from rp where full=? and on_=1",
            pa![repo.to_string()],
        ) != 1
    {
        return Ok(());
    }
    a.db.ks(&format!("steward:close_error:{repo}"), &Value::Null);
    let login = crate::gh::login(a).await;
    if login.is_empty() {
        return Ok(());
    }
    let metadata = crate::gh::meta(a, repo).await?;
    let base = metadata["main"].as_str().unwrap_or("");
    if base.is_empty() || metadata["arch"] == true {
        return Ok(());
    }

    let candidates: Vec<_> = open
        .iter()
        .filter(|pr| {
            author(pr).eq_ignore_ascii_case(&login)
                && pr["baseRefName"] == base
                && !issue_refs(pr).is_empty()
        })
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }

    let merged = ghj(
        &[
            "pr",
            "list",
            "-R",
            repo,
            "--state",
            "merged",
            "--author",
            &login,
            "--base",
            base,
            "--limit",
            "1000",
            "--json",
            "number,title,body,url,author,baseRefName,mergedAt",
        ],
        90,
    )
    .await?;
    let Some(merged) = merged.as_array() else {
        return Err(format!(
            "{repo}: lista de PRs integradas inválida; nenhuma PR foi fechada"
        ));
    };
    if merged.len() >= 1000 {
        return Err(format!(
            "{repo}: lista de PRs integradas atingiu 1000; análise incompleta, nenhuma PR foi fechada"
        ));
    }

    for candidate in candidates {
        if a.hold("watcher") {
            return Ok(());
        }
        let Some(number) = candidate["number"].as_i64().filter(|n| *n > 0) else {
            continue;
        };
        let issue_set = issue_refs(candidate);
        if issue_set.is_empty() {
            continue;
        }
        for issue in &issue_set {
            let replacement = merged.iter().find(|pr| {
                pr["number"].as_i64() != Some(number)
                    && author(pr).eq_ignore_ascii_case(&login)
                    && pr["baseRefName"] == base
                    && pr["mergedAt"].is_string()
                    && issue_refs(pr).contains(issue)
            });
            let Some(replacement) = replacement else {
                continue;
            };
            let Some(replacement_number) = replacement["number"].as_i64().filter(|n| *n > 0) else {
                continue;
            };
            let issue_data = ghj(&["api", &format!("repos/{repo}/issues/{issue}")], 40).await?;
            if issue_data["state"] != "closed" || !issue_data["pull_request"].is_null() {
                continue;
            }

            // Revalida a PR substituta e a issue: a listagem pode estar desatualizada.
            let replacement_now = ghj(
                &["api", &format!("repos/{repo}/pulls/{replacement_number}")],
                40,
            )
            .await?;
            if !valid_merged_replacement(
                &replacement_now,
                replacement_number,
                repo,
                &login,
                base,
                issue,
            ) {
                continue;
            }
            let issue_now = ghj(&["api", &format!("repos/{repo}/issues/{issue}")], 40).await?;
            if issue_now["state"] != "closed" || !issue_now["pull_request"].is_null() {
                continue;
            }

            // Última leitura do alvo e da configuração antes da alteração remota.
            let target_now = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 40).await?;
            if !valid_open_target(&target_now, number, repo, &login, base, &issue_set)
                || a.hold("watcher")
                || a.db.n(
                    "select steward_close from rp where full=? and on_=1",
                    pa![repo.to_string()],
                ) != 1
            {
                continue;
            }

            ghj(
                &[
                    "api",
                    "-X",
                    "PATCH",
                    &format!("repos/{repo}/pulls/{number}"),
                    "-f",
                    "state=closed",
                ],
                60,
            )
            .await?;
            let closed = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 40).await?;
            if closed["state"] != "closed" || closed["merged_at"].is_string() {
                return Err(format!("GitHub não confirmou o fecho de {repo}#{number}"));
            }

            let replacement_url = replacement_now["html_url"]
                .as_str()
                .or_else(|| replacement["url"].as_str())
                .unwrap_or("");
            let comment = if crate::pipe::rp(a, repo)["lang"] == "pt-PT" {
                format!(
                    "A fechar esta PR porque a issue #{issue} já foi fechada pela PR #{replacement_number}, integrada em `{base}`. PR que a substitui: {replacement_url}"
                )
            } else {
                format!(
                    "Closing this PR because issue #{issue} was closed by merged PR #{replacement_number} into `{base}`. Replacement PR: {replacement_url}"
                )
            };
            let comment_error = gh(
                &[
                    "pr",
                    "comment",
                    &number.to_string(),
                    "-R",
                    repo,
                    "--body",
                    &comment,
                ],
                60,
            )
            .await
            .err();
            a.db.run(
                "update prs set st='closed',upd=? where repo=? and num=?",
                pa![
                    closed["updated_at"].as_str().unwrap_or("").to_string(),
                    repo.to_string(),
                    number
                ],
            );
            a.db.ks(&format!("steward:close_error:{repo}"), &Value::Null);
            let message = format!(
                "Fechada {repo}#{number}: substituída pela PR #{replacement_number} integrada; issue #{issue} fechada"
            );
            a.say("watcher", &message, None);
            a.note(
                "PR substituída fechada",
                &format!(
                    "{message}\n{}",
                    target_now["html_url"].as_str().unwrap_or("")
                ),
                3,
            );
            if let Some(error) = comment_error {
                a.db.ks(
                    &format!("steward:close_comment_error:{repo}#{number}"),
                    &json!({"error":crate::sh::head(&error,500)}),
                );
                a.say(
                    "watcher",
                    &format!(
                        "{repo}#{number} foi fechada, mas não foi possível publicar o comentário: {}",
                        crate::sh::head(&error, 200)
                    ),
                    None,
                );
            }
            return Ok(());
        }
    }
    Ok(())
}
