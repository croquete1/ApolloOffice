//! Leitura isolada dos limites Codex através do protocolo oficial app-server.
//! Não consulta nem exporta tokens, histórico de sessões ou credenciais.
use crate::app::Ax;
use crate::db::now;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const POLL_INTERVAL: Duration = Duration::from_secs(180);
const RPC_TIMEOUT: Duration = Duration::from_secs(15);
const FIVE_HOURS: u64 = 300;
const WEEK: u64 = 10_080;

/// Valida uma janela Codex pela duração e percentagem de utilização.
fn window(w: &Value, mins: u64) -> Option<Value> {
    if w["windowDurationMins"].as_u64()? != mins {
        return None;
    }
    let used = w["usedPercent"].as_f64()?;
    if !used.is_finite() || !(0.0..=100.0).contains(&used) {
        return None;
    }
    let resets_at = w["resetsAt"].as_i64().filter(|v| *v > 0);
    Some(json!({
        "remaining": (100.0 - used),
        "resets_at": resets_at,
    }))
}

/// Extrai apenas a quota principal Codex, sem confundir outros buckets.
fn parse_limits(response: &Value, at: i64) -> Result<Value, &'static str> {
    // Os outros limit_ids podem representar modelos ou funcionalidades
    // diferentes. Nunca apresentar esses limites como quota Codex geral.
    let by_id = &response["rateLimitsByLimitId"]["codex"];
    let limits = if by_id.is_object() {
        by_id
    } else {
        &response["rateLimits"]
    };
    if !limits.is_object() || limits["limitId"].as_str().is_some_and(|id| id != "codex") {
        return Err("Resposta Codex sem o limite principal");
    }
    let mut five_hour = None;
    let mut weekly = None;
    for key in ["primary", "secondary"] {
        let value = &limits[key];
        if let Some(v) = window(value, FIVE_HOURS) {
            five_hour = Some(v);
        } else if let Some(v) = window(value, WEEK) {
            weekly = Some(v);
        }
    }
    if five_hour.is_none() && weekly.is_none() {
        return Err("A conta não disponibilizou janelas reconhecidas");
    }
    Ok(json!({
        "status": "ok",
        "updated_at": at,
        "five_hour": five_hour,
        "weekly": weekly,
    }))
}

/// Escreve um pedido JSON-RPC e confirma que foi enviado.
async fn write_rpc(input: &mut tokio::process::ChildStdin, v: Value) -> Result<(), String> {
    input
        .write_all(v.to_string().as_bytes())
        .await
        .map_err(|_| "Não foi possível contactar Codex")?;
    input
        .write_all(b"\n")
        .await
        .map_err(|_| "Não foi possível contactar Codex")?;
    input
        .flush()
        .await
        .map_err(|_| "Não foi possível contactar Codex")?;
    Ok(())
}

/// Aguarda pelo ID RPC solicitado e ignora notificações intercalares.
async fn answer(
    output: &mut BufReader<tokio::process::ChildStdout>,
    id: i64,
) -> Result<Value, String> {
    // Notificações assíncronas podem chegar antes da resposta solicitada.
    // Limitar linhas recebidas para não consumir memória indefinidamente.
    for _ in 0..256 {
        // Limitar bytes ANTES de os acumular, mesmo num stream sem newline.
        let mut raw = Vec::new();
        let read = (&mut *output)
            .take(131_074)
            .read_until(b'\n', &mut raw)
            .await
            .map_err(|_| "Falha de leitura do Codex")?;
        if read == 0 {
            return Err("O Codex terminou a comunicação".into());
        }
        if raw.last() == Some(&b'\n') {
            raw.pop();
            if raw.last() == Some(&b'\r') {
                raw.pop();
            }
        }
        if raw.len() > 131_072 {
            return Err("Resposta Codex demasiado extensa".into());
        }
        let line =
            std::str::from_utf8(&raw).map_err(|_| "Resposta Codex não contém UTF-8 válido")?;
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value["id"] == id {
            if !value["error"].is_null() {
                return Err("Codex recusou a consulta de limites".into());
            }
            return value["result"]
                .as_object()
                .map(|_| value["result"].clone())
                .ok_or_else(|| "Resposta Codex inválida".to_string());
        }
    }
    Err("Demasiadas mensagens Codex antes da resposta".into())
}

/// Consulta a quota através de um subprocesso Codex de curta duração.
async fn request(bin: &str) -> Result<Value, String> {
    // Execução sem shell, exclusivamente leitura da quota. Não abrir sessões,
    // não iniciar turns e nunca aceder ao ficheiro auth.json.
    let mut child = Command::new(bin)
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Codex CLI não encontrado")?;
    let mut input = child.stdin.take().ok_or("Codex sem entrada standard")?;
    let output = child.stdout.take().ok_or("Codex sem saída standard")?;
    let mut output = BufReader::new(output);

    write_rpc(
        &mut input,
        json!({
            "id": 1,
            "method": "initialize",
            "params": {"clientInfo": {
                "name": "apollo_office",
                "title": "Apollo Office",
                "version": env!("CARGO_PKG_VERSION")
            }}
        }),
    )
    .await?;
    answer(&mut output, 1).await?;
    write_rpc(&mut input, json!({"method": "initialized", "params": {}})).await?;
    write_rpc(
        &mut input,
        json!({"id": 2, "method": "account/rateLimits/read"}),
    )
    .await?;
    let result = answer(&mut output, 2).await?;
    drop(input);
    // Encerrar e recolher o subprocesso após a leitura. O kill_on_drop
    // continua a proteger os caminhos de erro ou timeout.
    let _ = child.kill().await;
    Ok(result)
}

/// Executa a consulta oficial com timeout e validação da resposta.
async fn fetch() -> Result<Value, String> {
    let bin = crate::sh::tool("codex");
    let response = tokio::time::timeout(RPC_TIMEOUT, request(&bin))
        .await
        .map_err(|_| "Consulta Codex excedeu 15 segundos")??;
    parse_limits(&response, now()).map_err(str::to_string)
}

/// Atualiza a quota em segundo plano, sem bloquear o servidor HTTP.
pub async fn watch(a: Ax) {
    loop {
        let next = match fetch().await {
            Ok(mut value) => {
                crate::codex_forecast::update(&a.db, &mut value);
                value
            },
            Err(message) => {
                let previous = a
                    .codex_usage
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if previous["updated_at"].as_i64().is_some() {
                    let mut stale = previous;
                    stale["status"] = json!("stale");
                    stale["message"] = json!(message);
                    stale
                } else {
                    json!({"status":"unavailable","message":message})
                }
            }
        };
        *a.codex_usage.lock().unwrap_or_else(|e| e.into_inner()) = next;
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_codex_windows_by_duration_and_remaining() {
        let raw = json!({
            "rateLimits": {
                "limitId":"codex",
                "primary":{"usedPercent":12,"windowDurationMins":300,"resetsAt":1780000000},
                "secondary":{"usedPercent":41,"windowDurationMins":10080,"resetsAt":1780500000}
            }
        });
        let v = parse_limits(&raw, 111).unwrap();
        assert_eq!(v["five_hour"]["remaining"], 88.0);
        assert_eq!(v["weekly"]["remaining"], 59.0);
        assert_eq!(v["five_hour"]["resets_at"], 1780000000);
        assert_eq!(v["updated_at"], 111);
    }

    #[test]
    fn accepts_weekly_only_or_reversed_windows() {
        let weekly = json!({"rateLimits":{
            "limitId":"codex",
            "primary":{"usedPercent":19.5,"windowDurationMins":10080,"resetsAt":1780000000},
            "secondary":null
        }});
        let v = parse_limits(&weekly, 100).unwrap();
        assert!(v["five_hour"].is_null());
        assert_eq!(v["weekly"]["remaining"], 80.5);
        let reverse = json!({"rateLimits":{"limitId":"codex",
            "primary":{"usedPercent":60,"windowDurationMins":10080,"resetsAt":1780000000},
            "secondary":{"usedPercent":1,"windowDurationMins":300,"resetsAt":1780000200}
        }});
        let v = parse_limits(&reverse, 100).unwrap();
        assert_eq!(v["weekly"]["remaining"], 40.0);
        assert_eq!(v["five_hour"]["remaining"], 99.0);
    }

    #[test]
    fn prefers_explicit_codex_bucket_and_ignores_other_quota_types() {
        let raw = json!({
            "rateLimits":{"limitId":"codex_other",
                "primary":{"usedPercent":99,"windowDurationMins":300,"resetsAt":1780000000}},
            "rateLimitsByLimitId":{"codex":{"limitId":"codex",
                "primary":{"usedPercent":7,"windowDurationMins":300,"resetsAt":1780000000}}}
        });
        assert_eq!(
            parse_limits(&raw, 100).unwrap()["five_hour"]["remaining"],
            93.0
        );
        assert!(parse_limits(
            &json!({"rateLimits":{"limitId":"codex_other","primary":{
                "usedPercent":99,"windowDurationMins":300,"resetsAt":1780000000
            }}}),
            100
        )
        .is_err());
    }

    #[test]
    fn rejects_invalid_or_absent_values_without_inventing_remaining() {
        for used in [json!(-1), json!(101), json!("50"), Value::Null] {
            let v = json!({"rateLimits":{"limitId":"codex","primary":{
                "usedPercent":used,"windowDurationMins":300,"resetsAt":1780000000
            }}});
            assert!(parse_limits(&v, 100).is_err());
        }
        assert!(parse_limits(&json!({}), 100).is_err());
        assert!(parse_limits(
            &json!({"rateLimits":{"limitId":"codex",
                "primary":{"usedPercent":10,"windowDurationMins":60,"resetsAt":1780000000}
            }}),
            100
        )
        .is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rpc_handshake_is_read_only_and_returns_real_quota_fields() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "apollo-codex-usage-mock-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let script = r#"#!/bin/sh
[ "$1" = "app-server" ] || exit 4
IFS= read -r hello || exit 5
case "$hello" in *'"method":"initialize"'*) ;; *) exit 6;; esac
printf '%s\n' '{"id":1,"result":{}}'
IFS= read -r ready || exit 7
case "$ready" in *'"method":"initialized"'*) ;; *) exit 8;; esac
IFS= read -r query || exit 9
case "$query" in *'"method":"account/rateLimits/read"'*) ;; *) exit 10;; esac
printf '%s\n' '{"id":2,"result":{"rateLimits":{"limitId":"codex","primary":{"usedPercent":12,"windowDurationMins":300,"resetsAt":1780000000},"secondary":{"usedPercent":41,"windowDurationMins":10080,"resetsAt":1781000000}}}}'
"#;
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(4), request(path.to_str().unwrap()))
            .await
            .unwrap()
            .unwrap();
        let _ = std::fs::remove_file(&path);
        let parsed = parse_limits(&result, 123).unwrap();
        assert_eq!(parsed["five_hour"]["remaining"], 88.0);
        assert_eq!(parsed["weekly"]["remaining"], 59.0);
        assert_eq!(parsed["five_hour"]["resets_at"], 1780000000);
    }

    #[test]
    fn missing_reset_timestamp_does_not_create_a_fake_countdown() {
        let raw = json!({"rateLimits":{"limitId":"codex",
            "primary":{"usedPercent":25,"windowDurationMins":300}
        }});
        let v = parse_limits(&raw, 100).unwrap();
        assert_eq!(v["five_hour"]["remaining"], 75.0);
        assert!(v["five_hour"]["resets_at"].is_null());
    }
}
