import { readFileSync, readdirSync, existsSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import assert from 'node:assert/strict';
const settings = readFileSync(new URL('../../ui/settings.js', import.meta.url),'utf8');
const imageSource = readFileSync(new URL('../../ui/character/image.js', import.meta.url),'utf8');
function harness(manifest, { failImage=false, contextNull=false, exportFail=false, rig, fetchFail=false }={}) {
  const requests=[];let rigFetches=0;
  const sandbox={setTimeout,clearTimeout,Number,Image:class {
    naturalWidth=5000;naturalHeight=5000;removeAttribute(){}
    set src(url){requests.push(url);queueMicrotask(()=>failImage?this.onerror():this.onload());}
  },fetch:async()=>{rigFetches++;return {ok:!fetchFail,status:404,json:async()=>rig};},document:{createElement:()=>({getContext:()=>contextNull?null:{drawImage(){}},toDataURL:()=>{if(exportFail)throw Error('書出し拒否');return 'data:image/png;base64,AA';}})},
  loadCharacterManifestForSettings:async()=>manifest,errorDetail:e=>e.message,logApp(){}};
  vm.createContext(sandbox);
  vm.runInContext(settings.slice(settings.indexOf('const characterIconRigCache'),settings.indexOf('\nfunction characterRenderType'))+settings.slice(settings.indexOf('function characterRenderKind'),settings.indexOf('\nfunction applyCharacterFilters')),sandbox);
  return {sandbox,requests,rigFetches:()=>rigFetches,run:face=>sandbox.characterIconUrl({id:'試験',root:'pack'},{face})};
}
const goodRig={schema_version:3,profile:"lvs-anime25d-v1",layers:{face:{bbox:[20,20,60,60]},neutral:{url:'parts/neutral.png',texture_box:[0,0,100,100]}}};
test('宣言した一枚の404は理由を返し別画像を一切読まない',async()=>{
  const h=harness({animations:{idle:{path:'idle'}},meshRig:{rig:'rig.json'},boot:{sleeping:'sleep.png'}},{failImage:true,rig:{mode:'layered-grid-v2',sourceCanvas:[100,100],canvas:[100,100]}});
  const r=await h.run(true);assert.equal(r.ok,false);assert.match(r.reason,/読み込めません/);assert.deepEqual(h.requests,['pack/idle/00.png']);
});
test('顔切出しの定義とcanvas失敗は代替画像へ進まない',async()=>{
  for(const opts of [{contextNull:true},{exportFail:true},{rig:{...goodRig,layers:{...goodRig.layers,face:{bbox:[0,0,NaN,20]}}}},{rig:{...goodRig,layers:{...goodRig.layers,neutral:{...goodRig.layers.neutral,texture_box:[70,70,100,100]}}}}]){
    const h=harness({meshRig:{rig:'rig.json'}},{rig:goodRig,...opts});const r=await h.run(true);assert.equal(r.ok,false);assert.ok(r.reason);assert.equal(h.requests.length,1);
  }
  const h=harness({meshRig:{rig:'rig.json'}},{fetchFail:true});assert.equal((await h.run(true)).ok,false);assert.equal(h.requests.length,0);
});
test('顔なし構図も同じ宣言画像を使い画像なし宣言は失敗と区別する',async()=>{
  const h=harness({meshRig:{rig:'rig.json'}},{rig:goodRig});assert.equal((await h.run(false)).url,'pack/parts/neutral.png');assert.equal(h.requests.length,1);
  for(const m of [{live2d:{model:'x'}},{mmd:{model:'x'}}]){const n=harness(m);const r=await n.run(true);assert.equal(r.ok,true);assert.equal(r.imageAbsent,true);assert.equal(n.requests.length,0);}
  assert.equal((await harness({}).run(true)).ok,false);
});
test('画素取得とcanvas不成立は全画像範囲へ降格せず親へ投げる',()=>{
  const block=imageSource.slice(imageSource.indexOf('function measureContent'),imageSource.indexOf('\nexport class'));
  for(const context of [null,{drawImage(){},getImageData(){throw Error('画素拒否');}}]){
    const box={getContext:()=>context};const ctx={el:()=>box};vm.createContext(ctx);vm.runInContext(block,ctx);assert.throws(()=>ctx.measureContent({naturalWidth:10,naturalHeight:20}),/canvas|画素拒否/);
  }
});
test('全13キャラは同じ宣言選択境界を通り選択資産が存在する',()=>{
  const root=new URL('../../characters/',import.meta.url);let count=0;
  for(const id of readdirSync(root)){
    const file=new URL(`${id}/character.json`,root);if(!existsSync(file))continue;count++;
    const manifest=JSON.parse(readFileSync(file));const h=harness(manifest);const rig = manifest.meshRig ? JSON.parse(readFileSync(new URL(`${id}/${manifest.meshRig.rig}`,root))) : null;const selected=h.sandbox.characterIconSource(manifest,rig);
    assert.ok(['image','mesh','none'].includes(selected.kind),id);
    if(selected.kind==='none')assert.ok(manifest.live2d||manifest.mmd,id);
    else assert.ok(existsSync(new URL(`${id}/${selected.path}`,root)),`${id}: ${selected.path}`);
    if(selected.kind==='mesh'){
      const rig=JSON.parse(readFileSync(new URL(`${id}/${selected.path}`,root)));
      h.sandbox.iconBox(rig.layers.face?.bbox||rig.layers.scene_face?.texture_box,'顔');h.sandbox.iconBox(rig.layers.neutral?.texture_box,'合成画像');
      assert.ok(existsSync(new URL(`${id}/${selected.path.replace(/[^/]+$/,'')}${rig.layers.neutral.url.replace(/^.*\/parts\//,'parts/')}`,root)),id);
    }
  }
  assert.equal(count,13);
});
test('カードの失敗は短文と理由を表示し画像なしだけ頭文字になる',()=>{
  assert.doesNotMatch(settings,/firstLoadableImage/);
  assert.match(settings,/fallback\.textContent = "アイコンを作れません"/);
  assert.match(settings,/fallback\.title = reason/);
  assert.match(settings,/if \(result\.imageAbsent\) \{ fallback\.textContent = initial/);
});

test('リグ型は存在確認に先立ち確定し一度読んだ定義を顔と構図で共有する',async()=>{
  const manifest={meshRig:{rig:'rig.json'},animations:{idle:{path:'idle'}}};
  const h=harness(manifest,{rig:goodRig});assert.equal((await h.run(true)).url,'data:image/png;base64,AA');assert.equal((await h.run(false)).url,'pack/parts/neutral.png');assert.equal(h.rigFetches(),1);assert.ok(h.requests.every(url=>url==='pack/parts/neutral.png'));
  for(const rig of [{mode:'不明'},{mode:'layered-grid-v2',sourceCanvas:[NaN,10],canvas:[10,10]}]){const bad=harness(manifest,{rig});assert.equal((await bad.run(true)).ok,false);assert.equal(bad.requests.length,0);}
});
