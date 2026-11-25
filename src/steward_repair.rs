//! Encaminhamento conservador de revisões pendentes para o Fixer existente.
//! Não publica, fecha nem integra PRs. Desligado por omissão em cada repositório.
use crate::app::Ax;
use crate::db::now;
use crate::pa;
use crate::sh::{apip_unique, ghj};
use serde_json::{json, Value};

const ALLOWED_REVIEWERS: &[&str] = &["OWNER", "MEMBER", "COLLABORATOR"];

// Compare actual instants, not RFC3339 strings (offsets and fractional
// seconds can sort differently). Ambiguous revocations block automation.
fn review_time(review: &Value) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(review["submitted_at"].as_str()?).ok()
}

fn actionable_request(
    reviews: &[Value],
    comments: &[Value],
    head: &str,
    author: &str,
) -> Option<(i64, String, i64)> {
    // Incomplete identity on a potentially eligible request must not
    // silently authorize an older request from the same maintainer.
    for review in reviews.iter().filter(|r| {
        r["state"] == "CHANGES_REQUESTED"
            && r["commit_id"] == head
            && ALLOWED_REVIEWERS.contains(&r["author_association"].as_str().unwrap_or(""))
    }) {
        let user = &review["user"];
        if user["type"] == "Bot" {
            continue;
        }
        if user["type"] != "User"
            || !user["id"].as_i64().is_some_and(|id| id > 0)
            || user["login"].as_str().is_none_or(str::is_empty)
        {
            return None;
        }
    }
    let mut requests = reviews
        .iter()
        .filter(|r| r["state"] == "CHANGES_REQUESTED" && r["commit_id"] == head)
        .filter(|r| ALLOWED_REVIEWERS.contains(&r["author_association"].as_str().unwrap_or("")))
        .filter(|r| {
            let user = r["user"]["login"].as_str().unwrap_or("");
            !user.is_empty()
                && !user.eq_ignore_ascii_case(author)
                && r["user"]["type"] == "User"
                && r["user"]["id"].as_i64().is_some_and(|id| id > 0)
                && !user.to_ascii_lowercase().ends_with("[bot]")
        })
        // A malformed eligible request might supersede a valid older one.
        // Reject ambiguous chronology rather than silently dropping it.
        .map(|r| Some((r, review_time(r)?)))
        .collect::<Option<Vec<_>>>()?;
    // Não reutilizar uma objeção se o mesmo revisor a aprovou/revogou depois.
    requests.retain(|(r, at)| {
        let reviewer = r["user"]["login"].as_str().unwrap_or("");
        !reviews.iter().any(|other| {
            (other["user"]["id"] == r["user"]["id"]
                || other["user"]["login"]
                    .as_str()
                    .is_some_and(|login| login.eq_ignore_ascii_case(reviewer)))
                && (other["state"] == "APPROVED" || other["state"] == "DISMISSED")
                && review_time(other).is_none_or(|other_at| other_at >= *at)
        })
    });
    requests.sort_by_key(|(_, at)| *at);
    let (last, at) = requests.pop()?;
    if requests.last().is_some_and(|(_, other_at)| *other_at == at) {
        return None;
    }
    let id = last["id"].as_i64().filter(|id| *id > 0)?;
    let since = at.timestamp_millis();
    let reviewer = last["user"]["login"].as_str()?;
    let mut parts = Vec::new();
    if let Some(body) = last["body"].as_str().filter(|s| !s.trim().is_empty()) {
        parts.push(body.trim().to_string());
    }
    for c in comments
        .iter()
        .filter(|c| c["pull_request_review_id"].as_i64() == Some(id))
    {
        // An inline comment must belong to the same human account.
        if c["user"]["id"] != last["user"]["id"]
            || c["user"]["type"] != "User"
            || !c["user"]["login"]
                .as_str()
                .is_some_and(|login| login.eq_ignore_ascii_case(reviewer))
        {
            return None;
        }
        if let Some(body) = c["body"].as_str().filter(|s| !s.trim().is_empty()) {
            let path = c["path"].as_str().unwrap_or("ficheiro");
            parts.push(format!("{path}: {}", body.trim()));
        }
    }
    if parts.is_empty() {
        return None;
    }
    let request = parts.join("\n\n");
    // The Guard inspects at most 3500 bytes. Never authorize a prefix and
    // forward a longer review, even when it came from a maintainer.
    if request.len() > 3500 {
        return None;
    }
    Some((id, request, since))
}

fn permission_matches_reviewer(permission: &Value, reviewer: &Value) -> bool {
    reviewer["id"].as_i64().is_some_and(|id| id > 0)
        && permission["user"]["id"] == reviewer["id"]
        && permission["user"]["type"] == "User"
        && permission["user"]["login"]
            .as_str()
            .zip(reviewer["login"].as_str())
            .is_some_and(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
        && matches!(
            permission["permission"].as_str(),
            Some("admin" | "write" | "maintain")
        )
}

async fn reviewer_still_authorized(
    repo: &str,
    reviews: &[Value],
    rid: i64,
) -> Result<bool, String> {
    let Some(review) = reviews.iter().find(|r| r["id"].as_i64() == Some(rid)) else {
        return Ok(false);
    };
    let reviewer = &review["user"];
    let Some(login) = reviewer["login"].as_str().filter(|login| {
        !login.is_empty()
            && login
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }) else {
        return Ok(false);
    };
    let permission = ghj(
        &[
            "api",
            &format!("repos/{repo}/collaborators/{login}/permission"),
        ],
        45,
    )
    .await?;
    Ok(permission_matches_reviewer(&permission, reviewer))
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

// Admission, authorization and durable deduplication share one write transaction.
fn admit_review(
    db: &crate::db::Db,
    scope: (&str, i64, &str),
    review: (i64, i64),
    title: &str,
    url: &str,
    body: &Value,
) -> rusqlite::Result<bool> {
    let (repo, number, head) = scope;
    let (rid, since) = review;
    let key = format!("steward:repair:{repo}#{number}:{head}:{rid}");
    let activation = body["steward_activation"]
        .as_str()
        .filter(|token| token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let mut connection = db.0.lock().unwrap();
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let eligible: bool = tx.query_row(
        "select exists(select 1 from rp where full=?1 and on_=1 and steward_repair=1 and steward_activation=?7)
         and exists(select 1 from prs where repo=?1 and num=?2 and st='open')
         and exists(select 1 from kv,json_each(kv.v,'$.prs') p where kv.k=?3
           and json_extract(p.value,'$.number')=?2
           and json_extract(p.value,'$.head_sha')=?4
           and json_extract(p.value,'$.state')='revisao_pendente'
           and json_type(p.value,'$.overlaps')='array' and json_array_length(p.value,'$.overlaps')=0
           and json_type(p.value,'$.possible_duplicates')='array' and json_array_length(p.value,'$.possible_duplicates')=0)
         and not exists(select 1 from kv where k=?5)
         and not exists(select 1 from jobs where repo=?1 and pr=?2
           and ((kind in ('review','rebase','mergecheck') and st not in ('done','drop'))
                or (kind='review' and t0>=?6)))",
        rusqlite::params![repo, number, format!("steward:{repo}"), head, key, since, activation],
        |row| row.get(0),
    )?;
    if !eligible {
        return Ok(false);
    }
    let at = now();
    tx.execute(
        "insert into jobs(repo,num,kind,title,url,st,why,d,pr,prio,t0,t1)
         values(?,?,'review',?,?,'ready',?,?,?,-1.0,?,?)",
        rusqlite::params![
            repo,
            number,
            title,
            url,
            format!("Revisão humana #{rid} no HEAD atual"),
            body.to_string(),
            number,
            at,
            at
        ],
    )?;
    tx.execute(
        "insert into kv(k,v) values(?,?)",
        rusqlite::params![
            key,
            json!({"at":at,"review_id":rid,"head":head}).to_string()
        ],
    )?;
    tx.commit()?;
    crate::app::chg();
    Ok(true)
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
    let policy =
        a.db.try_all(
            "select steward_activation from rp where full=? and on_=1 and steward_repair=1",
            pa![repo.to_string()],
        )
        .map_err(|error| error.to_string())?;
    let Some(activation) = policy
        .first()
        .and_then(|row| row["steward_activation"].as_str())
    else {
        return Ok(false);
    };
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
    let reviews = apip_unique(&format!("repos/{repo}/pulls/{number}/reviews?per_page=100")).await?;
    let comments = apip_unique(&format!(
        "repos/{repo}/pulls/{number}/comments?per_page=100"
    ))
    .await?;
    let Some((rid, request, since)) = actionable_request(&reviews, &comments, head, me) else {
        return Ok(false);
    };
    if !reviewer_still_authorized(repo, &reviews, rid).await? {
        return Ok(false);
    }
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
    // Re-read review evidence: a review can be edited or dismissed without
    // changing the commit SHA. A previous Guard verdict is not transferable.
    let current_reviews =
        apip_unique(&format!("repos/{repo}/pulls/{number}/reviews?per_page=100")).await?;
    let current_comments = apip_unique(&format!(
        "repos/{repo}/pulls/{number}/comments?per_page=100"
    ))
    .await?;
    if actionable_request(&current_reviews, &current_comments, head, me)
        != Some((rid, request.clone(), since))
    {
        return Ok(false);
    }
    if !reviewer_still_authorized(repo, &current_reviews, rid).await? {
        return Ok(false);
    }
    let fresh = ghj(&["api", &format!("repos/{repo}/pulls/{number}")], 45).await?;
    if fresh["head"]["sha"] != head
        || fresh["state"] != "open"
        || fresh["draft"] != false
        || fresh["head"]["ref"] != branch
        || fresh["head"]["repo"]["full_name"] != repo
        || fresh["base"]["repo"]["full_name"] != repo
        || fresh["base"]["ref"] != detail["base"]["ref"]
        || fresh["user"]["login"].as_str() != Some(me)
        || fresh["updated_at"] != detail["updated_at"]
        || a.hold("watcher")
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
        "steward_activation": activation,
        "steward_head": head,
        "source_head": head
    });
    let inserted = admit_review(&a.db, (repo, number, head), (rid, since), title, url, &body)
        .map_err(|error| error.to_string())?;
    if inserted {
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
            "user":{"login":"reviewer","type":"User","id":11},"body":"Rever tratamento de null"});
        let comments = json!([{"pull_request_review_id":91,"user":{"login":"reviewer","type":"User","id":11},
            "path":"src/form.ts","body":"Adicionar teste de regressão"}]);
        let req = actionable_request(
            std::slice::from_ref(&review),
            comments.as_array().unwrap(),
            head,
            "author",
        );
        assert!(req.as_ref().is_some_and(|(id, text, _)| *id == 91
            && text.contains("null")
            && text.contains("src/form.ts")));
        assert!(actionable_request(std::slice::from_ref(&review), &[], head, "reviewer").is_none());
        assert!(
            actionable_request(std::slice::from_ref(&review), &[], "outro-head", "author")
                .is_none()
        );
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
    fn review_revocations_use_actual_instants_and_fail_closed() {
        let head = "0123456789012345678901234567890123456789";
        let request = json!({"id":91,"state":"CHANGES_REQUESTED","commit_id":head,
            "submitted_at":"2026-11-09T12:00:00Z","author_association":"OWNER",
            "user":{"login":"maintainer","type":"User","id":12},"body":"Fix the regression"});
        let mut approval = request.clone();
        approval["id"] = json!(92);
        approval["state"] = json!("APPROVED");
        approval["user"]["login"] = json!("Maintainer");
        for at in [
            json!("2026-11-09T11:30:00-01:00"),
            json!("2026-11-09T12:00:00Z"),
            json!("invalid"),
            Value::Null,
        ] {
            approval["submitted_at"] = at;
            assert!(
                actionable_request(&[request.clone(), approval.clone()], &[], head, "author")
                    .is_none()
            );
        }
        approval["submitted_at"] = json!("2026-11-09T13:30:00+02:00");
        assert!(actionable_request(&[request.clone(), approval], &[], head, "author").is_some());
        let mut newer = request.clone();
        newer["id"] = json!(93);
        newer["submitted_at"] = json!("2026-11-09T11:30:00-01:00");
        let selected = actionable_request(&[request, newer], &[], head, "author").unwrap();
        assert_eq!(selected.0, 93);
    }

    #[test]
    fn ambiguous_change_requests_cannot_authorize_older_work() {
        let head = "0123456789012345678901234567890123456789";
        let request = json!({"id":91,"state":"CHANGES_REQUESTED","commit_id":head,
            "submitted_at":"2026-11-09T12:00:00Z","author_association":"OWNER",
            "user":{"login":"maintainer","type":"User","id":12},"body":"Fix the regression"});
        let mut other = request.clone();
        other["id"] = json!(92);
        other["body"] = json!("Use a different fix");
        for at in [
            json!("invalid"),
            Value::Null,
            json!("2026-11-09T13:00:00+01:00"),
        ] {
            other["submitted_at"] = at;
            assert!(
                actionable_request(&[request.clone(), other.clone()], &[], head, "author")
                    .is_none()
            );
        }
        other["submitted_at"] = json!("2026-11-09T13:00:01+01:00");
        assert_eq!(
            actionable_request(&[request, other], &[], head, "author")
                .unwrap()
                .0,
            92
        );
    }

    #[test]
    fn incomplete_maintainer_identity_blocks_older_request() {
        let head = "0123456789012345678901234567890123456789";
        let request = json!({"id":91,"state":"CHANGES_REQUESTED","commit_id":head,
            "submitted_at":"2026-11-09T12:00:00Z","author_association":"OWNER",
            "user":{"login":"maintainer","type":"User","id":12},"body":"Fix regression"});
        let mut newer = request.clone();
        newer["id"] = json!(92);
        newer["submitted_at"] = json!("2026-11-09T13:00:00Z");
        newer["user"]["id"] = Value::Null;
        assert!(
            actionable_request(&[request.clone(), newer.clone()], &[], head, "author")
                .is_none()
        );
        newer["user"]["id"] = json!(12);
        assert_eq!(
            actionable_request(&[request, newer], &[], head, "author")
                .unwrap()
                .0,
            92
        );
    }

    #[test]
    fn current_repository_permission_must_match_reviewer() {
        let reviewer = json!({"login":"maintainer","id":12,"type":"User"});
        let mut permission = json!({"permission":"write","user":reviewer.clone()});
        assert!(permission_matches_reviewer(&permission, &reviewer));
        permission["permission"] = json!("read");
        assert!(!permission_matches_reviewer(&permission, &reviewer));
        permission["permission"] = json!("admin");
        permission["user"]["id"] = json!(13);
        assert!(!permission_matches_reviewer(&permission, &reviewer));
    }

    #[test]
    fn review_comments_require_the_same_human_identity() {
        let head = "0123456789012345678901234567890123456789";
        let review = json!({"id":91,"state":"CHANGES_REQUESTED","commit_id":head,
            "submitted_at":"2026-11-09T12:00:00Z","author_association":"OWNER",
            "user":{"login":"maintainer","type":"User","id":12},"body":"Fix regression"});
        let mut comment = json!({"pull_request_review_id":91,
            "user":{"login":"Maintainer","type":"User","id":12},
            "path":"src/main.rs","body":"Add a test"});
        let requests = std::slice::from_ref(&review);
        assert!(actionable_request(requests, &[comment.clone()], head, "author").is_some());
        comment["user"]["id"] = json!(13);
        assert!(actionable_request(requests, &[comment], head, "author").is_none());
        let mut invalid = review.clone();
        invalid["user"]["type"] = Value::Null;
        assert!(actionable_request(&[invalid], &[], head, "author").is_none());

        let mut approval = review.clone();
        approval["state"] = json!("APPROVED");
        approval["submitted_at"] = json!("2026-11-09T13:00:00Z");
        approval["user"]["login"] = json!("renamed-maintainer");
        assert!(actionable_request(&[review, approval], &[], head, "author").is_none());
    }

    #[test]
    fn oversized_review_is_not_truncated_into_an_authorization() {
        let head = "0123456789012345678901234567890123456789";
        let review = json!({"id":91,"state":"CHANGES_REQUESTED","commit_id":head,
            "submitted_at":"2026-11-09T12:00:00Z","author_association":"OWNER",
            "user":{"login":"maintainer","type":"User","id":12},"body":"é".repeat(1751)});
        assert!(actionable_request(&[review], &[], head, "author").is_none());
    }

    #[test]
    fn admission_rechecks_report_authorization_and_commits_deduplication() {
        let db = crate::db::Db::mem();
        let repo = "o/r";
        let head = "0123456789012345678901234567890123456789";
        db.run(
            "insert into rp(full,on_,steward_repair) values('o/r',1,1)",
            vec![],
        );
        db.run(
            "insert into prs(repo,num,st,d) values('o/r',1,'open','{}')",
            vec![],
        );
        let policy = db.one("select steward_activation from rp", vec![]).unwrap();
        let mut body = json!({"source_head":head,"steward_head":head,"steward_activation":policy["steward_activation"]});
        let admit = |body: &Value| admit_review(&db, (repo, 1, head), (91, 0), "PR", "url", body);
        assert!(!admit(&body).unwrap());
        db.ks("steward:o/r", &json!({"prs":[{"number":1,"head_sha":head,"state":"revisao_pendente","overlaps":[2],"possible_duplicates":[]}]}));
        assert!(!admit(&body).unwrap());
        db.ks("steward:o/r", &json!({"prs":[{"number":1,"head_sha":head,"state":"revisao_pendente","overlaps":[],"possible_duplicates":[]}]}));
        db.run("update rp set steward_repair=0", vec![]);
        assert!(!admit(&body).unwrap());
        db.run("update rp set steward_repair=1", vec![]);
        assert!(!admit(&body).unwrap());
        body["steward_activation"] = db.one("select steward_activation from rp", vec![]).unwrap()
            ["steward_activation"]
            .clone();
        db.0.lock().unwrap().execute_batch("create trigger fail_repair_ledger before insert on kv begin select raise(abort,'ledger failure'); end;").unwrap();
        assert!(admit(&body).is_err());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 0);
        db.0.lock()
            .unwrap()
            .execute_batch("drop trigger fail_repair_ledger")
            .unwrap();
        assert!(admit(&body).unwrap());
        db.run("update jobs set st='done'", vec![]);
        assert!(!admit(&body).unwrap());
        assert_eq!(db.n("select count(*) from jobs", vec![]), 1);
        assert!(!db
            .kg(&format!("steward:repair:{repo}#1:{head}:91"))
            .is_null());
    }

    #[test]
    fn empty_review_is_not_a_task() {
        let head = "0123456789012345678901234567890123456789";
        let review = json!({"id":91,"state":"CHANGES_REQUESTED","commit_id":head,
            "submitted_at":"2026-11-09T12:00:00Z","author_association":"OWNER",
            "user":{"login":"maintainer","type":"User","id":12},"body":""});
        assert!(actionable_request(&[review], &[], head, "author").is_none());
    }
}
