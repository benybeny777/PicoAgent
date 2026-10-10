// 配信字幕のページ分けと、読み上げとの同期、選択禁止の契約。
import assert from "node:assert/strict";
import test from "node:test";
import { uiFile } from "./lib/ui-source.mjs";

const pager = uiFile("chat/pager.js");
const panel = uiFile("chat/panel.js");
const voice = uiFile("app/voice.js");
const subtitle = uiFile("sources/subtitle.js");
const stream = uiFile("components/stream.css");
const chatCss = uiFile("components/chat.css");

test("配信字幕は実DOM寸法で複数ページへ分ける", () => {
  assert.match(pager, /plainText\(text, sanitizeSpeechText\)/);
  assert.match(pager, /function pages\(text\)/);
  assert.match(pager, /function timed\(text\)/);
  // 合否は字幕欄そのものの実寸（固定文字数では幅や文字種で欠ける）。
  assert.match(panel, /return reply\.scrollHeight <= reply\.clientHeight \+ 1;/);
  assert.match(stream, /\.subtitle \.reply \{[\s\S]*?overflow: hidden;/);
});

test("音声ありは字幕ページと発話単位を同期し音声なしも時間差表示する", () => {
  assert.match(voice, /const chunks = pager\.paginates\(\) \? pager\.pages\(text\) : splitSpeechChunks\(text\);/);
  // 音声なし（設定不正・無効・音量0）は合成を待たず時間差で送る。
  assert.match(voice, /if \(volumeError \|\| enabled === false \|\| volume <= 0\.001\) \{\s*stop\(\);\s*pager\.timed\(text\);/);
});

test("字幕は子要素まで選択できず、入り込んだ選択も解除する", () => {
  assert.match(stream, /\.subtitle \.reply, \.subtitle \.reply \* \{ user-select: none; -webkit-user-select: none; \}/);
  assert.match(stream, /\.subtitle::selection, \.subtitle \*::selection \{ color: inherit; background: transparent; \}/);
  assert.match(subtitle, /element\.addEventListener\("selectstart", event => event\.preventDefault\(\)\)/);
  assert.match(subtitle, /document\.addEventListener\("selectionchange", \(\) => \{[\s\S]*?intersectsNode\(element\)[\s\S]*?removeAllRanges\(\)/);
});

test("配信画面でも処理中表示を隠さない", () => {
  // 状態行の見た目は形式で分岐しない（配信だけ消す指定を持たない）。
  assert.match(chatCss, /\.status\.busy, \.status\.engine-pending \{ display: flex;/);
  assert.doesNotMatch(stream, /\.status[^{]*\{[^}]*display: none/);
});
