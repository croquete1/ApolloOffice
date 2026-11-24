import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
const source=readFileSync(new URL("../ui/p.js",import.meta.url),"utf8");
const start=source.indexOf("  // Operational diagnostics only:");
const end=source.indexOf("  const health = async",start);
assert.ok(start>=0 && end>start);
const esc=v=>String(v??"").replaceAll("&","&amp;").replaceAll("<","&lt;").replaceAll(">","&gt;").replaceAll('"',"&quot;");
const render=runInNewContext("(()=>{"+source.slice(start,end)+";return resourceDiagnostics;})()",{esc});
const at=200000;
function state(){return {t:at,ops:{waiting:3,resources:{profile:"balanced",hosts:[{id:"<host>",enabled:true,active:2,effective_lanes:0,configured_lanes:4,active_ci:1,effective_ci:0,configured_ci:3,completion_ci_floor:1,reason:"critical_pressure",telemetry_fresh:true,gpu:[{index:0,vram_used_mib:1024,vram_total_mib:8192,temperature_c:null,utilization_percent:55}]}]}}};}
test("shows reported capacities, completion exception and escaped host",()=>{
  const out=render(state(),at);
  assert.match(out,/&lt;host&gt;/);
  assert.match(out,/CI: 1 em uso · 0\/3/);
  assert.match(out,/Uma vaga reservada/);
  assert.match(out,/Pressão crítica/);
  assert.match(out,/VRAM 1024\/8192 MiB · — °C/);
});
test("missing or stale metrics are unknown, not fake zero or stale GPU",()=>{
  assert.match(render(null,at),/indisponível/);
  assert.match(render(state(),at+180001),/desatualizado/);
  const s=state();s.ops.resources.hosts[0].telemetry_fresh=false;
  delete s.ops.resources.hosts[0].active_ci;
  const out=render(s,at);
  assert.match(out,/CI: — em uso/);
  assert.match(out,/GPU\/VRAM: sem leitura disponível/);
  assert.doesNotMatch(out,/1024\/8192/);
});
