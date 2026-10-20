import {copyFile,mkdir,readdir,readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';

const root=fileURLToPath(new URL('../',import.meta.url));
const targets={
  'x86_64-pc-windows-msvc':{platform:'windows-x64',bundles:[['nsis',/-setup\.exe$/,'-setup.exe']],portable:true},
  'x86_64-unknown-linux-gnu':{platform:'linux-x64',bundles:[['appimage',/\.AppImage$/,'.AppImage'],['deb',/\.deb$/,'.deb']]},
  'aarch64-apple-darwin':{platform:'macos-arm64',bundles:[['dmg',/\.dmg$/,'.dmg']]},
  'x86_64-apple-darwin':{platform:'macos-x64',bundles:[['dmg',/\.dmg$/,'.dmg']]},
};

export async function stageReleaseFiles(directory,target,version){
  const spec=targets[target];
  if(!spec||!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)){
    throw new Error('Unsupported build target or version.');
  }
  const source=join(directory,'src-tauri/target',target,'release');
  const output=join(directory,'release-files');
  await mkdir(output);
  const name=`Session-Lantern-${version}-${spec.platform}`;
  if(spec.portable)await copyFile(join(source,'session-lantern.exe'),join(output,`${name}.exe`));
  for(const [bundle,pattern,suffix] of spec.bundles){
    const folder=join(source,'bundle',bundle);
    const entries=(await readdir(folder,{withFileTypes:true})).filter(entry=>entry.isFile()&&pattern.test(entry.name));
    if(entries.length!==1)throw new Error(`Expected one ${bundle} artifact.`);
    await copyFile(join(folder,entries[0].name),join(output,`${name}${suffix}`));
  }
}

export async function writeChecksums(directory,version){
  const output=join(directory,'release-files');
  const expected=Object.values(targets).flatMap(spec=>[
    ...(spec.portable?[`Session-Lantern-${version}-${spec.platform}.exe`]:[]),
    ...spec.bundles.map(([,,suffix])=>`Session-Lantern-${version}-${spec.platform}${suffix}`),
  ]).sort();
  const actual=(await readdir(output,{withFileTypes:true})).filter(entry=>entry.isFile()).map(entry=>entry.name).sort();
  if(JSON.stringify(actual)!==JSON.stringify(expected))throw new Error('Release artifacts are missing or unexpected.');
  const sums=await Promise.all(expected.map(async name=>{
    const hash=createHash('sha256').update(await readFile(join(output,name))).digest('hex');
    return `${hash}  ${name}\n`;
  }));
  await writeFile(join(output,'SHA256SUMS.txt'),sums.join(''));
}

if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  try{
    const {version}=JSON.parse(await readFile(join(root,'package.json'),'utf8'));
    if(process.argv[2]==='stage')await stageReleaseFiles(root,process.argv[3],version);
    else if(process.argv[2]==='checksum')await writeChecksums(root,version);
    else throw new Error('Expected stage or checksum.');
  }catch{
    console.error('Release artifact validation failed. Check native bundle outputs.');
    process.exitCode=1;
  }
}
