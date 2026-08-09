// settings.js - 設定ウィンドウのロジック。
// main から渡されるグローバル設定 + 各コネクタの configSchema を動的に描画し、
// 保存時に { 設定キー: 値 } のフラットなパッチを main へ送る。

const form = document.getElementById("form");
const statusEl = document.getElementById("status");
const saveBtn = document.getElementById("save");
const resetChangesBtn = document.getElementById("resetChanges");
const modelOptions = document.getElementById("modelOptions");
const settingsSearch = document.getElementById("settingsSearch");
const clearSettingsSearch = document.getElementById("clearSettingsSearch");
const dirtySummary = document.getElementById("dirtySummary");
const { errorDetail, logApp } = window.PicoUI;

let data; // settings:load の結果
const inputs = new Map(); // key -> input 要素
const inputWrappers = new Map(); // key -> field/card 要素
let savedSnapshot = null; // 最後に読み込んだ、または保存できた設定値
let renderingSettings = false;
let characterOverrides = {};
let characterVoiceOverrides = {};
let characterLive2dSdkOverrides = {};
let characterFraming = {};
let characterFramingEditor = null;
let characterStreamProfiles = {};
let characterStreamProfileEditor = null;
let characterDefaultVoices = {};
let characterSelect;
let userCharacterRoot = ""; // 追加キャラクターの置き場（案内表示用）
let tha3ProgressUnlisten = null;
let componentProgressUnlisten = null;
let updateProgressUnlisten = null;
let tha3GenerationRunning = false;
let characterPersona;
let characterSpeaker;
let characterLive2dSdkSelect;
let characterLive2dSdkField;
let characterLive2dSdkHint;
let characterCatalog;
let characterAddGroup;
const characterManifestCache = new Map();
let modelInput;
let alarmEditorState = null; // 目覚ましエディタ（保存時に PICOAGENT_ALARMS を組み立てる）
let scheduleEditorState = null; // スケジュールタスクエディタ（保存時に PICOAGENT_SCHEDULED_TASKS を組み立てる）
const characterManifestCacheKey = "2026-07-03-live2d-jp-voice-v1";
const noSearchResultMessage = "検索に一致する設定はありません";
const SETTINGS_TAB_IDS = new Set(["basic", "display", "character", "components", "providers", "tools", "permissions", "skillguide", "diagnostics"]);
const LAYOUT_CHOICES = [
  { id: "pet", label: "ペット", size: "900×600", description: "キャラと会話をゆったり表示。普段使い向け" },
  { id: "mascot", label: "ミニ", size: "360×560", description: "キャラ中心。必要な時だけ会話する" },
  { id: "chat", label: "チャット", size: "520×760", description: "履歴を読みながら入力する" },
  { id: "talk", label: "トーク", size: "520×760", description: "メッセージアプリ風に会話する" },
  { id: "bar", label: "バー", size: "900×320", description: "呼び出してすばやく質問する" },
  { id: "workspace", label: "作業", size: "1400×850", description: "履歴・回答・実行状況を並べて見る" },
  { id: "stream", label: "配信", size: "1560×920", description: "キャラを大きく、字幕を見やすく映す" },
  { id: "widget", label: "ウィジェット", size: "440×600", description: "時計・通知・入力をまとめて置く" },
];
const requestedSettingsTab = new URLSearchParams(window.location.search).get("tab");
let activeSettingsTab = requestedSettingsTab === "voice" || requestedSettingsTab === "character-add"
  ? "character"
  : (SETTINGS_TAB_IDS.has(requestedSettingsTab) ? requestedSettingsTab : "basic");

function activateSettingsTab(tabId) {
  const focusCharacterAdd = tabId === "character-add";
  if (focusCharacterAdd) tabId = "character";
  if (tabId === "voice") tabId = "character";
  if (!SETTINGS_TAB_IDS.has(tabId)) return false;
  activeSettingsTab = tabId;
  const button = document.getElementById(`tab-${tabId}`);
  if (button) {
    button.click();
    form.scrollTop = 0;
    if (focusCharacterAdd) {
      requestAnimationFrame(() => {
        if (!characterAddGroup) return;
        characterAddGroup.open = true;
        characterAddGroup.scrollIntoView({ block: "start" });
      });
    }
  }
  return true;
}

window.addEventListener("error", event => {
  logApp("error", "設定画面の未捕捉エラー", `${event.message || ""} ${event.filename || ""}:${event.lineno || ""}:${event.colno || ""}\n${errorDetail(event.error)}`);
});
window.addEventListener("unhandledrejection", event => {
  logApp("error", "設定画面の未処理 Promise rejection", errorDetail(event.reason));
});

window.picoagent.listen("settings:navigate", tabId => {
  activateSettingsTab(String(tabId || ""));
}).catch(error => {
  logApp("warn", "設定タブ遷移イベントの購読に失敗しました", errorDetail(error));
});

window.addEventListener("beforeunload", () => {
  if (tha3ProgressUnlisten) tha3ProgressUnlisten();
  if (componentProgressUnlisten) componentProgressUnlisten();
  if (updateProgressUnlisten) updateProgressUnlisten();
  // フォールバック許可: ウィンドウ破棄中は応答を表示できない。バックエンド側のkill_on_dropも最終防壁になる。
  if (tha3GenerationRunning) window.picoagent.settings.cancelTha3Generation().catch(() => {});
});

function el(tag, props = {}, children = []) {
  const node = Object.assign(document.createElement(tag), props);
  for (const c of children) node.append(c);
  return node;
}

function searchText(...parts) {
  return parts
    .filter(part => part != null)
    .map(part => String(part).toLowerCase())
    .join(" ");
}

function markSearchable(node, ...parts) {
  node.classList.add("searchable");
  node.dataset.search = searchText(...parts);
  return node;
}

function registerInput(key, input, wrapper) {
  wrapper.dataset.settingKey = key;
  inputs.set(key, input);
  inputWrappers.set(key, wrapper);
  return wrapper;
}

function unregisterInputsIn(node) {
  if (!node) return;
  for (const wrapper of node.querySelectorAll("[data-setting-key]")) {
    const key = wrapper.dataset.settingKey;
    if (!key) continue;
    inputs.delete(key);
    inputWrappers.delete(key);
  }
}

function cloneSettings(value) {
  return JSON.parse(JSON.stringify(value || {}));
}

function stableStringify(value) {
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${stableStringify(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function updateDirtyState() {
  if (renderingSettings || !savedSnapshot || !resetChangesBtn) return;
  const current = collect();
  const changedKeys = new Set(
    [...Object.keys(savedSnapshot), ...Object.keys(current)]
      .filter(key => stableStringify(current[key]) !== stableStringify(savedSnapshot[key]))
  );
  const dirty = changedKeys.size > 0;
  for (const [key, wrapper] of inputWrappers) {
    const currentValue = current[key];
    const savedValue = savedSnapshot[key];
    wrapper.classList.toggle("changed", stableStringify(currentValue) !== stableStringify(savedValue));
  }
  characterPersona?.closest(".field")?.classList.toggle(
    "changed",
    stableStringify(current.PICOAGENT_CHARACTER_OVERRIDES) !== stableStringify(savedSnapshot.PICOAGENT_CHARACTER_OVERRIDES)
  );
  characterSpeaker?.closest(".field")?.classList.toggle(
    "changed",
    stableStringify(current.PICOAGENT_TTS_CHARACTER_OVERRIDES) !== stableStringify(savedSnapshot.PICOAGENT_TTS_CHARACTER_OVERRIDES)
  );
  characterLive2dSdkField?.classList.toggle(
    "changed",
    stableStringify(current.PICOAGENT_LIVE2D_SDK_OVERRIDES) !== stableStringify(savedSnapshot.PICOAGENT_LIVE2D_SDK_OVERRIDES)
  );
  characterFramingEditor?.node?.classList.toggle(
    "changed",
    stableStringify(current.PICOAGENT_CHARACTER_FRAMING) !== stableStringify(savedSnapshot.PICOAGENT_CHARACTER_FRAMING)
  );
  form.querySelector(".alarm-editor")?.closest(".card-config")?.classList.toggle(
    "changed",
    stableStringify(current.PICOAGENT_ALARMS) !== stableStringify(savedSnapshot.PICOAGENT_ALARMS)
  );
  form.querySelector(".schedule-editor")?.closest(".card-config")?.classList.toggle(
    "changed",
    stableStringify(current.PICOAGENT_SCHEDULED_TASKS) !== stableStringify(savedSnapshot.PICOAGENT_SCHEDULED_TASKS)
  );
  // 折りたたんだ「詳細設定」の中を変更しても、閉じている間は .changed の色が見えない。
  // どこを触ったか分からないまま保存することになるので、見出し側にも印を出す。
  for (const group of form.querySelectorAll(".advanced-group")) {
    group.classList.toggle("has-changed", Boolean(group.querySelector(".changed")));
  }
  resetChangesBtn.disabled = !dirty;
  saveBtn.classList.toggle("dirty", dirty);
  saveBtn.textContent = dirty ? `変更を保存（${changedKeys.size}件）` : "変更を保存";
  if (dirtySummary) {
    dirtySummary.textContent = dirty ? `未保存 ${changedKeys.size}件` : "保存済み";
    dirtySummary.classList.toggle("dirty", dirty);
  }
}

function nodeSearchText(node) {
  return searchText(node.dataset.search || "", node.textContent || "");
}

function currentActivePanel(group) {
  return group?.querySelector(":scope > .tab-panel.active") || group?.querySelector(":scope > .tab-panel");
}

// ジャンル（カテゴリ）フィルタの選択状態。"all" は全表示。スキル設定と活用ガイドで独立に持つ。
let skillGenreFilter = "all";
let guideGenreFilter = "all";
let characterTypeFilter = "all";
let characterSearchQuery = "";

// ジャンル別フィルタのチップ列を作る。クリックで getValue/setValue 経由の状態を切り替え、再フィルタする。
function buildGenreChips(getValue, setValue) {
  const ids = ["all", ...Object.keys(SKILL_CATEGORY_LABELS)];
  const bar = el("div", { className: "genre-filter" });
  const buttons = new Map();
  function refresh() {
    for (const [id, button] of buttons) button.classList.toggle("active", id === getValue());
  }
  for (const id of ids) {
    const label = id === "all" ? "すべて" : (SKILL_CATEGORY_LABELS[id] || id);
    const button = el("button", { type: "button", className: "genre-chip", textContent: label });
    button.addEventListener("click", () => {
      setValue(id);
      refresh();
      applySettingsSearch();
    });
    buttons.set(id, button);
    bar.append(button);
  }
  refresh();
  return bar;
}

function applySettingsSearch() {
  if (!settingsSearch) return;
  const query = settingsSearch.value.trim().toLowerCase();
  document.body.classList.toggle("searching", Boolean(query));
  const topPanels = form.querySelector(":scope > .tab-panels");
  const topActive = currentActivePanel(topPanels);
  for (const panel of form.querySelectorAll(".tab-panel")) {
    if (!query) {
      const parent = panel.parentElement;
      const active = parent === topPanels ? panel === topActive : panel.classList.contains("active");
      panel.hidden = !active;
    } else {
      panel.hidden = false;
    }
  }
  for (const item of form.querySelectorAll(".searchable")) {
    // スキル設定の ⚙ パネルはアコーディオンが開閉を管理するため、ここでは触らない（検索時のみ下で開く）。
    if (item.classList.contains("card-config")) continue;
    const conditionHidden = item.dataset.conditionHidden === "true";
    item.hidden = conditionHidden || (query ? !nodeSearchText(item).includes(query) : false);
  }
  applyCharacterFilters(query);
  // ⚙ パネル: 検索中はマッチしたものだけ開き、非検索時はアコーディオンの開閉状態（.open）に従う。
  for (const panel of form.querySelectorAll(".card-config")) {
    const card = panel.previousElementSibling;
    if (query) {
      const open = nodeSearchText(panel).includes(query);
      panel.hidden = !open;
      if (card) {
        card.classList.toggle("open", open);
        if (open) card.hidden = false;
      }
    } else {
      panel.hidden = !(card && card.classList.contains("open"));
    }
  }
  // ジャンルフィルタ: スキル設定タブのカテゴリ群を絞る（検索と併用可能）。
  // ジャンル不一致、または検索中にそのカテゴリの可視カードが無ければ見出しごと隠す。
  for (const group of form.querySelectorAll(".skill-cat-group")) {
    const genreHidden = skillGenreFilter !== "all" && group.dataset.genre !== skillGenreFilter;
    const noMatch = Boolean(query) && ![...group.querySelectorAll(".card-item.searchable")].some(card => !card.hidden);
    group.hidden = genreHidden || noMatch;
  }
  // ジャンルフィルタ: 活用ガイドの行を絞る（検索結果にさらにジャンル条件を重ねる）。
  if (guideGenreFilter !== "all") {
    for (const row of form.querySelectorAll(".skill-guide-row")) {
      if (row.dataset.genre !== guideGenreFilter) row.hidden = true;
    }
  }
  for (const sectionNode of form.querySelectorAll(".section")) {
    if (!query) {
      sectionNode.hidden = false;
      continue;
    }
    const ownMatch = nodeSearchText(sectionNode.querySelector("h2") || sectionNode).includes(query);
    const childMatch = [...sectionNode.querySelectorAll(".searchable")].some(item => !item.hidden);
    sectionNode.hidden = !(ownMatch || childMatch);
  }
  // 「詳細設定」などの折りたたみ: 中身が一致したら開いて見せ、一致しなければ畳んで隠す。
  // 開かないと検索でヒットした項目が summary の裏に残り、見つからないのと同じになる。
  // 隠さないと、中身が全部消えたタブが「まだ何かある」ように見えて空判定も効かなくなる。
  for (const group of form.querySelectorAll(".advanced-group")) {
    if (query) {
      // 手で開閉した状態は検索終了時に戻す。
      if (group.dataset.userOpen == null) group.dataset.userOpen = group.open ? "1" : "0";
      const matched = [...group.querySelectorAll(".searchable, .section")].some(item => !item.hidden);
      group.hidden = !matched;
      group.open = matched;
    } else {
      group.hidden = false;
      if (group.dataset.userOpen != null) {
        group.open = group.dataset.userOpen === "1";
        delete group.dataset.userOpen;
      }
    }
  }
  for (const panel of form.querySelectorAll(".tab-panel")) {
    if (!query) continue;
    const hasVisibleContent = [...panel.children].some(child => !child.hidden);
    panel.hidden = !hasVisibleContent;
  }
  if (query && !form.querySelector(".tab-panel:not([hidden])")) {
    statusEl.textContent = noSearchResultMessage;
  } else if (statusEl.textContent === noSearchResultMessage) {
    statusEl.textContent = "";
  }
}

const MODEL_GROUPS = {
  OPENAI_MODEL: [
    {
      label: "GPT-5.5",
      options: [
        { value: "gpt-5.5", label: "標準（最高品質）", hint: "OpenAI の最上位候補。品質優先のときに使います。" },
      ],
    },
    {
      label: "GPT-5.4",
      options: [
        { value: "gpt-5.4", label: "標準", hint: "品質と速度のバランス候補です。" },
        { value: "gpt-5.4-mini", label: "mini", hint: "低コスト・低遅延の既定候補です。" },
        { value: "gpt-5.4-nano", label: "nano", hint: "最軽量候補です。" },
      ],
    },
  ],
  PICOAGENT_ANTHROPIC_MODEL: [
    {
      label: "Claude Opus",
      options: [
        { value: "claude-opus-4-8", label: "4.8", hint: "Claude の高品質候補です。" },
      ],
    },
    {
      label: "Claude Sonnet",
      options: [
        { value: "claude-sonnet-4-6", label: "4.6", hint: "Claude の標準候補です。" },
      ],
    },
    {
      label: "Claude Haiku",
      options: [
        { value: "claude-haiku-4-5", label: "4.5", hint: "Claude の軽量候補です。" },
      ],
    },
  ],
  PICOAGENT_DEEPSEEK_MODEL: [
    {
      label: "DeepSeek V4",
      options: [
        { value: "deepseek-v4-flash", label: "Flash", hint: "DeepSeek の標準・低コスト候補です。" },
        { value: "deepseek-v4-pro", label: "Pro", hint: "DeepSeek の高品質候補です。" },
      ],
    },
    {
      label: "旧互換",
      options: [
        { value: "deepseek-chat", label: "chat（旧互換）", hint: "旧 API 互換名です。新規設定は V4 Flash 推奨です。" },
        { value: "deepseek-reasoner", label: "reasoner（旧互換）", hint: "旧 API 互換名です。新規設定は V4 Flash + Thinking 推奨です。" },
      ],
    },
  ],
  PICOAGENT_GEMINI_MODEL: [
    {
      label: "Gemini 2.5",
      options: [
        { value: "gemini-2.5-flash", label: "Flash", hint: "速度と費用を抑える候補です。" },
        { value: "gemini-2.5-pro", label: "Pro", hint: "品質優先の候補です。" },
      ],
    },
  ],
  PICOAGENT_XAI_MODEL: [
    {
      label: "Grok",
      options: [
        { value: "grok-4.3", label: "4.3", hint: "xAI の汎用主要候補です。" },
        { value: "grok-build-0.1", label: "Build 0.1", hint: "コーディング用途向けの候補です。" },
      ],
    },
  ],
  PICOAGENT_MISTRAL_MODEL: [
    {
      label: "Mistral",
      options: [
        { value: "mistral-medium-latest", label: "Medium", hint: "Mistral の標準候補です。" },
        { value: "mistral-small-latest", label: "Small", hint: "軽量候補です。" },
        { value: "mistral-large-latest", label: "Large", hint: "高品質候補です。" },
      ],
    },
    {
      label: "Ministral",
      options: [
        { value: "ministral-14b-latest", label: "14B", hint: "小型モデルの高品質候補です。" },
        { value: "ministral-8b-latest", label: "8B", hint: "小型モデルの標準候補です。" },
        { value: "ministral-3b-latest", label: "3B", hint: "小型モデルの軽量候補です。" },
      ],
    },
  ],
};

function groupedModelField(key, label, value, groups, options = {}) {
  const hidden = el("input", { type: "hidden", value: value || "" });

  const family = el("select");
  const size = el("select");
  const custom = el("input", { type: "text", placeholder: "モデルIDを直接入力", value: value || "" });
  const hint = el("div", { className: "hint" });
  const wrapper = el("div", { className: "field model-picker" }, [
    el("label", { textContent: label }),
    family,
    size,
    custom,
    hint,
    hidden,
  ]);
  markSearchable(wrapper, key, label, groups.map(group => group.label).join(" "));
  registerInput(key, hidden, wrapper);
  const customGroup = { label: "カスタム", options: [{ value: "__custom__", label: "直接入力", hint: "一覧にないモデルIDを入力します。" }] };
  const allGroups = options.allowCustom === false ? groups : [...groups, customGroup];
  let initialFamily = allGroups.findIndex(group => group.options.some(option => option.value === value));
  if (initialFamily < 0) initialFamily = value ? allGroups.length - 1 : 0;

  function rebuildSize() {
    const group = allGroups[Number(family.value)] || allGroups[0];
    size.innerHTML = "";
    for (const option of group.options) {
      size.append(el("option", {
        value: option.value,
        textContent: option.label,
        selected: option.value === hidden.value,
      }));
    }
    const customMode = group === customGroup;
    size.hidden = customMode;
    custom.hidden = !customMode;
    if (customMode) {
      hidden.value = custom.value.trim();
      hint.textContent = customGroup.options[0].hint;
      return;
    }
    if (!group.options.some(option => option.value === hidden.value)) {
      hidden.value = group.options[0]?.value || "";
    }
    size.value = hidden.value;
    hint.textContent = group.options.find(option => option.value === hidden.value)?.hint || "";
  }

  for (const [index, group] of allGroups.entries()) {
    family.append(el("option", {
      value: String(index),
      textContent: group.label,
      selected: index === initialFamily,
    }));
  }
  family.addEventListener("change", rebuildSize);
  size.addEventListener("change", () => {
    hidden.value = size.value;
    const group = allGroups[Number(family.value)] || allGroups[0];
    hint.textContent = group.options.find(option => option.value === hidden.value)?.hint || "";
  });
  custom.addEventListener("input", () => { hidden.value = custom.value.trim(); });
  rebuildSize();
  return wrapper;
}

// 1つの設定項目を描画して inputs に登録する。
function field(key, label, type, value, placeholder) {
  if (key === "PICOAGENT_LOCAL_MODEL_PRESET") {
    return localModelPresetField(key, label, value || "auto");
  }
  if (MODEL_GROUPS[key]) {
    return groupedModelField(key, label, value || placeholder || "", MODEL_GROUPS[key]);
  }
  if (type === "textarea") {
    const ta = el("textarea", { value: value ?? "", placeholder: placeholder || "" });
    return registerInput(key, ta, markSearchable(el("div", { className: "field" }, [el("label", { textContent: label }), ta]), key, label, placeholder));
  }
  if (type === "checkbox") {
    const cb = el("input", { type: "checkbox", checked: Boolean(value) });
    return registerInput(key, cb, markSearchable(el("div", { className: "field inline" }, [cb, el("label", { textContent: label })]), key, label));
  }
  const inputType = type === "password" ? "password" : type === "number" ? "number" : "text";
  const input = el("input", { type: inputType, value: value ?? "", placeholder: placeholder || "" });
  if (key === "PICOAGENT_TTS_VOLUME") {
    input.min = "0";
    input.max = "100";
    input.step = "1";
  }
  if (key === "PICOAGENT_MODEL") input.setAttribute("list", "modelOptions");
  return registerInput(key, input, markSearchable(el("div", { className: "field" }, [el("label", { textContent: label }), input]), key, label, placeholder));
}

function localModelPresetField(key, label, value) {
  const presets = data.localModelPresets || [];
  const localPresetLabel = (preset) => `${preset.localSize || preset.label} / 推奨VRAM ${preset.recommendedVramGb || 0}GB以上 / RAM ${preset.recommendedRamGb || 0}GB以上 / DL ${preset.downloadSize || "-"}`;
  const modelFamilies = ["Gemma", "Qwen", "Llama", "Mistral", "Phi", "DeepSeek"];
  const presetGroups = modelFamilies.map((family) => ({
    label: family,
    options: presets.filter(preset => preset.family === family).map(preset => ({
      value: preset.id,
      label: localPresetLabel(preset),
      hint: localModelPresetHint(preset.id),
    })),
  }));
  const groups = [
    {
      label: "自動",
      options: [{ value: "auto", label: "PC性能から自動選択", hint: localModelPresetHint("auto") }],
    },
    ...presetGroups,
    {
      label: "カスタム",
      options: [{ value: "custom", label: "URL指定", hint: localModelPresetHint("custom") }],
    },
  ];
  return groupedModelField(key, label, value, groups.filter(group => group.options.length > 0), { allowCustom: false });
}

function localModelPresetHint(value) {
  if (value === "auto") return "初回起動時にVRAM/RAM目安で small / balanced / large を選び、以後は選定結果を保存します。";
  if (value === "custom") return "下のカスタムモデルURLを使います。GGUFファイルの直接URLを指定してください。";
  const preset = (data.localModelPresets || []).find(item => item.id === value);
  if (!preset) return "";
  return `${preset.note} 推奨VRAM: ${preset.recommendedVramGb}GB以上、推奨RAM: ${preset.recommendedRamGb}GB以上、ダウンロード目安: ${preset.downloadSize}`;
}

// 保存済み JSON を配列として読む。壊れていた場合は空扱いにせず、その事実を返す。
//
// 空一覧で開始してそのまま保存すると、**登録済みの目覚まし・スケジュールが黙って全部消える**
// （設定ファイル破損時に「空で継続」してしまう典型の失敗）。読めなかったときは元の値を
// 温存し、ユーザーに知らせる。
function parseStoredList(currentJson) {
  const raw = currentJson || "[]";
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) return { items: parsed, broken: false, raw };
  } catch { /* 下の broken 扱いへ */ }
  return { items: [], broken: true, raw };
}

// 壊れていたことを知らせる警告表示。
function brokenListNotice(label) {
  return el("div", {
    className: "hint",
    textContent: `⚠ 保存済みの${label}を読み取れませんでした。上書きで失わないよう、ここで編集して保存するまで元の内容はそのまま残します。`,
  });
}

// 目覚ましの専用エディタ（時刻ピッカー＋曜日チェック＋繰り返し/有効/ラベル/削除/追加）。
// 保存時は alarmEditorState.getJson() で PICOAGENT_ALARMS（JSON文字列）を組み立てる。
const ALARM_WEEKDAYS = [
  ["mon", "月"], ["tue", "火"], ["wed", "水"], ["thu", "木"],
  ["fri", "金"], ["sat", "土"], ["sun", "日"],
];
function buildAlarmEditor(currentJson) {
  const stored = parseStoredList(currentJson);
  const alarms = stored.items;
  let touched = false;

  const wrap = el("div", { className: "alarm-editor" });
  if (stored.broken) wrap.append(brokenListNotice("目覚まし"));
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
  addBtn.addEventListener("click", () => { touched = true; addRow({}); });
  const hint = el("div", { className: "hint", textContent: "曜日を選ばないと毎日鳴ります。「繰り返し」OFFは一度だけ鳴って自動で無効になります。アプリ起動中のみ鳴ります。" });
  wrap.append(addBtn, hint);

  alarmEditorState = {
    getJson() {
      // 読めなかった内容を、ユーザーが何も編集していないのに空で上書きしない。
      if (stored.broken && !touched) return stored.raw;
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

const SCHEDULE_WEEKDAYS = ALARM_WEEKDAYS;
function buildScheduleEditor(currentJson) {
  const stored = parseStoredList(currentJson);
  const tasks = stored.items;
  let touched = false;

  const wrap = el("div", { className: "schedule-editor" });
  if (stored.broken) wrap.append(brokenListNotice("スケジュール"));
  const listEl = el("div", { className: "schedule-list" });
  const rows = [];
  wrap.append(listEl);

  function addRow(task = {}) {
    const kind = el("select", { className: "schedule-kind" }, [
      el("option", { value: "tool", textContent: "スキル", selected: (task.kind || "tool") === "tool" }),
      el("option", { value: "command", textContent: "コマンド", selected: task.kind === "command" }),
    ]);
    const time = el("input", { type: "time", value: task.time || "09:00", className: "schedule-time" });
    const cron = el("input", { type: "text", value: task.cron || "", placeholder: "cron 任意: 0 9 * * 1", className: "schedule-cron" });
    const label = el("input", { type: "text", value: task.label || "", placeholder: "ラベル", className: "schedule-label" });
    const toolName = el("input", { type: "text", value: task.tool_name || "weather", placeholder: "スキル名 例: weather", className: "schedule-tool" });
    const toolArgs = el("textarea", { value: JSON.stringify(task.tool_args || {}, null, 2), placeholder: "{ }", className: "schedule-json" });
    const program = el("input", { type: "text", value: task.program || "git", placeholder: "program", className: "schedule-program" });
    const args = el("input", { type: "text", value: Array.isArray(task.args) ? task.args.join(" ") : "", placeholder: "引数（空白区切り）", className: "schedule-args" });
    const cwd = el("input", { type: "text", value: task.cwd || "", placeholder: "作業ディレクトリ（任意）", className: "schedule-cwd" });
    const timeout = el("input", { type: "number", min: "1", max: "120", step: "1", value: String(task.timeout_secs || ""), placeholder: "秒", className: "schedule-timeout" });
    const repeat = el("input", { type: "checkbox", checked: task.repeat !== false });
    const enabled = el("input", { type: "checkbox", checked: task.enabled !== false });
    const dayBoxes = SCHEDULE_WEEKDAYS.map(([code, jp]) => {
      const cb = el("input", { type: "checkbox", checked: Array.isArray(task.days) && task.days.includes(code) });
      return { code, cb, label: el("label", { className: "alarm-day" }, [cb, document.createTextNode(jp)]) };
    });
    const target = el("div", { className: "schedule-target" });
    const commandFields = el("div", { className: "schedule-command-fields" }, [program, args, cwd, timeout]);
    const toolFields = el("div", { className: "schedule-tool-fields" }, [toolName, toolArgs]);
    target.append(toolFields, commandFields);
    const del = el("button", { type: "button", className: "alarm-del", textContent: "削除" });
    const row = el("div", { className: "schedule-row" }, [
      el("div", { className: "schedule-row-top" }, [kind, time, cron, label]),
      el("div", { className: "alarm-days" }, dayBoxes.map(d => d.label)),
      target,
      el("div", { className: "schedule-row-bottom" }, [
        el("label", { className: "alarm-opt" }, [repeat, document.createTextNode("繰り返し")]),
        el("label", { className: "alarm-opt" }, [enabled, document.createTextNode("有効")]),
        del,
      ]),
    ]);
    const entry = { id: task.id || `st${Date.now()}${Math.random().toString(36).slice(2, 6)}`, row, kind, time, cron, label, toolName, toolArgs, program, args, cwd, timeout, repeat, enabled, dayBoxes, toolFields, commandFields };
    function updateKind() {
      const isTool = kind.value === "tool";
      toolFields.hidden = !isTool;
      commandFields.hidden = isTool;
    }
    kind.addEventListener("change", updateKind);
    del.addEventListener("click", () => {
      listEl.removeChild(row);
      const idx = rows.indexOf(entry);
      if (idx >= 0) rows.splice(idx, 1);
    });
    updateKind();
    rows.push(entry);
    listEl.append(row);
  }

  for (const task of tasks) addRow(task);
  const addBtn = el("button", { type: "button", className: "alarm-add", textContent: "＋ スケジュールを追加" });
  addBtn.addEventListener("click", () => { touched = true; addRow({ kind: "tool", tool_name: "weather", tool_args: {} }); });
  const hint = el("div", { className: "hint", textContent: "cron を空にすると時刻と曜日で実行します。スキル実行はコマンド許可なしで使えます。コマンド実行は許可ルートと allowlist の制限を受けます。" });
  wrap.append(addBtn, hint);

  scheduleEditorState = {
    getJson() {
      // 読めなかった内容を、ユーザーが何も編集していないのに空で上書きしない。
      if (stored.broken && !touched) return stored.raw;
      const out = rows.map(r => {
        let parsedArgs = {};
        try { parsedArgs = JSON.parse(r.toolArgs.value || "{}"); }
        catch {
          throw new Error(`スケジュール「${r.label.value.trim() || r.toolName.value.trim() || r.program.value.trim() || r.id}」の tool_args が JSON として読めません。`);
        }
        return {
          id: r.id,
          kind: r.kind.value,
          time: r.cron.value.trim() ? "" : (r.time.value || "09:00"),
          cron: r.cron.value.trim(),
          days: r.dayBoxes.filter(d => d.cb.checked).map(d => d.code),
          repeat: r.repeat.checked,
          label: r.label.value.trim(),
          enabled: r.enabled.checked,
          program: r.program.value.trim(),
          args: r.args.value.trim().split(/\s+/).filter(Boolean),
          cwd: r.cwd.value.trim(),
          timeout_secs: r.timeout.value ? Math.min(120, Math.max(1, Number(r.timeout.value))) : null,
          tool_name: r.toolName.value.trim(),
          tool_args: parsedArgs,
          last_run_minute: "",
        };
      });
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
  return registerInput(key, select, markSearchable(
    el("div", { className: "field" }, [el("label", { textContent: label }), select]),
    key,
    label,
    options.map(option => option.label).join(" ")
  ));
}

function layoutPickerField(value) {
  const select = el("select", { className: "layout-native-select", ariaLabel: "画面レイアウト" });
  const cards = el("div", { className: "layout-card-grid" });
  const livePreview = el("div", { className: "layout-live-preview" }, [
    el("span", { textContent: "選択中レイアウトの実画面プレビュー" }),
    el("img", { alt: "選択中レイアウトの実画面プレビュー" }),
  ]);
  const buttons = new Map();
  for (const layout of LAYOUT_CHOICES) {
    select.append(el("option", { value: layout.id, textContent: layout.label, selected: layout.id === value }));
    const badge = el("span", { className: "layout-selected-badge", textContent: "選択中" });
    const button = el("button", {
      className: "layout-card",
      type: "button",
      title: `${layout.label}へ変更`,
    }, [
      el("img", {
        src: `assets/layout-previews/${layout.id}.webp`,
        alt: `${layout.label}表示のプレビュー`,
        loading: "lazy",
      }),
      el("span", { className: "layout-card-copy" }, [
        el("span", { className: "layout-card-heading" }, [
          el("strong", { textContent: layout.label }),
          el("small", { textContent: layout.size }),
        ]),
        el("span", { className: "layout-card-description", textContent: layout.description }),
      ]),
      badge,
    ]);
    button.addEventListener("click", () => {
      select.value = layout.id;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    cards.append(button);
    buttons.set(layout.id, button);
  }
  const sync = () => {
    for (const [id, button] of buttons) {
      const selected = id === select.value;
      button.classList.toggle("selected", selected);
      button.setAttribute("aria-pressed", selected ? "true" : "false");
    }
    livePreview.querySelector("img").src = `assets/layout-previews/${select.value}.webp`;
  };
  select.addEventListener("change", sync);
  sync();
  const wrapper = markSearchable(el("div", { className: "layout-picker" }, [
    el("p", {
      className: "layout-picker-hint",
      textContent: "使い方に近い画面を選んでください。保存すると現在のウィンドウへすぐ反映します。",
    }),
    select,
    livePreview,
    cards,
  ]), "PICOAGENT_LAYOUT", "画面 レイアウト 表示形式", ...LAYOUT_CHOICES.map(layout => `${layout.label} ${layout.description}`));
  return registerInput("PICOAGENT_LAYOUT", select, wrapper);
}

function quickSettingsGuide(v) {
  const layout = LAYOUT_CHOICES.find(choice => choice.id === (v.PICOAGENT_LAYOUT || "pet"));
  const provider = data.providers.find(item => item.id === (v.PICOAGENT_PROVIDER || data.active.providerId));
  const cards = [
    { tab: "character", title: "キャラと声", summary: `${v.PICOAGENT_CHARACTER || "既定キャラ"}・${v.PICOAGENT_TTS_ENABLED === true ? "読み上げ有効" : "ミュート中"}` },
    { tab: "display", title: "画面レイアウト", summary: layout?.label || "ペット" },
    { tab: "providers", title: "使うAI", summary: provider?.label || v.PICOAGENT_PROVIDER || "ローカル" },
    { tab: "components", title: "追加機能", summary: "音声・文字起こし・文書ツール" },
    { tab: "diagnostics", title: "困ったとき", summary: "接続確認と修復" },
  ];
  const grid = el("div", { className: "quick-settings-grid" });
  for (const item of cards) {
    const button = el("button", { type: "button", className: "quick-settings-card" }, [
      el("strong", { textContent: item.title }),
      el("span", { textContent: item.summary }),
      el("small", { textContent: "開く" }),
    ]);
    button.dataset.tab = item.tab;
    button.addEventListener("click", () => activateSettingsTab(item.tab));
    grid.append(button);
  }
  return markSearchable(el("section", { className: "quick-settings" }, [
    el("div", { className: "quick-settings-heading" }, [
      el("h2", { textContent: "まずはここから" }),
      el("p", { textContent: "普段はこの5つを入口にすれば大丈夫です。" }),
    ]),
    grid,
  ]), "よく使う 設定 キャラクター 音声 画面 レイアウト AI モデル 追加機能 診断 修復");
}

function setQuickSettingsSummary(tabId, summary) {
  const node = form.querySelector(`.quick-settings-card[data-tab="${tabId}"] span`);
  if (node) node.textContent = summary;
}

function applySettingsTheme(value) {
  const theme = value === "light" || value === "dark" ? value : "system";
  if (theme === "system") {
    document.documentElement.removeAttribute("data-theme");
  } else {
    document.documentElement.dataset.theme = theme;
  }
}

function section(title) {
  return el("div", { className: "section" }, [el("h2", { textContent: title })]);
}

function advancedGroup(title, note, children) {
  return el("details", { className: "advanced-group" }, [
    el("summary", { textContent: title }),
    note ? el("p", { className: "advanced-group-note", textContent: note }) : null,
    ...children,
  ].filter(Boolean));
}

function buildComponentManager() {
  const sec = section("追加機能と公式取得元");
  sec.classList.add("component-manager");
  sec.append(el("p", {
    className: "settings-intro",
    textContent: "第三者ソフトをPicoAgentから再配布しません。AivisとWhisperは各公式配布元からPicoAgent専用領域へ直接取得し、それ以外は公式ページと設定手順を案内します。読み上げのミュート設定は変わりません。",
  }));
  const core = el("div", { className: "component-card is-ready" }, [
    el("div", { className: "component-card-copy" }, [
      el("strong", { textContent: "基本機能" }),
      el("span", { textContent: "PicoAgent本体・ローカルAIエンジン・Live2D表示" }),
      el("small", { textContent: "常に使用できます。ローカルAIモデルはPC性能に合わせて起動時に準備します。" }),
    ]),
    el("span", { className: "component-badge", textContent: "基本" }),
  ]);
  const list = el("div", { className: "component-list" }, [el("p", { className: "hint", textContent: "状態を確認中…" })]);
  const status = el("p", { className: "hint component-manager-status" });
  sec.append(core, list, status);

  async function refresh() {
    const items = await window.picoagent.settings.componentStatus();
    list.innerHTML = "";
    for (const item of items) {
      const managed = item.installMode === "managed";
      const button = el("button", {
        className: item.ready ? "secondary" : "primary",
        type: "button",
        textContent: managed ? (item.ready ? "公式配布から修復" : "公式配布から導入") : "公式ページを開く",
        disabled: !item.available,
      });
      button.addEventListener("click", async () => {
        if (!managed) {
          const url = item.sources?.[0]?.url;
          if (url) window.open(url, "_blank", "noopener,noreferrer");
          status.textContent = `${item.label}は提供元の手順で導入し、必要なら「スキル」で実行ファイルを指定してください。`;
          return;
        }
        button.disabled = true;
        status.textContent = `${item.label}を公式配布元から取得しています…`;
        try {
          await window.picoagent.settings.installComponents([item.id]);
          status.textContent = `${item.label}を公式配布元から導入しました。`;
          await refresh();
        } catch (error) {
          status.textContent = navigator.onLine
            ? `導入できませんでした: ${error?.message || error}`
            : "オフラインのため導入を見送りました。接続後にもう一度押してください。";
          button.disabled = false;
        }
      });
      const actions = el("div", { className: "component-card-actions" }, [button]);
      if (item.removable) {
        const remove = el("button", { className: "danger", type: "button", textContent: "削除" });
        remove.addEventListener("click", async () => {
          if (remove.dataset.confirming !== "true") {
            remove.dataset.confirming = "true";
            remove.textContent = "もう一度押して削除";
            return;
          }
          remove.disabled = true;
          status.textContent = `${item.label}を削除しています…`;
          try {
            await window.picoagent.settings.removeComponent(item.id);
            status.textContent = `${item.label}を削除しました。`;
            await refresh();
          } catch (error) {
            status.textContent = `削除できませんでした: ${error?.message || error}`;
            remove.disabled = false;
          }
        });
        remove.addEventListener("blur", () => {
          remove.dataset.confirming = "false";
          remove.textContent = "削除";
        });
        actions.append(remove);
      }
      const sourceLinks = el("div", { className: "component-source-links" });
      for (const source of item.sources || []) {
        sourceLinks.append(el("a", {
          href: source.url,
          target: "_blank",
          rel: "noopener noreferrer",
          textContent: source.name,
        }));
      }
      list.append(markSearchable(el("div", { className: `component-card${item.ready ? " is-ready" : ""}` }, [
        el("div", { className: "component-card-copy" }, [
          el("strong", { textContent: item.label }),
          el("span", { textContent: item.description }),
          el("small", { textContent: item.sizeLabel }),
          sourceLinks,
        ]),
        el("span", { className: "component-badge", textContent: item.ready ? "準備済み" : (managed ? "公式から取得" : "外部導入") }),
        actions,
      ]), item.id, item.label, item.description));
    }
    applySettingsSearch();
  }

  if (componentProgressUnlisten) componentProgressUnlisten();
  window.picoagent.listen("component:progress", progress => {
    const percent = progress.total > 0 ? ` ${Math.min(100, Math.round(progress.downloaded / progress.total * 100))}%` : "";
    status.textContent = `${progress.message}${percent}`;
  }).then(unlisten => { componentProgressUnlisten = unlisten; }).catch(error => {
    logApp("warn", "追加機能の進捗を受け取れませんでした", errorDetail(error));
  });
  refresh().catch(error => {
    list.innerHTML = "";
    list.append(el("p", { className: "hint", textContent: `状態を確認できません: ${error?.message || error}` }));
  });
  return sec;
}

function createTabs(items) {
  const tabs = el("div", { className: "tabs", role: "tablist" });
  const panels = el("div", { className: "tab-panels" });
  const byId = {};
  items.forEach(item => {
    const selected = item.id === activeSettingsTab;
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
      activeSettingsTab = item.id;
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
      applySettingsSearch();
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
  if (["alarm", "reminder", "focus_timer", "praise_log", "daily_briefing", "clipboard_text", "chitchat", "task_scheduler", "meeting_recorder", "whisper_transcribe"].includes(name)) {
    return "daily";
  }
  if (["get_time", "weather", "currency_rates", "market_indices", "rss_feed", "web_search", "web_fetch", "research_report"].includes(name)) {
    return "info";
  }
  if (["read_file", "workspace_files", "edit_file", "glob_files", "grep_search", "todo_plan", "notebook_edit", "view_image", "ask_user", "project", "workflow", "run_command", "command_job", "http_api"].includes(name)) {
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

const SKILL_CATEGORY_LABELS = {
  daily: "日常",
  info: "検索・情報",
  workspace: "ファイル/実行",
  connect: "連携",
  social: "SNS/投稿",
  docs: "文書",
  other: "その他",
};

const SKILL_EXAMPLE_FALLBACKS = {
  get_time: ["今何時？"],
  weather: ["今日の東京の天気を教えて"],
  currency_rates: ["100ドルは何円？"],
  market_indices: ["今日の日経平均を見て"],
  rss_feed: ["このRSSの新着を要約して"],
  web_search: ["最新のAIニュースを調べて"],
  web_fetch: ["このURLの本文を読んで"],
  research_report: ["AIスライド生成サービスを出典付きで調査して"],
  spark_workspace: ["Gensparkみたいに資料一式を作って"],
  genspark_suite: ["Genspark風の機能を全部やって"],
  alarm: ["平日の7時に起こして"],
  reminder: ["15分後に洗濯物を取り込むって教えて"],
  focus_timer: ["25分集中タイマーを開始して"],
  praise_log: ["今日できたことを記録して"],
  daily_briefing: ["出かける前にチェックして"],
  clipboard_text: ["クリップボードを要約して"],
  task_scheduler: ["毎朝9時に天気スキルを実行して"],
  meeting_recorder: ["会議の録音を開始して", "話者分類つきで議事録にして"],
  whisper_transcribe: ["最新の録音を文字起こしして"],
  read_file: ["このファイルを読んで"],
  workspace_files: ["作業フォルダにメモを書いて"],
  edit_file: ["このファイルのこの行だけ直して"],
  glob_files: ["プロジェクトの .rs ファイルを一覧して"],
  grep_search: ["この関数を使っている箇所を探して"],
  todo_plan: ["やることを整理して進めて", "いまどこまで終わってる？"],
  project: ["動画制作プロジェクトへ切り替えて", "このプロジェクトの残りの課題を教えて", "この方針を決定事項として残して"],
  workflow: ["この手順を週報作成として覚えて"],
  run_command: ["git statusを実行して", "開発サーバをバックグラウンドで起動して"],
  command_job: ["さっきのビルドの様子を見せて", "動かしっぱなしのサーバを止めて"],
  notebook_edit: ["ノートブックのセルを一覧して", "3番目のセルを差し替えて"],
  view_image: ["このスクリーンショットに何が写っているか教えて"],
  ask_user: ["どっちがいいか聞いて決めて"],
  http_api: ["このAPIにPOSTして"],
  ms_office: ["Wordの文字を置換して"],
  document_convert: ["このdocxをPDFにして"],
  pdf_tools: ["このPDFを分割して"],
  productivity_tools: ["このJSONを整形して", "10kmをmileに換算して"],
  google_calendar: ["明日の予定を見て"],
  google_docs: ["Google Docsに追記して"],
  google_drive: ["Driveから企画書を探して"],
  google_sheets: ["スプレッドシートに行を追加して"],
  google_tasks: ["Google Tasksにタスクを追加して"],
  gmail_message: ["Gmailの返信下書きを作って"],
  microsoft_graph: ["Outlookで予定を作って"],
  slack_send_message: ["Slackにメッセージを送って"],
  discord_send_message: ["Discordに投稿して"],
  notion: ["Notionにページを作って"],
  jira: ["Jiraに課題を作って"],
  linear: ["LinearにIssueを作って"],
  trello: ["Trelloにカードを追加して"],
  todoist: ["Todoistにタスクを追加して"],
  github_issue: ["GitHub Issueを作って"],
  x_twitter: ["Xに投稿して"],
  mastodon: ["Mastodonに投稿して"],
  bluesky: ["Blueskyに投稿して"],
  reddit: ["Redditのhotを見て"],
  linkedin: ["LinkedInに投稿して"],
  misskey: ["Misskeyにノートして"],
  comfyui_generate: ["ComfyUIで画像を生成して"],
  image_generate: ["画像を生成して"],
  chitchat: ["待機中に話題を出して"],
};

function skillExamples(tool) {
  const explicit = Array.isArray(tool.examples) ? tool.examples.filter(Boolean) : [];
  return explicit.length ? explicit : (SKILL_EXAMPLE_FALLBACKS[tool.name] || [`${tool.label || tool.name}を使って`]);
}

function configStatus(tool, values) {
  const schema = Array.isArray(tool.configSchema) ? tool.configSchema : [];
  if (!schema.length) return { label: "設定不要", tone: "neutral" };
  const important = schema.filter(field => field.type === "password" || /TOKEN|KEY|SECRET|WEBHOOK|ACCESS|URL|ROOT|PATH|ENABLED/.test(field.key));
  const targets = important.length ? important : schema;
  const filled = targets.filter(field => {
    const value = values[field.key];
    if (typeof value === "boolean") return value;
    return String(value ?? "").trim() !== "";
  }).length;
  if (filled === 0) return { label: "未設定", tone: "warn" };
  if (filled < targets.length) return { label: `一部設定 ${filled}/${targets.length}`, tone: "partial" };
  return { label: "設定済み", tone: "ok" };
}

function supportsConnectionTest(tool) {
  return ["gmail_message", "slack_send_message", "google_calendar"].includes(tool.name);
}

function approvalPolicy(tool) {
  const name = tool.name || "";
  const send = new Set([
    "gmail_message", "slack_send_message", "discord_send_message", "microsoft_graph",
    "x_twitter", "mastodon", "bluesky", "reddit", "linkedin", "misskey",
  ]);
  const write = new Set([
    "workspace_files", "edit_file", "notebook_edit", "command_job", "todo_plan", "project", "google_calendar", "google_docs", "google_drive", "google_sheets",
    "google_tasks", "github_issue", "notion", "jira", "linear", "trello", "todoist",
    "ms_office", "document_convert", "pdf_tools", "http_api", "task_scheduler",
  ]);
  if (name === "run_command") {
    return { tone: "danger", label: "実行前確認", text: "未許可コマンドや許可ルート外 cwd はチャットで承認が必要です。物理削除コマンドは拒否します。" };
  }
  if (send.has(name)) {
    return { tone: "danger", label: "送信前確認", text: "外部へ送信・投稿します。宛先と本文がユーザー意図に合う場合だけ実行します。" };
  }
  if (write.has(name)) {
    return { tone: "warn", label: "書込/変更", text: "外部サービスや許可ルート内のデータを作成・更新できます。削除相当は物理削除ではなく安全な扱いを前提にします。" };
  }
  if (supportsConnectionTest(tool) || skillCategory(tool) === "connect" || skillCategory(tool) === "social" || ["web_search", "web_fetch", "weather", "rss_feed", "market_indices", "currency_rates"].includes(name)) {
    return { tone: "read", label: "外部読取", text: "外部サービスやWebから情報を取得します。取得結果は信頼できない外部データとして扱います。" };
  }
  return { tone: "safe", label: "低リスク", text: "ローカル処理または副作用のない参照が中心です。" };
}

function approvalPolicyNode(tool) {
  const policy = approvalPolicy(tool);
  return el("div", { className: `approval-policy ${policy.tone}` }, [
    el("span", { textContent: policy.label }),
    el("p", { textContent: policy.text }),
  ]);
}

const SETUP_GUIDES = {
  openai: {
    title: "OpenAI API キー",
    steps: [
      "OpenAI Platform にログインし、API keys で新しい secret key を作ります。",
      "作成直後に表示される sk-... を OPENAI_API_KEY に貼ります。後から同じ値は再表示できません。",
      "必要なら Billing / Usage limits で上限も設定します。",
    ],
    links: [
      ["API keys", "https://platform.openai.com/api-keys"],
      ["公式ヘルプ", "https://help.openai.com/en/articles/4936850-where-do-i-find-my-openai-api-key"],
    ],
  },
  anthropic: {
    title: "Anthropic / Claude API キー",
    steps: [
      "Claude Console にログインし、API keys でキーを作ります。",
      "作成した sk-ant-... を PICOAGENT_ANTHROPIC_API_KEY に貼ります。",
      "API利用は課金対象なので、Console 側の請求設定も確認します。",
    ],
    links: [["Claude Console", "https://platform.claude.com/"]],
  },
  gemini: {
    title: "Gemini API キー",
    steps: [
      "Google AI Studio の API Keys ページを開き、Create API key でキーを作ります。",
      "AIza... のキーを PICOAGENT_GEMINI_API_KEY に貼ります。",
      "公開リポジトリやログには絶対に貼らず、必要なら Google 側で利用制限を設定します。",
    ],
    links: [
      ["AI Studio API Keys", "https://aistudio.google.com/app/apikey"],
      ["公式ドキュメント", "https://ai.google.dev/gemini-api/docs/api-key"],
    ],
  },
  deepseek: {
    title: "DeepSeek API キー",
    steps: [
      "DeepSeek Platform にログインし、API keys でキーを作ります。",
      "sk-... を PICOAGENT_DEEPSEEK_API_KEY に貼ります。",
      "Thinking モードは必要に応じて設定します。",
    ],
    links: [["DeepSeek Platform", "https://platform.deepseek.com/api_keys"]],
  },
  xai: {
    title: "xAI API キー",
    steps: [
      "xAI Console にログインし、API key を作ります。",
      "xai-... を PICOAGENT_XAI_API_KEY に貼ります。",
      "モデル名は利用できるモデル一覧に合わせます。",
    ],
    links: [["xAI Console", "https://console.x.ai/"]],
  },
  mistral: {
    title: "Mistral API キー",
    steps: [
      "Mistral Console にログインし、API key を作ります。",
      "作成したキーを PICOAGENT_MISTRAL_API_KEY に貼ります。",
      "必要なら Console 側で請求と利用上限を確認します。",
    ],
    links: [["Mistral Console", "https://console.mistral.ai/"]],
  },
  gmail_message: {
    title: "Gmail OAuth access token",
    steps: [
      "Google Cloud Console で OAuth クライアントを用意します。個人利用なら OAuth Playground で access token を発行できます。",
      "Gmail の読み取り/変更/送信に必要な scope を許可します。例: gmail.modify と gmail.send。",
      "発行された ya29... の access token を PICOAGENT_GMAIL_ACCESS_TOKEN に貼ります。access token は期限切れになるため、切れたら取り直します。",
    ],
    links: [
      ["Google 認証情報", "https://console.cloud.google.com/apis/credentials"],
      ["OAuth Playground", "https://developers.google.com/oauthplayground"],
      ["Gmail API scopes", "https://developers.google.com/workspace/gmail/api/auth/scopes"],
    ],
  },
  google_calendar: {
    title: "Google Calendar OAuth access token",
    steps: [
      "Google Cloud Console で OAuth クライアントを用意し、Calendar API を有効にします。",
      "OAuth Playground などで Calendar scope を許可して access token を発行します。",
      "ya29... を PICOAGENT_GOOGLE_CALENDAR_ACCESS_TOKEN に貼ります。既定カレンダーは通常 primary で動きます。",
    ],
    links: [
      ["Google 認証情報", "https://console.cloud.google.com/apis/credentials"],
      ["OAuth Playground", "https://developers.google.com/oauthplayground"],
      ["Calendar API scopes", "https://developers.google.com/workspace/calendar/api/auth"],
    ],
  },
  google_docs: {
    title: "Google Docs OAuth access token",
    steps: [
      "Google Cloud Console で OAuth クライアントを用意し、Google Docs API を有効にします。",
      "OAuth Playground などで docs scope を許可して access token を発行します。",
      "ya29... を PICOAGENT_GOOGLE_DOCS_ACCESS_TOKEN に貼ります。",
    ],
    links: [
      ["Google 認証情報", "https://console.cloud.google.com/apis/credentials"],
      ["OAuth Playground", "https://developers.google.com/oauthplayground"],
      ["Docs API scopes", "https://developers.google.com/workspace/docs/api/auth"],
    ],
  },
  google_drive: {
    title: "Google Drive OAuth access token",
    steps: [
      "Google Cloud Console で OAuth クライアントを用意し、Google Drive API を有効にします。",
      "OAuth Playground などで Drive scope を許可して access token を発行します。",
      "ya29... を PICOAGENT_GOOGLE_DRIVE_ACCESS_TOKEN に貼ります。",
    ],
    links: [
      ["Google 認証情報", "https://console.cloud.google.com/apis/credentials"],
      ["OAuth Playground", "https://developers.google.com/oauthplayground"],
      ["Drive API scopes", "https://developers.google.com/workspace/drive/api/guides/api-specific-auth"],
    ],
  },
  google_sheets: {
    title: "Google Sheets OAuth access token",
    steps: [
      "Google Cloud Console で OAuth クライアントを用意し、Google Sheets API を有効にします。",
      "OAuth Playground などで spreadsheets scope を許可して access token を発行します。",
      "ya29... を PICOAGENT_GOOGLE_SHEETS_ACCESS_TOKEN に貼り、既定 spreadsheet ID も設定します。",
    ],
    links: [
      ["Google 認証情報", "https://console.cloud.google.com/apis/credentials"],
      ["OAuth Playground", "https://developers.google.com/oauthplayground"],
      ["Sheets API scopes", "https://developers.google.com/workspace/sheets/api/scopes"],
    ],
  },
  google_tasks: {
    title: "Google Tasks OAuth access token",
    steps: [
      "Google Cloud Console で OAuth クライアントを用意し、Google Tasks API を有効にします。",
      "OAuth Playground などで tasks scope を許可して access token を発行します。",
      "ya29... を PICOAGENT_GOOGLE_TASKS_ACCESS_TOKEN に貼ります。通常の tasklist ID は @default です。",
    ],
    links: [
      ["Google 認証情報", "https://console.cloud.google.com/apis/credentials"],
      ["OAuth Playground", "https://developers.google.com/oauthplayground"],
      ["Tasks API", "https://developers.google.com/tasks"],
    ],
  },
  slack_send_message: {
    title: "Slack Bot token / Webhook URL",
    steps: [
      "Slack API の Your Apps でアプリを作成します。",
      "OAuth & Permissions で bot scopes を追加します。投稿は chat:write、履歴取得は channels:history などが必要です。",
      "Install to Workspace 後、xoxb-... を PICOAGENT_SLACK_BOT_TOKEN に貼ります。Webhook を使う場合は Incoming Webhooks を有効化して URL を貼ります。",
    ],
    links: [
      ["Slack Your Apps", "https://api.slack.com/apps"],
      ["Slack tokens", "https://docs.slack.dev/authentication/tokens"],
      ["Scopes", "https://api.slack.com/scopes"],
    ],
  },
  github_issue: {
    title: "GitHub token",
    steps: [
      "GitHub の Personal access tokens で fine-grained token を作ります。",
      "対象 repository を選び、Issues の Read/Write 権限を付けます。",
      "github_pat_... を PICOAGENT_GITHUB_TOKEN に貼り、owner / repo も設定します。",
    ],
    links: [
      ["Fine-grained tokens", "https://github.com/settings/personal-access-tokens"],
      ["公式ドキュメント", "https://docs.github.com/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens"],
    ],
  },
  notion: {
    title: "Notion integration token",
    steps: [
      "Notion の My integrations で integration を作成します。",
      "Internal Integration Secret を PICOAGENT_NOTION_TOKEN に貼ります。",
      "使うページ/DBの Share から integration を招待し、database ID も設定します。",
    ],
    links: [
      ["My integrations", "https://www.notion.so/my-integrations"],
      ["Notion API docs", "https://developers.notion.com/docs/create-a-notion-integration"],
    ],
  },
  discord_send_message: {
    title: "Discord Webhook URL",
    steps: [
      "Discord のチャンネル設定で 連携サービス > ウェブフック を開きます。",
      "新しい Webhook を作成し、URL をコピーします。",
      "URL を PICOAGENT_DISCORD_WEBHOOK_URL に貼ります。",
    ],
    links: [["Discord Webhooks", "https://support.discord.com/hc/articles/228383668-Intro-to-Webhooks"]],
  },
  todoist: {
    title: "Todoist API token",
    steps: [
      "Todoist の設定 > Integrations > Developer を開きます。",
      "API token をコピーして PICOAGENT_TODOIST_TOKEN に貼ります。",
      "必要なら既定 project ID も設定します。",
    ],
    links: [["Todoist Developer", "https://app.todoist.com/app/settings/integrations/developer"]],
  },
  trello: {
    title: "Trello key / token",
    steps: [
      "Trello Developer API Keys で key を確認します。",
      "同じページから token を生成します。",
      "key / token / list ID をそれぞれ設定します。",
    ],
    links: [["Trello API Keys", "https://trello.com/app-key"]],
  },
  bluesky: {
    title: "Bluesky app password",
    steps: [
      "Bluesky の Settings > Privacy and security > App passwords を開きます。",
      "新しい App Password を作り、PICOAGENT_BLUESKY_APP_PASSWORD に貼ります。",
      "identifier には user.bsky.social のようなハンドルを入れます。",
    ],
    links: [["Bluesky App passwords", "https://bsky.app/settings/app-passwords"]],
  },
};

function setupGuideFor(id, schema = []) {
  if (SETUP_GUIDES[id]) return SETUP_GUIDES[id];
  const keys = schema.map(field => field.key).join(" ");
  if (/GOOGLE_.*ACCESS_TOKEN|GMAIL_ACCESS_TOKEN/.test(keys)) {
    return {
      title: "Google OAuth access token",
      steps: [
        "Google Cloud Console で OAuth クライアントを用意します。",
        "OAuth Playground などで対象APIの scope を許可し、access token を発行します。",
        "ya29... の値を access token 欄に貼ります。access token は期限切れになるため、切れたら取り直します。",
      ],
      links: [
        ["Google 認証情報", "https://console.cloud.google.com/apis/credentials"],
        ["OAuth Playground", "https://developers.google.com/oauthplayground"],
      ],
    };
  }
  if (/TOKEN|API_KEY|SECRET|WEBHOOK|APP_PASSWORD|BEARER/.test(keys)) {
    return {
      title: "トークン/キー設定の基本",
      steps: [
        "対象サービスの開発者設定または連携アプリ設定で API key / token / webhook URL を作ります。",
        "作成直後に表示される秘密値を、この設定欄へ貼ります。秘密値は後から再表示できないサービスがあります。",
        "必要最小限の権限だけを付け、不要になったキーはサービス側で無効化してください。",
      ],
      links: [],
    };
  }
  return null;
}

function setupGuideNode(id, schema = []) {
  const guide = setupGuideFor(id, schema);
  if (!guide) return null;
  const details = el("details", { className: "setup-guide" }, [
    el("summary", { textContent: `設定ガイド: ${guide.title}` }),
    el("ol", {}, guide.steps.map(step => el("li", { textContent: step }))),
  ]);
  if (guide.links?.length) {
    details.append(el("div", { className: "setup-guide-links" }, guide.links.map(([label, href]) => {
      const link = el("a", { href, textContent: label });
      link.setAttribute("target", "_blank");
      link.setAttribute("rel", "noreferrer");
      link.addEventListener("click", event => event.stopPropagation());
      return link;
    })));
  }
  details.append(el("p", { className: "setup-guide-note", textContent: "キーやトークンは秘密情報です。チャット、ログ、Git、スクリーンショットには貼らないでください。" }));
  details.addEventListener("click", event => event.stopPropagation());
  return details;
}

function buildConnectionTest(tool) {
  const wrap = el("div", { className: "connection-test" });
  const button = el("button", { type: "button", textContent: "接続テスト" });
  const result = el("div", { className: "connection-test-result", textContent: "保存済みの設定値で確認します。" });
  const diagnosis = el("div", { className: "connection-test-diagnosis" });
  button.addEventListener("click", async event => {
    event.stopPropagation();
    button.disabled = true;
    result.className = "connection-test-result";
    result.textContent = "確認中…";
    diagnosis.textContent = "";
    try {
      const response = await window.picoagent.settings.testTool(tool.name);
      result.className = `connection-test-result ${response.ok ? "ok" : "error"}`;
      result.textContent = response.message || (response.ok ? "OK" : "確認できませんでした");
      diagnosis.textContent = response.diagnosis || "";
    } catch (error) {
      logApp("warn", "スキル接続テストに失敗しました", `${tool.name}: ${errorDetail(error)}`);
      result.className = "connection-test-result error";
      result.textContent = `接続テストに失敗: ${error.message || error}`;
      diagnosis.textContent = "PicoAgent 内部の接続テスト呼び出しに失敗しました。アプリログを確認してください。";
    } finally {
      button.disabled = false;
    }
  });
  wrap.append(button, el("div", { className: "connection-test-output" }, [result, diagnosis]));
  return wrap;
}

function buildSkillGuide(tools) {
  const guide = el("div", { className: "skill-guide" });
  const header = el("div", { className: "skill-guide-head" }, [
    el("div", { textContent: "区分" }),
    el("div", { textContent: "スキル" }),
    el("div", { textContent: "できること" }),
    el("div", { textContent: "頼み方例" }),
  ]);
  guide.append(header);
  const sorted = [...tools].sort((a, b) => {
    const ca = skillCategory(a);
    const cb = skillCategory(b);
    if (ca !== cb) return Object.keys(SKILL_CATEGORY_LABELS).indexOf(ca) - Object.keys(SKILL_CATEGORY_LABELS).indexOf(cb);
    return String(a.label || a.name).localeCompare(String(b.label || b.name), "ja");
  });
  for (const tool of sorted) {
    const examples = skillExamples(tool);
    const row = markSearchable(el("div", { className: "skill-guide-row" }, [
      el("div", { className: "skill-guide-cat", textContent: SKILL_CATEGORY_LABELS[skillCategory(tool)] || "その他" }),
      el("div", { className: "skill-guide-name", textContent: tool.label || tool.name }),
      el("div", { className: "skill-guide-desc", textContent: tool.description || "" }),
      el("div", { className: "skill-guide-examples" }, examples.slice(0, 2).map(example => el("span", { textContent: example }))),
    ]), tool.name, tool.label, tool.description, examples.join(" "));
    row.dataset.genre = skillCategory(tool);
    guide.append(row);
  }
  return guide;
}

function renderSkillCard(tool, values) {
  const hasConfig = !!tool.configSchema?.length;
  const status = configStatus(tool, values);
  const name = el("div", { className: "card-name" }, [
    document.createTextNode(hasConfig ? `${tool.label || tool.name}　⚙` : (tool.label || tool.name)),
    el("span", { className: `config-badge ${status.tone}`, textContent: status.label }),
  ]);
  const card = markSearchable(el("div", { className: hasConfig ? "card-item card-accordion" : "card-item" }, [
    name,
    el("div", { className: "card-desc", textContent: tool.description || "" }),
    approvalPolicyNode(tool),
  ]), tool.name, tool.label, tool.description);
  const nodes = [card];
  if (hasConfig) {
    const panel = markSearchable(
      el("div", { className: "card-config" }),
      tool.name,
      tool.label,
      tool.configSchema.map(field => `${field.key} ${field.label}`).join(" ")
    );
    panel.hidden = true;
    const guide = setupGuideNode(tool.name, tool.configSchema);
    if (guide) panel.append(guide);
    if (supportsConnectionTest(tool)) {
      panel.append(buildConnectionTest(tool));
    }
    if (tool.name === "alarm") {
      // 目覚ましは JSON ではなく、時刻ピッカー＋曜日チェックの専用フォームで編集する。
      panel.append(buildAlarmEditor(values.PICOAGENT_ALARMS));
    } else if (tool.name === "task_scheduler") {
      panel.append(buildScheduleEditor(values.PICOAGENT_SCHEDULED_TASKS));
    } else {
      for (const f of tool.configSchema) panel.append(field(f.key, f.label, f.type, values[f.key], f.placeholder));
    }
    panel.addEventListener("click", event => event.stopPropagation());
    card.addEventListener("click", () => {
      const open = panel.hidden;
      panel.hidden = !open;
      card.classList.toggle("open", open);
      applySettingsSearch();
    });
    nodes.push(panel);
  }
  return nodes;
}

function buildSkillCategoryList(tools, values) {
  const categories = [
    { id: "daily", label: SKILL_CATEGORY_LABELS.daily },
    { id: "info", label: SKILL_CATEGORY_LABELS.info },
    { id: "workspace", label: SKILL_CATEGORY_LABELS.workspace },
    { id: "connect", label: SKILL_CATEGORY_LABELS.connect },
    { id: "social", label: SKILL_CATEGORY_LABELS.social },
    { id: "docs", label: SKILL_CATEGORY_LABELS.docs },
    { id: "other", label: SKILL_CATEGORY_LABELS.other },
  ];
  const grouped = new Map(categories.map(category => [category.id, []]));
  for (const tool of tools) grouped.get(skillCategory(tool)).push(tool);

  // サブタブで隠さず、カテゴリ見出し付きで全スキルを縦に並べる（全件が見つかる）。
  // 各スキルの設定(⚙)は renderSkillCard 側で既定折りたたみ（クリックで開く）。
  const wrap = el("div", { className: "skill-browser" });
  for (const category of categories) {
    const toolsInCategory = grouped.get(category.id);
    if (!toolsInCategory.length) continue;
    const group = el("div", { className: "skill-cat-group" });
    group.dataset.genre = category.id;
    group.append(el("h3", {
      className: "skill-cat-head",
      textContent: `${category.label}（${toolsInCategory.length}）`,
    }));
    const list = el("div", { className: "card-list" });
    for (const tool of toolsInCategory) {
      for (const node of renderSkillCard(tool, values)) list.append(node);
    }
    group.append(list);
    wrap.append(group);
  }
  return wrap;
}

// 自分で追加したキャラクター（app_data_dir/characters 配下）を一覧の形へそろえる。
// 同梱と同じ id は同梱側を優先し、追加分が同梱キャラを乗っ取らないようにする。
async function loadUserCharacters(bundled) {
  const known = new Set(bundled.map(character => character.id));
  try {
    const result = await window.picoagent.settings.userCharacters();
    userCharacterRoot = result?.root || "";
    return (result?.characters || [])
      .filter(character => !known.has(character.id))
      .map(character => ({
        id: character.id,
        name: character.name,
        description: character.description || "",
        manifest: `${character.id}/character.json`,
        root: window.picoagent.assetUrl(character.path),
        user: true,
        live2d: !!character.live2d,
      }));
  } catch (error) {
    // フォールバック許可: 追加キャラを取れなくても同梱キャラの設定は開けるようにする。
    statusEl.textContent = `追加キャラクターを読み込めませんでした: ${error?.message || error}`;
    return [];
  }
}

async function loadCharacterPersona(id) {
  const entry = characterCatalog.characters.find(character => character.id === id);
  if (!entry) throw new Error(`キャラクター ${id} が一覧にありません`);
  const manifest = await loadCharacterManifestForSettings(entry);
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

function live2dModelEntries(config) {
  if (!config) return [];
  const values = Array.isArray(config.models) && config.models.length ? config.models : [config.model];
  return values.flatMap(value => {
    if (typeof value === "string") {
      const match = value.match(/(?:^|\/)sdk([45])(?:\/|$)/i);
      return value ? [{ path: value, sdk: match?.[1] || "" }] : [];
    }
    const path = String(value?.path || value?.model || "");
    return path ? [{ path, sdk: String(value?.sdk || "") }] : [];
  });
}

function live2dSdkVersions(manifest) {
  const configs = [manifest?.live2d];
  for (const variant of manifest?.variants || []) {
    if (variant?.live2d && variant.live2d !== false) configs.push(variant.live2d);
  }
  return new Set(configs.flatMap(live2dModelEntries).map(entry => entry.sdk).filter(Boolean));
}

async function loadCharacterLive2dSdk(id) {
  if (!characterLive2dSdkSelect || !characterLive2dSdkField) return;
  const entry = characterCatalog.characters.find(character => character.id === id);
  if (!entry) throw new Error(`キャラクター ${id} が一覧にありません`);
  const manifest = await loadCharacterManifestForSettings(entry);
  const versions = live2dSdkVersions(manifest);
  const isLive2d = characterRenderKind(manifest) === "live2d";
  characterLive2dSdkField.hidden = !isLive2d;
  for (const option of characterLive2dSdkSelect.options) {
    option.disabled = option.value !== "auto" && !versions.has(option.value);
  }
  const saved = String(characterLive2dSdkOverrides[id] || "auto");
  characterLive2dSdkSelect.value = saved === "4" || saved === "5" ? saved : "auto";
  if (characterLive2dSdkSelect.selectedOptions[0]?.disabled) characterLive2dSdkSelect.value = "auto";
  const available = ["4", "5"].filter(version => versions.has(version));
  characterLive2dSdkHint.textContent = available.length > 1
    ? "自動はパック先頭の推奨世代を使います。表示が崩れるときは Cubism 4／5 の片方に固定できます。"
    : available.length === 1
      ? `このパックに選択可能なモデルは Cubism ${available[0]} だけです。`
      : "このパックは SDK 世代を明示していないため、自動選択で読み込みます。";
}

function saveCurrentCharacterLive2dSdk(id) {
  if (!characterLive2dSdkSelect || characterLive2dSdkField?.hidden) return;
  const value = characterLive2dSdkSelect.value;
  if (value === "4" || value === "5") characterLive2dSdkOverrides[id] = value;
  else delete characterLive2dSdkOverrides[id];
}

async function loadCharacterManifestForSettings(character) {
  if (characterManifestCache.has(character.id)) return characterManifestCache.get(character.id);
  const response = await fetch(`${character.root}/character.json?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!response.ok) throw new Error(`キャラクター定義: HTTP ${response.status}`);
  const manifest = await response.json();
  characterManifestCache.set(character.id, manifest);
  return manifest;
}

async function characterIconUrl(character) {
  try {
    const manifest = await loadCharacterManifestForSettings(character);
    const root = character.root;
    const idle = manifest.animations?.idle;
    const imagePath = idle?.path ? `${idle.path}/00.png` : manifest.boot?.sleeping;
    if (!imagePath) return "assets/characters/_preview-placeholder.png";
    const version = manifest.assetVersion ? `?v=${encodeURIComponent(manifest.assetVersion)}` : "";
    return `${root}/${imagePath}${version}`;
  } catch (error) {
    logApp("warn", "キャラクターアイコンの読込に失敗しました", `${character.id}: ${errorDetail(error)}`);
    return "";
  }
}

function characterRenderType(character, manifest) {
  const kind = characterRenderKind(manifest);
  if (kind === "live2d") return "Live2D";
  if (kind === "tha3") return "自作THA3";
  if (character.user) return "追加PNG";
  return "PNGアニメ";
}

function characterRenderKind(manifest) {
  const hasLive2d = !!manifest.live2d
    || (Array.isArray(manifest.variants) && manifest.variants.some(variant => variant?.live2d));
  if (hasLive2d) return "live2d";
  if (manifest.generator?.type === "tha3") return "tha3";
  return "png";
}

function applyCharacterFilters(globalQuery = settingsSearch?.value.trim().toLowerCase() || "") {
  const list = form.querySelector(".character-browser-list");
  if (!list) return;
  const cards = [...list.querySelectorAll(".character-card")];
  let visible = 0;
  for (const card of cards) {
    const matchesGlobal = !globalQuery || nodeSearchText(card).includes(globalQuery);
    const matchesLocal = !characterSearchQuery || nodeSearchText(card).includes(characterSearchQuery);
    const matchesType = characterTypeFilter === "all"
      || card.dataset.characterRender === characterTypeFilter
      || (characterTypeFilter === "user" && card.dataset.characterOrigin === "user");
    card.hidden = !(matchesGlobal && matchesLocal && matchesType);
    if (!card.hidden) visible += 1;
  }
  const count = form.querySelector(".character-browser-count");
  if (count) count.textContent = `${visible} / ${cards.length}体を表示`;
  const empty = form.querySelector(".character-browser-empty");
  if (empty) empty.hidden = visible !== 0;
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
  // 音声タブ簡素化でキャラ別 speaker 入力欄は非表示（未生成）になったため、無い場合は何もしない。
  if (!characterSpeaker) return;
  const current = characterVoiceOverrides[id] || {};
  const defaultVoice = characterDefaultVoices[id] || {};
  characterSpeaker.value = current.speaker || "";
  characterSpeaker.placeholder = defaultVoice.styleId || inputs.get("PICOAGENT_TTS_SPEAKER")?.value || "888753760";
}

function formatNumber(value) {
  const number = Number(value || 0);
  return Number.isFinite(number) ? number.toLocaleString("ja-JP") : "0";
}

function statusTone(ok, warn = false) {
  if (ok) return "ok";
  if (warn) return "warn";
  return "error";
}

function diagnosticCard(title, value, detail = "", tone = "neutral", searchParts = []) {
  return markSearchable(el("div", { className: `diagnostic-card ${tone}` }, [
    el("div", { className: "diagnostic-label", textContent: title }),
    el("div", { className: "diagnostic-value", textContent: value }),
    el("div", { className: "diagnostic-detail", textContent: detail }),
  ]), title, value, detail, ...searchParts);
}

function enginePhaseLabel(phase) {
  const labels = {
    idle: "待機中",
    preparing: "準備中",
    download: "取得中",
    starting: "起動中",
    ready: "利用可能",
    error: "エラー",
  };
  return labels[phase] || phase || "不明";
}

function appendUsageSummary(grid, usage) {
  const monthly = Array.isArray(usage.monthlySummary) ? usage.monthlySummary : [];
  const top = monthly[0];
  grid.append(diagnosticCard(
    "今月の使用量",
    `${formatNumber(monthly.reduce((sum, item) => sum + Number(item.totalTokens || 0), 0))} tokens`,
    top ? `最多: ${top.key} / ${formatNumber(top.totalTokens)} tokens` : "今月の記録はまだありません",
    monthly.length ? "ok" : "neutral",
    ["usage", "token", "tokens", "使用量"]
  ));
  grid.append(diagnosticCard(
    "累計使用量",
    `${formatNumber(usage.totalTokens)} tokens`,
    `${formatNumber(usage.recordCount)} 件の記録`,
    Number(usage.totalTokens || 0) > 0 ? "ok" : "neutral",
    ["usage", "token", "tokens", "累計"]
  ));
}

function appendTraceSummary(grid, traces) {
  const summary = Array.isArray(traces.summary) ? traces.summary : [];
  const worst = summary[0];
  grid.append(diagnosticCard(
    "スキル実行",
    `${formatNumber(Array.isArray(traces.entries) ? traces.entries.length : 0)} 件`,
    worst ? `最長平均: ${worst.tool} ${formatNumber(worst.avgMs)}ms / エラー ${formatNumber(worst.errorCount)}` : "直近の実行記録はありません",
    summary.some(item => Number(item.errorCount || 0) > 0) ? "warn" : "ok",
    ["trace", "traces", "tool", "latency", "スキル"]
  ));
}

function diagnosticsFailureCard(title, error) {
  return diagnosticCard(
    title,
    "取得失敗",
    error?.message || String(error || "不明なエラー"),
    "error",
    ["診断", "エラー"]
  );
}

// ウィンドウとフキダシの位置・サイズを既定へ戻すボタン。
// ディスプレイ構成を変えてウィンドウを画面外で見失うと右クリックメニューにも辿り着けないため、
// 右クリックメニューの「フキダシ位置とサイズをリセット」とは別に、設定画面にも復帰口を置く。
function layoutResetField() {
  const label = "画面位置を初期化";
  const button = el("button", { type: "button", className: "secondary", textContent: label });
  button.addEventListener("click", async () => {
    button.disabled = true;
    statusEl.textContent = `${label}を実行しています…`;
    try {
      await window.picoagent.resetLayout();
      statusEl.textContent = "ウィンドウとフキダシの位置・サイズを既定へ戻しました。";
    } catch (error) {
      logApp("error", `${label}に失敗しました`, errorDetail(error));
      statusEl.textContent = `${label}に失敗: ${error?.message || error}`;
    } finally {
      button.disabled = false;
    }
  });
  const hint = el("p", { className: "hint", textContent: "ウィンドウが画面外や別のディスプレイへ行って見つからないときに使ってください。" });
  return markSearchable(
    el("div", { className: "field" }, [el("label", { textContent: "画面位置" }), button, hint]),
    "PICOAGENT_WINDOW_LEFT_PX",
    label,
    "ウィンドウ 位置 リセット 初期化 ディスプレイ 画面外"
  );
}

function windowPlacementField() {
  const actions = el("div", { className: "placement-actions" });
  for (const [id, label] of [["left", "左画面"], ["right", "右画面"], ["current", "現在の画面"], ["primary", "主画面"]]) {
    const button = el("button", { type: "button", className: "secondary", textContent: label });
    button.addEventListener("click", async () => {
      try {
        await window.picoagent.placeWindow(id);
        statusEl.textContent = `${label}の中央へ移動しました。`;
      } catch (error) {
        statusEl.textContent = `移動できませんでした: ${error?.message || error}`;
      }
    });
    actions.append(button);
  }
  return markSearchable(el("div", { className: "field" }, [
    el("label", { textContent: "表示するモニター" }), actions,
    el("p", { className: "hint", textContent: "動画を見ながら使う場合は「左画面」などを選ぶと、接続中ディスプレイの中央へ移動します。" }),
  ]), "ウィンドウ 配置 左画面 右画面 主画面 ディスプレイ");
}

function streamSettingsField(values) {
  const wrapper = markSearchable(el("div", { className: "stream-settings" }), "配信 背景 ディスプレイ 画像 日付 時刻 LIVE");
  wrapper.append(selectField("PICOAGENT_STREAM_BACKGROUND", "配信背景", values.PICOAGENT_STREAM_BACKGROUND || "pastel", [
    { value: "pastel", label: "パステルスタジオ" },
    { value: "gaming", label: "落ち着いたゲーム部屋" },
    { value: "simple", label: "シンプルグラデーション" },
    { value: "custom", label: "自分の画像" },
  ]));
  const backgroundPath = el("input", { type: "text", readOnly: true, value: values.PICOAGENT_STREAM_BACKGROUND_IMAGE || "", placeholder: "画像未選択" });
  const backgroundButton = el("button", { type: "button", className: "secondary", textContent: "背景画像を選ぶ" });
  backgroundButton.addEventListener("click", async () => {
    try {
      const result = await window.picoagent.settings.pickStreamImage("background");
      if (result?.path) { backgroundPath.value = result.path; inputs.get("PICOAGENT_STREAM_BACKGROUND").value = "custom"; updateDirtyState(); }
    } catch (error) { statusEl.textContent = `背景画像を取り込めませんでした: ${error?.message || error}`; }
  });
  wrapper.append(registerInput("PICOAGENT_STREAM_BACKGROUND_IMAGE", backgroundPath, el("div", { className: "field" }, [el("label", { textContent: "自分の背景画像" }), backgroundPath, backgroundButton])));
  wrapper.append(selectField("PICOAGENT_STREAM_DISPLAY", "背景内ディスプレイ", values.PICOAGENT_STREAM_DISPLAY || "dashboard", [
    { value: "dashboard", label: "配信ダッシュボード" },
    { value: "game", label: "ゲーム画面風" },
    { value: "clock", label: "時計・ON AIR" },
    { value: "image", label: "自分の画像" },
    { value: "none", label: "表示しない" },
  ]));
  const displayPath = el("input", { type: "text", readOnly: true, value: values.PICOAGENT_STREAM_DISPLAY_IMAGE || "", placeholder: "画像未選択" });
  const displayButton = el("button", { type: "button", className: "secondary", textContent: "ディスプレイ画像を選ぶ" });
  displayButton.addEventListener("click", async () => {
    try {
      const result = await window.picoagent.settings.pickStreamImage("display");
      if (result?.path) { displayPath.value = result.path; inputs.get("PICOAGENT_STREAM_DISPLAY").value = "image"; updateDirtyState(); }
    } catch (error) { statusEl.textContent = `ディスプレイ画像を取り込めませんでした: ${error?.message || error}`; }
  });
  wrapper.append(registerInput("PICOAGENT_STREAM_DISPLAY_IMAGE", displayPath, el("div", { className: "field" }, [el("label", { textContent: "ディスプレイに映す画像" }), displayPath, displayButton])));
  wrapper.append(el("p", { className: "hint", textContent: "配信表示では、左上に LIVE・日付・時刻を常時表示します。" }));
  return wrapper;
}

function buildCharacterStreamProfileEditor(values) {
  const enabled = el("input", { type: "checkbox" });
  const background = el("select", {}, [
    ["pastel", "パステルスタジオ"], ["gaming", "落ち着いたゲーム部屋"],
    ["simple", "シンプルグラデーション"], ["custom", "自分の画像"],
  ].map(([value, label]) => el("option", { value, textContent: label })));
  const display = el("select", {}, [
    ["dashboard", "配信ダッシュボード"], ["game", "ゲーム画面風"], ["clock", "時計・ON AIR"],
    ["image", "自分の画像"], ["none", "表示しない"],
  ].map(([value, label]) => el("option", { value, textContent: label })));
  const captionColor = el("input", { type: "color", value: "#5a3b4b" });
  const backgroundPath = el("input", { type: "text", readOnly: true, placeholder: "キャラ専用画像なし" });
  const displayPath = el("input", { type: "text", readOnly: true, placeholder: "キャラ専用画像なし" });
  const backgroundButton = el("button", { type: "button", className: "secondary", textContent: "キャラ用背景画像" });
  const displayButton = el("button", { type: "button", className: "secondary", textContent: "キャラ用画面画像" });
  const preview = el("div", { className: "stream-profile-preview" }, [
    el("span", { textContent: "● LIVE　12:34" }), el("strong", { textContent: "キャラクター別の字幕プレビュー" }),
  ]);
  const controls = [background, display, captionColor, backgroundPath, displayPath, backgroundButton, displayButton];
  const globalDefaults = () => ({
    background: inputs.get("PICOAGENT_STREAM_BACKGROUND")?.value || values.PICOAGENT_STREAM_BACKGROUND || "pastel",
    backgroundImage: inputs.get("PICOAGENT_STREAM_BACKGROUND_IMAGE")?.value || values.PICOAGENT_STREAM_BACKGROUND_IMAGE || "",
    display: inputs.get("PICOAGENT_STREAM_DISPLAY")?.value || values.PICOAGENT_STREAM_DISPLAY || "dashboard",
    displayImage: inputs.get("PICOAGENT_STREAM_DISPLAY_IMAGE")?.value || values.PICOAGENT_STREAM_DISPLAY_IMAGE || "",
    captionColor: "#5a3b4b",
  });
  const updatePreview = () => {
    preview.dataset.background = background.value;
    preview.dataset.display = display.value;
    preview.querySelector("strong").style.color = captionColor.value;
  };
  const save = () => {
    const characterId = characterSelect.value;
    if (!enabled.checked) delete characterStreamProfiles[characterId];
    else characterStreamProfiles[characterId] = {
      background: background.value, backgroundImage: backgroundPath.value,
      display: display.value, displayImage: displayPath.value, captionColor: captionColor.value,
    };
    for (const control of controls) control.disabled = !enabled.checked;
    updatePreview();
    updateDirtyState();
  };
  const load = () => {
    const profile = characterStreamProfiles?.[characterSelect.value];
    const current = profile || globalDefaults();
    enabled.checked = Boolean(profile);
    background.value = current.background || "pastel";
    backgroundPath.value = current.backgroundImage || "";
    display.value = current.display || "dashboard";
    displayPath.value = current.displayImage || "";
    captionColor.value = /^#[0-9a-f]{6}$/i.test(current.captionColor || "") ? current.captionColor : "#5a3b4b";
    for (const control of controls) control.disabled = !enabled.checked;
    updatePreview();
  };
  enabled.addEventListener("change", save);
  for (const control of [background, display, captionColor]) control.addEventListener("change", save);
  backgroundButton.addEventListener("click", async () => {
    const result = await window.picoagent.settings.pickStreamImage("background");
    if (result?.path) { backgroundPath.value = result.path; background.value = "custom"; save(); }
  });
  displayButton.addEventListener("click", async () => {
    const result = await window.picoagent.settings.pickStreamImage("display");
    if (result?.path) { displayPath.value = result.path; display.value = "image"; save(); }
  });
  const reset = el("button", { type: "button", className: "secondary", textContent: "共通の配信設定へ戻す" });
  reset.addEventListener("click", () => { enabled.checked = false; save(); load(); });
  const node = markSearchable(el("div", { className: "field stream-profile-editor" }, [
    el("label", {}, [enabled, document.createTextNode(" このキャラ専用の配信設定を使う")]),
    el("div", { className: "stream-profile-controls" }, [background, display, captionColor]),
    el("div", { className: "stream-profile-image-controls" }, [backgroundPath, backgroundButton, displayPath, displayButton]),
    preview, reset,
    el("p", { className: "hint", textContent: "OFFでは画面・操作タブの共通背景を使います。背景、画面内ディスプレイ、字幕文字色をキャラごとに保存できます。" }),
  ]), "キャラ 配信 背景 ディスプレイ 字幕 色 プレビュー");
  return { node, load };
}

function buildCharacterFramingEditor() {
  const layoutSelect = el("select", {}, LAYOUT_CHOICES.map(layout => el("option", { value: layout.id, textContent: layout.label })));
  const presetSelect = el("select", {}, [
    el("option", { value: "bust", textContent: "胸上（配信向け）" }),
    el("option", { value: "upper", textContent: "上半身（基本）" }),
    el("option", { value: "full", textContent: "全身" }),
    el("option", { value: "custom", textContent: "細かく調整" }),
  ]);
  const zoom = el("input", { type: "range", min: "1", max: "4", step: ".05" });
  const x = el("input", { type: "number", min: "-4000", max: "4000", step: "1" });
  const y = el("input", { type: "number", min: "-4000", max: "4000", step: "1" });
  const preview = el("div", { className: "framing-preview" }, [el("img", { alt: "選択キャラの構図プレビュー" }), el("span", { textContent: "字幕テロップ" })]);
  const defaults = { bust: { zoom: 2.05, x: 0, y: 38 }, upper: { zoom: 1.55, x: 0, y: 18 }, full: { zoom: 1, x: 0, y: 0 } };
  const current = () => characterFraming?.[characterSelect.value]?.[layoutSelect.value];
  const updatePreview = () => {
    const image = preview.querySelector("img");
    image.style.transform = `translate(${Number(x.value) / 8}px,${Number(y.value) / 8}px) scale(${Number(zoom.value) / 1.55})`;
  };
  const load = async () => {
    const saved = current() || { preset: "upper", ...defaults.upper };
    const fallback = defaults[saved.preset] || saved;
    presetSelect.value = saved.preset || "custom";
    zoom.value = String(saved.zoom ?? fallback.zoom ?? 1.55);
    x.value = String(saved.x ?? fallback.x ?? 0);
    y.value = String(saved.y ?? fallback.y ?? 0);
    updatePreview();
    const character = characterCatalog.characters.find(item => item.id === characterSelect.value);
    const url = character ? await characterIconUrl(character) : "";
    if (url) preview.querySelector("img").src = url;
  };
  const save = () => {
    characterFraming[characterSelect.value] ||= {};
    characterFraming[characterSelect.value][layoutSelect.value] = { preset: presetSelect.value, zoom: Number(zoom.value), x: Number(x.value), y: Number(y.value) };
    updatePreview(); updateDirtyState();
  };
  presetSelect.addEventListener("change", () => {
    const next = defaults[presetSelect.value];
    if (next) { zoom.value = next.zoom; x.value = next.x; y.value = next.y; }
    save();
  });
  for (const input of [zoom, x, y]) input.addEventListener("input", () => { presetSelect.value = "custom"; save(); });
  layoutSelect.addEventListener("change", load);
  const reset = el("button", { type: "button", className: "secondary", textContent: "この構図を初期値へ戻す" });
  reset.addEventListener("click", async () => {
    const characterId = characterSelect.value;
    const layout = layoutSelect.value;
    if (characterFraming?.[characterId]) {
      delete characterFraming[characterId][layout];
      if (!Object.keys(characterFraming[characterId]).length) delete characterFraming[characterId];
    }
    await load();
    updateDirtyState();
  });
  const node = markSearchable(el("div", { className: "field framing-editor" }, [
    el("label", { textContent: "キャラごとの構図（表示形式別）" }),
    el("div", { className: "framing-controls" }, [layoutSelect, presetSelect, zoom, x, y]), preview,
    reset,
    el("p", { className: "hint", textContent: "胸上・上半身・全身を選び、必要なら拡大率と位置を微調整できます。キャラと表示形式ごとに保存します。" }),
  ]), "キャラ 構図 胸上 上半身 全身 拡大 位置 プレビュー");
  return { node, load };
}

function diagnosticRepairButton(label, handler) {
  const button = el("button", { type: "button", className: "diagnostic-repair", textContent: label });
  button.addEventListener("click", async () => {
    button.disabled = true;
    statusEl.textContent = `${label}を実行しています…`;
    try { await handler(); }
    catch (error) {
      logApp("error", `${label}に失敗しました`, errorDetail(error));
      statusEl.textContent = `${label}に失敗: ${error?.message || error}`;
      button.disabled = false;
    }
  });
  return button;
}

function buildDiagnosticsSection() {
  const diagnosticsSec = section("診断");
  const toolbar = el("div", { className: "diagnostics-toolbar" });
  const refresh = el("button", { type: "button", textContent: "更新" });
  const note = el("div", { className: "hint", textContent: "実行状態を確認し、停止している同梱機能はここから再起動できます。" });
  toolbar.append(refresh, note);
  const grid = el("div", { className: "diagnostic-grid" });
  diagnosticsSec.append(toolbar, grid);

  async function refreshDiagnostics() {
    refresh.disabled = true;
    grid.replaceChildren(diagnosticCard("診断", "取得中…", "状態を読み込んでいます", "neutral", ["診断"]));
    try {
      const [engineResult, aivisResult, usageResult, tracesResult] = await Promise.allSettled([
        window.picoagent.engineStatus(),
        window.picoagent.aivisStatus(),
        window.picoagent.usage(),
        window.picoagent.traces(),
      ]);
      grid.replaceChildren();
      grid.append(diagnosticCard(
        "叩くAI",
        data.providers.find(p => p.id === data.active.providerId)?.label || data.active.providerId,
        data.active.model || "モデル未選択",
        data.active.providerId ? "ok" : "warn",
        ["provider", "model", data.active.providerId, data.active.model]
      ));
      if (engineResult.status === "fulfilled") {
        const engine = engineResult.value || {};
        const phase = engine.phase || "";
        const progress = engine.percent != null ? ` / ${Math.round(Number(engine.percent))}%` : "";
        const engineCard = diagnosticCard(
          "同梱エンジン",
          enginePhaseLabel(phase),
          `${engine.message || "状態メッセージなし"}${progress}`,
          phase === "ready" ? "ok" : statusTone(false, phase !== "error"),
          ["engine", "local", phase, engine.message]
        );
        if (phase === "error") engineCard.append(diagnosticRepairButton("同梱エンジンを再起動", () => window.picoagent.restartApp(true, false)));
        grid.append(engineCard);
      } else {
        grid.append(diagnosticsFailureCard("同梱エンジン", engineResult.reason));
      }
      if (aivisResult.status === "fulfilled") {
        const aivis = aivisResult.value || {};
        const aivisCard = diagnosticCard(
          "Aivis",
          aivis.enabled ? (aivis.up ? "利用可能" : "起動待ち") : "無効",
          aivis.enabled ? "読み上げ設定は有効です" : "読み上げ設定は無効です",
          aivis.enabled ? statusTone(aivis.up, !aivis.up) : "neutral",
          ["aivis", "tts", "音声", "読み上げ"]
        );
        if (aivis.enabled && !aivis.up) aivisCard.append(diagnosticRepairButton("Aivisを再起動", () => window.picoagent.restartApp(false, true)));
        grid.append(aivisCard);
      } else {
        grid.append(diagnosticsFailureCard("Aivis", aivisResult.reason));
      }
      if (usageResult.status === "fulfilled") appendUsageSummary(grid, usageResult.value || {});
      else grid.append(diagnosticsFailureCard("使用量", usageResult.reason));
      if (tracesResult.status === "fulfilled") appendTraceSummary(grid, tracesResult.value || {});
      else grid.append(diagnosticsFailureCard("スキル実行", tracesResult.reason));
    } catch (error) {
      grid.replaceChildren(diagnosticsFailureCard("診断", error));
    } finally {
      refresh.disabled = false;
      applySettingsSearch();
    }
  }

  refresh.addEventListener("click", refreshDiagnostics);
  refreshDiagnostics();
  return diagnosticsSec;
}

function buildProductInfoSection() {
  const product = data.product || {};
  const license = data.license || { active: false, edition: "free", message: "無料版として利用中です。" };
  const productSec = section("製品情報・ライセンス");
  productSec.append(el("div", { className: "product-info-grid" }, [
    el("div", { className: "product-info-card" }, [
      el("span", { textContent: "製品" }),
      el("strong", { textContent: `${product.name || "PicoAgent"} ${product.version || ""}`.trim() }),
      el("small", { textContent: product.identifier || "com.picoagent.desktop" }),
    ]),
    el("div", { className: "product-info-card" }, [
      el("span", { textContent: "ライセンス" }),
      el("strong", { textContent: license.active ? `${license.edition.toUpperCase()} 有効` : "無料版" }),
      el("small", { textContent: license.customer ? `${license.customer} ・ ${license.message}` : license.message }),
    ]),
    el("div", { className: "product-info-card" }, [
      el("span", { textContent: "ローカルAI" }),
      el("strong", { textContent: "会話を外部AIへ送信しません" }),
      el("small", { textContent: "クラウドAIや外部連携を選んだ場合だけ、実行に必要な情報をその接続先へ送ります。" }),
    ]),
    el("div", { className: "product-info-card" }, [
      el("span", { textContent: "保存データ" }),
      el("strong", { textContent: "設定・会話・追加キャラはPC内" }),
      el("small", { textContent: "Windows版のアンインストール時は削除するか確認し、既定では残します。" }),
    ]),
    el("div", { className: "product-info-card" }, [
      el("span", { textContent: "第三者ソフトウェア" }),
      el("strong", { textContent: "各提供元の条件に従います" }),
      el("small", { textContent: "Live2D、Aivis、ローカルモデルなどの表示と配布条件は販売前監査の対象です。" }),
    ]),
  ]));
  const updateStatus = el("p", {
    className: "hint product-update-status",
    textContent: "「更新を確認」を押すまで通信しません。起動時確認がオフラインの場合は何も表示しません。",
  });
  const checkUpdate = el("button", { type: "button", className: "secondary", textContent: "更新を確認" });
  const installUpdate = el("button", { type: "button", className: "primary", textContent: "アップデート", hidden: true });
  checkUpdate.addEventListener("click", async () => {
    checkUpdate.disabled = true;
    installUpdate.hidden = true;
    updateStatus.textContent = "最新版を確認しています…";
    try {
      const result = await window.picoagent.settings.checkUpdate();
      updateStatus.textContent = result.message || "更新情報を確認しました。";
      installUpdate.hidden = !(result.status === "available" && result.downloadReady);
    } catch (error) {
      updateStatus.textContent = navigator.onLine
        ? `更新情報を確認できませんでした: ${error?.message || error}`
        : "オフラインのため更新確認を見送りました。";
    } finally {
      checkUpdate.disabled = false;
    }
  });
  installUpdate.addEventListener("click", async () => {
    installUpdate.disabled = true;
    checkUpdate.disabled = true;
    updateStatus.textContent = "アップデートをダウンロードしています…";
    try {
      const result = await window.picoagent.settings.installUpdate();
      updateStatus.textContent = result.message || "更新インストーラを起動します。";
      if (result.status === "offline") {
        installUpdate.disabled = false;
        checkUpdate.disabled = false;
      }
    } catch (error) {
      updateStatus.textContent = navigator.onLine
        ? `アップデートできませんでした: ${error?.message || error}`
        : "オフラインのためアップデートを見送りました。";
      installUpdate.disabled = false;
      checkUpdate.disabled = false;
    }
  });
  const updateActions = el("div", { className: "product-update-actions" }, [checkUpdate, installUpdate]);
  productSec.append(el("div", { className: "product-update-card" }, [
    el("strong", { textContent: "アプリのアップデート" }),
    updateStatus,
    updateActions,
  ]));
  if (updateProgressUnlisten) updateProgressUnlisten();
  window.picoagent.listen("update:progress", progress => {
    const percent = progress.total > 0 ? Math.min(100, Math.round(progress.downloaded / progress.total * 100)) : 0;
    updateStatus.textContent = `アップデートをダウンロードしています… ${percent}%`;
  }).then(unlisten => { updateProgressUnlisten = unlisten; }).catch(error => {
    logApp("warn", "アップデート進捗を受け取れませんでした", errorDetail(error));
  });
  productSec.append(field(
    "PICOAGENT_LICENSE_TOKEN",
    "BOOTHなどで購入したライセンスキー",
    "password",
    data.values?.PICOAGENT_LICENSE_TOKEN || "",
    "PA1. から始まるキーを貼り付け"
  ));
  productSec.append(el("p", {
    className: "hint",
    textContent: "無料版は会話・天気・時刻などを利用できます。ProではPC操作、ファイル編集、Office・外部サービス・MCPなどの高度なスキルが有効になります。キーはWindowsのDPAPIで暗号化して保存します。Steam版は将来Steamworksの所有確認へ対応予定です。",
  }));
  return markSearchable(productSec, "製品 バージョン データ プライバシー ライセンス サポート 更新");
}

function buildDownloadedModelsSection() {
  const sec = section("ダウンロード済みモデル");
  sec.append(el("p", { className: "hint", textContent: "ローカルエンジンが取得した GGUF モデルです。使わないモデルを削除してディスクを空けられます（使用中のモデルは削除できません）。" }));
  const toolbar = el("div", { className: "diagnostics-toolbar" });
  const refresh = el("button", { type: "button", textContent: "更新" });
  const total = el("div", { className: "hint", textContent: "" });
  toolbar.append(refresh, total);
  const list = el("div", { className: "model-list" });
  sec.append(toolbar, list);

  async function refreshModels() {
    refresh.disabled = true;
    list.replaceChildren(el("div", { className: "hint", textContent: "読み込み中…" }));
    try {
      const result = await window.picoagent.listDownloadedModels();
      const models = Array.isArray(result?.models) ? result.models : [];
      total.textContent = models.length ? `合計 ${result.totalLabel}（${models.length}件）` : "";
      list.replaceChildren();
      if (!models.length) {
        list.append(el("div", { className: "hint", textContent: "ダウンロード済みのモデルはありません。" }));
        return;
      }
      for (const model of models) {
        const meta = [model.label || model.name, model.sizeLabel].filter(Boolean).join(" ・ ");
        const info = el("div", { className: "model-info" }, [
          el("div", { className: "model-name", textContent: model.name }),
          el("div", { className: "model-meta", textContent: meta }),
        ]);
        const action = model.active
          ? el("span", { className: "model-active", textContent: "使用中" })
          : el("button", { type: "button", className: "model-delete", textContent: "削除" });
        if (!model.active) {
          action.addEventListener("click", async () => {
            if (!confirm(`モデル「${model.name}」を削除します。よろしいですか？\n（再度使うときは再ダウンロードが必要です）`)) return;
            action.disabled = true;
            try {
              await window.picoagent.deleteDownloadedModel(model.name);
              await refreshModels();
            } catch (error) {
              logApp("warn", "モデル削除に失敗しました", `${model.name}: ${errorDetail(error)}`);
              action.disabled = false;
              alert(`削除に失敗しました: ${error?.message || error}`);
            }
          });
        }
        list.append(el("div", { className: "model-row" }, [info, action]));
      }
    } catch (error) {
      logApp("warn", "モデル一覧の取得に失敗しました", errorDetail(error));
      list.replaceChildren(el("div", { className: "hint", textContent: `一覧の取得に失敗しました: ${error?.message || error}` }));
    } finally {
      refresh.disabled = false;
      applySettingsSearch();
    }
  }

  refresh.addEventListener("click", refreshModels);
  refreshModels();
  return sec;
}

async function render() {
  renderingSettings = true;
  form.innerHTML = "";
  inputs.clear();
  inputWrappers.clear();
  alarmEditorState = null;
  scheduleEditorState = null;
  const v = data.values;
  const panels = createTabs([
    { id: "basic", label: "基本設定" },
    { id: "display", label: "画面・操作" },
    { id: "character", label: "キャラ・音声" },
    { id: "components", label: "追加機能" },
    { id: "providers", label: "AIモデル・連携" },
    { id: "tools", label: "スキル" },
    { id: "permissions", label: "安全・許可" },
    { id: "skillguide", label: "使い方" },
    { id: "diagnostics", label: "診断・修復" },
  ]);
  panels.components.append(buildComponentManager());

  // --- グローバル設定 ---
  const global = section("基本");
  global.append(el("p", {
    className: "settings-intro",
    textContent: "普段変更するのは、使うAI・記憶・居眠り・更新確認です。プロンプトやホットキーなどは下の「詳細設定」にまとめています。",
  }));
  const providerSelect = el("select");
  for (const p of data.providers) {
    providerSelect.append(el("option", { value: p.id, textContent: p.label, selected: p.id === data.active.providerId }));
  }
  global.append(registerInput(
    "PICOAGENT_PROVIDER",
    providerSelect,
    markSearchable(el("div", { className: "field" }, [el("label", { textContent: "使うAI" }), providerSelect]), "PICOAGENT_PROVIDER", "叩く AI", "使うAI", "プロバイダ")
  ));
  const modelField = field("PICOAGENT_MODEL", "AIモデル", "text", v.PICOAGENT_MODEL ?? data.active.model, data.active.model);
  global.append(modelField);
  modelInput = inputs.get("PICOAGENT_MODEL");
  // Ollama だけは実行中のローカル一覧を基本タブのモデル欄に出す。
  // 他プロバイダは「AIモデル・連携」タブで種類とサイズを選ぶため、ここでは隠す。
  function updateModelFieldVisibility() {
    const hidden = providerSelect.value !== "ollama";
    modelField.dataset.conditionHidden = hidden ? "true" : "false";
    modelField.hidden = hidden;
  }
  updateModelFieldVisibility();
  const displaySec = section("画面レイアウト");
  displaySec.append(el("p", {
    className: "settings-intro",
    textContent: "用途に合う画面レイアウトをプレビューから選べます。キャラは全形式で上半身を基準に表示し、ウィンドウは端をドラッグして拡大・縮小できます。",
  }));
  displaySec.append(layoutPickerField(v.PICOAGENT_LAYOUT || "pet"));
  displaySec.append(selectField("PICOAGENT_THEME", "表示テーマ", v.PICOAGENT_THEME || "system", [
    { value: "system", label: "システム設定に追従" },
    { value: "light", label: "通常" },
    { value: "dark", label: "ダーク" },
  ]));
  displaySec.append(field("PICOAGENT_BUBBLE_FONT_PX", "会話文字の大きさ（px）", "text", v.PICOAGENT_BUBBLE_FONT_PX ?? "", "13"));
  displaySec.append(field("PICOAGENT_ALWAYS_ON_TOP", "常に手前に表示", "checkbox", v.PICOAGENT_ALWAYS_ON_TOP !== false));
  displaySec.append(windowPlacementField());
  displaySec.append(streamSettingsField(v));
  displaySec.append(layoutResetField());
  global.append(field("PICOAGENT_MEMORY_ENABLED", "会話から好みや継続指示を暗黙的に覚える", "checkbox", v.PICOAGENT_MEMORY_ENABLED !== false));
  global.append(field("PICOAGENT_NAP_MINUTES", "無操作で居眠りするまでの分数（0で居眠りしない）", "text", v.PICOAGENT_NAP_MINUTES ?? "", "15"));
  global.append(field("PICOAGENT_UPDATE_CHECK_ENABLED", "起動時に新しいバージョンを確認する", "checkbox", v.PICOAGENT_UPDATE_CHECK_ENABLED !== false));
  // 変更頻度の低い項目は閉じておき、基本画面を短く保つ。
  global.append(advancedGroup("詳細設定", "困っていなければ変更する必要はありません。", [
    field("PICOAGENT_SYSTEM_PROMPT", "全キャラ共通の追加指示", "textarea", v.PICOAGENT_SYSTEM_PROMPT ?? "", data.defaults.systemPrompt),
    field("PICOAGENT_HOTKEY_ENABLED", "グローバルホットキーを使う", "checkbox", v.PICOAGENT_HOTKEY_ENABLED !== false),
    field("PICOAGENT_HOTKEY_SUMMON", "呼び出すキー", "text", v.PICOAGENT_HOTKEY_SUMMON ?? "", "CommandOrControl+Alt+P"),
    field("PICOAGENT_HOTKEY_HIDE", "隠す／戻すキー", "text", v.PICOAGENT_HOTKEY_HIDE ?? "", "CommandOrControl+Alt+H"),
    field("PICOAGENT_MEMORY_MAX_ITEMS", "長期メモリの最大件数", "text", v.PICOAGENT_MEMORY_MAX_ITEMS ?? "", "80"),
    field("PICOAGENT_SUPER_AGENT", "計画・自己評価・分担を強化する", "checkbox", v.PICOAGENT_SUPER_AGENT === true),
    field("PICOAGENT_SUPER_AGENT_MAX_STEPS", "強化モードの最大ループ数", "text", v.PICOAGENT_SUPER_AGENT_MAX_STEPS ?? "", "12"),
    field("PICOAGENT_QUICK_ACTIONS", "クイックアクション（1行1件「ラベル|送る文章」）", "textarea", v.PICOAGENT_QUICK_ACTIONS ?? "", "👀 画面を見る|今の画面を見て、状況を短く教えて"),
  ]));
  // ローカルAPI（Ollama風）: 外部ツールから 127.0.0.1 の HTTP でエージェントを呼べる。
  const apiSec = section("ローカルAPI（Ollama風）");
  apiSec.append(field("PICOAGENT_API_ENABLED", "ローカルAPIを有効にする（127.0.0.1 のみ。変更は再起動後に反映）", "checkbox", v.PICOAGENT_API_ENABLED === true));
  apiSec.append(field("PICOAGENT_API_PORT", "APIポート", "text", v.PICOAGENT_API_PORT ?? "", "11500"));
  apiSec.append(field("PICOAGENT_API_KEY", "APIキー（空なら認証なし。Authorization: Bearer で送る）", "password", v.PICOAGENT_API_KEY ?? ""));
  // MCP: 外部 MCP サーバのツールをエージェントから使えるようにする（stdio）。
  const mcpSec = section("MCP（外部ツール連携）");
  mcpSec.append(field(
    "PICOAGENT_MCP_SERVERS",
    "外部 MCP サーバ（1行1件「名前|コマンド 引数…」。stdio 接続。# でコメント。変更は再起動後に反映）",
    "textarea",
    v.PICOAGENT_MCP_SERVERS ?? "",
    "memory|npx -y @modelcontextprotocol/server-memory"
  ));
  const voiceSec = section("読み上げ");
  const ttsMasterStatus = el("div", {
    className: v.PICOAGENT_TTS_ENABLED === true ? "tts-master-status enabled" : "tts-master-status muted",
    textContent: v.PICOAGENT_TTS_ENABLED === true ? "読み上げ：有効" : "読み上げ：ミュート中（起動時も話しません）",
  });
  voiceSec.append(ttsMasterStatus);
  voiceSec.append(el("p", {
    className: "hint",
    textContent: "読み上げは必要な時だけ有効にできます。キャラごとの声は上のキャラクター選択に連動します。",
  }));
  voiceSec.append(field("PICOAGENT_TTS_ENABLED", "Aivis で回答を読み上げる", "checkbox", v.PICOAGENT_TTS_ENABLED === true));
  inputs.get("PICOAGENT_TTS_ENABLED")?.addEventListener("change", event => {
    const enabled = event.target.checked;
    ttsMasterStatus.className = `tts-master-status ${enabled ? "enabled" : "muted"}`;
    ttsMasterStatus.textContent = enabled ? "読み上げ：有効" : "読み上げ：ミュート中（起動時も話しません）";
    const selectedEntry = characterCatalog?.characters?.find(character => character.id === characterSelect?.value);
    setQuickSettingsSummary("character", `${selectedEntry?.name || characterSelect?.value || v.PICOAGENT_CHARACTER || "既定キャラ"}・${enabled ? "読み上げ有効" : "ミュート中"}`);
  });
  voiceSec.append(field("PICOAGENT_TTS_VOLUME", "キャラボイスの音量（0〜100）", "number", v.PICOAGENT_TTS_VOLUME ?? "100", "100"));
  panels.basic.append(quickSettingsGuide(v), global);
  panels.display.append(displaySec);
  providerSelect.addEventListener("change", () => {
    const selected = data.providers.find(item => item.id === providerSelect.value);
    setQuickSettingsSummary("providers", selected?.label || providerSelect.value);
  });
  inputs.get("PICOAGENT_LAYOUT")?.addEventListener("change", event => {
    const selected = LAYOUT_CHOICES.find(item => item.id === event.target.value);
    setQuickSettingsSummary("display", selected?.label || event.target.value);
  });
  applySettingsTheme(v.PICOAGENT_THEME || "system");
  inputs.get("PICOAGENT_THEME")?.addEventListener("change", event => {
    applySettingsTheme(event.target.value);
  });

  // --- キャラクター ---
  const characterResponse = await fetch(`assets/characters/index.json?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!characterResponse.ok) throw new Error(`キャラクター一覧: HTTP ${characterResponse.status}`);
  characterCatalog = await characterResponse.json();
  // 同梱キャラは配信物からの相対パス、追加キャラは実ファイルの配信 URL を root にする。
  // 以降のマニフェスト読込・アイコン表示は root だけを見る（置き場の違いを1箇所に閉じ込める）。
  for (const character of characterCatalog.characters) {
    character.root = `assets/characters/${character.manifest.replace(/\/character\.json$/, "")}`;
  }
  characterCatalog.characters = characterCatalog.characters.concat(await loadUserCharacters(characterCatalog.characters));
  await Promise.all(characterCatalog.characters.map(async character => {
    try {
      const manifest = await loadCharacterManifestForSettings(character);
      character.renderType = characterRenderType(character, manifest);
      character.renderKind = characterRenderKind(manifest);
    } catch (error) {
      character.renderType = character.user ? "追加パック" : "形式不明";
      character.renderKind = "unknown";
      logApp("warn", "キャラクター形式を判定できませんでした", `${character.id}: ${errorDetail(error)}`);
    }
  }));
  characterOverrides = v.PICOAGENT_CHARACTER_OVERRIDES || {};
  characterVoiceOverrides = v.PICOAGENT_TTS_CHARACTER_OVERRIDES || {};
  characterLive2dSdkOverrides = v.PICOAGENT_LIVE2D_SDK_OVERRIDES || {};
  characterFraming = cloneSettings(v.PICOAGENT_CHARACTER_FRAMING || {});
  characterStreamProfiles = cloneSettings(v.PICOAGENT_STREAM_PROFILES || {});
  const characterSec = section("キャラクター設定");
  characterSelect = el("select");
  const requestedCharacter = v.PICOAGENT_CHARACTER || characterCatalog.default;
  const selectedCharacter = characterCatalog.characters.some(character => character.id === requestedCharacter)
    ? requestedCharacter
    : characterCatalog.default;
  const selectedCharacterEntry = characterCatalog.characters.find(character => character.id === selectedCharacter);
  setQuickSettingsSummary("character", `${selectedCharacterEntry?.name || selectedCharacter}・${v.PICOAGENT_TTS_ENABLED === true ? "読み上げ有効" : "ミュート中"}`);
  for (const character of characterCatalog.characters) {
    characterSelect.append(el("option", {
      value: character.id,
      textContent: `${character.name} — ${character.renderType} [${character.id}]`,
      selected: character.id === selectedCharacter,
    }));
  }
  characterSec.append(registerInput(
    "PICOAGENT_CHARACTER",
    characterSelect,
    markSearchable(el("div", { className: "field" }, [el("label", { textContent: "キャラクター" }), characterSelect]), "PICOAGENT_CHARACTER", "キャラクター")
  ));
  characterSec.append(selectField("PICOAGENT_COMPANION_MODE", "相棒の役割", v.PICOAGENT_COMPANION_MODE || "both", [
    { value: "life", label: "いっしょに暮らす" },
    { value: "work", label: "いっしょに働く" },
    { value: "both", label: "暮らしと仕事の両方" },
  ]));
  characterSec.append(field("PICOAGENT_RELATIONSHIP_MEMORY_ENABLED", "いっしょの記録を保存する", "checkbox", v.PICOAGENT_RELATIONSHIP_MEMORY_ENABLED !== false));
  characterFramingEditor = buildCharacterFramingEditor();
  characterSec.append(characterFramingEditor.node);
  await characterFramingEditor.load();
  characterStreamProfileEditor = buildCharacterStreamProfileEditor(v);
  characterSec.append(characterStreamProfileEditor.node);
  characterStreamProfileEditor.load();
  characterLive2dSdkSelect = el("select");
  for (const option of [
    { value: "auto", label: "自動（推奨）" },
    { value: "5", label: "Cubism 5（5.3形式）に固定" },
    { value: "4", label: "Cubism 4 に固定" },
  ]) characterLive2dSdkSelect.append(el("option", { value: option.value, textContent: option.label }));
  characterLive2dSdkHint = el("p", { className: "hint" });
  characterLive2dSdkField = markSearchable(el("div", { className: "field" }, [
    el("label", { textContent: "Live2D の SDK 世代" }),
    characterLive2dSdkSelect,
    characterLive2dSdkHint,
  ]), "Live2D Cubism SDK 4 5 自動 互換");
  characterLive2dSdkSelect.addEventListener("change", updateDirtyState);
  characterSec.append(characterLive2dSdkField);
  characterPersona = el("textarea", { placeholder: "性格・口調・一人称など" });
  characterSec.append(el("div", { className: "field" }, [el("label", { textContent: "このキャラの性格・口調" }), characterPersona]));
  await loadCharacterPersona(selectedCharacter);
  loadCharacterVoice(selectedCharacter);
  await loadCharacterLive2dSdk(selectedCharacter);
  let previousCharacter = selectedCharacter;
  let characterSwitchQueue = Promise.resolve();
  characterSelect.addEventListener("change", event => {
    // 人格の読込は非同期なので、form の委譲 change が先に collect() すると、切替前の
    // 人格を新しいキャラの上書き値として保存してしまう。切替完了後に未保存判定する。
    event.stopPropagation();
    const requestedCharacter = characterSelect.value;
    // 素早い連続選択でも、表示中の人格を読み終えてから次へ進む。並列に読むと、
    // 遅く完了した古い読込が新しいキャラの編集欄を上書きしてしまう。
    characterSwitchQueue = characterSwitchQueue.then(async () => {
      characterOverrides[previousCharacter] = characterPersona.value;
      saveCurrentCharacterVoice(previousCharacter);
      saveCurrentCharacterLive2dSdk(previousCharacter);
      try {
        await loadCharacterPersona(requestedCharacter);
        loadCharacterVoice(requestedCharacter);
        await loadCharacterLive2dSdk(requestedCharacter);
        await characterFramingEditor?.load();
        characterStreamProfileEditor?.load();
        previousCharacter = requestedCharacter;
        characterSelect.value = requestedCharacter;
        const selectedEntry = characterCatalog.characters.find(character => character.id === requestedCharacter);
        setQuickSettingsSummary("character", `${selectedEntry?.name || requestedCharacter}・${inputs.get("PICOAGENT_TTS_ENABLED")?.checked ? "読み上げ有効" : "ミュート中"}`);
      } catch (error) {
        characterSelect.value = previousCharacter;
        statusEl.textContent = `人格設定を読み込めませんでした: ${error.message}`;
      }
      refreshCharacterCards();
      updateDirtyState();
    });
  });
  // 関係性は人格そのものではなく「人格の上に重ねる差分」なので、キャラ選択とは別の欄にする。
  const relationshipSelect = el("select");
  const relationships = Array.isArray(data.relationships) ? data.relationships : [];
  const selectedRelationship = v.PICOAGENT_RELATIONSHIP || "assistant";
  for (const relationship of relationships) {
    relationshipSelect.append(el("option", {
      value: relationship.id,
      textContent: relationship.name,
      selected: relationship.id === selectedRelationship,
    }));
  }
  const relationshipHint = el("p", { className: "hint" });
  const describeRelationship = () => {
    const found = relationships.find(entry => entry.id === relationshipSelect.value);
    relationshipHint.textContent = found ? found.description : "";
    relationshipCustomField.style.display = relationshipSelect.value === "custom" ? "" : "none";
  };
  const relationshipCustom = el("textarea", { placeholder: "例: ご近所さんとして接する。世間話を挟みつつ、頼まれごとは丁寧にこなす。" });
  relationshipCustom.value = v.PICOAGENT_RELATIONSHIP_CUSTOM || "";
  const relationshipCustomField = el("div", { className: "field" }, [
    el("label", { textContent: "関係の自由設定" }),
    relationshipCustom,
  ]);
  relationshipSelect.addEventListener("change", describeRelationship);
  characterSec.append(registerInput(
    "PICOAGENT_RELATIONSHIP",
    relationshipSelect,
    markSearchable(el("div", { className: "field" }, [
      el("label", { textContent: "ユーザーとの関係" }),
      relationshipSelect,
      relationshipHint,
    ]), "PICOAGENT_RELATIONSHIP", "ユーザーとの関係 関係性 恋人 友達 妹 姉 先輩")
  ));
  characterSec.append(registerInput("PICOAGENT_RELATIONSHIP_CUSTOM", relationshipCustom, relationshipCustomField));
  describeRelationship();
  panels.character.append(characterSec);
  panels.character.append(voiceSec);

  // --- スキル設定（⚙ のあるスキルはクリックでその場に開く）。活用ガイドとは別タブに分ける ---
  const skillSec = section("スキル設定");
  if (!data.tools.length) {
    skillSec.append(el("p", { className: "hint", textContent: "登録されているスキルはありません。" }));
  } else {
    skillSec.append(el("p", { className: "hint", textContent: "カテゴリ別の一覧です。ジャンルで絞り込めます。⚙ のあるスキルをクリックすると設定を開けます（既定は折りたたみ）。頼み方の例は「使い方」タブを参照。" }));
    skillSec.append(buildGenreChips(() => skillGenreFilter, value => { skillGenreFilter = value; }));
    skillSec.append(buildSkillCategoryList(data.tools, v));
  }
  panels.tools.append(skillSec);

  // --- スキル活用ガイド（読み取り専用の一覧）。設定とは分離して別タブに置く ---
  const skillGuideSec = section("スキル活用ガイド");
  if (!data.tools.length) {
    skillGuideSec.append(el("p", { className: "hint", textContent: "登録されているスキルはありません。" }));
  } else {
    skillGuideSec.append(el("p", { className: "hint", textContent: "各スキルの「できること」と頼み方の例の一覧です。ジャンルで絞り込めます。設定（APIキー等）は「スキル」タブで行います。" }));
    skillGuideSec.append(buildGenreChips(() => guideGenreFilter, value => { guideGenreFilter = value; }));
    skillGuideSec.append(buildSkillGuide(data.tools));
  }
  const onboardingGuideSec = section("初回セットアップ");
  const reopenOnboardingStatus = el("p", { className: "hint" });
  const reopenOnboardingButton = el("button", {
    className: "primary",
    type: "button",
    textContent: "初回セットアップをもう一度開く",
  });
  reopenOnboardingButton.addEventListener("click", async () => {
    reopenOnboardingButton.disabled = true;
    reopenOnboardingStatus.textContent = "メイン画面へ切り替えています…";
    try {
      await window.picoagent.showOnboarding();
      reopenOnboardingStatus.textContent = "";
    } catch (error) {
      reopenOnboardingStatus.textContent = `開けませんでした: ${error?.message || error}`;
    } finally {
      reopenOnboardingButton.disabled = false;
    }
  });
  onboardingGuideSec.append(
    el("p", { className: "hint", textContent: "用途の選択とLive2D追加案内をいつでもやり直せます。会話履歴、設定、追加済みキャラクターは削除しません。" }),
    reopenOnboardingButton,
    reopenOnboardingStatus,
  );
  panels.skillguide.append(onboardingGuideSec, skillGuideSec);

  // --- 導入済みキャラクターの検索・形式フィルター・選択 ---
  const charSec = section("キャラクターを探して選ぶ");
  charSec.append(el("p", {
    className: "hint",
    textContent: "名前・ID・説明で検索し、描画形式で絞り込めます。選択後は上の性格・口調などを調整して「保存」してください。",
  }));
  const characterSearch = el("input", {
    className: "character-browser-search",
    type: "search",
    autocomplete: "off",
    placeholder: "キャラクター名・ID・説明を検索",
    value: characterSearchQuery,
  });
  characterSearch.addEventListener("input", () => {
    characterSearchQuery = characterSearch.value.trim().toLowerCase();
    applySettingsSearch();
  });
  const filterDefinitions = [
    { id: "all", label: "すべて" },
    { id: "live2d", label: "Live2D" },
    { id: "png", label: "PNGアニメ" },
    { id: "tha3", label: "自作THA3" },
    { id: "user", label: "追加キャラ" },
  ];
  const filterBar = el("div", { className: "genre-filter character-type-filter" });
  const filterButtons = new Map();
  const refreshFilterButtons = () => {
    for (const [id, button] of filterButtons) {
      const active = id === characterTypeFilter;
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    }
  };
  for (const filter of filterDefinitions) {
    const button = el("button", { type: "button", className: "genre-chip", textContent: filter.label });
    button.addEventListener("click", () => {
      characterTypeFilter = filter.id;
      refreshFilterButtons();
      applySettingsSearch();
    });
    filterButtons.set(filter.id, button);
    filterBar.append(button);
  }
  refreshFilterButtons();
  const count = el("span", { className: "character-browser-count", ariaLive: "polite" });
  charSec.append(el("div", { className: "character-browser-controls" }, [characterSearch, filterBar, count]));
  const charList = el("div", { className: "card-list character-browser-list" });
  const refreshCharacterCards = () => {
    charList.querySelectorAll(".character-card").forEach(card => {
      const active = card.dataset.characterId === characterSelect.value;
      card.classList.toggle("selected", active);
      const button = card.querySelector(".character-select-button");
      if (button) {
        button.disabled = active;
        button.textContent = active ? "選択中" : "このキャラを選ぶ";
      }
    });
  };
  characterSelect.addEventListener("change", refreshCharacterCards);
  for (const character of characterCatalog.characters) {
    const fallback = el("div", { className: "character-icon-fallback", textContent: String(character.name || character.id || "?").slice(0, 1) });
    const icon = el("img", { className: "character-icon", alt: `${character.name}のアイコン`, draggable: false });
    icon.hidden = true;
    characterIconUrl(character).then(url => {
      if (!url) return;
      icon.src = url;
      icon.hidden = false;
      fallback.hidden = true;
    });
    const selectButton = el("button", { className: "secondary character-select-button", type: "button" });
    selectButton.addEventListener("click", () => {
      characterSelect.value = character.id;
      characterSelect.dispatchEvent(new Event("change"));
      statusEl.textContent = `${character.name} [${character.id}] を選びました。「保存」で反映します。`;
    });
    const texts = [
      el("div", { className: "card-name" }, [
        document.createTextNode(character.name),
        el("span", { className: "config-badge ok", textContent: character.renderType }),
      ]),
      el("div", { className: "card-desc character-id", textContent: `ID: ${character.id}` }),
      el("div", { className: "card-desc", textContent: character.description || character.id }),
      selectButton,
    ];
    // 削除できるのは自分で追加したものだけ（同梱キャラは実行ファイル側にあるため消せない）。
    if (character.user) {
      const removeStatus = el("div", { className: "card-desc" });
      const removeButton = el("button", { className: "secondary", type: "button", textContent: "削除" });
      removeButton.addEventListener("click", async () => {
        if (removeButton.dataset.confirm !== "1") {
          removeButton.dataset.confirm = "1";
          removeButton.textContent = "もう一度押すと削除";
          removeStatus.textContent = "OS のゴミ箱へ移動します。";
          return;
        }
        removeButton.disabled = true;
        try {
          await window.picoagent.settings.removeCharacter(character.id);
          removeStatus.textContent = "ゴミ箱へ移動しました。「設定」を開き直すと一覧から消えます。";
        } catch (error) {
          removeButton.disabled = false;
          removeStatus.textContent = `削除できませんでした: ${error?.message || error}`;
        }
      });
      texts.push(removeStatus, removeButton);
    }
    const card = markSearchable(el("div", { className: "card-item character-card" }, [
      el("div", { className: "character-icon-wrap" }, [fallback, icon]),
      el("div", { className: "character-card-text" }, texts),
    ]), character.id, character.name, character.description, character.renderType, character.user ? "追加キャラ" : "同梱キャラ");
    card.dataset.characterId = character.id;
    card.dataset.characterRender = character.renderKind;
    card.dataset.characterOrigin = character.user ? "user" : "bundled";
    charList.append(card);
  }
  refreshCharacterCards();
  const empty = el("p", { className: "hint character-browser-empty", textContent: "条件に合うキャラクターが見つかりません。" });
  charSec.append(charList, empty);
  applyCharacterFilters();

  // --- 1枚絵からTHA3自作キャラクターを生成する ---
  // 生成物は一時パックとして検証してから app_data_dir/characters へ導入する。
  // Pythonプロセスはバックエンドで管理し、設定画面を閉じた場合もキャンセルして回収する。
  const tha3Sec = section("1枚絵からキャラクターを作成（THA3）");
  const tha3Hint = el("p", {
    className: "hint",
    textContent: "512×512の透過PNGから、待機・呼吸・左右揺れ・まばたき・口パクなど17キーポーズ／42フレームを生成します。WindowsはVRAMに合わせてモデルを自動調整し、Apple Silicon MacはMPSを使います。",
  });
  const tha3Credit = el("p", {
    className: "hint",
    textContent: "THA3: © Pramook Khungurn (pkhungurn)。コード MIT License、学習済みモデル CC BY 4.0。",
  });
  const tha3ToolStatus = el("div", { className: "tha3-tool-status", textContent: "制作ツールを確認中…" });
  const pickTha3RootButton = el("button", { className: "secondary", type: "button", textContent: "THA3フォルダを選ぶ" });
  const sourceInput = el("input", { type: "text", readOnly: true, placeholder: "透過PNGを選んでください" });
  const sourcePreview = el("img", { className: "tha3-source-preview", alt: "選択した立ち絵のプレビュー" });
  sourcePreview.hidden = true;
  const pickSourceButton = el("button", { className: "secondary", type: "button", textContent: "画像を選ぶ" });
  const sourceActions = el("div", { className: "character-add-actions" }, [pickSourceButton]);
  const idInput = el("input", {
    type: "text",
    value: `custom-${Date.now().toString(36)}`,
    placeholder: "例: my-character",
    maxLength: 64,
  });
  const nameInput = el("input", { type: "text", value: "マイキャラ", maxLength: 80 });
  const createButton = el("button", { className: "primary", type: "button", textContent: "キャラクターを作成" });
  const cancelButton = el("button", { className: "secondary", type: "button", textContent: "生成をキャンセル" });
  cancelButton.hidden = true;
  const progress = el("pre", { className: "tha3-progress", textContent: "" });
  progress.hidden = true;
  const createStatus = el("p", { className: "hint" });
  let selectedSource = "";
  let selectedTha3Root = "";
  let toolReady = false;

  function updateCreateEnabled() {
    createButton.disabled = tha3GenerationRunning || !toolReady || !selectedSource || !idInput.value.trim() || !nameInput.value.trim();
    pickSourceButton.disabled = tha3GenerationRunning;
    pickTha3RootButton.disabled = tha3GenerationRunning;
    idInput.disabled = tha3GenerationRunning;
    nameInput.disabled = tha3GenerationRunning;
    cancelButton.hidden = !tha3GenerationRunning;
  }

  pickSourceButton.addEventListener("click", async () => {
    createStatus.textContent = "画像を選択中…";
    try {
      const picked = await window.picoagent.settings.pickTha3Source();
      if (!picked?.path) {
        createStatus.textContent = "画像選択をキャンセルしました。";
        return;
      }
      selectedSource = picked.path;
      sourceInput.value = picked.path;
      sourcePreview.src = picked.dataUrl;
      sourcePreview.onerror = () => { sourcePreview.hidden = true; };
      sourcePreview.hidden = false;
      createStatus.textContent = "画像を選びました。512×512・背景透過・正面向きか確認してください。";
    } catch (error) {
      createStatus.textContent = `画像を選べませんでした: ${error?.message || error}`;
    } finally {
      updateCreateEnabled();
    }
  });
  idInput.addEventListener("input", updateCreateEnabled);
  nameInput.addEventListener("input", updateCreateEnabled);

  async function refreshTha3Status(rootPath = null) {
    const tha3Status = await window.picoagent.settings.tha3Status(rootPath);
    toolReady = Boolean(tha3Status.ready);
    // 自動検出パスはバックエンド内部で再検出する。ユーザーが選んだ場合だけ明示パスを渡す。
    selectedTha3Root = toolReady && rootPath ? tha3Status.root : "";
    tha3ToolStatus.textContent = toolReady
      ? `同梱制作ツール準備済み: ${tha3Status.root}`
      : `${tha3Status.installHint} 本体=${tha3Status.sourceReady ? "OK" : "不足"}、モデル=${tha3Status.modelReady ? "OK" : "不足"}、Python=${tha3Status.runtimeReady ? "OK" : "不足"}。確認先: ${tha3Status.root}`;
    tha3ToolStatus.classList.toggle("ready", toolReady);
    updateCreateEnabled();
  }

  pickTha3RootButton.addEventListener("click", async () => {
    createStatus.textContent = "THA3フォルダを選択中…";
    try {
      const picked = await window.picoagent.settings.pickTha3Root();
      if (!picked?.path) {
        createStatus.textContent = "フォルダ選択をキャンセルしました。";
        return;
      }
      await refreshTha3Status(picked.path);
      createStatus.textContent = toolReady ? "THA3制作ツールを確認しました。" : "必要な本体またはモデルが不足しています。";
    } catch (error) {
      toolReady = false;
      selectedTha3Root = "";
      tha3ToolStatus.classList.remove("ready");
      tha3ToolStatus.textContent = `選択したフォルダを使えません: ${error?.message || error}`;
      createStatus.textContent = "THA3フォルダを確認できませんでした。";
      updateCreateEnabled();
    }
  });

  if (tha3ProgressUnlisten) {
    tha3ProgressUnlisten();
    tha3ProgressUnlisten = null;
  }
  tha3ProgressUnlisten = await window.picoagent.listen("tha3:progress", event => {
    const line = String(event?.message || "").trim();
    if (!line) return;
    progress.hidden = false;
    progress.textContent = `${progress.textContent}${progress.textContent ? "\n" : ""}${line}`.slice(-6000);
    progress.scrollTop = progress.scrollHeight;
  });

  cancelButton.addEventListener("click", async () => {
    cancelButton.disabled = true;
    createStatus.textContent = "生成を停止し、GPUと一時ファイルを解放しています…";
    try {
      await window.picoagent.settings.cancelTha3Generation();
    } catch (error) {
      createStatus.textContent = `キャンセル要求に失敗しました: ${error?.message || error}`;
    }
  });

  createButton.addEventListener("click", async () => {
    tha3GenerationRunning = true;
    progress.textContent = "";
    progress.hidden = false;
    createStatus.textContent = "生成中です。初回はモデル読込を含め数分かかることがあります…";
    updateCreateEnabled();
    try {
      const result = await window.picoagent.settings.generateTha3Character(
        selectedSource,
        selectedTha3Root,
        idInput.value.trim(),
        nameInput.value.trim(),
      );
      createStatus.textContent = `${result.name}（${result.id}）を作成しました。`;
      statusEl.textContent = `${result.name}を追加しました。キャラクター一覧を更新します。`;
      data.values = collect();
      tha3GenerationRunning = false;
      await render();
      activateSettingsTab("character");
    } catch (error) {
      createStatus.textContent = `作成できませんでした: ${error?.message || error}`;
      logApp("error", "THA3キャラクター生成に失敗しました", errorDetail(error));
    } finally {
      tha3GenerationRunning = false;
      cancelButton.disabled = false;
      updateCreateEnabled();
    }
  });

  try {
    await refreshTha3Status();
  } catch (error) {
    tha3ToolStatus.textContent = `制作ツールを確認できませんでした: ${error?.message || error}`;
  }
  updateCreateEnabled();
  tha3Sec.append(
    tha3Hint,
    tha3Credit,
    tha3ToolStatus,
    el("div", { className: "character-add-actions" }, [pickTha3RootButton]),
    el("div", { className: "field" }, [el("label", { textContent: "元画像" }), sourceInput, sourceActions]),
    sourcePreview,
    el("div", { className: "field" }, [el("label", { textContent: "キャラクターID（英数字・-・_）" }), idInput]),
    el("div", { className: "field" }, [el("label", { textContent: "表示名" }), nameInput]),
    el("div", { className: "character-add-actions" }, [createButton, cancelButton]),
    progress,
    createStatus,
  );

  // --- キャラクターを自分で追加する ---
  // 同梱キャラは実行ファイルへ焼き込まれているため増減できない。追加分はユーザー領域
  // （app_data_dir/characters）へ入れる。導入は検証を通ってから確定するので、
  // 壊れたパックが一覧へ出ることはない。
  const addSec = section("キャラクターを追加");
  const addHint = el("p", {
    className: "hint",
    textContent: "Live2Dは配布元から自分で取得したzip／フォルダを、そのまま追加できます。PicoAgentがサンプルモデルを代理取得・再配布することはありません。",
  });
  const addStatus = el("p", { className: "hint" });
  const live2dIdInput = el("input", {
    type: "text",
    value: `live2d-${Date.now().toString(36)}`,
    placeholder: "例: my-live2d",
    maxLength: 64,
  });
  const live2dNameInput = el("input", { type: "text", value: "マイLive2D", maxLength: 80 });
  const termsInput = el("input", { type: "checkbox" });

  const officialLinks = el("div", { className: "setup-guide-links" }, [
    ["Live2D公式サンプル", "https://www.live2d.com/learn/sample/"],
    ["サンプル利用条件", "https://www.live2d.com/learn/sample/model-terms/"],
  ].map(([label, href]) => {
    const link = el("a", { href, textContent: label });
    link.setAttribute("target", "_blank");
    link.setAttribute("rel", "noreferrer");
    return link;
  }));

  async function installRawLive2d(folder) {
    addStatus.textContent = "選択中…";
    try {
      if (!termsInput.checked) throw new Error("配布元の利用条件とモデルの利用権を確認してください。");
      const id = live2dIdInput.value.trim();
      const name = live2dNameInput.value.trim();
      if (!id || !name) throw new Error("キャラクターIDと表示名を入力してください。");
      const picked = await window.picoagent.settings.pickCharacterPack(folder);
      if (!picked?.path) {
        addStatus.textContent = "選択をキャンセルしました。";
        return;
      }
      addStatus.textContent = "Live2Dモデルを確認し、キャラクターパックを作成しています…";
      const result = await window.picoagent.settings.installLive2dCharacter(picked.path, id, name);
      addStatus.textContent = `${result.name}（${result.id}）を追加しました。「設定」を開き直すと一覧に出ます。`;
    } catch (error) {
      addStatus.textContent = `追加できませんでした: ${error?.message || error}`;
    }
  }

  const rawLive2dButtons = el("div", { className: "character-add-actions" }, [
    Object.assign(el("button", { className: "primary", type: "button", textContent: "Live2Dのzipを追加" }), {
      onclick: () => installRawLive2d(false),
    }),
    Object.assign(el("button", { className: "secondary", type: "button", textContent: "Live2Dフォルダを追加" }), {
      onclick: () => installRawLive2d(true),
    }),
  ]);

  async function installCharacterPack(folder) {
    addStatus.textContent = "選択中…";
    try {
      const picked = await window.picoagent.settings.pickCharacterPack(folder);
      if (!picked?.path) {
        addStatus.textContent = "選択をキャンセルしました。";
        return;
      }
      addStatus.textContent = "確認しています…";
      const result = await window.picoagent.settings.installCharacter(picked.path);
      addStatus.textContent = `${result.name}（${result.id}）を追加しました。「設定」を開き直すと一覧に出ます。`;
    } catch (error) {
      // 検証で弾いた理由をそのまま見せる（何を直せばよいか分かるようにする）。
      addStatus.textContent = `追加できませんでした: ${error?.message || error}`;
    }
  }
  const addButtons = el("div", { className: "character-add-actions" }, [
    Object.assign(el("button", { className: "secondary", type: "button", textContent: "完成済みパックのzip" }), {
      onclick: () => installCharacterPack(false),
    }),
    Object.assign(el("button", { className: "secondary", type: "button", textContent: "完成済みパックのフォルダ" }), {
      onclick: () => installCharacterPack(true),
    }),
  ]);
  addSec.append(
    addHint,
    officialLinks,
    el("div", { className: "field" }, [el("label", { textContent: "キャラクターID（英数字・-・_）" }), live2dIdInput]),
    el("div", { className: "field" }, [el("label", { textContent: "表示名" }), live2dNameInput]),
    el("label", { className: "check-row" }, [
      termsInput,
      el("span", { textContent: "配布元の利用条件を確認し、このモデルを使う権利があります" }),
    ]),
    rawLive2dButtons,
    el("p", { className: "hint", textContent: ".model3.jsonが1個だけ含まれるデータを選んでください。編集用ファイルは取り込まず、同じフォルダの組み込み用データだけを保存します。" }),
    el("div", { className: "subhead", textContent: "PicoAgentキャラクターパックを追加" }),
    el("p", { className: "hint", textContent: "character.jsonを含む完成済みパックを持っている場合はこちらを使います。" }),
    addButtons,
    addStatus,
  );
  if (userCharacterRoot) {
    addSec.append(el("p", { className: "hint", textContent: `置き場: ${userCharacterRoot}` }));
  }
  panels.character.append(charSec);
  characterAddGroup = advancedGroup(
    "キャラクターを追加・作成",
    "新しいキャラクターパックを導入したい時だけ開いてください。",
    [addSec, tha3Sec],
  );
  characterAddGroup.id = "character-add-group";
  if (requestedSettingsTab === "character-add") characterAddGroup.open = true;
  panels.character.append(characterAddGroup);

  // --- 叩く AI コネクタごとの設定（選択中のプロバイダの設定だけ表示する） ---
  const provSec = section("AIサービスごとの設定");
  let activeProviderBlock = null;
  function renderProviderConfig(id) {
    if (activeProviderBlock) {
      unregisterInputsIn(activeProviderBlock);
      activeProviderBlock.remove();
      activeProviderBlock = null;
    }
    const provider = data.providers.find(p => p.id === id);
    if (!provider?.configSchema?.length) {
      activeProviderBlock = el("p", { className: "hint", textContent: "このプロバイダに変更できる設定はありません。" });
      provSec.append(activeProviderBlock);
      return;
    }
    const block = markSearchable(
      el("div", { className: "prov-block" }),
      provider.id,
      provider.label,
      provider.configSchema.map(field => `${field.key} ${field.label}`).join(" ")
    );
    block.append(el("div", { className: "subhead", textContent: provider.label }));
    const guide = setupGuideNode(provider.id, provider.configSchema);
    if (guide) block.append(guide);
    for (const f of provider.configSchema) block.append(field(f.key, f.label, f.type, v[f.key], f.placeholder));
    activeProviderBlock = block;
    provSec.append(block);
  }
  renderProviderConfig(data.active.providerId);
  panels.providers.append(el("p", {
    className: "settings-intro",
    textContent: "AIの種類と外部連携をまとめています。普段は選択中のAIだけ確認すれば十分です。",
  }));
  panels.providers.append(provSec);
  panels.providers.append(buildDownloadedModelsSection());
  panels.providers.append(apiSec);
  panels.providers.append(mcpSec);

  // スキルごとの設定は上のスキル一覧（クリックで展開）に統合済み。
  // --- 許可（ファイル/実行）: 全スキル共通のファイル/ディレクトリ許可ルール ---
  const permSec = section("許可（ファイル/実行）");
  permSec.append(el("p", { className: "hint", textContent: "ファイル操作やコマンド実行で触ってよい場所を決めます。許可したフォルダの中だけを読み書き・操作・実行できます。C:\\Windows と Program Files は設定に関わらず常に拒否します。" }));
  permSec.append(field("PICOAGENT_ALLOWED_ROOT", "既定で許可するフォルダ", "text", v.PICOAGENT_ALLOWED_ROOT ?? "", "例: C:\\Users\\you\\work"));
  permSec.append(el("p", { className: "field-note", textContent: "ここは相対パスの基準にもなります。空ならホームフォルダを使います。" }));
  permSec.append(field("PICOAGENT_ALLOWED_PATHS", "ほかにも許可するフォルダ（改行または ; 区切り）", "textarea", v.PICOAGENT_ALLOWED_PATHS ?? "", ""));
  permSec.append(field("PICOAGENT_DENIED_PATHS", "拒否パス（改行または ; 区切り。この配下は常に拒否）", "textarea", v.PICOAGENT_DENIED_PATHS ?? "", ""));
  permSec.append(el("p", { className: "hint", textContent: "コマンド実行（run_command）と自力実行（code_runner）の有効化は「スキル」タブの各スキルカードで切り替えます。既定はどちらも無効です。" }));
  panels.permissions.append(permSec);

  panels.diagnostics.append(buildProductInfoSection(), buildDiagnosticsSection());

  // プロバイダ変更でモデル候補を更新し、選択中プロバイダの設定だけ表示する。
  providerSelect.addEventListener("change", () => {
    v.PICOAGENT_PROVIDER = providerSelect.value;
    loadModels(providerSelect.value, true);
    renderProviderConfig(providerSelect.value);
    updateModelFieldVisibility();
    updateDirtyState();
    applySettingsSearch();
  });
  inputs.get("PICOAGENT_LOCAL_MODEL_PRESET")?.addEventListener("change", () => {
    if (providerSelect.value !== "local" || !modelInput) return;
    const preset = inputs.get("PICOAGENT_LOCAL_MODEL_PRESET").value;
    modelInput.value = preset === "auto" ? providerDefaultModel("local") : preset;
    updateDirtyState();
  });
  await loadModels(providerSelect.value);
  applySettingsSearch();
  renderingSettings = false;
  updateDirtyState();
  if (requestedSettingsTab === "character-add") activateSettingsTab("character-add");
}

function providerDefaultModel(providerId) {
  const provider = data.providers.find(p => p.id === providerId);
  if (!provider) return "";
  if (providerId === "ollama") return inputs.get("OLLAMA_MODEL")?.value || provider.defaultModel || "";
  if (providerId === "openai") return inputs.get("OPENAI_MODEL")?.value || provider.defaultModel || "";
  if (providerId === "anthropic") return inputs.get("PICOAGENT_ANTHROPIC_MODEL")?.value || provider.defaultModel || "";
  if (providerId === "deepseek") return inputs.get("PICOAGENT_DEEPSEEK_MODEL")?.value || provider.defaultModel || "";
  if (providerId === "gemini") return inputs.get("PICOAGENT_GEMINI_MODEL")?.value || provider.defaultModel || "";
  if (providerId === "xai") return inputs.get("PICOAGENT_XAI_MODEL")?.value || provider.defaultModel || "";
  if (providerId === "mistral") return inputs.get("PICOAGENT_MISTRAL_MODEL")?.value || provider.defaultModel || "";
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
  updateDirtyState();
}

function collect() {
  const patch = cloneSettings(savedSnapshot || data.values || {});
  for (const [key, input] of inputs) {
    patch[key] = input.type === "checkbox" ? input.checked : input.value;
  }
  if (patch.PICOAGENT_PROVIDER === "ollama" && patch.PICOAGENT_MODEL) patch.OLLAMA_MODEL = patch.PICOAGENT_MODEL;
  if (patch.PICOAGENT_PROVIDER === "openai") patch.PICOAGENT_MODEL = patch.OPENAI_MODEL || "";
  if (patch.PICOAGENT_PROVIDER === "anthropic") patch.PICOAGENT_MODEL = patch.PICOAGENT_ANTHROPIC_MODEL || "";
  if (patch.PICOAGENT_PROVIDER === "deepseek") patch.PICOAGENT_MODEL = patch.PICOAGENT_DEEPSEEK_MODEL || "";
  if (patch.PICOAGENT_PROVIDER === "gemini") patch.PICOAGENT_MODEL = patch.PICOAGENT_GEMINI_MODEL || "";
  if (patch.PICOAGENT_PROVIDER === "xai") patch.PICOAGENT_MODEL = patch.PICOAGENT_XAI_MODEL || "";
  if (patch.PICOAGENT_PROVIDER === "mistral") patch.PICOAGENT_MODEL = patch.PICOAGENT_MISTRAL_MODEL || "";
  if (patch.PICOAGENT_PROVIDER === "local") {
    const preset = patch.PICOAGENT_LOCAL_MODEL_PRESET || "auto";
    patch.PICOAGENT_MODEL = preset === "auto" ? "" : preset;
  }
  if (characterSelect && characterPersona) characterOverrides[characterSelect.value] = characterPersona.value;
  if (characterSelect && characterSpeaker) saveCurrentCharacterVoice(characterSelect.value);
  if (characterSelect && characterLive2dSdkSelect) saveCurrentCharacterLive2dSdk(characterSelect.value);
  patch.PICOAGENT_CHARACTER_OVERRIDES = characterOverrides;
  patch.PICOAGENT_TTS_CHARACTER_OVERRIDES = characterVoiceOverrides;
  patch.PICOAGENT_LIVE2D_SDK_OVERRIDES = characterLive2dSdkOverrides;
  patch.PICOAGENT_CHARACTER_FRAMING = characterFraming;
  patch.PICOAGENT_STREAM_PROFILES = characterStreamProfiles;
  if (alarmEditorState) patch.PICOAGENT_ALARMS = alarmEditorState.getJson();
  if (scheduleEditorState) patch.PICOAGENT_SCHEDULED_TASKS = scheduleEditorState.getJson();
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
      el("p", { textContent: "選んだ設定（使うAI・モデル・音声・キャラクターなど）は、PicoAgent を再起動して起動画面でまとめて反映します。今すぐ再起動しますか？" }),
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
    const patch = collect();
    const res = await window.picoagent.settings.save(patch);
    savedSnapshot = cloneSettings(patch);
    data.values = cloneSettings(patch);
    updateDirtyState();
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
      statusEl.textContent = res.up ? "保存しました（接続OK）" : "保存しました（AIに未接続）";
    }
    // ホットキーは保存と同時に張り直す。登録できなかった場合は黙って無効にせず伝える。
    if (res.hotkeyWarning) {
      logApp("error", "ホットキーの登録に失敗しました", res.hotkeyWarning);
      statusEl.textContent = `${statusEl.textContent} / ${res.hotkeyWarning}`;
    }
  } catch (error) {
    logApp("error", "設定保存に失敗しました", errorDetail(error));
    statusEl.textContent = `保存に失敗: ${error.message}`;
  } finally {
    saveBtn.disabled = false;
  }
});

resetChangesBtn?.addEventListener("click", async () => {
  if (!savedSnapshot || resetChangesBtn.disabled) return;
  renderingSettings = true;
  data.values = cloneSettings(savedSnapshot);
  statusEl.textContent = "未保存の変更を戻しました";
  try {
    await render();
    resetChangesBtn.disabled = true;
    saveBtn.classList.remove("dirty");
  } catch (error) {
    logApp("error", "設定の復元表示に失敗しました", errorDetail(error));
    statusEl.textContent = `設定の復元表示に失敗: ${error.message}`;
  } finally {
    renderingSettings = false;
    updateDirtyState();
  }
});

form.addEventListener("input", updateDirtyState);
form.addEventListener("change", updateDirtyState);
form.addEventListener("click", event => {
  if (event.target instanceof Element && event.target.closest(".alarm-add,.alarm-del")) {
    setTimeout(updateDirtyState, 0);
  }
});

settingsSearch?.addEventListener("input", applySettingsSearch);
clearSettingsSearch?.addEventListener("click", () => {
  settingsSearch.value = "";
  clearSettingsSearch.hidden = true;
  applySettingsSearch();
  settingsSearch.focus();
});
for (const shortcut of document.querySelectorAll("[data-settings-query]")) {
  shortcut.addEventListener("click", () => {
    settingsSearch.value = shortcut.dataset.settingsQuery || "";
    if (clearSettingsSearch) clearSettingsSearch.hidden = false;
    applySettingsSearch();
    settingsSearch.focus();
  });
}
settingsSearch?.addEventListener("input", () => {
  if (clearSettingsSearch) clearSettingsSearch.hidden = !settingsSearch.value;
});
document.addEventListener("keydown", event => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
    event.preventDefault();
    if (!saveBtn.disabled && !resetChangesBtn.disabled) saveBtn.click();
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
    event.preventDefault();
    settingsSearch?.focus();
    settingsSearch?.select();
  }
});

(async () => {
  data = await window.picoagent.settings.load();
  try {
    await render();
    savedSnapshot = cloneSettings(collect());
    updateDirtyState();
  }
  catch (error) {
    renderingSettings = false;
    logApp("error", "設定画面の読み込みに失敗しました", errorDetail(error));
    statusEl.textContent = `設定画面の読み込みに失敗: ${error.message}`;
  }
})().catch(error => {
  renderingSettings = false;
  logApp("error", "設定初期化に失敗しました", errorDetail(error));
  statusEl.textContent = `設定初期化に失敗: ${error?.message || error}`;
});
