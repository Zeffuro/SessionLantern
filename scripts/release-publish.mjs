import {spawnSync} from 'node:child_process';
import {readdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';

const root=fileURLToPath(new URL('../',import.meta.url));
const execute=args=>spawnSync('gh',args,{encoding:'utf8',stdio:['ignore','pipe','pipe']});

export async function publishDraft(directory,tag,run=execute){
  if(!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(tag??'')){
    throw new Error('Use a version tag.');
  }
  const output=join(directory,'release-files');
  const files=(await readdir(output,{withFileTypes:true})).filter(entry=>entry.isFile()).map(entry=>join(output,entry.name)).sort();
  if(!files.some(file=>file.endsWith('SHA256SUMS.txt')))throw new Error('Release checksums are missing.');
  const prerelease=tag.split('+')[0].includes('-');
  const view=()=>run(['release','view',tag,'--json','isDraft,isPrerelease']);
  const verify=release=>{
    const metadata=JSON.parse(release.stdout);
    if(metadata.isDraft!==true)throw new Error('Refusing to change a published release.');
    if(metadata.isPrerelease!==prerelease)throw new Error('Release prerelease flag must match its version tag.');
  };
  let release=view();
  if(release.status===0){
    verify(release);
  }else{
    const args=['release','create',tag,'--verify-tag','--draft','--generate-notes','--title',`Session Lantern ${tag}`];
    if(prerelease)args.push('--prerelease');
    const created=run(args);
    if(created.status!==0)throw new Error('Draft release creation failed.');
  }
  release=view();
  if(release.status!==0)throw new Error('Release must be a draft.');
  verify(release);
  if(run(['release','upload',tag,...files,'--clobber']).status!==0)throw new Error('Draft asset upload failed.');
}

if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  try{await publishDraft(root,process.env.RELEASE_TAG);}catch(error){
    console.error(error.message);
    process.exitCode=1;
  }
}
