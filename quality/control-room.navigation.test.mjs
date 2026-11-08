// node --test quality/control-room.navigation.test.mjs
// Exercise the production keyboard navigation handler without a browser.
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {test} from "node:test";
import {runInNewContext} from "node:vm";

const source=readFileSync(new URL("../ui/control-room.js",import.meta.url),"utf8");
const start=source.indexOf("  // Arrow navigation follows");
const end=source.indexOf("  // One-shot visual reactions",start);
assert.ok(start>=0 && end>start);
const implementation=source.slice(start,end);

function scene(mobile=false) {
  const roles=["scout","guard","vet","watcher","hunter","repro","fixer","ci","reviewer","submit"];
  const stations={handler:null,addEventListener(type,callback){assert.equal(type,"keydown");this.handler=callback;},
    contains:(el)=>roles.includes(el.dataset.role)};
  let focused=null, prevented=false;
  const stationNodes=Object.fromEntries(roles.map(role=>[role,{
    focus(){focused=role}
  }]));
  const window={matchMedia:()=>({matches:mobile})};
  runInNewContext("(() => {"+implementation+"})()",{roles,stations,stationNodes,window});
  return {
    press(role,key,modifier={}){
      focused=null;prevented=false;
      const node={dataset:{role},closest:()=>node};
      const e={target:node,key,preventDefault(){prevented=true},...modifier};
      stations.handler(e);
      return {focused,prevented};
    }
  };
}

test("desktop arrows follow five-column station grid",()=>{
  const s=scene(false);
  assert.deepEqual(s.press("scout","ArrowRight"),{focused:"guard",prevented:true});
  assert.deepEqual(s.press("scout","ArrowDown"),{focused:"repro",prevented:true});
  assert.deepEqual(s.press("repro","ArrowUp"),{focused:"scout",prevented:true});
});

test("mobile arrows follow the two-column arrangement",()=>{
  const s=scene(true);
  assert.deepEqual(s.press("guard","ArrowDown"),{focused:"watcher",prevented:true});
  assert.deepEqual(s.press("ci","ArrowUp"),{focused:"repro",prevented:true});
});

test("out of bounds and unrelated keys keep native behaviour",()=>{
  const s=scene(false);
  assert.deepEqual(s.press("scout","ArrowLeft"),{focused:null,prevented:false});
  assert.deepEqual(s.press("submit","ArrowDown"),{focused:null,prevented:false});
  assert.deepEqual(s.press("scout","Tab"),{focused:null,prevented:false});
});

test("modified arrow keys preserve browser navigation",()=>{
  const s=scene();
  assert.deepEqual(s.press("scout","ArrowRight",{ctrlKey:true}),{focused:null,prevented:false});
});
