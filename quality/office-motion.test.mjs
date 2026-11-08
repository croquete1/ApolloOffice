// Run with: node --test quality/office-motion.test.mjs
// Real interaction tests for the presentation-only handoff controller.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/office-motion.js", import.meta.url), "utf8");

function harness() {
  const listeners = {};
  const motion = { checked: true, addEventListener: (name, fn) => { listeners["motion:" + name] = fn; } };
  const prefers = { matches: false, addEventListener: (name, fn) => { listeners["reduced:" + name] = fn; } };
  const app = { isConnected: true, classList: { contains: () => false } };
  const classes = () => {
    const entries = new Set();
    return {
      add: (name) => entries.add(name),
      remove: (name) => entries.delete(name),
      contains: (name) => entries.has(name)
    };
  };
  const agents = Object.fromEntries(["fixer", "ci", "reviewer", "submit", "vet", "hunter", "repro", "guard"].map((name, index) => [
    name,
    {
      classList: classes(),
      getBoundingClientRect: () => ({ left: 20 + index * 90, top: 10, width: 70, height: 95 }),
      querySelector: (selector) => selector === ".ao-person"
        ? { cloneNode: () => ({ querySelectorAll: () => [] }) } : null
    }
  ]));
  const visits = [];
  let indicator;
  const stage = {
    isConnected: true,
    querySelector: (selector) => agents[selector.match(/data-role="([^"]+)"/)?.[1]] ?? null,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 900, height: 210 }),
    appendChild: (node) => { visits.push(node); }
  };
  const office = { appendChild: (node) => { indicator = node; } };
  const document = {
    visibilityState: "visible",
    hidden: false,
    getElementById: (id) => ({
      aoControlRoom: app, aoStations: stage, aoMotion: motion
    }[id]),
    querySelector: (selector) => selector === ".ao-office" ? office : null,
    addEventListener: (name, fn) => { listeners["document:" + name] = fn; },
    createElement: () => {
      const node = {
        style: {}, removed: false, hidden: false,
        setAttribute() {},
        appendChild() {},
        remove() { this.removed = true; },
        animate(frames, options) {
          this.frames = frames;
          this.options = options;
          const animation = {
            onfinish: null, oncancel: null,
            cancel() { this.oncancel?.(); }
          };
          this.animation = animation;
          return animation;
        }
      };
      return node;
    }
  };
  let oldCalls = 0;
  const subscribers = new Set();
  const AO = {
    modernRoom: true,
    NM: { fixer: "Fixer", ci: "CI", reviewer: "Reviewer", submit: "Submit" },
    onState: () => { oldCalls++; },
    subscribeState: (fn) => { subscribers.add(fn); return () => subscribers.delete(fn); }
  };
  runInNewContext(source, {
    window: { AO, matchMedia: () => prefers },
    document,
    console
  });
  const send = (q, o = {}) => {
    const state = { q, o };
    AO.S = state;
    AO.onState(state);
    for (const listener of subscribers) listener(state);
  };
  return { visits, send, agents, motion, prefers, app, document, listeners, AO, get indicator() { return indicator; }, get oldCalls() { return oldCalls; } };
}

test("initial state never creates imaginary handoffs", () => {
  const h = harness();
  h.send([{ id: 9, st: "fix", title: "Corrigir" }]);
  assert.equal(h.visits.length, 0);
  assert.equal(h.oldCalls, 1);
});

test("same job moving from Fixer to CI triggers one walk and cleans up", () => {
  const h = harness();
  h.send([{ id: 12, st: "fix", title: "Corrigir subtract" }]);
  h.send([{ id: 12, st: "ci", title: "Corrigir subtract" }]);
  assert.equal(h.visits.length, 1);
  const walker = h.visits[0];
  assert.equal(walker.options.duration, 2800);
  assert.equal(walker.frames.length, 5);
  assert.equal(h.agents.fixer.classList.contains("ao-travelling"), true);
  assert.equal(h.agents.ci.classList.contains("ao-receiving"), true);
  assert.match(h.indicator.textContent, /Fixer → CI · tarefa #12/);
  assert.equal(h.indicator.hidden, false);
  walker.animation.onfinish();
  assert.equal(walker.removed, true);
  assert.equal(h.agents.fixer.classList.contains("ao-travelling"), false);
  assert.equal(h.agents.ci.classList.contains("ao-receiving"), false);
  assert.equal(h.indicator.hidden, true);
  assert.equal(h.oldCalls, 2);
});

test("another job or unchanged stage cannot trigger a walk", () => {
  const h = harness();
  h.send([{ id: 1, st: "fix" }]);
  h.send([{ id: 2, st: "ci" }]);
  h.send([{ id: 2, st: "ci" }]);
  assert.equal(h.visits.length, 0);
});

test("same agent stage change is not a handoff", () => {
  const h = harness();
  h.send([{ id: 8, st: "gate" }]);
  h.send([{ id: 8, st: "submit" }]);
  assert.equal(h.visits.length, 0);
});

test("paused, offline or motion-disabled sessions do not animate", () => {
  const h = harness();
  h.send([{ id: 1, st: "fix" }]);
  h.send([{ id: 1, st: "ci" }], {man: true});
  assert.equal(h.visits.length, 0);
  h.send([{ id: 1, st: "fix" }], {off: true});
  assert.equal(h.visits.length, 0);
  h.motion.checked = false;
  h.send([{ id: 1, st: "ci" }]);
  assert.equal(h.visits.length, 0);
});

test("system reduced-motion setting prevents new walks", () => {
  const h = harness();
  h.send([{ id: 1, st: "fix" }]);
  h.prefers.matches = true;
  h.send([{ id: 1, st: "ci" }]);
  assert.equal(h.visits.length, 0);
});

test("disabling motion cancels a walk and restores the source avatar", () => {
  const h = harness();
  h.send([{ id: 4, st: "fix" }]);
  h.send([{ id: 4, st: "ci" }]);
  const walker = h.visits[0];
  h.motion.checked = false;
  h.listeners["motion:change"]();
  assert.equal(walker.removed, true);
  assert.equal(h.agents.fixer.classList.contains("ao-travelling"), false);
});

test("hidden tabs never animate transitions", () => {
  const h = harness();
  h.send([{ id: 1, st: "fix" }]);
  h.document.visibilityState = "hidden";
  h.send([{ id: 1, st: "ci" }]);
  assert.equal(h.visits.length, 0);
});


test("failing Web Animations API restores both agents without aborting updates", () => {
  const h = harness();
  const create = h.document.createElement;
  h.document.createElement = (...args) => {
    const element = create(...args);
    element.animate = () => { throw new Error("animation unavailable"); };
    return element;
  };
  h.send([{ id: 5, st: "fix" }]);
  h.send([{ id: 5, st: "ci" }]);
  assert.equal(h.visits.length, 1);
  assert.equal(h.visits[0].removed, true);
  assert.equal(h.agents.fixer.classList.contains("ao-travelling"), false);
  assert.equal(h.agents.ci.classList.contains("ao-receiving"), false);
});

test("simultaneous changes never pile several walkers on top of each other", () => {
  const h = harness();
  h.send([{id:1,st:"fix"},{id:2,st:"ci"}]);
  h.send([{id:1,st:"ci"},{id:2,st:"review"}]);
  assert.equal(h.visits.length, 1);
  h.send([{id:1,st:"review"},{id:2,st:"gate"}]);
  assert.equal(h.visits.length, 1);
});
