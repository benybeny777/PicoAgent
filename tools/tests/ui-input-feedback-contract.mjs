import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const bridge = readFileSync(new URL("../../ui/picoagent.js", import.meta.url), "utf8");
const between = (start, end) => renderer.slice(renderer.indexOf(start), renderer.indexOf(end, renderer.indexOf(start)));

function keyboard() {
  const input = {
    value: "下書き", selectionStart: 0, selectionEnd: 0,
    addEventListener(_, callback) { this.keydown = callback; },
    setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; },
  };
  const context = vm.createContext({
    input, status: {}, setTimeout() {},
    setInputValue(value) { input.value = value; },
    composer: { requestSubmit() { throw new Error("意図しない送信"); } },
  });
  vm.runInContext('const promptHistory = ["前の入力", "直前の入力"]; let promptHistoryIndex = null; let promptDraft = "";', context);
  vm.runInContext(between("function recallPromptHistory(direction)", "function openComposer()"), context);
  vm.runInContext(between("function isImeComposing(event)", "let historySearchTimer"), context);
  return input;
}

function press(input, key, extra = {}) {
  let prevented = false;
  input.keydown({ key, preventDefault() { prevented = true; }, ...extra });
  return prevented;
}

test("提案文とテキストドロップは入力履歴の途中でも新しい下書きになる", () => {
  for (const mode of ["suggestion", "drop"]) {
    let saved = "";
    let drop;
    const buttons = [];
    const input = { value: "前の入力", focus() {}, setSelectionRange() {} };
    const context = vm.createContext({
      input, status: {}, setTimeout() {}, markActivity() {},
      composer: { hidden: false },
      attachmentSuggestions: { replaceChildren() {}, append(button) { buttons.push(button); } },
      attachedFiles: [{ name: "資料.pdf" }], attachedImages: [],
      document: {
        createElement() { return { addEventListener(_, callback) { this.click = callback; } }; },
        addEventListener(_, callback) { drop = callback; },
      },
      setInputValue(value) { input.value = value; },
      persistDraft() { saved = input.value; },
    });
    vm.runInContext('const promptHistory = ["前の入力", "直前の入力"]; let promptHistoryIndex = 0; let promptDraft = "以前の下書き";', context);
    vm.runInContext(between("function resetPromptHistoryNavigation()", "function openComposer()"), context);
    if (mode === "suggestion") {
      vm.runInContext(between("function renderAttachmentSuggestions()", "function fileIcon(name)"), context);
      vm.runInContext("renderAttachmentSuggestions()", context);
      buttons[0].click();
    } else {
      vm.runInContext(between('document.addEventListener("drop", (e) => {', "// 画面を見てほしい系"), context);
      drop({ preventDefault() {}, dataTransfer: { files: [], getData() { return "追加テキスト"; } } });
    }
    const draft = input.value;
    assert.notEqual(draft, "前の入力");
    assert.equal(saved, draft);
    vm.runInContext("recallPromptHistory(-1); recallPromptHistory(1);", context);
    assert.equal(input.value, draft);
  }
});

test("記録からの再開は新しい下書きを保存し処理中には入力を変えない", async () => {
  for (const busy of [false, true]) {
    let listener;
    let saved;
    const input = { value: "前の入力", focus() {}, setSelectionRange() {} };
    class Target { closest() { return { dataset: { action: "resume", prompt: "記録の続き" } }; } }
    const context = vm.createContext({
      input, busy, Element: Target, status: {}, setTimeout() {},
      composer: { hidden: false }, relationshipCard: { hidden: false }, relationshipButton: { setAttribute() {} },
      relationshipMoments: { addEventListener(_, callback) { listener = callback; } },
      setInputValue(value) { input.value = value; }, persistDraft() { saved = input.value; },
      setTemporaryStatus() {}, setRelationshipBusy() {},
    });
    vm.runInContext('const promptHistory = ["前の入力"]; let promptHistoryIndex = 0; let promptDraft = "以前の下書き";', context);
    vm.runInContext(between("function resetPromptHistoryNavigation()", "function openComposer()"), context);
    vm.runInContext(between("function reuseHistoryItem(text)", "// 会話履歴を描き直し"), context);
    vm.runInContext(between('relationshipMoments?.addEventListener("click"', "const relationshipClearConfirmation"), context);
    await listener({ target: new Target() });
    if (busy) {
      assert.equal(input.value, "前の入力");
      assert.equal(saved, undefined);
      assert.equal(context.relationshipCard.hidden, false);
    } else {
      assert.equal(saved, "記録の続き");
      vm.runInContext("recallPromptHistory(-1); recallPromptHistory(1);", context);
      assert.equal(input.value, "記録の続き");
    }
  }
});

test("上下キーは未定義処理を呼ばず履歴と下書きを往復する", () => {
  const input = keyboard();
  assert.equal(press(input, "ArrowUp"), true);
  assert.equal(input.value, "直前の入力");
  assert.equal(press(input, "ArrowDown"), true);
  assert.equal(input.value, "下書き");
});

test("前の録音の遅延結果や失敗は再録音の入力と送信予約を変更しない", async () => {
  for (const mode of ["success", "failure", "empty", "interim-failure", "stale-success", "stale-failure", "stale-encoding"]) {
    let settle;
    let entered;
    const waiting = new Promise(resolve => { entered = resolve; });
    const delayed = new Promise((resolve, reject) => { settle = () => mode.endsWith("failure") ? reject(new Error("文字起こし失敗")) : resolve({ text: mode === "empty" ? "" : "確定した音声" }); });
    const effects = [];
    const input = { value: "次の下書き", focus() {} };
    const context = vm.createContext({
      input, Blob, status: { textContent: "", classList: { add() {}, remove() { effects.push("状態解除"); } } },
      encodeWav() { return new ArrayBuffer(2); },
      blobToDataUrl() { if (mode === "stale-encoding") { entered(); return delayed; } return "音声"; },
      window: { picoagent: { transcribeAudio() { effects.push("変換要求"); entered(); return delayed; } } },
      setInputValue(value) { input.value = value; },
      clearTransientStatus() {}, logApp() {}, errorDetail() {}, preview(value) { return value; },
      showToast() { effects.push("通知"); }, scheduleVoiceSubmit() { effects.push("送信予約"); },
    });
    vm.runInContext('let micStream = {}; let micChunks = [new Float32Array(8000)]; let micSamples = 8000; const micSampleRate = 16000; const micInterimBase = ""; const micInterimTexts = []; let micSession = 0; let micStartRequest = 0; function stopMicCapture() { micStream = null; micSession++; micStartRequest++; }', context);
    if (mode === "interim-failure") vm.runInContext('micInterimTexts.push("途中の音声"); input.value = "途中の音声";', context);
    vm.runInContext(between("async function finishRecording()", "async function startRecording()"), context);
    const pending = vm.runInContext("finishRecording()", context);
    await waiting;
    if (mode.startsWith("stale")) vm.runInContext('micStartRequest++; status.textContent = "次の録音中";', context);
    settle();
    await pending;
    if (mode.startsWith("stale")) {
      assert.equal(input.value, "次の下書き");
      assert.deepEqual(effects, mode === "stale-encoding" ? [] : ["変換要求"]);
      assert.equal(context.status.textContent, "次の録音中");
    } else {
      assert.equal(input.value, mode === "success" ? "確定した音声" : mode === "interim-failure" ? "途中の音声" : "次の下書き");
      assert.deepEqual(effects, ["変換要求", ...(mode === "empty" ? [] : ["通知"]), "状態解除", ...(["success", "interim-failure"].includes(mode) ? ["送信予約"] : [])]);
    }
  }
});

test("前の逐次文字起こしの失敗は次の録音のタイマーを止めない", async () => {
  for (const stale of [false, true]) {
    let reject;
    let entered;
    const waiting = new Promise(resolve => { entered = resolve; });
    const effects = [];
    const context = vm.createContext({
      Blob, encodeWav() { return new ArrayBuffer(2); }, blobToDataUrl() { return "音声"; },
      window: { picoagent: { transcribeAudio() { entered(); return new Promise((_, fail) => { reject = fail; }); } } },
      logApp() { effects.push("失敗通知"); }, errorDetail() {}, clearInterval(timer) { effects.push(timer); },
    });
    vm.runInContext('let micInterimBusy = false; let micSession = 1; const micStream = {}; let micInterimIndex = 0; const micChunks = [new Float32Array(32000)]; const micSampleRate = 16000; let micInterimTimer = "録音1";', context);
    vm.runInContext(between("async function transcribeInterim(session)", "// Float32 の PCM"), context);
    const pending = vm.runInContext("transcribeInterim(1)", context);
    await waiting;
    if (stale) vm.runInContext('micSession = 2; micInterimTimer = "録音2";', context);
    reject(new Error("遅延した失敗"));
    await pending;
    assert.deepEqual(effects, stale ? [] : ["失敗通知", "録音1"]);
    assert.equal(vm.runInContext("micInterimTimer", context), stale ? "録音2" : null);
    assert.equal(vm.runInContext("micInterimBusy", context), false);
  }
});

test("録音の再開はマイク許可を待つ前に以前の送信予約を取り消す", async () => {
  const effects = [];
  const context = vm.createContext({
    input: { focus() {} }, status: { textContent: "" }, micButton: {},
    clearTimeout(timer) { effects.push(timer); }, clearTransientStatus() {}, showToast() {}, stopVoice() {}, logApp() {}, errorDetail() {},
    navigator: { mediaDevices: { async getUserMedia() { effects.push("マイク要求"); throw new Error("テスト用の許可拒否"); } } },
  });
  vm.runInContext('let pendingVoiceSubmitTimer = "前の送信予約"; let micStream = null; let micStarting = false; let micStartRequest = 0; const busy = false;', context);
  vm.runInContext(between("function cancelPendingVoiceSubmit(", "function scheduleVoiceSubmit()"), context);
  vm.runInContext(between("async function startRecording()", "function updateContinuousVoiceButton()"), context);
  await vm.runInContext("startRecording()", context);
  assert.deepEqual(effects, ["前の送信予約", "マイク要求"]);
  assert.equal(vm.runInContext("pendingVoiceSubmitTimer", context), null);
  assert.equal(context.micButton.disabled, false);
});

test("古いマイク許可要求の失敗は新しい録音の所有状態を消さない", async () => {
  let reject;
  const effects = [];
  const context = vm.createContext({
    status: { textContent: "" }, micButton: {}, cancelPendingVoiceSubmit() {}, stopVoice() {}, errorDetail() {},
    logApp() { effects.push("エラー"); }, showToast() { effects.push("通知"); },
    navigator: { mediaDevices: { getUserMedia() { return new Promise((_, fail) => { reject = fail; }); } } },
  });
  vm.runInContext('let micStream = null; let micStarting = false; let micStartRequest = 0; const busy = false;', context);
  vm.runInContext(between("async function startRecording()", "function updateContinuousVoiceButton()"), context);
  const pending = vm.runInContext("startRecording()", context);
  vm.runInContext('micStartRequest++; micStream = { id: "新しい録音" }; micStarting = true;', context);
  reject(new Error("古い要求の許可拒否"));
  await pending;
  assert.equal(vm.runInContext("micStream.id", context), "新しい録音");
  assert.equal(vm.runInContext("micStarting", context), true);
  assert.deepEqual(effects, []);
});

test("文中と選択中の上下キーは通常のカーソル操作を保つ", () => {
  const input = keyboard();
  input.selectionStart = input.selectionEnd = 1;
  assert.equal(press(input, "ArrowUp"), false);
  assert.equal(press(input, "ArrowDown"), false);
  input.selectionStart = 0;
  assert.equal(press(input, "ArrowUp"), false);
  assert.equal(input.value, "下書き");
});

test("IME候補操作と変換確定は履歴・送信へ流さない", () => {
  for (const extra of [{ isComposing: true }, { keyCode: 229 }]) {
    const input = keyboard();
    for (const key of ["ArrowUp", "ArrowDown", "Enter", "Escape"]) assert.equal(press(input, key, extra), false);
    assert.equal(input.value, "下書き");
  }
});

test("修飾キー付きの上下操作は履歴で入力を置き換えない", () => {
  for (const modifier of ["shiftKey", "ctrlKey", "altKey", "metaKey"]) {
    const input = keyboard();
    assert.equal(press(input, "ArrowUp", { [modifier]: true }), false);
    assert.equal(input.value, "下書き");
    press(input, "ArrowUp");
    input.selectionStart = input.selectionEnd = input.value.length;
    assert.equal(press(input, "ArrowDown", { [modifier]: true }), false);
    assert.equal(input.value, "直前の入力");
    assert.equal(press(input, "ArrowDown"), true);
    assert.equal(input.value, "下書き");
  }
});

test("履歴検索のEscapeは画面を閉じず検索だけを解除しIME取消を妨げない", () => {
  const handlers = {};
  let renders = 0;
  let cancelled = 0;
  const historySearch = { value: "検索語", addEventListener(type, handler) { handlers[type] = handler; } };
  const context = vm.createContext({
    historySearch, historyQuery: "検索語",
    isImeComposing: event => event.isComposing || event.keyCode === 229,
    renderHistory() { renders++; },
    clearTimeout() { cancelled++; }, setTimeout() { return 1; },
  });
  vm.runInContext(between("let historySearchTimer = 0;", 'document.addEventListener("keydown", event => {'), context);
  for (const extra of [{ key: "Enter" }, { isComposing: true }, { keyCode: 229 }]) {
    handlers.keydown({ key: "Escape", ...extra, preventDefault() { assert.fail("通常入力を阻害"); }, stopPropagation() { assert.fail("IME取消を阻害"); } });
    assert.equal(historySearch.value, "検索語");
  }
  handlers.input();
  let prevented = false;
  let stopped = false;
  handlers.keydown({ key: "Escape", preventDefault() { prevented = true; }, stopPropagation() { stopped = true; } });
  assert.equal(historySearch.value, "");
  assert.equal(context.historyQuery, "");
  assert.equal(renders, 1);
  assert.equal(cancelled, 2);
  assert.equal(prevented && stopped, true);
});

test("最初の添付のみの送信でも検索結果なしではなく考え中を表示する", () => {
  const node = () => ({ children: [], append(...children) { this.children.push(...children); }, replaceChildren() { this.children = []; } });
  const historyLog = node();
  Object.assign(historyLog, { scrollHeight: 100, scrollTop: 0, clientHeight: 100 });
  const context = vm.createContext({
    historyLog, document: { createElement: node },
    bubble: { classList: { contains: () => false } }, layoutShowsHistory: () => true,
    conversationHistory: [], historyThinking: true, historyQuery: "", historyThinkingText: "考え中",
    characterManifest: { name: "むぎ" }, createTypingIndicator: label => ({ label }),
  });
  vm.runInContext(between("function renderHistoryRows()", "function scheduleAppQuit()"), context);
  context.renderHistoryRows();
  assert.equal(historyLog.children.length, 1);
  assert.equal(historyLog.children[0].className, "history-item assistant history-thinking");
  assert.equal(historyLog.children[0].children[1].children[0].label, "考え中");
  context.historyQuery = "存在しない履歴";
  context.renderHistoryRows();
  assert.equal(historyLog.children[0].className, "history-empty");
  assert.equal(historyLog.children[0].textContent, "一致する履歴はありません。");
  context.historyQuery = "";
  context.historyThinking = false;
  context.renderHistoryRows();
  assert.equal(historyLog.children.some(item => item.className?.includes("history-thinking")), false);
  assert.match(historyLog.children[0].textContent, /まだ会話はありません/);
});

test("会話リセット後に旧質問の再生成や旧回答のコピーと読み上げをしない", () => {
  let reset;
  const context = vm.createContext({
    window: { picoagent: { on: (_, callback) => { reset = callback; } } },
    conversationRevision: 0, speakingToken: 0, conversationHistory: ["古い会話"], promptHistory: ["古い質問"], promptHistoryIndex: 0, promptDraft: "下書き",
    lastUserText: "古い質問", lastAssistantText: "古い回答", lastUserImages: ["画像"], lastUserFiles: ["資料"],
    historyQuery: "古い検索", historySearch: { value: "古い検索" },
    pendingChoices: ["古い選択肢"], setPendingChoices(choices) { context.pendingChoices = choices; },
    reply: { textContent: "古い回答" }, input: { value: "未送信の下書き" },
    streamRenderFrame: 12, streamBuffer: "リセット前の返答", historyThinking: true,
    cancelAnimationFrame(id) { assert.equal(id, 12); }, cancelStreamSpeech() {},
    setThinkingUi(active) { context.historyThinking = active; },
    renderHistory() {}, stopVoice() {}, closeComposer() {}, scheduleChatter() {},
    renderReply(text) { context.reply.textContent = text; },
  });
  vm.runInContext(between('window.picoagent.on("pet:reset",', "// 感情状態が更新されたら"), context);
  vm.runInContext(between("function hasLastRequest()", "// エラー表示後に"), context);
  assert.equal(context.hasLastRequest(), true);
  reset();
  assert.equal(context.hasLastRequest(), false);
  assert.equal(context.lastAssistantText, "");
  assert.equal(context.lastAssistantText || context.reply.textContent, "新しい話をしよう。");
  assert.equal(context.input.value, "未送信の下書き");
  assert.equal(context.conversationHistory.length, 0);
  assert.equal(context.streamRenderFrame, null);
  assert.equal(context.streamBuffer, "");
  assert.equal(context.historyThinking, false);
  assert.equal(context.speakingToken, 1);
  assert.equal(context.pendingChoices, null);
});

test("文字送りの途中で無効化した回答は予約描画も最終描画もしない", async () => {
  let paint;
  let resume;
  let rendered = 0;
  const context = vm.createContext({
    speakingToken: 0, reply: {}, currentVoice: null,
    requestAnimationFrame(callback) { paint = callback; return 1; }, cancelAnimationFrame() {},
    setTimeout(callback) { resume = callback; }, renderMarkdown() { rendered++; },
    setAssistantReply() { rendered++; }, setAnimation() {},
  });
  vm.runInContext(between("async function renderReplySpeaking(text)", "function showStartupReadyMessage"), context);
  const pending = context.renderReplySpeaking("古い回答");
  context.speakingToken++;
  paint();
  resume();
  await pending;
  assert.equal(rendered, 0);
});

test("文字送り完了後もリセット済みの回答は履歴と終了操作へ渡さない", () => {
  const effects = [];
  const context = vm.createContext({
    requestRevision: 0, conversationRevision: 1, jobState: "running",
    result: { reply: "古い回答", quitApp: true },
    addHistory() { effects.push("履歴"); }, scheduleAppQuit() { effects.push("終了"); },
  });
  const source = between("// 文字送りを待っている間に会話がリセットされた場合も", "\n    }\n  } catch (error) {");
  vm.runInContext(`function completeReply() { ${source} }`, context);
  context.completeReply();
  assert.equal(context.jobState, "cancelled");
  assert.deepEqual(effects, []);
  context.conversationRevision = 0;
  context.completeReply();
  assert.deepEqual(effects, ["履歴", "終了"]);
});

test("リセット前の進捗と予約済みストリーム描画は画面へ戻らない", async () => {
  const callbacks = {};
  let frame;
  let renders = 0;
  let activities = 0;
  const context = vm.createContext({
    window: { picoagent: { async listen(name, callback) { callbacks[name] = callback; } } },
    busy: true, requestRevision: 0, conversationRevision: 0,
    status: { textContent: "" }, setThinkingUi() {}, upsertJob() {}, jobId: "検証", setAnimation() {},
    taskRun: { status() {}, activity() { activities++; } }, jobTools: new Set(), jobDidWork: false, jobToolFailed: false,
    streamBuffer: "", historyThinking: false, currentLayout: "chat", feedStreamSpeech() {},
    reply: { classList: { add() {} }, scrollHeight: 100, scrollTop: 0, clientHeight: 100 },
    streamRenderFrame: null, requestAnimationFrame(callback) { frame = callback; return 1; },
    renderMarkdown() { renders++; }, animationName: "idle",
  });
  const registrations = ["unlistenStatus", "unlistenActivity", "unlistenStream"].map(name => {
    const start = renderer.indexOf(`${name} = await window.picoagent.listen(`);
    const end = renderer.indexOf("\n      });", start) + "\n      });".length;
    return renderer.slice(start, end);
  }).join("\n");
  await vm.runInContext(`(async () => { ${registrations} })()`, context);
  callbacks["pet:agent_status"]("処理中");
  callbacks["pet:agent_activity"]({ phase: "started", tools: ["検証"] });
  callbacks["pet:stream_chunk"]("古い返答");
  assert.equal(context.status.textContent, "処理中");
  assert.equal(activities, 1);
  context.conversationRevision++;
  context.status.textContent = "新しい会話";
  callbacks["pet:agent_status"]("古い進捗");
  callbacks["pet:agent_activity"]({ phase: "started", tools: ["古い処理"] });
  frame();
  assert.equal(context.status.textContent, "新しい会話");
  assert.equal(activities, 1);
  assert.equal(renders, 0);
});

test("リセット前の通信失敗は新しい会話に書き戻さず通常の失敗は表示する", () => {
  const marker = renderer.indexOf('logApp("error", "AI応答の取得に失敗しました"');
  const start = renderer.lastIndexOf("} catch (error) {", marker) + "} catch (error) {".length;
  const end = renderer.indexOf("} finally {", marker);
  for (const revision of [0, 1]) {
    const replies = [];
    const history = [];
    const context = vm.createContext({
      requestRevision: 0, conversationRevision: revision, jobState: "running", speakingToken: 0,
      setThinkingUi() {}, logApp() {}, errorDetail: String, cancelStreamSpeech() {}, stopVoice() {},
      renderReply: text => replies.push(text), addHistory: (_, text) => history.push(text),
      reply: { classList: { add() {} } }, addRetryButton() {}, status: { textContent: "" }, setAnimation() {}, setTimeout() {},
    });
    vm.runInContext(`function handleFailure(error) { ${renderer.slice(start, end)} }`, context);
    context.handleFailure(new Error("接続失敗"));
    assert.equal(context.jobState, revision ? "cancelled" : "failed");
    assert.equal(replies.length, revision ? 0 : 1);
    assert.equal(history.length, revision ? 0 : 1);
    if (!revision) assert.match(replies[0], /接続失敗/);
  }
});

test("再生成は送信控えとAIの準備が揃い処理中でないときだけ有効になる", () => {
  const context = vm.createContext({
    regenerateReplyButton: {}, sendButton: {}, quickActions: null,
    frontendInitialized: false, localEngineRequired: true, localEngineReady: false, busy: false,
    input: { value: "" }, attachedImages: [], attachedFiles: [], lastUserText: "", lastUserImages: [], lastUserFiles: [],
  });
  vm.runInContext(between("function hasLastRequest()", "// エラー表示後に"), context);
  vm.runInContext(between("function updateComposerState()", "function setInputValue"), context);
  const check = disabled => { context.updateComposerState(); assert.equal(context.regenerateReplyButton.disabled, disabled); };
  check(true);
  context.frontendInitialized = context.localEngineReady = true;
  check(true);
  context.lastUserImages = ["添付のみの送信"];
  check(false);
  context.busy = true;
  check(true);
  context.busy = false;
  context.localEngineReady = false;
  check(true);
  context.localEngineRequired = false;
  check(false);
});

test("添付準備中や画像送信後にリセットした質問を新しい会話へ送らない", async () => {
  const source = between("// ── ファイル内容をテキストへ変換してコンテキストに付加", "    if (result?.stale || requestRevision !== conversationRevision)");
  for (const phase of ["開始前", "ファイル読込", "画面取得", "画像送信", "通常送信"]) {
    const calls = [];
    let context;
    const invoke = async name => {
      calls.push(name);
      if (name === phase) context.conversationRevision++;
      return name === "画面取得" ? { dataUrl: "画像" } : undefined;
    };
    context = vm.createContext({
      requestRevision: 0, conversationRevision: phase === "開始前" ? 1 : 0, jobState: "running",
      pendingFiles: [{ name: "資料1" }, { name: "資料2" }], pendingImages: [], text: "質問", status: {},
      isScreenshotQuery: () => phase !== "通常送信", logApp() {},
      window: { picoagent: {
        readFileContent: () => invoke("ファイル読込"), captureScreen: () => invoke("画面取得"),
        sendImage: () => invoke("画像送信"), send: () => { calls.push("送信"); return Promise.resolve({ reply: "回答" }); },
      } },
    });
    await vm.runInContext(`(async () => { ${source} })()`, context);
    assert.equal(calls.includes("送信"), phase === "通常送信");
    assert.equal(context.jobState, phase === "通常送信" ? "running" : "cancelled");
    if (phase === "開始前") assert.deepEqual(calls, []);
    if (phase === "ファイル読込") assert.deepEqual(calls, ["ファイル読込"]);
    if (phase === "画面取得") assert.equal(calls.includes("画像送信"), false);
  }
});

test("コードコピーは成功・拒否・非対応を区別し再操作できる", async () => {
  for (const outcome of [true, false, new Error("拒否")]) {
    let finish;
    let calls = 0;
    const context = vm.createContext({
      async copyText() { calls++; if (outcome instanceof Error) throw outcome; return outcome; },
      logApp() {}, errorDetail: String,
      setTimeout(callback) { finish = callback; },
    });
    vm.runInContext(between("async function copyCodeBlock(", "function renderAlarmReply("), context);
    const button = { textContent: "コピー", disabled: false };
    await context.copyCodeBlock(button, "code");
    assert.equal(button.textContent, outcome === true ? "コピーしました" : "コピーできませんでした");
    await context.copyCodeBlock(button, "code");
    assert.equal(calls, 1);
    finish();
    assert.equal(button.disabled, false);
    assert.equal(button.textContent, "コピー");
  }
});

test("設定のJSONが不正でも保存失敗後に入力と復元を操作できる", async () => {
  const button = () => ({ disabled: false, classList: { add() {}, toggle() {} }, addEventListener(_, callback) { this.click = callback; } });
  const saveBtn = button();
  const resetChangesBtn = button();
  const form = { inert: false, setAttribute() {} };
  let saves = 0;
  const context = vm.createContext({
    saveBtn, resetChangesBtn, form, dirtySummary: button(), statusEl: {},
    collect() { throw new Error("ツール引数は正しいJSONを入力してください"); },
    logApp() {}, errorDetail: String,
    window: { picoagent: { settings: { save() { saves++; } } } },
  });
  vm.runInContext("let renderingSettings = false; let savedSnapshot = {}; let settingsMutationBusy = false;", context);
  vm.runInContext(settings.slice(settings.indexOf("function updateDirtyStateNow()"), settings.indexOf("// collect()と全設定")), context);
  vm.runInContext("const dirtyStateScheduler = { flush: updateDirtyStateNow };", context);
  vm.runInContext(settings.slice(settings.indexOf("function setSettingsMutationBusy("), settings.indexOf("function nodeSearchText(")), context);
  vm.runInContext(settings.slice(settings.indexOf('saveBtn.addEventListener("click"'), settings.indexOf('resetChangesBtn?.addEventListener("click"')), context);
  context.updateDirtyStateNow();
  assert.equal(resetChangesBtn.disabled, false);
  await saveBtn.click();
  assert.equal(saves, 0);
  assert.equal(form.inert, false);
  assert.equal(saveBtn.disabled, false);
  assert.equal(resetChangesBtn.disabled, false);
  assert.match(context.statusEl.textContent, /保存に失敗.*JSON/);
  await saveBtn.click();
  assert.equal(form.inert, false);
});

test("モデル一覧の逆順応答と古い失敗は現在の選択を上書きしない", async () => {
  for (const oldFails of [false, true]) {
    const pending = [];
    const options = [];
    const context = vm.createContext({
      modelInput: { value: "初期値" }, modelOptions: { set innerHTML(_) { options.length = 0; }, append(node) { options.push(node.value); } },
      statusEl: {}, data: { active: { providerId: "b" } },
      providerDefaultModel: id => id, el: (_, value) => value,
      updateDirtyState() {}, logApp() {}, errorDetail: String,
      window: { picoagent: { settings: { listModels() { return new Promise((resolve, reject) => pending.push({ resolve, reject })); } } } },
    });
    vm.runInContext("let modelLoadGeneration = 0; const cliModelSelect = null;", context);
    vm.runInContext(settings.slice(settings.indexOf("async function loadModels("), settings.indexOf("function collect()")), context);
    const old = context.loadModels("a", true);
    const current = context.loadModels("b", true);
    pending[1].resolve(["b"]);
    await current;
    if (oldFails) pending[0].reject(new Error("古い接続失敗"));
    else pending[0].resolve(["a"]);
    await old;
    assert.equal(context.modelInput.value, "b");
    assert.deepEqual(options, ["b"]);
    assert.equal(context.statusEl.textContent, "");
    const manual = context.loadModels("b", true);
    context.modelInput.value = "手入力モデル";
    pending[2].resolve(["b"]);
    await manual;
    assert.equal(context.modelInput.value, "手入力モデル");
  }
});

test("非表示へ切り替えたAIの入力は保存収集でも保持し復号不能秘密を消さない", () => {
  const inputs = new Map([
    ["API_KEY", { type: "password", value: "編集中のキー" }],
    ["BROKEN_KEY", { type: "password", value: "" }],
  ]);
  const context = vm.createContext({
    inputs, cloneSettings: value => JSON.parse(JSON.stringify(value)),
    data: { values: {}, unavailableSecrets: [{ key: "BROKEN_KEY" }] },
  });
  vm.runInContext(`let providerDraftValues = {}; const savedSnapshot = {};
    const PRESERVE_UNREADABLE_SECRET = "keep";
    const characterSelect = null, characterPersona = null, characterSpeaker = null, characterLive2dSdkSelect = null;
    const characterOverrides = {}, characterVoiceOverrides = {}, characterLive2dSdkOverrides = {}, characterFraming = {}, characterStreamProfiles = {};
    const alarmEditorState = null, scheduleEditorState = null;`, context);
  vm.runInContext(settings.slice(settings.indexOf("function rememberProviderDraft("), settings.indexOf("function cloneSettings(")), context);
  vm.runInContext(settings.slice(settings.indexOf("function collect()"), settings.indexOf("// 再起動の確認画面")), context);
  context.rememberProviderDraft({ querySelectorAll: () => [...inputs.keys()].map(settingKey => ({ dataset: { settingKey } })) });
  inputs.clear();
  const patch = context.collect();
  assert.equal(patch.API_KEY, "編集中のキー");
  assert.equal(patch.BROKEN_KEY, "keep");
  assert.match(settings, /providerDraftValues\[f\.key\] \?\? v\[f\.key\]/);
  assert.match(settings, /async function render\(\) \{\s*\+\+modelLoadGeneration;\s*providerDraftValues = \{\};/);
});

test("設定保存は変更差分だけを送り内部状態を競合対象にしない", () => {
  const context = vm.createContext({});
  vm.runInContext(`const PRESERVE_UNREADABLE_SECRET = "keep";
    const SETTINGS_RUNTIME_KEYS = new Set(["PICOAGENT_WINDOW_LAYOUTS", "PICOAGENT_SCHEDULED_TASK_STATE"]);`, context);
  vm.runInContext(settings.slice(settings.indexOf("function stableStringify("), settings.indexOf("function updateDirtyStateNow(")), context);
  const request = context.settingsChangeRequest(
    { SAME: "x", CHANGE: "new", SECRET: "keep", NEW_KEY: "value", PICOAGENT_WINDOW_LAYOUTS: { pet: {} } },
    { SAME: "x", CHANGE: "old", SECRET: "", PICOAGENT_WINDOW_LAYOUTS: { pet: { width: 1 } } },
    { SAME: "r1", CHANGE: "r2", SECRET: "r3", PICOAGENT_WINDOW_LAYOUTS: "runtime" },
  );
  assert.deepEqual({ ...request.patch }, { CHANGE: "new", NEW_KEY: "value" });
  assert.equal(request.baseRevisions.CHANGE, "r2");
  assert.equal(request.baseRevisions.NEW_KEY, "missing");
  assert.equal(request.baseRevisions.SAME, undefined);
  assert.equal(request.baseRevisions.PICOAGENT_WINDOW_LAYOUTS, undefined);
  assert.match(bridge, /save: \(patch, baseRevisions\) => call\("settings_save", \{ patch, baseRevisions \}\)/);
});

test("設定競合は草稿を捨てず最新値へ載せ直す", () => {
  const saveFlow = settings.slice(
    settings.indexOf('saveBtn.addEventListener("click"'),
    settings.indexOf('resetChangesBtn?.addEventListener("click"'),
  );
  assert.match(saveFlow, /const draft = cloneSettings\(request\.patch\)/);
  assert.match(saveFlow, /const latest = await window\.picoagent\.settings\.load\(\)/);
  assert.match(saveFlow, /data\.values = \{ \.\.\.cloneSettings\(latest\.values\), \.\.\.draft \}/);
  assert.match(saveFlow, /savedSnapshot = latestSnapshot/);
  assert.match(saveFlow, /入力内容を確認して、もう一度保存してください/);
});

test("変更判定のcollectはキャラ草稿を書き換えない", () => {
  const collect = settings.slice(settings.indexOf("function collect()"), settings.indexOf("// 再起動の確認画面"));
  assert.doesNotMatch(collect, /saveCurrentCharacter/);
  assert.match(settings, /saveCurrentCharacterPersona\(displayedCharacterId\)/);
  assert.match(settings, /saveCurrentCharacterVoice\(displayedCharacterId\)/);
  assert.match(settings, /saveCurrentCharacterLive2dSdk\(displayedCharacterId\)/);
});

test("停止要求のあと応答が返らなくても画面の待機を解除できる", () => {
  // 停止を押したあと実行中の送信が返らないと、待機表示が残って入力も再送もできず、
  // 復帰する手段が無くなる。一定時間で画面側だけ中断扱いにして戻す。
  assert.match(renderer, /const CANCEL_RELEASE_MS = \d+/);
  const cancel = renderer.match(/async function requestAgentCancellation\(button\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.ok(cancel.indexOf("cancelReleaseTimer = setTimeout(") < cancel.indexOf("await window.picoagent.cancel()"),
    "cancel IPC自体が返らない場合にも備え、待機解除の見張りを先に開始してください");
  assert.match(cancel, /releasePendingSend\(\{ stale: true, releasedByCancel: true \}\)/);
  // 送信は打ち切りの合図と競走させ、既存の「中断扱い」経路（stale）で後始末する。
  assert.match(renderer, /const releasedByCancel = new Promise\(resolve => \{ releasePendingSend = resolve; \}\)/);
  assert.match(renderer, /Promise\.race\(\[window\.picoagent\.send\(fullText\), releasedByCancel\]\)/);
  assert.match(renderer, /result\?\.releasedByCancel\s*\n?\s*\? "停止しました/);
  // 見張りは送信の後始末で必ず解く（次の送信へ持ち越さない）。
  assert.match(renderer, /clearCancelRelease\(\);\s*\n\s*busy = false;/);
});

test("窓に収まらないフキダシ保存値は表示側で収める", () => {
  // 広い画面で広げたフキダシの保存値を狭い窓へそのまま当てると、入力欄と送信ボタンが
  // 窓の外へ出て操作できなくなる。保存値は変えず、当てる寸法だけ窓へ収める。
  assert.match(renderer, /function fitBubbleSizeToWindow\(width, height\) \{[\s\S]*?Math\.min\(width, maxWidth\)[\s\S]*?Math\.min\(height, maxHeight\)/);
  const apply = renderer.match(/function applyBubbleLayout\(width, height, left, top\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.match(apply, /bubble\.style\.width = `\$\{fitBubbleSizeToWindow\(w, h\)\.width\}px`/);
  assert.match(apply, /bubble\.style\.height = `\$\{fitBubbleSizeToWindow\(w, h\)\.height\}px`/);
  // 窓が広がれば保存どおりへ戻す。つまみ操作中と拡大中は触らない。
  const refit = renderer.match(/function refitBubbleToWindow\(\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.match(refit, /bubble\.classList\.contains\("fullscreen"\) \|\| resizeStart/);
  assert.match(refit, /savedBubbleLayout\?\.width/);
  assert.match(renderer, /window\.addEventListener\("resize", \(\) => \{\s*\n\s*refitBubbleToWindow\(\);/);
});

test("ダブルクリックでは全画面へ切り替えず拡大ボタンだけを使う", () => {
  assert.doesNotMatch(renderer, /addEventListener\("dblclick"/);
  assert.match(renderer, /toggleFullscreenButton\?\.addEventListener\("click"[\s\S]*?toggleBubbleFullscreen\(\)/);
});

test("準備中・失敗の理由は入力や読み上げ操作で消さない", () => {
  // SPEC: `error` は診断・修復へ進める理由として常時表示する。入力のたびに状態表示を
  // 空にすると、赤い空欄と無効な送信ボタンだけが残り、送れない理由が分からなくなる。
  assert.match(renderer, /let localEngineStatusText = ""/);
  const clear = renderer.match(/function clearTransientStatus\(\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.match(clear, /localEngineStatusText && !localEngineReady \? localEngineStatusText : ""/);
  // 文字数表示は消す側の入口を通す（直接空にしない）。
  assert.match(renderer, /if \(len > 100\) status\.textContent = `\$\{len\}文字`;\s*\n\s*else clearTransientStatus\(\);/);
  // 準備中・失敗の文言は applyEngineStatus が控える。
  const apply = renderer.match(/function applyEngineStatus\(s\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.match(apply, /localEngineStatusText = "";/);
  assert.match(apply, /準備に失敗: \$\{s\.message \|\| "不明なエラー"\}`\);\s*\n\s*localEngineStatusText = status\.textContent;/);
});

test("一時表示を消すすべての経路でローカルAIの失敗理由へ戻す", () => {
  const helperStart = renderer.indexOf("function clearTransientStatus()");
  const helperEnd = renderer.indexOf("\n}", helperStart) + 2;
  assert.ok(helperStart >= 0 && helperEnd > helperStart);
  const withoutHelper = renderer.slice(0, helperStart) + renderer.slice(helperEnd);
  assert.doesNotMatch(withoutHelper, /status\.textContent\s*=\s*""/,
    "状態表示を空にせず clearTransientStatus() で失敗理由へ戻してください");
});

test("フキダシの自動配置は配置基準の枠内へ収める", () => {
  // 位置はフキダシの配置基準（.pet-app）からの座標で決まる。ウィンドウ全体の高さで
  // 上限を計算すると上端メニューぶん（body の padding-top 32px）だけ緩くなり、
  // フキダシが窓の下へはみ出して状態表示が切れる。
  const place = renderer.match(/function placeBubbleAwayFromCharacter\([\s\S]*?\n\}/)?.[0] || "";
  assert.match(place, /const container = bubble\.offsetParent instanceof HTMLElement \? bubble\.offsetParent : document\.documentElement/);
  assert.match(place, /const viewportWidth = container\.clientWidth/);
  assert.match(place, /const viewportHeight = container\.clientHeight/);
  assert.doesNotMatch(place, /document\.documentElement\.clientHeight/);
});
