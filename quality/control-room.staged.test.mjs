// node --test quality/control-room.staged.test.mjs
// Exercise the staged display resolver with API snapshots, without starting jobs.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/control-room.js", import.meta.url), "utf8");
const from = source.indexOf("  const stagedPhases = [");
const to = source.indexOf("  let lastStagedSignature", from);
assert.ok(from >= 0 && to > from, "staged pipeline resolver must be present");
const implementation = source.slice(from, to);
const resolve = runInNewContext(`(() => {
  const safeArray = (value) => Array.isArray(value) ? value : [];
  ${implementation}
  return resolveStagedPipeline;
})()`);

const config = {
  active: true,
  repos: ["o/r"],
  primary: "ollama:qwen3:8b",
  fallback: "codex:gpt-6-luna",
  fallback_effort: "max",
  reviewer: "cc:opus"
};
const snapshot = (stage, say = "", fields = {}) => ({
  tdd: config,
  q: [{id: 1, repo: "o/r", kind: "fix", st: stage, why: "", ...fields}],
  dn: [],
  ev: [],
  w: {fixer: {job: 1, say}}
});
const states = (view) => Array.from(view.phases, (phase) => phase.state).join(",");

test("RED and GREEN reflect the Fixer's current controller message", () => {
  const red = resolve(snapshot("fix", "RED: writing the failing test"));
  assert.equal(red.status, "RED em execução");
  assert.equal(states(red), "active,pending,pending,pending,pending");

  const green = resolve(snapshot("fix", "GREEN: implementing the fix", {
    red_stage: "committed", red_commit: "red-sha"
  }));
  assert.equal(green.status, "GREEN em execução");
  assert.equal(states(green), "complete,active,pending,pending,pending");
});

test("CI, independent review, and human approval follow the real job status", () => {
  const ci = resolve(snapshot("ci", "", {red_commit:"red", green_commit:"green"}));
  assert.equal(ci.status, "CI em execução");
  assert.equal(states(ci), "complete,complete,active,pending,pending");

  const review = resolve(snapshot("review", "", {red_commit:"red", green_commit:"green"}));
  assert.equal(review.status, "Revisão independente");
  assert.equal(states(review), "complete,complete,complete,active,pending");

  const approval = resolve(snapshot("approve", "", {red_commit:"red", green_commit:"green"}));
  assert.equal(approval.status, "Aprovação humana pendente");
  assert.equal(states(approval), "complete,complete,complete,complete,active");
});

test("rejection and crash recovery appear only when reported by the job", () => {
  const rejection = snapshot("fix", "back to Fixer: Reviewer objections");
  rejection.ev = [{j:1,m:"back to Fixer: Reviewer objections"}];
  assert.equal(resolve(rejection).notice.kind, "rejection");

  const recovery = snapshot("fix", "resuming from the preserved RED commit");
  recovery.ev = [{j:1,m:"resuming from the preserved RED commit"}];
  assert.equal(resolve(recovery).notice.kind, "recovery");
});

test("disabled routes and unrelated repositories do not display staged activity", () => {
  const disabled = snapshot("approve");
  disabled.tdd = {...config, active:false};
  assert.equal(resolve(disabled).visible, false);

  const unrelated = snapshot("approve");
  unrelated.q[0].repo = "o/other";
  const waiting = resolve(unrelated);
  assert.equal(waiting.visible, true);
  assert.equal(waiting.job, null);
  assert.equal(waiting.status, "À espera de tarefa staged");
  assert.equal(states(waiting), "pending,pending,pending,pending,pending");
});
