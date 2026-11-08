// node --test quality/control-room.reactions.test.mjs
// Verify one-shot animation cleanup without a browser.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source=readFileSync(new URL("../ui/control-room.js",import.meta.url),"utf8");
const start=source.indexOf("  // One-shot visual reactions");
const end=source.indexOf("  let snapshot = null", start);
assert.ok(start >= 0 && end > start, "reaction cleanup code must be found");
const implementation=source.slice(start,end);

function fixture(){
  const listeners={};
  const stations={};
  for(const role of ["fixer","reviewer","ci"]){
    const classes=new Set(["ao-celebrate","ao-action-pulse"]);
    stations[role]={
      classList:{
        remove(...names){for(const name of names)classes.delete(name);},
        contains(name){return classes.has(name)}
      },
      addEventListener(name,listener){listeners[role+":"+name]=listener;}
    };
  }
  const {clearReactions}=runInNewContext(`(() => {
    const stationNodes=__stations;
    ${implementation}
    return {clearReactions};
  })()`,{__stations:stations});
  return {clearReactions,stations,listeners};
}

test("celebration ends without stopping permanent breathing",()=>{
  const h=fixture();
  h.listeners["fixer:animationend"]({animationName:"ao-celebrate"});
  assert.equal(h.stations.fixer.classList.contains("ao-celebrate"),false);
  assert.equal(h.stations.fixer.classList.contains("ao-action-pulse"),true);
});

test("monitor sweep cleans itself up",()=>{
  const h=fixture();
  h.listeners["ci:animationend"]({animationName:"ao-command-sweep"});
  assert.equal(h.stations.ci.classList.contains("ao-action-pulse"),false);
  assert.equal(h.stations.ci.classList.contains("ao-celebrate"),true);
});

test("other animation completions do not alter reactions",()=>{
  const h=fixture();
  h.listeners["reviewer:animationend"]({animationName:"ao-blink"});
  assert.equal(h.stations.reviewer.classList.contains("ao-celebrate"),true);
  assert.equal(h.stations.reviewer.classList.contains("ao-action-pulse"),true);
});

test("disabling motion removes all transient effects",()=>{
  const h=fixture();
  h.clearReactions();
  for(const station of Object.values(h.stations)){
    assert.equal(station.classList.contains("ao-celebrate"),false);
    assert.equal(station.classList.contains("ao-action-pulse"),false);
  }
});
