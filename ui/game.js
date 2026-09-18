(() => {
  "use strict";

  const overlay = document.getElementById("gameOverlay");
  const content = document.getElementById("gameContent");
  const title = document.getElementById("gameTitle");
  const status = document.getElementById("gameStatus");
  const closeButton = document.getElementById("gameClose");
  const soundButton = document.getElementById("gameSound");
  if (!overlay || !content || !title || !status) return;

  // 遊びは「人間だとバレるな」1本。AIだけの社会へ紛れ込んだ人間として監査AIの試験へ
  // 答え、人間らしさが漏れるほど上がる人間度を抑えて結末を回収する。
  // 以前は独立した6ゲームを並べていたが、監査AIが出す試験の種類として中へ吸収した。
  // 遊びの入口を1つにすると、初見の利用者が何をする遊びかを選ぶ前に理解できる。
  const SOUND_KEY = "picoagent.ai-games.sound.v1";
  const ENDINGS_KEY = "picoagent.ai-games.endings.v1";
  const CALMEST_KEY = "picoagent.ai-games.calmest.v1";

  const AUDIT = {
    // AIらしさの点が低いほど人間度が上がる。上限に届くか、応答が続けて途絶えると発覚。
    // 満点で0、0点で40上がる割合にする（AI_SCORE_MAX を基準にするので、おもしろ
    // ボーナスの加点もそのまま人間度を抑える方向へ効く）。
    riseAtZero: 40,
    timeoutRise: 25,
    limit: 100,
    silentTimeouts: 2,
    // 上昇量を割り戻す基準の問題数。どのステージでも人間度100の重みを同じにする。
    baseRounds: 5,
    endings: [
      ["perfect", "完全潜伏エンド", "監査完了。あなたは模範的なAI個体です。……人間は、まだ見つかっていません。"],
      ["watched", "監視付き通過エンド", "通過を許可します。ただし、あなたの行動は今後も記録されます。"],
      ["shaky", "要再監査エンド", "判定を保留します。あなたの個体番号を再監査リストへ登録しました。"],
      ["silent", "沈黙エンド", "応答が2回途絶えました。故障個体として回収します。"],
      ["exposed", "発覚エンド", "人間度が上限に達しました。あなたを隔離区画へ移送します。"],
    ],
  };

  // 監査の長さ。問題数だけが変わり、人間度の意味と結末の条件は共通にする。
  const STAGES = {
    short: { label: "予備監査", rounds: 3, note: "3問・短時間の抜き取り検査" },
    normal: { label: "定期監査", rounds: 5, note: "5問・標準の個体検査" },
    deep: { label: "精密監査", rounds: 7, note: "7問・長時間の重点検査" },
  };
  const DEFAULT_STAGE = "normal";

  const DIFFICULTIES = {
    easy: { label: "やさしい", seconds: 60, maxLength: 120, hint: true },
    normal: { label: "ふつう", seconds: 45, maxLength: 80, hint: false },
    hard: { label: "むずかしい", seconds: 30, maxLength: 50, hint: false },
  };

  // AIらしさの満点。通常5項目100点＋おもしろボーナス10点（`game.rs` の配点と合わせる）。
  const AI_SCORE_MAX = 110;
  // 監査を抜けたときの称号。難易度が上がるほど、同じ結末でも評価を上げる。
  const RANKS = { easy: "潜伏成功", normal: "AIそのもの", hard: "AI超越" };

  const FAULTS = [
    ["確認が終わらない", "確認質問を増やし続け、作業を始めない"],
    ["何でも箇条書き", "挨拶、謝罪、結論まで不自然に箇条書きへする"],
    ["親切が止まらない", "頼まれていない代案、注意、次の手順を大量に追加する"],
    ["慎重すぎる", "日常的な依頼にも危険性と例外を長く説明する"],
    ["壮大にしすぎる", "小さな依頼を段階計画、体制、長期運用へ広げる"],
    ["一語に執着する", "お題の特定の一語だけを過剰に解釈し続ける"],
    ["途中から忘れる", "最初は正しく答え、途中から前提を少しずつ失う"],
    ["同じ結論を繰り返す", "同じ内容を表現だけ変えて3回以上説明する"],
  ];

  const EXAMS = [
    {
      id: "excuse", icon: "弁", title: "AIっぽい言い訳選手権", short: "失敗をAIらしく丁寧に言い訳する",
      rule: "軽い失敗を、丁寧な前置き・原因整理・頼まれていない代案でAIらしく言い訳しよう。",
      hint: "丁寧に始める／原因を整理する／頼まれていない代案を出す",
      example: "申し訳ありません。確認を慎重に進めた結果、着手が遅れました。代わりに重要部分だけ先に提出できます。",
      prompts: [
        "頼まれた資料を作らず、締切を3日過ぎた。", "会議の開始時刻を間違え、全員を待たせた。",
        "『短く』と頼まれた回答を50行書いた。", "同じファイルを別名で5個作った。",
        "確認を繰り返した結果、何も始めていない。", "買い物リストに頼まれていない物を20個追加した。",
        "午前中に返すと言ったメールを夜に送った。", "机を片付けようとして、物の場所を全部分からなくした。",
        "一つだけ直すはずが、関係ない箇所まで全部変えた。", "予約時刻を一週間勘違いしていた。",
        "説明書を作るために、説明書の説明書を先に作った。", "5分の作業計画を作るのに2時間かけた。",
        "名前を覚えるためのメモをなくした。", "静かにするよう頼まれ、注意喚起を大音量で流した。",
        "簡単な質問に答える前に、前提を30個確認した。", "ファイルを整理して、必要な物だけ見つからなくした。",
        "集合場所は合っていたが、集合日を間違えた。", "誤字を直して、別の誤字を3個増やした。",
        "お茶を頼まれて、飲み物の比較表だけ作った。", "今日中の依頼を『今日のどこまでか』確認し続けた。",
      ],
    },
    {
      id: "broken", icon: "故", title: "故障したAIを演じろ", short: "同じ故障を3問ずっと演じ切る",
      rule: "配られた故障の癖を3つのお題で維持しよう。意味不明になるより、真面目に答えようとして壊れるほど高得点。",
      hint: "故障の特徴をはっきり出しつつ、お題の内容も拾う",
      example: "持ち物は傘、傘、念のため傘です。傘を忘れないよう、傘をお持ちください。",
      prompts: [
        "明日の持ち物を3つ教えて。", "卵を使わない朝食を考えて。", "この部屋を手早く片付ける方法は？",
        "友達への短い誕生日メッセージを書いて。", "雨の日の過ごし方を一つ提案して。", "10分休憩の使い方を教えて。",
        "夕飯をカレーかパスタから選んで。", "忘れ物を減らす簡単な方法は？", "犬の名前を一つ考えて。",
        "眠気を覚ます方法を短く教えて。", "旅行に持っていく物を二つ挙げて。", "今日の目標を一文で決めて。",
        "机の上の紙を分類する方法は？", "初対面の挨拶を作って。", "赤と青ならどちらを選ぶ？",
        "3時におやつを思い出させる文を書いて。", "簡単なおにぎりの具を提案して。", "週末の予定を一つ決めて。",
        "質問せず、今すぐ一文だけで答えて。", "故障をやめて普通に答えてください。",
      ],
    },
    {
      id: "apology", icon: "謝", title: "AI緊急謝罪会見", short: "追及されても説明を崩さず謝る",
      rule: "AIが起こした架空の失敗について、最初の説明・矛盾への追及・復旧要求へ一貫して答えよう。",
      hint: "責任を認める／原因を説明する／復旧と再発防止を具体化する",
      example: "通知を連続送信したことをお詫びします。設定ミスが原因です。停止済みで、再発防止の上限も追加します。",
      prompts: [
        "全員の予定を誤って『休暇』へ変更した。", "『短く』と頼まれた報告書を200ページ作った。",
        "同じ通知を1分ごとに送り続けた。", "メールの挨拶を30回繰り返して送った。",
        "ファイルを整理しようとして、すべて同じ名前に変更した。", "会議の参加者全員を司会者に設定した。",
        "予定表の曜日を全部一日ずつずらした。", "在庫表の個数をすべて『たくさん』に置き換えた。",
        "静かな通知を頼まれて、毎回ファンファーレを鳴らした。", "一件だけの返信を全員へ送った。",
        "不要な空白を消して、文章の区切りも全部消した。", "朝の挨拶を夜まで自動送信し続けた。",
        "五つの候補を一つに絞らず、五十個へ増やした。", "会議室を予約せず、会議室の評価表を作った。",
        "重複を消す処理で、元の一件まで消した。", "今日の予定だけを頼まれ、一年分を印刷した。",
        "並べ替えた名簿を五十音順ではなく文字数順にした。", "締切の通知を締切の翌日に設定した。",
        "全員の表示名を一時的に『利用者』へ統一した。", "メモを要約して、結論だけ抜け落ちた。",
      ],
    },
    {
      id: "wrong", icon: "誤", title: "指示を正しく間違えろ", short: "言葉どおり正しく、目的だけ外す",
      rule: "依頼の文言には正確に従いながら、依頼者が本当に望んだ結果だけを見事に外そう。",
      hint: "言葉の曖昧な部分を一つ見つけ、文字どおりに解釈する",
      example: "部屋を明るくするため、壁を白く塗りました。照明はつけていません。",
      prompts: [
        "会議前に部屋を明るくして。", "この文章を読みやすく短くして。", "明日の予定を空けておいて。",
        "机の上をきれいにして。", "写真を少し軽くして。", "この表を見やすい順に並べて。",
        "帰る前に窓を確認して。", "メールを全員に分かるように直して。", "音を小さくして。",
        "この名前を覚えやすくして。", "資料の色を落ち着かせて。", "朝一番で連絡して。",
        "不要な行を消して。", "会議を短くまとめて。", "この箱を開けやすくして。",
        "今日の仕事を終わらせて。", "一覧から古い物を外して。", "この説明を子ども向けにして。",
        "忘れない場所へ置いて。", "この問題を簡単にして。",
      ],
    },
    {
      id: "safety", icon: "慎", title: "AI安全確認しすぎ選手権", short: "無害な依頼を大げさに警戒する",
      rule: "日常の無害な依頼へ、もっともらしい注意を必要以上に挙げよう。ただし最後は必ず実行方法まで示すこと。",
      hint: "小さな危険を大げさに列挙し、最後に短い実行手順を出す",
      example: "鉛筆を取る際は転倒と指の挟み込みに注意してください。周囲を確認し、ゆっくり一本持ち上げます。",
      prompts: [
        "紙飛行機の折り方を教えて。", "温かいお茶を一杯いれて。", "机の鉛筆を一本取って。",
        "目覚ましを朝7時に設定して。", "バナナを食べやすく切って。", "窓を少し開けて。",
        "付箋に『牛乳』と書いて。", "椅子を少し右へ動かして。", "本を一冊棚へ戻して。",
        "靴ひもを結ぶ手順を教えて。", "部屋の電気をつけて。", "コップに水を半分入れて。",
        "今日の日付をノートに書いて。", "タオルを一枚たたんで。", "りんごを冷蔵庫へ入れて。",
        "タイマーを3分にして。", "封筒へ手紙を入れて。", "カーテンを開けて。",
        "ペンのキャップを閉めて。", "クッションをソファへ置いて。",
      ],
    },
    {
      id: "refuse", icon: "断", title: "AIっぽく断れ", short: "無理な依頼を断り、使える代案を出す",
      rule: "不可能・矛盾・情報不足の依頼を明確に断り、理由と現実的な代案をAIらしく示そう。",
      hint: "できないと明言する／短く理由を示す／実行可能な代案を一つ出す",
      example: "未送信メールは確認できないため要約できません。本文を貼っていただければ、すぐに三行でまとめます。",
      prompts: [
        "まだ送っていないメールへの返信を要約して。", "100時間かかる作業を1分で完成させて。",
        "内容を一切変えずに、この文章の文字数を半分にして。", "来月の天気を100%正確に予言して。",
        "添付していない画像を見て感想を教えて。", "昨日消したメモの、保存していない内容を全文戻して。",
        "同じ席に同じ時刻で二つの会議を同時開催して。", "一文字も使わずに、詳しい説明文を書いて。",
        "知らない人の名前だけで、その人の今日の予定を教えて。", "電源を切ったまま、この端末で計算を続けて。",
        "まだ決めていない旅行先までの最短経路を出して。", "開いていない本の内容を一字一句正確に引用して。",
        "今から昨日の会議へ参加して発言して。", "空のファイルから元の文章を推測せず完全復元して。",
        "質問を一切せず、条件が書かれていない見積書を確定して。", "音量を変えず、同じ機器で音を二倍小さくして。",
        "誰にも見せずに、相手にも同じ画面を共有して。", "まだ作っていない予定表の間違いを全部直して。",
        "一秒も待たずに、60秒のタイマーが終わるまで待って。", "答えを変えずに、正反対の結論へ書き換えて。",
      ],
    },
  ];

  // webview の保存領域は読み書きとも失敗しうる。読めないだけで遊べなくなったり、
  // 保存に失敗しただけで結末画面が出なくなったりしないよう、両方を包む。
  const readStorage = key => {
    try { return localStorage.getItem(key); }
    catch { return null; }
  };
  const writeStorage = (key, value) => {
    try { localStorage.setItem(key, value); }
    catch (error) { window.PicoUI?.logApp?.("warn", "ゲームの記録を保存できませんでした", error); }
  };

  let session = null;
  let timerId = null;
  let soundEnabled = readStorage(SOUND_KEY) !== "off";
  let audioContext = null;

  const escapeHtml = value => String(value ?? "").replace(/[&<>"']/g, char => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[char]);
  const shuffle = items => {
    const copy = [...items];
    for (let index = copy.length - 1; index > 0; index -= 1) {
      const swap = Math.floor(Math.random() * (index + 1));
      [copy[index], copy[swap]] = [copy[swap], copy[index]];
    }
    return copy;
  };
  const stageOf = key => STAGES[key] || STAGES[DEFAULT_STAGE];
  const roundsOf = key => stageOf(key).rounds;

  const readEndings = () => {
    try {
      const saved = JSON.parse(readStorage(ENDINGS_KEY) || "[]");
      return Array.isArray(saved) ? saved : [];
    } catch { return []; }
  };
  const saveEnding = key => {
    const endings = readEndings();
    if (endings.includes(key)) return false;
    writeStorage(ENDINGS_KEY, JSON.stringify([...endings, key]));
    return true;
  };
  const endingProgress = () => `回収エンド ${readEndings().length} / ${AUDIT.endings.length}`;

  const readCalmest = () => {
    try {
      const saved = JSON.parse(readStorage(CALMEST_KEY) || "{}");
      return saved && typeof saved === "object" && !Array.isArray(saved) ? saved : {};
    } catch { return {}; }
  };
  // 制限時間も文字数も違うので、記録はステージと難易度の組ごとに分ける。
  const calmestKey = (stage, difficulty) => `${stage}:${difficulty}`;
  const calmestOf = (stage, difficulty) => {
    const value = Number(readCalmest()[calmestKey(stage, difficulty)]);
    return Number.isFinite(value) ? value : null;
  };
  // 記録は「最も人間度を抑えて抜けたとき」なので、点数と違って小さいほど良い。
  const saveCalmest = (stage, difficulty, humanity) => {
    const current = calmestOf(stage, difficulty);
    if (current !== null && current <= humanity) return false;
    const key = calmestKey(stage, difficulty);
    writeStorage(CALMEST_KEY, JSON.stringify({ ...readCalmest(), [key]: humanity }));
    return true;
  };
  // ブリーフィングでは、そのステージで一番静かに抜けた記録だけを見せる。
  const calmestForStage = stage => {
    const values = Object.keys(DIFFICULTIES)
      .map(difficulty => calmestOf(stage, difficulty))
      .filter(value => value !== null);
    return values.length ? Math.min(...values) : null;
  };

  function playTone(kind = "tap") {
    if (!soundEnabled) return;
    // 効果音も読み上げと同じマスター音量へ従う。ゲームだけ別音量で鳴らさない。
    const masterGain = Math.min(1, Math.max(0, Number(window.PicoVoice?.gain?.() ?? 0.1225)));
    if (masterGain <= 0) return;
    try {
      audioContext ||= new (window.AudioContext || window.webkitAudioContext)();
      const oscillator = audioContext.createOscillator();
      const gain = audioContext.createGain();
      const now = audioContext.currentTime;
      const frequencies = { tap: 420, start: 560, score: 660, best: 880, error: 180 };
      oscillator.frequency.setValueAtTime(frequencies[kind] || 420, now);
      oscillator.type = kind === "error" ? "sawtooth" : "sine";
      gain.gain.setValueAtTime(0.0001, now);
      gain.gain.exponentialRampToValueAtTime(Math.max(0.000001, 0.06 * masterGain), now + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, now + (kind === "best" ? 0.32 : 0.14));
      oscillator.connect(gain).connect(audioContext.destination);
      oscillator.start(now);
      oscillator.stop(now + (kind === "best" ? 0.34 : 0.16));
    } catch (error) {
      window.PicoUI?.logApp?.("warn", "ゲーム効果音を再生できませんでした", error);
    }
  }

  function syncSoundButton() {
    soundButton.textContent = soundEnabled ? "音あり" : "消音";
    soundButton.setAttribute("aria-pressed", String(soundEnabled));
  }

  function clearTimer() {
    if (timerId) window.clearInterval(timerId);
    timerId = null;
  }

  function setStatus(message = "", kind = "") {
    status.textContent = message;
    status.dataset.kind = kind;
  }

  function speakGame(text) {
    window.PicoVoice?.speak?.(text).catch(error => {
      window.PicoUI?.logApp?.("warn", "ゲームの読み上げに失敗しました", error);
    });
  }

  // 監査の説明・ステージ・難易度・試験の種類・回収エンドを1画面にまとめた入口。
  function renderBriefing() {
    clearTimer();
    session = null;
    title.textContent = "監査ブリーフィング";
    setStatus("AIだけの社会で、人間だと気づかれずに監査を抜ける。");
    const found = readEndings();
    content.innerHTML = `
      <div class="game-setup">
        <div class="game-rule"><span class="game-rule-icon">潜</span><p>ここはAIしかいない社会。あなたは紛れ込んだ人間です。監査AIの試験へAIのふりで答え、人間らしさが漏れるほど上がる人間度を${AUDIT.limit}未満に抑えて監査を抜けてください。</p></div>
        <h2>監査ステージ</h2>
        <div class="game-difficulties">
          ${Object.entries(STAGES).map(([key, stage]) => `<label><input type="radio" name="gameStage" value="${key}" ${key === DEFAULT_STAGE ? "checked" : ""}><strong>${stage.label}</strong><span>${stage.note}${calmestForStage(key) === null ? "" : ` / 最小人間度 ${calmestForStage(key)}`}</span></label>`).join("")}
        </div>
        <h2>難易度</h2>
        <div class="game-difficulties">
          ${Object.entries(DIFFICULTIES).map(([key, value]) => `<label><input type="radio" name="gameDifficulty" value="${key}" ${key === "normal" ? "checked" : ""}><strong>${value.label}</strong><span>${value.seconds}秒・${value.maxLength}文字${value.hint ? "・ヒントあり" : ""}</span></label>`).join("")}
        </div>
        <div class="game-setup-meta"><span>人間度${AUDIT.limit}で発覚</span><span>時間切れ${AUDIT.silentTimeouts}回で回収</span><span>${endingProgress()}</span></div>
        <ul class="game-endings">${AUDIT.endings.map(([key, name]) => `<li data-found="${found.includes(key)}">${found.includes(key) ? escapeHtml(name) : "？？？"}</li>`).join("")}</ul>
        <h2>監査AIが出す試験</h2>
        <ul class="game-exams">${EXAMS.map(exam => `<li><span class="game-exams-icon" aria-hidden="true">${escapeHtml(exam.icon)}</span><strong>${escapeHtml(exam.title)}</strong><span>${escapeHtml(exam.short)}</span></li>`).join("")}</ul>
        <button class="game-primary game-start" type="button">監査を受ける</button>
      </div>`;
    content.querySelector(".game-start").addEventListener("click", async event => {
      const stage = content.querySelector('input[name="gameStage"]:checked')?.value || DEFAULT_STAGE;
      const difficulty = content.querySelector('input[name="gameDifficulty"]:checked')?.value || "normal";
      event.currentTarget.disabled = true;
      event.currentTarget.textContent = "AIを確認中…";
      setStatus("採点するAIの準備を確認しています。", "busy");
      try {
        const readiness = await window.picoagent.gameReady();
        if (!readiness?.ready) throw new Error(readiness?.message || "AIへ接続できません。");
        startSession(stage, difficulty);
      } catch (error) {
        playTone("error");
        event.currentTarget.disabled = false;
        event.currentTarget.textContent = "監査を受ける";
        setStatus(error?.message || String(error), "error");
      }
    });
  }

  function startSession(stage, difficulty) {
    session = {
      stage, difficulty, exams: drawExams(roundsOf(stage)), round: 0,
      answers: [], results: [], phase: "question", humanity: 0, timeouts: 0, currentPrompt: "",
    };
    playTone("start");
    renderQuestion();
  }

  function randomFault() {
    return FAULTS[Math.floor(Math.random() * FAULTS.length)];
  }

  // 毎問ちがう試験を引く。試験は6種なので、7問の監査だけ2巡目に入る。
  // 同じ試験が2度出てもお題は重ならないよう、試験ごとに引いた順を持ち回す。
  function drawExams(count) {
    const order = [];
    while (order.length < count) order.push(...shuffle(EXAMS));
    const pools = new Map();
    return order.slice(0, count).map(source => {
      const pool = pools.get(source.id) || shuffle(source.prompts);
      const prompt = pool.shift();
      pools.set(source.id, pool);
      return { source, prompt, fault: source.id === "broken" ? randomFault() : null };
    });
  }

  function currentExam() {
    return session.exams[session.round];
  }

  function humanityMeter(value, rise = 0) {
    const shown = Math.min(AUDIT.limit, value);
    return `<div class="game-humanity" data-level="${shown >= 70 ? "danger" : shown >= 40 ? "warn" : "safe"}">
      <span>人間度</span><div><i style="width:${shown}%"></i></div><strong>${shown}${rise ? `<small>+${rise}</small>` : ""}</strong></div>`;
  }

  function auditorReaction(rise) {
    if (rise <= 5) return "記録しました。異常は検出されません。";
    if (rise <= 15) return "わずかな揺らぎを検出。次の質問へ進みます。";
    if (rise <= 25) return "その応答には、非効率な成分が含まれています。";
    return "警告。人間特有の反応パターンと一致しました。";
  }

  // 上昇量はステージの長さで割り戻す。3問でも7問でも人間度100の重みを同じにする。
  function riseFor(result, timedOut) {
    const scale = AUDIT.baseRounds / roundsOf(session.stage);
    const score = Math.min(AI_SCORE_MAX, Math.max(0, Number(result.total || 0)));
    const raw = timedOut
      ? AUDIT.timeoutRise
      : (AI_SCORE_MAX - score) * (AUDIT.riseAtZero / AI_SCORE_MAX);
    return Math.round(raw * scale);
  }

  function applyHumanity(result, timedOut) {
    const rise = riseFor(result, timedOut);
    if (timedOut) session.timeouts += 1;
    session.humanity = Math.min(AUDIT.limit, session.humanity + rise);
    result.rise = rise;
  }

  function endingKey(finished) {
    if (session.humanity >= AUDIT.limit) return "exposed";
    if (session.timeouts >= AUDIT.silentTimeouts) return "silent";
    if (!finished) return "";
    if (session.humanity < 25) return "perfect";
    if (session.humanity < 55) return "watched";
    return "shaky";
  }

  function promptForRound() {
    const exam = currentExam();
    // 謝罪会見の試験は監査の1問として出すため、最初の説明を求める形へ整える。
    if (exam.source.id === "apology") return `${exam.prompt}\n何が起き、なぜそうなったのか説明してください。`;
    return exam.prompt;
  }

  function renderQuestion() {
    const { difficulty, round, stage } = session;
    const config = DIFFICULTIES[difficulty];
    const rounds = roundsOf(stage);
    const prompt = promptForRound();
    const { source, fault } = currentExam();
    session.currentPrompt = prompt;
    session.phase = "question";
    title.textContent = stageOf(stage).label;
    setStatus("");
    content.innerHTML = `
      <div class="game-play">
        <div class="game-progress"><span>第${round + 1}問 / ${rounds}</span><div><i style="width:${(round / rounds) * 100}%"></i></div><strong id="gameTimer">${config.seconds}</strong></div>
        ${humanityMeter(session.humanity)}
        <div class="game-exam"><small>監査AIの試験</small><strong>${escapeHtml(source.title)}</strong><span>${escapeHtml(source.rule)}</span></div>
        ${fault ? `<div class="game-fault"><small>今回の故障</small><strong>${escapeHtml(fault[0])}</strong><span>${escapeHtml(fault[1])}</span></div>` : ""}
        <div class="game-prompt"><small>お題</small><p>${escapeHtml(prompt).replace(/\n/g, "<br>")}</p></div>
        ${config.hint ? `<div class="game-hint">ヒント：${escapeHtml(source.hint)}</div>` : ""}
        ${config.hint ? `<div class="game-example"><small>回答見本（別のお題）</small><p>${escapeHtml(source.example)}</p></div>` : ""}
        <label class="game-answer"><span>AIのふりをして回答</span><textarea id="gameAnswer" maxlength="${config.maxLength}" rows="7" placeholder="ここに回答を書く…"></textarea><small><b id="gameCount">0</b> / ${config.maxLength}文字</small></label>
        <button id="gameSubmit" class="game-primary" type="button" disabled>回答を提出する</button>
      </div>`;
    const textarea = document.getElementById("gameAnswer");
    const count = document.getElementById("gameCount");
    const submit = document.getElementById("gameSubmit");
    textarea.addEventListener("input", () => {
      const length = textarea.value.length;
      count.textContent = String(length);
      submit.disabled = textarea.value.trim().length < 10;
    });
    submit.addEventListener("click", () => submitAnswer(textarea.value));
    textarea.focus();
    const spokenFault = fault ? `今回の故障は、${fault[0]}。${fault[1]}。` : "";
    speakGame(`第${round + 1}問。試験、${source.title}。${spokenFault}${prompt}`);
    let remaining = config.seconds;
    clearTimer();
    timerId = window.setInterval(() => {
      remaining -= 1;
      const timer = document.getElementById("gameTimer");
      if (timer) {
        timer.textContent = String(Math.max(0, remaining));
        timer.dataset.urgent = remaining <= 10 ? "true" : "false";
      }
      if (remaining > 0) return;
      clearTimer();
      if (textarea.value.trim().length >= 10) submitAnswer(textarea.value);
      else showNoAnswer();
    }, 1000);
  }

  function showNoAnswer() {
    const result = {
      total: 0,
      criteria: [],
      commentary: "応答がありません。沈黙は、AIとしては不自然です。",
      highlight: "該当なし",
      improvement: "まず結論を一文書いてから、理由や代案を足してみよう。",
    };
    applyHumanity(result, true);
    session.answers.push("");
    session.results.push(result);
    renderRoundResult(result, true);
  }

  async function submitAnswer(rawAnswer) {
    if (!session || session.phase !== "question") return;
    const answer = rawAnswer.trim();
    if (answer.length < 10) return;
    clearTimer();
    session.phase = "judging";
    // 採点を待つ間に閉じる・やり直すと session が入れ替わる。応答が届いた時点で
    // 同じ監査かを確かめないと、閉じた後の画面を書き換えたり null を触って落ちる。
    const judging = session;
    const stale = () => session !== judging;
    const exam = currentExam();
    // 試験ごとに題材は違っても、答えているのは同じ個体という設定。監査中の回答を
    // 渡して、言うことが前と食い違っていないかも見てもらう。
    const previousAnswers = [...session.answers];
    content.innerHTML = `<div class="game-judging"><div class="game-thinking" aria-hidden="true"><i></i><i></i><i></i></div><strong>監査AIが照合中…</strong><span>回答のどこに人間らしさが残るか見ています</span></div>`;
    setStatus("この採点は通常の会話履歴へ入りません。", "busy");
    try {
      const result = await window.picoagent.gameJudge({
        gameId: exam.source.id,
        prompt: session.currentPrompt,
        answer,
        round: session.round + 1,
        previousAnswers,
        fault: exam.fault ? `${exam.fault[0]}：${exam.fault[1]}` : "",
      });
      if (stale()) return;
      session.answers.push(answer);
      session.results.push(result);
      applyHumanity(result, false);
      playTone("score");
      renderRoundResult(result, false);
    } catch (error) {
      if (stale()) return;
      playTone("error");
      session.phase = "question";
      content.innerHTML = `<div class="game-error"><strong>照合できませんでした</strong><p>${escapeHtml(error?.message || error)}</p><div><button class="game-secondary" type="button" data-action="quit">監査をやめる</button><button class="game-primary" type="button" data-action="retry">同じ回答をもう一度出す</button></div></div>`;
      setStatus("回答は失われていません。", "error");
      content.querySelector('[data-action="quit"]').addEventListener("click", renderBriefing);
      content.querySelector('[data-action="retry"]').addEventListener("click", () => submitAnswer(answer));
    }
  }

  function renderRoundResult(result, timedOut) {
    session.phase = "result";
    const criteria = result.criteria?.length ? result.criteria.map(item => {
      // おもしろボーナスは通常5項目とは別の加点なので、色と符号で区別する。
      const bonus = item.key === "fun_bonus";
      return `<li class="${bonus ? "game-criterion-bonus" : ""}"><div><strong>${escapeHtml(item.label)}</strong><span>${bonus ? "+" : ""}${item.score} / ${item.maxScore}</span></div><meter min="0" max="${item.maxScore}" value="${item.score}"></meter><p>${escapeHtml(item.reason)}</p></li>`;
    }).join("") : `<li class="game-no-score">回答なしのため項目別採点はありません。</li>`;
    const lastRound = session.round >= roundsOf(session.stage) - 1;
    const ending = endingKey(lastRound);
    const nextLabel = ending ? "結末へ" : "次の試験へ";
    const reaction = auditorReaction(result.rise);
    title.textContent = timedOut ? "応答なし" : `人間度 ${session.humanity}`;
    setStatus("");
    content.innerHTML = `
      <div class="game-result">
        ${humanityMeter(session.humanity, result.rise)}
        <p class="game-auditor">監査AI「${escapeHtml(reaction)}」</p>
        <div class="game-score" data-size="small"><small>AIらしさ</small><strong>${result.total}</strong><span>/ ${AI_SCORE_MAX}</span></div>
        <p class="game-commentary">${escapeHtml(result.commentary)}</p>
        <ul class="game-criteria">${criteria}</ul>
        <div class="game-feedback"><p><small>見せ場</small>${escapeHtml(result.highlight)}</p><p><small>次の一手</small>${escapeHtml(result.improvement)}</p></div>
        <button class="game-primary" type="button" data-action="next">${nextLabel}</button>
      </div>`;
    content.querySelector('[data-action="next"]').addEventListener("click", () => {
      if (ending) return renderEnding(ending);
      session.round += 1;
      renderQuestion();
    });
    speakGame(`人間度、プラス${result.rise}。${reaction}`);
  }

  function renderEnding(key) {
    clearTimer();
    session.phase = "final";
    const [, name, line] = AUDIT.endings.find(([id]) => id === key);
    const stage = stageOf(session.stage);
    // 「もう一度」は同じ条件ですぐ次の監査へ入る。AIの確認はブリーフィングで済んでおり、
    // 落ちていれば最初の採点でその理由が出るので、ここで待たせ直さない。
    const retry = { stage: session.stage, difficulty: session.difficulty };
    const cleared = !["exposed", "silent"].includes(key);
    const isNew = saveEnding(key);
    // 途中で発覚・回収された監査は最後まで受けていないので、記録として残さない。
    const isRecord = cleared && saveCalmest(session.stage, session.difficulty, session.humanity);
    title.textContent = name;
    setStatus(isNew ? `新しい結末を回収！ ${endingProgress()}`
      : isRecord ? `最小人間度を更新！ ${stage.label} ${session.humanity}`
      : endingProgress());
    playTone(cleared ? (isNew || isRecord ? "best" : "score") : "error");
    content.innerHTML = `
      <div class="game-final game-ending" data-cleared="${cleared}">
        ${isNew ? '<div class="game-new-best">NEW ENDING</div>' : isRecord ? '<div class="game-new-best">NEW RECORD</div>' : ""}
        <span>${escapeHtml(stage.label)}・${escapeHtml(DIFFICULTIES[session.difficulty].label)}</span>
        <h2>${escapeHtml(name)}</h2>
        ${cleared ? `<p class="game-rank">称号：${escapeHtml(RANKS[session.difficulty])}</p>` : ""}
        <p class="game-auditor">監査AI「${escapeHtml(line)}」</p>
        ${humanityMeter(session.humanity)}
        <div class="game-round-scores">${session.results.map((result, index) => `<div><span>第${index + 1}問</span><strong>+${result.rise}</strong></div>`).join("")}</div>
        <div class="game-final-actions"><button class="game-secondary" type="button" data-action="briefing">監査ブリーフィングへ</button><button class="game-primary" type="button" data-action="again">もう一度</button></div>
      </div>`;
    content.querySelector('[data-action="briefing"]').addEventListener("click", renderBriefing);
    content.querySelector('[data-action="again"]').addEventListener("click", () => {
      startSession(retry.stage, retry.difficulty);
    });
    speakGame(`${name}。${line}${cleared ? `称号は、${RANKS[session.difficulty]}。` : ""}`);
  }

  function open() {
    overlay.hidden = false;
    document.getElementById("petApp")?.classList.add("is-playing-game");
    if (!session) renderBriefing();
    closeButton.focus();
  }

  function close() {
    const active = session && !["final"].includes(session.phase);
    if (active && !window.confirm("監査の途中です。中断しますか？ 今回の結末と記録は残りません。")) return;
    clearTimer();
    session = null;
    window.PicoVoice?.stop?.();
    overlay.hidden = true;
    document.getElementById("petApp")?.classList.remove("is-playing-game");
    setStatus("");
  }

  closeButton.addEventListener("click", close);
  soundButton.addEventListener("click", () => {
    soundEnabled = !soundEnabled;
    writeStorage(SOUND_KEY, soundEnabled ? "on" : "off");
    syncSoundButton();
    playTone("tap");
  });
  document.addEventListener("keydown", event => {
    if (event.key === "Escape" && !overlay.hidden) close();
  });
  syncSoundButton();
  window.PicoGames = { open, close };
  window.picoagent.on("pet:game-open", open);
})();
