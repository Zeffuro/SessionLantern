import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {runInNewContext} from 'node:vm';

const code=await readFile(new URL('../src/updates.js',import.meta.url),'utf8');
function ui(invoke){
  const nodes=new Map();
  const get=key=>{if(!nodes.has(key))nodes.set(key,{textContent:'',disabled:false});return nodes.get(key);};
  runInNewContext(code,{document:{querySelector:get},window:{__TAURI__:{core:{invoke},app:{getVersion:async()=> '0.2.1'}}}});
  return get;
}
test('update checks are explicit, version-only and distinguish newer, current, unpublished and failed results',async()=>{
  for(const state of ['available','current','unpublished','failed']){
    const calls=[];
    const get=ui(async(command,args)=>{calls.push({command,args});if(state==='failed')throw 'network';return {state,latest:'0.3.0',current:'0.2.1'};});
    await Promise.resolve();assert.equal(calls.length,0);assert.equal(get('#app-version').textContent,'v0.2.1');
    await get('#check-updates').onclick();assert.deepEqual(calls,[{command:'check_updates',args:undefined}]);
    assert.equal(get('#check-updates').disabled,false);
    assert.match(get('#update-status').textContent,{available:/Update available: v0.3.0/,current:/up to date/,unpublished:/No published release/,failed:/Could not check/}[state]);
    await get('#open-releases').onclick();assert.equal(calls[1].command,'open_releases');assert.equal(calls[1].args,undefined);
  }
});
test('repeated clicks cannot create parallel update requests and browser-open errors remain visible',async()=>{
  let finish;let calls=0;
  const get=ui(async command=>{calls++;if(command==='check_updates')return new Promise(resolve=>{finish=resolve;});throw 'could not open';});
  const pending=get('#check-updates').onclick();await get('#check-updates').onclick();assert.equal(calls,1);
  finish({state:'unpublished'});await pending;
  await get('#open-releases').onclick();assert.match(get('#update-status').textContent,/Could not open/);assert.equal(get('#open-releases').disabled,false);
});
