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
let characterDefaultVoices = {};
let characterSelect;
let characterPersona;
let characterSpeaker;
let characterCatalog;
let modelInput;
let alarmEditorState = null; // 目覚ましエディタ（保存時に PICOAGENT_ALARMS を組み立てる）
const characterManifestCacheKey = "2026-06-27-rin-tsundere-v1";

function errorDetail(error) {
  if (!error) return "";
  if (error instanceof Error) return error.stack || error.message;
  if (typeof error === "string") return error;
  try { return JSON.stringify(error); }
  catch { return String(error); }
}

function logApp(level, message, detail) {
  try {
    window.picoagent?.log?.(level, message, detail == null ? "" : String(detail)).catch(() => {});
  } catch {}
}

window.addEventListener("error", event => {
  logApp("error", "設定画面の未捕捉エラー", `${event.message || ""} ${event.filename || ""}:${event.lineno || ""}:${event.colno || ""}\n${errorDetail(event.error)}`);
});
window.addEventListener("unhandledrejection", event => {
  logApp("error", "設定画面の未処理 Promise rejection", errorDetail(event.reason));
});

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

// 目覚ましの専用エディタ（時刻ピッカー＋曜日チェック＋繰り返し/有効/ラベル/削除/追加）。
// 保存時は alarmEditorState.getJson() で PICOAGENT_ALARMS（JSON文字列）を組み立てる。
const ALARM_WEEKDAYS = [
  ["mon", "月"], ["tue", "火"], ["wed", "水"], ["thu", "木"],
  ["fri", "金"], ["sat", "土"], ["sun", "日"],
];
function buildAlarmEditor(currentJson) {
  let alarms = [];
  try {
    const parsed = JSON.parse(currentJson || "[]");
    if (Array.isArray(parsed)) alarms = parsed;
  } catch { alarms = []; }

  const wrap = el("div", { className: "alarm-editor" });
  const listEl = el("div", { className: "alarm-list" });
  wrap.append(listEl);
  const rows = [];

  function addRow(a = {}) {
    const time = el("input", { type: "time", value: a.time || "07:00", className: "alarm-time" });
    const label = el("input", { type: "text", placeholder: "ラベル（任意）", value: a.label || "", className: "alarm-label" });
    const snooze = el("input", { type: "number", min: "1", max: "60", step: "1", value: String(Math.min(60, Math.max(1, Number(a.snooze_minutes || 5)))), className: "alarm-snooze" });
    const repeat = el("input", { type: "checkbox", checked: a.repeat !== false });
    const enabled = el("input", { type: "checkbox", checked: a.enabled !== false });
    const dayBoxes = ALARM_WEEKDAYS.map(([code, jp]) => {
      const cb = el("input", { type: "checkbox", checked: Array.isArray(a.days) && a.days.includes(code) });
      return { code, cb, label: el("label", { className: "alarm-day" }, [cb, document.createTextNode(jp)]) };
    });
    const del = el("button", { type: "button", className: "alarm-del", textContent: "削除" });
    const row = el("div", { className: "alarm-row" }, [
      el("div", { className: "alarm-row-top" }, [time, el("span", { className: "alarm-days" }, dayBoxes.map(d => d.label))]),
      el("div", { className: "alarm-row-bottom" }, [
        el("label", { className: "alarm-opt" }, [repeat, document.createTextNode("繰り返し")]),
        el("label", { className: "alarm-opt" }, [enabled, document.createTextNode("有効")]),
        el("label", { className: "alarm-opt" }, [snooze, document.createTextNode("分後に再通知")]),
        label,
        del,
      ]),
    ]);
    const entry = {
      id: a.id || `al${Date.now()}${Math.random().toString(36).slice(2, 6)}`,
      time, label, repeat, enabled, snooze, dayBoxes, row,
    };
    del.addEventListener("click", () => {
      listEl.removeChild(row);
      const idx = rows.indexOf(entry);
      if (idx >= 0) rows.splice(idx, 1);
    });
    rows.push(entry);
    listEl.append(row);
  }

  for (const a of alarms) addRow(a);

  const addBtn = el("button", { type: "button", className: "alarm-add", textContent: "＋ 目覚ましを追加" });
  addBtn.addEventListener("click", () => addRow({}));
  const hint = el("div", { className: "hint", textContent: "曜日を選ばないと毎日鳴ります。「繰り返し」OFFは一度だけ鳴って自動で無効になります。アプリ起動中のみ鳴ります。" });
  wrap.append(addBtn, hint);

  alarmEditorState = {
    getJson() {
      const out = rows.map(r => ({
        id: r.id,
        time: r.time.value || "07:00",
        days: r.dayBoxes.filter(d => d.cb.checked).map(d => d.code),
        repeat: r.repeat.checked,
        label: r.label.value.trim(),
        enabled: r.enabled.checked,
        snooze_minutes: Math.min(60, Math.max(1, Number(r.snooze.value || 5))),
      }));
      return JSON.stringify(out);
    },
  };
  return wrap;
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
      for (const tab of tabs.children) {
        const active = tab === button;
        tab.classList.toggle("active", active);
        tab.setAttribute("aria-selected", active ? "true" : "false");
      }
      for (const current of panels.children) {
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

function skillCategory(tool) {
  const name = tool.name || "";
  if (["alarm", "reminder", "focus_timer", "praise_log", "daily_briefing", "clipboard_text", "chitchat"].includes(name)) {
    return "daily";
  }
  if (["get_time", "weather", "currency_rates", "market_indices", "rss_feed", "web_search", "web_fetch"].includes(name)) {
    return "info";
  }
  if (["read_file", "workspace_files", "run_command", "http_api"].includes(name)) {
    return "workspace";
  }
  if (["google_calendar", "google_docs", "google_drive", "google_sheets", "google_tasks", "gmail_message", "microsoft_graph", "slack_send_message", "discord_send_message", "notion", "jira", "linear", "trello", "todoist", "github_issue"].includes(name)) {
    return "connect";
  }
  if (["x_twitter", "mastodon", "bluesky", "reddit", "linkedin", "misskey"].includes(name)) {
    return "social";
  }
  if (["ms_office", "document_convert", "pdf_tools"].includes(name)) {
    return "docs";
  }
  return "other";
}

function renderSkillCard(tool, values) {
  const hasConfig = !!tool.configSchema?.length;
  const name = el("div", { className: "card-name", textContent: hasConfig ? `${tool.label || tool.name}　⚙` : (tool.label || tool.name) });
  const card = el("div", { className: hasConfig ? "card-item card-accordion" : "card-item" }, [
    name,
    el("div", { className: "card-desc", textContent: tool.description || "" }),
  ]);
  const nodes = [card];
  if (hasConfig) {
    const panel = el("div", { className: "card-config" });
    panel.hidden = true;
    if (tool.name === "alarm") {
      // 目覚ましは JSON ではなく、時刻ピッカー＋曜日チェックの専用フォームで編集する。
      panel.append(buildAlarmEditor(values.PICOAGENT_ALARMS));
    } else {
      for (const f of tool.configSchema) panel.append(field(f.key, f.label, f.type, values[f.key], f.placeholder));
    }
    panel.addEventListener("click", event => event.stopPropagation());
    card.addEventListener("click", () => {
      const open = panel.hidden;
      panel.hidden = !open;
      card.classList.toggle("open", open);
    });
    nodes.push(panel);
  }
  return nodes;
}

function buildSkillCategoryTabs(tools, values) {
  const categories = [
    { id: "daily", label: "日常" },
    { id: "info", label: "検索・情報" },
    { id: "workspace", label: "ファイル/実行" },
    { id: "connect", label: "連携" },
    { id: "social", label: "SNS/投稿" },
    { id: "docs", label: "文書" },
    { id: "other", label: "その他" },
  ];
  const grouped = new Map(categories.map(category => [category.id, []]));
  for (const tool of tools) grouped.get(skillCategory(tool)).push(tool);

  const wrap = el("div", { className: "skill-browser" });
  const available = categories.filter(category => grouped.get(category.id).length > 0);
  const tabs = el("div", { className: "tabs sub-tabs", role: "tablist" });
  const panels = el("div", { className: "tab-panels sub-tab-panels" });
  available.forEach((category, index) => {
    const selected = index === 0;
    const toolsInCategory = grouped.get(category.id);
    const button = el("button", {
      className: selected ? "tab sub-tab active" : "tab sub-tab",
      id: `skill-tab-${category.id}`,
      textContent: `${category.label} ${toolsInCategory.length}`,
      type: "button",
    });
    button.setAttribute("role", "tab");
    button.setAttribute("aria-selected", selected ? "true" : "false");
    button.setAttribute("aria-controls", `skill-panel-${category.id}`);
    const panel = el("div", {
      className: selected ? "tab-panel sub-tab-panel active" : "tab-panel sub-tab-panel",
      id: `skill-panel-${category.id}`,
    });
    panel.setAttribute("role", "tabpanel");
    panel.setAttribute("aria-labelledby", button.id);
    if (!selected) panel.hidden = true;
    const list = el("div", { className: "card-list" });
    for (const tool of toolsInCategory) {
      for (const node of renderSkillCard(tool, values)) list.append(node);
    }
    panel.append(list);
    button.addEventListener("click", () => {
      for (const tab of tabs.querySelectorAll(".sub-tab")) {
        const active = tab === button;
        tab.classList.toggle("active", active);
        tab.setAttribute("aria-selected", active ? "true" : "false");
      }
      for (const current of panels.querySelectorAll(".sub-tab-panel")) {
        const active = current === panel;
        current.classList.toggle("active", active);
        current.hidden = !active;
      }
    });
    tabs.append(button);
    panels.append(panel);
  });
  wrap.append(tabs, panels);
  return wrap;
}

async function loadCharacterPersona(id) {
  const entry = characterCatalog.characters.find(character => character.id === id);
  if (!entry) throw new Error(`キャラクター ${id} が一覧にありません`);
  const response = await fetch(`assets/characters/${entry.manifest}?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!response.ok) throw new Error(`キャラクター定義: HTTP ${response.status}`);
  const manifest = await response.json();
  characterDefaultVoices[id] = manifest.voice || {};
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
  const defaultVoice = characterDefaultVoices[id] || {};
  const speaker = characterSpeaker.value.trim();
  const next = { ...current, speaker };
  if (!speaker || speaker === String(defaultVoice.styleId || "")) {
    if (defaultVoice.modelUuid && !next.modelUuid) next.modelUuid = defaultVoice.modelUuid;
    if (defaultVoice.modelName && !next.modelName) next.modelName = defaultVoice.modelName;
    if (defaultVoice.speakerUuid && !next.speakerUuid) next.speakerUuid = defaultVoice.speakerUuid;
    if (defaultVoice.styleName && !next.styleName) next.styleName = defaultVoice.styleName;
    if (defaultVoice.sourceUrl && !next.sourceUrl) next.sourceUrl = defaultVoice.sourceUrl;
  } else {
    delete next.modelUuid;
    delete next.modelName;
    delete next.speakerUuid;
    delete next.styleName;
    delete next.sourceUrl;
  }
  characterVoiceOverrides[id] = next;
}

function loadCharacterVoice(id) {
  const current = characterVoiceOverrides[id] || {};
  const defaultVoice = characterDefaultVoices[id] || {};
  characterSpeaker.value = current.speaker || "";
  characterSpeaker.placeholder = defaultVoice.styleId || inputs.get("PICOAGENT_TTS_SPEAKER")?.value || "888753760";
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
  const modelField = field("PICOAGENT_MODEL", "モデル", "text", v.PICOAGENT_MODEL ?? data.active.model, data.active.model);
  global.append(modelField);
  modelInput = inputs.get("PICOAGENT_MODEL");
  // local はモデルを「叩くAI」タブのプリセットで選ぶので、基本タブのモデル欄（候補に
  // エンジン報告の生IDなど不要な項目が混ざる）は隠す。ollama/openai では表示する。
  function updateModelFieldVisibility() { modelField.hidden = providerSelect.value === "local"; }
  updateModelFieldVisibility();
  global.append(field("PICOAGENT_SYSTEM_PROMPT", "システムプロンプト", "textarea", v.PICOAGENT_SYSTEM_PROMPT ?? "", data.defaults.systemPrompt));
  global.append(field("PICOAGENT_BUBBLE_FONT_PX", "フキダシの文字サイズ(px)", "text", v.PICOAGENT_BUBBLE_FONT_PX ?? "", "13"));
  global.append(field("PICOAGENT_BUBBLE_WIDTH_PX", "フキダシの幅(px)", "text", v.PICOAGENT_BUBBLE_WIDTH_PX ?? "", "ドラッグで設定"));
  global.append(field("PICOAGENT_BUBBLE_HEIGHT_PX", "フキダシの高さ(px)", "text", v.PICOAGENT_BUBBLE_HEIGHT_PX ?? "", "ドラッグで設定"));
  global.append(field("PICOAGENT_ALWAYS_ON_TOP", "常に手前に表示", "checkbox", v.PICOAGENT_ALWAYS_ON_TOP !== false));
  global.append(field("PICOAGENT_MEMORY_ENABLED", "会話から好みや継続指示を暗黙的に覚える", "checkbox", v.PICOAGENT_MEMORY_ENABLED !== false));
  global.append(field("PICOAGENT_MEMORY_MAX_ITEMS", "長期メモリの最大件数", "text", v.PICOAGENT_MEMORY_MAX_ITEMS ?? "", "80"));
  const voiceSec = section("読み上げ");
  voiceSec.append(field("PICOAGENT_TTS_ENABLED", "Aivis で回答を読み上げる", "checkbox", v.PICOAGENT_TTS_ENABLED === true));
  voiceSec.append(selectField("PICOAGENT_TTS_DEVICE", "Aivis 推論デバイス", v.PICOAGENT_TTS_DEVICE || "auto", [
    { value: "auto", label: "自動判定（GPU候補があればGPU）" },
    { value: "cpu", label: "CPU固定" },
    { value: "gpu", label: "GPU固定" },
  ]));
  voiceSec.append(field("PICOAGENT_TTS_PORT", "Aivis ポート", "text", v.PICOAGENT_TTS_PORT ?? "11436", "11436"));
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

  // --- 導入済みスキル（エージェント機能）。設定があるスキルはクリックでその場に開く ---
  const skillSec = section("導入済みスキル（エージェント機能）");
  if (!data.tools.length) {
    skillSec.append(el("p", { className: "hint", textContent: "登録されているスキルはありません。" }));
  } else {
    skillSec.append(el("p", { className: "hint", textContent: "カテゴリを選び、⚙ のあるスキルをクリックすると設定を開けます。" }));
    skillSec.append(buildSkillCategoryTabs(data.tools, v));
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

  // --- 叩く AI コネクタごとの設定（選択中のプロバイダの設定だけ表示する） ---
  const provSec = section("叩く AI コネクタの設定");
  const provBlocks = {};
  for (const p of data.providers) {
    if (!p.configSchema?.length) continue;
    const block = el("div", { className: "prov-block" });
    block.append(el("div", { className: "subhead", textContent: p.label }));
    for (const f of p.configSchema) block.append(field(f.key, f.label, f.type, v[f.key], f.placeholder));
    block.hidden = p.id !== data.active.providerId;
    provBlocks[p.id] = block;
    provSec.append(block);
  }
  const noProvConfig = el("p", { className: "hint", textContent: "このプロバイダに変更できる設定はありません。" });
  provSec.append(noProvConfig);
  function showProviderConfig(id) {
    let shown = false;
    for (const [pid, block] of Object.entries(provBlocks)) {
      block.hidden = pid !== id;
      if (pid === id) shown = true;
    }
    noProvConfig.hidden = shown;
  }
  showProviderConfig(data.active.providerId);
  panels.providers.append(provSec);

  // スキルごとの設定は上のスキル一覧（クリックで展開）に統合済み。

  // プロバイダ変更でモデル候補を更新し、選択中プロバイダの設定だけ表示する。
  providerSelect.addEventListener("change", () => {
    loadModels(providerSelect.value, true);
    showProviderConfig(providerSelect.value);
    updateModelFieldVisibility();
  });
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
    logApp("warn", "モデル一覧を取得できませんでした", errorDetail(error));
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
  if (alarmEditorState) patch.PICOAGENT_ALARMS = alarmEditorState.getJson();
  return patch;
}

// 再起動の確認画面を出し、ユーザーの選択（true=今すぐ再起動）を返す。
// Tauri webview では window.confirm が使えないことがあるため自前の画面を出す。
function confirmRestart() {
  return new Promise((resolve) => {
    const overlay = el("div", { className: "restart-overlay" });
    const card = el("div", { className: "restart-card" });
    card.append(
      el("h2", { textContent: "再起動して反映します" }),
      el("p", { textContent: "選んだ設定（叩く AI・モデル・音声・キャラクターなど）は、PicoAgent を再起動して起動画面でまとめて反映します。今すぐ再起動しますか？" }),
    );
    const actions = el("div", { className: "restart-actions" });
    const later = el("button", { className: "btn", textContent: "あとで", type: "button" });
    const now = el("button", { className: "btn primary", textContent: "今すぐ再起動", type: "button" });
    const close = (value) => { overlay.remove(); resolve(value); };
    later.addEventListener("click", () => close(false));
    now.addEventListener("click", () => close(true));
    actions.append(later, now);
    card.append(actions);
    overlay.append(card);
    document.body.append(overlay);
    now.focus();
  });
}

saveBtn.addEventListener("click", async () => {
  saveBtn.disabled = true;
  statusEl.textContent = "保存中…";
  try {
    const res = await window.picoagent.settings.save(collect());
    if (res.requiresRestart) {
      const restartNow = await confirmRestart();
      if (restartNow) {
        statusEl.textContent = "反映のため再起動しています…";
        // 変わった部分だけ立ち上げ直す（キャラ変更だけなら LLM は再起動しない）。
        await window.picoagent.restartApp(res.restartEngine, res.restartAivis);
        statusEl.textContent = "反映しました（起動画面で再読み込み中）";
      } else {
        statusEl.textContent = "保存しました（次回の起動時に反映されます）";
      }
    } else {
      statusEl.textContent = res.up ? "保存しました（接続OK）" : "保存しました（叩く AI に未接続）";
    }
  } catch (error) {
    logApp("error", "設定保存に失敗しました", errorDetail(error));
    statusEl.textContent = `保存に失敗: ${error.message}`;
  } finally {
    saveBtn.disabled = false;
  }
});

(async () => {
  data = await window.picoagent.settings.load();
  try { await render(); }
  catch (error) {
    logApp("error", "設定画面の読み込みに失敗しました", errorDetail(error));
    statusEl.textContent = `設定画面の読み込みに失敗: ${error.message}`;
  }
})().catch(error => {
  logApp("error", "設定初期化に失敗しました", errorDetail(error));
  statusEl.textContent = `設定初期化に失敗: ${error?.message || error}`;
});
