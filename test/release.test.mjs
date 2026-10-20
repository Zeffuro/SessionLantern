import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,rm,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {stageReleaseFiles,writeChecksums} from '../scripts/release-artifacts.mjs';
import {publishDraft} from '../scripts/release-publish.mjs';
import {createHash} from 'node:crypto';
import {verifyReleaseVersion} from '../scripts/release-version.mjs';

const root=new URL('../',import.meta.url);
const files=['package.json','package-lock.json','src-tauri/tauri.conf.json','src-tauri/Cargo.toml','src-tauri/Cargo.lock'];
async function fixture(t,change){
  const directory=await mkdtemp(join(tmpdir(),'session-lantern-release-'));
  t.after(()=>rm(directory,{recursive:true,force:true}));
  await mkdir(join(directory,'src-tauri'));
  for(const file of files){
    const content=await readFile(new URL(file,root),'utf8');
    await writeFile(join(directory,file),change?.(file,content)??content);
  }
  return directory;
}

test('release tag must exactly match consistent application versions',async t=>{
  const directory=await fixture(t);
  const {version}=JSON.parse(await readFile(join(directory,'package.json'),'utf8'));
  assert.equal(await verifyReleaseVersion(directory,`v${version}`),version);
  for(const tag of [version,`v${version}-other`,'v999.0.0',undefined,'v0.2.0\n']){
    await assert.rejects(verifyReleaseVersion(directory,tag),/tag must exactly match/);
  }
});

test('each application version mismatch blocks a release',async t=>{
  const {version}=JSON.parse(await readFile(new URL('package.json',root),'utf8'));
  for(const target of files.slice(1)){
    await t.test(target,async t=>{
      const directory=await fixture(t,(file,content)=>{
        if(file!==target)return content;
        if(file.endsWith('.json')){
          const value=JSON.parse(content);value.version='999.0.0';return JSON.stringify(value);
        }
        if(file.endsWith('Cargo.lock'))return content.replace(/(\[\[package\]\]\s*\r?\nname = "session-lantern"\r?\nversion = ")[^"]+/,(_,prefix)=>`${prefix}999.0.0`);
        return content.replace(/(^version\s*=\s*")[^"]+/m,(_,prefix)=>`${prefix}999.0.0`);
      });
      await assert.rejects(verifyReleaseVersion(directory,`v${version}`),/versions do not match/);
    });
  }
  const directory=await fixture(t,(file,content)=>{
    if(file!=='package-lock.json')return content;
    const value=JSON.parse(content);value.packages[''].version='999.0.0';return JSON.stringify(value);
  });
  await assert.rejects(verifyReleaseVersion(directory,`v${version}`),/versions do not match/);
});

test('desktop workflow builds every native platform before a draft release',async()=>{
  const workflow=await readFile(new URL('.github/workflows/desktop.yml',root),'utf8');
  const actions=[...workflow.matchAll(/uses: ([\w/-]+)@([^\s]+)/g)];
  assert.ok(actions.length>=5);
  for(const [,name,ref] of actions){
    assert.match(ref,/^[a-f0-9]{40}$/,name);
    assert.ok(['actions/checkout','actions/setup-node','dtolnay/rust-toolchain','actions/upload-artifact','actions/download-artifact'].includes(name));
  }
  for(const required of ["tags: ['v*']","branches: ['**']",'pull_request:','workflow_dispatch:',
    "node-version: '24'",'persist-credentials: false','npm ci','npm run check','npm test',
    'cargo test --manifest-path src-tauri/Cargo.toml --locked','--features native-smoke',
    'windows-2022','ubuntu-22.04','macos-15','macos-15-intel',
    'x86_64-pc-windows-msvc','x86_64-unknown-linux-gnu','aarch64-apple-darwin','x86_64-apple-darwin',
    'bundles: nsis','bundles: appimage,deb','bundles: dmg','node: arm64',
    'libwebkit2gtk-4.1-dev','libfuse2','fail-fast: false','needs: build','contents: read',
    'node scripts/release-version.mjs','node scripts/release-artifacts.mjs stage',
    'node scripts/release-artifacts.mjs checksum','node scripts/release-publish.mjs',
    'merge-multiple: true','if-no-files-found: error'])assert.ok(workflow.includes(required),required);
  assert.equal((workflow.match(/secrets\./g)??[]).length,1);
  for(const script of workflow.split(/\n\s+run: /).slice(1)){
    assert.doesNotMatch(script.split(/\n\s+- (?:name:|uses:|run:)/)[0],/\$\{\{/);
  }
});

const platforms=[
  ['x86_64-pc-windows-msvc','windows-x64',[['nsis','synthetic_x64-setup.exe','-setup.exe']]],
  ['x86_64-unknown-linux-gnu','linux-x64',[['appimage','synthetic.AppImage','.AppImage'],['deb','synthetic.deb','.deb']]],
  ['aarch64-apple-darwin','macos-arm64',[['dmg','synthetic_aarch64.dmg','.dmg']]],
  ['x86_64-apple-darwin','macos-x64',[['dmg','synthetic_x64.dmg','.dmg']]],
];
async function artifactFixture(t,target,bundles){
  const directory=await mkdtemp(join(tmpdir(),'session-lantern-artifacts-'));
  t.after(()=>rm(directory,{recursive:true,force:true}));
  const release=join(directory,'src-tauri/target',target,'release');
  await mkdir(release,{recursive:true});
  if(target.includes('windows'))await writeFile(join(release,'session-lantern.exe'),'synthetic portable');
  for(const [bundle,name] of bundles){
    await mkdir(join(release,'bundle',bundle),{recursive:true});
    await writeFile(join(release,'bundle',bundle,name),`synthetic ${bundle}`);
  }
  return directory;
}

test('stages native artifacts and checksums exactly the complete release',async t=>{
  const combined=await mkdtemp(join(tmpdir(),'session-lantern-combined-'));
  t.after(()=>rm(combined,{recursive:true,force:true}));
  await mkdir(join(combined,'release-files'));
  const expected=[];
  for(const [target,platform,bundles] of platforms){
    const directory=await artifactFixture(t,target,bundles);
    await stageReleaseFiles(directory,target,'0.2.0');
    const outputs=[...bundles.map(([bundle,,suffix])=>[suffix,`synthetic ${bundle}`])];
    if(target.includes('windows'))outputs.push(['.exe','synthetic portable']);
    for(const [suffix,content] of outputs){
      const name=`Session-Lantern-0.2.0-${platform}${suffix}`;
      assert.equal(await readFile(join(directory,'release-files',name),'utf8'),content);
      await writeFile(join(combined,'release-files',name),content);
      expected.push(`${createHash('sha256').update(content).digest('hex')}  ${name}\n`);
    }
  }
  await writeChecksums(combined,'0.2.0');
  assert.equal(await readFile(join(combined,'release-files/SHA256SUMS.txt'),'utf8'),expected.sort((a,b)=>a.split('  ')[1].localeCompare(b.split('  ')[1],'en')).join(''));
});

test('artifact staging rejects unsupported, missing and duplicate native outputs',async t=>{
  await assert.rejects(stageReleaseFiles('unused','other','0.2.0'),/Unsupported/);
  await assert.rejects(stageReleaseFiles('unused',platforms[0][0],'../../escape'),/Unsupported/);
  const [target,,bundles]=platforms[1];
  const missing=await artifactFixture(t,target,[]);
  await assert.rejects(stageReleaseFiles(missing,target,'0.2.0'));
  const duplicate=await artifactFixture(t,target,bundles);
  await writeFile(join(duplicate,'src-tauri/target',target,'release/bundle/deb/extra.deb'),'extra');
  await assert.rejects(stageReleaseFiles(duplicate,target,'0.2.0'),/Expected one deb/);
  const partial=await artifactFixture(t,target,bundles);
  await stageReleaseFiles(partial,target,'0.2.0');
  await assert.rejects(writeChecksums(partial,'0.2.0'),/missing or unexpected/);
});

test('draft publishing refuses published releases and checks each gh failure',async t=>{
  const directory=await mkdtemp(join(tmpdir(),'session-lantern-publish-'));
  t.after(()=>rm(directory,{recursive:true,force:true}));
  await mkdir(join(directory,'release-files'));
  await writeFile(join(directory,'release-files/SHA256SUMS.txt'),'synthetic');
  for(const mode of ['published','draft','missing','create-failure','upload-failure','became-published','view-failure']){
    const calls=[];
    let views=0;
    const run=args=>{
      calls.push(args);
      const action=args[1];
      if(action==='view'){
        views++;
        if(views===1&&['missing','create-failure'].includes(mode))return {status:1,stdout:''};
        if(views===2&&mode==='view-failure')return {status:1,stdout:''};
        return {status:0,stdout:JSON.stringify({isDraft:mode!=='published'&&!(views===2&&mode==='became-published'),isPrerelease:false})};
      }
      return {status:(action==='create'&&mode==='create-failure')||(action==='upload'&&mode==='upload-failure')?1:0};
    };
    if(['draft','missing'].includes(mode))await publishDraft(directory,'v0.2.0',run);
    else await assert.rejects(publishDraft(directory,'v0.2.0',run),/published release|failed|must be a draft/);
    if(['published','became-published','view-failure','create-failure'].includes(mode)){
      assert.ok(calls.every(args=>args[1]!=='upload'),mode);
    }
    if(mode==='missing')assert.deepEqual(calls.find(args=>args[1]==='create'),[
      'release','create','v0.2.0','--verify-tag','--draft','--generate-notes','--title','Session Lantern v0.2.0',
    ]);
    if(['draft','missing'].includes(mode))assert.equal(calls.at(-1).at(-1),'--clobber');
  }
  await assert.rejects(publishDraft(directory,'--unexpected'),/Use a version tag/);
});

test('prerelease tags stay excluded from stable releases and mismatched drafts block upload',async t=>{
  const directory=await mkdtemp(join(tmpdir(),'session-lantern-prerelease-'));
  t.after(()=>rm(directory,{recursive:true,force:true}));
  await mkdir(join(directory,'release-files'));
  await writeFile(join(directory,'release-files/SHA256SUMS.txt'),'synthetic');
  for(const [tag,prerelease] of [['v0.2.1-beta.1',true],['v0.2.1+build-beta.1',false]]){
    const calls=[];
    const run=args=>{
      calls.push(args);
      if(calls.length===1)return {status:1};
      return {status:0,stdout:JSON.stringify({isDraft:true,isPrerelease:prerelease})};
    };
    await publishDraft(directory,tag,run);
    assert.equal(calls.find(args=>args[1]==='create').includes('--prerelease'),prerelease);
    const blocked=[];
    await assert.rejects(publishDraft(directory,tag,args=>{
      blocked.push(args);
      return {status:0,stdout:JSON.stringify({isDraft:true,isPrerelease:!prerelease})};
    }),/prerelease flag must match/);
    assert.equal(blocked.length,1);
  }
});
