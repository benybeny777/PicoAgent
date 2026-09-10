// 全キャラで機能を単独駆動し、全身の揺れを口・胸の成功と誤認しない。
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';
const modulePath = process.env.PICO_PLAYWRIGHT_MODULE;
if (!modulePath) throw new Error('PICO_PLAYWRIGHT_MODULEが必要です');
const { chromium } = await import(pathToFileURL(modulePath).href);
const out = path.resolve(process.argv[2] || 'temp/rig-feature-matrix');
await mkdir(out, {recursive:true});
const browser = await chromium.launch({channel:'chrome',headless:true});
const page = await browser.newPage({viewport:{width:1500,height:1000},deviceScaleFactor:1});
const errors = [], report = {};
page.on('pageerror', e => errors.push(e.message));
try {
  for (const id of ['photoreal','aoi','mugi','mugi-normal','rin','mai','sena']) {
    await page.goto(`http://127.0.0.1:8776/tools/mesh-rig-preview/?character=${id}&autoBlink=off`,{waitUntil:'networkidle'});
    await page.waitForFunction(()=>document.querySelector('#runtime')?.dataset.meshRig==='ready');
    const states = [
      ...['neutral','happy','concern','thinking','shy','angry'].map(expression=>({name:expression,expression})),
      ...[[0,0],[1,0],[0.3,1],[0.6,-1],[0.65,0.5],[1,-0.8]].map(([mouth,form],i)=>({name:`mouth-${i}`,mouth,form})),
      {name:'blink',eye:0},{name:'speech-on',speech:true},{name:'speech-off',speech:false},
    ];
    report[id] = [];
    for(const state of states) {
      const result = await page.evaluate(state=>{
        controller.app.ticker.stop(); controller.setMotion('idle');
        controller.setExpression(state.expression||'neutral');
        controller.setEyeOpen(state.eye??1); controller.setMouth(state.mouth??0,state.form??0);
        controller.angleX=controller.angleY=controller.gazeX=controller.gazeY=0;
        controller.speechEnvelope=0; controller.speechBody={value:0,velocity:0}; controller.chestFollow={value:0,velocity:0};
        if (state.speech !== undefined) {
          controller.setMouth(0.8,0);
          for(let i=0;i<20;i++) controller.updateMesh(controller.startedAt,1/60);
          if(!state.speech) {
            controller.setMouth(0,0);
            for(let i=0;i<600;i++) controller.updateMesh(controller.startedAt,1/60);
          }
          controller.angleX=controller.angleY=0;
        }
        controller.updateLvsAppearance(); controller.updateAlphas();
        controller.updateMesh(controller.startedAt,0); controller.updateBlush();
        controller.app.renderer.render(controller.app.stage);
        const face=controller.rig.anchors?.face;
        const first=controller.planes.values().next().value;
        const scale=first.plane.scale.x,rect=controller.canvas.getBoundingClientRect();
        const ox=first.plane.position.x-first.offset[0]*scale,oy=first.plane.position.y-first.offset[1]*scale;
        const sx=rect.width/controller.app.screen.width,sy=rect.height/controller.app.screen.height;
        const clip=face && !state.name.startsWith('speech') ? {x:rect.x+(ox+face.x0*scale)*sx,y:rect.y+(oy+face.y0*scale)*sy,width:(face.x1-face.x0)*scale*sx,height:(face.y1-face.y0)*scale*sy} : {x:rect.x,y:rect.y,width:rect.width,height:rect.height};
        const torso=[...controller.planes.values()].find(e=>e.item?.role==='torso'||e.motion==='torso');
        const eyePixels = [], sourceEyes = [];
        const eyes = ['eyeL','eyeR'].map(side=>{
          const eye=controller.rig.anchors?.[side];
          if(!eye) throw new Error('目の実測座標がないため開口の混入を検査できません');
          const resolution=controller.canvas.width/controller.app.screen.width;
          const x=Math.floor((ox+eye.x0*scale)*resolution),y=Math.floor((oy+eye.y0*scale)*resolution);
          const width=Math.ceil((eye.x1-eye.x0)*scale*resolution),height=Math.ceil((eye.y1-eye.y0)*scale*resolution);
          const crop=document.createElement('canvas');crop.width=width;crop.height=height;
          crop.getContext('2d').drawImage(controller.canvas,x,y,width,height,0,0,width,height);
          eyePixels.push(Array.from(crop.getContext('2d').getImageData(0,0,width,height).data));
          if(controller.lvsFace) {
            const box=controller.rig.sourceRig.layers.scene_face.texture_box;
            const data=controller.lvsFace.ctx.getImageData(Math.floor(eye.x0-box[0]),Math.floor(eye.y0-box[1]),Math.ceil(eye.x1-eye.x0),Math.ceil(eye.y1-eye.y0));
            sourceEyes.push(Array.from(data.data));
          }
          return crop.toDataURL();
        });
        return {clip,body:controller.speechBody.value,follow:controller.chestFollow.value,
          torsoVertices:torso?Array.from(torso.buffer.data):null,displayScale:scale*sy,eyes,eyePixels,sourceEyes};
      },state);
      for (const [side,value] of result.eyes.entries()) {
        await writeFile(path.join(out,`${id}-${state.name}-eye${side}.png`),Buffer.from(value.split(',')[1],'base64'));
      }
      result.eyes=result.eyes.map(value=>createHash('sha256').update(value).digest('hex'));
      result.sourceEyes=result.sourceEyes.map(value=>createHash('sha256').update(Buffer.from(value)).digest('hex'));
      await page.screenshot({path:path.join(out,`${id}-${state.name}.png`),clip:result.clip});
      report[id].push({name:state.name,...result});
    }
    const on=report[id].find(x=>x.name==='speech-on'),off=report[id].find(x=>x.name==='speech-off');
    for(let side=0;side<2;side++) {
      const mouthFrames=report[id].filter(x=>x.name.startsWith('mouth-'));
      if(mouthFrames[0].sourceEyes.length && new Set(mouthFrames.map(x=>x.sourceEyes[side])).size!==1) throw new Error(`${id}: 開口操作で原寸の目素材が変わっています`);
      const base=mouthFrames[0].eyePixels[side];
      for(const frame of mouthFrames) {
        const pixels=frame.eyePixels[side];let maximum=0,changed=0;
        for(let i=0;i<base.length;i+=4) {
          let delta=0;
          for(let j=0;j<4;j++)delta=Math.max(delta,Math.abs(base[i+j]-pixels[i+j]));
          maximum=Math.max(maximum,delta);if(delta)changed++;
        }
        // 原寸は完全一致を要求し、GPU縮小描画だけ量子化差を厳しく制限する。
        (frame.eyeDifferences??=[])[side]={maximum,changed,pixels:base.length/4};
        if(maximum>2||changed/(base.length/4)>0.001) throw new Error(`${id}: 開口操作で目の描画が変わっています (${maximum}, ${changed})`);
      }
    }
    if(!on.torsoVertices || !off.torsoVertices) throw new Error(`${id}: 胴体の検証対象がありません`);
    const delta=on.torsoVertices.filter((_,i)=>i%2===1).map((v,i)=>(v-off.torsoVertices[i*2+1])*on.displayScale);
    const localSpan=Math.max(...delta)-Math.min(...delta);
    report[id].push({name:'chest-local-span',pixels:localSpan});
    if(localSpan<1) throw new Error(`${id}: 胸の局所変位が不足しています (${localSpan}px)`);
    if(Math.abs(off.body)>0.0001||Math.abs(off.follow)>0.0001) throw new Error(`${id}: 発話停止後に収束しません`);
    console.log(`${id}: 単独機能撮影完了・胸の局所変位 ${localSpan.toFixed(2)}px（目視は別途）`);
  }
  if(errors.length) throw new Error(errors.join('\n'));
} catch (error) {
  errors.push(error.message);
  throw error;
} finally {
  for(const frames of Object.values(report))for(const frame of frames)delete frame.eyePixels;
  await writeFile(path.join(out,'report.json'),JSON.stringify({report,errors},null,2));
  await browser.close();
}
