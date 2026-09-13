// 回答操作の「読上」と「停止」を同じ枠で入れ替える契約を固定する。
// 読み上げていないときの「停止」は押しても何も起きず、狭い表示形式で他のボタンを縮めていた。
// 入れ替えが崩れると、停止が出ないまま読み上げが止められない／何もしない停止が戻る、のどちらかになる。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");
const [renderer, html, css, spec] = await Promise.all([
  read("ui/renderer.js"),
  read("ui/index.html"),
  read("ui/styles.css"),
  read("SPEC.md"),
]);

test("読み上げていない起動直後は停止を出さず、読上を出す", () => {
  assert.match(html, /<button id="stopVoice"[^>]*\shidden[\s>]/);
  assert.doesNotMatch(html, /<button id="replayReply"[^>]*\shidden[\s>]/);
});

test("読み上げの開始と終了で、読上と停止と読み上げ中止を同じ関数が切り替える", () => {
  const start = renderer.indexOf("function setVoiceCancelVisible(visible) {");
  assert.ok(start >= 0, "setVoiceCancelVisible が見つからない");
  const body = renderer.slice(start, renderer.indexOf("\n}\n", start));
  assert.match(body, /cancelVoiceButton\.hidden = !visible/);
  assert.match(body, /stopVoiceButton\.hidden = !visible/);
  assert.match(body, /replayReplyButton\.hidden = visible/);
  // 押したボタンが隠れても、キーボード操作のフォーカスを入れ替わった側へ渡す。
  assert.match(body, /\.focus\(\)/);
});

test("回答操作ボタンの display 指定が hidden を打ち消さない", () => {
  assert.match(css, /\.reply-actions button\[hidden\]\s*\{\s*display:\s*none;?\s*\}/);
});

test("仕様書の回答操作ラベルと一致する", () => {
  assert.match(spec, /「通知・コピー・再生成・読上・拡大」/);
  assert.doesNotMatch(spec, /「通知・コピー・再生成・読上・停止・拡大」/);
});
