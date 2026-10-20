() => {
  const host=document.createElement('div');
  host.setAttribute('data-session-lantern-notice','');
  host.setAttribute('role','note');
  host.style.cssText='all:initial!important;position:fixed!important;top:0!important;left:0!important;right:0!important;z-index:2147483647!important;display:block!important;pointer-events:none!important;';
  const content=document.createElement('div');
  content.style.cssText='box-sizing:border-box;background:#ffe08a;color:#342500;border-bottom:1px solid #b88c20;padding:6px 16px;font:600 12px/1.4 "Segoe UI",system-ui,sans-serif;text-align:center;overflow-wrap:anywhere;';
  content.textContent='⚠ Session Lantern · Keep this window open. Return to Session Lantern to check and export your session.';
  let revision=0;
  let documentId;
  try{
    documentId=Array.from(crypto.getRandomValues(new Uint8Array(16)),byte=>byte.toString(16).padStart(2,'0')).join('');
    Object.defineProperty(window,'__sessionLanternDocument',{value:documentId});
  }catch{}
  window.addEventListener('session-lantern-document',event=>{
    if(!Number.isSafeInteger(event.detail)||event.detail<revision)return;
    revision=event.detail;
    content.style.background='#ffe08a';content.style.color='#342500';
    content.textContent='⚠ Session Lantern · Checking available cookies. Keep this window open; TikTok’s page can show an error while the cookies are still usable.';
  });
  window.addEventListener('session-lantern-readiness',event=>{
    const value=event.detail;
    if(!documentId||value?.document!==documentId||!Number.isSafeInteger(value?.revision)||value.revision<revision
      ||!Number.isInteger(value.available)||value.available<0||value.available>4)return;
    revision=value.revision;
    const complete=value.complete===true&&value.available===4;
    content.style.background=complete?'#b9f3cf':'#ffe08a';
    content.style.color=complete?'#143d24':'#342500';
    content.textContent=complete
      ?'✓ Session Lantern · All 4 cookies are available. Return to Session Lantern to export. Keep this window open, even if TikTok shows an error.'
      :`⚠ Session Lantern · ${value.available}/4 cookies available. Finish signing in or refresh, then check again. Keep this window open.`;
  });
  host.attachShadow({mode:'closed'}).appendChild(content);
  const observer=new MutationObserver(mount);
  let root;
  function mount(){
    if(root!==document.documentElement){
      root=document.documentElement;
      if(root)observer.observe(root,{childList:true});
    }
    if(root&&!host.isConnected)root.appendChild(host);
  }
  observer.observe(document,{childList:true});
  mount();
}
