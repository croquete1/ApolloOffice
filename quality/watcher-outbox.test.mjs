import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {runInNewContext} from "node:vm";
import {test} from "node:test";
const source=readFileSync(new URL("../ui/p.js",import.meta.url),"utf8");
const start=source.indexOf("  // Owner diagnostics: text is escaped");
const end=source.indexOf("  let outboxRequest=",start);
assert.ok(start>=0&&end>start);
const esc=v=>String(v??"").replaceAll("&","&amp;").replaceAll("<","&lt;").replaceAll(">","&gt;").replaceAll('"',"&quot;");
const render=runInNewContext("(()=>{"+source.slice(start,end)+";return outboxRows;})()",{esc});
test("uncertain delivery has a verification action, never a resend or cancellation",()=>{
  const html=render([{repo:"o/r",num:1,state:"uncertain",author:"owner",body:"reply"}]);
  assert.match(html,/data-outbox-action="verify"/);
  assert.doesNotMatch(html,/data-outbox-action="cancel"|data-outbox-action="resend"/);
});
test("pending only can cancel; terminal entries are read-only and all stored text is escaped",()=>{
  const row={repo:"<repo>",num:1,author:"<owner>",body:"<script>"};
  const pending=render([{...row,state:"pending"}]);
  assert.match(pending,/data-outbox-action="cancel"/);
  assert.match(pending,/&lt;script&gt;/);
  assert.match(pending,/&lt;repo&gt;/);
  assert.match(pending,/&lt;owner&gt;/);
  assert.doesNotMatch(pending,/<script>/);
  assert.doesNotMatch(render([{...row,state:"sent"},{...row,state:"cancelled"}]),/data-outbox-action=/);
});
