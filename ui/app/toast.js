// toast.js - 画面下部の短い通知。失敗は読み終わるまで消さず、必要なら次の操作へ進むボタンを付ける。

import { el, errorDetail, errorMessage, logApp, preview } from "./util.js";

export function toast(message, kind = "info", options = {}) {
  const duration = Math.max(1200, Number(options.duration) || (kind === "error" ? 5000 : 1800));
  const node = el("div", { className: `toast toast-${kind}` }, [el("span", { textContent: message })], { role: kind === "error" ? "alert" : "status" });
  let timer = null;
  const dismiss = () => {
    if (timer) clearTimeout(timer);
    node.classList.remove("toast-show");
    node.addEventListener("transitionend", () => node.remove(), { once: true });
    setTimeout(() => node.remove(), 300);
  };
  if (options.actionLabel && typeof options.onAction === "function") {
    // 操作ボタンのある通知だけクリックを受け取る。それ以外は背後のデスクトップへクリックを通す。
    node.classList.add("toast-actionable");
    const action = el("button", { type: "button", className: "toast-action", textContent: options.actionLabel });
    action.addEventListener("click", async () => {
      action.disabled = true;
      try { await options.onAction(); }
      catch (error) {
        logApp("error", "通知アクションに失敗しました", errorDetail(error));
        toast(`操作できませんでした: ${preview(errorMessage(error), 60)}`, "error");
      }
      dismiss();
    });
    node.append(action);
  }
  document.body.append(node);
  requestAnimationFrame(() => node.classList.add("toast-show"));
  timer = setTimeout(dismiss, duration);
}

// 目覚まし・通知のように裏で繰り返し確認している機能は、失敗しても画面へ何も出ないと
// 「登録したのに動かない」だけが残る。原因ごとに1度だけ知らせ、復旧したら次の失敗をまた知らせる。
const notifiedFailures = new Set();
export function notifyBackgroundFailure(key, message, error) {
  logApp("warn", message, errorDetail(error));
  if (notifiedFailures.has(key)) return;
  notifiedFailures.add(key);
  toast(message, "error", { duration: 8000 });
}
export function clearBackgroundFailure(key) {
  notifiedFailures.delete(key);
}
