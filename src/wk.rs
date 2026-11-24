use crate::ai::{ask, by_id, p as P, wrap, Ask};
use crate::app::{sl, Ax, R, X};
use crate::db::{now, Job};
use crate::net::need;
use crate::pa;
use crate::pipe::{bl, qn, rp};
use crate::sh::{apip_unique, gh, ghj};
use regex::Regex;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;

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
            let mut l = guard1(a, b[..h].to_vec(), job, d + 1).await?;
            let r = guard1(a, b[h..].to_vec(), job, d + 1).await?;
            for (k, v) in r.as_object().cloned().unwrap_or_default() {
                l[k] = v;
            }
            return Ok(l);
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
        Err(X::E("Guard gave no readable verdict".into()))
    })
}

pub async fn guard(a: &Ax, xs: Vec<(String, String)>, from: &str, job: Option<i64>) -> R<Value> {
    let mut r = json!({});
    for c in xs.chunks(10) {
        a.flow(from, "guard", "check");
        let v = guard1(a, c.to_vec(), job, 0).await?;
        for (k, x) in v.as_object().cloned().unwrap_or_default() {
            r[k] = x;
        }
    }
    a.st("guard", "idle", None, "");
    Ok(r)
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
        let of: Vec<Value> = a.db.all("select id, d from jobs where repo=? and num=0 and kind='fix' and id<>? and st not in ('drop','done') and (st<>'vet' or id<?) order by id desc limit 50", pa![j.repo.clone(), id, id]);
        if !h.is_empty() || !ot.is_empty() || !of.is_empty() {
            let mut ps = vec![wrap("bug", &format!("{}\n\n{}", j.s("title"), j.s("body")))];
            for x in &h {
                ps.push(wrap(
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
                ));
            }
            ps.push(wrap(
                "other",
                &format!(
                    "Other open PRs (title and start of body):\n\n{}",
                    ot.iter()
                        .map(|x| format!(
                            "#{} {}\n{}",
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
                        ))
                        .collect::<Vec<_>>()
                        .join("\n\n")
                ),
            ));
            if !of.is_empty() {
                ps.push(wrap(
                    "ours",
                    &format!(
                        "Our own earlier finds, fix in progress, no PR yet:\n\n{}",
                        of.iter()
                            .map(|x| format!(
                                "find {}: {}\nwhere: {}\n{}",
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
                            ))
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    ),
                ));
            }
            let mut o = Ask::new(
                "vet",
                a.md("vet"),
                P("twin"),
                crate::sh::head(&ps.join("\n\n"), 250_000),
            );
            o.job = Some(id);
            o.ef = Some("low");
            let (_, dj) = ask(a, o).await?;
            if dj["dup"].as_bool().unwrap_or(false) {
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

/// Admits Vet work under the same shutdown barrier as other workers.
pub(crate) fn admit_vet(a: &Ax, id: i64) -> bool {
    let _transition = a.update_transition.lock().unwrap();
    if a.drain.load(std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    a.vtk.lock().unwrap().insert(id)
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
        o.push(json!({"id": format!("i{}", x["id"]), "by": x["user"]["login"], "as": x["author_association"], "t": x["body"].as_str().unwrap_or(""), "k": "comment", "at": x["created_at"]}));
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
            o.push(json!({"id": format!("r{}", x["id"]), "rid": x["node_id"], "by": x["user"]["login"], "as": x["author_association"], "t": format!("[review: {}] {}", x["state"].as_str().unwrap_or(""), x["body"].as_str().unwrap_or("")), "k": "review", "at": x["submitted_at"]}));
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
        o.push(json!({"id": format!("c{}", x["id"]), "by": x["user"]["login"], "as": x["author_association"], "t": format!("[on {}:{}]\n{dh}{}", x["path"].as_str().unwrap_or(""), x["line"].as_i64().or(x["original_line"].as_i64()).map(|l| l.to_string()).unwrap_or("?".into()), x["body"].as_str().unwrap_or("")), "k": "line", "at": x["created_at"]}));
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

async fn late(a: &Ax, r: &str, n: i64, me: &str, v: &Value) -> R<()> {
    let tag = format!("{r}#{n}");
    let updated = v["updatedAt"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| X::E("missing merged PR update timestamp".into()))?;
    let mut ids = Vec::new();
    let mut nw: Vec<Value> = vec![];
    for c in comments(r, n, me).await? {
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
    if nw.is_empty() {
        a.db.ack_pr_comments(r, n, updated, &ids)
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
        a.db.ack_pr_comments(r, n, updated, &ids)
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
        a.db.ack_pr_comments(r, n, updated, &ids)
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
    if !a
        .db
        .queue_followup_and_ack(r, n, updated, &ids, &request)
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

fn valid_pr_timestamp(v:&Value)->R<String>{
    v["updatedAt"].as_str().filter(|s|issue_updated_at(s).is_some()).map(String::from)
        .ok_or_else(||X::E("invalid PR update timestamp".into()))
}

fn valid_pr_base(v:&Value)->R<String>{
    v["baseRefName"].as_str().filter(|s|!s.trim().is_empty()).map(String::from)
        .ok_or_else(||X::E("missing PR base branch".into()))
}

fn pending_checked(cs:Vec<Value>,last:&str)->R<Vec<Value>>{
    if issue_updated_at(last).is_none() || cs.iter().any(|c|c["at"].as_str().and_then(issue_updated_at).is_none()){
        return Err(X::E("comment or last commit timestamp unavailable; keep batch pending".into()));
    }
    Ok(pending(cs,last))
}

async fn watch_pr(a:&Ax,p:Value,me:&str,
    lu:&mut std::collections::HashMap<String,std::collections::HashMap<i64,String>>,
    default_branches:&mut std::collections::HashMap<String,String>)->R<()>{
    let r=p["repo"].as_str().filter(|s|!s.trim().is_empty()).ok_or_else(||X::E("tracked PR without repository".into()))?.to_string();
    let n=p["num"].as_i64().filter(|n|*n>0).ok_or_else(||X::E("tracked PR without number".into()))?;
        let default_branch = repo_default_branch(a, default_branches, &r).await;
        if p["st"] == "merged" {
            let wk = format!("wm:{r}#{n}");
            if a.db.kg(&wk).as_i64().map(|t| now() - t < 1_800_000).unwrap_or(false) {
                return Ok(());
            }
            let v = ghj(&["pr", "view", &n.to_string(), "-R", &r, "--json", "updatedAt,title,url,baseRefName"], 60).await.map_err(X::E)?;
            let up = valid_pr_timestamp(&v)?;
            let base = valid_pr_base(&v)?;
            if a.db.run("update prs set upd=?, d=json_set(d,'$.base',?,'$.default_branch',?) where repo=? and num=? and st='merged'", pa![up.clone(), base, default_branch.clone(), r.clone(), n]) < 0 {
                return Err(X::E("could not persist merged PR metadata".into()));
            }
            if p["d"]["comment_seen_at"].as_str() != Some(up.as_str()) {
                late(a,&r,n,me,&v).await?;
            }
            a.db.ks(&wk,&json!(now()));
            return Ok(());
        }
        let ag = p["upd"].as_str().and_then(|u| chrono::DateTime::parse_from_rfc3339(u).ok()).map(|u| now() - u.timestamp_millis()).unwrap_or(0);
        let gap = if ag > 14 * 86_400_000 { 6 * 3_600_000 } else if ag > 7 * 86_400_000 { 3_600_000 } else { 0 };
        if gap > 0 {
            let sk = format!("ws:{r}#{n}");
            if !lu.contains_key(&r) {
                let m: std::collections::HashMap<i64, String> = ghj(&["pr", "list", "-R", &r, "--state", "open", "--author", "@me", "--limit", "1000", "--json", "number,updatedAt"], 120).await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default().iter().map(|x| (x["number"].as_i64().unwrap_or(0), x["updatedAt"].as_str().unwrap_or("").to_string())).collect();
                lu.insert(r.clone(), m);
            }
            let ch = lu.get(&r).and_then(|m| m.get(&n)).map(|u| Some(u.as_str()) != p["upd"].as_str()).unwrap_or(false);
            if !ch && a.db.kg(&sk).as_i64().map(|t| now() - t < gap).unwrap_or(false) {
                return Ok(());
            }
            a.db.ks(&sk, &json!(now()));
        }
        let fields = if p["d"]["auto_merge"].as_bool() == Some(true) {
            "state,mergedAt,updatedAt,title,url,mergeable,headRefName,headRepositoryOwner,headRepository,baseRefName,reviewDecision,isDraft,headRefOid,statusCheckRollup"
        } else {
            "state,mergedAt,updatedAt,title,url,mergeable,headRefName,headRepositoryOwner,headRepository,baseRefName,reviewDecision"
        };
        let v = match ghj(&["pr", "view", &n.to_string(), "-R", &r, "--json", fields], 60).await {
            Ok(v) => v,
            Err(e) => {
                a.say("watcher", &format!("{r}#{n}: {}", crate::sh::head(&e, 200)), None);
                return Ok(());
            }
        };
        let up=valid_pr_timestamp(&v)?;
        let base=valid_pr_base(&v)?;
        let st=match v["state"].as_str() {
            Some("MERGED") if v["mergedAt"].as_str().and_then(issue_updated_at).is_some()=> "merged",
            Some("OPEN") if v["mergedAt"].is_null()=> "open",
            Some("CLOSED") if v["mergedAt"].is_null()=> "closed",
            _=>return Err(X::E("invalid PR state or merge timestamp".into())),
        };
        if a.db.run("update prs set st=?,title=?,upd=?,merged=?,d=json_set(d,'$.base',?,'$.default_branch',?) where repo=? and num=? and st='open'",pa![st.to_string(),v["title"].as_str().unwrap_or("").to_string(),up.clone(),v["mergedAt"].as_str().map(String::from),base,default_branch.clone(),r.clone(),n])<0 {
            return Err(X::E("could not persist PR metadata".into()));
        }
        let tag = format!("{r}#{n}");
        if st == "merged" {
            a.note("Merged", &format!("{tag} {}\n{}", v["title"].as_str().unwrap_or(""), v["url"].as_str().unwrap_or("")), 3);
            a.flow("watcher", "submit", "merged");
            a.say("watcher", &format!("{tag} merged"), None);
            a.db.ks("lm", &json!({"repo": r, "num": n, "title": v["title"], "url": v["url"], "t": now()}));
            a.db.ks(&format!("wm:{tag}"), &json!(now()));
            if let Err(error)=late(a,&r,n,me,&v).await {
                a.say("watcher",&format!("{tag}: follow-up pending: {}",errm(&error)),None);
            }
            return Ok(());
        }
        if st == "closed" {
            a.note("Closed without merge", &format!("{tag} {}\n{}", v["title"].as_str().unwrap_or(""), v["url"].as_str().unwrap_or("")), 4);
            a.say("watcher", &format!("{tag} closed without merge"), None);
            return Ok(());
        }
        let hd = format!("{}/{}", v["headRepositoryOwner"]["login"].as_str().unwrap_or(""), v["headRepository"]["name"].as_str().unwrap_or(""));
        let hrn = v["headRefName"].as_str().unwrap_or("").to_string();
        if v["mergeable"] == "CONFLICTING" && v["headRepositoryOwner"]["login"].as_str() == Some(me) && a.db.n("select count(*) from jobs where repo=? and pr=? and kind='rebase' and (st not in ('done','drop') or t1>?)", pa![r.clone(), n, now() - 86_400_000]) == 0 {
            a.db.jnew(&r, n, "rebase", v["title"].as_str().unwrap_or(""), v["url"].as_str().unwrap_or(""), "ready", &format!("merge conflict with {}", v["baseRefName"].as_str().unwrap_or("")), json!({"branch": hrn, "head": hd, "test_cmd": p["d"]["test_cmd"]}), Some(n), -1.0);
            a.flow("watcher", "fixer", "rebase");
            a.say("watcher", &format!("{tag} has a merge conflict, sent to rebase"), None);
        }
        observe_auto_merge(a, &r, n, &v, &p["d"], me)?;
        if p["d"]["comment_seen_at"].as_str() == Some(up.as_str()) && p["d"]["seen"].as_bool().unwrap_or(false) {
            return Ok(());
        }
        let first = !p["d"]["seen"].as_bool().unwrap_or(false);
        let cs = match comments(&r, n, me).await {
            Ok(x) => x,
            Err(e) => {
                a.say("watcher", &format!("{tag}: {}", crate::sh::head(&errm(&e), 200)), None);
                return Ok(());
            }
        };
        let cr: Vec<String> = if v["reviewDecision"] == "CHANGES_REQUESTED" {
            match pushers(&r,n).await{
                Ok(ids)=>ids,
                Err(error)=>{a.say("watcher",&format!("{tag}: reviewer permissions unavailable: {}",crate::sh::head(&error,160)),None);return Ok(());}
            }
        } else { vec![] };
        let tr = |c: &Value| MA.contains(&c["as"].as_str().unwrap_or("")) || (c["k"] == "review" && c["t"].as_str().map(|t| t.starts_with("[review: CHANGES_REQUESTED]")).unwrap_or(false) && c["rid"].as_str().map(|x| cr.iter().any(|y| y == x)).unwrap_or(false));
        let mut nw = Vec::new();
        for c in cs {
            let id = c["id"].as_str().filter(|id| !id.is_empty())
                .ok_or_else(|| X::E("PR comment has no identifier".into()))?;
            if !a.db.comment_acknowledged(&r, id).map_err(|e| X::E(e.to_string()))? {
                nw.push(c);
            }
        }
        let received_ids: Vec<String> = nw.iter().filter_map(|c| c["id"].as_str().map(String::from)).collect();
        let mut batch_acked = false;
        if first && !nw.is_empty() {
            let lc = ghj(&["pr", "view", &n.to_string(), "-R", &r, "--json", "commits", "--jq", ".commits[-1].committedDate"], 60).await.map_err(X::E)?;
            nw = pending_checked(nw,lc.as_str().unwrap_or(""))?;
        }
        if !nw.is_empty() {
            a.say("watcher", &format!("{tag}: {} new comment(s)", nw.len()), None);
            let g = guard(a, nw.iter().map(|c| (c["id"].as_str().unwrap_or("").to_string(), c["t"].as_str().unwrap_or("").to_string())).collect(), "watcher", None).await?;
            for c in nw.iter().filter(|c| g[c["id"].as_str().unwrap_or("")]["inj"].as_bool().unwrap_or(false)) {
                a.note("Guard: suspicious comment", &format!("{tag} by {}: {}\n{}", c["by"].as_str().unwrap_or(""), g[c["id"].as_str().unwrap_or("")]["why"].as_str().unwrap_or(""), v["url"].as_str().unwrap_or("")), 4);
            }
            let okc: Vec<&Value> = nw.iter().filter(|c| !g[c["id"].as_str().unwrap_or("")]["inj"].as_bool().unwrap_or(false)).collect();
            if !okc.is_empty() {
                let mc: Vec<&Value> = okc.iter().filter(|c| tr(c)).cloned().collect();
                let ma = !mc.is_empty();
                let src: &Vec<&Value> = if ma { &mc } else { &okc };
                let pv = ghj(&["pr", "view", &n.to_string(), "-R", &r, "--json", "body,files"], 60).await.map_err(X::E)?;
                if !pv["files"].is_array() {
                    return Err(X::E("PR file list is unavailable".into()));
                }
                let fl = pv["files"].as_array().map(|x| x.iter().filter_map(|f| f["path"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                let pr = format!("PR {tag}: {}\n\n{}\n\n{}\n\n{}", v["title"].as_str().unwrap_or(""), wrap("description", &crate::sh::head(pv["body"].as_str().unwrap_or(""), 4000)), wrap("files", &crate::sh::head(&fl, 2000)), src.iter().map(|c| wrap(c["id"].as_str().unwrap_or(""), &format!("{} by {} ({})\n\n{}", c["k"].as_str().unwrap_or(""), c["by"].as_str().unwrap_or(""), c["as"].as_str().unwrap_or(""), crate::sh::head(c["t"].as_str().unwrap_or(""), 4000)))).collect::<Vec<_>>().join("\n\n"));
                let mut o = Ask::new("watcher", a.md("watcher"), P("watcher"), pr);
                o.ef = Some("low");
                let (_, js) = ask(a, o).await?;
                if !js["act"].is_boolean() {
                    return Err(X::E("Watcher gave no readable action verdict".into()));
                }
                let by = okc.iter().filter_map(|c| c["by"].as_str()).collect::<Vec<_>>().join(", ");
                let act = js["act"].as_bool().unwrap_or(false);
                if act && ma {
                    let ch=js["change"].as_str().filter(|s|!s.trim().is_empty()).ok_or_else(||X::E("missing requested change".into()))?.to_string();
                    let data = json!({"change": ch, "comment": crate::sh::head(&mc.iter().map(|c| format!("{}: {}", c["by"].as_str().unwrap_or(""), c["t"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n\n"), 6000), "branch": hrn, "head": hd, "test_cmd": p["d"]["test_cmd"]});
                    let request = json!({"title":v["title"],"url":v["url"],"change":ch,"data":data});
                    if !a.db.queue_review_and_ack(&r, n, &up, &received_ids, &request).map_err(|e| X::E(e.to_string()))? {
                        a.say("watcher", &format!("{tag}: new review deferred until active review finishes"), None);
                        return Ok(());
                    }
                    batch_acked = true;
                    a.flow("watcher", "fixer", "review");
                    a.say("watcher", &format!("{tag}: change requested by {by}, sent to Fixer"), None);
                    a.note("Change requested", &format!("{tag} ({by}): {ch}\n\nDraft reply:\n{}\n{}", js["draft"].as_str().unwrap_or("-"), v["url"].as_str().unwrap_or("")), 4);
                } else {
                    a.say("watcher", &format!("{tag}: {by} commented{}", if act && !ma { " (not a maintainer, not applied)" } else { "" }), None);
                    a.note(&format!("Comment on {tag}"), &format!("{by}: {}\n\nDraft reply:\n{}\n{}", crate::sh::head(okc.last().and_then(|c| c["t"].as_str()).unwrap_or(""), 600), js["draft"].as_str().unwrap_or("-"), v["url"].as_str().unwrap_or("")), 3);
                }
                let rp = js["reply"].as_str().unwrap_or("").trim().to_string();
                let wk = format!("wr:{tag}");
                if ma && !rp.is_empty() && a.db.kg(&wk).as_i64().map(|t| now() - t > 86_400_000).unwrap_or(true) {
                    match gh(&["pr", "comment", &n.to_string(), "-R", &r, "--body", &crate::sh::head(&rp, 1500)], 60).await {
                        Ok(_) => {
                            a.db.ks(&wk, &json!(now()));
                            a.say("watcher", &format!("{tag}: answered {by}"), None);
                        }
                        Err(e) => a.say("watcher", &format!("{tag}: reply failed: {e}"), None),
                    }
                }
            }
        }
        // Advance the comment cursor only after successful analysis and queue persistence.
        if !batch_acked {
            a.db.ack_pr_comments(&r, n, &up, &received_ids).map_err(|e| X::E(e.to_string()))?;
        }

    Ok(())
}

async fn watch1(a: &Ax) -> R<()> {
    need(a, "watcher", &["gh"]).await?;
    a.st("watcher", "work", None, "");
    let me=crate::gh::login(a).await;
    if me.trim().is_empty(){return Err(X::E("GitHub identity unavailable".into()));}
    let mut lu=std::collections::HashMap::new();
    let mut default_branches=std::collections::HashMap::new();
    let rows=a.db.try_all("select * from prs where st='open' or (st='merged' and merged>strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 days')) order by repo,num",pa![]).map_err(|e|X::E(e.to_string()))?;
    for p in rows{
        a.held("watcher").await;
        let tag=format!("{}#{}",p["repo"].as_str().unwrap_or("?"),p["num"]);
        if let Err(error)=watch_pr(a,p,&me,&mut lu,&mut default_branches).await{
            a.say("watcher",&format!("{tag}: processing deferred: {}",crate::sh::head(&errm(&error),200)),None);
        }
    }
    a.st("watcher","idle",None,"");
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
    fn incomplete_pr_metadata_and_comment_timestamps_defer_processing() {
        assert!(valid_pr_timestamp(&json!({"updatedAt":"invalid"})).is_err());
        assert!(valid_pr_base(&json!({"baseRefName":""})).is_err());
        assert!(pending_checked(vec![json!({"at":"invalid"})],"2026-10-18T10:00:00Z").is_err());
        assert!(pending_checked(vec![json!({"at":"2026-10-19T10:00:00Z"})],"").is_err());
        assert_eq!(pending_checked(vec![json!({"at":"2026-10-19T10:00:00Z"})],"2026-10-18T10:00:00Z").unwrap().len(),1);
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
