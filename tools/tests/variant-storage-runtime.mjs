// 見た目の保存失敗は後続読込・人格登録を止め、読込不能は確定診断へ渡す。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";
const manifest = readFileSync(new URL("../../ui/character/manifest.js", import.meta.url), "utf8")
  .replace(/^import[\s\S]*?;\r?\n/gm, "").replace(/^export /gm, "");
const main = readFileSync(new URL("../../ui/app/main.js", import.meta.url), "utf8");

test("保存無しだけ既定選択で読込不能と破損を隠さない", () => {
  let result = { ok: true, value: null };
  const api = runInNewContext(`${manifest}\n({ savedVariant, rememberVariant })`, {
    uiStore: { read: () => result, write: () => result },
  });
  assert.equal(api.savedVariant("mugi"), "default");
  result = { ok: false, reason: "保存領域にアクセスできません" };
  assert.throws(() => api.savedVariant("mugi"), /見た目の選択を読めません/);
  assert.throws(() => api.rememberVariant("mugi", "selected"), /保存できません/);
  result = { ok: true, value: "" };
  assert.throws(() => api.savedVariant("mugi"), /形式が壊れています/);
});

test("保存失敗後は読込と成功演出を行わず操作を解除する", async () => {
  const start = main.indexOf("function renderCharacterOptions(");
  const end = main.indexOf("/** 見た目", start);
  for (const phase of ["保存失敗", "読込失敗", "成功"]) {
    let handler, reads = 0, saved = 0, reactions = 0;
    const messages = [];
    const select = { value: "new", disabled: false, addEventListener: (_, callback) => { handler = callback; } };
    const render = runInNewContext(`${main.slice(start, end)}\nrenderCharacterOptions`, {
      relationship: { extra: { replaceChildren() {}, append() {} } }, host: { kind: "image" },
      el: tag => tag === "select" ? select : {}, loadedSignature: "old",
      rememberVariant() { if (phase === "保存失敗") throw new Error("保存領域が使用できません"); saved += 1; },
      async loadCharacter() { reads += 1; if (phase === "読込失敗") throw new Error("画像を読み込めません"); },
      motion: { react() { reactions += 1; } }, logApp() {}, errorDetail: String, errorMessage: error => error.message,
      toast: message => messages.push(message),
    });
    render({ id: "mugi", variantId: "old", variants: [{ id: "old" }, { id: "new" }] });
    await handler();
    assert.equal(select.disabled, false);
    assert.equal(saved, phase === "保存失敗" ? 0 : 1);
    assert.equal(reads, phase === "保存失敗" ? 0 : 1);
    assert.equal(reactions, phase === "成功" ? 1 : 0);
    assert.equal(select.value, phase === "保存失敗" ? "old" : "new");
    assert.equal(messages.length, phase === "成功" ? 0 : 1);
  }
  assert.doesNotMatch(main, /function localStorageVariant/);
  assert.match(main, /rendererOverrides\[characterId\].*savedVariant\(characterId\)/);
});
