const bubble = document.getElementById("bubble");
const reply = document.getElementById("reply");
const composer = document.getElementById("composer");
const input = document.getElementById("input");
const status = document.getElementById("status");
const character = document.getElementById("character");
const petSprite = document.getElementById("petSprite");
const online = document.getElementById("online");
const bubbleResize = document.getElementById("bubbleResize");
let busy = false;

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

async function loadCharacter(characterId, overrides = {}) {
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
  await window.coda.setCharacterProfile(manifest.id, profile);
  loadedCharacterSignature = signature;
  if (manifest.greeting) renderReply(manifest.greeting);
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

function openComposer() { if (busy) return; composer.hidden = false; reply.classList.add("compact"); input.focus(); }
function closeComposer() { composer.hidden = true; reply.classList.remove("compact"); }

// ダブルクリックでウィンドウをフルスクリーン化し、その中でフキダシを大きく広げる／元に戻すトグル。
async function setBubbleFullscreen(on) {
  if (bubble.classList.contains("fullscreen") === on) return;
  bubble.classList.toggle("fullscreen", on);
  try { await window.coda.setFullscreen(on); }
  catch (error) { bubble.classList.toggle("fullscreen", !on); status.textContent = `全画面切替に失敗: ${error}`; }
}
function toggleBubbleFullscreen() { setBubbleFullscreen(!bubble.classList.contains("fullscreen")); }
function exitBubbleFullscreen() { setBubbleFullscreen(false); }

// Markdownリンクと通常のURLだけを安全なDOMリンクへ変換する。
function renderReply(text) {
  reply.replaceChildren();
  const source = String(text ?? "");
  const links = /\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)|(https?:\/\/[^\s<]+)/g;
  let offset = 0;
  for (const match of source.matchAll(links)) {
    reply.append(document.createTextNode(source.slice(offset, match.index)));
    const href = match[2] || match[3];
    const anchor = document.createElement("a");
    anchor.href = href;
    anchor.textContent = match[1] || href;
    anchor.target = "_blank";
    anchor.rel = "noopener noreferrer";
    reply.append(anchor);
    offset = match.index + match[0].length;
  }
  reply.append(document.createTextNode(source.slice(offset)));
}

let speakingToken = 0;
let currentVoice = null;
function stopVoice() {
  if (!currentVoice) return;
  currentVoice.pause();
  URL.revokeObjectURL(currentVoice.src);
  currentVoice = null;
}
async function speakReply(text) {
  stopVoice();
  try {
    const result = await window.coda.speak(text);
    if (!result?.enabled || result.skipped || !result.audio?.length) return;
    const bytes = new Uint8Array(result.audio);
    const url = URL.createObjectURL(new Blob([bytes], { type: result.mime || "audio/wav" }));
    currentVoice = new Audio(url);
    currentVoice.addEventListener("ended", stopVoice, { once: true });
    currentVoice.addEventListener("error", stopVoice, { once: true });
    await currentVoice.play();
  } catch (error) {
    status.textContent = `読み上げに失敗: ${error.message || error}`;
    setTimeout(() => { if (status.textContent.startsWith("読み上げに失敗")) status.textContent = ""; }, 3200);
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
  setAnimation("idle");
}

bubble.addEventListener("click", event => { if (!event.target.closest("button,a")) openComposer(); });
character.addEventListener("dblclick", toggleBubbleFullscreen);
// フキダシ自体をダブルクリックしても、画面最大化のように大きく広げる／戻す。
bubble.addEventListener("dblclick", event => {
  if (event.target.closest("button,a,textarea,.bubble-resize")) return;
  toggleBubbleFullscreen();
});
document.addEventListener("contextmenu", event => { event.preventDefault(); window.coda.menu(); });

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
    await window.coda.setBubbleSize(Math.round(rect.width), Math.round(rect.height));
  } catch (error) {
    status.textContent = `サイズを保存できませんでした: ${error}`;
  }
}
bubbleResize.addEventListener("pointerup", finishBubbleResize);
bubbleResize.addEventListener("pointercancel", finishBubbleResize);

composer.addEventListener("submit", async event => {
  event.preventDefault(); const text = input.value.trim(); if (!text || busy) return;
  busy = true; closeComposer(); input.value = ""; status.textContent = "Web検索して考え中…"; setAnimation("thinking"); reply.textContent = text;
  const result = await window.coda.send(text);
  status.textContent = result.error ? "接続を確認してね" : "";
  if (result.error) {
    ++speakingToken;
    stopVoice();
    renderReply(result.reply);
    setAnimation("sad");
    setTimeout(() => setAnimation("idle"), 2200);
  } else {
    const speech = speakReply(result.reply);
    await renderReplySpeaking(result.reply);
    await speech.catch(() => {});
  }
  busy = false;
});
input.addEventListener("keydown", event => { if (event.key === "Escape") closeComposer(); if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); composer.requestSubmit(); } });
document.addEventListener("keydown", event => { if (event.key === "Escape") exitBubbleFullscreen(); });
window.coda.on("pet:model", model => { status.textContent = model; setTimeout(() => status.textContent = "", 1800); });
window.coda.on("pet:reset", () => { renderReply("新しい話をしよう。"); closeComposer(); });

// 同梱エンジン（local）の準備状況を表示する。
function applyEngineStatus(s) {
  if (!s || !s.phase) return;
  if (s.phase === "download") {
    status.textContent = `モデルを準備中… ${s.percent ?? 0}%`;
    online.dataset.up = false;
  } else if (s.phase === "starting") {
    status.textContent = "エンジンを起動中…";
    online.dataset.up = false;
  } else if (s.phase === "ready") {
    status.textContent = "準備ができたよ。";
    online.dataset.up = true;
    setTimeout(() => { if (status.textContent === "準備ができたよ。") status.textContent = ""; }, 2500);
  } else if (s.phase === "error") {
    status.textContent = `準備に失敗: ${s.message || "不明なエラー"}`;
    online.dataset.up = false;
  }
}
window.coda.on("engine:status", applyEngineStatus);

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
window.coda.on("pet:ui", (ui) => {
  if (!ui) return;
  if (ui.fontPx != null) applyFontPx(ui.fontPx);
  applyBubbleSize(ui.bubbleWidth, ui.bubbleHeight);
  if (ui.characterId) loadCharacter(ui.characterId, ui.characterOverrides || {}).catch(error => {
    status.textContent = `キャラクター変更に失敗: ${error.message}`;
  });
});

(async () => {
  const info = await window.coda.init();
  online.dataset.up = info.up;
  if (info.fontPx != null) applyFontPx(info.fontPx);
  applyBubbleSize(info.bubbleWidth, info.bubbleHeight);
  try { await loadCharacter(info.characterId || "mugi", info.characterOverrides || {}); }
  catch (error) { status.textContent = `キャラクター読込に失敗: ${error.message}`; }
  if (!info.up) status.textContent = "起動中…";
  // 取りこぼし対策で現在のエンジン状態も反映。
  try { applyEngineStatus(await window.coda.engineStatus()); } catch {}
})();
