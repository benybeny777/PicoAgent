// 会話の文字サイズとタイトルバー表示の約束。実画面の動きは tools/screenshots/ui-check.mjs が確かめる。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { installFakeDom } from "./lib/fake-dom.mjs";

const read = path => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");

test("会話の文字サイズは共通宣言の範囲を受け取り不正値を丸めず拒否する", async () => {
  const document = installFakeDom();
  const { createFontSizeControl } = await import("../../ui/chat/font-size.js");
  const declarations = read("src-tauri/src/numeric_settings.rs");
  const commands = read("src-tauri/src/commands.rs");
  const metadata = {};
  for (const name of ["BUBBLE_FONT_PX", "STREAM_CHAT_FONT_PX"]) {
    const found = declarations.match(new RegExp(name + ': \\("[^"\\n]*", Some\\((\\d+)\\), (\\d+), (\\d+), "px"\\)'));
    assert.ok(found, name);
    metadata["PICOAGENT_" + name] = { default: found[1], range: [found[2], found[3]], unit: "px" };
  }
  let variant = "bubble";
  const control = createFontSizeControl({ panel: document.createElement("div"), variant: () => variant, apply() {} });
  control.refresh();
  assert.equal(control.toggle.disabled, true, "設定を受け取るまでは操作させない");
  control.setNumberSettings(metadata);
  assert.equal(control.toggle.disabled, true, "宣言だけでは未検証のサイズを生成しない");
  for (const [kind, key] of [["bubble", "PICOAGENT_BUBBLE_FONT_PX"], ["column", "PICOAGENT_STREAM_CHAT_FONT_PX"]]) {
    variant = kind;
    const [min, max] = metadata[key].range.map(Number);
    control.setSize(kind, min);
    assert.equal(control.popover.children[1].disabled, true);
    control.setSize(kind, max);
    assert.equal(control.popover.children[3].disabled, true);
    for (const invalid of [min - 1, max + 1, min + 0.5, "不正"]) assert.throws(() => control.setSize(kind, invalid), /整数/);
    assert.equal(control.popover.children[2].textContent, max + "px", "不正値で公開済み表示を書き換えない");
  }
  assert.match(commands, /fn checked_main_ui_pref[\s\S]*?config::validate_int_patch/);
  const font = read("ui/chat/font-size.js");
  assert.match(font, /bridge\.saveUiPref\(key, String\(sizes\[current\]\)\)/);
  assert.doesNotMatch(font, /settings\.save|settings_save|clampFontPx|fallback/);
  assert.match(read("ui/app/main.js"), /chat\.setNumberSettings\(payload\.numberSettings\)/);
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
  assert.match(main, /chrome\.setPinned\(info\.titleBarPinned === true\)/);
  assert.match(main, /if \(ui\.titleBarPinned != null\) chrome\.setPinned\(ui\.titleBarPinned === true\)/);
  assert.match(main, /bridge\.saveUiPref\("PICOAGENT_TITLE_BAR_PINNED", pinned\)/);
});
