// alarm-runtime.js - 目覚まし画面処理の副作用を持たない状態更新。

(function registerAlarmRuntime(root) {
  "use strict";

  function markFired(firedKeys, key, maxEntries = 50) {
    if (firedKeys.has(key)) return false;
    const limit = Math.max(1, Number(maxEntries) || 50);
    // 現在のキーを追加してから全消去すると、同じ分の次回ポーリングで二重発火する。
    if (firedKeys.size >= limit) firedKeys.clear();
    firedKeys.add(key);
    return true;
  }

  root.PicoAlarmRuntime = Object.freeze({ markFired });
})(typeof window === "undefined" ? globalThis : window);
