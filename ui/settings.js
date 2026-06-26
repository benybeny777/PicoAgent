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
let characterVoiceOverrides = {};
let characterSelect;
let characterPersona;
let characterSpeaker;
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
  if (key === "PICOAGENT_LOCAL_MODEL_PRESET") {
    return localModelPresetField(key, label, value || "auto");
  }
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
  if (key === "PICOAGENT_MODEL") input.setAttribute("list", "modelOptions");
  inputs.set(key, input);
  return el("div", { className: "field" }, [el("label", { textContent: label }), input]);
}

function localModelPresetField(key, label, value) {
  const options = [
    { value: "auto", label: "自動（初回だけPC性能から選ぶ）" },
    ...(data.localModelPresets || []).map(preset => ({
      value: preset.id,
      label: `${preset.label} / 推奨VRAM ${preset.recommendedVramGb || 0}GB以上 / RAM ${preset.recommendedRamGb || 0}GB以上 / DL ${preset.downloadSize}`,
    })),
    { value: "custom", label: "カスタムURLを使う" },
  ];
  const wrapper = selectField(key, label, value, options);
  const hint = el("div", { className: "hint", textContent: localModelPresetHint(value) });
  const select = wrapper.querySelector("select");
  select.addEventListener("change", () => { hint.textContent = localModelPresetHint(select.value); });
  wrapper.append(hint);
  return wrapper;
}

function localModelPresetHint(value) {
  if (value === "auto") return "初回起動時にVRAM/RAM目安で small / balanced / large を選び、以後は選定結果を保存します。";
  if (value === "custom") return "下のカスタムモデルURLを使います。GGUFファイルの直接URLを指定してください。";
  const preset = (data.localModelPresets || []).find(item => item.id === value);
  if (!preset) return "";
  return `${preset.note} 推奨VRAM: ${preset.recommendedVramGb}GB以上、推奨RAM: ${preset.recommendedRamGb}GB以上、ダウンロード目安: ${preset.downloadSize}`;
}

function selectField(key, label, value, options) {
  const select = el("select");
  for (const option of options) {
    select.append(el("option", {
      value: option.value,
      textContent: option.label,
      selected: option.value === value,
    }));
  }
  inputs.set(key, select);
  return el("div", { className: "field" }, [el("label", { textContent: label }), select]);
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

function saveCurrentCharacterVoice(id) {
  if (!characterSpeaker) return;
  const current = characterVoiceOverrides[id] || {};
  const speaker = characterSpeaker.value.trim();
  characterVoiceOverrides[id] = { ...current, speaker };
}

function loadCharacterVoice(id) {
  const current = characterVoiceOverrides[id] || {};
  characterSpeaker.value = current.speaker || "";
  characterSpeaker.placeholder = inputs.get("PICOAGENT_TTS_SPEAKER")?.value || "888753760";
}

async function render() {
  form.innerHTML = "";
  const v = data.values;
  const panels = createTabs([
    { id: "basic", label: "基本" },
    { id: "character", label: "キャラクター" },
    { id: "voice", label: "音声" },
    { id: "providers", label: "叩くAI" },
    { id: "tools", label: "スキル" },
  ]);

  // --- グローバル設定 ---
  const global = section("基本");
  const providerSelect = el("select");
  for (const p of data.providers) {
    providerSelect.append(el("option", { value: p.id, textContent: p.label, selected: p.id === data.active.providerId }));
  }
  inputs.set("PICOAGENT_PROVIDER", providerSelect);
  global.append(el("div", { className: "field" }, [el("label", { textContent: "叩く AI（プロバイダ）" }), providerSelect]));
  global.append(field("PICOAGENT_MODEL", "モデル", "text", v.PICOAGENT_MODEL ?? data.active.model, data.active.model));
  modelInput = inputs.get("PICOAGENT_MODEL");
  global.append(field("PICOAGENT_SYSTEM_PROMPT", "システムプロンプト", "textarea", v.PICOAGENT_SYSTEM_PROMPT ?? "", data.defaults.systemPrompt));
  global.append(field("PICOAGENT_BUBBLE_FONT_PX", "フキダシの文字サイズ(px)", "text", v.PICOAGENT_BUBBLE_FONT_PX ?? "", "13"));
  global.append(field("PICOAGENT_BUBBLE_WIDTH_PX", "フキダシの幅(px)", "text", v.PICOAGENT_BUBBLE_WIDTH_PX ?? "", "ドラッグで設定"));
  global.append(field("PICOAGENT_BUBBLE_HEIGHT_PX", "フキダシの高さ(px)", "text", v.PICOAGENT_BUBBLE_HEIGHT_PX ?? "", "ドラッグで設定"));
  global.append(field("PICOAGENT_ALWAYS_ON_TOP", "常に手前に表示", "checkbox", v.PICOAGENT_ALWAYS_ON_TOP !== false));
  const voiceSec = section("読み上げ");
  voiceSec.append(field("PICOAGENT_TTS_ENABLED", "Aivis で回答を読み上げる", "checkbox", v.PICOAGENT_TTS_ENABLED === true));
  voiceSec.append(selectField("PICOAGENT_TTS_DEVICE", "Aivis 推論デバイス", v.PICOAGENT_TTS_DEVICE || "auto", [
    { value: "auto", label: "自動（GPU優先）" },
    { value: "cpu", label: "CPU固定" },
    { value: "gpu", label: "GPU固定" },
  ]));
  voiceSec.append(field("PICOAGENT_TTS_SPEAKER", "共通 Aivis speaker ID", "text", v.PICOAGENT_TTS_SPEAKER ?? "", "888753760"));
  panels.basic.append(global);
  panels.voice.append(voiceSec);

  // --- キャラクター ---
  const characterResponse = await fetch(`assets/characters/index.json?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!characterResponse.ok) throw new Error(`キャラクター一覧: HTTP ${characterResponse.status}`);
  characterCatalog = await characterResponse.json();
  characterOverrides = v.PICOAGENT_CHARACTER_OVERRIDES || {};
  characterVoiceOverrides = v.PICOAGENT_TTS_CHARACTER_OVERRIDES || {};
  const characterSec = section("キャラクター設定");
  characterSelect = el("select");
  const selectedCharacter = v.PICOAGENT_CHARACTER || characterCatalog.default;
  for (const character of characterCatalog.characters) {
    characterSelect.append(el("option", { value: character.id, textContent: character.name, selected: character.id === selectedCharacter }));
  }
  inputs.set("PICOAGENT_CHARACTER", characterSelect);
  characterSec.append(el("div", { className: "field" }, [el("label", { textContent: "キャラクター" }), characterSelect]));
  characterPersona = el("textarea", { placeholder: "性格・口調・一人称など" });
  characterSec.append(el("div", { className: "field" }, [el("label", { textContent: "このキャラの性格・口調" }), characterPersona]));
  characterSpeaker = el("input", { type: "text", placeholder: "888753760" });
  characterSec.append(el("div", { className: "field" }, [el("label", { textContent: "このキャラの Aivis speaker ID" }), characterSpeaker]));
  await loadCharacterPersona(selectedCharacter);
  loadCharacterVoice(selectedCharacter);
  let previousCharacter = selectedCharacter;
  characterSelect.addEventListener("change", async () => {
    characterOverrides[previousCharacter] = characterPersona.value;
    saveCurrentCharacterVoice(previousCharacter);
    previousCharacter = characterSelect.value;
    try { await loadCharacterPersona(previousCharacter); }
    catch (error) { statusEl.textContent = `人格設定を読み込めませんでした: ${error.message}`; }
    loadCharacterVoice(previousCharacter);
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
  inputs.get("PICOAGENT_LOCAL_MODEL_PRESET")?.addEventListener("change", () => {
    if (providerSelect.value !== "local" || !modelInput) return;
    const preset = inputs.get("PICOAGENT_LOCAL_MODEL_PRESET").value;
    modelInput.value = preset === "auto" ? providerDefaultModel("local") : preset;
  });
  loadModels(providerSelect.value);
}

function providerDefaultModel(providerId) {
  const provider = data.providers.find(p => p.id === providerId);
  if (!provider) return "";
  if (providerId === "ollama") return inputs.get("OLLAMA_MODEL")?.value || provider.defaultModel || "";
  if (providerId === "openai") return inputs.get("OPENAI_MODEL")?.value || provider.defaultModel || "";
  if (providerId === "local") {
    const preset = inputs.get("PICOAGENT_LOCAL_MODEL_PRESET")?.value || provider.defaultModel || "balanced";
    return preset === "auto" ? (provider.defaultModel || "balanced") : preset;
  }
  return provider.defaultModel || "";
}

async function loadModels(providerId, applyProviderDefault = false) {
  modelOptions.innerHTML = "";
  statusEl.textContent = providerId === "ollama" ? "Ollama のモデル一覧を取得中…" : "";
  let models = [];
  try {
    models = await window.picoagent.settings.listModels(providerId);
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
  if (patch.PICOAGENT_PROVIDER === "ollama" && patch.PICOAGENT_MODEL) patch.OLLAMA_MODEL = patch.PICOAGENT_MODEL;
  if (patch.PICOAGENT_PROVIDER === "openai" && patch.PICOAGENT_MODEL) patch.OPENAI_MODEL = patch.PICOAGENT_MODEL;
  if (patch.PICOAGENT_PROVIDER === "local") {
    const preset = patch.PICOAGENT_LOCAL_MODEL_PRESET || "auto";
    patch.PICOAGENT_MODEL = preset === "auto" ? "" : preset;
  }
  if (characterSelect && characterPersona) characterOverrides[characterSelect.value] = characterPersona.value;
  if (characterSelect && characterSpeaker) saveCurrentCharacterVoice(characterSelect.value);
  patch.PICOAGENT_CHARACTER_OVERRIDES = characterOverrides;
  patch.PICOAGENT_TTS_CHARACTER_OVERRIDES = characterVoiceOverrides;
  return patch;
}

saveBtn.addEventListener("click", async () => {
  saveBtn.disabled = true;
  statusEl.textContent = "保存中…";
  try {
    const res = await window.picoagent.settings.save(collect());
    statusEl.textContent = res.up ? "保存しました（接続OK）" : "保存しました（叩く AI に未接続）";
  } catch (error) {
    statusEl.textContent = `保存に失敗: ${error.message}`;
  } finally {
    saveBtn.disabled = false;
  }
});

(async () => {
  data = await window.picoagent.settings.load();
  try { await render(); }
  catch (error) { statusEl.textContent = `設定画面の読み込みに失敗: ${error.message}`; }
})();
