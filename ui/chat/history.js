// history.js - セッション内の会話履歴（表示・検索・コピー・再利用・書き出し）の持ち主。
//
// 会話が伸びただけのときは末尾へ足すだけにする（80件たまった会話で発言のたびに全件を Markdown から
// 組み直すと、数百要素を作り直すことになる）。見えていない間は描かない。

import * as bridge from "../app/bridge.js";
import { toast } from "../app/toast.js";
import { copyText, el, errorDetail, logApp, preview } from "../app/util.js";
import { renderMarkdown } from "./markdown.js";

const MAX_ITEMS = 80;

export function createHistory({ log, search, jumpLatest, exportButton, characterName, onReuse, onExported, status, markdownHandlers }) {
  const items = [];
  let query = "";
  let visible = false;
  let thinking = null; // 考え中の行の文言（null なら出さない）
  let rendered = { query: null, items: [], name: "" };
  let taskPanel = null;
  let searchTimer = 0;

  const nearBottom = () => log.scrollHeight - log.scrollTop - log.clientHeight < 48;

  function updateJump() {
    jumpLatest.hidden = log.clientHeight === 0 || nearBottom();
  }

  function typingIndicator(label) {
    return el("div", { className: "typing-indicator" }, [
      el("span", { className: "typing-label", textContent: label }),
      el("span", { className: "typing-dots" }, [el("i"), el("i"), el("i")], { "aria-hidden": "true" }),
    ], { "aria-label": label });
  }

  function row(item) {
    const name = item.role === "user" ? "あなた" : characterName();
    const time = item.at
      ? el("span", { className: "history-time", textContent: new Date(item.at).toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" }) })
      : null;
    const copy = el("button", { type: "button", textContent: "コピー" });
    copy.addEventListener("click", async () => {
      try { status.flash((await copyText(item.content)) ? "履歴をコピーしました" : "コピーできませんでした", 1400); }
      catch (error) {
        logApp("warn", "履歴コピーに失敗しました", errorDetail(error));
        status.flash("コピーできませんでした", 1400);
      }
    });
    const actions = el("div", { className: "history-actions" }, [copy]);
    if (item.role === "user") {
      const reuse = el("button", { type: "button", textContent: "再利用" });
      reuse.addEventListener("click", () => onReuse(item.content));
      actions.append(reuse);
    }
    const body = el("div", { className: "history-body" });
    renderMarkdown(body, item.content, markdownHandlers);
    return el("div", { className: `history-item ${item.role}` }, [
      el("div", { className: "history-meta" }, [
        el("div", { className: "history-role" }, [el("span", { className: "history-role-name", textContent: name }), time]),
        actions,
      ]),
      body,
    ]);
  }

  function thinkingRow() {
    return el("div", { className: "history-item assistant history-thinking" }, [
      el("div", { className: "history-meta" }, [el("div", { className: "history-role" }, [el("span", { className: "history-role-name", textContent: characterName() })])]),
      el("div", { className: "history-body" }, [typingIndicator(thinking)]),
    ]);
  }

  function placeTaskPanel(stick) {
    if (!taskPanel) return;
    if (visible) {
      if (taskPanel.parentElement !== log || taskPanel.nextSibling) log.append(taskPanel);
      if (stick) log.scrollTop = log.scrollHeight;
    } else if (taskPanel.parentElement === log) {
      taskPanel.home?.after(taskPanel);
    }
  }

  function render() {
    const stick = nearBottom();
    if (!visible) {
      rendered = { query: null, items: [], name: "" };
      placeTaskPanel(false);
      return;
    }
    const name = characterName();
    if (!items.length && thinking == null) {
      log.replaceChildren(el("div", { className: "history-empty", textContent: "まだ会話はありません。入力欄から話しかけてください。" }));
      rendered = { query: null, items: [], name };
    } else {
      const normalized = query.trim().toLowerCase();
      const shown = normalized ? items.filter(item => item.content.toLowerCase().includes(normalized)) : items;
      if (!shown.length && normalized) {
        log.replaceChildren(el("div", { className: "history-empty", textContent: "一致する履歴はありません。" }));
        rendered = { query: null, items: [], name };
      } else {
        const appendOnly = rendered.query === normalized && rendered.name === name && rendered.items.length > 0
          && shown.length >= rendered.items.length && rendered.items.every((item, index) => shown[index] === item);
        const added = appendOnly ? shown.slice(rendered.items.length) : shown;
        if (appendOnly) log.querySelector(".history-thinking")?.remove();
        else log.replaceChildren();
        for (const item of added) log.append(row(item));
        if (thinking != null && !normalized) log.append(thinkingRow());
        rendered = { query: normalized, items: [...shown], name };
      }
    }
    placeTaskPanel(stick);
    if (stick) log.scrollTop = log.scrollHeight;
    updateJump();
  }

  log.addEventListener("scroll", updateJump, { passive: true });
  jumpLatest.addEventListener("click", () => {
    log.scrollTop = log.scrollHeight;
    updateJump();
  });
  search.addEventListener("input", () => {
    query = search.value;
    // 毎キーで全件を組み直さないよう 120ms まとめる。
    clearTimeout(searchTimer);
    searchTimer = setTimeout(render, 120);
  });
  search.addEventListener("keydown", event => {
    if (event.key !== "Escape" || event.isComposing) return;
    event.preventDefault();
    event.stopPropagation();
    clearTimeout(searchTimer);
    search.value = "";
    query = "";
    render();
  });
  exportButton.addEventListener("click", async () => {
    if (!items.length) { toast("書き出す会話がありません", "warn"); return; }
    exportButton.disabled = true;
    const name = characterName();
    const lines = [`# PicoAgent 会話履歴（${name}） ${new Date().toLocaleString("ja-JP")}`, ""];
    for (const item of items) {
      const at = item.at ? ` (${new Date(item.at).toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" })})` : "";
      lines.push(`## ${item.role === "user" ? "あなた" : name}${at}`, "", item.content, "");
    }
    try {
      const result = await bridge.exportHistory(lines.join("\n"));
      toast("書き出しました", "success");
      onExported?.(String(result?.path || ""));
    } catch (error) {
      toast(preview(error?.message || error || "書き出しに失敗しました", 60), "error");
    } finally {
      exportButton.disabled = false;
    }
  });

  return {
    get items() { return items; },
    add(role, text) {
      const content = String(text ?? "").trim();
      if (!content) return null;
      const item = { role, content, at: Date.now() };
      items.push(item);
      if (items.length > MAX_ITEMS) items.shift();
      render();
      return item;
    },
    /** 起動時・キャラ切替時に、保存された会話履歴から組み直す。 */
    hydrate(list) {
      items.length = 0;
      for (const entry of Array.isArray(list) ? list : []) {
        const role = entry?.role === "user" ? "user" : entry?.role === "assistant" ? "assistant" : "";
        const content = String(entry?.content ?? "").trim();
        if (role && content) items.push({ role, content });
      }
      if (items.length > MAX_ITEMS) items.splice(0, items.length - MAX_ITEMS);
      render();
    },
    clear() {
      items.length = 0;
      query = "";
      search.value = "";
      render();
    },
    /** 考え中の行。文言だけの更新で全件を組み直さない。 */
    setThinking(label) {
      const next = label == null ? null : String(label || "考え中").replace(/…+$/, "");
      const showing = next != null && thinking != null ? log.querySelector(".history-thinking") : null;
      thinking = next;
      if (showing) {
        showing.querySelector(".typing-indicator")?.setAttribute("aria-label", thinking);
        const node = showing.querySelector(".typing-label");
        if (node) node.textContent = thinking;
      } else {
        render();
      }
    },
    setVisible(value) {
      const next = Boolean(value);
      if (next === visible) return;
      visible = next;
      render();
    },
    get visible() { return visible; },
    /** 実行中タスクの一覧を、履歴を見せているときは会話の末尾へ置く（キャラの描画面へ重ねない）。 */
    attachTaskPanel(panel) {
      taskPanel = panel;
      if (!panel.home) {
        panel.home = el("span", { hidden: true });
        panel.before(panel.home);
      }
      placeTaskPanel(false);
    },
    scrollToEnd() {
      if (taskPanel?.parentElement === log) log.scrollTop = log.scrollHeight;
    },
    focusSearch() {
      search.focus();
      search.select();
    },
    typingIndicator,
    render,
  };
}
