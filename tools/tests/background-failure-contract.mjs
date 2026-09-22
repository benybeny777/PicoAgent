// 裏で繰り返し確認している機能（目覚まし・リマインダー）と、利用者が「できている」と
// 思い込む機能（会話履歴の保存）が失敗したときに、画面へ理由が出ることを固定する。
// ログだけで済ませると、利用者には「登録したのに動かない」結果しか残らない。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const commands = await readFile(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");

const notifierSource = renderer.match(
  /const notifiedBackgroundFailures = new Set\(\);[\s\S]*?\nfunction clearBackgroundFailure\(key\) \{[\s\S]*?\n\}/,
)?.[0];
assert.ok(notifierSource, "notifyBackgroundFailure の正本が見つかりません");

function makeNotifier() {
  const toasts = [];
  const logs = [];
  const build = Function(
    "logApp",
    "showToast",
    "errorDetail",
    `"use strict"; ${notifierSource}; return { notifyBackgroundFailure, clearBackgroundFailure };`,
  );
  const api = build(
    (level, message) => logs.push(`${level}:${message}`),
    (message, kind) => toasts.push(`${kind}:${message}`),
    error => String(error),
  );
  return { ...api, toasts, logs };
}

test("同じ原因の失敗は1度だけ知らせる", () => {
  const n = makeNotifier();
  n.notifyBackgroundFailure("alarms", "目覚ましが鳴りません", "boom");
  n.notifyBackgroundFailure("alarms", "目覚ましが鳴りません", "boom");
  n.notifyBackgroundFailure("alarms", "目覚ましが鳴りません", "boom");
  assert.equal(n.toasts.length, 1, "20秒ごとに同じ通知を出している");
  assert.match(n.toasts[0], /^error:/, "失敗はエラー表示にする");
  assert.equal(n.logs.length, 3, "ログは毎回残す（原因追跡のため）");
});

test("原因が違えばそれぞれ知らせる", () => {
  const n = makeNotifier();
  n.notifyBackgroundFailure("alarms", "目覚ましが鳴りません", "boom");
  n.notifyBackgroundFailure("daily-events", "リマインダーが出ません", "boom");
  assert.equal(n.toasts.length, 2);
});

test("復旧したら次の失敗はまた知らせる", () => {
  const n = makeNotifier();
  n.notifyBackgroundFailure("alarms", "目覚ましが鳴りません", "boom");
  n.clearBackgroundFailure("alarms");
  n.notifyBackgroundFailure("alarms", "目覚ましが鳴りません", "boom");
  assert.equal(n.toasts.length, 2, "復旧後の再発を黙って握りつぶしている");
});

test("目覚まし・リマインダー・履歴保存の失敗が画面へ出る", () => {
  // 取得できた回に印を落とさないと、一度失敗したあとは復旧しても知らせられない。
  for (const key of ["alarms", "daily-events"]) {
    assert.ok(
      renderer.includes(`notifyBackgroundFailure(\n      "${key}",`),
      `${key} の失敗を画面へ出していません`,
    );
    assert.ok(
      renderer.includes(`clearBackgroundFailure("${key}")`),
      `${key} の復旧で印を落としていません`,
    );
  }
  assert.match(renderer, /info\.historySaveBlocked === true/, "履歴の保存停止を画面へ出していません");
  assert.match(commands, /"historySaveBlocked": crate::history::saving_blocked/, "保存停止の状態を返していません");
});

test("履歴の保存失敗と保存停止は会話中も画面へ届く", async () => {
  // 起動時の案内だけでは、会話中に書けなくなった場合（容量不足・権限・切替後の保護）に
  // 何も出ない。保存できていないまま会話を続け、再起動でその分が消える。
  const history = await readFile(new URL("../../src-tauri/src/history.rs", import.meta.url), "utf8");
  assert.match(history, /"kind": "background_failure",\s*\n\s*"key": "history-save",/);
  assert.match(history, /"kind": "background_failure",\s*\n\s*"key": "history-save-blocked",/);
  // 失敗と復旧は状態が変わったときだけ送る（毎ターン同じ通知を流さない）。
  assert.match(history, /if !SAVE_FAILED\.swap\(true, Ordering::Relaxed\)/);
  assert.match(history, /if SAVE_FAILED\.swap\(false, Ordering::Relaxed\)/);
  // 画面側は通知一覧へ積まず、原因ごとに1度だけ知らせる経路へ回す。
  assert.match(renderer, /if \(String\(notice\?\.kind \|\| ""\) === "background_failure"\) \{[\s\S]*?clearBackgroundFailure\(key\); return;[\s\S]*?notifyBackgroundFailure\(key,/);
});

test("GPU描画の失敗だけで描画キャッシュ削除を予約する", () => {
  // 描画キャッシュはふだん残す（毎回消すと初回描画がもたつく）。GPU由来の失敗だけで消す。
  const source = renderer.match(/const GPU_FAILURE_PATTERN = [\s\S]*?\n}\n/)?.[0];
  assert.ok(source, "reportGpuFailure の正本が見つかりません");
  const calls = [];
  const build = Function("window", "errorMessage", "logApp", "errorDetail",
    `"use strict"; ${source}; return reportGpuFailure;`);
  const report = build(
    { picoagent: { requestGpuCacheReset: reason => { calls.push(reason); return Promise.resolve(); } } },
    error => error?.message || String(error), () => {}, String,
  );
  report(new Error("キャラ定義: HTTP 404"));
  assert.deepEqual(calls, [], "素材不足では消さない");
  report(new Error("WebGLを初期化できません"));
  report(new Error("WebGL context lost"));
  assert.deepEqual(calls, ["WebGLを初期化できません"], "1回の起動で予約は1度だけ");
  assert.match(renderer, /document\.addEventListener\("webglcontextlost", event => \{\s*reportGpuFailure\("WebGL context lost", \{ force: true \}\);/);
});
