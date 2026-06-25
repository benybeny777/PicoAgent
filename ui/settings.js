// settings.js - 設定ウィンドウのロジック。
// main から渡されるグローバル設定 + 各コネクタの configSchema を動的に描画し、
// 保存時に { 設定キー: 値 } のフラットなパッチを main へ送る。

const form = document.getElementById("form");
const statusEl = document.getElementById("status");
const saveBtn = document.getElementById("save");
const modelOptions = document.getElementById("modelOptions");

let data; // settings:load の結果
const inputs = new Map(); // key -> input 要素
let characterOverrides = {};
let characterSelect;
let characterPersona;
let characterCatalog;
let modelInput;
const characterManifestCacheKey = "2026-06-25-mugi-expression-v11";

function el(tag, props = {}, children = []) {
  const node = Object.assign(document.createElement(tag), props);
  for (const c of children) node.append(c);
  return node;
}

// 1つの設定項目を描画して inputs に登録する。
function field(key, label, type, value, placeholder) {
  if (type === "textarea") {
    const ta = el("textarea", { value: value ?? "", placeholder: placeholder || "" });
    inputs.set(key, ta);
    return el("div", { className: "field" }, [el("label", { textContent: label }), ta]);
  }
  if (type === "checkbox") {
    const cb = el("input", { type: "checkbox", checked: Boolean(value) });
    inputs.set(key, cb);
    return el("div", { className: "field inline" }, [cb, el("label", { textContent: label })]);
  }
  const input = el("input", { type: type === "password" ? "password" : "text", value: value ?? "", placeholder: placeholder || "" });
  if (key === "CODA_MODEL") input.setAttribute("list", "modelOptions");
  inputs.set(key, input);
  return el("div", { className: "field" }, [el("label", { textContent: label }), input]);
}

function section(title) {
  return el("div", { className: "section" }, [el("h2", { textContent: title })]);
}

function createTabs(items) {
  const tabs = el("div", { className: "tabs", role: "tablist" });
  const panels = el("div", { className: "tab-panels" });
  const byId = {};
  items.forEach((item, index) => {
    const selected = index === 0;
    const button = el("button", {
      className: selected ? "tab active" : "tab",
      id: `tab-${item.id}`,
      textContent: item.label,
      type: "button",
    });
    button.setAttribute("role", "tab");
    button.setAttribute("aria-selected", selected ? "true" : "false");
    button.setAttribute("aria-controls", `panel-${item.id}`);
    const panel = el("div", {
      className: selected ? "tab-panel active" : "tab-panel",
      id: `panel-${item.id}`,
    });
    panel.setAttribute("role", "tabpanel");
    panel.setAttribute("aria-labelledby", button.id);
    if (!selected) panel.hidden = true;
    button.addEventListener("click", () => {
      for (const tab of tabs.querySelectorAll(".tab")) {
        const active = tab === button;
        tab.classList.toggle("active", active);
        tab.setAttribute("aria-selected", active ? "true" : "false");
      }
      for (const current of panels.querySelectorAll(".tab-panel")) {
        const active = current === panel;
        current.classList.toggle("active", active);
        current.hidden = !active;
      }
    });
    tabs.append(button);
    panels.append(panel);
    byId[item.id] = panel;
  });
  form.append(tabs, panels);
  return byId;
}

async function loadCharacterPersona(id) {
  const entry = characterCatalog.characters.find(character => character.id === id);
  if (!entry) throw new Error(`キャラクター ${id} が一覧にありません`);
  const response = await fetch(`assets/characters/${entry.manifest}?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!response.ok) throw new Error(`キャラクター定義: HTTP ${response.status}`);
  const manifest = await response.json();
  const persona = manifest.persona || {};
  const defaultText = [
    `性格: ${persona.personality || ""}`,
    `口調: ${persona.speechStyle || ""}`,
    `一人称: ${persona.firstPerson || ""}`,
    `ユーザーの呼び方: ${persona.userName || ""}`,
    persona.systemPrompt || "",
  ].filter(Boolean).join("\n");
  characterPersona.value = characterOverrides[id] || defaultText;
}

async function render() {
  form.innerHTML = "";
  const v = data.values;
  const panels = createTabs([
    { id: "basic", label: "基本" },
    { id: "character", label: "キャラクター" },
    { id: "providers", label: "叩くAI" },
    { id: "tools", label: "スキル" },
  ]);

  // --- グローバル設定 ---
  const global = section("基本");
  const providerSelect = el("select");
  for (const p of data.providers) {
    providerSelect.append(el("option", { value: p.id, textContent: p.label, selected: p.id === data.active.providerId }));
  }
  inputs.set("CODA_PROVIDER", providerSelect);
  global.append(el("div", { className: "field" }, [el("label", { textContent: "叩く AI（プロバイダ）" }), providerSelect]));
  global.append(field("CODA_MODEL", "モデル", "text", v.CODA_MODEL ?? data.active.model, data.active.model));
  modelInput = inputs.get("CODA_MODEL");
  global.append(field("CODA_SYSTEM_PROMPT", "システムプロンプト", "textarea", v.CODA_SYSTEM_PROMPT ?? "", data.defaults.systemPrompt));
  global.append(field("CODA_BUBBLE_FONT_PX", "フキダシの文字サイズ(px)", "text", v.CODA_BUBBLE_FONT_PX ?? "", "13"));
  global.append(field("CODA_BUBBLE_WIDTH_PX", "フキダシの幅(px)", "text", v.CODA_BUBBLE_WIDTH_PX ?? "", "ドラッグで設定"));
  global.append(field("CODA_BUBBLE_HEIGHT_PX", "フキダシの高さ(px)", "text", v.CODA_BUBBLE_HEIGHT_PX ?? "", "ドラッグで設定"));
  global.append(field("CODA_ALWAYS_ON_TOP", "常に手前に表示", "checkbox", v.CODA_ALWAYS_ON_TOP !== false));
  panels.basic.append(global);

  // --- キャラクター ---
  const characterResponse = await fetch(`assets/characters/index.json?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!characterResponse.ok) throw new Error(`キャラクター一覧: HTTP ${characterResponse.status}`);
  characterCatalog = await characterResponse.json();
  characterOverrides = v.CODA_CHARACTER_OVERRIDES || {};
  const characterSec = section("キャラクター設定");
  characterSelect = el("select");
  const selectedCharacter = v.CODA_CHARACTER || characterCatalog.default;
  for (const character of characterCatalog.characters) {
    characterSelect.append(el("option", { value: character.id, textContent: character.name, selected: character.id === selectedCharacter }));
  }
  inputs.set("CODA_CHARACTER", characterSelect);
  characterSec.append(el("div", { className: "field" }, [el("label", { textContent: "キャラクター" }), characterSelect]));
  characterPersona = el("textarea", { placeholder: "性格・口調・一人称など" });
  characterSec.append(el("div", { className: "field" }, [el("label", { textContent: "このキャラの性格・口調" }), characterPersona]));
  await loadCharacterPersona(selectedCharacter);
  let previousCharacter = selectedCharacter;
  characterSelect.addEventListener("change", async () => {
    characterOverrides[previousCharacter] = characterPersona.value;
    previousCharacter = characterSelect.value;
    try { await loadCharacterPersona(previousCharacter); }
    catch (error) { statusEl.textContent = `人格設定を読み込めませんでした: ${error.message}`; }
  });
  panels.character.append(characterSec);

  // --- 導入済みスキル（エージェント機能）の一覧（読み取り専用） ---
  const skillSec = section("導入済みスキル（エージェント機能）");
  if (!data.tools.length) {
    skillSec.append(el("p", { className: "hint", textContent: "登録されているスキルはありません。" }));
  } else {
    const list = el("div", { className: "card-list" });
    for (const t of data.tools) {
      list.append(el("div", { className: "card-item" }, [
        el("div", { className: "card-name", textContent: t.label || t.name }),
        el("div", { className: "card-desc", textContent: t.description || "" }),
      ]));
    }
    skillSec.append(list);
  }
  panels.tools.append(skillSec);

  // --- 導入済みキャラクターの一覧（読み取り専用） ---
  const charSec = section("導入済みキャラクター");
  const charList = el("div", { className: "card-list" });
  for (const character of characterCatalog.characters) {
    const name = character.id === selectedCharacter ? `${character.name}（使用中）` : character.name;
    charList.append(el("div", { className: "card-item" }, [
      el("div", { className: "card-name", textContent: name }),
      el("div", { className: "card-desc", textContent: character.description || character.id }),
    ]));
  }
  charSec.append(charList);
  panels.character.append(charSec);

  // --- 叩く AI コネクタごとの設定 ---
  const provSec = section("叩く AI コネクタ");
  for (const p of data.providers) {
    if (!p.configSchema?.length) continue;
    provSec.append(el("div", { className: "subhead", textContent: p.label }));
    for (const f of p.configSchema) provSec.append(field(f.key, f.label, f.type, v[f.key], f.placeholder));
  }
  if (provSec.querySelector(".field")) panels.providers.append(provSec);

  // --- エージェント機能コネクタごとの設定 ---
  const toolSec = section("エージェント機能コネクタ");
  for (const t of data.tools) {
    if (!t.configSchema?.length) continue;
    toolSec.append(el("div", { className: "subhead", textContent: t.label }));
    for (const f of t.configSchema) toolSec.append(field(f.key, f.label, f.type, v[f.key], f.placeholder));
  }
  if (toolSec.querySelector(".field")) panels.tools.append(toolSec);

  // プロバイダ変更でモデル候補を更新し、候補が取れたらモデル欄へ反映する。
  providerSelect.addEventListener("change", () => loadModels(providerSelect.value, true));
  loadModels(providerSelect.value);
}

function providerDefaultModel(providerId) {
  const provider = data.providers.find(p => p.id === providerId);
  if (!provider) return "";
  if (providerId === "ollama") return inputs.get("OLLAMA_MODEL")?.value || provider.defaultModel || "";
  if (providerId === "openai") return inputs.get("OPENAI_MODEL")?.value || provider.defaultModel || "";
  return provider.defaultModel || "";
}

async function loadModels(providerId, applyProviderDefault = false) {
  modelOptions.innerHTML = "";
  statusEl.textContent = providerId === "ollama" ? "Ollama のモデル一覧を取得中…" : "";
  let models = [];
  try {
    models = await window.coda.settings.listModels(providerId);
  } catch (error) {
    statusEl.textContent = `モデル一覧を取得できませんでした: ${error.message}`;
    return;
  }
  for (const m of models) modelOptions.append(el("option", { value: m }));
  if (!modelInput) return;
  const preferred = providerDefaultModel(providerId);
  if (applyProviderDefault) {
    modelInput.value = models.includes(preferred) ? preferred : (models[0] || preferred || modelInput.value);
  } else if (providerId === data.active.providerId && !models.includes(modelInput.value)) {
    modelInput.value = models.includes(preferred) ? preferred : (models[0] || modelInput.value);
  }
  statusEl.textContent = providerId === "ollama"
    ? (models.length ? `${models.length}件の Ollama モデルを読み込みました。` : "Ollama のモデルが見つかりません。")
    : "";
}

function collect() {
  const patch = {};
  for (const [key, input] of inputs) {
    patch[key] = input.type === "checkbox" ? input.checked : input.value;
  }
  if (patch.CODA_PROVIDER === "ollama" && patch.CODA_MODEL) patch.OLLAMA_MODEL = patch.CODA_MODEL;
  if (patch.CODA_PROVIDER === "openai" && patch.CODA_MODEL) patch.OPENAI_MODEL = patch.CODA_MODEL;
  if (characterSelect && characterPersona) characterOverrides[characterSelect.value] = characterPersona.value;
  patch.CODA_CHARACTER_OVERRIDES = characterOverrides;
  return patch;
}

saveBtn.addEventListener("click", async () => {
  saveBtn.disabled = true;
  statusEl.textContent = "保存中…";
  try {
    const res = await window.coda.settings.save(collect());
    statusEl.textContent = res.up ? "保存しました（接続OK）" : "保存しました（叩く AI に未接続）";
  } catch (error) {
    statusEl.textContent = `保存に失敗: ${error.message}`;
  } finally {
    saveBtn.disabled = false;
  }
});

(async () => {
  data = await window.coda.settings.load();
  try { await render(); }
  catch (error) { statusEl.textContent = `設定画面の読み込みに失敗: ${error.message}`; }
})();
