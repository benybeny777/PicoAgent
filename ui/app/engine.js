// engine.js - 同梱推論エンジン（local）の準備状況。送れない理由を状態行へ常設で出す。

import * as bridge from "./bridge.js";
import { errorDetail, logApp } from "./util.js";

export function createEngine({ status, onChange, onReady }) {
  let initialized = false;
  let required = false;
  let ready = true;
  let phase = "ready";
  let online = null;

  const gb = bytes => (Number(bytes) / 1e9).toFixed(1);

  function apply(state) {
    if (!state?.phase || !required) return;
    phase = state.phase;
    ready = phase === "ready";
    if (phase === "download") {
      // 初回だけ数GBを取る。割合しか出さないと待ち時間の見当が付かず固まって見える。
      const size = Number(state.totalBytes) > 0 ? `（${gb(state.receivedBytes)} / ${gb(state.totalBytes)}GB）` : "";
      status.setSticky(`ローカルモデルを取得中… ${state.percent ?? 0}%${size} ※初回だけ`, "pending");
    } else if (phase === "verify") {
      status.setSticky("ローカルモデルを確認中… ※数分かかることがあります", "pending");
    } else if (phase === "starting") {
      status.setSticky("ローカルAIを起動中…", "pending");
    } else if (phase === "idle") {
      status.setSticky("ローカルAIの起動を待っています…", "pending");
    } else if (phase === "error") {
      status.setSticky(`準備に失敗: ${state.message || "不明なエラー"}`, "error");
    } else if (phase === "ready") {
      status.setSticky("");
      onReady?.();
    }
    online?.(ready);
    onChange();
  }

  return {
    init(info, setOnline) {
      online = setOnline;
      required = (info.providerId || "local") === "local";
      ready = !required || info.up === true;
      phase = ready ? "ready" : "idle";
      initialized = true;
      online?.(info.up === true);
      onChange();
    },
    apply,
    get phase() { return phase; },
    get required() { return required; },
    /** 送信できない理由（送れるなら空文字）。 */
    blockedReason() {
      if (!initialized) return "初期化完了後に送信できます";
      if (required && !ready) return "ローカルAIの準備完了後に送信できます";
      return "";
    },
    explainBlocked() {
      if (!initialized) { status.flash("初期化を続けています。完了すると送信できます", 3000); return; }
      status.flash(phase === "error"
        ? "ローカルAIの準備に失敗しています。設定の「診断・修復」を確認してください"
        : "ローカルAIを準備中です。完了すると送信できます", 3000);
    },
    /** 準備が済むか、明示的な失敗になるまで待つ（送信できない通常画面を先に見せないため）。 */
    async waitUntilSettled(onPoll) {
      for (;;) {
        let state;
        try { state = await bridge.engineStatus(); }
        catch (error) {
          logApp("warn", "エンジン状態取得に失敗しました", errorDetail(error));
          const failed = { phase: "error", message: "ローカルAIの準備状態を確認できませんでした" };
          apply(failed);
          return failed;
        }
        apply(state);
        onPoll?.(state);
        if (state && (state.phase === "ready" || state.phase === "error")) return state;
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
    },
  };
}
