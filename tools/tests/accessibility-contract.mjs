// 画面読み上げへ「何を・いつ知らせるか」の契約を固定する。
// ラベルの有無より、生きた領域（aria-live）の範囲を誤ると実害が大きい。会話パネル全体を
// 生きた領域にすると、履歴の追加・クイック操作の差し替え・通知パネルの開閉まで読み上げられ、
// 肝心の回答が埋もれる。会話パネルの要素は ui/chat/panel.js が作る。
import assert from "node:assert/strict";
import test from "node:test";
import { uiFile } from "./lib/ui-source.mjs";

const panel = uiFile("chat/panel.js");
const character = uiFile("sources/character.js");

/** panel.js の `const <name> = el(...)` 1行を取り出す。 */
function declaration(source, name) {
  const match = source.match(new RegExp(`const ${name} = el\\([^\\n]+`));
  assert.ok(match, `${name} の生成が見つかりません`);
  return match[0];
}

test("生きた領域は回答欄だけに絞る", () => {
  assert.doesNotMatch(declaration(panel, "element"), /aria-live/);
  const reply = declaration(panel, "reply");
  assert.match(reply, /"aria-live": "polite"/);
  // 回答は1つのまとまりとして知らせる（差分だけを読み上げない）。
  assert.match(reply, /"aria-atomic": "true"/);
  // 履歴は replaceChildren で全件を再構築するため、role=log にすると過去分も再通知される。
  const log = declaration(panel, "historyLog");
  assert.doesNotMatch(log, /role: "log"|aria-live/);
  assert.match(log, /"aria-label": "会話履歴"/);
});

test("状態行は短い状態を1回ずつ知らせる", () => {
  const status = declaration(panel, "statusElement");
  assert.match(status, /role: "status"/);
  assert.match(status, /"aria-live": "polite"/);
  assert.match(status, /"aria-atomic": "true"/);
});

test("生成中は回答欄の読み上げを保留する", () => {
  // 生成中は本文が何度も書き変わる。確定するまで読み上げさせない。
  assert.match(panel, /if \(active\) reply\.setAttribute\("aria-busy", "true"\)/);
  // 確定描画と、回答を描かない中断の両方で保留を解く。
  const renderReply = panel.slice(panel.indexOf("renderReply(text, { remember = true }"));
  assert.match(renderReply.slice(0, 300), /reply\.removeAttribute\("aria-busy"\)/);
  const clearThinking = panel.slice(panel.indexOf("clearThinking() {"));
  assert.match(clearThinking.slice(0, 200), /reply\.removeAttribute\("aria-busy"\)/);
});

test("キャラ枠のキー操作は中のボタンを奪わない", () => {
  // キャラ枠は role="button" で、中に ♡（いっしょの記録）ボタンを重ねている。
  // 枠のkeydownでEnter・Spaceを無条件に preventDefault すると、中のボタンを選んで
  // 押したキーまで「キャラに触れる」になり、キーボードだけではそのボタンを押せない。
  const handler = character.match(/element\.addEventListener\("keydown", event => \{[\s\S]*?\n {2}\}\);/)?.[0];
  assert.ok(handler, "キャラ枠のkeydown処理が見つかりません");
  const guard = handler.indexOf("event.target !== element");
  const prevent = handler.indexOf("event.preventDefault()");
  assert.ok(guard >= 0 && guard < prevent, "枠自体に当たったときだけ扱う判定が preventDefault より前にない");
  assert.match(character, /className: "relationship-button icon-button", type: "button"/);
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
  // JavaScript で作る div も同じ（el("div", …, { "aria-label": … }) に role を添える）。
  const { mainModules } = await import("./lib/ui-source.mjs");
  for (const path of mainModules()) {
    for (const line of uiFile(path).split("\n")) {
      if (/el\("div"[^\n]*"aria-label"/.test(line) && !/\brole: "/.test(line)) offenders.push(`${path}: ${line.trim().slice(0, 80)}`);
    }
  }
  assert.deepEqual(offenders, [], "role を付けるか、aria-label をやめてください");
});
