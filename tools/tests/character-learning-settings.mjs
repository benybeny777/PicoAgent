import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

// 利用者操作で状態を変えるDOM。属性検査だけでなく、非同期応答と再描画を実行する。
class Element {
  constructor(tag, props={}) {this.tag=tag;this.children=[];this.listeners={};this.value="";this.disabled=false;Object.assign(this,props);this.dataset={};this.pauses=0;this.loads=0;}
  append(...items){this.children.push(...items);if(this.tag==="select" && !this.value)this.value=this.children[0]?.value || "";}
  replaceChildren(...items){this.children=items;}
  setAttribute(name,value){this[name]=value;}
  removeAttribute(name){delete this[name];}
  addEventListener(name,fn){(this.listeners[name] ||= []).push(fn);}
  async fire(name){await Promise.all((this.listeners[name] || []).map(fn=>fn({target:this})));await settle();}
  querySelectorAll(tag){return this.children.flatMap(child=>child instanceof Element ? [...(child.tag===tag?[child]:[]),...child.querySelectorAll(tag)] : []);}
  pause(){this.pauses++;}
  load(){this.loads++;}
}
const settle=async()=>{for(let i=0;i<6;i++)await Promise.resolve();};
const deferred=()=>{let resolve;const promise=new Promise(r=>resolve=r);return {promise,resolve};};
function harness(jobs=[],quality=true){
  const state={jobs,activeId:null,supported:true,runtimeInstalled:true};
  const calls=[],timers=new Map();let timerId=0,target={id:"mugi",name:"むぎ"};
  const api={characterLearningList:async()=>state,characterLearningStart:async(...args)=>calls.push(["start",...args]),characterLearningSetup:async()=>{},characterLearningCancel:async id=>calls.push(["cancel",id]),characterLearningSample:async(id,index)=>({url:`sample:${index}`,text:`文${index}`}),characterLearningCheckVoice:async()=>{}};
  const context=vm.createContext({window:{picoagent:{settings:api},PicoUI:{errorMessage:String}},document:{createTextNode:text=>text},setTimeout:fn=>{timers.set(++timerId,fn);return timerId;},clearTimeout:id=>timers.delete(id)});
  let source=readFileSync(new URL("../../ui/character-learning-settings.js",import.meta.url),"utf8");
  // 検査が退行を検出することを、作業ツリーを戻さず実行できる。
  if(process.env.LEARNING_SETTINGS_REGRESSION==="lock") source=source.replace("lock(pending || !!active || !supported)","lock(!!active || !supported)");
  if(process.env.LEARNING_SETTINGS_REGRESSION==="draft") source=source.replace("drafts.get(job.id) ?? job.persona.profile","job.persona.profile");
  if(process.env.LEARNING_SETTINGS_REGRESSION==="listen") source=source.replace("heard.checked && played.size>=2","heard.checked");
  vm.runInContext(source,context);
  let section;
  const build=()=>{section=context.window.PicoCharacterLearning.build({el:(tag,props,children=[])=>{const e=new Element(tag,props);e.append(...children);return e;},section:()=>new Element("section"),getTarget:()=>target,applyPersona:(...args)=>calls.push(["persona",...args]),applyVoice:(...args)=>calls.push(["voice",...args])});return section;};
  const button=text=>section.querySelectorAll("button").find(e=>e.textContent===text);
  const job={id:"job1",input:{characterId:"mugi",characterName:"むぎ",mode:"both"},state:"ready",persona:{profile:"元草稿",evidence:["文字起こしの根拠"]},result:{artifactPaths:{aivmx:"candidate.aivmx"},quality:{defaultAssignable:quality},voice:{modelUuid:"voice1"}}};
  return {state,calls,api,timers,build,button,job,setTarget:v=>target=v,get section(){return section;}};
}
test("開始中の古い空一覧はロックを解除せず対象と素材を固定する",async()=>{
  const h=harness();h.build();await settle();const wait=deferred();h.api.characterLearningStart=async input=>{h.calls.push(input);await wait.promise;};
  h.section.querySelectorAll("input")[0].value="C:/音声.wav";
  await h.button("候補を作る").fire("click");h.setTarget({id:"other",name:"別キャラ"});
  await h.button("結果を再読み込み").fire("click");assert.equal(h.button("候補を作る").disabled,true);
  await h.button("候補を作る").fire("click");assert.equal(h.calls.length,1);assert.equal(h.calls[0].characterId,"mugi");wait.resolve();await settle();
});
test("YouTube URLを選び入力して開始するとURLとGPU既定または明示CPUを送る",async()=>{
  for(const device of ["cuda","cpu"]){
    const h=harness();h.build();await settle();
    const selects=h.section.querySelectorAll("select");
    const kind=selects.find(e=>e.children.some(option=>option.value==="url"));
    const asr=selects.find(e=>e.ariaLabel==="文字起こしの処理");
    assert.equal(asr.value,"cuda");
    kind.value="url";await kind.fire("change");assert.equal(h.button("素材を選ぶ").disabled,true);
    h.section.querySelectorAll("input")[0].value="  https://www.youtube.com/watch?v=example1234  ";
    if(device==="cpu"){asr.value="cpu";await asr.fire("change");}
    await h.button("候補を作る").fire("click");
    const input=h.calls.find(call=>call[0]==="start")[1];
    assert.equal(input.sourceKind,"url");assert.equal(input.sourceValue,"https://www.youtube.com/watch?v=example1234");assert.equal(input.asrDevice,device);
  }
});
test("GPUの開始失敗後も処理方式をCPUへ自動変更しない",async()=>{
  const h=harness();h.api.characterLearningStart=async()=>{throw new Error("GPUを利用できません");};h.build();await settle();
  h.section.querySelectorAll("input")[0].value="C:/音声.wav";await h.button("候補を作る").fire("click");await settle();
  assert.equal(h.section.querySelectorAll("select").find(e=>e.ariaLabel==="文字起こしの処理").value,"cuda");
  assert.ok(h.section.querySelectorAll("p").some(e=>e.textContent==="Error: GPUを利用できません"));
});
test("ジョブは省略された旧CPUと新GPUを区別し実処理の方式と理由を優先表示する",async()=>{
  for(const [input,progress,expected] of [[undefined,undefined,"CPU"],["cuda",undefined,"GPU"],["cuda",{asr:{device:"cpu",reason:"このジョブはCPUを明示指定して実行しました"}},"CPU"]]){
    const h=harness();h.job.input.asrDevice=input;h.job.progress=progress;h.state.jobs=[h.job];h.build();await settle();
    const texts=h.section.querySelectorAll("p").map(e=>e.textContent);
    assert.ok(texts.includes(`文字起こし: ${expected}`));
    if(progress)assert.ok(texts.includes(progress.asr.reason));
  }
  const setup=harness();setup.job.input.mode="setup";setup.state.jobs=[setup.job];setup.build();await settle();
  assert.equal(setup.section.querySelectorAll("p").some(e=>e.textContent?.startsWith("文字起こし:")),false);
});
test("進捗再取得は編集と試聴を維持し画面再構築後も草稿を保持する",async()=>{
  const h=harness();h.state.jobs=[h.job];h.build();await settle();let draft=h.section.querySelectorAll("textarea")[0];draft.value="編集した人格";await draft.fire("input");
  assert.ok(h.section.querySelectorAll("p").some(e=>e.textContent==="文字起こしの根拠"));
  const audio=h.section.querySelectorAll("audio")[0];await h.button("試聴音声を読み込む").fire("click");const pauses=audio.pauses;
  await h.button("結果を再読み込み").fire("click");assert.equal(h.section.querySelectorAll("audio")[0],audio);assert.equal(audio.pauses,pauses);
  h.build();await settle();assert.equal(h.section.querySelectorAll("textarea")[0].value,"編集した人格");assert.ok(audio.loads>0);assert.equal(audio.src,undefined);
  await h.button("編集した人格を設定へ反映").fire("click");assert.equal(h.calls[0][2],"編集した人格");
  h.setTarget({id:"other"});await h.button("編集した人格を設定へ反映").fire("click");assert.equal(h.calls.length,1);
});
test("画面破棄後に届いた試聴応答は音声を再確保しない",async()=>{
  const h=harness();h.state.jobs=[h.job];h.build();await settle();const audio=h.section.querySelectorAll("audio")[0];const wait=deferred();h.api.characterLearningSample=()=>wait.promise;
  const pending=h.button("試聴音声を読み込む").fire("click");h.build();await settle();wait.resolve({url:"late-sample"});await pending;assert.equal(audio.src,undefined);assert.ok(audio.loads>0);
});
test("二つの異なる文の再生完了と了承とQA合格が揃うまで声は反映できない",async()=>{
  for(const quality of [false,true]){
    const h=harness([],quality);h.state.jobs=[h.job];h.build();await settle();const heard=h.section.querySelectorAll("input").find(e=>e.type==="checkbox");heard.checked=true;await heard.fire("change");
    const accept=h.button("確認した声を設定へ反映");assert.equal(accept.disabled,true);const audio=h.section.querySelectorAll("audio")[0];const samples=h.section.querySelectorAll("select").at(-1);
    for(const index of [0,0,1]){samples.value=String(index);await h.button("試聴音声を読み込む").fire("click");await audio.fire("ended");if(index===0)assert.equal(accept.disabled,true);}
    assert.equal(accept.disabled,!quality);h.setTarget({id:"other"});await accept.fire("click");assert.equal(h.calls.length,0);
    h.setTarget({id:"mugi"});await accept.fire("click");assert.equal(h.calls.length,quality?1:0);
  }
});
test("中断した学習の再開と実行中の取消を識別子で行い画面破棄時タイマーを解放する",async()=>{
  const h=harness();h.job.state="interrupted";h.state.jobs=[h.job];h.build();await settle();await h.button("この学習を再開").fire("click");await settle();assert.deepEqual(h.calls[0],["start",null,"job1"]);
  h.state.activeId="job1";await h.button("結果を再読み込み").fire("click");await h.button("停止する").fire("click");assert.deepEqual(h.calls.at(-1),["cancel","job1"]);assert.ok(h.timers.size>0);
  h.state.activeId=null;h.build();await settle();assert.equal(h.timers.size,0);
});
test("学習した同一speakerの保存はmodelUuidを保持し手動変更だけでメタデータを解除する",()=>{
  const settings=readFileSync(new URL("../../ui/settings.js",import.meta.url),"utf8");
  const begin=settings.indexOf("function saveCurrentCharacterVoice(id)");
  const end=settings.indexOf("function formatNumber",begin);
  const voice={speaker:"12345",modelUuid:"learned-model",speakerUuid:"learned-speaker",styleName:"通常",sourceUrl:"素材"};
  const context=vm.createContext({characterSpeaker:{value:"",placeholder:""},characterVoiceOverrides:{mugi:{...voice}},characterDefaultVoices:{mugi:{styleId:"12345"}},inputs:new Map()});
  vm.runInContext(settings.slice(begin,end),context);
  vm.runInContext('loadCharacterVoice("mugi");saveCurrentCharacterVoice("mugi")',context);
  assert.equal(context.characterVoiceOverrides.mugi.modelUuid,"learned-model");
  context.characterSpeaker.value="54321";vm.runInContext('saveCurrentCharacterVoice("mugi")',context);
  assert.equal(context.characterVoiceOverrides.mugi.speaker,"54321");assert.equal(context.characterVoiceOverrides.mugi.modelUuid,undefined);
});
