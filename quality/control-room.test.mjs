// Dependency-free static checks for the opt-in Control Room frontend.
// Run with: node --test quality/control-room.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { Script } from "node:vm";

const read = (file) => readFileSync(new URL("../" + file, import.meta.url), "utf8");
const html = read("ui/index.html");
const css = read("ui/control-room.css");
const modern = read("ui/control-room.js");
const original = read("ui/o.js");
const handoffs = read("ui/office-motion.js");

test("frontend scripts parse without extra dependencies", () => {
  assert.doesNotThrow(() => new Script(modern, { filename: "control-room.js" }));
  assert.doesNotThrow(() => new Script(original, { filename: "o.js" }));
  assert.doesNotThrow(() => new Script(handoffs, { filename: "office-motion.js" }));
});

test("Control Room is loaded after the original application", () => {
  assert.match(html, /href="control-room\.css"/);
  const iO = html.indexOf('src="o.js"');
  const iP = html.indexOf('src="p.js"');
  const iModern = html.indexOf('src="control-room.js"');
  assert.ok(iO >= 0 && iP > iO && iModern > iP);
  const iWalk = html.indexOf('src="office-motion.js"');
  assert.ok(iWalk > iModern, "handoff code must load after the modern dashboard");
});

test("all ten original agents have distinct live stations", () => {
  for (const role of ["scout","guard","vet","watcher","hunter","repro","fixer","ci","reviewer","submit"]) {
    assert.match(modern, new RegExp("\\b" + role + "\\b"));
  }
  assert.match(modern, /const stationNodes =/);
  assert.match(modern, /s\.w\?\.\[role\]/);
  assert.match(original, /window\.AO\.subscribeState/);
  assert.match(modern, /A\.subscribeState\(\(s\)/);
  assert.match(handoffs, /A\.subscribeState\(\(s\)/);
});

test("motion can be disabled and stays accessible", () => {
  assert.match(css, /prefers-reduced-motion:\s*reduce/);
  assert.match(css, /\.ao-motion-disabled/);
  assert.match(modern, /aoMotion/);
  assert.match(modern, /aoAgentOverlay/);
  assert.match(modern, /aria-modal="true"/);
});

test("no fictional GPU data or percentages of task completion", () => {
  assert.doesNotMatch(modern, /GPU:\s*\d+|Efici[eê]ncia\s*\d+%|successRate/i);
  assert.match(modern, /s\.hst/);
  assert.match(modern, /s\.mg/);
  assert.doesNotMatch(modern, /Ações Rápidas|Nova tarefa|Executar testes/);
});

test("original canvas sleeps while hidden and remains available for fallback", () => {
  assert.match(original, /if \(window\.AO\?\.modernRoom\)/);
  assert.match(modern, /A\.modernRoom = false/);
  assert.match(modern, /legacy\.replaceWith/);
});


test("live lists are not rebuilt on identical status updates", () => {
  assert.match(modern, /lastListSignature/);
  assert.match(modern, /signature === lastListSignature/);
  assert.match(modern, /lastDetailSignature/);
});

test("handoffs are grounded and safely cancelled", () => {
  assert.match(handoffs, /prior\.stage === next\.stage/);
  assert.match(handoffs, /previous\.get\(id\)/);
  assert.match(handoffs, /stateAllowed\(s\)/);
  assert.match(handoffs, /animation\.oncancel = finish/);
  assert.match(handoffs, /finish\(\);\s*console\.warn/);
});
