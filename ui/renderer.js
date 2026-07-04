const petApp = document.getElementById("petApp");
const bubble = document.getElementById("bubble");
const historySearch = document.getElementById("historySearch");
const historyLog = document.getElementById("historyLog");
const reply = document.getElementById("reply");
const composer = document.getElementById("composer");
const input = document.getElementById("input");
const status = document.getElementById("status");
const bootStatus = document.getElementById("bootStatus");
const bootLog = document.getElementById("bootLog");
const bootPetImage = document.getElementById("bootPetImage");
const sendButton = document.getElementById("sendButton");
const cancelSendButton = document.getElementById("cancelSend");
const cancelVoiceButton = document.getElementById("cancelVoice");
const copyReplyButton = document.getElementById("copyReply");
const replayReplyButton = document.getElementById("replayReply");
const stopVoiceButton = document.getElementById("stopVoice");
const toggleFullscreenButton = document.getElementById("toggleFullscreen");
const noticeToggle = document.getElementById("noticeToggle");
const noticePanel = document.getElementById("noticePanel");
const noticeList = document.getElementById("noticeList");
const noticeClear = document.getElementById("noticeClear");
const character = document.getElementById("character");
const petSprite = document.getElementById("petSprite");
const online = document.getElementById("online");
const bubbleResize = document.getElementById("bubbleResize");
const imagePreview = document.getElementById("imagePreview");
const quickActions = document.getElementById("quickActions");
let busy = false;
// 添付中の画像（data URL の配列）。送信後にリセットする。
let attachedImages = [];
// 添付中のドキュメントファイル（{name, dataUrl} の配列）。送信後にリセットする。
let attachedFiles = [];
// エラーリトライ用に最後のユーザー入力テキストを保持する。
let lastUserText = "";
// エージェント進捗（pet:agent_status）を受信するリスナー解除関数。
let unlistenStatus = null;
const conversationHistory = [];
let historyQuery = "";
let conversationRevision = 0;
const promptHistory = [];
let promptHistoryIndex = null;
let promptDraft = "";
const QUICK_ACTIONS = [
  { label: "👀 画面を見る", text: "今の画面を見て、状況を短く教えて" },
  { label: "📋 クリップボード要約", text: "クリップボードの内容を要約して" },
  { label: "🗓 今日の予定", text: "今日の予定と気をつけることを教えて" },
  { label: "📧 未読メール要約", text: "Gmailの未読メールを一覧取得して、重要そうな順に短く要約して" },
  { label: "💬 Slack要約", text: "Slackの既定チャンネルの最新10件を読んで、話題と対応が必要なことを短く要約して" },
  { label: "📅 予定確認", text: "Googleカレンダーの今日の予定を確認して、次に気をつけることを教えて" },
  { label: "⏩ さっきの続き", text: "さっきの続きとして、次にやることを整理して" },
  { label: "✨ できること", text: "あなたが今できることを具体的に教えて" },
];
// アイドル時の自発雑談（雑談スキル）設定とタイマー。
const chatterCfg = { enabled: false, minSecs: 90, maxSecs: 240 };
let chatterTimer = null;
let dailyTimer = null;
let animationRequestId = null;
let bootLogTimer = null;
let bootLogLane = 0;
let lastBootStatus = "";
let lastAssistantText = "";
let voiceVolume = 1;
const pendingNotices = [];
let unreadNoticeCount = 0;
// 目覚まし時計の発火管理（同じ分に二重で鳴らさないための済みキー集合）。
const ALARM_POLL_MS = 20000;
let alarmTimer = null;
const firedAlarms = new Set();
let activeAlarmSnooze = null;

// 画面下部に短いトースト通知を表示する（1.8秒後に自動消去）。
// kind: "info"（既定）/ "success" / "warn" / "error" で配色が変わる。
function showToast(msg, kind = "info") {
  const el = document.createElement("div");
  el.className = kind === "info" ? "toast" : `toast toast-${kind}`;
  el.textContent = msg;
  document.body.append(el);
  requestAnimationFrame(() => el.classList.add("toast-show"));
  setTimeout(() => {
    el.classList.remove("toast-show");
    el.addEventListener("transitionend", () => el.remove(), { once: true });
  }, 1800);
}

// エラー表示後にリトライボタンを返信エリアへ追加する。
function addRetryButton() {
  const md = reply.querySelector(".md");
  if (!md || !lastUserText) return;
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "retry-btn";
  btn.textContent = "もう一度試す";
  btn.addEventListener("click", () => {
    if (busy) return;
    input.value = lastUserText;
    if (composer.hidden) openComposer();
    else updateComposerState();
  });
  md.append(btn);
}

function errorDetail(error) {
  if (!error) return "";
  if (error instanceof Error) return error.stack || error.message;
  if (typeof error === "string") return error;
  try { return JSON.stringify(error); }
  catch { return String(error); }
}

function logApp(level, message, detail) {
  try {
    window.picoagent?.log?.(level, message, detail == null ? "" : String(detail)).catch(error => {
      // フォールバック許可: ログ保存自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
      console.warn("アプリログ保存に失敗しました", error);
    });
  } catch (error) {
    // フォールバック許可: ログ保存自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
    console.warn("アプリログ保存に失敗しました", error);
  }
}

window.addEventListener("error", event => {
  logApp("error", "未捕捉フロントエラー", `${event.message || ""} ${event.filename || ""}:${event.lineno || ""}:${event.colno || ""}\n${errorDetail(event.error)}`);
});
window.addEventListener("unhandledrejection", event => {
  logApp("error", "未処理 Promise rejection", errorDetail(event.reason));
});
window.addEventListener("beforeunload", () => {
  if (chatterTimer) clearTimeout(chatterTimer);
  if (dailyTimer) clearInterval(dailyTimer);
  if (alarmTimer) clearInterval(alarmTimer);
  if (bootLogTimer) clearInterval(bootLogTimer);
  if (animationRequestId) cancelAnimationFrame(animationRequestId);
  if (unlistenStatus) { unlistenStatus(); unlistenStatus = null; }
  stopVoice(false);
  stopLive2D();
  if (live2dApp) { live2dApp.destroy(true); live2dApp = null; live2dCanvas = null; }
});

const characterManifestCacheKey = "2026-07-03-live2d-jp-voice-v1";
let characterRoot = "assets/characters/mugi";
let characterManifest = null;
let characterAssetVersion = "";
let bootCharacterName = "キャラクター";
let loadedCharacterSignature = "";
let animations = {
  idle: { path: "animations/idle", frames: 6, fps: 3.6, motion: "breathe" },
  thinking: { path: "animations/running", frames: 6, fps: 6.2, motion: "thinking" },
  talking: { path: "animations/review", frames: 6, fps: 5.4, motion: "talking" },
  sad: { path: "animations/failed", frames: 8, fps: 4.6, motion: "sad" },
};
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
let animationName = "idle";
let frame = 0;
let lastFrameAt = 0;

function setStatus(text) {
  status.textContent = text;
  if (bootStatus && petApp.classList.contains("is-booting")) {
    const next = text || "";
    bootStatus.textContent = next;
    if (next && next !== lastBootStatus) {
      lastBootStatus = next;
      appendBootLog(next, "status");
    }
  }
}

function updateNoticeBadge() {
  if (!noticeToggle) return;
  noticeToggle.dataset.count = unreadNoticeCount > 0 ? String(Math.min(99, unreadNoticeCount)) : "";
  noticeToggle.classList.toggle("has-notices", pendingNotices.length > 0);
}

function renderNoticePanel() {
  if (!noticeList) return;
  noticeList.replaceChildren();
  if (pendingNotices.length === 0) {
    noticeList.append(document.createElement("div"));
    noticeList.firstElementChild.className = "notice-empty";
    noticeList.firstElementChild.textContent = "通知はありません";
    return;
  }
  for (const notice of pendingNotices) {
    const item = document.createElement("div");
    item.className = `notice-item ${notice.ok === false ? "error" : "ok"}`;
    const title = document.createElement("div");
    title.className = "notice-title";
    title.textContent = notice.title || "通知";
    const meta = document.createElement("div");
    meta.className = "notice-meta";
    meta.textContent = notice.at ? new Date(notice.at).toLocaleString("ja-JP") : "";
    const body = document.createElement("div");
    body.className = "notice-body";
    body.textContent = notice.body || "";
    item.append(title, meta, body);
    if (notice.kind === "schedule" || notice.taskId) {
      const actions = document.createElement("div");
      actions.className = "notice-actions";
      if (notice.taskId) {
        const rerun = document.createElement("button");
        rerun.type = "button";
        rerun.textContent = "再実行";
        rerun.addEventListener("click", async event => {
          event.stopPropagation();
          rerun.disabled = true;
          try {
            await window.picoagent.rerunScheduleTask(notice.taskId);
            setStatus("スケジュールを再実行しました");
          } catch (error) {
            pushNotice({
              kind: "schedule",
              title: "スケジュール再実行失敗",
              body: error?.message || String(error),
              ok: false,
              at: new Date().toISOString(),
            });
          } finally {
            rerun.disabled = false;
          }
        });
        actions.append(rerun);
      }
      const settings = document.createElement("button");
      settings.type = "button";
      settings.textContent = "設定";
      settings.addEventListener("click", async event => {
        event.stopPropagation();
        try { await window.picoagent.openSettings(); }
        catch (error) { setStatus(`設定を開けませんでした: ${error?.message || error}`); }
      });
      actions.append(settings);
      item.append(actions);
    }
    noticeList.append(item);
  }
}

function pushNotice(notice) {
  const entry = {
    kind: String(notice?.kind || ""),
    title: String(notice?.title || "通知"),
    body: String(notice?.body || ""),
    ok: notice?.ok !== false,
    taskId: notice?.taskId ? String(notice.taskId) : "",
    runId: notice?.runId ? String(notice.runId) : "",
    at: notice?.at || new Date().toISOString(),
  };
  pendingNotices.unshift(entry);
  while (pendingNotices.length > 20) pendingNotices.pop();
  unreadNoticeCount += 1;
  updateNoticeBadge();
  renderNoticePanel();
  setStatus(entry.ok ? entry.title : `${entry.title}: エラー`);
  setTimeout(() => {
    if (status.textContent === entry.title || status.textContent === `${entry.title}: エラー`) status.textContent = "";
  }, 6000);
}

async function loadScheduleRunNotices() {
  try {
    const runs = await window.picoagent.scheduleRuns();
    if (!Array.isArray(runs)) return;
    pendingNotices.length = 0;
    for (const run of runs.slice(0, 20)) {
      pendingNotices.push({
        kind: "schedule",
        title: String(run.title || "スケジュール"),
        body: String(run.body || ""),
        ok: run.ok !== false,
        taskId: run.task_id ? String(run.task_id) : "",
        runId: run.id ? String(run.id) : "",
        at: run.at || "",
      });
    }
    unreadNoticeCount = 0;
    renderNoticePanel();
    updateNoticeBadge();
  } catch (error) {
    logApp("warn", "スケジュール実行履歴の読み込みに失敗しました", errorDetail(error));
  }
}

function appendBootLog(text, kind = "info") {
  if (!bootLog || !petApp.classList.contains("is-booting")) return;
  const line = document.createElement("div");
  line.className = `boot-log-line boot-kind-${kind}`;
  line.textContent = text;
  const lanes = Math.max(1, Math.floor((bootLog.clientHeight - 24) / 24));
  const lane = bootLogLane % lanes;
  bootLogLane += 1;
  line.style.top = `${12 + lane * 24}px`;
  line.style.animationDuration = `${Math.min(26, Math.max(14, line.textContent.length * 0.34))}s`;
  line.addEventListener("animationend", () => line.remove(), { once: true });
  bootLog.append(line);
  while (bootLog.children.length > 24) bootLog.firstElementChild?.remove();
}

function startBootLogTicker() {
  if (!bootLog || bootLogTimer) return;
}

function stopBootLogTicker() {
  if (!bootLogTimer) return;
  clearInterval(bootLogTimer);
  bootLogTimer = null;
}

function frameUrl(name, index) {
  const animation = animations[name] || animations.idle;
  const url = `${characterRoot}/${animation.path}/${String(index).padStart(2, "0")}.png`;
  return characterAssetVersion ? `${url}?v=${encodeURIComponent(characterAssetVersion)}` : url;
}

function versionedAssetUrl(url, version) {
  return version ? `${url}?v=${encodeURIComponent(version)}` : url;
}

async function loadCharacterManifest(characterId) {
  const catalogResponse = await fetch(`assets/characters/index.json?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!catalogResponse.ok) throw new Error(`キャラクター一覧: HTTP ${catalogResponse.status}`);
  const catalog = await catalogResponse.json();
  const entry = catalog.characters.find(character => character.id === characterId)
    || catalog.characters.find(character => character.id === catalog.default);
  if (!entry) throw new Error("利用できるキャラクターがありません");
  const root = `assets/characters/${entry.manifest.replace(/\/character\.json$/, "")}`;
  const manifestResponse = await fetch(`assets/characters/${entry.manifest}?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!manifestResponse.ok) throw new Error(`キャラクター定義: HTTP ${manifestResponse.status}`);
  const manifest = await manifestResponse.json();
  return { entry, manifest, root };
}

async function prepareBootCharacter(characterId) {
  if (!bootPetImage) return;
  const { manifest, root } = await loadCharacterManifest(characterId);
  const version = String(manifest.assetVersion || "");
  const bootPath = manifest.boot?.sleeping || `${manifest.animations?.idle?.path || "animations/idle"}/00.png`;
  bootPetImage.src = versionedAssetUrl(`${root}/${bootPath}`, version);
  bootPetImage.alt = `起動を待つ${manifest.name || "キャラクター"}`;
  bootCharacterName = manifest.name || "キャラクター";
}

function preloadAnimations() {
 for (const [name, animation] of Object.entries(animations)) {
  for (let index = 0; index < animation.frames; index += 1) {
    const image = new Image(); image.src = frameUrl(name, index);
  }
 }
}
preloadAnimations();

function personaText(manifest, override) {
  if (override?.trim()) return override.trim();
  const persona = manifest.persona || {};
  return [
    `名前: ${manifest.name || manifest.id}`,
    `性格: ${persona.personality || ""}`,
    `口調: ${persona.speechStyle || ""}`,
    `一人称: ${persona.firstPerson || ""}`,
    `ユーザーの呼び方: ${persona.userName || ""}`,
    persona.systemPrompt || "",
  ].filter(Boolean).join("\n");
}

async function loadCharacter(characterId, overrides = {}, options = {}) {
  const { manifest, root } = await loadCharacterManifest(characterId);
  if (!manifest.live2d?.model && !manifest.animations?.idle) throw new Error("live2d モデルも idle アニメーションも定義されていません");
  const profile = personaText(manifest, overrides[manifest.id]);
  const assetVersion = String(manifest.assetVersion || "");
  const signature = `${manifest.id}\n${assetVersion}\n${profile}`;
  if (signature === loadedCharacterSignature) return;
  characterRoot = root;
  characterManifest = manifest;
  characterAssetVersion = assetVersion;
  animations = manifest.animations || {};
  stopRig();
  detachLipSyncAudio(); // Live2D キャラの口パラメータ解析も解除する
  if (manifest.live2d?.model) {
    rigAvailable = false; // まばたき・口パクはモデル定義側が担当する
    try {
      await startLive2D(manifest);
    } catch (error) {
      stopLive2D();
      if (!manifest.animations?.idle) throw error;
      // フォールバック許可: Live2D ランタイム未取得でも、フレームアニメを持つキャラは表示を継続する（通知済み）。
      logApp("error", "Live2D の初期化に失敗しました（フレームアニメで継続）", errorDetail(error));
      setStatus("Live2D を初期化できませんでした");
      preloadAnimations();
      await probeRigLayers();
    }
  } else {
    stopLive2D();
    preloadAnimations();
    await probeRigLayers(); // layers 素材があればリップシンク対応
  }
  frame = 0;
  setAnimation("idle", true);
  character.title = `${manifest.name}：右クリックで設定`;
  await window.picoagent.setCharacterProfile(manifest.id, profile);
  loadedCharacterSignature = signature;
  if (manifest.greeting) {
    renderReply(manifest.greeting);
    // 起動時は描画と同時に再生するため、ここでは合成しない（boot 側が事前合成→表示と同時再生）。
    if (options.speakGreeting !== false) speakReply(manifest.greeting);
  }
}
function setAnimation(name, force = false) {
  if (!force && animationName === name) return;
  animationName = (animations[name] || live2dActive) ? name : "idle"; frame = 0; lastFrameAt = performance.now();
  if (live2dActive) { character.dataset.motion = "none"; playLive2DMotion(animationName); return; }
  if (!animations[animationName]) return; // Live2D 初期化失敗直後などフレーム定義が無い場合は何もしない
  character.dataset.motion = animations[animationName].motion || animationName;
  // layers を持つキャラは待機・返答状態を常時リグ描画にする（Live2D 風の常時まばたき）。
  if (rigAvailable && !reduceMotion && (animationName === "idle" || animationName === "talking")) {
    startRig();
    return;
  }
  stopRig();
  petSprite.src = frameUrl(animationName, 0);
}
function frameDuration(animation, index) {
  const duration = animation.frameDurations?.[index];
  if (Number.isFinite(duration) && duration > 0) return duration;
  return 1000 / animation.fps;
}
function animate(now) {
  // Live2D 動作中はフレーム送り不要（PIXI 側の ticker が描画を担当する）。
  if (live2dActive) { animationRequestId = requestAnimationFrame(animate); return; }
  if (reduceMotion) { if (animations.idle) petSprite.src = frameUrl("idle", 0); return; }
  // リグ動作中はフレーム送りを止める（ベース＋オーバーレイ側が表情を担当する）。
  if (rigActive) { animationRequestId = requestAnimationFrame(animate); return; }
  const animation = animations[animationName] || animations.idle;
  if (!animation) { animationRequestId = requestAnimationFrame(animate); return; } // フレーム定義が無いキャラ（Live2D 初期化失敗時）はループだけ維持する
  if (!lastFrameAt) lastFrameAt = now;
  if (now - lastFrameAt >= frameDuration(animation, frame)) {
    frame = (frame + 1) % animation.frames;
    petSprite.src = frameUrl(animationName, frame);
    lastFrameAt = now;
  }
  animationRequestId = requestAnimationFrame(animate);
}
animationRequestId = requestAnimationFrame(animate);

// ── 疑似Live2Dリグ ─────────────────────────────────────────────────
// layers/（base・mouth_open/closed・blink_half/closed）は「同一ポーズの全身差分絵」。
// 読み上げ中はフレームアニメの代わりに差分絵を切り替え（PNGTuber方式）、
// WebAudio の振幅解析で実際の声量に合わせて口を動かし、ランダムにまばたきする。
let rigAvailable = false; // 選択中キャラに layers 素材が揃っているか
let rigActive = false;
let rigFace = "";          // 表示中の差分名（無駄な src 再設定を避ける）
let rigMouthOpen = false;  // 口の開閉（振幅解析またはタイマーが更新）
let rigBlinkPhase = null;  // まばたき中の差分名（null = まばたき中でない）
let rigMouthTimer = 0;
let rigBlinkTimer = 0;
let rigVolFrame = 0;
let audioCtx = null;
let rigSource = null;
let rigAnalyser = null;

function layerUrl(name) {
  const url = `${characterRoot}/layers/${name}.png`;
  return characterAssetVersion ? `${url}?v=${encodeURIComponent(characterAssetVersion)}` : url;
}

// layers 一式が読み込めるかを一度だけ確認する（無いキャラは従来のフレームアニメのみ）。
// 読み込み成功した Image はブラウザキャッシュに乗るためプリロードも兼ねる。
async function probeRigLayers() {
  rigAvailable = false;
  const names = ["base", "mouth_open", "mouth_closed", "blink_half", "blink_closed"];
  const results = await Promise.all(names.map(name => new Promise(resolve => {
    const img = new Image();
    img.onload = () => resolve(true);
    img.onerror = () => resolve(false);
    img.src = layerUrl(name);
  })));
  rigAvailable = results.every(Boolean);
}

// まばたき > 口 の優先度で現在の差分絵を決めて反映する。
function rigRefresh() {
  if (!rigActive) return;
  const name = rigBlinkPhase || (rigMouthOpen ? "mouth_open" : "mouth_closed");
  if (rigFace === name) return;
  rigFace = name;
  petSprite.src = layerUrl(name);
}

// まばたき: half → closed → half を素早く挟み、1.4〜4.2秒間隔で繰り返す。
function scheduleBlink() {
  clearTimeout(rigBlinkTimer);
  if (!rigActive) return;
  rigBlinkTimer = setTimeout(async () => {
    if (!rigActive) return;
    const step = (phase, ms) => new Promise(resolve => {
      rigBlinkPhase = phase;
      rigRefresh();
      setTimeout(resolve, ms);
    });
    await step("blink_half", 60);
    await step("blink_closed", 90);
    await step("blink_half", 60);
    rigBlinkPhase = null;
    rigRefresh();
    scheduleBlink();
  }, 1400 + Math.random() * 2800);
}

// 現在のチャンク音声の解析だけを解除する（リグの見た目は維持）。
function detachLipSyncAudio() {
  if (rigMouthTimer) { clearInterval(rigMouthTimer); rigMouthTimer = 0; }
  if (rigVolFrame) { cancelAnimationFrame(rigVolFrame); rigVolFrame = 0; }
  try { rigSource?.disconnect(); rigAnalyser?.disconnect(); } catch {}
  rigSource = null;
  rigAnalyser = null;
  rigMouthOpen = false;
  live2dMouth = -1;
  rigRefresh();
}

// 音声要素へリップシンクを接続する。リグ未対応キャラでは何もしない。
// Live2D キャラでは差分絵の代わりに live2dMouth（モデルの口パラメータ）を動かす。
function attachLipSync(audio) {
  if ((!rigAvailable && !live2dActive) || reduceMotion) return;
  if (!live2dActive) startRig();
  detachLipSyncAudio();
  // WebAudio の振幅解析を試み、使えない環境ではタイマー口パクへフォールバックする。
  let useTimer = true;
  try {
    audioCtx = audioCtx || new (window.AudioContext || window.webkitAudioContext)();
    if (audioCtx.state === "suspended") audioCtx.resume().catch(() => {});
    if (audioCtx.state === "running") {
      rigSource = audioCtx.createMediaElementSource(audio);
      rigAnalyser = audioCtx.createAnalyser();
      rigAnalyser.fftSize = 512;
      rigSource.connect(rigAnalyser);
      rigAnalyser.connect(audioCtx.destination);
      useTimer = false;
    }
  } catch (error) {
    logApp("warn", "リップシンクの音声解析を初期化できませんでした（タイマー式で継続）", errorDetail(error));
  }
  if (useTimer) {
    rigMouthTimer = setInterval(() => {
      rigMouthOpen = !rigMouthOpen;
      if (live2dActive) live2dMouth = rigMouthOpen ? 0.8 : 0;
      else rigRefresh();
    }, 120);
    return;
  }
  const data = new Uint8Array(rigAnalyser.fftSize);
  let peak = 0.04; // 音量差に追従する適応しきい値
  const analyserLoop = () => {
    if ((!rigActive && !live2dActive) || !rigAnalyser) return;
    rigAnalyser.getByteTimeDomainData(data);
    let sum = 0;
    for (let i = 0; i < data.length; i += 1) {
      const v = (data[i] - 128) / 128;
      sum += v * v;
    }
    const rms = Math.sqrt(sum / data.length);
    peak = Math.max(peak * 0.995, rms, 0.02);
    if (live2dActive) {
      // 声量に比例して口を開く（0〜1）。小さな音は閉じたままにする。
      live2dMouth = rms > peak * 0.3 ? Math.min(1, rms / (peak * 0.9)) : 0;
    } else {
      rigMouthOpen = rms > peak * 0.45;
      rigRefresh();
    }
    rigVolFrame = requestAnimationFrame(analyserLoop);
  };
  rigVolFrame = requestAnimationFrame(analyserLoop);
}

// リグ描画を開始する（既に動作中なら何もしない。口は閉じ・まばたき開始）。
function startRig() {
  if (rigActive) return;
  rigActive = true;
  rigFace = "";
  rigMouthOpen = false;
  rigBlinkPhase = null;
  rigRefresh();
  scheduleBlink();
}

// リグを完全に終了する（キャラ切替・thinking/sad などフレームアニメ状態への遷移用）。
function stopRig() {
  if (!rigActive) return;
  rigActive = false;
  detachLipSyncAudio();
  clearTimeout(rigBlinkTimer);
  rigBlinkPhase = null;
  rigFace = "";
}

// 読み上げ終了時の処理。layers を持つキャラは待機中も常時リグ（まばたき）を続けるため
// 音声解析だけ解除し、リグ非対応キャラのみ完全に終了してフレーム表示へ戻す。
function stopLipSync() {
  if (live2dActive) { detachLipSyncAudio(); return; }
  if (rigAvailable && !reduceMotion) { detachLipSyncAudio(); return; }
  const wasActive = rigActive;
  stopRig();
  if (wasActive && animations[animationName]) petSprite.src = frameUrl(animationName, frame);
}

// ── Live2D 本体レンダリング ─────────────────────────────────────────
// character.json に live2d.model がある場合、PNG フレームの代わりに Cubism ランタイム
// （ui/vendor/live2d/、npm run setup:live2d で取得）でモデルを WebGL 描画する。
// まばたきとアイドルモーションはモデル定義（EyeBlink・Idle グループ）が自動で担当し、
// 読み上げ中は attachLipSync の振幅解析が live2dMouth を介して口パラメータを直接動かす。
let live2dActive = false;
let live2dModel = null;
let live2dApp = null;
let live2dCanvas = null;
let live2dMouth = -1; // 読み上げ中の口の開き（0〜1）。-1 はモーション任せ。
let live2dRuntimePromise = null;

function loadScriptOnce(src) {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = src;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(`スクリプトを読み込めませんでした: ${src}`));
    document.head.append(script);
  });
}

// ランタイム3点を一度だけ順に読み込む（Cubism Core → PIXI → pixi-live2d-display）。
async function ensureLive2DRuntime() {
  if (window.PIXI?.live2d?.Live2DModel) return;
  if (!live2dRuntimePromise) {
    live2dRuntimePromise = (async () => {
      await loadScriptOnce("vendor/live2d/live2dcubismcore.min.js");
      await loadScriptOnce("vendor/live2d/pixi.min.js");
      await loadScriptOnce("vendor/live2d/cubism4.min.js");
      // モデル同梱のモーション音声（サンプルボイス）は TTS 読み上げと二重再生になるため無効化する。
      if (window.PIXI?.live2d?.config) PIXI.live2d.config.sound = false;
    })().catch(error => {
      live2dRuntimePromise = null; // 取得後の再選択で再試行できるようにする
      throw new Error(`Live2D ランタイムを読み込めません（npm run setup:live2d を実行してください）: ${error.message}`);
    });
  }
  await live2dRuntimePromise;
}

async function startLive2D(manifest) {
  await ensureLive2DRuntime();
  stopLive2D();
  const cfg = manifest.live2d;
  if (!live2dCanvas) {
    live2dCanvas = document.createElement("canvas");
    live2dCanvas.id = "live2dCanvas";
    character.insertBefore(live2dCanvas, petSprite);
  }
  live2dCanvas.style.display = "";
  const width = character.clientWidth || 265;
  const height = character.clientHeight || 315;
  if (!live2dApp) {
    live2dApp = new PIXI.Application({
      view: live2dCanvas,
      width,
      height,
      backgroundAlpha: 0,
      antialias: true,
      resolution: window.devicePixelRatio || 1,
      autoDensity: true,
    });
  }
  const model = await PIXI.live2d.Live2DModel.from(`${characterRoot}/${cfg.model}`, { autoInteract: false });
  // 表示枠へ等比率で収め、下端中央に立たせる。
  const fit = Math.min(width / model.internalModel.originalWidth, height / model.internalModel.originalHeight);
  model.scale.set(fit * (Number(cfg.scale) || 1));
  model.anchor.set(0.5, 1);
  model.position.set(width / 2, height + (Number(cfg.offsetY) || 0));
  // モーション更新の直後にリップシンク値を上書きする（モーション側が口を閉じ戻すのを防ぐ）。
  // 口パラメータIDはモデル定義（Groups: LipSync）から自動検出する。旧モデルは
  // PARAM_MOUTH_OPEN_Y のような旧命名のため、固定既定値だと口が動かない。
  const coreModel = model.internalModel.coreModel;
  const lipSyncGroup = model.internalModel.settings?.groups?.find(group => group.Name === "LipSync");
  const lipParam = String(cfg.lipSyncParam || lipSyncGroup?.Ids?.[0] || "ParamMouthOpenY");
  const motionUpdate = model.internalModel.motionManager.update.bind(model.internalModel.motionManager);
  model.internalModel.motionManager.update = (...args) => {
    const updated = motionUpdate(...args);
    if (live2dMouth >= 0) coreModel.setParameterValueById(lipParam, live2dMouth);
    return updated;
  };
  if (reduceMotion) {
    // モーション再生を止め、呼吸などの静かな自動更新だけにする。
    model.internalModel.motionManager.groups.idle = "";
  } else if (cfg.motions?.idle) {
    model.internalModel.motionManager.groups.idle = cfg.motions.idle;
  }
  live2dApp.stage.addChild(model);
  live2dModel = model;
  live2dActive = true;
  petSprite.style.display = "none";
}

// 状態名（idle/thinking/talking/sad）を character.json の live2d.motions でグループへ写像して再生する。
function playLive2DMotion(name) {
  if (!live2dModel || reduceMotion) return;
  if (name === "idle") { live2dModel.internalModel.motionManager.stopAllMotions(); return; } // Idle グループの自動再生へ戻す
  const motions = characterManifest?.live2d?.motions || {};
  const group = motions[name] || motions.idle;
  if (!group) return;
  const priority = window.PIXI?.live2d?.MotionPriority?.FORCE ?? 3;
  live2dModel.motion(group, undefined, priority);
}

// モデルを破棄して PNG フレーム表示へ戻す（PIXI アプリとキャンバスは再利用のため保持）。
function stopLive2D() {
  live2dMouth = -1;
  if (live2dModel) {
    live2dApp?.stage?.removeChild(live2dModel);
    live2dModel.destroy();
    live2dModel = null;
  }
  if (live2dCanvas) live2dCanvas.style.display = "none";
  if (live2dActive) {
    live2dActive = false;
    petSprite.style.display = "";
  }
}

// ── 疑似視線追従: マウスの方向へキャラクターがわずかに傾く ─────────────
if (!reduceMotion) {
  let lookResetTimer = 0;
  document.addEventListener("mousemove", event => {
    // Live2D はモデル自身の視線・顔向きをカーソルへ向ける。
    if (live2dActive && live2dModel && live2dCanvas) {
      const canvasRect = live2dCanvas.getBoundingClientRect();
      live2dModel.focus(event.clientX - canvasRect.left, event.clientY - canvasRect.top);
    }
    const rect = character.getBoundingClientRect();
    const centerX = rect.left + rect.width / 2;
    const dx = Math.max(-1, Math.min(1, (event.clientX - centerX) / Math.max(240, window.innerWidth / 2)));
    character.style.setProperty("--look-rot", `${(dx * 2.6).toFixed(2)}deg`);
    clearTimeout(lookResetTimer);
    lookResetTimer = setTimeout(() => character.style.setProperty("--look-rot", "0deg"), 2600);
  });
}

function autosizeInput() {
  input.style.height = "auto";
  input.style.height = `${Math.min(180, Math.max(62, input.scrollHeight))}px`;
}
function updateComposerState() {
  if (sendButton) sendButton.disabled = busy || (!input.value.trim() && attachedImages.length === 0 && attachedFiles.length === 0);
  quickActions?.querySelectorAll("button").forEach(button => { button.disabled = busy; });
}
function setInputValue(value) {
  input.value = String(value ?? "");
  autosizeInput();
  updateComposerState();
}
function rememberPrompt(text) {
  const value = String(text ?? "").trim();
  if (!value) return;
  if (promptHistory[promptHistory.length - 1] === value) return;
  promptHistory.push(value);
  if (promptHistory.length > 40) promptHistory.shift();
}
function resetPromptHistoryNavigation() {
  promptHistoryIndex = null;
  promptDraft = input.value;
}
function recallPromptHistory(direction) {
  if (!promptHistory.length) return false;
  if (promptHistoryIndex == null) {
    promptDraft = input.value;
    promptHistoryIndex = promptHistory.length;
  }
  promptHistoryIndex = Math.min(promptHistory.length, Math.max(0, promptHistoryIndex + direction));
  const next = promptHistoryIndex === promptHistory.length ? promptDraft : promptHistory[promptHistoryIndex];
  setInputValue(next);
  input.setSelectionRange(input.value.length, input.value.length);
  status.textContent = promptHistoryIndex === promptHistory.length ? "編集中の入力に戻しました" : "過去の入力を呼び戻しました";
  setTimeout(() => {
    if (status.textContent === "過去の入力を呼び戻しました" || status.textContent === "編集中の入力に戻しました") {
      status.textContent = "";
    }
  }, 1400);
  return true;
}
function openComposer() {
  if (busy) return;
  composer.hidden = false;
  reply.classList.add("compact");
  autosizeInput();
  updateComposerState();
  input.focus();
  scheduleChatter();
}
function closeComposer() {
  promptDraft = input.value;
  composer.hidden = true;
  reply.classList.remove("compact");
  updateComposerState();
  setTimeout(flushPendingNotice, 0);
}

// ダブルクリックでウィンドウをフルスクリーン化し、その中でフキダシを大きく広げる／元に戻すトグル。
async function setBubbleFullscreen(on) {
  if (bubble.classList.contains("fullscreen") === on) return;
  bubble.classList.toggle("fullscreen", on);
  refreshBubblePlacement();
  renderHistory();
  try { await window.picoagent.setFullscreen(on); }
  catch (error) { bubble.classList.toggle("fullscreen", !on); status.textContent = `全画面切替に失敗: ${error}`; }
  refreshBubblePlacement();
  setTimeout(refreshBubblePlacement, 120);
  if (!on) setTimeout(flushPendingNotice, 0);
}
function toggleBubbleFullscreen() { setBubbleFullscreen(!bubble.classList.contains("fullscreen")); }
function exitBubbleFullscreen() { setBubbleFullscreen(false); }

// ローカルサーバー（ComfyUI 等）の http:// 画像 URL を安全と判定する。外部 http:// は除く。
function isSafeImageUrl(url) {
  return url.startsWith("https://") ||
    /^http:\/\/(localhost|127\.0\.0\.1)(:\d+)?\//.test(url);
}

// インライン Markdown（**bold** / *italic* / `code` / リンク / 画像）を DOM へ追加する。
function appendInline(parent, raw) {
  // bold → italic → code → 画像 → リンク → 裸URL の優先順でマッチ。
  const pat = /\*\*([^*\n]+?)\*\*|\*([^*\n]+?)\*|`([^`\n]+)`|!\[([^\]]*)\]\((https?:\/\/[^\s)]+)\)|\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)|(https?:\/\/[^\s<>,)[\]]+)/g;
  let off = 0;
  for (const m of raw.matchAll(pat)) {
    if (m.index > off) parent.append(document.createTextNode(raw.slice(off, m.index)));
    if (m[1] !== undefined) {
      const s = document.createElement("strong"); s.textContent = m[1]; parent.append(s);
    } else if (m[2] !== undefined) {
      const e = document.createElement("em"); e.textContent = m[2]; parent.append(e);
    } else if (m[3] !== undefined) {
      const c = document.createElement("code"); c.textContent = m[3]; parent.append(c);
    } else if (m[4] !== undefined) {
      // 画像 ![alt](url)
      if (isSafeImageUrl(m[5])) {
        const img = document.createElement("img");
        img.src = m[5]; img.alt = m[4] || "画像";
        img.style.cssText = "max-width:100%;border-radius:6px;display:block;margin:4px 0;cursor:pointer";
        img.title = "クリックして開く";
        img.addEventListener("click", () => window.open(m[5], "_blank", "noopener,noreferrer"));
        parent.append(img);
      } else {
        const a = document.createElement("a"); a.href = m[5]; a.textContent = m[4] || m[5];
        a.target = "_blank"; a.rel = "noopener noreferrer"; parent.append(a);
      }
    } else if (m[6] !== undefined) {
      const a = document.createElement("a"); a.href = m[7]; a.textContent = m[6];
      a.target = "_blank"; a.rel = "noopener noreferrer"; parent.append(a);
    } else {
      const a = document.createElement("a"); a.href = m[0]; a.textContent = m[0];
      a.target = "_blank"; a.rel = "noopener noreferrer"; parent.append(a);
    }
    off = m.index + m[0].length;
  }
  if (off < raw.length) parent.append(document.createTextNode(raw.slice(off)));
}

// Markdown をブロック単位でパースして container に DOM を構築する。
// ストリーミング中に部分テキストで繰り返し呼ばれることを想定し、未完了の記法は無視する。
function renderMarkdown(container, src) {
  container.replaceChildren();
  const wrap = document.createElement("div");
  wrap.className = "md";
  const lines = String(src ?? "").split('\n');
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    // コードフェンス（``` で始まる）
    const fence = line.match(/^```(\w*)/);
    if (fence) {
      i++;
      const codeLines = [];
      while (i < lines.length && !lines[i].startsWith('```')) { codeLines.push(lines[i]); i++; }
      if (i < lines.length) i++; // 閉じ ``` をスキップ
      const pre = document.createElement("pre");
      if (fence[1]) pre.dataset.lang = fence[1];
      const code = document.createElement("code");
      code.textContent = codeLines.join('\n');
      const copyBtn = document.createElement("button");
      copyBtn.type = "button"; copyBtn.className = "copy-code"; copyBtn.textContent = "コピー";
      copyBtn.addEventListener("click", () => {
        navigator.clipboard.writeText(code.textContent || "");
        copyBtn.textContent = "✓";
        setTimeout(() => copyBtn.textContent = "コピー", 1500);
      });
      pre.append(copyBtn, code);
      wrap.append(pre);
      continue;
    }
    // 見出し（# ## ###）
    const h = line.match(/^(#{1,3})\s(.+)/);
    if (h) {
      const el = document.createElement(`h${h[1].length}`);
      appendInline(el, h[2]); wrap.append(el); i++; continue;
    }
    // 水平線
    if (/^(---+|\*\*\*+|___+)$/.test(line.trim())) {
      wrap.append(document.createElement("hr")); i++; continue;
    }
    // 引用ブロック
    if (/^>\s?/.test(line)) {
      const bq = document.createElement("blockquote");
      const qLines = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) { qLines.push(lines[i].replace(/^>\s?/, "")); i++; }
      appendInline(bq, qLines.join('\n'));
      wrap.append(bq); continue;
    }
    // 箇条書きリスト（- * +）
    if (/^[-*+]\s/.test(line)) {
      const ul = document.createElement("ul");
      while (i < lines.length && /^[-*+]\s/.test(lines[i])) {
        const li = document.createElement("li");
        appendInline(li, lines[i].replace(/^[-*+]\s+/, ""));
        ul.append(li); i++;
      }
      wrap.append(ul); continue;
    }
    // 番号付きリスト
    if (/^\d+\.\s/.test(line)) {
      const ol = document.createElement("ol");
      while (i < lines.length && /^\d+\.\s/.test(lines[i])) {
        const li = document.createElement("li");
        appendInline(li, lines[i].replace(/^\d+\.\s+/, ""));
        ol.append(li); i++;
      }
      wrap.append(ol); continue;
    }
    // 空行 → 改行
    if (line.trim() === "") { wrap.append(document.createElement("br")); i++; continue; }
    // 段落（連続する非ブロック行をまとめる）
    const pLines = [];
    while (i < lines.length && lines[i].trim() !== ""
      && !/^#{1,3}\s/.test(lines[i]) && !/^[-*+]\s/.test(lines[i])
      && !/^\d+\.\s/.test(lines[i]) && !/^```/.test(lines[i])
      && !/^>\s?/.test(lines[i]) && !/^(---+|\*\*\*+|___+)$/.test(lines[i].trim())) {
      pLines.push(lines[i]); i++;
    }
    if (pLines.length) {
      const p = document.createElement("p");
      pLines.forEach((pl, j) => { if (j) p.append(document.createElement("br")); appendInline(p, pl); });
      wrap.append(p);
    }
  }
  // run_command スキルが承認待ちのとき、クリックで送信できるボタンを表示する。
  // コードブロック・引用内のマッチは除外する（フェンス外の行のみ走査）。
  const approvalMarker = "このコマンドはまだ許可されていません";
  const hasApproval = (() => {
    let inFence = false;
    for (const ln of String(src ?? "").split('\n')) {
      if (ln.startsWith('```')) { inFence = !inFence; continue; }
      if (!inFence && !ln.startsWith('>') && ln.includes(approvalMarker)) return true;
    }
    return false;
  })();
  if (hasApproval) {
    // 承認ボタン（キャンセル→常に許可→今回のみ実行の順。主ボタンは右端）。
    const makeButtons = () => {
      const btns = document.createElement("div");
      btns.className = "approval-actions";
      [["キャンセル", "approval-cancel", "キャンセル"],
       ["常に許可", "approval-persist", "永続許可"],
       ["今回のみ実行", "approval-ok", "許可"]].forEach(([label, cls, val]) => {
        const btn = document.createElement("button");
        btn.type = "button"; btn.className = cls; btn.textContent = label;
        btn.addEventListener("click", () => {
          if (busy) {
            status.textContent = "処理中です…少し待ってください";
            setTimeout(() => { if (status.textContent.startsWith("処理中")) status.textContent = ""; }, 2000);
            return;
          }
          input.value = val;
          if (composer.hidden) openComposer();
          composer.requestSubmit();
        });
        btns.append(btn);
      });
      return btns;
    };
    // 承認プロンプトから「コマンド/作業ディレクトリ/理由」を取り出せたら、
    // 一般的な許可ダイアログ風のカードとして描画する（生テキストは出さない）。
    const command = String(src ?? "").match(/^コマンド:\s?(.+)$/m)?.[1];
    const cwd = String(src ?? "").match(/^作業ディレクトリ:\s?(.+)$/m)?.[1];
    const reason = String(src ?? "").match(/^理由:\s?(.+)$/m)?.[1];
    if (command) {
      const card = document.createElement("div");
      card.className = "approval-card";
      const title = document.createElement("div");
      title.className = "approval-card-title";
      title.textContent = "🛡️ コマンドの実行許可";
      const desc = document.createElement("p");
      desc.className = "approval-card-desc";
      desc.textContent = "次のコマンドを実行してもいい？";
      const cmd = document.createElement("div");
      cmd.className = "approval-cmd";
      cmd.textContent = `$ ${command}`;
      card.append(title, desc, cmd);
      const meta = document.createElement("div");
      meta.className = "approval-meta";
      if (cwd) { const row = document.createElement("div"); row.textContent = `📁 ${cwd}`; meta.append(row); }
      if (reason && reason !== "(理由未記載)") { const row = document.createElement("div"); row.textContent = `💬 ${reason}`; meta.append(row); }
      if (meta.childElementCount) card.append(meta);
      card.append(makeButtons());
      wrap.replaceChildren(card);
    } else {
      // 形式が読み取れないときは従来どおり本文の下へボタンだけ付ける。
      wrap.append(makeButtons());
    }
  }
  container.append(wrap);
}

function renderQuickActions() {
  if (!quickActions) return;
  quickActions.replaceChildren();
  for (const action of QUICK_ACTIONS) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = action.label;
    button.title = action.text;
    button.addEventListener("click", () => {
      if (busy) return;
      if (composer.hidden) openComposer();
      setInputValue(action.text);
      resetPromptHistoryNavigation();
      composer.requestSubmit();
    });
    quickActions.append(button);
  }
  updateComposerState();
}

function renderReply(text) {
  renderMarkdown(reply, text);
}

function setAssistantReply(text, options = {}) {
  const source = String(text ?? "");
  renderReply(source);
  if (options.remember !== false && source.trim()) {
    lastAssistantText = source;
  }
}

async function copyText(text) {
  const source = String(text ?? "");
  if (!source.trim()) return false;
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(source);
    return true;
  }
  const area = document.createElement("textarea");
  area.value = source;
  area.style.position = "fixed";
  area.style.left = "-9999px";
  document.body.append(area);
  area.select();
  let ok = false;
  try { ok = document.execCommand("copy"); }
  finally { area.remove(); }
  return ok;
}

function renderAlarmReply(text) {
  setAssistantReply(text);
  const actions = document.createElement("div");
  actions.className = "alarm-actions";
  const awake = document.createElement("button");
  awake.type = "button";
  awake.className = "alarm-awake";
  awake.textContent = "おきたよ！";
  awake.addEventListener("click", () => clearAlarmSnooze(true));
  actions.append(awake);
  reply.append(actions);
}

function addHistory(role, text) {
  const content = String(text ?? "").trim();
  if (!content) return;
  conversationHistory.push({ role, content });
  if (role === "user") rememberPrompt(content);
  if (role === "assistant") lastAssistantText = content;
  if (conversationHistory.length > 80) conversationHistory.shift();
  renderHistory();
}

function hydrateHistory(items) {
  conversationHistory.length = 0;
  promptHistory.length = 0;
  promptHistoryIndex = null;
  if (!Array.isArray(items)) {
    renderHistory();
    return;
  }
  for (const item of items) {
    const role = item?.role === "user" ? "user" : item?.role === "assistant" ? "assistant" : "";
    const content = String(item?.content ?? "").trim();
    if (role && content) {
      conversationHistory.push({ role, content });
      if (role === "user") rememberPrompt(content);
    }
  }
  if (conversationHistory.length > 80) {
    conversationHistory.splice(0, conversationHistory.length - 80);
  }
  lastAssistantText = [...conversationHistory].reverse().find(item => item.role === "assistant")?.content || "";
  renderHistory();
}

function setTemporaryStatus(message, ms = 1400) {
  status.textContent = message;
  setTimeout(() => {
    if (status.textContent === message) status.textContent = "";
  }, ms);
}

async function copyHistoryItem(text) {
  try {
    const ok = await copyText(text);
    setTemporaryStatus(ok ? "履歴をコピーしました" : "コピーできませんでした");
  } catch (error) {
    logApp("warn", "履歴コピーに失敗しました", errorDetail(error));
    setTemporaryStatus("コピーできませんでした");
  }
}

function reuseHistoryItem(text) {
  if (busy) return;
  if (composer.hidden) openComposer();
  setInputValue(text);
  resetPromptHistoryNavigation();
  input.focus();
  setTemporaryStatus("入力欄へ戻しました");
}

function renderHistory() {
  if (!historyLog) return;
  historyLog.replaceChildren();
  if (!conversationHistory.length) {
    historyLog.append(document.createElement("div"));
    return;
  }
  const query = historyQuery.trim().toLowerCase();
  const items = query
    ? conversationHistory.filter(item => item.content.toLowerCase().includes(query))
    : conversationHistory;
  if (!items.length) {
    const empty = document.createElement("div");
    empty.className = "history-empty";
    empty.textContent = "一致する履歴はありません。";
    historyLog.append(empty);
    return;
  }
  for (const item of items) {
    const row = document.createElement("div");
    row.className = `history-item ${item.role}`;
    const meta = document.createElement("div");
    meta.className = "history-meta";
    const label = document.createElement("div");
    label.className = "history-role";
    label.textContent = item.role === "user" ? "あなた" : (characterManifest?.name || "むぎ");
    const actions = document.createElement("div");
    actions.className = "history-actions";
    const copy = document.createElement("button");
    copy.type = "button";
    copy.textContent = "コピー";
    copy.addEventListener("click", () => copyHistoryItem(item.content));
    actions.append(copy);
    if (item.role === "user") {
      const reuse = document.createElement("button");
      reuse.type = "button";
      reuse.textContent = "再利用";
      reuse.addEventListener("click", () => reuseHistoryItem(item.content));
      actions.append(reuse);
    }
    meta.append(label, actions);
    const body = document.createElement("div");
    body.className = "history-body";
    renderMarkdown(body, item.content);
    row.append(meta, body);
    historyLog.append(row);
  }
  historyLog.scrollTop = historyLog.scrollHeight;
}

function scheduleAppQuit() {
  setTimeout(() => {
    window.picoagent.quit?.().catch(error => {
      logApp("error", "アプリ終了に失敗しました", errorDetail(error));
    });
  }, 900);
}

let speakingToken = 0;
let currentVoice = null;
let voiceSession = 0; // 文分割読み上げ（パイプライン）の世代。変わると進行中の読み上げを打ち切る。
function setVoiceCancelVisible(visible) {
  if (cancelVoiceButton) cancelVoiceButton.hidden = !visible;
}
function finishVoice(toIdle = true) {
  stopLipSync();
  setVoiceCancelVisible(false);
  if (!currentVoice) return;
  const voice = currentVoice;
  currentVoice = null;
  URL.revokeObjectURL(voice.src);
  if (toIdle && animationName === "talking") setAnimation("idle");
  setTimeout(flushPendingNotice, 0);
}
function stopVoice(toIdle = true) {
  voiceSession++; // 進行中のパイプライン読み上げ（合成待ち・連続再生）を打ち切る。
  stopLipSync();
  setVoiceCancelVisible(false);
  if (!currentVoice) return;
  currentVoice.pause();
  finishVoice(toIdle);
}
// 合成のみ行い、再生はしない（起動時に描画と同時再生するための事前合成）。失敗・無効・空は null。
async function prepareSpeech(text) {
  try {
    const result = await window.picoagent.speak(text);
    if (!result?.enabled || result.skipped || !result.audio?.length) return null;
    return result;
  } catch (error) {
    logApp("warn", "音声の事前合成に失敗しました", errorDetail(error));
    return null;
  }
}
// 事前合成済みの音声を再生し、会話アニメを開始する。再生終了・失敗時に Object URL を解放する。
function playPreparedSpeech(result) {
  stopVoice(false);
  setVoiceCancelVisible(true);
  const bytes = new Uint8Array(result.audio);
  const url = URL.createObjectURL(new Blob([bytes], { type: result.mime || "audio/wav" }));
  currentVoice = new Audio(url);
  currentVoice.volume = voiceVolume;
  currentVoice.addEventListener("ended", () => {
    finishVoice(true);
    if (!activeAlarmSnooze) status.textContent = "";
  }, { once: true });
  currentVoice.addEventListener("error", () => {
    const audioError = currentVoice?.error?.message || "";
    finishVoice(true);
    logApp("error", "音声データの読み込みに失敗しました", audioError);
    status.textContent = activeAlarmSnooze ? status.textContent : "音声再生に失敗: 音声データを読み込めません";
  }, { once: true });
  currentVoice.play().catch(error => {
    finishVoice(true);
    logApp("error", "音声再生に失敗しました", errorDetail(error));
    if (!activeAlarmSnooze) {
      status.textContent = `音声再生に失敗: ${error?.message || error || "再生がブロックされました"}`;
    }
  });
  status.textContent = "読み上げ中…";
  setAnimation("talking", true);
  attachLipSync(currentVoice);
}
// 起動時の挨拶を組み立てる。キャラ固定挨拶と日次の時刻挨拶で「挨拶語」が二重にならないようまとめる。
// - 固定挨拶が時刻挨拶語（おはよう/こんにちは/こんばんは等）で始まる場合: 時刻に合った挨拶語を1回だけ使い、
//   名前紹介＋日付時刻を続ける（例: 「こんにちは。むぎだよ。今日もよろしくね。今日は6月30日、今は14:23だよ。」）。
// - 固定挨拶がキャラ独自の挨拶（おかえりなさい / ふ、ふん 等）の場合: 固定挨拶を活かし、日次からは挨拶語を外して日付時刻だけ足す。
function combineStartupGreeting(manifestGreeting, dailyReply) {
  const manifest = String(manifestGreeting ?? "").trim();
  const daily = String(dailyReply ?? "").trim();
  if (!manifest) return daily;
  if (!daily) return manifest;
  const TIME_GREETINGS = ["おはようございます", "おはよう", "こんにちは", "こんにちわ", "こんばんは", "夕方だね"];
  const stripLead = (text) => {
    for (const greeting of TIME_GREETINGS) {
      if (text.startsWith(greeting)) {
        return { word: greeting, rest: text.slice(greeting.length).replace(/^[、,。.\s]+/, "") };
      }
    }
    return { word: "", rest: text };
  };
  const m = stripLead(manifest);
  const d = stripLead(daily);
  if (m.word) {
    const lead = d.word || m.word;
    return `${lead}。${m.rest}${d.rest}`;
  }
  return `${manifest}${d.rest}`;
}
// 読み上げ用に本文を文単位のチャンクへ分割する。長文を小分けにして「先頭から順次再生」する
// ことで、全文合成を待たずに早く話し始められる（体感待ち時間を短縮）。
function sanitizeSpeechText(text) {
  let out = String(text ?? "").replace(/\r\n/g, "\n");
  out = out.split(/\n\s*参考[:：]/)[0];
  out = out.replace(/!?\[([^\]]*)\]\((?:https?:\/\/|mailto:)[^)]+\)/g, "$1");
  out = out.replace(/https?:\/\/[^\s、。）」』】]+/g, "");
  return out
    .split("\n")
    .map(line => line.trim())
    .filter(line => line && line.replace(/^[-*・\s:：]+$/, ""))
    .filter(line => !/^フィード[:：]/.test(line))
    .filter(line => !/^-\s+\d{4}-\d{2}-\d{2}/.test(line))
    .join("\n")
    .trim();
}

function splitSpeechChunks(text, maxLen = 90) {
  const norm = sanitizeSpeechText(text);
  if (!norm) return [];
  // 文末記号・改行の直後で区切る。短い断片は結合し、長すぎる文はそのまま1チャンクにする。
  const parts = norm.split(/(?<=[。．.!?！？\n])/);
  const chunks = [];
  let buf = "";
  for (const p of parts) {
    if (buf && (buf + p).length > maxLen) { chunks.push(buf.trim()); buf = p; }
    else buf += p;
  }
  if (buf.trim()) chunks.push(buf.trim());
  return chunks.filter(c => c.length > 0);
}
// 合成済みチャンクを1つ再生し、再生終了（または失敗・打ち切り）で解決する。
// チャンク間ではアニメ/ステータスを idle に戻さず、talking を維持する。
function playChunkAndWait(result, session) {
  return new Promise(resolve => {
    if (session !== voiceSession) { resolve(); return; }
    const bytes = new Uint8Array(result.audio);
    const url = URL.createObjectURL(new Blob([bytes], { type: result.mime || "audio/wav" }));
    const audio = new Audio(url);
    currentVoice = audio;
    audio.volume = voiceVolume;
    let settled = false;
    const cleanup = () => {
      if (settled) return;
      settled = true;
      URL.revokeObjectURL(url);
      if (currentVoice === audio) currentVoice = null;
      resolve();
    };
    audio.addEventListener("ended", cleanup, { once: true });
    audio.addEventListener("pause", cleanup, { once: true }); // stopVoice からの打ち切りで解決
    audio.addEventListener("error", () => {
      logApp("error", "音声データの読み込みに失敗しました", audio.error?.message || "");
      cleanup();
    }, { once: true });
    audio.play().catch(error => {
      logApp("error", "音声再生に失敗しました", errorDetail(error));
      cleanup();
    });
    status.textContent = "読み上げ中…";
    setAnimation("talking", true);
    attachLipSync(audio);
  });
}
async function speakReply(text) {
  const chunks = splitSpeechChunks(text);
  stopVoice(false);                // 直前の読み上げを止める（idle にはしない）
  const session = ++voiceSession;  // このセッションIDが変わったら以降を打ち切る
  if (chunks.length === 0) { status.textContent = ""; return; }
  setVoiceCancelVisible(true);
  let prepareTimer = null;
  try {
    const startedAt = Date.now();
    status.textContent = "読み上げ準備中…";
    prepareTimer = setInterval(() => {
      const seconds = Math.floor((Date.now() - startedAt) / 1000);
      if (status.textContent.startsWith("読み上げ準備中")) {
        status.textContent = `読み上げ準備中… ${seconds}秒`;
      }
    }, 1000);
    // パイプライン: 先頭チャンクを合成→再生しつつ、次チャンクを裏で先読み合成する。
    let nextSynth = prepareSpeech(chunks[0]);
    let played = false;
    for (let i = 0; i < chunks.length; i += 1) {
      const result = await nextSynth;
      if (session !== voiceSession) {
        if (prepareTimer) clearInterval(prepareTimer);
        setVoiceCancelVisible(false);
        return;
      }
      nextSynth = i + 1 < chunks.length ? prepareSpeech(chunks[i + 1]) : null;
      if (!result) continue;
      if (!played) { clearInterval(prepareTimer); prepareTimer = null; played = true; }
      await playChunkAndWait(result, session);
      if (session !== voiceSession) {
        if (prepareTimer) clearInterval(prepareTimer);
        setVoiceCancelVisible(false);
        return;
      }
    }
    if (prepareTimer) clearInterval(prepareTimer);
    setVoiceCancelVisible(false);
    stopLipSync();
    if (animationName === "talking") setAnimation("idle");
    if (status.textContent === "読み上げ中…" || status.textContent.startsWith("読み上げ準備中")) {
      status.textContent = "";
    }
  } catch (error) {
    if (prepareTimer) clearInterval(prepareTimer);
    setVoiceCancelVisible(false);
    if (session === voiceSession) stopVoice();
    logApp("error", "読み上げに失敗しました", errorDetail(error));
    status.textContent = `読み上げに失敗: ${error.message || error}`;
    setTimeout(() => { if (status.textContent.startsWith("読み上げに失敗")) status.textContent = ""; }, 12000);
  }
}
async function renderReplySpeaking(text) {
  const token = ++speakingToken;
  const source = String(text ?? "");
  const chars = [...source];
  reply.textContent = "";
  setAnimation("talking", true);
  const stepMs = chars.length > 180 ? 14 : 22;
  for (let index = 0; index < chars.length; index += 1) {
    if (token !== speakingToken) return;
    reply.textContent += chars[index];
    await new Promise(resolve => setTimeout(resolve, stepMs));
  }
  if (token !== speakingToken) return;
  setAssistantReply(source);
  if (!currentVoice) setAnimation("idle");
}

function showStartupReadyMessage(options = {}) {
  if (busy || !composer.hidden) return;
  const text = `${bootCharacterName}の準備ができたよ。`;
  const current = String(reply.textContent || "").trim();
  if (current && current !== "こんにちは。クリックして話しかけてね。") return;
  renderReply(text);
  if (options.speak !== false) speakReply(text);
}

function canShowNotice() {
  return !busy && composer.hidden && !currentVoice && !bubble.classList.contains("fullscreen");
}

function queueNotice(line) {
  const text = String(line ?? "").trim();
  if (!text) return;
  pendingNotices.push(text);
  if (pendingNotices.length > 6) pendingNotices.shift();
}

async function flushPendingNotice() {
  if (!pendingNotices.length || !canShowNotice()) return;
  const line = pendingNotices.shift();
  await showAssistantNotice(line, { enqueue: false });
}

bubble.addEventListener("click", event => {
  if (event.target.closest("button,a,textarea,.reply-actions")) return;
  // テキスト選択中（コピー操作）はフキダシを入力欄へ切り替えない。
  if (String(window.getSelection?.() ?? "").trim()) return;
  openComposer();
});
character.addEventListener("click", event => {
  // クリックのたびに小さくはねる（連打時はアニメーションを巻き戻して再生）。
  character.classList.remove("boop");
  void character.offsetWidth;
  character.classList.add("boop");
  // Live2D はタップモーションで反応する（boop の CSS は img にしか効かない）。
  if (live2dActive && !currentVoice) playLive2DMotion("talking");
  if (!currentVoice) return;
  event.stopPropagation();
  stopVoice();
  status.textContent = "";
});
character.addEventListener("animationend", event => {
  if (event.animationName === "pet-boop") character.classList.remove("boop");
});
character.addEventListener("dblclick", toggleBubbleFullscreen);
// フキダシ自体をダブルクリックしても、画面最大化のように大きく広げる／戻す。
bubble.addEventListener("dblclick", event => {
  if (event.target.closest("button,a,textarea,.reply-actions,.bubble-resize")) return;
  toggleBubbleFullscreen();
});
document.addEventListener("contextmenu", event => {
  if (event.target instanceof Element && event.target.closest("textarea,a,.reply,.history-log")) return;
  event.preventDefault();
  window.picoagent.menu();
});

copyReplyButton?.addEventListener("click", async event => {
  event.preventDefault();
  event.stopPropagation();
  try {
    const ok = await copyText(lastAssistantText || reply.textContent);
    status.textContent = ok ? "コピーしました" : "コピーできませんでした";
    setTimeout(() => { if (status.textContent === "コピーしました") status.textContent = ""; }, 1600);
  } catch (error) {
    logApp("warn", "回答コピーに失敗しました", errorDetail(error));
    status.textContent = "コピーできませんでした";
  }
});

replayReplyButton?.addEventListener("click", event => {
  event.preventDefault();
  event.stopPropagation();
  const text = lastAssistantText || reply.textContent;
  if (!text.trim() || busy) return;
  speakReply(text);
});

stopVoiceButton?.addEventListener("click", event => {
  event.preventDefault();
  event.stopPropagation();
  stopVoice();
  if (!activeAlarmSnooze) status.textContent = "";
});

cancelVoiceButton?.addEventListener("click", event => {
  event.preventDefault();
  event.stopPropagation();
  stopVoice();
  if (!activeAlarmSnooze) status.textContent = "";
});

toggleFullscreenButton?.addEventListener("click", event => {
  event.preventDefault();
  event.stopPropagation();
  toggleBubbleFullscreen();
});

// 左上のつまみをドラッグしてフキダシをリサイズし、確定値を設定へ保存する。
let resizeStart = null;
let resizeWindowSize = { width: 0, height: 0 };
function clampNumber(value, min, max) {
  return Math.max(min, Math.min(max, value));
}
function expandWindowForBubble(left, top, width, height, leftDelta = 0, topDelta = 0) {
  const margin = 24;
  // 非同期リサイズの反映待ちによる連鎖的な計算ずれで際限なく膨張しないよう、
  // 画面サイズを最終的な上限にする（多重防御。実際の上限は pointermove 側の距離クランプ）。
  const maxWidth = Math.max(window.innerWidth, screen.availWidth || Infinity);
  const maxHeight = Math.max(window.innerHeight, screen.availHeight || Infinity);
  const nextWidth = Math.min(maxWidth, Math.ceil(Math.max(window.innerWidth + leftDelta, left + width + margin)));
  const nextHeight = Math.min(maxHeight, Math.ceil(Math.max(window.innerHeight + topDelta, top + height + margin)));
  if (nextWidth <= window.innerWidth && nextHeight <= window.innerHeight && leftDelta <= 0 && topDelta <= 0) return;
  if (leftDelta <= 0 && topDelta <= 0 && nextWidth <= resizeWindowSize.width && nextHeight <= resizeWindowSize.height) return;
  resizeWindowSize = { width: nextWidth, height: nextHeight };
  const resize = leftDelta > 0 || topDelta > 0
    ? window.picoagent.moveAndResizeWindow(leftDelta, topDelta, nextWidth, nextHeight)
    : window.picoagent.setWindowSize(nextWidth, nextHeight);
  resize.catch(error => {
    logApp("warn", "フキダシ拡大に合わせたウィンドウサイズ変更に失敗しました", errorDetail(error));
  });
}
// フキダシの右側にキャラクター分の幅（と下側の高さ）を確保するよう、必要ならウィンドウを左上へ拡大する。
// 右下は固定したまま広げるので、右端固定のキャラクターと重ならない。
async function ensureWindowFitsBubble(width, height, left, top) {
  if (bubble.classList.contains("fullscreen")) return;
  const gap = 10;
  const margin = 12;
  const charRight = 6; // .character の right:6px
  const charW = character.offsetWidth || 265;
  const charH = character.offsetHeight || 315;
  const targetW = Math.ceil(left + width + gap + charW + charRight);
  const targetH = Math.ceil(Math.max(top + height + margin, charH + 2));
  const curW = Math.round(window.innerWidth);
  const curH = Math.round(window.innerHeight);
  const leftDelta = Math.max(0, targetW - curW);
  const topDelta = Math.max(0, targetH - curH);
  if (leftDelta === 0 && topDelta === 0) return;
  try {
    await window.picoagent.moveAndResizeWindow(leftDelta, topDelta, curW + leftDelta, curH + topDelta);
  } catch (error) {
    logApp("warn", "フキダシに合わせたウィンドウ拡大に失敗しました", errorDetail(error));
  }
}
function resetFullscreenBubbleLayoutVars() {
  bubble.style.removeProperty("--fullscreen-bubble-top");
  bubble.style.removeProperty("--fullscreen-bubble-left");
  bubble.style.removeProperty("--fullscreen-bubble-right");
  bubble.style.removeProperty("--fullscreen-bubble-bottom");
}

function applyFullscreenBubbleLayout() {
  if (!bubble.classList.contains("fullscreen")) {
    resetFullscreenBubbleLayoutVars();
    return;
  }
  const margin = 14;
  const viewportWidth = document.documentElement.clientWidth;
  const rect = character.getBoundingClientRect();
  const minBubbleWidth = 180;
  const maxRight = Math.max(margin, viewportWidth - margin - minBubbleWidth);
  const characterPane = Math.max(291, viewportWidth - rect.left + margin);
  bubble.style.setProperty("--fullscreen-bubble-top", `${margin}px`);
  bubble.style.setProperty("--fullscreen-bubble-left", `${margin}px`);
  bubble.style.setProperty("--fullscreen-bubble-right", `${Math.min(maxRight, characterPane)}px`);
  bubble.style.setProperty("--fullscreen-bubble-bottom", `${margin}px`);
}

function refreshBubblePlacement() {
  requestAnimationFrame(() => {
    if (bubble.classList.contains("fullscreen")) {
      applyFullscreenBubbleLayout();
      return;
    }
    resetFullscreenBubbleLayoutVars();
    repositionBubbleToAvoidCharacter();
  });
}

function placeBubbleAwayFromCharacter(width, height, preferredLeft = bubble.offsetLeft, preferredTop = bubble.offsetTop) {
  if (bubble.classList.contains("fullscreen")) return;
  const margin = 12;
  const gap = 10;
  const viewportWidth = document.documentElement.clientWidth;
  const viewportHeight = document.documentElement.clientHeight;
  const charLeftEdge = character.offsetLeft - gap;
  const charTopEdge = character.offsetTop - gap;
  const charRightEdge = character.offsetLeft + character.offsetWidth + gap;
  const charBottomEdge = character.offsetTop + character.offsetHeight + gap;
  const maxLeft = Math.max(margin, viewportWidth - width - margin);
  const maxTop = Math.max(margin, viewportHeight - height - margin);
  const baseLeft = clampNumber(preferredLeft, margin, maxLeft);
  const baseTop = clampNumber(preferredTop, margin, maxTop);

  const xCandidates = [
    baseLeft,
    charLeftEdge - width,
    charRightEdge,
    margin,
    maxLeft,
  ];
  const yCandidates = [
    baseTop,
    charTopEdge - height,
    charBottomEdge,
    margin,
    maxTop,
  ];

  const candidates = [];
  for (const rawLeft of xCandidates) {
    for (const rawTop of yCandidates) {
      const left = clampNumber(rawLeft, margin, maxLeft);
      const top = clampNumber(rawTop, margin, maxTop);
      if (candidates.some(item => item.left === left && item.top === top)) continue;
      const overlap = overlapAreaWithCharacter(left, top, width, height, gap);
      const distance = Math.hypot(left - baseLeft, top - baseTop);
      candidates.push({ left, top, overlap, distance });
    }
  }

  candidates.sort((a, b) => a.overlap - b.overlap || a.distance - b.distance);
  const best = candidates[0] || { left: baseLeft, top: baseTop };
  // サイズは縮めず（自動縮小は行わない）、重なりが最小になる位置へ移動するだけにする。
  bubble.style.left = `${Math.round(best.left)}px`;
  bubble.style.top = `${Math.round(best.top)}px`;
}

function overlapAreaWithCharacter(left, top, width, height, gap = 10) {
  const charLeft = character.offsetLeft - gap;
  const charTop = character.offsetTop - gap;
  const charRight = character.offsetLeft + character.offsetWidth + gap;
  const charBottom = character.offsetTop + character.offsetHeight + gap;
  const overlapWidth = Math.max(0, Math.min(left + width, charRight) - Math.max(left, charLeft));
  const overlapHeight = Math.max(0, Math.min(top + height, charBottom) - Math.max(top, charTop));
  return overlapWidth * overlapHeight;
}
function repositionBubbleToAvoidCharacter() {
  if (bubble.classList.contains("fullscreen")) {
    applyFullscreenBubbleLayout();
    return;
  }
  const rect = bubble.getBoundingClientRect();
  placeBubbleAwayFromCharacter(rect.width, rect.height);
}
bubbleResize.addEventListener("pointerdown", event => {
  event.preventDefault();
  event.stopPropagation();
  const rect = bubble.getBoundingClientRect();
  resizeWindowSize = { width: window.innerWidth, height: window.innerHeight };
  resizeStart = {
    x: event.clientX,
    y: event.clientY,
    // screenX/screenY はウィンドウ移動の影響を受けない絶対座標。
    // moveAndResizeWindow の非同期反映待ちで clientX 系の計算がずれても、
    // 実際にドラッグした distance 以上ウィンドウが膨張しないようにする安全弁に使う。
    screenX: event.screenX,
    screenY: event.screenY,
    width: rect.width,
    height: rect.height,
    left: bubble.offsetLeft,
    top: bubble.offsetTop,
    leftDelta: 0,
    topDelta: 0,
  };
  bubbleResize.setPointerCapture(event.pointerId);
});
bubbleResize.addEventListener("pointermove", event => {
  if (!resizeStart) return;
  const margin = 12;
  const originalPointerX = event.clientX - resizeStart.leftDelta;
  const originalPointerY = event.clientY - resizeStart.topDelta;
  // ウィンドウの非同期な移動/リサイズ完了前に次の pointermove が来ると、
  // clientX/clientY ベースの計算だけでは補正がずれて際限なく膨張しうる。
  // 実際にドラッグした画面座標の移動量を上限として必ずクランプする。
  const maxLeftDelta = Math.max(0, resizeStart.screenX - event.screenX);
  const maxTopDelta = Math.max(0, resizeStart.screenY - event.screenY);
  const nextLeftDelta = Math.min(Math.max(resizeStart.leftDelta, margin - originalPointerX, 0), maxLeftDelta);
  const nextTopDelta = Math.min(Math.max(resizeStart.topDelta, margin - originalPointerY, 0), maxTopDelta);
  const right = resizeStart.left + resizeStart.width + nextLeftDelta;
  const bottom = resizeStart.top + resizeStart.height + nextTopDelta;
  const pointerLeft = Math.min(originalPointerX + nextLeftDelta, right - 180);
  const pointerTop = Math.min(originalPointerY + nextTopDelta, bottom - 92);
  const left = Math.max(margin, pointerLeft);
  const top = Math.max(margin, pointerTop);
  const width = right - left;
  const height = bottom - top;
  const moveLeft = nextLeftDelta - resizeStart.leftDelta;
  const moveTop = nextTopDelta - resizeStart.topDelta;
  resizeStart.leftDelta = nextLeftDelta;
  resizeStart.topDelta = nextTopDelta;
  bubble.style.width = `${Math.round(width)}px`;
  bubble.style.height = `${Math.round(height)}px`;
  bubble.style.left = `${Math.round(left)}px`;
  bubble.style.top = `${Math.round(top)}px`;
  expandWindowForBubble(left, top, width, height, moveLeft, moveTop);
});
async function finishBubbleResize(event) {
  if (!resizeStart) return;
  resizeStart = null;
  if (bubbleResize.hasPointerCapture(event.pointerId)) bubbleResize.releasePointerCapture(event.pointerId);
  const rect = bubble.getBoundingClientRect();
  const w = Math.round(rect.width);
  const h = Math.round(rect.height);
  const left = Math.round(bubble.offsetLeft);
  const top = Math.round(bubble.offsetTop);
  try {
    // 右側にキャラ分の幅を確保してウィンドウを拡大し、フキダシとキャラが重ならないようにする。
    await ensureWindowFitsBubble(w, h, left, top);
    await window.picoagent.setBubbleLayout(w, h, left, top);
  } catch (error) {
    status.textContent = `フキダシ位置を保存できませんでした: ${error}`;
  }
}
bubbleResize.addEventListener("pointerup", finishBubbleResize);
bubbleResize.addEventListener("pointercancel", finishBubbleResize);

// ── 画像添付（ペースト・ドロップ）────────────────────────────────────────────

const MAX_ATTACHED_IMAGES = 4;
const MAX_ATTACHED_FILES = 4;

function renderAttachmentPreview() {
  const hasAny = attachedImages.length > 0 || attachedFiles.length > 0;
  imagePreview.hidden = !hasAny;
  imagePreview.replaceChildren();
  // ファイルチップ（拡張子アイコン付き）
  attachedFiles.forEach((file, idx) => {
    const chip = document.createElement("div");
    chip.className = "file-chip";
    chip.title = file.name;
    const icon = document.createElement("span");
    icon.className = "file-chip-icon";
    icon.textContent = fileIcon(file.name);
    const label = document.createElement("span");
    label.className = "file-chip-name";
    label.textContent = file.name.length > 22 ? file.name.slice(0, 20) + "…" : file.name;
    const removeBtn = document.createElement("button");
    removeBtn.type = "button";
    removeBtn.className = "file-remove";
    removeBtn.textContent = "×";
    removeBtn.setAttribute("aria-label", `${file.name} を削除`);
    removeBtn.addEventListener("click", () => {
      attachedFiles.splice(idx, 1);
      renderAttachmentPreview();
      updateComposerState();
    });
    chip.append(icon, label, removeBtn);
    imagePreview.append(chip);
  });
  // 画像サムネイル
  attachedImages.forEach((dataUrl, idx) => {
    const wrap = document.createElement("div");
    wrap.className = "image-thumb-wrap";
    const img = document.createElement("img");
    img.src = dataUrl;
    img.alt = `添付画像 ${idx + 1}`;
    const removeBtn = document.createElement("button");
    removeBtn.type = "button";
    removeBtn.className = "image-remove";
    removeBtn.textContent = "×";
    removeBtn.setAttribute("aria-label", `添付画像 ${idx + 1} を削除`);
    removeBtn.addEventListener("click", () => {
      attachedImages.splice(idx, 1);
      renderAttachmentPreview();
      updateComposerState();
    });
    wrap.append(img, removeBtn);
    imagePreview.append(wrap);
  });
}

function fileIcon(name) {
  const ext = name.split(".").pop().toLowerCase();
  const map = { xlsx: "📊", xls: "📊", xlsm: "📊", csv: "📋", tsv: "📋",
    docx: "📝", doc: "📝", docm: "📝", pptx: "📄", pptm: "📄", ppt: "📄",
    pdf: "📕", json: "📦", toml: "📦", yaml: "🗂", yml: "🗂",
    txt: "📃", md: "📃", log: "📃" };
  return map[ext] || "📎";
}

function addImageFile(file) {
  if (!file || !file.type.startsWith("image/")) return;
  if (attachedImages.length >= MAX_ATTACHED_IMAGES) { showToast(`画像は最大 ${MAX_ATTACHED_IMAGES} 枚まで`, "warn"); return; }
  const reader = new FileReader();
  reader.onload = (e) => {
    attachedImages.push(e.target.result);
    renderAttachmentPreview();
    updateComposerState();
    showToast("画像を添付しました", "success");
  };
  reader.readAsDataURL(file);
}

function addDocumentFile(file) {
  if (!file) return;
  if (attachedFiles.length >= MAX_ATTACHED_FILES) { showToast(`ファイルは最大 ${MAX_ATTACHED_FILES} 件まで`, "warn"); return; }
  const reader = new FileReader();
  reader.onload = (e) => {
    attachedFiles.push({ name: file.name, dataUrl: e.target.result });
    renderAttachmentPreview();
    updateComposerState();
    showToast(`${file.name.length > 20 ? file.name.slice(0, 18) + "…" : file.name} を添付`, "success");
  };
  reader.readAsDataURL(file);
}

// Ctrl+V で画像をペーストしたとき添付する。
input.addEventListener("paste", (event) => {
  const items = event.clipboardData?.items;
  if (!items) return;
  for (const item of items) {
    if (!item.type.startsWith("image/")) continue;
    event.preventDefault();
    addImageFile(item.getAsFile());
  }
});

// テキストエリア・プレビューエリアへのドラッグ&ドロップ。
composer.addEventListener("dragover", (e) => {
  if ([...e.dataTransfer.items].some(i => i.kind === "file")) {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    composer.classList.add("drag-over");
  }
});
composer.addEventListener("dragleave", () => composer.classList.remove("drag-over"));
composer.addEventListener("drop", (e) => {
  composer.classList.remove("drag-over");
  if (![...e.dataTransfer.items].some(i => i.kind === "file")) return;
  e.preventDefault();
  for (const file of e.dataTransfer.files) {
    if (file.type.startsWith("image/")) addImageFile(file);
    else addDocumentFile(file);
  }
});

// 画面を見てほしい系の発話を検出する（スクリーンショット自動添付に使用）。
function isScreenshotQuery(text) {
  return /画面を見て|スクリーンショット|screenshot|画面の状態|画面を確認|今の画面|デスクトップを見て|画面キャプチャ/.test(text.toLowerCase());
}

composer.addEventListener("submit", async event => {
  event.preventDefault();
  const text = input.value.trim();
  if ((!text && attachedImages.length === 0 && attachedFiles.length === 0) || busy) return;
  const requestRevision = conversationRevision;
  clearAlarmSnooze(false);
  lastUserText = text; // エラーリトライ用に保持
  busy = true;
  ++speakingToken;
  stopVoice(false);
  updateComposerState();
  closeComposer();
  status.textContent = "";
  const pendingImages = [...attachedImages];
  const pendingFiles = [...attachedFiles];
  attachedImages = [];
  attachedFiles = [];
  renderAttachmentPreview();
  setInputValue("");
  resetPromptHistoryNavigation();
  status.textContent = "考え中…";
  reply.classList.remove("reply-error"); // 前回のエラー表示をリセット
  status.classList.add("busy"); // 処理中スピナーを表示する
  setAnimation("thinking");
  // 処理中は停止ボタンを出す（長いエージェント実行を途中でやめられるように）。
  if (cancelSendButton) { cancelSendButton.hidden = false; cancelSendButton.disabled = false; }
  reply.textContent = text;
  addHistory("user", text);

  // スーパーエージェントモードの進捗（計画→実行）をステータス行に表示する。
  try {
    if (window.picoagent.listen) {
      if (unlistenStatus) { unlistenStatus(); unlistenStatus = null; } // 多重購読防止
      unlistenStatus = await window.picoagent.listen("pet:agent_status", (progress) => {
        if (busy && typeof progress === "string" && progress) {
          // 改行は1行表示に収める（計画本文など複数行が来てもステータス行が崩れないように）。
          status.textContent = progress.replace(/\s*\n+\s*/g, " / ");
        }
      });
    }
  } catch (error) {
    logApp("warn", "進捗購読の設定に失敗しました", errorDetail(error));
  }

  try {
    // ── ファイル内容をテキストへ変換してコンテキストに付加 ──────────────────
    let fileContext = "";
    if (pendingFiles.length > 0 && window.picoagent.readFileContent) {
      status.textContent = "ファイルを読み込み中…";
      for (const f of pendingFiles) {
        try {
          const extracted = await window.picoagent.readFileContent(f.name, f.dataUrl);
          fileContext += `[添付ファイル: ${f.name}]\n${extracted}\n\n`;
        } catch (err) {
          logApp("warn", `ファイル読み込み失敗: ${f.name}`, String(err));
          fileContext += `[添付ファイル: ${f.name}] (読み込みに失敗しました)\n\n`;
        }
      }
    }
    const fullText = fileContext
      ? `${fileContext}${text || "このファイルについて教えて"}`
      : text;

    // ── 画像送信（スクリーンショット自動添付 + 貼り付け画像）─────────────────
    let result;
    const images = [...pendingImages];
    if (isScreenshotQuery(text) && window.picoagent.captureScreen) {
      try {
        status.textContent = "画面を撮影中…";
        const shot = await window.picoagent.captureScreen();
        if (shot?.dataUrl) images.push(shot.dataUrl);
      } catch (shotErr) {
        logApp("warn", "スクリーンショット取得に失敗しました。", String(shotErr));
      }
    }
    if (images.length > 0 && window.picoagent.sendImage) {
      result = await window.picoagent.sendImage(fullText, images);
    }
    if (!result) result = await window.picoagent.send(fullText);
    if (result?.stale || requestRevision !== conversationRevision) {
      status.textContent = "";
      return;
    }
    status.textContent = result.error ? "接続を確認してね" : "";
    if (result.error) {
      logApp("error", "AI応答がエラーを返しました", result.reply || "");
      ++speakingToken;
      stopVoice();
      renderReply(result.reply);
      reply.classList.add("reply-error");
      addRetryButton();
      addHistory("assistant", result.reply);
      setAnimation("sad");
      setTimeout(() => setAnimation("idle"), 2200);
    } else {
      // 返信がユーザー入力のオウム返しのときは読み上げない（入力は声に出さない）。
      if (String(result.reply ?? "").trim() !== text) speakReply(result.reply);
      await renderReplySpeaking(result.reply);
      addHistory("assistant", result.reply);
      if (result.quitApp) scheduleAppQuit();
    }
  } catch (error) {
    logApp("error", "AI応答の取得に失敗しました", errorDetail(error));
    ++speakingToken;
    stopVoice();
    const message = `応答に失敗しました: ${error?.message || error}`;
    renderReply(message);
    reply.classList.add("reply-error");
    addRetryButton();
    addHistory("assistant", message);
    status.textContent = "接続を確認してね";
    setAnimation("sad");
    setTimeout(() => setAnimation("idle"), 2200);
  } finally {
    status.classList.remove("busy");
    if (cancelSendButton) cancelSendButton.hidden = true;
    // 進捗リスナーを解除する。
    if (unlistenStatus) { unlistenStatus(); unlistenStatus = null; }
    busy = false;
    updateComposerState();
    scheduleChatter(); // 会話後はアイドル計測をやり直す。
    setTimeout(flushPendingNotice, 0);
  }
});
// 停止ボタン: バックエンドへ中断を要求する。エージェントは次のステップ境界で止まる。
cancelSendButton?.addEventListener("click", async () => {
  if (!busy) return;
  cancelSendButton.disabled = true;
  status.textContent = "止めるね…";
  try { await window.picoagent.cancel(); }
  catch (error) { logApp("warn", "中断要求の送信に失敗しました", errorDetail(error)); }
});
input.addEventListener("input", () => {
  autosizeInput();
  updateComposerState();
  promptHistoryIndex = null;
  promptDraft = input.value;
  const len = input.value.length;
  if (!busy) status.textContent = len > 100 ? `${len}文字` : "";
});
input.addEventListener("keydown", event => {
  if (event.key === "Escape") {
    event.stopPropagation();
    closeComposer();
  }
  if (event.key === "ArrowUp" && input.selectionStart === 0 && input.selectionEnd === 0) {
    if (recallPromptHistory(-1)) event.preventDefault();
  }
  if (event.key === "ArrowDown" && input.selectionStart === input.value.length && input.selectionEnd === input.value.length) {
    if (recallPromptHistory(1)) event.preventDefault();
  }
  if (event.key === "Enter" && (!event.shiftKey || event.ctrlKey || event.metaKey)) {
    event.preventDefault();
    composer.requestSubmit();
  }
  if (event.key === "ArrowUp" && recallInputHistory(-1)) event.preventDefault();
  else if (event.key === "ArrowDown" && inputHistoryPos !== -1 && recallInputHistory(1)) event.preventDefault();
});
historySearch?.addEventListener("input", () => {
  historyQuery = historySearch.value;
  renderHistory();
});
document.addEventListener("keydown", event => {
  if (event.key !== "Escape") return;
  if (!composer.hidden) {
    closeComposer();
    return;
  }
  exitBubbleFullscreen();
});
window.picoagent.on("pet:model", model => { status.textContent = model; setTimeout(() => status.textContent = "", 1800); });
window.picoagent.on("pet:notice", pushNotice);
window.picoagent.on("pet:reset", () => {
  conversationRevision += 1;
  conversationHistory.length = 0;
  promptHistory.length = 0;
  promptHistoryIndex = null;
  promptDraft = "";
  historyQuery = "";
  if (historySearch) historySearch.value = "";
  renderHistory();
  stopVoice();
  renderReply("新しい話をしよう。");
  closeComposer();
  scheduleChatter();
});
// 感情状態が更新されたらキャラクターのアニメーション・スタイルへ反映する。
window.picoagent.on("pet:emotion", (emotion) => {
  if (!emotion || !emotion.mood) return;
  character.dataset.emotion = emotion.mood;
  character.dataset.energyLevel = emotion.energy > 0.6 ? "high" : emotion.energy < 0.4 ? "low" : "mid";
});
renderQuickActions();

// 同梱エンジン（local）の準備状況を表示する。
function applyEngineStatus(s) {
  if (!s || !s.phase) return;
  // モデル DL・起動中の表示は起動シーケンス（ローディング画面）の間だけ出し、
  // 通常時はフキダシに「準備中…」を出さない（重い反映は再起動側で行う方針のため）。
  const booting = petApp.classList.contains("is-booting");
  if (s.phase === "download") {
    if (booting) setStatus(`ローカルモデルを取得中… ${s.percent ?? 0}%`);
    online.dataset.up = false;
  } else if (s.phase === "starting") {
    if (booting) setStatus("ローカル推論エンジンを起動中…");
    online.dataset.up = false;
  } else if (s.phase === "ready") {
    online.dataset.up = true;
    if (petApp.classList.contains("is-booting")) {
      status.textContent = "";
      if (bootStatus) bootStatus.textContent = `${bootCharacterName}を起こしているよ…`;
    } else {
      showStartupReadyMessage();
    }
  } else if (s.phase === "error") {
    setStatus(`準備に失敗: ${s.message || "不明なエラー"}`);
    online.dataset.up = false;
  }
}
window.picoagent.on("engine:status", applyEngineStatus);
// 設定反映（再起動相当）の合図。webview を読み込み直して起動シーケンスを再実行する。
window.picoagent.on("pet:reload", () => window.location.reload());

// フキダシの文字サイズを適用する（小さくて見えない問題への対応。設定で変更可能）。
function applyFontPx(px) {
  const n = parseFloat(px);
  if (!Number.isFinite(n) || n <= 0) return;
  reply.style.fontSize = `${n}px`;
  // 入力欄・ステータスも比率を合わせて読みやすくする。
  input.style.fontSize = `${n}px`;
  status.style.fontSize = `${Math.max(9, Math.round(n * 0.72))}px`;
}
function applyVoiceVolume(value) {
  const n = parseFloat(value);
  if (!Number.isFinite(n)) return;
  voiceVolume = Math.min(1, Math.max(0, n / 100));
  if (currentVoice) currentVoice.volume = voiceVolume;
}
function applyTheme(value) {
  const theme = value === "light" || value === "dark" ? value : "system";
  if (theme === "system") {
    document.documentElement.removeAttribute("data-theme");
  } else {
    document.documentElement.dataset.theme = theme;
  }
}
function applyBubbleLayout(width, height, left, top) {
  const w = parseFloat(width);
  const h = parseFloat(height);
  const x = parseFloat(left);
  const y = parseFloat(top);
  const hasSize = Number.isFinite(w) && w >= 180 && Number.isFinite(h) && h >= 92;
  if (Number.isFinite(w) && w >= 180) bubble.style.width = `${w}px`;
  if (Number.isFinite(h) && h >= 92) bubble.style.height = `${h}px`;
  if (Number.isFinite(x) && x >= 0) bubble.style.left = `${x}px`;
  if (Number.isFinite(y) && y >= 0) bubble.style.top = `${y}px`;
  // 保存済みの大きいフキダシでも右のキャラと重ならないよう、必要ならウィンドウを拡大する。
  if (hasSize) {
    const bx = Number.isFinite(x) && x >= 0 ? x : bubble.offsetLeft;
    const by = Number.isFinite(y) && y >= 0 ? y : bubble.offsetTop;
    ensureWindowFitsBubble(w, h, bx, by);
  }
  refreshBubblePlacement();
}
window.addEventListener("resize", refreshBubblePlacement);
petSprite.addEventListener("load", refreshBubblePlacement);
window.picoagent.on("pet:ui", (ui) => {
  if (!ui) return;
  if (ui.theme != null) applyTheme(ui.theme);
  if (ui.fontPx != null) applyFontPx(ui.fontPx);
  if (ui.voiceVolume != null) applyVoiceVolume(ui.voiceVolume);
  applyBubbleLayout(ui.bubbleWidth, ui.bubbleHeight, ui.bubbleLeft, ui.bubbleTop);
  if (ui.characterId) loadCharacter(ui.characterId, ui.characterOverrides || {}).catch(error => {
    logApp("error", "キャラクター変更に失敗しました", errorDetail(error));
    status.textContent = `キャラクター変更に失敗: ${error.message}`;
  });
  applyChatterConfig(ui); // 設定変更を即反映。
});

noticeToggle?.addEventListener("click", event => {
  event.stopPropagation();
  if (!noticePanel) return;
  const open = noticePanel.hidden;
  noticePanel.hidden = !open;
  if (open) {
    unreadNoticeCount = 0;
    renderNoticePanel();
    updateNoticeBadge();
  }
});
noticeClear?.addEventListener("click", event => {
  event.stopPropagation();
  pendingNotices.length = 0;
  unreadNoticeCount = 0;
  renderNoticePanel();
  updateNoticeBadge();
});
bubble?.addEventListener("click", event => {
  if (!noticePanel || noticePanel.hidden) return;
  if (event.target instanceof Element && event.target.closest("#noticePanel,#noticeToggle")) return;
  noticePanel.hidden = true;
});
renderNoticePanel();
updateNoticeBadge();

// TTS 有効時は同梱 Aivis が応答するまで待ってからキャラを描画する（無効時は待たない）。
// 起動失敗で永久に出ないことを避けるため、上限（約120秒）を超えたら諦めて描画する。
async function waitForAivisReady() {
  let s;
  try { s = await window.picoagent.aivisStatus(); }
  catch (error) {
    logApp("warn", "Aivis 状態取得に失敗しました", errorDetail(error));
    return;
  }
  if (!s || !s.enabled || s.up) return;
  const startedAt = Date.now();
  setStatus("音声エンジンの起動を確認中…");
  for (let i = 0; i < 240; i += 1) {
    await new Promise(resolve => setTimeout(resolve, 500));
    try { s = await window.picoagent.aivisStatus(); }
    catch (error) {
      logApp("warn", "Aivis 起動待ち中の状態取得に失敗しました", errorDetail(error));
      break;
    }
    if (s && s.up) break;
    if (i % 2 === 1) {
      const seconds = Math.floor((Date.now() - startedAt) / 1000);
      setStatus(`音声エンジンの起動を確認中… ${seconds}秒`);
    }
  }
  if (status.textContent.startsWith("音声エンジンの起動を確認中")) setStatus("");
}

// 選択中キャラの AivisHub 音声モデルを、初回読み上げ前にローディング画面で導入する。
async function prepareSelectedVoiceModel() {
  let statusInfo;
  try { statusInfo = await window.picoagent.aivisStatus(); }
  catch (error) { logApp("warn", "音声モデル準備前の Aivis 状態取得に失敗しました", errorDetail(error)); }
  if (!statusInfo?.enabled) return null;
  const startedAt = Date.now();
  let timer = null;
  try {
    setStatus(`${bootCharacterName}の声を準備中…`);
    timer = setInterval(() => {
      const seconds = Math.floor((Date.now() - startedAt) / 1000);
      setStatus(`${bootCharacterName}の声を準備中… ${seconds}秒`);
    }, 1000);
    await window.picoagent.prepareVoiceModel();
    if (timer) clearInterval(timer);
    timer = null;
    setStatus("");
    return true;
  } catch (error) {
    if (timer) clearInterval(timer);
    logApp("error", "音声モデルの準備に失敗しました", errorDetail(error));
    setStatus(`音声モデル準備に失敗: ${error?.message || error}`);
    await new Promise(resolve => setTimeout(resolve, 2500));
    return false;
  }
}

// 同梱エンジン(local)のモデル DL・起動をローディング画面の間に終わらせる。
// 通常時にフキダシへ「モデルを準備中…」を出さないため、ここで ready/error まで待つ。
// DL は数 GB で時間がかかりうるので上限は長め（約30分）。error は即終了する。
async function waitForEngineReady() {
  for (let i = 0; i < 1800; i += 1) {
    let s;
    try { s = await window.picoagent.engineStatus(); }
    catch (error) {
      logApp("warn", "エンジン状態取得に失敗しました", errorDetail(error));
      return;
    }
    applyEngineStatus(s); // download/starting はローディング画面に進捗表示。
    if (!s || s.phase === "idle") setStatus("ローカルモデルと推論エンジンを確認中…");
    if (s && (s.phase === "ready" || s.phase === "error")) return;
    await new Promise(resolve => setTimeout(resolve, 1000));
  }
}

// 自発雑談の設定を反映し、タイマーを張り直す（無効なら止める）。
function clampSecs(value, fallback) {
  const n = parseFloat(value);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(3600, Math.max(30, n));
}
function applyChatterConfig(c) {
  if (!c) return;
  chatterCfg.enabled = Boolean(c.chatterEnabled);
  chatterCfg.minSecs = clampSecs(c.chatterMinSecs, 90);
  chatterCfg.maxSecs = Math.max(chatterCfg.minSecs, clampSecs(c.chatterMaxSecs, 240));
  scheduleChatter();
}
async function showAssistantNotice(line, options = {}) {
  if (!line) return;
  const requestRevision = conversationRevision;
  if (!canShowNotice()) {
    if (options.enqueue !== false) queueNotice(line);
    return;
  }
  const prepared = await prepareSpeech(line);
  if (requestRevision !== conversationRevision) return;
  if (!canShowNotice()) {
    if (options.enqueue !== false) queueNotice(line);
    return;
  }
  setAssistantReply(line);
  addHistory("assistant", line);
  if (prepared) playPreparedSpeech(prepared);
}
// 次の自発雑談を [min, max] 秒のランダム遅延で予約する。呼ぶたびにアイドル計測をやり直す。
function scheduleChatter() {
  if (chatterTimer) { clearTimeout(chatterTimer); chatterTimer = null; }
  if (!chatterCfg.enabled) return;
  const span = chatterCfg.maxSecs - chatterCfg.minSecs;
  const delay = (chatterCfg.minSecs + Math.random() * span) * 1000;
  chatterTimer = setTimeout(fireChatter, delay);
}
async function fireChatter() {
  chatterTimer = null;
  // 入力中・応答中・読み上げ中・全画面表示中は割り込まず、次の機会に回す。
  if (busy || !composer.hidden || currentVoice || bubble.classList.contains("fullscreen")) {
    scheduleChatter();
    return;
  }
  try {
    const res = await window.picoagent.idleChatter();
    if (res?.enabled === false) { chatterCfg.enabled = false; return; }
    const line = (res?.reply || "").trim();
    await showAssistantNotice(line);
  } catch (error) {
    logApp("warn", "自発雑談の取得に失敗しました", errorDetail(error));
  }
  finally { scheduleChatter(); }
}

async function checkDailyEvents() {
  let res;
  try { res = await window.picoagent.dailyEvents(); }
  catch (error) {
    logApp("warn", "日次イベント取得に失敗しました", errorDetail(error));
    return;
  }
  const events = Array.isArray(res?.events) ? res.events : [];
  if (!events.length) return;
  const line = (events[0]?.text || "時間だよ。").trim();
  await showAssistantNotice(line);
}

function startDailyPolling() {
  if (dailyTimer) clearInterval(dailyTimer);
  dailyTimer = setInterval(checkDailyEvents, ALARM_POLL_MS);
  checkDailyEvents();
}

// 目覚まし時計: 一定間隔で現在時刻と登録アラームを照合し、一致したら声でやさしく起こす。
function pad2(n) { return String(n).padStart(2, "0"); }
function weekdayCode(date) { return ["sun", "mon", "tue", "wed", "thu", "fri", "sat"][date.getDay()]; }
function alarmSnoozeMinutes(a) {
  const value = Number(a?.snooze_minutes ?? a?.snoozeMinutes ?? 5);
  if (!Number.isFinite(value)) return 5;
  return Math.min(60, Math.max(1, Math.round(value)));
}
function clearAlarmSnooze(stopCurrentVoice) {
  activeAlarmSnooze = null;
  reply.querySelector(".alarm-actions")?.remove();
  if (stopCurrentVoice) stopVoice();
  status.textContent = "";
}
function armAlarmSnooze(a) {
  const minutes = alarmSnoozeMinutes(a);
  activeAlarmSnooze = {
    alarm: { ...a, snooze_minutes: minutes },
    dueAt: Date.now() + minutes * 60 * 1000,
  };
  status.textContent = `「おきたよ！」を押すまで${minutes}分おきに起こします`;
}
async function checkAlarms() {
  const now = new Date();
  if (activeAlarmSnooze && Date.now() >= activeAlarmSnooze.dueAt) {
    const alarm = activeAlarmSnooze.alarm;
    await fireAlarm(alarm);
    return;
  }
  let res;
  try { res = await window.picoagent.alarms(); }
  catch (error) {
    logApp("warn", "目覚まし一覧取得に失敗しました", errorDetail(error));
    return;
  }
  const alarms = res?.alarms;
  if (!Array.isArray(alarms) || !alarms.length) return;
  const hhmm = `${pad2(now.getHours())}:${pad2(now.getMinutes())}`;
  const today = weekdayCode(now);
  const minuteKey = `${now.getFullYear()}-${pad2(now.getMonth() + 1)}-${pad2(now.getDate())} ${hhmm}`;
  for (const a of alarms) {
    if (!a || a.enabled === false || a.time !== hhmm) continue;
    const days = Array.isArray(a.days) ? a.days : [];
    if (days.length && !days.includes(today)) continue;
    const key = `${a.id}@${minuteKey}`;
    if (firedAlarms.has(key)) continue;
    firedAlarms.add(key);
    if (firedAlarms.size > 50) firedAlarms.clear();
    await fireAlarm(a);
    break; // 1ティックにつき1件だけ鳴らす。
  }
}
async function fireAlarm(a) {
  let line = "";
  try {
    const res = await window.picoagent.alarmWake(a.id);
    line = (res?.reply || "").trim();
  } catch (error) {
    logApp("warn", "目覚まし起こし文の生成に失敗しました", errorDetail(error));
  }
  if (!line) line = "おはよう…そろそろ起きる時間だよ。ゆっくりでいいからね。";
  // 起こすときは読み上げ中でも割り込む。音声合成→表示→即時再生（挨拶と同じ同期再生）。
  const prepared = await prepareSpeech(line);
  renderAlarmReply(line);
  addHistory("assistant", line);
  if (prepared) playPreparedSpeech(prepared);
  else setAnimation("talking", true);
  armAlarmSnooze(a);
}
function startAlarmPolling() {
  if (alarmTimer) clearInterval(alarmTimer);
  alarmTimer = setInterval(checkAlarms, ALARM_POLL_MS);
  checkAlarms();
}

(async () => {
  startBootLogTicker();
  setStatus("設定と履歴を読み込み中…");
  const info = await window.picoagent.init();
  online.dataset.up = info.up;
  if (info.theme != null) applyTheme(info.theme);
  if (info.fontPx != null) applyFontPx(info.fontPx);
  if (info.voiceVolume != null) applyVoiceVolume(info.voiceVolume);
  applyBubbleLayout(info.bubbleWidth, info.bubbleHeight, info.bubbleLeft, info.bubbleTop);
  try {
    setStatus("起動画面のキャラクター画像を読み込み中…");
    await prepareBootCharacter(info.characterId || "mugi");
  } catch (error) {
    logApp("warn", "ローディング画面のキャラクター読込に失敗しました", errorDetail(error));
  }
  setStatus("音声設定を確認中…");
  await waitForAivisReady();
  await prepareSelectedVoiceModel();
  // local プロバイダはモデル DL・エンジン起動もローディング画面の間に終わらせる。
  if ((info.providerId || "local") === "local") {
    try { await waitForEngineReady(); }
    catch (error) { logApp("warn", "エンジン起動待ちに失敗しました", errorDetail(error)); }
  }
  let preparedGreeting = null;
  try {
    // 非表示のまま挨拶を描画し、表示と同時に再生できるよう音声を事前合成しておく。
    setStatus(`${bootCharacterName}の通常表示を読み込み中…`);
    await loadCharacter(info.characterId || "mugi", info.characterOverrides || {}, { speakGreeting: false });
    setStatus("会話履歴を画面へ復元中…");
    hydrateHistory(info.history);
    let greeting = characterManifest?.greeting || "";
    try {
      setStatus("今日のあいさつを確認中…");
      const daily = await window.picoagent.dailyGreeting();
      if (daily?.reply) greeting = combineStartupGreeting(greeting, daily.reply);
    } catch (error) {
      logApp("warn", "動的あいさつの取得に失敗しました", errorDetail(error));
    }
    if (greeting) {
      characterManifest.greeting = greeting;
      setStatus("起動時の読み上げ音声を準備中…");
      preparedGreeting = await prepareSpeech(greeting);
    }
  } catch (error) {
    logApp("error", "キャラクター読込に失敗しました", errorDetail(error));
    setStatus(`キャラクター読込に失敗: ${error.message}`);
  } finally {
    // ここで初めて全描画を見せ、描画した瞬間に挨拶を再生する。
    if (!status.textContent) setStatus(`${bootCharacterName}を起こしているよ…`);
    petApp.classList.remove("is-booting");
    stopBootLogTicker();
    if (preparedGreeting) playPreparedSpeech(preparedGreeting);
  }
  if (!info.up && !status.textContent) status.textContent = "起動中…";
  // 取りこぼし対策で現在のエンジン状態も反映。
  try { applyEngineStatus(await window.picoagent.engineStatus()); }
  catch (error) { logApp("warn", "エンジン状態取得に失敗しました", errorDetail(error)); }
  applyChatterConfig(info); // アイドル雑談タイマーを開始。
  startDailyPolling(); // リマインダー・集中・見守りの通知を開始。
  startAlarmPolling(); // 目覚まし時計の時刻監視を開始。
  loadScheduleRunNotices(); // 前回までのスケジュール実行履歴を通知一覧へ復元。
})().catch(error => {
  logApp("error", "起動初期化に失敗しました", errorDetail(error));
  setStatus(`起動初期化に失敗: ${error?.message || error}`);
  petApp.classList.remove("is-booting");
  stopBootLogTicker();
});
window.picoagent.on("pet:layout-reset", () => {
  resizeWindowSize = { width: 0, height: 0 };
  bubble.style.removeProperty("width");
  bubble.style.removeProperty("height");
  bubble.style.removeProperty("left");
  bubble.style.removeProperty("top");
  resetFullscreenBubbleLayoutVars();
  setTimeout(refreshBubblePlacement, 0);
});
