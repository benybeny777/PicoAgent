// 回答操作の「読上」と「停止」を同じ枠で入れ替える契約を固定する。
// 読み上げていないときの「停止」は押しても何も起きず、狭い表示形式で他のボタンを縮めていた。
// 入れ替えが崩れると、停止が出ないまま読み上げが止められない／何もしない停止が戻る、のどちらかになる。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { uiFile } from "./lib/ui-source.mjs";

const panel = uiFile("chat/panel.js");
const base = uiFile("components/base.css");
const spec = await readFile(new URL("../../SPEC.md", import.meta.url), "utf8");

test("読み上げていない起動直後は停止を出さず、読上を出す", () => {
  assert.match(panel, /const stopButton = el\("button", \{[^\n]*hidden: true/);
  assert.doesNotMatch(panel.match(/const replayButton = el\([^\n]+/)[0], /hidden: true/);
});

test("読み上げの開始と終了で、読上と停止と読み上げ中止を同じ関数が切り替える", () => {
  const start = panel.indexOf("setVoiceActive(active) {");
  assert.ok(start >= 0, "setVoiceActive が見つからない");
  const body = panel.slice(start, panel.indexOf("\n    },", start));
  assert.match(body, /cancelVoiceButton\.hidden = !active/);
  assert.match(body, /stopButton\.hidden = !active/);
  assert.match(body, /replayButton\.hidden = active/);
  // 押したボタンが隠れても、キーボード操作のフォーカスを入れ替わった側へ渡す。
  assert.match(body, /\.focus\(\)/);
});

test("ボタンの display 指定が hidden を打ち消さない", () => {
  // 部品側で display を書いても、[hidden] の打ち消しが必ず勝つ（全画面で唯一の !important）。
  assert.match(base, /\[hidden\]\s*\{\s*display:\s*none\s*!important;?\s*\}/);
});

test("仕様書の回答操作ラベルと一致する", () => {
  assert.match(spec, /「通知・コピー・再生成・読上・拡大」/);
  assert.doesNotMatch(spec, /「通知・コピー・再生成・読上・停止・拡大」/);
});
