// settings.js - 設定ウィンドウのロジック。
// main から渡されるグローバル設定 + 各コネクタの configSchema を動的に描画し、
// 保存時に { 設定キー: 値 } のフラットなパッチを main へ送る。

const form = document.getElementById("form");
const statusEl = document.getElementById("status");
const saveBtn = document.getElementById("save");
const resetChangesBtn = document.getElementById("resetChanges");
const modelOptions = document.getElementById("modelOptions");
const settingsSearch = document.getElementById("settingsSearch");

let data; // settings:load の結果
const inputs = new Map(); // key -> input 要素
const inputWrappers = new Map(); // key -> field/card 要素
let savedSnapshot = null; // 最後に読み込んだ、または保存できた設定値
let renderingSettings = false;
let characterOverrides = {};
let characterVoiceOverrides = {};
let characterDefaultVoices = {};
let characterSelect;
let characterPersona;
let characterSpeaker;
let characterCatalog;
const characterManifestCache = new Map();
let modelInput;
let alarmEditorState = null; // 目覚ましエディタ（保存時に PICOAGENT_ALARMS を組み立てる）
let scheduleEditorState = null; // スケジュールタスクエディタ（保存時に PICOAGENT_SCHEDULED_TASKS を組み立てる）
const characterManifestCacheKey = "2026-07-03-live2d-jp-voice-v1";
const noSearchResultMessage = "検索に一致する設定はありません";
const SETTINGS_TAB_IDS = new Set(["basic", "character", "voice", "providers", "tools", "permissions", "skillguide", "diagnostics"]);
const requestedSettingsTab = new URLSearchParams(window.location.search).get("tab");
let activeSettingsTab = SETTINGS_TAB_IDS.has(requestedSettingsTab) ? requestedSettingsTab : "basic";

function activateSettingsTab(tabId) {
  if (!SETTINGS_TAB_IDS.has(tabId)) return false;
  activeSettingsTab = tabId;
  const button = document.getElementById(`tab-${tabId}`);
  if (button) button.click();
  return true;
}

function errorDetail(error) {
  if (!error) return "";
  if (error instanceof Error) return error.stack || error.message;
  if (typeof error === "string") return error;
  try { return JSON.stringify(error); }
  catch { return String(error); }
}

function logApp(level, message, detail) {
  try {
    window.picoagent?.log?.(level, message, detail == null ? "" : String(detail)).catch(error => {
      // フォールバック許可: ログ保存自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
      console.warn("設定画面ログ保存に失敗しました", error);
    });
  } catch (error) {
    // フォールバック許可: ログ保存自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
    console.warn("設定画面ログ保存に失敗しました", error);
  }
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
  const dirty = stableStringify(current) !== stableStringify(savedSnapshot);
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
  form.querySelector(".alarm-editor")?.closest(".card-config")?.classList.toggle(
    "changed",
    stableStringify(current.PICOAGENT_ALARMS) !== stableStringify(savedSnapshot.PICOAGENT_ALARMS)
  );
  form.querySelector(".schedule-editor")?.closest(".card-config")?.classList.toggle(
    "changed",
    stableStringify(current.PICOAGENT_SCHEDULED_TASKS) !== stableStringify(savedSnapshot.PICOAGENT_SCHEDULED_TASKS)
  );
  resetChangesBtn.disabled = !dirty;
  saveBtn.classList.toggle("dirty", dirty);
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
    item.hidden = query ? !nodeSearchText(item).includes(query) : false;
  }
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

const SCHEDULE_WEEKDAYS = ALARM_WEEKDAYS;
function buildScheduleEditor(currentJson) {
  let tasks = [];
  try {
    const parsed = JSON.parse(currentJson || "[]");
    if (Array.isArray(parsed)) tasks = parsed;
  } catch { tasks = []; }

  const wrap = el("div", { className: "schedule-editor" });
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
  addBtn.addEventListener("click", () => addRow({ kind: "tool", tool_name: "weather", tool_args: {} }));
  const hint = el("div", { className: "hint", textContent: "cron を空にすると時刻と曜日で実行します。スキル実行はコマンド許可なしで使えます。コマンド実行は許可ルートと allowlist の制限を受けます。" });
  wrap.append(addBtn, hint);

  scheduleEditorState = {
    getJson() {
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
  if (["read_file", "workspace_files", "project_workspace", "workflow", "run_command", "http_api"].includes(name)) {
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
  project_workspace: ["このフォルダを仕事用ワークスペースとして登録して"],
  workflow: ["この手順を週報作成として覚えて"],
  run_command: ["git statusを実行して"],
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
    "workspace_files", "google_calendar", "google_docs", "google_drive", "google_sheets",
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

async function loadCharacterManifestForSettings(character) {
  if (characterManifestCache.has(character.id)) return characterManifestCache.get(character.id);
  const response = await fetch(`assets/characters/${character.manifest}?v=${characterManifestCacheKey}`, { cache: "no-store" });
  if (!response.ok) throw new Error(`キャラクター定義: HTTP ${response.status}`);
  const manifest = await response.json();
  characterManifestCache.set(character.id, manifest);
  return manifest;
}

async function characterIconUrl(character) {
  try {
    const manifest = await loadCharacterManifestForSettings(character);
    const root = `assets/characters/${character.manifest.replace(/\/character\.json$/, "")}`;
    const idle = manifest.animations?.idle;
    const imagePath = idle?.path ? `${idle.path}/00.png` : (manifest.boot?.sleeping || "animations/idle/00.png");
    const version = manifest.assetVersion ? `?v=${encodeURIComponent(manifest.assetVersion)}` : "";
    return `${root}/${imagePath}${version}`;
  } catch (error) {
    logApp("warn", "キャラクターアイコンの読込に失敗しました", `${character.id}: ${errorDetail(error)}`);
    return "";
  }
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
    { id: "basic", label: "基本" },
    { id: "character", label: "キャラクター" },
    { id: "voice", label: "音声" },
    { id: "providers", label: "叩くAI" },
    { id: "tools", label: "スキル設定" },
    { id: "permissions", label: "許可" },
    { id: "skillguide", label: "スキル活用ガイド" },
    { id: "diagnostics", label: "診断" },
  ]);

  // --- グローバル設定 ---
  const global = section("基本");
  const providerSelect = el("select");
  for (const p of data.providers) {
    providerSelect.append(el("option", { value: p.id, textContent: p.label, selected: p.id === data.active.providerId }));
  }
  global.append(registerInput(
    "PICOAGENT_PROVIDER",
    providerSelect,
    markSearchable(el("div", { className: "field" }, [el("label", { textContent: "叩く AI（プロバイダ）" }), providerSelect]), "PICOAGENT_PROVIDER", "叩く AI", "プロバイダ")
  ));
  const modelField = field("PICOAGENT_MODEL", "モデル", "text", v.PICOAGENT_MODEL ?? data.active.model, data.active.model);
  global.append(modelField);
  modelInput = inputs.get("PICOAGENT_MODEL");
  // Ollama だけは実行中のローカル一覧を基本タブのモデル欄に出す。
  // 他プロバイダは「叩くAI」タブで種類とサイズを選ぶため、ここでは隠す。
  function updateModelFieldVisibility() { modelField.hidden = providerSelect.value !== "ollama"; }
  updateModelFieldVisibility();
  global.append(field("PICOAGENT_SYSTEM_PROMPT", "システムプロンプト", "textarea", v.PICOAGENT_SYSTEM_PROMPT ?? "", data.defaults.systemPrompt));
  global.append(field("PICOAGENT_BUBBLE_FONT_PX", "フキダシの文字サイズ(px)", "text", v.PICOAGENT_BUBBLE_FONT_PX ?? "", "13"));
  global.append(selectField("PICOAGENT_THEME", "表示テーマ", v.PICOAGENT_THEME || "system", [
    { value: "system", label: "システム設定に追従" },
    { value: "light", label: "通常" },
    { value: "dark", label: "ダーク" },
  ]));
  global.append(field("PICOAGENT_ALWAYS_ON_TOP", "常に手前に表示", "checkbox", v.PICOAGENT_ALWAYS_ON_TOP !== false));
  global.append(field("PICOAGENT_MEMORY_ENABLED", "会話から好みや継続指示を暗黙的に覚える", "checkbox", v.PICOAGENT_MEMORY_ENABLED !== false));
  global.append(field("PICOAGENT_MEMORY_MAX_ITEMS", "長期メモリの最大件数", "text", v.PICOAGENT_MEMORY_MAX_ITEMS ?? "", "80"));
  global.append(field("PICOAGENT_SUPER_AGENT", "スーパーエージェント（計画→実行・自己評価・サブエージェント分担を強化）", "checkbox", v.PICOAGENT_SUPER_AGENT === true));
  global.append(field("PICOAGENT_SUPER_AGENT_MAX_STEPS", "スーパーエージェントの最大ループ数", "text", v.PICOAGENT_SUPER_AGENT_MAX_STEPS ?? "", "12"));
  const quickActionsField = field("PICOAGENT_QUICK_ACTIONS", "クイックアクション（1行1件「ラベル|送る文章」。空なら既定の8件）", "textarea", v.PICOAGENT_QUICK_ACTIONS ?? "", "👀 画面を見る|今の画面を見て、状況を短く教えて");
  global.append(quickActionsField);
  global.append(field("PICOAGENT_NAP_MINUTES", "無操作で居眠りするまでの分数（0で居眠りしない）", "text", v.PICOAGENT_NAP_MINUTES ?? "", "15"));
  global.append(field("PICOAGENT_UPDATE_CHECK_ENABLED", "起動時に新しいバージョンを確認する", "checkbox", v.PICOAGENT_UPDATE_CHECK_ENABLED !== false));
  // ローカルAPI（Ollama風）: 外部ツールから 127.0.0.1 の HTTP でエージェントを呼べる。
  const apiSec = section("ローカルAPI（Ollama風）");
  apiSec.append(field("PICOAGENT_API_ENABLED", "ローカルAPIを有効にする（127.0.0.1 のみ。変更は再起動後に反映）", "checkbox", v.PICOAGENT_API_ENABLED === true));
  apiSec.append(field("PICOAGENT_API_PORT", "APIポート", "text", v.PICOAGENT_API_PORT ?? "", "11500"));
  apiSec.append(field("PICOAGENT_API_KEY", "APIキー（空なら認証なし。Authorization: Bearer で送る）", "password", v.PICOAGENT_API_KEY ?? ""));
  const voiceSec = section("読み上げ");
  voiceSec.append(field("PICOAGENT_TTS_ENABLED", "Aivis で回答を読み上げる", "checkbox", v.PICOAGENT_TTS_ENABLED === true));
  voiceSec.append(field("PICOAGENT_TTS_VOLUME", "キャラボイスの音量（0〜100）", "number", v.PICOAGENT_TTS_VOLUME ?? "100", "100"));
  panels.basic.append(global);
  panels.basic.append(apiSec);
  panels.voice.append(voiceSec);
  applySettingsTheme(v.PICOAGENT_THEME || "system");
  inputs.get("PICOAGENT_THEME")?.addEventListener("change", event => {
    applySettingsTheme(event.target.value);
  });

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
  characterSec.append(registerInput(
    "PICOAGENT_CHARACTER",
    characterSelect,
    markSearchable(el("div", { className: "field" }, [el("label", { textContent: "キャラクター" }), characterSelect]), "PICOAGENT_CHARACTER", "キャラクター")
  ));
  characterPersona = el("textarea", { placeholder: "性格・口調・一人称など" });
  characterSec.append(el("div", { className: "field" }, [el("label", { textContent: "このキャラの性格・口調" }), characterPersona]));
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

  // --- スキル設定（⚙ のあるスキルはクリックでその場に開く）。活用ガイドとは別タブに分ける ---
  const skillSec = section("スキル設定");
  if (!data.tools.length) {
    skillSec.append(el("p", { className: "hint", textContent: "登録されているスキルはありません。" }));
  } else {
    skillSec.append(el("p", { className: "hint", textContent: "カテゴリ別の一覧です。ジャンルで絞り込めます。⚙ のあるスキルをクリックすると設定を開けます（既定は折りたたみ）。頼み方の例は「スキル活用ガイド」タブを参照。" }));
    skillSec.append(buildGenreChips(() => skillGenreFilter, value => { skillGenreFilter = value; }));
    skillSec.append(buildSkillCategoryList(data.tools, v));
  }
  panels.tools.append(skillSec);

  // --- スキル活用ガイド（読み取り専用の一覧）。設定とは分離して別タブに置く ---
  const skillGuideSec = section("スキル活用ガイド");
  if (!data.tools.length) {
    skillGuideSec.append(el("p", { className: "hint", textContent: "登録されているスキルはありません。" }));
  } else {
    skillGuideSec.append(el("p", { className: "hint", textContent: "各スキルの「できること」と頼み方の例の一覧です。ジャンルで絞り込めます。設定（APIキー等）は「スキル設定」タブで行います。" }));
    skillGuideSec.append(buildGenreChips(() => guideGenreFilter, value => { guideGenreFilter = value; }));
    skillGuideSec.append(buildSkillGuide(data.tools));
  }
  panels.skillguide.append(skillGuideSec);

  // --- 導入済みキャラクターの一覧（読み取り専用） ---
  const charSec = section("導入済みキャラクター");
  const charList = el("div", { className: "card-list" });
  for (const character of characterCatalog.characters) {
    const name = character.id === selectedCharacter ? `${character.name}（使用中）` : character.name;
    const fallback = el("div", { className: "character-icon-fallback", textContent: String(character.name || character.id || "?").slice(0, 1) });
    const icon = el("img", { className: "character-icon", alt: `${character.name}のアイコン`, draggable: false });
    icon.hidden = true;
    characterIconUrl(character).then(url => {
      if (!url) return;
      icon.src = url;
      icon.hidden = false;
      fallback.hidden = true;
    });
    const card = markSearchable(el("div", { className: "card-item character-card" }, [
      el("div", { className: "character-icon-wrap" }, [fallback, icon]),
      el("div", { className: "character-card-text" }, [
        el("div", { className: "card-name", textContent: name }),
        el("div", { className: "card-desc", textContent: character.description || character.id }),
      ]),
    ]), character.id, character.name, character.description);
    charList.append(card);
  }
  charSec.append(charList);
  panels.character.append(charSec);

  // --- 叩く AI コネクタごとの設定（選択中のプロバイダの設定だけ表示する） ---
  const provSec = section("叩く AI コネクタの設定");
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
  panels.providers.append(provSec);
  panels.providers.append(buildDownloadedModelsSection());

  // スキルごとの設定は上のスキル一覧（クリックで展開）に統合済み。
  // --- 許可（ファイル/実行）: 全スキル共通のファイル/ディレクトリ許可ルール ---
  const permSec = section("許可（ファイル/実行）");
  permSec.append(el("p", { className: "hint", textContent: "ファイル操作やコマンド実行で触ってよい場所を決めます。許可したフォルダの中だけを読み書き・操作・実行できます。C:\\Windows と Program Files は設定に関わらず常に拒否します。" }));
  permSec.append(field("PICOAGENT_ALLOWED_ROOT", "既定で許可するフォルダ", "text", v.PICOAGENT_ALLOWED_ROOT ?? "", "例: C:\\Users\\you\\work"));
  permSec.append(el("p", { className: "field-note", textContent: "ここは相対パスの基準にもなります。空ならホームフォルダを使います。" }));
  permSec.append(field("PICOAGENT_ALLOWED_PATHS", "ほかにも許可するフォルダ（改行または ; 区切り）", "textarea", v.PICOAGENT_ALLOWED_PATHS ?? "", ""));
  permSec.append(field("PICOAGENT_DENIED_PATHS", "拒否パス（改行または ; 区切り。この配下は常に拒否）", "textarea", v.PICOAGENT_DENIED_PATHS ?? "", ""));
  permSec.append(el("p", { className: "hint", textContent: "コマンド実行（run_command）と自力実行（code_runner）の有効化は「スキル設定」タブの各スキルカードで切り替えます。既定はどちらも無効です。" }));
  panels.permissions.append(permSec);

  panels.diagnostics.append(buildDiagnosticsSection());

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
  patch.PICOAGENT_CHARACTER_OVERRIDES = characterOverrides;
  patch.PICOAGENT_TTS_CHARACTER_OVERRIDES = characterVoiceOverrides;
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
      statusEl.textContent = res.up ? "保存しました（接続OK）" : "保存しました（叩く AI に未接続）";
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
