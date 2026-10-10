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

  // PIXIの0fpsは無制限なので、停止は必ずticker.stopで表す。
  let frameReporter = null;
  const frameSamples = new Map();
  function recordCharacterFrame(kind) {
    if (root.PicoCharacterFrameDebug !== true) { frameSamples.clear(); return; }
    const now = root.performance.now();
    const sample = frameSamples.get(kind) || { start: now, count: 0 };
    sample.count += 1;
    if (now - sample.start >= 1000) {
      frameReporter?.(kind, sample.count * 1000 / (now - sample.start));
      frameSamples.set(kind, { start: now, count: 0 });
    } else frameSamples.set(kind, sample);
  }
  function setFrameReporter(reporter) { frameReporter = reporter; frameSamples.clear(); }
  function createPixiBudget(ticker, onResume = () => {}, kind = "PIXI") {
    let current = 0;
    let requested = 0;
    let active = false;
    ticker.stop();
    ticker.add(() => recordCharacterFrame(kind), undefined, -1000);
    function apply() {
      const fps = active ? requested : 0;
      if (current === fps) return false;
      const wasStopped = current === 0;
      current = fps;
      if (fps === 0) ticker.stop();
      else {
        ticker.maxFPS = fps;
        if (wasStopped) {
          ticker.lastTime = root.performance.now();
          onResume();
          ticker.start();
        }
      }
      return true;
    }
    return Object.freeze({
      setBudget(fps) {
        if (!Number.isFinite(fps) || fps < 0) throw new Error("描画上限が不正です");
        requested = fps;
        return apply();
      },
      setActive(value) { active = Boolean(value); return apply(); },
    });
  }

  root.PicoFrameScheduler = Object.freeze({ create, createLatest, createLoop, createPixiBudget, recordCharacterFrame, setFrameReporter });
})(typeof window === "undefined" ? globalThis : window);
