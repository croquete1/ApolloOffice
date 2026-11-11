use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn sh(d: &Path, c: &str) {
    let b = if cfg!(windows) {
        r"C:\Program Files\Git\bin\bash.exe"
    } else {
        "bash"
    };
    let o = Command::new(b)
        .arg("-c")
        .arg(c)
        .current_dir(d)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{c}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[cfg(windows)]
fn wrap(d: &Path, n: &str, py: &Path) -> PathBuf {
    let f = d.join(format!("{n}.cmd"));
    std::fs::write(&f, format!("@python \"{}\" %*\r\n", py.display())).unwrap();
    f
}

#[cfg(unix)]
fn wrap(d: &Path, n: &str, py: &Path) -> PathBuf {
    let f = d.join(n);
    std::fs::write(
        &f,
        format!("#!/bin/sh\nexec python3 \"{}\" \"$@\"\n", py.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
    f
}

fn furl(p: &Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}/")
    } else {
        format!("file:///{s}/")
    }
}

struct K(Child);
impl Drop for K {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn public_repo() {
    full_run(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn private_repo() {
    full_run(true).await;
}

async fn full_run(pv: bool) {
    let t =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("e2e-{}-{pv}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    std::fs::create_dir_all(&t).unwrap();
    let rm = t.join("remotes");
    let fk = t.join("ghd");
    std::fs::create_dir_all(rm.join("o")).unwrap();
    std::fs::create_dir_all(&fk).unwrap();
    let seed = t.join("seed");
    std::fs::create_dir_all(seed.join("t")).unwrap();
    std::fs::write(seed.join("add.sh"), "add() { echo $(( $1 - $2 )); }\n").unwrap();
    std::fs::write(
        seed.join("t/run.sh"),
        "e=0\nfor f in t/*_test.sh; do bash \"$f\" || e=1; done\nexit $e\n",
    )
    .unwrap();
    std::fs::write(seed.join("t/zero_test.sh"), "source ./add.sh\n[ \"$(add 0 0)\" = 0 ] && echo 'ok 1 - zero' || { echo 'not ok 1 - zero'; exit 1; }\n").unwrap();
    sh(&seed, "git init -q -b main . && git -c user.name=t -c user.email=t@t add -A && git -c user.name=t -c user.email=t@t commit -q -m init");
    sh(
        &rm.join("o"),
        &format!(
            "git clone -q --bare \"{}\" r.git",
            seed.display().to_string().replace('\\', "/")
        ),
    );
    std::fs::write(fk.join("issues.json"), json!([{"number": 1, "title": "add subtracts instead of adding", "body": "`add 2 3` prints -1, expected 5.", "author": {"login": "someone"}, "updatedAt": "2026-01-01T00:00:00Z", "labels": [], "assignees": [], "url": "https://github.com/o/r/issues/1"}]).to_string()).unwrap();
    let td = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let gw = wrap(&t, "gh", &td.join("fake-gh.py"));
    let cw = wrap(&t, "claude", &td.join("fake-claude.py"));
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let data = t.join("data");
    let exe = env!("CARGO_BIN_EXE_apollo-office");
    let envs = [
        ("APOLLO_OFFICE_GH", gw.display().to_string()),
        ("APOLLO_OFFICE_CLAUDE", cw.display().to_string()),
        ("APOLLO_OFFICE_REMOTE", furl(&rm)),
        ("APOLLO_OFFICE_FAST", "1".into()),
        ("FAKE_GH_DIR", fk.display().to_string()),
        ("FAKE_GH_PRIVATE", if pv { "1".into() } else { "0".into() }),
    ];
    let _k = K(Command::new(exe)
        .args([
            "--data",
            &data.display().to_string(),
            "--port",
            &port.to_string(),
            "--quiet",
        ])
        .envs(envs.iter().cloned())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap());
    std::thread::sleep(Duration::from_millis(1500));
    let l = Command::new(exe)
        .args(["--data", &data.display().to_string(), "login-link"])
        .output()
        .unwrap();
    let l = String::from_utf8_lossy(&l.stdout)
        .lines()
        .find(|x| x.starts_with("http"))
        .unwrap()
        .trim()
        .to_string();
    let o = format!("http://127.0.0.1:{port}");
    let c = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let r = c.post(&l).header("origin", &o).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 303);
    let ck = r
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let get = |p: &str| {
        let c = c.clone();
        let u = format!("{o}{p}");
        let ck = ck.clone();
        async move {
            c.get(u)
                .header("cookie", ck)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let post = |p: &str, b: Value| {
        let c = c.clone();
        let u = format!("{o}{p}");
        let ck = ck.clone();
        let o = o.clone();
        async move {
            c.post(u)
                .header("cookie", ck)
                .header("origin", o)
                .header("x-ao", "1")
                .json(&b)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    assert_eq!(
        c.get(format!("{o}/api/state"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
    assert_eq!(
        c.post(format!("{o}/api/act"))
            .header("cookie", &ck)
            .header("origin", "http://evil.example")
            .header("x-ao", "1")
            .json(&json!({"a": "pause", "on": true}))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );
    let h = post(
        "/api/host",
        json!({"op": "set", "id": "local", "dk": false}),
    )
    .await;
    assert!(h["error"].is_string(), "direct mode must need confirmation");
    let h = post(
        "/api/host",
        json!({"op": "set", "id": "local", "dk": false, "raw": true, "ln": 1}),
    )
    .await;
    assert_eq!(h["ok"], true, "{h}");
    let a = post("/api/repo", json!({"op": "add", "full": "o/r"})).await;
    assert_eq!(a["full"], "o/r", "{a}");
    post(
        "/api/repo",
        json!({"op": "set", "full": "o/r", "ci": "bash t/run.sh", "hunt": false}),
    )
    .await;
    let t0 = Instant::now();
    let jid = loop {
        let s = get("/api/state").await;
        if let Some(j) = s["q"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["st"] == "approve")
        {
            break j["id"].as_i64().unwrap();
        }
        if let Some(j) = s["dn"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["st"] == "drop")
        {
            panic!("dropped: {j}\n{}", s["ev"]);
        }
        assert!(
            t0.elapsed() < Duration::from_secs(240),
            "no approval in time: {}",
            s["ev"]
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let j = get(&format!("/api/job?id={jid}")).await;
    let pb = j["pb"].as_str().unwrap();
    assert!(pb.starts_with("Fixes #1"), "{pb}");
    assert!(
        pb.contains("`adds two numbers`: fails on `main` without the fix"),
        "{pb}"
    );
    assert!(pb.contains("`bash t/run.sh` passes."), "{pb}");
    assert!(pb.contains("Generated with AI by [ApolloOffice]"), "{pb}");
    assert!(
        j["df"]
            .as_str()
            .unwrap()
            .contains("+add() { echo $(( $1 + $2 )); }"),
        "{}",
        j["df"]
    );
    let r = post(
        "/api/act",
        json!({"a": "approve", "id": jid, "pt": "Make add sum its arguments"}),
    )
    .await;
    assert!(r["error"].is_null(), "{r}");
    let t0 = Instant::now();
    loop {
        let s = get("/api/state").await;
        if let Some(x) = s["dn"].as_array().unwrap().iter().find(|x| x["id"] == jid) {
            assert_eq!(x["st"], "done", "{x}\n{}", s["ev"]);
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(120),
            "not submitted: {}",
            s["ev"]
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let ps: Value =
        serde_json::from_str(&std::fs::read_to_string(fk.join("prs.json")).unwrap()).unwrap();
    assert_eq!(ps.as_array().unwrap().len(), 1);
    assert_eq!(ps[0]["base"], "main");
    assert_eq!(ps[0]["title"], "Make add sum its arguments");
    let br = ps[0]["head"].as_str().unwrap().to_string();
    assert!(br.starts_with("fix/1-"), "{br}");
    let o = Command::new("git")
        .args([
            "--git-dir",
            &rm.join("o/r.git").display().to_string(),
            "log",
            "-1",
            "--format=%B",
            &br,
        ])
        .output()
        .unwrap();
    let m = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(
        m.starts_with("fix: Make add sum its arguments\n\nFixes #1"),
        "{m}"
    );
    assert!(
        m.contains("Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"),
        "{m}"
    );
    assert!(
        m.contains("Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"),
        "{m}"
    );
    let o = Command::new("git")
        .args([
            "--git-dir",
            &rm.join("o/r.git").display().to_string(),
            "show",
            &format!("{br}:add.sh"),
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&o.stdout).contains("$1 + $2"));
    let s = get("/api/state").await;
    assert_eq!(s["pr"][0]["num"], 100);
}
