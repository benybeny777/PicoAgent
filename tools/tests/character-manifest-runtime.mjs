// 実manifest読込を、共有一覧の失敗・選択ID・variantの画像構造で確かめる。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";

const source = readFileSync(new URL("../../ui/character/manifest.js", import.meta.url), "utf8")
  .replace(/^import[\s\S]*?;\r?\n/gm, "").replace(/^export /gm, "");

function setup({ users = [], error = null, variants = {}, manifests = {} } = {}) {
  const fetched = [], calls = [];
  const bundled = { characters: [{ id: "bundled", name: "同梱", manifest: "bundled/character.json" }], default: "bundled" };
  const bridge = { userCharacters() {}, assetUrl() {} };
  const api = runInNewContext(`${source}\n({ listCharacters, loadPack, imageAnimationsOf: typeof imageAnimationsOf === "function" ? imageAnimationsOf : null })`, {
    bridge, fetchJson: async url => {
      fetched.push(url);
      if (url.startsWith("assets/characters/index.json")) return bundled;
      const id = url.split("/").at(-2);
      return manifests[id] || { id, name: id, animations: { idle: { framePaths: ["idle.png"] } } };
    },
    window: { PicoUI: {
      async loadUserCharacters(fetchEntries, assetUrl, known) {
        calls.push({ fetchEntries, assetUrl, known });
        return { entries: users, error, root: "user-root" };
      }, errorMessage: error => error.message || String(error),
    } },
    localStorage: { getItem: key => variants[key.slice("picoagent.variant.".length)] },
    errorDetail: String, logApp() {},
  });
  return { ...api, fetched, calls, bridge };
}

test("一覧正規化は共有口へ委ね追加metadataと取得失敗を同梱一覧へ添える", async () => {
  const entry = { id: "user", name: "追加", root: "user-root/user", metadata: { license: "利用条件" } };
  const cause = new Error("一覧の保存場所を読めません");
  const state = setup({ users: [entry], error: cause });
  const result = await state.listCharacters();
  assert.equal(result.entries[0].id, "bundled");
  assert.equal(result.entries[1], entry);
  assert.equal(result.userCharactersError, cause);
  assert.equal(result.userCharactersRoot, "user-root");
  assert.equal(state.calls.length, 1);
  assert.equal(state.calls[0].fetchEntries, state.bridge.userCharacters);
  assert.equal(state.calls[0].assetUrl, state.bridge.assetUrl);
  assert.equal(state.calls[0].known[0].id, "bundled");
});

test("追加一覧取得が失敗しても選択された同梱パックだけは読める", async () => {
  const state = setup({ error: "追加一覧の権限不足" });
  assert.equal((await state.loadPack("bundled")).id, "bundled");
  assert.equal(state.fetched.length, 2);
});

test("不明な追加IDは取得失敗の理由で停止し既定パックを代読しない", async () => {
  const state = setup({ error: new Error("追加一覧の権限不足") });
  await assert.rejects(state.loadPack("user"), /追加一覧の権限不足/);
  assert.equal(state.fetched.length, 1);
  assert.ok(state.fetched[0].includes("index.json"));
});

test("一覧取得成功後の不明IDも既定や先頭へ切り替えず理由付きで停止する", async () => {
  const state = setup();
  await assert.rejects(state.loadPack("missing"), /missing.*一覧にありません/);
  assert.equal(state.fetched.length, 1);
});

test("選択した追加パックのrootと人格をそのまま使い一覧をキャッシュで二重取得しない", async () => {
  const state = setup({ users: [{ id: "user", root: "user-root/user" }] });
  const pack = await state.loadPack("user");
  assert.equal(pack.id, "user"); assert.equal(pack.root, "user-root/user");
  assert.equal(state.calls.length, 1);
  assert.equal(state.fetched.filter(url => url.includes("index.json")).length, 1);
});

test("画像構造はvariant指定・本体指定・メッシュpreviewの順で同じ口から決める", async () => {
  const state = setup({ variants: { bundled: "mesh" }, manifests: { bundled: {
    id: "bundled", variants: [{ id: "mesh", meshRig: { rig: "rig.json" } }],
  } } });
  const pack = await state.loadPack("bundled");
  assert.equal(pack.animations.idle.framePaths[0], "preview.png");
  assert.equal(pack.meshRig.rig, "rig.json");
  const manifest = { animations: { idle: { framePaths: ["main.png"] } }, meshRig: { rig: "main.json" } };
  const variant = { animations: { idle: { framePaths: ["variant.png"] } } };
  assert.equal(state.imageAnimationsOf(manifest, variant), variant.animations);
  assert.equal(state.imageAnimationsOf(manifest), manifest.animations);
  assert.equal(state.imageAnimationsOf({ live2d: { model: "model.json" } }).idle, undefined);
});


test("追加定義の別IDは人格や声と描画方式を混線させる前に停止する", async () => {
  for (const id of ["other", " user ", 0]) {
    const state = setup({ users: [{ id: "user", root: "user-root/user" }], manifests: {
      user: { id, animations: { idle: { framePaths: ["idle.png"] } } },
    } });
    await assert.rejects(state.loadPack("user"), /キャラクター定義のID.*選択ID「user」と一致しません/);
    assert.equal(state.fetched.length, 2);
  }
});

test("定義でIDが省略された時だけ一覧の選択IDを補う", async () => {
  for (const extra of [{}, { id: null }, { id: "" }]) {
    const state = setup({ users: [{ id: "user", root: "user-root/user" }], manifests: {
      user: { ...extra, animations: { idle: { framePaths: ["idle.png"] } } },
    } });
    const pack = await state.loadPack("user");
    assert.equal(pack.id, "user");
    assert.equal(pack.manifest.id, "user");
  }
});


test("同梱パックは追加一覧失敗と復旧を通知し不明IDも停止前に理由を渡す", async () => {
  for (const error of [new Error("追加一覧の権限不足"), null]) {
    const state = setup({ error });
    const results = [];
    const pack = await state.loadPack("bundled", { onUserCharacters: result => results.push(result) });
    assert.equal(pack.id, "bundled");
    assert.equal(results.length, 1);
    assert.equal(results[0].error, error);
    assert.equal(results[0].root, "user-root");
    await assert.rejects(state.loadPack("missing", { onUserCharacters: result => results.push(result) }));
    assert.equal(results.length, 2);
    assert.equal(results[1].error, error);
  }
});
