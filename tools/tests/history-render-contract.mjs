// 会話履歴の描き直しを固定する。
//
// 発言のたびに全件を Markdown から組み直すと、80件たまった会話ではチャット・作業形式で
// 毎ターン数百要素を作り直すことになる。伸びただけなら末尾へ足すだけにし、
// 切替・検索・古い発言の切り捨て・非表示からの復帰では必ず全件を組み直す。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const source = renderer.slice(
  renderer.indexOf("// 直前に描いた内容の控え。"),
  renderer.indexOf("function scheduleAppQuit()"),
);
assert.ok(source.includes("function renderHistoryRows()"), "描き直しの正本が見つかりません");

function node(tag = "div") {
  return {
    tag,
    className: "",
    textContent: "",
    children: [],
    append(...items) { this.children.push(...items); },
    remove() {},
    addEventListener() {},
    querySelector(selector) {
      const wanted = selector.replace(".", "");
      return this.children.find(child => String(child.className).split(/\s+/).includes(wanted)) || null;
    },
  };
}

function harness({ layoutShowsHistory = () => true } = {}) {
  const historyLog = {
    ...node(),
    scrollHeight: 0, scrollTop: 0, clientHeight: 0,
    replaceChildren(...items) { this.children = items; },
    querySelector(selector) {
      const wanted = selector.replace(".", "");
      const index = this.children.findIndex(child => String(child.className).split(/\s+/).includes(wanted));
      if (index < 0) return null;
      const found = this.children[index];
      return { ...found, remove: () => this.children.splice(this.children.indexOf(found), 1) };
    },
  };
  let markdownCalls = 0;
  const context = vm.createContext({
    historyLog,
    bubble: { classList: { contains: () => false } },
    layoutShowsHistory,
    document: { createElement: tag => node(tag) },
    renderMarkdown() { markdownCalls += 1; },
    copyHistoryItem() {}, reuseHistoryItem() {},
    createTypingIndicator: () => node("span"),
    Date,
  });
  vm.runInContext(`
    var conversationHistory = [];
    var historyQuery = "";
    var historyThinking = false;
    var historyThinkingText = "考え中";
    var characterManifest = { name: "むぎ" };
  `, context);
  vm.runInContext(source, context);
  return {
    context,
    historyLog,
    markdown: () => markdownCalls,
    rows: () => historyLog.children.filter(child => String(child.className).includes("history-item")).length,
    add(role, content) {
      context.conversationHistory.push({ role, content, at: 0 });
    },
    render() { context.renderHistoryRows(); },
  };
}

test("会話が伸びただけなら増えたぶんだけ組み立てる", () => {
  const ui = harness();
  ui.add("user", "こんにちは");
  ui.add("assistant", "やあ");
  ui.render();
  assert.equal(ui.rows(), 2);
  assert.equal(ui.markdown(), 2);

  ui.add("user", "次の質問");
  ui.render();
  assert.equal(ui.rows(), 3, "増えたぶんが出ていません");
  assert.equal(ui.markdown(), 3, "既に出ている発言まで組み直しています");
});

test("検索・キャラ切替・古い発言の切り捨てでは全件を組み直す", () => {
  const ui = harness();
  ui.add("user", "りんご");
  ui.add("assistant", "みかん");
  ui.render();
  assert.equal(ui.markdown(), 2);

  // 絞り込みは並びが変わるので作り直す。
  ui.context.historyQuery = "りんご";
  ui.render();
  assert.equal(ui.rows(), 1);
  assert.equal(ui.markdown(), 3);

  // 名前が変わったら、既に出ている行の表示名も直す必要がある。
  ui.context.historyQuery = "";
  ui.render();
  const before = ui.markdown();
  ui.context.characterManifest = { name: "ひより" };
  ui.render();
  assert.equal(ui.markdown(), before + 2, "キャラ切替で表示名が古いまま残ります");

  // 上限で古い発言を捨てたら先頭がずれるので作り直す。
  ui.context.conversationHistory.shift();
  const shifted = ui.markdown();
  ui.render();
  assert.equal(ui.markdown(), shifted + 1);
});

test("非表示の間に進んだ会話は、見えたときに全件出す", () => {
  let visible = true;
  const ui = harness({ layoutShowsHistory: () => visible });
  ui.add("user", "1件目");
  ui.render();
  assert.equal(ui.rows(), 1);

  visible = false;
  ui.add("assistant", "2件目");
  ui.render();
  assert.equal(ui.rows(), 1, "見えていない間は描かない");

  visible = true;
  ui.render();
  assert.equal(ui.rows(), 2, "見えたときに取りこぼした発言が出ていません");
});

test("考え中の行は増やさず入れ替える", () => {
  const ui = harness();
  ui.add("user", "おねがい");
  ui.context.historyThinking = true;
  ui.render();
  assert.equal(ui.rows(), 2);

  ui.render();
  assert.equal(ui.rows(), 2, "考え中の行が二重に出ています");

  ui.context.historyThinking = false;
  ui.render();
  assert.equal(ui.rows(), 1, "考え中の行が残っています");
});
