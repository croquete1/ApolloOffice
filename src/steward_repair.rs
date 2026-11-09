//! Encaminhamento conservador de revisões pendentes para o Fixer existente.
//! Não publica, fecha nem integra PRs. Desligado por omissão em cada repositório.
use crate::app::Ax;
use crate::db::now;
use crate::pa;
use crate::sh::{apip, ghj};
use serde_json::{json, Value};

const ALLOWED_REVIEWERS: &[&str] = &["OWNER", "MEMBER", "COLLABORATOR"];

fn actionable_request(
    reviews: &[Value],
    comments: &[Value],
    head: &str,
    author: &str,
) -> Option<(i64, String, i64)> {
    let last = reviews
        .iter()
        .filter(|r| r["state"] == "CHANGES_REQUESTED" && r["commit_id"] == head)
        .filter(|r| ALLOWED_REVIEWERS.contains(&r["author_association"].as_str().unwrap_or("")))
        .filter(|r| {
            let user = r["user"]["login"].as_str().unwrap_or("");
            !user.is_empty()
                && !user.eq_ignore_ascii_case(author)
                && r["user"]["type"] != "Bot"
                && !user.ends_with("[bot]")
        })
        // Não reutilizar uma objeção se o mesmo revisor a aprovou/revogou depois.
        .filter(|r| {
            !reviews.iter().any(|other| {
                other["user"]["login"] == r["user"]["login"]
                    && other["submitted_at"].as_str().unwrap_or("")
                        > r["submitted_at"].as_str().unwrap_or("")
                    && (other["state"] == "APPROVED" || other["state"] == "DISMISSED")
            })
        })
        .max_by(|a, b| a["submitted_at"].as_str().cmp(&b["submitted_at"].as_str()))?;
    let id = last["id"].as_i64().filter(|id| *id > 0)?;
    let since = chrono::DateTime::parse_from_rfc3339(last["submitted_at"].as_str()?)
        .ok()?
        .timestamp_millis();
    let reviewer = last["user"]["login"].as_str()?;
    let mut parts = Vec::new();
    if let Some(body) = last["body"].as_str().filter(|s| !s.trim().is_empty()) {
        parts.push(body.trim().to_string());
    }
    for c in comments
        .iter()
        .filter(|c| c["pull_request_review_id"].as_i64() == Some(id))
        .filter(|c| c["user"]["login"].as_str() == Some(reviewer))
    {
        if let Some(body) = c["body"].as_str().filter(|s| !s.trim().is_empty()) {
            let path = c["path"].as_str().unwrap_or("ficheiro");
            parts.push(format!("{path}: {}", body.trim()));
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some((id, crate::sh::head(&parts.join("\n\n"), 6000), since))
}

fn eligible(pr: &Value, assessed: &Value) -> bool {
    pr["reviewDecision"] == "CHANGES_REQUESTED"
        && pr["isDraft"] == false
        && assessed["state"] == "revisao_pendente"
        && assessed["overlaps"].as_array().is_some_and(Vec::is_empty)
        && assessed["possible_duplicates"]
            .as_array()
            .is_some_and(Vec::is_empty)
}

async fn enqueue(
    a: &Ax,
    repo: &str,
    pr: &Value,
    assessed: &Value,
    me: &str,
) -> Result<bool, String> {
    if !eligible(pr, assessed) || a.hold("watcher") {
        return Ok(false);
    }
    let Some(number) = pr["number"].as_i64().filter(|n| *n > 0) else {
        return Ok(false);
    };
    let Some(tracked) = a.db.one(
        "select st,d from prs where repo=? and num=?",
        pa![repo.to_string(), number],
    ) else {
        return Ok(false); // Não alterar PRs que o ApolloOffice não criou/acompanha.
    };
    if tracked["st"] != "open" {
        return Ok(false);
    }
    let detail = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 45).await?;
    let Some(head) = detail["head"]["sha"].as_str() else {
        return Ok(false);
    };
    let branch = detail["head"]["ref"].as_str().unwrap_or("");
    let owner = detail["head"]["repo"]["owner"]["login"]
        .as_str()
        .unwrap_or("");
    if head != pr["headRefOid"].as_str().unwrap_or("")
        || detail["user"]["login"].as_str() != Some(me)
        || detail["base"]["repo"]["full_name"] != repo
        || detail["state"] != "open"
        || detail["draft"] != false
        || detail["head"]["repo"]["full_name"] != repo
        || !owner.eq_ignore_ascii_case(me)
        || branch.is_empty()
        || !branch
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'_' | b'.'))
    {
        return Ok(false);
    }
    let reviews = apip(&format!("repos/{repo}/pulls/{number}/reviews?per_page=100")).await?;
    let comments = apip(&format!(
        "repos/{repo}/pulls/{number}/comments?per_page=100"
    ))
    .await?;
    if reviews.len() >= 100 || comments.len() >= 100 {
        return Ok(false); // Não tomar decisões com feedback incompleto.
    }
    let Some((rid, request, since)) = actionable_request(&reviews, &comments, head, me) else {
        return Ok(false);
    };
    let key = format!("steward:repair:{repo}#{number}:{head}:{rid}");
    if !a.db.kg(&key).is_null() {
        return Ok(false);
    }
    let id = format!("review:{repo}#{number}:{rid}");
    let verdict = crate::wk::guard(a, vec![(id.clone(), request.clone())], "watcher", None)
        .await
        .map_err(|e| crate::wk::errm(&e))?;
    if verdict[&id]["inj"] != false {
        return Ok(false); // Guard sem parecer explícito = bloqueio.
    }
    if a.hold("watcher") {
        return Ok(false);
    }
    // Confirmar HEAD e estado após as chamadas externas e o Guard.
    let fresh = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 45).await?;
    if fresh["head"]["sha"] != head
        || fresh["state"] != "open"
        || fresh["draft"] != false
        || fresh["head"]["ref"] != branch
        || fresh["head"]["repo"]["full_name"] != repo
        || a.db.n(
            "select steward_repair from rp where full=? and on_=1",
            pa![repo.to_string()],
        ) != 1
    {
        return Ok(false);
    }
    let title = detail["title"].as_str().unwrap_or("Revisão pendente");
    let url = detail["html_url"].as_str().unwrap_or("");
    let test_cmd = tracked["d"]["test_cmd"].as_str().unwrap_or("");
    let body = json!({
        "change": request,
        "comment": format!("Revisão #{rid} em {repo}#{number}:\n{request}"),
        "branch": branch,
        "head": repo,
        "test_cmd": test_cmd,
        "steward_review_id": rid,
        "steward_head": head
    });
    // Única inserção atómica; impede corrida com o Watcher na fila ativa.
    let inserted = a.db.ch(
        "insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1)
         select ?,?,'review',?,?,'ready',?,?,?,-1.0,?,?
         where not exists (
           select 1 from jobs where repo=? and pr=? and kind='review'
             and (st not in ('done','drop') or t0 >= ?)
         )",
        pa![
            repo.to_string(),
            number,
            title.to_string(),
            url.to_string(),
            format!("Revisão humana #{rid} no HEAD atual"),
            body.to_string(),
            number,
            now(),
            now(),
            repo.to_string(),
            number,
            since
        ],
    ) == 1;
    if inserted {
        a.db.ks(&key, &json!({"at":now(),"review_id":rid,"head":head}));
        a.say(
            "watcher",
            &format!("{repo}#{number}: revisão humana encaminhada para o Fixer"),
            None,
        );
    }
    Ok(inserted)
}

/// Encaminha no máximo uma revisão por repositório e sincronização.
pub async fn run(a: &Ax, repo: &str, listed: &[Value], assessed: &[Value]) {
    if a.hold("watcher")
        || a.db.n(
            "select steward_repair from rp where full=? and on_=1",
            pa![repo.to_string()],
        ) != 1
    {
        return;
    }
    let me = crate::gh::login(a).await;
    if me.is_empty() {
        return;
    }
    for (pr, status) in listed.iter().zip(assessed.iter()) {
        if !eligible(pr, status) {
            continue;
        }
        match enqueue(a, repo, pr, status, &me).await {
            Ok(true) => break,
            Ok(false) => {}
            Err(error) => {
                a.db.ks(
                    &format!("steward:repair_error:{repo}"),
                    &json!({"at":now(),"pr":pr["number"],"error":crate::sh::head(&error,500)}),
                );
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denies_draft_overlaps_and_unrequested_changes() {
        let mut pr = json!({"isDraft":false,"reviewDecision":"CHANGES_REQUESTED"});
        let mut assessment =
            json!({"state":"revisao_pendente","overlaps":[],"possible_duplicates":[]});
        assert!(eligible(&pr, &assessment));
        pr["isDraft"] = json!(true);
        assert!(!eligible(&pr, &assessment));
        pr["isDraft"] = json!(false);
        assessment["overlaps"] = json!([7]);
        assert!(!eligible(&pr, &assessment));
        assessment["overlaps"] = json!([]);
        pr["reviewDecision"] = json!("APPROVED");
        assert!(!eligible(&pr, &assessment));
    }

    #[test]
    fn only_maintainer_requests_on_current_head_are_actionable() {
        let head = "0123456789012345678901234567890123456789";
        let review = json!({"id":91,"state":"CHANGES_REQUESTED","commit_id":head,
            "submitted_at":"2026-11-09T12:00:00Z","author_association":"COLLABORATOR",
            "user":{"login":"reviewer","type":"User"},"body":"Rever tratamento de null"});
        let comments = json!([{"pull_request_review_id":91,"user":{"login":"reviewer"},
            "path":"src/form.ts","body":"Adicionar teste de regressão"}]);
        let req = actionable_request(
            &[review.clone()],
            comments.as_array().unwrap(),
            head,
            "author",
        );
        assert!(req.as_ref().is_some_and(|(id, text, _)| *id == 91
            && text.contains("null")
            && text.contains("src/form.ts")));
        assert!(actionable_request(&[review.clone()], &[], head, "reviewer").is_none());
        assert!(actionable_request(&[review.clone()], &[], "outro-head", "author").is_none());
        let mut outsider = review.clone();
        outsider["author_association"] = json!("NONE");
        assert!(actionable_request(&[outsider], &[], head, "author").is_none());
        let mut newer = review.clone();
        newer["state"] = json!("APPROVED");
        newer["submitted_at"] = json!("2026-11-09T13:00:00Z");
        assert!(
            actionable_request(&[review, newer], &[], head, "author").is_none(),
            "uma aprovação posterior torna a objeção obsoleta"
        );
    }

    #[test]
    fn empty_review_is_not_a_task() {
        let head = "0123456789012345678901234567890123456789";
        let review = json!({"id":91,"state":"CHANGES_REQUESTED","commit_id":head,
            "author_association":"OWNER","user":{"login":"maintainer","type":"User"},"body":""});
        assert!(actionable_request(&[review], &[], head, "author").is_none());
    }
}
