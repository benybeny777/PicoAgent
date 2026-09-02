// 画面読み上げへ「何を・いつ知らせるか」の契約を固定する。
// ラベルの有無より、生きた領域（aria-live）の範囲を誤ると実害が大きい。フキダシ全体を
// 生きた領域にすると、履歴の追加・クイック操作の差し替え・通知パネルの開閉まで読み上げられ、
// 肝心の回答が埋もれる。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const html = readFileSync(new URL("../../ui/index.html", import.meta.url), "utf8");
const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");

function tagOf(id) {
  const match = html.match(new RegExp(`<[a-z]+ id="${id}"[^>]*>`));
  assert.ok(match, `#${id} が ui/index.html にありません`);
  return match[0];
}

test("生きた領域は回答欄と会話履歴だけに絞る", () => {
  assert.doesNotMatch(tagOf("bubble"), /aria-live/);
  const replyTag = tagOf("reply");
  assert.match(replyTag, /aria-live="polite"/);
  // 回答は1つのまとまりとして知らせる（差分だけを読み上げない）。
  assert.match(replyTag, /aria-atomic="true"/);
  assert.match(tagOf("historyLog"), /role="log"/);
  assert.match(tagOf("historyLog"), /aria-label="会話履歴"/);
});

test("状態行は短い状態を1回ずつ知らせる", () => {
  const statusTag = tagOf("status");
  assert.match(statusTag, /role="status"/);
  assert.match(statusTag, /aria-live="polite"/);
  assert.match(statusTag, /aria-atomic="true"/);
});

test("生成中は回答欄の読み上げを保留する", () => {
  // 生成中は本文が何度も書き変わる。確定するまで読み上げさせない。
  assert.match(renderer, /if \(active\) reply\.setAttribute\("aria-busy", "true"\)/);
  assert.match(renderer, /else reply\.removeAttribute\("aria-busy"\)/);
  // 確定描画で保留を解く（回答が出そろった時点で1回だけ知らせる）。
  const renderReply = renderer.slice(renderer.indexOf("function renderReply(text)"));
  assert.match(renderReply.slice(0, 400), /reply\.removeAttribute\("aria-busy"\)/);
});
