//! Read-only CodeRabbit evidence. Never grants review/merge authority.
use std::collections::BTreeSet;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

const BOT_ID: u64 = 136622811;
const COVERAGE: &str = "<!-- final_review_risk_coverage:";
const PAGE_SIZE: usize = 10;
const MAX_PAGES: usize = 20;
const MAX_BYTES: usize = 4 * 1024 * 1024;
pub(crate) static OBSERVATIONS: Semaphore = Semaphore::const_new(2);

const METADATA_BYTES: usize = 512 * 1024;
const API: &str = "https://api.github.com";

// Separate, anonymous public reads: no ambient gh host or stored GitHub tokens.
// Refuse redirects rather than following a changed repository/host.
fn public_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("ApolloOffice-CodeRabbit-observation")
        .build()
        .map_err(|_| "public GitHub client unavailable".into())
}

async fn read_public(client: &reqwest::Client, endpoint: &str, limit: usize) -> Result<String, String> {
    let response = client.get(format!("{API}/{endpoint}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send().await.map_err(|_| "public GitHub request failed")?;
    if !response.status().is_success() {
        return Err(format!("public GitHub HTTP {}", response.status().as_u16()));
    }
    bounded_body(response, limit).await
}

async fn bounded_body(mut response: reqwest::Response, limit: usize) -> Result<String, String> {
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("public GitHub response exceeds byte budget".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "public GitHub read failed")? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err("public GitHub response exceeds byte budget".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "invalid public GitHub UTF-8".into())
}

async fn public_json(client: &reqwest::Client, endpoint: &str) -> Result<Value, String> {
    let raw = read_public(client, endpoint, METADATA_BYTES).await?;
    serde_json::from_str(&raw).map_err(|_| "invalid public GitHub metadata".into())
}

fn count_matches(before: &Value, after: &Value, collected: usize) -> bool {
    let count = collected as u64;
    before["comments"].as_u64() == Some(count) && after["comments"].as_u64() == Some(count)
}

fn accept_page(rows: &mut Vec<Value>, ids: &mut BTreeSet<u64>, bytes: &mut usize, raw: &str) -> Result<bool, String> {
    *bytes = bytes.checked_add(raw.len()).ok_or("comment byte counter overflow")?;
    if *bytes > MAX_BYTES {
        return Err("CodeRabbit comment byte budget exceeded".into());
    }
    let page: Vec<Value> = serde_json::from_str(raw).map_err(|_| "invalid comment page")?;
    if page.len() > PAGE_SIZE {
        return Err("invalid comment page size".into());
    }
    let complete = page.len() < PAGE_SIZE;
    for row in page {
        let id = row["id"].as_u64().filter(|id| *id > 0).ok_or("invalid comment ID")?;
        if !ids.insert(id) {
            return Err("comment inventory changed during pagination".into());
        }
        rows.push(row);
    }
    Ok(complete)
}

async fn bounded_comments(client: &reqwest::Client, repo: &str, number: i64) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    let mut ids = BTreeSet::new();
    let mut bytes = 0;
    for page in 1..=MAX_PAGES {
        let endpoint = format!("repos/{repo}/issues/{number}/comments?per_page={PAGE_SIZE}&page={page}");
        let raw = read_public(client, &endpoint, MAX_BYTES.saturating_sub(bytes)).await?;
        if accept_page(&mut rows, &mut ids, &mut bytes, &raw)? {
            return Ok(rows);
        }
    }
    Err("CodeRabbit comment page budget exceeded".into())
}

fn public_repository(value: &Value, repo: &str) -> bool {
    value["private"] == false
        && value["visibility"] == "public"
        && value["full_name"] == repo
        && value["id"].as_u64().is_some_and(|id| id > 0)
}

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
    // These observed markers are advisory only; a processing/skipped summary
    // must not inherit coverage from a previous review.
    if body.contains("<!-- This is an auto-generated comment: review in progress by coderabbit.ai -->") {
        result["state"] = json!("pending");
        return result;
    }
    if body.contains("<!-- This is an auto-generated comment: skip review by coderabbit.ai -->") {
        result["state"] = json!("skipped");
        return result;
    }
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
    let client = public_client()?;
    let repository = public_json(&client, &format!("repos/{repo}")).await?;
    if !public_repository(&repository, repo) {
        return Err("Only verified public repositories are eligible".into());
    }
    let endpoint = format!("repos/{repo}/pulls/{number}");
    let before = public_json(&client, &endpoint).await?;
    if !matches_pr(&before, repo, number, head) {
        return Err("PR scope or HEAD changed; refresh Steward".into());
    }
    let count = before["comments"].as_u64().ok_or("missing PR comment count")?;
    if count >= (PAGE_SIZE * MAX_PAGES) as u64 {
        return Err("PR comment count exceeds observation budget".into());
    }
    let rows = tokio::time::timeout(std::time::Duration::from_secs(60), bounded_comments(&client, repo, number))
        .await.map_err(|_| "CodeRabbit comment read timed out")??;
    let after = public_json(&client, &endpoint).await?;
    if !matches_pr(&after, repo, number, head) || before["updated_at"] != after["updated_at"] {
        return Err("PR changed during observation; refresh Steward".into());
    }
    if !count_matches(&before, &after, rows.len()) {
        return Err("PR comment count changed or inventory incomplete".into());
    }
    let final_repo = public_json(&client, &format!("repos/{repo}")).await?;
    if !public_repository(&final_repo, repo) || final_repo["id"] != repository["id"] {
        return Err("Repository visibility or identity changed".into());
    }
    Ok(classify(&rows, head))
}

// Reuse the versioned database's existing KV table; one bounded record per PR.
// A durable ticket prevents a slower concurrent read from replacing a newer one.
const RESERVE: &str = "insert into kv(k,v)
    select ?1,json_object('ticket',lower(hex(randomblob(16)))) where exists(select 1 from rp where full=?2 and on_=1)
    on conflict(k) do update set v=json_set(kv.v,'$.ticket',lower(hex(randomblob(16))))
    returning json_extract(v,'$.ticket')";
const SAVE: &str = "update kv set v=json_set(v,'$.observation',json(?3),'$.checked_at',?4)
    where k=?1 and json_extract(v,'$.ticket')=?2
    and exists(select 1 from rp where full=?5 and on_=1)";

fn observation_key(repo: &str, number: i64) -> String {
    format!("coderabbit:observation:{repo}#{number}")
}

pub(crate) fn reserve(db: &crate::db::Db, repo: &str, number: i64) -> rusqlite::Result<String> {
    let c = db.0.lock().unwrap();
    c.query_row(RESERVE, rusqlite::params![observation_key(repo, number), repo], |r| r.get(0))
}

pub(crate) fn save(db: &crate::db::Db, repo: &str, number: i64, ticket: &str, value: &Value) -> rusqlite::Result<bool> {
    let c = db.0.lock().unwrap();
    let changed = c.execute(SAVE, rusqlite::params![
        observation_key(repo, number), ticket, value.to_string(), crate::db::now(), repo
    ])?;
    Ok(changed == 1)
}

pub(crate) fn previous(db: &crate::db::Db, repo: &str, number: i64) -> Value {
    // Display-only historical data; never used in scheduling or merge decisions.
    let row = db.kg(&observation_key(repo, number));
    let value = &row["observation"];
    if !value.is_object() || row["checked_at"].as_i64().is_none() {
        return Value::Null;
    }
    json!({"observation":value,"checked_at":row["checked_at"]})
}

pub(crate) fn valid_scope(repo: &str, number: i64, head: &str) -> bool {
    valid_repo(repo) && number > 0 && valid_sha(head)
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
    fn processing_or_skipped_summary_does_not_inherit_coverage() {
        let mut progress = row(HEAD);
        progress["body"] = json!(format!("{}\n<!-- This is an auto-generated comment: review in progress by coderabbit.ai -->", progress["body"].as_str().unwrap()));
        assert_eq!(classify(&[progress], HEAD)["state"], "pending");
        let mut skipped = row(HEAD);
        skipped["body"] = json!(format!("{}\n<!-- This is an auto-generated comment: skip review by coderabbit.ai -->", skipped["body"].as_str().unwrap()));
        assert_eq!(classify(&[skipped], HEAD)["state"], "skipped");
    }
    #[test]
    fn durable_ticket_and_history_are_scoped() {
        let db = crate::db::Db::mem();
        let repo = "croquete1/ApolloOffice";
        db.0.lock().unwrap().execute("insert into rp(full,on_) values(?,1)", [repo]).unwrap();
        let first = reserve(&db, repo, 67).unwrap();
        let second = reserve(&db, repo, 67).unwrap();
        assert_ne!(second, first);
        let value = classify(&[row(HEAD)], HEAD);
        assert!(!save(&db, repo, 67, &first, &value).unwrap());
        assert!(save(&db, repo, 67, &second, &value).unwrap());
        assert_eq!(previous(&db, repo, 67)["observation"], value);
        assert!(previous(&db, repo, 68).is_null());
        let third = reserve(&db, repo, 67).unwrap();
        assert_eq!(previous(&db, repo, 67)["observation"], value);
        db.0.lock().unwrap().execute("update rp set on_=0 where full=?", [repo]).unwrap();
        assert!(!save(&db, repo, 67, &third, &value).unwrap());
        assert!(reserve(&db, repo, 67).is_err());
        db.0.lock().unwrap().execute("update rp set on_=1 where full=?", [repo]).unwrap();
        let fourth = reserve(&db, repo, 67).unwrap();
        assert!(previous(&db, repo, 67).is_null());
        assert!(!save(&db, repo, 67, &third, &value).unwrap());
        assert!(save(&db, repo, 67, &fourth, &value).unwrap());
        db.0.lock().unwrap().execute("delete from rp where full=?", [repo]).unwrap();
        db.0.lock().unwrap().execute("insert into rp(full,on_) values(?,1)", [repo]).unwrap();
        assert!(previous(&db, repo, 67).is_null());
        assert!(!save(&db, repo, 67, &fourth, &value).unwrap());
    }
    #[test]
    fn page_budgets_and_duplicates_fail_closed() {
        let (mut rows, mut ids, mut bytes) = (Vec::new(), BTreeSet::new(), 0);
        let page = json!((1..=PAGE_SIZE).map(|id| json!({"id":id})).collect::<Vec<_>>()).to_string();
        assert!(!accept_page(&mut rows, &mut ids, &mut bytes, &page).unwrap());
        assert!(accept_page(&mut rows, &mut ids, &mut bytes, &page).is_err());
        let (mut rows, mut ids, mut bytes) = (Vec::new(), BTreeSet::new(), MAX_BYTES);
        assert!(accept_page(&mut rows, &mut ids, &mut bytes, "[]").is_err());
        let (mut rows, mut ids, mut bytes) = (Vec::new(), BTreeSet::new(), 0);
        assert!(accept_page(&mut rows, &mut ids, &mut bytes, "[]").unwrap());
    }
    async fn mock_response(wire: &'static [u8]) -> reqwest::Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            let _ = socket.write_all(wire).await;
        });
        reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(5))
            .build().unwrap().get(format!("http://{addr}/")).send().await.unwrap()
    }

    #[tokio::test]
    async fn streamed_response_obeys_byte_budget() {
        let exact = mock_response(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd").await;
        assert_eq!(bounded_body(exact, 4).await.unwrap(), "abcd");
        let declared = mock_response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nabcde").await;
        assert!(bounded_body(declared, 4).await.is_err());
        let chunked = mock_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n3\r\ndef\r\n0\r\n\r\n").await;
        assert!(bounded_body(chunked, 4).await.is_err());
    }
    #[test]
    fn comment_counts_reject_offset_gap_even_with_same_timestamp() {
        let before = json!({"comments":20,"updated_at":"2026-11-24T12:22:00Z"});
        let after = json!({"comments":19,"updated_at":"2026-11-24T12:22:00Z"});
        assert!(!count_matches(&before, &after, 19));
        assert!(!count_matches(&before, &before, 19));
        assert!(!count_matches(&before, &Value::Null, 20));
        assert!(count_matches(&before, &before, 20));
        assert!(count_matches(&json!({"comments":0}), &json!({"comments":0}), 0));
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
