import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {test} from "node:test";
import {runInNewContext} from "node:vm";

const js=readFileSync(new URL("../ui/office-v2.js",import.meta.url),"utf8");
const css=readFileSync(new URL("../ui/office-v2.css",import.meta.url),"utf8");
const modern=readFileSync(new URL("../ui/control-room.js",import.meta.url),"utf8");
const html=readFileSync(new URL("../ui/index.html",import.meta.url),"utf8");

function fixture(reduced=false){
  const window={matchMedia:()=>({matches:reduced})};
  runInNewContext(js,{window});
  return window.AOOfficeV2;
}
const roles=["scout","guard","vet","watcher","hunter","repro","fixer","ci","reviewer","submit"];

test("ten distinct SVG people have body joints and unique paint identifiers",()=>{
  const office=fixture(),svgs=roles.map((role,i)=>office.character(i));
  assert.equal(svgs.length,10);
  for(let i=0;i<svgs.length;i++){
    for(const part of ["ao-v2-torso","ao-v2-leg-left","ao-v2-leg-right","ao-v2-arm-left","ao-v2-arm-right","ao-v2-eyes"])assert.ok(svgs[i].includes(part));
    assert.ok(svgs[i].includes('id="ao-v2-'+roles[i]+'-skin"'));
  }
});

test("static office decoration is idempotent and includes physical depth",()=>{
  const office=fixture(),nodes=roles.map(role=>({
    dataset:{role},calls:0,
    insertAdjacentHTML(where,s){this.calls++;this.html=s},
    addEventListener(){}
  }));
  const stations={querySelectorAll:()=>nodes};
  const room={dataset:{},calls:0,insertAdjacentHTML(where,s){this.calls++;this.html=s}};
  office.decorate(room,stations);
  office.decorate(room,stations);
  assert.equal(room.dataset.visualVersion,"2");
  assert.equal(room.calls,1);
  assert.ok(nodes.every(node=>node.calls===1));
  assert.match(room.html,/ao-v2-window-left/);
  assert.match(room.html,/ao-v2-plant/);
  assert.match(nodes[0].html,/ao-v2-desk-legs/);
});

test("seat and rise motions depend on real state changes",()=>{
  const office=fixture(),classes=new Set();
  const node={classList:{add:x=>classes.add(x),remove:(...x)=>x.forEach(y=>classes.delete(y))},closest:()=>({classList:{contains:()=>false}})};
  office.onAgentState(node,"idle","work");
  assert.ok(classes.has("ao-v2-arriving"));
  office.onAgentState(node,"work","idle");
  assert.ok(classes.has("ao-v2-rising"));
  classes.clear();
  office.onAgentState(node,"unknown","work");
  assert.equal(classes.size,0);
});

test("reduced motion and the animation switch prohibit new poses",()=>{
  const set=new Set();
  const node={classList:{add:x=>set.add(x),remove:x=>set.delete(x)},closest:()=>({classList:{contains:()=>false}})};
  fixture(true).onAgentState(node,"idle","work");
  assert.equal(set.size,0);
  const disabled={...node,closest:()=>({classList:{contains:x=>x==="ao-motion-disabled"}})};
  fixture().onAgentState(disabled,"idle","work");
  assert.equal(set.size,0);
});

test("the premium room loads before the live dashboard, which remains primary",()=>{
  assert.ok(html.indexOf("office-v2.js")<html.indexOf("control-room.js"));
  assert.ok(html.indexOf("control-room.css")<html.indexOf("office-v2.css"));
  assert.match(modern,/window\.AOOfficeV2\?\.decorate/);
  assert.match(modern,/window\.AOOfficeV2\?\.onAgentState/);
  assert.ok(modern.indexOf("ao-operations-grid")<modern.indexOf("ao-primary-grid"));
});

test("the walker has leg cycles, real handoff gesture and reduced-motion support",()=>{
  assert.match(css,/\.ao-walking-agent \.ao-v2-leg-left/);
  assert.match(css,/\.ao-walking-agent \.ao-v2-leg-right/);
  assert.match(css,/ao-v2-handoff-wave/);
  assert.match(css,/prefers-reduced-motion:reduce/);
  assert.doesNotMatch(js,/<canvas/);
});


test("cancelling an old arrival cannot remove a newer rise", () => {
  const studio=fixture();
  const classes=new Set();
  const handlers={};
  const agent={
    classList:{
      add:x=>classes.add(x),
      remove:(...xs)=>xs.forEach(x=>classes.delete(x)),
      contains:x=>classes.has(x)
    },
    dataset:{},insertAdjacentHTML(){},
    addEventListener:(event,fn)=>{handlers[event]=fn;},
    closest:()=>({classList:{contains:()=>false}})
  };
  const room={dataset:{},insertAdjacentHTML(){}};
  studio.decorate(room,{querySelectorAll:()=>[agent]});
  studio.onAgentState(agent,"idle","work");
  assert.ok(classes.has("ao-v2-arriving"));
  studio.onAgentState(agent,"work","idle");
  assert.ok(classes.has("ao-v2-rising"));
  const target={classList:{contains:x=>x==="ao-v2-character"},getAnimations:()=>[]};
  handlers.animationcancel({type:"animationcancel",animationName:"ao-v2-arrive",target});
  assert.ok(classes.has("ao-v2-rising"),"old arrival must not cancel a newer rise");
  handlers.animationcancel({type:"animationcancel",animationName:"ao-v2-rise",target});
  assert.equal(classes.has("ao-v2-rising"),false);
});

test("cancel of a replaced animation with the same name preserves its live pose",()=>{
  const studio=fixture();
  const classes=new Set(["ao-v2-arriving"]);
  const listeners={};
  const agent={
    classList:{add:x=>classes.add(x),remove:x=>classes.delete(x),contains:x=>classes.has(x)},
    dataset:{},insertAdjacentHTML(){},
    addEventListener:(type,fn)=>{listeners[type]=fn}
  };
  studio.decorate({dataset:{},insertAdjacentHTML(){}},{querySelectorAll:()=>[agent]});
  const target={
    classList:{contains:x=>x==="ao-v2-character"},
    getAnimations:()=>[{animationName:"ao-v2-arrive",playState:"running"}]
  };
  listeners.animationcancel({type:"animationcancel",animationName:"ao-v2-arrive",target});
  assert.ok(classes.has("ao-v2-arriving"),"new arrival still running");
  studio.clearTransitions({querySelectorAll:()=>[agent]});
  assert.equal(classes.size,0);
});

test("motion disabled clears a stale arrival instead of replaying it later",()=>{
  const studio=fixture();
  const classes=new Set(["ao-v2-arriving"]);
  const agent={
    classList:{add:x=>classes.add(x),remove:(...x)=>x.forEach(y=>classes.delete(y))},
    closest:()=>({classList:{contains:x=>x==="ao-motion-disabled"}})
  };
  studio.onAgentState(agent,"idle","work");
  assert.equal(classes.has("ao-v2-arriving"),false);
});
