const $=selector=>document.querySelector(selector);
const invoke=window.__TAURI__?.core?.invoke;
const check=$('#check-updates'),open=$('#open-releases'),status=$('#update-status');
let checking=false;
window.__TAURI__?.app?.getVersion().then(version=>{$('#app-version').textContent=`v${version}`;}).catch(()=>{});
check.onclick=async()=>{
  if(checking)return;
  if(!invoke){status.textContent='Open the desktop application to check for updates.';return;}
  checking=true;check.disabled=true;status.textContent='Checking published GitHub releases…';
  try{
    const result=await invoke('check_updates');
    status.textContent=result.state==='available'?`Update available: v${result.latest} (installed: v${result.current}). Open GitHub releases to download it.`
      :result.state==='current'?`You are up to date (v${result.current}).`:'No published release is available yet. Check again after the first release.';
  }catch{status.textContent='Could not check for updates. Try again later or open GitHub releases.';}
  finally{checking=false;check.disabled=false;}
};
open.onclick=async()=>{
  if(open.disabled||!invoke)return;
  open.disabled=true;
  try{await invoke('open_releases');}
  catch{status.textContent='Could not open GitHub releases. Visit github.com/Zeffuro/SessionLantern/releases in your browser.';}
  finally{open.disabled=false;}
};
