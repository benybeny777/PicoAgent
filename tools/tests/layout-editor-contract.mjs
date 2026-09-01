// 配置編集（OBS風）の保存先と安全域の契約を、実DOMを起こさずに固定する。
// 表示形式・キャラクターの切替は属性変更の「後」に監視が走るため、保存先を取り違えると
// 別の形式・別キャラへ位置と倍率が漏れる。画面を撮らずに気づけるようにここで押さえる。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const editor = readFileSync(new URL("../../ui/layout-editor.js", import.meta.url), "utf8");
const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const bridge = readFileSync(new URL("../../ui/picoagent.js", import.meta.url), "utf8");
const commands = readFileSync(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");

test("配置は設定へ保存し、旧localStorageからは1度だけ移行する", () => {
  // 正本は設定（PICOAGENT_LAYOUT_PLACEMENTS）。webviewのlocalStorageに置くと
  // WebViewデータの初期化・再インストール・別PCへの引き継ぎで消える。
  assert.match(commands, /pub fn pet_save_layout_placements[\s\S]*?layout_placements::normalize_checked/);
  assert.match(commands, /"layoutPlacements": crate::layout_placements::normalize/);
  assert.match(bridge, /saveLayoutPlacements: \(placements\) => call\("pet_save_layout_placements", \{ placements \}\)/);
  assert.match(renderer, /window\.PicoLayoutEditor\?\.hydrate\(window\.__picoLayoutPlacements\)/);
  assert.match(editor, /const persist = \(\) => \{[\s\S]*?saveLayoutPlacements/);
  assert.match(editor, /LEGACY_MIGRATED_KEY/);
  // 確定時だけ保存する（ドラッグのたびに設定ファイルを書かない）。
  const dragHandler = editor.slice(editor.indexOf('moveable.on("drag"'), editor.indexOf('moveable.on("dragEnd"'));
  const scaleHandler = editor.slice(editor.indexOf('moveable.on("scale"'), editor.indexOf('moveable.on("scaleEnd"'));
  assert.doesNotMatch(dragHandler, /persist\(\)/);
  assert.doesNotMatch(scaleHandler, /persist\(\)/);
  assert.doesNotMatch(editor, /localStorage\.setItem\(LEGACY_STORAGE_KEY/);
});

test("キャラの位置と倍率はキャラごと・表示形式ごとに保存する", () => {
  assert.match(renderer, /character\.dataset\.characterId = manifest\.id/);
  assert.match(editor, /const characterId = \(\) => sources\.character\?\.dataset\.characterId \|\| ""/);
  assert.match(editor, /store\.characters\[id\]\[layout\] = characterState/);
  assert.match(editor, /const savedCharacter = characterId\(\) && store\.characters\?\.\[characterId\(\)\]\?\.\[layout\]/);
});

test("切替後の監視は切替前の保存先へ書く", () => {
  assert.match(editor, /const save = \(layout = layoutId\(\), id = characterId\(\)\) =>/);
  const layoutObserver = editor.slice(editor.indexOf("const layoutObserver"));
  assert.match(layoutObserver, /if \(editing\) saveAndLeave\(editingLayout, editingCharacter\)/);
  const characterObserver = editor.slice(editor.indexOf("const characterObserver"));
  assert.match(characterObserver, /if \(editing\) saveAndLeave\(editingLayout, editingCharacter\)/);
  // 直接購読すると保存先へ Event が渡るため、必ず包んで呼ぶ。
  assert.doesNotMatch(editor, /addEventListener\("click", saveAndLeave\)/);
  assert.doesNotMatch(editor, /addEventListener\("click", save\)/);
});

test("キャラ要素が無い画面でも初期化を最後まで通す", () => {
  assert.match(editor, /if \(sources\.character\) \{\s*\n\s*characterObserver\.observe\(sources\.character/);
});

test("縮小しすぎと画面外への消失を防ぐ", () => {
  assert.match(editor, /const MIN_SCALE = 0\.25/);
  assert.match(editor, /const MAX_SCALE = 4/);
  assert.match(editor, /const KEEP_VISIBLE = 48/);
  assert.match(editor, /moveable\.on\("drag", event => \{[\s\S]*?constrainSource\(event\.target\)/);
  assert.match(editor, /moveable\.on\("scale", event => \{[\s\S]*?constrainSource\(event\.target\)/);
  assert.match(editor, /window\.addEventListener\("resize", \(\) => \{\s*\n\s*Object\.values\(sources\)\.forEach\(constrainSource\)/);
  // 押し戻した軸は安全側へ丸める（四捨五入だと最大0.5px戻し足りない）。
  assert.ok(editor.includes("correction > 0 ? Math.ceil(value) : correction < 0 ? Math.floor(value)"),
    "押し戻した軸の丸めが安全側になっていません");
  // translate と scale では省略時の既定が違う（Yは0、倍率はXと同じ）。
  assert.match(editor, /numbers\.length \? \[numbers\[0\], numbers\[1\] \?\? 0\] : \[0, 0\]/);
  assert.match(editor, /numbers\.length \? \[numbers\[0\], numbers\[1\] \?\? numbers\[0\]\] : \[1, 1\]/);
});
