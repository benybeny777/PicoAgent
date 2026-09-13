// 実行中タスク一覧の状態遷移と、画面・送信処理への配線を固定する。
// 表示が崩れても例外にならず「動いているのか分からない」だけになるため、状態を直接検査する。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

await import("../../ui/task-run.js");

const TaskRun = globalThis.PicoTaskRun;
const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");
const [renderer, html, agent, styles] = await Promise.all([
  read("ui/renderer.js"),
  read("ui/index.html"),
  read("src-tauri/src/agent.rs"),
  read("ui/styles.css"),
]);

const summary = run => run.rows.map(row => [row.label, row.state]);

test("送信直後は考え中の行だけを実行中にする", () => {
  const run = TaskRun.createRun(1000);
  assert.equal(run.state, "running");
  assert.deepEqual(summary(run), [["考え中", "running"]]);
});

test("進捗文言は考え中の行の名前にし、ツール実行中の文言では行を増やさない", () => {
  const run = TaskRun.createRun(0);
  TaskRun.applyStatus(run, "計画を立てています…", 10);
  TaskRun.applyStatus(run, "ツールを実行中: web_search", 20);
  assert.deepEqual(summary(run), [["計画を立てています", "running"]]);
});

test("ツール開始で考え中を完了にし、表示名で実行中の行を足す", () => {
  const run = TaskRun.createRun(0);
  TaskRun.applyActivity(run, { phase: "started", tools: ["web_search", "comfyui_generate"], labels: ["Web検索", "ローカル画像・動画生成"] }, 2000);
  assert.deepEqual(summary(run), [["考え中", "completed"], ["Web検索", "running"], ["ローカル画像・動画生成", "running"]]);
  assert.equal(TaskRun.rowElapsed(run.rows[0], 99_999), 2000);
  assert.equal(TaskRun.rowElapsed(run.rows[1], 5000), 3000);
});

test("表示名が届かないツールは内部名で出す", () => {
  const run = TaskRun.createRun(0);
  TaskRun.applyActivity(run, { phase: "started", tools: ["web_fetch"] }, 10);
  assert.equal(run.rows[1].label, "web_fetch");
});

test("失敗したツールだけを失敗にし、結果の確認を始める", () => {
  const run = TaskRun.createRun(0);
  TaskRun.applyActivity(run, { phase: "started", tools: ["web_search", "web_fetch"], labels: ["Web検索", "Webページ取得"] }, 100);
  TaskRun.applyActivity(run, { phase: "completed", tools: ["web_search", "web_fetch"], failedTools: ["web_fetch"] }, 700);
  assert.deepEqual(summary(run).slice(1), [["Web検索", "completed"], ["Webページ取得", "failed"], ["結果を確認中", "running"]]);
});

test("すぐ終わった結果確認の行は完了時に残さない", () => {
  const run = TaskRun.createRun(0);
  TaskRun.applyActivity(run, { phase: "started", tools: ["comfyui_generate"], labels: ["ローカル画像・動画生成"] }, 100);
  TaskRun.applyActivity(run, { phase: "completed", tools: ["comfyui_generate"] }, 540_000);
  TaskRun.finishRun(run, "completed", 540_200);
  assert.equal(run.state, "completed");
  assert.deepEqual(summary(run), [["考え中", "completed"], ["ローカル画像・動画生成", "completed"]]);
});

test("考え直した結果確認の行は完了として残す", () => {
  const run = TaskRun.createRun(0);
  TaskRun.applyActivity(run, { phase: "started", tools: ["web_search"], labels: ["Web検索"] }, 100);
  TaskRun.applyActivity(run, { phase: "completed", tools: ["web_search"] }, 900);
  TaskRun.applyStatus(run, "考え直しています…", 950);
  TaskRun.finishRun(run, "completed", 1000);
  assert.deepEqual(summary(run).at(-1), ["考え直しています", "completed"]);
});

test("停止した会話は実行中の行を中断にする", () => {
  const run = TaskRun.createRun(0);
  TaskRun.applyActivity(run, { phase: "started", tools: ["comfyui_generate"], labels: ["ローカル画像・動画生成"] }, 100);
  TaskRun.finishRun(run, "cancelled", 5000);
  assert.equal(run.state, "cancelled");
  assert.deepEqual(summary(run).at(-1), ["ローカル画像・動画生成", "cancelled"]);
});

test("返事待ちのツールは実行中の行を増やさず、終了時に閉じる", () => {
  const run = TaskRun.createRun(0);
  TaskRun.applyActivity(run, { phase: "started", tools: ["ask_user"], labels: ["質問"] }, 100);
  TaskRun.applyActivity(run, { phase: "waiting", tools: ["ask_user"] }, 200);
  assert.deepEqual(summary(run).at(-1), ["質問", "waiting"]);
  TaskRun.finishRun(run, "completed", 300);
  assert.deepEqual(summary(run).at(-1), ["質問", "completed"]);
});

test("終了後に届いた進捗やイベントは一覧を書き換えない", () => {
  const run = TaskRun.createRun(0);
  TaskRun.finishRun(run, "failed", 100);
  const before = JSON.stringify(run);
  TaskRun.applyStatus(run, "考え直しています…", 200);
  TaskRun.applyActivity(run, { phase: "started", tools: ["web_search"] }, 200);
  TaskRun.finishRun(run, "completed", 300);
  assert.equal(JSON.stringify(run), before);
  assert.deepEqual(summary(run), [["考え中", "failed"]]);
});

test("経過時間を秒・分・時間で読みやすく出す", () => {
  assert.equal(TaskRun.formatElapsed(0), "0秒");
  assert.equal(TaskRun.formatElapsed(59_999), "59秒");
  assert.equal(TaskRun.formatElapsed(65_000), "1分05秒");
  assert.equal(TaskRun.formatElapsed(3_725_000), "1時間02分");
  assert.equal(TaskRun.formatElapsed(Number.NaN), "0秒");
  assert.equal(TaskRun.formatElapsed(-5), "0秒");
});

test("行数が上限を超えたら実行中を残して古い終了行から捨てる", () => {
  const run = TaskRun.createRun(0);
  for (let index = 0; index < TaskRun.MAX_ROWS + 5; index += 1) {
    TaskRun.applyActivity(run, { phase: "started", tools: [`tool_${index}`] }, index * 10);
    TaskRun.applyActivity(run, { phase: "completed", tools: [`tool_${index}`] }, index * 10 + 5);
  }
  TaskRun.applyActivity(run, { phase: "started", tools: ["last_tool"] }, 1000);
  assert.ok(run.rows.length <= TaskRun.MAX_ROWS);
  assert.deepEqual(summary(run).at(-1), ["last_tool", "running"]);
  assert.ok(!run.rows.some(row => row.label === "考え中"));
});

test("経過時間のタイマーは終了・再開始・破棄のどの経路でも止める", () => {
  const active = new Map();
  let nextId = 1;
  const timers = {
    setInterval: () => { const id = nextId++; active.set(id, "interval"); return id; },
    clearInterval: id => active.delete(id),
    setTimeout: () => { const id = nextId++; active.set(id, "timeout"); return id; },
    clearTimeout: id => active.delete(id),
  };
  const controller = TaskRun.createController({ container: null, timers, now: () => 0 });
  controller.start();
  assert.deepEqual([...active.values()], ["interval"]);
  controller.start();
  assert.deepEqual([...active.values()], ["interval"], "再開始で前の更新タイマーを残さない");
  controller.finish("completed");
  assert.deepEqual([...active.values()], ["timeout"], "終了したら更新タイマーを止め、閉じるタイマーだけにする");
  controller.start();
  assert.deepEqual([...active.values()], ["interval"], "再開始で閉じるタイマーを取り消す");
  controller.dispose();
  assert.equal(active.size, 0);
});

test("送信処理と画面が実行中タスクへ状態を渡す", () => {
  assert.match(html, /<div id="taskRun" class="task-run"[^>]*hidden><\/div>/);
  assert.ok(html.indexOf('src="task-run.js"') > 0 && html.indexOf('src="task-run.js"') < html.indexOf('src="renderer.js"'));
  assert.match(renderer, /globalThis\.PicoTaskRun\.createController\(/);
  assert.match(renderer, /taskRun\.start\(\);/);
  assert.match(renderer, /taskRun\.status\(progress\);/);
  assert.match(renderer, /taskRun\.activity\(activity\);/);
  assert.match(renderer, /taskRun\.finish\(jobFailed \? "failed" : jobCancelled \? "cancelled" : "completed"\);/);
  assert.match(renderer, /closest\("button,a,textarea,\.reply-actions,\.task-run"\)/);
  assert.match(agent, /"labels": tools\.iter\(\)\.map\(\|name\| crate::tools::label_of\(name\)\)/);
  assert.match(styles, /\.bubble\.task-run-visible \.status\.busy \{ display:none; \}/);
  // フキダシ内のボタンは既定で絶対配置されるため、見出しに留めるには static の明示が要る。
  assert.match(styles, /\.task-run-cancel \{ position:static;/);
  assert.match(styles, /\.bubble\.task-run-visible \.cancel-send \{ display:none; \}/);
});
