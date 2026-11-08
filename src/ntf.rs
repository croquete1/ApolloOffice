use crate::app::Ax;
use serde_json::json;

pub async fn send(a: &Ax, ti: &str, m: &str, p: u8) {
    let s = a.set();
    let n = s["ntfy"].as_str().unwrap_or("").trim().to_string();
    let d = s["discord"].as_str().unwrap_or("").trim().to_string();
    for k in 0..3 {
        let mut ok = true;
        if n.starts_with("https://") || n.starts_with("http://") {
            ok &=
                a.hc.post(&n)
                    .header("Title", ti.replace(|c: char| !c.is_ascii(), ""))
                    .header("Priority", p.to_string())
                    .body(m.to_string())
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
        }
        if d.starts_with("https://") {
            let c = crate::sh::head(&format!("**{ti}**\n{m}"), 1900);
            ok &=
                a.hc.post(&d)
                    .json(&json!({"content": c, "allowed_mentions": {"parse": []}}))
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
        }
        if ok {
            return;
        }
        crate::app::sl(30_000 * (k + 1)).await;
    }
}
