// ui-source.mjs - 画面の契約テストが読むソースの置き場を1か所にまとめる。
// メイン画面は ui/app・ui/chat・ui/sources・ui/character の ES Modules に分かれている。
// 「どのファイルに書いたか」ではなく「メイン画面のどこかに約束がある」を確かめたい検査は mainScript() を使う。
import { readFileSync, readdirSync } from "node:fs";

const uiRoot = new URL("../../../ui/", import.meta.url);
export const MAIN_DIRS = Object.freeze(["app", "chat", "sources", "character"]);

export function uiFile(path) {
  return readFileSync(new URL(path, uiRoot), "utf8");
}

function listJs(dir) {
  return readdirSync(new URL(`${dir}/`, uiRoot)).filter(name => name.endsWith(".js")).sort().map(name => `${dir}/${name}`);
}

/** メイン画面の ES Modules の相対パス一覧（ui/ 基準）。 */
export function mainModules() {
  return MAIN_DIRS.flatMap(listJs);
}

/** メイン画面の ES Modules を連結した文字列（ファイルの境目に見出しコメントを挟む）。 */
export function mainScript() {
  return mainModules().map(path => `\n// ==== ${path} ====\n${uiFile(path)}`).join("\n");
}

/** メイン画面のスタイル（テーマの色 + 部品）。 */
export function mainStyles() {
  const themes = readdirSync(new URL("themes/", uiRoot)).sort().map(name => `themes/${name}/tokens.css`);
  const components = readdirSync(new URL("components/", uiRoot)).filter(name => name.endsWith(".css")).sort().map(name => `components/${name}`);
  return [...themes, ...components].map(path => `\n/* ==== ${path} ==== */\n${uiFile(path)}`).join("\n");
}
