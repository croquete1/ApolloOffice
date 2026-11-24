import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {test} from "node:test";
import {runInNewContext} from "node:vm";
const source=readFileSync(new URL("../ui/p.js",import.meta.url),"utf8");
const start=source.indexOf('    for (const button of B.querySelectorAll("[data-cr-repo]"))');
const end=source.indexOf("\n  };\n  A.steward",start);
assert.ok(start>=0 && end>start);
function fixture(result){
  let calls=0,resolve;
  const status={textContent:""};
  const button={disabled:false,isConnected:true,nextElementSibling:status,dataset:{crRepo:"croquete1/ApolloOffice",crNumber:"67",crHead:"abc"}};
  const B={querySelectorAll(){return [button];}};
  const post=(body,path)=>{
    calls++;
    assert.equal(path,"/api/steward/coderabbit");
    assert.equal(body.repo,"croquete1/ApolloOffice");
    assert.equal(body.number,67);
    assert.equal(body.head,"abc");
    return result==="defer"?new Promise(r=>{resolve=r;}):Promise.resolve(result);
  };
  runInNewContext(source.slice(start,end),{B,post,cur:"steward",dr:{hidden:false}});
  return {button,status,calls:()=>calls,resolve:v=>resolve(v)};
}
test("current evidence is advisory",async()=>{
  const f=fixture({head:"abc",state:"observed_current"});
  await f.button.onclick();
  assert.equal(f.status.textContent,"Evidência de revisão no HEAD indicado · não equivale a aprovação");
});
test("mismatched head cannot appear current",async()=>{
  const f=fixture({head:"wrong",state:"observed_current"});
  await f.button.onclick();
  assert.match(f.status.textContent,/indisponível/);
});
test("unknown state cannot render arbitrary content",async()=>{
  const f=fixture({head:"abc",state:"<script>"});
  await f.button.onclick();
  assert.match(f.status.textContent,/indisponível/);
  assert.doesNotMatch(f.status.textContent,/<script>/);
});
test("in-flight request is unique and detached panel is ignored",async()=>{
  const f=fixture("defer"),pending=f.button.onclick();
  await f.button.onclick();
  assert.equal(f.calls(),1);
  f.button.isConnected=false;
  f.resolve({head:"abc",state:"observed_current"});
  await pending;
  assert.equal(f.status.textContent,"A consultar evidência no GitHub…");
  assert.equal(f.button.disabled,false);
});

test("processing and skipped reviews are advisory, not approvals",async()=>{
  for(const [state,label] of [["pending","Revisão em processamento"],["skipped","Revisão não executada"]]){
    const f=fixture({head:"abc",state});
    await f.button.onclick();
    assert.equal(f.status.textContent,label+" · não equivale a aprovação");
  }
});
