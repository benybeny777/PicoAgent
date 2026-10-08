// character-learning-settings.js - 学習素材から作った候補を確認し、設定の草稿へ反映する。
(() => {
  let dispose = () => {};
  const drafts = new Map();
  const stageNames = {download:"素材を取得中",snapshot:"素材を保存中",transcribe:"文字起こし中",measure:"音声品質を測定中",select:"学習素材を選別中",preprocess:"学習の準備中",train:"音声を学習中",qa:"合成音声を検査中",export:"音声を書き出し中",persona:"人格の草稿を作成中",prepared:"素材の準備完了",candidate:"学習候補を作成済み"};
  function build({el,section,getTarget,applyPersona,applyVoice}) {
    dispose();
    const sec = section("声・人格を学習する");
    sec.dataset.characterLearning = "true";
    sec.append(el("p",{className:"hint",textContent:"音声ファイルやYouTube URLから、選択中キャラの声と話し方の候補を作ります。一人の声が明瞭に入った素材を使ってください。現在はWindows向けです。"}));
    const mode = el("select",{},[el("option",{value:"both",textContent:"声と人格"}),el("option",{value:"persona",textContent:"人格だけ"}),el("option",{value:"voice",textContent:"声だけ"})]);
    const sourceKind = el("select",{},[el("option",{value:"file",textContent:"音声・動画ファイル"}),el("option",{value:"url",textContent:"YouTube URL"})]);
    const source = el("input",{type:"text",placeholder:"音声ファイルを選ぶか、YouTube URLを入力"});
    const pick = el("button",{type:"button",textContent:"素材を選ぶ"});
    const runtime = el("input",{type:"text",placeholder:"空欄ならPicoAgent管理の学習環境"});
    const pickRuntime = el("button",{type:"button",textContent:"既存の学習環境を選ぶ"});
    const setup = el("button",{type:"button",textContent:"学習環境を準備する"});
    const start = el("button",{type:"button",textContent:"候補を作る"});
    const refresh = el("button",{type:"button",textContent:"結果を再読み込み"});
    const status = el("p",{className:"hint"});
    status.setAttribute("aria-live","polite");
    const list = el("div",{className:"learning-job-list"});
    const line = (label,input) => el("div",{className:"field"},[el("label",{textContent:label}),input]);
    const advanced = el("details",{},[el("summary",{textContent:"既存の学習環境を使う"}),line("Style-Bert-VITS2のフォルダー",runtime),pickRuntime]);
    sec.append(line("作成するもの",mode),line("素材の種類",sourceKind),line("素材",source),pick,
      el("p",{className:"hint",textContent:"音声学習にはNVIDIA GPUと十分な量の音声が必要です。学習環境の初回準備は数GB以上を取得します。通常起動では取得しません。人格の作成には選択中AIを使い、クラウドAIの場合は文字起こしの抜粋を送ります。"}),setup,advanced,
      el("div",{className:"learning-actions"},[start,refresh]),status,list);
    let timer = null;
    let disposed = false;
    let active = null;
    let loading = false;
    let pending = false;
    let supported = false;
    const cards = new Map();
    const audios = new Set();
    dispose = () => { disposed = true; clearTimeout(timer); for (const audio of audios) {audio.pause();audio.removeAttribute("src");audio.load();} audios.clear(); };
    function fail(error) { if (!disposed) status.textContent = window.PicoUI.errorMessage(error); }
    function lock(running) {start.disabled = setup.disabled = !!running;}
    sourceKind.addEventListener("change",()=>{pick.disabled=sourceKind.value!=="file";});
    pick.addEventListener("click",async()=>{try {const result=await window.picoagent.settings.characterLearningPick(false);if (!disposed && result.path) {sourceKind.value="file";source.value=result.path;pick.disabled=false;}}catch(error){fail(error);}});
    pickRuntime.addEventListener("click",async()=>{try {const result=await window.picoagent.settings.characterLearningPick(true);if (!disposed && result.path) runtime.value=result.path;}catch(error){fail(error);}});
    async function run(action) {
      if (pending || active || !supported || disposed) return;
      pending=true;
      lock(true);status.textContent="処理を開始しています…";
      const poll = () => {clearTimeout(timer);timer=setTimeout(async()=>{await reload();if (!disposed && start.disabled) poll();},1200);};
      poll();
      try {await action();if (!disposed) status.textContent="候補を作成しました。結果を確認してください。";}catch(error){fail(error);}
      finally {pending=false;if (!disposed) {clearTimeout(timer);await reload();lock(!!active || !supported);}}
    }
    start.addEventListener("click",()=>{
      const target=getTarget();
      if (!target?.id) {status.textContent="先に適用先のキャラを選択してください。";return;}
      if (!source.value.trim()) {status.textContent="音声ファイルかYouTube URLを指定してください。";return;}
      const input={characterId:target.id,characterName:target.name,sourceKind:sourceKind.value,sourceValue:source.value.trim(),mode:mode.value,runtimeRoot:runtime.value.trim()};
      run(()=>window.picoagent.settings.characterLearningStart(input,null));
    });
    setup.addEventListener("click",()=>run(()=>window.picoagent.settings.characterLearningSetup()));
    refresh.addEventListener("click",reload);
    function sameTarget(job) {
      if (getTarget()?.id!==job.input.characterId) {status.textContent=`適用先の「${job.input.characterName}」をキャラ選択で選んでください。`;return false;}
      return true;
    }
    function card(job) {
      const box=el("div",{className:"learning-job"});
      const title=el("strong",{textContent:job.input.characterName});
      const detail=el("p",{className:"hint",textContent:job.error || job.progress?.error || job.progress?.message || (job.input.mode === "setup" && job.state === "ready" ? "学習環境を確認しました" : "") || stageNames[job.progress?.stage] || ({ready:"候補の確認待ち",running:"処理中",interrupted:"前回の処理が中断されました",cancelled:"停止しました",failed:"処理に失敗しました"}[job.state] || job.state)});
      box.append(title,detail);
      if (job.id===active) {
        const cancel=el("button",{type:"button",textContent:"停止する"});
        cancel.addEventListener("click",async()=>{cancel.disabled=true;try {await window.picoagent.settings.characterLearningCancel(job.id);status.textContent="停止を要求しました。処理の回収を待っています…";}catch(error){fail(error);cancel.disabled=false;}});
        box.append(cancel);return box;
      }
      if (job.persona?.profile) {
        const draft=el("textarea",{value:drafts.get(job.id) ?? job.persona.profile,maxLength:4000,rows:6});
        draft.addEventListener("input",()=>drafts.set(job.id,draft.value));
        draft.setAttribute("aria-label",`${job.input.characterName}の人格草稿`);
        const evidence=el("details",{},[el("summary",{textContent:"文字起こしからの根拠"}),...(job.persona.evidence || []).map(text=>el("p",{textContent:text}))]);
        const apply=el("button",{type:"button",textContent:"編集した人格を設定へ反映"});
        apply.addEventListener("click",()=>{if (!sameTarget(job)) return;if (!draft.value.trim()){status.textContent="人格の草稿が空です。";return;}applyPersona(job.input.characterId,draft.value.trim());status.textContent="人格を設定の草稿へ反映しました。設定を保存すると適用されます。";});
        box.append(line("人格の草稿（編集できます）",draft),evidence,apply);
      }
      if (job.result?.artifactPaths?.aivmx) {
        box.append(el("p",{className:"hint",textContent:job.result.quality?.defaultAssignable?"合成検査に合格しました。複数文を試聴して確認してください。":"学習候補・品質確認中。学習時と読み上げエンジンの両方の合成検査に合格するまでキャラへ割り当てません。"}));
        const check=el("button",{type:"button",textContent:"読み上げエンジンで品質を検査"});
        check.disabled=!!active || pending;
        check.addEventListener("click",()=>run(()=>window.picoagent.settings.characterLearningCheckVoice(job.id)));
        const audio=el("audio",{controls:true});audios.add(audio);
        const played=new Set();let loadedSample=null;
        const samples=el("select");
        for(let i=0;i<30;i++) samples.append(el("option",{value:String(i),textContent:`試聴 ${i+1}`}));
        const listen=el("button",{type:"button",textContent:"試聴音声を読み込む"});
        listen.addEventListener("click",async()=>{listen.disabled=true;const index=Number(samples.value);try {const sample=await window.picoagent.settings.characterLearningSample(job.id,index);if (!disposed && audios.has(audio)) {audio.pause();loadedSample=index;audio.src=sample.url;status.textContent=sample.text || "再生ボタンで試聴してください。";}}catch(error){fail(error);}finally{listen.disabled=false;}});
        const heard=el("input",{type:"checkbox"});
        const accept=el("button",{type:"button",textContent:"確認した声を設定へ反映",disabled:true});
        const updateAccept=()=>{accept.disabled=!(heard.checked && played.size>=2 && job.result.quality?.defaultAssignable && job.result.voice);};
        audio.addEventListener("ended",()=>{if(loadedSample!==null) played.add(loadedSample);updateAccept();});
        heard.addEventListener("change",updateAccept);
        accept.addEventListener("click",()=>{if (!sameTarget(job) || accept.disabled) return;applyVoice(job.input.characterId,job.result.voice);status.textContent="声を設定の草稿へ反映しました。設定を保存すると適用されます。";});
        box.append(check,samples,listen,audio,el("label",{className:"learning-listened"},[heard,document.createTextNode("複数文を聴き、声・ノイズ・発音を確認しました")]),accept);
      }
      if (["failed","cancelled","interrupted"].includes(job.state) && job.input.mode!=="setup") {
        const resume=el("button",{type:"button",textContent:"この学習を再開",disabled:!!active || pending});
        resume.addEventListener("click",()=>run(()=>window.picoagent.settings.characterLearningStart(null,job.id)));
        box.append(resume);
      }
      return box;
    }
    async function reload() {
      if (loading || disposed) return;
      loading=true;
      try {
        const result=await window.picoagent.settings.characterLearningList();
        if (disposed) return;
        active=result.activeId;supported=!!result.supported;
        const jobs=result.jobs.slice(0,20);
        const keep=new Set(jobs.map(job=>job.id));
        function release(entry) {for(const audio of entry.node.querySelectorAll("audio")){audio.pause();audio.removeAttribute("src");audio.load();audios.delete(audio);}}
        for(const [id,entry] of cards) if(!keep.has(id)){release(entry);cards.delete(id);}
        list.replaceChildren(...jobs.map(job=>{
          const key=JSON.stringify([job,active===job.id]);
          let entry=cards.get(job.id);
          if(!entry || entry.key!==key){if(entry)release(entry);entry={key,node:card(job)};cards.set(job.id,entry);}
          return entry.node;
        }));
        for(const button of list.querySelectorAll("button")) if(["読み上げエンジンで品質を検査","この学習を再開"].includes(button.textContent)) button.disabled=!!active || pending;
        lock(pending || !!active || !supported);
        setup.textContent=result.runtimeInstalled?"学習環境を確認する":"学習環境を準備する";
        if (!result.supported) status.textContent="この機能は現在Windows専用です。";
        if(active){clearTimeout(timer);timer=setTimeout(reload,1200);}
      } catch(error){fail(error);}
      finally {loading=false;}
    }
    lock(true);reload();
    return sec;
  }
  window.PicoCharacterLearning={build};
})();
