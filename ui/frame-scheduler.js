// frame-scheduler.js - 高頻度イベントの表示更新を1フレーム1回へまとめる。

(function registerFrameScheduler(root) {
  "use strict";

  function create(callback, timing = {}) {
    const requestFrame = timing.requestFrame || (next => root.requestAnimationFrame(next));
    const cancelFrame = timing.cancelFrame || (id => root.cancelAnimationFrame(id));
    let frameId = null;

    function run() {
      frameId = null;
      callback();
    }

    function schedule() {
      if (frameId !== null) return false;
      frameId = requestFrame(run);
      return true;
    }

    function cancel() {
      if (frameId === null) return false;
      cancelFrame(frameId);
      frameId = null;
      return true;
    }

    function flush() {
      cancel();
      callback();
    }

    return Object.freeze({
      schedule,
      cancel,
      flush,
      pending: () => frameId !== null,
    });
  }

  root.PicoFrameScheduler = Object.freeze({ create });
})(typeof window === "undefined" ? globalThis : window);
