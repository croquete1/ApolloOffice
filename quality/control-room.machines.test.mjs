// node --test quality/control-room.machines.test.mjs
// Test the exact production machine renderer against a fixed clock.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/control-room.js", import.meta.url), "utf8");
const from = source.indexOf("  const renderMachines = (s) =>");
const to = source.indexOf("  const render = (s) =>", from);
assert.ok(from >= 0 && to > from, "machine renderer must exist");
const implementation = source.slice(from, to);
const now = 1_800_000_000_000;
class FixedDate extends Date { static now() { return now; } }

function fixture() {
  const root = {innerHTML:""};
  const api = runInNewContext(`(() => {
    let lastMachineSignature = "";
    const byId = (id) => {
      if (id !== "aoMachines") throw Error("unexpected DOM target");
      return __root;
    };
    const safeArray = (v) => Array.isArray(v) ? v : [];
    const escapeHTML = (value) => String(value ?? "").replace(/[&<>"]/g, (c) => ({
      "&":"&amp;", "<":"&lt;", ">":"&gt;", '"':"&quot;"
    })[c]);
    ${implementation}
    return {renderMachines};
  })()`, {__root:root, Date:FixedDate});
  return {root, render:api.renderMachines};
}
const base = (age, enabled = 1, available = true) => ({
  hs:[{id:"local",on_:enabled}],
  hst:{local:{t:now-age,cpu:8,load:1.2,temp:62,mem:[16384,8192],disk:[500,200]}},
  o:{net:{local:available}}
});

test("fresh active host displays its actual measurements", () => {
  const x=fixture();
  x.render(base(30000));
  assert.match(x.root.innerHTML, /Dados recentes/);
  assert.match(x.root.innerHTML, /8 núcleos/);
  assert.match(x.root.innerHTML, /62 °C/);
  assert.match(x.root.innerHTML, /8.0 \/ 16.0 GB RAM/);
  assert.doesNotMatch(x.root.innerHTML,/Última leitura/);
});

test("stale telemetry is labelled historical rather than shown as live", () => {
  const x=fixture();
  x.render(base(180000));
  assert.match(x.root.innerHTML,/Por confirmar/);
  assert.match(x.root.innerHTML,/Última leitura:/);
  assert.doesNotMatch(x.root.innerHTML,/62 °C/);
  assert.doesNotMatch(x.root.innerHTML,/8 núcleos/);
});

test("unavailable or disabled hosts never show outdated measurements as live", () => {
  const x=fixture();
  x.render(base(30000,1,false));
  assert.match(x.root.innerHTML,/Indisponível/);
  assert.doesNotMatch(x.root.innerHTML,/62 °C/);
  x.render(base(30000,0,true));
  assert.match(x.root.innerHTML,/Desativada/);
  assert.doesNotMatch(x.root.innerHTML,/62 °C/);
});

test("hosts without measurements are reported without fake performance values", () => {
  const x=fixture();
  x.render({hs:[{id:"local",on_:1}],hst:{},o:{net:{local:true}}});
  assert.match(x.root.innerHTML,/Sem leituras recentes/);
  assert.doesNotMatch(x.root.innerHTML,/GB RAM|°C/);
});

test("empty inventory displays a neutral status", () => {
  const x=fixture();
  x.render({hs:[],hst:{},o:{net:{}}});
  assert.match(x.root.innerHTML,/Ainda não existem máquinas configuradas/);
});
