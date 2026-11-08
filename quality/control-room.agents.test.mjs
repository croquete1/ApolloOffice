// node --test quality/control-room.agents.test.mjs
// Execute the production role-state rendering logic against isolated UI elements.
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {test} from "node:test";
import {runInNewContext} from "node:vm";

const file=readFileSync(new URL("../ui/control-room.js",import.meta.url),"utf8");
const from=file.indexOf("  // External provider/network failures");
const to=file.indexOf("  const rowHTML = (label, caption",from);
const staleFrom=file.indexOf("  const markStale = () =>");
const staleTo=file.indexOf("  const render = (s) =>",staleFrom);
assert.ok(from>=0 && to>from && staleFrom>=0 && staleTo>staleFrom);
const implementation=file.slice(from,to);
const staleImplementation=file.slice(staleFrom,staleTo);

function harness() {
  const roles=["scout","guard","vet","watcher","hunter","repro","fixer","ci","reviewer","submit"];
  const rows=Object.fromEntries(roles.map((role)=>{
    const status={textContent:"",className:""};
    const activity={textContent:"",title:""};
    const model={textContent:"",hidden:true};
    return [role,{
      status,activity,model,
      querySelector(sel){
        return {".ao-agent-status":status,".ao-agent-activity":activity,".ao-agent-model":model}[sel];
      }
    }];
  }));
  const nodes=Object.fromEntries(roles.map((role)=>{
    const parts={".ao-station-state":{textContent:""},".ao-result-mark":{textContent:""}};
    const attrs={};
    const classes=new Set();
    return [role,{
      dataset:{mode:"unknown"},
      classList:{add:(v)=>classes.add(v),remove:(v)=>classes.delete(v)},
      querySelector:(q)=>parts[q],
      setAttribute:(k,v)=>{attrs[k]=v},
      getAttribute:(k)=>attrs[k]
    }];
  }));
  const list={children:roles.map(()=>({})),querySelector:(query)=>{
    const role=query.match(/data-agent="([^"]+)"/)?.[1];
    return rows[role];
  }};
  const elements={aoAgentList:list,aoMetricWorkers:{textContent:""},aoAgentSummary:{textContent:""}};
  const classes=new Set();
  const app={classList:{add:(n)=>classes.add(n),contains:(n)=>classes.has(n)}};
  const env={
    roles, stationNodes:nodes, names:Object.fromEntries(roles.map((n)=>[n,n])),
    colors:roles.map(()=>"#0ff"), safeArray:(v)=>Array.isArray(v)?v:[],
    formatNum:(v)=>String(v), escapeHTML:(v)=>String(v),
    byId:(id)=>elements[id], animationsOn:false,
    document:{hidden:false}, window:{matchMedia:()=>({matches:false})},
    A:{mdn:(s)=>s.replace("ollama:","")+" · local"},
    app, clearReactions:()=>{}, snapshot:null, renderMachines:()=>{}
  };
  const src="(() => {" + implementation + staleImplementation + " return {updateAgents,markStale}; })()";
  const api=runInNewContext(src,env);
  return {api,rows,nodes,elements,app};
}

test("active agent reflects work and the actual Ollama model",()=>{
  const h=harness();
  h.api.updateAgents({w:{fixer:{st:"work",md:"ollama:qwen3:8b",say:"A rever alterações"}},o:{},q:[]});
  assert.equal(h.nodes.fixer.dataset.mode,"work");
  assert.equal(h.rows.fixer.activity.textContent,"A rever alterações");
  assert.equal(h.rows.fixer.model.textContent,"qwen3:8b · local");
  assert.equal(h.rows.fixer.status.textContent,"A trabalhar");
  assert.equal(h.elements.aoMetricWorkers.textContent,"1 / 10");
});

test("an unrelated queued task does not invent a working agent",()=>{
  const h=harness();
  h.api.updateAgents({w:{fixer:{st:"idle",say:""}},q:[{id:5,st:"fix"}],o:{}});
  assert.equal(h.nodes.fixer.dataset.mode,"idle");
  assert.equal(h.elements.aoMetricWorkers.textContent,"0 / 10");
});

test("missing worker records remain visibly unknown",()=>{
  const h=harness();
  h.api.updateAgents({w:{},q:[],o:{}});
  assert.equal(h.nodes.reviewer.dataset.mode,"unknown");
  assert.equal(h.rows.reviewer.status.textContent,"Sem dados");
});

test("stale observations stop claiming that workers are active",()=>{
  const h=harness();
  h.api.updateAgents({w:{fixer:{st:"work",say:"Compilação"}},q:[],o:{}});
  h.api.markStale();
  assert.equal(h.nodes.fixer.dataset.mode,"unknown");
  assert.equal(h.rows.fixer.status.textContent,"Por confirmar");
  assert.equal(h.elements.aoMetricWorkers.textContent,"—");
  assert.equal(h.app.classList.contains("ao-data-stale"),true);
});
