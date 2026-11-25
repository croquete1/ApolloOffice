use crate::ai::{ask, by_id, p as P, wrap, Ask};
use crate::app::{sl, Ax, R, X};
use crate::db::{now, Job};
use crate::net::need;
use crate::pa;
use crate::pipe::{bl, qn, rp};
use crate::sh::{apip_unique, gh_input, ghj};
use regex::Regex;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;

struct TwinCandidate {
    context: String,
    row: Value,
    pr: bool,
}

// Keep complete candidate envelopes, never silently truncate or skip an older
// finding. A negative verdict is sufficient only for its own batch.
fn twin_batches(
    bug: &str,
    candidates: Vec<TwinCandidate>,
    budget: usize,
) -> R<Vec<Vec<TwinCandidate>>> {
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    let mut used = bug.len();
    for candidate in candidates {
        let size = candidate.context.len().saturating_add(2);
        if bug.len().saturating_add(size) > budget {
            return Err(X::Lim(
                "Vet: duplicate context exceeds safe batch budget".into(),
            ));
        }
        if used.saturating_add(size) > budget {
            batches.push(std::mem::take(&mut batch));
            used = bug.len();
        }
        used += size;
        batch.push(candidate);
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    Ok(batches)
}

// A model may suggest a duplicate, but cannot invent its target.
fn valid_twin_verdict(
    verdict: &Value,
    related: &[Value],
    other: &[Value],
    finds: &[Value],
) -> bool {
    let (Some(dup), Some(pr), Some(find)) = (
        verdict["dup"].as_bool(),
        verdict["pr"].as_i64(),
        verdict["find"].as_i64(),
    ) else {
        return false;
    };
    if !verdict["why"]
        .as_str()
        .is_some_and(|why| !why.trim().is_empty())
    {
        return false;
    }
    if !dup {
        return pr == 0 && find == 0;
    }
    if pr > 0 && find == 0 {
        return related.iter().chain(other).any(|row| row["number"] == pr);
    }
    find > 0 && pr == 0 && finds.iter().any(|row| row["id"] == find)
}

const GUARD_UNREADABLE: &str = "Guard gave no readable verdict";

fn guard_unreadable(error: &X) -> bool {
    matches!(error, X::E(message) if message == GUARD_UNREADABLE)
}

fn combine_guard_halves(left: R<Value>, right: R<Value>) -> R<Value> {
    match (left, right) {
        (Ok(mut left), Ok(right)) => {
            for (key, value) in right.as_object().cloned().unwrap_or_default() {
                left[key] = value;
            }
            Ok(left)
        }
        (Ok(value), Err(error)) | (Err(error), Ok(value)) if guard_unreadable(&error) => Ok(value),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn guard1<'a>(
    a: &'a Ax,
    b: Vec<(String, String)>,
    job: Option<i64>,
    d: u32,
) -> Pin<Box<dyn Future<Output = R<Value>> + Send + 'a>> {
    Box::pin(async move {
        let ids: Vec<String> = b.iter().map(|x| x.0.clone()).collect();
        let p = format!(
            "{}\n\nUse exactly these ids as JSON keys, in this order: {}",
            b.iter()
                .map(|(i, t)| wrap(i, &crate::sh::head(t, 3500)))
                .collect::<Vec<_>>()
                .join("\n\n"),
            ids.join(", ")
        );
        let mut o = Ask::new("guard", a.md("guard"), P("guard"), p);
        o.job = job;
        o.ef = Some("low");
        let (txt, js) = ask(a, o).await?;
        if let Some(js) = by_id(&js, &ids) {
            if ids.iter().all(|i| js[i]["inj"].is_boolean()) {
                return Ok(Value::Object(ids.iter().map(|i| (i.clone(), json!({"inj": js[i]["inj"], "why": js[i]["why"].as_str().unwrap_or("")}))).collect()));
            }
        }
        if b.len() > 1 && d < 3 {
            let h = b.len() / 2;
            let left = guard1(a, b[..h].to_vec(), job, d + 1).await;
            if left
                .as_ref()
                .err()
                .is_some_and(|error| !guard_unreadable(error))
            {
                return left;
            }
            let right = guard1(a, b[h..].to_vec(), job, d + 1).await;
            return combine_guard_halves(left, right);
        }
        a.say(
            "guard",
            &format!(
                "unreadable answer for {}: {}",
                ids.join(", "),
                crate::sh::head(&txt, 160)
            ),
            job,
        );
        Err(X::E(GUARD_UNREADABLE.into()))
    })
}

async fn guard_batches(
    a: &Ax,
    xs: Vec<(String, String)>,
    from: &str,
    job: Option<i64>,
    partial: bool,
) -> R<Value> {
    let mut r = json!({});
    for c in xs.chunks(10) {
        a.flow(from, "guard", "check");
        let v = match guard1(a, c.to_vec(), job, 0).await {
            Ok(value) => value,
            Err(error) if partial && guard_unreadable(&error) => continue,
            Err(error) => return Err(error),
        };
        if !partial && c.iter().any(|(id, _)| !v[id]["inj"].is_boolean()) {
            return Err(X::E(GUARD_UNREADABLE.into()));
        }
        for (k, x) in v.as_object().cloned().unwrap_or_default() {
            r[k] = x;
        }
    }
    a.st("guard", "idle", None, "");
    Ok(r)
}

pub async fn guard(a: &Ax, xs: Vec<(String, String)>, from: &str, job: Option<i64>) -> R<Value> {
    guard_batches(a, xs, from, job, false).await
}

// CodeRabbit routing treats missing verdicts as unavailable, never as approval.
pub(crate) async fn guard_partial(
    a: &Ax,
    xs: Vec<(String, String)>,
    from: &str,
    job: Option<i64>,
) -> R<Value> {
    guard_batches(a, xs, from, job, true).await
}

#[cfg(test)]
#[test]
fn guard_partial_merge_preserves_only_valid_verdicts() {
    let valid = json!({"second":{"inj":false,"why":"safe"}});
    let merged =
        combine_guard_halves(Err(X::E(GUARD_UNREADABLE.into())), Ok(valid.clone())).unwrap();
    assert_eq!(merged, valid);
    assert!(merged["first"].is_null());
    assert!(combine_guard_halves(Ok(valid.clone()), Err(X::E("transport failed".into()))).is_err());
    assert!(combine_guard_halves(
        Err(X::E(GUARD_UNREADABLE.into())),
        Err(X::E(GUARD_UNREADABLE.into()))
    )
    .is_err());
}

pub fn errm(e: &X) -> String {
    match e {
        X::Lim(m) => format!("usage limit: {m}"),
        X::Net(m) => format!("network: {m}"),
        o => format!("error: {}", crate::sh::head(&o.to_string(), 200)),
    }
}

fn issue_priority(issue: &Value) -> i64 {
    issue["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|label| label["name"].as_str())
        .map(
            |label| match label.trim().to_ascii_lowercase().replace('_', "-").as_str() {
                "p0" | "priority:p0" | "priority:0" | "priority:critical" | "critical"
                | "blocker" => 0,
                "p1" | "priority:p1" | "priority:1" | "priority:high" | "high" => 1,
                "p3" | "priority:p3" | "priority:3" | "priority:low" | "low" => 3,
                _ => 2,
            },
        )
        .min()
        .unwrap_or(2)
}

fn issue_age_key(issue: &Value) -> i64 {
    issue["createdAt"]
        .as_str()
        .or_else(|| issue["updatedAt"].as_str())
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp_millis())
        .unwrap_or(i64::MAX)
}

fn issue_updated_at(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.timestamp_millis())
}

fn changed_since_drop(updated_at: &str, created_at: i64, data: &Value) -> bool {
    let Some(current) = issue_updated_at(updated_at) else {
        return false;
    };
    let previous = data["source_updated_at"]
        .as_str()
        .and_then(issue_updated_at)
        .unwrap_or(created_at);
    current > previous
}

// Recheck after remote calls; the caller holds update_transition when writing.
fn scout_repo_enabled(a: &Ax, repo: &str) -> bool {
    matches!(
        a.db.try_n(
            "select count(*) from rp where full=? and on_=1 and iss=1 and ci<>''",
            pa![repo.to_string()],
        ),
        Ok(1)
    )
}

async fn scout1(a: &Ax) -> R<()> {
    need(a, "scout", &["gh"]).await?;
    a.st("scout", "work", None, "");
    let me = crate::gh::login(a).await;
    let no = Regex::new(r"(?i)\b(question|wontfix|won't fix|duplicate|invalid|discussion|idea|roadmap|docs?|documentation)\b").unwrap();
    let ft = Regex::new(r"(?i)^\s*(\[?feat(ure)?\]?|\[?request\]?|idea|proposal)[:\] ]").unwrap();
    let nf = Regex::new(
        r"(?i)\b(question|wontfix|won't fix|duplicate|invalid|discussion|docs?|documentation)\b",
    )
    .unwrap();
    let hold = Regex::new(r"(?i)paused|blocked|on[- ]?hold|frozen|needs[- ]?(design|decision|discussion|triage|info)|\b(epic|tracking|meta|umbrella|rfc)\b").unwrap();
    let rs = a.db.all(
        "select full, feat, skl from rp where on_=1 and iss=1 and ci<>''",
        pa![],
    );
    for r in rs {
        let fe = r["feat"].as_i64().unwrap_or(0) == 1;
        let sk = skl(r["skl"].as_str().unwrap_or(""));
        let r = r["full"].as_str().unwrap_or("").to_string();
        let b = bl(a, &r);
        let cap = 4 * b;
        let iv = ivl(a, &r);
        let cooling_down = iv > 0
            && a.db
                .kg(&format!("sc:{r}"))
                .as_i64()
                .map(|t| now() - t < iv)
                .unwrap_or(false);
        let capacity = b > 0 && qn(a, &r, "'vet','ready'") < 4 * b;
        a.held("scout").await;
        a.st("scout", "work", None, "");
        let is = match crate::gh::open_issues(&r).await {
            Ok(items) => items,
            Err(error) => {
                let _transition = a.update_transition.lock().unwrap();
                if !scout_repo_enabled(a, &r) {
                    continue;
                }
                let previous = a.db.kg(&format!("iscan:{r}"));
                a.db.ks(&format!("iscan:{r}"), &json!({
                    "at":previous["at"],"count":previous["count"],"limit":previous["limit"],
                    "truncated":previous["truncated"],"error":crate::sh::head(&error,300),"error_at":now()
                }));
                a.say(
                    "scout",
                    &format!("{r}: não foi possível atualizar os issues do GitHub"),
                    None,
                );
                continue;
            }
        };
        if let Err(error) = a.db.replace_issue_snapshot(&r, &is, now()) {
            a.say(
                "scout",
                &format!("{r}: falha ao guardar snapshot de issues: {error}"),
                None,
            );
            continue;
        }
        if !capacity || cooling_down {
            continue;
        }
        a.db.ks(&format!("sc:{r}"), &json!(now()));
        let mut nw = vec![];
        for i in is {
            let n = i["number"].as_i64().unwrap_or(0);
            let up = i["updatedAt"].as_str().unwrap_or("").to_string();
            if i["assignees"]
                .as_array()
                .map(|x| x.iter().any(|y| y["login"].as_str() != Some(&me)))
                .unwrap_or(false)
            {
                let assignees = i["assignees"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x["login"].as_str())
                    .filter(|login| *login != me.as_str())
                    .map(|login| format!("@{login}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                a.db.run(
                    "update github_issues set skip_reason=? where repo=? and num=?",
                    pa![format!("Atribuída a {assignees}"), r.clone(), n],
                );
                continue;
            }
            if let Some(previous) = a.db.one(
                "select id,st,t0,d from jobs where repo=? and num=? and kind='fix' order by id desc limit 1",
                pa![r.clone(), n],
            ) {
                if previous["st"] != "drop" {
                    continue;
                }
                // Db::all already decodes the job's JSON column.
                let data = previous["d"].clone();
                if !changed_since_drop(&up, previous["t0"].as_i64().unwrap_or(0), &data) {
                    continue;
                }
            }
            if a.db
                .one(
                    "select upd from seen where repo=? and num=?",
                    pa![r.clone(), n],
                )
                .map(|x| x["upd"].as_str() == Some(&up))
                .unwrap_or(false)
            {
                continue;
            }
            let nx = if fe { &nf } else { &no };
            let matching_label = i["labels"].as_array().and_then(|l| {
                l.iter().find_map(|x| {
                    let label = x["name"].as_str().unwrap_or("");
                    (nx.is_match(label)
                        || hold.is_match(label)
                        || sk.iter().any(|k| k == &label.to_lowercase()))
                    .then(|| label.to_string())
                })
            });
            let filtered_title = !fe && ft.is_match(i["title"].as_str().unwrap_or(""));
            if matching_label.is_some() || filtered_title {
                let reason = matching_label
                    .map(|label| format!("Ignorado pela etiqueta ‘{label}’"))
                    .unwrap_or_else(|| {
                        "Ignorado pelo formato do título de pedido de funcionalidade".into()
                    });
                a.db.run(
                    "update github_issues set skip_reason=? where repo=? and num=?",
                    pa![reason.clone(), r.clone(), n],
                );
                a.db.run(
                    "insert into seen(repo,num,upd,why) values(?,?,?,?) on conflict do update set upd=excluded.upd,why=excluded.why",
                    pa![r.clone(), n, up, reason],
                );
                continue;
            }
            nw.push(i);
        }
        let mut nk = vec![];
        for i in nw {
            let n = i["number"].as_i64().unwrap_or(0);
            let related = match crate::gh::refs(&r, n).await {
                Ok(refs) => !crate::gh::live(refs).is_empty(),
                Err(error) => {
                    a.say(
                        "scout",
                        &format!(
                            "{r}#{n}: falha ao verificar PRs relacionadas: {}",
                            crate::sh::head(&error, 180)
                        ),
                        None,
                    );
                    continue;
                }
            };
            if related {
                let reason = "Já existe um pull request relacionado com este issue";
                a.db.run(
                    "insert into seen(repo,num,upd,why) values(?,?,?,?) on conflict do update set upd=excluded.upd,why=excluded.why",
                    pa![r.clone(), n, i["updatedAt"].as_str().unwrap_or("").to_string(), reason],
                );
                a.db.run(
                    "update github_issues set skip_reason=? where repo=? and num=?",
                    pa![reason, r.clone(), n],
                );
            } else {
                nk.push(i);
            }
        }
        let mut nw = nk;
        nw.sort_by_key(|issue| {
            (
                issue_priority(issue),
                issue_age_key(issue),
                issue["number"].as_i64().unwrap_or(i64::MAX),
            )
        });
        if nw.is_empty() {
            continue;
        }
        a.say("scout", &format!("{r}: {} new issues", nw.len()), None);
        for b in nw.chunks(15) {
            let room = (cap - qn(a, &r, "'vet','ready'")).max(0) as usize;
            if room == 0 {
                break;
            }
            let ids: Vec<String> = b.iter().map(|i| format!("{r}#{}", i["number"])).collect();
            let p = format!(
                "{}\n\nUse exactly these ids as JSON keys, in this order: {}",
                b.iter()
                    .map(|i| wrap(
                        &format!("{r}#{}", i["number"]),
                        &format!(
                            "#{} {}\n\n{}",
                            i["number"],
                            i["title"].as_str().unwrap_or(""),
                            crate::sh::head(i["body"].as_str().unwrap_or(""), 2500)
                        )
                    ))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                ids.join(", ")
            );
            let mut o = Ask::new(
                "scout",
                a.md("scout"),
                P(if fe { "scoutf" } else { "scout" }),
                p,
            );
            o.ef = Some("low");
            let (_, j0) = ask(a, o).await?;
            let Some(js) = by_id(&j0, &ids) else {
                return Err(X::E("Scout gave no readable verdict".into()));
            };
            let mut pk = vec![];
            for i in b {
                let id = format!("{r}#{}", i["number"]);
                if js[&id]["pick"].as_bool().unwrap_or(false) {
                    pk.push(i.clone());
                } else {
                    let reason = js[&id]["why"]
                        .as_str()
                        .filter(|why| !why.trim().is_empty())
                        .map(String::from)
                        .unwrap_or_else(|| {
                            "Scout decidiu não encaminhar este issue para correção.".into()
                        });
                    a.db.run(
                        "insert into seen(repo,num,upd,why) values(?,?,?,?) on conflict do update set upd=excluded.upd,why=excluded.why",
                        pa![r.clone(), i["number"].as_i64().unwrap_or(0), i["updatedAt"].as_str().unwrap_or("").to_string(), reason.clone()],
                    );
                    a.db.run(
                        "update github_issues set skip_reason=? where repo=? and num=?",
                        pa![reason, r.clone(), i["number"].as_i64().unwrap_or(0)],
                    );
                }
            }
            if pk.is_empty() {
                continue;
            }
            pk.truncate(room);
            let g = guard(
                a,
                pk.iter()
                    .map(|i| {
                        (
                            format!("{r}#{}", i["number"]),
                            format!(
                                "#{} {}\n\n{}",
                                i["number"],
                                i["title"].as_str().unwrap_or(""),
                                crate::sh::head(i["body"].as_str().unwrap_or(""), 5000)
                            ),
                        )
                    })
                    .collect(),
                "scout",
                None,
            )
            .await?;
            for i in pk {
                let _transition = a.update_transition.lock().unwrap();
                if !scout_repo_enabled(a, &r) {
                    break;
                }
                let n = i["number"].as_i64().unwrap_or(0);
                let id = format!("{r}#{n}");
                let ti = i["title"].as_str().unwrap_or("").to_string();
                let u = i["url"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or(format!("https://github.com/{r}/issues/{n}"));
                let mut data = json!({
                    "source_updated_at": i["updatedAt"],
                    "issue_priority": issue_priority(&i)
                });
                if g[&id]["inj"].as_bool().unwrap_or(false) {
                    a.db.jnew(
                        &r,
                        n,
                        "fix",
                        &ti,
                        &u,
                        "drop",
                        &format!("Guard: {}", g[&id]["why"].as_str().unwrap_or("")),
                        data,
                        None,
                        0.0,
                    );
                    a.say(
                        "guard",
                        &format!("{id} blocked: {}", g[&id]["why"].as_str().unwrap_or("")),
                        None,
                    );
                    continue;
                }
                let v = &js[&id];
                data["cx"] = json!(if v["cx"] == "complex" {
                    "complex"
                } else {
                    "simple"
                });
                data["feat"] = json!(fe && v["kind"] == "feat");
                a.db.jnew(
                    &r,
                    n,
                    "fix",
                    &ti,
                    &u,
                    "vet",
                    v["why"].as_str().unwrap_or(""),
                    data,
                    None,
                    0.0,
                );
                a.flow("scout", "vet", &id);
                a.say(
                    "scout",
                    &format!("{id} → Vet: {}", v["why"].as_str().unwrap_or("")),
                    None,
                );
            }
        }
    }
    a.st("scout", "idle", None, "");
    Ok(())
}

pub fn skl(s: &str) -> Vec<String> {
    s.split([',', '\n'])
        .map(|x| x.trim().to_lowercase())
        .filter(|x| !x.is_empty())
        .collect()
}

pub fn ivl(a: &Ax, r: &str) -> i64 {
    let t = now() - 14 * 86_400_000;
    let n = a.db.n(
        "select count(*) from jobs where repo=? and num>0 and kind='fix' and t0>?",
        pa![r.to_string(), t],
    );
    let k = a.db.n("select count(*) from jobs where repo=? and num>0 and kind='fix' and t0>? and st not in ('drop','vet')", pa![r.to_string(), t]);
    if n >= 30 && k == 0 {
        6 * 3_600_000
    } else if n >= 30 && k * 20 < n {
        3_600_000
    } else {
        0
    }
}

pub async fn scout(a: Ax) {
    loop {
        a.held("scout").await;
        if let Err(x) = scout1(&a).await {
            a.st("scout", "idle", None, "err");
            a.say("scout", &errm(&x), None);
        }
        tokio::select! {
            _ = sl(300_000) => {}
            _ = a.sw.notified() => {}
        }
    }
}

async fn thread(r: &str, n: i64) -> R<Value> {
    ghj(
        &[
            "issue",
            "view",
            &n.to_string(),
            "-R",
            r,
            "--json",
            "title,body,author,state,comments,assignees,labels",
        ],
        60,
    )
    .await
    .map_err(X::E)
}

fn vet_classification(d: &mut Value, opt: bool) {
    d["opt"] = json!(opt);
    if opt {
        d["feat"] = json!(false);
    }
}

async fn vet1(a: &Ax, id: i64) -> R<()> {
    let Some(j) = a.db.jg(id) else { return Ok(()) };
    let rl = rp(a, &j.repo)["rules"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let rl = if rl.is_empty() {
        String::new()
    } else {
        format!("\n\nOwner's rules for this repository (trusted):\n{rl}")
    };
    a.st("vet", "work", Some(id), "");
    if j.own() {
        if j.ext() {
            let g = guard(
                a,
                vec![(
                    "r".into(),
                    format!(
                        "from: {}\n{}\n\n{}",
                        j.d["ext"]["src"].as_str().unwrap_or(""),
                        j.s("title"),
                        j.s("body")
                    ),
                )],
                "vet",
                Some(id),
            )
            .await?;
            if g["r"]["inj"].as_bool().unwrap_or(false) {
                a.db.jset(
                    id,
                    &[
                        ("st", json!("drop")),
                        (
                            "why",
                            json!(format!("Guard: {}", g["r"]["why"].as_str().unwrap_or(""))),
                        ),
                    ],
                );
                a.say(
                    "guard",
                    &format!(
                        "{} blocked: {}",
                        j.tag(),
                        g["r"]["why"].as_str().unwrap_or("")
                    ),
                    Some(id),
                );
                return Ok(());
            }
        }
        let q = if j.ext() {
            format!("This is not a GitHub issue but a bug report sent to us directly from {}. Nobody has proved it yet. Judge only: is it plausibly a real bug in this repository's code, a focused fix (complex is fine, no rewrite or feature), worth a PR, and testable headless?", j.d["ext"]["src"].as_str().unwrap_or("an external tool"))
        } else {
            "This is not a GitHub issue but a bug our own testing found and proved with a failing test. Judge only: is it a real bug, a focused fix (complex is fine, no rewrite or feature), worth a PR, and testable headless?".to_string()
        };
        let mut o = Ask::new(
            "vet",
            a.md("vet"),
            P("vet"),
            format!(
                "Repository {}. {q}\n\n{}{rl}",
                j.repo,
                wrap("issue", &format!("{}\n\n{}", j.s("title"), j.s("body")))
            ),
        );
        o.job = Some(id);
        o.ef = Some("low");
        let (_, js) = ask(a, o).await?;
        if !js["ok"].as_bool().unwrap_or(false) {
            a.db.jset(
                id,
                &[
                    ("st", json!("drop")),
                    (
                        "why",
                        json!(format!(
                            "Vet: {}",
                            js["why"].as_str().unwrap_or("no answer")
                        )),
                    ),
                ],
            );
            return Ok(());
        }
        let (h, ot) = crate::gh::twins(a, &j.repo, &j.wh()).await.map_err(X::E)?;
        let of = a.db.try_all("select id, d from jobs where repo=? and num=0 and kind='fix' and id<>? and st not in ('drop','done') and (st<>'vet' or id<?) order by id desc", pa![j.repo.clone(), id, id])
            .map_err(|e| X::Lim(format!("Vet: local duplicate inventory unavailable: {e}")))?;
        if !h.is_empty() || !ot.is_empty() || !of.is_empty() {
            let bug = wrap("bug", &format!("{}\n\n{}", j.s("title"), j.s("body")));
            let mut candidates = Vec::new();
            for x in h {
                let context = wrap(
                    &format!("pr{}", x["number"]),
                    &format!(
                        "PR #{}: {}\nFiles: {}\n\n{}",
                        x["number"],
                        x["title"].as_str().unwrap_or(""),
                        x["files"]
                            .as_array()
                            .map(|f| f
                                .iter()
                                .filter_map(|y| y["path"].as_str())
                                .collect::<Vec<_>>()
                                .join(", "))
                            .unwrap_or_default(),
                        crate::sh::head(x["body"].as_str().unwrap_or(""), 3000)
                    ),
                );
                candidates.push(TwinCandidate {
                    context,
                    row: x,
                    pr: true,
                });
            }
            for x in ot {
                let context = wrap(
                    &format!("pr{}", x["number"]),
                    &format!(
                        "Other open PR #{}: {}\n{}",
                        x["number"],
                        x["title"].as_str().unwrap_or(""),
                        crate::sh::head(
                            &x["body"]
                                .as_str()
                                .unwrap_or("")
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" "),
                            400
                        )
                    ),
                );
                candidates.push(TwinCandidate {
                    context,
                    row: x,
                    pr: true,
                });
            }
            for x in of {
                let context = wrap(
                    &format!("find{}", x["id"]),
                    &format!(
                        "Office find {}: {}\nwhere: {}\n{}",
                        x["id"],
                        x["d"]["title"].as_str().unwrap_or(""),
                        x["d"]["hunt"]["where"]
                            .as_str()
                            .or(x["d"]["ext"]["where"].as_str())
                            .unwrap_or(""),
                        crate::sh::head(
                            &x["d"]["body"]
                                .as_str()
                                .unwrap_or("")
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" "),
                            600
                        )
                    ),
                );
                candidates.push(TwinCandidate {
                    context,
                    row: x,
                    pr: false,
                });
            }
            let batches = twin_batches(&bug, candidates, 64_000)?;
            for batch in batches {
                let mut prompt = bug.clone();
                let mut prs = Vec::new();
                let mut finds = Vec::new();
                for item in batch {
                    prompt.push_str("\n\n");
                    prompt.push_str(&item.context);
                    if item.pr {
                        prs.push(item.row);
                    } else {
                        finds.push(item.row);
                    }
                }
                let mut o = Ask::new("vet", a.md("vet"), P("twin"), prompt);
                o.job = Some(id);
                o.ef = Some("low");
                let (_, dj) = ask(a, o).await?;
                if !valid_twin_verdict(&dj, &prs, &[], &finds) {
                    return Err(X::E("Vet returned an invalid duplicate target".into()));
                }
                if dj["dup"] == true {
                    let w = if dj["find"].as_i64().unwrap_or(0) > 0 {
                        format!(
                            "same bug as office find #{} ({})",
                            dj["find"],
                            dj["why"].as_str().unwrap_or("")
                        )
                    } else {
                        format!(
                            "PR already exists: {}#{} ({})",
                            j.repo,
                            dj["pr"],
                            dj["why"].as_str().unwrap_or("")
                        )
                    };
                    a.db.jset(id, &[("st", json!("drop")), ("why", json!(w))]);
                    return Ok(());
                }
            }
        }
        let mut d = j.d.clone();
        d["hint"] = json!(js["hint"].as_str().unwrap_or(""));
        d["cx"] = json!(if js["cx"] == "complex" {
            "complex"
        } else {
            "simple"
        });
        let k = if j.ext() { "report" } else { "Hunter find" };
        a.db.jset(
            id,
            &[
                ("st", json!("ready")),
                (
                    "why",
                    json!(format!("{k}: {}", js["why"].as_str().unwrap_or(""))),
                ),
                ("d", d),
                ("prio", json!(-0.5)),
            ],
        );
        a.flow("vet", "repro", &j.repo);
        a.say(
            "vet",
            &format!("{} {} approved: {}", j.repo, k.to_lowercase(), j.s("title")),
            Some(id),
        );
        return Ok(());
    }
    a.say("vet", &format!("checking {}", j.tag()), Some(id));
    let i = thread(&j.repo, j.num).await?;
    if i["state"] != "OPEN" {
        a.db.jset(id, &[("st", json!("drop")), ("why", json!("issue closed"))]);
        return Ok(());
    }
    let me = crate::gh::login(a).await;
    let oa: Vec<String> = i["assignees"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x["login"].as_str().map(String::from))
        .filter(|x| *x != me)
        .collect();
    if !oa.is_empty() {
        a.db.jset(
            id,
            &[
                ("st", json!("drop")),
                ("why", json!(format!("assigned to {}", oa.join(", ")))),
            ],
        );
        return Ok(());
    }
    let lv = crate::gh::live(crate::gh::refs(&j.repo, j.num).await.map_err(X::E)?);
    if !lv.is_empty() {
        a.db.jset(
            id,
            &[
                ("st", json!("drop")),
                (
                    "why",
                    json!(format!(
                        "PR already exists: {}",
                        lv.iter()
                            .map(|p| format!("{}#{} ({})", p.rp, p.n, p.st))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                ),
            ],
        );
        return Ok(());
    }
    let cs: Vec<Value> = i["comments"].as_array().cloned().unwrap_or_default();
    let cs: Vec<Value> = cs[cs.len().saturating_sub(25)..].to_vec();
    let mut gs: Vec<(String, String)> = cs
        .iter()
        .enumerate()
        .map(|(k, c)| {
            (
                format!("c{k}"),
                c["body"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    if j.d["gi"].as_bool().unwrap_or(false) {
        gs.push((
            "issue".into(),
            format!(
                "#{} {}\n\n{}",
                j.num,
                i["title"].as_str().unwrap_or(""),
                crate::sh::head(i["body"].as_str().unwrap_or(""), 5000)
            ),
        ));
    }
    if !gs.is_empty() {
        let g = guard(a, gs, "vet", Some(id)).await?;
        if let Some((k, v)) = g
            .as_object()
            .and_then(|o| o.iter().find(|(_, v)| v["inj"].as_bool().unwrap_or(false)))
        {
            a.db.jset(
                id,
                &[
                    ("st", json!("drop")),
                    (
                        "why",
                        json!(format!(
                            "Guard: {} {}",
                            if k == "issue" { "issue" } else { "comment" },
                            v["why"].as_str().unwrap_or("")
                        )),
                    ),
                ],
            );
            return Ok(());
        }
    }
    let mut ps = vec![wrap(
        "issue",
        &format!(
            "#{} {}\nby {}\nlabels: {}\n\n{}",
            j.num,
            i["title"].as_str().unwrap_or(""),
            i["author"]["login"].as_str().unwrap_or(""),
            i["labels"]
                .as_array()
                .map(|l| l
                    .iter()
                    .filter_map(|x| x["name"].as_str())
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default(),
            crate::sh::head(i["body"].as_str().unwrap_or(""), 8000)
        ),
    )];
    for (k, c) in cs.iter().enumerate() {
        ps.push(wrap(
            &format!("c{k}"),
            &format!(
                "comment by {} ({})\n\n{}",
                c["author"]["login"].as_str().unwrap_or(""),
                c["authorAssociation"].as_str().unwrap_or(""),
                crate::sh::head(c["body"].as_str().unwrap_or(""), 3000)
            ),
        ));
    }
    let mut o = Ask::new(
        "vet",
        a.md("vet"),
        P(if j.feat() { "vetf" } else { "vet" }),
        format!("Repository {}.\n\n{}{rl}", j.repo, ps.join("\n\n")),
    );
    o.job = Some(id);
    o.ef = Some("low");
    let (_, js) = ask(a, o).await?;
    if !js["ok"].as_bool().unwrap_or(false) {
        a.db.jset(
            id,
            &[
                ("st", json!("drop")),
                (
                    "why",
                    json!(format!(
                        "Vet: {}",
                        js["why"].as_str().unwrap_or("no answer")
                    )),
                ),
            ],
        );
        return Ok(());
    }
    let mut d = j.d.clone();
    d["va"] = json!(now());
    // Keep optimizations distinct from feature requests: they need a plan, but
    // must preserve behavior and pass checks both before and after the change.
    let opt = js["opt"].as_bool().unwrap_or(false);
    vet_classification(&mut d, opt);
    d["cx"] = json!(
        if js["cx"] == "complex" || j.s("cx") == "complex" || j.feat() || opt {
            "complex"
        } else {
            "simple"
        }
    );
    d["hint"] = json!(js["hint"].as_str().unwrap_or(""));
    d["body"] = json!(crate::sh::head(i["body"].as_str().unwrap_or(""), 8000));
    d["title"] = json!(i["title"].as_str().unwrap_or(""));
    a.db.jset(
        id,
        &[
            ("st", json!("ready")),
            ("why", json!(js["why"].as_str().unwrap_or(""))),
            ("d", d),
            ("title", json!(i["title"].as_str().unwrap_or(&j.title))),
        ],
    );
    a.flow("vet", "repro", &j.tag());
    a.say(
        "vet",
        &format!("{} approved: {}", j.tag(), js["why"].as_str().unwrap_or("")),
        Some(id),
    );
    Ok(())
}

/// Performs Vet eligibility checks and reservation while the caller holds
/// the repository-update transition barrier.
pub(crate) fn admit_vet_locked(a: &Ax, id: i64) -> bool {
    if a.drain.load(std::sync::atomic::Ordering::SeqCst)
        || a.hold("vet")
        || !matches!(
            a.db.try_n(
                "select count(*) from jobs where id=? and st='vet' and exists(select 1 from rp where full=jobs.repo and on_=1)",
                pa![id]
            ),
            Ok(1)
        )
    {
        return false;
    }
    a.vtk.lock().unwrap().insert(id)
}

/// Admits Vet work under the same shutdown and repository-update barrier.
pub(crate) fn admit_vet(a: &Ax, id: i64) -> bool {
    let _transition = a.update_transition.lock().unwrap();
    admit_vet_locked(a, id)
}

async fn vetl(a: Ax) {
    loop {
        a.held("vet").await;
        let mut n = None;
        let mut fu = std::collections::HashMap::new();
        for x in a.db.all("select id, repo from jobs where st='vet' order by json_extract(d,'$.vt') is not null, t1", pa![]) {
            let id = x["id"].as_i64().unwrap_or(0);
            let r = x["repo"].as_str().unwrap_or("").to_string();
            if *fu.entry(r.clone()).or_insert_with(|| qn(&a, &r, "'ready'") >= 2 * bl(&a, &r)) {
                continue;
            }
            if admit_vet(&a, id) {
                n = Some(id);
                break;
            }
        }
        a.lv.lock().unwrap().o.vn = a.vtk.lock().unwrap().len();
        let Some(id) = n else {
            if a.vtk.lock().unwrap().is_empty() {
                a.st("vet", "idle", None, "");
            }
            sl(30_000).await;
            continue;
        };
        let r = async {
            need(&a, "vet", &["gh"]).await?;
            vet1(&a, id).await
        }
        .await;
        if let Some(j) = a.db.jg(id) {
            if j.st == "drop" {
                a.say("vet", &format!("{} dropped: {}", j.tag(), j.why), Some(id));
            }
        }
        if let Err(x) = r {
            a.say("vet", &errm(&x), Some(id));
            if !matches!(x, X::Lim(_) | X::Net(_)) {
                if let Some(j) = a.db.jg(id) {
                    let vt = j.d["vt"].as_i64().unwrap_or(0) + 1;
                    if vt >= 3 {
                        a.db.jset(
                            id,
                            &[
                                ("st", json!("drop")),
                                (
                                    "why",
                                    json!(format!(
                                        "Vet error: {}",
                                        crate::sh::head(&x.to_string(), 150)
                                    )),
                                ),
                            ],
                        );
                    } else {
                        let mut d = j.d.clone();
                        d["vt"] = json!(vt);
                        a.db.jset(id, &[("d", d)]);
                    }
                }
            }
            sl(30_000).await;
        }
        a.vtk.lock().unwrap().remove(&id);
        a.lv.lock().unwrap().o.vn = a.vtk.lock().unwrap().len();
    }
}

pub async fn vet(a: Ax) {
    for _ in 0..2 {
        tokio::spawn(vetl(a.clone()));
        sl(3000).await;
    }
}

const MA: &[&str] = &["OWNER", "MEMBER", "COLLABORATOR"];

pub fn pending(cs: Vec<Value>, last: &str) -> Vec<Value> {
    let Ok(l) = chrono::DateTime::parse_from_rfc3339(last) else {
        return cs;
    };
    cs.into_iter()
        .filter(|c| {
            c["at"]
                .as_str()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t > l)
                .unwrap_or(false)
        })
        .collect()
}

async fn comments(r: &str, n: i64, me: &str) -> R<Vec<Value>> {
    let bot = Regex::new(r"(?i)\[bot\]$|^github-actions$|^dependabot").unwrap();
    let (u1, u2, u3) = (
        format!("repos/{r}/issues/{n}/comments?per_page=100"),
        format!("repos/{r}/pulls/{n}/reviews?per_page=100"),
        format!("repos/{r}/pulls/{n}/comments?per_page=100"),
    );
    let (a, b, c) = tokio::join!(apip_unique(&u1), apip_unique(&u2), apip_unique(&u3));
    let mut o = vec![];
    for x in a.map_err(X::E)? {
        if !x["id"].as_i64().is_some_and(|id| id > 0) {
            return Err(X::E(
                "GitHub comment/review is missing a numeric identifier".into(),
            ));
        }
        o.push(json!({"id": format!("i{}", x["id"]), "by": x["user"]["login"], "as": x["author_association"], "t": x["body"].as_str().unwrap_or(""), "k": "comment", "at": x["created_at"], "edited_at": x["updated_at"], "actor_id": x["user"]["id"]}));
    }
    for x in b.map_err(X::E)? {
        if !x["id"].as_i64().is_some_and(|id| id > 0) {
            return Err(X::E(
                "GitHub comment/review is missing a numeric identifier".into(),
            ));
        }
        if x["body"].as_str().map(|s| !s.is_empty()).unwrap_or(false)
            || x["state"] == "CHANGES_REQUESTED"
        {
            o.push(json!({"id": format!("r{}", x["id"]), "rid": x["node_id"], "by": x["user"]["login"], "as": x["author_association"], "t": format!("[review: {}] {}", x["state"].as_str().unwrap_or(""), x["body"].as_str().unwrap_or("")), "k": "review", "at": x["submitted_at"], "commit": x["commit_id"], "actor_id": x["user"]["id"]}));
        }
    }
    for x in c.map_err(X::E)? {
        if !x["id"].as_i64().is_some_and(|id| id > 0) {
            return Err(X::E(
                "GitHub comment/review is missing a numeric identifier".into(),
            ));
        }
        let dh = x["diff_hunk"]
            .as_str()
            .map(|d| format!("```diff\n{}\n```\n", crate::sh::last(d, 800)))
            .unwrap_or_default();
        o.push(json!({"id": format!("c{}", x["id"]), "by": x["user"]["login"], "as": x["author_association"], "t": format!("[on {}:{}]\n{dh}{}", x["path"].as_str().unwrap_or(""), x["line"].as_i64().or(x["original_line"].as_i64()).map(|l| l.to_string()).unwrap_or("?".into()), x["body"].as_str().unwrap_or("")), "k": "line", "at": x["created_at"], "edited_at": x["updated_at"], "commit": x["commit_id"], "actor_id": x["user"]["id"]}));
    }
    Ok(o.into_iter()
        .filter(|x| {
            x["by"]
                .as_str()
                .map(|b| b != me && !bot.is_match(b))
                .unwrap_or(false)
        })
        .collect())
}

#[derive(Default)]
struct PusherScan {
    ids: std::collections::BTreeSet<String>,
    pushers: Vec<String>,
    cursors: std::collections::BTreeSet<String>,
    total: Option<u64>,
}
impl PusherScan {
    fn accept(&mut self, connection: &Value) -> Result<Option<String>, String> {
        let total = connection["totalCount"]
            .as_u64()
            .ok_or("missing review count")?;
        if total > 10_000 || self.total.is_some_and(|old| old != total) {
            return Err("review count changed or exceeded safe bounds".into());
        }
        self.total = Some(total);
        let nodes = connection["nodes"]
            .as_array()
            .ok_or("missing review nodes")?;
        for node in nodes {
            let id = node["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("missing review ID")?;
            if !self.ids.insert(id.to_string()) {
                return Err("repeated review ID".into());
            }
            if node["authorCanPushToRepository"]
                .as_bool()
                .ok_or("missing reviewer permission")?
            {
                self.pushers.push(id.to_string());
            }
        }
        match connection["pageInfo"]["hasNextPage"].as_bool() {
            Some(false) if self.ids.len() as u64 == total => Ok(None),
            Some(true) if !nodes.is_empty() && (self.ids.len() as u64) < total => {
                let cursor = connection["pageInfo"]["endCursor"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("missing review cursor")?;
                if !self.cursors.insert(cursor.to_string()) {
                    return Err("repeated review cursor".into());
                }
                Ok(Some(cursor.to_string()))
            }
            _ => Err("incomplete reviewer permission inventory".into()),
        }
    }
}

async fn pushers(r: &str, n: i64) -> Result<Vec<String>, String> {
    let (owner, name) = r.split_once('/').ok_or("invalid repository")?;
    let query = r#"query($owner:String!,$name:String!,$number:Int!,$cursor:String){repository(owner:$owner,name:$name){pullRequest(number:$number){reviews(first:100,after:$cursor,states:CHANGES_REQUESTED){totalCount nodes{id authorCanPushToRepository} pageInfo{hasNextPage endCursor}}}}}"#;
    let query_arg = format!("query={query}");
    let owner_arg = format!("owner={owner}");
    let name_arg = format!("name={name}");
    let number_arg = format!("number={n}");
    let mut scan = PusherScan::default();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = vec![
            "api",
            "graphql",
            "-f",
            &query_arg,
            "-F",
            &owner_arg,
            "-F",
            &name_arg,
            "-F",
            &number_arg,
        ];
        let cursor_arg = cursor.as_ref().map(|v| format!("cursor={v}"));
        if let Some(v) = cursor_arg.as_deref() {
            args.extend(["-f", v]);
        }
        let data = ghj(&args, 45).await?;
        if !data["errors"].is_null() {
            return Err("review permission query returned GraphQL errors".into());
        }
        match scan.accept(&data["data"]["repository"]["pullRequest"]["reviews"])? {
            Some(next) => cursor = Some(next),
            None => return Ok(scan.pushers),
        }
    }
}

async fn recheck_merged_evidence(
    repo: &str,
    number: i64,
    me: &str,
    original: &Value,
    observed: &[Value],
) -> R<()> {
    let current = comments(repo, number, me).await?;
    if !same_comment_inventory(observed, &current) {
        return Err(X::E(
            "Merged PR comments changed during analysis; comments remain pending".into(),
        ));
    }
    let fresh = ghj(
        &[
            "pr",
            "view",
            &number.to_string(),
            "-R",
            repo,
            "--json",
            "state,mergedAt,updatedAt,baseRefName",
        ],
        60,
    )
    .await
    .map_err(X::E)?;
    if !same_merged_snapshot(original, &fresh) {
        return Err(X::E(
            "Merged PR snapshot changed during analysis; comments remain pending".into(),
        ));
    }
    Ok(())
}

fn same_merged_snapshot(before: &Value, after: &Value) -> bool {
    before["state"] == "MERGED"
        && after["state"] == "MERGED"
        && ["mergedAt", "updatedAt", "baseRefName"].iter().all(|key| {
            before[*key].as_str().is_some_and(|value| !value.is_empty())
                && before[*key] == after[*key]
        })
}

async fn late(a: &Ax, r: &str, n: i64, me: &str, v: &Value) -> R<()> {
    let tag = format!("{r}#{n}");
    let updated = v["updatedAt"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| X::E("missing merged PR update timestamp".into()))?;
    let mut ids = Vec::new();
    let mut nw: Vec<Value> = vec![];
    let observed = comments(r, n, me).await?;
    for c in observed.iter().cloned() {
        let id = c["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| X::E("missing comment id".into()))?;
        if a.db
            .comment_acknowledged(r, id)
            .map_err(|e| X::E(e.to_string()))?
        {
            continue;
        }
        ids.push(id.to_string());
        if MA.contains(&c["as"].as_str().unwrap_or("")) {
            nw.push(c);
        }
    }
    let deferred = defer_oversized_comments(&mut nw);
    ids.retain(|id| !deferred.contains(id));
    let complete = deferred.is_empty();
    if !complete {
        a.say(
            "watcher",
            &format!(
                "{tag}: {} post-merge comment(s) require manual review",
                deferred.len()
            ),
            None,
        );
    }
    if nw.is_empty() {
        recheck_merged_evidence(r, n, me, v, &observed).await?;
        a.db.ack_merged_comments(r, n, updated, &ids, complete)
            .map_err(|e| X::E(e.to_string()))?;
        return Ok(());
    }
    let g = guard(
        a,
        nw.iter()
            .map(|c| {
                (
                    c["id"].as_str().unwrap_or("").to_string(),
                    c["t"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect(),
        "watcher",
        None,
    )
    .await?;
    let ok: Vec<&Value> = nw
        .iter()
        .filter(|c| {
            !g[c["id"].as_str().unwrap_or("")]["inj"]
                .as_bool()
                .unwrap_or(false)
        })
        .collect();
    if ok.is_empty() {
        recheck_merged_evidence(r, n, me, v, &observed).await?;
        a.db.ack_merged_comments(r, n, updated, &ids, complete)
            .map_err(|e| X::E(e.to_string()))?;
        return Ok(());
    }
    let cs = ok
        .iter()
        .map(|c| {
            wrap(
                c["id"].as_str().unwrap_or(""),
                &format!(
                    "{} by {} ({})\n\n{}",
                    c["k"].as_str().unwrap_or(""),
                    c["by"].as_str().unwrap_or(""),
                    c["as"].as_str().unwrap_or(""),
                    crate::sh::head(c["t"].as_str().unwrap_or(""), 4000)
                ),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let pr = format!("PR {tag}: {}\n\nThis PR is already merged. Set act only if a maintainer asks for a follow-up change or reports a problem the merged change caused; the change will become a new pull request from the default branch. A thank-you or merge notice is not a request.\n\n{cs}", v["title"].as_str().unwrap_or(""));
    let mut o = Ask::new("watcher", a.md("watcher"), P("watcher"), pr);
    o.ef = Some("low");
    let (_, js) = ask(a, o).await?;
    let by = ok
        .iter()
        .filter_map(|c| c["by"].as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let act = js["act"]
        .as_bool()
        .ok_or_else(|| X::E("Watcher gave no readable follow-up verdict".into()))?;
    if !act {
        recheck_merged_evidence(r, n, me, v, &observed).await?;
        a.db.ack_merged_comments(r, n, updated, &ids, complete)
            .map_err(|e| X::E(e.to_string()))?;
        return Ok(());
    }
    let ch = js["change"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| X::E("missing follow-up change".into()))?
        .to_string();
    let ti = format!("Follow-up to #{n}: {}", v["title"].as_str().unwrap_or(""));
    let body = format!("{ch}\n\nRequested after #{n} was merged ({}). Open a new pull request from the default branch that references #{n}.\n\nMaintainer comment:\n{}", v["url"].as_str().unwrap_or(""), crate::sh::head(&ok.iter().map(|c| format!("{}: {}", c["by"].as_str().unwrap_or(""), c["t"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n\n"), 6000));
    let request = json!({"title":ti,"body":body,"url":v["url"],"why":format!("follow-up requested by {by} after merge")});
    recheck_merged_evidence(r, n, me, v, &observed).await?;
    if !a
        .db
        .queue_followup_and_ack(r, n, updated, &ids, &request, complete)
        .map_err(|e| X::E(e.to_string()))?
    {
        a.say(
            "watcher",
            &format!("{tag}: follow-up remains pending until previous work completes"),
            None,
        );
        return Ok(());
    }
    a.say(
        "watcher",
        &format!("{tag}: {by} asked for a follow-up after merge, sent to Vet"),
        None,
    );
    a.note(
        "Follow-up after merge",
        &format!(
            "{tag} ({by}): {ch}\n\nDraft reply:\n{}\n{}",
            js["draft"].as_str().unwrap_or("-"),
            v["url"].as_str().unwrap_or("")
        ),
        4,
    );
    Ok(())
}

async fn repo_default_branch(
    a: &Ax,
    branches: &mut std::collections::HashMap<String, String>,
    repo: &str,
) -> String {
    if let Some(branch) = branches.get(repo) {
        return branch.clone();
    }
    let branch = crate::gh::meta(a, repo)
        .await
        .ok()
        .and_then(|meta| meta["main"].as_str().map(String::from))
        .unwrap_or_default();
    branches.insert(repo.to_string(), branch.clone());
    branch
}

fn checks_green(checks: &Value) -> bool {
    let Some(checks) = checks.as_array() else {
        return false;
    };
    !checks.is_empty()
        && checks.iter().all(|check| {
            if check["state"].is_string() {
                return check["state"] == "SUCCESS";
            }
            check["status"] == "COMPLETED"
                && matches!(
                    check["conclusion"].as_str(),
                    Some("SUCCESS" | "NEUTRAL" | "SKIPPED")
                )
        })
}

fn verified_bug_job(job: &Job) -> bool {
    job.st == "done"
        && job.kind == "fix"
        && !job.feat()
        && !job.opt()
        && job.d["proof"]["a_exit"]
            .as_i64()
            .map(|e| e != 0)
            .unwrap_or(false)
        && job.d["proof"]["b_exit"].as_i64() == Some(0)
        && !job.d["proof"]["command"]
            .as_str()
            .unwrap_or("")
            .trim()
            .is_empty()
        && job.d["f"]["tests"]
            .as_array()
            .map(|t| !t.is_empty())
            .unwrap_or(false)
        && job.d["ci_evidence"]["exit"].as_i64() == Some(0)
        && !job.d["ci_evidence"]["command"]
            .as_str()
            .unwrap_or("")
            .trim()
            .is_empty()
        && job.d["last_review"]["verdict"]["ok"] == true
}

fn auto_merge_blocker(pr: &Value, job: &Job, saved: &Value, me: &str) -> Option<&'static str> {
    if saved["auto_merge"].as_bool() != Some(true) {
        return Some("automatic merge is disabled for this pull request");
    }
    if !verified_bug_job(job) {
        return Some("the Apollo Office regression, CI, or independent review proof is incomplete");
    }
    if pr["state"] != "OPEN" || pr["mergedAt"].is_string() {
        return Some("the pull request is no longer open");
    }
    if pr["isDraft"] == true {
        return Some("the pull request is still a draft");
    }
    if pr["headRepositoryOwner"]["login"].as_str() != Some(me) {
        return Some("the pull request head is not owned by this GitHub account");
    }
    if pr["headRefName"].as_str() != saved["branch"].as_str() {
        return Some("the pull request branch changed after Apollo Office submitted it");
    }
    if pr["headRefOid"].as_str() != saved["head_sha"].as_str() {
        return Some("the pull request commit changed after Apollo Office submitted it");
    }
    if pr["baseRefName"].as_str() != saved["base_branch"].as_str() {
        return Some("the pull request base branch changed after Apollo Office submitted it");
    }
    if pr["reviewDecision"] == "CHANGES_REQUESTED" {
        return Some("a reviewer requested changes");
    }
    if pr["mergeable"] != "MERGEABLE" {
        return Some("GitHub has not confirmed that the pull request is mergeable");
    }
    if !checks_green(&pr["statusCheckRollup"]) {
        return Some("GitHub checks are missing, pending, or not all green");
    }
    None
}

// Watcher is an observer. Only the Steward may initiate new merges.
fn observe_auto_merge(
    a: &Ax,
    repo: &str,
    number: i64,
    pr: &Value,
    saved: &Value,
    me: &str,
) -> R<()> {
    if saved["auto_merge"] != true {
        return Ok(());
    }
    let head = pr["headRefOid"].as_str().unwrap_or("");
    let repo_enabled = a.db.n(
        "select auto_merge from rp where full=? and on_=1",
        pa![repo.to_string()],
    ) == 1;
    let (status, reason) = if !repo_enabled {
        (
            "blocked",
            "Repository automatic merge is disabled; no new merge requested.",
        )
    } else if saved["merge_enabled_head"].as_str() == Some(head) && !head.is_empty() {
        (
            "enabled",
            "GitHub auto-merge was previously enabled; monitoring existing request.",
        )
    } else if let Some(job) = saved["job"].as_i64().and_then(|id| a.db.jg(id)) {
        match auto_merge_blocker(pr, &job, saved, me) {
            Some(reason) => ("waiting", reason),
            None => (
                "waiting",
                "Local evidence observed; awaiting independent PR Steward gates.",
            ),
        }
    } else {
        ("blocked", "Apollo Office job evidence is unavailable.")
    };
    let changed = saved["merge_status"].as_str() != Some(status)
        || saved["merge_reason"].as_str() != Some(reason)
        || saved["merge_state_head"].as_str() != Some(head);
    if changed {
        // Update only diagnostic fields, never overwrite the comment cursor or
        // other metadata persisted by concurrent reconciliation.
        if a.db.run(
            "update prs set d=json_set(d,'$.merge_status',?,'$.merge_reason',?,'$.merge_state_head',?) where repo=? and num=?",
            pa![status.to_string(),reason.to_string(),head.to_string(),repo.to_string(),number],
        ) < 0 {
            return Err(X::E("Could not persist Watcher merge diagnosis".into()));
        }
        a.say("watcher", &format!("{repo}#{number}: {reason}"), None);
        if repo_enabled {
            a.steward_wake.notify_one();
        }
    }
    Ok(())
}

fn valid_pr_timestamp(v: &Value) -> R<String> {
    v["updatedAt"]
        .as_str()
        .filter(|s| issue_updated_at(s).is_some())
        .map(String::from)
        .ok_or_else(|| X::E("invalid PR update timestamp".into()))
}

fn valid_pr_base(v: &Value) -> R<String> {
    v["baseRefName"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .ok_or_else(|| X::E("missing PR base branch".into()))
}

fn pending_checked(cs: Vec<Value>, last: &str) -> R<Vec<Value>> {
    if issue_updated_at(last).is_none()
        || cs
            .iter()
            .any(|c| c["at"].as_str().and_then(issue_updated_at).is_none())
    {
        return Err(X::E(
            "comment or last commit timestamp unavailable; keep batch pending".into(),
        ));
    }
    Ok(pending(cs, last))
}

fn same_review_snapshot(original: &Value, fresh: &Value) -> bool {
    original["state"] == "OPEN"
        && fresh["state"] == "OPEN"
        && original["headRefOid"]
            .as_str()
            .is_some_and(crate::db::valid_git_sha)
        && original["updatedAt"]
            .as_str()
            .and_then(issue_updated_at)
            .is_some()
        && original["headRefName"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        && original["baseRefName"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        && original["headRepositoryOwner"]["login"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        && original["headRepository"]["name"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        && [
            "headRefOid",
            "updatedAt",
            "headRefName",
            "headRepositoryOwner",
            "headRepository",
            "baseRefName",
            "reviewDecision",
        ]
        .iter()
        .all(|key| original[*key] == fresh[*key])
}

async fn recheck_review_snapshot(repo: &str, number: i64, original: &Value) -> R<()> {
    let fresh=ghj(&["pr","view",&number.to_string(),"-R",repo,"--json","state,headRefOid,updatedAt,headRefName,headRepositoryOwner,headRepository,baseRefName,reviewDecision"],60).await.map_err(X::E)?;
    if !same_review_snapshot(original, &fresh) {
        return Err(X::E(
            "PR changed during review analysis; comments remain pending".into(),
        ));
    }
    Ok(())
}

// Compare complete normalized inventories, not just the PR's updatedAt.
// GitHub timestamps can have second-level precision and comment edits can be
// independent of the PR metadata. Duplicate identities are never evidence.
fn same_comment_inventory(before: &[Value], after: &[Value]) -> bool {
    fn index(rows: &[Value]) -> Option<std::collections::BTreeMap<&str, &Value>> {
        let mut result = std::collections::BTreeMap::new();
        for row in rows {
            let id = row["id"].as_str().filter(|id| !id.is_empty())?;
            if result.insert(id, row).is_some() {
                return None;
            }
        }
        Some(result)
    }
    match (index(before), index(after)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

async fn recheck_review_evidence(
    repo: &str,
    number: i64,
    me: &str,
    original: &Value,
    observed: &[Value],
    authorized_review_ids: &[String],
) -> R<()> {
    let current = comments(repo, number, me).await?;
    if !same_comment_inventory(observed, &current) {
        return Err(X::E(
            "PR comment inventory changed during analysis; receipts remain pending".into(),
        ));
    }
    if original["reviewDecision"] == "CHANGES_REQUESTED" {
        let current = pushers(repo, number).await.map_err(X::E)?;
        let before: std::collections::BTreeSet<_> = authorized_review_ids.iter().collect();
        let after: std::collections::BTreeSet<_> = current.iter().collect();
        if before != after {
            return Err(X::E(
                "Reviewer permissions changed during analysis; receipts remain pending".into(),
            ));
        }
    }
    recheck_review_snapshot(repo, number, original).await
}

// Guard's existing per-input limit is 3500 bytes. Never authorize a human
// comment based on only its prefix, or acknowledge its deferred receipt.
fn defer_oversized_comments(rows: &mut Vec<Value>) -> std::collections::BTreeSet<String> {
    let mut deferred = std::collections::BTreeSet::new();
    rows.retain(|row| {
        if row["t"].as_str().is_none_or(|text| text.len() > 3500) {
            if let Some(id) = row["id"].as_str() {
                deferred.insert(id.to_string());
            }
            false
        } else {
            true
        }
    });
    deferred
}

async fn watch_pr(
    a: &Ax,
    p: Value,
    me: &str,
    lu: &mut std::collections::HashMap<String, std::collections::HashMap<i64, String>>,
    default_branches: &mut std::collections::HashMap<String, String>,
) -> R<()> {
    let r = p["repo"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| X::E("tracked PR without repository".into()))?
        .to_string();
    let n = p["num"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or_else(|| X::E("tracked PR without number".into()))?;
    let default_branch = repo_default_branch(a, default_branches, &r).await;
    if p["st"] == "merged" {
        let wk = format!("wm:{r}#{n}");
        if a.db
            .kg(&wk)
            .as_i64()
            .map(|t| now() - t < 1_800_000)
            .unwrap_or(false)
        {
            return Ok(());
        }
        let v = ghj(
            &[
                "pr",
                "view",
                &n.to_string(),
                "-R",
                &r,
                "--json",
                "state,mergedAt,updatedAt,title,url,baseRefName",
            ],
            60,
        )
        .await
        .map_err(X::E)?;
        let up = valid_pr_timestamp(&v)?;
        let base = valid_pr_base(&v)?;
        if a.db.ch("update prs set upd=?, d=json_set(d,'$.base',?,'$.default_branch',?) where repo=? and num=? and st='merged' and upd is ?", pa![up.clone(), base, default_branch.clone(), r.clone(), n,p["upd"].as_str().map(String::from)]) != 1 {
                return Err(X::E("could not persist merged PR metadata".into()));
            }
        if p["d"]["comment_seen_at"].as_str() != Some(up.as_str()) {
            late(a, &r, n, me, &v).await?;
        }
        a.db.ks(&wk, &json!(now()));
        return Ok(());
    }
    let has_pending =
        a.db.has_pending_review(&r, n)
            .map_err(|e| X::E(e.to_string()))?;
    let ag = p["upd"]
        .as_str()
        .and_then(|u| chrono::DateTime::parse_from_rfc3339(u).ok())
        .map(|u| now() - u.timestamp_millis())
        .unwrap_or(0);
    let gap = if ag > 14 * 86_400_000 {
        6 * 3_600_000
    } else if ag > 7 * 86_400_000 {
        3_600_000
    } else {
        0
    };
    if gap > 0 && !has_pending {
        let sk = format!("ws:{r}#{n}");
        if !lu.contains_key(&r) {
            let m: std::collections::HashMap<i64, String> = ghj(
                &[
                    "pr",
                    "list",
                    "-R",
                    &r,
                    "--state",
                    "open",
                    "--author",
                    "@me",
                    "--limit",
                    "1000",
                    "--json",
                    "number,updatedAt",
                ],
                120,
            )
            .await
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|x| {
                (
                    x["number"].as_i64().unwrap_or(0),
                    x["updatedAt"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect();
            lu.insert(r.clone(), m);
        }
        let ch = lu
            .get(&r)
            .and_then(|m| m.get(&n))
            .map(|u| Some(u.as_str()) != p["upd"].as_str())
            .unwrap_or(true);
        if !ch
            && a.db
                .kg(&sk)
                .as_i64()
                .map(|t| now() - t < gap)
                .unwrap_or(false)
        {
            return Ok(());
        }
    }
    let fields = if p["d"]["auto_merge"].as_bool() == Some(true) {
        "state,mergedAt,updatedAt,title,url,mergeable,headRefName,headRepositoryOwner,headRepository,baseRefName,reviewDecision,isDraft,headRefOid,statusCheckRollup"
    } else {
        "state,mergedAt,updatedAt,title,url,mergeable,headRefName,headRepositoryOwner,headRepository,baseRefName,reviewDecision,headRefOid"
    };
    let v = match ghj(
        &["pr", "view", &n.to_string(), "-R", &r, "--json", fields],
        60,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            a.say(
                "watcher",
                &format!("{r}#{n}: {}", crate::sh::head(&e, 200)),
                None,
            );
            return Ok(());
        }
    };
    let up = valid_pr_timestamp(&v)?;
    let base = valid_pr_base(&v)?;
    let st = match v["state"].as_str() {
        Some("MERGED") if v["mergedAt"].as_str().and_then(issue_updated_at).is_some() => "merged",
        Some("OPEN") if v["mergedAt"].is_null() => "open",
        Some("CLOSED") if v["mergedAt"].is_null() => "closed",
        _ => return Err(X::E("invalid PR state or merge timestamp".into())),
    };
    let source_head = v["headRefOid"]
        .as_str()
        .filter(|s| crate::db::valid_git_sha(s))
        .unwrap_or("")
        .to_string();
    if st == "open" && source_head.is_empty() {
        return Err(X::E("open PR HEAD unavailable".into()));
    }
    if a.db.ch("update prs set st=?,title=?,upd=?,merged=?,d=json_set(d,'$.base',?,'$.default_branch',?,'$.watcher_head',?) where repo=? and num=? and st='open' and upd is ?",pa![st.to_string(),v["title"].as_str().unwrap_or("").to_string(),up.clone(),v["mergedAt"].as_str().map(String::from),base,default_branch.clone(),source_head.clone(),r.clone(),n,p["upd"].as_str().map(String::from)])!=1 {
            return Err(X::E("could not persist PR metadata".into()));
        }
    let tag = format!("{r}#{n}");
    if st == "merged" {
        a.note(
            "Merged",
            &format!(
                "{tag} {}\n{}",
                v["title"].as_str().unwrap_or(""),
                v["url"].as_str().unwrap_or("")
            ),
            3,
        );
        a.flow("watcher", "submit", "merged");
        a.say("watcher", &format!("{tag} merged"), None);
        a.db.ks(
            "lm",
            &json!({"repo": r, "num": n, "title": v["title"], "url": v["url"], "t": now()}),
        );
        late(a, &r, n, me, &v).await?;
        a.db.ks(&format!("wm:{tag}"), &json!(now()));
        return Ok(());
    }
    if st == "closed" {
        a.note(
            "Closed without merge",
            &format!(
                "{tag} {}\n{}",
                v["title"].as_str().unwrap_or(""),
                v["url"].as_str().unwrap_or("")
            ),
            4,
        );
        a.say("watcher", &format!("{tag} closed without merge"), None);
        return Ok(());
    }
    let hd = format!(
        "{}/{}",
        v["headRepositoryOwner"]["login"].as_str().unwrap_or(""),
        v["headRepository"]["name"].as_str().unwrap_or("")
    );
    let hrn = v["headRefName"].as_str().unwrap_or("").to_string();
    if v["mergeable"] == "CONFLICTING" && v["headRepositoryOwner"]["login"].as_str() == Some(me) {
        let request = json!({
            "title":v["title"],"url":v["url"],
            "why":format!("merge conflict with {}",v["baseRefName"].as_str().unwrap_or("")),
            "data":{"branch":hrn,"head":hd,"test_cmd":p["d"]["test_cmd"]}
        });
        if a.db
            .queue_watcher_rebase(&r, n, &up, &source_head, &request)
            .map_err(|e| X::E(e.to_string()))?
        {
            a.flow("watcher", "fixer", "rebase");
            a.say(
                "watcher",
                &format!("{tag} has a merge conflict, sent to rebase"),
                None,
            );
        }
    }
    observe_auto_merge(a, &r, n, &v, &p["d"], me)?;
    if !has_pending
        && p["d"]["comment_seen_at"].as_str() == Some(up.as_str())
        && p["d"]["watcher_head"] == source_head
        && p["d"]["seen"].as_bool().unwrap_or(false)
    {
        a.db.ks(&format!("ws:{tag}"), &json!(now()));
        return Ok(());
    }
    let first = !p["d"]["seen"].as_bool().unwrap_or(false);
    let cs = match comments(&r, n, me).await {
        Ok(x) => x,
        Err(e) => {
            a.say(
                "watcher",
                &format!("{tag}: {}", crate::sh::head(&errm(&e), 200)),
                None,
            );
            return Ok(());
        }
    };
    let cr: Vec<String> = if v["reviewDecision"] == "CHANGES_REQUESTED" {
        match pushers(&r, n).await {
            Ok(ids) => ids,
            Err(error) => {
                a.say(
                    "watcher",
                    &format!(
                        "{tag}: reviewer permissions unavailable: {}",
                        crate::sh::head(&error, 160)
                    ),
                    None,
                );
                return Ok(());
            }
        }
    } else {
        vec![]
    };
    let tr = |c: &Value| {
        MA.contains(&c["as"].as_str().unwrap_or(""))
            || (c["k"] == "review"
                && c["t"]
                    .as_str()
                    .map(|t| t.starts_with("[review: CHANGES_REQUESTED]"))
                    .unwrap_or(false)
                && c["rid"]
                    .as_str()
                    .map(|x| cr.iter().any(|y| y == x))
                    .unwrap_or(false))
    };
    let observed_comments = cs.clone();
    let mut nw = Vec::new();
    for c in cs {
        let id = c["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| X::E("PR comment has no identifier".into()))?;
        if !a
            .db
            .comment_acknowledged(&r, id)
            .map_err(|e| X::E(e.to_string()))?
        {
            nw.push(c);
        }
    }
    let received_ids: Vec<String> = nw
        .iter()
        .filter_map(|c| c["id"].as_str().map(String::from))
        .collect();
    if has_pending {
        recheck_review_evidence(&r, n, me, &v, &observed_comments, &cr).await?;
        let retired =
            a.db.reconcile_review_receipts(&r, n, &up, &source_head, &received_ids)
                .map_err(|e| X::E(e.to_string()))?;
        if retired > 0 {
            a.say("watcher",&format!("{tag}: {retired} receipt(s) no longer in observed pending inventory; no acknowledgement or action"),None);
        }
    }
    a.db.receive_review_batch(&r, n, &up, &source_head, &received_ids)
        .map_err(|e| X::E(e.to_string()))?;
    let pending = !received_ids.is_empty()
        || a.db
            .has_pending_review(&r, n)
            .map_err(|e| X::E(e.to_string()))?;
    if pending
        && !a
            .db
            .claim_review_attempt(&r, n, &up, &source_head, now())
            .map_err(|e| X::E(e.to_string()))?
    {
        return Ok(());
    }
    let mut batch_acked = false;
    let mut outbound = Value::Null;
    if first && !nw.is_empty() {
        let lc = ghj(
            &[
                "pr",
                "view",
                &n.to_string(),
                "-R",
                &r,
                "--json",
                "commits",
                "--jq",
                ".commits[-1].committedDate",
            ],
            60,
        )
        .await
        .map_err(X::E)?;
        nw = pending_checked(nw, lc.as_str().unwrap_or(""))?;
    }
    // Oversized comments remain in the durable inbox without acknowledgement.
    // Other comments can be handled independently; the existing retry
    // window stays active while any receipt remains pending.
    let deferred = defer_oversized_comments(&mut nw);
    let ack_ids: Vec<String> = received_ids
        .iter()
        .filter(|id| !deferred.contains(id.as_str()))
        .cloned()
        .collect();
    if !deferred.is_empty() {
        a.say(
            "watcher",
            &format!(
                "{tag}: {} comment(s) exceed complete Guard input limit; manual review required",
                deferred.len()
            ),
            None,
        );
        if nw.is_empty() && ack_ids.is_empty() {
            return Ok(());
        }
    }
    if !nw.is_empty() {
        a.say(
            "watcher",
            &format!("{tag}: {} new comment(s)", nw.len()),
            None,
        );
        let g = guard(
            a,
            nw.iter()
                .map(|c| {
                    (
                        c["id"].as_str().unwrap_or("").to_string(),
                        c["t"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect(),
            "watcher",
            None,
        )
        .await?;
        for c in nw.iter().filter(|c| {
            g[c["id"].as_str().unwrap_or("")]["inj"]
                .as_bool()
                .unwrap_or(false)
        }) {
            a.note(
                "Guard: suspicious comment",
                &format!(
                    "{tag} by {}: {}\n{}",
                    c["by"].as_str().unwrap_or(""),
                    g[c["id"].as_str().unwrap_or("")]["why"]
                        .as_str()
                        .unwrap_or(""),
                    v["url"].as_str().unwrap_or("")
                ),
                4,
            );
        }
        let okc: Vec<&Value> = nw
            .iter()
            .filter(|c| {
                !g[c["id"].as_str().unwrap_or("")]["inj"]
                    .as_bool()
                    .unwrap_or(false)
            })
            .collect();
        if !okc.is_empty() {
            let mc: Vec<&Value> = okc.iter().filter(|c| tr(c)).cloned().collect();
            let ma = !mc.is_empty();
            let src: &Vec<&Value> = if ma { &mc } else { &okc };
            let pv = ghj(
                &[
                    "pr",
                    "view",
                    &n.to_string(),
                    "-R",
                    &r,
                    "--json",
                    "body,files,headRefOid,updatedAt,changedFiles",
                ],
                60,
            )
            .await
            .map_err(X::E)?;
            if pv["headRefOid"] != source_head
                || pv["updatedAt"] != up
                || !pv["files"].as_array().is_some_and(|files| {
                    pv["changedFiles"].as_u64() == Some(files.len() as u64)
                        && files
                            .iter()
                            .all(|f| f["path"].as_str().is_some_and(|p| !p.is_empty()))
                })
            {
                return Err(X::E("PR file list is unavailable".into()));
            }
            let fl = pv["files"]
                .as_array()
                .map(|x| {
                    x.iter()
                        .filter_map(|f| f["path"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let pr = format!(
                "PR {tag}: {}\n\n{}\n\n{}\n\n{}",
                v["title"].as_str().unwrap_or(""),
                wrap(
                    "description",
                    &crate::sh::head(pv["body"].as_str().unwrap_or(""), 4000)
                ),
                wrap("files", &crate::sh::head(&fl, 2000)),
                src.iter()
                    .map(|c| wrap(
                        c["id"].as_str().unwrap_or(""),
                        &format!(
                            "{} by {} ({})\n\n{}",
                            c["k"].as_str().unwrap_or(""),
                            c["by"].as_str().unwrap_or(""),
                            c["as"].as_str().unwrap_or(""),
                            crate::sh::head(c["t"].as_str().unwrap_or(""), 4000)
                        )
                    ))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            );
            let mut o = Ask::new("watcher", a.md("watcher"), P("watcher"), pr);
            o.ef = Some("low");
            let (_, js) = ask(a, o).await?;
            if !js["act"].is_boolean() {
                return Err(X::E("Watcher gave no readable action verdict".into()));
            }
            let by = okc
                .iter()
                .filter_map(|c| c["by"].as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let act = js["act"].as_bool().unwrap_or(false);
            if ma {
                let reply = js["reply"].as_str().unwrap_or("").trim();
                if !reply.is_empty() {
                    outbound = json!({"author":me,"body":crate::sh::head(reply,1500)});
                }
            }
            if act && ma {
                let ch = js["change"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| X::E("missing requested change".into()))?
                    .to_string();
                let data = json!({"change": ch, "comment": crate::sh::head(&mc.iter().map(|c| format!("{}: {}", c["by"].as_str().unwrap_or(""), c["t"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n\n"), 6000), "branch": hrn, "head": hd, "test_cmd": p["d"]["test_cmd"],"source_head":source_head});
                recheck_review_evidence(&r, n, me, &v, &observed_comments, &cr).await?;
                let request = json!({"title":v["title"],"url":v["url"],"change":ch,"data":data,"reply":outbound});
                if !a
                    .db
                    .queue_review_and_ack(&r, n, &up, &ack_ids, &request)
                    .map_err(|e| X::E(e.to_string()))?
                {
                    a.say(
                        "watcher",
                        &format!("{tag}: new review deferred until active review finishes"),
                        None,
                    );
                    return Ok(());
                }
                batch_acked = true;
                a.flow("watcher", "fixer", "review");
                a.say(
                    "watcher",
                    &format!("{tag}: change requested by {by}, sent to Fixer"),
                    None,
                );
                a.note(
                    "Change requested",
                    &format!(
                        "{tag} ({by}): {ch}\n\nDraft reply:\n{}\n{}",
                        js["draft"].as_str().unwrap_or("-"),
                        v["url"].as_str().unwrap_or("")
                    ),
                    4,
                );
            } else {
                a.say(
                    "watcher",
                    &format!(
                        "{tag}: {by} commented{}",
                        if act && !ma {
                            " (not a maintainer, not applied)"
                        } else {
                            ""
                        }
                    ),
                    None,
                );
                a.note(
                    &format!("Comment on {tag}"),
                    &format!(
                        "{by}: {}\n\nDraft reply:\n{}\n{}",
                        crate::sh::head(
                            okc.last().and_then(|c| c["t"].as_str()).unwrap_or(""),
                            600
                        ),
                        js["draft"].as_str().unwrap_or("-"),
                        v["url"].as_str().unwrap_or("")
                    ),
                    3,
                );
            }
        }
    }
    // Advance the comment cursor only after successful analysis and queue persistence.
    if !batch_acked {
        recheck_review_evidence(&r, n, me, &v, &observed_comments, &cr).await?;
        a.db.ack_pr_comments_with_reply(&r, n, &up, &source_head, &ack_ids, &outbound)
            .map_err(|e| X::E(e.to_string()))?;
    }

    a.db.ks(&format!("ws:{tag}"), &json!(now()));
    Ok(())
}

pub(crate) fn reply_confirmed(rows: &[Value], author: &str, body: &str) -> bool {
    rows.iter().any(|row| {
        row["user"]["login"].as_str() == Some(author) && row["body"].as_str() == Some(body)
    })
}

// None means remote evidence is incomplete; defer without changing delivery state.
fn watcher_reply_remote_matches(pr: &Value, source_head: &str) -> Option<bool> {
    match pr["state"].as_str() {
        Some("MERGED" | "CLOSED") => Some(false),
        Some("OPEN") => {
            let head = pr["headRefOid"].as_str()?;
            if !crate::db::valid_git_sha(head) || !crate::db::valid_git_sha(source_head) {
                return None;
            }
            Some(head == source_head)
        }
        _ => None,
    }
}

async fn deliver_watcher_outbox(a: &Ax, me: &str) -> R<()> {
    // Pending sends first, then ambiguous sends. No uncertain row can be claimed again.
    let rows=a.db.try_all("select * from watcher_outbox where author=? and (state='pending' or (state='uncertain' and updated<=?)) order by updated,id limit 20",pa![me.to_string(),now()-600_000]).map_err(|e|X::E(e.to_string()))?;
    for row in rows {
        if a.hold("watcher") {
            break;
        }
        let (Some(id), Some(repo), Some(number), Some(author), Some(body)) = (
            row["id"].as_str(),
            row["repo"].as_str(),
            row["num"].as_i64(),
            row["author"].as_str(),
            row["body"].as_str(),
        ) else {
            continue;
        };
        let tag = format!("{repo}#{number}");
        if author != me {
            a.say(
                "watcher",
                &format!("{tag}: pending reply belongs to a different GitHub identity"),
                None,
            );
            continue;
        }
        if row["state"] == "uncertain" {
            // Positive confirmation only: absence is not evidence that a POST failed.
            if let Ok(comments) = apip_unique(&format!(
                "repos/{repo}/issues/{number}/comments?per_page=100"
            ))
            .await
            {
                if reply_confirmed(&comments, author, body) {
                    a.db.finish_watcher_reply(id, "sent")
                        .map_err(|e| X::E(e.to_string()))?;
                    a.say(
                        "watcher",
                        &format!("{tag}: previously uncertain reply confirmed"),
                        None,
                    );
                }
            }
            // Rotate ambiguous rows rather than starve later rows.
            a.db.run(
                "update watcher_outbox set updated=? where id=? and state='uncertain'",
                pa![now(), id.to_string()],
            );
            continue;
        }
        if now().saturating_sub(row["created"].as_i64().unwrap_or(0)) >= 86_400_000 {
            a.db.finish_watcher_reply(id, "cancelled")
                .map_err(|e| X::E(e.to_string()))?;
            a.say(
                "watcher",
                &format!("{tag}: unsent reply expired; manual review required"),
                None,
            );
            continue;
        }
        // Recheck the remote state before publishing previously queued content.
        a.db.run(
            "update watcher_outbox set updated=? where id=? and state='pending'",
            pa![now(), id.to_string()],
        );
        let Ok(pr) = ghj(
            &[
                "pr",
                "view",
                &number.to_string(),
                "-R",
                repo,
                "--json",
                "state,headRefOid",
            ],
            45,
        )
        .await
        else {
            continue;
        };
        match watcher_reply_remote_matches(&pr, row["source_head"].as_str().unwrap_or("")) {
            Some(true) => {}
            Some(false) => {
                if a.db
                    .finish_watcher_reply(id, "cancelled")
                    .map_err(|e| X::E(e.to_string()))?
                {
                    a.say(
                        "watcher",
                        &format!("{tag}: unsent reply cancelled because PR closed or HEAD changed"),
                        None,
                    );
                }
                continue;
            }
            None => continue,
        }
        let identity = ghj(&["api", "user"], 45).await.map_err(X::E)?;
        if identity["login"].as_str() != Some(author) {
            return Err(X::E("GitHub identity changed; delivery deferred".into()));
        }
        if a.hold("watcher") {
            break;
        }
        if !a
            .db
            .claim_watcher_reply(id, me)
            .map_err(|e| X::E(e.to_string()))?
        {
            continue;
        }
        match gh_input(
            &[
                "pr",
                "comment",
                &number.to_string(),
                "-R",
                repo,
                "--body-file",
                "-",
            ],
            60,
            Some(body.to_string()),
        )
        .await
        {
            Ok(_) => {
                a.db.finish_watcher_reply(id, "sent")
                    .map_err(|e| X::E(e.to_string()))?;
                a.db.ks(&format!("wr:{tag}"), &json!(now()));
                a.say("watcher", &format!("{tag}: queued reply delivered"), None);
            }
            Err(_) => a.say(
                "watcher",
                &format!("{tag}: reply delivery uncertain; automatic resend disabled"),
                None,
            ),
        }
    }
    Ok(())
}

async fn watch1(a: &Ax) -> R<()> {
    need(a, "watcher", &["gh"]).await?;
    a.st("watcher", "work", None, "");
    let me = crate::gh::login(a).await;
    if me.trim().is_empty() {
        return Err(X::E("GitHub identity unavailable".into()));
    }
    let mut lu = std::collections::HashMap::new();
    let mut default_branches = std::collections::HashMap::new();
    let rows=a.db.try_all("select * from prs where st='open' or (st='merged' and merged>strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 days')) order by repo,num",pa![]).map_err(|e|X::E(e.to_string()))?;
    for p in rows {
        a.held("watcher").await;
        let tag = format!("{}#{}", p["repo"].as_str().unwrap_or("?"), p["num"]);
        if let Err(error) = watch_pr(a, p, &me, &mut lu, &mut default_branches).await {
            a.say(
                "watcher",
                &format!(
                    "{tag}: processing deferred: {}",
                    crate::sh::head(&errm(&error), 200)
                ),
                None,
            );
        }
    }
    if let Err(error) = deliver_watcher_outbox(a, &me).await {
        a.say("watcher", &format!("outbox: {}", errm(&error)), None);
    }
    a.st("watcher", "idle", None, "");
    Ok(())
}

pub async fn watch(a: Ax) {
    loop {
        a.held("watcher").await;
        if let Err(x) = watch1(&a).await {
            a.st("watcher", "idle", None, "sad");
            a.say("watcher", &errm(&x), None);
        }
        sl(600_000).await;
    }
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn scout_rechecks_repository_registration_and_policy() {
        let db = crate::db::Db::mem();
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        assert!(!scout_repo_enabled(&a, "o/r"));
        a.db.run("insert into rp(full,ci) values('o/r','test')", pa![]);
        assert!(scout_repo_enabled(&a, "o/r"));
        a.db.run("update rp set iss=0 where full='o/r'", pa![]);
        assert!(!scout_repo_enabled(&a, "o/r"));
        a.db.run("delete from rp where full='o/r'", pa![]);
        assert!(!scout_repo_enabled(&a, "o/r"));
    }

    #[test]
    fn vet_keeps_optimizations_separate_from_features() {
        let mut optimization = json!({"feat":true});
        vet_classification(&mut optimization, true);
        assert_eq!(optimization["opt"], true);
        assert_eq!(optimization["feat"], false);

        let mut feature = json!({"feat":true,"opt":true});
        vet_classification(&mut feature, false);
        assert_eq!(feature["opt"], false);
        assert_eq!(feature["feat"], true);
    }

    #[test]
    fn skip_labels_and_machine_errors() {
        assert_eq!(
            skl(" Area:Kernel, needs-design\n\n"),
            vec!["area:kernel", "needs-design"]
        );
        assert!(crate::app::hostish("node-2: git clone → exit 1: Error response from daemon: Container apollo-office is paused, unpause the container before exec"));
        assert!(crate::app::hostish(
            "ssh: connect to host 10.0.0.2 port 22: Connection refused"
        ));
        assert!(!crate::app::hostish(
            "error[E0425]: cannot find value `x` in this scope"
        ));
    }

    #[test]
    fn pend() {
        let c = |at: &str| json!({"id": at, "at": at});
        let v = pending(
            vec![c("2026-10-18T10:00:00Z"), c("2026-10-19T10:00:00Z")],
            "2026-10-18T12:00:00Z",
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0]["at"], "2026-10-19T10:00:00Z");
        assert_eq!(pending(vec![c("2026-10-18T10:00:00Z")], "").len(), 1);
    }

    #[test]
    fn review_snapshot_requires_same_head_and_review_state() {
        let original = json!({"state":"OPEN","headRefOid":"a".repeat(40),"updatedAt":"2026-11-24T11:00:00Z","headRefName":"fix","headRepositoryOwner":{"login":"owner"},"headRepository":{"name":"repo"},"baseRefName":"main","reviewDecision":"CHANGES_REQUESTED"});
        assert!(same_review_snapshot(&original, &original));
        for (key, value) in [
            ("headRefOid", json!("b".repeat(40))),
            ("state", json!("CLOSED")),
            ("reviewDecision", json!("APPROVED")),
            ("updatedAt", json!("later")),
            ("baseRefName", json!("next")),
        ] {
            let mut changed = original.clone();
            changed[key] = value;
            assert!(!same_review_snapshot(&original, &changed));
        }
        let invalid = json!({"state":"OPEN","headRefOid":"invalid"});
        assert!(!same_review_snapshot(&invalid, &invalid));
    }

    #[test]
    fn uncertain_reply_requires_exact_body_and_author_confirmation() {
        let body = "reply\n\n<!-- apollo-office-reply:abc -->";
        assert!(reply_confirmed(
            &[json!({"user":{"login":"owner"},"body":body})],
            "owner",
            body
        ));
        assert!(!reply_confirmed(
            &[json!({"user":{"login":"other"},"body":body})],
            "owner",
            body
        ));
        assert!(!reply_confirmed(
            &[json!({"user":{"login":"owner"},"body":"different"})],
            "owner",
            body
        ));
        assert!(!reply_confirmed(&[], "owner", body));
    }

    #[test]
    fn pending_reply_requires_exact_remote_head_and_open_state() {
        let head = "a".repeat(40);
        let other = "b".repeat(40);
        assert_eq!(
            watcher_reply_remote_matches(&json!({"state":"OPEN","headRefOid":head}), &head),
            Some(true)
        );
        assert_eq!(
            watcher_reply_remote_matches(&json!({"state":"OPEN","headRefOid":other}), &head),
            Some(false)
        );
        for state in ["CLOSED", "MERGED"] {
            assert_eq!(
                watcher_reply_remote_matches(&json!({"state":state}), &head),
                Some(false)
            );
        }
        for incomplete in [
            json!({}),
            json!({"state":"OPEN"}),
            json!({"state":"OPEN","headRefOid":"invalid"}),
            json!({"state":"UNKNOWN","headRefOid":head}),
        ] {
            assert_eq!(watcher_reply_remote_matches(&incomplete, &head), None);
        }
        assert_eq!(
            watcher_reply_remote_matches(&json!({"state":"OPEN","headRefOid":head}), ""),
            None
        );
    }

    #[test]
    fn incomplete_pr_metadata_and_comment_timestamps_defer_processing() {
        assert!(valid_pr_timestamp(&json!({"updatedAt":"invalid"})).is_err());
        assert!(valid_pr_base(&json!({"baseRefName":""})).is_err());
        assert!(pending_checked(vec![json!({"at":"invalid"})], "2026-10-18T10:00:00Z").is_err());
        assert!(pending_checked(vec![json!({"at":"2026-10-19T10:00:00Z"})], "").is_err());
        assert_eq!(
            pending_checked(
                vec![json!({"at":"2026-10-19T10:00:00Z"})],
                "2026-10-18T10:00:00Z"
            )
            .unwrap()
            .len(),
            1
        );
    }

    #[test]
    fn checks_must_exist_and_all_be_green() {
        assert!(!checks_green(&json!([])));
        assert!(!checks_green(
            &json!([{"__typename":"CheckRun","status":"IN_PROGRESS"}])
        ));
        assert!(!checks_green(&json!([
            {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"},
            {"__typename":"StatusContext","state":"FAILURE"}
        ])));
        assert!(checks_green(&json!([
            {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"},
            {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SKIPPED"},
            {"__typename":"StatusContext","state":"SUCCESS"}
        ])));
    }

    #[test]
    fn auto_merge_requires_verified_bug_work_and_the_exact_green_head() {
        let job = Job::of(json!({
            "id": 7, "repo": "owner/repo", "num": 12, "kind": "fix", "st": "done",
            "d": {
                "feat": false,
                "proof": {"a_exit": 1, "b_exit": 0, "command": "node test.js"},
                "f": {"tests": ["regression"], "test_cmd": "node test.js"},
                "ci_evidence": {"exit": 0, "command": "npm test"},
                "last_review": {"verdict": {"ok": true}}
            }
        }));
        let saved = json!({"auto_merge": true, "job": 7, "branch": "fix/7-bug", "head_sha": "abc123", "base_branch": "main"});
        let pr = json!({
            "state": "OPEN", "mergedAt": null, "isDraft": false, "mergeable": "MERGEABLE",
            "headRepositoryOwner": {"login": "owner"}, "headRefName": "fix/7-bug", "headRefOid": "abc123",
            "baseRefName": "main", "reviewDecision": "REVIEW_REQUIRED",
            "statusCheckRollup": [{"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"}]
        });
        assert_eq!(auto_merge_blocker(&pr, &job, &saved, "owner"), None);

        let mut pending = pr.clone();
        pending["statusCheckRollup"] = json!([]);
        assert!(auto_merge_blocker(&pending, &job, &saved, "owner").is_some());

        let mut changed_head = pr.clone();
        changed_head["headRefOid"] = json!("new-sha");
        assert!(auto_merge_blocker(&changed_head, &job, &saved, "owner").is_some());

        let mut draft = pr;
        draft["isDraft"] = json!(true);
        assert!(auto_merge_blocker(&draft, &job, &saved, "owner").is_some());

        let mut changes_requested = draft;
        changes_requested["isDraft"] = json!(false);
        changes_requested["reviewDecision"] = json!("CHANGES_REQUESTED");
        assert!(auto_merge_blocker(&changes_requested, &job, &saved, "owner").is_some());

        let mut feature = job;
        feature.d["feat"] = json!(true);
        assert!(!verified_bug_job(&feature));
    }

    #[test]
    fn repositories_default_auto_merge_on_for_new_bug_fixes() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into rp(full) values(?)",
            pa!["owner/repo".to_string()],
        );
        assert_eq!(
            db.one(
                "select merge from rp where full=?",
                pa!["owner/repo".to_string()]
            )
            .unwrap()["merge"],
            1
        );
    }
}

#[cfg(test)]
mod pusher_pagination_tests {
    use super::*;
    #[test]
    fn reviewer_permissions_require_complete_pages() {
        let first = json!({"totalCount":2,"nodes":[{"id":"r1","authorCanPushToRepository":true}],"pageInfo":{"hasNextPage":true,"endCursor":"next"}});
        let last = json!({"totalCount":2,"nodes":[{"id":"r2","authorCanPushToRepository":false}],"pageInfo":{"hasNextPage":false}});
        let mut scan = PusherScan::default();
        assert_eq!(scan.accept(&first).unwrap().as_deref(), Some("next"));
        assert!(scan.accept(&last).unwrap().is_none());
        assert_eq!(scan.pushers, vec!["r1"]);
        let mut scan = PusherScan::default();
        scan.accept(&first).unwrap();
        assert!(scan.accept(&first).is_err());
        let mut scan = PusherScan::default();
        assert!(scan.accept(&last).is_err());
    }
}

#[cfg(test)]
mod watcher_evidence_inventory_tests {
    use super::*;

    #[test]
    fn merged_snapshot_requires_same_state_merge_time_and_update() {
        let original =
            json!({"state":"MERGED","mergedAt":"m","updatedAt":"u","baseRefName":"main"});
        assert!(same_merged_snapshot(&original, &original));
        for (key, value) in [
            ("state", "OPEN"),
            ("mergedAt", "other"),
            ("updatedAt", "new"),
            ("baseRefName", "dev"),
        ] {
            let mut changed = original.clone();
            changed[key] = json!(value);
            assert!(!same_merged_snapshot(&original, &changed));
        }
        assert!(!same_merged_snapshot(&json!({}), &json!({})));
    }

    #[test]
    fn oversized_receipts_are_deferred_without_blocking_other_comments() {
        let mut rows = vec![
            json!({"id":"r1","t":"a".repeat(3500)}),
            json!({"id":"r2","t":"a".repeat(3501)}),
            json!({"id":"r3","t":"é".repeat(1751)}),
            json!({"id":"r4","t":"Valid request"}),
        ];
        let deferred = defer_oversized_comments(&mut rows);
        assert_eq!(
            deferred.into_iter().collect::<Vec<_>>(),
            vec!["r2".to_string(), "r3".to_string()]
        );
        assert_eq!(
            rows.iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["r1", "r4"]
        );
    }

    #[test]
    fn inventory_rejects_edits_removals_duplicates_and_permission_changes() {
        let a = json!({"id":"r1","t":"Original request","as":"MEMBER","actor_id":42});
        let b = json!({"id":"c2","t":"Another comment","as":"OWNER","actor_id":43});
        let original = vec![a.clone(), b.clone()];
        assert!(same_comment_inventory(&original, &[b.clone(), a.clone()]));
        assert!(!same_comment_inventory(&original, std::slice::from_ref(&a)));
        assert!(!same_comment_inventory(&original, &[a.clone(), a.clone()]));
        let mut edited = a.clone();
        edited["t"] = json!("Edited request");
        assert!(!same_comment_inventory(&original, &[edited, b.clone()]));
        let mut changed = a.clone();
        changed["as"] = json!("NONE");
        assert!(!same_comment_inventory(&original, &[changed, b.clone()]));
        let mut actor = a.clone();
        actor["actor_id"] = json!(99);
        assert!(!same_comment_inventory(&original, &[actor, b]));
        assert!(!same_comment_inventory(&[json!({})], &[json!({})]));
    }
}

#[cfg(test)]
mod twin_verdict_tests {
    use super::*;

    #[test]
    fn duplicate_batches_preserve_all_candidates_and_reject_oversize() {
        let candidate = |n| TwinCandidate {
            context: "12345678".into(),
            row: json!({"id":n}),
            pr: false,
        };
        let batches = twin_batches("bug", (1..=101).map(candidate).collect(), 23).unwrap();
        assert_eq!(batches.len(), 51);
        assert_eq!(
            batches
                .iter()
                .flatten()
                .map(|x| x.row["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            (1..=101).collect::<Vec<_>>()
        );
        assert!(twin_batches("bug", vec![candidate(1)], 12).is_err());
    }

    #[test]
    fn duplicate_verdict_requires_an_observed_target() {
        let prs = [json!({"number":12})];
        let finds = [json!({"id":4})];
        let verdict = |dup, pr, find| json!({"dup":dup,"pr":pr,"find":find,"why":"same cause"});
        assert!(valid_twin_verdict(&verdict(true, 12, 0), &prs, &[], &finds));
        assert!(valid_twin_verdict(&verdict(true, 0, 4), &prs, &[], &finds));
        assert!(valid_twin_verdict(&verdict(false, 0, 0), &prs, &[], &finds));
        assert!(!valid_twin_verdict(
            &verdict(true, 99, 0),
            &prs,
            &[],
            &finds
        ));
        assert!(!valid_twin_verdict(
            &verdict(true, 12, 4),
            &prs,
            &[],
            &finds
        ));
        assert!(!valid_twin_verdict(&verdict(true, 0, 0), &prs, &[], &finds));
        assert!(!valid_twin_verdict(&json!({}), &prs, &[], &finds));
    }
}
