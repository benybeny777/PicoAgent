import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const settings = await readFile(new URL("../../ui/settings.js", import.meta.url), "utf8");
const loadModels = settings.slice(settings.indexOf("async function loadModels("), settings.indexOf("\n}", settings.indexOf("async function loadModels(")) + 2);

function harness(listModels, value = "codex") {
  const context = {
    modelLoadGeneration: 0,
    modelOptions: { append() {} },
    modelInput: { value },
    cliModelSelect: { replaceChildren(...options) { this.options = options; } },
    statusEl: {},
    window: { picoagent: { settings: { listModels } } },
    data: { active: { providerId: "agent_cli" } },
    providerDefaultModel: () => "codex",
    el: (tag, props) => ({ tag, ...props }),
    updateDirtyState() {}, logApp() {}, errorDetail: String,
  };
  vm.createContext(context);
  vm.runInContext(loadModels, context);
  return context;
}

test("取得した新モデルを選択肢へ追加しアプリ既定を残す", async () => {
  const ctx = harness(async () => ["codex", "codex:future-model", "claude", "claude:opus"]);
  await ctx.loadModels("agent_cli");
  assert.equal(ctx.modelInput.value, "codex");
  assert.equal(ctx.cliModelSelect.options[0].textContent, "codex：アプリのデフォルト");
  assert.ok(ctx.cliModelSelect.options.some(option => option.value === "codex:future-model"));
});

test("一覧から消えた保存済み選択を別モデルへ置換しない", async () => {
  const ctx = harness(async () => ["codex"], "codex:saved-model");
  await ctx.loadModels("agent_cli");
  assert.equal(ctx.modelInput.value, "codex:saved-model");
  assert.ok(ctx.cliModelSelect.options.find(option => option.value === "codex:saved-model").selected);
});

test("遅い古い取得結果で最新の選択肢を上書きしない", async () => {
  const pending = [];
  const ctx = harness(() => new Promise(resolve => pending.push(resolve)));
  const first = ctx.loadModels("agent_cli");
  const second = ctx.loadModels("agent_cli");
  pending[1](["codex", "codex:new"]);
  await second;
  pending[0](["codex", "codex:old"]);
  await first;
  assert.ok(ctx.cliModelSelect.options.some(option => option.value === "codex:new"));
  assert.ok(!ctx.cliModelSelect.options.some(option => option.value === "codex:old"));
});
