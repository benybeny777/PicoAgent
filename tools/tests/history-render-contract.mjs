// 会話履歴の描き直しを固定する。
//
// 発言のたびに全件を Markdown から組み直すと、80件たまった会話では発言のたびに数百要素を
// 作り直すことになる。伸びただけなら末尾へ足すだけにし、検索・キャラ切替・古い発言の
// 切り捨て・非表示からの復帰では必ず全件を組み直す。実物の ui/chat/history.js を最小の DOM で動かす。
import assert from "node:assert/strict";
import test from "node:test";
import { installFakeDom } from "./lib/fake-dom.mjs";

const document = installFakeDom();
const { createHistory } = await import("../../ui/chat/history.js");

function harness() {
  let bodies = 0;
  const createElement = document.createElement;
  const log = createElement("div");
  const search = createElement("input");
  search.value = "";
  let name = "むぎ";
  const history = createHistory({
    log,
    search,
    jumpLatest: createElement("button"),
    exportButton: createElement("button"),
    characterName: () => name,
    onReuse() {},
    status: { flash() {} },
    markdownHandlers: {},
  });
  // 行を組み立てた回数（1行につき本文の入れ物を1つ作る）。
  document.createElement = tag => {
    const node = createElement(tag);
    let className = "";
    Object.defineProperty(node, "className", {
      get: () => className,
      set: value => { className = value; if (value === "history-body") bodies += 1; },
    });
    return node;
  };
  const rows = () => log.children.filter(child => child.classSet?.().has("history-item")).length;
  return {
    history,
    log,
    rows,
    built: () => bodies,
    search(text) { search.value = text; search.dispatch("input"); },
    rename(next) { name = next; },
  };
}

test("会話が伸びただけなら増えたぶんだけ組み立てる", () => {
  const ui = harness();
  ui.history.setVisible(true);
  ui.history.add("user", "こんにちは");
  ui.history.add("assistant", "やあ");
  assert.equal(ui.rows(), 2);
  const before = ui.built();
  ui.history.add("user", "次の質問");
  assert.equal(ui.rows(), 3, "増えたぶんが出ていません");
  assert.equal(ui.built(), before + 1, "既に出ている発言まで組み直しています");
});

test("検索・キャラ切替では全件を組み直す", async () => {
  const ui = harness();
  ui.history.setVisible(true);
  ui.history.add("user", "りんご");
  ui.history.add("assistant", "みかん");
  // 絞り込みは並びが変わるので作り直す（入力は 120ms まとめてから描く）。
  ui.search("りんご");
  await new Promise(resolve => setTimeout(resolve, 150));
  assert.equal(ui.rows(), 1);
  ui.search("");
  await new Promise(resolve => setTimeout(resolve, 150));
  assert.equal(ui.rows(), 2);
  // 名前が変わったら、既に出ている行の表示名も直す必要がある。
  const before = ui.built();
  ui.rename("ひより");
  ui.history.render();
  assert.equal(ui.built(), before + 2, "キャラ切替で表示名が古いまま残ります");
});

test("古い発言の切り捨てでは全件を組み直す", () => {
  const ui = harness();
  ui.history.setVisible(true);
  for (let index = 0; index < 80; index++) ui.history.add("user", `発言${index}`);
  assert.equal(ui.rows(), 80);
  const before = ui.built();
  ui.history.add("user", "81件目");
  assert.equal(ui.rows(), 80, "上限を超えた古い発言が残っています");
  assert.equal(ui.built(), before + 80, "先頭がずれたのに末尾へ足すだけになっています");
});

test("非表示の間に進んだ会話は、見えたときに全件出す", () => {
  const ui = harness();
  ui.history.setVisible(true);
  ui.history.add("user", "1件目");
  assert.equal(ui.rows(), 1);
  ui.history.setVisible(false);
  const hiddenBuilt = ui.built();
  ui.history.add("assistant", "2件目");
  assert.equal(ui.built(), hiddenBuilt, "見えていない間は描かない");
  ui.history.setVisible(true);
  assert.equal(ui.rows(), 2, "見えたときに取りこぼした発言が出ていません");
});

test("考え中の行は増やさず入れ替える", () => {
  const ui = harness();
  ui.history.setVisible(true);
  ui.history.add("user", "おねがい");
  ui.history.setThinking("考え中");
  assert.equal(ui.rows(), 2);
  const before = ui.built();
  ui.history.setThinking("Web検索中");
  assert.equal(ui.rows(), 2, "考え中の行が二重に出ています");
  assert.equal(ui.built(), before, "文言だけの更新で組み直しています");
  assert.equal(ui.log.querySelector(".typing-label").textContent, "Web検索中");
  ui.history.setThinking(null);
  assert.equal(ui.rows(), 1, "考え中の行が残っています");
});
