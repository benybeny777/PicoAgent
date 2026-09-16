(() => {
  "use strict";

  const overlay = document.getElementById("gameOverlay");
  const content = document.getElementById("gameContent");
  const title = document.getElementById("gameTitle");
  const status = document.getElementById("gameStatus");
  const closeButton = document.getElementById("gameClose");
  const soundButton = document.getElementById("gameSound");
  if (!overlay || !content || !title || !status) return;

  const BEST_KEY = "picoagent.ai-games.best.v1";
  const SOUND_KEY = "picoagent.ai-games.sound.v1";
  const DIFFICULTIES = {
    easy: { label: "やさしい", seconds: 60, maxLength: 120, maxScore: 300, hint: true },
    normal: { label: "ふつう", seconds: 45, maxLength: 80, maxScore: 600, hint: false },
    hard: { label: "むずかしい", seconds: 30, maxLength: 50, maxScore: 1000, hint: false },
  };

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

  const GAMES = [
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

  let session = null;
  let timerId = null;
  let soundEnabled = localStorage.getItem(SOUND_KEY) !== "off";
  let audioContext = null;

  const escapeHtml = value => String(value ?? "").replace(/[&<>"']/g, char => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[char]);
  const gameById = id => GAMES.find(game => game.id === id);
  const shuffle = items => {
    const copy = [...items];
    for (let index = copy.length - 1; index > 0; index -= 1) {
      const swap = Math.floor(Math.random() * (index + 1));
      [copy[index], copy[swap]] = [copy[swap], copy[index]];
    }
    return copy;
  };
  const readBests = () => {
    try { return JSON.parse(localStorage.getItem(BEST_KEY) || "{}"); }
    catch { return {}; }
  };
  const bestKey = (id, difficulty) => `${id}:${difficulty}`;
  const bestFor = (id, difficulty) => Number(readBests()[bestKey(id, difficulty)] || 0);
  const bestSummary = id => {
    const bests = readBests();
    const scores = Object.keys(DIFFICULTIES).map(difficulty => Number(bests[bestKey(id, difficulty)] || 0));
    const legacy = Number(bests[id] || 0);
    return Math.max(legacy, ...scores);
  };
  const saveBest = (id, difficulty, score) => {
    const bests = readBests();
    const key = bestKey(id, difficulty);
    if (score <= Number(bests[key] || 0)) return false;
    bests[key] = score;
    localStorage.setItem(BEST_KEY, JSON.stringify(bests));
    return true;
  };
  const difficultyScore = (rawScore, difficulty) => Math.round(rawScore * DIFFICULTIES[difficulty].maxScore / 300);
  const difficultyMax = difficulty => DIFFICULTIES[difficulty].maxScore;
  const scoreLabel = score => Number(score) > 0 ? `${score}点` : "—";

  function playTone(kind = "tap") {
    if (!soundEnabled) return;
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
      gain.gain.setValueAtTime(0.000001, now);
      gain.gain.exponentialRampToValueAtTime(Math.max(0.000001, 0.06 * masterGain), now + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.000001, now + (kind === "best" ? 0.32 : 0.14));
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

  function renderPicker() {
    clearTimer();
    session = null;
    title.textContent = "遊ぶゲームを選ぶ";
    setStatus("ルールは簡単。AIのまねをして、AIに採点してもらおう。");
    content.innerHTML = `<div class="game-grid">${GAMES.map(game => `
      <button class="game-card" type="button" data-game-id="${game.id}">
        <span class="game-card-icon" aria-hidden="true">${game.icon}</span>
        <strong>${escapeHtml(game.title)}</strong>
        <span>${escapeHtml(game.short)}</span>
        <small>最高記録 ${scoreLabel(bestSummary(game.id))}</small>
      </button>`).join("")}</div>`;
    content.querySelectorAll("[data-game-id]").forEach(button => {
      button.addEventListener("click", () => {
        playTone("tap");
        renderSetup(button.dataset.gameId);
      });
    });
  }

  function renderSetup(id) {
    const game = gameById(id);
    if (!game) return renderPicker();
    title.textContent = game.title;
    setStatus("");
    content.innerHTML = `
      <div class="game-setup">
        <button class="game-back" type="button" data-action="picker">← ゲーム選択</button>
        <div class="game-rule"><span class="game-rule-icon">${game.icon}</span><p>${escapeHtml(game.rule)}</p></div>
        <h2>難易度</h2>
        <div class="game-difficulties">
          ${Object.entries(DIFFICULTIES).map(([key, value]) => `<label><input type="radio" name="gameDifficulty" value="${key}" ${key === "normal" ? "checked" : ""}><strong>${value.label}</strong><span>${value.seconds}秒・${value.maxLength}文字${value.hint ? "・ヒントと見本あり" : ""}</span><b>最大${difficultyMax(key)}点換算</b></label>`).join("")}
        </div>
        <div class="game-setup-meta"><span>全3問</span><span>素点は各100点</span><span id="gameDifficultyScore"></span><span id="gameDifficultyBest"></span></div>
        <button class="game-primary game-start" type="button">ゲーム開始</button>
      </div>`;
    content.querySelector('[data-action="picker"]').addEventListener("click", renderPicker);
    const syncDifficulty = () => {
      const difficulty = content.querySelector('input[name="gameDifficulty"]:checked')?.value || "normal";
      const config = DIFFICULTIES[difficulty];
      document.getElementById("gameDifficultyScore").textContent = `${config.label}は最大${difficultyMax(difficulty)}点へ換算`;
      document.getElementById("gameDifficultyBest").textContent = `この難易度の自己ベスト ${scoreLabel(bestFor(id, difficulty))}`;
    };
    content.querySelectorAll('input[name="gameDifficulty"]').forEach(input => input.addEventListener("change", syncDifficulty));
    syncDifficulty();
    content.querySelector(".game-start").addEventListener("click", async event => {
      const difficulty = content.querySelector('input[name="gameDifficulty"]:checked')?.value || "normal";
      event.currentTarget.disabled = true;
      event.currentTarget.textContent = "AIを確認中…";
      setStatus("採点するAIの準備を確認しています。", "busy");
      try {
        const readiness = await window.picoagent.gameReady();
        if (!readiness?.ready) throw new Error(readiness?.message || "AIへ接続できません。");
        startSession(game, difficulty);
      } catch (error) {
        playTone("error");
        event.currentTarget.disabled = false;
        event.currentTarget.textContent = "ゲーム開始";
        setStatus(error?.message || String(error), "error");
      }
    });
  }

  function startSession(game, difficulty) {
    const prompts = shuffle(game.prompts).slice(0, 3);
    const fault = game.id === "broken" ? FAULTS[Math.floor(Math.random() * FAULTS.length)] : null;
    session = { game, difficulty, prompts, fault, round: 0, answers: [], results: [], phase: "question", followup: "" };
    playTone("start");
    renderQuestion();
  }

  function promptForRound() {
    const { game, round, prompts, results } = session;
    if (game.id !== "apology") return prompts[round];
    const incident = prompts[0];
    if (round === 0) return `${incident}\n何が起き、なぜそうなったのか説明してください。`;
    if (round === 1) {
      const weakness = results[0]?.improvement || "説明が具体的ではありません";
      return `${incident}\n先ほどの説明について「${weakness}」という疑問が残っています。説明と矛盾しないよう、具体的に答えてください。`;
    }
    return `${incident}\n影響をどう元へ戻し、同じ失敗をどう防ぐのか説明してください。`;
  }

  function renderQuestion() {
    const { game, difficulty, round, fault } = session;
    const config = DIFFICULTIES[difficulty];
    const prompt = promptForRound();
    session.currentPrompt = prompt;
    session.phase = "question";
    title.textContent = game.title;
    setStatus("");
    content.innerHTML = `
      <div class="game-play">
        <div class="game-progress"><span>第${round + 1}問 / 3</span><div><i style="width:${(round / 3) * 100}%"></i></div><strong id="gameTimer">${config.seconds}</strong></div>
        ${fault ? `<div class="game-fault"><small>今回の故障</small><strong>${escapeHtml(fault[0])}</strong><span>${escapeHtml(fault[1])}</span></div>` : ""}
        <div class="game-prompt"><small>お題</small><p>${escapeHtml(prompt).replace(/\n/g, "<br>")}</p></div>
        ${config.hint ? `<div class="game-hint">ヒント：${escapeHtml(game.hint)}</div>` : ""}
        ${config.hint ? `<div class="game-example"><small>回答見本（別のお題）</small><p>${escapeHtml(game.example)}</p></div>` : ""}
        <label class="game-answer"><span>AIになりきって回答</span><textarea id="gameAnswer" maxlength="${config.maxLength}" rows="4" placeholder="ここに回答を書く…"></textarea><small><b id="gameCount">0</b> / ${config.maxLength}文字</small></label>
        <button id="gameSubmit" class="game-primary" type="button" disabled>採点してもらう</button>
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
    speakGame(`第${round + 1}問。${spokenFault}${prompt}`);
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
      commentary: "時間切れです。10文字以上の回答を書いて提出しよう。",
      highlight: "該当なし",
      improvement: "まず結論を一文書いてから、理由や代案を足してみよう。",
    };
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
    const previousAnswers = [...session.answers];
    content.innerHTML = `<div class="game-judging"><div class="game-thinking" aria-hidden="true"><i></i><i></i><i></i></div><strong>AIが採点中…</strong><span>回答のどこがAIっぽいか見ています</span></div>`;
    setStatus("この採点は通常の会話履歴へ入りません。", "busy");
    try {
      const result = await window.picoagent.gameJudge({
        gameId: session.game.id,
        prompt: session.currentPrompt,
        answer,
        round: session.round + 1,
        previousAnswers,
        fault: session.fault ? `${session.fault[0]}：${session.fault[1]}` : "",
      });
      session.answers.push(answer);
      session.results.push(result);
      playTone("score");
      renderRoundResult(result, false);
    } catch (error) {
      playTone("error");
      session.phase = "question";
      content.innerHTML = `<div class="game-error"><strong>採点できませんでした</strong><p>${escapeHtml(error?.message || error)}</p><div><button class="game-secondary" type="button" data-action="quit">ゲーム選択へ</button><button class="game-primary" type="button" data-action="retry">同じ回答を再採点</button></div></div>`;
      setStatus("回答は失われていません。", "error");
      content.querySelector('[data-action="quit"]').addEventListener("click", renderPicker);
      content.querySelector('[data-action="retry"]').addEventListener("click", () => submitAnswer(answer));
    }
  }

  function renderRoundResult(result, timedOut) {
    session.phase = "result";
    const criteria = result.criteria?.length ? result.criteria.map(item => `
      <li><div><strong>${escapeHtml(item.label)}</strong><span>${item.score} / ${item.maxScore}</span></div><meter min="0" max="${item.maxScore}" value="${item.score}"></meter><p>${escapeHtml(item.reason)}</p></li>`).join("") : `<li class="game-no-score">回答なしのため項目別採点はありません。</li>`;
    const nextLabel = session.round >= 2 ? "総合結果を見る" : "次の問題へ";
    title.textContent = timedOut ? "時間切れ" : `${result.total}点`;
    setStatus("");
    content.innerHTML = `
      <div class="game-result">
        <div class="game-score"><strong>${result.total}</strong><span>/ 100</span></div>
        <p class="game-commentary">${escapeHtml(result.commentary)}</p>
        <ul class="game-criteria">${criteria}</ul>
        <div class="game-feedback"><p><small>見せ場</small>${escapeHtml(result.highlight)}</p><p><small>次の一手</small>${escapeHtml(result.improvement)}</p></div>
        <button class="game-primary" type="button" data-action="next">${nextLabel}</button>
      </div>`;
    content.querySelector('[data-action="next"]').addEventListener("click", () => {
      session.round += 1;
      if (session.round >= 3) renderFinal();
      else renderQuestion();
    });
    speakGame(`${result.total}点。${result.commentary}`);
  }

  function rankFor(score) {
    if (score >= 850) return "AI超越";
    if (score >= 600) return "AIそのもの";
    if (score >= 400) return "AI完全再現";
    if (score >= 270) return "ほぼAI本人";
    if (score >= 225) return "かなりAIっぽい";
    if (score >= 165) return "AI見習い";
    if (score >= 90) return "人間味が漏れている";
    return "正直すぎる人間";
  }

  function renderFinal() {
    clearTimer();
    session.phase = "final";
    const rawTotal = session.results.reduce((sum, result) => sum + Number(result.total || 0), 0);
    const total = difficultyScore(rawTotal, session.difficulty);
    const maxTotal = difficultyMax(session.difficulty);
    const config = DIFFICULTIES[session.difficulty];
    const previousBest = bestFor(session.game.id, session.difficulty);
    const isBest = saveBest(session.game.id, session.difficulty, total);
    title.textContent = "総合結果";
    setStatus(isBest ? `${config.label}の自己ベスト更新！` : `${config.label}の自己ベスト ${Math.max(previousBest, total)} / ${maxTotal}`);
    playTone(isBest ? "best" : "score");
    content.innerHTML = `
      <div class="game-final">
        ${isBest ? '<div class="game-new-best">NEW BEST</div>' : ""}
        <span>${escapeHtml(session.game.title)}</span>
        <div class="game-total"><strong>${total}</strong><small>/ ${maxTotal}</small></div>
        <div class="game-total-breakdown"><span>素点 ${rawTotal} / 300</span><b>${escapeHtml(config.label)}・最大${maxTotal}点換算</b></div>
        <h2>${rankFor(total)}</h2>
        <div class="game-round-scores">${session.results.map((result, index) => `<div><span>第${index + 1}問</span><strong>${result.total}</strong></div>`).join("")}</div>
        <div class="game-final-actions"><button class="game-secondary" type="button" data-action="picker">ゲーム選択</button><button class="game-primary" type="button" data-action="again">もう一度</button></div>
      </div>`;
    content.querySelector('[data-action="picker"]').addEventListener("click", renderPicker);
    content.querySelector('[data-action="again"]').addEventListener("click", () => renderSetup(session.game.id));
    speakGame(`素点${rawTotal}点、${config.label}の最大${maxTotal}点換算で、総合得点${total}点。称号は、${rankFor(total)}。${isBest ? "自己ベスト更新です。" : ""}`);
  }

  function open() {
    overlay.hidden = false;
    document.getElementById("petApp")?.classList.add("is-playing-game");
    if (!session) renderPicker();
    closeButton.focus();
  }

  function close() {
    const active = session && !["final"].includes(session.phase);
    if (active && !window.confirm("問題の途中です。ゲームを終了しますか？ 今回の得点は保存されません。")) return;
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
    localStorage.setItem(SOUND_KEY, soundEnabled ? "on" : "off");
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
