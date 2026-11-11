import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Script } from "node:vm";

const read = (name) => readFileSync(new URL(name, import.meta.url), "utf8");
const pipe = read("../src/pipe.rs");
const web = read("../src/web.rs");
const health = read("../src/hunt_health.rs");
const shell = read("../src/sh.rs");
const original = read("../ui/o.js");
const panels = read("../ui/p.js");
const modern = read("../ui/control-room.js");

test("diagnostic UI scripts parse and expose authenticated error history", () => {
  assert.doesNotThrow(() => new Script(original));
  assert.doesNotThrow(() => new Script(panels));
  assert.doesNotThrow(() => new Script(modern));
  assert.match(web, /\("GET", "\/api\/failures"\)/);
  assert.match(web, /"failure": j\["d"\]\["failure"\]/);
  assert.match(panels, /data-reset-hunt/);
  assert.match(panels, /Diagnóstico técnico/);
});

test("failure diagnostics exclude intentional drops without hiding execution failures", () => {
  const query = web.match(/select id,repo,kind,title,why,t1,[^"\n]*from jobs where st='drop'[^"\n]*limit 80/)?.[0];
  assert.ok(query, "the authenticated failures endpoint must keep its bounded query");
  assert.match(query, /coalesce\(why,''\) not in \('dropped by the owner','repository removed'\)/);
  assert.match(query, /order by t1 desc limit 80/);
  assert.doesNotMatch(query, /json_extract\(d,'\$\.failure\.stage'\) is not null/);
  assert.match(web, /"why", json!\("dropped by the owner"\)/);
  assert.match(web, /why='repository removed'/);
});

test("repeated setup failures stop new hunts without disabling fixes", () => {
  assert.match(pipe, /crate::hunt_health::blocked\(a, r\)/);
  assert.match(pipe, /j\.kind == "hunt"/);
  assert.match(health, /const LIMIT: i64 = 3/);
  assert.match(health, /const COOLDOWN_MS: i64 = 30 \* 60 \* 1000/);
  assert.match(health, /pub fn success/);
  assert.match(pipe, /preflight_host_error\(&preflight\)/);
  assert.match(pipe, /crate::hunt_health::blocked\(a, repo\)/);
  assert.match(pipe, /crate::app::netish\(&result\.e\)/);
  assert.match(pipe, /let occupying = on/);
  assert.match(pipe, /let _transition = a.update_transition.lock\(\).unwrap\(\)/);
});

test("preflight runs before cloning and existing setup remains mandatory", () => {
  const prep = pipe.slice(pipe.indexOf("async fn prep("), pipe.indexOf("fn rn(", pipe.indexOf("async fn prep(")));
  assert.ok(prep.indexOf("setup_tool(&su)") < prep.indexOf("let mt = crate::gh::meta"));
  assert.match(prep, /command -v node/);
  assert.match(prep, /crate::hunt_health::diagnostic/);
  assert.match(prep, /setup_transport_error\(&r\)/);
  assert.match(pipe, /crate::hunt_health::host_diagnostic/);
  assert.match(prep, /"to 1800 bash -c/);
});

test("fix jobs outrank hunts and capacity is reserved for pending fixes", () => {
  assert.ok(pipe.indexOf('"fix" => 1.0') < pipe.indexOf('"hunt" => 2.0'));
  assert.match(pipe, /uncovered_fixes > 0/);
  assert.ok(pipe.includes("st='ready' and (num<>0 or json_extract(d,'$.ext') is not null)"));
  assert.doesNotMatch(pipe, /hf \+ i64::from\(pending_fixes > 0\)/);
  assert.match(web, /"idle_reason"/);
  assert.match(web, /let admitted = a\.lk\.lock\(\)\.unwrap\(\)/);
  assert.match(web, /\.filter\(\|job\| !admitted\.contains\(&job\["id"\]\.as_i64\(\)\.unwrap_or\(0\)\)\)/);
  assert.match(health, /DOCKER_AUTH_CONFIG/);
  assert.match(health, /identitytoken/);
  assert.match(panels, /data-retry-failures/);
  assert.match(panels, /Não foi possível carregar os diagnósticos/);
  assert.match(web, /let reason = if !idle \{/);
  assert.match(original, /textContent = !wk && w\.idle_reason/);
  assert.match(original, /w\.idle_reason/);
  assert.match(modern, /w\.idle_reason/);
  assert.match(modern, /s\.ops\?\.blocked_hunts/);
});

test("npm cache is persistent and npm ci is never silently bypassed", () => {
  assert.match(shell, /apollo-office-npm-downloads:\/var\/cache\/apollo-npm/);
  const workerLaunch = shell.slice(shell.indexOf("pub fn dkup("), shell.indexOf("pub async fn orph("));
  assert.doesNotMatch(workerLaunch, /-v apollo-office-npm:\/root\/\.npm/);
  assert.match(shell, /fn isolated_npm_cache_setup\(/);
  assert.match(shell, /mktemp -d \/tmp\/apollo-npm-/);
  assert.match(shell, /ln -s/);
  assert.match(shell, /export npm_config_cache/);
  assert.match(shell, /trap 'rm -rf/);
  assert.match(shell, /npm cache clean --force/);
  assert.match(shell, /rm -rf --/);
  assert.match(shell, /_npx/);
  assert.match(pipe, /clean_idle_npm_cache\(&a, &h\)\.await/);
  assert.match(pipe, /q\(&su\)/);
});
