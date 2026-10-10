// notice.js - アクションセンター（通知・会話タスク・スケジュール・永続ジョブ）の持ち主。
//
// `pet:notice`、`pet_schedule_runs`、会話タスクの状態を同じ一覧へ出し、未読件数をバッジ表示する。
// 既読だけが残っているときはボタンに印を出さない（常時印が付くと未読件数が目立たなくなる）。
// 直近50件は localStorage に持つ。

import * as bridge from "./bridge.js";
import { clearBackgroundFailure, notifyBackgroundFailure, createBackgroundFailureReporter } from "./toast.js";
import { el, errorMessage } from "./util.js";

import { readJson, write, remove } from "./ui-store.js";

const STORAGE_KEY = "picoagent.actionCenter.v1";
const LIMIT = 50;
const DURABLE_LABELS = { queued: "順番待ち", running: "実行中", completed: "完了", failed: "失敗", waiting_auth: "認証待ち", cancelled: "中断" };
const JOB_LABELS = { running: "実行中", failed: "失敗", cancelled: "中断", completed: "完了" };

/** 中止ボタンを出すかどうかの判定。待機中もまだ止められる。 */
const durableStatus = state => (state === "queued" || state === "running" ? "running" : String(state || "completed"));

function validEntries(value) {
  return Array.isArray(value) && value.every(entry => entry && typeof entry === "object" && !Array.isArray(entry)
    && typeof entry.kind === "string" && typeof entry.title === "string" && typeof entry.body === "string"
    && typeof entry.at === "string" && typeof entry.ok === "boolean"
    && ["taskId", "runId", "jobId", "rule", "status", "prompt", "state"].every(key => entry[key] === undefined || typeof entry[key] === "string")
    && (entry.progress === undefined || Number.isFinite(entry.progress)));
}

function durableEntry(notice) {
  return {
    kind: "durable_job",
    jobId: String(notice?.jobId || ""),
    title: String(notice?.title || "バックグラウンド処理"),
    body: String(notice?.body || ""),
    ok: notice?.ok !== false,
    state: String(notice?.state || "running"),
    progress: Number(notice?.progress) || 0,
    at: notice?.at || new Date().toISOString(),
  };
}

/** 遅延した進捗・一覧応答で完了状態を巻き戻さない。再試行の queued は新しい状態として受け取る。 */
function staleDurable(previous, next) {
  if (!previous) return false;
  if (String(next.at) < String(previous.at)) return true;
  return next.state === "running" && !["queued", "running"].includes(previous.state)
    && String(next.at) <= String(previous.at);
}

export function createNotices({ panel, button, status, onRetryPrompt, onCancelJob }) {
  const storageFailures = createBackgroundFailureReporter("notice-storage");
  const entries = [];
  const storageProblems = new Map();
  let savingBlocked = false;
  let unread = 0;
  let clearRevision = 0;
  const list = el("div", { className: "notice-list" });
  const clear = el("button", { type: "button", className: "chip", textContent: "消去" });
  const storageReason = el("div", { className: "notice-body", role: "alert", hidden: true });
  const reload = el("button", { type: "button", className: "chip", textContent: "通知履歴を再読込", hidden: true });
  panel.replaceChildren(el("div", { className: "notice-head" }, [el("strong", { textContent: "アクションセンター" }), clear]), storageReason, reload, list);

  function updateStorageReason() {
    storageReason.textContent = [...storageProblems.values()].join(" / ");
    storageReason.hidden = storageProblems.size === 0;
    reload.hidden = storageProblems.size === 0;
  }
  function storageFailure(operation, reason, message) {
    storageProblems.set(operation, message);
    updateStorageReason();
    storageFailures.failure(operation, reason, message);
  }
  function storageRecovery(operation) {
    storageProblems.delete(operation);
    updateStorageReason();
    storageFailures.recover(operation);
  }
  function storedIdentity(entry) {
    if (entry.kind === "schedule" && entry.runId) return `schedule:${entry.runId}`;
    if (["job", "durable_job"].includes(entry.kind) && entry.jobId) return `${entry.kind}:${entry.jobId}`;
    return JSON.stringify([entry.kind, entry.title, entry.body, entry.at]);
  }
  function reloadStored() {
    const loaded = readJson(STORAGE_KEY, validEntries);
    let restored;
    let removedCorruption = false;
    if (!loaded.ok) {
      if (!loaded.corrupted) {
        savingBlocked = true;
        storageFailure("read", loaded.reason, `通知履歴を読み込めず保存を停止しました: ${loaded.reason}`);
        return;
      }
      const deleted = remove(STORAGE_KEY);
      if (!deleted.ok) {
        savingBlocked = true;
        storageFailure("remove", deleted.reason, `通知履歴の原本を保持し保存を停止しました: ${deleted.reason}`);
        return;
      }
      removedCorruption = true;
      restored = [];
    } else restored = loaded.value ?? [];
    savingBlocked = false;
    storageRecovery("read");
    storageRecovery("remove");
    // 新着を先に置き、同時刻では新着を優先する。未読数は再読込で変更しない。
    const merged = new Map(entries.map(entry => [storedIdentity(entry), entry]));
    for (const original of restored.slice(0, LIMIT)) {
      const entry = original.kind === "job" && original.status === "running"
        ? { ...original, status: "failed", ok: false, body: "アプリ終了により処理結果を確認できませんでした。" } : original;
      const identity = storedIdentity(entry);
      const previous = merged.get(identity);
      if (!previous || (String(entry.at) > String(previous.at) && !(entry.kind === "durable_job" && staleDurable(previous, entry)))) merged.set(identity, entry);
    }
    entries.splice(0, entries.length, ...[...merged.values()].sort((a, b) => String(b.at).localeCompare(String(a.at))).slice(0, LIMIT));
    if (removedCorruption) notices.push({ kind: "通知", title: "通知履歴を消しました", body: "保存された通知履歴の形式が壊れていたため消しました。" });
    else changed();
  }

  const persistNow = () => {
    if (savingBlocked) return;
    const result = write(STORAGE_KEY, JSON.stringify(entries.slice(0, LIMIT)));
    if (result.ok) storageRecovery("write");
    else storageFailure("write", result.reason, `通知履歴を保存できません: ${result.reason}`);
  };
  // エージェント実行中は進捗のたびに呼ばれるため、保存は1フレーム1回へまとめる。
  const persistScheduler = window.PicoFrameScheduler.create(persistNow);
  window.addEventListener("beforeunload", () => persistScheduler.flush());

  function updateBadge() {
    button.dataset.count = unread > 0 ? String(Math.min(99, unread)) : "";
  }

  function actionButton(label, handler) {
    const node = el("button", { type: "button", className: "chip", textContent: label });
    node.addEventListener("click", async event => {
      event.stopPropagation();
      node.disabled = true;
      try { await handler(node); }
      finally { if (node.isConnected && node.dataset.keepDisabled !== "true") node.disabled = false; }
    });
    return node;
  }

  function render() {
    // 閉じている一覧は作り直さない（開くときに必ず呼び直す）。
    if (panel.hidden) return;
    if (!entries.length) {
      list.replaceChildren(el("div", { className: "notice-empty", textContent: "通知はありません" }));
      return;
    }
    list.replaceChildren(...entries.map(notice => {
      const title = el("div", { className: "notice-title", textContent: notice.title || "通知" });
      const item = el("div", { className: `notice-item ${notice.ok === false ? "error" : "ok"}` }, [
        title,
        el("div", { className: "notice-meta", textContent: notice.at ? new Date(notice.at).toLocaleString("ja-JP") : "" }),
        el("div", { className: "notice-body", textContent: notice.body || "" }),
      ]);
      if (notice.kind === "job") {
        title.append(el("span", { className: `notice-state notice-state-${notice.status || "completed"}`, textContent: JOB_LABELS[notice.status] || "完了" }));
      }
      if (notice.kind === "durable_job") {
        const state = durableStatus(notice.state);
        title.append(el("span", { className: `notice-state notice-state-${state}`, textContent: DURABLE_LABELS[notice.state] || "処理中" }));
        if (state === "running") {
          const fill = el("div", { className: "notice-progress-fill" });
          fill.style.width = `${Math.max(0, Math.min(100, Number(notice.progress) || 0))}%`;
          item.append(el("div", { className: "notice-progress" }, [fill]));
        }
      }
      const buttons = [];
      if (notice.taskId) {
        buttons.push(actionButton("再実行", async () => {
          try {
            await bridge.rerunScheduleTask(notice.taskId);
            status.flash("スケジュールを再実行しました", 2000);
          } catch (error) {
            notices.push({ kind: "schedule", title: "スケジュール再実行失敗", body: errorMessage(error), ok: false });
          }
        }));
      }
      if (notice.kind === "job" && notice.status === "running") buttons.push(actionButton("停止", node => onCancelJob(node)));
      else if (notice.kind === "job" && notice.prompt) buttons.push(actionButton("もう一度", () => onRetryPrompt(notice.prompt)));
      if (notice.kind === "durable_job" && notice.jobId) {
        const running = durableStatus(notice.state) === "running";
        buttons.push(actionButton(running ? "中止" : "再試行", async () => {
          try {
            if (running) await bridge.cancelDurableJob(notice.jobId);
            else await bridge.retryDurableJob(notice.jobId);
            status.flash(running ? "ジョブを中止しました" : "ジョブを再試行します", 2000);
          } catch (error) {
            // 押したのに何も起きない状態を作らない。
            status.flash(`${running ? "中止" : "再試行"}できませんでした: ${errorMessage(error)}`, 4000);
          }
        }));
      }
      if (notice.kind === "project_rule" && notice.rule) {
        buttons.push(actionButton("取り消し", async node => {
          try {
            const removed = await bridge.undoProjectRule(notice.rule);
            status.flash(removed ? "AGENTS.mdから取り消しました" : "対象の行は見つかりませんでした", 2000);
            if (removed) { node.textContent = "取り消し済み"; node.dataset.keepDisabled = "true"; }
          } catch (error) {
            status.flash(`取り消せませんでした: ${errorMessage(error)}`, 4000);
          }
        }));
      }
      if (buttons.length) {
        buttons.push(actionButton("設定", async () => {
          try { await bridge.openSettings(); }
          catch (error) { status.flash(`設定を開けませんでした: ${errorMessage(error)}`, 4000); }
        }));
        item.append(el("div", { className: "notice-actions" }, buttons));
      }
      return item;
    }));
  }

  function changed() {
    persistScheduler.schedule();
    updateBadge();
    render();
  }

  function upsertDurable(notice) {
    const entry = durableEntry(notice);
    if (!entry.jobId) return;
    const index = entries.findIndex(item => item.kind === "durable_job" && item.jobId === entry.jobId);
    if (index >= 0 && staleDurable(entries[index], entry)) return;
    const previousState = index >= 0 ? entries[index].state : "";
    if (index >= 0) entries.splice(index, 1);
    entries.unshift(entry);
    entries.length = Math.min(entries.length, LIMIT);
    // 未読は状態が変わったときだけ増やす（進捗のたびに数えると数字が暴れる）。
    if (previousState !== entry.state) {
      unread += 1;
      status.flash(entry.ok ? entry.title : `${entry.title}: エラー`, 6000);
    }
    changed();
  }

  const notices = {
    push(notice) {
      const kind = String(notice?.kind || "");
      if (kind === "durable_job" && notice?.jobId) { upsertDurable(notice); return; }
      // 裏で繰り返し失敗する機能は一覧へ積まず、原因ごとに1度だけ知らせる共通経路へ回す。
      if (kind === "background_failure") {
        const key = String(notice?.key || "background");
        if (notice?.ok === true) clearBackgroundFailure(key);
        else notifyBackgroundFailure(key, String(notice?.body || notice?.title || "処理に失敗しました"), key);
        return;
      }
      const entry = {
        kind,
        title: String(notice?.title || "通知"),
        body: String(notice?.body || notice?.message || ""),
        ok: notice?.ok !== false,
        taskId: notice?.taskId ? String(notice.taskId) : "",
        runId: notice?.runId ? String(notice.runId) : "",
        jobId: notice?.jobId ? String(notice.jobId) : "",
        rule: notice?.rule ? String(notice.rule) : "",
        status: notice?.status ? String(notice.status) : "",
        prompt: notice?.prompt ? String(notice.prompt) : "",
        at: notice?.at || new Date().toISOString(),
      };
      entries.unshift(entry);
      entries.length = Math.min(entries.length, LIMIT);
      unread += 1;
      status.flash(entry.ok ? entry.title : `${entry.title}: エラー`, 6000);
      changed();
    },
    /** 会話1件の状態（同じ jobId の行を上書きする）。 */
    upsertJob(jobId, patch) {
      let entry = entries.find(item => item.kind === "job" && item.jobId === jobId);
      if (!entry) {
        entry = { kind: "job", jobId, title: "エージェント処理", body: "開始しました", ok: true, status: "running", prompt: "", at: new Date().toISOString() };
        entries.unshift(entry);
      }
      Object.assign(entry, patch, { kind: "job", jobId });
      entries.length = Math.min(entries.length, LIMIT);
      changed();
    },
    toggle(open = panel.hidden) {
      panel.hidden = !open;
      if (open) { unread = 0; render(); updateBadge(); }
    },
    close() { panel.hidden = true; },
    /** 前回までのスケジュール実行履歴を一覧へ戻す。 */
    async loadScheduleRuns() {
      const revision = clearRevision;
      try {
        const runs = await bridge.scheduleRuns();
        if (revision !== clearRevision) return;
        if (!Array.isArray(runs)) throw new Error("スケジュール実行履歴の形式が不正です");
        clearBackgroundFailure("schedule-history-load");
        const others = entries.filter(entry => entry.kind !== "schedule");
        const scheduled = runs.slice(0, 20).map(run => ({
          kind: "schedule", title: String(run.title || "スケジュール"), body: String(run.body || ""), ok: run.ok !== false,
          taskId: run.task_id ? String(run.task_id) : "", runId: run.id ? String(run.id) : "", at: run.at || "",
        }));
        const restored = scheduled.map(entry => {
          const previous = entry.runId && entries.find(item => item.kind === "schedule" && item.runId === entry.runId);
          return previous && String(previous.at) > String(entry.at) ? previous : entry;
        });
        const restoredIds = new Set(restored.map(entry => entry.runId).filter(Boolean));
        const arriving = entries.filter(entry => entry.kind === "schedule" && (!entry.runId || !restoredIds.has(entry.runId)));
        entries.splice(0, entries.length, ...others, ...restored, ...arriving);
        entries.sort((a, b) => String(b.at).localeCompare(String(a.at)));
        entries.length = Math.min(entries.length, LIMIT);
        changed();
      } catch (error) {
        if (revision !== clearRevision) return;
        notifyBackgroundFailure("schedule-history-load", `スケジュール実行履歴を読み込めません: ${errorMessage(error)}`, error);
      }
    },
    /** 前回から残っている永続ジョブを戻す（起動前に流れたイベントは受け取れないため）。 */
    async loadDurableJobs() {
      const revision = clearRevision;
      try {
        const jobs = await bridge.durableJobs();
        if (revision !== clearRevision) return;
        if (!Array.isArray(jobs)) throw new Error("永続ジョブ一覧の形式が不正です");
        clearBackgroundFailure("durable-jobs-load");
        const others = entries.filter(entry => entry.kind !== "durable_job");
        const restored = jobs.slice(0, 20).map(job => {
          const entry = durableEntry({
          jobId: job.id, title: job.label, body: (job.error || job.result || job.step || "").split("\n")[0],
          ok: job.state !== "failed" && job.state !== "waiting_auth", state: job.state, progress: job.progress, at: job.updated_at,
          });
          const previous = entries.find(item => item.kind === "durable_job" && item.jobId === entry.jobId);
          return staleDurable(previous, entry) ? previous : entry;
        });
        const restoredIds = new Set(restored.map(entry => entry.jobId));
        const arriving = entries.filter(entry => entry.kind === "durable_job" && !restoredIds.has(entry.jobId));
        entries.splice(0, entries.length, ...others, ...restored, ...arriving);
        entries.sort((a, b) => String(b.at).localeCompare(String(a.at)));
        entries.length = Math.min(entries.length, LIMIT);
        changed();
      } catch (error) {
        if (revision !== clearRevision) return;
        notifyBackgroundFailure("durable-jobs-load", `永続ジョブ一覧を復元できません: ${errorMessage(error)}`, error);
      }
    },
  };

  button.addEventListener("click", event => {
    event.stopPropagation();
    notices.toggle();
  });
  clear.addEventListener("click", event => {
    event.stopPropagation();
    const deleted = remove(STORAGE_KEY);
    if (!deleted.ok) {
      storageFailure("remove", deleted.reason, `通知履歴を消去できません: ${deleted.reason}`);
      return;
    }
    storageRecovery("remove");
    storageRecovery("read");
    storageRecovery("write");
    savingBlocked = false;
    clearRevision += 1;
    entries.length = 0;
    unread = 0;
    changed();
  });
  reload.addEventListener("click", event => {
    event.stopPropagation();
    reloadStored();
  });
  reloadStored();
  updateBadge();
  return notices;
}
