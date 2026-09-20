// 画面のJavaScriptが付けるクラスが、実際に見た目か処理へ繋がっていることを固定する。
//
// 見た目の無いクラスを付けても例外にはならず、「状態は変わっているのに何も起きない」
// だけになる。実際に `has-notices`（通知あり）と `napping`（居眠り）は、対応する
// CSSが無いまま残り、利用者には何の変化も出ていなかった。
import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import test from "node:test";

const uiDir = new URL("../../ui/", import.meta.url);
const names = await readdir(uiDir);
const readAll = async extension => {
  const targets = names.filter(name => name.endsWith(extension));
  const bodies = await Promise.all(targets.map(name => readFile(new URL(name, uiDir), "utf8")));
  return { targets, text: bodies.join("\n"), bodies };
};

const scripts = await readAll(".js");
const styles = await readAll(".css");
const markup = await readAll(".html");

test("JavaScriptが付けるクラスは見た目か処理へ繋がっている", () => {
  const applied = new Map();
  scripts.targets.forEach((name, index) => {
    const body = scripts.bodies[index];
    for (const match of body.matchAll(/classList\.(?:add|toggle)\("([a-z0-9-]+)"/g)) {
      applied.set(match[1], name);
    }
    for (const match of body.matchAll(/className\s*=\s*"([a-z0-9 -]+)"/g)) {
      for (const className of match[1].split(/\s+/)) {
        if (className) applied.set(className, name);
      }
    }
  });
  assert.ok(applied.size > 40, `クラスの抽出に失敗している可能性: ${applied.size}件`);
  const dead = [...applied]
    .filter(([className]) => {
      if (styles.text.includes(`.${className}`)) return false; // CSSで見た目がある
      if (markup.text.includes(className)) return false; // HTML側の指定と対になる
      // JavaScript自身が読み返す状態（querySelector / classList.contains）も生きている。
      return !scripts.text.includes(`".${className}"`)
        && !scripts.text.includes(`classList.contains("${className}")`);
    })
    .map(([className, file]) => `${file}: ${className}`);
  assert.deepEqual(
    dead,
    [],
    "見た目も参照も無いクラスです。CSSを足すか、付けるのをやめてください",
  );
});
