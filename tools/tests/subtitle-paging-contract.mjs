import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const styles = readFileSync(new URL("../../ui/styles.css", import.meta.url), "utf8");

test("配信字幕は実DOM寸法で複数ページへ分ける", () => {
  assert.match(renderer, /function subtitlePlainText\(text\)/);
  assert.match(renderer, /sanitizeSpeechText\(text\)/);
  assert.match(renderer, /function buildSubtitlePages\(text\)/);
  assert.match(renderer, /reply\.scrollHeight <= reply\.clientHeight \+ 1/);
  assert.match(renderer, /function startTimedSubtitlePages\(text\)/);
  assert.match(styles, /\[data-layout="stream"\] \.reply[\s\S]*overflow:hidden/);
});

test("音声ありは字幕ページと発話単位を同期し音声なしも時間差表示する", () => {
  assert.match(renderer, /currentLayout === "stream" \? buildSubtitlePages\(text\) : splitSpeechChunks\(text\)/);
  assert.match(renderer, /showSubtitlePage\(chunks\[i\]\)/);
  assert.match(renderer, /voiceVolume <= 0\.001\) startTimedSubtitlePages\(result\.reply\)/);
});

test("配信画面でも処理中表示を隠さない", () => {
  assert.match(styles, /\[data-layout="stream"\] \.status\.busy,[\s\S]*?\[data-layout="stream"\] \.status\.engine-error\s*\{[\s\S]*?display:flex/);
  assert.match(styles, /\.status\.engine-pending/);
  assert.match(styles, /\.status\.engine-error/);
});

test("2Dキャラは小さい合成面をtransformで拡大しない", () => {
  assert.match(styles, /\.character\[data-render-kind="2d"\] \.character-view \{[\s\S]*?width:var\(--char-render-size/);
  const rule = styles.match(/\.character\[data-render-kind="2d"\] \.character-view \{([\s\S]*?)\}/)?.[1] || "";
  assert.doesNotMatch(rule, /scale\(/);
  assert.match(renderer, /character\.style\.setProperty\("--char-render-size", `\$\{viewZoom \* 100\}%`\)/);
});
