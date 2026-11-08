// node --test quality/control-room.worker.test.mjs
// Exercises the exact worker state mapper without a DOM or network access.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/control-room.js", import.meta.url), "utf8");
const start = source.indexOf("  // External provider/network failures");
const finish = source.indexOf("  const resultText =", start);
assert.ok(start >= 0 && finish > start, "authoritative worker state mapper is required");
const mapping = source.slice(start, finish);

const state = (st, opts = {}) => ({o: opts.o || {}, w: {fixer:{st, hold:!!opts.hold}}, q:opts.q || []});
const fixture = () => runInNewContext(`(() => {
  ${mapping}
  return {mode, statusText, paused};
})()`);

test("no server worker state remains unknown", () => {
  const {mode} = fixture();
  assert.equal(mode({w:null,q:[{st:"fix"}]}, "fixer"), "unknown");
  assert.equal(mode({w:{},q:[{st:"fix"}]}, "fixer"), "unknown");
});

test("a queued job alone never changes an idle worker into work", () => {
  const {mode} = fixture();
  assert.equal(mode(state("idle",{q:[{id:17,st:"fix"}]}), "fixer"), "idle");
});

test("work, idle and sleep reflect the server's exact role state", () => {
  const {mode} = fixture();
  for(const name of ["work","idle","sleep"]) assert.equal(mode(state(name), "fixer"), name);
  assert.equal(mode(state("unexpected")), "unknown");
});

test("explicit holds and office pause override work", () => {
  const {mode} = fixture();
  assert.equal(mode(state("work",{hold:true}), "fixer"), "paused");
  assert.equal(mode(state("work",{o:{man:true}}), "fixer"), "paused");
  assert.equal(mode(state("work",{o:{pause:"budget"}}), "fixer"), "paused");
});

test("external service outage does not falsely pause an active local agent", () => {
  const {mode} = fixture();
  assert.equal(mode(state("work",{o:{off:true,net:{an:false}}}), "fixer"), "work");
});
