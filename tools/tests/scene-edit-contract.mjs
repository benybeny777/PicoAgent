// 配置の直接操作（ui/app/edit.js）と保存（ui/app/stage.js → pet_save_scene）の契約。
// 旧画面の配置編集（Moveable）は枠を CSS で拡大したため、拡大するとキャラが切れ、保存先の取り違えで
// 別の形式・別キャラへ位置が漏れた。いまは矩形だけを、形式ごと・キャラごとの差分として保存する。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { uiFile } from "./lib/ui-source.mjs";

const edit = uiFile("app/edit.js");
const stage = uiFile("app/stage.js");
const main = uiFile("app/main.js");
const bridge = readFileSync(new URL("../../ui/picoagent.js", import.meta.url), "utf8");
const commands = readFileSync(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
const scene = readFileSync(new URL("../../src-tauri/src/scene.rs", import.meta.url), "utf8");

function section(source, start, end) {
  const from = source.indexOf(start);
  assert.ok(from >= 0, `${start} が見つかりません`);
  return source.slice(from, end ? source.indexOf(end, from) : undefined);
}

test("配置は設定（PICOAGENT_SCENES）へ保存し、正規化はバックエンドが持つ", () => {
  assert.match(commands, /pub fn pet_save_scene\(state: State<'_, AppState>, scenes: Value\)[\s\S]*?lock_state\(crate::scene::CONFIG_KEY\);[\s\S]*?crate::scene::normalize_checked\(&scenes\)\?/);
  assert.match(commands, /"scenes": crate::scene::load\(&state\.config\)/);
  assert.match(scene, /pub const CONFIG_KEY: &str = "PICOAGENT_SCENES";/);
  assert.match(bridge, /saveScene: \(scenes\) => call\("pet_save_scene", \{ scenes \}\)/);
  // 保存は1か所（stage の persist）だけで、順番に1件ずつ送る（古い写しで新しい保存を上書きしない）。
  assert.equal((uiFile("app/stage.js").match(/bridge\.saveScene\(/g) || []).length, 1);
  assert.match(stage, /const operation = saveQueue\.catch\(\(\) => undefined\)\.then\(\(\) => \{[\s\S]*?if \(generation !== saveGeneration\) return;[\s\S]*?return bridge\.saveScene\(snapshot\);/);
  // webview の localStorage へは置かない（WebViewデータの初期化・再インストールで消える）。
  assert.doesNotMatch(edit + stage, /localStorage/);
});

test("ドラッグ中は見た目だけを変え、保存はつまみを離した時だけ", () => {
  const move = section(edit, "function onDragMove(event)", "async function onDragEnd");
  assert.match(move, /stage\.preview\(drag\.name, draggedRect\(drag, event\)\);/);
  assert.doesNotMatch(move, /commit\(|persist\(/);
  const end = section(edit, "async function onDragEnd(event)", "async function commit");
  assert.match(end, /if \(!current\.moved \|\| event\.type === "pointercancel"\) \{\s*\n\s*stage\.cancelPreview\(current\.name\);/);
  assert.match(end, /await commit\(current\.name, draggedRect\(current, event\)\);/);
  const preview = section(stage, "    preview(target, windowRect) {", "    cancelPreview(");
  assert.doesNotMatch(preview, /persist\(/);
});

test("キャラの位置と大きさはキャラごと・形式ごとに保存し、縦横比を保つ", () => {
  const commit = section(stage, "    async commit(target, windowRect, extra = {}) {", "    /** ソースの表示");
  assert.match(commit, /const key = target === "character" \? \{ characterId \} : \{ source: target \};/);
  assert.match(commit, /writeSaved\(saved, layoutId, key, value\);/);
  // 保存するのは矩形（anchor / x / y / w / h）と構図名だけ。倍率は持たない。
  assert.doesNotMatch(commit, /zoom|scale/);
  const dragged = section(edit, "function draggedRect(current, event)", "function onDragMove");
  assert.match(dragged, /if \(current\.name === "character"\) \{[\s\S]*?const aspect = base\.width \/ base\.height;[\s\S]*?width = height \* aspect;/);
  // 原画を2倍以上に引き伸ばす大きさにはしない（ぼやけて見える）。
  assert.match(dragged, /maxCharacterHeight\(\)/);
});

test("形式・キャラを切り替えたら、編集中の一時値を持ち越さない", () => {
  assert.match(stage, /setLayout\(id[^)]*\) \{[\s\S]*?previews\.clear\(\);/);
  assert.match(stage, /setCharacter\(id\) \{[\s\S]*?previews\.clear\(\);/);
});

test("キャラを窓からはみ出す大きさにしたら窓を広げる（ペット・ミニ）", () => {
  const grow = section(stage, "function growWindowFor(target)", "\n  const stage =");
  assert.match(grow, /if \(requestedGeometry\.fixed \|\| target !== "character"\) return Promise\.resolve\(\);/);
  assert.match(grow, /const operation = growthQueue\.then\(async \(\) => \{/);
  assert.match(grow, /bridge\.moveAndResizeWindow\(/);
  assert.match(grow, /bridge\.setWindowSize\(width, height\)/);
});

test("右クリックの「キャラの位置とサイズ」「配置を編集」と上端の「配置」が同じ編集器を使う", () => {
  assert.match(main, /const editor = createEditor\(/);
  assert.match(main, /bridge\.on\("pet:layout-edit", target => editor\.start\(target === "character" \? "character" : "all"\)\);/);
  assert.match(main, /onEditLayout: \(\) => \(editor\.mode === "all" \? editor\.stop\(\) : editor\.start\("all"\)\),/);
  assert.match(main, /if \(action === "edit-character"\) editor\.start\("character"\);/);
});
