//! Projeção indicativa de esgotamento da quota semanal Codex.
//! Apenas observações de consumo e timestamps; nunca tokens ou credenciais.
use crate::db::Db;
use serde_json::{json, Value};

const KEY: &str = "codex_weekly_usage_history_v1";
const HOUR_MS: i64 = 3_600_000;
const WEEK_MS: i64 = 7 * 24 * HOUR_MS;
const TREND_MS: i64 = 24 * HOUR_MS;
const MIN_TREND_MS: i64 = 2 * HOUR_MS;
const SAVE_INTERVAL_MS: i64 = 15 * 60_000;
const MAX_SAMPLES: usize = 672;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    at: i64,
    used: f64,
}

fn valid_sample(v: &Value, now_ms: i64) -> Option<Sample> {
    let a = v.as_array()?;
    let at = a.first()?.as_i64()?;
    let used = a.get(1)?.as_f64()?;
    if at > now_ms || at <= now_ms.saturating_sub(WEEK_MS)
        || !used.is_finite() || !(0.0..=100.0).contains(&used)
    {
        return None;
    }
    Some(Sample { at, used })
}

fn note(status: &str, observed_hours: f64, exhaustion: Option<i64>) -> Value {
    json!({
        "status": status,
        "observed_hours": (observed_hours * 10.0).round() / 10.0,
        "projected_exhaust_at": exhaustion,
        "method": "recent_24h",
        "estimated": true,
    })
}

/// Calcula uma previsão apenas com amostras comparáveis do mesmo período.
/// Exige >= 2 horas e 3 observações. Evita apresentar falsa precisão.
fn forecast(samples: &[Sample], remaining: f64, reset_ms: i64, now_ms: i64) -> Value {
    if remaining <= 0.0 {
        return note("exhausted", 0.0, None);
    }
    if reset_ms <= now_ms {
        return note("expired", 0.0, None);
    }
    let recent: Vec<Sample> = samples
        .iter()
        .copied()
        .filter(|x| x.at >= now_ms.saturating_sub(TREND_MS) && x.at <= now_ms)
        .collect();
    let Some(first) = recent.first() else {
        return note("insufficient", 0.0, None);
    };
    let Some(last) = recent.last() else {
        return note("insufficient", 0.0, None);
    };
    let elapsed = last.at - first.at;
    let hours = (elapsed.max(0) as f64) / (HOUR_MS as f64);
    if recent.len() < 3 || elapsed < MIN_TREND_MS {
        return note("insufficient", hours, None);
    }
    let consumed = last.used - first.used;
    if consumed < -0.25 {
        return note("insufficient", hours, None);
    }
    if consumed < 0.2 {
        return note("low_use", hours, None);
    }
    let rate_per_hour = consumed / hours;
    if !rate_per_hour.is_finite() || rate_per_hour <= 0.0 {
        return note("insufficient", hours, None);
    }
    let until_exhaust_ms = remaining / rate_per_hour * (HOUR_MS as f64);
    let until_reset_ms = (reset_ms - now_ms) as f64;
    if !until_exhaust_ms.is_finite() || until_exhaust_ms <= 0.0 {
        return note("insufficient", hours, None);
    }
    // Perto do ponto de reposição, não sugerir certeza artificial.
    let ratio = until_exhaust_ms / until_reset_ms;
    let status = if ratio < 0.9 {
        "at_risk"
    } else if ratio <= 1.1 {
        "near_limit"
    } else {
        "on_track"
    };
    let projected = if until_exhaust_ms <= (4 * WEEK_MS) as f64 {
        Some(now_ms.saturating_add(until_exhaust_ms.round() as i64) / 1000)
    } else {
        None
    };
    note(status, hours, projected)
}

/// Guarda até sete dias de leituras, com intervalo mínimo de 15 minutos.
/// Se o reset mudar ou o consumo regredir, o histórico deixa de ser comparável.
pub fn update(db: &Db, snapshot: &mut Value) {
    if snapshot["status"] != "ok" {
        return;
    }
    let Some(now_ms) = snapshot["updated_at"].as_i64().filter(|t| *t > 0) else {
        snapshot["weekly_forecast"] = note("insufficient", 0.0, None);
        return;
    };
    let Some(rem) = snapshot["weekly"]["remaining"]
        .as_f64()
        .filter(|x| x.is_finite() && (0.0..=100.0).contains(x))
    else {
        snapshot["weekly_forecast"] = note("unavailable", 0.0, None);
        return;
    };
    let Some(reset_sec) = snapshot["weekly"]["resets_at"].as_i64() else {
        snapshot["weekly_forecast"] = note("insufficient", 0.0, None);
        return;
    };
    let Some(reset_ms) = reset_sec.checked_mul(1000) else {
        snapshot["weekly_forecast"] = note("insufficient", 0.0, None);
        return;
    };
    if reset_ms <= now_ms || reset_ms - now_ms > WEEK_MS {
        snapshot["weekly_forecast"] = note("insufficient", 0.0, None);
        return;
    }

    let stored = db.kg(KEY);
    let mut samples: Vec<Sample> = if stored["reset_at"] == reset_sec {
        stored["samples"]
            .as_array()
            .map(|rows| rows.iter().filter_map(|v| valid_sample(v, now_ms)).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    samples.sort_by_key(|s| s.at);
    // Rejeita históricos que regrediram, por reinício da quota, conta
    // diferente ou variações de uma janela de consumo deslizante.
    if samples.windows(2).any(|pair| pair[1].used + 0.25 < pair[0].used) {
        samples.clear();
    }
    let used = 100.0 - rem;
    if samples.last().is_some_and(|prev| used + 0.25 < prev.used) {
        samples.clear();
    }

    let current = Sample { at: now_ms, used };
    let save = samples.last().is_none_or(|last| {
        now_ms.saturating_sub(last.at) >= SAVE_INTERVAL_MS
            || used - last.used >= 1.0
    });
    if save {
        samples.push(current);
        if samples.len() > MAX_SAMPLES {
            let excess = samples.len() - MAX_SAMPLES;
            samples.drain(..excess);
        }
        let saved: Vec<Value> = samples.iter().map(|s| json!([s.at, s.used])).collect();
        db.ks(KEY, &json!({"reset_at":reset_sec,"samples":saved}));
    }

    // Usa o valor mais recente na previsão, mesmo quando a captura
    // persistente ainda não atingiu os 15 minutos.
    let mut observed = samples;
    if observed.last().is_none_or(|last| last.at < now_ms) {
        observed.push(current);
    }
    snapshot["weekly_forecast"] = forecast(&observed, rem, reset_ms, now_ms);
}

#[cfg(test)]
mod tests {
    use super::*;
    const H: i64 = HOUR_MS;
    const NOW: i64 = 1_800_000_000_000;

    fn sample(hours_ago: i64, used: f64) -> Sample {
        Sample { at: NOW - hours_ago * H, used }
    }

    #[test]
    fn predicts_depletion_before_reset_and_near_limit() {
        let history = [sample(4, 50.0), sample(2, 58.0), sample(0, 66.0)];
        let a = forecast(&history, 34.0, NOW + 20 * H, NOW);
        assert_eq!(a["status"], "at_risk"); // 8,5 h até esgotar; 20 h até reset
        let b = forecast(&history, 34.0, NOW + 9 * H, NOW);
        assert_eq!(b["status"], "near_limit");
    }

    #[test]
    fn reports_sustainable_and_low_use_without_promising_a_guarantee() {
        let history = [sample(6, 15.0), sample(3, 16.0), sample(0, 17.0)];
        assert_eq!(forecast(&history, 83.0, NOW + 24 * H, NOW)["status"], "on_track");
        let little = [sample(5, 20.00), sample(3, 20.01), sample(0, 20.05)];
        assert_eq!(forecast(&little, 79.95, NOW + 24 * H, NOW)["status"], "low_use");
    }

    #[test]
    fn refuses_scarce_or_decreasing_history() {
        let scarce = [sample(1, 20.0), sample(0, 21.0)];
        assert_eq!(forecast(&scarce, 79.0, NOW + 24 * H, NOW)["status"], "insufficient");
        let descending = [sample(3, 45.0), sample(2, 43.0), sample(0, 41.0)];
        assert_eq!(forecast(&descending, 59.0, NOW + 24 * H, NOW)["status"], "insufficient");
    }

    fn state(at: i64, used: f64, reset: i64) -> Value {
        json!({
            "status":"ok","updated_at":at,
            "weekly":{"remaining":100.0-used, "resets_at":reset / 1000}
        })
    }

    #[test]
    fn history_persists_but_resets_on_new_window_or_decreased_consumption() {
        let db = Db::mem();
        let reset = NOW + 48 * H;
        for (hours_ago, used) in [(5,40.0),(3,48.0),(0,60.0)] {
            let mut s = state(NOW - hours_ago*H, used, reset);
            update(&db, &mut s);
        }
        let previous = db.kg(KEY);
        assert_eq!(previous["samples"].as_array().unwrap().len(), 3);
        let mut next = state(NOW + H, 65.0, reset);
        update(&db, &mut next);
        assert_eq!(next["weekly_forecast"]["status"], "at_risk");

        let mut lower = state(NOW + 2*H, 12.0, reset);
        update(&db, &mut lower);
        assert_eq!(lower["weekly_forecast"]["status"], "insufficient");
        assert_eq!(db.kg(KEY)["samples"].as_array().unwrap().len(), 1);

        let mut new_window = state(NOW + 3*H, 2.0, reset + 3*H);
        update(&db, &mut new_window);
        assert_eq!(new_window["weekly_forecast"]["status"], "insufficient");
        assert_eq!(db.kg(KEY)["samples"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn stale_or_missing_weekly_never_contributes_to_trend() {
        let db = Db::mem();
        let mut stale = state(NOW, 20.0, NOW + 24 * H);
        stale["status"] = json!("stale");
        update(&db, &mut stale);
        assert!(db.kg(KEY).is_null());
        let mut no_reset = state(NOW, 20.0, NOW + 24 * H);
        no_reset["weekly"]["resets_at"] = Value::Null;
        update(&db, &mut no_reset);
        assert_eq!(no_reset["weekly_forecast"]["status"], "insufficient");
        assert!(db.kg(KEY).is_null());
    }

    #[test]
    fn caps_persisted_samples_and_rejects_expired_reset() {
        let db = Db::mem();
        let reset = NOW + 24 * H;
        let mut expired = state(NOW, 5.0, NOW - H);
        update(&db, &mut expired);
        assert_eq!(expired["weekly_forecast"]["status"], "insufficient");
        assert!(db.kg(KEY).is_null());
        for n in 0..900_i64 {
            let at = NOW - 6*24*H + n*10*60_000;
            let mut s = state(at, 10.0 + (n as f64)/1000.0, reset);
            update(&db, &mut s);
        }
        let stored = db.kg(KEY);
        assert!(stored["samples"].as_array().unwrap().len() <= MAX_SAMPLES);
    }
}
