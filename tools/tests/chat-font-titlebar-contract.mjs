// 会話の文字サイズとタイトルバー表示の約束。実画面の動きは tools/screenshots/ui-check.mjs が確かめる。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = path => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");

test("会話の文字サイズの範囲は画面とバックエンドの検証で一致する", () => {
  const font = read("ui/chat/font-size.js");
  const commands = read("src-tauri/src/commands.rs");
  assert.match(font, /bubble: \{ key: "PICOAGENT_BUBBLE_FONT_PX", min: 10, max: 28,/);
  assert.match(font, /column: \{ key: "PICOAGENT_STREAM_CHAT_FONT_PX", min: 18, max: 48,/);
  assert.match(commands, /"PICOAGENT_BUBBLE_FONT_PX" => value[\s\S]*?\(10\.\.=28\)\.contains/);
  assert.match(commands, /\(18\.\.=48\)\.contains/);
  // 保存はメイン画面専用の狭い入口だけを通す（設定画面の保存処理を流用しない）。
  assert.match(font, /bridge\.saveUiPref\(key, String\(sizes\[current\]\)\)/);
  assert.doesNotMatch(font, /settings\.save|settings_save/);
});

test("タイトルバーは見えているシーンへ吸着し、隠しても操作要素の位置を変えない", () => {
  const stage = read("ui/app/stage.js");
  const css = read("ui/components/window.css");
  for (const name of ["--scene-view-left", "--scene-view-top", "--scene-view-width"]) {
    assert.match(stage, new RegExp(`setProperty\\("${name}"`));
    assert.match(css, new RegExp(`var\\(${name}`));
  }
  const hidden = css.match(/\.app\.title-bar-floating:not\(\.title-bar-shown\) \.top-window-menu \{([^}]*)\}/)?.[1] || "";
  assert.match(hidden, /opacity: 0/);
  assert.match(hidden, /pointer-events: none/);
  assert.doesNotMatch(hidden, /translate|transform|visibility|display/);
  assert.match(css, /\.app\.title-bar-floating \{ --stage-top: 0px; \}/);
});

test("タイトルバーの常に表示は保存値を起動・設定反映の両方で当てる", () => {
  const main = read("ui/app/main.js");
  const commands = read("src-tauri/src/commands.rs");
  assert.match(main, /chrome\.setPinned\(info\.titleBarPinned === true\)/);
  assert.match(main, /if \(ui\.titleBarPinned != null\) chrome\.setPinned\(ui\.titleBarPinned === true\)/);
  assert.match(main, /bridge\.saveUiPref\("PICOAGENT_TITLE_BAR_PINNED", pinned\)/);
  assert.equal(commands.match(/"titleBarPinned": state\.config\.get_bool\("PICOAGENT_TITLE_BAR_PINNED", false\)/g)?.length, 2);
});
