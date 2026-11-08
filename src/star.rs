use crate::app::Ax;
use crate::db::now;
use crate::pa;
use serde_json::json;
use std::time::Duration;

pub fn repo() -> String {
    std::env::var("APOLLO_OFFICE_STAR_REPO")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "Silvertree2010/ApolloOffice".into())
}

pub async fn check(a: &Ax) {
    let r = repo();
    let pb = crate::sh::gh(&["api", &format!("repos/{r}"), "--jq", ".visibility"], 30)
        .await
        .map(|v| v.trim() == "public")
        .unwrap_or(false);
    let has = crate::sh::gh(&["api", &format!("user/starred/{r}")], 30)
        .await
        .is_ok();
    a.db.ks("star", &json!({"pub": pb, "has": has, "t": now()}));
}

pub fn show(a: &Ax) -> bool {
    let s = a.db.kg("star");
    s["pub"].as_bool() == Some(true)
        && s["has"].as_bool() == Some(false)
        && a.db.kg("star_later").as_i64().unwrap_or(0) < now()
        && a.db.n("select count(*) from prs where st='merged'", pa![]) > 0
}

pub async fn star(a: &Ax) -> Result<(), String> {
    crate::sh::gh(
        &["api", "-X", "PUT", &format!("user/starred/{}", repo())],
        30,
    )
    .await?;
    let mut s = a.db.kg("star");
    s["has"] = json!(true);
    a.db.ks("star", &s);
    Ok(())
}

pub fn later(a: &Ax) {
    a.db.ks("star_later", &json!(now() + 86_400_000));
}

pub fn spawn(a: Ax) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            check(&a).await;
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}
