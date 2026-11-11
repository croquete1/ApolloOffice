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
  assert.match(jobDetails, /data-requeue-message aria-live="polite"/);
  assert.match(jobDetails, /\$\{failure\}/);
  assert.match(controls, /data-requeue-message/);
  assert.match(controls, /result\.error/);
});

test("new main diagnostics API survives integration with safe dropped-job recovery", () => {
  assert.match(web, /\("GET", "\/api\/failures"\)/);
  assert.match(web, /coalesce\(why,''\) not in \('dropped by the owner','repository removed'\)/);
  assert.match(web, /"failure": j\["d"\]\["failure"\]/);
});
