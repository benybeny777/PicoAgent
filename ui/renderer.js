const petApp = document.getElementById("petApp");
const bubble = document.getElementById("bubble");
const historyLog = document.getElementById("historyLog");
const reply = document.getElementById("reply");
const composer = document.getElementById("composer");
const input = document.getElementById("input");
const status = document.getElementById("status");
const bootStatus = document.getElementById("bootStatus");
const character = document.getElementById("character");
const petSprite = document.getElementById("petSprite");
const online = document.getElementById("online");
const bubbleResize = document.getElementById("bubbleResize");
let busy = false;
const conversationHistory = [];
// アイドル時の自発雑談（雑談スキル）設定とタイマー。
const chatterCfg = { enabled: false, minSecs: 90, maxSecs: 240 };
let chatterTimer = null;
// 目覚まし時計の発火管理（同じ分に二重で鳴らさないための済みキー集合）。
const ALARM_POLL_MS = 20000;
let alarmTimer = null;
const firedAlarms = new Set();

const characterManifestCacheKey = "2026-06-25-mugi-expression-v11";
let characterRoot = "assets/characters/mugi";
let characterManifest = null;
let characterAssetVersion = "";
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
    bootStatus.textContent = text || "起動準備中…";
  }
}

function frameUrl(name, index) {
  const animation = animations[name] || animations.idle;
  const url = `${characterRoot}/${animation.path}/${String(index).padStart(2, "0")}.png`;
  return characterAssetVersion ? `${url}?v=${encodeURIComponent(characterAssetVersion)}` : url;
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
  const catalogResponse = await fetch(`assets/characters/index.json?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!catalogResponse.ok) throw new Error(`キャラクター一覧: HTTP ${catalogResponse.status}`);
  const catalog = await catalogResponse.json();
  const entry = catalog.characters.find(character => character.id === characterId)
    || catalog.characters.find(character => character.id === catalog.default);
  if (!entry) throw new Error("利用できるキャラクターがありません");
  const manifestResponse = await fetch(`assets/characters/${entry.manifest}?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!manifestResponse.ok) throw new Error(`キャラクター定義: HTTP ${manifestResponse.status}`);
  const manifest = await manifestResponse.json();
  if (!manifest.animations?.idle) throw new Error("idle アニメーションが定義されていません");
  const profile = personaText(manifest, overrides[manifest.id]);
  const assetVersion = String(manifest.assetVersion || "");
  const signature = `${manifest.id}\n${assetVersion}\n${profile}`;
  if (signature === loadedCharacterSignature) return;
  characterRoot = `assets/characters/${entry.manifest.replace(/\/character\.json$/, "")}`;
  characterManifest = manifest;
  characterAssetVersion = assetVersion;
  animations = manifest.animations;
  preloadAnimations();
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
  animationName = animations[name] ? name : "idle"; frame = 0; lastFrameAt = performance.now();
  character.dataset.motion = animations[animationName].motion || animationName;
  petSprite.src = frameUrl(animationName, 0);
}
function frameDuration(animation, index) {
  const duration = animation.frameDurations?.[index];
  if (Number.isFinite(duration) && duration > 0) return duration;
  return 1000 / animation.fps;
}
function animate(now) {
  if (reduceMotion) { petSprite.src = frameUrl("idle", 0); return; }
  const animation = animations[animationName] || animations.idle;
  if (!lastFrameAt) lastFrameAt = now;
  if (now - lastFrameAt >= frameDuration(animation, frame)) {
    frame = (frame + 1) % animation.frames;
    petSprite.src = frameUrl(animationName, frame);
    lastFrameAt = now;
  }
  requestAnimationFrame(animate);
}
requestAnimationFrame(animate);

function openComposer() { if (busy) return; composer.hidden = false; reply.classList.add("compact"); input.focus(); scheduleChatter(); }
function closeComposer() { composer.hidden = true; reply.classList.remove("compact"); }

// ダブルクリックでウィンドウをフルスクリーン化し、その中でフキダシを大きく広げる／元に戻すトグル。
async function setBubbleFullscreen(on) {
  if (bubble.classList.contains("fullscreen") === on) return;
  bubble.classList.toggle("fullscreen", on);
  renderHistory();
  try { await window.picoagent.setFullscreen(on); }
  catch (error) { bubble.classList.toggle("fullscreen", !on); status.textContent = `全画面切替に失敗: ${error}`; }
}
function toggleBubbleFullscreen() { setBubbleFullscreen(!bubble.classList.contains("fullscreen")); }
function exitBubbleFullscreen() { setBubbleFullscreen(false); }

function appendLinkedText(container, text) {
  const source = String(text ?? "");
  const links = /\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)|(https?:\/\/[^\s<]+)/g;
  let offset = 0;
  for (const match of source.matchAll(links)) {
    container.append(document.createTextNode(source.slice(offset, match.index)));
    const href = match[2] || match[3];
    const anchor = document.createElement("a");
    anchor.href = href;
    anchor.textContent = match[1] || href;
    anchor.target = "_blank";
    anchor.rel = "noopener noreferrer";
    container.append(anchor);
    offset = match.index + match[0].length;
  }
  container.append(document.createTextNode(source.slice(offset)));
}

// Markdownリンクと通常のURLだけを安全なDOMリンクへ変換する。
function renderReply(text) {
  reply.replaceChildren();
  appendLinkedText(reply, text);
}

function addHistory(role, text) {
  const content = String(text ?? "").trim();
  if (!content) return;
  conversationHistory.push({ role, content });
  if (conversationHistory.length > 80) conversationHistory.shift();
  renderHistory();
}

function renderHistory() {
  if (!historyLog) return;
  historyLog.replaceChildren();
  if (!conversationHistory.length) {
    historyLog.append(document.createElement("div"));
    return;
  }
  for (const item of conversationHistory) {
    const row = document.createElement("div");
    row.className = `history-item ${item.role}`;
    const label = document.createElement("div");
    label.className = "history-role";
    label.textContent = item.role === "user" ? "あなた" : (characterManifest?.name || "むぎ");
    const body = document.createElement("div");
    body.className = "history-body";
    appendLinkedText(body, item.content);
    row.append(label, body);
    historyLog.append(row);
  }
  historyLog.scrollTop = historyLog.scrollHeight;
}

let speakingToken = 0;
let currentVoice = null;
function finishVoice(toIdle = true) {
  if (!currentVoice) return;
  const voice = currentVoice;
  currentVoice = null;
  URL.revokeObjectURL(voice.src);
  if (toIdle && animationName === "talking") setAnimation("idle");
}
function stopVoice(toIdle = true) {
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
  } catch { return null; }
}
// 事前合成済みの音声を再生し、会話アニメを開始する。再生終了・失敗時に Object URL を解放する。
function playPreparedSpeech(result) {
  stopVoice(false);
  const bytes = new Uint8Array(result.audio);
  const url = URL.createObjectURL(new Blob([bytes], { type: result.mime || "audio/wav" }));
  currentVoice = new Audio(url);
  currentVoice.addEventListener("ended", () => { finishVoice(true); status.textContent = ""; }, { once: true });
  currentVoice.addEventListener("error", () => { finishVoice(true); status.textContent = "音声再生に失敗"; }, { once: true });
  currentVoice.play().catch(() => { finishVoice(true); });
  status.textContent = "読み上げ中…";
  setAnimation("talking", true);
}
async function speakReply(text) {
  let prepareTimer = null;
  try {
    const startedAt = Date.now();
    status.textContent = "読み上げ準備中…";
    prepareTimer = setInterval(() => {
      const seconds = Math.floor((Date.now() - startedAt) / 1000);
      status.textContent = `読み上げ準備中… ${seconds}秒`;
    }, 1000);
    const result = await window.picoagent.speak(text);
    clearInterval(prepareTimer);
    prepareTimer = null;
    if (!result?.enabled || result.skipped || !result.audio?.length) {
      status.textContent = "";
      return;
    }
    playPreparedSpeech(result);
  } catch (error) {
    if (prepareTimer) clearInterval(prepareTimer);
    stopVoice();
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
  renderReply(source);
  if (!currentVoice) setAnimation("idle");
}

bubble.addEventListener("click", event => {
  if (event.target.closest("button,a,textarea")) return;
  // テキスト選択中（コピー操作）はフキダシを入力欄へ切り替えない。
  if (String(window.getSelection?.() ?? "").trim()) return;
  openComposer();
});
character.addEventListener("dblclick", toggleBubbleFullscreen);
// フキダシ自体をダブルクリックしても、画面最大化のように大きく広げる／戻す。
bubble.addEventListener("dblclick", event => {
  if (event.target.closest("button,a,textarea,.bubble-resize")) return;
  toggleBubbleFullscreen();
});
document.addEventListener("contextmenu", event => { event.preventDefault(); window.picoagent.menu(); });

// 右下のつまみをドラッグしてフキダシをリサイズし、確定値を設定へ保存する。
let resizeStart = null;
bubbleResize.addEventListener("pointerdown", event => {
  event.preventDefault();
  event.stopPropagation();
  const rect = bubble.getBoundingClientRect();
  resizeStart = { x: event.clientX, y: event.clientY, width: rect.width, height: rect.height };
  bubbleResize.setPointerCapture(event.pointerId);
});
bubbleResize.addEventListener("pointermove", event => {
  if (!resizeStart) return;
  const maxWidth = document.documentElement.clientWidth - bubble.offsetLeft - 16;
  const maxHeight = document.documentElement.clientHeight - bubble.offsetTop - 16;
  const width = Math.min(maxWidth, Math.max(180, resizeStart.width + event.clientX - resizeStart.x));
  const height = Math.min(maxHeight, Math.max(92, resizeStart.height + event.clientY - resizeStart.y));
  bubble.style.width = `${Math.round(width)}px`;
  bubble.style.height = `${Math.round(height)}px`;
});
async function finishBubbleResize(event) {
  if (!resizeStart) return;
  resizeStart = null;
  if (bubbleResize.hasPointerCapture(event.pointerId)) bubbleResize.releasePointerCapture(event.pointerId);
  const rect = bubble.getBoundingClientRect();
  try {
    await window.picoagent.setBubbleSize(Math.round(rect.width), Math.round(rect.height));
  } catch (error) {
    status.textContent = `サイズを保存できませんでした: ${error}`;
  }
}
bubbleResize.addEventListener("pointerup", finishBubbleResize);
bubbleResize.addEventListener("pointercancel", finishBubbleResize);

composer.addEventListener("submit", async event => {
  event.preventDefault(); const text = input.value.trim(); if (!text || busy) return;
  busy = true; closeComposer(); input.value = ""; status.textContent = "Web検索して考え中…"; setAnimation("thinking"); reply.textContent = text; addHistory("user", text);
  const result = await window.picoagent.send(text);
  status.textContent = result.error ? "接続を確認してね" : "";
  if (result.error) {
    ++speakingToken;
    stopVoice();
    renderReply(result.reply);
    addHistory("assistant", result.reply);
    setAnimation("sad");
    setTimeout(() => setAnimation("idle"), 2200);
  } else {
    // 返信がユーザー入力のオウム返しのときは読み上げない（入力は声に出さない）。
    if (String(result.reply ?? "").trim() !== text) speakReply(result.reply);
    await renderReplySpeaking(result.reply);
    addHistory("assistant", result.reply);
  }
  busy = false;
  scheduleChatter(); // 会話後はアイドル計測をやり直す。
});
input.addEventListener("keydown", event => { if (event.key === "Escape") closeComposer(); if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); composer.requestSubmit(); } });
document.addEventListener("keydown", event => { if (event.key === "Escape") exitBubbleFullscreen(); });
window.picoagent.on("pet:model", model => { status.textContent = model; setTimeout(() => status.textContent = "", 1800); });
window.picoagent.on("pet:reset", () => { conversationHistory.length = 0; renderHistory(); renderReply("新しい話をしよう。"); closeComposer(); scheduleChatter(); });

// 同梱エンジン（local）の準備状況を表示する。
function applyEngineStatus(s) {
  if (!s || !s.phase) return;
  if (s.phase === "download") {
    setStatus(`モデルを準備中… ${s.percent ?? 0}%`);
    online.dataset.up = false;
  } else if (s.phase === "starting") {
    setStatus("エンジンを起動中…");
    online.dataset.up = false;
  } else if (s.phase === "ready") {
    online.dataset.up = true;
    if (petApp.classList.contains("is-booting")) {
      status.textContent = "";
      if (bootStatus) bootStatus.textContent = "むぎを起こしているよ…";
    } else {
      setStatus("準備ができたよ。");
      setTimeout(() => { if (status.textContent === "準備ができたよ。") status.textContent = ""; }, 2500);
    }
  } else if (s.phase === "error") {
    setStatus(`準備に失敗: ${s.message || "不明なエラー"}`);
    online.dataset.up = false;
  }
}
window.picoagent.on("engine:status", applyEngineStatus);

// フキダシの文字サイズを適用する（小さくて見えない問題への対応。設定で変更可能）。
function applyFontPx(px) {
  const n = parseFloat(px);
  if (!Number.isFinite(n) || n <= 0) return;
  reply.style.fontSize = `${n}px`;
  // 入力欄・ステータスも比率を合わせて読みやすくする。
  input.style.fontSize = `${n}px`;
  status.style.fontSize = `${Math.max(9, Math.round(n * 0.72))}px`;
}
function applyBubbleSize(width, height) {
  const w = parseFloat(width);
  const h = parseFloat(height);
  if (Number.isFinite(w) && w >= 180) bubble.style.width = `${w}px`;
  if (Number.isFinite(h) && h >= 92) bubble.style.height = `${h}px`;
}
window.picoagent.on("pet:ui", (ui) => {
  if (!ui) return;
  if (ui.fontPx != null) applyFontPx(ui.fontPx);
  applyBubbleSize(ui.bubbleWidth, ui.bubbleHeight);
  if (ui.characterId) loadCharacter(ui.characterId, ui.characterOverrides || {}).catch(error => {
    status.textContent = `キャラクター変更に失敗: ${error.message}`;
  });
  applyChatterConfig(ui); // 設定変更を即反映。
});

// TTS 有効時は同梱 Aivis が応答するまで待ってからキャラを描画する（無効時は待たない）。
// 起動失敗で永久に出ないことを避けるため、上限（約120秒）を超えたら諦めて描画する。
async function waitForAivisReady() {
  let s;
  try { s = await window.picoagent.aivisStatus(); }
  catch { return; }
  if (!s || !s.enabled || s.up) return;
  setStatus("音声エンジンを起動中…");
  for (let i = 0; i < 240; i += 1) {
    await new Promise(resolve => setTimeout(resolve, 500));
    try { s = await window.picoagent.aivisStatus(); }
    catch { break; }
    if (s && s.up) break;
  }
  if (status.textContent === "音声エンジンを起動中…") setStatus("");
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
    if (line && !busy && composer.hidden && !currentVoice) {
      // 先に音声を合成し、合成完了後に文字を表示して即時に読み上げる（表示と音声を同時に）。
      const prepared = await prepareSpeech(line);
      if (busy || !composer.hidden || currentVoice) return; // 合成中に状況が変わったら出さない。
      renderReply(line);
      addHistory("assistant", line);
      if (prepared) playPreparedSpeech(prepared);
    }
  } catch { /* 失敗は無視して次回へ */ }
  finally { scheduleChatter(); }
}

// 目覚まし時計: 一定間隔で現在時刻と登録アラームを照合し、一致したら声でやさしく起こす。
function pad2(n) { return String(n).padStart(2, "0"); }
function weekdayCode(date) { return ["sun", "mon", "tue", "wed", "thu", "fri", "sat"][date.getDay()]; }
async function checkAlarms() {
  let res;
  try { res = await window.picoagent.alarms(); }
  catch { return; }
  const alarms = res?.alarms;
  if (!Array.isArray(alarms) || !alarms.length) return;
  const now = new Date();
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
  } catch { /* 生成失敗時は固定文で起こす */ }
  if (!line) line = "おはよう…そろそろ起きる時間だよ。ゆっくりでいいからね。";
  // 起こすときは読み上げ中でも割り込む。音声合成→表示→即時再生（挨拶と同じ同期再生）。
  const prepared = await prepareSpeech(line);
  renderReply(line);
  addHistory("assistant", line);
  if (prepared) playPreparedSpeech(prepared);
  else setAnimation("talking", true);
}
function startAlarmPolling() {
  if (alarmTimer) clearInterval(alarmTimer);
  alarmTimer = setInterval(checkAlarms, ALARM_POLL_MS);
  checkAlarms();
}

(async () => {
  const info = await window.picoagent.init();
  online.dataset.up = info.up;
  if (info.fontPx != null) applyFontPx(info.fontPx);
  applyBubbleSize(info.bubbleWidth, info.bubbleHeight);
  await waitForAivisReady();
  let preparedGreeting = null;
  try {
    // 非表示のまま挨拶を描画し、表示と同時に再生できるよう音声を事前合成しておく。
    await loadCharacter(info.characterId || "mugi", info.characterOverrides || {}, { speakGreeting: false });
    if (characterManifest?.greeting) preparedGreeting = await prepareSpeech(characterManifest.greeting);
  } catch (error) {
    setStatus(`キャラクター読込に失敗: ${error.message}`);
  } finally {
    // ここで初めて全描画を見せ、描画した瞬間に挨拶を再生する。
    if (!status.textContent) setStatus("むぎを起こしているよ…");
    petApp.classList.remove("is-booting");
    if (preparedGreeting) playPreparedSpeech(preparedGreeting);
  }
  if (!info.up && !status.textContent) status.textContent = "起動中…";
  // 取りこぼし対策で現在のエンジン状態も反映。
  try { applyEngineStatus(await window.picoagent.engineStatus()); } catch {}
  applyChatterConfig(info); // アイドル雑談タイマーを開始。
  startAlarmPolling(); // 目覚まし時計の時刻監視を開始。
})();
