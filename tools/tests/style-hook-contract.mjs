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

// JavaScriptが組み立てるクラス名の接頭辞。`notice-state-${state}` のように
// 末尾が動くものは、CSS側の完全な名前がソースへ現れない。
const DYNAMIC_PREFIXES = ["boot-kind-", "notice-state-", "toast-"];
// 取り込んだ外部ランタイムが自分で付けるクラス。
const VENDOR_PREFIXES = ["moveable-"];

test("CSSのクラスは実際に付く要素がある", () => {
  // 名前を間違えたまま置かれた規則は、何も起きないのに残り続ける。実際に
  // `[data-layout="stream"] .history-name`（正しくは `.history-role`）は
  // 書かれた時から一度も効いておらず、`.placement-actions` は要素の削除後も残っていた。
  const referenced = `${scripts.text}\n${markup.text}`;
  const unused = new Set();
  for (const match of styles.text.matchAll(/\.([a-zA-Z][a-zA-Z0-9_-]*)/g)) {
    const className = match[1];
    if (referenced.includes(className)) continue;
    if (DYNAMIC_PREFIXES.some(prefix => className.startsWith(prefix))) continue;
    if (VENDOR_PREFIXES.some(prefix => className.startsWith(prefix))) continue;
    unused.add(className);
  }
  assert.deepEqual(
    [...unused],
    [],
    "どの要素にも付かないクラスです。名前を直すか、規則ごと消してください",
  );
});
