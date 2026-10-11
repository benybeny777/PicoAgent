import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const settings = await readFile(new URL("../../ui/settings.js", import.meta.url), "utf8");
const integerInput = await readFile(new URL("../../ui/settings-number.js", import.meta.url), "utf8");
const loadModels = settings.slice(settings.indexOf("async function loadModels("), settings.indexOf("\n}", settings.indexOf("async function loadModels(")) + 2);

function harness(listModels, value = "codex") {
  const context = {
    modelLoadGeneration: 0,
    providerModelDraftValues: {},
    modelOptions: { append() {} },
    modelInput: { type: "text", dataset: {}, value },
    cliModelSelect: { replaceChildren(...options) { this.options = options; } },
    statusEl: {},
    window: { picoagent: { settings: { listModels } } },
    data: { active: { providerId: "agent_cli" }, numberSettings: {} },
    providerDefaultModel: () => "codex",
    el: (tag, props) => ({ tag, ...props }),
    updateDirtyState() {}, logApp() {}, errorDetail: String, errorMessage: String,
  };
  vm.createContext(context);
  vm.runInContext(integerInput, context);
  vm.runInContext(loadModels, context);
  return context;
}

test("ローカル既定はバックエンド正本だけを使い明示選択を保持し欠損を画面へ返す", async () => {
  for (const [saved, backendDefault, expected] of [
    ["", "lfm25_1_2b_jp", "lfm25_1_2b_jp"],
    ["auto", "lfm25_1_2b_jp", "lfm25_1_2b_jp"],
    ["lfm25_2_6b", "lfm25_1_2b_jp", "lfm25_2_6b"],
    ["lfm25_2_6b", undefined, "lfm25_2_6b"],
    ["auto", undefined, null],
    ["", undefined, null],
  ]) {
    let listed = false;
    const ctx = harness(async () => { listed = true; return ["一覧の先頭モデル"]; }, "前のAIのモデル");
    ctx.inputs = new Map([["PICOAGENT_LOCAL_MODEL_PRESET", { value: saved }]]);
    ctx.data.defaultLocalModelPreset = backendDefault;
    ctx.data.providers = [{ id: "local", defaultModel: "旧既定モデル" }];
    ctx.data.active.providerId = "local";
    const start = settings.indexOf("function providerDefaultModel(");
    vm.runInContext(settings.slice(start, settings.indexOf("async function loadModels(", start)), ctx);
    await ctx.loadModels("local", true);
    if (expected === null) {
      assert.equal(listed, false);
      assert.equal(ctx.modelInput.value, "");
      assert.match(ctx.statusEl.textContent, /ローカルモデルの既定値を取得できません/);
    } else {
      assert.equal(ctx.modelInput.value, expected);
      assert.equal(ctx.statusEl.textContent, "");
    }
  }
});

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

test("モデル一覧の取得前に切替先AIのモデルを保存収集へ確定する", async () => {
  let resolveModels;
  const ctx = harness(() => new Promise(resolve => { resolveModels = resolve; }), "lfm25_2_6b");
  ctx.data.values = {};
  ctx.data.providers = [{ id: "ollama", defaultModel: "qwen3:8b" }];
  ctx.inputs = new Map([
    ["PICOAGENT_PROVIDER", { type: "select", dataset: {}, value: "ollama" }],
    ["PICOAGENT_MODEL", ctx.modelInput],
    ["OLLAMA_MODEL", { type: "text", dataset: {}, value: "gemma3" }],
  ]);
  ctx.cloneSettings = value => JSON.parse(JSON.stringify(value));
  vm.runInContext(`const savedSnapshot = {}; const providerDraftValues = {};
    const characterOverrides = {}, characterVoiceOverrides = {}, characterLive2dSdkOverrides = {}, characterRendererOverrides = {}, characterStreamProfiles = {};
    const alarmEditorState = null, scheduleEditorState = null;`, ctx);
  vm.runInContext(settings.slice(settings.indexOf("function collect()"), settings.indexOf("// 再起動の確認画面")), ctx);
  vm.runInContext(settings.slice(settings.indexOf("function providerDefaultModel("), settings.indexOf("async function loadModels(")), ctx);
  const loading = ctx.loadModels("ollama", true);
  assert.equal(ctx.collect().PICOAGENT_MODEL, "gemma3");
  assert.equal(ctx.collect().OLLAMA_MODEL, "gemma3");
  resolveModels(["別の候補モデル"]);
  await loading;
  assert.equal(ctx.modelInput.value, "別の候補モデル");
});

test("一覧取得の失敗でも前のAIのモデルへ戻らない", async () => {
  const ctx = harness(async () => { throw new Error("一覧取得失敗"); }, "lfm25_2_6b");
  ctx.providerDefaultModel = () => "gemma3";
  await ctx.loadModels("ollama", true);
  assert.equal(ctx.modelInput.value, "gemma3");
  assert.match(ctx.statusEl.textContent, /モデル一覧を取得できませんでした/);
});

test("CLI切替は即座に既定モデルを表示し取得中の選び直しを保持する", async () => {
  let resolveModels;
  const ctx = harness(() => new Promise(resolve => { resolveModels = resolve; }), "lfm25_2_6b");
  const loading = ctx.loadModels("agent_cli", true);
  assert.equal(ctx.modelInput.value, "codex");
  assert.equal(ctx.cliModelSelect.options[0].value, "codex");
  assert.equal(ctx.cliModelSelect.options[0].selected, true);
  ctx.modelInput.value = "codex:手入力モデル";
  resolveModels(["codex", "claude"]);
  await loading;
  assert.equal(ctx.modelInput.value, "codex:手入力モデル");
  assert.ok(ctx.cliModelSelect.options.find(option => option.value === "codex:手入力モデル").selected);
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

test("OllamaとCLIのモデル草稿を別AIへの往復で既定へ戻さない", async () => {
  for (const [provider, model] of [["ollama", "未取得の編集モデル"], ["agent_cli", "codex:編集モデル"]]) {
    const ctx = harness(async () => ["候補の先頭"], model);
    ctx.providerDraftValues = {};
    ctx.inputs = new Map([["OLLAMA_MODEL", { value: "古いモデル" }]]);
    ctx.data.providers = [{ id: provider, defaultModel: "既定モデル" }];
    vm.runInContext(settings.slice(settings.indexOf("function rememberProviderModelDraft("), settings.indexOf("async function loadModels(")), ctx);
    ctx.rememberProviderModelDraft(provider);
    ctx.modelInput.value = "別AIのモデル";
    await ctx.loadModels(provider, true);
    assert.equal(ctx.modelInput.value, model);
    if (provider === "ollama") {
      assert.equal(ctx.inputs.get("OLLAMA_MODEL").value, model);
      assert.equal(ctx.providerDraftValues.OLLAMA_MODEL, model);
    }
  }
});

test("モデル一覧待ちの保存中に旧一覧が完了しても保存値と表示モデルを一致させる", async () => {
  let resolveModels;
  let resolveSave;
  let saveHandler;
  let submitted;
  const ctx = harness(() => new Promise(resolve => { resolveModels = resolve; }), "前のAIのモデル");
  ctx.providerDefaultModel = () => "切替先の確定モデル";
  ctx.savedSnapshot = {};
  ctx.settingsMutationBusy = false;
  ctx.validateSettingsNumberFields = () => true;
  ctx.modelInput.removeAttribute = () => {};
  ctx.inputs = new Map([["PICOAGENT_MODEL", ctx.modelInput]]);
  ctx.inputWrappers = new Map();
  vm.runInContext(settings.slice(settings.indexOf("function clearNumberErrors()"), settings.indexOf("function showNumberErrors(")), ctx);
  ctx.setSettingsMutationBusy = busy => { ctx.settingsMutationBusy = busy; };
  ctx.collect = () => ({ PICOAGENT_MODEL: ctx.modelInput.value });
  ctx.settingsChangeRequest = current => ({ patch: current, baseRevisions: {} });
  ctx.cloneSettings = value => structuredClone(value);
  ctx.saveBtn = { addEventListener(type, handler) { saveHandler = handler; } };
  ctx.window.picoagent.settings.save = patch => {
    submitted = patch;
    return new Promise(resolve => { resolveSave = resolve; });
  };
  const start = settings.indexOf('saveBtn.addEventListener("click", async () => {');
  vm.runInContext(settings.slice(start, settings.indexOf('resetChangesBtn?.addEventListener(', start)), ctx);
  const loading = ctx.loadModels("ollama", true);
  const saving = saveHandler();
  assert.equal(ctx.settingsMutationBusy, true);
  resolveModels(["遅れて返る別モデル"]);
  await loading;
  assert.equal(ctx.modelInput.value, submitted.PICOAGENT_MODEL);
  assert.equal(ctx.statusEl.textContent, "保存中…");
  resolveSave({ up: true });
  await saving;
  assert.equal(ctx.settingsMutationBusy, false);
  assert.equal(ctx.savedSnapshot.PICOAGENT_MODEL, ctx.modelInput.value);
  assert.match(ctx.statusEl.textContent, /保存しました/);
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
  assert.match(settings, /async function meshFaceIconUrl\(rigUrl, version,/);
  assert.match(settings, /if \(\["live2d", "mmd"\]\.includes\(characterRenderKind\(manifest\)\)\) return \{ kind: "none" \};/);
  assert.match(settings, /メイン画面のキャラにマウスを乗せ/);
  assert.doesNotMatch(settings, /characterIconUrl\(character, \{ face: false \}\)/);
});
