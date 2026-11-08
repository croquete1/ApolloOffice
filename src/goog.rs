use crate::app::Ax;
use crate::db::now;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

// This fork must use an explicitly configured OAuth client. Client IDs are
// public, but embedding an upstream client ID or secret would bind users to
// credentials they cannot rotate in their own Google Cloud project.
fn cfg() -> (String, String) {
    let value = |name| std::env::var(name)
        .ok().map(|v| v.trim().to_string()).unwrap_or_default();
    (value("APOLLO_OFFICE_GOOGLE_ID"), value("APOLLO_OFFICE_GOOGLE_SECRET"))
}

pub fn available() -> bool {
    let (id, secret) = cfg();
    !id.is_empty() && !secret.is_empty()
}

fn ue(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn redir(a: &Ax) -> String {
    format!("http://127.0.0.1:{}/auth/google/cb", a.port)
}

pub fn linked(a: &Ax) -> Value {
    a.db.kg("google")
}

pub fn start(a: &Ax, mode: &str) -> String {
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let st = hex::encode(rand::random::<[u8; 16]>());
    let ve = b64.encode(rand::random::<[u8; 32]>());
    let ch = b64.encode(Sha256::digest(ve.as_bytes()));
    {
        let mut m = a.oa.lock().unwrap();
        m.retain(|_, v| v.2 > now());
        m.insert(st.clone(), (ve, mode.to_string(), now() + 600_000));
    }
    let (id, _) = cfg();
    let mut u = format!("https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&state={st}&code_challenge={ch}&code_challenge_method=S256&prompt=select_account", ue(&id), ue(&redir(a)), ue("openid email"));
    if mode == "login" {
        if let Some(e) = linked(a)["email"].as_str() {
            u.push_str(&format!("&login_hint={}", ue(e)));
        }
    }
    u
}

pub async fn back(a: &Ax, code: &str, st: &str) -> Result<(String, String, String), String> {
    let Some((ve, mode, exp)) = a.oa.lock().unwrap().remove(st) else { return Err("this sign-in attempt is unknown or was already used".into()) };
    if exp < now() {
        return Err("this sign-in attempt expired, start again".into());
    }
    let (id, sec) = cfg();
    if id.is_empty() || sec.is_empty() {
        return Err("Google sign-in requires a private OAuth client ID and secret from environment; rotate the previously published secret".into());
    }
    let body = format!("code={}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code&code_verifier={}", ue(code), ue(&id), ue(&sec), ue(&redir(a)), ue(&ve));
    let r = a.hc.post("https://oauth2.googleapis.com/token").timeout(Duration::from_secs(20)).header("content-type", "application/x-www-form-urlencoded").body(body).send().await.map_err(|e| format!("Google did not answer: {e}"))?;
    let t: Value = r.json().await.map_err(|e| e.to_string())?;
    let Some(at) = t["access_token"].as_str() else { return Err(format!("Google refused the sign-in: {}", t["error_description"].as_str().or(t["error"].as_str()).unwrap_or("unknown error"))) };
    let u: Value = a.hc.get("https://openidconnect.googleapis.com/v1/userinfo").bearer_auth(at).timeout(Duration::from_secs(20)).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let sub = u["sub"].as_str().unwrap_or("").to_string();
    let em = u["email"].as_str().unwrap_or("").to_string();
    if sub.is_empty() || u["email_verified"].as_bool() != Some(true) {
        return Err("Google did not return a verified account".into());
    }
    Ok((mode, sub, em))
}

pub fn link(a: &Ax, sub: &str, em: &str) {
    a.db.ks("google", &json!({"sub": sub, "email": em, "t": now()}));
}

pub fn ok(a: &Ax, sub: &str) -> bool {
    linked(a)["sub"].as_str() == Some(sub)
}

#[cfg(test)]
mod t {
    #[test]
    fn enc() {
        assert_eq!(super::ue("openid email"), "openid%20email");
        assert_eq!(super::ue("http://127.0.0.1:1/a"), "http%3A%2F%2F127.0.0.1%3A1%2Fa");
    }
}
