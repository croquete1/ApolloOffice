// node --test quality/control-room.repos.test.mjs
// Verify keyboard focus survives repository-filter selection and DOM replacement.
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {test} from "node:test";
import {runInNewContext} from "node:vm";

const source=readFileSync(new URL("../ui/control-room.js",import.meta.url),"utf8");
const from=source.indexOf("  const renderRepos = (s) =>");
const to=source.indexOf("  // Unlike the limited queue preview",from);
assert.ok(from>=0 && to>from);
const implementation=source.slice(from,to);

function fixture() {
  const document={activeElement:null};
  const wrap={
    children:[],
    replaceChildren(){this.children=[]},
    contains(node){return this.children.includes(node)},
    appendChild(node){this.children.push(node)},
    querySelectorAll(selector){
      assert.equal(selector,".ao-repo-item");
      return this.children.filter(n=>n.className?.includes("ao-repo-item"));
    }
  };
  document.createElement=(tag)=>({
    tagName:tag, dataset:{}, className:"", textContent:"", handlers:{},
    addEventListener(event,handler){this.handlers[event]=handler},
    focus(){document.activeElement=this},
    click(){this.focus();this.handlers.click?.()},
  });
  const mock=runInNewContext("(() => {" +
    " let lastRepoSignature='',selectedRepo='',snapshot=null;"+
    " const byId=(id)=>{if(id!=='aoRepos')throw Error(id);return __wrap};"+
    " const safeArray=(v)=>Array.isArray(v)?v:[];"+
    " const renderLists=()=>{};"+implementation+
    " return {renderRepos, setSnapshot(s){snapshot=s}}; })()",
    {document,__wrap:wrap});
  const state=(repos)=>({q:repos.map((repo)=>({repo})),pr:[],dn:[]});
  return {document,wrap,mock,state};
}

test("repository switch keeps keyboard focus on the newly selected filter",()=>{
  const h=fixture();const s=h.state(["owner/one","owner/two"]);
  h.mock.setSnapshot(s);h.mock.renderRepos(s);
  const first=h.wrap.children.find(x=>x.dataset.repo==="owner/one");
  first.focus();
  const second=h.wrap.children.find(x=>x.dataset.repo==="owner/two");
  second.click();
  assert.equal(h.document.activeElement.dataset.repo,"owner/two");
  assert.match(h.document.activeElement.className,/is-selected/);
});

test("an unchanged server snapshot does not replace focused filter buttons",()=>{
  const h=fixture();const s=h.state(["owner/one"]);
  h.mock.setSnapshot(s);h.mock.renderRepos(s);
  const current=h.wrap.children[0];current.focus();
  h.mock.renderRepos(s);
  assert.equal(h.document.activeElement,current);
});
