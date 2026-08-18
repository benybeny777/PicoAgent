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

  // 高頻度イベントの最新値だけを次フレームへ渡す。
  function createLatest(callback, timing = {}) {
    let latestValue;
    let hasValue = false;
    const scheduler = create(() => {
      if (!hasValue) return;
      const value = latestValue;
      latestValue = undefined;
      hasValue = false;
      callback(value);
    }, timing);

    function schedule(value) {
      latestValue = value;
      hasValue = true;
      return scheduler.schedule();
    }

    function cancel() {
      latestValue = undefined;
      hasValue = false;
      return scheduler.cancel();
    }

    return Object.freeze({ schedule, cancel, pending: scheduler.pending });
  }

  // step が false を返すまでだけ継続し、状態変化時は start() で再開できるループ。
  function createLoop(step, timing = {}) {
    const requestFrame = timing.requestFrame || (next => root.requestAnimationFrame(next));
    const cancelFrame = timing.cancelFrame || (id => root.cancelAnimationFrame(id));
    let frameId = null;

    function run(now) {
      frameId = null;
      if (step(now) !== false) start();
    }

    function start() {
      if (frameId !== null) return false;
      frameId = requestFrame(run);
      return true;
    }

    function stop() {
      if (frameId === null) return false;
      cancelFrame(frameId);
      frameId = null;
      return true;
    }

    return Object.freeze({ start, stop, pending: () => frameId !== null });
  }

  root.PicoFrameScheduler = Object.freeze({ create, createLatest, createLoop });
})(typeof window === "undefined" ? globalThis : window);
