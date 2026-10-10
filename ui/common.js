// common.js - 複数ウィンドウで共有するエラー整形とアプリログ。

(function exposePicoUi() {
  function errorDetail(error) {
    if (!error) return "";
    if (error instanceof Error) return error.stack || error.message;
    if (typeof error === "string") return error;
    try { return JSON.stringify(error); }
    catch { return String(error); }
  }

  // 画面へ出す短い理由。Tauri の invoke は失敗を文字列で返すため、`error.message` だけを
  // 読むと「undefined」と表示される。Error・文字列・{message} のどれでも本文を取り出す。
  function errorMessage(error) {
    if (error == null || error === "") return "原因不明のエラー";
    if (typeof error === "string") return error;
    if (typeof error.message === "string" && error.message) return error.message;
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

  // 一覧を取得できなかったことと、正常な空一覧を呼出側が区別できる形で返す。
  async function loadUserCharacters(fetchEntries, assetUrl, bundled = []) {
    try {
      const result = await fetchEntries();
      if (!result || !Array.isArray(result.characters)) throw new Error("追加キャラクター一覧の形式が不正です");
      const known = new Set(bundled.map(entry => entry.id));
      const entries = [];
      for (const character of result.characters) {
        if (!character || typeof character.id !== "string" || !character.id || typeof character.path !== "string" || !character.path) {
          throw new Error("追加キャラクター一覧のIDまたは配置先が不正です");
        }
        if (known.has(character.id)) continue;
        known.add(character.id);
        entries.push({
          id: character.id, name: character.name || character.id,
          description: character.description || "", manifest: `${character.id}/character.json`,
          root: assetUrl(character.path), user: true,
          live2d: Boolean(character.live2d), mmd: Boolean(character.mmd),
        });
      }
      return { entries, error: null, root: result.root || "" };
    } catch (error) {
      return { entries: [], error, root: "" };
    }
  }

  window.PicoUI = Object.freeze({ errorDetail, errorMessage, logApp, preview, tail, twoStepAction, loadUserCharacters });
}());
