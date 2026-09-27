// メイン画面の構成ルール。差し替え（テーマ・ソース・描画方式・形式）を1か所の追加で済ませるための約束を固定する。
//
// - バックエンドを呼ぶのは ui/app/bridge.js だけ（約束の正本は docs/UI-CONTRACT.md）。
// - 見た目の分岐は形式名・描画方式名の属性セレクタで書かない（シーン定義と描画器の側で吸収する）。
// - 色は themes/<名前>/tokens.css の変数だけ。テーマ同士で同じ変数をそろえる。
// - !important は [hidden] の打ち消し1か所だけ。
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";
import { mainModules, uiFile } from "./lib/ui-source.mjs";

const ui = new URL("../../ui/", import.meta.url);
const index = uiFile("index.html");
const componentFiles = readdirSync(new URL("components/", ui)).filter(name => name.endsWith(".css"));
const components = Object.fromEntries(componentFiles.map(name => [name, uiFile(`components/${name}`)]));
const themes = readdirSync(new URL("themes/", ui), { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort();

test("バックエンド（Tauri）を呼ぶのは bridge.js だけ", () => {
  const offenders = [];
  for (const path of mainModules()) {
    if (path === "app/bridge.js") continue;
    const source = uiFile(path);
    if (/__TAURI__|window\.picoagent\b|globalThis\.picoagent\b/.test(source)) offenders.push(path);
  }
  assert.deepEqual(offenders, [], "bridge.js に名前付きの関数を足して、そこを通してください");
  // bridge.js の関数は picoagent.js の同名の呼び出しを並べるだけで、業務処理を持たない。
  const bridge = uiFile("app/bridge.js");
  for (const chunk of bridge.split("\nexport const ").slice(1)) {
    const body = chunk.slice(0, chunk.indexOf(";") + 1);
    assert.match(body, /^\w+ = [^;]*api\.[\w.]+\([^;]*\);$/, `bridge.js に処理が入っています: ${body}`);
    assert.doesNotMatch(body, /\b(if|await|try|for|while)\b/, `bridge.js に処理が入っています: ${body}`);
  }
});

test("メイン画面は ES Modules 1本の入口から組み立て、共有の道具だけを古い形式で読む", () => {
  assert.equal((index.match(/<script type="module"/g) || []).length, 1);
  assert.match(index, /<script type="module" src="app\/main\.js"><\/script>/);
  // 古い形式で読むのは、設定画面・Node のテストと共有する状態を持たない道具だけ。
  const classic = [...index.matchAll(/<script src="([^"]+)"><\/script>/g)].map(match => match[1]);
  assert.deepEqual(classic, ["picoagent.js", "common.js", "frame-scheduler.js", "composer-draft.js", "quick-actions.js", "alarm-runtime.js", "task-run.js", "mesh-rig.js"]);
  assert.doesNotMatch(index, /\bon[a-z]+="/, "インラインのイベント属性を書かない");
});

test("!important は [hidden] の打ち消しだけ", () => {
  const found = [];
  for (const [name, css] of Object.entries(components)) {
    for (const line of css.split("\n")) if (line.includes("!important")) found.push(`${name}: ${line.trim()}`);
  }
  for (const theme of themes) {
    if (uiFile(`themes/${theme}/tokens.css`).includes("!important")) found.push(`themes/${theme}`);
  }
  assert.deepEqual(found, ["base.css: [hidden] { display: none !important; }"]);
});

test("見た目を形式名・描画方式名の属性で分岐しない", () => {
  // 形式ごとの違いは ui/scenes/<形式>.json の options（variant など）、描画方式の違いは描画器が吸収する。
  for (const [name, css] of Object.entries(components)) {
    assert.doesNotMatch(css, /\[data-layout[=\]]/, `${name} が形式名で分岐しています`);
    assert.doesNotMatch(css, /\[data-render-kind[=\]]/, `${name} が描画方式名で分岐しています`);
  }
  for (const path of mainModules()) {
    const source = uiFile(path);
    assert.doesNotMatch(source, /layoutId === "(pet|mascot|stream)"|layout === "(pet|mascot|stream)"/, `${path} が形式名で分岐しています`);
  }
});

test("テーマは index.html とテーマ一覧とフォルダがそろい、同じ変数を持つ", () => {
  const linked = [...index.matchAll(/href="themes\/([^/]+)\/tokens\.css"/g)].map(match => match[1]).sort();
  assert.deepEqual(linked, themes, "テーマのフォルダと index.html の読み込みがずれています");
  const main = uiFile("app/main.js");
  const listed = JSON.parse(main.match(/const UI_THEMES = (\[[^\]]*\]);/)[1]).sort();
  assert.deepEqual(listed, themes, "app/main.js の UI_THEMES とテーマのフォルダがずれています");
  const variables = theme => new Set([...uiFile(`themes/${theme}/tokens.css`).matchAll(/^\s*(--[a-z0-9-]+)\s*:/gm)].map(match => match[1]));
  const reference = variables(themes[0]);
  for (const theme of themes.slice(1)) {
    const own = variables(theme);
    assert.deepEqual([...own].filter(name => !reference.has(name)).sort(), [], `${theme} にだけある変数`);
    assert.deepEqual([...reference].filter(name => !own.has(name)).sort(), [], `${theme} に足りない変数`);
  }
  // テーマは自分の data-ui-theme の中だけに効く（読み込み順で他のテーマを上書きしない）。
  for (const theme of themes) {
    const css = uiFile(`themes/${theme}/tokens.css`);
    for (const match of css.matchAll(/^\s*([^{};@/\s][^{};]*)\{/gm)) {
      assert.match(match[1], new RegExp(`:root\\[data-ui-theme="${theme}"\\]`), `${theme} のセレクタが範囲外です: ${match[1].trim()}`);
    }
  }
});

test("部品が使う変数は、テーマか土台か実行時の設定で必ず定義されている", () => {
  const defined = new Set();
  const collect = css => { for (const match of css.matchAll(/(--[a-z0-9-]+)\s*:/g)) defined.add(match[1]); };
  for (const theme of themes) collect(uiFile(`themes/${theme}/tokens.css`));
  for (const css of Object.values(components)) collect(css);
  const script = mainModules().map(uiFile).join("\n");
  for (const match of script.matchAll(/setProperty\("(--[a-z0-9-]+)"/g)) defined.add(match[1]);
  const missing = new Set();
  for (const [name, css] of Object.entries(components)) {
    for (const match of css.matchAll(/var\((--[a-z0-9-]+)(\s*,)?/g)) {
      if (!defined.has(match[1]) && !match[2]) missing.add(`${name}: ${match[1]}`);
    }
  }
  assert.deepEqual([...missing], []);
});

test("ソースの登録表とファイルがそろい、どのシーンも登録済みのソースだけを置く", () => {
  const registry = uiFile("sources/index.js");
  const names = [...registry.match(/SOURCE_FACTORIES = Object\.freeze\(\{([\s\S]*?)\}\);/)[1].matchAll(/^\s*(\w+):/gm)].map(match => match[1]);
  const labels = [...registry.match(/SOURCE_LABELS = Object\.freeze\(\{([\s\S]*?)\}\);/)[1].matchAll(/^\s*(\w+):/gm)].map(match => match[1]);
  assert.deepEqual([...labels].sort(), [...names].sort(), "配置編集に出す名前が無いソースがあります");
  const scenes = readdirSync(new URL("scenes/", ui)).filter(name => name.endsWith(".json"));
  for (const file of scenes) {
    const scene = JSON.parse(uiFile(`scenes/${file}`));
    for (const name of Object.keys(scene.sources)) assert.ok(names.includes(name), `${file} の ${name} は登録されていません`);
  }
  // バックエンドの正規化も同じソース名を知っている（知らない名前は保存から落とす）。
  const sceneRs = readFileSync(new URL("../../src-tauri/src/scene.rs", import.meta.url), "utf8");
  const rsSources = JSON.parse(sceneRs.match(/const SOURCES: \[&str; \d+\] = (\[[^\]]*\]);/)[1].replace(/'/g, '"'));
  assert.deepEqual([...rsSources].sort(), names.filter(name => name !== "character").sort());
});

test("描画方式の登録表の各方式は同じ約束（メソッド）を実装する", () => {
  const host = uiFile("character/index.js");
  const registered = host.match(/export const RENDERERS = \[([^\]]*)\];/)[1].split(",").map(name => name.trim());
  const required = ["load", "framingRect", "defaultFraming", "resize", "setFraming", "setMotion", "setMouth", "setSpeaking", "setGaze", "setSleeping", "setActive", "setReducedMotion", "dispose"];
  const files = { MmdRenderer: "mmd", Live2dRenderer: "live2d", MeshRenderer: "mesh", ImageRenderer: "image" };
  for (const name of registered) {
    const source = uiFile(`character/${files[name]}.js`);
    assert.match(source, new RegExp(`export class ${name} \\{`));
    assert.match(source, /static kind = "[a-z0-9-]+";/);
    assert.match(source, /static supports\(pack\) \{/);
    for (const method of required) {
      assert.match(source, new RegExp(`\\n  (async )?${method}\\(`), `${name} に ${method} がありません`);
    }
  }
  // 画像（フレーム絵）は最後の受け皿として最後に置く。
  assert.equal(registered.at(-1), "ImageRenderer");
});
