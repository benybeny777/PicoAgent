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
