// 実際の起動待機を取得・ロード・記憶検索の各段階で止め、表示前に待ち切ることを確かめる。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { runInNewContext } from "node:vm";
import test from "node:test";

const source = (await readFile(new URL("../../ui/app/engine.js", import.meta.url), "utf8"))
  .replace(/^import .*;\r?\n/gm, "")
  .replace("export function createEngine", "function createEngine");
const main = await readFile(new URL("../../ui/app/main.js", import.meta.url), "utf8");
const preparation = main.slice(main.indexOf('    status.clear("boot");\n    const state ='), main.indexOf('\n  } catch (error) {\n    logApp("error", "キャラクター読込に失敗しました"'));
assert.ok(preparation.includes("await engine.waitUntilSettled"));

function setup(providerId = "local") {
  const polls = [];
  const messages = [];
  const status = {
    set: text => messages.push(text), clear() {},
    setSticky: text => messages.push(text),
  };
  const createEngine = runInNewContext(`${source}\ncreateEngine`, {
    bridge: { engineStatus: () => new Promise(resolve => polls.push(resolve)) },
    errorDetail: String, logApp() {}, setTimeout: callback => callback(),
  });
  const engine = createEngine({ status, onChange() {} });
  engine.init({ providerId, up: false }, () => {});
  return {
    engine, status, messages,
    async step(phase, memoryPhase = "prepare") {
      assert.equal(polls.length, 1, "状態監視は1本だけ進める");
      polls.shift()({ phase, percent: 37, memoryEmbedding: { phase: memoryPhase, message: "記憶モデルをロード中" } });
      await new Promise(resolve => setImmediate(resolve));
    },
  };
}

for (const onboardingDone of [false, true]) {
  test(`${onboardingDone ? "通常" : "初回ダウンロード"}起動もAIと記憶モデルのロード後に音声準備へ進む`, async () => {
    const state = setup();
    const calls = [];
    let settled = false;
    const run = runInNewContext(`(async () => {
      let startupError = "", voiceError = "", preparedGreeting = null;
      const startupGreeting = "こんにちは";
      ${preparation}
      return startupError;
    })`, {
      engine: state.engine, status: state.status, info: { onboardingDone },
      onboarding: { installVoice: async () => { calls.push("導入"); } },
      bridge: { aivisStatus: async () => ({ enabled: true }) },
      voice: { enabled: true, setEnabled() {}, prepareModel: async () => calls.push("音声モデル"), prepare: async () => calls.push("挨拶合成") },
      logApp() {}, errorDetail: String,
    });
    const pending = run().then(result => { settled = true; return result; });
    await state.step("download");
    assert.equal(settled, false);
    assert.deepEqual(calls, []);
    assert.ok(state.messages.some(text => text.includes("37%")), "ダウンロードの割合を伝える");
    await state.step("starting");
    assert.equal(settled, false);
    assert.deepEqual(calls, []);
    await state.step("ready");
    assert.equal(settled, false);
    assert.deepEqual(calls, []);
    assert.ok(state.messages.includes("記憶モデルをロード中"));
    await state.step("ready", "ready");
    assert.equal(await pending, "");
    assert.deepEqual(calls, [...(onboardingDone ? [] : ["導入"]), "音声モデル", "挨拶合成"]);
  });
}

test("外部AIを選んでも有効な記憶モデルは待ち、無効なら待たない", async () => {
  const state = setup("openai");
  let settled = false;
  const pending = state.engine.waitUntilSettled(null, { includeMemory: true }).then(() => { settled = true; });
  await state.step("idle");
  assert.equal(settled, false);
  await state.step("idle", "disabled");
  await pending;
  assert.equal(settled, true);
});

test("記憶モデルのロード失敗は理由を返し、準備成功にはしない", async () => {
  const state = setup();
  const pending = state.engine.waitUntilSettled(null, { includeMemory: true });
  await state.step("ready", "error");
  assert.equal((await pending).startupError, "記憶モデルをロード中");
});
