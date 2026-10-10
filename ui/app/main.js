// main.js - メイン画面の入口。各部品を組み立てて配線し、起動の手順を進める。
//
// 状態はそれぞれの部品が持つ（シーン: stage / キャラ: character host / 会話: chat・send /
// 読み上げ: voice / 通知: notices …）。ここは部品どうしをつなぐだけで、業務処理を持たない。

import * as bridge from "./bridge.js";
import { createAmbient } from "./ambient.js";
import { createBoot } from "./boot.js";
import { createContextMenu } from "./context-menu.js";
import { createEditor } from "./edit.js";
import { createEngine } from "./engine.js";
import { createGame } from "./game.js";
import { createLipSync } from "./lipsync.js";
import { createMic } from "./mic.js";
import { createMotion } from "./motion.js";
import { createNotices } from "./notice.js";
import { createOnboarding } from "./onboarding.js";
import { createRelationship } from "./relationship.js";
import { createStage } from "./stage.js";
import { toast, notifyBackgroundFailure } from "./toast.js";
import { copyText, el, errorDetail, errorMessage, fetchJson, isImeComposing, logApp } from "./util.js";
import { createVoice } from "./voice.js";
import { createWindowChrome } from "./window-chrome.js";
import { createCharacterHost } from "../character/index.js";
import { listCharacters, personaText, rememberVariant } from "../character/manifest.js";
import { createSender } from "../chat/send.js";
import { openExternalUrl } from "../chat/markdown.js";
import { createSources } from "../sources/index.js";

const LAYOUT_IDS = ["pet", "mascot", "stream"];
const app = document.getElementById("petApp");
const stageElement = document.getElementById("stage");
const sceneRoot = document.getElementById("sceneRoot");

window.addEventListener("error", event => {
  logApp("error", "未捕捉フロントエラー", `${event.message || ""} ${event.filename || ""}:${event.lineno || ""}:${event.colno || ""}\n${errorDetail(event.error)}`);
});
window.addEventListener("unhandledrejection", event => logApp("error", "未処理 Promise rejection", errorDetail(event.reason)));

// ── 部品の組み立て ───────────────────────────────────────────
const context = { toast };
const sources = createSources(sceneRoot, context);
context.sources = sources;
const chat = sources.get("chat");
const characterSource = sources.get("character");
const host = createCharacterHost({ container: characterSource.view });
context.host = host;
const stage = createStage({ stageElement, sceneRoot, sources, character: host, toast });
context.stage = stage;
const boot = createBoot({ app, screen: document.getElementById("bootScreen"), beforeReveal: () => host.capturePreview() });
context.boot = boot;
const { status, composer, buttons } = chat;
const motion = createMotion(host);
context.motion = motion;
const lipSync = createLipSync({ onMouth: (level, form) => host.setMouth(level, form) });

let sender = null;
let ambient = null;
let mic = null;
const voice = createVoice({
  motion,
  lipSync,
  pager: chat.pager,
  status,
  onCancelVisible: visible => chat.setVoiceActive(visible),
  onFinished: () => {
    setTimeout(() => ambient?.flushNotice(), 0);
    mic?.scheduleContinuous();
  },
});
context.voice = voice;
const notices = createNotices({
  panel: chat.noticePanel,
  button: buttons.noticeButton,
  status,
  onRetryPrompt: text => composer.reuse(text),
  onCancelJob: button => sender?.cancel(button),
});
context.notices = notices;
const relationship = createRelationship({ card: document.getElementById("relationshipCard"), onResume: text => composer.reuse(text) });
context.relationship = relationship;
const engine = createEngine({
  status,
  onChange: () => sender?.updateButtons(),
  onReady: () => {
    if (!boot.booting) showStartupReady();
  },
});
ambient = createAmbient({ chat, voice, host, motion, isBusy: () => Boolean(sender?.busy), isExpanded: () => stage.expanded });
context.ambient = ambient;
context.markActivity = () => ambient.markActivity();
const onboarding = createOnboarding({
  dialog: document.getElementById("onboarding"),
  status,
  openExternalUrl,
  isEngineBusy: () => engine.required && engine.phase !== "ready",
  onComplete: result => {
    relationship.render(result);
    motion.react("success", 1100);
    ambient.say("これからよろしくね。少しずつ、いっしょの時間を覚えていくよ。");
    mic?.scheduleContinuous();
  },
});
mic = createMic({
  chat,
  voice,
  motion,
  isBusy: () => Boolean(sender?.busy),
  canListen: () => !onboarding.open,
  onActivity: () => ambient.markActivity(),
  onListening: on => app.classList.toggle("is-listening", on),
});
sender = createSender({ chat, voice, motion, notices, engine, relationship, ambient, mic });
context.send = sender;
composer.onEscape = () => mic.cancelPendingSubmit();

const editor = createEditor({
  layer: document.getElementById("editLayer"),
  toolbar: document.getElementById("editToolbar"),
  stageElement,
  stage,
  host,
  toast,
  onModeChange: mode => chrome.setEditing(mode === "all"),
});
context.edit = editor;
editor.attachGrip("chat", chat.resizeGrip);
characterSource.element.addEventListener("wheel", event => editor.wheel(event, "character"), { passive: false });
stage.events.on("layout", () => chat.refreshTail());

const game = createGame({ voice, stage });
bridge.on("pet:game-open", () => game.open());

const chrome = createWindowChrome({
  menu: document.getElementById("topWindowMenu"),
  appElement: app,
  bootScreen: document.getElementById("bootScreen"),
  stageElement,
  onEditLayout: () => (editor.mode === "all" ? editor.stop() : editor.start("all")),
  onGame: () => game.open(),
  listCharacters: async () => (await listCharacters()).entries.map(entry => ({ id: entry.id, name: entry.name || entry.id })),
  onPinnedChange: pinned => {
    bridge.saveUiPref("PICOAGENT_TITLE_BAR_PINNED", pinned).catch(error => {
      logApp("error", "タイトルバーの表示設定を保存できませんでした", errorDetail(error));
      toast(`タイトルバーの表示設定を保存できませんでした: ${errorMessage(error)}`, "error");
    });
  },
});

createContextMenu({
  openExternalUrl,
  onCharacterAction(action, framing) {
    if (action === "edit-character") editor.start("character");
    else if (action === "reset-character") stage.resetCharacter().then(() => editor.render()).catch(() => {});
    else if (action === "set-framing") stage.setFraming(framing).then(() => editor.render()).catch(() => {});
  },
});

// ── 回答操作 ─────────────────────────────────────────────────
buttons.copyButton.addEventListener("click", async event => {
  event.stopPropagation();
  try {
    status.flash((await copyText(chat.lastAssistantText || chat.reply.textContent)) ? "コピーしました" : "コピーできませんでした", 1600);
  } catch (error) {
    logApp("warn", "回答コピーに失敗しました", errorDetail(error));
    status.flash("コピーできませんでした", 1600);
  }
});
buttons.replayButton.addEventListener("click", event => {
  event.stopPropagation();
  const text = chat.lastAssistantText || chat.reply.textContent;
  if (text.trim() && !sender.busy) voice.speak(text);
});
for (const button of [buttons.stopButton, buttons.cancelVoiceButton]) {
  button.addEventListener("click", event => {
    event.stopPropagation();
    voice.stop();
  });
}
buttons.expandButton.addEventListener("click", event => {
  event.stopPropagation();
  setExpanded(!stage.expanded);
});

/** 「拡大」: ウィンドウを全画面にし、会話パネルをキャラを避けた残りいっぱいへ広げて履歴を出す。 */
let fullscreenQueue = Promise.resolve();
let expandedGeneration = 0;
let confirmedExpanded = false;
async function setExpanded(on) {
  if (stage.expanded === on) return;
  const generation = ++expandedGeneration;
  stage.setExpanded(on);
  buttons.expandButton.textContent = on ? "戻す" : "拡大";
  // ネイティブ窓への変更は順番に送る。古い失敗で最新の操作を巻き戻さない。
  const operation = fullscreenQueue.catch(() => undefined).then(async () => {
    if (generation !== expandedGeneration) return;
    await bridge.setFullscreen(on);
    confirmedExpanded = on;
  });
  fullscreenQueue = operation;
  try { await operation; }
  catch (error) {
    if (generation !== expandedGeneration) return;
    stage.setExpanded(confirmedExpanded);
    buttons.expandButton.textContent = confirmedExpanded ? "戻す" : "拡大";
    status.flash(`全画面切替に失敗: ${errorMessage(error)}`, 4000);
  }
  if (generation !== expandedGeneration) return;
  if (!on) setTimeout(() => ambient.flushNotice(), 0);
}

// ── キーボード ───────────────────────────────────────────────
document.addEventListener("keydown", event => {
  if (event.defaultPrevented || isImeComposing(event)) return;
  if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "f" && chat.history.visible) {
    event.preventDefault();
    chat.history.focusSearch();
    return;
  }
  if (event.key !== "Escape" || editor.active) return;
  if (mic.cancelPendingSubmit()) return;
  if (relationship.open) { relationship.hide(); return; }
  if (composer.isOpen) { composer.close(); return; }
  setExpanded(false);
});

// ── ドロップ（どこへ落としても添付。文字列は入力欄へ） ─────────────────
const dropOverlay = document.getElementById("dropOverlay");
async function handleNativeDrop(paths) {
  ambient.markActivity();
  let added = 0;
  const failures = [];
  const release = composer.beginAttachmentLoad();
  try {
    for (const path of (paths || []).slice(0, 8)) {
      try {
        const reason = composer.addLoaded(await bridge.readDroppedPath(path));
        if (reason) failures.push(reason);
        else added += 1;
      } catch (error) {
        logApp("warn", "ドロップファイルの読み込みに失敗しました", `${path}: ${errorDetail(error)}`);
        failures.push(errorMessage(error));
      }
    }
  } finally {
    release();
  }
  if (added > 0) {
    composer.open();
    motion.react("pleased");
  }
  if (!failures.length) {
    if (added > 0) toast(`${added} 件を添付しました`, "success");
    return;
  }
  const prefix = added > 0 ? `${added} 件を添付、` : "";
  if (failures.some(message => message.includes("許可ルート") || message.includes("保護パス"))) {
    toast(`${prefix}許可されていない場所のファイルを読み込めませんでした`, "warn", {
      duration: 7000, actionLabel: "許可設定を開く", onAction: () => bridge.openSettings("permissions"),
    });
  } else {
    toast(`${prefix}${failures.length === 1 ? failures[0] : `${failures.length} 件を読み込めませんでした`}`, "error", { duration: 4500 });
  }
}
for (const [channel, callback] of [
  ["tauri://drag-enter", () => { dropOverlay.hidden = false; }],
  ["tauri://drag-leave", () => { dropOverlay.hidden = true; }],
  ["tauri://drag-drop", payload => { dropOverlay.hidden = true; handleNativeDrop(payload?.paths); }],
]) {
  bridge.listen(channel, callback).catch(error => logApp("warn", `${channel} の購読に失敗しました`, errorDetail(error)));
}
document.addEventListener("dragover", event => {
  const types = event.dataTransfer ? [...event.dataTransfer.types] : [];
  if (types.includes("text/plain") || types.includes("text/uri-list")) event.preventDefault();
});
document.addEventListener("drop", event => {
  if (!event.dataTransfer || event.dataTransfer.files.length > 0) return;
  const text = (event.dataTransfer.getData("text/uri-list") || event.dataTransfer.getData("text/plain")).trim();
  if (!text) return;
  event.preventDefault();
  ambient.markActivity();
  composer.insert(text);
});

// ── 見た目の設定 ─────────────────────────────────────────────
const UI_THEMES = ["glass", "pastel"];
function applyTheme(theme, uiTheme) {
  if (theme != null) {
    const value = theme === "light" || theme === "dark" ? theme : "system";
    if (value === "system") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = value;
  }
  if (uiTheme != null) document.documentElement.dataset.uiTheme = UI_THEMES.includes(uiTheme) ? uiTheme : "pastel";
}

function applyStreamAppearance(info) {
  if (info.streamBackground == null) return;
  sources.get("background").setAppearance({ background: info.streamBackground, image: info.streamBackgroundImage });
  sources.get("display").setAppearance({ display: info.streamDisplay, image: info.streamDisplayImage });
  const caption = /^#[0-9a-f]{6}$/i.test(String(info.streamCaptionColor)) ? String(info.streamCaptionColor) : "#5a3b4b";
  app.style.setProperty("--stream-caption-color", caption);
}

// ── キャラクター ─────────────────────────────────────────────
let characterOverrides = {};
let sdkOverrides = {};
let loadedSignature = "";
let characterName = "キャラクター";
let characterLoadGeneration = 0;
let characterLoads = 0;

function renderCharacterOptions(pack) {
  const extra = relationship.extra;
  extra.replaceChildren();
  if (pack.variants.length >= 2) {
    const select = el("select", {}, pack.variants.map(variant => el("option", { value: variant.id, textContent: variant.name || variant.id, selected: variant.id === pack.variantId })));
    select.addEventListener("change", async () => {
      const previous = pack.variantId;
      select.disabled = true;
      rememberVariant(pack.id, select.value);
      loadedSignature = "";
      try {
        await loadCharacter(pack.id, { speakGreeting: false });
        motion.react("pleased", 900);
      } catch (error) {
        logApp("error", "雰囲気の切替に失敗しました", errorDetail(error));
        rememberVariant(pack.id, previous);
        loadedSignature = "";
        await loadCharacter(pack.id, { speakGreeting: false }).catch(restoreError => logApp("error", "雰囲気の切替失敗後に元の表示へ戻せませんでした", errorDetail(restoreError)));
        toast("雰囲気を切り替えられませんでした", "error");
      } finally {
        select.disabled = false;
      }
    });
    extra.append(el("label", { className: "variant-row" }, ["スタイル", select]));
  }
  if (host.kind === "mmd") {
    const pose = el("select", {}, [
      el("option", { value: "natural", textContent: "自然な姿勢", selected: pack.mmd?.defaultPose !== "original" }),
      el("option", { value: "original", textContent: "元の姿勢", selected: pack.mmd?.defaultPose === "original" }),
    ]);
    pose.addEventListener("change", () => host.setPose(pose.value));
    extra.append(el("label", { className: "variant-row" }, ["待機姿勢", pose]));
  }
}

/** 見た目・人格・Live2D の世代・雰囲気のどれかが変わったときだけ描画を作り直す。 */
function signatureOf(characterId) {
  return [characterId, JSON.stringify(characterOverrides[characterId] || ""), sdkOverrides[characterId] || "", localStorageVariant(characterId)].join("\n");
}

async function loadCharacter(characterId, { speakGreeting = true } = {}) {
  const generation = ++characterLoadGeneration;
  const signature = signatureOf(characterId);
  if (!characterLoads && signature === loadedSignature) return null;
  characterLoads++;
  try {
    characterSource.element.style.visibility = "hidden";
    host.setDisplayReady(false);
    status.set("選択中のキャラクターを読み込み中…", "character-load");
    voice.stop(); // 切替前のキャラの合成待ち・再生音声を新しい姿へ持ち越さない
    const loaded = await host.load(characterId, { overrides: characterOverrides, sdkOverrides });
    if (!loaded || generation !== characterLoadGeneration) return null;
    const { pack, greeting } = loaded;
    characterName = pack.name;
    stage.setCharacter(pack.id);
    characterSource.element.dataset.characterId = pack.id;
    chrome.setCharacterName(pack.name);
    characterSource.element.title = `${pack.name}：右クリックでメニュー`;
    renderCharacterOptions(pack);
    motion.rest();
    // 挨拶も渡す（短い挨拶へモデルを呼ばず返すとき、全キャラ共通の定型文にしないため）。
    const appearance = await bridge.setCharacterProfile(pack.id, personaText(pack.manifest, characterOverrides[pack.id]), greeting);
    if (generation !== characterLoadGeneration) return null;
    // 人格登録まで確定したキャラの背景へ切り替え、直前のキャラ用背景を残さない。
    if (appearance) applyStreamAppearance(appearance);
    loadedSignature = signature;
    host.setDisplayReady(true);
    characterSource.element.style.visibility = ""; // 起動時の読込失敗後も、有効なキャラへ変更できれば表示を復旧する。
    status.clear("character-load");
    if (greeting && speakGreeting) {
      chat.renderReply(greeting);
      voice.speak(greeting);
    }
    return pack;
  } catch (error) {
    if (generation !== characterLoadGeneration) return null;
    loadedSignature = "";
    characterSource.element.style.visibility = "hidden";
    status.set(`キャラクターを読み込めません: ${errorMessage(error)}。設定からキャラを確認してください`, "character-load");
    throw error;
  } finally {
    characterLoads--;
  }
}

function localStorageVariant(characterId) {
  try { return localStorage.getItem(`picoagent.variant.${characterId}`) || ""; }
  catch { return ""; } // フォールバック許可: 保存領域を読めない環境では既定の見た目として扱う。
}

host.events.on("fallback", ({ kind }) => {
  status.flash(kind === "live2d" ? "Live2D を読み込めないので PNG 表示で続けるね" : "高品質リグを読み込めないので差分絵表示で続けるね", 5000);
});

// ── 表示形式 ─────────────────────────────────────────────────
function applyLayout(id) {
  const layout = LAYOUT_IDS.includes(id) ? id : "pet";
  // シーンの拡大を解除する時は、窓の全画面状態と操作ボタンも一緒に戻す。
  if (stage.layoutId !== layout && stage.expanded) setExpanded(false);
  app.dataset.layout = layout;
  stage.setLayout(layout);
  editor.render();
}

function showStartupReady() {
  if (sender?.busy || composer.isOpen) return;
  const current = String(chat.reply.textContent || "").trim();
  if (current && current !== chat.greeting) return;
  const text = `${characterName}の準備ができたよ。`;
  chat.renderReply(text);
  voice.speak(text);
}

function combineStartupGreeting(manifestGreeting, dailyReply) {
  // キャラ固定の挨拶と日次の時刻挨拶で「挨拶語」が二重にならないようにまとめる。
  const manifest = String(manifestGreeting ?? "").trim();
  const daily = String(dailyReply ?? "").trim();
  if (!manifest) return daily;
  if (!daily) return manifest;
  const greetings = ["おはようございます", "おはよう", "こんにちは", "こんにちわ", "こんばんは", "夕方だね"];
  const strip = text => {
    const word = greetings.find(item => text.startsWith(item)) || "";
    return { word, rest: word ? text.slice(word.length).replace(/^[、,。.\s]+/, "") : text };
  };
  const m = strip(manifest);
  const d = strip(daily);
  return m.word ? `${d.word || m.word}。${m.rest}${d.rest}` : `${manifest}${d.rest}`;
}

// ── バックエンドからのイベント ───────────────────────────────────
bridge.on("pet:layout", applyLayout);
bridge.on("pet:layout-reset", payload => {
  stage.replaceScenes(payload?.scenes || {});
  editor.render();
});
bridge.on("pet:layout-edit", target => editor.start(target === "character" ? "character" : "all"));
const noticeSubscription = bridge.listen("pet:notice", notice => notices.push(notice));
bridge.on("pet:model", model => status.flash(String(model), 1800));
bridge.on("pet:emotion", emotion => {
  if (!emotion?.mood) return;
  characterSource.element.dataset.emotion = emotion.mood;
  characterSource.element.dataset.energyLevel = emotion.energy > 0.6 ? "high" : emotion.energy < 0.4 ? "low" : "mid";
});
bridge.on("pet:hotkey", action => {
  if (action !== "summon") return;
  ambient.markActivity();
  composer.open();
});
bridge.on("pet:reset", () => {
  sender.reset();
  ambient.reset();
});
bridge.on("engine:status", state => engine.apply(state));
bridge.on("pet:reload", () => window.location.reload());
bridge.on("pet:show-onboarding", () => {
  voice.stop();
  onboarding.show();
});
bridge.on("pet:ui", ui => {
  if (!ui) return;
  applyTheme(ui.theme, ui.uiTheme);
  if (ui.titleBarPinned != null) chrome.setPinned(ui.titleBarPinned === true);
  if (ui.fontPx != null) chat.setFontPx(ui.fontPx);
  if (ui.streamChatFontPx != null) chat.setStreamFontPx(ui.streamChatFontPx);
  if (ui.voiceVolume != null) voice.setVolume(ui.voiceVolume);
  if (ui.relationshipMemoryEnabled != null) relationship.setMemoryEnabled(ui.relationshipMemoryEnabled, characterSource.heart);
  if (ui.gazeTrackingEnabled != null) host.setGazeTracking(ui.gazeTrackingEnabled);
  applyStreamAppearance(ui);
  if (ui.ttsEnabled != null) voice.setEnabled(ui.ttsEnabled);
  if (ui.layout != null) applyLayout(ui.layout);
  if (ui.characterOverrides) characterOverrides = ui.characterOverrides;
  if (ui.live2dSdkOverrides) sdkOverrides = ui.live2dSdkOverrides;
  if (ui.characterId) {
    loadCharacter(ui.characterId).catch(error => {
      logApp("error", "キャラクター変更に失敗しました", errorDetail(error));
      status.flash(`キャラクター変更に失敗: ${errorMessage(error)}`, 8000);
    });
  }
  ambient.applyChatter(ui);
  if (ui.quickActions != null) composer.setQuickActions(ui.quickActions);
  composer.refreshDefaultQuickActions();
});

// ── 起動 ────────────────────────────────────────────────────
async function loadSceneDefinitions() {
  const entries = await Promise.all(LAYOUT_IDS.map(async id => [id, await fetchJson(`scenes/${id}.json`, `画面構成（${id}）`)]));
  return Object.fromEntries(entries);
}

(async () => {
  // 起動画面のDOMを組み立ててから窓を表示する。以後の終了通知との順序も固定する。
  await bridge.frontendReady();
  status.set("設定と履歴を読み込み中…", "boot");
  await noticeSubscription;
  const [info, definitions] = await Promise.all([bridge.init(), loadSceneDefinitions()]);
  document.querySelector(".beta-badge")?.toggleAttribute("hidden", info.betaBuild !== true);
  applyTheme(info.theme, info.uiTheme);
  chrome.setPinned(info.titleBarPinned === true);
  if (info.fontPx != null) chat.setFontPx(info.fontPx);
  if (info.streamChatFontPx != null) chat.setStreamFontPx(info.streamChatFontPx);
  if (info.voiceVolume != null) voice.setVolume(info.voiceVolume);
  voice.setEnabled(info.ttsEnabled !== false);
  host.setGazeTracking(info.gazeTrackingEnabled === true);
  relationship.setMemoryEnabled(info.relationshipMemoryEnabled, characterSource.heart);
  relationship.render({ relationship: info.relationship, bondLabel: info.bondLabel });
  if (info.relationshipError) {
    notifyBackgroundFailure("relationship-load", `関係性データを読み込めません: ${info.relationshipError}`, info.relationshipError);
  }
  boot.setProgressInterval(info.bootProgressIntervalMs);
  engine.init(info, up => characterSource.setOnline(up));
  applyStreamAppearance(info);
  characterOverrides = info.characterOverrides || {};
  sdkOverrides = info.live2dSdkOverrides || {};
  stage.load(definitions, info.scenes || {});
  applyLayout(info.layout);
  // 原本を読めなかった履歴は保存を止めている。黙っていると再起動でその分が消えるので、起動時に知らせる。
  if (info.historySaveBlocked === true) {
    notifyBackgroundFailure("history-save-blocked", "会話履歴を読めなかったため、この会話は保存されません（元の履歴は残しています）", "history save blocked");
  }
  let startupGreeting = "";
  let preparedGreeting = null;
  let startupError = "";
  let voiceError = "";
  try {
    status.set("選択中のキャラクターを読み込み中…", "boot");
    const pack = await loadCharacter(info.characterId || "mugi", { speakGreeting: false });
    characterName = pack?.name || characterName;
    status.set("会話履歴を画面へ復元中…", "boot");
    chat.history.hydrate(info.history);
    composer.hydratePrompts(chat.history.items);
    const last = [...chat.history.items].reverse().find(item => item.role === "assistant");
    if (last) chat.lastAssistantText = last.content;
    let greeting = pack?.manifest?.greeting || "";
    try {
      status.set("今日のあいさつを確認中…", "boot");
      const daily = await bridge.dailyGreeting();
      if (daily?.reply) greeting = combineStartupGreeting(greeting, daily.reply);
    } catch (error) {
      logApp("warn", "動的あいさつの取得に失敗しました", errorDetail(error));
    }
    startupGreeting = greeting;
    if (engine.required && info.onboardingDone) {
      // 送信できない通常画面を先に見せず、準備完了か明示的な失敗になるまで起動画面で進捗を伝える。
      const state = await engine.waitUntilSettled(state => {
        if (!state || state.phase === "idle") status.set("ローカルAIを確認中…", "boot");
      });
      if (state?.phase === "error") startupError = state.message || "ローカルAIの準備に失敗しました";
    } else if (engine.required) {
      // 初回セットアップ中は待たせない（数GBの取得が終わるまで案内を出さないと何分も待たせる）。
      engine.waitUntilSettled().catch(error => logApp("error", "ローカルAIの起動確認に失敗しました", errorDetail(error)));
    }
    if (!startupError && info.onboardingDone && info.ttsEnabled !== false) {
      if (voice.enabled) {
        status.set("声を準備中…", "boot");
        try {
          await bridge.prepareVoiceModel();
          if (startupGreeting) preparedGreeting = await voice.prepare(startupGreeting);
        } catch (error) {
          logApp("error", "音声モデルの準備に失敗しました", errorDetail(error));
          voiceError = "音声を準備できませんでした。設定の診断を確認してください";
        }
      }
    }
  } catch (error) {
    logApp("error", "キャラクター読込に失敗しました", errorDetail(error));
    characterSource.element.style.visibility = "hidden";
    startupError = `選択したキャラを読み込めません: ${errorMessage(error)}。設定からキャラを確認してください`;
  } finally {
    status.set(`${characterName}を起こしているよ…`, "boot");
    stage.layoutNow();
    await boot.release(status);
    status.clear("boot");
    chrome.introduce();
  }
  if (startupGreeting) chat.renderReply(startupGreeting);
  if (preparedGreeting) voice.playPrepared(preparedGreeting);
  try { engine.apply(await bridge.engineStatus()); }
  catch (error) { logApp("warn", "エンジン状態取得に失敗しました", errorDetail(error)); }
  if (voiceError && engine.phase !== "error") status.setSticky(voiceError, "error");
  if (startupError) status.setSticky(startupError, "error");
  composer.setQuickActions(info.quickActions);
  composer.refreshDefaultQuickActions();
  sender.updateButtons();
  ambient.applyChatter(info);
  ambient.start({ napMinutes: info.napMinutes });
  notices.loadScheduleRuns();
  notices.loadDurableJobs();
  if (!info.onboardingDone) {
    voice.stop();
    onboarding.show();
    onboarding.installVoice().catch(error => logApp("warn", "音声読み上げの自動準備を開始できませんでした", errorDetail(error)));
  } else {
    mic.scheduleContinuous();
  }
})().catch(error => {
  logApp("error", "起動初期化に失敗しました", errorDetail(error));
  characterSource.element.style.visibility = "hidden";
  host.setDisplayReady(false);
  status.setSticky(`起動初期化に失敗: ${errorMessage(error)}`, "error");
  boot.release(status);
});

window.addEventListener("beforeunload", () => {
  composer.persistDraft();
  voice.stop();
  lipSync.dispose();
});
