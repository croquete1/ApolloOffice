//! Opt-in limits on new work, never termination of existing jobs.
use crate::app::App;
use serde_json::{json, Value};

pub const PROFILES: [&str; 5] = ["manual", "economic", "balanced", "performance", "night"];

fn profile(a: &App) -> String {
    let settings = a.set();
    settings["resource_profile"]
        .as_str()
        .filter(|p| PROFILES.contains(p))
        .unwrap_or("manual")
        .to_string()
}

fn capacity(profile: &str, configured: i64, telemetry: &Value, at: i64) -> (i64, &'static str) {
    let configured = configured.max(0);
    let nominal = match profile {
        "economic" => (configured + 1) / 2,
        "night" => configured.min(1),
        _ => configured,
    };
    if profile == "manual" {
        return (nominal, "manual");
    }
    let fresh = telemetry["t"]
        .as_i64()
        .is_some_and(|t| t > 0 && t <= at && at.saturating_sub(t) <= 180_000);
    if !fresh {
        return (nominal.min(1), "telemetry_unavailable");
    }
    let total = telemetry["mem"][0].as_f64().filter(|v| v.is_finite() && *v > 0.0);
    let free = telemetry["mem"][1].as_f64().filter(|v| v.is_finite() && *v >= 0.0);
    let memory = total.zip(free).filter(|(t, f)| f <= t);
    let temp = telemetry["temp"].as_f64().filter(|v| (0.0..=130.0).contains(v));
    if temp.is_some_and(|v| v >= 95.0) || memory.is_some_and(|(t, f)| f < (t * 0.05).max(256.0)) {
        return (0, "critical_pressure");
    }
    if temp.is_some_and(|v| v >= 85.0) || memory.is_some_and(|(t, f)| f < (t * 0.15).max(768.0)) {
        return (nominal.min(1), "memory_or_temperature");
    }
    let cpu = telemetry["cpu"].as_f64().filter(|v| v.is_finite() && *v > 0.0);
    let load = telemetry["load"].as_f64().filter(|v| v.is_finite() && *v >= 0.0);
    if cpu.zip(load).is_some_and(|(c, l)| l >= c * 1.25) {
        return (nominal.min(1), "cpu_load");
    }
    if memory.is_none() || cpu.is_none() || load.is_none() {
        return (nominal.min(1), "telemetry_incomplete");
    }
    (nominal, "profile")
}

pub fn host_capacity(a: &App, host: &str, configured: i64) -> (i64, &'static str) {
    let p = profile(a);
    let telemetry = a.lv.lock().unwrap().hst.get(host).cloned().unwrap_or(Value::Null);
    capacity(&p, configured, &telemetry, crate::db::now())
}

pub fn hunter_cap(a: &App, configured: i64) -> i64 {
    match profile(a).as_str() {
        "night" => 0,
        "economic" => configured.min(1),
        _ => configured,
    }
}

/// Reports observed capacity, not a promise that a task will pass every gate.
pub fn status(a: &App) -> Value {
    let p = profile(a);
    let active = a.act.lock().unwrap().clone();
    let hosts = crate::sh::hosts(a).into_iter().map(|h| {
        let (cap, reason) = host_capacity(a, &h.id, h.ln);
        let running = active.iter().filter(|s| s.split_once(' ').is_some_and(|(_, host)| host == h.id)).count();
        json!({"id":h.id,"enabled":h.on,"configured_lanes":h.ln,
            "effective_lanes":if h.on {cap} else {0},"active":running,"reason":reason})
    }).collect::<Vec<_>>();
    json!({"profile":p,"hosts":hosts,"hunter_cap":hunter_cap(a,a.set()["hunters"].as_i64().unwrap_or(2).clamp(0,12))})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_respect_limits_and_unknown_telemetry() {
        let at = 1_000_000;
        let mut hw = json!({"t":at,"cpu":8,"load":2.0,"mem":[16000,8000],"temp":60});
        assert_eq!(capacity("manual", 8, &Value::Null, at).0,8);
        assert_eq!(capacity("economic", 5, &hw, at).0,3);
        assert_eq!(capacity("balanced", 5, &hw, at).0,5);
        assert_eq!(capacity("performance", 5, &hw, at).0,5);
        assert_eq!(capacity("night", 5, &hw, at).0,1);
        assert_eq!(capacity("balanced", 5, &hw, at+180_001).0,1);
        hw["mem"][1]=json!(300);
        assert_eq!(capacity("balanced", 5, &hw, at).0,0);
        hw["mem"][1]=json!(8000);
        hw["temp"]=json!(90);
        assert_eq!(capacity("performance", 5, &hw, at).0,1);
        hw["temp"]=json!(96);
        assert_eq!(capacity("performance", 5, &hw, at).0,0);
        assert_eq!(capacity("manual", 5, &hw, at).0,5);
    }
}
