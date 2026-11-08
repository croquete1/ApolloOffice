// node --test quality/control-room.merge.test.mjs
// Execute the production merge-chart renderer against UTC-boundary timestamps.
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {test} from "node:test";
import {runInNewContext} from "node:vm";

const source=readFileSync(new URL("../ui/control-room.js",import.meta.url),"utf8");
const from=source.indexOf("  const renderMerges = (s) =>");
const to=source.indexOf("  const renderMachines = (s) =>",from);
assert.ok(from>=0 && to>from);
const implementation=source.slice(from,to);

function fixture(ms=Date.UTC(2026,2,1,0,30)) {
  const clock={ms};
  const out={writes:0,html:""};
  class FixedDate extends Date {
    static now(){return clock.ms}
    // Deliberately return a different local day from the UTC day.
    toDateString(){return "Sat Feb 28 2026"}
    getFullYear(){return 2026}
    getMonth(){return 1}
    getDate(){return 28}
  }
  const holder={
    set innerHTML(v){out.html=v;out.writes++},
    get innerHTML(){return out.html}
  };
  const script="(() => { let lastMergeSignature='';"+
    " const byId=(id)=>{if(id!=='aoMergeChart')throw Error('unexpected id');return __holder};"+
    " const safeArray=(v)=>Array.isArray(v)?v:[];"+
    implementation+"return {renderMerges};})()";
  const api=runInNewContext(script,{__holder:holder,Date:FixedDate});
  return {out,clock,render:api.renderMerges};
}

test("merge buckets match UTC dates even if local date is yesterday",()=>{
  const f=fixture();
  f.render({mg:[{d:"2026-03-01",n:3},{d:"2026-02-28",n:1}]});
  assert.match(f.out.html,/2026-03-01: 3 PRs/);
  assert.match(f.out.html,/2026-02-28: 1 PRs/);
  assert.match(f.out.html,/Hoje \(UTC\)/);
});

test("unchanged UTC date and records avoid unnecessary DOM writes",()=>{
  const f=fixture(),state={mg:[{d:"2026-03-01",n:1}]};
  f.render(state);f.render(state);
  assert.equal(f.out.writes,1);
});

test("UTC midnight forces the chart to update",()=>{
  const f=fixture(),state={mg:[{d:"2026-03-01",n:3},{d:"2026-03-02",n:1}]};
  f.render(state);
  f.clock.ms=Date.UTC(2026,2,2,0,1);
  f.render(state);
  assert.equal(f.out.writes,2);
  assert.match(f.out.html,/2026-03-02: 1 PRs/);
});
