import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {runInNewContext} from 'node:vm';
import {webcrypto} from 'node:crypto';

const code=await readFile(new URL('../src/app.js',import.meta.url),'utf8');
function ui(invoke,crypto=webcrypto){
  const nodes=new Map();
  const get=name=>{if(!nodes.has(name))nodes.set(name,{value:'',textContent:'',dataset:{},disabled:false,focus(){this.focused=true;},showModal(){this.open=true;},close(){this.open=false;}});return nodes.get(name);};
  let revision=0;
  const window={__TAURI__:{core:{invoke:async(command,args)=>{
    const result=await invoke(command,args);
    return command==='check_session'?{revision:++revision,load:revision,readiness:result}:result;
  }}}};
  runInNewContext(code,{document:{querySelector:get,body:{setAttribute:()=>{}}},window,TextEncoder,crypto});
  get.receive=window.sessionLanternReadiness;
  return get;
}
const complete={complete:true,items:['sessionid','tt-target-idc','msToken','s_v_web_id'].map(name=>({name,state:'present'}))};

test('remaining conflicts are explained and a closed sign-in window invalidates prior readiness',async()=>{
  let result={complete:false,items:complete.items.map(item=>item.name==='msToken'?{...item,state:'ambiguous'}:item)};
  const closed='Open the TikTok sign-in window and keep it open while checking or exporting your session.';
  const get=ui(async()=>{if(result)return result;throw closed;});
  await get('#check-session').onclick();
  assert.equal(get('#export-session').disabled,true);
  assert.match(get('#readiness-summary').textContent,/Different active values/);
  assert.match(get('[data-cookie="msToken"]').textContent,/Conflicting/);
  result=complete;await get('#check-session').onclick();
  assert.equal(get('#export-session').disabled,false);
  result=null;await get('#check-session').onclick();
  assert.equal(get('#export-session').disabled,true);
  assert.equal(get('#status').textContent,closed);
  assert.equal(get('#readiness-count').textContent,'0 / 4');
});
test('check exposes status only; export validates confirmation and keeps password after cancellation',async()=>{
  const requests=[];
  const get=ui(async(command,args)=>{requests.push({command,args});return command==='check_session'?complete:{outcome:'cancelled'};});
  assert.equal(get('#export-session').disabled,true);
  await get('#check-session').onclick();
  assert.equal(get('#export-session').disabled,false);
  assert.match(get('[data-cookie="sessionid"]').textContent,/Available/);
  get('#password').value='a sufficiently long password';get('#password-confirm').value='different';
  await get('#export-form').onsubmit({preventDefault(){}});
  assert.equal(requests.length,1);
  get('#password-confirm').value=get('#password').value;
  await get('#export-form').onsubmit({preventDefault(){}});
  assert.equal(requests[1].command,'export_session');
  assert.equal(get('#password').value,'a sufficiently long password');assert.equal(get('#password-confirm').value,get('#password').value);
  assert.equal(get('#copy-password').disabled,false);
  assert.match(get('#status').textContent,/cancelled/);
});

test('saved or failed export keeps password copyable until explicitly cleared',async()=>{
  for(const outcome of ['saved','failed']){
    const get=ui(async command=>{if(command==='check_session')return complete;if(outcome==='failed'&&command==='export_session')throw 'Save failed.';return {outcome};});
    await get('#check-session').onclick();get('#generate-password').onclick();
    const password=get('#password').value;
    await get('#export-form').onsubmit({preventDefault(){}});
    assert.equal(get('#password').value,password);
    assert.equal(get('#password-confirm').value,password);
    assert.equal(get('#copy-password').disabled,false);
    await get('#copy-password').onclick();assert.match(get('#status').textContent,/copied/);
    get('#clear-password').onclick();assert.equal(get('#password').value,'');
    assert.equal(get('#password-confirm').value,'');assert.equal(get('#copy-password').disabled,true);
  }
});

test('automatic updates respect native revision ordering and flush after clipboard use',async()=>{
  let copied;
  const get=ui(async command=>{if(command==='plugin:clipboard-manager|write_secret')return new Promise(resolve=>{copied=resolve;});});
  get.receive({revision:1,load:1,readiness:complete});
  assert.equal(get('#export-session').disabled,false);
  get('#generate-password').onclick();const copying=get('#copy-password').onclick();
  get.receive({revision:3,load:2,readiness:null});
  get.receive({revision:2,load:1,readiness:complete});
  copied();await copying;
  assert.equal(get('#export-session').disabled,true);
  assert.equal(get('#readiness-count').textContent,'0 / 4');
  get.receive({revision:4,load:2,readiness:complete});
  assert.equal(get('#export-session').disabled,false);
  get.receive({revision:3,load:2,readiness:null});
  assert.equal(get('#export-session').disabled,false);
});

test('authoritative automatic readiness survives a simultaneous manual busy rejection',async()=>{
  let get;
  get=ui(async()=>{get.receive({revision:2,load:1,readiness:complete});throw 'Finish the current action first.';});
  await get('#check-session').onclick();
  assert.equal(get('#export-session').disabled,false);
  assert.equal(get('#readiness-count').textContent,'4 / 4');
});
test('busy checks are exclusive, failures reset readiness, and clearing requires local confirmation',async()=>{
  let finish;
  let calls=0;
  const get=ui(async command=>{calls++;if(command==='check_session')return new Promise(resolve=>{finish=resolve;});if(command==='open_login')throw 'Open the sign-in window first.';});
  const checking=get('#check-session').onclick();
  await get('#check-session').onclick();assert.equal(calls,1);
  finish(complete);await checking;
  await get('#open-login').onclick();assert.equal(get('#export-session').disabled,true);
  assert.match(get('#status').textContent,/sign-in/);
  get('#clear-session').onclick();assert.equal(get('#clear-dialog').open,true);
  assert.equal(calls,2);
  get('#cancel-clear').onclick();assert.equal(get('#clear-dialog').open,false);
  get('#clear-session').onclick();await get('#confirm-clear').onclick();
  assert.equal(calls,3);assert.equal(get('#clear-dialog').open,false);
});
test('capabilities are restricted to the local control window and all custom commands are ACL registered',async()=>{
  const capability=JSON.parse(await readFile(new URL('../src-tauri/capabilities/control.json',import.meta.url),'utf8'));
  assert.equal(capability.local,true);assert.deepEqual(capability.windows,['main']);assert.equal(capability.remote,undefined);
  assert.ok(capability.permissions.includes('clipboard-manager:allow-write-secret'));
  assert.equal(capability.permissions.some(permission=>/clipboard.*(?:read|allow-write-text)/.test(permission)),false);
  const build=await readFile(new URL('../src-tauri/build.rs',import.meta.url),'utf8');
  assert.equal(capability.permissions.some(permission=>/^opener:/.test(permission)),false);
  for(const command of ['open_login','check_session','export_session','clear_session','check_updates','open_releases']){
    assert.match(build,new RegExp(`"${command}"`));
    assert.ok(capability.permissions.includes('allow-'+command.replaceAll('_','-')));
  }
});
test('secure generation fills both fields, produces fresh passwords and never copies automatically',async()=>{
  const requests=[];
  const get=ui(async(command,args)=>requests.push({command,args}));
  assert.equal(get('#copy-password').disabled,true);
  get('#generate-password').onclick();
  const first=get('#password').value;
  assert.match(first,/^[A-Za-z0-9_-]{32}$/);
  assert.equal(get('#password-confirm').value,first);
  assert.equal(get('#copy-password').disabled,false);
  assert.equal(requests.length,0);
  get('#generate-password').onclick();assert.notEqual(get('#password').value,first);
  const current=get('#password').value;
  await get('#copy-password').onclick();
  assert.equal(requests.length,1);
  assert.equal(requests[0].command,'plugin:clipboard-manager|write_secret');
  assert.deepEqual({...requests[0].args},{text:current});
  assert.match(get('#status').textContent,/copied/);
  assert.doesNotMatch(get('#status').textContent,new RegExp(current));
});
test('copy failure preserves readiness and passwords; generation failure never falls back to weak random',async()=>{
  const get=ui(async command=>{if(command==='check_session')return complete;throw 'native clipboard unavailable';});
  await get('#check-session').onclick();get('#generate-password').onclick();
  const password=get('#password').value;
  await get('#copy-password').onclick();
  assert.equal(get('#password').value,password);assert.equal(get('#export-session').disabled,false);
  assert.match(get('#status').textContent,/could not be copied/);
  get('#password').value='';get('#password').oninput();assert.equal(get('#copy-password').disabled,true);
  const unavailable=ui(async()=>{}, {getRandomValues(){throw new Error('random unavailable');}});
  unavailable('#generate-password').onclick();
  assert.equal(unavailable('#password').value,'');assert.match(unavailable('#status').textContent,/generation failed/);
});

test('duplicate-file failure is prominent, preserves password and permits a new export attempt',async()=>{
  let attempt=0;
  const message='Export not saved: a file already exists at that location. It was not replaced. Choose a different file name and save again.';
  const get=ui(async command=>command==='check_session'?complete:++attempt===1?{outcome:'failed',message}:{outcome:'saved'});
  await get('#check-session').onclick();get('#generate-password').onclick();const password=get('#password').value;
  await get('#export-form').onsubmit({preventDefault(){}});
  assert.equal(get('#export-feedback').hidden,false);assert.equal(get('#export-feedback').dataset.kind,'error');
  assert.match(get('#export-feedback').textContent,/It was not replaced/);assert.equal(get('#export-feedback').focused,true);
  assert.equal(get('#password').value,password);assert.equal(get('#export-session').disabled,false);
  await get('#export-form').onsubmit({preventDefault(){}});
  assert.equal(get('#export-feedback').dataset.kind,'success');assert.match(get('#export-feedback').textContent,/Export saved/);
});
