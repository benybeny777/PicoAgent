// 上端メニューの「表示」は、設定画面を開かずに画面レイアウトの一覧を直接出す。
// 以前は設定ウィンドウを開いてカードを選び、保存するまで数手かかっていた。
// 一覧の組み立てを右クリックと別に持つと、選択中のチェックや項目がずれるため共有を固定する。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");
const [index, topMenu, bridge, lib, menu, readme, spec, styles] = await Promise.all([
  read("ui/index.html"),
  read("ui/top-menu.js"),
  read("ui/picoagent.js"),
  read("src-tauri/src/lib.rs"),
  read("src-tauri/src/menu.rs"),
  read("README.md"),
  read("SPEC.md"),
  read("ui/styles.css"),
]);

test("狭い画面で上端メニューを畳む規則が「表示」ボタンに当たり続ける", () => {
  // 属性で選んでいた規則が外れると、最小寸法で「表示」が残って設定とウィンドウ操作を押し出す。
  assert.match(styles, /\.top-window-menu > #layoutMenuButton \{ display:none; \}/);
  assert.doesNotMatch(styles, /data-settings-tab="display"/);
});

test("上端メニューの項目を指すCSS規則はすべて実在するボタンに当たる", () => {
  // ボタンを属性からIDへ付け替えると、畳む規則だけが空振りして残る。「キャラ選択」は
  // キャラ名を表示するため、名前の長いキャラ（ひより）の最小寸法でだけ閉じるボタンを押し出していた。
  const nav = index.match(/<nav id="topWindowMenu"[\s\S]*?<\/nav>/)?.[0];
  assert.ok(nav, "上端メニューのマークアップが見つかりません");
  const selectors = [...styles.matchAll(/\.top-window-menu > (#[\w-]+|button\[[^\]]+\]|\.[\w-]+)/g)].map(match => match[1]);
  assert.ok(selectors.length > 0);
  const missing = [...new Set(selectors)].filter(selector => {
    if (selector.startsWith("#")) return !nav.includes(`id="${selector.slice(1)}"`);
    if (selector.startsWith(".")) return !new RegExp(`class="[^"]*\\b${selector.slice(1)}\\b`).test(nav);
    const [, name, value] = selector.match(/button\[([\w-]+)(?:="([^"]*)")?\]/) || [];
    return !nav.includes(value === undefined ? name : `${name}="${value}"`);
  });
  assert.deepEqual(missing, [], "上端メニューに無い要素を指す規則");
  assert.match(styles, /\.top-window-menu > #characterMenuButton \{ max-width:9em; overflow:hidden; text-overflow:ellipsis; \}/);
});

test("「表示」ボタンは設定タブではなく画面レイアウトの一覧を開く", () => {
  assert.match(index, /<button id="layoutMenuButton"[^>]*aria-haspopup="menu"[^>]*>表示<\/button>/);
  assert.doesNotMatch(index, /data-settings-tab="display"/);
  assert.match(topMenu, /layoutButton\.getBoundingClientRect\(\)/);
  assert.match(topMenu, /await window\.picoagent\.layoutMenu\(Math\.round\(rect\.left\), Math\.round\(rect\.bottom\)\)/);
  assert.match(topMenu, /if \(layoutButton\.disabled\) return;[\s\S]*?layoutButton\.disabled = true;/);
  assert.match(topMenu, /finally \{\s*layoutButton\.disabled = false;/);
  assert.match(bridge, /layoutMenu: \(x, y\) => call\("pet_layout_menu", \{ x, y \}\)/);
  assert.match(lib, /menu::pet_layout_menu,/);
});

test("一覧は右クリックと同じ組み立てを共有し、ボタンの直下へ出す", () => {
  assert.match(menu, /pub fn pet_layout_menu\(/);
  // 選択中の形式にチェックを付ける組み立ては1か所だけに置く。
  assert.equal(menu.match(/crate::layout::LAYOUTS/g)?.length, 1);
  const layoutMenu = menu.slice(menu.indexOf("pub fn pet_layout_menu("), menu.indexOf("pub fn handle_event("));
  assert.match(layoutMenu, /layout_items\(&app\)/);
  assert.match(layoutMenu, /"display_settings"/);
  assert.match(layoutMenu, /popup_menu_at\(/);
  assert.match(layoutMenu, /LogicalPosition::new\(x, y\)/);
});

test("README と SPEC の上端メニュー説明が実装と一致する", () => {
  assert.match(readme, /\*\*「表示」\*\*から画面レイアウトを直接切り替え/);
  assert.match(spec, /上端メニューの「表示」は `pet_layout_menu` で/);
});
