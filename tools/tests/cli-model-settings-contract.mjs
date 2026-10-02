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

function renderedProvider(values, activeProviderId) {
  const start = settings.indexOf('  const providerSelect = el("select");', settings.indexOf("async function render()"));
  const end = settings.indexOf('  aiSec.append(registerInput(', start);
  const context = vm.createContext({
    v: values,
    data: { active: { providerId: activeProviderId }, providers: [{ id: "local" }, { id: "ollama" }, { id: "openai" }] },
    el: (tag, props = {}) => ({ tag, ...props, options: [], append(option) { this.options.push(option); } }),
  });
  vm.runInContext(`${settings.slice(start, end)}; globalThis.options = providerSelect.options;`, context);
  return context.options.find(option => option.selected)?.value;
}

test("再起動をあとにしても保存済みのAIを設定画面へ戻す", () => {
  assert.equal(renderedProvider({ PICOAGENT_PROVIDER: "ollama" }, "local"), "ollama");
  assert.match(settings, /renderProviderConfig\(providerSelect\.value\);/);
  assert.doesNotMatch(settings, /renderProviderConfig\(data\.active\.providerId\)/);
});

test("競合後に載せ直した草稿のAIを実行中AIで上書きしない", () => {
  const latest = { PICOAGENT_PROVIDER: "ollama" };
  const draft = { PICOAGENT_PROVIDER: "openai" };
  assert.equal(renderedProvider({ ...latest, ...draft }, "local"), "openai");
  assert.equal(renderedProvider({}, "local"), "local");
});

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

test("狭い設定画面では見出しごとスクロールし、タブと保存の帯だけを固定する", async () => {
  // 見出し・検索・すぐ探すを固定したままだと、400px幅では設定の中身に高さの3割しか残らなかった。
  const { readFileSync } = await import("node:fs");
  const css = readFileSync(new URL("../../ui/settings.css", import.meta.url), "utf8");
  // 760px以下のメディアクエリは複数ある。今回の配置規則のブロックを説明文で探す。
  const start = css.indexOf("狭い幅では見出しごとページをスクロールさせ");
  assert.ok(start > 0, "狭幅の配置規則が見つかりません");
  const narrow = css.slice(start, css.indexOf("\n}", start));
  assert.match(narrow, /body \{ height: auto; min-height: 100vh; \}/);
  assert.match(narrow, /main \{ display: block; flex: 1 0 auto; min-width: 0; overflow: visible; overflow-x: clip;/);
  assert.match(narrow, /footer \{ position: sticky; bottom: 0; z-index: 5; \}/);
  assert.match(narrow, /\.tabs \{\s*top: 0;/);
});

test("キャラ一覧のアイコンは顔を切り出し、旧構図プレビューを表示しない", () => {
  // メッシュ化したキャラは待機アニメーションを持たず、一覧が同梱の仮画像（灰色の人型）になっていた。
  assert.match(settings, /async function meshFaceIconUrl\(rigUrl, version\)/);
  assert.match(settings, /if \(\["live2d", "mmd"\]\.includes\(characterRenderKind\(manifest\)\)\) return "";/);
  assert.match(settings, /メイン画面のキャラにマウスを乗せ/);
  assert.doesNotMatch(settings, /characterIconUrl\(character, \{ face: false \}\)/);
});
