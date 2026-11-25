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
        "economic" => configured / 2 + configured % 2,
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
    let total = telemetry["mem"][0]
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0);
    let free = telemetry["mem"][1]
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0);
    let memory = total.zip(free).filter(|(t, f)| f <= t);
    let temp = telemetry["temp"]
        .as_f64()
        .filter(|v| (0.0..=130.0).contains(v));
    // Worker probes report filesystem capacity in whole GiB. A zero free
    // sample means less than 1 GiB remains, not necessarily exactly zero.
    let disk = telemetry["disk"].as_array().and_then(|values| {
        if values.len() != 2 {
            return None;
        }
        let total = values[0].as_f64()?;
        let free = values[1].as_f64()?;
        (total.is_finite() && free.is_finite() && total > 0.0 && free >= 0.0 && free <= total)
            .then_some(free)
    });
    if temp.is_some_and(|v| v >= 95.0) || memory.is_some_and(|(t, f)| f < (t * 0.05).max(256.0)) {
        return (0, "critical_pressure");
    }
    if disk.is_some_and(|free| free < 1.0) {
        return (0, "disk_critical");
    }
    if disk.is_some_and(|free| free < 5.0) {
        return (nominal.min(1), "disk_pressure");
    }
    if temp.is_some_and(|v| v >= 85.0) || memory.is_some_and(|(t, f)| f < (t * 0.15).max(768.0)) {
        return (nominal.min(1), "memory_or_temperature");
    }
    let cpu = telemetry["cpu"]
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0);
    let load = telemetry["load"]
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0);
    if cpu.zip(load).is_some_and(|(c, l)| l >= c * 1.25) {
        return (nominal.min(1), "cpu_load");
    }
    if memory.is_none() || cpu.is_none() || load.is_none() {
        return (nominal.min(1), "telemetry_incomplete");
    }
    // Balanced leaves headroom for interactive work before hard pressure.
    // Performance may use the configured ceiling until the shared hard limits.
    if profile == "balanced"
        && (cpu.zip(load).is_some_and(|(c, l)| l >= c * 0.8)
            || memory.is_some_and(|(t, f)| f < t * 0.25)
            || temp.is_some_and(|v| v >= 78.0))
    {
        return (nominal / 2 + nominal % 2, "balanced_headroom");
    }
    (nominal, "profile")
}

pub fn host_capacity(a: &App, host: &str, configured: i64) -> (i64, &'static str) {
    let p = profile(a);
    let telemetry =
        a.lv.lock()
            .unwrap()
            .hst
            .get(host)
            .cloned()
            .unwrap_or(Value::Null);
    capacity(&p, configured, &telemetry, crate::db::now())
}

/// Restartable CI may defer under critical pressure. Existing non-restartable
/// work must retain one slot to finish safely without deadlocking its lane.
pub fn ci_capacity(a: &App, host: &str, configured: i64, restartable: bool) -> i64 {
    let configured = configured.clamp(1, 8);
    let (cap, _) = host_capacity(a, host, configured);
    if restartable {
        cap
    } else {
        cap.max(1)
    }
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
    let ci_active = a.sem.lock().unwrap().clone();
    let hosts = crate::sh::hosts(a)
        .into_iter()
        .map(|h| {
            let (cap, reason) = host_capacity(a, &h.id, h.ln);
            let running = active
                .iter()
                .filter(|s| s.split_once(' ').is_some_and(|(_, host)| host == h.id))
                .count();
            let telemetry =
                a.lv.lock()
                    .unwrap()
                    .hst
                    .get(&h.id)
                    .cloned()
                    .unwrap_or(Value::Null);
            let at = crate::db::now();
            let fresh = telemetry["t"]
                .as_i64()
                .is_some_and(|t| t > 0 && t <= at && at.saturating_sub(t) <= 180_000);
            json!({"id":h.id,"enabled":h.on,"configured_lanes":h.ln,
            "effective_lanes":if h.on {cap} else {0},"active":running,"reason":reason,
            "configured_ci":h.ci,"effective_ci":if h.on {ci_capacity(a,&h.id,h.ci,true)} else {0},
            "completion_ci_floor":if h.on {ci_capacity(a,&h.id,h.ci,false)} else {0},
            "active_ci":ci_active.get(&h.id).copied().unwrap_or(0),
            "gpu":if fresh {telemetry["gpu"].clone()} else {Value::Null},
            "telemetry_fresh":fresh})
        })
        .collect::<Vec<_>>();
    json!({"profile":p,"hosts":hosts,"hunter_cap":hunter_cap(a,a.set()["hunters"].as_i64().unwrap_or(2).clamp(0,12))})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn night_and_pressure_limit_only_new_admissions() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full) values('o/a'),('o/b'),('o/c')", vec![]);
        db.run("insert into hs(id,ln,on_) values('local',4,1)", vec![]);
        let a = std::sync::Arc::new(App::new(
            db,
            std::env::temp_dir(),
            1,
            "apollo-office".into(),
        ));
        let first = a.db.jnew(
            "o/a",
            1,
            "fix",
            "one",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        let second = a.db.jnew(
            "o/b",
            2,
            "fix",
            "two",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        a.db.ks("set", &json!({"resource_profile":"night","hunters":5}));
        a.lv.lock().unwrap().hst.insert(
            "local".into(),
            json!({"t":crate::db::now(),"cpu":8,"load":0,"mem":[16000,8000],"temp":60}),
        );
        assert_eq!(hunter_cap(&a, 5), 0);
        assert!(crate::pipe::admit(&a, "o/a", "local", first));
        assert!(!crate::pipe::admit(&a, "o/b", "local", second));
        a.db.ks("set", &json!({"resource_profile":"performance"}));
        assert!(crate::pipe::admit(&a, "o/b", "local", second));
        a.lv.lock().unwrap().hst.get_mut("local").unwrap()["temp"] = json!(96);
        let third = a.db.jnew(
            "o/c",
            3,
            "fix",
            "three",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        assert!(!crate::pipe::admit(&a, "o/c", "local", third));
        assert_eq!(a.act.lock().unwrap().len(), 2);
        assert_eq!(status(&a)["hosts"][0]["effective_lanes"], 0);
    }

    #[test]
    fn ci_slots_follow_profile_without_blocking_nonrestartable_completion() {
        let db = crate::db::Db::mem();
        db.run("insert into hs(id,ln,ci,on_) values('local',4,4,1)", vec![]);
        let a = App::new(db, std::env::temp_dir(), 1, "apollo-office".into());
        a.db.ks("set", &json!({"resource_profile":"night"}));
        assert_eq!(ci_capacity(&a, "local", 4, true), 1);
        a.db.ks("set", &json!({"resource_profile":"performance"}));
        assert_eq!(ci_capacity(&a, "local", 4, true), 1);
        a.lv.lock().unwrap().hst.insert("local".into(),json!({"t":crate::db::now(),"cpu":8,"load":1,"mem":[16000,8000],"temp":96,"gpu":[{"index":0}]}));
        assert_eq!(ci_capacity(&a, "local", 4, true), 0);
        assert_eq!(ci_capacity(&a, "local", 4, false), 1);
        assert_eq!(status(&a)["hosts"][0]["effective_ci"], 0);
        a.db.ks("set", &json!({"resource_profile":"manual"}));
        assert_eq!(ci_capacity(&a, "local", 4, true), 4);
        a.lv.lock().unwrap().hst.get_mut("local").unwrap()["t"] = json!(1);
        assert!(status(&a)["hosts"][0]["gpu"].is_null());
    }

    #[test]
    fn profiles_respect_limits_and_unknown_telemetry() {
        let at = 1_000_000;
        let mut hw = json!({"t":at,"cpu":8,"load":2.0,"mem":[16000,8000],"temp":60});
        assert_eq!(capacity("manual", 8, &Value::Null, at).0, 8);
        assert_eq!(capacity("economic", 5, &hw, at).0, 3);
        assert_eq!(capacity("balanced", 5, &hw, at).0, 5);
        assert_eq!(capacity("performance", 5, &hw, at).0, 5);
        assert_eq!(capacity("night", 5, &hw, at).0, 1);
        assert_eq!(capacity("balanced", 5, &hw, at + 180_001).0, 1);
        hw["mem"][1] = json!(300);
        assert_eq!(capacity("balanced", 5, &hw, at).0, 0);
        hw["mem"][1] = json!(8000);
        hw["temp"] = json!(90);
        assert_eq!(capacity("performance", 5, &hw, at).0, 1);
        hw["temp"] = json!(96);
        assert_eq!(capacity("performance", 5, &hw, at).0, 0);
        assert_eq!(capacity("manual", 5, &hw, at).0, 5);
    }

    #[test]
    fn disk_pressure_limits_new_work_but_not_manual_or_completion() {
        let db = crate::db::Db::mem();
        let a = App::new(db, std::env::temp_dir(), 1, "apollo-office".into());
        a.db.ks("set", &json!({"resource_profile":"performance"}));
        a.lv.lock().unwrap().hst.insert(
            "local".into(),
            json!({"t":crate::db::now(),"cpu":8,"load":1,"mem":[16000,8000],"disk":[100,0]}),
        );
        assert_eq!(host_capacity(&a, "local", 4), (0, "disk_critical"));
        assert_eq!(ci_capacity(&a, "local", 4, true), 0);
        assert_eq!(ci_capacity(&a, "local", 4, false), 1);
        a.lv.lock().unwrap().hst.get_mut("local").unwrap()["disk"] = json!([100, 4]);
        assert_eq!(host_capacity(&a, "local", 4), (1, "disk_pressure"));
        a.lv.lock().unwrap().hst.get_mut("local").unwrap()["disk"] = json!([100, 5]);
        assert_eq!(host_capacity(&a, "local", 4), (4, "profile"));
        a.lv.lock().unwrap().hst.get_mut("local").unwrap()["disk"] = json!([100, 0]);
        a.db.ks("set", &json!({"resource_profile":"manual"}));
        assert_eq!(host_capacity(&a, "local", 4), (4, "manual"));
    }

    #[test]
    fn disk_limits_require_fresh_valid_telemetry() {
        let at = 1_000_000;
        let mut hw = json!({"t":at,"cpu":8,"load":1,"mem":[16000,8000],"disk":[100,0]});
        assert_eq!(capacity("performance", 4, &hw, at), (0, "disk_critical"));
        assert_eq!(
            capacity("performance", 4, &hw, at + 180_000),
            (0, "disk_critical")
        );
        assert_eq!(
            capacity("performance", 4, &hw, at + 180_001),
            (1, "telemetry_unavailable")
        );
        assert_eq!(
            capacity("performance", 4, &hw, at - 1),
            (1, "telemetry_unavailable")
        );
        for invalid in [
            Value::Null,
            json!([100]),
            json!([100, 0, 0]),
            json!([0, 0]),
            json!([100, -1]),
            json!([100, 101]),
            json!([100, "0"]),
        ] {
            hw["disk"] = invalid;
            assert_eq!(capacity("performance", 4, &hw, at), (4, "profile"));
        }
        hw["disk"] = json!([100, 0]);
        hw["mem"][1] = json!(100);
        assert_eq!(
            capacity("performance", 4, &hw, at),
            (0, "critical_pressure")
        );
    }

    #[test]
    fn balanced_preserves_headroom_without_relaxing_hard_limits() {
        let at = 1_000_000;
        let mut hw = json!({"t":at,"cpu":8,"load":7,"mem":[16000,8000],"temp":60});
        assert_eq!(capacity("balanced", 5, &hw, at), (3, "balanced_headroom"));
        assert_eq!(capacity("performance", 5, &hw, at).0, 5);
        hw["load"] = json!(2);
        hw["mem"][1] = json!(3500);
        assert_eq!(capacity("balanced", 5, &hw, at).0, 3);
        hw["mem"][1] = json!(8000);
        hw["temp"] = json!(79);
        assert_eq!(capacity("balanced", 5, &hw, at).0, 3);
        assert_eq!(capacity("performance", 5, &hw, at).0, 5);
        hw["temp"] = json!(96);
        assert_eq!(capacity("balanced", 5, &hw, at).0, 0);
        assert_eq!(capacity("performance", 5, &hw, at).0, 0);
        assert_eq!(capacity("economic", i64::MAX, &hw, at).0, 0);
    }
}
