// 画面のJavaScriptが付けるクラスが、実際に見た目か処理へ繋がっていることを固定する。
//
// 見た目の無いクラスを付けても例外にはならず、「状態は変わっているのに何も起きない」
// だけになる。実際に `has-notices`（通知あり）と `napping`（居眠り）は、対応する
// CSSが無いまま残り、利用者には何の変化も出ていなかった。
import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import test from "node:test";

const uiDir = new URL("../../ui/", import.meta.url);
// 画面のソースがある場所（ui 直下の共有部品と、メイン画面のモジュール・部品・テーマ）。
const SOURCE_DIRS = ["", "app/", "chat/", "sources/", "character/", "components/", "themes/glass/", "themes/pastel/"];
const names = (await Promise.all(SOURCE_DIRS.map(async dir => (await readdir(new URL(dir, uiDir))).map(name => `${dir}${name}`)))).flat();
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
    // メイン画面のモジュールは el("div", { className: "…" }) で付ける。
    const pattern = /^(app|chat|sources|character)\//.test(name) ? /className\s*(?:=|:)\s*"([a-z0-9 -]+)"/g : /className\s*=\s*"([a-z0-9 -]+)"/g;
    for (const match of body.matchAll(pattern)) {
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
const VENDOR_PREFIXES = [];

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

test("画面が購読するイベントは橋渡しの一覧に載っている", async () => {
  // `on()` は一覧に無いチャンネルを購読しない。載せ忘れると、名前は合っているのに
  // 画面だけ無反応になるため、購読側と一覧を突き合わせる。
  const bridge = await readFile(new URL("picoagent.js", uiDir), "utf8");
  const listed = bridge.match(/const PET_EVENT_CHANNELS = \[([\s\S]*?)\];/)?.[1] || "";
  assert.ok(listed.includes("pet:notice"), "イベント一覧を読み取れませんでした");
  const subscribed = new Set();
  for (const match of scripts.text.matchAll(/\.on\??\.?\("([a-z][a-z:_-]+)"/g)) {
    subscribed.add(match[1]);
  }
  // `.on("drag")` のような外部ライブラリの購読は対象外（`pet:` と `engine:` だけ見る）。
  const petChannels = [...subscribed].filter(name => name.includes(":"));
  assert.ok(petChannels.length > 5, `購読の抽出に失敗している可能性: ${petChannels.length}件`);
  const unlisted = petChannels.filter(name => !listed.includes(`"${name}"`));
  assert.deepEqual(unlisted, [], "picoagent.js の PET_EVENT_CHANNELS へ追加してください");
  // 一覧に無いチャンネルは黙って捨てず、理由をログへ残す。
  assert.match(bridge, /if \(!PET_EVENT_CHANNELS\.includes\(channel\)\) \{[\s\S]*?logBridgeError\(/);
});

test("キーなしのローカルAPIは危険を画面で伝える", async () => {
  // 127.0.0.1 に閉じていても、キーが空なら同じPCのどのプログラムからでも
  // エージェント（ファイル操作・コマンド実行を含む）を呼べる。設定画面と設定一覧の
  // 両方で同じことを伝える（片方だけ直す取りこぼしを防ぐ）。
  const settings = await readFile(new URL("settings.js", uiDir), "utf8");
  assert.match(settings, /APIキーを空のまま有効にすると、このPCで動くどのプログラムからでもエージェント/);
  const doc = await readFile(new URL("../../docs/SETTINGS.md", import.meta.url), "utf8");
  assert.match(doc, /空のまま有効にすると、同じPCで動くどのプログラムからでもエージェント/);
});

test("画面のHTMLへ版を手書きしない", () => {
  // 配信用ビルドの `stamp_asset_versions()` が内容ハッシュの `?v=` を打つ。
  // 元のHTMLに書いた版は製品へ届かず、更新したつもりで効かない指定が残る。
  const written = [];
  markup.targets.forEach((name, index) => {
    for (const match of markup.bodies[index].matchAll(/(?:src|href)="([^"]*\?v=[^"]*)"/g)) {
      written.push(`${name}: ${match[1]}`);
    }
  });
  assert.deepEqual(written, [], "版は配信用ビルドが内容ハッシュで打ちます");
});
