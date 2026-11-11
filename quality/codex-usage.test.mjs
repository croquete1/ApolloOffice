// Regression: a Codex snapshot can arrive before the quota script is loaded.
// Run with: node --test quality/codex-usage.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { Script } from "node:vm";

const script = readFileSync(new URL("../ui/codex-usage.js", import.meta.url), "utf8");

function element() {
  return {
    textContent: "",
    style: {},
    dataset: {},
    attributes: {},
    isConnected: true,
    classList: { toggle() {} },
    setAttribute(key, value) { this.attributes[key] = value; },
    removeAttribute(key) { delete this.attributes[key]; },
    querySelector() { return null; }
  };
}

function quotaWindow() {
  const node = element();
  const amount = element();
  const bar = element();
  const fill = element();
  const reset = element();
  bar.querySelector = (selector) => selector === "i" ? fill : null;
  node.querySelector = (selector) => ({
    "[data-remaining]": amount,
    '[role="progressbar"]': bar,
    "[data-reset]": reset
  })[selector] || null;
  return node;
}

function createHarness(initial) {
  const status = element();
  const footer = element();
  const forecast = element();
  const windows = { five_hour: quotaWindow(), weekly: quotaWindow() };
  const panel = element();
  panel.querySelector = (selector) => ({
    "#codexQuotaStatus": status,
    "#codexQuotaUpdated": footer,
    "#codexWeeklyForecast": forecast,
    '[data-codex-window="five_hour"]': windows.five_hour,
    '[data-codex-window="weekly"]': windows.weekly
  })[selector] || null;
  const side = {
    querySelector() { return null; },
    append(child) { this.child = child; }
  };
  let listener;
  const AO = {
    S: initial,
    subscribeState(fn) { listener = fn; return () => {}; }
  };
  const document = {
    getElementById() { return null; },
    querySelector(selector) { return selector === ".side" ? side : null; },
    createElement() { return panel; }
  };
  const context = {
    window: { AO }, document, Intl, Date,
    performance: { now: () => 1000 },
    setInterval: () => 1, clearInterval() {}
  };
  new Script(script, { filename: "codex-usage.js" }).runInNewContext(context);
  return { status, forecast, send: (next) => listener(next), side };
}

function snapshot(status) {
  const now = Date.UTC(2026, 10, 11, 9, 0, 0);
  return {
    t: now,
    codex_usage: {
      status: "ok",
      updated_at: now,
      five_hour: { remaining: 88, resets_at: now / 1000 + 1800 },
      weekly: { remaining: 59, resets_at: now / 1000 + 400000 },
      weekly_forecast: {
        status,
        observed_hours: 12,
        projected_exhaust_at: now / 1000 + 72000,
        estimated: true
      }
    }
  };
}

test("late quota subscriber replays the initial near-limit snapshot", () => {
  const h = createHarness(snapshot("near_limit"));
  assert.equal(h.status.textContent, "Atualizado");
  assert.match(h.forecast.textContent, /Previsão próxima da reposição/);
  assert.match(h.forecast.textContent, /Estimativa, não garantia/);
  assert.equal(h.forecast.dataset.risk, "near");
});

test("future server snapshots replace the replayed forecast", () => {
  const h = createHarness(snapshot("near_limit"));
  h.send(snapshot("at_risk"));
  assert.match(h.forecast.textContent, /Risco de esgotamento/);
  assert.equal(h.forecast.dataset.risk, "risk");
  const stale = snapshot("near_limit");
  stale.codex_usage.status = "stale";
  stale.codex_usage.updated_at -= 30 * 60_000;
  h.send(stale);
  assert.match(h.forecast.textContent, /Previsão suspensa/);
  assert.equal(h.forecast.dataset.risk, "unknown");
});
