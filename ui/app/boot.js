// boot.js - 起動画面と「表示の確定」の持ち主。
//
// 起動画面を組み立てたら窓を表示し、準備中は仮画像・別キャラ・読込途中の canvas を隠す。
// 選択キャラ・背景・必要なAIと音声の準備後に保存したサイズと位置を戻し、
// 最終の DOM と描画を確定してから `pet_reveal_main` で通常画面へ切り替える。

import * as bridge from "./bridge.js";
import { el, errorDetail, logApp } from "./util.js";

const INITIAL_PROGRESS_INTERVAL_MS = 1000;
const DEFAULT_PROGRESS_INTERVAL_MS = 10000;
const RETRY_MAX_MS = 30000;

export function createBoot({ app, screen, beforeReveal }) {
  const log = el("div", { className: "boot-log" }, [], { role: "group", "aria-label": "起動ログ" });
  const statusLine = el("div", { className: "boot-status" });
  screen.replaceChildren(el("div", { className: "boot-stage" }, [
    el("div", { className: "boot-mark", textContent: "PicoAgent" }),
    log,
    el("div", { className: "boot-progress" }, [el("span")], { "aria-hidden": "true" }),
    statusLine,
  ]));
  let lastText = "";
  let progressTimer = null;
  let backendFinished = false;
  let finishPromise = null;
  let retryTimer = null;
  let retryDelay = 1500;
  let released = false;

  function startProgress(intervalMs) {
    const interval = Number(intervalMs) > 0 ? Number(intervalMs) : DEFAULT_PROGRESS_INTERVAL_MS;
    if (progressTimer) clearInterval(progressTimer);
    progressTimer = setInterval(() => {
      bridge.bootProgress().catch(error => logApp("warn", "起動進捗を通知できませんでした", errorDetail(error)));
    }, interval);
  }

  async function reveal() {
    // 描画確定に失敗しても表示だけは必ず行う（ここで止まると窓が出ないまま操作手段が無くなる）。
    try { beforeReveal?.(); }
    catch (error) { logApp("warn", "表示前の描画確定に失敗しました", errorDetail(error)); }
    await bridge.revealMain();
  }

  async function finishBackend() {
    if (backendFinished) return;
    if (!finishPromise) {
      finishPromise = bridge.finishBoot()
        .then(() => { backendFinished = true; })
        .finally(() => { finishPromise = null; });
    }
    await finishPromise;
    if (released) await reveal();
  }

  function scheduleRetry(status) {
    if (backendFinished || retryTimer) return;
    status.setSticky("位置・寸法の復旧中…", "pending");
    retryTimer = setTimeout(async () => {
      retryTimer = null;
      try {
        await finishBackend();
        retryDelay = 1500;
        status.setSticky("");
      } catch (error) {
        logApp("warn", "起動後の位置・寸法を再試行できませんでした", errorDetail(error));
        retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
        scheduleRetry(status);
      }
    }, retryDelay);
  }

  startProgress(INITIAL_PROGRESS_INTERVAL_MS);
  window.addEventListener("beforeunload", () => {
    if (progressTimer) clearInterval(progressTimer);
    if (retryTimer) clearTimeout(retryTimer);
  });

  return {
    get booting() { return app.classList.contains("is-booting"); },
    /** 起動中だけ、状態行の文言を起動画面の状態欄と起動ログへ流す。 */
    status(text) {
      if (!app.classList.contains("is-booting")) return;
      const next = String(text || "");
      statusLine.textContent = next;
      if (next && next !== lastText) {
        lastText = next;
        log.append(el("div", { className: "boot-log-line", textContent: next }));
        while (log.children.length > 6) log.firstElementChild?.remove();
      }
    },
    setProgressInterval: startProgress,
    /** 起動画面を外して通常画面を出す。保存した位置・寸法の復元に失敗したら裏で再試行する。 */
    async release(status) {
      if (progressTimer) clearInterval(progressTimer);
      progressTimer = null;
      if (!backendFinished) {
        try { await finishBackend(); }
        catch (error) {
          logApp("warn", "起動完了を通知できませんでした", errorDetail(error));
          scheduleRetry(status);
        }
      }
      released = true;
      app.classList.remove("is-booting");
      if (backendFinished) await reveal();
    },
  };
}
