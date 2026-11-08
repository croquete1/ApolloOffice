use crate::app::App;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;
use serde_json::json;
use std::path::Path;

const B: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const LOST: &str = "Stored API keys can not be decrypted. Start once with APOLLO_OFFICE_VAULT_RESET=1 to begin with a new vault, then enter the keys again.";

fn inst(dd: &Path) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(dd.display().to_string().as_bytes())[..8])
}

fn fresh() -> [u8; 32] {
    let mut k = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    k
}

pub fn key(dd: &Path, has: bool) -> Result<[u8; 32], String> {
    let f = dd.join("vault.key");
    let nofile = std::env::var("APOLLO_OFFICE_VAULT")
        .map(|v| v == "file")
        .unwrap_or(false);
    let reset = std::env::var("APOLLO_OFFICE_VAULT_RESET")
        .map(|v| v == "1")
        .unwrap_or(false);
    if let Ok(s) = std::fs::read_to_string(&f) {
        return B
            .decode(s.trim())
            .ok()
            .filter(|b| b.len() == 32)
            .map(|b| b.try_into().unwrap())
            .ok_or_else(|| format!("{} is broken. {LOST}", f.display()));
    }
    #[cfg(any(windows, target_os = "macos"))]
    if !nofile {
        let e =
            keyring::Entry::new("ApolloOffice", &inst(dd)).map_err(|e| format!("keychain: {e}"))?;
        match e.get_password() {
            Ok(s) => {
                if let Some(b) = B.decode(s.trim()).ok().filter(|b| b.len() == 32) {
                    return Ok(b.try_into().unwrap());
                }
                if !reset {
                    return Err(format!("the vault key in the keychain is broken. {LOST}"));
                }
            }
            Err(keyring::Error::NoEntry) => {
                if has && !reset {
                    return Err(format!("the vault key is missing from the keychain. {LOST}"));
                }
            }
            Err(x) => return Err(format!("the keychain could not be read ({x}). Unlock it or allow ApolloOffice access, then try again.")),
        }
        let k = fresh();
        if e.set_password(&B.encode(k)).is_ok()
            && e.get_password()
                .ok()
                .and_then(|s| B.decode(s.trim()).ok())
                .as_deref()
                == Some(&k[..])
        {
            return Ok(k);
        }
    }
    let _ = (nofile, inst(dd));
    if has && !reset {
        return Err(format!("{} is missing. {LOST}", f.display()));
    }
    let k = fresh();
    std::fs::write(&f, B.encode(k)).map_err(|e| e.to_string())?;
    tight(&f);
    Ok(k)
}

pub fn tight(f: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(windows)]
    {
        let u = std::env::var("USERNAME").unwrap_or_default();
        if !u.is_empty() {
            let _ = std::process::Command::new("icacls")
                .arg(f)
                .args(["/inheritance:r", "/grant:r"])
                .arg(format!("{u}:F"))
                .output();
        }
    }
}

pub fn enc(k: &[u8; 32], p: &str) -> Vec<u8> {
    let c = Aes256Gcm::new_from_slice(k).unwrap();
    let n: [u8; 12] = rand::random();
    let mut o = n.to_vec();
    o.extend(c.encrypt(Nonce::from_slice(&n), p.as_bytes()).unwrap());
    o
}

pub fn dec(k: &[u8; 32], b: &[u8]) -> Option<String> {
    if b.len() < 13 {
        return None;
    }
    let c = Aes256Gcm::new_from_slice(k).ok()?;
    let p = c.decrypt(Nonce::from_slice(&b[..12]), &b[12..]).ok()?;
    String::from_utf8(p).ok()
}

fn vk(a: &App) -> Result<[u8; 32], String> {
    let mut g = a.vk.lock().unwrap();
    if let Some(k) = *g {
        return Ok(k);
    }
    let has =
        a.db.n("select count(*) from kv where k like 'sec:%'", crate::pa![]) > 0;
    let k = key(&a.dd, has)?;
    *g = Some(k);
    Ok(k)
}

pub fn get(a: &App, n: &str) -> Result<String, String> {
    let v = a.db.kg(&format!("sec:{n}"));
    let Some(s) = v.as_str() else {
        return Ok(String::new());
    };
    let b = B.decode(s).map_err(|_| LOST.to_string())?;
    dec(&vk(a)?, &b).ok_or_else(|| LOST.to_string())
}

pub fn put(a: &App, n: &str, v: &str) -> Result<(), String> {
    if v.is_empty() {
        a.db.kd(&format!("sec:{n}"));
        return Ok(());
    }
    let k = vk(a)?;
    a.db.ks(&format!("sec:{n}"), &json!(B.encode(enc(&k, v))));
    Ok(())
}

pub fn has(a: &App, n: &str) -> bool {
    a.db.kg(&format!("sec:{n}")).is_string()
}

#[cfg(test)]
mod t {
    #[test]
    fn rt() {
        let k = [7u8; 32];
        let e = super::enc(&k, "sk-secret");
        assert_eq!(super::dec(&k, &e).unwrap(), "sk-secret");
        assert!(super::dec(&[8u8; 32], &e).is_none());
    }
}
