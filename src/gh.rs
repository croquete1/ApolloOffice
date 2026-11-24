use crate::app::App;
use crate::db::now;
use crate::pa;
use crate::sh::{apip, ghj};
use serde_json::{json, Value};

pub fn cg(a: &App, k: &str, ttl: i64) -> Option<Value> {
    let c = a.cache.lock().unwrap();
    c.get(k)
        .filter(|(t, _)| now() - t < ttl)
        .map(|(_, v)| v.clone())
}

pub fn cs(a: &App, k: &str, v: &Value) {
    a.cache.lock().unwrap().insert(k.into(), (now(), v.clone()));
}

pub async fn me(a: &App) -> Result<Value, String> {
    if let Some(v) = cg(a, "me", 3_600_000) {
        return Ok(v);
    }
    let v = ghj(&["api", "user"], 60).await?;
    let l = v["login"].as_str().unwrap_or("").to_string();
    if l.is_empty() {
        return Err("gh is not logged in".into());
    }
    let id = v["id"].as_i64().unwrap_or(0);
    let nm = v["name"]
        .as_str()
        .filter(|x| !x.trim().is_empty())
        .unwrap_or(&l)
        .to_string();
    let o = json!({"login": l, "name": nm, "mail": format!("{id}+{l}@users.noreply.github.com")});
    cs(a, "me", &o);
    Ok(o)
}

pub async fn login(a: &App) -> String {
    me(a)
        .await
        .ok()
        .and_then(|v| v["login"].as_str().map(String::from))
        .unwrap_or_default()
}

pub async fn meta(a: &App, r: &str) -> Result<Value, String> {
    let k = format!("meta:{r}");
    if let Some(v) = cg(a, &k, 600_000) {
        return Ok(v);
    }
    let v = ghj(&["api", &format!("repos/{r}")], 60).await?;
    let o = json!({"push": v["permissions"]["push"].as_bool().unwrap_or(false), "main": v["default_branch"].as_str().unwrap_or("main"), "private": v["private"].as_bool().unwrap_or(false), "fork": v["fork"].as_bool().unwrap_or(false), "arch": v["archived"].as_bool().unwrap_or(false), "full": v["full_name"].as_str().unwrap_or(r)});
    cs(a, &k, &o);
    Ok(o)
}

/// Fetch the complete open issue listing, excluding pull requests returned by REST.
pub async fn open_issues(repo: &str) -> Result<Vec<Value>, String> {
    let rows = apip(&format!(
        "repos/{repo}/issues?state=open&sort=created&direction=asc&per_page=100"
    ))
    .await?;
    normalize_open_issues(rows)
}

/// Complete lightweight inventory for Hunter. Do not use capped CLI lists.
pub async fn open_pr_summaries(repo: &str) -> Result<Vec<Value>, String> {
    let rows = apip(&format!(
        "repos/{repo}/pulls?state=open&sort=created&direction=asc&per_page=100"
    ))
    .await?;
    normalize_open_pr_summaries(rows)
}

fn normalize_open_pr_summaries(rows: Vec<Value>) -> Result<Vec<Value>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut summaries = Vec::new();
    for row in rows {
        let number = row["number"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or("PR inventory contains an invalid number")?;
        if !seen.insert(number)
            || row["state"] != "open"
            || !row["title"].is_string()
            || !(row["body"].is_null() || row["body"].is_string())
        {
            return Err(format!(
                "PR #{number} has duplicate or incomplete inventory metadata"
            ));
        }
        summaries.push(json!({"number":number,"title":row["title"],"body":row["body"]}));
    }
    Ok(summaries)
}

fn normalize_open_issues(rows: Vec<Value>) -> Result<Vec<Value>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut issues = Vec::new();
    for row in rows {
        let number = row["number"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or("GitHub issue listing contains an invalid number")?;
        if !seen.insert(number) {
            return Err("GitHub issue listing changed during pagination (duplicate number)".into());
        }
        if let Some(marker) = row.get("pull_request") {
            if !marker.is_object() {
                return Err(format!("GitHub issue #{number} has invalid PR metadata"));
            }
            continue;
        }
        if row["state"] != "open"
            || !row["title"].is_string()
            || !(row["body"].is_null() || row["body"].is_string())
            || !row["html_url"].as_str().is_some_and(|v| !v.is_empty())
            || !row["updated_at"]
                .as_str()
                .is_some_and(|v| chrono::DateTime::parse_from_rfc3339(v).is_ok())
            || !row["created_at"]
                .as_str()
                .is_some_and(|v| chrono::DateTime::parse_from_rfc3339(v).is_ok())
            || !row["labels"]
                .as_array()
                .is_some_and(|v| v.iter().all(|x| x["name"].is_string()))
            || !row["assignees"]
                .as_array()
                .is_some_and(|v| v.iter().all(|x| x["login"].is_string()))
        {
            return Err(format!(
                "GitHub issue #{number} has incomplete or invalid metadata"
            ));
        }
        issues.push(json!({
            "number":number,"title":row["title"],"body":row["body"],
            "author":row["user"],"createdAt":row["created_at"],"updatedAt":row["updated_at"],
            "labels":row["labels"],"assignees":row["assignees"],"url":row["html_url"]
        }));
    }
    Ok(issues)
}

#[cfg(test)]
mod issue_listing_tests {
    use super::*;

    fn issue(number: i64) -> Value {
        json!({"number":number,"state":"open","title":"Bug","html_url":format!("https://github.com/o/r/issues/{number}"),
            "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z",
            "labels":[],"assignees":[],"user":{"login":"reporter"}})
    }

    #[test]
    fn accepts_large_listing_and_excludes_pull_requests() {
        let mut rows: Vec<Value> = (1..=501).map(issue).collect();
        rows.push(json!({"number":502,"pull_request":{}}));
        let result = normalize_open_issues(rows).unwrap();
        assert_eq!(result.len(), 501);
        assert_eq!(result[500]["number"], 501);
        assert_eq!(result[0]["author"]["login"], "reporter");
    }

    #[test]
    fn pr_summaries_do_not_cap_or_accept_duplicate_and_incomplete_rows() {
        let row = |n| json!({"number":n,"state":"open","title":"Fix","body":null});
        let rows = (1..=1001).map(row).collect();
        assert_eq!(normalize_open_pr_summaries(rows).unwrap().len(), 1001);
        assert!(normalize_open_pr_summaries(vec![row(1), row(1)]).is_err());
        assert!(normalize_open_pr_summaries(vec![json!({"number":1})]).is_err());
        assert!(normalize_open_pr_summaries(vec![]).unwrap().is_empty());
    }

    #[test]
    fn rejects_malformed_issue_body_and_pull_request_marker() {
        let mut row = issue(1);
        row["body"] = json!({"unexpected":true});
        assert!(normalize_open_issues(vec![row]).is_err());
        let mut row = issue(2);
        row["pull_request"] = json!(false);
        assert!(normalize_open_issues(vec![row]).is_err());
        let mut row = issue(3);
        row["body"] = json!("valid description");
        assert_eq!(normalize_open_issues(vec![row]).unwrap().len(), 1);
    }

    #[test]
    fn overlap_inventory_checks_counts_duplicates_and_renames() {
        let renamed = json!({"filename":"src/new.rs","previous_filename":"src/old.rs","status":"renamed"});
        let paths = overlap_paths(vec![renamed.clone()], 1).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[1]["path"], "src/old.rs");
        assert!(overlap_paths(vec![renamed.clone()], 2).is_err());
        assert!(overlap_paths(vec![renamed.clone(), renamed], 2).is_err());
        assert!(overlap_paths(
            vec![json!({"filename":"src/new.rs","status":"renamed"})],
            1
        )
        .is_err());
    }

    #[test]
    fn rejects_duplicate_and_incomplete_listing() {
        assert!(normalize_open_issues(vec![issue(1), issue(1)]).is_err());
        assert!(normalize_open_issues(vec![issue(1), json!({"number":2})]).is_err());
        assert!(normalize_open_issues(vec![]).unwrap().is_empty());
    }
}

pub struct Rf {
    pub n: i64,
    pub rp: String,
    pub st: String,
    pub by: String,
}

pub async fn refs(r: &str, n: i64) -> Result<Vec<Rf>, String> {
    let mut o = vec![];
    for e in apip(&format!("repos/{r}/issues/{n}/timeline?per_page=100")).await? {
        let i = &e["source"]["issue"];
        if e["event"] == "cross-referenced" && i["pull_request"].is_object() {
            o.push(Rf {
                n: i["number"].as_i64().unwrap_or(0),
                rp: i["repository"]["full_name"].as_str().unwrap_or("").into(),
                st: if i["pull_request"]["merged_at"].is_string() {
                    "merged".into()
                } else {
                    i["state"].as_str().unwrap_or("").into()
                },
                by: i["user"]["login"].as_str().unwrap_or("").into(),
            });
        }
    }
    let s = ghj(
        &[
            "pr",
            "list",
            "-R",
            r,
            "--state",
            "all",
            "--search",
            &n.to_string(),
            "--json",
            "number,state,title,body,author",
            "--limit",
            "40",
        ],
        60,
    )
    .await?;
    let re = regex::Regex::new(&format!(r"#{n}(\D|$)|issues/{n}(\D|$)")).unwrap();
    for p in s.as_array().cloned().unwrap_or_default() {
        if re.is_match(&format!(
            "{} {}",
            p["title"].as_str().unwrap_or(""),
            p["body"].as_str().unwrap_or("")
        )) {
            o.push(Rf {
                n: p["number"].as_i64().unwrap_or(0),
                rp: r.into(),
                st: p["state"].as_str().unwrap_or("").to_lowercase(),
                by: p["author"]["login"].as_str().unwrap_or("").into(),
            });
        }
    }
    Ok(o)
}

pub fn live(x: Vec<Rf>) -> Vec<Rf> {
    let mut seen = std::collections::HashSet::new();
    x.into_iter()
        .filter(|p| (p.st == "open" || p.st == "merged") && seen.insert((p.rp.clone(), p.n)))
        .collect()
}

fn overlap_paths(rows: Vec<Value>, expected: u64) -> Result<Vec<Value>, String> {
    if rows.len() as u64 != expected {
        return Err("Incomplete PR file inventory".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut paths = std::collections::BTreeSet::new();
    for row in rows {
        let path = row["filename"]
            .as_str()
            .filter(|p| !p.is_empty())
            .ok_or("Missing PR filename")?;
        if !seen.insert(path.to_string()) {
            return Err("Duplicate PR filename".into());
        }
        paths.insert(path.to_string());
        if row["status"] == "renamed" || row["status"] == "copied" {
            let previous = row["previous_filename"]
                .as_str()
                .filter(|p| !p.is_empty())
                .ok_or("Missing previous PR filename")?;
            paths.insert(previous.to_string());
        }
    }
    Ok(paths.into_iter().map(|path| json!({"path":path})).collect())
}

pub async fn twins(_a: &App, r: &str, w: &str) -> Result<(Vec<Value>, Vec<Value>), String> {
    let re = regex::Regex::new(r"[\w./-]+\.\w+").unwrap();
    let fs: Vec<String> = re
        .find_iter(w)
        .map(|m| m.as_str().to_string())
        .collect();
    let before = open_pr_summaries(r).await?;
    let mut h = Vec::new();
    let mut o = Vec::new();
    for mut p in before.clone() {
        let number = p["number"].as_i64().ok_or("Invalid PR number")?;
        let endpoint = format!("repos/{r}/pulls/{number}");
        let detail = ghj(&["api", &endpoint], 60).await?;
        let count = detail["changed_files"]
            .as_u64()
            .ok_or("Missing PR file count")?;
        let head = detail["head"]["sha"]
            .as_str()
            .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("Missing PR HEAD")?;
        if detail["number"] != number
            || detail["state"] != "open"
            || detail["title"] != p["title"]
            || detail["body"] != p["body"]
        {
            return Err("PR changed before overlap scan".into());
        }
        let rows = apip(&format!("{endpoint}/files?per_page=100")).await?;
        let paths = overlap_paths(rows, count)?;
        let after = ghj(&["api", &endpoint], 60).await?;
        if after["number"] != number
            || after["state"] != "open"
            || after["head"]["sha"] != head
            || after["base"]["sha"] != detail["base"]["sha"]
            || after["updated_at"] != detail["updated_at"]
            || after["changed_files"] != count
            || after["title"] != p["title"]
            || after["body"] != p["body"]
        {
            return Err("PR changed during overlap scan".into());
        }
        let hit = paths.iter().any(|x| {
            let path = x["path"].as_str().unwrap_or("");
            fs.iter().any(|y| {
                y == path || y.strip_suffix(path).is_some_and(|prefix| prefix.ends_with('/'))
            })
        });
        p["files"] = json!(paths);
        if hit {
            h.push(p);
        } else {
            o.push(p);
        }
    }
    if open_pr_summaries(r).await? != before {
        return Err("Open PR inventory changed during overlap scan".into());
    }
    Ok((h, o))
}

pub async fn files(r: &str, p: &str) -> Vec<Value> {
    ghj(&["api", &format!("repos/{r}/contents/{p}")], 60)
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

pub async fn file(r: &str, p: &str) -> Option<String> {
    let v = ghj(&["api", &format!("repos/{r}/contents/{p}")], 60)
        .await
        .ok()?;
    let b = v["content"].as_str()?.replace(['\n', '\r'], "");
    use base64::Engine;
    String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b).ok()?).ok()
}

pub fn pick_ci(
    names: &[String],
    pkg: Option<&str>,
    mk: Option<&str>,
    xtask: bool,
) -> (String, String) {
    let has = |n: &str| names.iter().any(|x| x == n);
    if has("Cargo.toml") {
        if xtask {
            return ("cargo xtask ci".into(), String::new());
        }
        return ("cargo test --workspace".into(), String::new());
    }
    if has("package.json") {
        let p: Value = pkg
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(json!({}));
        let t = p["scripts"]["test"].as_str().unwrap_or("");
        if !t.is_empty() && !t.contains("no test specified") {
            let (m, i) = if has("pnpm-lock.yaml") {
                ("pnpm", "pnpm install --frozen-lockfile")
            } else if has("yarn.lock") {
                ("yarn", "yarn install --frozen-lockfile")
            } else if has("package-lock.json") {
                ("npm", "npm ci")
            } else {
                ("npm", "npm install")
            };
            return (format!("{m} test"), i.into());
        }
    }
    if has("go.mod") {
        return ("go test ./...".into(), String::new());
    }
    if has("pyproject.toml")
        || has("pytest.ini")
        || has("setup.py")
        || has("setup.cfg")
        || has("tox.ini")
    {
        let e = if has("pyproject.toml") || has("setup.py") {
            " -e ."
        } else {
            ""
        };
        let i = if has("requirements-dev.txt") {
            format!("python3 -m venv .venv && . .venv/bin/activate && pip install -q -r requirements-dev.txt{e}")
        } else if has("tool/requirements-test.txt") {
            format!("python3 -m venv .venv && . .venv/bin/activate && pip install -q -r tool/requirements-test.txt{e}")
        } else if has("requirements.txt") {
            format!("python3 -m venv .venv && . .venv/bin/activate && pip install -q pytest -r requirements.txt{e}")
        } else {
            format!("python3 -m venv .venv && . .venv/bin/activate && pip install -q pytest{e}")
        };
        return (". .venv/bin/activate && pytest".into(), i);
    }
    if has("Makefile")
        && mk
            .map(|m| m.lines().any(|l| l.starts_with("test:")))
            .unwrap_or(false)
    {
        return ("make test".into(), String::new());
    }
    (String::new(), String::new())
}

fn python_version(value: &str) -> Option<String> {
    let value = value.trim();
    regex::Regex::new(r"^3\.(12|13|14)(\.[0-9]{1,2})?$")
        .unwrap()
        .is_match(value)
        .then(|| value.to_string())
}

fn workflow_python_version(text: &str) -> Option<String> {
    if !text.contains("actions/setup-python@") {
        return None;
    }
    let pattern = regex::Regex::new(
        r#"(?m)^\s+python-version:\s*['\"]?(3\.(?:12|13|14)(?:\.[0-9]{1,2})?)['\"]?\s*(?:#.*)?$"#,
    )
    .unwrap();
    let versions: std::collections::BTreeSet<String> = pattern
        .captures_iter(text)
        .filter_map(|m| python_version(&m[1]))
        .collect();
    if versions.len() == 1 {
        versions.into_iter().next()
    } else {
        None
    }
}

fn isolated_python_setup(setup: &str, version: &str) -> String {
    let Some(version) = python_version(version) else {
        return setup.to_string();
    };
    // Only a validated version reaches this constant worker-only bootstrap.
    // Never copy run scripts, shell fragments or credentials from CI YAML.
    let prefix=format!("python3 -m venv .venv-bootstrap && .venv-bootstrap/bin/python -m pip install -q uv==0.13.3 && .venv-bootstrap/bin/uv venv --no-project --managed-python --seed --python {version} .venv");
    setup.replacen("python3 -m venv .venv", &prefix, 1)
}

pub async fn detect(r: &str) -> (String, String) {
    let fs = files(r, "").await;
    let mut names: Vec<String> = fs
        .iter()
        .filter_map(|f| f["name"].as_str().map(String::from))
        .collect();
    if names.iter().any(|x| x == "pyproject.toml")
        && names.iter().any(|x| x == "tool")
        && files(r, "tool")
            .await
            .iter()
            .any(|f| f["name"] == "requirements-test.txt")
    {
        names.push("tool/requirements-test.txt".into());
    }
    let pkg = if names.iter().any(|x| x == "package.json") {
        file(r, "package.json").await
    } else {
        None
    };
    let mk = if names.iter().any(|x| x == "Makefile") {
        file(r, "Makefile").await
    } else {
        None
    };
    let xt = names.iter().any(|x| x == "xtask")
        || names.iter().any(|x| x == ".cargo")
            && file(r, ".cargo/config.toml")
                .await
                .map(|c| c.contains("xtask"))
                .unwrap_or(false);
    let (ci, mut setup) = pick_ci(&names, pkg.as_deref(), mk.as_deref(), xt);
    if setup.starts_with("python3 -m venv .venv") {
        let mut version = if names.iter().any(|n| n == ".python-version") {
            file(r, ".python-version")
                .await
                .and_then(|s| python_version(&s))
        } else {
            None
        };
        if version.is_none() && names.iter().any(|n| n == ".github") {
            let mut workflows = files(r, ".github/workflows").await;
            workflows.sort_by_key(|f| f["name"].as_str().unwrap_or("").to_string());
            let mut versions = std::collections::BTreeSet::new();
            for f in workflows
                .into_iter()
                .filter(|f| {
                    let n = f["name"].as_str().unwrap_or("");
                    (n.ends_with(".yml") || n.ends_with(".yaml"))
                        && n.bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
                })
                .take(4)
            {
                if let Some(text) = file(
                    r,
                    &format!(".github/workflows/{}", f["name"].as_str().unwrap()),
                )
                .await
                {
                    if let Some(v) = workflow_python_version(&text) {
                        versions.insert(v);
                    }
                }
            }
            if versions.len() == 1 {
                version = versions.into_iter().next();
            }
        }
        if let Some(version) = version {
            setup = isolated_python_setup(&setup, &version);
        }
    }
    (ci, setup)
}

pub async fn opened(a: &App, r: &str) -> i64 {
    a.db.n(
        "select count(*) from prs where repo=? and st='open'",
        pa![r.to_string()],
    ) + a.db.n(
        "select count(*) from jobs where repo=? and st in ('approve') ",
        pa![r.to_string()],
    )
}

pub async fn today(a: &App, r: &str) -> i64 {
    a.db.n(
        "select count(*) from prs where repo=? and t0>=?",
        pa![r.to_string(), crate::db::day0()],
    )
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn ci_python_runtime_is_validated_and_isolated_in_the_worker() {
        let workflow="      - uses: actions/setup-python@v6\n        with:\n          python-version: '3.13'\n          cache: pip\n";
        assert_eq!(workflow_python_version(workflow), Some("3.13".into()));
        assert!(
            workflow_python_version(&format!("{workflow}          python-version: '3.12'\n"))
                .is_none()
        );
        assert!(workflow_python_version("python-version: '${{ matrix.python }}'").is_none());
        let setup="python3 -m venv .venv && . .venv/bin/activate && pip install -q -r tool/requirements-test.txt -e .";
        let isolated = isolated_python_setup(setup, "3.13");
        assert!(isolated.contains("--managed-python --seed --python 3.13 .venv"));
        assert!(isolated.ends_with("pip install -q -r tool/requirements-test.txt -e ."));
        assert!(!isolated.contains("sudo"));
        for value in [
            "3.13;id",
            "../python",
            "3.13 && curl evil",
            "${PYTHON}",
            "2.7",
        ] {
            assert!(python_version(value).is_none());
            assert_eq!(isolated_python_setup(setup, value), setup);
        }
    }

    #[test]
    fn lives() {
        let r = |n: i64, st: &str| Rf {
            n,
            rp: "o/r".into(),
            st: st.into(),
            by: "x".into(),
        };
        let l = live(vec![
            r(1, "merged"),
            r(1, "merged"),
            r(2, "closed"),
            r(3, "open"),
        ]);
        assert_eq!(l.iter().map(|p| p.n).collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn ci() {
        let n = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            pick_ci(&n(&["Cargo.toml", "xtask"]), None, None, true).0,
            "cargo xtask ci"
        );
        assert_eq!(
            pick_ci(&n(&["Cargo.toml"]), None, None, false).0,
            "cargo test --workspace"
        );
        assert_eq!(
            pick_ci(
                &n(&["package.json", "pnpm-lock.yaml"]),
                Some(r#"{"scripts":{"test":"vitest run"}}"#),
                None,
                false
            ),
            ("pnpm test".into(), "pnpm install --frozen-lockfile".into())
        );
        assert_eq!(
            pick_ci(
                &n(&["package.json"]),
                Some(r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#),
                None,
                false
            )
            .0,
            ""
        );
        assert_eq!(
            pick_ci(&n(&["go.mod"]), None, None, false).0,
            "go test ./..."
        );
        assert!(pick_ci(&n(&["pyproject.toml"]), None, None, false)
            .0
            .ends_with("pytest"));
        assert!(pick_ci(&n(&["pyproject.toml"]), None, None, false)
            .1
            .ends_with(" -e ."));
        assert_eq!(pick_ci(&n(&["pyproject.toml","tool/requirements-test.txt"]),None,None,false),
            (". .venv/bin/activate && pytest".into(),"python3 -m venv .venv && . .venv/bin/activate && pip install -q -r tool/requirements-test.txt -e .".into()));
        assert!(
            pick_ci(&n(&["pytest.ini", "requirements.txt"]), None, None, false)
                .1
                .ends_with("-r requirements.txt")
        );
        assert_eq!(
            pick_ci(
                &n(&["Makefile"]),
                None,
                Some("all:\n\ttrue\ntest:\n\t./t.sh\n"),
                false
            )
            .0,
            "make test"
        );
        assert_eq!(pick_ci(&n(&["README.md"]), None, None, false).0, "");
    }
}
