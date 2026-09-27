// 上端メニューの「表示」は、設定画面を開かずに画面レイアウトの一覧を直接出す。
// 一覧の組み立てを右クリックと別に持つと、選択中のチェックや項目がずれるため共有を固定する。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");
const [chrome, appBridge, bridge, contextMenu, lib, menu, readme, spec, styles] = await Promise.all([
  read("ui/app/window-chrome.js"),
  read("ui/app/bridge.js"),
  read("ui/picoagent.js"),
  read("ui/app/context-menu.js"),
  read("src-tauri/src/lib.rs"),
  read("src-tauri/src/menu.rs"),
  read("README.md"),
  read("SPEC.md"),
  read("ui/components/window.css"),
]);

const items = chrome.match(/el\("div", \{ className: "top-window-menu-items" \}, \[([^\]]+)\]\)/)?.[1].split(",").map(item => item.trim());

test("上端メニューの並びと、狭い窓で畳む規則が実在のボタンに当たる", () => {
  // 畳む規則は並び順（nth-child）で当てる。並びを変えたら規則も一緒に直す。
  assert.deepEqual(items, ["layoutButton", "editButton", "characterButton", "gameButton", "settingsButton"]);
  const folded = [...styles.matchAll(/\.top-window-menu-items button:nth-child\((\d)\)/g)].map(match => items[Number(match[1]) - 1]);
  assert.ok(folded.length > 0);
  assert.ok(!folded.includes("settingsButton"), "設定は最後まで残す");
  assert.ok(folded.every(Boolean), "上端メニューに無い位置を指す規則があります");
  // キャラ選択はキャラ名を出すので、名前が長くてもウィンドウ操作を押し出さない。
  assert.match(styles, /\.top-window-menu-items button:nth-child\(3\) \{ max-width: 9em; overflow: hidden; text-overflow: ellipsis; \}/);
});

test("右クリックの統一メニューは窓内のクリック位置を渡す", () => {
  assert.match(contextMenu, /bridge\.menu\(remembered\.context, Math\.round\(event\.clientX\), Math\.round\(event\.clientY\)\)/);
  assert.match(appBridge, /export const menu = \(context, x, y\) => api\.menu\(context, x, y\);/);
  assert.match(bridge, /menu: \(context = \{\}, x, y\) => call\("pet_menu", \{ context, x, y \}\)/);
  const handler = menu.slice(menu.indexOf("pub fn pet_menu("), menu.indexOf("pub fn pet_layout_menu("));
  assert.match(handler, /popup_menu_at\(&menu, LogicalPosition::new\(x, y\)\)/);
  assert.doesNotMatch(handler, /window\.popup_menu\(&menu\)/);
});

test("「表示」ボタンは設定タブではなく画面レイアウトの一覧を開く", () => {
  assert.match(chrome, /const layoutButton = el\("button", \{ type: "button", textContent: "表示"[^\n]*"aria-haspopup": "menu"/);
  assert.match(chrome, /menuButton\(layoutButton, \(x, y\) => bridge\.layoutMenu\(x, y\)\);/);
  // 押したボタンの直下へ出し、開いている間は二度押しで重ねない。
  const helper = chrome.slice(chrome.indexOf("function menuButton("), chrome.indexOf("export function createWindowChrome"));
  assert.match(helper, /const rect = button\.getBoundingClientRect\(\);/);
  assert.match(helper, /await open\(Math\.round\(rect\.left\), Math\.round\(rect\.bottom\)\)/);
  assert.match(helper, /if \(button\.disabled\) return;\s*\n\s*button\.disabled = true;/);
  assert.match(helper, /finally \{ button\.disabled = false; \}/);
  assert.match(appBridge, /export const layoutMenu = \(x, y\) => api\.layoutMenu\(x, y\);/);
  assert.match(bridge, /layoutMenu: \(x, y\) => call\("pet_layout_menu", \{ x, y \}\)/);
  assert.match(lib, /menu::pet_layout_menu,/);
});

test("一覧は右クリックと同じ組み立てを共有し、ボタンの直下へ出す", () => {
  assert.match(menu, /pub fn pet_layout_menu\(/);
  const layoutMenu = menu.slice(menu.indexOf("pub fn pet_layout_menu("), menu.indexOf("pub fn handle_event("));
  assert.match(layoutMenu, /popup_menu_at\(/);
  assert.match(layoutMenu, /LogicalPosition::new\(x, y\)/);
  // 選択中の形式にチェックを付ける組み立ては1か所だけに置く。
  assert.equal(menu.match(/crate::layout::AVAILABLE_LAYOUTS/g)?.length, 1);
});

test("README と SPEC の上端メニュー説明が実装と一致する", () => {
  assert.match(readme, /\*\*「表示」\*\*から画面レイアウトを直接切り替え/);
  assert.match(spec, /上端メニューの「表示」は `pet_layout_menu` で/);
});
