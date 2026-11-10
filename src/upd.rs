use crate::app::Ax;
use crate::db::now;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

// This customized fork must never silently install releases signed for upstream.
// Operators may opt in to a separately signed, HTTPS-only fork release channel.

pub fn asset() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "apollo-office-windows-arm64.exe",
        ("windows", _) => "apollo-office-windows-x64.exe",
        ("macos", "aarch64") => "apollo-office-macos-arm64",
        ("macos", _) => "apollo-office-macos-x64",
        ("linux", "aarch64") => "apollo-office-linux-arm64",
        _ => "apollo-office-linux-x64",
    }
}

static CK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn newer(a: &str, b: &str) -> bool {
    let p = |s: &str| -> (Vec<u64>, bool) {
        let s = s.trim().trim_start_matches('v');
        let (n, pre) = s
            .split_once('-')
            .map(|(n, _)| (n, true))
            .unwrap_or((s, false));
        let mut v: Vec<u64> = n
            .split('.')
            .take(3)
            .map(|x| x.parse().unwrap_or(0))
            .collect();
        v.resize(3, 0);
        (v, !pre)
    };
    p(a) > p(b)
}

fn writable(me: &Path) -> bool {
    let Some(d) = me.parent() else { return false };
    let f = d.join(format!(".apollo-office-w{}", std::process::id()));
    let ok = std::fs::write(&f, b"").is_ok();
    let _ = std::fs::remove_file(&f);
    ok
}

pub fn check_sig(pk: &str, body: &[u8], sig: &str) -> Result<Value, String> {
    let k: [u8; 32] = hex::decode(pk)
        .map_err(|_| "key")?
        .try_into()
        .map_err(|_| "key")?;
    let vk = VerifyingKey::from_bytes(&k).map_err(|_| "key")?;
    let sb = base64::engine::general_purpose::STANDARD
        .decode(sig.trim())
        .map_err(|_| "broken signature")?;
    let s = Signature::from_slice(&sb).map_err(|_| "broken signature")?;
    vk.verify(body, &s)
        .map_err(|_| "invalid signature on update manifest")?;
    serde_json::from_slice(body).map_err(|_| "unreadable update manifest".into())
}

fn st(a: &Ax, f: impl FnOnce(&mut Value)) {
    let mut v = a.db.kg("upd");
    if !v.is_object() {
        v = json!({});
    }
    f(&mut v);
    a.db.ks("upd", &v);
}

fn trusted_channel_inputs(
    url: Option<&str>,
    key: Option<&str>,
) -> Result<(String, String), String> {
    let url = url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("No trusted fork update source configured (APOLLO_OFFICE_UPDATE_URL)")?;
    let key = key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("No trusted fork signing key configured (APOLLO_OFFICE_UPDATE_PUBKEY)")?;
    let parsed = reqwest::Url::parse(url).map_err(|_| "Invalid fork update URL")?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "Fork update source must be an HTTPS URL without credentials, query or fragment".into(),
        );
    }
    let decoded: [u8; 32] = hex::decode(key)
        .map_err(|_| "Invalid fork signing key")?
        .try_into()
        .map_err(|_| "Fork signing key must be 32 bytes")?;
    VerifyingKey::from_bytes(&decoded).map_err(|_| "Invalid fork signing key")?;
    Ok((
        url.trim_end_matches('/').to_string(),
        key.to_ascii_lowercase(),
    ))
}

fn channel() -> Result<(String, String), String> {
    let url = std::env::var("APOLLO_OFFICE_UPDATE_URL").ok();
    let key = std::env::var("APOLLO_OFFICE_UPDATE_PUBKEY").ok();
    trusted_channel_inputs(url.as_deref(), key.as_deref())
}

fn channel_id(url: &str, key: &str) -> String {
    hex::encode(Sha256::digest(format!("{url}\n{key}").as_bytes()))
}

pub fn configured() -> bool {
    channel().is_ok()
}

pub fn on(a: &Ax) -> bool {
    a.set()["updates"].as_bool().unwrap_or(false) && configured()
}

pub fn info(a: &Ax) -> Value {
    let mut v = a.db.kg("upd");
    if !v.is_object() {
        v = json!({});
    }
    v["configured"] = json!(configured());
    v["auto"] = json!(on(a));
    v
}

async fn get(a: &Ax, u: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut r = a.hc.get(u)
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status().as_u16() == 404 {
        return Err("no signed fork release published".into());
    }
    if !r.status().is_success() {
        return Err(format!("HTTP {} for {u}", r.status().as_u16()));
    }
    if r.content_length().is_some_and(|size| size > limit as u64) {
        return Err("update download exceeds size limit".into());
    }
    let mut out = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| e.to_string())? {
        if chunk.len() > limit.saturating_sub(out.len()) {
            return Err("update download exceeds size limit".into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

pub async fn check(a: &Ax) -> Result<Value, String> {
    let _g = CK.lock().await;
    // No network access or staged binary mutation without an explicit
    // independently signed channel for this fork.
    let (b, key) = channel()?;
    let cid = channel_id(&b, &key);
    let r: Result<Value, String> = async {
        let body = get(a, &format!("{b}/latest.json"), 128 * 1024).await?;
        let sig = String::from_utf8_lossy(
            &get(a, &format!("{b}/latest.json.sig"), 4 * 1024).await?
        ).to_string();
        let m = check_sig(&key, &body, &sig)?;
        let v = m["v"].as_str().unwrap_or("").to_string();
        st(a, |x| {
            x["latest"] = json!(v);
        });
        if !newer(&v, crate::app::V) {
            return Ok(json!({"latest": v, "new": false}));
        }
        let want = m["files"][asset()]
            .as_str()
            .ok_or("no build for this system in the release")?
            .to_string();
        let bin = get(a, &format!("{b}/{}", asset()), 128 * 1024 * 1024).await?;
        if hex::encode(Sha256::digest(&bin)) != want {
            return Err("downloaded update does not match the signed checksum".into());
        }
        let f = staged(a);
        let tmp = f.with_extension("part");
        std::fs::write(&tmp, &bin).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &f).map_err(|e| e.to_string())?;
        st(a, |x| {
            x["ready"] = json!(v);
            x["sha"] = json!(want);
            x["channel"] = json!(cid);
        });
        a.say(
            "submit",
            &format!("update {v} downloaded, it installs once the running jobs are done"),
            None,
        );
        Ok(json!({"latest": v, "new": true}))
    }
    .await;
    st(a, |x| {
        x["t"] = json!(now());
        x["err"] = json!(r.as_ref().err().cloned().unwrap_or_default());
    });
    r
}

pub fn staged(a: &Ax) -> PathBuf {
    a.dd.join(if cfg!(windows) {
        "update.exe"
    } else {
        "update.bin"
    })
}

pub fn ready(a: &Ax) -> bool {
    let Ok((url, key)) = channel() else {
        return false;
    };
    let state = a.db.kg("upd");
    state["channel"].as_str() == Some(channel_id(&url, &key).as_str())
        && state["ready"]
            .as_str()
            .map(|v| newer(v, crate::app::V))
            .unwrap_or(false)
        && staged(a).exists()
}

pub fn idle(a: &Ax) -> bool {
    // Release each mutex before checking the SQLite state: workers may lock
    // these structures in a different order.
    if !a.act.lock().unwrap().is_empty() {
        return false;
    }
    if !a.lk.lock().unwrap().is_empty() {
        return false;
    }
    if !a.merge_in_progress.lock().unwrap().is_empty() {
        return false;
    }
    a.db.n(
        "select count(*) from jobs where st in ('hunt','repro','fix','ci','review','gate','submit')",
        crate::pa![],
    ) == 0
}

pub fn manually_paused(a: &Ax) -> bool {
    a.lv.lock().unwrap().o.man
}

pub fn spawn(a: Ax) {
    // Start the watcher whenever a trusted channel exists: the user's
    // automatic-updates switch may be enabled later without restarting.
    if !configured() {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            if on(&a) {
                let _ = check(&a).await;
            }
            for _ in 0..(6 * 60) {
                tokio::time::sleep(Duration::from_secs(60)).await;
                if on(&a) && ready(&a) && writable(&a.me) {
                    if !a.drain.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        a.say(
                            "submit",
                            "update ready: finishing running jobs, then restarting",
                            None,
                        );
                    }
                    if idle(&a) && apply(&a) {
                        return;
                    }
                }
            }
        }
    });
}

pub fn apply(a: &Ax) -> bool {
    if !ready(a) || !idle(a) {
        return false;
    }
    // Prevent workers from starting between the idle check and shutdown.
    a.drain.store(true, std::sync::atomic::Ordering::SeqCst);
    if !idle(a) {
        a.drain.store(false, std::sync::atomic::Ordering::SeqCst);
        return false;
    }
    let v = a.db.kg("upd")["ready"].as_str().unwrap_or("").to_string();
    a.say(
        "submit",
        &format!("installing update {} -> {v}, restarting", crate::app::V),
        None,
    );
    *a.restart.lock().unwrap() = true;
    a.quit.notify_waiters();
    true
}

pub fn install(a: &Ax) -> std::io::Result<()> {
    if !ready(a) {
        return Err(std::io::Error::other(
            "No trusted, verified fork update is ready",
        ));
    }
    let sha = a.db.kg("upd")["sha"].as_str().unwrap_or("").to_string();
    let me = &a.me;
    let f0 = staged(a);
    let bin = std::fs::read(&f0)?;
    let _ = std::fs::remove_file(&f0);
    if sha.len() != 64 || hex::encode(Sha256::digest(&bin)) != sha {
        return Err(std::io::Error::other(
            "staged update does not match the signed checksum",
        ));
    }
    let f = me.with_extension(if cfg!(windows) { "upd.exe" } else { "upd" });
    let _ = std::fs::remove_file(&f);
    std::fs::write(&f, &bin)?;
    if cfg!(windows) {
        let old = me.with_extension("old.exe");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(me, &old)?;
        if let Err(e) = std::fs::rename(&f, me) {
            let _ = std::fs::rename(&old, me);
            return Err(e);
        }
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755))?;
            // Preserve the previous executable for recovery if the new one
            // fails to exec or crashes after replacing the running image.
            std::fs::copy(me, me.with_extension("old"))?;
        }
        std::fs::rename(&f, me)?;
    }
    Ok(())
}

pub fn relaunch(me: &Path) -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = std::process::Command::new(me).args(&args).exec();
        eprintln!("restart failed: {e}");
        let old = me.with_extension("old");
        if old.exists() && std::fs::rename(&old, me).is_ok() {
            eprintln!("restored previous Apollo Office executable");
            let fallback = std::process::Command::new(me).args(&args).exec();
            eprintln!("rollback restart failed: {fallback}");
        }
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new(me).args(&args).spawn();
        std::process::exit(0);
    }
}

pub fn cleanup(me: &Path) {
    if cfg!(windows) {
        let _ = std::fs::remove_file(me.with_extension("old.exe"));
    }
}

#[cfg(test)]
mod t {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn versions() {
        assert!(newer("0.5.0", "0.4.0"));
        assert!(newer("v0.10.0", "0.9.9"));
        assert!(!newer("0.4.0", "0.4.0"));
        assert!(newer("0.5.0", "0.5.0-rc1"));
        assert!(!newer("0.5.0-rc1", "0.5.0"));
    }

    #[test]
    fn running_work_or_merge_blocks_update() {
        let db = crate::db::Db::mem();
        let a = std::sync::Arc::new(crate::app::App::new(
            db, PathBuf::from("/tmp/apollo-upd-test"), 47300,
            PathBuf::from("/tmp/apollo-upd-test/apollo-office"),
        ));
        assert!(idle(&a));
        assert!(!manually_paused(&a));
        a.db.run("insert into jobs(st) values('fix')", crate::pa![]);
        assert!(!idle(&a));
        a.db.run("delete from jobs", crate::pa![]);
        a.merge_in_progress.lock().unwrap().insert("test".into());
        assert!(!idle(&a));
        a.merge_in_progress.lock().unwrap().clear();
        a.act.lock().unwrap().insert("test".into());
        assert!(!idle(&a));
        a.act.lock().unwrap().clear();
        a.lv.lock().unwrap().o.man = true;
        assert!(manually_paused(&a));
        assert!(idle(&a));
    }

    #[test]
    fn manifest() {
        let sk = SigningKey::from_bytes(&[5u8; 32]);
        let pk = hex::encode(sk.verifying_key().to_bytes());
        let body = br#"{"v":"0.5.0","files":{}}"#;
        let sig = base64::engine::general_purpose::STANDARD.encode(sk.sign(body).to_bytes());
        assert_eq!(check_sig(&pk, body, &sig).unwrap()["v"], "0.5.0");
        assert!(check_sig(&pk, br#"{"v":"9.9.9","files":{}}"#, &sig).is_err());
        assert!(check_sig(&hex::encode([0u8; 32]), body, &sig).is_err());
    }

    #[test]
    fn fork_channel_fails_closed_until_url_and_signer_are_explicit() {
        let good = hex::encode(
            SigningKey::from_bytes(&[5u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        assert!(trusted_channel_inputs(None, None).is_err());
        assert!(trusted_channel_inputs(Some("https://fork.example/releases"), None).is_err());
        assert!(trusted_channel_inputs(Some("http://fork.example/releases"), Some(&good)).is_err());
        assert!(trusted_channel_inputs(Some("https://fork.example?token=x"), Some(&good)).is_err());
        assert!(
            trusted_channel_inputs(Some("https://fork.example/releases"), Some("bad")).is_err()
        );
        let (base, key) =
            trusted_channel_inputs(Some("https://fork.example/releases/"), Some(&good)).unwrap();
        assert_eq!(base, "https://fork.example/releases");
        assert_eq!(key, good);
        assert_ne!(
            channel_id(&base, &key),
            channel_id("https://other.example", &key)
        );
    }
}
