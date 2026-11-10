import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/codex-limits.js", import.meta.url), "utf8");
const ids = [
  "codexLimits", "codexUpdated",
  "codexFivePct", "codexFiveReset", "codexFiveFill", "codexFiveMeter", "codexFiveRow",
  "codexWeekPct", "codexWeekReset", "codexWeekFill", "codexWeekMeter", "codexWeekRow"
];

function makePanel(payload, modern = false) {
  const nodes = new Map(ids.map(id => [id, {
    id, textContent: "", title: "", style: {},
    attributes: {}, setAttribute(k, v) { this.attributes[k] = v; },
    removeAttribute(k) { delete this.attributes[k]; },
    classList: { add() {}, remove() {}, toggle() {} }
  }]));
  let moved = null;
  const now = Date.now();
  const doc = {
    hidden: false,
    getElementById(id) { return nodes.get(id); },
    querySelector() { return { prepend(element) { moved = element; } }; },
    addEventListener() {}
  };
  const waits = [];
  const context = {
    document: doc,
    window: { AO: { modernRoom: modern } },
    fetch: async () => ({ ok: true, json: async () => ({
      status: "ok", checkedAt: now, serverTime: now,
      fiveHour: { remainingPercent: 88, resetsAt: Math.floor((now + 18 * 60_000) / 1000) },
      weekly: { remainingPercent: 59, resetsAt: Math.floor((now + 5 * 86_400_000) / 1000) },
      ...payload
    }) }),
    performance: { now: () => 1_000 },
    setInterval: callback => { waits.push(callback); return waits.length; },
    Date, Math
  };
  runInNewContext(source, context);
  return { nodes, waits, get moved() { return moved; } };
}

const settle = () => new Promise(resolve => setImmediate(resolve));

test("Codex displays actual remaining percentages and reset times", async () => {
  const ui = makePanel();
  await settle();
  assert.equal(ui.nodes.get("codexFivePct").textContent, "88%");
  assert.equal(ui.nodes.get("codexWeekPct").textContent, "59%");
  assert.match(ui.nodes.get("codexFiveReset").textContent, /min/);
  assert.match(ui.nodes.get("codexWeekReset").textContent, /d/);
  assert.equal(ui.nodes.get("codexFiveFill").style.width, "88%");
  assert.equal(ui.nodes.get("codexWeekMeter").attributes["aria-valuenow"], "59");
  assert.equal(ui.nodes.get("codexUpdated").textContent, "Atualizado");
});

test("Unavailable windows are not invented and never display fake percentages", async () => {
  const ui = makePanel({ weekly: null });
  await settle();
  assert.equal(ui.nodes.get("codexFivePct").textContent, "88%");
  assert.equal(ui.nodes.get("codexWeekPct").textContent, "—");
  assert.equal(ui.nodes.get("codexWeekReset").textContent, "Sem dados");
  assert.equal(ui.nodes.get("codexWeekMeter").attributes["aria-valuenow"], undefined);
});

test("Optional modern view reuses the same quota panel", async () => {
  const ui = makePanel({}, true);
  await settle();
  assert.equal(ui.moved, ui.nodes.get("codexLimits"));
});

test("Errors show unavailable, not outdated quota values", async () => {
  const ui = makePanel({ status: "unavailable", message: "Sessão não autenticada" });
  await settle();
  assert.equal(ui.nodes.get("codexUpdated").textContent, "Indisponível");
  assert.equal(ui.nodes.get("codexFivePct").textContent, "—");
  assert.equal(ui.nodes.get("codexWeekPct").textContent, "—");
});
