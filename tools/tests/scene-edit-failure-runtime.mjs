// 配置操作の失敗記録と、編集枠の後処理を実関数で固定する。
import assert from "node:assert/strict";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";

test("配置操作が失敗しても理由を記録し編集枠を再描画する", async () => {
  const source = uiFile("app/edit.js");
  const start = source.indexOf("  async function runOperation(");
  const end = source.indexOf("  async function commit(", start);
  assert.ok(start >= 0 && end > start);
  for (const fails of [true, false]) {
    const logs = [];
    let renders = 0;
    const run = runInNewContext(`${source.slice(start, end)}\nrunOperation`, {
      logApp: (...args) => logs.push(args), errorDetail: error => error.message,
      render: () => { renders += 1; },
    });
    await run(async () => { if (fails) throw new Error("書き込みを拒否しました"); }, "配置保存");
    assert.equal(renders, 1);
    assert.equal(logs.length, fails ? 1 : 0);
    if (fails) assert.deepEqual(logs[0], ["error", "配置保存", "書き込みを拒否しました"]);
  }
});
