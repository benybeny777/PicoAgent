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

test("生きた領域は回答欄だけに絞る", () => {
  assert.doesNotMatch(tagOf("bubble"), /aria-live/);
  const replyTag = tagOf("reply");
  assert.match(replyTag, /aria-live="polite"/);
  // 回答は1つのまとまりとして知らせる（差分だけを読み上げない）。
  assert.match(replyTag, /aria-atomic="true"/);
  // 履歴は replaceChildren で全件を再構築するため、role=log にすると過去分も再通知される。
  assert.doesNotMatch(tagOf("historyLog"), /role="log"|aria-live/);
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
  assert.doesNotMatch(renderer, /if \(active\)[^\n]+\n\s*else reply\.removeAttribute\("aria-busy"\)/);
  // 確定描画で保留を解く（回答が出そろった時点で1回だけ知らせる）。
  const renderReply = renderer.slice(renderer.indexOf("function renderReply(text)"));
  assert.match(renderReply.slice(0, 400), /reply\.removeAttribute\("aria-busy"\)/);
  const clearThinkingReply = renderer.slice(renderer.indexOf("function clearThinkingReply()"));
  assert.match(clearThinkingReply.slice(0, 400), /reply\.removeAttribute\("aria-busy"\)/);
});

test("立ち絵枠のキー操作は中のボタンを奪わない", () => {
  // `#character` は role="button" の枠で、中に ♡（いっしょの記録）ボタンを重ねている。
  // 枠のkeydownでEnter・Spaceを無条件に preventDefault すると、中のボタンを選んで
  // 押したキーまで「キャラに触れる」になり、キーボードだけではそのボタンを押せない。
  const handler = renderer.match(/character\.addEventListener\("keydown", event => \{[\s\S]*?\n\}\);/)?.[0];
  assert.ok(handler, "立ち絵枠のkeydown処理が見つかりません");
  const guard = handler.indexOf("event.target !== character");
  const prevent = handler.indexOf("event.preventDefault()");
  assert.ok(guard >= 0 && guard < prevent, "枠自体に当たったときだけ扱う判定が preventDefault より前にない");
  assert.match(html, /<button id="relationshipButton"[^>]*type="button"/);
});

test("名前を付けた入れ物には役割も付ける", async () => {
  // `<div>` の既定の役割（generic）は名前を持てない。`aria-label` だけを書いても
  // 画面読み上げには一切届かず、付けたつもりの見出しが黙って消えていた。
  const { readdir, readFile } = await import("node:fs/promises");
  const uiDir = new URL("../../ui/", import.meta.url);
  const pages = (await readdir(uiDir)).filter(name => name.endsWith(".html"));
  const offenders = [];
  for (const page of pages) {
    const body = await readFile(new URL(page, uiDir), "utf8");
    for (const match of body.matchAll(/<div\b[^>]*aria-label="([^"]+)"[^>]*>/g)) {
      if (!/\brole="/.test(match[0])) offenders.push(`${page}: ${match[1]}`);
    }
  }
  assert.deepEqual(offenders, [], "role を付けるか、aria-label をやめてください");
});
