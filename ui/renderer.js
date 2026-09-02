// バックエンドへ「フロントを読み込めた」ことを最初に伝える。これが届かないと
// boot_screen が読み込み失敗と判断し、キャラクター付きの案内画面へ切り替える。
window.picoagent?.frontendReady?.();

const petApp = document.getElementById("petApp");
const betaBadge = document.querySelector(".beta-badge");
window.__picoagentPerformance = { startedAt: performance.now() };
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
const characterSpringMotion = document.getElementById("characterSpringMotion");
const characterView = document.getElementById("characterView");
const online = document.getElementById("online");
const relationshipButton = document.getElementById("relationshipButton");
const relationshipCard = document.getElementById("relationshipCard");
const relationshipLabel = document.getElementById("relationshipLabel");
const relationshipStats = document.getElementById("relationshipStats");
const relationshipMoments = document.getElementById("relationshipMoments");
const relationshipClear = document.getElementById("relationshipClear");
const variantRow = document.getElementById("variantRow");
const variantSelect = document.getElementById("variantSelect");
const onboarding = document.getElementById("onboarding");
const onboardingComponentStep = document.getElementById("onboardingComponentStep");
const onboardingCharacterStep = document.getElementById("onboardingCharacterStep");
const onboardingComponentStatus = document.getElementById("onboardingComponentStatus");
const bubbleResize = document.getElementById("bubbleResize");
const imagePreview = document.getElementById("imagePreview");
const attachmentSuggestions = document.getElementById("attachmentSuggestions");
const quickActions = document.getElementById("quickActions");
const windowResizeHandles = document.querySelectorAll("[data-resize-direction]");
const bootScreen = document.getElementById("bootScreen");
const topWindowMenu = document.getElementById("topWindowMenu");
let busy = false;
const { errorDetail, logApp, preview, twoStepAction } = window.PicoUI;

// decorations:false の透明ウィンドウにはOSの枠が無いため、各辺と四隅を明示的な
// リサイズ領域にする。幅だけ・高さだけ・縦横同時のいずれもOS標準操作で変更できる。
windowResizeHandles.forEach(handle => {
  handle.addEventListener("pointerdown", event => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    window.picoagent.startWindowResize(handle.dataset.resizeDirection).catch(error => {
      logApp("warn", "ウィンドウのサイズ変更を開始できませんでした", errorDetail(error));
    });
  });
});

function beginWindowDrag(event) {
  if (event.button !== 0) return;
  if (event.target instanceof Element && event.target.closest("button,input,textarea,select,a,[contenteditable]")) return;
  event.preventDefault();
  window.picoagent.startWindowDrag().catch(error => {
    logApp("warn", "ウィンドウの移動を開始できませんでした", errorDetail(error));
    showToast("画面を移動できませんでした。右クリックの「画面位置を初期化」も試してください", "error");
  });
}

// `data-tauri-drag-region` だけに依存すると、透明WebViewや子要素の構成によって
// Windowsでドラッグ開始が届かないことがある。起動画面と常設上部バーは明示的に渡す。
bootScreen?.addEventListener("pointerdown", beginWindowDrag);
topWindowMenu?.addEventListener("pointerdown", event => {
  beginWindowDrag(event);
});
petApp?.addEventListener("pointerdown", event => {
  if (event.target === petApp) beginWindowDrag(event);
});
// 添付中の画像（data URL の配列）。送信後にリセットする。
let attachedImages = [];
// 添付中のドキュメントファイル（{name, dataUrl} の配列）。送信後にリセットする。
let attachedFiles = [];
// エラーリトライ用に最後のユーザー入力テキストを保持する。
let lastUserText = "";
// エージェント進捗（pet:agent_status）を受信するリスナー解除関数。
let unlistenStatus = null;
let unlistenActivity = null;
// ストリーミング受信中のテキストバッファ・リスナー解除関数・描画スロットル用 rAF ハンドル。
let streamBuffer = "";
let unlistenStream = null;
let unlistenStreamReset = null;
let streamRenderFrame = null;
let subtitlePageToken = 0;
let subtitlePageTimer = null;
let localEngineRequired = false;
let localEngineReady = true;
let localEnginePhase = "ready";
let frontendInitialized = false;
const conversationHistory = [];
let historyQuery = "";
let conversationRevision = 0;
const promptHistory = [];
let promptHistoryIndex = null;
let promptDraft = "";
// 既定のクイックアクション。設定 PICOAGENT_QUICK_ACTIONS（1行1件「ラベル|本文」）で置き換え可能。
let quickActionItems = null; // null = 既定を使用
let pendingChoices = null; // ask_user の選択肢（null = 通常のクイック操作を表示）
function parseQuickActions(text) {
  const items = String(text ?? "")
    .split("\n")
    .map(line => line.trim())
    .filter(Boolean)
    .map(line => {
      const sep = line.indexOf("|");
      if (sep <= 0) return null;
      const label = line.slice(0, sep).trim();
      const body = line.slice(sep + 1).trim();
      return label && body ? { label, text: body } : null;
    })
    .filter(Boolean);
  return items.length ? items : null;
}
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
let animationLoop = null;
let bootLogTimer = null;
let lastBootStatus = "";
let lastAssistantText = "";
let voiceVolume = 1;
let relationshipState = null;
let relationshipMemoryEnabled = true;
const NOTICE_STORAGE_KEY = "picoagent.actionCenter.v1";
function loadStoredNotices() {
  try {
    const parsed = JSON.parse(localStorage.getItem(NOTICE_STORAGE_KEY) || "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.slice(0, 50).map(entry => {
      if (entry?.kind !== "job" || entry.status !== "running") return entry;
      return { ...entry, status: "failed", ok: false, body: "アプリ終了により処理結果を確認できませんでした。" };
    });
  } catch {
    // フォールバック許可: 破損したUIキャッシュは空のアクションセンターとして再構築する。
    return [];
  }
}
const noticeEntries = loadStoredNotices();
const speechNoticeQueue = [];
let unreadNoticeCount = 0;
// 目覚まし時計の発火管理（同じ分に二重で鳴らさないための済みキー集合）。
const ALARM_POLL_MS = 20000;
let alarmTimer = null;
const firedAlarms = new Set();
let activeAlarmSnooze = null;
let alarmCheckInFlight = false;

// 画面下部にトースト通知を表示する。必要な場合は次の操作へ進むボタンも付けられる。
// kind: "info"（既定）/ "success" / "warn" / "error" で配色が変わる。
function showToast(msg, kind = "info", options = {}) {
  // 失敗の通知は成功と同じ速さで消さない。1.8秒では、原因を含む文が読み終わる前に
  // 消えてしまい「何か起きたのは分かるが理由は分からない」で終わる。
  const defaultDuration = kind === "error" ? 5000 : 1800;
  const duration = Math.max(1200, Number(options.duration) || defaultDuration);
  const el = document.createElement("div");
  el.className = kind === "info" ? "toast" : `toast toast-${kind}`;
  const text = document.createElement("span");
  text.textContent = msg;
  el.append(text);
  let timer = null;
  const dismiss = () => {
    if (timer) clearTimeout(timer);
    el.classList.remove("toast-show");
    el.addEventListener("transitionend", () => el.remove(), { once: true });
    setTimeout(() => el.remove(), 300);
  };
  if (options.actionLabel && typeof options.onAction === "function") {
    el.classList.add("toast-actionable");
    const action = document.createElement("button");
    action.type = "button";
    action.className = "toast-action";
    action.textContent = options.actionLabel;
    action.addEventListener("click", async () => {
      action.disabled = true;
      try { await options.onAction(); }
      catch (error) {
        logApp("error", "通知アクションに失敗しました", errorDetail(error));
        showToast(`操作できませんでした: ${preview(error?.message || error, 60)}`, "error");
      }
      dismiss();
    });
    el.append(action);
  }
  // クリックで閉じる案は採らない。`.toast` は `pointer-events:none` で、透明な
  // ペットウィンドウの下にあるデスクトップへクリックを通すためにそうしている。
  // 受け取るようにすると、通知が出ている間だけ画面下部が押せない帯になる。
  document.body.append(el);
  requestAnimationFrame(() => el.classList.add("toast-show"));
  timer = setTimeout(dismiss, duration);
}

// 目覚まし・通知のように裏で繰り返し確認している機能は、失敗しても画面へ何も出ないと
// 「登録したのに動かない」という結果だけが利用者に残る。ログは利用者が見る場所ではない。
// 同じ理由で毎回（20秒ごとに）出すとうるさいので、原因ごとに1度だけ知らせ、
// 復旧したら次の失敗はまた知らせる。
const notifiedBackgroundFailures = new Set();
function notifyBackgroundFailure(key, message, error) {
  logApp("warn", message, errorDetail(error));
  if (notifiedBackgroundFailures.has(key)) return;
  notifiedBackgroundFailures.add(key);
  showToast(message, "error", { duration: 8000 });
}
function clearBackgroundFailure(key) {
  notifiedBackgroundFailures.delete(key);
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

window.addEventListener("error", event => {
  logApp("error", "未捕捉フロントエラー", `${event.message || ""} ${event.filename || ""}:${event.lineno || ""}:${event.colno || ""}\n${errorDetail(event.error)}`);
});
window.addEventListener("unhandledrejection", event => {
  logApp("error", "未処理 Promise rejection", errorDetail(event.reason));
});
window.addEventListener("beforeunload", () => {
  document.removeEventListener("mousemove", onGazeMouseMove);
  reduceMotionPreference.removeEventListener("change", onReduceMotionChange);
  if (lookResetTimer) { clearTimeout(lookResetTimer); lookResetTimer = 0; }
  if (chatterTimer) clearTimeout(chatterTimer);
  if (dailyTimer) clearInterval(dailyTimer);
  if (alarmTimer) clearInterval(alarmTimer);
  if (bootLogTimer) clearInterval(bootLogTimer);
  if (continuousVoiceTimer) clearTimeout(continuousVoiceTimer);
  if (pendingVoiceSubmitTimer) clearTimeout(pendingVoiceSubmitTimer);
  if (pendingTouchTimer) clearTimeout(pendingTouchTimer);
  if (pushToTalkTimer) clearTimeout(pushToTalkTimer);
  if (historySearchTimer) clearTimeout(historySearchTimer);
  if (napTimer) clearInterval(napTimer);
  if (widgetClockTimer) { clearInterval(widgetClockTimer); widgetClockTimer = null; }
  if (streamRenderFrame) { cancelAnimationFrame(streamRenderFrame); streamRenderFrame = null; }
  if (unlistenStatus) { unlistenStatus(); unlistenStatus = null; }
  if (unlistenActivity) { unlistenActivity(); unlistenActivity = null; }
  if (unlistenStream) { unlistenStream(); unlistenStream = null; }
  if (unlistenStreamReset) { unlistenStreamReset(); unlistenStreamReset = null; }
  stopVoice(false);
  stopRig();
  if (micStream) stopMicCapture();
  const closingAudioCtx = audioCtx;
  audioCtx = null;
  closingAudioCtx?.close().catch(error => {
    logApp("warn", "終了時に音声解析を解放できませんでした", errorDetail(error));
  });
  stopLive2D();
  destroyMeshRig();
  animationLoop?.stop();
  gazeFrameScheduler?.cancel();
  if (live2dApp) { live2dApp.destroy(true); live2dApp = null; live2dCanvas = null; }
});

const characterManifestCacheKey = "2026-07-03-live2d-jp-voice-v1";
let characterRoot = "assets/characters/mugi";
let characterManifest = null;
let characterOverridesCache = {};
let live2dSdkOverridesCache = {};
let activeLive2dSdkPreference = "auto";
let characterAssetVersion = "";
let runtimeLayers = {};
let bootCharacterName = "キャラクター";
let loadedCharacterSignature = "";
let animations = {
  idle: { path: "animations/idle", frames: 6, fps: 3.6, motion: "breathe" },
  thinking: { path: "animations/running", frames: 6, fps: 6.2, motion: "thinking" },
  talking: { path: "animations/review", frames: 6, fps: 5.4, motion: "talking" },
  sad: { path: "animations/failed", frames: 8, fps: 4.6, motion: "sad" },
};
const reduceMotionPreference = window.matchMedia("(prefers-reduced-motion: reduce)");
let reduceMotion = reduceMotionPreference.matches;
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
  noticeToggle.classList.toggle("has-notices", noticeEntries.length > 0);
}

function persistNotices() {
  try { localStorage.setItem(NOTICE_STORAGE_KEY, JSON.stringify(noticeEntries.slice(0, 50))); }
  catch (error) { logApp("warn", "アクションセンターの保存に失敗しました", errorDetail(error)); }
}
persistNotices();

async function requestAgentCancellation(button) {
  button.disabled = true;
  setStatus("停止を要求しています…");
  try {
    await window.picoagent.cancel();
  } catch (error) {
    logApp("warn", "中断要求の送信に失敗しました", errorDetail(error));
    setStatus(`停止を要求できませんでした: ${error?.message || error}`);
    button.disabled = false;
  }
}

function renderNoticePanel() {
  if (!noticeList) return;
  noticeList.replaceChildren();
  if (noticeEntries.length === 0) {
    noticeList.append(document.createElement("div"));
    noticeList.firstElementChild.className = "notice-empty";
    noticeList.firstElementChild.textContent = "通知はありません";
    return;
  }
  for (const notice of noticeEntries) {
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
    if (notice.kind === "job") {
      const state = document.createElement("span");
      state.className = `notice-state notice-state-${notice.status || "completed"}`;
      state.textContent = notice.status === "running" ? "実行中" : notice.status === "failed" ? "失敗" : notice.status === "cancelled" ? "中断" : "完了";
      title.append(state);
    }
    if (notice.kind === "durable_job") {
      const state = document.createElement("span");
      state.className = `notice-state notice-state-${durableStatus(notice.state)}`;
      state.textContent = DURABLE_STATE_LABELS[notice.state] || "処理中";
      title.append(state);
      // 進行中だけ進捗を出す。終わったジョブに 100% の帯を残しても情報が増えない。
      if (durableStatus(notice.state) === "running") {
        const bar = document.createElement("div");
        bar.className = "notice-progress";
        const fill = document.createElement("div");
        fill.className = "notice-progress-fill";
        fill.style.width = `${Math.max(0, Math.min(100, Number(notice.progress) || 0))}%`;
        bar.append(fill);
        item.append(bar);
      }
    }
    if (notice.kind === "schedule" || notice.taskId || notice.kind === "job" || notice.kind === "durable_job" || notice.kind === "project_rule") {
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
      if (notice.kind === "job" && notice.status === "running") {
        const cancel = document.createElement("button");
        cancel.type = "button";
        cancel.textContent = "停止";
        cancel.addEventListener("click", event => {
          event.stopPropagation();
          requestAgentCancellation(cancel);
        });
        actions.append(cancel);
      } else if (notice.kind === "job" && notice.prompt) {
        const retry = document.createElement("button");
        retry.type = "button";
        retry.textContent = "もう一度";
        retry.addEventListener("click", event => {
          event.stopPropagation();
          if (busy) return;
          setInputValue(notice.prompt);
          if (composer.hidden) openComposer();
          input.focus();
        });
        actions.append(retry);
      }
      if (notice.kind === "durable_job" && notice.jobId) {
        const running = durableStatus(notice.state) === "running";
        const action = document.createElement("button");
        action.type = "button";
        action.textContent = running ? "中止" : "再試行";
        action.addEventListener("click", async event => {
          event.stopPropagation();
          action.disabled = true;
          try {
            if (running) await window.picoagent.cancelDurableJob(notice.jobId);
            else await window.picoagent.retryDurableJob(notice.jobId);
            setStatus(running ? "ジョブを中止しました" : "ジョブを再試行します");
          } catch (error) {
            // 失敗を黙って飲まない（押したのに何も起きない状態を作らない）。
            setStatus(`${running ? "中止" : "再試行"}できませんでした: ${error?.message || error}`);
            action.disabled = false;
          }
        });
        actions.append(action);
      }
      if (notice.kind === "project_rule" && notice.rule) {
        const undo = document.createElement("button");
        undo.type = "button";
        undo.textContent = "取り消し";
        undo.addEventListener("click", async event => {
          event.stopPropagation();
          undo.disabled = true;
          try {
            const removed = await window.picoagent.undoProjectRule(notice.rule);
            setStatus(removed ? "AGENTS.mdから取り消しました" : "対象の行は見つかりませんでした");
            if (removed) undo.textContent = "取り消し済み";
            else undo.disabled = false;
          } catch (error) {
            setStatus(`取り消せませんでした: ${error?.message || error}`);
            undo.disabled = false;
          }
        });
        actions.append(undo);
      }
      const settings = document.createElement("button");
      settings.type = "button";
      settings.textContent = "設定";
      settings.addEventListener("click", async event => {
        event.stopPropagation();
        settings.disabled = true;
        try { await window.picoagent.openSettings(); }
        catch (error) { setStatus(`設定を開けませんでした: ${error?.message || error}`); }
        finally { settings.disabled = false; }
      });
      actions.append(settings);
      item.append(actions);
    }
    noticeList.append(item);
  }
}

// 永続ジョブ（AI Drive転送・成果物生成）の状態表示。バックエンドの JobState と対応する。
const DURABLE_STATE_LABELS = {
  queued: "順番待ち",
  running: "実行中",
  completed: "完了",
  failed: "失敗",
  waiting_auth: "認証待ち",
  cancelled: "中断",
};

// 中止ボタンを出すかどうかの判定。待機中もまだ止められる。
function durableStatus(state) {
  return state === "queued" || state === "running" ? "running" : String(state || "completed");
}

function durableEntry(notice) {
  return {
    kind: "durable_job",
    jobId: String(notice?.jobId || ""),
    title: String(notice?.title || "バックグラウンド処理"),
    body: String(notice?.body || ""),
    ok: notice?.ok !== false,
    state: String(notice?.state || "running"),
    progress: Number(notice?.progress) || 0,
    at: notice?.at || new Date().toISOString(),
  };
}

// 永続ジョブは同じ jobId で何度も届く（開始・進捗・完了）。行を増やさず上書きする。
// 未読バッジも、状態が変わったときだけ増やす（進捗のたびに数えると数字が暴れる）。
function upsertDurableJob(notice) {
  const entry = durableEntry(notice);
  if (!entry.jobId) return;
  const index = noticeEntries.findIndex(item => item.kind === "durable_job" && item.jobId === entry.jobId);
  const previousState = index >= 0 ? noticeEntries[index].state : "";
  if (index >= 0) noticeEntries.splice(index, 1);
  noticeEntries.unshift(entry);
  while (noticeEntries.length > 50) noticeEntries.pop();
  persistNotices();
  if (previousState !== entry.state) {
    unreadNoticeCount += 1;
    setStatus(entry.ok ? entry.title : `${entry.title}: エラー`);
  }
  updateNoticeBadge();
  renderNoticePanel();
}

// 起動直後に、前回から残っている永続ジョブを一覧へ戻す。
// イベントは起動前に流れた分を受け取れないため、走ったままのジョブが消えてしまう。
async function loadDurableJobNotices() {
  try {
    const jobs = await window.picoagent.durableJobs();
    if (!Array.isArray(jobs)) return;
    const others = noticeEntries.filter(entry => entry.kind !== "durable_job");
    const restored = jobs.slice(0, 20).map(job => durableEntry({
      jobId: job.id,
      title: job.label,
      body: (job.error || job.result || job.step || "").split("\n")[0],
      ok: job.state !== "failed" && job.state !== "waiting_auth",
      state: job.state,
      progress: job.progress,
      at: job.updated_at,
    }));
    noticeEntries.length = 0;
    noticeEntries.push(...others, ...restored);
    noticeEntries.sort((a, b) => String(b.at).localeCompare(String(a.at)));
    persistNotices();
    updateNoticeBadge();
    renderNoticePanel();
  } catch (error) {
    logApp("warn", "永続ジョブ一覧を復元できませんでした", errorDetail(error));
  }
}

function pushNotice(notice) {
  if (String(notice?.kind || "") === "durable_job" && notice?.jobId) {
    upsertDurableJob(notice);
    return;
  }
  const entry = {
    kind: String(notice?.kind || ""),
    title: String(notice?.title || "通知"),
    body: String(notice?.body || ""),
    ok: notice?.ok !== false,
    taskId: notice?.taskId ? String(notice.taskId) : "",
    runId: notice?.runId ? String(notice.runId) : "",
    jobId: notice?.jobId ? String(notice.jobId) : "",
    rule: notice?.rule ? String(notice.rule) : "",
    status: notice?.status ? String(notice.status) : "",
    prompt: notice?.prompt ? String(notice.prompt) : "",
    at: notice?.at || new Date().toISOString(),
  };
  noticeEntries.unshift(entry);
  while (noticeEntries.length > 50) noticeEntries.pop();
  persistNotices();
  unreadNoticeCount += 1;
  updateNoticeBadge();
  renderNoticePanel();
  setStatus(entry.ok ? entry.title : `${entry.title}: エラー`);
  setTimeout(() => {
    if (status.textContent === entry.title || status.textContent === `${entry.title}: エラー`) status.textContent = "";
  }, 6000);
}

function upsertJob(jobId, patch) {
  let entry = noticeEntries.find(notice => notice.kind === "job" && notice.jobId === jobId);
  if (!entry) {
    entry = { kind: "job", jobId, title: "エージェント処理", body: "開始しました", ok: true, status: "running", prompt: "", at: new Date().toISOString() };
    noticeEntries.unshift(entry);
  }
  Object.assign(entry, patch, { kind: "job", jobId });
  persistNotices();
  renderNoticePanel();
  updateNoticeBadge();
}

async function loadScheduleRunNotices() {
  try {
    const runs = await window.picoagent.scheduleRuns();
    if (!Array.isArray(runs)) return;
    const otherEntries = noticeEntries.filter(entry => entry.kind !== "schedule");
    const scheduleEntries = [];
    for (const run of runs.slice(0, 20)) {
      scheduleEntries.push({
        kind: "schedule",
        title: String(run.title || "スケジュール"),
        body: String(run.body || ""),
        ok: run.ok !== false,
        taskId: run.task_id ? String(run.task_id) : "",
        runId: run.id ? String(run.id) : "",
        at: run.at || "",
      });
    }
    noticeEntries.splice(0, noticeEntries.length, ...scheduleEntries, ...otherEntries);
    while (noticeEntries.length > 50) noticeEntries.pop();
    persistNotices();
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
  bootLog.append(line);
  while (bootLog.children.length > 6) bootLog.firstElementChild?.remove();
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
  const relative = animationFramePath(animation, index);
  const url = `${characterRoot}/${relative}`;
  return characterAssetVersion ? `${url}?v=${encodeURIComponent(characterAssetVersion)}` : url;
}

function animationFramePaths(animation) {
  if (Array.isArray(animation?.framePaths) && animation.framePaths.length) return animation.framePaths;
  const count = Number.isInteger(animation?.frames) && animation.frames > 0 ? animation.frames : 0;
  if (!animation?.path || count === 0) return [];
  return Array.from({ length: count }, (_unused, index) =>
    `${animation.path}/${String(index).padStart(2, "0")}.png`);
}

function animationFramePath(animation, index) {
  const paths = animationFramePaths(animation);
  if (!paths.length) return "preview.png";
  return paths[index % paths.length];
}

function animationFrameCount(animation) {
  return animationFramePaths(animation).length;
}

function versionedAssetUrl(url, version) {
  return version ? `${url}?v=${encodeURIComponent(version)}` : url;
}

// 自分で追加したキャラクター（app_data_dir 配下）を同梱一覧と同じ形へそろえる。
// 同梱分は配信物に焼き込まれていて実行時に増減しないため、追加分だけをバックエンドから取る。
async function userCharacterEntries() {
  try {
    const result = await window.picoagent?.userCharacters?.();
    return (result?.characters || []).map(character => ({
      id: character.id,
      name: character.name,
      description: character.description || "",
      // 追加キャラはファイルパスから配信 URL を作る（同梱分は配信物からの相対パス）。
      root: window.picoagent.assetUrl(character.path),
    }));
  } catch (error) {
    // フォールバック許可: 追加キャラを取れなくても同梱キャラだけで動かす（記録は残す）。
    logApp("error", "追加キャラクターの一覧を取得できませんでした", errorDetail(error));
    return [];
  }
}

const STARTUP_RESOURCE_TIMEOUT_MS = 4000;
const LIVE2D_MODEL_TIMEOUT_MS = 8000;

async function fetchJsonWithTimeout(url, label, timeoutMs = STARTUP_RESOURCE_TIMEOUT_MS) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { cache: "no-store", signal: controller.signal });
    if (!response.ok) throw new Error(`${label}: HTTP ${response.status}`);
    return await response.json();
  } catch (error) {
    if (error?.name === "AbortError") {
      throw new Error(`${label}が${timeoutMs}ミリ秒以内に応答しませんでした`);
    }
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

// 同梱キャラの一覧は配布物へ焼き込まれ、実行中に増減しない。起動経路で何度も
// 取りに行かないよう、最初の1回だけ読んで使い回す。
let characterCatalogPromise = null;
function loadCharacterCatalog() {
  if (!characterCatalogPromise) {
    characterCatalogPromise = fetchJsonWithTimeout(
      `assets/characters/index.json?v=${characterManifestCacheKey}`,
      "キャラクター一覧",
    ).catch(error => {
      characterCatalogPromise = null;
      throw error;
    });
  }
  return characterCatalogPromise;
}

async function loadCharacterManifest(characterId) {
  const catalog = await loadCharacterCatalog();
  const bundled = catalog.characters.map(character => ({
    id: character.id,
    name: character.name,
    description: character.description || "",
    root: `assets/characters/${character.manifest.replace(/\/character\.json$/, "")}`,
  }));
  // 同じ id なら同梱側を優先する（追加分が同梱キャラを乗っ取らないようにする）。
  const known = new Set(bundled.map(character => character.id));
  const entries = bundled.concat((await userCharacterEntries()).filter(character => !known.has(character.id)));
  const entry = entries.find(character => character.id === characterId)
    || entries.find(character => character.id === catalog.default)
    || entries[0];
  if (!entry) throw new Error("利用できるキャラクターがありません");
  const root = entry.root;
  const manifest = await fetchJsonWithTimeout(
    `${root}/character.json?v=${characterManifestCacheKey}`,
    "キャラクター定義",
  );
  window.__picoagentPerformance.manifestReady = performance.now();
  performance.mark("picoagent-character-manifest-ready");
  return { entry, manifest, root };
}

// 起動画面へ寝姿で出せるのは、専用原画を持つ（`boot.useSleepingOnStartup`）同梱キャラだけ。
// IDを直書きすると、キャラを足したときの更新漏れが黙って起きるため一覧から取る。
function startupSleepingCandidates(catalog) {
  return (catalog?.characters || [])
    .filter(character => character.bootSleeping === true && character.id)
    .map(character => character.id);
}

function chooseStartupSleepingCharacter(candidates, randomValue = Math.random()) {
  if (!candidates.length) return "";
  const value = Number.isFinite(Number(randomValue)) ? Number(randomValue) : 0;
  const index = Math.min(
    candidates.length - 1,
    Math.max(0, Math.floor(value * candidates.length)),
  );
  return candidates[index];
}

async function prepareBootCharacter(characterId) {
  if (!bootPetImage) return;
  const { manifest, root } = await loadCharacterManifest(characterId);
  const version = String(manifest.assetVersion || "");
  bootCharacterName = manifest.name || "キャラクター";
  bootPetImage.alt = `起動を待つ${bootCharacterName}`;
  petApp.dataset.bootCharacterId = manifest.id || characterId;
  const dedicatedSleeping = manifest.boot?.useSleepingOnStartup === true
    ? manifest.boot?.sleeping
    : "";
  petApp.dataset.bootArtwork = dedicatedSleeping ? "sleeping" : "idle";
  // 寝姿を起動時にも使うのは専用原画を持つ3キャラだけ。他キャラの preview は
  // 紹介画像や全身コラージュの場合があるため、通常表示と同じ idle 先頭フレームを使う。
  if (dedicatedSleeping) {
    bootPetImage.onerror = () => {
      bootPetImage.onerror = null;
      bootPetImage.src = "assets/characters/_preview-placeholder.png";
    };
    bootPetImage.src = versionedAssetUrl(`${root}/${dedicatedSleeping}`, version);
    return;
  }
  // Live2D の preview.jpg は配布元の紹介画像や全身コラージュの場合があり、実モデルへ
  // 切り替わる一瞬だけ別の絵に見える。Live2D は仮画像を出さず、読み込み完了した
  // 実モデルをそのまま起動画面へ出す。2Dキャラは通常表示と同じ idle 先頭フレームを使う。
  if (manifest.live2d) {
    // 仮画像が無いので、起動画面のキャラ枠は CSS 側で実モデル（.character）へ差し替える。
    petApp.dataset.bootArtwork = "live2d";
    bootPetImage.onerror = null;
    bootPetImage.removeAttribute("src");
    return;
  }
  const bootPath = animationFramePath(manifest.animations?.idle, 0);
  bootPetImage.onerror = () => {
    // プレビュー画像を取得できていないキャラ（setup が警告を出した場合）でも起動画面を壊さない。
    bootPetImage.onerror = null;
    bootPetImage.src = "assets/characters/_preview-placeholder.png";
  };
  bootPetImage.src = versionedAssetUrl(`${root}/${bootPath}`, version);
}

const animationPreloadCache = new Map();
const animationPreloadPromises = new Map();
let deferredAnimationPreload = null;

function cancelDeferredAnimationPreload() {
  if (!deferredAnimationPreload) return;
  if (deferredAnimationPreload.kind === "idle" && typeof cancelIdleCallback === "function") {
    cancelIdleCallback(deferredAnimationPreload.id);
  }
  else clearTimeout(deferredAnimationPreload.id);
  deferredAnimationPreload = null;
}

function clearAnimationPreloads() {
  cancelDeferredAnimationPreload();
  animationPreloadCache.clear();
  animationPreloadPromises.clear();
}

function preloadAnimation(name) {
  const animation = animations[name] || animations.idle;
  if (!animation) return Promise.resolve();
  const pending = [];
  for (let index = 0; index < animationFrameCount(animation); index += 1) {
    const url = frameUrl(name, index);
    if (animationPreloadPromises.has(url)) {
      pending.push(animationPreloadPromises.get(url));
      continue;
    }
    const image = new Image();
    let settle;
    const loaded = new Promise(resolve => {
      let finished = false;
      let timeoutId = null;
      settle = () => {
        if (finished) return;
        finished = true;
        if (timeoutId) clearTimeout(timeoutId);
        image.onload = null;
        image.onerror = null;
        resolve();
      };
      image.onload = settle;
      image.onerror = settle;
      // 壊れたキャッシュやWebView側のイベント欠落が1枚だけあっても、全体の準備完了を塞がない。
      timeoutId = setTimeout(settle, 5000);
    });
    image.src = url;
    // メモリ／ディスクキャッシュから同期完了した画像はloadイベントを待たず完了扱いにする。
    if (image.complete) queueMicrotask(settle);
    animationPreloadCache.set(url, image);
    animationPreloadPromises.set(url, loaded);
    pending.push(loaded);
  }
  return Promise.all(pending);
}

function boundedPreload(promise, timeoutMs = 5000) {
  return new Promise(resolve => {
    let finished = false;
    const finish = () => {
      if (finished) return;
      finished = true;
      clearTimeout(timeoutId);
      resolve();
    };
    const timeoutId = setTimeout(finish, timeoutMs);
    Promise.resolve(promise).then(finish, finish);
  });
}

// 速度優先: 最初に待機モーションを全枚読み、120ms後に残りも一括で温める。
// 状態切替が先に来た場合は setAnimation() がそのモーションを即時先読みする。
function preloadAnimations() {
  clearAnimationPreloads();
  boundedPreload(preloadAnimation("idle")).then(() => {
    window.__picoagentPerformance.idlePreloadReady = performance.now();
    performance.mark("picoagent-idle-preload-ready");
  });
  const warmRest = () => {
    deferredAnimationPreload = null;
    boundedPreload(Promise.all(Object.keys(animations).map(preloadAnimation))).then(() => {
      window.__picoagentPerformance.allPreloadReady = performance.now();
      performance.mark("picoagent-all-preload-ready");
    });
  };
  // requestIdleCallbackはWebViewの継続描画中に呼ばれない環境があり、全件準備が永久に
  // 保留されるため、短いタイマーで開始時刻を確定させる。
  deferredAnimationPreload = { kind: "timer", id: setTimeout(warmRest, 120) };
}

// 未取得の大判PNGへsrcを切り替えると、WebViewがデコード途中の細い帯だけを一瞬描く。
// 先読み用Imageが完了したフレームだけを表示し、冷間起動でも前の完成フレームを保つ。
function displayPreloadedFrame(name, index) {
  const url = frameUrl(name, index);
  const image = animationPreloadCache.get(url);
  if (!image?.complete || image.naturalWidth < 1 || image.naturalHeight < 1) return false;
  if (petSprite.getAttribute("src") !== url) petSprite.src = url;
  return true;
}

// 読み込みが終わったのに寸法を持たないフレーム（404・途中で切れたPNG）は、待っても表示できない。
// 「まだ読込中」と同じ扱いにすると animate() が同じ1枚を送り続け、モーションが以後ずっと止まる。
function isBrokenFrame(name, index) {
  const image = animationPreloadCache.get(frameUrl(name, index));
  return !!image?.complete && (image.naturalWidth < 1 || image.naturalHeight < 1);
}

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

// 読み上げから外す人格プロンプト（`loadCharacter` が選択中キャラのぶんへ入れ替える）。
// blocked は行照合用の正規化済み、inline は1文へ流し込まれた場合に備えた原文。
let personaSpeechBlocklist = { blocked: [], inline: [] };

// `personaText` が人格設定を組み立てるときの項目名。この形の行は話し言葉ではなく
// 設定文なので、短くてもプロンプト混入として扱う（Rust の PERSONA_FIELD_LABELS と対）。
const PERSONA_FIELD_LINE = /^(名前|性格|口調|一人称|ユーザーの呼び方)[:：]/;

// 混入判定のために行を正規化する（空白除去・先頭の箇条書き記号除去・コロンの全角統一）。
// モデルは `- 名前：むぎ` のように整形し直して復唱することがあるため、見た目の差を均す。
// Rust の `normalize_leak_line` と同じ規則。
function normalizeLeakLine(line) {
  return String(line ?? "")
    .replace(/\s+/g, "")
    .replace(/^[-*・>#]+/, "")
    .replace(/：/g, ":");
}

// 人格プロンプトが混ざった返答をモデルが返しても、設定文を声に出さないための照合用。
// 偶然の一致で普通の返答を消さないよう、8文字以上か人格項目行だけを対象にする。
function setPersonaSpeechBlocklist(profile) {
  const raw = String(profile ?? "").split("\n").map(line => line.trim());
  const blocked = raw
    .map(normalizeLeakLine)
    .filter(line => Array.from(line).length >= 8 || PERSONA_FIELD_LINE.test(line));
  personaSpeechBlocklist = {
    // Rust 側が人格の前へ置く見出し。モデルがプロンプト構造ごと復唱したときに拾う。
    blocked: [...new Set([...blocked, normalizeLeakLine("選択中キャラクターの人格設定:")])],
    inline: raw.filter(line => Array.from(line).length >= 12),
  };
}

// 人格プロンプトの混入を読み上げテキストから取り除く。
//
// 表示と履歴は Rust の `remove_prompt_leak` が消すが、フロント側を省略できない理由が2つある。
// ストリーム読み上げは最終回答が整う前の途中テキストを声にすること、挨拶・自発雑談・
// 目覚ましは `prepareSpeech` へ直接来ることの2点。
function stripPersonaLeak(text) {
  let out = String(text ?? "");
  const { blocked, inline } = personaSpeechBlocklist;
  if (!blocked.length) return out;
  // 中身が同じ行を落とす（整形し直された復唱も正規化して照合する）。
  out = out
    .split("\n")
    .filter(line => {
      const normalized = normalizeLeakLine(line);
      return !normalized || !blocked.includes(normalized);
    })
    .join("\n");
  // 改行を潰して1文へ流し込まれた場合に備え、長い行は文中からも取り除く。
  for (const line of inline) out = out.split(line).join("");
  return out;
}

// variant は見た目の切替なので、フレームアニメと Live2D の切替もここで解決する。
// variant.live2d があればそれを使い、`false` なら（キャラ本体が Live2D でも）フレーム表示に戻す。
// これで「同じキャラの絵版と Live2D 版」を1つの character.json で持てる。
function resolveLive2dConfig(manifest, variant) {
  if (variant?.live2d === false) return null;
  return variant?.live2d || manifest.live2d || null;
}

let activeSpringMotion = [];
let springLastAt = 0;
let activeBodyMotion = null;
let springRenderingPaused = true;

function syncSpringMotionFrame() {
  if (!activeSpringMotion.length || live2dActive || meshRigActive) return;
  const source = petSprite.currentSrc || petSprite.src;
  for (const spring of activeSpringMotion) {
    if (source && spring.sprite.src !== source) spring.sprite.src = source;
  }
}

function updateSpringMotionVisibility() {
  if (!characterSpringMotion) return;
  characterSpringMotion.hidden = !activeSpringMotion.length || live2dActive || meshRigActive;
  if (!characterSpringMotion.hidden) syncSpringMotionFrame();
}

function configureSpringMotion(config, legacyConfig = null) {
  if (!characterSpringMotion) return;
  characterSpringMotion.replaceChildren();
  activeSpringMotion = [];
  activeBodyMotion = null;
  characterView?.style.removeProperty("--body-breathe-y");
  const legacy = legacyConfig?.type === "bust" ? [{
    id: "bust", centerX: legacyConfig.centerX, centerY: legacyConfig.centerY,
    radiusX: legacyConfig.radiusX, radiusY: legacyConfig.radiusY,
    drive: Number(legacyConfig.strength || 1) * 0.55,
  }] : [];
  const regions = Array.isArray(config?.regions) ? config.regions : legacy;
  const clamp = (value, minimum, maximum, fallback) => {
    const numeric = Number(value);
    return Number.isFinite(numeric) ? Math.min(maximum, Math.max(minimum, numeric)) : fallback;
  };
  const bodyMotion = config?.bodyMotion;
  if (bodyMotion && typeof bodyMotion === "object") {
    activeBodyMotion = {
      amplitudeY: clamp(bodyMotion.amplitudeY, 0, 4, 1.2),
      frequency: clamp(bodyMotion.frequency, 0.08, 0.8, 0.18),
      phase: clamp(bodyMotion.phase, -6.3, 6.3, 0),
    };
  }
  regions.slice(0, 12).forEach((region, index) => {
    const layer = document.createElement("div");
    layer.className = "character-spring-region";
    layer.style.setProperty("--spring-center-x", `${clamp(region.centerX, 0, 100, 50)}%`);
    layer.style.setProperty("--spring-center-y", `${clamp(region.centerY, 0, 100, 34)}%`);
    layer.style.setProperty("--spring-radius-x", `${clamp(region.radiusX, 3, 48, 16)}%`);
    layer.style.setProperty("--spring-radius-y", `${clamp(region.radiusY, 3, 42, 10)}%`);
    const centerX = clamp(region.centerX, 0, 100, 50);
    const centerY = clamp(region.centerY, 0, 100, 34);
    const radiusX = clamp(region.radiusX, 3, 48, 16);
    const radiusY = clamp(region.radiusY, 3, 42, 10);
    layer.style.left = `${centerX - radiusX}%`;
    layer.style.top = `${centerY - radiusY}%`;
    layer.style.width = `${radiusX * 2}%`;
    layer.style.height = `${radiusY * 2}%`;
    const inferredLayer = String(region.id || "").startsWith("hair-") ? 4
      : String(region.id || "").startsWith("bust-") ? 3
        : String(region.id || "").startsWith("shoulder-") ? 2 : 1;
    layer.style.zIndex = String(clamp(region.zIndex, 1, 8, inferredLayer));
    const sprite = document.createElement("img");
    sprite.alt = "";
    sprite.draggable = false;
    sprite.style.left = `${-((centerX - radiusX) / (radiusX * 2)) * 100}%`;
    sprite.style.top = `${-((centerY - radiusY) / (radiusY * 2)) * 100}%`;
    sprite.style.width = `${10000 / (radiusX * 2)}%`;
    sprite.style.height = `${10000 / (radiusY * 2)}%`;
    layer.append(sprite);
    characterSpringMotion.append(layer);
    activeSpringMotion.push({
      layer, sprite, position: 0, velocity: 0,
      stiffness: clamp(region.stiffness, 4, 80, 22),
      damping: clamp(region.damping, 1, 24, 8),
      drive: clamp(region.drive, 0.05, 2.4, 0.45),
      frequency: clamp(region.frequency, 0.08, 1.2, 0.24),
      phase: clamp(region.phase, -6.3, 6.3, index * 0.7),
      // 軸値はバネ変位へ掛けるピクセル倍率。±1以内では配信画面上の柔らかい部位が
      // 1px未満しか動かず見えなかったため、許容範囲を±8までにする。
      axisX: clamp(region.axisX, -8, 8, 0),
      axisY: clamp(region.axisY, -8, 8, -1),
      rotation: clamp(region.rotation, -6, 6, 0),
      scale: clamp(region.scale, 0, 0.03, 0.0025),
    });
  });
  springLastAt = 0;
  springRenderingPaused = true;
  updateSpringMotionVisibility();
  wakeAnimationLoop();
}

function animateSpringMotion(now) {
  if (!activeSpringMotion.length || live2dActive || meshRigActive) return false;
  const layout = petApp.dataset.layout || "pet";
  const shouldRender = !reduceMotion
    && !document.hidden
    && !napping
    && !petApp.classList.contains("is-booting")
    && layout !== "chat"
    && layout !== "bar";
  if (!shouldRender) {
    if (!springRenderingPaused) {
      for (const spring of activeSpringMotion) {
        spring.position = 0;
        spring.velocity = 0;
        spring.sprite.style.transform = "none";
      }
      characterView?.style.setProperty("--body-breathe-y", "0px");
    }
    springRenderingPaused = true;
    springLastAt = 0;
    return false;
  }
  springRenderingPaused = false;
  if (springLastAt && now - springLastAt < 1000 / 30) return true;
  const dt = springLastAt ? Math.min(1 / 30, Math.max(1 / 240, (now - springLastAt) / 1000)) : 1 / 60;
  springLastAt = now;
  if (activeBodyMotion && characterView) {
    const bodyY = Math.sin(
      (now / 1000) * Math.PI * 2 * activeBodyMotion.frequency + activeBodyMotion.phase,
    ) * activeBodyMotion.amplitudeY;
    characterView.style.setProperty("--body-breathe-y", `${bodyY.toFixed(3)}px`);
  }
  for (const spring of activeSpringMotion) {
    const target = Math.sin((now / 1000) * Math.PI * 2 * spring.frequency + spring.phase) * spring.drive;
    const acceleration = (target - spring.position) * spring.stiffness - spring.velocity * spring.damping;
    spring.velocity += acceleration * dt;
    spring.position += spring.velocity * dt;
    const p = spring.position;
    spring.sprite.style.transform = `translate(${(p * spring.axisX).toFixed(3)}px, ${(p * spring.axisY).toFixed(3)}px) rotate(${(p * spring.rotation).toFixed(3)}deg) scale(${(1 + Math.abs(p) * spring.scale).toFixed(5)})`;
  }
  return true;
}

async function loadCharacter(characterId, overrides = {}, options = {}) {
  characterOverridesCache = overrides;
  if (options.live2dSdkOverrides) live2dSdkOverridesCache = options.live2dSdkOverrides;
  const { manifest, root } = await loadCharacterManifest(characterId);
  const hasLive2dDefinition = (config) => Boolean(config?.model || config?.models?.length);
  const hasAnyLive2d = hasLive2dDefinition(manifest.live2d)
    || (Array.isArray(manifest.variants) && manifest.variants.some(item => hasLive2dDefinition(item?.live2d)));
  if (!hasAnyLive2d && !manifest.animations?.idle) throw new Error("Live2D、idleアニメーションのいずれも定義されていません");
  const profile = personaText(manifest, overrides[manifest.id]);
  // 同じキャラの再読込で早期 return する場合も含め、照合表は必ず現在の人格へ合わせる。
  setPersonaSpeechBlocklist(profile);
  const assetVersion = String(manifest.assetVersion || "");
  const savedVariant = localStorage.getItem(`picoagent.variant.${manifest.id}`) || "default";
  const sdkPreference = String(live2dSdkOverridesCache[manifest.id] || "auto");
  const signature = `${manifest.id}\n${assetVersion}\n${savedVariant}\n${sdkPreference}\n${profile}`;
  if (signature === loadedCharacterSignature) return;
  clearAnimationPreloads();
  characterRoot = root;
  activeLive2dSdkPreference = sdkPreference === "4" || sdkPreference === "5" ? sdkPreference : "auto";
  characterManifest = manifest;
  // 配置編集のキャラ位置・倍率は、表示形式だけでなくキャラクターごとに分離する。
  character.dataset.characterId = manifest.id;
  characterAssetVersion = assetVersion;
  character.dataset.bodyProportion = manifest.bodyProportion === "compact" ? "compact" : "standard";
  character.dataset.sourceFraming = manifest.sourceFraming === "native-bust" ? "native-bust" : "default";
  const variants = Array.isArray(manifest.variants) ? manifest.variants : [];
  const variant = variants.find(item => item.id === savedVariant) || variants[0] || null;
  character.dataset.variant = variant?.id || "default";
  if (variant?.root) characterRoot = `${root}/${String(variant.root).replace(/^\/+|\/+$/g, "")}`;
  animations = variant?.animations || manifest.animations || {};
  runtimeLayers = variant?.runtimeLayers || manifest.runtimeLayers || {};
  activeLive2dConfig = resolveLive2dConfig(manifest, variant);
  activeMeshRigConfig = variant?.meshRig || manifest.meshRig || null;
  configureSpringMotion(variant?.springPhysics ?? manifest.springPhysics, variant?.secondaryMotion ?? manifest.secondaryMotion);
  renderVariantPicker(variants, variant?.id || "default");
  stopRig();
  detachLipSyncAudio(); // Live2D キャラの口パラメータ解析も解除する
  destroyMeshRig();
  if (hasLive2dDefinition(activeLive2dConfig)) {
    activeMeshRigConfig = null; // 同一キャラで両方ある場合は審査済みLive2D定義を優先する
    rigAvailable = false; // まばたき・口パクはモデル定義側が担当する
    try {
      await startLive2D(activeLive2dConfig);
    } catch (error) {
      stopLive2D();
      activeLive2dConfig = null;
      if (!animations.idle) throw error;
      // フォールバック許可: Live2D ランタイム未取得やモデル未書き出しでも、フレームアニメを
      // 持つキャラは表示を継続する。制作途中のモデルを variant に登録しておけるようにするため、
      // ここは「失敗」ではなく「PNG 表示で続行」として伝える。
      logApp("error", "Live2D の初期化に失敗しました（PNG 表示で継続）", errorDetail(error));
      setStatus("Live2D を読み込めないので PNG 表示で続けるね");
      preloadAnimations();
      await probeRigLayers();
    }
  } else {
    stopLive2D();
    preloadAnimations();
    await probeRigLayers(); // layers 素材があればリップシンク対応
    if (activeMeshRigConfig?.rig) {
      try {
        await prepareMeshRig(activeMeshRigConfig);
      } catch (error) {
        destroyMeshRig();
        activeMeshRigConfig = null;
        logApp("error", "メッシュリグの初期化に失敗しました（差分絵表示で継続）", errorDetail(error));
        setStatus("高品質リグを読み込めないので差分絵表示で続けるね");
      }
    }
  }
  applyCharacterFraming();
  frame = 0;
  setAnimation("idle", true);
  character.title = `${manifest.name}：右クリックで設定`;
  // 挨拶も渡す。短い挨拶へモデルを呼ばず返すとき、全キャラ共通の定型文にしないため。
  await window.picoagent.setCharacterProfile(manifest.id, profile, manifest.greeting || "");
  loadedCharacterSignature = signature;
  if (manifest.greeting) {
    renderReply(manifest.greeting);
    // 起動時は描画と同時に再生するため、ここでは合成しない（boot 側が事前合成→表示と同時再生）。
    if (options.speakGreeting !== false) speakReply(manifest.greeting);
  }
}

function renderVariantPicker(variants, selectedId) {
  if (!variantRow || !variantSelect) return;
  variantSelect.replaceChildren();
  if (!Array.isArray(variants) || variants.length < 2) {
    variantRow.hidden = true;
    return;
  }
  for (const variant of variants) {
    const option = document.createElement("option");
    option.value = variant.id;
    option.textContent = variant.name || variant.id;
    option.selected = variant.id === selectedId;
    variantSelect.append(option);
  }
  variantRow.hidden = false;
}

variantSelect?.addEventListener("change", async () => {
  if (!characterManifest?.id) return;
  const characterId = characterManifest.id;
  const storageKey = `picoagent.variant.${characterId}`;
  const previousVariant = character.dataset.variant || "default";
  const requestedVariant = variantSelect.value;
  variantSelect.disabled = true;
  localStorage.setItem(storageKey, requestedVariant);
  loadedCharacterSignature = "";
  try {
    await loadCharacter(characterId, characterOverridesCache, { speakGreeting: false });
    setAnimation("pleased", true);
    setTimeout(() => setAnimation("idle"), 900);
  } catch (error) {
    logApp("error", "雰囲気の切替に失敗しました", errorDetail(error));
    localStorage.setItem(storageKey, previousVariant);
    variantSelect.value = previousVariant;
    loadedCharacterSignature = "";
    try {
      await loadCharacter(characterId, characterOverridesCache, { speakGreeting: false });
    } catch (restoreError) {
      logApp("error", "雰囲気の切替失敗後に元の表示へ戻せませんでした", errorDetail(restoreError));
    }
    showToast("雰囲気を切り替えられませんでした", "error");
  } finally {
    variantSelect.disabled = false;
  }
});
function setAnimation(name, force = false) {
  const fallbacks = { listening: "idle", searching: "thinking", working: "thinking", success: "talking", pleased: "talking", shy: "idle", concern: "sad", wake: "idle", sleep: "idle" };
  const sourceName = animations[name] ? name : (fallbacks[name] && animations[fallbacks[name]] ? fallbacks[name] : "idle");
  const requestedMotion = animations[name]?.motion || name;
  if (!force && animationName === sourceName && character.dataset.motion === requestedMotion) {
    wakeAnimationLoop();
    return;
  }
  animationName = (animations[sourceName] || live2dActive) ? sourceName : "idle"; frame = 0; lastFrameAt = performance.now();
  wakeAnimationLoop();
  if (live2dActive) { character.dataset.motion = "none"; playLive2DMotion(animationName); return; }
  if (!animations[animationName]) return; // Live2D 初期化失敗直後などフレーム定義が無い場合は何もしない
  // 待機・会話・思考中は同じキャンバスを保つ。thinkingだけ差分絵へ切り替えると、
  // 配信表示のフレーミング差でキャラクター全体が拡縮して見える。
  const meshMotion = animationName === "idle" || animationName === "thinking" || animationName === "talking";
  if (meshRigController && meshMotion && meshRigCanRender()) {
    setMeshRigVisible(true);
    character.dataset.motion = requestedMotion;
    meshRigController.setMotion(requestedMotion);
    startRig();
    return;
  }
  setMeshRigVisible(false);
  preloadAnimation(animationName);
  character.dataset.motion = requestedMotion;
  // layers を持つキャラは待機・返答状態を常時リグ描画にする。
  // 口パクは Audio の playing 後にだけ attachLipSync() から開始する。
  if (rigAvailable && (animationName === "idle" || animationName === "talking")) {
    startRig();
    return;
  }
  stopRig();
  displayPreloadedFrame(animationName, 0);
}
function frameDuration(animation, index) {
  const duration = animation.frameDurations?.[index];
  if (Number.isFinite(duration) && duration > 0) return duration;
  return 1000 / animation.fps;
}

function wakeAnimationLoop() {
  animationLoop?.start();
}

function animate(now) {
  const springActive = animateSpringMotion(now);
  if (document.hidden) return false;
  // Live2D 動作中はフレーム送り不要（PIXI 側の ticker が描画を担当する）。
  if (live2dActive) return false;
  // 「アニメーションを減らす」でも、発話中の口パクは状態を伝える機能なので残す。
  // 待機・思考などの装飾モーションは先頭フレームへ止め、次の状態変化までループも休止する。
  if (reduceMotion && animationName !== "talking") {
    if (!rigActive && animations[animationName]) displayPreloadedFrame(animationName, 0);
    return false;
  }
  // リグ動作中・居眠り中はフレーム送りを止める（それぞれ差分絵・寝姿を表示している）。
  // 局所バネが実際に描画中のリグだけは、バネ更新のためループを維持する。
  if (rigActive || napping) return springActive;
  const animation = animations[animationName] || animations.idle;
  if (!animation) return springActive;
  if (!lastFrameAt) lastFrameAt = now;
  if (!displayPreloadedFrame(animationName, frame)) {
    // 壊れたフレームで送りを止めると、そのモーションが以後ずっと固まる。通常の間隔で次へ進める。
    if (isBrokenFrame(animationName, frame) && now - lastFrameAt >= frameDuration(animation, frame)) {
      frame = (frame + 1) % animationFrameCount(animation);
      lastFrameAt = now;
    }
    return true;
  }
  if (now - lastFrameAt >= frameDuration(animation, frame)) {
    const nextFrame = (frame + 1) % animationFrameCount(animation);
    // 壊れたフレームは表示できないが、送りは進める（前の完成フレームを出したまま先へ抜ける）。
    if (displayPreloadedFrame(animationName, nextFrame) || isBrokenFrame(animationName, nextFrame)) {
      frame = nextFrame;
    }
    lastFrameAt = now;
  }
  return true;
}
animationLoop = window.PicoFrameScheduler.createLoop(animate);
wakeAnimationLoop();

// ── 疑似Live2Dリグ ─────────────────────────────────────────────────
// runtimeLayers（未指定の追加パックはlayers/*.png）は「同一ポーズの全身差分絵」。
// 読み上げ中はフレームアニメの代わりに差分絵を切り替え（PNGTuber方式）、
// WebAudio の振幅解析で実際の声量に合わせて口を動かし、ランダムにまばたきする。
let rigAvailable = false; // 選択中キャラに layers 素材が揃っているか
let rigGazeAvailable = false; // 左右視線レイヤーも使えるか
let rigActive = false;
let rigFace = "";          // 表示中の差分名（無駄な src 再設定を避ける）
let rigMouthOpen = false;  // 口の開閉（振幅解析またはタイマーが更新）
let rigBlinkPhase = null;  // まばたき中の差分名（null = まばたき中でない）
let rigLookDirection = 0;  // -1=左、0=正面、1=右
let rigGazeSyncAt = 0;     // カーソル追従を反映した時刻（mousemove の間引き用）
let gazeTrackingEnabled = false; // 既定OFF。設定で明示した場合だけ視線を動かす
let rigMouthTimer = 0;
let rigBlinkTimer = 0;
let rigBlinkStepTimer = 0;
let rigGazeTimer = 0;
let rigVolFrame = 0;
let lookResetTimer = 0;
let gazeFrameScheduler = null;
let audioCtx = null;
let rigSource = null;
let rigAnalyser = null;
let rigMouthLevel = 0; // 音量の瞬間値をそのまま口へ入れず、開閉を滑らかにする包絡線
const RIG_MOUTH_OPEN_THRESHOLD = 0.34;
const RIG_MOUTH_CLOSE_THRESHOLD = 0.22;
const RIG_BLINK_TIMINGS = Object.freeze({
  normal: Object.freeze({ closing: 60, closed: 80, opening: 170 }),
  sleepy: Object.freeze({ closing: 130, closed: 260, opening: 300 }),
});

function quantizeRigMouthOpen(level, wasOpen) {
  return level > (wasOpen ? RIG_MOUTH_CLOSE_THRESHOLD : RIG_MOUTH_OPEN_THRESHOLD);
}

function rigBlinkTimings(sleepy) {
  return sleepy ? RIG_BLINK_TIMINGS.sleepy : RIG_BLINK_TIMINGS.normal;
}

function layerUrl(name) {
  const relative = typeof runtimeLayers[name] === "string"
    ? runtimeLayers[name]
    : `layers/${name}.png`;
  const url = `${characterRoot}/${relative}`;
  return characterAssetVersion ? `${url}?v=${encodeURIComponent(characterAssetVersion)}` : url;
}

// layers 一式が読み込めるかを一度だけ確認する（無いキャラは従来のフレームアニメのみ）。
// 読み込み成功した Image はブラウザキャッシュに乗るためプリロードも兼ねる。
async function probeRigLayers() {
  rigAvailable = false;
  rigGazeAvailable = false;
  const probe = name => new Promise(resolve => {
    const img = new Image();
    let finished = false;
    const finish = result => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      img.onload = null;
      img.onerror = null;
      // src="" は文書URLの再取得になるため、属性ごと外して読み込みを手放す。
      if (!result) img.removeAttribute("src");
      resolve(result);
    };
    const timer = setTimeout(() => finish(false), STARTUP_RESOURCE_TIMEOUT_MS);
    img.onload = () => finish(true);
    img.onerror = () => finish(false);
    img.src = layerUrl(name);
    if (img.complete) queueMicrotask(() => finish(img.naturalWidth > 0));
  });
  const names = ["base", "mouth_open", "mouth_closed", "blink_half", "blink_closed"];
  const results = await Promise.all(names.map(probe));
  rigAvailable = results.every(Boolean);
  if (rigAvailable) {
    const gazeResults = await Promise.all(["gaze_left", "gaze_right"].map(probe));
    rigGazeAvailable = gazeResults.every(Boolean);
  }
  retryPendingLipSync(); // 素材の判定待ちで保留していた読み上げへ口パクを付ける
}

// まばたき > 口 の優先度で現在の差分絵を決めて反映する。
// ただしよそ見中だけは視線を優先する（まばたき差分は正面顔しか無いため、割り込ませると
// 横を向いている数百msの途中で一瞬だけ顔が正面へ戻って見える）。
function rigRefresh() {
  if (!rigActive) return;
  if (meshRigActive && meshRigController) {
    meshRigController.setMouth(rigMouthLevel || (rigMouthOpen ? 0.58 : 0));
    meshRigController.setGaze(rigLookDirection, 0);
    meshRigController.setMotion(character.dataset.motion || animationName);
    return;
  }
  const gazing = rigGazeAvailable && !currentVoice && animationName === "idle" && rigLookDirection !== 0;
  const gaze = gazing ? (rigLookDirection < 0 ? "gaze_left" : "gaze_right") : "mouth_closed";
  const blink = gazing ? null : rigBlinkPhase;
  const happy = ["success", "pleased", "shy"].includes(character.dataset.motion) ? "expression_happy" : null;
  const name = blink || (rigMouthOpen ? "mouth_open" : (happy || gaze));
  if (rigFace === name) return;
  rigFace = name;
  petSprite.src = layerUrl(name);
}

// 待機中のよそ見は短く低頻度にする。頻繁に視線差分へ切り替えると、会話相手を
// 見ずに常に下や横を向いている印象になる。
function scheduleGaze() {
  clearTimeout(rigGazeTimer);
  if (reduceMotion || !gazeTrackingEnabled || !rigActive || !rigGazeAvailable) return;
  rigGazeTimer = setTimeout(() => {
    if (reduceMotion || !gazeTrackingEnabled || !rigActive) return;
    if (currentVoice) { scheduleGaze(); return; }
    rigLookDirection = Math.random() < 0.5 ? -1 : 1;
    rigRefresh();
    rigGazeTimer = setTimeout(() => {
      rigLookDirection = 0;
      rigRefresh();
      scheduleGaze();
    }, 350 + Math.random() * 300);
  }, 8000 + Math.random() * 6000);
}

// まばたき: half → closed → half を挟み、1.4〜4.2秒間隔で繰り返す。
// 実際のまばたきは閉じが速く開きが遅い。3段を同じ長さにすると機械的な明滅に見えるため、
// 閉じ側を詰めて開き側を伸ばした非対称の配分にする。
function scheduleBlink() {
  clearTimeout(rigBlinkTimer);
  if (!rigActive) return;
  // 深夜〜早朝（23時〜5時）は眠そうに: まばたき間隔を詰め、半目を長めに保つ。
  const hour = new Date().getHours();
  const sleepy = hour >= 23 || hour < 5;
  const timings = rigBlinkTimings(sleepy);
  const interval = sleepy ? 900 + Math.random() * 1600 : 1400 + Math.random() * 2800;
  rigBlinkTimer = setTimeout(async () => {
    if (!rigActive) return;
    const step = (phase, ms, meshTarget) => new Promise(resolve => {
      rigBlinkPhase = phase;
      if (meshRigActive) meshRigController?.setBlink(meshTarget, ms);
      rigRefresh();
      rigBlinkStepTimer = setTimeout(() => {
        rigBlinkStepTimer = 0;
        resolve();
      }, ms);
    });
    // 閉じ際は素早く通過させる。
    await step("blink_half", timings.closing, 0.5);
    if (!rigActive) return;
    // 眠い時間帯は閉じたまま保つ長さで眠さを出す。
    await step("blink_closed", timings.closed, 0);
    if (!rigActive) return;
    // 開き際は閉じ際の倍以上かけて戻す。
    await step("blink_half", timings.opening, 1);
    if (!rigActive) return;
    rigBlinkPhase = null;
    rigRefresh();
    scheduleBlink();
  }, interval);
}

// 現在のチャンク音声の解析だけを解除する（リグの見た目は維持）。
function detachLipSyncAudio() {
  pendingLipSyncAudio = null;
  if (rigMouthTimer) { clearInterval(rigMouthTimer); rigMouthTimer = 0; }
  if (rigVolFrame) { cancelAnimationFrame(rigVolFrame); rigVolFrame = 0; }
  // フォールバック許可: 解放処理なので、既に切断済み・AudioContext 破棄済みで
  // 例外になっても続けて残りの参照を捨てる（ここで止まると解放漏れになる）。
  try { rigSource?.disconnect(); rigAnalyser?.disconnect(); }
  catch (error) { logApp("warn", "リップシンクの音声解析を切断できませんでした", errorDetail(error)); }
  rigSource = null;
  rigAnalyser = null;
  rigMouthLevel = 0;
  rigMouthOpen = false;
  live2dMouth = -1;
  rigRefresh();
}

// リグ（差分絵）も Live2D もまだ準備できていない間に始まった読み上げは、準備完了後に
// 張り直せるよう覚えておく。Live2D はモデルとテクスチャの読み込みに数秒かかるため、
// 起動直後の挨拶や読み込み中に届いた返答が、口の動かないまま読み上げられていた。
let pendingLipSyncAudio = null;

// リップシンクの正本。読み上げ無効（ミュート）・音量0・再生前/停止後は、
// 2D / Live2D / タイマーフォールバックのどの経路でも口を動かさない。
function shouldLipSync(audio) {
  return Boolean(
    audio
    && audio === currentVoice
    && ttsEnabledCache !== false
    && voiceVolume > 0.001
    && !audio.paused
    && !audio.ended
  );
}

function retryPendingLipSync() {
  const audio = pendingLipSyncAudio;
  if (!audio) return;
  pendingLipSyncAudio = null;
  if (!shouldLipSync(audio)) return;
  attachLipSync(audio);
}

// 音声要素へリップシンクを接続する。リグ未対応キャラでは何もしない。
// Live2D キャラでは差分絵の代わりに live2dMouth（モデルの口パラメータ）を動かす。
function attachLipSync(audio) {
  if (!shouldLipSync(audio)) {
    detachLipSyncAudio();
    return;
  }
  if (!rigAvailable && !live2dActive) {
    logApp("info", "リップシンクを保留しました", "キャラクターの表示準備が終わってから接続します");
    pendingLipSyncAudio = audio;
    return;
  }
  if (!live2dActive) startRig();
  detachLipSyncAudio();
  // WebAudio の振幅解析を試み、使えない環境ではタイマー口パクへフォールバックする。
  let useTimer = true;
  try {
    audioCtx = audioCtx || new (window.AudioContext || window.webkitAudioContext)();
    if (audioCtx.state === "suspended") {
      audioCtx.resume().catch(error => {
        logApp("warn", "リップシンク用の音声解析を再開できませんでした", errorDetail(error));
      });
    }
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
      if (voiceVolume <= 0.001) {
        rigMouthOpen = false;
        live2dMouth = 0;
        rigRefresh();
        return;
      }
      rigMouthOpen = !rigMouthOpen;
      // フォールバックでも最大開口と完全閉口を往復させない。ひよりのような
      // デフォルメ口では 0↔0.8 の矩形波が「口だけ跳ねる」見え方になる。
      if (live2dActive) live2dMouth = rigMouthOpen ? 0.58 : 0.08;
      else rigRefresh();
    }, 150);
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
    // 解析点は音量調整（audio.volume）より後ろなので、そのままだと読み上げ音量を
    // 下げただけでしきい値（peak の下限 0.02）に届かなくなり、口が閉じたままになる。
    // 要素の音量で割り戻し、聞こえる大きさに依存しない振幅で判定する。
    const gain = Math.max(0.05, audio.volume || 1);
    const rms = Math.sqrt(sum / data.length) / gain;
    peak = Math.max(peak * 0.995, rms, 0.02);
    // 無音域を除いた声量を 0〜1 へ正規化する。従来は瞬間値を直接入れていたため、
    // ひよりではほぼ「完全閉口↔最大開口」を毎フレーム往復し、README の滑らかな
    // モーションと違って口だけが痙攣するように見えていた。開く側は素早く、閉じる側は
    // 少し遅い包絡線にして、音節へ追従しつつフレーム間の細かな揺れを吸収する。
    const floor = peak * 0.22;
    const span = Math.max(peak * 0.72, 0.001);
    const target = voiceVolume <= 0.001 ? 0 : Math.max(0, Math.min(1, (rms - floor) / span));
    const desired = Math.min(0.78, target * 0.78);
    const smoothing = desired > rigMouthLevel ? 0.34 : 0.16;
    rigMouthLevel += (desired - rigMouthLevel) * smoothing;
    if (rigMouthLevel < 0.015) rigMouthLevel = 0;
    if (live2dActive) {
      live2dMouth = rigMouthLevel;
    } else {
      // 差分絵は開閉2枚しか無く、単一しきい値だと包絡線が境界付近に留まる音節で
      // 開閉が細かく往復する。開く側を高く閉じる側を低くして、跳ねを止める。
      rigMouthOpen = quantizeRigMouthOpen(rigMouthLevel, rigMouthOpen);
      rigRefresh();
    }
    rigVolFrame = requestAnimationFrame(analyserLoop);
  };
  rigVolFrame = requestAnimationFrame(analyserLoop);
}

// リグ描画を開始する（既に動作中なら何もしない。口は閉じ・まばたき開始）。
function startRig() {
  if (rigActive) { wakeAnimationLoop(); return; }
  rigActive = true;
  rigFace = "";
  rigMouthOpen = false;
  rigBlinkPhase = null;
  if (meshRigActive) meshRigController?.setBlink(1, 0);
  rigLookDirection = 0;
  rigRefresh();
  scheduleBlink();
  scheduleGaze();
  wakeAnimationLoop();
}

// リグを完全に終了する（キャラ切替・sad などフレームアニメ状態への遷移用）。
function stopRig() {
  if (!rigActive) { wakeAnimationLoop(); return; }
  rigActive = false;
  detachLipSyncAudio();
  clearTimeout(rigBlinkTimer);
  clearTimeout(rigBlinkStepTimer);
  clearTimeout(rigGazeTimer);
  rigBlinkTimer = 0;
  rigBlinkStepTimer = 0;
  rigGazeTimer = 0;
  rigBlinkPhase = null;
  rigLookDirection = 0;
  rigFace = "";
  wakeAnimationLoop();
}

// 読み上げ終了時の処理。layers を持つキャラは待機中も常時リグ（まばたき）を続けるため
// 音声解析だけ解除し、リグ非対応キャラのみ完全に終了してフレーム表示へ戻す。
function stopLipSync() {
  if (live2dActive) { detachLipSyncAudio(); return; }
  if (rigAvailable) { detachLipSyncAudio(); return; }
  const wasActive = rigActive;
  stopRig();
  if (wasActive && animations[animationName]) displayPreloadedFrame(animationName, frame);
}

// ── 自前メッシュリグ ──────────────────────────────────────────────
let activeMeshRigConfig = null;
let meshRigController = null;
let meshRigCanvas = null;
let meshRigActive = false;
let pixiRuntimePromise = null;

function meshRigCanRender() {
  const layout = petApp.dataset.layout || "pet";
  return Boolean(
    meshRigController
    && !reduceMotion
    && !document.hidden
    && !napping
    && !petApp.classList.contains("is-booting")
    && layout !== "chat"
    && layout !== "bar"
  );
}

function setMeshRigVisible(visible) {
  const active = Boolean(visible && meshRigCanRender());
  meshRigActive = active;
  meshRigController?.setActive(active);
  if (meshRigCanvas) meshRigCanvas.style.display = active ? "" : "none";
  petSprite.style.display = active ? "none" : "";
  character.dataset.renderKind = active ? "mesh-rig" : "2d";
  updateSpringMotionVisibility();
  return active;
}

function destroyMeshRig() {
  meshRigActive = false;
  meshRigController?.destroy();
  meshRigController = null;
  meshRigCanvas = null;
  petSprite.style.display = "";
  if (!live2dActive) character.dataset.renderKind = "2d";
  updateSpringMotionVisibility();
}

async function prepareMeshRig(cfg) {
  await ensurePixiRuntime();
  if (!window.PicoMeshRig?.create) throw new Error("メッシュリグ描画モジュールがありません");
  meshRigCanvas = document.createElement("canvas");
  meshRigCanvas.id = "meshRigCanvas";
  meshRigCanvas.setAttribute("aria-hidden", "true");
  characterView.insertBefore(meshRigCanvas, petSprite);
  const rigPath = String(cfg.rig || "").replace(/^\/+/, "");
  const slash = rigPath.lastIndexOf("/");
  const rigRoot = slash >= 0 ? rigPath.slice(0, slash) : ".";
  meshRigController = await window.PicoMeshRig.create({
    canvas: meshRigCanvas,
    rigUrl: `${characterRoot}/${rigPath}`,
    rootUrl: `${characterRoot}/${rigRoot}`,
    assetVersion: characterAssetVersion,
    width: LIVE2D_RENDER_WIDTH,
    height: LIVE2D_RENDER_HEIGHT,
  });
  meshRigController.setActive(false);
}

// ── Live2D 本体レンダリング ─────────────────────────────────────────
// character.json に live2d.model がある場合、PNG フレームの代わりに Cubism ランタイム
// （ui/vendor/live2d/、cargo xtask setup live2d で取得）でモデルを WebGL 描画する。
// まばたき（EyeBlink グループ）・髪の揺れ（physics3）・呼吸と自然な揺らぎはランタイムの
// 自動処理に任せる。手で書いた固定波形はこれらを毎フレーム上書きして不自然になるため使わない。
// ただしランタイムはモーション再生中に自動まばたきを回さないので、startLive2D で自前に回す。
// 読み上げ中は attachLipSync の振幅解析が live2dMouth を介して口パラメータを直接動かす。
let live2dActive = false;
let activeLive2dConfig = null; // いま表示している variant の live2d 定義（無ければ null）
let live2dModel = null;
let live2dApp = null;
let live2dCanvas = null;
let live2dMouth = -1; // 読み上げ中の口の開き（0〜1）。-1 はモーション任せ。
let live2dRuntimePromise = null;
let live2dRuntimeSdk = "";
// ページへ挿入したCubism Coreの世代。script要素を消してもグローバルは戻せないため、
// 「読み込みを試みた」時点で確定させ、以後は別世代を読ませない。
let live2dCoreSdk = "";
// 表示形式が chat / bar の状態で起動すると character は小型アイコン寸法しかない。
// その寸法でモデルの scale / offsetY を計算すると描画が全てキャンバス外へ出るため、
// Live2D 自体は常にペット表示の基準寸法で描き、各形式では CSS で縮小・切り出す。
const LIVE2D_RENDER_WIDTH = 285;
const LIVE2D_RENDER_HEIGHT = 315;
// 配信形式では基準キャンバスを最大2.75倍で表示する。CSSだけを拡大すると285x315の
// 描画結果を引き伸ばして全Live2Dがぼけるため、最初から表示倍率とDPIを含むバッファを持つ。
const LIVE2D_MAX_DISPLAY_SCALE = 2.75;

function desiredLive2DResolution() {
  return Math.max(1, (window.devicePixelRatio || 1) * LIVE2D_MAX_DISPLAY_SCALE);
}

function syncLive2DResolution() {
  const renderer = live2dApp?.renderer;
  if (!renderer) return;
  const resolution = desiredLive2DResolution();
  if (Math.abs(renderer.resolution - resolution) < 0.01) return;
  renderer.resolution = resolution;
  renderer.resize(LIVE2D_RENDER_WIDTH, LIVE2D_RENDER_HEIGHT);
  logApp("info", "Live2D描画密度をモニターへ追従", JSON.stringify({
    resolution,
    backing: [live2dCanvas?.width || 0, live2dCanvas?.height || 0],
  }));
}
// CSS の立ち絵枠と配信形式のキャンバス拡大は、この基準寸法から計算する。
// styles.css の :root にも同じ既定値を書いてあるが、正本はここ1箇所にする。
document.documentElement.style.setProperty("--char-frame-width", `${LIVE2D_RENDER_WIDTH}px`);
document.documentElement.style.setProperty("--char-frame-height", `${LIVE2D_RENDER_HEIGHT}px`);

function loadScriptOnce(src) {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    let finished = false;
    const finish = error => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      script.onload = null;
      script.onerror = null;
      if (error) {
        script.remove();
        reject(error);
      } else resolve();
    };
    const timer = setTimeout(
      () => finish(new Error(`スクリプトが${STARTUP_RESOURCE_TIMEOUT_MS}ミリ秒以内に応答しませんでした: ${src}`)),
      STARTUP_RESOURCE_TIMEOUT_MS,
    );
    script.src = src;
    script.onload = () => finish(null);
    script.onerror = () => finish(new Error(`スクリプトを読み込めませんでした: ${src}`));
    document.head.append(script);
  });
}

async function ensurePixiRuntime() {
  if (window.PIXI?.Application && window.PIXI?.SimplePlane) return;
  if (!pixiRuntimePromise) {
    pixiRuntimePromise = loadScriptOnce("vendor/pixi/pixi.min.js?v=6.5.10-pinned").catch(error => {
      pixiRuntimePromise = null;
      throw new Error(`PIXIランタイムを読み込めません（cargo xtask setup pixi を実行してください）: ${error.message}`);
    });
  }
  await pixiRuntimePromise;
}

function destroyDetachedLive2DModel(model) {
  try { model?.destroy?.(); }
  catch (error) { logApp("warn", "遅れて完了したLive2Dモデルを破棄できませんでした", errorDetail(error)); }
}

function loadLive2DModelWithTimeout(url) {
  const pending = PIXI.live2d.Live2DModel.from(url, { autoInteract: false });
  return new Promise((resolve, reject) => {
    let finished = false;
    const timer = setTimeout(() => {
      finished = true;
      reject(new Error(`Live2Dモデルが${LIVE2D_MODEL_TIMEOUT_MS}ミリ秒以内に応答しませんでした: ${url}`));
    }, LIVE2D_MODEL_TIMEOUT_MS);
    pending.then(model => {
      if (finished) {
        destroyDetachedLive2DModel(model);
        return;
      }
      finished = true;
      clearTimeout(timer);
      resolve(model);
    }, error => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      reject(error);
    });
  });
}

// ランタイム3点を一度だけ順に読み込む（Cubism Core → PIXI → pixi-live2d-display）。
async function ensureLive2DRuntime(sdk) {
  const requestedSdk = sdk === "5" ? "5" : "4";
  // 読み込みが途中で失敗・タイムアウトしても、挿入済みCoreは残る（scriptを外しても実行は止まらない）。
  // 成否ではなく挿入済み世代で判定しないと、再試行時に1ページへ2世代が同居する。
  if (live2dCoreSdk && live2dCoreSdk !== requestedSdk) {
    throw new Error("Cubism Coreの世代を切り替えるにはPicoAgentの再起動が必要です");
  }
  if (window.PIXI?.live2d?.Live2DModel) return;
  if (!live2dRuntimePromise) {
    live2dRuntimeSdk = requestedSdk;
    live2dRuntimePromise = (async () => {
      live2dCoreSdk = requestedSdk;
      const core = live2dRuntimeSdk === "5"
        ? "vendor/live2d/live2dcubismcore-5.3.min.js?v=5.3-pinned"
        : "vendor/live2d/live2dcubismcore-compat.min.js?v=compat-pinned";
      await loadScriptOnce(core);
      await ensurePixiRuntime();
      await loadScriptOnce("vendor/live2d/cubism4.min.js");
      // モデル同梱のモーション音声（サンプルボイス）は TTS 読み上げと二重再生になるため無効化する。
      if (window.PIXI?.live2d?.config) PIXI.live2d.config.sound = false;
    })().catch(error => {
      live2dRuntimePromise = null; // 取得後の再選択で再試行できるようにする
      live2dRuntimeSdk = "";
      throw new Error(`Live2D ランタイムを読み込めません（cargo xtask setup live2d を実行してください）: ${error.message}`);
    });
  }
  await live2dRuntimePromise;
}

async function startLive2D(cfg) {
  const rawCandidates = Array.isArray(cfg.models) && cfg.models.length ? cfg.models : [cfg.model];
  const normalizedCandidates = rawCandidates.flatMap(candidate => {
    if (typeof candidate === "string") {
      const match = candidate.match(/(?:^|\/)sdk([45])(?:\/|$)/i);
      return candidate ? [{ path: candidate, sdk: match?.[1] || "" }] : [];
    }
    const path = String(candidate?.path || candidate?.model || "");
    return path ? [{ path, sdk: String(candidate?.sdk || "") }] : [];
  });
  const selectedSdk = activeLive2dSdkPreference === "auto"
    ? (normalizedCandidates[0]?.sdk === "5" ? "5" : "4")
    : activeLive2dSdkPreference;
  // 1ページ内でCubism Coreは交換できないため、異なる世代へは無言フォールバックしない。
  // 同じ世代で複数の互換モデルがある場合だけ、定義順に次の候補を試す。
  const modelCandidates = normalizedCandidates.filter(candidate =>
    (candidate.sdk === "5" ? "5" : "4") === selectedSdk);
  if (!modelCandidates.length) {
    throw new Error(`Cubism ${activeLive2dSdkPreference} のモデルがこのキャラクターパックにありません`);
  }
  // 5.3 MOCは専用Core、SDK世代不明／Cubism 4は旧モデルで口メッシュが消えない互換Coreを使う。
  await ensureLive2DRuntime(selectedSdk);
  stopLive2D();
  // cfg は variant で解決済みの live2d 定義（キャラ本体か variant 側のどちらか）。
  const motionEnabled = !reduceMotion || cfg.alwaysAnimate === true;
  if (!live2dCanvas) {
    live2dCanvas = document.createElement("canvas");
    live2dCanvas.id = "live2dCanvas";
    characterView.insertBefore(live2dCanvas, petSprite);
  }
  live2dCanvas.style.display = "";
  const width = LIVE2D_RENDER_WIDTH;
  const height = LIVE2D_RENDER_HEIGHT;
  if (!live2dApp) {
    const transparentContextOptions = {
      alpha: true,
      premultipliedAlpha: true,
      antialias: true,
    };
    const transparentContext = live2dCanvas.getContext("webgl2", transparentContextOptions)
      || live2dCanvas.getContext("webgl", transparentContextOptions);
    live2dApp = new PIXI.Application({
      view: live2dCanvas,
      context: transparentContext,
      width,
      height,
      transparent: true,
      useContextAlpha: true,
      backgroundAlpha: 0,
      antialias: true,
      resolution: desiredLive2DResolution(),
      autoDensity: true,
    });
    live2dCanvas.style.background = "transparent";
    if (live2dApp.renderer.background) live2dApp.renderer.background.alpha = 0;
    live2dApp.renderer.backgroundAlpha = 0;
    live2dApp.renderer.transparent = true;
    if (live2dApp.renderer._backgroundColorRgba) {
      live2dApp.renderer._backgroundColorRgba[3] = 0;
    }
  }
  let model = null;
  let modelError = null;
  for (const candidate of modelCandidates) {
    try {
      model = await loadLive2DModelWithTimeout(`${characterRoot}/${candidate.path}`);
      break;
    } catch (error) {
      modelError = error;
      logApp("warn", `Live2Dモデル ${candidate.path} は現在のCubism Coreで読めません。次の互換モデルを試します`, errorDetail(error));
    }
  }
  if (!model) throw modelError || new Error("利用可能なLive2D互換モデルがありません");
  logApp("info", "Live2D表示寸法", JSON.stringify({
    canvas: [width, height],
    backing: [live2dCanvas.width, live2dCanvas.height],
    resolution: live2dApp.renderer.resolution,
    original: [model.internalModel.originalWidth, model.internalModel.originalHeight],
    bounds: [model.width, model.height],
    scale: Number(cfg.scale) || 1,
    motionEnabled,
  }));
  // 表示枠へ等比率で収め、下端中央に立たせる。
  const fit = Math.min(width / model.internalModel.originalWidth, height / model.internalModel.originalHeight);
  model.scale.set(fit * (Number(cfg.scale) || 1));
  model.anchor.set(0.5, 1);
  model.position.set(width / 2, height + (Number(cfg.offsetY) || 0));
  // 表情・視線・物理・ポーズ更新の後、モデル描画の直前にリップシンク値を上書きする。
  // 口パラメータIDはモデル定義（Groups: LipSync）から自動検出する。旧モデルは
  // PARAM_MOUTH_OPEN_Y のような旧命名のため、固定既定値だと口が動かない。
  const coreModel = model.internalModel.coreModel;
  const lipSyncGroup = model.internalModel.settings?.groups?.find(group => group.Name === "LipSync");
  // 口のパラメータを持たないモデル（公式サンプルの Rice など）は lipSync:false で明示的に切る。
  // 既定値を当てても存在しない ID を毎フレーム書き込むだけになるため。
  const lipSyncEnabled = cfg.lipSync !== false;
  const lipParam = String(cfg.lipSyncParam || lipSyncGroup?.Ids?.[0] || "ParamMouthOpenY");
  // 呼吸・体の揺らぎ（updateNaturalMovements）・髪の揺れ（physics3）・まばたき（EyeBlink）は
  // すべてランタイムの自動処理が担当する。ここで固定 sin 波を上書きすると、
  // 物理演算の出力（例: ParamHairBack）や自動の自然な揺らぎを毎フレーム潰してしまい、
  // 機械的で不自然な動きになる。手で書くのはリップシンクだけにとどめる。
  model.internalModel.on("beforeModelUpdate", () => {
    // 口はモーションや表情が閉じ戻すため、全更新の後に読み上げ振幅で上書きする。
    if (lipSyncEnabled && live2dMouth >= 0) {
      coreModel.setParameterValueById(lipParam, live2dMouth);
    }
  });
  // ランタイムは「モーション再生中は自動まばたきを回さない」（Cubism4InternalModel.update の
  // `if (!motionUpdated) eyeBlink.updateParameters(...)`）。PicoAgent はアイドルモーションを
  // ループさせ続けるため、放置すると自動まばたきが一度も動かない。
  // モーション適用後に自前で回して、ランダム間隔のまばたきを取り戻す。
  const internalModel = model.internalModel;
  if (internalModel.eyeBlink) {
    let lastBlinkAt = performance.now();
    internalModel.on("afterMotionUpdate", () => {
      const now = performance.now();
      const deltaSeconds = Math.min((now - lastBlinkAt) / 1000, 0.5); // タブ復帰時の巨大な差分で一気に進めない
      lastBlinkAt = now;
      internalModel.eyeBlink.updateParameters(coreModel, deltaSeconds);
    });
  }
  if (!motionEnabled) {
    // モーション再生を止め、呼吸などの静かな自動更新だけにする。
    model.internalModel.motionManager.groups.idle = "";
  } else if (cfg.motions?.idle) {
    model.internalModel.motionManager.groups.idle = cfg.motions.idle;
  }
  live2dApp.stage.addChild(model);
  live2dModel = model;
  live2dActive = true;
  character.dataset.renderKind = "live2d";
  applyCharacterZoom(characterZoom);
  petSprite.style.display = "none";
  updateSpringMotionVisibility();
  if (motionEnabled && cfg.motions?.idle) {
    const idlePriority = window.PIXI?.live2d?.MotionPriority?.IDLE ?? 1;
    model.motion(cfg.motions.idle, 0, idlePriority);
  }
  retryPendingLipSync(); // 読み込み中に始まった読み上げへ、いまから口パクを付ける
  wakeAnimationLoop(); // PNG側の空ループを1回進め、Live2D中はその場で休止させる
}

// 状態名（idle/thinking/talking/sad）を character.json の live2d.motions でグループへ写像して再生する。
function playLive2DMotion(name) {
  const alwaysAnimate = activeLive2dConfig?.alwaysAnimate === true;
  if (!live2dModel || (reduceMotion && !alwaysAnimate)) return;
  const motions = activeLive2dConfig?.motions || {};
  const group = motions[name] || motions.idle;
  if (!group) return;
  const priority = name === "idle"
    ? (window.PIXI?.live2d?.MotionPriority?.IDLE ?? 1)
    : (window.PIXI?.live2d?.MotionPriority?.FORCE ?? 3);
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
    character.dataset.renderKind = "2d";
    applyCharacterZoom(characterZoom);
    petSprite.style.display = "";
  }
  updateSpringMotionVisibility();
  wakeAnimationLoop(); // Live2D終了後にPNGフレームまたは局所バネを再開する
}

// ── 疑似視線追従: 設定ONかつ顔の近くにいるときだけ、短時間マウス方向を見る ──
function resetLook() {
  character.style.setProperty("--look-rot", "0deg");
  rigLookDirection = 0;
  rigRefresh();
  meshRigController?.setGaze(0, 0);
  // Live2DModel.focus()へcanvas外の座標を渡すと上下限へ張り付きやすい。
  // 内部の正規化済みコントローラを中央へ戻して、下向きが残らないようにする。
  live2dModel?.internalModel?.focusController?.focus(0, 0);
}

function applyGazePoint({ clientX, clientY }) {
    if (reduceMotion || !gazeTrackingEnabled) return;
    // 高頻度mousemoveは最新座標だけを1フレームへ合流するため、矩形の読取も最大1回になる。
    const rect = character.getBoundingClientRect();
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height * 0.38;
    const normalizedX = (clientX - centerX) / Math.max(160, rect.width * 0.9);
    const normalizedY = (clientY - centerY) / Math.max(180, rect.height * 0.8);
    const insideFaceBand = clientY >= rect.top + rect.height * 0.02
      && clientY <= rect.top + rect.height * 0.58;
    const nearFace = insideFaceBand && Math.abs(normalizedX) <= 1.15;
    const deadZone = Math.abs(normalizedX) < 0.28 && Math.abs(normalizedY) < 0.28;

    // Live2Dはライブラリ内部の正規化視線へ弱く渡す。canvasローカルの大きなpx値を
    // 直接focus()へ渡す旧実装では、画面下のカーソルに反応して下限へ張り付いていた。
    if (live2dActive && live2dModel && live2dCanvas) {
      const controller = live2dModel.internalModel?.focusController;
      if (controller) {
        if (nearFace && !deadZone) {
          controller.focus(
            Math.max(-0.38, Math.min(0.38, normalizedX * 0.32)),
            Math.max(-0.2, Math.min(0.2, -normalizedY * 0.18)),
          );
        } else controller.focus(0, 0);
      }
    }
    const dx = nearFace ? Math.max(-1, Math.min(1, normalizedX)) : 0;
    if (meshRigActive) {
      meshRigController?.setGaze(
        nearFace && !deadZone ? dx : 0,
        nearFace && !deadZone ? Math.max(-1, Math.min(1, -normalizedY)) : 0,
      );
    }
    character.style.setProperty("--look-rot", `${(dx * 1.15).toFixed(2)}deg`);
    // mousemove は毎ピクセル届くので、向きが変わったときと一定間隔でだけ反映する
    // （毎イベントで自動視線タイマーを張り直すとタイマーの作り直しだけが積み上がる）。
    if (rigActive && rigGazeAvailable && !currentVoice && animationName === "idle") {
      const direction = nearFace && !deadZone ? (dx < -0.38 ? -1 : dx > 0.38 ? 1 : 0) : 0;
      const now = performance.now();
      if (direction !== rigLookDirection || now - rigGazeSyncAt > 400) {
        rigGazeSyncAt = now;
        rigLookDirection = direction;
        rigRefresh();
        scheduleGaze(); // カーソル追従中は自動よそ見を先送りする
      }
    }
    clearTimeout(lookResetTimer);
    lookResetTimer = setTimeout(() => {
      resetLook();
      scheduleGaze();
    }, 700);
}

gazeFrameScheduler = window.PicoFrameScheduler.createLatest(applyGazePoint);

function applyGazeTrackingSetting(enabled) {
  gazeTrackingEnabled = enabled === true;
  gazeFrameScheduler.cancel();
  clearTimeout(lookResetTimer);
  clearTimeout(rigGazeTimer);
  lookResetTimer = 0;
  rigGazeTimer = 0;
  resetLook();
  if (!reduceMotion && gazeTrackingEnabled) scheduleGaze();
}

function onGazeMouseMove(event) {
  if (reduceMotion || !gazeTrackingEnabled) return;
  gazeFrameScheduler.schedule({ clientX: event.clientX, clientY: event.clientY });
}

function onReduceMotionChange(event) {
  reduceMotion = event.matches;
  gazeFrameScheduler.cancel();
  clearTimeout(lookResetTimer);
  clearTimeout(rigGazeTimer);
  lookResetTimer = 0;
  rigGazeTimer = 0;
  resetLook();
  if (!reduceMotion && gazeTrackingEnabled) scheduleGaze();
  if (live2dActive) {
    const alwaysAnimate = activeLive2dConfig?.alwaysAnimate === true;
    if (reduceMotion && !alwaysAnimate) {
      live2dModel?.internalModel?.motionManager?.stopAllMotions?.();
    } else {
      playLive2DMotion(animationName);
    }
  }
  if (meshRigController) setAnimation(animationName, true);
  wakeAnimationLoop();
}

window.applyGazeTrackingSetting = applyGazeTrackingSetting;
document.addEventListener("mousemove", onGazeMouseMove);
reduceMotionPreference.addEventListener("change", onReduceMotionChange);

function autosizeInput() {
  input.style.height = "auto";
  input.style.height = `${Math.min(180, Math.max(62, input.scrollHeight))}px`;
}
function updateComposerState() {
  const engineBlocked = !frontendInitialized || (localEngineRequired && !localEngineReady);
  if (sendButton) {
    sendButton.disabled = busy || engineBlocked || (!input.value.trim() && attachedImages.length === 0 && attachedFiles.length === 0);
    sendButton.title = engineBlocked
      ? (frontendInitialized ? "ローカルAIの準備完了後に送信できます" : "初期化完了後に送信できます")
      : "Enter または Ctrl+Enter で送信 / Shift+Enterで改行 / ↑↓で入力履歴";
  }
  quickActions?.querySelectorAll("button").forEach(button => { button.disabled = busy || engineBlocked; });
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
    // テーブル（| セル | セル | 形式。2行目が |---|---| の区切り行）
    if (/^\s*\|.+\|\s*$/.test(line) && i + 1 < lines.length
      && /^\s*\|?[\s:|-]+\|?\s*$/.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      const splitRow = (row) => row.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map(c => c.trim());
      const header = splitRow(line);
      i += 2; // ヘッダ行と区切り行を消費
      const rows = [];
      while (i < lines.length && /^\s*\|.+\|\s*$/.test(lines[i])) {
        rows.push(splitRow(lines[i])); i++;
      }
      const table = document.createElement("table");
      const thead = document.createElement("thead");
      const headRow = document.createElement("tr");
      for (const cell of header) {
        const th = document.createElement("th"); appendInline(th, cell); headRow.append(th);
      }
      thead.append(headRow);
      const tbody = document.createElement("tbody");
      for (const cells of rows) {
        const tr = document.createElement("tr");
        for (let c = 0; c < header.length; c += 1) {
          const td = document.createElement("td"); appendInline(td, cells[c] ?? ""); tr.append(td);
        }
        tbody.append(tr);
      }
      table.append(thead, tbody);
      const scroller = document.createElement("div");
      scroller.className = "md-table-wrap";
      scroller.append(table);
      wrap.append(scroller);
      continue;
    }
    // 空行 → 改行
    if (line.trim() === "") { wrap.append(document.createElement("br")); i++; continue; }
    // 段落（連続する非ブロック行をまとめる）
    const pLines = [];
    while (i < lines.length && lines[i].trim() !== ""
      && !/^#{1,3}\s/.test(lines[i]) && !/^[-*+]\s/.test(lines[i])
      && !/^\d+\.\s/.test(lines[i]) && !/^```/.test(lines[i])
      && !/^>\s?/.test(lines[i]) && !/^(---+|\*\*\*+|___+)$/.test(lines[i].trim())
      && !/^\s*\|.+\|\s*$/.test(lines[i])) {
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

// ask_user の選択肢を受け取る。null / 空なら通常のクイック操作へ戻す。
function setPendingChoices(choices) {
  const items = Array.isArray(choices)
    ? choices
        .map(choice => ({ label: String(choice?.label || "").trim(), text: String(choice?.label || "").trim(), title: String(choice?.description || "").trim() }))
        .filter(item => item.label)
    : [];
  const next = items.length > 0 ? items : null;
  if (next === null && pendingChoices === null) return;
  pendingChoices = next;
  renderQuickActions();
}

function renderQuickActions() {
  if (!quickActions) return;
  quickActions.replaceChildren();
  for (const action of (pendingChoices || quickActionItems || QUICK_ACTIONS)) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = action.label;
    button.title = action.title || action.text;
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
  // 直前のエラー表示を引き継がない（エラー分岐は描画後に reply-error を付け直す）。
  reply.classList.remove("reply-error", "reply-thinking");
  // 確定した本文なので、読み上げの保留を解いてここで1回だけ知らせる。
  reply.removeAttribute("aria-busy");
  renderMarkdown(reply, text);
}

let historyThinking = false;
let historyThinkingText = "考え中";
function createTypingIndicator(label = "考え中") {
  const indicator = document.createElement("div");
  indicator.className = "typing-indicator";
  indicator.setAttribute("aria-label", label);
  const text = document.createElement("span");
  text.className = "typing-label";
  text.textContent = label;
  const dots = document.createElement("span");
  dots.className = "typing-dots";
  dots.setAttribute("aria-hidden", "true");
  for (let index = 0; index < 3; index += 1) dots.append(document.createElement("i"));
  indicator.append(text, dots);
  return indicator;
}
function setThinkingUi(active, label = "考え中") {
  // 生成中は回答欄が何度も書き変わる。書き変わるたびに読み上げられないよう保留する。
  if (active) reply.setAttribute("aria-busy", "true");
  // 進捗の文言は数秒おきに変わる。文言だけの更新で履歴全件をMarkdownから作り直さない。
  const showing = active && historyThinking ? historyLog?.querySelector(".history-thinking") : null;
  historyThinking = active;
  historyThinkingText = String(label || "考え中").replace(/…+$/, "");
  if (showing) {
    showing.querySelector(".typing-indicator")?.setAttribute("aria-label", historyThinkingText);
    const labelNode = showing.querySelector(".typing-label");
    if (labelNode) labelNode.textContent = historyThinkingText;
  } else {
    renderHistory();
  }
  if (active && !layoutShowsHistory()) {
    reply.classList.remove("reply-error", "streaming");
    reply.classList.add("reply-thinking");
    reply.replaceChildren(createTypingIndicator(historyThinkingText));
  }
}

// 回答を描画しないまま終わる中断・停止でも、回答欄の3点表示を残さない。
function clearThinkingReply() {
  // 確定描画へ到達しない中断・停止でも、live regionを保留したまま残さない。
  reply.removeAttribute("aria-busy");
  if (!reply.classList.contains("reply-thinking")) return;
  reply.classList.remove("reply-thinking");
  reply.replaceChildren();
}

function setAssistantReply(text, options = {}) {
  const source = String(text ?? "");
  if (currentLayout === "stream" && options.singlePage !== true && options.paginate !== false && source.trim()) {
    if (options.remember !== false) lastAssistantText = source;
    startTimedSubtitlePages(source);
    return;
  }
  renderReply(source);
  reply.scrollTop = 0;
  if (options.remember !== false && source.trim()) {
    lastAssistantText = source;
  }
}

function cancelSubtitlePages() {
  subtitlePageToken += 1;
  if (subtitlePageTimer) clearTimeout(subtitlePageTimer);
  subtitlePageTimer = null;
}

function subtitleUnits(text) {
  const source = String(text ?? "");
  const units = source.match(/[^。．.!?！？\n]+[。．.!?！？]?\s*|\n+/g) || [source];
  const out = [];
  for (const unit of units) {
    const chars = [...unit];
    for (let index = 0; index < chars.length; index += 54) {
      out.push(chars.slice(index, index + 54).join(""));
    }
  }
  return out.filter(Boolean);
}

function subtitlePlainText(text) {
  return sanitizeSpeechText(text)
    .replace(/^\s*```[^\n]*$/gm, "")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/^\s{0,3}(?:#{1,6}\s+|>\s?|[-*+]\s+|\d+[.)]\s+)/gm, "")
    .replace(/[*_~]{1,3}/g, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

// 実際の字幕DOMへ仮描画し、現在の画面幅・文字サイズで収まる単位に分ける。
// 固定文字数だけではモニターや日本語/英数字の幅で再び欠けるため、scrollHeightを合否に使う。
function buildSubtitlePages(text) {
  const source = subtitlePlainText(text);
  if (currentLayout !== "stream" || !source.trim()) return source ? [source] : [];
  const pages = [];
  let page = "";
  const fits = candidate => {
    renderMarkdown(reply, candidate);
    reply.scrollTop = 0;
    return reply.scrollHeight <= reply.clientHeight + 1;
  };
  for (const unit of subtitleUnits(source)) {
    const candidate = page + unit;
    if (!page && fits(candidate)) {
      page = candidate;
      continue;
    }
    if (page && fits(candidate)) {
      page = candidate;
      continue;
    }
    if (page) pages.push(page);
    page = unit;
    if (!fits(page)) {
      // 54文字でも収まらない極端な狭幅では、実測で収まる最大長を二分探索する。
      let remaining = [...page];
      while (remaining.length && !fits(remaining.join(""))) {
        let low = 1;
        let high = remaining.length;
        while (low < high) {
          const middle = Math.ceil((low + high) / 2);
          if (fits(remaining.slice(0, middle).join(""))) low = middle;
          else high = middle - 1;
        }
        pages.push(remaining.slice(0, low).join(""));
        remaining = remaining.slice(low);
      }
      page = remaining.join("");
    }
  }
  if (page) pages.push(page);
  if (pages.length) setAssistantReply(pages[0], { remember: false, singlePage: true });
  return pages;
}

function showSubtitlePage(text) {
  if (currentLayout === "stream") setAssistantReply(text, { remember: false, singlePage: true });
}

function startTimedSubtitlePages(text) {
  cancelSubtitlePages();
  const pages = buildSubtitlePages(text);
  if (pages.length <= 1) {
    if (pages[0]) showSubtitlePage(pages[0]);
    return;
  }
  const token = subtitlePageToken;
  let index = 0;
  const advance = () => {
    if (token !== subtitlePageToken) return;
    showSubtitlePage(pages[index]);
    if (++index >= pages.length) return;
    const visibleChars = [...pages[index - 1]].length;
    const delay = Math.max(1800, Math.min(6000, 900 + visibleChars * 55));
    subtitlePageTimer = setTimeout(advance, delay);
  };
  advance();
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
  markActivity(); // 居眠り中でも目覚ましでは起きる
  // 操作ボタンをページ切替で消さない。目覚まし本文は短文契約で、操作を最優先する。
  setAssistantReply(text, { paginate: false });
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
  conversationHistory.push({ role, content, at: Date.now() });
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
  // 履歴ログが見えていないときに毎メッセージ全件を Markdown 再構築するのは無駄なので
  // 描画しない（全画面切替時と表示形式の切替時に呼び直す）。
  if (!bubble.classList.contains("fullscreen") && !layoutShowsHistory()) return;
  // 「考え中」の文言更新でも作り直すため、読み返している最中に最下部へ引き戻さない。
  // 既に最下部付近にいるときだけ追従する（新しい発言は今までどおり見える）。
  const stickToBottom = historyLog.scrollHeight - historyLog.scrollTop - historyLog.clientHeight < 48;
  historyLog.replaceChildren();
  if (!conversationHistory.length && !historyThinking) {
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
    if (item.at) {
      // 今セッションで発生した発言だけ時刻を添える（復元分には時刻情報が無い）。
      const time = document.createElement("span");
      time.className = "history-time";
      time.textContent = new Date(item.at).toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" });
      label.append(time);
    }
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
  if (historyThinking && !query) {
    const row = document.createElement("div");
    row.className = "history-item assistant history-thinking";
    // 名前行は他の発言と同じ `history-meta` に入れる（表示形式ごとの並び・余白を共有する）。
    const meta = document.createElement("div");
    meta.className = "history-meta";
    const label = document.createElement("div");
    label.className = "history-role";
    label.textContent = characterManifest?.name || "むぎ";
    meta.append(label);
    const body = document.createElement("div");
    body.className = "history-body";
    body.append(createTypingIndicator(historyThinkingText));
    row.append(meta, body);
    historyLog.append(row);
  }
  if (stickToBottom) historyLog.scrollTop = historyLog.scrollHeight;
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
function finishVoice(toIdle = true, expectedVoice = currentVoice) {
  if (!expectedVoice) return;
  // 古い Audio の play() rejection / error が遅れて届いても、新しく再生中の音声を
  // 終了させない。古い URL の解放だけ行う。
  if (currentVoice !== expectedVoice) {
    URL.revokeObjectURL(expectedVoice.src);
    return;
  }
  stopLipSync();
  setVoiceCancelVisible(false);
  const voice = expectedVoice;
  currentVoice = null;
  URL.revokeObjectURL(voice.src);
  if (toIdle && animationName === "talking") setAnimation("idle");
  setTimeout(flushPendingNotice, 0);
  scheduleContinuousListening();
}
function stopVoice(toIdle = true) {
  voiceSession++; // 進行中のパイプライン読み上げ（合成待ち・連続再生）を打ち切る。
  cancelSubtitlePages();
  stopLipSync();
  setVoiceCancelVisible(false);
  if (!currentVoice) return;
  const voice = currentVoice;
  voice.pause();
  finishVoice(toIdle, voice);
}
// 読み上げが有効かどうかの学習キャッシュ（pet_speak の結果から更新）。
// false のときはストリーミング先行読み上げの合成要求を出さない（無駄な往復を省く）。
let ttsEnabledCache = null;
// 読み上げ失敗をこのセッションで既に知らせたか。
// 読み上げをONにしたのにエンジンが無いと、以前は診断ログへ書くだけで画面には何も出ず、
// キャラが黙るだけだった（利用者からは故障に見える）。かといって発話のたびに出すと
// うるさいので、原因が分かる形で1度だけ知らせる。
let voiceFailureNotified = false;
function notifyVoiceFailure(error) {
  logApp("warn", "音声の事前合成に失敗しました", errorDetail(error));
  if (voiceFailureNotified) return;
  voiceFailureNotified = true;
  const detail = String(error?.message || error || "").trim();
  status.textContent = detail
    ? `読み上げできません: ${detail}`
    : "読み上げできません。設定の「追加機能」から音声読み上げを導入してください";
}
// 合成のみ行い、再生はしない（起動時に描画と同時再生するための事前合成）。失敗・無効・空は null。
// 合成前の最後の関門。挨拶・自発雑談・目覚ましは `sanitizeSpeechText` を通さずここへ
// 直接来るため、人格プロンプトの除去はこの1か所で全経路へ効かせる。
async function prepareSpeech(text) {
  const speech = stripPersonaLeak(String(text ?? "")).trim();
  if (!speech) return null; // 混入だけの文は合成しない（無音の再生を挟まない）
  try {
    const result = await window.picoagent.speak(speech);
    if (result?.enabled === false) ttsEnabledCache = false;
    else if (result?.enabled) {
      ttsEnabledCache = true;
      voiceFailureNotified = false; // 復旧したら次の失敗はまた知らせる
    }
    if (!result?.enabled || result.skipped || !result.audio?.length) return null;
    return result;
  } catch (error) {
    notifyVoiceFailure(error);
    return null;
  }
}
// 事前合成済みの音声を再生し、会話アニメを開始する。再生終了・失敗時に Object URL を解放する。
function playPreparedSpeech(result) {
  stopVoice(false);
  setVoiceCancelVisible(true);
  const bytes = new Uint8Array(result.audio);
  const url = URL.createObjectURL(new Blob([bytes], { type: result.mime || "audio/wav" }));
  const audio = new Audio(url);
  currentVoice = audio;
  audio.volume = voiceVolume;
  audio.addEventListener("ended", () => {
    finishVoice(true, audio);
    if (!activeAlarmSnooze) status.textContent = "";
  }, { once: true });
  audio.addEventListener("error", () => {
    const audioError = audio.error?.message || "";
    finishVoice(true, audio);
    logApp("error", "音声データの読み込みに失敗しました", audioError);
    status.textContent = activeAlarmSnooze ? status.textContent : "音声再生に失敗: 音声データを読み込めません";
  }, { once: true });
  audio.addEventListener("playing", () => {
    if (currentVoice !== audio) return;
    status.textContent = "読み上げ中…";
    if (!shouldLipSync(audio)) {
      setAnimation("idle", true);
      return;
    }
    setAnimation("talking", true);
    attachLipSync(audio);
  }, { once: true });
  audio.play().catch(error => {
    finishVoice(true, audio);
    logApp("error", "音声再生に失敗しました", errorDetail(error));
    if (!activeAlarmSnooze) {
      status.textContent = `音声再生に失敗: ${error?.message || error || "再生がブロックされました"}`;
    }
  });
  status.textContent = "音声再生準備中…";
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
  out = stripPersonaLeak(out); // 人格プロンプトの混入は声に出さない
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
    audio.addEventListener("playing", () => {
      if (session !== voiceSession || currentVoice !== audio) return;
      status.textContent = "読み上げ中…";
      if (!shouldLipSync(audio)) {
        setAnimation("idle", true);
        return;
      }
      setAnimation("talking", true);
      attachLipSync(audio);
    }, { once: true });
    audio.play().catch(error => {
      logApp("error", "音声再生に失敗しました", errorDetail(error));
      cleanup();
    });
    status.textContent = "音声再生準備中…";
  });
}
async function speakReply(text) {
  const chunks = currentLayout === "stream" ? buildSubtitlePages(text) : splitSpeechChunks(text);
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
      showSubtitlePage(chunks[i]);
      if (!result) {
        // 合成失敗でも字幕を飛ばさず、読める時間だけ表示して次ページへ進む。
        const delay = Math.max(1800, Math.min(6000, 900 + [...chunks[i]].length * 55));
        await new Promise(resolve => setTimeout(resolve, delay));
        continue;
      }
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
// ── ストリーミング先行読み上げ: 応答の生成中に、確定した文から順に合成・再生する ──
// 全文の生成完了を待たずに話し始めることで、読み上げ開始までの体感遅延を大きく縮める。
// 最終本文が届いたら残り（未読分）をキューへ足して読み切る（finishStreamSpeech）。
let streamSpeech = null; // { queue, consumed, spoken, done, disabled, running }

function beginStreamSpeech() {
  streamSpeech = {
    queue: [],       // 合成待ちのチャンク（文単位）
    consumed: "",    // ストリーム本文のうちキューへ載せ終えた先頭部分
    spoken: false,   // 1つでも再生したか
    done: false,     // 最終本文まで投入済みか
    disabled: ttsEnabledCache === false,
    running: false,
  };
}

function cancelStreamSpeech() {
  if (streamSpeech?.running) stopVoice(false);
  streamSpeech = null;
}

// エージェントのステップ境界（ツール実行を挟んだ再 chat）で読み上げを仕切り直す。
// 前ステップの途中テキストは最終回答に含まれないため、キューを捨てて次の本文を待つ。
function resetStreamSpeech() {
  const disabled = streamSpeech?.disabled ?? (ttsEnabledCache === false);
  cancelStreamSpeech();
  beginStreamSpeech();
  streamSpeech.disabled = disabled;
}

// 読み上げ対象のテキストから reasoning モデルの <think> 推論ブロックを取り除く。
// バックエンドの strip_reasoning と同じ規則（閉じタグが無ければ以降すべて推論扱い）で、
// 推論内容を声に出さないようにする。戻り値は常に元テキストの「読み上げ確定済み前方部分」。
function stripThinkForSpeech(text) {
  let rest = String(text ?? "");
  let out = "";
  for (;;) {
    const start = rest.indexOf("<think>");
    if (start === -1) return out + rest;
    out += rest.slice(0, start);
    const after = rest.slice(start + "<think>".length);
    const end = after.indexOf("</think>");
    if (end === -1) return out; // 推論の途中: ここから先は確定していない
    rest = after.slice(end + "</think>".length);
  }
}

// ストリーム本文から「文として確定した部分」を切り出して読み上げキューへ足す。
function feedStreamSpeech(buffer) {
  const ss = streamSpeech;
  if (!ss || ss.disabled || ss.done) return;
  // consumed は <think> を除いた「読み上げ済みテキスト」の前方一致部分として管理する
  // （最終本文も strip_reasoning 済みのため、finishStreamSpeech の前方一致判定と揃う）。
  const cleaned = stripThinkForSpeech(buffer);
  if (cleaned.length <= ss.consumed.length) return;
  const rest = cleaned.slice(ss.consumed.length);
  const settledMatch = rest.match(/^[\s\S]*[。．.!?！？\n]/); // 最後の文末までを確定分とみなす
  if (!settledMatch) return;
  const settled = settledMatch[0];
  if (settled.trim().length < 8) return; // 極端に短い断片は次の文とまとめる
  ss.consumed += settled;
  for (const chunk of splitSpeechChunks(settled)) ss.queue.push(chunk);
  runStreamSpeech();
}

// キューを順に合成→再生する（再生中に次チャンクを先読み合成するパイプライン）。
async function runStreamSpeech() {
  const ss = streamSpeech;
  if (!ss || ss.running || ss.disabled || ss.queue.length === 0) return;
  ss.running = true;
  stopVoice(false);
  const session = ++voiceSession;
  setVoiceCancelVisible(true);
  try {
    let pending = ss.queue.shift();
    let synth = prepareSpeech(pending);
    while (pending !== undefined) {
      const result = await synth;
      if (session !== voiceSession || streamSpeech !== ss) return;
      if (!result && !ss.spoken) {
        // 最初のチャンクから合成できない（読み上げ無効など）: 先行読み上げを諦めて
        // 完了時の通常読み上げ経路（speakReply）に任せる。
        ss.disabled = true;
        ss.queue.length = 0;
        setVoiceCancelVisible(false);
        return;
      }
      pending = ss.queue.shift();
      synth = pending !== undefined ? prepareSpeech(pending) : null;
      if (result) {
        ss.spoken = true;
        await playChunkAndWait(result, session);
        if (session !== voiceSession || streamSpeech !== ss) return;
      }
      // キューが空でも生成が続いていれば、次の文が確定するまで待つ。
      while (pending === undefined && !ss.done) {
        await new Promise(resolve => setTimeout(resolve, 120));
        if (session !== voiceSession || streamSpeech !== ss) return;
        pending = ss.queue.shift();
        if (pending !== undefined) synth = prepareSpeech(pending);
      }
    }
    // 最終本文まで読み終えた。
    setVoiceCancelVisible(false);
    stopLipSync();
    if (animationName === "talking") setAnimation("idle");
    if (status.textContent === "読み上げ中…") status.textContent = "";
    setTimeout(flushPendingNotice, 0);
  } finally {
    ss.running = false;
  }
}

// 最終本文が届いたときの締め。残り（未読分）をキューへ足して読み切る。
// 先行読み上げが無効・本文不一致のときは false を返し、呼び出し側の全文読み上げに任せる。
function finishStreamSpeech(finalText) {
  const ss = streamSpeech;
  if (!ss || ss.disabled) {
    streamSpeech = null;
    return false;
  }
  const text = String(finalText ?? "");
  if (!text.startsWith(ss.consumed)) {
    // 後処理で本文が変わった（まれ）: 進行中の先行読み上げを止めて全文読み上げへ。
    cancelStreamSpeech();
    return false;
  }
  for (const chunk of splitSpeechChunks(text.slice(ss.consumed.length))) ss.queue.push(chunk);
  ss.done = true;
  runStreamSpeech(); // まだ話し始めていなければここで開始する
  return true;
}

async function renderReplySpeaking(text) {
  const token = ++speakingToken;
  const source = String(text ?? "");
  const chars = [...source];
  const stepMs = chars.length > 180 ? 14 : 22;
  // 生の Markdown 記号を見せないよう、部分テキストを都度 Markdown 描画する
  // （rAF スロットルで1フレーム1回に抑える）。
  let shown = 0;
  let frame = 0;
  const paint = () => { frame = 0; renderMarkdown(reply, chars.slice(0, shown).join("")); };
  for (; shown < chars.length; shown += 1) {
    if (token !== speakingToken) { if (frame) cancelAnimationFrame(frame); return; }
    if (!frame) frame = requestAnimationFrame(paint);
    await new Promise(resolve => setTimeout(resolve, stepMs));
  }
  if (frame) cancelAnimationFrame(frame);
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

function renderRelationship(data) {
  if (!data) return;
  relationshipState = data.relationship || data;
  if (relationshipLabel) relationshipLabel.textContent = data.bondLabel || relationshipState.bondLabel || "いっしょの相棒";
  if (relationshipStats) {
    const talks = Number(relationshipState.conversations || 0);
    const tasks = Number(relationshipState.completedTasks || 0);
    const touches = Number(relationshipState.interactions || 0);
    relationshipStats.textContent = `会話 ${talks}回 ・ 完了 ${tasks}件 ・ ふれあい ${touches}回`;
  }
  if (relationshipMoments) {
    relationshipMoments.replaceChildren();
    const moments = Array.isArray(relationshipState.recentMoments) ? relationshipState.recentMoments.slice(-5).reverse() : [];
    if (!moments.length) {
      const empty = document.createElement("div");
      empty.className = "relationship-moment";
      empty.textContent = "これからの出来事がここに残ります。";
      relationshipMoments.append(empty);
    } else {
      for (const moment of moments) {
        const row = document.createElement("div");
        row.className = "relationship-moment";
        row.dataset.pinned = String(moment.pinned === true);
        const text = document.createElement("div");
        text.textContent = moment.text;
        row.append(text);
        const actions = document.createElement("div");
        actions.className = "relationship-moment-actions";
        if (moment.resumePrompt) {
          const resume = document.createElement("button");
          resume.type = "button";
          resume.dataset.action = "resume";
          resume.dataset.prompt = moment.resumePrompt;
          resume.textContent = "続きをやる";
          actions.append(resume);
        }
        const pin = document.createElement("button");
        pin.type = "button";
        pin.dataset.action = "pin";
        pin.dataset.id = moment.id;
        pin.dataset.pinned = String(moment.pinned === true);
        pin.textContent = moment.pinned ? "固定解除" : "固定";
        const remove = document.createElement("button");
        remove.type = "button";
        remove.dataset.action = "delete";
        remove.dataset.id = moment.id;
        remove.textContent = "削除";
        remove.picoConfirmation = twoStepAction(remove, {
          initialLabel: "削除",
          confirmLabel: "もう一度押す",
          onArm: () => showToast("この記録だけを消します。もう一度押すと実行します", "info"),
        });
        actions.append(pin, remove);
        row.append(actions);
        relationshipMoments.append(row);
      }
    }
  }
  if (relationshipClear) relationshipClear.hidden = !relationshipState.recentMoments?.length;
}

function setRelationshipBusy(busy) {
  relationshipCard?.setAttribute("aria-busy", String(busy));
  relationshipMoments?.querySelectorAll("button").forEach(button => { button.disabled = busy; });
  if (relationshipClear) relationshipClear.disabled = busy;
}

async function recordRelationship(kind, detail = "") {
  if (!relationshipMemoryEnabled) return;
  try {
    const result = await window.picoagent.relationshipEvent(kind, detail);
    renderRelationship(result);
  } catch (error) {
    logApp("warn", "いっしょの記録を更新できませんでした", errorDetail(error));
  }
}

// ふれあいは連打される導線なので、記録はまとめて間引く。1回ごとに送ると
// そのたびにバックエンドが関係性ファイルを読み書きすることになる。
const TOUCH_RECORD_INTERVAL_MS = 5000;
let lastTouchRecordAt = 0;
let pendingTouchZone = "";
let pendingTouchTimer = null;
function recordTouch(zone) {
  const now = Date.now();
  if (now - lastTouchRecordAt >= TOUCH_RECORD_INTERVAL_MS) {
    lastTouchRecordAt = now;
    recordRelationship(`touch_${zone}`);
    return;
  }
  // 間引いた分は最後の1回だけ、間隔が空いてから送る（反応そのものは毎回返している）。
  pendingTouchZone = zone;
  if (pendingTouchTimer) return;
  pendingTouchTimer = setTimeout(() => {
    pendingTouchTimer = null;
    lastTouchRecordAt = Date.now();
    recordRelationship(`touch_${pendingTouchZone}`);
  }, TOUCH_RECORD_INTERVAL_MS - (now - lastTouchRecordAt));
}

relationshipButton?.addEventListener("click", async event => {
  event.stopPropagation();
  relationshipCard.hidden = !relationshipCard.hidden;
  relationshipButton.setAttribute("aria-expanded", String(!relationshipCard.hidden));
  if (!relationshipCard.hidden) {
    relationshipButton.disabled = true;
    relationshipMoments.textContent = "記録を読み込んでいます…";
    try {
      renderRelationship(await window.picoagent.relationshipState());
    } catch (error) {
      logApp("warn", "いっしょの記録を読めませんでした", errorDetail(error));
      relationshipMoments.textContent = `記録を読み込めませんでした: ${error?.message || error}`;
    } finally {
      relationshipButton.disabled = false;
    }
    if (!relationshipCard.hidden) document.getElementById("relationshipClose")?.focus();
  }
});
document.getElementById("relationshipClose")?.addEventListener("click", () => {
  relationshipCard.hidden = true;
  relationshipButton?.setAttribute("aria-expanded", "false");
  relationshipButton?.focus();
});

relationshipMoments?.addEventListener("click", async event => {
  const button = event.target instanceof Element ? event.target.closest("button[data-action]") : null;
  if (!button) return;
  try {
    if (button.dataset.action === "resume") {
      relationshipCard.hidden = true;
      relationshipButton?.setAttribute("aria-expanded", "false");
      openComposer();
      setInputValue(button.dataset.prompt || "");
      input.focus();
      return;
    }
    if (button.dataset.action === "delete" && !button.picoConfirmation?.request()) return;
    setRelationshipBusy(true);
    if (button.dataset.action === "delete") button.textContent = "削除中…";
    const result = button.dataset.action === "pin"
      ? await window.picoagent.pinRelationshipMoment(button.dataset.id, button.dataset.pinned !== "true")
      : await window.picoagent.deleteRelationshipMoment(button.dataset.id);
    renderRelationship(result);
  } catch (error) {
    logApp("warn", "いっしょの記録を更新できませんでした", errorDetail(error));
    showToast("記録を更新できませんでした", "error");
    if (button.dataset.action === "delete") button.textContent = "削除";
  } finally {
    setRelationshipBusy(false);
  }
});

const relationshipClearConfirmation = relationshipClear ? twoStepAction(relationshipClear, {
  initialLabel: "記録をすべて消す",
  confirmLabel: "もう一度押してすべて消す",
  onArm: () => showToast("このキャラクターとの記録をすべて消します。もう一度押すと実行します", "info"),
}) : null;
relationshipClear?.addEventListener("click", async () => {
  if (!relationshipClearConfirmation.request()) return;
  setRelationshipBusy(true);
  relationshipClear.textContent = "記録を削除中…";
  try {
    renderRelationship(await window.picoagent.clearRelationship());
    showToast("いっしょの記録を消しました", "success");
  } catch (error) {
    logApp("warn", "いっしょの記録を消せませんでした", errorDetail(error));
    showToast("記録を消せませんでした", "error");
  } finally {
    setRelationshipBusy(false);
    relationshipClear.textContent = "記録をすべて消す";
  }
});

function queueNotice(line) {
  const text = String(line ?? "").trim();
  if (!text) return;
  speechNoticeQueue.push(text);
  if (speechNoticeQueue.length > 6) speechNoticeQueue.shift();
}

async function flushPendingNotice() {
  if (!speechNoticeQueue.length || !canShowNotice()) return;
  const line = speechNoticeQueue.shift();
  if (!line.trim()) return;
  await showAssistantNotice(line, { enqueue: false });
}

bubble.addEventListener("click", event => {
  if (event.target.closest("button,a,textarea,.reply-actions")) return;
  // テキスト選択中（コピー操作）はフキダシを入力欄へ切り替えない。
  if (String(window.getSelection?.() ?? "").trim()) return;
  openComposer();
});
character.addEventListener("click", event => {
  if (event.target instanceof Element && event.target.closest("button")) return;
  // 立ち絵を動かした直後のクリックは、はねる反応ではなくドラッグの終わりなので無視する。
  // クリックのたびに小さくはねる（連打時はアニメーションを巻き戻して再生）。
  character.classList.remove("boop");
  void character.offsetWidth;
  character.classList.add("boop");
  // Live2D はタップモーションで反応する（boop の CSS は img にしか効かない）。
  if (live2dActive && !currentVoice) playLive2DMotion("talking");
  const rect = character.getBoundingClientRect();
  const y = (event.clientY - rect.top) / Math.max(1, rect.height);
  const x = (event.clientX - rect.left) / Math.max(1, rect.width);
  const zone = y < .34 ? "head" : (y < .67 && (x < .3 || x > .7)) ? "cheek" : y > .7 ? "hand" : "head";
  const reactions = characterManifest?.interactions?.reactions?.[zone];
  const reaction = Array.isArray(reactions) && reactions.length ? reactions[Math.floor(Math.random() * reactions.length)] : ({ head: "ふふっ", cheek: "ちょっと照れるね", hand: "どうしたの？" }[zone]);
  if (reaction) {
    status.textContent = reaction;
    setTimeout(() => { if (status.textContent === reaction) status.textContent = ""; }, 1600);
  }
  setAnimation(zone === "cheek" ? "shy" : "pleased", true);
  setTimeout(() => { if (!currentVoice) setAnimation("idle"); }, 850);
  recordTouch(zone);
  if (!currentVoice) return;
  event.stopPropagation();
  stopVoice();
  status.textContent = "";
});
character.addEventListener("keydown", event => {
  if (event.key !== "Enter" && event.key !== " ") return;
  event.preventDefault();
  character.click();
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
  // キャラを避ける自動配置はペット表示の仕組み。他の形式では CSS の配置を壊さない。
  if (!layoutUsesFreeBubble() && !bubble.classList.contains("fullscreen")) return;
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
    label.textContent = preview(file.name, 20);
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
  renderAttachmentSuggestions();
}

function renderAttachmentSuggestions() {
  if (!attachmentSuggestions) return;
  const extensions = new Set(attachedFiles.map(file => String(file.name || "").split(".").pop().toLowerCase()));
  const suggestions = [];
  const add = (label, text) => { if (!suggestions.some(item => item.label === label)) suggestions.push({ label, text }); };
  if (extensions.has("pdf")) { add("PDFを要約", "添付したPDFを要約して"); add("本文を抽出", "添付したPDFの本文を抽出して"); }
  if (["wav", "mp3", "m4a", "flac", "ogg"].some(ext => extensions.has(ext))) { add("文字起こし", "添付した音声を文字起こしして"); add("議事録にする", "添付した音声から議事録を作って"); }
  if (["psd", "xcf", "heic", "heif", "tif", "tiff", "avif"].some(ext => extensions.has(ext))) { add("PNGに変換", "添付した画像をPNGに変換して"); add("画像情報", "添付した画像のサイズとレイヤー情報を教えて"); }
  if (["docx", "xlsx", "pptx", "odt", "ods", "odp"].some(ext => extensions.has(ext))) { add("内容を要約", "添付した文書を要約して"); add("PDFに変換", "添付した文書をPDFに変換して"); }
  if (attachedFiles.length + attachedImages.length > 1) add("まとめて比較", "添付したファイルを比較して、違いと要点を整理して");
  if (attachedImages.length > 0) add("画像を説明", "添付した画像の内容を詳しく説明して");
  attachmentSuggestions.replaceChildren();
  for (const suggestion of suggestions.slice(0, 5)) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = suggestion.label;
    button.addEventListener("click", () => {
      setInputValue(suggestion.text);
      autosizeInput();
      input.focus();
    });
    attachmentSuggestions.append(button);
  }
  attachmentSuggestions.hidden = suggestions.length === 0;
}

function fileIcon(name) {
  const ext = name.split(".").pop().toLowerCase();
  const map = { xlsx: "📊", xls: "📊", xlsm: "📊", csv: "📋", tsv: "📋",
    docx: "📝", doc: "📝", docm: "📝", pptx: "📄", pptm: "📄", ppt: "📄",
    pdf: "📕", json: "📦", toml: "📦", yaml: "🗂", yml: "🗂",
    txt: "📃", md: "📃", log: "📃",
    psd: "🎨", xcf: "🎨", heic: "🖼", heif: "🖼", tif: "🖼", tiff: "🖼",
    webp: "🖼", avif: "🖼", bmp: "🖼", ico: "🖼" };
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
  reader.onerror = () => {
    logApp("error", "画像の読み込みに失敗しました", `${file.name}: ${reader.error?.message || ""}`);
    showToast("画像を読み込めませんでした", "error");
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
    showToast(`${preview(file.name, 18)} を添付`, "success");
  };
  reader.onerror = () => {
    logApp("error", "ファイルの読み込みに失敗しました", `${file.name}: ${reader.error?.message || ""}`);
    showToast("ファイルを読み込めませんでした", "error");
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

// ── ネイティブ D&D: ウィンドウのどこへでも（キャラの上でも）ドロップして添付できる ──
// Tauri は既定でネイティブのファイルドラッグを横取りするため（dragDropEnabled）、
// HTML5 の drop にはファイルが届かない。tauri://drag-* イベントで受けて添付フローへ流す。
const dropOverlay = document.getElementById("dropOverlay");

function setDropOverlay(visible) {
  if (dropOverlay) dropOverlay.hidden = !visible;
}

async function handleNativeDrop(paths) {
  markActivity();
  let added = 0;
  const failures = [];
  for (const path of (paths || []).slice(0, MAX_ATTACHED_IMAGES + MAX_ATTACHED_FILES)) {
    try {
      const info = await window.picoagent.readDroppedPath(path);
      if (info.kind === "image") {
        if (attachedImages.length >= MAX_ATTACHED_IMAGES) {
          failures.push(`画像は最大 ${MAX_ATTACHED_IMAGES} 枚までです`);
          continue;
        }
        attachedImages.push(info.dataUrl);
      } else {
        if (attachedFiles.length >= MAX_ATTACHED_FILES) {
          failures.push(`ファイルは最大 ${MAX_ATTACHED_FILES} 件までです`);
          continue;
        }
        attachedFiles.push({ name: info.name, dataUrl: info.dataUrl, path: info.path });
      }
      added += 1;
    } catch (error) {
      logApp("warn", "ドロップファイルの読み込みに失敗しました", `${path}: ${error}`);
      failures.push(String(error));
    }
  }
  if (added > 0) {
    renderAttachmentPreview();
    updateComposerState();
    if (composer.hidden) openComposer();
    // キャラがファイルを受け取った反応（クリック時と同じはねる演出）。
    character.classList.remove("boop");
    void character.offsetWidth;
    character.classList.add("boop");
  }
  if (failures.length === 0) {
    if (added > 0) showToast(`${added} 件を添付しました`, "success");
    return;
  }
  const permissionFailure = failures.some(message => message.includes("許可ルート") || message.includes("保護パス"));
  const prefix = added > 0 ? `${added} 件を添付、` : "";
  if (permissionFailure) {
    showToast(`${prefix}許可されていない場所のファイルを読み込めませんでした`, "warn", {
      duration: 7000,
      actionLabel: "許可設定を開く",
      onAction: () => window.picoagent.openSettings("permissions"),
    });
  } else {
    const detail = failures.length === 1 ? failures[0] : `${failures.length} 件を読み込めませんでした`;
    showToast(`${prefix}${detail}`, "error", { duration: 4500 });
  }
}

for (const [channel, callback] of [
  ["tauri://drag-enter", () => setDropOverlay(true)],
  ["tauri://drag-leave", () => setDropOverlay(false)],
  ["tauri://drag-drop", (payload) => {
    setDropOverlay(false);
    handleNativeDrop(payload?.paths);
  }],
]) {
  window.picoagent.listen(channel, callback).catch(error => {
    logApp("warn", `${channel} の購読に失敗しました`, errorDetail(error));
  });
}

// ── テキスト/URL のドロップ: ブラウザ等からの文字列は入力欄へ挿入する ─────────
document.addEventListener("dragover", (e) => {
  const types = e.dataTransfer ? [...e.dataTransfer.types] : [];
  if (types.includes("text/plain") || types.includes("text/uri-list")) e.preventDefault();
});
document.addEventListener("drop", (e) => {
  if (!e.dataTransfer || e.dataTransfer.files.length > 0) return;
  const text = (e.dataTransfer.getData("text/uri-list") || e.dataTransfer.getData("text/plain")).trim();
  if (!text) return;
  e.preventDefault();
  markActivity();
  if (composer.hidden) openComposer();
  setInputValue(input.value.trim() ? `${input.value.trimEnd()} ${text}` : text);
  autosizeInput();
  input.focus();
});

// 画面を見てほしい系の発話を検出する（スクリーンショット自動添付に使用）。
function isScreenshotQuery(text) {
  return /画面を見て|スクリーンショット|screenshot|画面の状態|画面を確認|今の画面|デスクトップを見て|画面キャプチャ/.test(text.toLowerCase());
}

composer.addEventListener("submit", async event => {
  event.preventDefault();
  const text = input.value.trim();
  if (!frontendInitialized) {
    setStatus("初期化を続けています。完了すると送信できます");
    status.classList.add("engine-pending");
    return;
  }
  if (localEngineRequired && !localEngineReady) {
    setStatus(localEnginePhase === "error"
      ? "ローカルAIの準備に失敗しています。設定の「診断・修復」を確認してください"
      : "ローカルAIを準備中です。完了すると送信できます");
    status.classList.add(localEnginePhase === "error" ? "engine-error" : "engine-pending");
    return;
  }
  if ((!text && attachedImages.length === 0 && attachedFiles.length === 0) || busy) return;
  const requestRevision = conversationRevision;
  const jobId = `chat-${Date.now()}-${requestRevision}`;
  let jobState = "running";
  let jobDidWork = false;
  let jobToolFailed = false;
  const jobTools = new Set();
  clearAlarmSnooze(false);
  markActivity();
  lastUserText = text; // エラーリトライ用に保持
  busy = true;
  cancelSubtitlePages();
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
  status.textContent = "";
  reply.classList.remove("reply-error"); // 前回のエラー表示をリセット
  status.classList.add("busy"); // 送信中状態。会話の待機表示はチャット欄の3点UIへ出す。
  setAnimation("thinking");
  // 処理中は停止ボタンを出す（長いエージェント実行を途中でやめられるように）。
  if (cancelSendButton) { cancelSendButton.hidden = false; cancelSendButton.disabled = false; }
  addHistory("user", text);
  setThinkingUi(true);
  upsertJob(jobId, { title: text || "添付ファイルの処理", body: "考え中…", status: "running", prompt: text, at: new Date().toISOString() });

  // スーパーエージェントモードの進捗（計画→実行）をステータス行に表示する。
  try {
    if (window.picoagent.listen) {
      if (unlistenStatus) { unlistenStatus(); unlistenStatus = null; } // 多重購読防止
      unlistenStatus = await window.picoagent.listen("pet:agent_status", (progress) => {
        if (busy && typeof progress === "string" && progress) {
          // 改行は1行表示に収める（計画本文など複数行が来てもステータス行が崩れないように）。
          status.textContent = progress.replace(/\s*\n+\s*/g, " / ");
          setThinkingUi(true, status.textContent);
          upsertJob(jobId, { body: status.textContent, status: "running" });
          if (/検索|search|web|ブラウザ/i.test(progress)) setAnimation("searching", true);
          else if (/ツールを実行中|ステップ/.test(progress)) setAnimation("working", true);
          else if (/計画|考え/.test(progress)) setAnimation("thinking", true);
        }
      });
    }
  } catch (error) {
    logApp("warn", "進捗購読の設定に失敗しました", errorDetail(error));
  }

  // 表示文言ではなく構造化イベントで、実際に動いたツールと成否を追跡する。
  try {
    if (window.picoagent.listen) {
      if (unlistenActivity) { unlistenActivity(); unlistenActivity = null; }
      unlistenActivity = await window.picoagent.listen("pet:agent_activity", activity => {
        if (!busy || !activity || typeof activity !== "object") return;
        const tools = Array.isArray(activity.tools) ? activity.tools.filter(name => typeof name === "string") : [];
        for (const name of tools) jobTools.add(name);
        if (activity.phase === "started") jobDidWork = tools.length > 0;
        if (activity.phase === "completed" && activity.success === false) jobToolFailed = true;
      });
    }
  } catch (error) {
    logApp("warn", "作業結果購読の設定に失敗しました", errorDetail(error));
  }

  // ストリーミングチャンクをリアルタイム表示する（rAF スロットルで1フレーム1描画）。
  streamBuffer = "";
  beginStreamSpeech(); // 生成中の先行読み上げを準備（読み上げ無効なら何もしない）
  try {
    if (window.picoagent.listen) {
      if (unlistenStream) { unlistenStream(); unlistenStream = null; } // 多重購読防止
      // ステップ境界（ツール実行を挟んだ再 chat）では、前ステップの途中テキストを捨てて
      // 最終回答だけを表示・読み上げする。
      if (unlistenStreamReset) { unlistenStreamReset(); unlistenStreamReset = null; }
      unlistenStreamReset = await window.picoagent.listen("pet:stream_reset", () => {
        if (!busy || requestRevision !== conversationRevision) return;
        if (!streamBuffer) return;
        streamBuffer = "";
        // 表示にも残さない（最終回答に含まれない途中テキストを見せ続けない）。
        if (streamRenderFrame) { cancelAnimationFrame(streamRenderFrame); streamRenderFrame = null; }
        renderMarkdown(reply, "");
        setThinkingUi(true, status.textContent || "考え中");
        resetStreamSpeech();
      });
      unlistenStream = await window.picoagent.listen("pet:stream_chunk", (chunk) => {
        if (!busy || requestRevision !== conversationRevision) return;
        streamBuffer += chunk;
        if (historyThinking) setThinkingUi(false);
        if (currentLayout !== "stream") feedStreamSpeech(streamBuffer); // 配信字幕は検査済み全文をページ表示する
        reply.classList.add("streaming"); // 文末の点滅カーソル表示
        if (currentLayout === "stream") return;
        if (!streamRenderFrame) {
          streamRenderFrame = requestAnimationFrame(() => {
            streamRenderFrame = null;
            // ユーザーが上へスクロール中でなければ、生成に合わせて下端へ追従する。
            const nearBottom = reply.scrollHeight - reply.scrollTop - reply.clientHeight < 48;
            renderMarkdown(reply, streamBuffer);
            if (nearBottom) reply.scrollTop = reply.scrollHeight;
            if (animationName === "thinking") setAnimation("talking");
          });
        }
      });
    }
  } catch (error) {
    logApp("warn", "ストリーミング購読の設定に失敗しました", errorDetail(error));
  }

  try {
    // ── ファイル内容をテキストへ変換してコンテキストに付加 ──────────────────
    let fileContext = "";
    if (pendingFiles.length > 0 && window.picoagent.readFileContent) {
      status.textContent = "ファイルを読み込み中…";
      for (const f of pendingFiles) {
        // ネイティブD&D由来の添付は実パスも伝え、PDF抽出や画像変換などのスキルが直接使えるようにする。
        const label = f.path ? `${f.name}（パス: ${f.path}）` : f.name;
        try {
          const extracted = await window.picoagent.readFileContent(f.name, f.dataUrl);
          fileContext += `[添付ファイル: ${label}]\n${extracted}\n\n`;
        } catch (err) {
          logApp("warn", `ファイル読み込み失敗: ${f.name}`, String(err));
          fileContext += `[添付ファイル: ${label}] (読み込みに失敗しました)\n\n`;
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
      jobState = "cancelled";
      setThinkingUi(false);
      clearThinkingReply();
      cancelStreamSpeech();
      status.textContent = "";
      return;
    }
    if (String(result?.reply || "").includes("いまの処理は途中で止めたよ")) jobState = "cancelled";
    setThinkingUi(false);
    status.textContent = result.error ? "接続を確認してね" : "";
    // 質問カードの選択肢を反映する（choices が無い応答で通常のクイック操作へ戻る）。
    setPendingChoices(result.error ? null : result.choices);
    if (result.error) {
      jobState = "failed";
      logApp("error", "AI応答がエラーを返しました", result.reply || "");
      ++speakingToken;
      cancelStreamSpeech();
      stopVoice();
      renderReply(result.reply);
      reply.classList.add("reply-error");
      addRetryButton();
      addHistory("assistant", result.reply);
      setAnimation("sad");
      setTimeout(() => setAnimation("idle"), 2200);
    } else {
      // バックエンド処理は完了しているので、処理中表示はここで消す（読み上げ・描画は続く）。
      status.classList.remove("busy");
      if (cancelSendButton) cancelSendButton.hidden = true;
      // 返信がユーザー入力のオウム返しのときは読み上げない（入力は声に出さない）。
      if (String(result.reply ?? "").trim() !== text) {
        // ストリーミング中に先行読み上げが走っていれば残りを読み切り、無ければ全文読み上げ。
        if (currentLayout === "stream") {
          cancelStreamSpeech();
          if (ttsEnabledCache === false || voiceVolume <= 0.001) startTimedSubtitlePages(result.reply);
          else speakReply(result.reply);
        } else if (!finishStreamSpeech(result.reply)) speakReply(result.reply);
      } else {
        cancelStreamSpeech();
      }
      if (currentLayout === "stream") {
        // 字幕ページャーまたはページ同期読み上げが表示を所有する。
      } else if (streamBuffer.trim()) {
        // ストリーミングで本文は表示済み。消してタイプライターで打ち直さず、最終形を確定描画する。
        setAssistantReply(result.reply);
        if (!currentVoice) setAnimation("idle");
      } else {
        await renderReplySpeaking(result.reply);
      }
      addHistory("assistant", result.reply);
      if (result.quitApp) scheduleAppQuit();
    }
  } catch (error) {
    jobState = "failed";
    setThinkingUi(false);
    logApp("error", "AI応答の取得に失敗しました", errorDetail(error));
    ++speakingToken;
    cancelStreamSpeech();
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
    setThinkingUi(false);
    clearThinkingReply();
    status.classList.remove("busy");
    if (cancelSendButton) cancelSendButton.hidden = true;
    reply.classList.remove("streaming");
    // ストリーム・進捗リスナーを解除する。
    if (streamRenderFrame) { cancelAnimationFrame(streamRenderFrame); streamRenderFrame = null; }
    if (unlistenStream) { unlistenStream(); unlistenStream = null; }
    if (unlistenStreamReset) { unlistenStreamReset(); unlistenStreamReset = null; }
    streamBuffer = "";
    if (unlistenStatus) { unlistenStatus(); unlistenStatus = null; }
    if (unlistenActivity) { unlistenActivity(); unlistenActivity = null; }
    busy = false;
    const jobFailed = jobState === "failed";
    const jobCancelled = jobState === "cancelled";
    upsertJob(jobId, {
      body: jobFailed ? "処理に失敗しました。詳細は会話を確認してください。" : jobCancelled ? "処理を中断しました。" : "処理が完了しました。",
      status: jobFailed ? "failed" : jobCancelled ? "cancelled" : "completed",
      ok: !jobFailed,
    });
    updateComposerState();
    if (!jobCancelled) {
      const relationshipDetail = JSON.stringify({ tools: [...jobTools], resumePrompt: lastUserText });
      if (jobFailed || jobToolFailed) {
        setAnimation("concern", true);
        if (jobDidWork) recordRelationship("task_failure", relationshipDetail);
      } else {
        setAnimation("success", true);
        if (jobDidWork) recordRelationship("task_success", relationshipDetail);
      }
      setTimeout(() => { if (!currentVoice && !micStream) setAnimation("idle"); }, 1100);
    }
    scheduleChatter(); // 会話後はアイドル計測をやり直す。
    setTimeout(flushPendingNotice, 0);
    scheduleContinuousListening();
  }
});
// ── 音声入力（マイク → WAV → ローカルWhisper文字起こし） ──────────────
const micButton = document.getElementById("micButton");
const voiceLoopButton = document.getElementById("voiceLoopButton");
let micStream = null;
let micStarting = false;
let micStartRequest = 0;
let micCtx = null;
let micNode = null;
let micChunks = [];
let micSampleRate = 16000;
// 録音の上限バイト数（16bit モノラル WAV 換算）。バックエンド pet_transcribe_audio の
// 上限は 30MB で、超えると「音声が大きすぎます」で確定に失敗する。上限に当たってから
// 弾くと録音した内容がまるごと無駄になるうえ、それまでの PCM を WebView 上に抱え続けて
// メモリも食う。そこで録音側で先に打ち切り、そこまでの内容を確定させる。
// 16kHz なら約 14 分、WebView が 16kHz 非対応で 48kHz になる環境でも約 4.7 分で止まる。
const MAX_RECORD_BYTES = 27 * 1024 * 1024;
let micSamples = 0;        // 録音した総サンプル数（上限判定用）
let micAutoStopping = false;
let micSpeechDetected = false;
let micLastVoiceAt = 0;
// 連続会話はマイクの意図しない再開を避けるため、起動ごとにOFFから始める。
let continuousVoice = false;
let continuousVoiceTimer = null;
let pendingVoiceSubmitTimer = null;
let micNoiseFloor = 0;
let micNoiseFrames = 0;
let micStartedAt = 0;
// 逐次文字起こし（リアルタイム風プレビュー）の状態。録音中に数秒ごとの区間を裏で
// 文字起こしして入力欄へ出し、停止時に全体の文字起こしで置き換えて確定する。
let micSession = 0;        // 録音の世代。停止後に届いた途中結果を捨てる
let micInterimTimer = null;
let micInterimBusy = false;
let micInterimIndex = 0;   // 変換済みチャンク数（micChunks の先頭からの消費位置）
let micInterimTexts = [];
let micInterimBase = "";   // 録音開始時点の入力欄の内容

function blobToDataUrl(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

// 録音中の未変換区間（約2秒以上たまったら）を逐次文字起こしして入力欄へ反映する。
// 区間切りの境界では単語が欠けることがあるため、確定は停止時の全体文字起こしで行う。
async function transcribeInterim(session) {
  if (micInterimBusy || session !== micSession || !micStream) return;
  const from = micInterimIndex;
  const to = micChunks.length;
  const seconds = micChunks.slice(from, to).reduce((sum, c) => sum + c.length, 0) / micSampleRate;
  if (seconds < 2) return;
  micInterimBusy = true;
  micInterimIndex = to;
  try {
    const wav = encodeWav(micChunks.slice(from, to), micSampleRate);
    const b64 = await blobToDataUrl(new Blob([wav], { type: "audio/wav" }));
    const result = await window.picoagent.transcribeAudio(b64);
    if (session !== micSession) return; // 停止・再録音後に届いた途中結果は捨てる
    const text = String(result?.text ?? "").trim();
    if (text) {
      micInterimTexts.push(text);
      const joined = micInterimTexts.join(" ");
      setInputValue(micInterimBase ? `${micInterimBase} ${joined}` : joined);
      status.textContent = "🎙 録音中…（認識中の文はあとで確定します）";
    }
  } catch (error) {
    // Whisper 未設定などで失敗したら、この録音では逐次変換を止める（停止時の一括変換に任せる）。
    logApp("warn", "途中の文字起こしに失敗しました", errorDetail(error));
    if (micInterimTimer) { clearInterval(micInterimTimer); micInterimTimer = null; }
  } finally {
    micInterimBusy = false;
  }
}

// Float32 の PCM を 16bit モノラル WAV に変換する（whisper-cli は wav 入力のため）。
function encodeWav(chunks, sampleRate) {
  const length = chunks.reduce((sum, c) => sum + c.length, 0);
  const buffer = new ArrayBuffer(44 + length * 2);
  const view = new DataView(buffer);
  const writeStr = (offset, text) => { for (let i = 0; i < text.length; i += 1) view.setUint8(offset + i, text.charCodeAt(i)); };
  writeStr(0, "RIFF"); view.setUint32(4, 36 + length * 2, true); writeStr(8, "WAVE");
  writeStr(12, "fmt "); view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true); view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  writeStr(36, "data"); view.setUint32(40, length * 2, true);
  let offset = 44;
  for (const chunk of chunks) {
    for (let i = 0; i < chunk.length; i += 1) {
      const v = Math.max(-1, Math.min(1, chunk[i]));
      view.setInt16(offset, v < 0 ? v * 0x8000 : v * 0x7fff, true);
      offset += 2;
    }
  }
  return buffer;
}

function stopMicCapture() {
  micStartRequest += 1;
  micStarting = false;
  if (micButton) micButton.disabled = false;
  micSession++; // 進行中の逐次文字起こし結果を無効化する
  if (micInterimTimer) { clearInterval(micInterimTimer); micInterimTimer = null; }
  micNode?.disconnect();
  micNode = null;
  micStream?.getTracks().forEach(track => track.stop());
  micStream = null;
  const ctx = micCtx;
  micCtx = null;
  ctx?.close().catch(error => {
    logApp("warn", "マイク用の音声処理を解放できませんでした", errorDetail(error));
  });
  micButton?.classList.remove("recording");
  petApp.classList.remove("is-listening");
  if (!busy && !currentVoice) setAnimation("idle");
}

function cancelPendingVoiceSubmit(message = "音声の送信を取り消しました") {
  if (!pendingVoiceSubmitTimer) return false;
  clearTimeout(pendingVoiceSubmitTimer);
  pendingVoiceSubmitTimer = null;
  status.textContent = "";
  showToast(message, "warn");
  input.focus();
  return true;
}

function scheduleVoiceSubmit() {
  if (!continuousVoice || !input.value.trim() || busy) return;
  if (pendingVoiceSubmitTimer) clearTimeout(pendingVoiceSubmitTimer);
  status.textContent = "1.5秒後に送信（Escで取消）";
  pendingVoiceSubmitTimer = setTimeout(() => {
    pendingVoiceSubmitTimer = null;
    status.textContent = "";
    if (continuousVoice && input.value.trim() && !busy) composer.requestSubmit();
  }, 1500);
}

async function finishRecording() {
  // 上限による自動確定と、その直後のボタン操作が二重に走らないようにする。
  if (!micStream) return;
  const chunks = micChunks;
  micChunks = [];
  micSamples = 0;
  const sampleRate = micSampleRate;
  const base = micInterimBase;      // 録音開始時点の入力欄（逐次プレビューを除いた確定部分）
  const hadInterim = micInterimTexts.length > 0;
  stopMicCapture();
  const seconds = chunks.reduce((sum, c) => sum + c.length, 0) / sampleRate;
  if (seconds < 0.4) { status.textContent = ""; showToast("録音が短すぎました", "warn"); return; }
  status.textContent = hadInterim ? "文字起こしを確定中…" : "文字起こし中…";
  status.classList.add("busy");
  try {
    const wav = encodeWav(chunks, sampleRate);
    const b64 = await blobToDataUrl(new Blob([wav], { type: "audio/wav" }));
    const result = await window.picoagent.transcribeAudio(b64);
    const text = String(result?.text ?? "").trim();
    if (text) {
      // 逐次プレビューは区間切りの境界で欠けることがあるため、全体の文字起こしで置き換えて確定する。
      setInputValue(base ? `${base} ${text}` : text);
      input.focus();
      showToast("文字起こししました", "success");
    }
  } catch (error) {
    logApp("error", "音声入力に失敗しました", errorDetail(error));
    // 逐次プレビューが入力欄に残っていれば、それを最終結果として使う。
    if (!hadInterim) showToast(preview(error?.message || error || "文字起こしに失敗しました", 60), "error");
    else showToast("確定に失敗したため途中結果を残しました", "warn");
  } finally {
    status.classList.remove("busy");
    if (status.textContent === "文字起こし中…" || status.textContent === "文字起こしを確定中…") status.textContent = "";
    scheduleVoiceSubmit();
  }
}

async function startRecording() {
  if (micStream || micStarting) return;
  const request = ++micStartRequest;
  micStarting = true;
  if (micButton) micButton.disabled = true;
  stopVoice(false); // 読み上げ中でも話し始められるよう、マイク操作を割り込みとして扱う。
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
    if (request !== micStartRequest || document.hidden) {
      stream.getTracks().forEach(track => track.stop());
      return;
    }
    micStream = stream;
  } catch (error) {
    logApp("warn", "マイクを利用できません", errorDetail(error));
    showToast("マイクを利用できません（権限を確認してね）", "error");
    micStream = null;
    return;
  } finally {
    if (request === micStartRequest && !micStream) {
      micStarting = false;
      if (micButton) micButton.disabled = false;
    }
  }
  try {
    micCtx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 16000 });
    micSampleRate = micCtx.sampleRate; // 16kHz 非対応環境では実サンプルレートで記録する
    const source = micCtx.createMediaStreamSource(micStream);
    // ScriptProcessor は非推奨だが WebView 互換性が高い（AudioWorklet はファイル分離が必要）。
    micNode = micCtx.createScriptProcessor(4096, 1, 1);
    micChunks = [];
    micSamples = 0;
    micAutoStopping = false;
    micSpeechDetected = false;
    micStartedAt = performance.now();
    micLastVoiceAt = micStartedAt;
    micNoiseFloor = 0;
    micNoiseFrames = 0;
    const maxSamples = Math.floor((MAX_RECORD_BYTES - 44) / 2);
    micNode.onaudioprocess = (event) => {
      if (micAutoStopping) return;
      const raw = event.inputBuffer.getChannelData(0);
      // 上限をまたぐフレームは入る分だけ取り込む。フレーム丸ごと足してから判定すると、
      // フレーム長しだいで上限を超えた WAV ができてしまう（バックエンドが弾く）。
      const room = maxSamples - micSamples;
      const frame = new Float32Array(raw.length <= room ? raw : raw.subarray(0, room));
      if (frame.length > 0) {
        micChunks.push(frame);
        micSamples += frame.length;
        let energy = 0;
        for (let i = 0; i < frame.length; i += 1) energy += frame[i] * frame[i];
        const rms = Math.sqrt(energy / frame.length);
        const now = performance.now();
        if (now - micStartedAt < 700) {
          micNoiseFrames += 1;
          micNoiseFloor += (rms - micNoiseFloor) / micNoiseFrames;
          return;
        }
        const voiceThreshold = Math.max(.012, micNoiseFloor * 2.8);
        if (rms > voiceThreshold) {
          micSpeechDetected = true;
          micLastVoiceAt = now;
        } else if (continuousVoice && micSpeechDetected && now - micLastVoiceAt > 1300 && micSamples / micSampleRate > 1.2) {
          micAutoStopping = true;
          finishRecording();
          return;
        } else if (continuousVoice && !micSpeechDetected && now - micStartedAt > 15000) {
          micAutoStopping = true;
          continuousVoice = false;
          updateContinuousVoiceButton();
          stopMicCapture();
          showToast("無音が続いたため連続会話を停止しました", "warn");
          return;
        }
      }
      // 上限に達したら、そこまでの録音を捨てずに自動で確定する。
      if (micSamples >= maxSamples) {
        micAutoStopping = true;
        showToast("録音が上限に達したので自動で確定しました", "warn");
        finishRecording();
      }
    };
    source.connect(micNode);
    micNode.connect(micCtx.destination);
    micButton.classList.add("recording");
    petApp.classList.add("is-listening");
    setAnimation("listening", true);
    status.textContent = "🎙 録音中…（マイクをもう一度クリックで停止）";
    // 逐次文字起こし（リアルタイム風プレビュー）を開始する。
    const session = ++micSession;
    micInterimBase = input.value.trim();
    micInterimTexts = [];
    micInterimIndex = 0;
    micInterimTimer = setInterval(() => transcribeInterim(session), 3000);
    markActivity();
  } catch (error) {
    logApp("error", "録音を開始できません", errorDetail(error));
    showToast("録音を開始できませんでした", "error");
    stopMicCapture();
  } finally {
    if (request === micStartRequest) {
      micStarting = false;
      if (micButton) micButton.disabled = false;
    }
  }
}

function updateContinuousVoiceButton() {
  if (!voiceLoopButton) return;
  voiceLoopButton.setAttribute("aria-pressed", String(continuousVoice));
  voiceLoopButton.title = continuousVoice ? "連続会話を止める" : "返答後も自動で聞き続ける";
}

// 待てないときは間隔を伸ばして待ち直す。長い応答のあいだ 450ms で回り続けると、
// 何もしていない時間にタイマーだけが動く（常駐アプリでは避けたい）。
const CONTINUOUS_VOICE_MIN_MS = 450;
const CONTINUOUS_VOICE_MAX_MS = 5000;
function scheduleContinuousListening(delay = CONTINUOUS_VOICE_MIN_MS) {
  if (continuousVoiceTimer) { clearTimeout(continuousVoiceTimer); continuousVoiceTimer = null; }
  if (!continuousVoice) return;
  continuousVoiceTimer = setTimeout(async () => {
    continuousVoiceTimer = null;
    if (!continuousVoice) return;
    if (busy || currentVoice || micStream || !onboarding?.hidden) {
      scheduleContinuousListening(Math.min(delay * 2, CONTINUOUS_VOICE_MAX_MS));
      return;
    }
    if (composer.hidden) openComposer();
    await startRecording();
  }, delay);
}

voiceLoopButton?.addEventListener("click", () => {
  continuousVoice = !continuousVoice;
  updateContinuousVoiceButton();
  if (!continuousVoice) {
    if (continuousVoiceTimer) { clearTimeout(continuousVoiceTimer); continuousVoiceTimer = null; }
    if (micStream) stopMicCapture();
    setAnimation("idle", true);
    showToast("連続会話を終了しました", "success");
    return;
  }
  showToast("この起動中だけ、話し終わると自動送信します", "success");
  scheduleContinuousListening();
});
updateContinuousVoiceButton();

document.addEventListener("visibilitychange", () => {
  if (document.hidden) {
    animationLoop?.stop();
    gazeFrameScheduler?.cancel();
    setMeshRigVisible(false);
  } else {
    lastFrameAt = performance.now();
    springLastAt = 0;
    wakeAnimationLoop();
    if (meshRigController) setAnimation(animationName, true);
  }
  if (!document.hidden || !continuousVoice) return;
  continuousVoice = false;
  updateContinuousVoiceButton();
  if (continuousVoiceTimer) { clearTimeout(continuousVoiceTimer); continuousVoiceTimer = null; }
  if (micStream) stopMicCapture();
  cancelPendingVoiceSubmit("画面が非表示になったため音声送信を取り消しました");
});

let pushToTalkTimer = null;
let pushToTalkActive = false;
let suppressMicClick = false;
micButton?.addEventListener("pointerdown", event => {
  if (event.button !== 0 || micStream || micStarting) return;
  pushToTalkTimer = setTimeout(async () => {
    pushToTalkTimer = null;
    pushToTalkActive = true;
    suppressMicClick = true;
    await startRecording();
    if (micStream) status.textContent = "🎙 押している間だけ録音中…";
  }, 320);
});
micButton?.addEventListener("pointerup", async () => {
  if (pushToTalkTimer) { clearTimeout(pushToTalkTimer); pushToTalkTimer = null; }
  if (!pushToTalkActive) return;
  pushToTalkActive = false;
  if (micStream) await finishRecording();
});
micButton?.addEventListener("pointercancel", () => {
  if (pushToTalkTimer) { clearTimeout(pushToTalkTimer); pushToTalkTimer = null; }
  if (pushToTalkActive) { pushToTalkActive = false; stopMicCapture(); }
});
micButton?.addEventListener("click", async () => {
  if (suppressMicClick) { suppressMicClick = false; return; }
  if (micStream) { await finishRecording(); return; }
  await startRecording();
});

// ── 再生成: 直前の質問をもう一度送って別の答えをもらう ──────────────────
document.getElementById("regenReply")?.addEventListener("click", () => {
  if (busy || !lastUserText) return;
  setInputValue(lastUserText);
  if (composer.hidden) openComposer();
  composer.requestSubmit();
});

// ── 会話履歴の Markdown 書き出し ──────────────────────────────────────
document.getElementById("exportHistory")?.addEventListener("click", async event => {
  if (!conversationHistory.length) { showToast("書き出す会話がありません", "warn"); return; }
  const exportButton = event.currentTarget;
  exportButton.disabled = true;
  const name = characterManifest?.name || "キャラクター";
  const lines = [`# PicoAgent 会話履歴（${name}） ${new Date().toLocaleString("ja-JP")}`, ""];
  for (const item of conversationHistory) {
    const who = item.role === "user" ? "あなた" : name;
    const at = item.at ? ` (${new Date(item.at).toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" })})` : "";
    lines.push(`## ${who}${at}`, "", item.content, "");
  }
  try {
    const result = await window.picoagent.exportHistory(lines.join("\n"));
    showToast("書き出しました", "success");
    pushNotice({ kind: "export", ok: true, title: "会話履歴を書き出しました", body: String(result?.path || ""), at: new Date().toISOString() });
  } catch (error) {
    showToast(preview(error?.message || error || "書き出しに失敗しました", 60), "error");
  } finally {
    exportButton.disabled = false;
  }
});

// ── 立ち絵のドラッグ移動 ────────────────────────────────────────────
// キャラの構図は設定画面のプレビューで調整する。通常画面ではキャラのどこを
// 掴んでもウィンドウを移動できるよう、OS標準のドラッグへ渡す。
let characterOffset = { x: 0, y: 0 };
let characterZoom = 1.55;
let characterFraming = {};
// 保存済みの値。同じ位置での重複保存を避けるために持つ。

/** 枠がウィンドウから完全に出ていかないよう移動量を丸める。 */
function clampCharacterOffset(x, y) {
  const keep = 48; // 掴み直せるだけの見えしろ（px）
  const width = character.offsetWidth || 1;
  const height = character.offsetHeight || 1;
  const left = character.offsetLeft;
  const top = character.offsetTop;
  const viewWidth = document.documentElement.clientWidth;
  const viewHeight = document.documentElement.clientHeight;
  return {
    x: clampNumber(x, keep - left - width, viewWidth - keep - left),
    y: clampNumber(y, keep - top - height, viewHeight - keep - top),
  };
}

function applyCharacterOffset(x, y, { clamp = true } = {}) {
  const next = clamp ? clampCharacterOffset(Number(x) || 0, Number(y) || 0) : { x: Number(x) || 0, y: Number(y) || 0 };
  characterOffset = next;
  character.style.setProperty("--char-x", `${next.x}px`);
  character.style.setProperty("--char-y", `${next.y}px`);
}

function applyCharacterZoom(zoom) {
  const value = Number(zoom);
  // 1 未満は枠より小さくなって余白だけが増えるので、下限を 1 にする。
  characterZoom = Number.isFinite(value) && value >= 1 ? Math.min(value, 4) : 1.55;
  // Live2D は character.json の scale / offsetY がすでにモデルごとの上半身表示へ調整済み。
  // PNG向けの既定 1.55 倍をさらに重ねると、ひよりは顔だけに切り取られてしまう。
  // Live2D では 1.55 を基準倍率（1倍）として、ユーザーが変えた差分だけを反映する。
  const viewZoom = live2dActive ? characterZoom / 1.55 : characterZoom;
  character.style.setProperty("--char-zoom", String(viewZoom));
  character.style.setProperty("--char-render-size", `${viewZoom * 100}%`);
}

const FRAMING_PRESETS = {
  bust: { zoom: 2.05, x: 0, y: 38 },
  upper: { zoom: 1.55, x: 0, y: 18 },
  full: { zoom: 1, x: 0, y: 0 },
};

function applyCharacterFraming() {
  const characterId = characterManifest?.id;
  const saved = characterId && characterFraming?.[characterId]?.[currentLayout];
  // 旧版の全キャラ共通offsetを別キャラへ引き継がない。個別構図がないキャラは
  // 画面中央基準へ戻さないと、前に調整したキャラの座標で画面外へ消える。
  if (!saved) {
    // 低頭身2Dは以前の上半身位置を維持し、長身・実写は胸上を標準にする。
    // Live2Dはモデル側のscale/offsetYで胸上へ調整済みなので追加拡大しない。
    // manifestの既定構図は原画固有の余白だけを吸収し、利用者の保存値がある場合は使わない。
    const isCompact2d = character.dataset.renderKind === "2d"
      && character.dataset.bodyProportion === "compact";
    const isLive2d = character.dataset.renderKind === "live2d";
    const usesStream2dCrop = character.dataset.renderKind === "2d" && currentLayout === "stream";
    const manifestFrame = usesStream2dCrop
      ? characterManifest?.streamFraming
      : characterManifest?.defaultFraming;
    const fallback = manifestFrame && Number.isFinite(Number(manifestFrame.zoom))
      ? {
          zoom: Number(manifestFrame.zoom),
          x: Number.isFinite(Number(manifestFrame.x)) ? Number(manifestFrame.x) : 0,
          y: Number.isFinite(Number(manifestFrame.y)) ? Number(manifestFrame.y) : 0,
        }
      : isCompact2d || usesStream2dCrop || isLive2d ? FRAMING_PRESETS.upper : FRAMING_PRESETS.bust;
    applyCharacterZoom(fallback.zoom);
    applyCharacterOffset(fallback.x, fallback.y, { clamp: false });
    return;
  }
  const preset = FRAMING_PRESETS[saved.preset] || null;
  const frame = preset || saved;
  applyCharacterZoom(saved.zoom ?? frame.zoom ?? FRAMING_PRESETS.bust.zoom);
  applyCharacterOffset(saved.x ?? frame.x ?? 0, saved.y ?? frame.y ?? 0, { clamp: false });
}

function applyStreamAppearance(background, imagePath, display, displayImagePath, captionColor = "#5a3b4b") {
  petApp.dataset.streamBackground = ["pastel", "gaming", "simple", "custom"].includes(background) ? background : "pastel";
  petApp.dataset.streamDisplay = ["dashboard", "game", "clock", "image", "none"].includes(display) ? display : "dashboard";
  const image = imagePath ? `url("${window.picoagent.assetUrl(imagePath)}")` : "none";
  petApp.style.setProperty("--stream-custom-background", image);
  const displayImage = displayImagePath ? `url("${window.picoagent.assetUrl(displayImagePath)}")` : "none";
  document.getElementById("streamDisplay")?.style.setProperty("--stream-display-image", displayImage);
  const safeCaptionColor = /^#[0-9a-f]{6}$/i.test(String(captionColor)) ? String(captionColor) : "#5a3b4b";
  petApp.style.setProperty("--stream-caption-color", safeCaptionColor);
}

character.addEventListener("pointerdown", event => {
  if (event.button !== 0) return;
  if (petApp.classList.contains("is-layout-editing")) return;
  if (event.target instanceof Element && event.target.closest("button")) return;
  event.preventDefault();
  markActivity();
  window.picoagent.startWindowDrag().catch(error => {
    logApp("warn", "キャラクターからウィンドウを移動できませんでした", errorDetail(error));
    showToast("画面を移動できませんでした。上部のPicoAgent部分からも移動できます", "error");
  });
});

// 最大化・復元で構図の保存座標を変えない。サイズ変更のたびに現在の画面へ丸めると、
// 復元後も丸めた座標が残り、設定で決めたキャラ位置がずれてしまう。
window.addEventListener("resize", () => {
  applyCharacterOffset(characterOffset.x, characterOffset.y, { clamp: false });
  syncLive2DResolution();
  syncPngNaturalResolution();
});

// ── なでなで: キャラの上でマウスを素早く左右に往復させると照れる ─────────
let petStrokes = [];
let petCooldownUntil = 0;
character.addEventListener("pointermove", event => {
  if (reduceMotion || (event.pointerType === "mouse" && event.buttons)) return; // マウスのウィンドウ移動中だけ撫で判定しない
  const now = performance.now();
  petStrokes.push({ x: event.clientX, t: now });
  petStrokes = petStrokes.filter(s => now - s.t < 1200);
  if (now < petCooldownUntil || petStrokes.length < 6) return;
  let flips = 0;
  let travel = 0;
  let prevDx = 0;
  for (let i = 1; i < petStrokes.length; i += 1) {
    const dx = petStrokes[i].x - petStrokes[i - 1].x;
    travel += Math.abs(dx);
    if (dx !== 0 && prevDx !== 0 && Math.sign(dx) !== Math.sign(prevDx)) flips += 1;
    if (dx !== 0) prevDx = dx;
  }
  if (flips >= 4 && travel > 150) {
    petCooldownUntil = now + 6000;
    petStrokes = [];
    character.classList.remove("boop");
    void character.offsetWidth;
    character.classList.add("boop");
    for (let i = 0; i < 3; i += 1) {
      const heart = document.createElement("span");
      heart.className = "pet-heart";
      heart.textContent = "♥";
      heart.style.left = `${30 + Math.random() * 40}%`;
      heart.style.animationDelay = `${i * 130}ms`;
      characterView.append(heart);
      heart.addEventListener("animationend", () => heart.remove());
    }
    status.textContent = "えへへ…";
    setTimeout(() => { if (status.textContent === "えへへ…") status.textContent = ""; }, 2000);
    markActivity();
    setAnimation("shy", true);
    recordRelationship("petting");
  }
});

// ── 居眠り: しばらく操作がないと寝て、操作すると起きる ────────────────────
let napMinutes = 15; // pet_init の設定値で上書き（0 で無効）
let lastActivityAt = Date.now();
let napping = false;

function markActivity() {
  lastActivityAt = Date.now();
  if (napping) wakeUp();
}

function fallAsleep() {
  // 全画面で履歴を読んでいる間（マウス操作なしでも閲覧中）は寝ない。
  if (napping || busy || currentVoice || !composer.hidden || live2dActive
    || bubble.classList.contains("fullscreen")) return;
  const sleeping = characterManifest?.boot?.sleeping;
  if (!sleeping) return;
  napping = true;
  setMeshRigVisible(false);
  stopRig();
  character.dataset.motion = "none";
  character.classList.add("napping");
  petSprite.src = versionedAssetUrl(`${characterRoot}/${sleeping}`, characterAssetVersion);
  status.textContent = "すやすや……";
}

function wakeUp() {
  if (!napping) return;
  napping = false;
  character.classList.remove("napping");
  if (status.textContent === "すやすや……") status.textContent = "";
  setAnimation("idle", true);
}

// グローバルホットキーで呼び出されたとき。ウィンドウの表示・前面化はバックエンドが済ませて
// いるので、ここでは居眠りを解除して入力欄を開き、すぐ打てる状態にする。
window.picoagent.on("pet:hotkey", (action) => {
  if (action !== "summon") return;
  markActivity();
  if (composer.hidden) openComposer();
  // 処理中は openComposer が開かないので、開いているときだけ入力欄へ移す。
  if (!composer.hidden) input.focus();
});

const napTimer = setInterval(() => {
  if (napMinutes > 0 && !napping && Date.now() - lastActivityAt > napMinutes * 60 * 1000) fallAsleep();
}, 30 * 1000);
["pointerdown", "keydown"].forEach(type => document.addEventListener(type, markActivity));

// 停止ボタン: バックエンドへ中断を要求する。エージェントは次のステップ境界で止まる。
cancelSendButton?.addEventListener("click", async () => {
  if (!busy) return;
  await requestAgentCancellation(cancelSendButton);
});
input.addEventListener("input", () => {
  autosizeInput();
  updateComposerState();
  promptHistoryIndex = null;
  promptDraft = input.value;
  const len = input.value.length;
  if (!busy) status.textContent = len > 100 ? `${len}文字` : "";
});
// 日本語入力（IME）の変換中かどうか。変換中のキーはアプリの操作へ流さない。
function isImeComposing(event) {
  return event.isComposing === true || event.keyCode === 229;
}
input.addEventListener("keydown", event => {
  // 日本語入力（IME）の変換中はキーをアプリの操作に使わない。
  // 変換確定の Enter で書きかけが送信され、候補選択の ↑↓ で入力欄が履歴に
  // 置き換わり、変換取消の Escape で入力欄ごと閉じてしまう。
  // `keyCode === 229` は isComposing を出さない環境向けの保険。
  if (isImeComposing(event)) return;
  if (event.key === "Escape") {
    if (cancelPendingVoiceSubmit()) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
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
let historySearchTimer = 0;
historySearch?.addEventListener("input", () => {
  historyQuery = historySearch.value;
  // 毎キーで全件を Markdown 再構築しないよう 120ms デバウンスする。
  clearTimeout(historySearchTimer);
  historySearchTimer = setTimeout(renderHistory, 120);
});
document.addEventListener("keydown", event => {
  if (event.key !== "Escape") return;
  if (isImeComposing(event)) return; // 変換の取消を画面の Escape として扱わない
  if (cancelPendingVoiceSubmit()) return;
  if (relationshipCard && !relationshipCard.hidden) {
    relationshipCard.hidden = true;
    relationshipButton?.setAttribute("aria-expanded", "false");
    relationshipButton?.focus();
    return;
  }
  if (!composer.hidden) {
    closeComposer();
    return;
  }
  exitBubbleFullscreen();
});
// 右クリックメニュー「表示形式」からの切り替え。ウィンドウ寸法はバックエンドが当て、
// 中の配置はここで data-layout を書き替えて CSS に任せる。
window.picoagent.on("pet:layout", applyLayout);
// 右クリックメニュー「立ち絵の位置を戻す」。動かしすぎたときの戻し先。
window.picoagent.on("pet:character-reset", (reset) => {
  const characterId = reset?.characterId || characterManifest?.id;
  const layout = reset?.layout || currentLayout;
  if (characterId && characterFraming?.[characterId]) {
    delete characterFraming[characterId][layout];
    if (!Object.keys(characterFraming[characterId]).length) delete characterFraming[characterId];
  }
  applyCharacterOffset(0, 0, { clamp: false });
  applyCharacterFraming();
});
window.picoagent.on("pet:model", model => {
  status.textContent = model;
  setTimeout(() => { if (status.textContent === model) status.textContent = ""; }, 1800);
});
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
  if (!localEngineRequired) return;
  localEnginePhase = s.phase;
  localEngineReady = s.phase === "ready";
  const booting = petApp.classList.contains("is-booting");
  status.classList.toggle("engine-pending", ["idle", "download", "verify", "starting"].includes(s.phase));
  status.classList.toggle("engine-error", s.phase === "error");
  if (s.phase === "download") {
    // 初回だけ数GBを取る。割合しか出さないと、待ち時間の見当が付かず固まって見える。
    const gb = bytes => (Number(bytes) / (1024 ** 3)).toFixed(1);
    const size = Number(s.totalBytes) > 0 ? `（${gb(s.receivedBytes)} / ${gb(s.totalBytes)}GB）` : "";
    setStatus(`ローカルモデルを取得中… ${s.percent ?? 0}%${size} ※初回だけ`);
    online.dataset.up = false;
  } else if (s.phase === "verify") {
    // 数GBのモデル照合は分単位かかる。無表示だと起動が固まったように見える。
    setStatus("ローカルモデルを確認中… ※数分かかることがあります");
    online.dataset.up = false;
  } else if (s.phase === "starting") {
    setStatus("ローカルAIを起動中…");
    online.dataset.up = false;
  } else if (s.phase === "ready") {
    online.dataset.up = true;
    status.classList.remove("engine-pending", "engine-error");
    if (petApp.classList.contains("is-booting")) {
      status.textContent = "";
      if (bootStatus) bootStatus.textContent = `${bootCharacterName}を起こしているよ…`;
    } else {
      showStartupReadyMessage();
    }
  } else if (s.phase === "error") {
    setStatus(`準備に失敗: ${s.message || "不明なエラー"}`);
    online.dataset.up = false;
  } else if (s.phase === "idle") {
    setStatus("ローカルAIの起動を待っています…");
    online.dataset.up = false;
  }
  updateComposerState();
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
  if (voiceVolume <= 0.001) {
    rigMouthLevel = 0;
    rigMouthOpen = false;
    live2dMouth = 0;
    rigRefresh();
    if (animationName === "talking") setAnimation("idle", true);
    return;
  }
  if (shouldLipSync(currentVoice)) {
    setAnimation("talking", true);
    if (!rigAnalyser && !rigMouthTimer) attachLipSync(currentVoice);
  }
}
function applyTheme(value) {
  const theme = value === "light" || value === "dark" ? value : "system";
  if (theme === "system") {
    document.documentElement.removeAttribute("data-theme");
  } else {
    document.documentElement.dataset.theme = theme;
  }
}
// ── 表示形式（画面構成） ──────────────────────────────────────────
// 配置そのものは CSS（#petApp[data-layout=…]）が決める。JS の仕事は
// 「どれが選ばれているか」を DOM へ書くことと、ペット表示専用の処理を止めること。
let currentLayout = "pet";
// ペット表示だけ、フキダシの位置とサイズをユーザーが自由に決められる（つまみでドラッグ）。
// 他の形式はフキダシの置き場所が形式の一部なので、JS からは動かさない。
function layoutUsesFreeBubble() { return currentLayout === "pet"; }
// 会話履歴を常に出しておく形式。全画面にしなくても過去の発言が読める。
function layoutShowsHistory() {
  return currentLayout === "chat"
    || currentLayout === "workspace" || currentLayout === "stream";
}
// ペット表示へ戻ったときに復元するため、保存済みのフキダシ寸法・位置を控えておく。
let savedBubbleLayout = { width: "", height: "", left: "", top: "" };

const widgetClock = document.getElementById("widgetClock");
const streamInfo = document.getElementById("streamInfo");
let widgetClockTimer = null;
function renderWidgetClock() {
  const now = new Date();
  const hh = String(now.getHours()).padStart(2, "0");
  const mm = String(now.getMinutes()).padStart(2, "0");
  const weekday = "日月火水木金土"[now.getDay()];
  if (widgetClock) {
    widgetClock.querySelector("strong").textContent = `${hh}:${mm}`;
    widgetClock.querySelector("span").textContent = `${now.getMonth() + 1}月${now.getDate()}日（${weekday}）`;
  }
  if (streamInfo) {
    streamInfo.querySelector("strong").textContent = `${hh}:${mm}`;
    streamInfo.querySelector(":scope > span:last-child").textContent = `${now.getFullYear()}.${String(now.getMonth() + 1).padStart(2, "0")}.${String(now.getDate()).padStart(2, "0")} ${weekday}`;
  }
}
function syncWidgetClock() {
  const on = currentLayout === "widget" || currentLayout === "stream";
  if (on && !widgetClockTimer) {
    renderWidgetClock();
    // 分単位の表示なので 15 秒ごとで足りる（常駐アプリなので秒間更新はしない）。
    widgetClockTimer = setInterval(renderWidgetClock, 15000);
  } else if (!on && widgetClockTimer) {
    clearInterval(widgetClockTimer);
    widgetClockTimer = null;
  }
}

function applyLayout(id) {
  const requested = typeof id === "string" && id ? id : "pet";
  const next = requested === "talk" ? "chat" : requested;
  currentLayout = next;
  petApp.dataset.layout = next;
  if (layoutUsesFreeBubble()) {
    applyBubbleLayout(savedBubbleLayout.width, savedBubbleLayout.height, savedBubbleLayout.left, savedBubbleLayout.top);
  } else {
    // CSS に任せるので、ペット表示で当てたインライン指定を残さない。
    for (const prop of ["width", "height", "left", "top"]) bubble.style.removeProperty(prop);
    resetFullscreenBubbleLayoutVars();
    // 形式ごとの配置と全画面フキダシは両立しないので、切り替え時に全画面は解除する。
    if (bubble.classList.contains("fullscreen")) exitBubbleFullscreen();
  }
  syncWidgetClock();
  applyCharacterOffset(characterOffset.x, characterOffset.y);
  applyCharacterFraming();
  // 履歴を常設する形式では、切り替えた時点の履歴をその場で描き直す。
  renderHistory();
  refreshBubblePlacement();
  gazeFrameScheduler?.cancel();
  resetLook();
  if (!reduceMotion && gazeTrackingEnabled) scheduleGaze();
  if (meshRigController) setAnimation(animationName, true);
  wakeAnimationLoop();
}

function applyBubbleLayout(width, height, left, top) {
  // ペット表示以外はフキダシを CSS が置くので、保存値を当てない
  // （当てるとチャットの縦長パネルがペット時代の寸法に化ける）。
  if (!layoutUsesFreeBubble()) return;
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
function syncPngNaturalResolution() {
  if (!petSprite.complete || petSprite.naturalWidth < 1 || petSprite.naturalHeight < 1) return;
  const dpr = Math.max(1, window.devicePixelRatio || 1);
  // 1つの原画画素を複数の表示画素へ引き伸ばさない。配信構図の260%指定は
  // 原画に余裕がある範囲だけで効き、高DPIほどCSS上限を小さくする。
  character.style.setProperty("--char-natural-css-width", `${petSprite.naturalWidth / dpr}px`);
  character.style.setProperty("--char-natural-css-height", `${petSprite.naturalHeight / dpr}px`);
}
petSprite.addEventListener("load", () => {
  syncPngNaturalResolution();
  const dpr = Math.max(1, window.devicePixelRatio || 1);
  refreshBubblePlacement();
  syncSpringMotionFrame();
  if (character.dataset.renderKind === "2d") {
    requestAnimationFrame(() => {
      const view = petSprite.getBoundingClientRect();
      logApp("info", "2D表示寸法", JSON.stringify({
        natural: [petSprite.naturalWidth, petSprite.naturalHeight],
        rendered: view ? [Math.round(view.width), Math.round(view.height)] : null,
        devicePixelRatio: dpr,
        sourcePixelsPerDisplayPixel: view ? [
          petSprite.naturalWidth / Math.max(1, view.width * dpr),
          petSprite.naturalHeight / Math.max(1, view.height * dpr),
        ] : null,
        transform: getComputedStyle(petSprite.parentElement).transform,
      }));
    });
  }
});
window.picoagent.on("pet:ui", (ui) => {
  if (!ui) return;
  if (ui.theme != null) applyTheme(ui.theme);
  if (ui.fontPx != null) applyFontPx(ui.fontPx);
  if (ui.voiceVolume != null) applyVoiceVolume(ui.voiceVolume);
  if (ui.relationshipMemoryEnabled != null) {
    relationshipMemoryEnabled = ui.relationshipMemoryEnabled !== false;
    if (relationshipButton) relationshipButton.title = relationshipMemoryEnabled
      ? "いっしょの記録を見る"
      : "いっしょの記録は設定で停止中";
  }
  if (ui.characterZoom != null) applyCharacterZoom(ui.characterZoom);
  if (ui.gazeTrackingEnabled != null) window.applyGazeTrackingSetting?.(ui.gazeTrackingEnabled);
  if (ui.characterFraming != null) characterFraming = ui.characterFraming || {};
  if (ui.streamBackground != null) applyStreamAppearance(ui.streamBackground, ui.streamBackgroundImage, ui.streamDisplay, ui.streamDisplayImage, ui.streamCaptionColor);
  if (ui.ttsEnabled != null) {
    ttsEnabledCache = ui.ttsEnabled !== false;
    if (!ttsEnabledCache) stopVoice(true);
  }
  if (ui.characterOffsetX != null) applyCharacterOffset(ui.characterOffsetX, ui.characterOffsetY);
  if (ui.layout != null) applyLayout(ui.layout);
  savedBubbleLayout = { width: ui.bubbleWidth, height: ui.bubbleHeight, left: ui.bubbleLeft, top: ui.bubbleTop };
  applyBubbleLayout(ui.bubbleWidth, ui.bubbleHeight, ui.bubbleLeft, ui.bubbleTop);
  if (ui.characterId) loadCharacter(ui.characterId, ui.characterOverrides || {}, {
    live2dSdkOverrides: ui.live2dSdkOverrides || {},
  }).catch(error => {
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
  noticeEntries.length = 0;
  persistNotices();
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

// 選択中キャラの AivisHub 音声モデルを準備する。読み上げが有効な場合は、通常画面へ
// 移る前にモデル導入・エンジン起動まで終えて、最初の発話から待たずに話せるようにする。
async function prepareSelectedVoiceModel() {
  let statusInfo;
  try { statusInfo = await window.picoagent.aivisStatus(); }
  catch (error) {
    // 読み上げが有効かどうかを確認できない状態で「音声を準備できません」とは出さない
    // （ミュート中の利用者へ無関係な失敗を見せない）。理由はログへ残す。
    logApp("warn", "音声モデル準備前の Aivis 状態取得に失敗しました", errorDetail(error));
    return null;
  }
  if (!statusInfo?.enabled) return null;
  setStatus("声を準備中…");
  try {
    await window.picoagent.prepareVoiceModel();
    return true;
  } catch (error) {
    logApp("error", "音声モデルの準備に失敗しました", errorDetail(error));
    return false;
  }
}

// 同梱エンジン(local)のモデルDL・検証・起動を監視する。
// localのモデル取得・検証・起動を通常画面の状態表示へ反映する。
async function waitForEngineReady() {
  while (true) {
    let s;
    try { s = await window.picoagent.engineStatus(); }
    catch (error) {
      logApp("warn", "エンジン状態取得に失敗しました", errorDetail(error));
      const failed = { phase: "error", message: "ローカルAIの準備状態を確認できませんでした" };
      applyEngineStatus(failed);
      return failed;
    }
    applyEngineStatus(s); // download/verify/starting はローディング画面に進捗表示。
    if ((!s || s.phase === "idle") && petApp.classList.contains("is-booting")) {
      setStatus("ローカルAIを確認中…");
    }
    if (s && (s.phase === "ready" || s.phase === "error")) return s;
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
  if (napping) wakeUp(); // 寝たまま話さない（無操作タイマーは進めない: 雑談で永遠に寝られなくなるため）
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
  // 入力中・応答中・読み上げ中・全画面表示中・居眠り中は割り込まず、次の機会に回す。
  if (busy || !composer.hidden || currentVoice || bubble.classList.contains("fullscreen") || napping) {
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
    notifyBackgroundFailure(
      "daily-events",
      "リマインダーを確認できないため通知が出ません。設定の「リマインダー」を確認してください",
      error,
    );
    return;
  }
  clearBackgroundFailure("daily-events");
  // 日時を読めない予定は永久に発火しない。バックエンドが1件につき1度だけ返すので、
  // 届いたぶんはそのまま知らせる（黙って飛ばすと、鳴らない理由が利用者に分からない）。
  const broken = Array.isArray(res?.broken) ? res.broken : [];
  for (const item of broken) {
    const label = String(item?.text || item?.label || "").trim() || "登録した通知";
    logApp("warn", "通知の日時を解析できません", `${label} / ${item?.due_at ?? ""}`);
    showToast(`「${label}」は日時を読めないため鳴りません。登録し直してください`, "error");
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
  if (alarmCheckInFlight) return;
  alarmCheckInFlight = true;
  try {
  const now = new Date();
  if (activeAlarmSnooze && Date.now() >= activeAlarmSnooze.dueAt) {
    const snoozeState = activeAlarmSnooze;
    const alarm = activeAlarmSnooze.alarm;
    // 遅い音声合成を待つ間に次のポーリングが同じスヌーズを発火しないよう、先に期限を進める。
    activeAlarmSnooze.dueAt = Date.now() + alarmSnoozeMinutes(alarm) * 60 * 1000;
    await fireAlarm(alarm, snoozeState);
    return;
  }
  let res;
  try { res = await window.picoagent.alarms(); }
  catch (error) {
    notifyBackgroundFailure(
      "alarms",
      "目覚ましの一覧を読めないため鳴りません。設定の「目覚まし時計」を確認してください",
      error,
    );
    return;
  }
  clearBackgroundFailure("alarms");
  const alarms = res?.alarms;
  if (!Array.isArray(alarms) || !alarms.length) return;
  // 分ちょうどだけを見ると、スリープ復帰・起動直後・直前の発火が長引いた場合に
  // その日の目覚ましが黙って鳴らない。直近5分以内に来た予定まで拾う。
  const dueItems = window.PicoAlarmRuntime.dueAlarms(alarms, now);
  for (const due of dueItems) {
    const key = `${due.alarm.id}@${due.scheduledKey}`;
    // 1ティックにつき1件だけ鳴らす（発火記録で二重には鳴らない）。
    if (!window.PicoAlarmRuntime.markFired(firedAlarms, key)) continue;
    await fireAlarm(due.alarm, null, due);
    break;
  }
  } finally {
    alarmCheckInFlight = false;
  }
}
// 目覚ましの起こし文の生成（モデル呼び出し）と音声合成は、どちらも遅れることがある
// （ローカルAIの起動直後、Aivisの初回合成はHTTPタイムアウトが180秒）。目覚ましは
// 「鳴らなかった」がいちばん困るので、待つ上限を決めて文字を先に出す。
const ALARM_WAKE_TIMEOUT_MS = 20000;
const ALARM_SPEECH_TIMEOUT_MS = 20000;
const ALARM_FALLBACK_LINE = "おはよう…そろそろ起きる時間だよ。ゆっくりでいいからね。";
const ALARM_RACE_TIMEOUT = Symbol("alarm-timeout");

// 上限まで待ち、間に合わなければ `timedOut` を返す（遅れて届いた結果は使わない）。
async function waitWithLimit(promise, timeoutMs) {
  let timer = null;
  const limit = new Promise(resolve => {
    timer = setTimeout(() => resolve(ALARM_RACE_TIMEOUT), timeoutMs);
  });
  try {
    const result = await Promise.race([promise, limit]);
    return result === ALARM_RACE_TIMEOUT ? { timedOut: true, value: null } : { timedOut: false, value: result };
  } finally {
    if (timer) clearTimeout(timer);
  }
}

async function fireAlarm(a, snoozeState = null, due = null) {
  const wakeRequest = window.picoagent.alarmWake(a.id).catch(error => {
    logApp("warn", "目覚まし起こし文の生成に失敗しました", errorDetail(error));
    return null;
  });
  const wake = await waitWithLimit(wakeRequest, ALARM_WAKE_TIMEOUT_MS);
  if (wake.timedOut) {
    logApp("warn", "目覚ましの起こし文が時間内に届きませんでした", `${ALARM_WAKE_TIMEOUT_MS}ms`);
  }
  let line = (wake.value?.reply || "").trim() || ALARM_FALLBACK_LINE;
  // 遅れて鳴らす場合は予定時刻を添える（いまの時刻の目覚ましだと誤解させない）。
  if (due && due.lateMinutes >= 1) line = `（${due.scheduledTime}の目覚ましだよ）${line}`;
  // 声より先に文字を出す。合成が詰まっても目覚ましが黙って消えないようにする。
  renderAlarmReply(line);
  addHistory("assistant", line);
  // 再通知の起点も合成待ちで遅らせない。
  // 生成中に「おきたよ！」で解除された場合は、完了後にスヌーズを復活させない。
  const shouldArmSnooze = !snoozeState || activeAlarmSnooze === snoozeState;
  if (shouldArmSnooze) armAlarmSnooze(a);
  const armedSnoozeState = shouldArmSnooze ? activeAlarmSnooze : null;
  const speechRequest = prepareSpeech(line).catch(error => {
    logApp("warn", "目覚ましの音声合成に失敗しました", errorDetail(error));
    return null;
  });
  const speech = await waitWithLimit(speechRequest, ALARM_SPEECH_TIMEOUT_MS);
  if (speech.timedOut) {
    // 数分後に突然鳴るほうが困るため、遅れて届いた音声は鳴らさない。
    logApp("warn", "目覚ましの音声が時間内に用意できませんでした", `${ALARM_SPEECH_TIMEOUT_MS}ms`);
  }
  if (armedSnoozeState && !speech.timedOut && speech.value && activeAlarmSnooze === armedSnoozeState) {
    playPreparedSpeech(speech.value);
  }
  else if (animationName === "talking") setAnimation("idle");
}
function startAlarmPolling() {
  if (alarmTimer) clearInterval(alarmTimer);
  alarmTimer = setInterval(checkAlarms, ALARM_POLL_MS);
  checkAlarms();
}

let startupLayout = null;
let bootUiReleased = false;
// 生存通知の間隔はバックエンドが監視しきい値から決める。設定が届く前は、許可される
// 最短しきい値3秒にも間に合う1秒で始め、`pet_init` 後に計算済みの間隔へ張り替える。
const DEFAULT_BOOT_PROGRESS_INTERVAL_MS = 10000;
const INITIAL_BOOT_PROGRESS_INTERVAL_MS = 1000;
let bootProgressTimer = null;
function startBootProgressTicker(intervalMs = DEFAULT_BOOT_PROGRESS_INTERVAL_MS) {
  const interval = Number(intervalMs) > 0 ? Number(intervalMs) : DEFAULT_BOOT_PROGRESS_INTERVAL_MS;
  if (bootProgressTimer) clearInterval(bootProgressTimer);
  bootProgressTimer = setInterval(() => {
    window.picoagent.bootProgress().catch(error => {
      logApp("warn", "起動進捗を通知できませんでした", errorDetail(error));
    });
  }, interval);
}
startBootProgressTicker(INITIAL_BOOT_PROGRESS_INTERVAL_MS);
let bootBackendFinished = false;
let bootBackendFinishPromise = null;
let bootBackendRetryTimer = null;
let bootBackendRetryDelayMs = 1500;
const BOOT_BACKEND_RETRY_MAX_MS = 30000;

async function finishBootBackend() {
  if (bootBackendFinished) return;
  if (!bootBackendFinishPromise) {
    bootBackendFinishPromise = window.picoagent.finishBoot()
      .then(() => { bootBackendFinished = true; })
      .finally(() => { bootBackendFinishPromise = null; });
  }
  await bootBackendFinishPromise;
}

function scheduleBootBackendRetry() {
  if (bootBackendFinished || bootBackendRetryTimer) return;
  setStatus("位置・寸法の復旧中…");
  status.classList.add("engine-pending");
  bootBackendRetryTimer = setTimeout(async () => {
    bootBackendRetryTimer = null;
    try {
      await finishBootBackend();
      bootBackendRetryDelayMs = 1500;
      status.classList.remove("engine-pending");
      if (status.textContent === "位置・寸法の復旧中…") setStatus("");
    } catch (error) {
      logApp("warn", "起動後の位置・寸法を再試行できませんでした", errorDetail(error));
      bootBackendRetryDelayMs = Math.min(bootBackendRetryDelayMs * 2, BOOT_BACKEND_RETRY_MAX_MS);
      scheduleBootBackendRetry();
    }
  }, bootBackendRetryDelayMs);
}

async function releaseBootUi() {
  if (bootProgressTimer) clearInterval(bootProgressTimer);
  bootProgressTimer = null;
  if (!bootBackendFinished) {
    try { await finishBootBackend(); }
    catch (error) {
      logApp("warn", "起動完了を通知できませんでした", errorDetail(error));
      scheduleBootBackendRetry();
    }
  }
  bootUiReleased = true;
  if (startupLayout) {
    try { applyLayout(startupLayout); }
    catch (error) { logApp("warn", "保存中の表示形式へ戻せませんでした", errorDetail(error)); }
  }
  petApp.classList.remove("is-booting");
  if (meshRigController) setAnimation(animationName, true);
  stopBootLogTicker();
}

(async () => {
  startBootLogTicker();
  setStatus("設定と履歴を読み込み中…");
  const info = await window.picoagent.init();
  startupLayout = info.layout;
  localEngineRequired = (info.providerId || "local") === "local";
  localEngineReady = !localEngineRequired || info.up === true;
  localEnginePhase = localEngineReady ? "ready" : "idle";
  frontendInitialized = true;
  updateComposerState();
  if (betaBadge) betaBadge.hidden = info.betaBuild !== true;
  relationshipMemoryEnabled = info.relationshipMemoryEnabled !== false;
  renderRelationship({ relationship: info.relationship, bondLabel: info.bondLabel });
  if (relationshipButton) relationshipButton.title = relationshipMemoryEnabled
    ? "いっしょの記録を見る"
    : "いっしょの記録は設定で停止中";
  online.dataset.up = info.up;
  // 原本を読めなかった履歴は保存を止めている。黙っていると、保存されていると思ったまま
  // 会話を続けて、再起動でその分がまるごと消える。起動時に1度だけはっきり知らせる。
  if (info.historySaveBlocked === true) {
    notifyBackgroundFailure(
      "history-save-blocked",
      "会話履歴を読めなかったため、この会話は保存されません（元の履歴は残しています）",
      "history save blocked",
    );
  }
  if (info.theme != null) applyTheme(info.theme);
  if (info.fontPx != null) applyFontPx(info.fontPx);
  if (info.voiceVolume != null) applyVoiceVolume(info.voiceVolume);
  window.applyGazeTrackingSetting?.(info.gazeTrackingEnabled === true);
  applyCharacterZoom(info.characterZoom);
  startBootProgressTicker(info.bootProgressIntervalMs);
  // 配置編集の保存先は設定側。読み込み順に関係なく届くよう、値も残してから渡す。
  window.__picoLayoutPlacements = info.layoutPlacements || {};
  window.PicoLayoutEditor?.hydrate(window.__picoLayoutPlacements);
  characterFraming = info.characterFraming || {};
  applyStreamAppearance(info.streamBackground, info.streamBackgroundImage, info.streamDisplay, info.streamDisplayImage, info.streamCaptionColor);
  ttsEnabledCache = info.ttsEnabled !== false;
  applyCharacterOffset(info.characterOffsetX, info.characterOffsetY, { clamp: false });
  savedBubbleLayout = { width: info.bubbleWidth, height: info.bubbleHeight, left: info.bubbleLeft, top: info.bubbleTop };
  // 起動準備中だけは保存中の形式に関係なく、小型の配信レイアウトを見せる。
  // finishBoot 後にバックエンドの pet:layout イベントで保存形式へ戻る。
  if (bootUiReleased) applyLayout(startupLayout);
  else applyLayout("stream");
  applyBubbleLayout(info.bubbleWidth, info.bubbleHeight, info.bubbleLeft, info.bubbleTop);
  try {
    setStatus("起動画面のキャラクター画像を読み込み中…");
    // 候補が無い一覧（寝姿を持つキャラを外した等）でも起動画面は出す。既定キャラへ落とす。
    const catalog = await loadCharacterCatalog();
    const sleeping = chooseStartupSleepingCharacter(startupSleepingCandidates(catalog));
    await prepareBootCharacter(sleeping || catalog.default || info.characterId || "mugi");
  } catch (error) {
    logApp("warn", "ローディング画面のキャラクター読込に失敗しました", errorDetail(error));
  }
  // 通常キャラクターを読み込む。完了したら起動画面のプレビュー画像から実キャラへ
  // 切り替え、配信画面と同じ大きな立ち絵で見せる。推論・音声エンジンの準備は
  // このあと起動画面のまま待つ（送信できない通常画面を先に見せない）。
  // Live2D も基準寸法で初期化済みなので、現在の表示形式が小さくても崩れない。
  setStatus("選択中のキャラクターを読み込み中…");
  const requestedCharacterId = info.characterId || "mugi";
  const loadOptions = { speakGreeting: false, live2dSdkOverrides: info.live2dSdkOverrides || {} };
  const loadStartupCharacter = async () => {
    try {
      await loadCharacter(requestedCharacterId, info.characterOverrides || {}, loadOptions);
    } catch (error) {
      if (requestedCharacterId === "mugi") throw error;
      // フォールバック許可: 保存中の任意キャラが欠損・破損・応答停止でも、同梱の
      // 既定PNGキャラで画面を起動する。失敗理由はログへ残し、1回だけ切り替える。
      logApp("error", `${requestedCharacterId}を起動できないため既定キャラへ切り替えます`, errorDetail(error));
      setStatus("選択中のキャラクターを読み込めないので、むぎで起動するね");
      await loadCharacter("mugi", info.characterOverrides || {}, loadOptions);
    }
  };
  const characterLoadPromise = loadStartupCharacter().then(
    () => {
      petApp.classList.add("boot-character-ready");
      return { ok: true };
    },
    error => ({ ok: false, error }),
  );
  let startupGreeting = "";
  let startupPreparedSpeech = null;
  let startupVoiceError = "";
  try {
    const characterLoad = await characterLoadPromise;
    if (!characterLoad.ok) throw characterLoad.error;
    bootCharacterName = characterManifest?.name || bootCharacterName;
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
      startupGreeting = greeting;
    }
    if (localEngineRequired && info.onboardingDone) {
      // 送信できない通常画面を先に見せず、利用可能または明示的な失敗になるまで
      // 寝姿の起動画面で進捗を伝える。
      await waitForEngineReady();
    } else if (localEngineRequired) {
      // 初回セットアップ（追加機能の選択・Live2D追加）はローカルAIを使わない。
      // 数GBのモデル取得が終わるまで案内を出さないと、初回だけ何分も待たせることになる。
      // 監視は続け、準備できるまでは送信ボタン側が理由を出して止める。
      waitForEngineReady().catch(error => {
        logApp("error", "ローカルAIの起動確認に失敗しました", errorDetail(error));
      });
    }
    if (info.onboardingDone && startupGreeting) {
      const voiceReady = await prepareSelectedVoiceModel();
      if (voiceReady) startupPreparedSpeech = await prepareSpeech(startupGreeting);
      else if (voiceReady === false) {
        startupVoiceError = "音声を準備できませんでした。設定の診断を確認してください";
        setStatus(startupVoiceError);
        status.classList.add("engine-error");
      }
    }
  } catch (error) {
    logApp("error", "キャラクター読込に失敗しました", errorDetail(error));
    setStatus(`キャラクター読込に失敗: ${error.message}`);
  } finally {
    // キャラクター・履歴・必要なAIと音声が揃ってから通常画面へ戻す。
    if (!status.textContent) setStatus(`${bootCharacterName}を起こしているよ…`);
    await releaseBootUi();
  }
  if (startupPreparedSpeech) playPreparedSpeech(startupPreparedSpeech);
  if (!info.up && !status.textContent) status.textContent = "起動中…";
  // 取りこぼし対策で現在のエンジン状態も反映。
  try { applyEngineStatus(await window.picoagent.engineStatus()); }
  catch (error) { logApp("warn", "エンジン状態取得に失敗しました", errorDetail(error)); }
  // ローカルAIが ready になると engine-error を落とすため、音声準備の失敗はここで出し直す。
  // 配信形式はこの印が無いとステータス行ごと隠れ、失敗に気づけない。
  // ただし送信そのものを塞ぐローカルAIの失敗表示は上書きしない（そちらが先に要る）。
  if (startupVoiceError && localEnginePhase !== "error") {
    setStatus(startupVoiceError);
    status.classList.add("engine-error");
  }
  quickActionItems = parseQuickActions(info.quickActions);
  renderQuickActions();
  napMinutes = Number(info.napMinutes) > 0 ? Number(info.napMinutes) : 0;
  markActivity();
  applyChatterConfig(info); // アイドル雑談タイマーを開始。
  startDailyPolling(); // リマインダー・集中・見守りの通知を開始。
  startAlarmPolling(); // 目覚まし時計の時刻監視を開始。
  loadScheduleRunNotices(); // 前回までのスケジュール実行履歴を通知一覧へ復元。
  loadDurableJobNotices(); // 再起動後も残る永続ジョブ（転送・成果物生成）を復元。
  if (!info.onboardingDone && onboarding) {
    onboarding.hidden = false;
    stopVoice();
    showOnboardingStep("components");
  } else {
    scheduleContinuousListening();
  }
})().catch(error => {
  logApp("error", "起動初期化に失敗しました", errorDetail(error));
  setStatus(`起動初期化に失敗: ${error?.message || error}`);
  releaseBootUi().finally(() => wakeAnimationLoop());
});

function showOnboardingStep(step) {
  const components = step === "components";
  const character = step === "character";
  if (onboardingComponentStep) onboardingComponentStep.hidden = !components;
  if (onboardingCharacterStep) onboardingCharacterStep.hidden = !character;
  const title = character ? "onboardingCharacterTitle" : "onboardingComponentTitle";
  onboarding?.setAttribute("aria-labelledby", title);
  const focusTarget = character ? onboardingCharacterStep?.querySelector("button[data-onboarding-action]")
    : onboardingComponentStep?.querySelector("input:not([disabled]),button[data-onboarding-action]");
  focusTarget?.focus();
  if (components) refreshOnboardingComponents();
}

async function refreshOnboardingComponents() {
  if (!onboardingComponentStep) return;
  try {
    const statuses = await window.picoagent.settings.componentStatus();
    let attentionCount = 0;
    for (const item of statuses) {
      const row = onboardingComponentStep.querySelector(`[data-component-id="${item.id}"]`);
      if (!row) continue;
      row.classList.toggle("is-ready", item.ready);
      row.classList.toggle("is-unavailable", !item.available);
      row.classList.toggle("is-external", item.installMode === "external");
      const input = row.querySelector("input");
      if (input) input.disabled = item.ready || !item.available || item.installMode === "external";
      const badge = row.querySelector("em");
      const needsAttention = item.updateAvailable || item.newerThanTarget || item.versionUnknown;
      if (needsAttention) attentionCount += 1;
      if (badge) badge.textContent = item.updateAvailable
        ? "設定で更新"
        : (item.newerThanTarget || item.versionUnknown
          ? "設定で確認"
          : (item.ready ? "準備済み" : (item.installMode === "managed" ? "公式から取得" : "設定で案内")));
      let source = row.querySelector("a.onboarding-component-source");
      if (item.installMode === "external" && item.sources?.[0]?.url) {
        if (!source) {
          source = document.createElement("a");
          source.className = "onboarding-component-source";
          source.target = "_blank";
          source.rel = "noopener noreferrer";
          source.textContent = "公式ページ";
          row.append(source);
        }
        source.href = item.sources[0].url;
      } else {
        source?.remove();
      }
    }
    if (onboardingComponentStatus) {
      onboardingComponentStatus.textContent = attentionCount
        ? `導入済みの追加機能に${attentionCount}件の確認項目があります。初回設定の完了後、「設定 → 追加機能・更新」で確認できます。`
        : "";
    }
  } catch (error) {
    // 状態を取れなかったときは、初期表示の「確認中」を必ず終わらせる。
    // 放置すると5項目すべてが「確認中」のまま固まり、待てば変わるように見えてしまう。
    settleOnboardingComponentBadges(navigator.onLine ? "確認できません" : "オフライン");
    if (!navigator.onLine) {
      if (onboardingComponentStatus) onboardingComponentStatus.textContent = "オフラインのため取得確認を見送りました。基本機能はそのまま使えます。あとから「設定 → 追加機能・更新」で導入できます。";
      return;
    }
    logApp("warn", "追加機能の状態を確認できませんでした", errorDetail(error));
    if (onboardingComponentStatus) onboardingComponentStatus.textContent = `状態を確認できません: ${error?.message || error}。このまま進めて、あとから「設定 → 追加機能・更新」で導入できます。`;
  }
}

// 取得できなかった項目の見出しバッジを、待ち状態のままにしない。
function settleOnboardingComponentBadges(label) {
  if (!onboardingComponentStep) return;
  for (const row of onboardingComponentStep.querySelectorAll("[data-component-id]")) {
    const badge = row.querySelector("em");
    if (badge && badge.textContent.trim() === "確認中") badge.textContent = label;
    const input = row.querySelector("input");
    if (input) input.disabled = true; // 状態不明のまま取得を走らせない
  }
}

async function installSelectedOnboardingComponents() {
  const ids = [...onboardingComponentStep.querySelectorAll("input:checked:not([disabled])")].map(input => input.value);
  if (!ids.length) {
    showOnboardingStep("character");
    return;
  }
  onboardingComponentStep.querySelectorAll("input,button").forEach(item => { item.disabled = true; });
  if (onboardingComponentStatus) onboardingComponentStatus.textContent = "選んだ追加機能を準備しています…";
  try {
    // 初回画面で「音声読み上げ」を明示選択した場合だけ、導入成功後に読み上げも有効にする。
    // 設定画面からの単なる導入・更新では、既存のON/OFFを勝手に変えない。
    await window.picoagent.settings.installComponents(ids, ids.includes("aivis"));
    if (onboardingComponentStatus) onboardingComponentStatus.textContent = "準備できました。";
    // 成功時も操作を戻しておく。戻さないと、次の画面から「追加機能へ戻る」で帰ってきたとき
    // 全部が無効のまま操作できない行き止まりになる。
    onboardingComponentStep.querySelectorAll("input,button").forEach(item => { item.disabled = false; });
    showOnboardingStep("character");
  } catch (error) {
    logApp("error", "追加機能を準備できませんでした", errorDetail(error));
    if (onboardingComponentStatus) onboardingComponentStatus.textContent = navigator.onLine
      ? `準備できませんでした: ${error?.message || error}`
      : "オフラインのため取得を見送りました。基本機能はそのまま使えます。";
    onboardingComponentStep.querySelectorAll("input,button").forEach(item => { item.disabled = false; });
    await refreshOnboardingComponents();
  }
}

async function completeSelectedOnboarding(openCharacterSettings) {
  onboarding.querySelectorAll("button").forEach(item => { item.disabled = true; });
  try {
    const result = await window.picoagent.completeOnboarding();
    renderRelationship(result);
    onboarding.hidden = true;
    setAnimation("success", true);
    showAssistantNotice("これからよろしくね。少しずつ、いっしょの時間を覚えていくよ。");
    setTimeout(() => { if (!currentVoice) setAnimation("idle"); }, 1100);
    scheduleContinuousListening();
  } catch (error) {
    logApp("error", "初回設定を保存できませんでした", errorDetail(error));
    showToast("初回設定を保存できませんでした", "error");
    onboarding.querySelectorAll("button").forEach(item => { item.disabled = false; });
    return;
  }
  if (openCharacterSettings) {
    try {
      await window.picoagent.openSettings("character-add");
    } catch (error) {
      logApp("error", "キャラクター追加画面を開けませんでした", errorDetail(error));
      showToast("キャラクター追加画面を開けませんでした", "error");
    }
  }
}

onboarding?.addEventListener("click", async event => {
  const target = event.target instanceof Element ? event.target : null;
  const actionButton = target?.closest("button[data-onboarding-action]");
  const action = actionButton?.dataset.onboardingAction;
  if (action === "back-components") {
    showOnboardingStep("components");
  } else if (action === "install-components") {
    await installSelectedOnboardingComponents();
  } else if (action === "skip-components") {
    showOnboardingStep("character");
  } else if (action === "add-character") {
    await completeSelectedOnboarding(true);
  } else if (action === "skip-character") {
    await completeSelectedOnboarding(false);
  }
});
onboarding?.addEventListener("keydown", event => {
  if (event.key !== "Tab") return;
  const controls = [...onboarding.querySelectorAll("button:not([disabled]),a[href]")]
    .filter(item => item.getClientRects().length > 0);
  if (!controls.length) return;
  const first = controls[0];
  const last = controls[controls.length - 1];
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
});
window.picoagent.listen("pet:show-onboarding", () => {
  onboarding?.querySelectorAll("button").forEach(item => { item.disabled = false; });
  showOnboardingStep("components");
  if (onboarding) onboarding.hidden = false;
  stopVoice();
}).catch(error => {
  logApp("warn", "初回セットアップ再表示イベントを受け取れませんでした", errorDetail(error));
});
window.picoagent.on("pet:layout-reset", () => {
  resizeWindowSize = { width: 0, height: 0 };
  bubble.style.removeProperty("width");
  bubble.style.removeProperty("height");
  bubble.style.removeProperty("left");
  bubble.style.removeProperty("top");
  resetFullscreenBubbleLayoutVars();
  // 立ち絵の移動量も既定（上半身が出る位置）へ戻す。
  applyCharacterOffset(0, 0, { clamp: false });
  setTimeout(refreshBubblePlacement, 0);
});
