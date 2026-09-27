// 裏で繰り返し確認している機能（目覚まし・リマインダー）と、利用者が「できている」と
// 思い込む機能（会話履歴の保存）が失敗したときに、画面へ理由が出ることを固定する。
// ログだけで済ませると、利用者には「登録したのに動かない」結果しか残らない。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { installFakeDom } from "./lib/fake-dom.mjs";
import { uiFile } from "./lib/ui-source.mjs";

const document = installFakeDom();
globalThis.picoagent = { log: () => Promise.resolve() };
const { notifyBackgroundFailure, clearBackgroundFailure } = await import("../../ui/app/toast.js");
const commands = await readFile(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
const ambient = uiFile("app/ambient.js");
const notice = uiFile("app/notice.js");
const main = uiFile("app/main.js");
const character = uiFile("character/index.js");

const errorToasts = () => document.body.children.filter(node => node.classSet?.().has("toast-error")).length;

test("同じ原因の失敗は1度だけ知らせ、原因が違えばそれぞれ知らせる", () => {
  const before = errorToasts();
  notifyBackgroundFailure("test-a", "目覚ましが鳴りません", "boom");
  notifyBackgroundFailure("test-a", "目覚ましが鳴りません", "boom");
  notifyBackgroundFailure("test-a", "目覚ましが鳴りません", "boom");
  assert.equal(errorToasts() - before, 1, "20秒ごとに同じ通知を出している");
  notifyBackgroundFailure("test-b", "リマインダーが出ません", "boom");
  assert.equal(errorToasts() - before, 2);
});

test("復旧したら次の失敗はまた知らせる", () => {
  const before = errorToasts();
  notifyBackgroundFailure("test-c", "目覚ましが鳴りません", "boom");
  clearBackgroundFailure("test-c");
  notifyBackgroundFailure("test-c", "目覚ましが鳴りません", "boom");
  assert.equal(errorToasts() - before, 2, "復旧後の再発を黙って握りつぶしている");
});

test("目覚まし・リマインダー・履歴保存の失敗が画面へ出る", () => {
  // 取得できた回に印を落とさないと、一度失敗したあとは復旧しても知らせられない。
  for (const key of ["alarms", "daily-events"]) {
    assert.ok(ambient.includes(`notifyBackgroundFailure("${key}",`), `${key} の失敗を画面へ出していません`);
    assert.ok(ambient.includes(`clearBackgroundFailure("${key}")`), `${key} の復旧で印を落としていません`);
  }
  assert.match(main, /info\.historySaveBlocked === true/, "履歴の保存停止を画面へ出していません");
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
  assert.match(notice, /if \(kind === "background_failure"\) \{[\s\S]*?clearBackgroundFailure\(key\);[\s\S]*?notifyBackgroundFailure\(key,[\s\S]*?return;/);
});

test("GPU描画の失敗だけで描画キャッシュ削除を予約する", () => {
  // 描画キャッシュはふだん残す（毎回消すと初回描画がもたつく）。GPU由来の失敗だけで消す。
  const pattern = character.match(/const GPU_FAILURE_PATTERN = (\/.*\/[a-z]*);/)?.[1];
  assert.ok(pattern, "GPU_FAILURE_PATTERN が見つかりません");
  const regex = Function(`return ${pattern};`)();
  assert.equal(regex.test("キャラ定義: HTTP 404"), false, "素材不足では消さない");
  assert.equal(regex.test("WebGLを初期化できません"), true);
  const report = character.match(/function reportGpuFailure\(reason, force = false\) \{[\s\S]*?\n {2}\}/)?.[0] || "";
  // 1回の起動で予約は1度だけ。
  assert.match(report, /if \(gpuResetRequested \|\| \(!force && !GPU_FAILURE_PATTERN\.test\(text\)\)\) return;\s*\n\s*gpuResetRequested = true;/);
  assert.match(character, /document\.addEventListener\("webglcontextlost", event => \{\s*reportGpuFailure\("WebGL context lost", true\);/);
});
