const petApp = document.getElementById("petApp");
const bubble = document.getElementById("bubble");
const historyLog = document.getElementById("historyLog");
const reply = document.getElementById("reply");
const composer = document.getElementById("composer");
const input = document.getElementById("input");
const status = document.getElementById("status");
const bootStatus = document.getElementById("bootStatus");
const bootLog = document.getElementById("bootLog");
const bootPetImage = document.getElementById("bootPetImage");
const sendButton = document.getElementById("sendButton");
const copyReplyButton = document.getElementById("copyReply");
const replayReplyButton = document.getElementById("replayReply");
const stopVoiceButton = document.getElementById("stopVoice");
const toggleFullscreenButton = document.getElementById("toggleFullscreen");
const character = document.getElementById("character");
const petSprite = document.getElementById("petSprite");
const online = document.getElementById("online");
const bubbleResize = document.getElementById("bubbleResize");
let busy = false;
const conversationHistory = [];
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
// 目覚まし時計の発火管理（同じ分に二重で鳴らさないための済みキー集合）。
const ALARM_POLL_MS = 20000;
let alarmTimer = null;
const firedAlarms = new Set();
let activeAlarmSnooze = null;

function errorDetail(error) {
  if (!error) return "";
  if (error instanceof Error) return error.stack || error.message;
  if (typeof error === "string") return error;
  try { return JSON.stringify(error); }
  catch { return String(error); }
}

function logApp(level, message, detail) {
  try {
    window.picoagent?.log?.(level, message, detail == null ? "" : String(detail)).catch(() => {});
  } catch {}
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
  stopVoice(false);
});

const characterManifestCacheKey = "2026-06-28-standard-motion-v4";
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
  if (!manifest.animations?.idle) throw new Error("idle アニメーションが定義されていません");
  const profile = personaText(manifest, overrides[manifest.id]);
  const assetVersion = String(manifest.assetVersion || "");
  const signature = `${manifest.id}\n${assetVersion}\n${profile}`;
  if (signature === loadedCharacterSignature) return;
  characterRoot = root;
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
  animationRequestId = requestAnimationFrame(animate);
}
animationRequestId = requestAnimationFrame(animate);

function autosizeInput() {
  input.style.height = "auto";
  input.style.height = `${Math.min(180, Math.max(62, input.scrollHeight))}px`;
}
function updateComposerState() {
  if (sendButton) sendButton.disabled = busy || !input.value.trim();
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
  if (role === "assistant") lastAssistantText = content;
  if (conversationHistory.length > 80) conversationHistory.shift();
  renderHistory();
}

function hydrateHistory(items) {
  conversationHistory.length = 0;
  if (!Array.isArray(items)) {
    renderHistory();
    return;
  }
  for (const item of items) {
    const role = item?.role === "user" ? "user" : item?.role === "assistant" ? "assistant" : "";
    const content = String(item?.content ?? "").trim();
    if (role && content) conversationHistory.push({ role, content });
  }
  if (conversationHistory.length > 80) {
    conversationHistory.splice(0, conversationHistory.length - 80);
  }
  lastAssistantText = [...conversationHistory].reverse().find(item => item.role === "assistant")?.content || "";
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
  setTimeout(flushPendingNotice, 0);
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
  } catch (error) {
    logApp("warn", "音声の事前合成に失敗しました", errorDetail(error));
    return null;
  }
}
// 事前合成済みの音声を再生し、会話アニメを開始する。再生終了・失敗時に Object URL を解放する。
function playPreparedSpeech(result) {
  stopVoice(false);
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
  if (!currentVoice) return;
  event.stopPropagation();
  stopVoice();
  status.textContent = "";
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
function expandWindowForBubble(left, top, width, height) {
  const margin = 24;
  const nextWidth = Math.ceil(Math.max(window.innerWidth, left + width + margin));
  const nextHeight = Math.ceil(Math.max(window.innerHeight, top + height + margin));
  if (nextWidth <= window.innerWidth && nextHeight <= window.innerHeight) return;
  if (nextWidth <= resizeWindowSize.width && nextHeight <= resizeWindowSize.height) return;
  resizeWindowSize = { width: nextWidth, height: nextHeight };
  window.picoagent.setWindowSize(nextWidth, nextHeight).catch(error => {
    logApp("warn", "フキダシ拡大に合わせたウィンドウサイズ変更に失敗しました", errorDetail(error));
  });
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
  const best = candidates[0] || { left: baseLeft, top: baseTop, overlap: 0 };
  if (best.overlap > 0) {
    fitBubbleIntoFreePane(width, height, preferredLeft, preferredTop, {
      margin,
      gap,
      viewportWidth,
      viewportHeight,
      charLeftEdge,
      charTopEdge,
    });
    return;
  }
  const left = best.left;
  const top = best.top;
  bubble.style.left = `${Math.round(left)}px`;
  bubble.style.top = `${Math.round(top)}px`;
}

function fitBubbleIntoFreePane(width, height, preferredLeft, preferredTop, bounds) {
  const { margin, viewportWidth, viewportHeight, charLeftEdge, charTopEdge } = bounds;
  const options = [];
  const leftPaneWidth = Math.floor(charLeftEdge - margin);
  if (leftPaneWidth >= 180) {
    const nextWidth = Math.min(width, leftPaneWidth);
    const nextHeight = Math.min(height, viewportHeight - margin * 2);
    options.push({
      width: nextWidth,
      height: nextHeight,
      left: clampNumber(preferredLeft, margin, charLeftEdge - nextWidth),
      top: clampNumber(preferredTop, margin, Math.max(margin, viewportHeight - nextHeight - margin)),
      area: nextWidth * nextHeight,
      delta: Math.abs(width - nextWidth) + Math.abs(height - nextHeight),
    });
  }
  const topPaneHeight = Math.floor(charTopEdge - margin);
  if (topPaneHeight >= 92) {
    const nextWidth = Math.min(width, viewportWidth - margin * 2);
    const nextHeight = Math.min(height, topPaneHeight);
    options.push({
      width: nextWidth,
      height: nextHeight,
      left: clampNumber(preferredLeft, margin, Math.max(margin, viewportWidth - nextWidth - margin)),
      top: clampNumber(preferredTop, margin, charTopEdge - nextHeight),
      area: nextWidth * nextHeight,
      delta: Math.abs(width - nextWidth) + Math.abs(height - nextHeight),
    });
  }
  options.sort((a, b) => b.area - a.area || a.delta - b.delta);
  const best = options[0] || {
    width: Math.min(width, viewportWidth - margin * 2),
    height: Math.min(height, viewportHeight - margin * 2),
    left: margin,
    top: margin,
  };
  bubble.style.width = `${Math.round(best.width)}px`;
  bubble.style.height = `${Math.round(best.height)}px`;
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
    width: rect.width,
    height: rect.height,
    left: bubble.offsetLeft,
    top: bubble.offsetTop,
  };
  bubbleResize.setPointerCapture(event.pointerId);
});
bubbleResize.addEventListener("pointermove", event => {
  if (!resizeStart) return;
  const margin = 12;
  const dx = event.clientX - resizeStart.x;
  const dy = event.clientY - resizeStart.y;
  const desiredWidth = Math.max(180, resizeStart.width + dx);
  const desiredHeight = Math.max(92, resizeStart.height + dy);
  const left = clampNumber(resizeStart.left, margin, document.documentElement.clientWidth - 180 - margin);
  const top = clampNumber(resizeStart.top, margin, document.documentElement.clientHeight - 92 - margin);
  const width = desiredWidth;
  const height = desiredHeight;
  bubble.style.width = `${Math.round(width)}px`;
  bubble.style.height = `${Math.round(height)}px`;
  bubble.style.left = `${Math.round(left)}px`;
  bubble.style.top = `${Math.round(top)}px`;
  expandWindowForBubble(left, top, width, height);
});
async function finishBubbleResize(event) {
  if (!resizeStart) return;
  resizeStart = null;
  if (bubbleResize.hasPointerCapture(event.pointerId)) bubbleResize.releasePointerCapture(event.pointerId);
  const rect = bubble.getBoundingClientRect();
  const placed = bubble.getBoundingClientRect();
  try {
    await window.picoagent.setBubbleLayout(
      Math.round(placed.width || rect.width),
      Math.round(placed.height || rect.height),
      Math.round(bubble.offsetLeft),
      Math.round(bubble.offsetTop),
    );
  } catch (error) {
    status.textContent = `フキダシ位置を保存できませんでした: ${error}`;
  }
}
bubbleResize.addEventListener("pointerup", finishBubbleResize);
bubbleResize.addEventListener("pointercancel", finishBubbleResize);

composer.addEventListener("submit", async event => {
  event.preventDefault();
  const text = input.value.trim();
  if (!text || busy) return;
  clearAlarmSnooze(false);
  busy = true;
  ++speakingToken;
  stopVoice(false);
  updateComposerState();
  closeComposer();
  input.value = "";
  autosizeInput();
  status.textContent = "考え中…";
  setAnimation("thinking");
  reply.textContent = text;
  addHistory("user", text);
  try {
    const result = await window.picoagent.send(text);
    status.textContent = result.error ? "接続を確認してね" : "";
    if (result.error) {
      logApp("error", "AI応答がエラーを返しました", result.reply || "");
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
  } catch (error) {
    logApp("error", "AI応答の取得に失敗しました", errorDetail(error));
    ++speakingToken;
    stopVoice();
    const message = `応答に失敗しました: ${error?.message || error}`;
    renderReply(message);
    addHistory("assistant", message);
    status.textContent = "接続を確認してね";
    setAnimation("sad");
    setTimeout(() => setAnimation("idle"), 2200);
  } finally {
    busy = false;
    updateComposerState();
    scheduleChatter(); // 会話後はアイドル計測をやり直す。
    setTimeout(flushPendingNotice, 0);
  }
});
input.addEventListener("input", () => { autosizeInput(); updateComposerState(); });
input.addEventListener("keydown", event => {
  if (event.key === "Escape") {
    event.stopPropagation();
    closeComposer();
  }
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    composer.requestSubmit();
  }
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
window.picoagent.on("pet:reset", () => { conversationHistory.length = 0; renderHistory(); stopVoice(); renderReply("新しい話をしよう。"); closeComposer(); scheduleChatter(); });

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
      setStatus("準備ができたよ。");
      setTimeout(() => { if (status.textContent === "準備ができたよ。") status.textContent = ""; }, 2500);
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
function applyBubbleLayout(width, height, left, top) {
  const w = parseFloat(width);
  const h = parseFloat(height);
  const x = parseFloat(left);
  const y = parseFloat(top);
  if (Number.isFinite(w) && w >= 180) bubble.style.width = `${w}px`;
  if (Number.isFinite(h) && h >= 92) bubble.style.height = `${h}px`;
  if (Number.isFinite(x) && x >= 0) bubble.style.left = `${x}px`;
  if (Number.isFinite(y) && y >= 0) bubble.style.top = `${y}px`;
  refreshBubblePlacement();
}
window.addEventListener("resize", refreshBubblePlacement);
petSprite.addEventListener("load", refreshBubblePlacement);
window.picoagent.on("pet:ui", (ui) => {
  if (!ui) return;
  if (ui.fontPx != null) applyFontPx(ui.fontPx);
  if (ui.voiceVolume != null) applyVoiceVolume(ui.voiceVolume);
  applyBubbleLayout(ui.bubbleWidth, ui.bubbleHeight, ui.bubbleLeft, ui.bubbleTop);
  if (ui.characterId) loadCharacter(ui.characterId, ui.characterOverrides || {}).catch(error => {
    logApp("error", "キャラクター変更に失敗しました", errorDetail(error));
    status.textContent = `キャラクター変更に失敗: ${error.message}`;
  });
  applyChatterConfig(ui); // 設定変更を即反映。
});

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
  if (!canShowNotice()) {
    if (options.enqueue !== false) queueNotice(line);
    return;
  }
  const prepared = await prepareSpeech(line);
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
      if (daily?.reply) greeting = `${greeting}\n${daily.reply}`.trim();
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
})().catch(error => {
  logApp("error", "起動初期化に失敗しました", errorDetail(error));
  setStatus(`起動初期化に失敗: ${error?.message || error}`);
  petApp.classList.remove("is-booting");
  stopBootLogTicker();
});
