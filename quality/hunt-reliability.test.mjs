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

test("diagnostic UI scripts parse and expose authenticated error history", () => {
  assert.doesNotThrow(() => new Script(original));
  assert.doesNotThrow(() => new Script(panels));
  assert.match(web, /\("GET", "\/api\/failures"\)/);
  assert.match(web, /"failure": j\["d"\]\["failure"\]/);
  assert.match(panels, /data-reset-hunt/);
  assert.match(panels, /Diagnóstico técnico/);
});

test("repeated setup failures stop new hunts without disabling fixes", () => {
  assert.match(pipe, /crate::hunt_health::blocked\(a, r\)/);
  assert.match(pipe, /j\.kind == "hunt"/);
  assert.match(health, /const LIMIT: i64 = 3/);
  assert.match(health, /const COOLDOWN_MS: i64 = 30 \* 60 \* 1000/);
  assert.match(health, /pub fn success/);
});

test("preflight runs before cloning and existing setup remains mandatory", () => {
  const prep = pipe.slice(pipe.indexOf("async fn prep("), pipe.indexOf("fn rn(", pipe.indexOf("async fn prep(")));
  assert.ok(prep.indexOf("setup_tool(&su)") < prep.indexOf("let mt = crate::gh::meta"));
  assert.match(prep, /command -v node/);
  assert.match(prep, /crate::hunt_health::diagnostic\(a, j, "setup"/);
  assert.match(prep, /"to 1800 bash -c/);
});

test("fix jobs outrank hunts and capacity is reserved for pending fixes", () => {
  assert.ok(pipe.indexOf('"fix" => 1.0') < pipe.indexOf('"hunt" => 2.0'));
  assert.match(pipe, /pending_fixes > 0/);
  assert.match(web, /"idle_reason"/);
  assert.match(original, /w\.idle_reason/);
});

test("npm cache is persistent and npm ci is never silently bypassed", () => {
  assert.match(shell, /apollo-office-npm:\/root\/\.npm/);
  assert.match(pipe, /q\(&su\)/);
});
