const $=selector=>document.querySelector(selector);
const fields=['sessionid','tt-target-idc','msToken','s_v_web_id'];
const labels={present:'✓ Available',missing:'Missing',expired:'Expired',ambiguous:'Conflicting',invalid:'Invalid'};
let ready=false;
let busy=false;
let revision=0;
let pendingReadiness;

function status(text,kind='info'){$('#status').textContent=text;$('#status').dataset.kind=kind;}
function exportStatus(text,kind='info',focus=false){
  status(text,kind);
  const node=$('#export-feedback');node.hidden=false;node.dataset.kind=kind;
  node.textContent=(kind==='success'?'✓ ':kind==='error'?'✕ ':'')+text;
  if(focus)node.focus();
}
function controls(){
  for(const id of ['open-login','check-session','clear-session','password','password-confirm','generate-password','clear-password'])$('#'+id).disabled=busy;
  $('#copy-password').disabled=busy||!$('#password').value;
  $('#export-session').disabled=busy||!ready;
  document.body.setAttribute('aria-busy',String(busy));
}
function reset(){
  ready=false;
  for(const name of fields){const node=$(`[data-cookie="${name}"]`);node.textContent='Not checked';delete node.dataset.state;}
  $('#readiness-count').textContent='0 / 4';
  $('#readiness-summary').textContent='Nothing has been read yet.';
  controls();
}
function paint(value){
  ready=value.complete===true;
  let available=0;
  for(const name of fields){
    const item=value.items.find(item=>item.name===name);
    const kind=item&&Object.hasOwn(labels,item.state)?item.state:'missing';
    const node=$(`[data-cookie="${name}"]`);
    node.textContent=labels[kind];node.dataset.state=kind;
    if(kind==='present')available++;
  }
  ready=ready&&available===4;
  $('#readiness-count').textContent=`${available} / 4`;
  const conflicting=value.items.some(item=>item.state==='ambiguous');
  $('#readiness-summary').textContent=ready?'All required values are available for export.':conflicting
    ?'Different active values remain in the required cookie scope. Export is blocked; finish signing in and check again.'
    :'Finish signing in, keep the TikTok window open, then check again. Missing values are never guessed.';
  controls();
}
function receiveReadiness(value){
  if(!Number.isSafeInteger(value?.revision)||value.revision<revision)return;
  revision=value.revision;
  if(busy){pendingReadiness=value;return;}
  if(value.readiness)paint(value.readiness);else reset();
}
window.sessionLanternReadiness=receiveReadiness;
function finish(){
  busy=false;
  if(pendingReadiness){const value=pendingReadiness;pendingReadiness=undefined;receiveReadiness(value);}
  controls();
}
async function action(command,args,success,failure=status){
  if(busy)return;
  const invoke=window.__TAURI__?.core?.invoke;
  if(!invoke){failure('Open the desktop application to use this tool.','error');return;}
  busy=true;controls();
  try{return await success(await invoke(command,args));}
  catch(error){reset();failure(typeof error==='string'?error:'The action could not finish. Try again.','error');}
  finally{finish();}
}
$('#password').oninput=controls;
$('#generate-password').onclick=()=>{
  if(busy)return;
  const bytes=new Uint8Array(32);
  try{
    crypto.getRandomValues(bytes);
    const alphabet='ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
    const password=Array.from(bytes,byte=>alphabet[byte&63]).join('');
    $('#password').value=password;$('#password-confirm').value=password;
    status('Generated a 32-character password. You can copy it before or after saving the encrypted session.','success');
  }catch{status('Secure password generation failed. Enter your own password instead.','error');}
  finally{bytes.fill(0);controls();}
};
$('#copy-password').onclick=async()=>{
  if(busy||!$('#password').value)return;
  const invoke=window.__TAURI__?.core?.invoke;
  if(!invoke){status('Open the desktop application to copy the password.','error');return;}
  busy=true;controls();
  try{
    await invoke('plugin:clipboard-manager|write_secret',{text:$('#password').value});
    status('Export password copied. Keep it private for dashboard import.','success');
  }catch{status('The password could not be copied. Try again; your password has been kept.','error');}
  finally{finish();}
};
$('#open-login').onclick=()=>action('open_login',undefined,()=>{
  reset();status('Sign in in the TikTok window, keep it open, then return here and check your session.');
});
$('#check-session').onclick=()=>action('check_session',undefined,value=>{
  receiveReadiness(value);const complete=value.readiness?.complete===true;
  status(complete?'Session values are ready. Choose an export password below.':'Some session values are unavailable. Keep the sign-in window open and check again.',complete?'success':'info');
});
$('#export-form').onsubmit=async event=>{
  event.preventDefault();if(!ready||busy)return;
  const password=$('#password').value;
  if([...password].length<12||new TextEncoder().encode(password).length>1024){exportStatus('Use at least 12 characters, up to 1024 UTF-8 bytes.','error',true);return;}
  if(password!==$('#password-confirm').value){exportStatus('The two passwords do not match.','error',true);return;}
  exportStatus('Encrypting your current session. A save dialog will open…');
  await action('export_session',{password},value=>{
    if(value.outcome==='failed'){exportStatus(value.message,'error',true);return;}
    exportStatus(value.outcome==='saved'?'Export saved. Copy the password when you are ready to import it in the dashboard.':'Export cancelled. Your password is still available to retry.',value.outcome==='saved'?'success':'info',true);
  },(text,kind)=>exportStatus(text,kind,true));
};
$('#clear-password').onclick=()=>{
  if(busy)return;
  $('#password').value='';$('#password-confirm').value='';controls();status('Export password cleared from this window.');
};
$('#clear-session').onclick=()=>{if(!busy)$('#clear-dialog').showModal();};
$('#cancel-clear').onclick=()=>$('#clear-dialog').close();
$('#confirm-clear').onclick=()=>{
  $('#clear-dialog').close();
  return action('clear_session',undefined,()=>{
    reset();$('#password').value='';$('#password-confirm').value='';status('The dedicated login was cleared. Exported files were kept.','success');
  });
};
controls();
