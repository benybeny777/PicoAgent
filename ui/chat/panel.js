// panel.js - 会話パネル（chat ソース）。回答欄・回答操作・履歴・入力欄・状態行を1つのパネルにまとめる。
//
// 見せ方はシーン定義の `options.variant` で決まる（形式名で分岐しない）。
//   bubble: 最新の回答と入力欄。拡大中だけ履歴を出す。`tail` があればキャラの方へ尻尾を向ける。
//   column: 履歴と入力欄。回答欄（#reply）は字幕のソースへ移して表示する。
// 画面読み上げへ知らせるのは回答欄と状態行だけ（パネル全体や履歴を生きた領域にすると繰り返し読まれる）。

import { createComposer } from "./composer.js";
import { createFontSizeControl } from "./font-size.js";
import { createHistory } from "./history.js";
import { renderMarkdown } from "./markdown.js";
import { createPager } from "./pager.js";
import { createStatus } from "../app/status.js";
import { el } from "../app/util.js";

const GREETING = "こんにちは。クリックして話しかけてね。";

export function createChatSource(context) {
  // ── 回答操作 ─────────────────────────────────────────────
  const noticeButton = el("button", { type: "button", className: "chip notice-toggle", textContent: "通知", title: "実行中・完了・通知" }, [], { "aria-label": "アクションセンター" });
  const copyButton = el("button", { type: "button", className: "chip", textContent: "コピー", title: "回答をコピー" }, [], { "aria-label": "回答をコピー" });
  const regenerateButton = el("button", { type: "button", className: "chip", textContent: "再生成", title: "会話を送信すると再生成できます", disabled: true }, [], { "aria-label": "別の答えを生成" });
  const replayButton = el("button", { type: "button", className: "chip", textContent: "読上", title: "もう一度読み上げ" }, [], { "aria-label": "もう一度読み上げ" });
  const stopButton = el("button", { type: "button", className: "chip", textContent: "停止", title: "読み上げを停止", hidden: true }, [], { "aria-label": "読み上げを停止" });
  const expandButton = el("button", { type: "button", className: "chip", textContent: "拡大", title: "会話パネルを拡大" }, [], { "aria-label": "会話パネルを拡大" });
  const actions = el("div", { className: "reply-actions" }, [noticeButton, copyButton, regenerateButton, replayButton, stopButton, expandButton], { role: "group", "aria-label": "回答操作" });

  const noticePanel = el("div", { className: "notice-panel", hidden: true }, [], { role: "region", "aria-label": "アクションセンター" });

  // ── 履歴 ────────────────────────────────────────────────
  const search = el("input", { type: "search", autocomplete: "off", placeholder: "履歴を検索", title: "履歴を検索（Ctrl+F／⌘+F、Escapeで解除）" }, [], { "aria-label": "会話履歴を検索" });
  const jumpLatest = el("button", { type: "button", className: "chip", textContent: "末尾へ", title: "表示中の履歴の末尾へ移動", hidden: true });
  const exportButton = el("button", { type: "button", className: "chip", textContent: "書き出し", title: "会話をMarkdownで書き出し" });
  const historyTools = el("div", { className: "history-tools" }, [search, jumpLatest, exportButton], { role: "group", "aria-label": "会話履歴操作" });
  const historyLog = el("div", { className: "history-log" }, [], { role: "group", "aria-label": "会話履歴" });

  // ── 回答欄（生きた領域は1つだけ） ───────────────────────────
  const reply = el("div", { id: "reply", className: "reply" }, [], { "aria-live": "polite", "aria-atomic": "true" });
  const replyHome = el("div", { className: "reply-home" }, [reply]);

  // ── 入力欄 ──────────────────────────────────────────────
  const input = el("textarea", { id: "input", rows: 2, placeholder: "話しかける… / 画像をペーストまたはドロップ" }, [], { "aria-label": "メッセージ" });
  const voiceLoopButton = el("button", { type: "button", className: "voice-loop-button", textContent: "連続", title: "返答後も自動で聞き続ける" }, [], { "aria-pressed": "false" });
  const micButton = el("button", { type: "button", id: "micButton", textContent: "音声", title: "クリックで録音開始・停止 / 長押し中だけ録音" }, [], { "aria-label": "音声入力" });
  const sendButton = el("button", { type: "submit", id: "sendButton", className: "primary", textContent: "送る" }, [], { "aria-label": "送信" });
  const previewBox = el("div", { className: "image-preview", hidden: true });
  const suggestions = el("div", { className: "attachment-suggestions", hidden: true }, [], { role: "group", "aria-label": "添付ファイルのおすすめ操作" });
  const quickActions = el("div", { className: "quick-actions" }, [], { role: "group", "aria-label": "クイック操作" });
  const form = el("form", { id: "composer", className: "composer", hidden: true }, [
    previewBox, suggestions, quickActions, input,
    el("div", { className: "composer-actions" }, [voiceLoopButton, micButton, sendButton]),
  ]);

  // ── 状態・実行中タスク・中止 ──────────────────────────────
  const statusElement = el("div", { id: "status", className: "status" }, [], { role: "status", "aria-live": "polite", "aria-atomic": "true" });
  const taskRun = el("div", { id: "taskRun", className: "task-run", hidden: true }, [], { role: "group", "aria-label": "実行中のタスク" });
  const cancelVoiceButton = el("button", { type: "button", className: "cancel-voice", textContent: "読み上げを中止", hidden: true });
  const cancelSendButton = el("button", { type: "button", className: "cancel-send", textContent: "処理を止める", hidden: true });
  const tail = el("span", { className: "chat-tail" }, [], { "aria-hidden": "true" });
  const resizeGrip = el("button", { type: "button", className: "source-grip", title: "ドラッグで会話パネルの大きさを変える" }, [], { "aria-label": "会話パネルの大きさを変更" });

  const element = el("section", { className: "chat" }, [
    actions, noticePanel, historyTools, historyLog, replyHome, form, statusElement, taskRun, cancelVoiceButton, cancelSendButton, tail, resizeGrip,
  ]);

  const status = createStatus(statusElement, { onBootText: text => context.boot?.status(text) });
  let variant = "bubble";
  let lastAssistantText = "";

  // ── 文字サイズ（見た目の適用はここ、保存は font-size.js） ─────────
  function applyFontPx(size) {
    element.style.setProperty("--chat-font-size", `${size}px`);
    // 状態表示は補助の1行。会話文字を大きくしても上限を設け、入力欄を窓外へ押し出さない。
    element.style.setProperty("--status-font-size", `${Math.min(18, Math.max(9, Math.round(size * 0.72)))}px`);
  }
  function applyStreamFontPx(size) {
    element.style.setProperty("--stream-chat-font-size", `${size}px`);
    element.style.setProperty("--stream-chat-control-font-size", `${Math.round(size * 0.8)}px`);
    element.style.setProperty("--stream-chat-action-font-size", `${Math.round(size * 0.65)}px`);
  }
  const fontSize = createFontSizeControl({
    panel: element,
    variant: () => variant,
    apply: (kind, px) => (kind === "column" ? applyStreamFontPx(px) : applyFontPx(px)),
  });
  actions.append(fontSize.toggle);
  actions.after(fontSize.popover);
  let tailWanted = false;

  const markdownHandlers = {
    onApproval: value => context.send?.approve(value),
  };
  const history = createHistory({
    log: historyLog,
    search,
    jumpLatest,
    exportButton,
    status,
    markdownHandlers,
    characterName: () => context.host?.pack?.name || "キャラクター",
    onReuse: text => composer.reuse(text),
    onExported: path => context.notices?.push({ kind: "export", ok: true, title: "会話履歴を書き出しました", body: path, at: new Date().toISOString() }),
  });
  history.attachTaskPanel(taskRun);
  const composer = createComposer({ form, input, preview: previewBox, suggestions, quickActions, sendButton, status });

  const reusable = () => context.sources?.get("subtitle")?.element;
  const pager = createPager({
    isActive: () => variant === "column",
    render: text => chat.renderReply(text, { remember: false }),
    fits: text => {
      renderMarkdown(reply, text, markdownHandlers);
      reply.scrollTop = 0;
      return reply.scrollHeight <= reply.clientHeight + 1;
    },
  });

  // パネルの空いた所をクリックしたら入力欄を開く（ボタン・リンク・文字選択中は除く）。
  element.addEventListener("click", event => {
    if (!noticePanel.hidden && !(event.target instanceof Element && event.target.closest(".notice-panel,.notice-toggle"))) noticePanel.hidden = true;
    if (event.target instanceof Element && event.target.closest("button,a,textarea,input,.reply-actions,.task-run,.notice-panel,.history-log")) return;
    if (String(window.getSelection?.() ?? "").trim()) return;
    composer.open();
  });
  composer.onOpenChange(open => {
    element.classList.toggle("composing", open);
    if (!open) setTimeout(() => context.ambient?.flushNotice(), 0);
  });

  const chat = {
    element,
    reply,
    status,
    history,
    composer,
    pager,
    taskRun,
    noticePanel,
    buttons: { noticeButton, copyButton, regenerateButton, replayButton, stopButton, expandButton, voiceLoopButton, micButton, sendButton, cancelVoiceButton, cancelSendButton },
    resizeGrip,
    get lastAssistantText() { return lastAssistantText; },
    set lastAssistantText(value) { lastAssistantText = String(value || ""); },
    get variant() { return variant; },

    /** 確定した本文を描く。生成中の保留（aria-busy）を解いて、ここで1回だけ読み上げさせる。 */
    renderReply(text, { remember = true } = {}) {
      reply.classList.remove("reply-error", "reply-thinking", "streaming");
      reply.removeAttribute("aria-busy");
      renderMarkdown(reply, String(text ?? ""), markdownHandlers);
      reply.scrollTop = 0;
      if (remember && String(text ?? "").trim()) lastAssistantText = String(text);
    },
    /** 回答を出す。配信の字幕はページに分けて送る。 */
    showAnswer(text, { paginate = true } = {}) {
      const source = String(text ?? "");
      if (variant === "column" && paginate && source.trim()) {
        lastAssistantText = source;
        pager.timed(source);
        return;
      }
      chat.renderReply(source);
    },
    /** 生成途中の本文（読み上げの保留は続ける）。下端付近にいるときだけ追従する。 */
    renderStreaming(text) {
      const nearBottom = reply.scrollHeight - reply.scrollTop - reply.clientHeight < 48;
      reply.classList.remove("reply-thinking");
      reply.classList.add("streaming");
      renderMarkdown(reply, text, markdownHandlers);
      if (nearBottom) reply.scrollTop = reply.scrollHeight;
    },
    setThinking(active, label = "考え中") {
      if (active) reply.setAttribute("aria-busy", "true");
      history.setThinking(active ? label : null);
      if (active && !history.visible) {
        reply.classList.remove("reply-error", "streaming");
        reply.classList.add("reply-thinking");
        reply.replaceChildren(history.typingIndicator(String(label || "考え中").replace(/…+$/, "")));
      }
    },
    /** 回答を描かないまま終わる中断・停止でも、3点表示と読み上げの保留を残さない。 */
    clearThinking() {
      reply.removeAttribute("aria-busy");
      reply.classList.remove("streaming");
      if (!reply.classList.contains("reply-thinking")) return;
      reply.classList.remove("reply-thinking");
      reply.replaceChildren();
    },
    showError(text, onRetry) {
      chat.renderReply(text, { remember: false });
      reply.classList.add("reply-error");
      if (onRetry) {
        const retry = el("button", { type: "button", className: "retry-btn", textContent: "もう一度試す" });
        retry.addEventListener("click", onRetry);
        reply.querySelector(".md")?.append(retry);
      }
    },
    showAlarm(text, onAwake) {
      chat.renderReply(text);
      const awake = el("button", { type: "button", className: "primary", textContent: "おきたよ！" });
      awake.addEventListener("click", onAwake);
      reply.append(el("div", { className: "alarm-actions" }, [awake]));
    },
    clearAlarm() {
      reply.querySelector(".alarm-actions")?.remove();
    },
    greeting: GREETING,
    /** 読み上げ中だけ「読上」の枠を「停止」へ入れ替える（押したボタンのフォーカスも移す）。 */
    setVoiceActive(active) {
      cancelVoiceButton.hidden = !active;
      const handOff = document.activeElement === (active ? replayButton : stopButton);
      stopButton.hidden = !active;
      replayButton.hidden = active;
      if (handOff) (active ? stopButton : replayButton).focus();
    },
    setFontPx(px) {
      const size = parseFloat(px);
      if (!Number.isFinite(size) || size <= 0) return;
      applyFontPx(size);
      fontSize.setSize("bubble", size);
    },
    setStreamFontPx(px) {
      const requested = Number(px);
      const size = Number.isInteger(requested) && requested >= 18 && requested <= 48 ? requested : 22;
      applyStreamFontPx(size);
      fontSize.setSize("column", size);
    },
    update({ options, expanded }) {
      const nextVariant = options.variant === "column" ? "column" : "bubble";
      if (nextVariant !== variant) pager.cancel();
      variant = nextVariant;
      element.dataset.variant = variant;
      fontSize.refresh();
      element.dataset.compact = String(Boolean(options.compact));
      element.classList.toggle("expanded", Boolean(expanded));
      history.setVisible(variant === "column" || Boolean(expanded));
      // 配信の履歴列は空き面をクリックしても入力が開かないため、入力欄を常設する。
      composer.setPersistent(variant === "column");
      // 回答欄は1つだけ。配信では字幕のソースへ移し、それ以外はパネルの中へ戻す。
      const host = variant === "column" ? reusable() : replyHome;
      if (host && reply.parentElement !== host) host.append(reply);
      tailWanted = options.tail === "toward-character" && !expanded;
    },
    /** 尻尾をキャラの方へ向ける（キャラ矩形の中心がパネルのどちら側にあるか）。配置の確定後に呼ぶ。 */
    refreshTail() {
      tail.hidden = !tailWanted;
      if (!tailWanted) return;
      const layout = context.stage?.lastLayout;
      const own = layout?.placed?.chat;
      const target = layout?.placed?.character;
      if (!own || !target) return;
      const cx = target.left + target.width / 2;
      const cy = target.top + target.height / 2;
      const faceY = target.top + target.height * 0.3; // 横向きの尻尾は顔のあたりを指す
      const side = cx > own.left + own.width ? "right"
        : cx < own.left ? "left"
          : cy > own.top + own.height ? "bottom" : "top";
      tail.dataset.side = side;
      const along = side === "right" || side === "left"
        ? Math.min(own.height - 28, Math.max(28, faceY - own.top))
        : Math.min(own.width - 28, Math.max(28, cx - own.left));
      tail.style.setProperty("--tail-offset", `${along}px`);
    },
  };
  reply.textContent = GREETING;
  return chat;
}
