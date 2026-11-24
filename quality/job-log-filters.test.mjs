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
  const nodes=Object.fromEntries(["jobLogWorker","jobLogSearch","jobLogWrap","jobLogRows","jobLogCount"].map(k=>[k,{value:"",checked:true,classList:{toggle(name,on){this[name]=on;}}}]));
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
