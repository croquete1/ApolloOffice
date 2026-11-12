import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const settings = readFileSync(new URL("../ui/p.js", import.meta.url), "utf8");
const web = readFileSync(new URL("../src/web.rs", import.meta.url), "utf8");
const updater = readFileSync(new URL("../src/upd.rs", import.meta.url), "utf8");

test("settings explicitly distinguishes source main from installable signed releases", () => {
  assert.ok(settings.includes("Verificar versão assinada"));
  assert.ok(settings.includes("Comparar com a main do GitHub"));
  assert.ok(settings.includes("Verificar não instala"));
  assert.ok(settings.includes("ainda não é uma atualização assinada instalável"));
  assert.ok(settings.includes("python3 ~/Downloads/atualizar-apollo-fixflow.py"));
  assert.ok(!settings.includes("Up to date (latest"));
});

test("source comparison API cannot install or stage an unsigned binary", () => {
  assert.ok(web.includes('"check-main" => match crate::upd::check_main().await'));
  const main = updater.slice(
    updater.indexOf("pub async fn check_main()"),
    updater.indexOf("/// Downloads a bounded asset")
  );
  assert.ok(main.includes("main_comparison(&main, &release, &compare)"));
  assert.ok(!main.includes("std::process::Command"));
  assert.ok(!main.includes("std::fs::write"));
  assert.ok(!main.includes("staged("));
  assert.ok(!main.includes("apply("));
  assert.ok(!main.includes("install("));
  assert.match(main, /compare\/\{tag\}\.\.\.\{head\}/);
  assert.doesNotMatch(main, /\.head_commit\.sha/);
});

test("main comparison reports divergence accurately and preserves the open settings form", () => {
  assert.match(settings, /r\.status === "ahead"/);
  assert.match(settings, /r\.status === "diverged"/);
  assert.match(settings, /\$\{ahead\} commit\(s\).*\$\{behind\} commit\(s\)/);
  const handler = settings.slice(settings.indexOf('if ($("umc"))'), settings.indexOf('if ($("ua"))'));
  assert.doesNotMatch(handler, /settings\(\)/);
  assert.match(handler, /busy\(button, false/);
  assert.match(handler, /refreshMainMessage\(\)/);
});

test("signed release apply continues to require verified binary and explicit pause", () => {
  assert.ok(web.includes('if !crate::upd::ready(a)'));
  assert.ok(web.includes('if !crate::upd::manually_paused(a)'));
  assert.ok(web.includes('if !crate::upd::idle(a)'));
  assert.ok(web.includes('if !crate::upd::apply(a, true)'));
});
