import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {runInNewContext} from 'node:vm';
import {webcrypto} from 'node:crypto';

const code=await readFile(new URL('../src-tauri/src/login-notice.js',import.meta.url),'utf8');

test('production initialization wrapper only mounts on top-level HTTPS TikTok pages',async()=>{
  const browser=await readFile(new URL('../src-tauri/src/browser.rs',import.meta.url),'utf8');
  const literal=browser.match(/fn login_notice_script\(\)[\s\S]*?format!\(\s*("[^\n]*")/)[1];
  const script=JSON.parse(literal).replace('{LOGIN_NOTICE}',"() => calls.push('shown')").replaceAll('{{','{').replaceAll('}}','}');
  for(const [address,top,expected] of [
    ['https://www.tiktok.com/login',true,1],['https://tiktok.com/',true,1],
    ['https://live.tiktok.com/',true,1],['https://www.tiktok.com/',false,0],
    ['http://www.tiktok.com/',true,0],['https://accounts.google.com/',true,0],
    ['https://tiktok.com.evil.example/',true,0],['https://eviltiktok.com/',true,0],
  ]){
    const window={};window.top=top?window:{};
    const calls=[];runInNewContext(script,{window,location:new URL(address),calls});
    assert.equal(calls.length,expected,address);
  }
});

test('static reminder survives root replacement without touching cookies, account content or IPC',()=>{
  const created=[];
  const observed=[];
  const events={};
  const window={addEventListener(name,callback){events[name]=callback;}};
  let callback;
  const element=()=>{
    const node={style:{},attributes:{},children:[],isConnected:false,
      setAttribute(name,value){this.attributes[name]=value;},
      appendChild(child){this.children.push(child);child.isConnected=true;},
      attachShadow(options){assert.equal(options.mode,'closed');this.shadow=element();return this.shadow;}};
    return node;
  };
  const document={documentElement:null,createElement(tag){assert.equal(tag,'div');const node=element();created.push(node);return node;}};
  Object.defineProperty(document,'cookie',{get(){throw new Error('Cookie access forbidden');}});
  runInNewContext(`(${code})()`,{document,window,crypto:webcrypto,MutationObserver:class{
    constructor(fn){callback=fn;}
    observe(target,options){observed.push({target,options});}
  }});
  assert.equal(created.length,2);
  const [host,content]=created;
  assert.equal(host.isConnected,false);
  assert.equal(host.attributes.role,'note');
  assert.match(content.textContent,/Keep this window open/);
  assert.match(content.textContent,/check and export/);
  assert.match(content.style.cssText,/background:#ffe08a;color:#342500/);
  document.documentElement=element();callback();
  assert.equal(document.documentElement.children[0],host);
  callback();assert.equal(document.documentElement.children.length,1);
  host.isConnected=false;document.documentElement=element();callback();
  assert.equal(document.documentElement.children[0],host);
  assert.ok(observed.every(entry=>entry.options.childList&&!entry.options.subtree));
  const documentId=window.__sessionLanternDocument;
  const previousDocument=(documentId[0]==='0'?'1':'0')+documentId.slice(1);
  events['session-lantern-readiness']({detail:{revision:1,document:previousDocument,available:4,complete:true}});
  assert.match(content.textContent,/Keep this window open/);
  assert.doesNotMatch(content.textContent,/All 4/);
  events['session-lantern-document']({detail:1});
  events['session-lantern-readiness']({detail:{revision:2,document:documentId,available:4,complete:true}});
  assert.match(content.textContent,/All 4 cookies are available/);
  assert.equal(content.style.background,'#b9f3cf');
  events['session-lantern-document']({detail:3});
  assert.equal(content.style.background,'#ffe08a');
  events['session-lantern-readiness']({detail:{revision:4,document:previousDocument,available:4,complete:true}});
  assert.match(content.textContent,/Checking available/);
  events['session-lantern-document']({detail:1});
  events['session-lantern-readiness']({detail:{revision:5,document:documentId,available:3,complete:false}});
  assert.match(content.textContent,/3\/4 cookies available/);
  events['session-lantern-readiness']({detail:{revision:6,document:documentId,available:4,complete:true}});
  assert.equal(content.style.background,'#b9f3cf');
});
