import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { mainScript } from "./lib/ui-source.mjs";

const main = mainScript();
const agent = readFileSync(new URL("../../src-tauri/src/agent.rs", import.meta.url), "utf8");
const prompt = readFileSync(
  new URL("../../src-tauri/src/agent/prompt.rs", import.meta.url),
  "utf8",
);

test("人格の照合元は組み立てた場所で登録する", () => {
  // 呼び出し側へ登録を任せると、増えた経路のどれか1つが忘れた時点で穴が開く。
  assert.match(prompt, /pub fn compose\([\s\S]+?super::remember_private_context\(&prompt\);/);
  assert.match(agent, /pub\(crate\) fn remember_private_context\(prompt: &str\)/);
});

test("公開検査は呼び出し側の文脈だけに頼らない", () => {
  // `guard_public_answer` が登録済みの人格も必ず照合すること。
  assert.match(
    agent,
    /fn guard_public_answer\([\s\S]{0,900}?remembered_private_context\(\)/,
  );
});

test("人格プロンプトの除去を画面側（出口）でやり直さない", () => {
  // 表示・読み上げの直前で削り直す形は、経路が増えるたびに同じ実装を足す必要があり、
  // 実際に何度も漏れを許してきた。入り口（Rust の公開検査）だけが持つ。
  for (const removed of [
    "stripPersonaLeak",
    "personaSpeechBlocklist",
    "setPersonaSpeechBlocklist",
    "normalizeLeakLine",
  ]) {
    assert.ok(!main.includes(removed), `画面側に出口フィルタが残っています: ${removed}`);
  }
});
