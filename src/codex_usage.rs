//! Leitura exclusiva dos limites da sessão local do Codex CLI.
//! Não acede a auth.json, não recebe tokens e não persiste dados de conta.
use crate::app::Ax;
use crate::db::now;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const CACHE_MS: i64 = 120_000;
const ERROR_CACHE_MS: i64 = 20_000;
const MAX_LINES: usize = 32;
static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn window(data: &Value) -> Option<Value> {
    let duration = data["windowDurationMins"].as_u64()?;
    if duration != 300 && duration != 10_080 {
        return None;
    }
    let used = data["usedPercent"].as_f64()?;
    if !used.is_finite() || !(0.0..=100.0).contains(&used) {
        return None;
    }
    let reset = data["resetsAt"].as_i64()?;
    if !(1_000_000_000..=99_999_999_999).contains(&reset) {
        return None;
    }
    Some(json!({
        "remainingPercent": (100.0 - used),
        "usedPercent": used,
        "resetsAt": reset,
    }))
}

/// Identifica as janelas pela duração, nunca pela posição primary/secondary.
/// Usa apenas o bucket Codex, sem misturar outras quotas ou créditos.
fn normalize(raw: &Value) -> Value {
    let bucket = raw["rateLimitsByLimitId"]["codex"]
        .as_object()
        .map(|_| &raw["rateLimitsByLimitId"]["codex"])
        .unwrap_or(&raw["rateLimits"]);
    let mut five = Value::Null;
    let mut weekly = Value::Null;
    for name in ["primary", "secondary"] {
        let w = &bucket[name];
        let Some(valid) = window(w) else {
            continue;
        };
        match w["windowDurationMins"].as_u64() {
            Some(300) => five = valid,
            Some(10_080) => weekly = valid,
            _ => {}
        }
    }
    json!({
        "status": if five.is_null() && weekly.is_null() { "unavailable" } else { "ok" },
        "fiveHour": five,
        "weekly": weekly
    })
}

async fn until_id(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: i64,
) -> Result<Value, &'static str> {
    for _ in 0..MAX_LINES {
        let line = lines
            .next_line()
            .await
            .map_err(|_| "erro ao ler resposta do Codex")?
            .ok_or("Codex terminou sem responder")?;
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if msg["id"].as_i64() != Some(id) {
            continue;
        }
        if !msg["error"].is_null() {
            return Err("a sessão Codex não está autenticada ou o serviço recusou a consulta");
        }
        return msg
            .get("result")
            .cloned()
            .ok_or("resposta inválida do Codex");
    }
    Err("resposta do Codex não encontrada")
}

async fn query() -> Result<Value, &'static str> {
    query_with(&crate::sh::tool("codex")).await
}

async fn query_with(binary: &str) -> Result<Value, &'static str> {
    let mut proc = crate::sh::cmd(binary, &["app-server".to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Codex CLI indisponível neste utilizador")?;
    let mut input = proc.stdin.take().ok_or("stdin indisponível")?;
    let output = proc.stdout.take().ok_or("stdout indisponível")?;
    let mut lines = BufReader::new(output).lines();
    let init = json!({
        "method":"initialize","id":1,
        "params":{"clientInfo":{
            "name":"apollo_office","title":"Apollo Office (local)",
            "version":env!("CARGO_PKG_VERSION")
        }}
    });
    input
        .write_all(format!("{init}\n").as_bytes())
        .await
        .map_err(|_| "falha na inicialização Codex")?;
    until_id(&mut lines, 1).await?;
    input
        .write_all(
            b"{\"method\":\"initialized\"}\n{\"method\":\"account/rateLimits/read\",\"id\":2}\n",
        )
        .await
        .map_err(|_| "falha na consulta Codex")?;
    let result = until_id(&mut lines, 2).await?;
    // Fechar o processo de consulta; nunca manter sessões de agente abertas.
    let _ = proc.start_kill();
    let _ = proc.wait().await;
    Ok(normalize(&result))
}

fn cached(a: &Ax) -> Option<Value> {
    let guard = a.cache.lock().ok()?;
    guard
        .get("codex:rate-limits")
        .filter(|(t, value)| {
            let lifetime = if value["status"] == "ok" {
                CACHE_MS
            } else {
                ERROR_CACHE_MS
            };
            now().saturating_sub(*t) < lifetime
        })
        .map(|(_, v)| v.clone())
}

fn with_server_time(mut value: Value) -> Value {
    value["serverTime"] = json!(now());
    value
}

/// Só expõe percentagens, instantes de reposição e estado operacional.
pub async fn read(a: &Ax) -> Value {
    if let Some(v) = cached(a) {
        return with_server_time(v);
    }
    let _guard = REFRESH.lock().await;
    if let Some(v) = cached(a) {
        return with_server_time(v);
    }
    let data = match tokio::time::timeout(Duration::from_secs(10), query()).await {
        Ok(Ok(value)) => value,
        Ok(Err(message)) => {
            json!({"status":"unavailable","fiveHour":null,"weekly":null,"message":message})
        }
        Err(_) => {
            json!({"status":"unavailable","fiveHour":null,"weekly":null,"message":"consulta Codex excedeu o tempo disponível"})
        }
    };
    let mut value = data;
    value["checkedAt"] = json!(now());
    if let Ok(mut cache) = a.cache.lock() {
        cache.insert("codex:rate-limits".into(), (now(), value.clone()));
    }
    with_server_time(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn handshake_reads_local_app_server_response_without_auth_file() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "apollo-codex-usage-{:016x}.sh",
            rand::random::<u64>()
        ));
        let script = r#"#!/bin/sh
IFS= read -r first || exit 2
printf '%s\n' '{"id":1,"result":{}}'
IFS= read -r initialized || exit 3
IFS= read -r request || exit 4
case "$request" in *account/rateLimits/read*) ;; *) exit 5;; esac
printf '%s\n' '{"id":2,"result":{"rateLimits":{"primary":{"usedPercent":12,"windowDurationMins":300,"resetsAt":1794350000},"secondary":{"usedPercent":41,"windowDurationMins":10080,"resetsAt":1794500000}}}}'
"#;
        std::fs::write(&path, script).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).unwrap();
        let result =
            tokio::time::timeout(Duration::from_secs(4), query_with(path.to_str().unwrap()))
                .await
                .expect("o app-server local não deve bloquear")
                .expect("resposta de app-server válida");
        let _ = std::fs::remove_file(path);
        assert_eq!(result["fiveHour"]["remainingPercent"], 88.0);
        assert_eq!(result["weekly"]["remainingPercent"], 59.0);
    }

    #[test]
    fn maps_real_windows_regardless_of_primary_order() {
        let r = normalize(&json!({"rateLimits":{
            "primary":{"usedPercent":41,"windowDurationMins":10080,"resetsAt":1794500000},
            "secondary":{"usedPercent":12,"windowDurationMins":300,"resetsAt":1794350000}
        }}));
        assert_eq!(r["status"], "ok");
        assert_eq!(r["fiveHour"]["remainingPercent"], 88.0);
        assert_eq!(r["weekly"]["remainingPercent"], 59.0);
        assert_eq!(r["fiveHour"]["resetsAt"], 1794350000_i64);
    }

    #[test]
    fn uses_codex_bucket_not_other_meters() {
        let r = normalize(
            &json!({"rateLimits":{"primary":{"usedPercent":95,"windowDurationMins":300,"resetsAt":1794350000}},
                "rateLimitsByLimitId":{
                    "codex":{"primary":{"usedPercent":35,"windowDurationMins":300,"resetsAt":1794350000}},
                    "codex_other":{"primary":{"usedPercent":80,"windowDurationMins":10080,"resetsAt":1794500000}}
                }
            }),
        );
        assert_eq!(r["fiveHour"]["remainingPercent"], 65.0);
        assert!(r["weekly"].is_null());
    }

    #[test]
    fn failures_expire_earlier_than_valid_results() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            47302,
            "apollo-office".into(),
        ));
        a.cache.lock().unwrap().insert(
            "codex:rate-limits".into(),
            (now() - ERROR_CACHE_MS - 100, json!({"status":"unavailable"})),
        );
        assert!(cached(&a).is_none());
        a.cache.lock().unwrap().insert(
            "codex:rate-limits".into(),
            (now() - 40_000, json!({"status":"ok"})),
        );
        assert!(cached(&a).is_some());
    }

    #[test]
    fn never_invents_missing_or_invalid_limits() {
        let r = normalize(&json!({"rateLimits":{
            "primary":{"usedPercent":120,"windowDurationMins":300,"resetsAt":1794350000},
            "secondary":{"usedPercent":34,"windowDurationMins":60,"resetsAt":1794500000}
        }}));
        assert_eq!(r["status"], "unavailable");
        assert!(r["fiveHour"].is_null());
        assert!(r["weekly"].is_null());
        let r = normalize(
            &json!({"rateLimits":{"primary":{"usedPercent":0,"windowDurationMins":10080,"resetsAt":1794500000}}}),
        );
        assert_eq!(r["weekly"]["remainingPercent"], 100.0);
        assert!(r["fiveHour"].is_null());
    }
}
