// frame-budget.js - キャラの状態から描画上限と停止を決める唯一の正本。

export function frameBudget({ active, speaking = false, dragging = false, motion = "idle", reducedMotion = false }) {
  if (!active) return 0;
  if (reducedMotion) return 30;
  return speaking || dragging || motion !== "idle" ? 60 : 30;
}

/** 描画時刻の間引き。更新頻度の違いを吸収し、停止期間を復帰後へ持ち越さない。 */
export function createFrameGate() {
  let fps = 0;
  let last = null;
  return {
    setBudget(value) {
      if (!Number.isFinite(value) || value < 0) throw new Error("描画上限が不正です");
      if (fps === value) return false;
      fps = value;
      last = null;
      return true;
    },
    reset() { last = null; },
    allow(now) {
      if (!fps) return false;
      if (last === null) { last = now; return true; }
      const interval = 1000 / fps;
      const elapsed = now - last;
      if (elapsed + 1e-7 < interval) return false;
      last += Math.floor((elapsed + 1e-7) / interval) * interval;
      return true;
    },
  };
}
