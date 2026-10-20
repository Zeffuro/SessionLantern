import {readFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const root=fileURLToPath(new URL('../',import.meta.url));
const versionPattern=/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const text=(directory,path)=>readFile(resolve(directory,path),'utf8');
const json=async(directory,path)=>JSON.parse(await text(directory,path));
function field(block,name){
  const values=[...block.matchAll(new RegExp(`^${name}\\s*=\\s*"([^"\\r\\n]+)"\\s*(?:#.*)?$`,'gm'))];
  if(values.length!==1)throw new Error('Release manifest field is missing or duplicated.');
  return values[0][1];
}

export async function verifyReleaseVersion(directory,tag){
  const [pkg,lock,tauri,cargo,cargoLock]=await Promise.all([
    json(directory,'package.json'),json(directory,'package-lock.json'),
    json(directory,'src-tauri/tauri.conf.json'),text(directory,'src-tauri/Cargo.toml'),
    text(directory,'src-tauri/Cargo.lock'),
  ]);
  if(!versionPattern.test(pkg.version)||tag!==`v${pkg.version}`){
    throw new Error('Release tag must exactly match v followed by the package version.');
  }
  const packageBlock=cargo.match(/^\[package\][ \t]*\r?\n([\s\S]*?)(?=^\[|(?![\s\S]))/m)?.[1];
  if(!packageBlock)throw new Error('Cargo package manifest is missing.');
  const cargoName=field(packageBlock,'name');
  const lockedPackages=cargoLock.split(/^\[\[package\]\]\s*\r?\n/m).slice(1)
    .filter(block=>new RegExp(`^name\\s*=\\s*"${cargoName.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')}"\\s*$`,'m').test(block));
  if(lockedPackages.length!==1)throw new Error('Cargo lock package is missing or duplicated.');
  const versions=[lock.version,lock.packages?.['']?.version,tauri.version,
    field(packageBlock,'version'),field(lockedPackages[0],'version')];
  if(versions.some(version=>version!==pkg.version))throw new Error('Release manifest versions do not match.');
  return pkg.version;
}

if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  try{
    await verifyReleaseVersion(root,process.argv[2]??process.env.RELEASE_TAG);
    console.log('Release tag and manifest versions match.');
  }catch{
    console.error('Release version validation failed. Check the tag and every application manifest version.');
    process.exitCode=1;
  }
}
