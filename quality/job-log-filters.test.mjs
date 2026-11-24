import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {test} from "node:test";
import {runInNewContext} from "node:vm";
const source=readFileSync(new URL("../ui/p.js",import.meta.url),"utf8");
const start=source.indexOf("  // Task drawer logs only:");
const end=source.indexOf("  const job = async",start);
assert.ok(start>=0 && end>start);
const events=[{t:0,w:"fixer",m:"Build FAILED\nline 2"},{t:1,w:"ci",m:"Build passed"},{t:2,w:"ci",m:"<script>"}];
function fixture(){
  const nodes=Object.fromEntries(["jobLogWorker","jobLogSearch","jobLogWrap","jobLogRows","jobLogCount","jobLogRefresh","jobLogStatus"].map(k=>[k,{value:"",checked:true,disabled:false,classList:{toggle(name,on){this[name]=on;}}}]));
  const esc=v=>String(v??"").replaceAll("&","&amp;").replaceAll("<","&lt;").replaceAll(">","&gt;").replaceAll('"',"&quot;");
  const fn=runInNewContext("(()=>{"+source.slice(start,end)+";return {filterJobLogs,jobLogPanel,bindJobLogs};})()",{esc,$:id=>nodes[id]});
  return {fn,nodes};
}
test("search and worker filters combine, ignoring case and surrounding whitespace",()=>{
  const {fn}=fixture();
  assert.equal(fn.filterJobLogs(events,"ci"," BUILD ").length,1);
  assert.equal(fn.filterJobLogs(events,"fixer","failed").length,1);
  assert.equal(fn.filterJobLogs(events,"","").length,3);
  assert.equal(fn.filterJobLogs(events,"ci","missing").length,0);
});
test("drawer controls update counts, escape content and toggle wrapping",()=>{
  const {fn,nodes}=fixture();
  assert.match(fn.jobLogPanel(events),/value="fixer"/);
  fn.bindJobLogs(events);
  assert.equal(nodes.jobLogCount.textContent,"3 de 3 mensagens recebidas");
  assert.match(nodes.jobLogRows.innerHTML,/&lt;script&gt;/);
  nodes.jobLogWorker.value="ci"; nodes.jobLogWorker.onchange();
  assert.equal(nodes.jobLogCount.textContent,"2 de 3 mensagens recebidas");
  nodes.jobLogSearch.value="missing";nodes.jobLogSearch.oninput();
  assert.match(nodes.jobLogRows.innerHTML,/Sem mensagens/);
  nodes.jobLogWrap.checked=false;nodes.jobLogWrap.onchange();
  assert.equal(nodes.jobLogRows.classList["job-log-nowrap"],true);
});

test("refresh preserves filters, wrap and prior logs on failure",async()=>{
  const {fn,nodes}=fixture();
  let next=[{t:3,w:"ci",m:"new <script>"}], fail=false;
  fn.bindJobLogs(events,async()=>{if(fail)throw Error("network");return next;});
  nodes.jobLogWorker.value="ci";nodes.jobLogSearch.value="new";nodes.jobLogWrap.checked=false;
  await nodes.jobLogRefresh.onclick();
  assert.equal(nodes.jobLogCount.textContent,"1 de 1 mensagens recebidas");
  assert.match(nodes.jobLogRows.innerHTML,/&lt;script&gt;/);
  assert.equal(nodes.jobLogWorker.value,"ci");
  assert.equal(nodes.jobLogSearch.value,"new");
  assert.equal(nodes.jobLogRows.classList["job-log-nowrap"],true);
  fail=true;await nodes.jobLogRefresh.onclick();
  assert.match(nodes.jobLogRows.innerHTML,/&lt;script&gt;/);
  assert.match(nodes.jobLogStatus.textContent,/Mantido/);
  fail=false;next=[{t:4,w:"fixer",m:"another"}];
  await nodes.jobLogRefresh.onclick();
  assert.match(nodes.jobLogWorker.innerHTML,/value="ci"/);
  assert.equal(nodes.jobLogCount.textContent,"0 de 1 mensagens recebidas");
});
test("refresh deduplicates in-flight requests and ignores a replaced drawer",async()=>{
  const {fn,nodes}=fixture();
  let resolve, calls=0;
  fn.bindJobLogs(events,()=>{calls++;return new Promise(r=>{resolve=r;});});
  const previous=nodes.jobLogRows;
  const pending=nodes.jobLogRefresh.onclick();
  await nodes.jobLogRefresh.onclick();
  assert.equal(calls,1);
  nodes.jobLogRows={innerHTML:"new drawer"};
  resolve([{t:4,w:"ci",m:"stale result"}]);
  await pending;
  assert.equal(nodes.jobLogRows.innerHTML,"new drawer");
  assert.doesNotMatch(previous.innerHTML,/stale result/);
  assert.equal(nodes.jobLogRefresh.disabled,false);
});
