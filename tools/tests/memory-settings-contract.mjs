import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

await import("../../ui/memory-settings.js");

const memorySettings = globalThis.PicoMemorySettings;
const settings = await readFile(new URL("../../ui/settings.js", import.meta.url), "utf8");
const bridge = await readFile(new URL("../../ui/picoagent.js", import.meta.url), "utf8");

test("長期メモリは最終更新日時の新しい順に並べる", () => {
  const sorted = memorySettings.sortMemories([
    { text: "古い", updated_at: "2026-08-17T00:00:00Z" },
    { text: "新しい", updated_at: "2026-08-18T00:00:00Z" },
  ]);
  assert.deepEqual(sorted.map(item => item.text), ["新しい", "古い"]);
});

test("不正な日時も消さずに表示する", () => {
  assert.equal(memorySettings.formatTimestamp("不明な日時"), "不明な日時");
  assert.equal(memorySettings.formatTimestamp(""), "日時不明");
});

test("設定画面は専用モジュールと一覧・個別削除APIを使う", () => {
  assert.match(settings, /PicoMemorySettings\.buildMemoryManager/);
  assert.match(bridge, /listMemories: \(\) => call\("settings_memory_list"\)/);
  assert.match(bridge, /deleteMemory: \(category, text, updatedAt\)/);
});

// 応答に items が無い形を受け取ったときに空一覧で描き直すと、実際は残っている記憶を
// 「0件」と誤って見せてしまう。表示中の内容を置き換えないことを固定する。
function stubNode(tag) {
  const node = {
    tag,
    children: [],
    handlers: {},
    isConnected: true,
    classList: { add() {} },
    setAttribute() {},
    addEventListener(type, handler) { this.handlers[type] = handler; },
    append(...items) { this.children.push(...items); },
    replaceChildren(...items) { this.children = items; },
  };
  return node;
}

function buildHarness(api) {
  const logs = [];
  const el = (tag, props = {}, children = []) => {
    const node = Object.assign(stubNode(tag), props);
    node.children = [...children];
    return node;
  };
  const manager = memorySettings.buildMemoryManager({
    api,
    section: () => stubNode("section"),
    el,
    markSearchable: node => node,
    refreshSearch: () => {},
    setPageStatus: () => {},
    setBusy: () => {},
    isBusy: () => false,
    logApp: (level, message) => logs.push(`${level}:${message}`),
    errorDetail: error => String(error),
  });
  const status = manager.children.flatMap(child => child.children ?? []).find(child => child.tag === "p");
  const list = manager.children.find(child => child.tag === "div" && child.className === "memory-list");
  return { manager, status, list, logs };
}

test("一覧を受け取れなかったときは表示を空へ置き換えない", async () => {
  const items = [{ category: "好み", text: "紅茶が好き", updated_at: "2026-08-18T00:00:00Z", hits: 2 }];
  let response = { items };
  const harness = buildHarness({ listMemories: async () => response });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(harness.list.children.length, 1, "最初の読み込みで1件描いている");
  assert.match(harness.status.textContent, /1件/);

  response = { ok: true }; // items を含まない想定外の応答
  harness.manager.children
    .flatMap(child => child.children ?? [])
    .find(child => child.tag === "button")
    .handlers.click();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(harness.list.children.length, 1, "空一覧で描き直している");
  assert.match(harness.status.textContent, /応答の形式が想定と違います/);
  assert.ok(harness.logs.some(line => line.startsWith("warn:")), "理由をログへ残していない");
});
