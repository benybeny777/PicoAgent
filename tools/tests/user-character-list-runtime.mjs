// 両画面が共有する実際の一覧取得処理を評価し、取得失敗と空一覧を区別する。
import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";

function harness() {
  const context = vm.createContext({ window: {}, console });
  vm.runInContext(uiFile("common.js"), context, { filename: "ui/common.js" });
  return context.window.PicoUI.loadUserCharacters;
}

const plain = value => JSON.parse(JSON.stringify(value));

test("追加キャラがない正常な一覧は取得失敗と区別できる", async () => {
  const result = await harness()(async () => ({ characters: [], root: "C:/キャラ" }), () => {
    assert.fail("空一覧で画像URLの変換は不要です");
  });
  assert.deepEqual(plain(result.entries), []);
  assert.equal(result.error, null);
  assert.equal(result.root, "C:/キャラ");
});

test("一覧取得の拒否理由を空一覧の正常扱いで消さない", async () => {
  for (const reason of [new Error("保存領域を読めません"), "権限がありません"]) {
    const result = await harness()(async () => { throw reason; }, () => {
      assert.fail("取得拒否後に画像URLを解決してはいけません");
    });
    assert.deepEqual(plain(result.entries), []);
    assert.equal(result.error, reason);
  }
});

test("不正な応答や途中の不正キャラを理由付きで失敗にする", async () => {
  const valid = { id: "追加", path: "C:/追加" };
  const invalidPayloads = [
    null, {}, { characters: {} },
    ...[null, {}, { id: "", path: "C:/追加" }, { id: 1, path: "C:/追加" },
      { id: "壊れた", path: "" }, { id: "壊れた", path: null }]
      .map(invalid => ({ characters: [valid, invalid] })),
  ];
  for (const payload of invalidPayloads) {
    const result = await harness()(async () => payload, path => `asset:${path}`);
    assert.deepEqual(plain(result.entries), [], "途中までの結果を正常な一覧にしません");
    assert.ok(result.error, `失敗理由がありません: ${JSON.stringify(payload)}`);
    assert.match(result.error.message, /追加キャラクター一覧.*不正/);
  }
});

test("同梱IDとの衝突と追加一覧の重複を除き描画用メタデータを保持する", async () => {
  const converted = [];
  const characters = [
    { id: "mugi", path: "C:/同梱衝突" },
    { id: "live", name: "追加Live2D", description: "説明", path: "C:/Live2D", live2d: true },
    { id: "live", name: "重複", path: "C:/重複" },
    { id: "dance", name: "追加MMD", path: "C:/MMD", mmd: true },
    { id: "png", path: "C:/PNG" },
  ];
  const result = await harness()(async () => ({ characters, root: "C:/キャラ" }), path => {
    converted.push(path);
    return `asset:${path}`;
  }, [{ id: "mugi" }]);
  assert.equal(result.error, null);
  assert.equal(result.root, "C:/キャラ");
  assert.deepEqual(converted, ["C:/Live2D", "C:/MMD", "C:/PNG"]);
  assert.deepEqual(plain(result.entries), [
    { id: "live", name: "追加Live2D", description: "説明", manifest: "live/character.json", root: "asset:C:/Live2D", user: true, live2d: true, mmd: false },
    { id: "dance", name: "追加MMD", description: "", manifest: "dance/character.json", root: "asset:C:/MMD", user: true, live2d: false, mmd: true },
    { id: "png", name: "png", description: "", manifest: "png/character.json", root: "asset:C:/PNG", user: true, live2d: false, mmd: false },
  ]);
});

test("配置先URLの解決失敗でも理由を保持し正常な空一覧にしない", async () => {
  const reason = new Error("配置先URLを解決できません");
  const result = await harness()(async () => ({ characters: [{ id: "追加", path: "C:/追加" }] }), () => { throw reason; });
  assert.deepEqual(plain(result.entries), []);
  assert.equal(result.error, reason);
});
