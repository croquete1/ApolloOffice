// node --test quality/control-room.focus.test.mjs
// Execute the actual list renderer with a minimal focus-aware DOM.
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {test} from "node:test";
import {runInNewContext} from "node:vm";

const full = readFileSync(new URL("../ui/control-room.js", import.meta.url), "utf8");
const from = full.indexOf("  const rowHTML = (label, caption, badge");
const to = full.indexOf("  const renderRepos = (s) =>", from);
assert.ok(from !== -1 && to > from);
const implementation = full.slice(from, to);

function scene() {
  const document = {activeElement:null};
  const elements = {};
  const createList = (id) => {
    let html = "";
    const rows = [];
    const el = {
      id,
      querySelectorAll(selector) {
        const key = selector.includes("data-job") ? "data-job" : "data-pr-url";
        return rows.filter((row) => row.hasAttribute(key));
      },
      get innerHTML(){return html},
      set innerHTML(value) {
        html = value;
        rows.length = 0;
        for (const match of value.matchAll(/<button\s+[^>]*data-(job|pr-url)="([^"]+)"/g)) {
          const key = "data-" + match[1], val = match[2];
          const row = {
            getAttribute(attr){return attr===key ? val : null},
            hasAttribute(attr){return attr===key},
            matches(selector){return selector.includes("[" + key + "]")},
            closest(selector){return selector.includes("#" + id) ? el : null},
            focus(){document.activeElement=this}
          };
          rows.push(row);
        }
      },
      get rows(){return rows}
    };
    elements[id]=el;
  };
  createList("aoTasksList");
  createList("aoPRList");
  createList("aoEventList");
  for (const id of ["aoMetricQueue","aoMetricPR","aoMetricDone","aoTasks","aoPulls"]) {
    elements[id]={textContent:"",focus(){document.activeElement=this}};
  }
  const formatNum = (n)=>String(n);
  const safeArray = (v)=>Array.isArray(v)?v:[];
  const escapeHTML = (v)=>String(v ?? "").replace(/[&<>"]/g, c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
  const datum=()=> "10:00";
  const names = {};
  const stageLabels={fix:"Correção",ci:"Testes",approve:"Aprovação"};
  const byId=(id)=>elements[id];
  const context = {document,formatNum,safeArray,escapeHTML,datum,names,stageLabels,byId};
  const block = "(() => { let selectedRepo='', lastListSignature='';" +
    " const countTotals=(s)=>({active:s.n?.fix||0,done:s.n?.done||0}); " +
    implementation + " return {renderLists}; })()";
  const api = runInNewContext(block, context);
  const state=(q,pr=[])=>({q,pr,n:{fix:q.length,done:0},ev:[]});
  return {api,state,document,elements};
}

test("focused task is restored after a different item changes",()=>{
  const h=scene();
  h.api.renderLists(h.state([{id:42,title:"A",repo:"owner/repo",st:"fix"}]));
  const old=h.elements.aoTasksList.rows[0];
  old.focus();
  h.api.renderLists(h.state([{id:42,title:"A updated",repo:"owner/repo",st:"fix"}]));
  assert.notEqual(h.document.activeElement,old);
  assert.equal(h.document.activeElement.getAttribute("data-job"),"42");
});

test("when an item disappears focus returns to its section",()=>{
  const h=scene();
  h.api.renderLists(h.state([{id:18,title:"A",repo:"owner/repo",st:"fix"}]));
  h.elements.aoTasksList.rows[0].focus();
  h.api.renderLists(h.state([]));
  assert.equal(h.document.activeElement,h.elements.aoTasks);
});

test("focused pull request survives unrelated live updates",()=>{
  const h=scene();
  const pr={repo:"owner/repo",num:12,title:"PR",st:"open",url:"https://github.com/owner/repo/pull/12"};
  h.api.renderLists(h.state([], [pr]));
  h.elements.aoPRList.rows[0].focus();
  h.api.renderLists(h.state([{id:1,title:"Other",repo:"owner/repo",st:"fix"}],[pr]));
  assert.equal(h.document.activeElement.getAttribute("data-pr-url"),pr.url);
});

test("unfocused lists do not capture focus from sidebar",()=>{
  const h=scene();
  const sidebar={};
  h.document.activeElement=sidebar;
  h.api.renderLists(h.state([{id:1,title:"One",repo:"x",st:"fix"}]));
  h.api.renderLists(h.state([{id:2,title:"Two",repo:"x",st:"fix"}]));
  assert.equal(h.document.activeElement,sidebar);
});
