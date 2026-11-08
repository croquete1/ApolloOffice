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

test("retired pixel canvas stays hidden in modern and emergency fallback modes", () => {
  assert.match(original, /window\.AO\?\.modernRoom \|\| window\.AO\?\.disablePixelArt/);
  assert.match(modern, /A\.modernRoom = false/);
  assert.match(modern, /A\.disablePixelArt = true/);
  assert.match(modern, /ao-fallback-no-pixel/);
  assert.match(css, /body\.ao-fallback-no-pixel #cv/);
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


test("live events use an isolated subscription channel", () => {
  assert.match(original, /const stateListeners = new Set\(\)/);
  assert.match(original, /const notifyState = \(state\)/);
  assert.match(original, /try \{ listener\(state\); \} catch \(error\)/);
  assert.match(original, /finally \{\s*notifyState\(s\)/);
  assert.match(modern, /unsubscribe = A\.subscribeState/);
  assert.match(handoffs, /const unsubscribe = A\.subscribeState/);
});

test("actual command and mood telemetry drive visual reactions", () => {
  assert.match(modern, /worker\.cmds/);
  assert.match(modern, /lastCommands\(w\)/);
  assert.match(modern, /el\.dataset\.lastCommand !== commandKey/);
  assert.match(modern, /Object\.hasOwn\(resultText, w\.mood\)/);
  assert.match(modern, /label\.textContent = cmd\.slice\(0, 160\)/);
  assert.match(css, /ao-command-sweep/);
  assert.match(css, /ao-result-mark/);
});

test("metrics and host connectivity never imply unavailable totals", () => {
  assert.match(modern, /Total registado no sistema/);
  assert.match(modern, /Até 40 PRs registados/);
  assert.match(modern, /countTotals\(s\)/);
  assert.match(modern, /key !== "done" && key !== "drop"/);
  assert.match(modern, /age < 120_000/);
  assert.match(modern, /Dados recentes/);
  assert.match(modern, /Por confirmar/);
  assert.match(modern, /Serviços externos indisponíveis/);
});

test("buttons do not offer unavailable PR links", () => {
  assert.match(modern, /attr === "data-pr-url" && !val/);
  assert.match(modern, /disabled title="Ligação indisponível"/);
  assert.match(modern, /aria-expanded/);
});


test("initial loading and fallback do not leave zombie status timers", () => {
  assert.match(modern, /const startedAt = Date\.now\(\)/);
  assert.match(modern, /lastStateAt \|\| startedAt/);
  assert.match(modern, /if \(staleTimer !== null\) clearInterval\(staleTimer\)/);
  assert.match(modern, /!healthy \|\| !app\.isConnected/);
});


test("initial agent states are unknown rather than fictitious idle activity", () => {
  assert.match(modern, /data-mode="unknown"/);
  assert.match(modern, /A ligar…/);
  assert.match(css, /data-mode="unknown"/);
});

test("all pipeline values derive from the server's untruncated status counts", () => {
  assert.match(modern, /const pipelineSteps = \[/);
  assert.match(modern, /const renderPipeline = \(s\)/);
  assert.match(modern, /const counts = s\.n && typeof s\.n === "object" \? s\.n : \{\}/);
  assert.match(modern, /renderPipeline\(s\)/);
  assert.match(modern, /lastPipelineSignature/);
  assert.match(css, /\.ao-pipeline-stage\.is-busy/);
});

test("celebrations require a recently observed finished job and reduced-motion permission", () => {
  assert.match(modern, /const activeJob = w\.job;/);
  assert.match(modern, /String\(j\.id\) === priorJob/);
  assert.match(modern, /age >= 0 && age <= 60_000/);
  assert.match(modern, /prefers-reduced-motion: reduce/);
});


test("authoritative worker status wins over unassigned queued jobs", () => {
  assert.match(modern, /if \(w\.st === "work"\) return "work"/);
  assert.doesNotMatch(modern, /safeArray\(s\?\.q\)\.some/);
  assert.doesNotMatch(modern, /workerStages\[role\]/);
  assert.match(modern, /if \(!s\?\.w \|\| !Object\.hasOwn\(s\.w, role\)\) return "unknown"/);
  assert.match(modern, /const activeJob = w\.job;/);
});


test("fallback releases global keyboard and visibility listeners", () => {
  assert.match(modern, /document\.removeEventListener\("keydown", onModalKeydown\)/);
  assert.match(modern, /document\.removeEventListener\("visibilitychange", onVisibilityChange\)/);
  assert.match(modern, /A\.stopOfficeMotion\?\.\(\)/);
  assert.match(modern, /if \(!healthy\) return;\s*\/\/ Keep the connection/);
});

test("handoff engine supports explicit unmount disposal", () => {
  assert.match(handoffs, /const dispose = \(\) => \{/);
  assert.match(handoffs, /motionControl\?\.removeEventListener\?\./);
  assert.match(handoffs, /document\.removeEventListener\?\.\("visibilitychange"/);
  assert.match(handoffs, /A\.stopOfficeMotion = dispose/);
});


test("operational dashboard precedes the secondary virtual office", () => {
  const iMetrics = modern.indexOf('<section class="ao-metrics"');
  const iPipeline = modern.indexOf('<section class="ao-panel ao-pipeline-panel"');
  const iOperations = modern.indexOf('<section class="ao-operations-grid"');
  const iOffice = modern.indexOf('<section class="ao-primary-grid"');
  assert.ok(iMetrics >= 0 && iPipeline > iMetrics && iOperations > iPipeline && iOffice > iOperations);
});

test("approval is not conflated with automated gate and submission", () => {
  assert.match(modern, /label: "Validação", statuses: \["gate"\]/);
  assert.match(modern, /label: "Aprovação", statuses: \["approve"\]/);
  assert.match(modern, /label: "Submissão", statuses: \["submit"\]/);
  assert.doesNotMatch(modern, /label: "Aprovação", statuses: \["gate", "approve", "submit"\]/);
});
