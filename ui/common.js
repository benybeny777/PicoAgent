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

  window.PicoUI = Object.freeze({ errorDetail, logApp });
}());
