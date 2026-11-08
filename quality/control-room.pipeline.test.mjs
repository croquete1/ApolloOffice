// node --test quality/control-room.pipeline.test.mjs
// Runs the exact pipeline/counting implementation against a DOM-free fixture.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/control-room.js", import.meta.url), "utf8");
const from = source.indexOf("  // Unlike the limited queue preview");
const to = source.indexOf("  const renderMerges = (s) =>", from);
assert.ok(from >= 0 && to > from, "pipeline implementation must be present");
const implementation = source.slice(from, to);

function fixture() {
  const stages = [];
  let updates = 0;
  const pipeline = {
    children: stages,
    set innerHTML(markup) {
      assert.match(markup, /ao-pipeline-stage/);
      stages.length = 0;
      for (let i = 0; i < 7; i++) {
        let active = false;
        const value = { textContent: "" };
        const attributes = {};
        stages.push({
          classList: { toggle(name, flag) {
            assert.equal(name, "is-busy");
            active = flag;
            updates++;
          } },
          querySelector(selector) {
            assert.equal(selector, ".ao-pipeline-value");
            return value;
          },
          setAttribute(name, text) {
            attributes[name] = text;
          },
          get active() { return active; },
          get count() { return value.textContent; },
          get label() { return attributes["aria-label"]; }
        });
      }
    }
  };
  const script = `(() => {
    let lastPipelineSignature = "";
    const byId = (id) => {
      if (id !== "aoPipeline") throw Error("unexpected DOM lookup");
      return pipeline;
    };
    const formatNum = (number) => Number(number || 0).toLocaleString("pt-PT");
    ${implementation}
    return { renderPipeline, countTotals, pipelineSteps };
  })()`;
  const api = runInNewContext(script, {pipeline});
  return { api, stages, get updates() { return updates; } };
}

test("pipeline uses real phase totals independently of the limited queue", () => {
  const x = fixture();
  x.api.renderPipeline({
    n: {vet:4,ready:1,hunt:2,repro:0,fix:3,ci:1,review:2,gate:1,approve:5,submit:1,done:26,drop:7},
    q: [{id: 1, st:"ci"}]
  });
  assert.deepEqual(x.stages.map((s) => s.count), ["5","2","0","3","1","2","7"]);
  assert.deepEqual(x.stages.map((s) => s.active), [true,true,false,true,true,true,true]);
  assert.match(x.stages[6].label, /Aprovação: 7 tarefas/);
});

test("active and finished metrics derive from the full status map", () => {
  const x = fixture();
  const totals = x.api.countTotals({
    n: {ready: 7, fix: 13, approve: 9, done: 88, drop: 11},
    q: []
  });
  assert.equal(totals.active, 29);
  assert.equal(totals.done, 88);
});

test("unchanged snapshots do not trigger DOM writes", () => {
  const x = fixture(), state = {n: {fix: 2, review: 4}};
  x.api.renderPipeline(state);
  const changes = x.updates;
  x.api.renderPipeline(state);
  assert.equal(x.updates, changes);
  x.api.renderPipeline({n: {fix: 3, review: 4}});
  assert.equal(x.updates, changes + 7);
});

test("absent or invalid numeric counts remain harmless", () => {
  const x = fixture();
  x.api.renderPipeline({n: {fix: -3,ci: "not-a-number",review: null}});
  assert.deepEqual(x.stages.map((s) => s.count), ["0","0","0","0","0","0","0"]);
  assert.equal(x.api.countTotals({n: {ready:-4,done: null}}).active, 0);
});
