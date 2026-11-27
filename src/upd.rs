use crate::app::Ax;
use crate::db::now;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

// This customized fork must never silently install releases signed for upstream.
// Operators may opt in to a separately signed, HTTPS-only fork release channel.

/// Selects the release asset for the current target operating system.
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
const MAX_UPDATE_BYTES: usize = 128 * 1024 * 1024;

fn release_version(value: &str) -> Result<semver::Version, semver::Error> {
    semver::Version::parse(value.strip_prefix('v').unwrap_or(value))
}

/// Rejects malformed versions and compares SemVer precedence, not build metadata.
pub fn newer(a: &str, b: &str) -> bool {
    match (release_version(a), release_version(b)) {
        (Ok(a), Ok(b)) => a.cmp_precedence(&b).is_gt(),
        _ => false,
    }
}

fn manifest_version(manifest: &Value) -> Result<String, String> {
    let version = manifest["v"]
        .as_str()
        .ok_or("signed manifest has no release version")?;
    release_version(version).map_err(|_| "invalid release version in signed manifest")?;
    Ok(version.to_string())
}

/// Confirms that the executable directory allows atomic replacement.
fn writable(me: &Path) -> bool {
    let Some(d) = me.parent() else { return false };
    let f = d.join(format!(".apollo-office-w{}", std::process::id()));
    let ok = std::fs::write(&f, b"").is_ok();
    let _ = std::fs::remove_file(&f);
    ok
}

/// Verifies the exact manifest bytes against the pinned fork signing key.
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

fn persist_staged_receipt(db: &crate::db::Db, receipt: &Value) -> rusqlite::Result<()> {
    use rusqlite::OptionalExtension;
    let mut connection = db.0.lock().unwrap();
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let raw: Option<String> = tx
        .query_row("select v from kv where k='upd'", [], |row| row.get(0))
        .optional()?;
    let mut state = match raw {
        Some(raw) => {
            serde_json::from_str::<Value>(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?
        }
        None => json!({}),
    };
    let fields = state.as_object_mut().ok_or(rusqlite::Error::InvalidQuery)?;
    let proof = receipt.as_object().ok_or(rusqlite::Error::InvalidQuery)?;
    fields.extend(proof.clone());
    tx.execute(
        "insert into kv(k,v) values('upd',?) on conflict(k) do update set v=excluded.v",
        [state.to_string()],
    )?;
    tx.commit()
}

/// Rejects update channels without an explicit HTTPS origin and Ed25519 key.
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
    // The signed staging proof is internal, not dashboard state.
    if let Some(fields) = v.as_object_mut() {
        fields.remove("manifest_body");
        fields.remove("manifest_sig");
    }
    v["configured"] = json!(configured());
    v["auto"] = json!(on(a));
    v
}

/// Compares the GitHub source branch with the latest published GitHub fork
/// release. This is informational only: no code download or executable stage.
fn main_comparison(main: &Value, release: &Value, compare: &Value) -> Result<Value, String> {
    let head = main["sha"]
        .as_str()
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("GitHub devolveu um SHA inválido para a main")?;
    let tag = release["tag"]
        .as_str()
        .filter(|tag| {
            tag.strip_prefix("fork-v").is_some_and(|version| {
                version.split('.').count() == 3
                    && version
                        .split('.')
                        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            })
        })
        .ok_or("A última release GitHub não pertence ao canal de versões deste fork")?;
    let ahead = compare["ahead_by"]
        .as_u64()
        .filter(|v| *v <= 100_000)
        .ok_or("Comparação GitHub sem contagem válida")?;
    let behind = compare["behind_by"]
        .as_u64()
        .filter(|v| *v <= 100_000)
        .ok_or("Comparação GitHub sem contagem válida")?;
    let status = compare["status"]
        .as_str()
        .filter(|s| matches!(*s, "ahead" | "behind" | "identical" | "diverged"))
        .ok_or("Comparação GitHub sem estado válido")?;
    Ok(json!({
        "sha": head,
        "tag": tag,
        "ahead": ahead,
        "behind": behind,
        "status": status,
        "source": "croquete1/ApolloOffice:main",
        "installable": false
    }))
}

/// Checks the code on main separately from the signed binary release channel.
/// Never presents a source commit as a verified, installable binary.
pub async fn check_main() -> Result<Value, String> {
    let main = crate::sh::ghj(
        &[
            "api",
            "repos/croquete1/ApolloOffice/branches/main",
            "--jq",
            "{sha: .commit.sha}",
        ],
        45,
    )
    .await?;
    let release = crate::sh::ghj(
        &[
            "api",
            "repos/croquete1/ApolloOffice/releases/latest",
            "--jq",
            "{tag: .tag_name}",
        ],
        45,
    )
    .await?;
    let tag = release["tag"]
        .as_str()
        .ok_or("A release GitHub não tem tag")?;
    if !tag.starts_with("fork-v")
        || tag["fork-v".len()..].split('.').count() != 3
        || !tag["fork-v".len()..]
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err("Tag da release GitHub inesperada".into());
    }
    let head = main["sha"]
        .as_str()
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("GitHub devolveu um SHA inválido para a main")?;
    // Pin comparison to the validated commit. The compare response has no
    // `head_commit` field, and a moving branch could otherwise race this check.
    let endpoint = format!("repos/croquete1/ApolloOffice/compare/{tag}...{head}");
    let compare = crate::sh::ghj(
        &[
            "api",
            &endpoint,
            "--jq",
            "{status: .status, ahead_by: .ahead_by, behind_by: .behind_by}",
        ],
        60,
    )
    .await?;
    main_comparison(&main, &release, &compare)
}

/// Downloads a bounded asset, including when Content-Length is absent.
async fn get(a: &Ax, u: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut r =
        a.hc.get(u)
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

fn write_synced_update(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Stages only a signed, integrity-checked update for this installation.
pub async fn check(a: &Ax) -> Result<Value, String> {
    let _g = CK.lock().await;
    // No network access or staged binary mutation without an explicit
    // independently signed channel for this fork.
    let (b, key) = channel()?;
    let cid = channel_id(&b, &key);
    let r: Result<Value, String> = async {
        let body = get(a, &format!("{b}/latest.json"), 128 * 1024).await?;
        let sig_bytes = get(a, &format!("{b}/latest.json.sig"), 4 * 1024).await?;
        let sig = String::from_utf8_lossy(&sig_bytes).to_string();
        let m = check_sig(&key, &body, &sig)?;
        let v = manifest_version(&m)?;
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
        let bin = get(a, &format!("{b}/{}", asset()), MAX_UPDATE_BYTES).await?;
        if hex::encode(Sha256::digest(&bin)) != want {
            return Err("downloaded update does not match the signed checksum".into());
        }
        let f = staged(a);
        let tmp = f.with_extension("part");
        write_synced_update(&tmp, &bin).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &f).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        std::fs::File::open(&a.dd)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())?;
        let receipt = json!({
            "ready":v,"sha":want,"channel":cid,
            "manifest_body":base64::engine::general_purpose::STANDARD.encode(&body),
            "manifest_sig":sig
        });
        persist_staged_receipt(&a.db, &receipt).map_err(|e| e.to_string())?;
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

// A cached checksum is not itself an authenticated update manifest.
// Legacy staging records without signed proof must be downloaded again.
fn staged_checksum(
    state: &Value,
    url: &str,
    key: &str,
    current: &str,
    target: &str,
) -> Result<String, String> {
    if state["channel"].as_str() != Some(channel_id(url, key).as_str()) {
        return Err("staged update belongs to another channel".into());
    }
    let encoded = state["manifest_body"]
        .as_str()
        .filter(|s| s.len() <= 4 * (128 * 1024_usize).div_ceil(3))
        .ok_or("staged signed manifest missing or oversized")?;
    let signature = state["manifest_sig"]
        .as_str()
        .filter(|s| s.len() <= 4 * 1024)
        .ok_or("staged manifest signature missing or oversized")?;
    let body = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "invalid staged manifest encoding")?;
    if body.len() > 128 * 1024 {
        return Err("staged manifest exceeds size limit".into());
    }
    let manifest = check_sig(key, &body, signature)?;
    let version = manifest_version(&manifest)?;
    if state["ready"].as_str() != Some(version.as_str()) || !newer(&version, current) {
        return Err("staged manifest version does not authorize this update".into());
    }
    let checksum = manifest["files"][target]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("missing or invalid signed update checksum")?;
    if state["sha"].as_str() != Some(checksum) {
        return Err("cached checksum differs from signed manifest".into());
    }
    Ok(checksum.to_string())
}

fn valid_staged_size(size: u64) -> bool {
    (1..=MAX_UPDATE_BYTES as u64).contains(&size)
}

fn staged_file_matches(path: &Path, checksum: &str) -> std::io::Result<bool> {
    let meta = path.metadata()?;
    if !meta.is_file() || !valid_staged_size(meta.len()) {
        return Ok(false);
    }
    let mut reader = std::fs::File::open(path)?.take(MAX_UPDATE_BYTES as u64 + 1);
    let mut digest = Sha256::new();
    let mut buf = [0u8; 8192];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        digest.update(&buf[..n]);
    }
    Ok(valid_staged_size(total) && hex::encode(digest.finalize()) == checksum)
}

/// Checks that the staged update is still authorized by the trusted signer.
pub fn ready(a: &Ax) -> bool {
    let Ok((url, key)) = channel() else {
        return false;
    };
    let Ok(checksum) = staged_checksum(&a.db.kg("upd"), &url, &key, crate::app::V, asset()) else {
        return false;
    };
    staged_file_matches(&staged(a), &checksum).unwrap_or(false)
}

/// Reports whether no admitted job or merge is still running.
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
    if !a.vtk.lock().unwrap().is_empty() {
        return false;
    }
    if !a.cache_maintenance.lock().unwrap().is_empty() {
        return false;
    }
    // A queued 'submit' row may be retried after a machine error. Only
    // admitted work (act/lk) can prevent shutdown in that state.
    // A failed inventory read is not proof that the office is idle.
    matches!(
        a.db.try_n(
            "select count(*) from jobs where st in ('hunt','repro','fix','ci','review','gate')",
            crate::pa![],
        ),
        Ok(0)
    )
}

/// Requires the explicit Pause office switch for manual installation.
pub fn manually_paused(a: &Ax) -> bool {
    a.lv.lock().unwrap().o.man
}

/// Watches the signed channel and drains jobs before a safe idle restart.
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
                if !on(&a) || !ready(&a) || !writable(&a.me) {
                    cancel_watcher_drain(&a);
                    continue;
                }
                if request_drain(&a) {
                    a.say(
                        "submit",
                        "update ready: finishing running jobs, then restarting",
                        None,
                    );
                }
                if apply(&a, false) {
                    return;
                }
            }
        }
    });
}

/// Reserves the update drain under the work-admission lock.
/// A failed manual request never clears a drain owned by the automatic watcher.
fn request_drain(a: &Ax) -> bool {
    let _transition = a.update_transition.lock().unwrap();
    !a.drain.swap(true, std::sync::atomic::Ordering::SeqCst)
}

/// Releases a watcher-owned drain when automatic updates are turned off.
/// A committed restart is never cancelled while shutdown is in progress.
fn cancel_watcher_drain(a: &Ax) {
    let _transition = a.update_transition.lock().unwrap();
    if !*a.restart.lock().unwrap() {
        a.drain.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Prevents new jobs and merges atomically with the final idle decision.
/// The manual path additionally requires an explicit operator pause.
fn reserve_install(a: &Ax, manual: bool) -> bool {
    let _transition = a.update_transition.lock().unwrap();
    if (manual && !manually_paused(a)) || !idle(a) {
        return false;
    }
    // Do not allow drain cancellation between claiming the update and shutdown.
    a.drain.store(true, std::sync::atomic::Ordering::SeqCst);
    *a.restart.lock().unwrap() = true;
    true
}

/// Only an authenticated, verified, idle update can request process replacement.
pub fn apply(a: &Ax, manual: bool) -> bool {
    if !ready(a) || !writable(&a.me) || !reserve_install(a, manual) {
        return false;
    }
    let v = a.db.kg("upd")["ready"].as_str().unwrap_or("").to_string();
    a.say(
        "submit",
        &format!("installing update {} -> {v}, restarting", crate::app::V),
        None,
    );
    a.quit.notify_waiters();
    true
}

fn read_bounded_update(reader: impl Read, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bin = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bin)?;
    if bin.is_empty() || bin.len() > limit {
        return Err(std::io::Error::other(
            "staged update size outside allowed bounds",
        ));
    }
    Ok(bin)
}

/// Installs the hash-verified staged binary, preserving the previous executable.
pub fn install(a: &Ax) -> std::io::Result<()> {
    let (url, key) = channel().map_err(std::io::Error::other)?;
    let sha = staged_checksum(&a.db.kg("upd"), &url, &key, crate::app::V, asset())
        .map_err(std::io::Error::other)?;
    let me = &a.me;
    let f0 = staged(a);
    let bin = read_bounded_update(std::fs::File::open(&f0)?, MAX_UPDATE_BYTES)?;
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
            // Persist the replacement before making it the executable.
            std::fs::File::open(&f)?.sync_all()?;
            // Preserve the previous executable before replacing its directory
            // entry. An atomic rename alone does not make backup data durable.
            let dir = std::fs::File::open(
                me.parent()
                    .ok_or_else(|| std::io::Error::other("no executable directory"))?,
            )?;
            let backup_tmp = me.with_extension("old.part");
            std::fs::copy(me, &backup_tmp)?;
            std::fs::File::open(&backup_tmp)?.sync_all()?;
            std::fs::rename(&backup_tmp, me.with_extension("old"))?;
            dir.sync_all()?;
            std::fs::rename(&f, me)?;
            dir.sync_all()?;
        }
        #[cfg(not(unix))]
        std::fs::rename(&f, me)?;
    }
    let _ = std::fs::remove_file(&f0);
    Ok(())
}

/// Reexecutes the updated program; on Unix exec failure, restores the old binary.
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

/// Removes Windows restart leftovers after a successful new process launch.
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
    fn staged_write_syncs_the_writable_handle() {
        let dir = std::env::temp_dir().join(format!("apollo-sync-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        let temporary = dir.join("update.part");
        let staged = dir.join("update.bin");
        write_synced_update(&temporary, b"first candidate").unwrap();
        write_synced_update(&temporary, b"new").unwrap();
        std::fs::rename(&temporary, &staged).unwrap();
        assert_eq!(std::fs::read(&staged).unwrap(), b"new");
        assert!(!temporary.exists());
        assert!(write_synced_update(&dir, b"invalid destination").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn staging_receipt_reports_sqlite_write_failure() {
        let db = crate::db::Db::mem();
        db.ks("upd", &json!({"latest":"1.0.0","ready":"0.9.0"}));
        db.0.lock().unwrap().execute_batch(
            "create trigger reject_update_receipt before update on kv when new.k='upd' begin select raise(abort,'fixture'); end;"
        ).unwrap();
        let receipt = json!({"ready":"1.0.0","sha":"fixture"});
        assert!(persist_staged_receipt(&db, &receipt).is_err());
        assert_eq!(db.kg("upd")["ready"], "0.9.0");
        db.0.lock()
            .unwrap()
            .execute_batch("drop trigger reject_update_receipt")
            .unwrap();
        persist_staged_receipt(&db, &receipt).unwrap();
        assert_eq!(db.kg("upd")["ready"], "1.0.0");
        assert_eq!(db.kg("upd")["latest"], "1.0.0");
    }

    #[test]
    fn staged_update_reader_enforces_size_bound() {
        assert!(!valid_staged_size(0));
        assert!(valid_staged_size(1));
        assert!(valid_staged_size(MAX_UPDATE_BYTES as u64));
        assert!(!valid_staged_size(MAX_UPDATE_BYTES as u64 + 1));
        assert_eq!(read_bounded_update(&b"1234"[..], 4).unwrap(), b"1234");
        assert!(read_bounded_update(&b"12345"[..], 4).is_err());
        assert!(read_bounded_update(&b""[..], 4).is_err());
    }

    #[test]
    fn rejected_staging_stays_ineligible_until_valid_replacement() {
        let path = std::env::temp_dir().join(format!("apollo-stage-{}", rand::random::<u64>()));
        let checksum = hex::encode(Sha256::digest(b"valid"));
        std::fs::write(&path, b"wrong").unwrap();
        assert!(!staged_file_matches(&path, &checksum).unwrap());
        // A fresh eligibility check has no dependency on process-local state.
        assert!(!staged_file_matches(&path, &checksum).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"wrong");
        std::fs::write(&path, b"valid").unwrap();
        assert!(staged_file_matches(&path, &checksum).unwrap());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn source_main_is_not_a_signed_installable_update() {
        let main = json!({"sha": "0123456789abcdef0123456789abcdef01234567"});
        let release = json!({"tag": "fork-v0.10.3"});
        // Mirrors the GitHub compare response shape: it has no `head_commit`.
        let comparison: Value = serde_json::from_str(
            r#"{"base_commit":{"sha":"abcdef0123456789abcdef0123456789abcdef01"},"merge_base_commit":{"sha":"abcdef0123456789abcdef0123456789abcdef01"},"status":"ahead","ahead_by":2,"behind_by":0,"total_commits":2,"commits":[]}"#,
        )
        .unwrap();
        assert!(comparison.get("head_commit").is_none());
        let status = main_comparison(&main, &release, &comparison).unwrap();
        assert_eq!(status["ahead"], 2);
        assert_eq!(status["sha"], main["sha"]);
        assert_eq!(status["tag"], "fork-v0.10.3");
        assert_eq!(status["installable"], false);
        assert!(main_comparison(&json!({"sha":"wrong"}), &release, &comparison).is_err());
        assert!(main_comparison(&main, &json!({"tag":"v0.10.3"}), &comparison).is_err());
        assert!(main_comparison(
            &main,
            &release,
            &json!({
                "status":"diverged", "ahead_by":2, "behind_by":1
            })
        )
        .is_ok());
    }

    #[test]
    fn versions() {
        assert!(newer("0.5.0", "0.4.0"));
        assert!(newer("v0.10.0", "0.9.9"));
        assert!(!newer("0.4.0", "0.4.0"));
        assert!(newer("0.5.0", "0.5.0-rc1"));
        assert!(!newer("0.5.0-rc1", "0.5.0"));
        // A primeira atualização assinada do fork parte da release 0.10.1.
        assert!(newer("0.10.2", "0.10.1"));
        assert!(!newer("0.10.1", "0.10.2"));
    }

    #[test]
    fn malformed_versions_never_authorize_an_update() {
        for bad in [
            "",
            "999",
            "999.1",
            "999.bad.1",
            "999.1.1.2",
            "999.1.01",
            "vv999.1.1",
            "999.1.1-",
            "18446744073709551616.0.0",
        ] {
            assert!(!newer(bad, "0.10.15"), "{bad}");
            assert!(!newer("999.1.1", bad), "{bad}");
            assert!(manifest_version(&json!({"v": bad})).is_err(), "{bad}");
        }
        assert!(manifest_version(&json!({})).is_err());
        assert!(manifest_version(&json!({"v": 123})).is_err());
        assert_eq!(
            manifest_version(&json!({"v": "0.10.16"})).unwrap(),
            "0.10.16"
        );
    }

    #[test]
    fn version_precedence_orders_prereleases_and_ignores_build_metadata() {
        assert!(newer("0.10.16-rc.10", "0.10.16-rc.2"));
        assert!(!newer("0.10.16-rc.2", "0.10.16-rc.10"));
        assert!(newer("0.10.16", "0.10.16-rc.10"));
        assert!(!newer("0.10.16-rc.10", "0.10.16"));
        assert!(!newer("0.10.16+new", "0.10.16+old"));
        assert!(newer("0.10.17+build", "0.10.16"));
    }

    #[test]
    fn running_work_or_merge_blocks_update() {
        let db = crate::db::Db::mem();
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            PathBuf::from("/tmp/apollo-upd-test"),
            47300,
            PathBuf::from("/tmp/apollo-upd-test/apollo-office"),
        ));
        assert!(idle(&a));
        assert!(!manually_paused(&a));
        assert!(!apply(&a, true));
        assert!(!a.drain.load(std::sync::atomic::Ordering::SeqCst));
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
    fn maintenance_and_database_failure_block_idle_and_restart() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            PathBuf::from("/tmp/apollo-idle-failure-test"),
            47300,
            PathBuf::from("/tmp/apollo-idle-failure-test/apollo-office"),
        ));
        a.lv.lock().unwrap().o.man = true;
        assert!(idle(&a));
        a.cache_maintenance.lock().unwrap().insert("test".into());
        assert!(!idle(&a));
        assert!(!reserve_install(&a, true));
        a.cache_maintenance.lock().unwrap().clear();
        assert!(idle(&a));
        a.db.0
            .lock()
            .unwrap()
            .execute_batch("drop table jobs")
            .unwrap();
        assert!(!idle(&a));
        assert!(!reserve_install(&a, true));
        assert!(!a.drain.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!*a.restart.lock().unwrap());
    }

    #[test]
    fn manual_failure_never_clears_auto_drain() {
        let db = crate::db::Db::mem();
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            PathBuf::from("/tmp/apollo-drain-test"),
            47300,
            PathBuf::from("/tmp/apollo-drain-test/apollo-office"),
        ));
        a.db.run(
            "insert or ignore into rp(full,ci) values('sample/repo','true'),('other/repo','true')",
            crate::pa![],
        );
        a.db.run(
            "insert into jobs(id,repo,kind,st) values(42,'sample/repo','fix','ready')",
            crate::pa![],
        );
        assert!(crate::pipe::admit(&a, "sample/repo", "local", 42));
        assert!(request_drain(&a));
        assert!(!reserve_install(&a, false));
        assert!(a.drain.load(std::sync::atomic::Ordering::SeqCst));
        a.db.run(
            "insert into jobs(id,repo,kind,st) values(43,'other/repo','fix','ready')",
            crate::pa![],
        );
        assert!(!crate::pipe::admit(&a, "other/repo", "local", 43));
        a.act.lock().unwrap().clear();
        a.lk.lock().unwrap().clear();
        assert!(reserve_install(&a, false));
        assert!(a.drain.load(std::sync::atomic::Ordering::SeqCst));
        assert!(*a.restart.lock().unwrap());
        cancel_watcher_drain(&a);
        assert!(a.drain.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn disabling_automatic_updates_releases_uncommitted_drain() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            PathBuf::from("/tmp/apollo-cancel-test"),
            47300,
            PathBuf::from("/tmp/apollo-cancel-test/apollo-office"),
        ));
        assert!(request_drain(&a));
        cancel_watcher_drain(&a);
        assert!(!a.drain.load(std::sync::atomic::Ordering::SeqCst));
        a.db.run(
            "insert or ignore into rp(full,ci) values('sample/repo','true')",
            crate::pa![],
        );
        a.db.run(
            "insert into jobs(id,repo,kind,st) values(42,'sample/repo','fix','ready')",
            crate::pa![],
        );
        assert!(crate::pipe::admit(&a, "sample/repo", "local", 42));
    }

    #[test]
    fn queued_submit_does_not_block_update_when_not_admitted() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            PathBuf::from("/tmp/apollo-submit-test"),
            47300,
            PathBuf::from("/tmp/apollo-submit-test/apollo-office"),
        ));
        a.db.run("insert into jobs(st) values('submit')", crate::pa![]);
        assert!(idle(&a));
        assert!(reserve_install(&a, false));
        assert!(a.drain.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn admitted_vet_work_prevents_update_until_completion() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            PathBuf::from("/tmp/apollo-vet-test"),
            47300,
            PathBuf::from("/tmp/apollo-vet-test/apollo-office"),
        ));
        a.db.run("insert into rp(full) values('sample/repo')", crate::pa![]);
        a.db.run(
            "insert into jobs(id,repo,st) values(42,'sample/repo','vet'),(43,'sample/repo','drop'),(44,'missing/repo','vet')",
            crate::pa![],
        );
        assert!(!crate::wk::admit_vet(&a, 44));
        a.db.run("update rp set on_=0", crate::pa![]);
        assert!(!crate::wk::admit_vet(&a, 42));
        assert!(a.vtk.lock().unwrap().is_empty());
        a.db.run("update rp set on_=1", crate::pa![]);
        a.lv.lock().unwrap().o.man = true;
        assert!(!crate::wk::admit_vet(&a, 42));
        a.lv.lock().unwrap().o.man = false;
        assert!(!crate::wk::admit_vet(&a, 43));
        assert!(crate::wk::admit_vet(&a, 42));
        assert!(!idle(&a));
        assert!(!reserve_install(&a, false));
        a.vtk.lock().unwrap().remove(&42);
        assert!(reserve_install(&a, false));
        assert!(!crate::wk::admit_vet(&a, 43));
    }

    #[test]
    fn manual_update_requires_pause_and_blocks_admission() {
        let db = crate::db::Db::mem();
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            PathBuf::from("/tmp/apollo-manual-test"),
            47300,
            PathBuf::from("/tmp/apollo-manual-test/apollo-office"),
        ));
        assert!(!reserve_install(&a, true));
        assert!(!a.drain.load(std::sync::atomic::Ordering::SeqCst));
        a.lv.lock().unwrap().o.man = true;
        assert!(reserve_install(&a, true));
        assert!(!crate::pipe::admit(&a, "sample/repo", "local", 42));
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
    fn staged_update_requires_original_signed_manifest() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let key = hex::encode(sk.verifying_key().to_bytes());
        let url = "https://fork.example/releases";
        let target = asset();
        let checksum = hex::encode(Sha256::digest(b"fixture binary"));
        let mut manifest = json!({"v":"0.10.17","files":{}});
        manifest["files"][target] = json!(checksum);
        let body = serde_json::to_vec(&manifest).unwrap();
        let sig = base64::engine::general_purpose::STANDARD.encode(sk.sign(&body).to_bytes());
        let state = json!({
            "channel":channel_id(url, &key),
            "ready":"0.10.17",
            "sha":checksum,
            "manifest_body":base64::engine::general_purpose::STANDARD.encode(&body),
            "manifest_sig":sig
        });
        assert_eq!(
            staged_checksum(&state, url, &key, "0.10.16", target).unwrap(),
            checksum
        );
        for field in ["manifest_body", "manifest_sig", "sha", "ready", "channel"] {
            let mut missing = state.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(staged_checksum(&missing, url, &key, "0.10.16", target).is_err());
        }
        let mut forged = state.clone();
        forged["sha"] = json!("a".repeat(64));
        manifest["files"][target] = forged["sha"].clone();
        forged["manifest_body"] = json!(base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&manifest).unwrap()));
        assert!(staged_checksum(&forged, url, &key, "0.10.16", target).is_err());
        let mut mismatched = state.clone();
        mismatched["sha"] = json!("a".repeat(64));
        assert!(staged_checksum(&mismatched, url, &key, "0.10.16", target).is_err());
        assert!(staged_checksum(&state, url, &key, "0.10.17", target).is_err());
        assert!(staged_checksum(&state, url, &key, "0.10.16", "wrong-target").is_err());
        assert!(staged_checksum(&state, "https://other.example", &key, "0.10.16", target).is_err());
        let other = hex::encode(
            SigningKey::from_bytes(&[8u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        let mut wrong_key = state.clone();
        wrong_key["channel"] = json!(channel_id(url, &other));
        assert!(staged_checksum(&wrong_key, url, &other, "0.10.16", target).is_err());
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
