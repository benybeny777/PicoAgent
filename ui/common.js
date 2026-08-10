// common.js - 複数ウィンドウで共有するエラー整形とアプリログ。

(function exposePicoUi() {
  function errorDetail(error) {
    if (!error) return "";
    if (error instanceof Error) return error.stack || error.message;
    if (typeof error === "string") return error;
    try { return JSON.stringify(error); }
    catch { return String(error); }
  }

  function logApp(level, message, detail) {
    try {
      window.picoagent?.log?.(level, message, detail == null ? "" : String(detail)).catch(error => {
        // フォールバック許可: ログ保存自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
        console.warn("アプリログ保存に失敗しました", error);
      });
    } catch (error) {
      // フォールバック許可: ログ保存自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
      console.warn("アプリログ保存に失敗しました", error);
    }
  }

  // 文字数で切り詰める。JS の `slice()` は UTF-16 単位なので、絵文字のような
  // サロゲートペアを半分で切って壊れた文字を残す（Rust 側で `text::preview()` に
  // 統一しているのと同じ理由）。`Array.from` は書記素ではなくコードポイント単位だが、
  // ペアを割らない点で素の slice より安全。
  function preview(text, maxChars) {
    const value = String(text ?? "");
    const chars = Array.from(value);
    if (chars.length <= maxChars) return value;
    return chars.slice(0, maxChars).join("") + "…";
  }

  function tail(text, maxChars) {
    const chars = Array.from(String(text ?? ""));
    return chars.length <= maxChars ? chars.join("") : chars.slice(-maxChars).join("");
  }

  function twoStepAction(button, options = {}) {
    const initialLabel = options.initialLabel || button.textContent || "実行";
    const confirmLabel = options.confirmLabel || "もう一度押して実行";
    const timeoutMs = Number.isFinite(options.timeoutMs) && options.timeoutMs > 0
      ? options.timeoutMs
      : 10_000;
    let armed = false;
    let timer = null;

    function reset({ notify = true } = {}) {
      if (timer != null) {
        clearTimeout(timer);
        timer = null;
      }
      if (!armed) return;
      armed = false;
      button.dataset.confirming = "false";
      button.textContent = initialLabel;
      if (notify) options.onCancel?.();
    }

    function request() {
      if (armed) {
        reset({ notify: false });
        return true;
      }
      armed = true;
      button.dataset.confirming = "true";
      button.textContent = confirmLabel;
      options.onArm?.();
      timer = setTimeout(() => reset(), timeoutMs);
      return false;
    }

    button.addEventListener("blur", () => reset());
    return Object.freeze({ request, reset, isArmed: () => armed });
  }

  window.PicoUI = Object.freeze({ errorDetail, logApp, preview, tail, twoStepAction });
}());
