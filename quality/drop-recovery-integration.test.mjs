import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const web = readFileSync(new URL("../src/web.rs", import.meta.url), "utf8");
const drawer = readFileSync(new URL("../ui/p.js", import.meta.url), "utf8");
const controls = readFileSync(new URL("../ui/o.js", import.meta.url), "utf8");
const recovery = readFileSync(new URL("../src/drop_recovery.rs", import.meta.url), "utf8");

test("requeue uses server-side guarded recovery rather than resetting a dropped job", () => {
  assert.match(web, /"requeue"\s*=>\s*match crate::drop_recovery::retry_manual\(a, id\)\.await/);
  assert.match(web, /Job alterado durante a recuperação/);
  assert.doesNotMatch(web, /"requeue"\s*=>\s*\{\s*if let Some\(j\) = a\.db\.jg\(id\)/);
  assert.match(recovery, /has_related_pr\(list, job\.num\)/);
  assert.match(recovery, /cache_default_head\(a, &job\.repo, &head\)\?/);
});

test("job details retain failure diagnostics and show blocked retry errors", () => {
  const jobDetails = drawer.slice(drawer.indexOf("const failure = j.failure?.stage"));
  assert.ok(jobDetails.length > 100, "job detail view is present");
  assert.match(jobDetails, /Diagnóstico técnico/);
  assert.ok(jobDetails.includes('data-requeue-message="${id}" aria-live="polite"'));
  assert.match(jobDetails, /\$\{failure\}/);
  assert.match(controls, /data-requeue-message/);
  assert.ok(controls.includes("result?.error"));
});

test("successful requeue clears the previous error before applying server state", () => {
  const handler = controls.slice(controls.indexOf('const rq = e.target.closest("[data-rq]")'));
  assert.ok(handler.includes("retryStates.delete(id)"));
  assert.ok(handler.includes('message("");'));
  assert.ok(handler.indexOf("retryStates.delete(id)") < handler.indexOf("apply(result)"));
});

test("retry remains disabled and errors remain attached to job ID across queue refreshes", () => {
  assert.ok(controls.includes("const retryStates = new Map()"));
  assert.ok(controls.includes("const retry = retryStates.get(j.id)"));
  assert.ok(controls.includes('retry?.pending ? "disabled"'));
  assert.ok(controls.includes("retry?.error"));
  assert.ok(controls.includes("retryStates.get(id)?.pending"));
  assert.ok(controls.includes("document.querySelectorAll("));
  assert.ok(controls.includes("retryStates.delete(id)"));
  assert.ok(drawer.includes("window.AO.retryStates?.get(id)?.pending"));
});

test("new main diagnostics API survives integration with safe dropped-job recovery", () => {
  assert.match(web, /\("GET", "\/api\/failures"\)/);
  assert.match(web, /coalesce\(why,''\) not in \('dropped by the owner','repository removed'\)/);
  assert.match(web, /"failure": j\["d"\]\["failure"\]/);
});
