// task-run.js - 会話1件の「実行中のタスク」（考え中・ツール実行）の状態と経過時間を持ち、フキダシへ一覧表示する。
//
// 状態の正本はこのモジュールの run だけにする。renderer.js は送信開始・進捗文言・
// ツールの開始／完了イベント・終了を渡すだけで、行の組み立てとタイマーには触らない。

(function registerTaskRun(root) {
  "use strict";

  // 終わった一覧を少し残し、何が動いてどう終わったかを見られるようにする。
  const FINISHED_VISIBLE_MS = 5000;
  const TICK_MS = 1000;
  // 長いエージェント実行でも、一覧がフキダシを埋め尽くさないようにする。
  const MAX_ROWS = 12;
  // ツール完了直後に自動で足す「結果を確認中」を、この時間未満で終わったら残さない。
  const AUTO_ROW_MIN_MS = 1000;

  const STATE_LABELS = Object.freeze({
    running: "実行中",
    waiting: "返事待ち",
    completed: "完了",
    failed: "失敗",
    cancelled: "中断",
  });
  const TITLES = Object.freeze({
    running: "実行中のタスク",
    completed: "タスク完了",
    failed: "タスク失敗",
    cancelled: "タスクを中断しました",
  });

  function thinkingRow(run, label, now, auto = false) {
    run.sequence += 1;
    return { id: `think-${run.sequence}`, kind: "think", name: "", label, state: "running", startedAt: now, finishedAt: 0, auto };
  }

  function createRun(now = Date.now()) {
    const run = { state: "running", startedAt: now, finishedAt: 0, sequence: 0, step: 0, rows: [] };
    run.rows.push(thinkingRow(run, "考え中", now));
    return run;
  }

  function isOpen(row) {
    return row.state === "running" || row.state === "waiting";
  }

  function settle(row, state, now) {
    row.state = state;
    row.finishedAt = now;
  }

  // 実行中・返事待ちの行は捨てず、古い終了行から減らす。
  function trimRows(run) {
    while (run.rows.length > MAX_ROWS) {
      const index = run.rows.findIndex(row => !isOpen(row));
      if (index < 0) break;
      run.rows.splice(index, 1);
    }
  }

  // 進捗文言（計画中・考え直し中・ステップ）を考え中の行の名前にする。ツールの行はイベントだけで作る。
  function applyStatus(run, text, now = Date.now()) {
    if (run.state !== "running") return run;
    const label = String(text ?? "").replace(/\s*\n+\s*/g, " / ").replace(/…+$/, "").trim();
    if (!label || label.startsWith("ツールを実行中")) return run;
    const thinking = run.rows.filter(row => row.kind === "think" && row.state === "running").pop();
    if (thinking) {
      thinking.label = label;
      thinking.auto = false;
    } else if (!run.rows.some(row => row.kind === "tool" && isOpen(row))) {
      run.rows.push(thinkingRow(run, label, now));
      trimRows(run);
    }
    return run;
  }

  function applyActivity(run, activity, now = Date.now()) {
    if (run.state !== "running" || !activity || typeof activity !== "object") return run;
    const names = Array.isArray(activity.tools) ? activity.tools.filter(name => typeof name === "string") : [];
    const labels = Array.isArray(activity.labels) ? activity.labels : [];
    const failed = new Set(Array.isArray(activity.failedTools) ? activity.failedTools : []);
    if (activity.phase === "started") {
      for (const row of run.rows) {
        if (row.kind === "think" && row.state === "running") settle(row, "completed", now);
      }
      run.step += 1;
      names.forEach((name, index) => {
        const given = typeof labels[index] === "string" ? labels[index].trim() : "";
        run.rows.push({ id: `tool-${run.step}-${index}`, kind: "tool", name, label: given || name, state: "running", startedAt: now, finishedAt: 0, auto: false });
      });
    } else if (activity.phase === "completed" || activity.phase === "waiting") {
      for (const row of run.rows) {
        if (row.kind !== "tool" || row.state !== "running" || !names.includes(row.name)) continue;
        if (activity.phase === "waiting") row.state = "waiting";
        else settle(row, failed.has(row.name) ? "failed" : "completed", now);
      }
      if (activity.phase === "completed" && !run.rows.some(isOpen)) {
        run.rows.push(thinkingRow(run, "結果を確認中", now, true));
      }
    }
    trimRows(run);
    return run;
  }

  function finishRun(run, outcome, now = Date.now()) {
    if (run.state !== "running") return run;
    const state = outcome === "failed" || outcome === "cancelled" ? outcome : "completed";
    // スキルの結果をそのまま返した場合、自動で足した確認行は実際には動いていない。
    run.rows = run.rows.filter(row => !(row.auto && row.state === "running" && now - row.startedAt < AUTO_ROW_MIN_MS));
    for (const row of run.rows) {
      if (isOpen(row)) settle(row, state, now);
    }
    run.state = state;
    run.finishedAt = now;
    return run;
  }

  function formatElapsed(ms) {
    const total = Math.max(0, Math.floor(Number(ms) / 1000) || 0);
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const seconds = total % 60;
    if (hours > 0) return `${hours}時間${String(minutes).padStart(2, "0")}分`;
    if (minutes > 0) return `${minutes}分${String(seconds).padStart(2, "0")}秒`;
    return `${seconds}秒`;
  }

  function rowElapsed(row, now = Date.now()) {
    return (row.finishedAt || now) - row.startedAt;
  }

  function runElapsed(run, now = Date.now()) {
    return (run.finishedAt || now) - run.startedAt;
  }

  // フキダシの表示とタイマーを持つ。タイマーは終了・再開始・破棄のどの経路でも止める。
  function createController(options = {}) {
    const container = options.container || null;
    const now = options.now || (() => Date.now());
    const timers = options.timers || root;
    let run = null;
    let tick = null;
    let hideTimer = null;
    let renderedSignature = "";

    function stopTick() {
      if (tick === null) return;
      timers.clearInterval(tick);
      tick = null;
    }

    function stopHide() {
      if (hideTimer === null) return;
      timers.clearTimeout(hideTimer);
      hideTimer = null;
    }

    function setVisible(visible) {
      if (!container) return;
      container.hidden = !visible;
      if (options.onVisibilityChange) options.onVisibilityChange(visible);
    }

    // 表示に出る内容だけを並べた印。経過時間は `refreshTimes` が書き換えるので含めない。
    function runSignature(target) {
      if (!target) return "";
      const rows = target.rows
        .map(row => `${row.id}${row.state}${row.label}`)
        .join("");
      return `${target.state}|${rows}`;
    }

    function render() {
      if (!container || !run) return;
      // 進捗イベントは同じ文言が何度も届く。見た目が変わらないのに作り直すと、
      // 押している最中の「停止」ボタンが入れ替わってクリックが空振りする
      // （1秒ごとの更新で作り直さない理由と同じ）。
      const signature = runSignature(run);
      if (signature === renderedSignature) return;
      renderedSignature = signature;
      const doc = container.ownerDocument;
      const at = now();
      const head = doc.createElement("div");
      head.className = "task-run-head";
      const title = doc.createElement("span");
      title.className = "task-run-title";
      title.textContent = TITLES[run.state];
      const total = doc.createElement("span");
      total.className = "task-run-total";
      total.dataset.taskRunTotal = "";
      total.textContent = `経過 ${formatElapsed(runElapsed(run, at))}`;
      head.append(title, total);
      if (run.state === "running" && options.onCancel) {
        const cancel = doc.createElement("button");
        cancel.type = "button";
        cancel.className = "task-run-cancel";
        cancel.textContent = "停止";
        cancel.addEventListener("click", event => {
          event.stopPropagation();
          options.onCancel(cancel);
        });
        head.append(cancel);
      }
      const list = doc.createElement("ol");
      list.className = "task-run-list";
      for (const row of run.rows) {
        const item = doc.createElement("li");
        item.className = "task-run-row";
        item.dataset.state = row.state;
        const mark = doc.createElement("span");
        mark.className = "task-run-mark";
        mark.setAttribute("aria-hidden", "true");
        const label = doc.createElement("span");
        label.className = "task-run-label";
        label.textContent = row.label;
        label.title = row.label;
        const state = doc.createElement("span");
        state.className = "task-run-state";
        state.textContent = STATE_LABELS[row.state] || row.state;
        const time = doc.createElement("span");
        time.className = "task-run-time";
        time.dataset.taskRunRow = row.id;
        time.textContent = formatElapsed(rowElapsed(row, at));
        item.append(mark, label, state, time);
        list.append(item);
      }
      container.replaceChildren(head, list);
      if (options.onRender) options.onRender();
    }

    // 1秒ごとの更新は時間の文字だけを書き換え、停止ボタンを作り直さない（押している途中で消えないように）。
    function refreshTimes() {
      if (!container || !run) return;
      const at = now();
      const total = container.querySelector("[data-task-run-total]");
      if (total) total.textContent = `経過 ${formatElapsed(runElapsed(run, at))}`;
      for (const row of run.rows) {
        const time = container.querySelector(`[data-task-run-row="${row.id}"]`);
        if (time) time.textContent = formatElapsed(rowElapsed(row, at));
      }
    }

    return {
      start() {
        stopTick();
        stopHide();
        run = createRun(now());
        renderedSignature = ""; // 前回の実行と同じ内容でも必ず描き直す
        render();
        setVisible(true);
        tick = timers.setInterval(refreshTimes, TICK_MS);
      },
      status(text) {
        if (!run) return;
        applyStatus(run, text, now());
        render();
      },
      activity(event) {
        if (!run) return;
        applyActivity(run, event, now());
        render();
      },
      finish(outcome) {
        if (!run) return;
        stopTick();
        finishRun(run, outcome, now());
        render();
        stopHide();
        hideTimer = timers.setTimeout(() => {
          hideTimer = null;
          setVisible(false);
        }, FINISHED_VISIBLE_MS);
      },
      dispose() {
        stopTick();
        stopHide();
        run = null;
        renderedSignature = "";
        setVisible(false);
      },
      snapshot() {
        return run;
      },
    };
  }

  root.PicoTaskRun = Object.freeze({
    FINISHED_VISIBLE_MS,
    MAX_ROWS,
    STATE_LABELS,
    createRun,
    applyStatus,
    applyActivity,
    finishRun,
    formatElapsed,
    rowElapsed,
    createController,
  });
}(globalThis));
