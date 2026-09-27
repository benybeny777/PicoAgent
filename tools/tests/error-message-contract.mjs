// 画面へ出すエラー理由が「undefined」にならないことを固定する。
// Tauri の invoke は失敗を文字列で返すため、`${error.message}` だけで表示すると
// 利用者には「モデル一覧を取得できませんでした: undefined」しか残らない。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import { mainScript } from "./lib/ui-source.mjs";

const common = await readFile(new URL("../../ui/common.js", import.meta.url), "utf8");
const main = mainScript();
const settings = await readFile(new URL("../../ui/settings.js", import.meta.url), "utf8");

function loadPicoUi() {
  const window = {};
  vm.runInNewContext(common, { window, console });
  return window.PicoUI;
}

test("errorMessage は Error・文字列・message 付きオブジェクトから本文を取り出す", () => {
  const { errorMessage } = loadPicoUi();
  assert.equal(errorMessage(new Error("壊れた")), "壊れた");
  assert.equal(errorMessage("Ollama に接続できません"), "Ollama に接続できません");
  assert.equal(errorMessage({ message: "権限がありません" }), "権限がありません");
  assert.equal(errorMessage({ code: 5 }), '{"code":5}');
  assert.equal(errorMessage(undefined), "原因不明のエラー");
  assert.equal(errorMessage(""), "原因不明のエラー");
});

test("画面表示の文言で catch した値の .message を直接埋め込まない", () => {
  for (const [name, source] of [["メイン画面（ui/app ほか）", main], ["settings.js", settings]]) {
    const offenders = source
      .split("\n")
      .filter(line => /\$\{(error|err|e)\??\.message( \|\| (error|err|e))?\}/.test(line));
    assert.deepEqual(offenders, [], `${name} は errorMessage(error) を使う`);
  }
});
