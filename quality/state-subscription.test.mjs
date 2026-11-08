// Run with: node --test quality/state-subscription.test.mjs
// Exercises the exact subscription code embedded in ui/o.js without a browser.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/o.js", import.meta.url), "utf8");
const start = source.indexOf("  const stateListeners = new Set();");
const end = source.indexOf("  const open = (u) =>", start);

function harness() {
  assert.ok(start >= 0 && end > start, "state subscription block must exist");
  const block = source.slice(start, end);
  return runInNewContext(`(() => {
    const window = { AO: {} };
    const errors = [];
    const console = { error: (...items) => errors.push(items) };
    ${block}
    return { subscribe: window.AO.subscribeState, notify: notifyState, errors };
  })()`);
}

test("two independent subscribers receive every state in order", () => {
  const h = harness(), output = [];
  h.subscribe((s) => output.push("first:" + s.id));
  h.subscribe((s) => output.push("second:" + s.id));
  h.notify({ id: 13 });
  assert.deepEqual(output, ["first:13", "second:13"]);
});

test("one failed subscriber does not prevent other UI components updating", () => {
  const h = harness(), output = [];
  h.subscribe(() => { throw new Error("broken dashboard"); });
  h.subscribe((s) => output.push(s.id));
  h.notify({ id: 27 });
  assert.deepEqual(output, [27]);
  assert.equal(h.errors.length, 1);
});

test("unsubscribe cleans up and does not affect other listeners", () => {
  const h = harness(), output = [];
  const off = h.subscribe((s) => output.push("gone:" + s.id));
  h.subscribe((s) => output.push("live:" + s.id));
  off(); off();
  h.notify({ id: 2 });
  assert.deepEqual(output, ["live:2"]);
});

test("non-function subscriber is rejected", () => {
  const h = harness();
  assert.throws(() => h.subscribe(null));
});

test("the legacy onState is preserved and subscribers notified even if it throws", () => {
  assert.match(source, /window\.AO\.onState\?\.\(s\)/);
  assert.match(source, /finally \{\s*notifyState\(s\)/);
});
