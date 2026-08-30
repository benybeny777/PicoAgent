import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const agent = readFileSync(new URL("../../src-tauri/src/agent.rs", import.meta.url), "utf8");
const prompt = readFileSync(
  new URL("../../src-tauri/src/agent/prompt.rs", import.meta.url),
  "utf8",
);

test("通常会話のユーザー発話へ内部の回答方針を追記しない", () => {
  assert.doesNotMatch(agent, /これは雑談や相談です。Web検索は不要です。/);
  assert.match(prompt, /思考・推論の途中経過は出力せず、結論だけを話してください/);
});

test("短い挨拶の表記揺れはモデルを呼ばず直接返す", () => {
  assert.match(agent, /"こんにちは" \| "こんにちわ" \| "こんちは" \| "こんちわ"/);
  assert.match(agent, /simple_greeting_answer\("こんちわ"/);
});
