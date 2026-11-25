use crate::app::{sl, Ax, R, X};
use crate::db::now;
use crate::sh::{gh, ho, hosts, hraw};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

const HW: &str = r#"o=$(uname -s); echo "os $o"
if [ "$o" = Darwin ]; then echo "cpu $(sysctl -n hw.ncpu)"; echo "load $(sysctl -n vm.loadavg | awk '{print $2}')"; p=$(pagesize); echo "mem $(($(sysctl -n hw.memsize)/1048576)) $(vm_stat | awk -v p=$p '/Pages (free|inactive|speculative)/{gsub(/[.]/,"",$NF);s+=$NF} END{print int(s*p/1048576)}')"
else echo "cpu $(nproc)"; echo "load $(cut -d' ' -f1 /proc/loadavg)"; echo "mem $(awk '/MemTotal/{t=$2}/MemAvailable/{a=$2}END{print int(t/1024), int(a/1024)}' /proc/meminfo)"; t=$(cat /sys/class/thermal/thermal_zone*/temp | sort -n | tail -1); [ -n "$t" ] && echo "temp $((t/1000))"; fi
echo "disk $(df -Pk "$HOME" | awk 'NR==2{print int($2/1048576), int($4/1048576)}')"
echo "up $(uptime | sed 's/.*up *//; s/, *[0-9]* user.*//')"
# Optional, read-only NVIDIA telemetry. Never install a driver or require a GPU.
if command -v nvidia-smi >/dev/null 2>&1 && command -v timeout >/dev/null 2>&1; then
    timeout 5s nvidia-smi --query-gpu=index,memory.total,memory.used,temperature.gpu,utilization.gpu --format=csv,noheader,nounits 2>/dev/null | awk 'NR<=16 {print "gpu " $0}'
fi"#;

fn hw(o: &str) -> serde_json::Value {
    let mut v = serde_json::json!({"t": now()});
    let mut gpus = Vec::new();
    let mut gpu_ids = std::collections::HashSet::new();
    let mut numeric_keys = std::collections::HashSet::new();
    for l in o.lines() {
        let (k, x) = l.split_once(' ').unwrap_or((l, ""));
        if k == "gpu" {
            if let Some(gpu) = gpu_sample(x) {
                if gpu_ids.insert(gpu["index"].as_u64().unwrap_or(u64::MAX)) {
                    gpus.push(gpu);
                }
            }
            continue;
        }
        if matches!(k, "os" | "up") {
            v[k] = serde_json::json!(x.trim());
            continue;
        }
        if matches!(k, "cpu" | "load" | "temp" | "mem" | "disk") && !numeric_keys.insert(k) {
            return serde_json::json!({"t":null});
        }
        let Ok(ns) = x
            .split_whitespace()
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>()
        else {
            continue;
        };
        if !ns.iter().all(|n| n.is_finite()) {
            continue;
        }
        match k {
            "cpu" | "load" | "temp" if ns.len() == 1 => v[k] = serde_json::json!(ns[0]),
            "mem" | "disk" if ns.len() == 2 => v[k] = serde_json::json!(ns),
            _ => {}
        }
    }
    if !gpus.is_empty() {
        v["gpu"] = serde_json::json!(gpus);
    }
    v
}

fn gpu_sample(line: &str) -> Option<serde_json::Value> {
    let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
    if fields.len() != 5 {
        return None;
    }
    let index = fields[0].parse::<u64>().ok()?;
    if index > 255 {
        return None;
    }
    let total = fields[1].parse::<u64>().ok().filter(|v| *v > 0);
    let used = fields[2].parse::<u64>().ok();
    let memory = total.zip(used).filter(|(t, u)| u <= t);
    let temp = fields[3]
        .parse::<f64>()
        .ok()
        .filter(|v| (0.0..=130.0).contains(v));
    let util = fields[4]
        .parse::<f64>()
        .ok()
        .filter(|v| (0.0..=100.0).contains(v));
    if memory.is_none() && temp.is_none() && util.is_none() {
        return None;
    }
    Some(
        serde_json::json!({"index":index,"vram_total_mib":memory.map(|(t,_)|t),
        "vram_used_mib":memory.map(|(_,u)|u),"temperature_c":temp,"utilization_percent":util}),
    )
}

static FAIL: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, u32>>> =
    std::sync::LazyLock::new(Default::default);

static NEXT_PROBE: AtomicU64 = AtomicU64::new(1);
#[derive(Default)]
struct ProbeBook {
    pending: HashMap<String, u64>,
    checked: HashMap<String, i64>,
}

impl ProbeBook {
    fn begin(&mut self, key: &str, ticket: u64) {
        self.pending.insert(key.to_string(), ticket);
    }

    fn publish(&mut self, key: &str, ticket: u64, at: i64) -> bool {
        if self.pending.get(key) != Some(&ticket) {
            return false;
        }
        self.pending.remove(key);
        self.checked.insert(key.to_string(), at);
        true
    }

    fn invalidate(&mut self, key: &str) {
        self.pending.remove(key);
        self.checked.remove(key);
    }

    fn fresh(&self, key: &str, at: i64) -> bool {
        !self.pending.contains_key(key)
            && self
                .checked
                .get(key)
                .and_then(|last| at.checked_sub(*last))
                .is_some_and(|age| (0..=20_000).contains(&age))
    }
}

static PROBES: std::sync::LazyLock<std::sync::Mutex<ProbeBook>> =
    std::sync::LazyLock::new(Default::default);

fn begin_probe(key: &str) -> u64 {
    let mut probes = PROBES.lock().unwrap();
    let ticket = NEXT_PROBE.fetch_add(1, Ordering::Relaxed);
    probes.begin(key, ticket);
    ticket
}

fn fresh(key: &str) -> bool {
    PROBES.lock().unwrap().fresh(key, now())
}

async fn ck(a: &Ax, k: &str) -> (bool, Option<serde_json::Value>) {
    match k {
        "gh" => (
            gh(&["api", "rate_limit", "--jq", ".rate.remaining"], 30)
                .await
                .is_ok(),
            None,
        ),
        "an" if crate::routing::enabled(a) => (true, None),
        "an" => (
            a.hc.get("https://api.anthropic.com/")
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await
                .is_ok(),
            None,
        ),
        h => match ho(a, h) {
            Some(x) => {
                let dkcheck = format!(
                    "docker inspect -f '{{{{.State.Status}}}}' {} | grep -qx running",
                    crate::sh::ct()
                );
                let o = hraw(
                    &x,
                    &format!(
                        "{{ {HW}; }} 2>/dev/null; {}",
                        if x.dk { dkcheck.as_str() } else { "true" }
                    ),
                    45,
                )
                .await;
                (o.c == 0, (o.c == 0).then(|| hw(&o.o)))
            }
            None => (false, None),
        },
    }
}

pub async fn probe(a: &Ax, ks: &[String]) {
    let tickets: Vec<_> = ks.iter().map(|k| begin_probe(k)).collect();
    let v = futures::future::join_all(ks.iter().map(|k| ck(a, k))).await;
    publish_probe_results(a, ks, tickets, v);
}

fn publish_probe_results(
    a: &Ax,
    ks: &[String],
    tickets: Vec<u64>,
    v: Vec<(bool, Option<serde_json::Value>)>,
) {
    let mut l = a.lv.lock().unwrap();
    let mut probes = PROBES.lock().unwrap();
    let mut f = FAIL.lock().unwrap();
    for ((k, ticket), (ok, sample)) in ks.iter().zip(tickets).zip(v) {
        if !probes.publish(k, ticket, now()) {
            continue;
        }
        if let Some(sample) = sample {
            l.hst.insert(k.clone(), sample);
        }
        let n = if ok {
            0
        } else {
            f.get(k).copied().unwrap_or(0).saturating_add(1)
        };
        f.insert(k.clone(), n);
        if !ok {
            l.hst.remove(k);
        }
        if l.o.net.insert(k.clone(), n < 2) == Some(false) && n == 0 && k != "gh" && k != "an" {
            let (a2, id) = (a.clone(), k.clone());
            tokio::spawn(async move { crate::sh::orph(&a2, &id).await });
        }
    }
    l.o.off = !*l.o.net.get("gh").unwrap_or(&true) || !*l.o.net.get("an").unwrap_or(&true);
}

pub fn down(a: &Ax, k: &str) {
    let mut l = a.lv.lock().unwrap();
    PROBES.lock().unwrap().invalidate(k);
    FAIL.lock().unwrap().insert(k.to_string(), 2);
    l.hst.remove(k);
    l.o.net.insert(k.to_string(), false);
}

pub async fn all(a: &Ax) {
    let mut ks: Vec<String> = vec!["gh".into(), "an".into()];
    ks.extend(hosts(a).into_iter().filter(|h| h.on).map(|h| h.id));
    probe(a, &ks).await;
}

/// Single availability check for an explicitly restartable phase boundary.
/// Unlike need(), this never occupies a lane through exponential sleep.
pub async fn need_once(a: &Ax, ks: &[&str]) -> R<()> {
    need_once_with(a, ks, |keys| async move { probe(a, &keys).await }).await
}

async fn need_once_with<F, Fut>(a: &Ax, ks: &[&str], probe_keys: F) -> R<()>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let keys = ks
        .iter()
        .filter(|k| !k.is_empty())
        .map(|k| k.to_string())
        .collect::<Vec<_>>();
    if keys.iter().any(|k| !fresh(k) || !a.net(k)) {
        probe_keys(keys.clone()).await;
    }
    // An overlapping newer probe owns the decision. Defer conservatively
    // rather than authorizing work with a rejected older result.
    let missing = keys
        .into_iter()
        .filter(|k| !fresh(k) || !a.net(k))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    for k in &missing {
        if k != "gh" && k != "an" && crate::sh::ho(a, k).map(|h| !h.on).unwrap_or(true) {
            return Err(X::Back(format!("{k} left the office")));
        }
    }
    Err(X::Defer(format!(
        "network unavailable: {}",
        missing.join(", ")
    )))
}

pub async fn need(a: &Ax, w: &str, ks: &[&str]) -> R<()> {
    let mut b = 30_000u64;
    let ks: Vec<String> = ks
        .iter()
        .filter(|k| !k.is_empty())
        .map(|k| k.to_string())
        .collect();
    loop {
        if ks.iter().any(|k| !fresh(k) || !a.net(k)) {
            probe(a, &ks).await;
        }
        let miss: Vec<String> = ks
            .iter()
            .filter(|k| !fresh(k) || !a.net(k))
            .cloned()
            .collect();
        if miss.is_empty() {
            return Ok(());
        }
        for k in &miss {
            if k != "gh" && k != "an" && crate::sh::ho(a, k).map(|h| !h.on).unwrap_or(true) {
                return Err(X::Back(format!("{k} left the office")));
            }
        }
        a.say(
            w,
            &format!(
                "offline: {} unreachable, retry in {} s",
                miss.join(", "),
                b / 1000
            ),
            None,
        );
        sl(b).await;
        b = (b * 2).min(300_000);
    }
}

pub async fn watch(a: Ax) {
    loop {
        all(&a).await;
        sl(60_000).await;
    }
}

#[cfg(test)]
mod gpu_telemetry_tests {
    use super::*;

    #[test]
    fn worker_down_invalidates_only_its_telemetry() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            47300,
            "apollo-office".into(),
        ));
        let host = "fixture-telemetry-down";
        let other = "fixture-telemetry-other";
        {
            let mut live = a.lv.lock().unwrap();
            live.hst.insert(host.into(), hw("cpu 8"));
            live.hst.insert(other.into(), hw("cpu 4"));
        }
        let old = begin_probe(host);
        let latest = begin_probe(host);
        assert_ne!(old, latest);
        assert_eq!(PROBES.lock().unwrap().pending.get(host), Some(&latest));
        down(&a, host);
        assert!(!PROBES.lock().unwrap().pending.contains_key(host));
        assert!(!fresh(host));
        let live = a.lv.lock().unwrap();
        assert!(!live.hst.contains_key(host));
        assert!(live.hst.contains_key(other));
        assert_eq!(live.o.net.get(host), Some(&false));
        drop(live);
        FAIL.lock().unwrap().remove(host);
    }

    #[test]
    fn stale_probe_cannot_refresh_its_key_or_another_worker() {
        let mut book = ProbeBook::default();
        book.begin("a", 1);
        book.begin("a", 2);
        assert!(book.publish("a", 2, 1_000));
        assert!(book.fresh("a", 20_000));
        assert!(!book.publish("a", 1, 20_000));
        assert!(!book.fresh("a", 21_001));
        book.begin("b", 3);
        assert!(book.publish("b", 3, 21_001));
        assert!(book.fresh("b", 21_001));
        assert!(!book.fresh("a", 21_001));
        book.begin("a", 4);
        book.begin("a", 5);
        assert!(!book.publish("a", 4, 22_000));
        assert!(!book.fresh("a", 22_000));
        assert!(book.publish("a", 5, 22_001));
        assert!(book.fresh("a", 22_001));
        assert!(!book.fresh("a", 22_000));
        book.invalidate("a");
        assert!(!book.fresh("a", 22_001));
        assert!(book.fresh("b", 22_001));
    }

    #[test]
    fn stale_probe_cannot_publish_after_newer_result_or_down() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            47300,
            "apollo-office".into(),
        ));
        let host = "fixture-probe-publication";
        let keys = [host.to_string()];
        let older = begin_probe(host);
        let newer = begin_probe(host);
        let sample = hw("cpu 8");
        publish_probe_results(&a, &keys, vec![newer], vec![(true, Some(sample.clone()))]);
        publish_probe_results(&a, &keys, vec![older], vec![(false, None)]);
        {
            let live = a.lv.lock().unwrap();
            assert_eq!(live.hst.get(host), Some(&sample));
            assert_eq!(live.o.net.get(host), Some(&true));
            assert_eq!(FAIL.lock().unwrap().get(host), Some(&0));
        }
        let late = begin_probe(host);
        down(&a, host);
        publish_probe_results(&a, &keys, vec![late], vec![(true, Some(hw("cpu 64")))]);
        {
            let live = a.lv.lock().unwrap();
            assert!(!live.hst.contains_key(host));
            assert_eq!(live.o.net.get(host), Some(&false));
            assert_eq!(FAIL.lock().unwrap().get(host), Some(&2));
        }
        assert!(!fresh(host));
        PROBES.lock().unwrap().invalidate(host);
        FAIL.lock().unwrap().remove(host);
    }

    #[tokio::test]
    async fn need_once_defers_until_newer_probe_publishes() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            47300,
            "apollo-office".into(),
        ));
        let host = "fixture-overlapping-probe";
        a.db.run(
            "insert into hs(id,on_) values(?,1)",
            crate::pa![host.to_string()],
        );
        let newer = std::cell::Cell::new(0);
        let a_ref = &a;
        let newer_ref = &newer;
        let result = need_once_with(&a, &[host], |keys| async move {
            let older = begin_probe(host);
            newer_ref.set(begin_probe(host));
            publish_probe_results(
                a_ref,
                &keys,
                vec![older],
                vec![(true, Some(hw("cpu 8")))],
            );
        })
        .await;
        assert!(matches!(result, Err(X::Defer(_))));
        assert_eq!(
            PROBES.lock().unwrap().pending.get(host),
            Some(&newer.get())
        );
        assert!(!fresh(host));
        let keys = [host.to_string()];
        publish_probe_results(
            &a,
            &keys,
            vec![newer.get()],
            vec![(true, Some(hw("cpu 16")))],
        );
        assert!(need_once(&a, &[host]).await.is_ok());
        assert_eq!(a.lv.lock().unwrap().hst[host]["cpu"], 16.0);
        PROBES.lock().unwrap().invalidate(host);
        FAIL.lock().unwrap().remove(host);
    }

    #[test]
    fn duplicate_numeric_samples_are_unavailable() {
        for sample in [
            "cpu 8\ncpu 64",
            "mem 16000 100\nmem 16000 8000",
            "temp invalid\ntemp 50",
            "temp 50\ntemp invalid",
            "load 1\nload 2",
            "disk 100 20\ndisk 100 90",
        ] {
            assert!(hw(sample)["t"].is_null());
        }
        assert!(hw("cpu 8\nload 1\nmem 16000 8000")["t"].is_i64());
    }

    #[test]
    fn malformed_numeric_samples_are_not_reinterpreted() {
        let v = hw("cpu 8 bad\nload NaN\ntemp 50 90\nmem 16000 invalid 8000\ndisk inf 100");
        for key in ["cpu", "load", "temp", "mem", "disk"] {
            assert!(v[key].is_null());
        }
        let valid = hw("cpu 8\nload 1.5\nmem 16000 8000");
        assert_eq!(valid["cpu"], 8.0);
        assert_eq!(valid["load"], 1.5);
        assert_eq!(valid["mem"][1], 8000.0);
    }

    #[test]
    fn gpu_metrics_are_optional_bounded_and_per_device() {
        let v = hw("cpu 8\ngpu 0,8192,1024,55,23\ngpu 1,16384,8192,N/A,99\ngpu 0,8192,4096,70,100");
        let gpu = v["gpu"].as_array().unwrap();
        assert_eq!(gpu.len(), 2);
        assert_eq!(gpu[0]["vram_used_mib"], 1024);
        assert_eq!(gpu[1]["temperature_c"], serde_json::Value::Null);
        assert_eq!(gpu[1]["vram_total_mib"], 16384);
        assert!(hw("cpu 8\ngpu 0,N/A,N/A,N/A,N/A")["gpu"].is_null());
        let invalid = gpu_sample("0,1024,2048,999,101");
        assert!(invalid.is_none());
        assert!(gpu_sample("no GPU").is_none());
    }
}
