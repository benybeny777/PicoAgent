import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const commands = readFileSync(
  new URL("../../src-tauri/src/commands.rs", import.meta.url),
  "utf8",
);
const coreText = readFileSync(
  new URL("../../crates/picoagent-core/src/text.rs", import.meta.url),
  "utf8",
);

/** JavaScript の文字クラス（`[\u0000-\u0008…]`）を符号位置の集合へ展開する。 */
function jsClassCodePoints(source) {
  const body = source.match(/const DISPLAY_CONTROL_PATTERN\s*=\s*\/\[([^\]]+)\]\/g;/s);
  assert.ok(body, "DISPLAY_CONTROL_PATTERN が見つかりません");
  const points = new Set();
  const pattern = /\\u([0-9A-Fa-f]{4})(?:-\\u([0-9A-Fa-f]{4}))?/g;
  for (const [, from, to] of body[1].matchAll(pattern)) {
    const start = parseInt(from, 16);
    const end = to ? parseInt(to, 16) : start;
    for (let code = start; code <= end; code += 1) points.add(code);
  }
  return points;
}

/** Rust の `is_display_control` が持つ `'\u{…}'..='\u{…}'` を符号位置の集合へ展開する。 */
function rustMatchCodePoints(source) {
  const body = source.match(/fn is_display_control\(c: char\) -> bool \{([\s\S]+?)\n\}/);
  assert.ok(body, "is_display_control が見つかりません");
  const points = new Set();
  const pattern = /'\\u\{([0-9A-Fa-f]+)\}'(?:\s*\.\.=\s*'\\u\{([0-9A-Fa-f]+)\}')?/g;
  for (const [, from, to] of body[1].matchAll(pattern)) {
    const start = parseInt(from, 16);
    const end = to ? parseInt(to, 16) : start;
    for (let code = start; code <= end; code += 1) points.add(code);
  }
  return points;
}

test("画面とRustで、落とす文字の範囲を同じに保つ", () => {
  const js = jsClassCodePoints(renderer);
  const rust = rustMatchCodePoints(coreText);
  // Rust 側はタブ・改行・復帰を `sanitize_input` の分岐で先に通すため、
  // `is_display_control` の範囲には含まれたままになっている。画面側の
  // 文字クラスはその3つだけを外した集合と一致しなければならない。
  const expected = new Set([...rust].filter(code => ![0x09, 0x0a, 0x0d].includes(code)));
  const onlyJs = [...js].filter(code => !expected.has(code));
  const onlyRust = [...expected].filter(code => !js.has(code));
  assert.deepEqual(onlyJs, [], "画面側だけが落としている文字があります");
  assert.deepEqual(onlyRust, [], "Rust側だけが落としている文字があります");
  // 絵文字の結合子と ZWNJ を落とすと絵文字が分解される。どちらにも入れない。
  for (const keep of [0x200c, 0x200d]) {
    assert.ok(!js.has(keep) && !rust.has(keep), `U+${keep.toString(16)} は残す`);
  }
});

test("入力欄の上限は受け口側の上限を超えない", () => {
  const front = renderer.match(/const MAX_INPUT_CHARS = (\d+);/);
  const back = commands.match(/pub const USER_TEXT_MAX_CHARS: usize = ([\d_]+);/);
  assert.ok(front && back, "入力上限の定義が見つかりません");
  const frontMax = Number(front[1]);
  const backMax = Number(back[1].replaceAll("_", ""));
  // 画面側は入力中に知らせるための手前の関門。受け口側は添付本文を前置きした
  // 文字列も受けるため、画面側より緩くなければ添付付きの送信を弾いてしまう。
  assert.ok(frontMax < backMax, `入力欄 ${frontMax} は受け口 ${backMax} より小さくする`);
});

test("入力と表示の両方が共通の正規化を通る", () => {
  // 送信前（入力）。`trim()` だけで受けない。
  assert.match(renderer, /const text = sanitizeText\(input\.value\)\.trim\(\);/);
  // 表示前（モデル出力・ツール出力・履歴）。
  assert.match(renderer, /function renderMarkdown\(container, src\) \{[\s\S]{0,200}src = sanitizeText\(src\);/);
  // 受け口（Tauri command とローカルAPI）。
  assert.match(commands, /let text = accept_user_text\(&text\)\?;/);
  assert.match(commands, /pub fn accept_user_text\(text: &str\) -> Result<String, String> \{\n    let text = crate::text::sanitize_input\(text\);/);
});
