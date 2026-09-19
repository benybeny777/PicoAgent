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
const { errorDetail, logApp, tail, twoStepAction } = window.PicoUI;

let data; // settings:load の結果
const inputs = new Map(); // key -> input 要素
const inputWrappers = new Map(); // key -> field/card 要素
let savedSnapshot = null; // 最後に読み込んだ、または保存できた設定値
let providerDraftValues = {}; // 非表示へ切り替えたAI設定の未保存入力。
let renderingSettings = false;
let characterOverrides = {};
let characterVoiceOverrides = {};
let characterLive2dSdkOverrides = {};
let characterFraming = {};
let characterFramingEditor = null;
let characterStreamProfiles = {};
let characterStreamProfileEditor = null;
let characterDefaultVoices = {};
let characterDefaultPersonas = {};
let characterSelect;
let userCharacterRoot = ""; // 追加キャラクターの置き場（案内表示用）
let modalOpen = false; // 確認ダイアログ表示中は全体ショートカットを止める
let settingsMutationBusy = false;
const settingsListeners = new Map();
let characterPersona;
let characterSpeaker;
let characterLive2dSdkSelect;
let characterLive2dSdkField;
let characterLive2dSdkHint;
let characterCatalog;
const characterManifestCache = new Map();
let modelInput;
let cliModelSelect;
let modelLoadGeneration = 0;
let alarmEditorState = null; // 目覚ましエディタ（保存時に PICOAGENT_ALARMS を組み立てる）
let scheduleEditorState = null; // スケジュールタスクエディタ（保存時に PICOAGENT_SCHEDULED_TASKS を組み立てる）
let dirtyStateScheduler = null; // 高頻度入力の変更印更新を1フレーム1回へまとめる。
const characterManifestCacheKey = "2026-07-03-live2d-jp-voice-v1";
const noSearchResultMessage = "検索に一致する設定はありません";
const SETTINGS_TAB_IDS = new Set(["basic", "display", "character-maker", "character-register", "character", "voice", "components", "providers", "tools", "permissions", "skillguide", "diagnostics"]);
const PRESERVE_UNREADABLE_SECRET = "__PRESERVE_UNREADABLE_SECRET__";
const SETTINGS_RUNTIME_KEYS = new Set([
  "PICOAGENT_WINDOW_LAYOUTS",
  "PICOAGENT_SCHEDULED_TASK_STATE",
]);
const LAYOUT_CHOICES = [
  { id: "pet", label: "ペット", size: "900×600", description: "キャラと会話をゆったり表示。普段使い向け" },
  { id: "mascot", label: "ミニ", size: "360×560", description: "キャラ中心。必要な時だけ会話する" },
  { id: "chat", label: "チャット", size: "520×760", description: "LINE風の吹き出しで会話する" },
  { id: "bar", label: "バー", size: "900×320", description: "呼び出してすばやく質問する" },
  { id: "workspace", label: "作業", size: "1400×850", description: "履歴・回答・実行状況を並べて見る" },
  { id: "stream", label: "配信", size: "1920×1080", description: "フルHD背景でキャラと字幕を映す" },
  { id: "widget", label: "ウィジェット", size: "440×600", description: "時計・通知・入力をまとめて置く" },
];
const requestedSettingsTab = new URLSearchParams(window.location.search).get("tab");
let activeSettingsTab = requestedSettingsTab === "character-add"
  ? "character-register"
  : (SETTINGS_TAB_IDS.has(requestedSettingsTab) ? requestedSettingsTab : "basic");

function activateSettingsTab(tabId) {
  if (tabId === "character-add") tabId = "character-register";
  if (!SETTINGS_TAB_IDS.has(tabId)) return false;
  activeSettingsTab = tabId;
  const button = document.getElementById(`tab-${tabId}`);
  if (button) {
    button.click();
    form.scrollTop = 0;
    // タブ一覧は高さ上限の中でスクロールする。下側のタブを開いても選択中が隠れないようにする。
    button.scrollIntoView({ block: "nearest" });
  }
  return true;
}

window.addEventListener("error", event => {
  logApp("error", "設定画面の未捕捉エラー", `${event.message || ""} ${event.filename || ""}:${event.lineno || ""}:${event.colno || ""}\n${errorDetail(event.error)}`);
});
window.addEventListener("unhandledrejection", event => {
  logApp("error", "設定画面の未処理 Promise rejection", errorDetail(event.reason));
});

async function replaceSettingsListener(key, channel, callback) {
  const previous = settingsListeners.get(key);
  previous?.release?.();
  const slot = { release: null };
  settingsListeners.set(key, slot);
  const release = await window.picoagent.listen(channel, callback);
  if (settingsListeners.get(key) !== slot) {
    release();
    return;
  }
  slot.release = release;
}

replaceSettingsListener("navigate", "settings:navigate", tabId => {
  activateSettingsTab(String(tabId || ""));
}).catch(error => {
  logApp("warn", "設定タブ遷移イベントの購読に失敗しました", errorDetail(error));
});

window.addEventListener("beforeunload", () => {
  dirtyStateScheduler?.cancel();
  for (const slot of settingsListeners.values()) slot.release?.();
  settingsListeners.clear();
});

function el(tag, props = {}, children = []) {
  const node = Object.assign(document.createElement(tag), props);
  for (const child of children) {
    if (child != null && child !== false) node.append(child);
  }
  return node;
}

const { buildAlarmEditor, buildScheduleEditor } = window.PicoSettingsSchedules.create({
  el,
  setAlarmEditorState: state => { alarmEditorState = state; },
  setScheduleEditorState: state => { scheduleEditorState = state; },
});

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
  const label = wrapper.querySelector(":scope > label");
  // 複合エディタの保存専用hiddenには、見出しラベルを結び付けない。
  if (label && input.type !== "hidden" && !label.htmlFor) {
    if (!input.id) input.id = `setting-${key}`;
    label.htmlFor = input.id;
  }
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

function rememberProviderDraft(node) {
  for (const wrapper of node.querySelectorAll("[data-setting-key]")) {
    const key = wrapper.dataset.settingKey;
    const input = inputs.get(key);
    if (input) providerDraftValues[key] = input.type === "checkbox" ? input.checked : input.value;
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

function comparisonValue(value) {
  return value === PRESERVE_UNREADABLE_SECRET ? "" : value;
}

function settingsChangeRequest(current, saved, revisions) {
  const patch = {};
  for (const key of new Set([...Object.keys(saved || {}), ...Object.keys(current || {})])) {
    if (SETTINGS_RUNTIME_KEYS.has(key)) continue;
    if (stableStringify(comparisonValue(current[key])) !== stableStringify(comparisonValue(saved[key]))) {
      patch[key] = current[key];
    }
  }
  const baseRevisions = {};
  for (const key of Object.keys(patch)) {
    baseRevisions[key] = revisions?.[key] || "missing";
  }
  return { patch, baseRevisions };
}

function updateDirtyStateNow() {
  if (renderingSettings || !savedSnapshot || !resetChangesBtn) return;
  let current;
  try {
    current = collect();
  } catch (error) {
    // JSONなどの編集中の不正値も未保存の変更。保存時の検証へ渡し、修正・復元を妨げない。
    resetChangesBtn.disabled = settingsMutationBusy;
    saveBtn.disabled = settingsMutationBusy;
    saveBtn.classList.add("dirty");
    saveBtn.textContent = "変更を保存";
    if (dirtySummary) {
      dirtySummary.textContent = "未保存（入力を確認）";
      dirtySummary.classList.add("dirty");
    }
    return;
  }
  const changedKeys = new Set(Object.keys(settingsChangeRequest(
    current,
    savedSnapshot,
    data.valueRevisions,
  ).patch));
  const dirty = changedKeys.size > 0;
  for (const [key, wrapper] of inputWrappers) {
    const currentValue = current[key];
    const savedValue = savedSnapshot[key];
    wrapper.classList.toggle("changed", stableStringify(comparisonValue(currentValue)) !== stableStringify(comparisonValue(savedValue)));
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
  resetChangesBtn.disabled = settingsMutationBusy || !dirty;
  saveBtn.disabled = settingsMutationBusy;
  saveBtn.classList.toggle("dirty", dirty);
  saveBtn.textContent = dirty ? `変更を保存（${changedKeys.size}件）` : "変更を保存";
  if (dirtySummary) {
    dirtySummary.textContent = dirty ? `未保存 ${changedKeys.size}件` : "保存済み";
    dirtySummary.classList.toggle("dirty", dirty);
  }
}

// collect()と全設定の比較は高コストなので、入力・ドラッグが連続しても1フレーム1回だけ行う。
dirtyStateScheduler = window.PicoFrameScheduler.create(updateDirtyStateNow);

function updateDirtyState() {
  dirtyStateScheduler.schedule();
}

function setSettingsMutationBusy(busy) {
  settingsMutationBusy = busy;
  form.inert = busy;
  form.setAttribute("aria-busy", String(busy));
  // 保存中のボタン無効化は次フレームまで遅らせず、二重押しの窓を作らない。
  dirtyStateScheduler.flush();
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
    for (const [id, button] of buttons) {
      const active = id === getValue();
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    }
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
  if (!settingsSearch || !data) return;
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
      if (panel.dataset.userOpen == null) panel.dataset.userOpen = card?.classList.contains("open") ? "1" : "0";
      const open = nodeSearchText(panel).includes(query);
      panel.hidden = !open;
      if (card) {
        card.classList.toggle("open", open);
        if (open) card.hidden = false;
      }
    } else {
      if (panel.dataset.userOpen != null) {
        card?.classList.toggle("open", panel.dataset.userOpen === "1");
        delete panel.dataset.userOpen;
      }
      panel.hidden = !(card && card.classList.contains("open"));
    }
    card?.setAttribute("aria-expanded", String(!panel.hidden));
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
    const childMatch = [...sectionNode.querySelectorAll(".searchable")].some(item => {
      if (item.hidden) return false;
      for (let parent = item.parentElement; parent && parent !== sectionNode; parent = parent.parentElement) {
        // セクションと詳細グループの表示は検索結果から再計算する。ジャンルなどの非表示条件だけを先に尊重する。
        if (parent.hidden && !parent.matches(".section, .advanced-group")) return false;
      }
      return true;
    });
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
  for (const panel of [...form.querySelectorAll(".tab-panel")].reverse()) {
    if (!query) continue;
    // 説明文だけ残るタブを検索結果に数えず、隠れた親の中にある一致も除外する。
    const hasVisibleContent = [...panel.querySelectorAll(".searchable, .section")]
      .some(child => !child.closest("[hidden]"));
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
      label: "GPT-5.6",
      options: [
        { value: "gpt-5.6-sol", label: "Sol", hint: "OpenAI の最上位候補です。" },
        { value: "gpt-5.6-terra", label: "Terra", hint: "品質と費用のバランス候補です。" },
        { value: "gpt-5.6-luna", label: "Luna", hint: "低コスト・高頻度利用向けの既定候補です。" },
      ],
    },
  ],
  PICOAGENT_ANTHROPIC_MODEL: [
    {
      label: "Claude Fable",
      options: [
        { value: "claude-fable-5", label: "5", hint: "Claude の最上位一般提供候補です。" },
      ],
    },
    {
      label: "Claude Opus",
      options: [
        { value: "claude-opus-5", label: "5", hint: "Claude の高品質候補です。" },
      ],
    },
    {
      label: "Claude Sonnet",
      options: [
        { value: "claude-sonnet-5", label: "5", hint: "Claude の標準候補です。" },
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
      label: "Gemini 3",
      options: [
        { value: "gemini-3.6-flash", label: "3.6 Flash", hint: "最新の安定版Flashです。" },
        { value: "gemini-3.5-flash-lite", label: "3.5 Flash-Lite", hint: "低遅延・低コストの安定版です。" },
      ],
    },
  ],
  PICOAGENT_XAI_MODEL: [
    {
      label: "Grok",
      options: [
        { value: "grok-4.5", label: "4.5", hint: "xAI の最新安定版です。" },
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

  function updateHidden(nextValue, notify) {
    const changed = hidden.value !== nextValue;
    hidden.value = nextValue;
    if (notify && changed) hidden.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function rebuildSize(notify = false) {
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
      updateHidden(custom.value.trim(), notify);
      hint.textContent = customGroup.options[0].hint;
      return;
    }
    if (!group.options.some(option => option.value === hidden.value)) {
      updateHidden(group.options[0]?.value || "", notify);
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
  family.addEventListener("change", () => rebuildSize(true));
  size.addEventListener("change", () => {
    updateHidden(size.value, true);
    const group = allGroups[Number(family.value)] || allGroups[0];
    hint.textContent = group.options.find(option => option.value === hidden.value)?.hint || "";
  });
  custom.addEventListener("input", () => { updateHidden(custom.value.trim(), true); });
  wrapper.setValue = (nextValue) => {
    const changed = hidden.value !== nextValue;
    let groupIndex = allGroups.findIndex(group => group.options.some(option => option.value === nextValue));
    if (groupIndex < 0) groupIndex = allGroups.indexOf(customGroup);
    if (groupIndex < 0) return false;
    family.value = String(groupIndex);
    const group = allGroups[groupIndex];
    if (group === customGroup) custom.value = nextValue;
    updateHidden(nextValue, false);
    rebuildSize(false);
    if (group !== customGroup) size.value = nextValue;
    if (changed) hidden.dispatchEvent(new Event("change", { bubbles: true }));
    hint.textContent = group === customGroup
      ? customGroup.options[0].hint
      : (group.options.find(option => option.value === nextValue)?.hint || "");
    return true;
  };
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
  // 天気の地点は一覧から選べるようにする（自由入力も残す）。
  if (key === "PICOAGENT_WEATHER_DEFAULT_LOCATION") {
    input.setAttribute("list", "weatherLocationOptions");
  }
  return registerInput(key, input, markSearchable(el("div", { className: "field" }, [el("label", { textContent: label }), input]), key, label, placeholder));
}

function multiAiSettingsField(values) {
  const wrapper = markSearchable(
    el("div", { className: "multi-ai-settings" }),
    "マルチAI 合議 分担 統合 複数の叩くAI Claude Codex CLI"
  );
  const intro = el("p", {
    className: "settings-intro",
    textContent: "既定は単一AIです。複数AIを使うと、参加AIの推論に加えて分担計画や最終統合の推論が発生し、API利用量と処理時間が増えます。",
  });
  // 単一AI以外は Pro 対象の subagent を通る。保存を拒否されてから気づくことのないよう、
  // 無料版では選ぶ前に理由を出す。
  const proNote = el("p", {
    className: "field-note",
    textContent: "単一AI以外は PicoAgent Pro の機能です。「製品情報・ライセンス」でライセンスキーを登録すると保存できます。",
  });
  proNote.hidden = Boolean(data.license?.active);
  const mode = el("select");
  const modes = [
    { value: "single", label: "単一AI（既定・現在と同じ）" },
    { value: "consensus", label: "同じ質問を合議して統合" },
    { value: "collaborate", label: "仕事を分担して統合" },
    { value: "divide", label: "仕事を分担して個別結果を返す" },
  ];
  const selectedMode = values.PICOAGENT_MULTI_AI_MODE || "single";
  for (const item of modes) {
    mode.append(el("option", { value: item.value, textContent: item.label, selected: item.value === selectedMode }));
  }
  wrapper.append(intro, proNote, registerInput(
    "PICOAGENT_MULTI_AI_MODE",
    mode,
    el("div", { className: "field" }, [el("label", { textContent: "回答の作り方" }), mode])
  ));

  const raw = values.PICOAGENT_MULTI_AI_AGENTS || "[]";
  let agents = [];
  let parseError = "";
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) throw new Error("配列ではありません");
    agents = parsed.map(item => ({
      name: String(item?.name || ""),
      provider: String(item?.provider || data.active.providerId || "local"),
      model: String(item?.model || ""),
      focus: String(item?.focus || ""),
    }));
  } catch (error) {
    parseError = `保存済みの参加AI設定を読めません: ${errorDetail(error)}`;
  }
  const hidden = el("input", { type: "hidden", value: raw });
  const editor = el("div", { className: "multi-ai-editor" });
  const rows = el("div", { className: "multi-ai-rows" });
  const warning = el("p", { className: "field-note", textContent: parseError });
  warning.hidden = !parseError;

  function providerSelect(value) {
    const select = el("select");
    for (const provider of data.providers) {
      select.append(el("option", {
        value: provider.id,
        textContent: provider.label,
        selected: provider.id === value,
      }));
    }
    return select;
  }

  function syncAgents() {
    hidden.value = JSON.stringify(agents);
    hidden.dispatchEvent(new Event("input", { bubbles: true }));
    warning.hidden = true;
    updateDirtyState();
  }

  function renderRows() {
    rows.replaceChildren();
    agents.forEach((agent, index) => {
      const provider = providerSelect(agent.provider);
      const model = el("input", {
        type: "text",
        value: agent.model,
        placeholder: provider.value === "agent_cli" ? "claude または codex" : "空なら既定モデル",
      });
      const focus = el("input", {
        type: "text",
        value: agent.focus,
        placeholder: "担当の希望（空なら自動）",
      });
      const remove = el("button", { type: "button", className: "btn", textContent: "削除" });
      provider.addEventListener("change", () => {
        agent.provider = provider.value;
        model.placeholder = provider.value === "agent_cli" ? "claude または codex" : "空なら既定モデル";
        syncAgents();
      });
      model.addEventListener("input", () => { agent.model = model.value.trim(); syncAgents(); });
      focus.addEventListener("input", () => { agent.focus = focus.value.trim(); syncAgents(); });
      remove.addEventListener("click", () => {
        agents.splice(index, 1);
        renderRows();
        syncAgents();
      });
      rows.append(el("div", { className: "multi-ai-row" }, [
        el("span", { className: "subhead", textContent: `AI ${index + 1}` }),
        provider,
        model,
        focus,
        remove,
      ]));
    });
  }

  const add = el("button", { type: "button", className: "btn", textContent: "参加AIを追加" });
  add.addEventListener("click", () => {
    if (agents.length >= 8) {
      warning.textContent = "参加AIは最大8件です。";
      warning.hidden = false;
      return;
    }
    agents.push({ name: "", provider: data.active.providerId || "local", model: "", focus: "" });
    renderRows();
    syncAgents();
  });
  const agentsField = registerInput(
    "PICOAGENT_MULTI_AI_AGENTS",
    hidden,
    el("div", { className: "field" }, [
      el("label", { textContent: "参加するAI（2〜8件）" }),
      hidden,
      rows,
      add,
      warning,
      el("p", { className: "field-note", textContent: "同じAIを複数追加して回答のばらつきを比較することもできます。内蔵ローカルAIは安定性のため順番に実行します。" }),
    ])
  );
  editor.append(agentsField);

  const judgeProvider = providerSelect(values.PICOAGENT_MULTI_AI_JUDGE_PROVIDER || data.active.providerId || "local");
  editor.append(registerInput(
    "PICOAGENT_MULTI_AI_JUDGE_PROVIDER",
    judgeProvider,
    el("div", { className: "field" }, [el("label", { textContent: "分担計画・最終統合に使うAI" }), judgeProvider])
  ));
  editor.append(field(
    "PICOAGENT_MULTI_AI_JUDGE_MODEL",
    "分担計画・最終統合に使うモデル",
    "text",
    values.PICOAGENT_MULTI_AI_JUDGE_MODEL || "",
    "空なら選んだAIの既定モデル"
  ));
  editor.append(field(
    "PICOAGENT_MULTI_AGENT_MAX_CONCURRENCY",
    "最大並列数（1〜8、内蔵ローカルAIは常に1）",
    "number",
    values.PICOAGENT_MULTI_AGENT_MAX_CONCURRENCY || "3",
    "3"
  ));
  wrapper.append(editor);

  function updateVisibility() {
    editor.hidden = mode.value === "single";
    if (!editor.hidden && agents.length < 2 && !parseError) {
      while (agents.length < 2) {
        agents.push({ name: "", provider: data.active.providerId || "local", model: "", focus: "" });
      }
      renderRows();
      syncAgents();
    }
  }
  mode.addEventListener("change", () => {
    updateVisibility();
    updateDirtyState();
  });
  renderRows();
  updateVisibility();
  return wrapper;
}

function localModelPresetField(key, label, value) {
  const presets = data.localModelPresets || [];
  const localPresetLabel = (preset) => `${preset.localSize || preset.label} / 推奨VRAM ${preset.recommendedVramGb || 0}GB以上 / RAM ${preset.recommendedRamGb || 0}GB以上 / DL ${preset.downloadSize || "-"}`;
  const modelFamilies = [...new Set(presets.map(preset => preset.family).filter(Boolean))];
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
      label: "初期設定",
      options: [{ value: "auto", label: "軽量LFMを既定にする", hint: localModelPresetHint("auto") }],
    },
    ...presetGroups,
    {
      label: "カスタム",
      options: [{ value: "custom", label: "URL指定", hint: localModelPresetHint("custom") }],
    },
  ];
  const picker = groupedModelField(key, label, value, groups.filter(group => group.options.length > 0), { allowCustom: false });
  const hardware = data.localHardware || {};
  const recommendation = markSearchable(
    el("div", { className: "local-model-recommendation" }),
    "このPC向け ローカルAI 日本語 モデル RAM VRAM Liquid LFM Gemma Qwen LLM-jp"
  );
  const ramKnown = Number.isFinite(hardware.ramGb);
  const vramKnown = Number.isFinite(hardware.vramGb);
  const memorySummary = hardware.unifiedMemory
    ? `共有メモリ ${ramKnown ? `${hardware.ramGb}GB` : "取得できませんでした"}`
    : `RAM ${ramKnown ? `${hardware.ramGb}GB` : "取得できませんでした"} / 専用VRAM ${vramKnown ? `${hardware.vramGb}GB` : "取得できませんでした"}`;
  recommendation.append(
    el("div", { className: "local-model-recommendation-heading" }, [
      el("div", {}, [
        el("strong", { textContent: "このPC向けの日本語モデル候補" }),
        el("p", { textContent: `${memorySummary}。判定は起動保証ではなく、モデルを選ぶ目安です。` }),
      ]),
    ])
  );

  function suitability(preset) {
    const ramFit = ramKnown && hardware.ramGb >= Number(preset.recommendedRamGb || 0);
    const requiredVram = Number(preset.recommendedVramGb || 0);
    const vramFit = requiredVram > 0 && vramKnown && hardware.vramGb >= requiredVram;
    if (hardware.unifiedMemory && ramFit) return { rank: 0, kind: "fit", label: "共有メモリ向け" };
    if (requiredVram === 0 && ramFit) return { rank: 0, kind: "fit", label: "軽量・CPU向け" };
    if (vramFit) return { rank: 0, kind: "fit", label: "GPU向け" };
    if (ramFit) return { rank: 1, kind: "fit", label: "CPU/RAM向け" };
    if (!ramKnown || (requiredVram > 0 && !vramKnown)) return { rank: 2, kind: "unknown", label: "判定情報不足" };
    return { rank: 3, kind: "avoid", label: "このPCでは非推奨" };
  }

  const cards = presets.map((preset, index) => ({ preset, index, fit: suitability(preset) }))
    .sort((a, b) => a.fit.rank - b.fit.rank || a.index - b.index);
  const cardGrid = el("div", { className: "local-model-card-grid" });
  const buttons = new Map();
  function refreshCards() {
    const selected = inputs.get(key)?.value || "";
    for (const [id, button] of buttons) {
      const active = id === selected;
      button.classList.toggle("selected", active);
      button.textContent = active ? "選択中" : "このモデルを使う";
      button.disabled = active;
    }
  }
  for (const { preset, fit } of cards) {
    const button = el("button", { type: "button", className: "btn local-model-select", textContent: "このモデルを使う" });
    button.addEventListener("click", () => {
      picker.setValue?.(preset.id);
      refreshCards();
    });
    buttons.set(preset.id, button);
    cardGrid.append(el("article", { className: `local-model-card ${fit.kind}` }, [
      el("div", { className: "local-model-card-top" }, [
        el("strong", { textContent: preset.localSize || preset.label }),
        el("span", { className: `local-model-fit ${fit.kind}`, textContent: fit.label }),
      ]),
      el("p", { className: "local-model-japanese", textContent: preset.japaneseNote || "日本語対応" }),
      el("p", { textContent: preset.note || "" }),
      el("dl", {}, [
        el("div", {}, [el("dt", { textContent: "目安" }), el("dd", { textContent: `VRAM ${preset.recommendedVramGb || 0}GB / RAM ${preset.recommendedRamGb || 0}GB` })]),
        el("div", {}, [el("dt", { textContent: "取得" }), el("dd", { textContent: preset.downloadSize || "-" })]),
      ]),
      button,
    ]));
  }
  recommendation.append(cardGrid);
  inputs.get(key)?.addEventListener("change", refreshCards);
  refreshCards();
  return el("div", { className: "local-model-picker-block" }, [recommendation, picker]);
}

function localModelPresetHint(value) {
  if (value === "auto") return "初回起動時に連続ツール利用を検証済みのLFM2.5 2.6Bを選び、以後は選定結果を保存します。上のPC向け候補からいつでも変更できます。";
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
  return registerInput(key, select, markSearchable(
    el("div", { className: "field" }, [el("label", { textContent: label }), select]),
    key,
    label,
    options.map(option => option.label).join(" ")
  ));
}

function layoutPickerField(value) {
  if (value === "talk") value = "chat";
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
  const layoutId = v.PICOAGENT_LAYOUT === "talk" ? "chat" : (v.PICOAGENT_LAYOUT || "pet");
  const layout = LAYOUT_CHOICES.find(choice => choice.id === layoutId);
  const provider = data.providers.find(item => item.id === (v.PICOAGENT_PROVIDER || data.active.providerId));
  const cards = [
    // 変更頻度の高い順。テスト機能のキャラ作成はタブ一覧からだけ開く。
    { tab: "character", title: "キャラ選択", summary: v.PICOAGENT_CHARACTER || "既定キャラ" },
    { tab: "voice", title: "キャラの音声", summary: v.PICOAGENT_TTS_ENABLED === true ? "読み上げ有効" : "ミュート中" },
    { tab: "display", title: "画面レイアウト", summary: layout?.label || "ペット" },
    { tab: "providers", title: "使うAI", summary: provider?.label || v.PICOAGENT_PROVIDER || "ローカル" },
    { tab: "character-register", title: "キャラ登録", summary: "MMD・Live2D・完成パック" },
    { tab: "components", title: "追加機能・更新", summary: "音声・文字起こし・アプリ更新" },
    { tab: "permissions", title: "安全・許可", summary: "触ってよいフォルダ" },
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
      el("p", { textContent: "よく使う設定を目的別に開けます。" }),
    ]),
    grid,
  ]), "よく使う 設定 キャラクター 登録 選択 音声 画面 レイアウト AI モデル 追加機能 診断 修復");
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

function createOperationProgress(label = "処理の進捗") {
  const message = el("span", { className: "operation-progress-message" });
  message.setAttribute("role", "status");
  message.setAttribute("aria-live", "polite");
  const detail = el("span", { className: "operation-progress-detail" });
  const elapsed = el("span", { className: "operation-progress-elapsed" });
  const fill = el("span", { className: "operation-progress-fill" });
  const track = el("div", { className: "operation-progress-track is-indeterminate" }, [fill]);
  track.setAttribute("role", "progressbar");
  track.setAttribute("aria-label", label);
  const root = el("div", { className: "operation-progress" }, [
    el("div", { className: "operation-progress-copy" }, [message, detail, elapsed]),
    track,
  ]);
  let startedAt = 0;
  let elapsedTimer = null;

  function formatSize(value) {
    const bytes = Math.max(0, Number(value) || 0);
    if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)}GB`;
    if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)}MB`;
    if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)}KB`;
    return `${Math.round(bytes)}B`;
  }

  function updateElapsed() {
    if (!startedAt) return;
    const seconds = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
    const minutes = Math.floor(seconds / 60);
    const rest = seconds % 60;
    elapsed.textContent = minutes ? `経過 ${minutes}分${String(rest).padStart(2, "0")}秒` : `経過 ${rest}秒`;
  }

  function start(initialMessage = "処理中…") {
    if (!startedAt) startedAt = Date.now();
    if (!elapsedTimer) elapsedTimer = window.setInterval(updateElapsed, 1000);
    message.textContent = initialMessage;
    root.classList.add("is-active");
    track.classList.add("is-indeterminate");
    track.removeAttribute("aria-valuenow");
    fill.style.width = "35%";
    detail.textContent = "算出中";
    updateElapsed();
  }

  function update(progress) {
    if (!root.classList.contains("is-active")) start();
    if (progress?.message) message.textContent = progress.message;
    const downloaded = Number(progress?.downloaded) || 0;
    const total = Number(progress?.total) || 0;
    if (total > 0) {
      const percent = Math.max(0, Math.min(100, Math.round(downloaded / total * 100)));
      track.classList.remove("is-indeterminate");
      track.setAttribute("aria-valuenow", String(percent));
      fill.style.width = `${percent}%`;
      detail.textContent = `${percent}% ・ ${formatSize(downloaded)} / ${formatSize(total)}`;
    } else {
      track.classList.add("is-indeterminate");
      track.removeAttribute("aria-valuenow");
      fill.style.width = "35%";
      detail.textContent = downloaded > 0 ? formatSize(downloaded) : "算出中";
    }
  }

  function stop(finalMessage = "") {
    if (elapsedTimer) window.clearInterval(elapsedTimer);
    elapsedTimer = null;
    startedAt = 0;
    root.classList.remove("is-active");
    track.classList.add("is-indeterminate");
    track.removeAttribute("aria-valuenow");
    fill.style.width = "35%";
    detail.textContent = "";
    elapsed.textContent = "";
    if (finalMessage) message.textContent = finalMessage;
  }

  return { element: root, message, start, update, stop };
}

function advancedGroup(title, note, children) {
  return el("details", { className: "advanced-group" }, [
    el("summary", { textContent: title }),
    note ? el("p", { className: "advanced-group-note", textContent: note }) : null,
    ...children,
  ].filter(Boolean));
}

function componentPresentation(item) {
  const managed = item.installMode === "managed";
  let actionLabel = "公式ページを開く";
  if (managed && !item.ready) actionLabel = "公式からインストール";
  if (managed && item.ready) actionLabel = "修復（再ダウンロード）";
  if (item.versionUnknown) actionLabel = "修復して版を確認";
  if (item.newerThanTarget) actionLabel = `推奨版${item.targetVersion ? ` ${item.targetVersion}` : ""}へ戻す`;
  if (item.updateAvailable) actionLabel = `${item.targetVersion || "推奨版"}へ更新`;

  let stateClass = "";
  let badgeText = item.ready ? "準備済み" : (managed ? "公式から取得" : "外部導入");
  if (item.versionUnknown) [stateClass, badgeText] = [" has-warning", "確認が必要"];
  if (item.newerThanTarget) [stateClass, badgeText] = [" is-newer", "推奨版より新しい"];
  if (item.updateAvailable) [stateClass, badgeText] = [" has-update", "更新あり"];
  const versionText = item.targetVersion
    ? (item.installedVersion
      ? `インストール済み ${item.installedVersion} ／ 推奨版 ${item.targetVersion}`
      : `${item.ready ? "インストール済み版は未確認" : "未導入"} ／ 推奨版 ${item.targetVersion}`)
    : "";
  let confirmLabel = "もう一度押して修復";
  let pendingLabel = item.ready ? "修復中…" : "インストール中…";
  if (item.newerThanTarget) [confirmLabel, pendingLabel] = ["もう一度押して戻す", "推奨版へ変更中…"];
  if (item.updateAvailable) [confirmLabel, pendingLabel] = ["もう一度押して更新", "更新中…"];
  return { managed, actionLabel, stateClass, badgeText, versionText, confirmLabel, pendingLabel };
}

function buildComponentManager() {
  const sec = section("追加機能と公式取得元");
  sec.classList.add("component-manager");
  sec.append(el("p", {
    className: "settings-intro",
    textContent: "第三者ソフトや生成モデルをPicoAgentから再配布しません。管理対象は各公式配布元からPicoAgent専用領域へ直接取得し、WanとLTXは選んだ一方だけを追加取得できます。読み上げのミュート設定は変わりません。",
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
  const operationProgress = createOperationProgress("追加機能の処理進捗");
  const status = operationProgress.message;
  operationProgress.element.classList.add("component-manager-status");
  // 画像・動画・キャラクター生成に使うGPUと実行バックエンドを、取得を始める前に見せる。
  const generation = el("p", { className: "hint", textContent: "GPUを確認中…" });
  sec.append(core, generation, operationProgress.element, list);

  async function refreshGenerationEnvironment() {
    try {
      const info = await window.picoagent.settings.generationEnvironment();
      const gpu = info?.gpuName ? `${info.gpuName}（${info.gpuVendor}）` : "GPU不明";
      if (!info?.supported) {
        generation.textContent = `画像・動画・キャラクター生成: ${info?.reason || "対応GPUを確認できません。"}`;
        return;
      }
      const start = info.lowvram ? "省メモリ設定（lowvram）で起動" : "通常起動";
      const reinstall = info.needsReinstall
        ? " 導入済みランタイムがこのGPU向けではありません。入れ直してください。"
        : "";
      const unverified = (info.mpsUnverified || []).length
        ? ` このMacでは次のモデルが実機未確認です: ${info.mpsUnverified.join("、")}。`
        : "";
      generation.textContent = `画像・動画・キャラクター生成: ${gpu} を ${info.backendLabel} で使用します。ComfyUIは${start}します（${info.lowvramReason}）。${reinstall}${unverified}`;
    } catch (error) {
      generation.textContent = `画像・動画・キャラクター生成: GPUを確認できません（${error?.message || error}）`;
    }
  }

  function setBusy(busy) {
    sec.setAttribute("aria-busy", busy ? "true" : "false");
    if (busy) operationProgress.start(status.textContent || "処理中…");
    else operationProgress.stop();
    list.querySelectorAll("button").forEach(action => {
      action.disabled = busy || action.dataset.available === "false";
    });
  }

  async function refresh() {
    refreshGenerationEnvironment();
    const items = await window.picoagent.settings.componentStatus();
    list.innerHTML = "";
    const attentionCount = items.filter(item => item.updateAvailable || item.newerThanTarget || item.versionUnknown).length;
    const updateCount = items.filter(item => item.updateAvailable).length;
    const componentTab = document.getElementById("tab-components");
    if (componentTab) componentTab.textContent = `追加機能・更新${attentionCount ? `（${attentionCount}）` : ""}`;
    setQuickSettingsSummary("components", updateCount
      ? `${updateCount}件の更新があります`
      : (attentionCount ? `${attentionCount}件の確認が必要です` : "導入・修復・更新を管理"));
    for (const item of items) {
      const { managed, actionLabel, stateClass, badgeText, versionText, confirmLabel, pendingLabel } = componentPresentation(item);
      const button = el("button", {
        className: item.updateAvailable ? "primary" : (item.ready ? "secondary" : "primary"),
        type: "button",
        textContent: actionLabel,
        disabled: !item.available,
      });
      button.dataset.available = String(Boolean(item.available));
      const versionMove = item.updateAvailable || item.newerThanTarget
        ? `${item.installedVersion || "現在の版"}から${item.targetVersion || "推奨版"}へ入れ替えます。`
        : "公式配布を再ダウンロードして入れ替えます。";
      const preserved = item.id === "aivis"
        ? "音声モデル・辞書・設定は残ります。"
        : "PicoAgentの設定は残ります。";
      const confirmationMessage = `${item.label}: ${versionMove}${preserved}`;
      const actionConfirmation = item.ready ? twoStepAction(button, {
        initialLabel: actionLabel,
        confirmLabel,
        onArm: () => { status.textContent = confirmationMessage; },
        onCancel: () => {
          if (status.textContent === confirmationMessage) status.textContent = "";
        },
      }) : null;
      button.addEventListener("click", async () => {
        if (!managed) {
          const url = item.sources?.[0]?.url;
          if (url) window.open(url, "_blank", "noopener,noreferrer");
          status.textContent = `${item.label}は提供元の手順で導入し、必要なら「スキル・連携」で実行ファイルを指定してください。`;
          return;
        }
        if (actionConfirmation && !actionConfirmation.request()) return;
        setBusy(true);
        // 取得はバックエンドで走るので、この画面を閉じても止まらない。黙っていると
        // 「閉じれば止まる」と誤解され、数百MBの取得が知らないうちに続くことになる。
        const keepsRunning = "（この画面を閉じても取得は続きます）";
        button.textContent = pendingLabel;
        status.textContent = item.updateAvailable
          ? `${item.label} ${item.targetVersion}を公式配布元から取得しています…${keepsRunning}`
          : `${item.label}を公式配布元から取得しています…${keepsRunning}`;
        try {
          await window.picoagent.settings.installComponents([item.id]);
          const successMessage = item.updateAvailable
            ? `${item.label}を${item.targetVersion}へ更新しました。`
            : (item.newerThanTarget
              ? `${item.label}を推奨版${item.targetVersion ? ` ${item.targetVersion}` : ""}へ戻しました。`
              : `${item.label}を公式配布元から${item.ready ? "修復" : "導入"}しました。`);
          status.textContent = successMessage;
          try {
            await refresh();
          } catch (refreshError) {
            logApp("warn", "追加機能の状態を再読み込みできませんでした", errorDetail(refreshError));
            status.textContent = `${successMessage} 状態表示だけ再読み込みできませんでした。`;
          }
        } catch (error) {
          status.textContent = navigator.onLine
            ? `導入できませんでした: ${error?.message || error}`
            : "オフラインのため導入を見送りました。接続後にもう一度押してください。";
          button.textContent = actionLabel;
        } finally {
          setBusy(false);
        }
      });
      const actions = el("div", { className: "component-card-actions" }, [button]);
      if (item.removable) {
        const removeLabel = item.id === "aivis" ? "本体を削除" : "削除";
        const remove = el("button", { className: "danger", type: "button", textContent: removeLabel });
        remove.dataset.available = "true";
        const removeMessage = item.id === "aivis"
          ? "音声読み上げ本体だけを削除します。ダウンロード済みの音声モデル・辞書・設定は残ります。"
          : `${item.label}のPicoAgent管理下の本体とモデルを削除します。必要になれば再ダウンロードできます。`;
        const removeConfirmation = twoStepAction(remove, {
          initialLabel: removeLabel,
          confirmLabel: "もう一度押して削除",
          onArm: () => { status.textContent = removeMessage; },
          onCancel: () => {
            if (status.textContent === removeMessage) status.textContent = "";
          },
        });
        remove.addEventListener("click", async () => {
          if (!removeConfirmation.request()) return;
          setBusy(true);
          remove.textContent = "削除中…";
          status.textContent = `${item.label}を削除しています…`;
          try {
            await window.picoagent.settings.removeComponent(item.id);
            const successMessage = `${item.label}を削除しました。`;
            status.textContent = successMessage;
            try {
              await refresh();
            } catch (refreshError) {
              logApp("warn", "追加機能の状態を再読み込みできませんでした", errorDetail(refreshError));
              status.textContent = `${successMessage} 状態表示だけ再読み込みできませんでした。`;
            }
          } catch (error) {
            status.textContent = `削除できませんでした: ${error?.message || error}`;
            remove.textContent = removeLabel;
          } finally {
            setBusy(false);
          }
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
      list.append(markSearchable(el("div", { className: `component-card${item.ready ? " is-ready" : ""}${stateClass}` }, [
        el("div", { className: "component-card-copy" }, [
          el("strong", { textContent: item.label }),
          el("span", { textContent: item.description }),
          el("small", { textContent: item.sizeLabel }),
          versionText ? el("small", { textContent: versionText }) : null,
          item.versionError ? el("small", { className: "component-version-warning", textContent: item.versionError }) : null,
          sourceLinks,
        ].filter(Boolean)),
        el("span", { className: "component-badge", textContent: badgeText }),
        actions,
      ].filter(Boolean)), item.id, item.label, item.description, versionText, item.versionError));
    }
    applySettingsSearch();
  }

  replaceSettingsListener("component-progress", "component:progress", progress => {
    operationProgress.update(progress);
  }).catch(error => {
    logApp("warn", "追加機能の進捗を受け取れませんでした", errorDetail(error));
  });
  refresh().catch(error => {
    const componentTab = document.getElementById("tab-components");
    if (componentTab) componentTab.textContent = "追加機能・更新（要確認）";
    setQuickSettingsSummary("components", "状態を確認できません");
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
    if (item.heading) {
      // 分類見出しはタブではない。支援技術がタブ数へ数えないよう読み上げ対象から外す。
      const heading = el("div", { className: "tab-group-heading", textContent: item.heading });
      heading.setAttribute("aria-hidden", "true");
      tabs.append(heading);
      return;
    }
    const selected = item.id === activeSettingsTab;
    const button = el("button", {
      className: selected ? "tab active" : "tab",
      id: `tab-${item.id}`,
      textContent: item.label,
      type: "button",
      tabIndex: selected ? 0 : -1,
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
      for (const tab of tabs.querySelectorAll(".tab")) {
        const active = tab === button;
        tab.classList.toggle("active", active);
        tab.setAttribute("aria-selected", active ? "true" : "false");
        tab.tabIndex = active ? 0 : -1;
      }
      for (const current of panels.children) {
        const active = current === panel;
        current.classList.toggle("active", active);
        current.hidden = !active;
      }
      form.scrollTop = 0;
      applySettingsSearch();
    });
    button.addEventListener("keydown", event => {
      if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
      const tabButtons = [...tabs.querySelectorAll(".tab")];
      const index = tabButtons.indexOf(button);
      let next;
      if (["ArrowRight", "ArrowDown"].includes(event.key)) next = (index + 1) % tabButtons.length;
      else if (["ArrowLeft", "ArrowUp"].includes(event.key)) next = (index - 1 + tabButtons.length) % tabButtons.length;
      else if (event.key === "Home") next = 0;
      else if (event.key === "End") next = tabButtons.length - 1;
      else return;
      event.preventDefault();
      tabButtons[next].click();
      tabButtons[next].focus();
    });
    tabs.append(button);
    panels.append(panel);
    byId[item.id] = panel;
  });
  form.append(tabs, panels);
  // 起動時に「診断・修復」など一覧の下側を指定して開いた場合も、選択中のタブを見える位置へ送る。
  tabs.querySelector(".tab.active")?.scrollIntoView({ block: "nearest" });
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
    title: "Gmail の OAuth アクセストークン",
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
    title: "Google カレンダーの OAuth アクセストークン",
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
    title: "Google ドキュメントの OAuth アクセストークン",
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
    title: "Google ドライブの OAuth アクセストークン",
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
    title: "Google スプレッドシートの OAuth アクセストークン",
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
    title: "Google ToDo リストの OAuth アクセストークン",
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
    title: "Slack の Bot トークン／Webhook URL",
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
    title: "GitHub のアクセストークン",
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
    title: "Notion のインテグレーショントークン",
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
    title: "Discord の Webhook URL",
    steps: [
      "Discord のチャンネル設定で 連携サービス > ウェブフック を開きます。",
      "新しい Webhook を作成し、URL をコピーします。",
      "URL を PICOAGENT_DISCORD_WEBHOOK_URL に貼ります。",
    ],
    links: [["Discord Webhooks", "https://support.discord.com/hc/articles/228383668-Intro-to-Webhooks"]],
  },
  todoist: {
    title: "Todoist の API トークン",
    steps: [
      "Todoist の設定 > Integrations > Developer を開きます。",
      "API token をコピーして PICOAGENT_TODOIST_TOKEN に貼ります。",
      "必要なら既定 project ID も設定します。",
    ],
    links: [["Todoist Developer", "https://app.todoist.com/app/settings/integrations/developer"]],
  },
  trello: {
    title: "Trello の API キー／トークン",
    steps: [
      "Trello Developer API Keys で key を確認します。",
      "同じページから token を生成します。",
      "key / token / list ID をそれぞれ設定します。",
    ],
    links: [["Trello API Keys", "https://trello.com/app-key"]],
  },
  bluesky: {
    title: "Bluesky のアプリパスワード",
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
      title: "Google の OAuth アクセストークン",
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
      el("div", { className: "card-config", id: `skill-config-${tool.name}` }),
      tool.name,
      tool.label,
      tool.configSchema.map(field => `${field.key} ${field.label}`).join(" ")
    );
    panel.hidden = true;
    card.tabIndex = 0;
    card.setAttribute("role", "button");
    card.setAttribute("aria-label", `${tool.label || tool.name}の設定`);
    card.setAttribute("aria-controls", panel.id);
    card.setAttribute("aria-expanded", "false");
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
    card.addEventListener("keydown", event => {
      if (event.target !== card || event.isComposing) return;
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      if (!event.repeat) card.click();
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

// 自分で追加したキャラクター（PicoAgent/characters 配下）を一覧の形へそろえる。
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
        mmd: !!character.mmd,
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
  characterDefaultPersonas[id] = defaultText;
  characterPersona.value = characterOverrides[id] || defaultText;
}

function saveCurrentCharacterPersona(id) {
  if (!characterPersona) return;
  const value = characterPersona.value;
  if (value === characterDefaultPersonas[id]) delete characterOverrides[id];
  else characterOverrides[id] = value;
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
  if (kind === "mmd") return "MMD";
  if (character.user) return "追加PNG";
  return "PNGアニメ";
}

function characterRenderKind(manifest) {
  if (manifest.mmd?.model) return "mmd";
  const hasLive2d = !!manifest.live2d
    || (Array.isArray(manifest.variants) && manifest.variants.some(variant => variant?.live2d));
  if (hasLive2d) return "live2d";
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
  // 空欄またはパック既定値は上書きではない。初期表示しただけでメタデータを生成して
  // 「未保存」にしない。既存のメタデータだけの指定は、そのまま維持する。
  if (!speaker || speaker === String(defaultVoice.styleId || "")) {
    if (current.speaker) delete characterVoiceOverrides[id];
    return;
  }
  const next = { ...current, speaker };
  delete next.modelUuid;
  delete next.modelName;
  delete next.speakerUuid;
  delete next.styleName;
  delete next.sourceUrl;
  characterVoiceOverrides[id] = next;
}

function loadCharacterVoice(id) {
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

function bindStreamImagePicker(button, kind, label, onPicked, isAvailable = () => true) {
  button.addEventListener("click", async () => {
    button.disabled = true;
    statusEl.textContent = `${label}を選択しています…`;
    try {
      const result = await window.picoagent.settings.pickStreamImage(kind);
      if (!result?.path) {
        statusEl.textContent = `${label}の選択をキャンセルしました。`;
        return;
      }
      onPicked(result.path);
      statusEl.textContent = `${label}を選びました。「変更を保存」で反映します。`;
    } catch (error) {
      logApp("warn", `${label}を取り込めませんでした`, errorDetail(error));
      statusEl.textContent = `${label}を取り込めませんでした: ${error?.message || error}`;
    } finally {
      button.disabled = !isAvailable();
    }
  });
}

function streamSettingsField(values) {
  const wrapper = markSearchable(el("div", { className: "stream-settings" }), "配信 背景 ディスプレイ 画像 日付 時刻 LIVE");
  wrapper.append(selectField("PICOAGENT_STREAM_BACKGROUND", "配信背景", values.PICOAGENT_STREAM_BACKGROUND || "pastel", [
    { value: "pastel", label: "パステルスタジオ" },
    { value: "gaming", label: "落ち着いたゲーム部屋" },
    { value: "simple", label: "シンプルグラデーション" },
    { value: "sakura-room", label: "桜色のくつろぎ部屋" },
    { value: "dog-hobby-room", label: "犬の趣味部屋" },
    { value: "custom", label: "自分の画像" },
  ]));
  const backgroundPath = el("input", { type: "text", readOnly: true, value: values.PICOAGENT_STREAM_BACKGROUND_IMAGE || "", placeholder: "画像未選択" });
  const backgroundButton = el("button", { type: "button", className: "secondary", textContent: "背景画像を選ぶ" });
  bindStreamImagePicker(backgroundButton, "background", "背景画像", path => {
    backgroundPath.value = path;
    inputs.get("PICOAGENT_STREAM_BACKGROUND").value = "custom";
    updateDirtyState();
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
  bindStreamImagePicker(displayButton, "display", "ディスプレイ画像", path => {
    displayPath.value = path;
    inputs.get("PICOAGENT_STREAM_DISPLAY").value = "image";
    updateDirtyState();
  });
  wrapper.append(registerInput("PICOAGENT_STREAM_DISPLAY_IMAGE", displayPath, el("div", { className: "field" }, [el("label", { textContent: "ディスプレイに映す画像" }), displayPath, displayButton])));
  wrapper.append(el("p", { className: "hint", textContent: "配信表示では、左上に LIVE・日付・時刻を常時表示します。" }));
  return wrapper;
}

function buildCharacterStreamProfileEditor(values) {
  const enabled = el("input", { type: "checkbox" });
  const background = el("select", {}, [
    ["pastel", "パステルスタジオ"], ["gaming", "落ち着いたゲーム部屋"],
    ["simple", "シンプルグラデーション"], ["sakura-room", "桜色のくつろぎ部屋"],
    ["dog-hobby-room", "犬の趣味部屋"], ["custom", "自分の画像"],
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
  bindStreamImagePicker(backgroundButton, "background", "キャラ用背景画像", path => {
    backgroundPath.value = path;
    background.value = "custom";
    save();
  }, () => enabled.checked);
  bindStreamImagePicker(displayButton, "display", "キャラ用画面画像", path => {
    displayPath.value = path;
    display.value = "image";
    save();
  }, () => enabled.checked);
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
    el("option", { value: "bust", textContent: "胸上（基本）" }),
    el("option", { value: "upper", textContent: "上半身" }),
    el("option", { value: "full", textContent: "全身" }),
    el("option", { value: "custom", textContent: "細かく調整" }),
  ]);
  const zoom = el("input", { type: "range", min: "1", max: "4", step: ".05" });
  const x = el("input", { type: "number", min: "-4000", max: "4000", step: "1" });
  const y = el("input", { type: "number", min: "-4000", max: "4000", step: "1" });
  const zoomValue = el("output", { textContent: "2.05倍" });
  const positionValue = el("output", { textContent: "X 0 / Y 38" });
  const preview = el("div", {
    className: "framing-preview",
    role: "group",
    tabIndex: "0",
    ariaLabel: "キャラをドラッグして位置を調整",
  }, [
    el("img", { alt: "選択キャラの構図プレビュー", draggable: false }),
    el("strong", { textContent: "キャラをドラッグして位置調整" }),
    el("span", { textContent: "字幕テロップ" }),
  ]);
  const defaults = { bust: { zoom: 2.05, x: 0, y: 38 }, upper: { zoom: 1.55, x: 0, y: 18 }, full: { zoom: 1, x: 0, y: 0 } };
  const current = () => characterFraming?.[characterSelect.value]?.[layoutSelect.value];
  const updatePreview = () => {
    const image = preview.querySelector("img");
    image.style.transform = `translate(${Number(x.value) / 8}px,${Number(y.value) / 8}px) scale(${Number(zoom.value) / 1.55})`;
    zoomValue.textContent = `${Number(zoom.value).toFixed(2)}倍`;
    positionValue.textContent = `X ${Number(x.value)} / Y ${Number(y.value)}`;
  };
  const load = async () => {
    const character = characterCatalog.characters.find(item => item.id === characterSelect.value);
    const defaultPreset = character?.renderKind === "2d" ? "upper" : "bust";
    const saved = current() || { preset: defaultPreset, ...defaults[defaultPreset] };
    const fallback = defaults[saved.preset] || saved;
    presetSelect.value = saved.preset || "custom";
    zoom.value = String(saved.zoom ?? fallback.zoom ?? defaults[defaultPreset].zoom);
    x.value = String(saved.x ?? fallback.x ?? 0);
    y.value = String(saved.y ?? fallback.y ?? 0);
    updatePreview();
    const image = preview.querySelector("img");
    image.removeAttribute("src");
    try {
      const url = character ? await characterIconUrl(character) : "";
      if (url) image.src = url;
      image.alt = "選択キャラの構図プレビュー";
    } catch (error) {
      image.alt = "構図プレビューを読み込めません";
      logApp("warn", "構図プレビューを読み込めませんでした", errorDetail(error));
    }
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
  let drag = null;
  preview.addEventListener("pointerdown", event => {
    if (event.button !== 0) return;
    event.preventDefault();
    drag = { pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, x: Number(x.value), y: Number(y.value) };
    preview.setPointerCapture(event.pointerId);
    preview.classList.add("is-dragging");
  });
  preview.addEventListener("pointermove", event => {
    if (!drag || drag.pointerId !== event.pointerId) return;
    x.value = String(Math.round(Math.max(-4000, Math.min(4000, drag.x + (event.clientX - drag.startX) * 8))));
    y.value = String(Math.round(Math.max(-4000, Math.min(4000, drag.y + (event.clientY - drag.startY) * 8))));
    presetSelect.value = "custom";
    save();
  });
  const endDrag = event => {
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (preview.hasPointerCapture(event.pointerId)) preview.releasePointerCapture(event.pointerId);
    preview.classList.remove("is-dragging");
    drag = null;
  };
  preview.addEventListener("pointerup", endDrag);
  preview.addEventListener("pointercancel", endDrag);
  preview.addEventListener("lostpointercapture", endDrag);
  layoutSelect.addEventListener("change", load);
  const reset = el("button", { type: "button", className: "secondary", textContent: "この構図を初期値へ戻す" });
  reset.addEventListener("click", async () => {
    reset.disabled = true;
    const characterId = characterSelect.value;
    const layout = layoutSelect.value;
    if (characterFraming?.[characterId]) {
      delete characterFraming[characterId][layout];
      if (!Object.keys(characterFraming[characterId]).length) delete characterFraming[characterId];
    }
    try {
      await load();
      updateDirtyState();
      statusEl.textContent = "この表示形式の構図を初期値へ戻しました。「変更を保存」で反映します。";
    } finally {
      reset.disabled = false;
    }
  });
  const node = markSearchable(el("div", { className: "field framing-editor" }, [
    el("label", { textContent: "キャラごとの構図（表示形式別）" }),
    el("div", { className: "framing-controls" }, [
      el("label", {}, [el("span", { textContent: "表示形式" }), layoutSelect]),
      el("label", {}, [el("span", { textContent: "基本構図" }), presetSelect]),
      el("label", { className: "framing-zoom" }, [el("span", { textContent: "キャラサイズ" }), zoomValue, zoom]),
      el("label", {}, [el("span", { textContent: "横位置（X）" }), x]),
      el("label", {}, [el("span", { textContent: "縦位置（Y）" }), y]),
      positionValue,
    ]), preview,
    reset,
    el("p", { className: "hint", textContent: "サイズはスライダー、位置はプレビュー内のキャラをドラッグして調整できます。キャラと表示形式ごとに保存します。" }),
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
  // 不具合の連絡へ添える情報を1ファイルにまとめる。版・環境・設定・直近のログを入れ、
  // APIキー等の秘密値は「設定済み／未設定」だけにする。
  const exportDiagnostics = el("button", { type: "button", textContent: "診断情報を書き出す" });
  const exportStatus = markSearchable(
    el("div", { className: "hint diagnostics-export-status" }),
    "診断情報 書き出し ログ 問い合わせ サポート",
  );
  exportDiagnostics.addEventListener("click", async () => {
    exportDiagnostics.disabled = true;
    exportStatus.textContent = "書き出しています…";
    try {
      const result = await window.picoagent.settings.exportDiagnostics();
      exportStatus.textContent = `保存しました（APIキー等は含みません）: ${result?.path || ""}`;
    } catch (error) {
      exportStatus.textContent = `書き出せませんでした: ${error?.message || error}`;
    } finally {
      exportDiagnostics.disabled = false;
    }
  });
  toolbar.append(refresh, exportDiagnostics, note);
  const grid = el("div", { className: "diagnostic-grid" });
  diagnosticsSec.append(toolbar, exportStatus, grid);

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
          "ローカルAI",
          enginePhaseLabel(phase),
          `${engine.message || "状態メッセージなし"}${progress}`,
          phase === "ready" ? "ok" : statusTone(false, phase !== "error"),
          ["engine", "local", phase, engine.message]
        );
        if (phase === "error") engineCard.append(diagnosticRepairButton("ローカルAIを再起動", () => window.picoagent.restartApp(true, false)));
        grid.append(engineCard);
      } else {
        grid.append(diagnosticsFailureCard("ローカルAI", engineResult.reason));
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
  productSec.append(field(
    "PICOAGENT_LICENSE_TOKEN",
    "BOOTHなどで購入したライセンスキー",
    "password",
    data.values?.PICOAGENT_LICENSE_TOKEN || "",
    "PA1. から始まるキーを貼り付け"
  ));
  productSec.append(el("p", {
    className: "hint",
    textContent: "無料版は会話・天気・時刻などを利用できます。ProではPC操作、ファイル編集、Office・外部サービス・MCPなどの高度なスキルが有効になります。キーはWindowsではDPAPIで暗号化、macOSではログインキーチェーンへ預けて保存します。Steam版は将来Steamworksの所有確認へ対応予定です。",
  }));
  return markSearchable(productSec, "製品 バージョン データ プライバシー ライセンス サポート");
}

// アプリ本体の更新は「追加機能・更新」タブの先頭に置き、追加機能の版管理と同じ場所で済ませる。
function buildAppUpdateSection(v) {
  const updateSec = section("アプリのアップデート");
  const updateStatus = el("p", {
    className: "hint product-update-status",
    textContent: "起動時の確認が有効なら公開Gitへ一度だけ問い合わせます。オフラインの場合は何も表示しません。ここからも手動で確認できます。",
  });
  const checkUpdate = el("button", { type: "button", className: "secondary", textContent: "更新を確認" });
  const installUpdate = el("button", { type: "button", className: "primary", textContent: "アップデート", hidden: true });
  const installUpdateConfirmation = twoStepAction(installUpdate, {
    initialLabel: "アップデート",
    confirmLabel: "もう一度押してダウンロード",
    onArm: () => {
      updateStatus.textContent = "PicoAgent本体を更新します。設定・会話・追加キャラ・モデル・追加機能は残ります。";
    },
  });
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
    if (!installUpdateConfirmation.request()) return;
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
  updateSec.append(el("div", { className: "product-update-card" }, [
    el("p", { className: "hint", textContent: "更新するのはPicoAgent本体です。設定・会話履歴・追加キャラ・ダウンロード済みLLMモデル・導入済み追加機能はそのまま使い、再ダウンロードしません。各追加機能の版は下の一覧で個別に管理します。" }),
    updateStatus,
    updateActions,
  ]));
  updateSec.append(field("PICOAGENT_UPDATE_CHECK_ENABLED", "起動時に新しいバージョンを確認する", "checkbox", v.PICOAGENT_UPDATE_CHECK_ENABLED !== false));
  replaceSettingsListener("update-progress", "update:progress", progress => {
    const percent = progress.total > 0 ? Math.min(100, Math.round(progress.downloaded / progress.total * 100)) : 0;
    updateStatus.textContent = `アップデートをダウンロードしています… ${percent}%`;
  }).catch(error => {
    logApp("warn", "アップデート進捗を受け取れませんでした", errorDetail(error));
  });
  return markSearchable(updateSec, "更新 アップデート バージョン 最新版");
}

function buildDownloadedModelsSection() {
  const sec = section("ダウンロード済みモデル");
  sec.append(el("p", { className: "hint", textContent: "ローカルエンジンが取得した GGUF モデルです。使わないモデルを削除してディスクを空けられます（使用中のモデルは削除できません）。" }));
  const toolbar = el("div", { className: "diagnostics-toolbar" });
  const refresh = el("button", { type: "button", textContent: "一覧を再読み込み" });
  const total = el("div", { className: "hint", textContent: "" });
  const modelStatus = el("p", { className: "hint model-manager-status" });
  modelStatus.setAttribute("aria-live", "polite");
  toolbar.append(refresh, total);
  const list = el("div", { className: "model-list" });
  sec.append(toolbar, modelStatus, list);

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
          const deleteMessage = `モデル「${model.name}」を削除すると、再度使うときに再ダウンロードが必要です。`;
          const deleteConfirmation = twoStepAction(action, {
            initialLabel: "削除",
            confirmLabel: "もう一度押して削除",
            onArm: () => { modelStatus.textContent = deleteMessage; },
            onCancel: () => {
              if (modelStatus.textContent === deleteMessage) modelStatus.textContent = "";
            },
          });
          action.addEventListener("click", async () => {
            if (!deleteConfirmation.request()) return;
            action.disabled = true;
            action.textContent = "削除中…";
            modelStatus.textContent = `モデル「${model.name}」を削除しています…`;
            try {
              await window.picoagent.deleteDownloadedModel(model.name);
              modelStatus.textContent = `モデル「${model.name}」を削除しました。`;
              await refreshModels();
            } catch (error) {
              logApp("warn", "モデル削除に失敗しました", `${model.name}: ${errorDetail(error)}`);
              action.disabled = false;
              action.textContent = "削除";
              modelStatus.textContent = `削除に失敗しました: ${error?.message || error}`;
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

function buildCharacterMaker(registerInstalledCharacter) {
  const maker = section("原画からキャラクターを作成（テスト）");
  maker.append(el("p", {
    className: "settings-intro",
    textContent: "テスト機能です。1枚の原画から、背景除去・部位分解・2.5Dリグ・局所補完をPC内だけで実行します。元画像は上書きしません。RTX 5060 Laptop GPUの既知原画1体では約77分かかりました。所要時間は原画とPCにより変わります。",
  }));
  const sourceInput = el("input", { type: "text", readOnly: true, placeholder: "PNG / JPEGを選択" });
  const nameInput = el("input", { type: "text", placeholder: "表示名" });
  const personaInput = el("textarea", { placeholder: "性格や話し方（任意）" });
  const tagsInput = el("input", { type: "text", placeholder: "例: 青髪、正面、笑顔" });
  const pickButton = el("button", { className: "secondary", type: "button", textContent: "原画を選ぶ" });
  const createButton = el("button", { className: "primary", type: "button", textContent: "作成を開始" });
  const status = el("div", { className: "hint" });
  const list = el("div", { className: "card-list" });
  let runtimeInstalled = false;

  pickButton.addEventListener("click", async () => {
    try {
      const picked = await window.picoagent.settings.characterMakerPickSource();
      if (picked?.path) sourceInput.value = picked.path;
    } catch (error) {
      status.textContent = `原画を選べませんでした: ${error?.message || error}`;
    }
  });

  async function run(character, fromStage) {
    status.textContent = `${character.displayName}を生成しています…`;
    try {
      await window.picoagent.settings.characterMakerRun(character.characterId, fromStage);
      status.textContent = `${character.displayName}の生成が完了しました。`;
    } catch (error) {
      status.textContent = `生成を完了できませんでした: ${error?.message || error}`;
    } finally {
      await refresh();
    }
  }

  function renderCharacter(character) {
    const stageLabels = { isolate: "背景除去", decompose: "部位分解", rig2d: "2.5Dリグ", complete: "局所補完" };
    const stages = Object.entries(stageLabels).map(([id, label]) => {
      const state = character.stages?.[id] || { status: "pending" };
      return `${label}: ${state.status === "complete" ? "完了" : state.status === "running" ? "実行中" : state.status === "failed" ? "失敗" : "未実行"}`;
    }).join(" / ");
    const firstPending = Object.keys(stageLabels).find(id => character.stages?.[id]?.status !== "complete");
    const runButton = el("button", {
      className: "primary",
      type: "button",
      textContent: firstPending ? "続きから生成" : "最初から再生成",
      disabled: !runtimeInstalled,
    });
    runButton.addEventListener("click", () => run(character, firstPending || "isolate"));
    const installButton = el("button", {
      className: "secondary",
      type: "button",
      textContent: "PicoAgentへ反映",
      disabled: character.stages?.complete?.status !== "complete",
    });
    installButton.addEventListener("click", async () => {
      try {
        const result = await window.picoagent.settings.characterMakerInstall(character.characterId);
        if (!result.updated) await registerInstalledCharacter(result.id);
        status.textContent = result.updated
          ? `${result.name}を新しい生成結果へ更新しました。キャラを選び直すと反映されます。`
          : `${result.name}を追加して選びました。「保存」で反映します。`;
      } catch (error) {
        status.textContent = `PicoAgentへ反映できませんでした: ${error?.message || error}`;
      }
    });
    const previewButton = el("button", {
      className: "secondary",
      type: "button",
      textContent: "動作を確認",
      disabled: character.stages?.complete?.status !== "complete",
    });
    previewButton.addEventListener("click", async () => {
      previewButton.disabled = true;
      try {
        await window.picoagent.settings.characterMakerPreview(character.characterId);
        status.textContent = `${character.displayName}の確認画面を開きました。`;
      } catch (error) {
        status.textContent = `確認画面を開けませんでした: ${error?.message || error}`;
      } finally {
        previewButton.disabled = character.stages?.complete?.status !== "complete";
      }
    });
    return markSearchable(el("div", { className: "card-item" }, [
      el("div", { className: "card-title", textContent: character.displayName }),
      el("div", { className: "card-desc character-id", textContent: character.characterId }),
      el("div", { className: "card-desc", textContent: stages }),
      el("div", { className: "character-add-actions" }, [runButton, previewButton, installButton]),
    ]), character.displayName, character.characterId, stages);
  }

  async function refresh() {
    try {
      const result = await window.picoagent.settings.characterMakerList();
      runtimeInstalled = result.runtimeInstalled === true;
      list.replaceChildren(...(result.characters || []).map(renderCharacter));
      if (!result.supported) status.textContent = "キャラ作成は現在Windows 10/11 x64専用です。";
      else if (!runtimeInstalled) status.textContent = "キャラ作成用のローカル実行環境が未導入です。追加機能・更新から導入してください。";
      else if (!(result.characters || []).length) status.textContent = "原画を選び、最初のキャラクターを作成できます。";
      applySettingsSearch();
    } catch (error) {
      status.textContent = `キャラ作成の状態を取得できませんでした: ${error?.message || error}`;
    }
  }

  createButton.addEventListener("click", async () => {
    if (!sourceInput.value || !nameInput.value.trim()) {
      status.textContent = "原画と表示名を入力してください。";
      return;
    }
    createButton.disabled = true;
    try {
      const character = await window.picoagent.settings.characterMakerCreate({
        displayName: nameInput.value.trim(),
        sourcePath: sourceInput.value,
        personaPrompt: personaInput.value.trim(),
        identityTags: tagsInput.value.trim(),
      });
      await refresh();
      if (runtimeInstalled) await run(character, "isolate");
    } catch (error) {
      status.textContent = `作成を開始できませんでした: ${error?.message || error}`;
    } finally {
      createButton.disabled = false;
    }
  });
  replaceSettingsListener("character-maker-progress", "character-maker:progress", progress => {
    status.textContent = progress?.message || "キャラクターを生成しています…";
  });
  replaceSettingsListener("character-maker-state", "character-maker:state", refresh);
  const cancelButton = el("button", { className: "secondary", type: "button", textContent: "生成を中断" });
  cancelButton.addEventListener("click", async () => {
    await window.picoagent.settings.characterMakerCancel();
    status.textContent = "工程の停止を要求しました。";
  });
  maker.append(
    el("div", { className: "field" }, [el("label", { textContent: "原画" }), sourceInput, pickButton]),
    el("div", { className: "field" }, [el("label", { textContent: "表示名" }), nameInput]),
    el("div", { className: "field" }, [el("label", { textContent: "性格・話し方" }), personaInput]),
    el("div", { className: "field" }, [el("label", { textContent: "見た目の特徴" }), tagsInput]),
    el("div", { className: "character-add-actions" }, [createButton, cancelButton]),
    status,
    list,
  );
  refresh();
  return maker;
}

async function render() {
  ++modelLoadGeneration;
  providerDraftValues = {};
  renderingSettings = true;
  form.innerHTML = "";
  inputs.clear();
  inputWrappers.clear();
  alarmEditorState = null;
  scheduleEditorState = null;
  const v = data.values;
  const panels = createTabs([
    // 同じ分類のタブを見出しの下へ集め、分類内はよく使う順に並べる。
    { heading: "よく使う" },
    { id: "basic", label: "基本設定" },
    { id: "display", label: "画面・操作" },
    { heading: "キャラクター" },
    { id: "character", label: "キャラ選択" },
    { id: "voice", label: "キャラの音声" },
    { id: "character-register", label: "キャラ登録" },
    { id: "character-maker", label: "キャラ作成（テスト）" },
    { heading: "AI・機能" },
    { id: "providers", label: "AIモデル" },
    { id: "tools", label: "スキル・連携" },
    { id: "components", label: "追加機能・更新" },
    { heading: "安全・サポート" },
    { id: "permissions", label: "安全・許可" },
    { id: "skillguide", label: "使い方" },
    { id: "diagnostics", label: "診断・修復" },
  ]);
  panels.components.append(buildAppUpdateSection(v), buildComponentManager());
  panels["character-maker"].append(buildCharacterMaker(registerInstalledCharacter));

  // --- 会話と記憶（基本設定タブ） ---
  const global = section("会話と記憶");
  global.append(el("p", {
    className: "settings-intro",
    textContent: "キャラクターが会話から覚えることを決めます。使うAIは「AIモデル」、居眠りやホットキーは「画面・操作」にあります。",
  }));
  // --- 使うAI（AIモデルタブの先頭） ---
  const aiSec = section("使うAI");
  const providerSelect = el("select");
  for (const p of data.providers) {
    providerSelect.append(el("option", { value: p.id, textContent: p.label, selected: p.id === data.active.providerId }));
  }
  aiSec.append(registerInput(
    "PICOAGENT_PROVIDER",
    providerSelect,
    markSearchable(el("div", { className: "field" }, [el("label", { textContent: "使うAI" }), providerSelect]), "PICOAGENT_PROVIDER", "叩く AI", "使うAI", "プロバイダ")
  ));
  const modelField = field("PICOAGENT_MODEL", "AIモデル", "text", v.PICOAGENT_MODEL ?? data.active.model, data.active.model);
  aiSec.append(modelField);
  modelInput = inputs.get("PICOAGENT_MODEL");
  cliModelSelect = el("select", { ariaLabel: "CLIのAIモデル" });
  cliModelSelect.addEventListener("change", () => {
    modelInput.value = cliModelSelect.value;
    modelInput.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const refreshCliModels = el("button", { type: "button", className: "btn", textContent: "モデル一覧を更新" });
  refreshCliModels.addEventListener("click", () => loadModels("agent_cli"));
  const cliModelControls = el("div", {}, [cliModelSelect, refreshCliModels,
    el("p", { className: "hint", textContent: "「アプリのデフォルト」はClaude／Codexの設定に追従します。Codexの候補はCLIが更新するローカル一覧から取得します。" }),
  ]);
  modelField.append(cliModelControls);
  // OllamaとCLIは「使うAI」のモデル欄に出す。
  // 他プロバイダは下の「AIサービスごとの設定」で種類とサイズを選ぶため、ここでは隠す。
  function updateModelFieldVisibility() {
    const isCli = providerSelect.value === "agent_cli";
    const hidden = providerSelect.value !== "ollama" && !isCli;
    modelInput.hidden = isCli;
    cliModelControls.hidden = !isCli;
    modelField.dataset.conditionHidden = hidden ? "true" : "false";
    modelField.hidden = hidden;
  }
  updateModelFieldVisibility();
  const displaySec = section("画面レイアウト");
  displaySec.append(el("p", {
    className: "settings-intro",
    textContent: "用途に合う画面レイアウトをプレビューから選べます。通常2Dは従来の上半身位置、長身キャラとLive2Dは胸上を基準に表示し、ウィンドウは端をドラッグして拡大・縮小できます。",
  }));
  displaySec.append(layoutPickerField(v.PICOAGENT_LAYOUT === "talk" ? "chat" : (v.PICOAGENT_LAYOUT || "pet")));
  displaySec.append(selectField("PICOAGENT_THEME", "表示テーマ", v.PICOAGENT_THEME || "system", [
    { value: "system", label: "システム設定に追従" },
    { value: "light", label: "通常" },
    { value: "dark", label: "ダーク" },
  ]));
  displaySec.append(field("PICOAGENT_BUBBLE_FONT_PX", "会話文字の大きさ（px）", "text", v.PICOAGENT_BUBBLE_FONT_PX ?? "", "13"));
  displaySec.append(field("PICOAGENT_ALWAYS_ON_TOP", "常に手前に表示", "checkbox", v.PICOAGENT_ALWAYS_ON_TOP !== false));
  displaySec.append(streamSettingsField(v));
  displaySec.append(layoutResetField());
  // --- 操作（画面・操作タブ）: 普段の触り方に関わる項目だけを集める ---
  const operationSec = section("操作");
  operationSec.append(field("PICOAGENT_NAP_MINUTES", "無操作で居眠りするまでの分数（0で居眠りしない）", "text", v.PICOAGENT_NAP_MINUTES ?? "", "15"));
  operationSec.append(field("PICOAGENT_QUICK_ACTIONS", "入力欄のクイック操作（1行1件「ラベル|送る文章」。空欄なら既定の一覧）", "textarea", v.PICOAGENT_QUICK_ACTIONS ?? "", "👀 画面を見る|今の画面を見て、状況を短く教えて"));
  operationSec.append(advancedGroup("ホットキー", "キーボードから呼び出す・隠す組み合わせです。空欄は既定値、off と書くとそのキーだけ無効です。", [
    field("PICOAGENT_HOTKEY_ENABLED", "グローバルホットキーを使う", "checkbox", v.PICOAGENT_HOTKEY_ENABLED !== false),
    field("PICOAGENT_HOTKEY_SUMMON", "呼び出すキー", "text", v.PICOAGENT_HOTKEY_SUMMON ?? "", "CommandOrControl+Alt+P"),
    field("PICOAGENT_HOTKEY_HIDE", "隠す／戻すキー", "text", v.PICOAGENT_HOTKEY_HIDE ?? "", "CommandOrControl+Alt+H"),
  ]));
  global.append(field("PICOAGENT_MEMORY_ENABLED", "会話から好みや継続指示を暗黙的に覚える", "checkbox", v.PICOAGENT_MEMORY_ENABLED !== false));
  global.append(field("PICOAGENT_MEMORY_MAX_ITEMS", "長期メモリの最大件数", "text", v.PICOAGENT_MEMORY_MAX_ITEMS ?? "", "80"));
  global.append(field("PICOAGENT_PROJECT_RULES_AUTO_CAPTURE", "強く言い渡した作法を作業フォルダのAGENTS.mdへ自動で追記する", "checkbox", v.PICOAGENT_PROJECT_RULES_AUTO_CAPTURE === true));
  // AIの振る舞いを変える項目は変更頻度が低いので、使うAIの下で閉じておく。
  aiSec.append(advancedGroup("AIの詳細設定", "困っていなければ変更する必要はありません。", [
    field("PICOAGENT_SYSTEM_PROMPT", "全キャラ共通の追加指示", "textarea", v.PICOAGENT_SYSTEM_PROMPT ?? "", data.defaults.systemPrompt),
    field("PICOAGENT_SUPER_AGENT", "計画・自己評価・分担を強化する", "checkbox", v.PICOAGENT_SUPER_AGENT === true),
    field("PICOAGENT_SUPER_AGENT_MAX_STEPS", "強化モードの最大ループ数", "text", v.PICOAGENT_SUPER_AGENT_MAX_STEPS ?? "", "12"),
    field("PICOAGENT_TOOL_CONTEXT_TURNS", "スキル選びで見る過去発話数（0ですべて）", "text", v.PICOAGENT_TOOL_CONTEXT_TURNS ?? "", "5"),
    field("PICOAGENT_HTTP_METADATA_TIMEOUT_SECS", "AIモデル一覧の待ち時間（秒）", "text", v.PICOAGENT_HTTP_METADATA_TIMEOUT_SECS ?? "", "15"),
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
  const voiceSec = section("キャラの音声設定");
  const ttsMasterStatus = el("div", {
    className: v.PICOAGENT_TTS_ENABLED === true ? "tts-master-status enabled" : "tts-master-status muted",
    textContent: v.PICOAGENT_TTS_ENABLED === true ? "読み上げ：有効" : "読み上げ：ミュート中（起動時も話しません）",
  });
  voiceSec.append(ttsMasterStatus);
  voiceSec.append(el("p", {
    className: "hint",
    textContent: "読み上げ全体の有効・無効と、選択中キャラの声を設定します。キャラを変える場合は「キャラ選択」を開いてください。",
  }));
  voiceSec.append(field("PICOAGENT_TTS_ENABLED", "Aivis で回答を読み上げる", "checkbox", v.PICOAGENT_TTS_ENABLED === true));
  inputs.get("PICOAGENT_TTS_ENABLED")?.addEventListener("change", event => {
    const enabled = event.target.checked;
    ttsMasterStatus.className = `tts-master-status ${enabled ? "enabled" : "muted"}`;
    ttsMasterStatus.textContent = enabled ? "読み上げ：有効" : "読み上げ：ミュート中（起動時も話しません）";
    setQuickSettingsSummary("voice", enabled ? "読み上げ有効" : "ミュート中");
  });
  voiceSec.append(field("PICOAGENT_TTS_VOLUME", "音声・ゲーム効果音の音量（0〜100・初期値35）", "number", v.PICOAGENT_TTS_VOLUME ?? "35", "35"));
  voiceSec.append(field(
    "PICOAGENT_TTS_ENDPOINT",
    "外部VOICEVOX互換API（任意・このPC上のURL）",
    "text",
    v.PICOAGENT_TTS_ENDPOINT ?? "",
    "例: http://127.0.0.1:50021"
  ));
  voiceSec.append(el("p", {
    className: "hint",
    textContent: "空欄ではPicoAgent専用のAivisを11436番ポートで起動します。外部APIを使うときだけURLを明示してください。既定ポートへ自動接続したり、外部APIのモデルを変更したりしません。",
  }));
  const memoryManager = window.PicoMemorySettings.buildMemoryManager({
    api: window.picoagent.settings,
    section,
    el,
    markSearchable,
    refreshSearch: applySettingsSearch,
    setPageStatus: message => { statusEl.textContent = message; },
    setBusy: setSettingsMutationBusy,
    isBusy: () => settingsMutationBusy,
    logApp,
    errorDetail,
  });
  panels.basic.append(quickSettingsGuide(v), global, memoryManager);
  panels.display.append(displaySec, operationSec);
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
  let displayedCharacterId = selectedCharacter;
  setQuickSettingsSummary("character", selectedCharacterEntry?.name || selectedCharacter);
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
  const characterVoiceName = el("strong", { textContent: selectedCharacterEntry?.name || selectedCharacter });
  const characterVoiceDefault = el("span");
  const chooseCharacterButton = el("button", { className: "secondary", type: "button", textContent: "キャラを選び直す" });
  chooseCharacterButton.addEventListener("click", () => activateSettingsTab("character"));
  const characterVoiceContext = markSearchable(el("div", { className: "character-voice-context" }, [
    el("div", {}, [el("span", { textContent: "選択中: " }), characterVoiceName, characterVoiceDefault]),
    chooseCharacterButton,
  ]), "選択中 キャラクター 声 Aivis Style ID");
  characterSpeaker = el("input", { type: "text", inputMode: "numeric", ariaLabel: "このキャラの Aivis Style ID" });
  characterSpeaker.addEventListener("input", () => {
    saveCurrentCharacterVoice(displayedCharacterId);
    updateDirtyState();
  });
  const characterSpeakerField = markSearchable(el("div", { className: "field" }, [
    el("label", { textContent: "このキャラの Aivis Style ID" }),
    characterSpeaker,
    el("p", { className: "hint", textContent: "空欄にするとキャラクターパックの既定の声へ戻ります。Aivisの音声スタイルIDを入力してください。" }),
  ]), "キャラ 音声 Aivis speaker style ID 声を変える");
  const updateCharacterVoiceContext = id => {
    const entry = characterCatalog.characters.find(character => character.id === id);
    const voice = characterDefaultVoices[id] || {};
    const details = [voice.modelName, voice.styleName].filter(Boolean).join(" / ");
    const styleId = voice.styleId != null ? `ID ${voice.styleId}` : "";
    characterVoiceName.textContent = entry?.name || id;
    characterVoiceDefault.textContent = [details, styleId].filter(Boolean).length
      ? `（既定: ${[details, styleId].filter(Boolean).join(" / ")}）`
      : "（既定の声）";
  };
  voiceSec.append(characterVoiceContext, characterSpeakerField);
  characterSec.append(selectField("PICOAGENT_COMPANION_MODE", "相棒の役割", v.PICOAGENT_COMPANION_MODE || "both", [
    { value: "life", label: "いっしょに暮らす" },
    { value: "work", label: "いっしょに働く" },
    { value: "both", label: "暮らしと仕事の両方" },
  ]));
  characterSec.append(field("PICOAGENT_RELATIONSHIP_MEMORY_ENABLED", "いっしょの記録を保存する", "checkbox", v.PICOAGENT_RELATIONSHIP_MEMORY_ENABLED !== false));
  characterSec.append(field(
    "PICOAGENT_GAZE_TRACKING_ENABLED",
    "視線追従を使う（マウスと自動よそ見）",
    "checkbox",
    v.PICOAGENT_GAZE_TRACKING_ENABLED === true,
  ));
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
  characterLive2dSdkSelect.addEventListener("change", () => {
    saveCurrentCharacterLive2dSdk(displayedCharacterId);
    updateDirtyState();
  });
  characterSec.append(characterLive2dSdkField);
  characterPersona = el("textarea", { placeholder: "性格・口調・一人称など" });
  characterPersona.addEventListener("input", () => {
    saveCurrentCharacterPersona(displayedCharacterId);
    updateDirtyState();
  });
  characterSec.append(el("div", { className: "field" }, [el("label", { textContent: "このキャラの性格・口調" }), characterPersona]));
  await loadCharacterPersona(selectedCharacter);
  loadCharacterVoice(selectedCharacter);
  updateCharacterVoiceContext(selectedCharacter);
  await loadCharacterLive2dSdk(selectedCharacter);
  let previousCharacter = selectedCharacter;
  let characterSwitchQueue = Promise.resolve();
  let characterSwitchPending = 0;
  let characterSwitchOwnsBusy = false;
  const characterSwitchMessage = "キャラ設定を読み込み中…";
  let restoreCharacterSelectFocus = false;
  function queueCharacterSwitch(requestedCharacter) {
    restoreCharacterSelectFocus ||= document.activeElement === characterSelect;
    if (characterSwitchPending === 0 && !settingsMutationBusy) {
      characterSwitchOwnsBusy = true;
      setSettingsMutationBusy(true);
      statusEl.textContent = characterSwitchMessage;
    }
    characterSwitchPending += 1;
    characterSelect.disabled = true;
    characterSelect.setAttribute("aria-busy", "true");
    // 素早い連続選択でも、表示中の人格を読み終えてから次へ進む。並列に読むと、
    // 遅く完了した古い読込が新しいキャラの編集欄を上書きしてしまう。
    characterSwitchQueue = characterSwitchQueue.then(async () => {
      try {
        saveCurrentCharacterPersona(displayedCharacterId);
        saveCurrentCharacterVoice(displayedCharacterId);
        saveCurrentCharacterLive2dSdk(displayedCharacterId);
        await loadCharacterPersona(requestedCharacter);
        loadCharacterVoice(requestedCharacter);
        await loadCharacterLive2dSdk(requestedCharacter);
        await characterFramingEditor?.load();
        characterStreamProfileEditor?.load();
        previousCharacter = requestedCharacter;
        displayedCharacterId = requestedCharacter;
        characterSelect.value = requestedCharacter;
        const selectedEntry = characterCatalog.characters.find(character => character.id === requestedCharacter);
        setQuickSettingsSummary("character", selectedEntry?.name || requestedCharacter);
        updateCharacterVoiceContext(requestedCharacter);
      } catch (error) {
        characterSelect.value = previousCharacter;
        logApp("warn", "キャラクター設定の切替に失敗しました", errorDetail(error));
        statusEl.textContent = `人格設定を読み込めませんでした: ${error?.message || error}`;
      }
      refreshCharacterCards();
      updateDirtyState();
    }).catch(error => {
      // 表示更新の想定外エラーでもキューを rejected のまま残さず、次の切替操作を受け付ける。
      characterSelect.value = previousCharacter;
      logApp("error", "キャラクター設定の表示更新に失敗しました", errorDetail(error));
      statusEl.textContent = `キャラクター設定の表示を更新できませんでした: ${error?.message || error}`;
    });
    return characterSwitchQueue.finally(() => {
      characterSwitchPending -= 1;
      if (characterSwitchPending === 0) {
        characterSelect.disabled = false;
        characterSelect.setAttribute("aria-busy", "false");
        // 名前変更・削除の内側から呼ばれた場合、その処理のロックは解除しない。
        if (characterSwitchOwnsBusy) {
          characterSwitchOwnsBusy = false;
          setSettingsMutationBusy(false);
          if (statusEl.textContent === characterSwitchMessage) {
            const selected = characterCatalog.characters.find(item => item.id === displayedCharacterId);
            statusEl.textContent = `${selected?.name || displayedCharacterId} [${displayedCharacterId}] を選びました。「保存」で反映します。`;
          }
        }
        if (restoreCharacterSelectFocus) characterSelect.focus({ preventScroll: true });
        restoreCharacterSelectFocus = false;
      }
    });
  }
  characterSelect.addEventListener("change", event => {
    // 人格の読込は非同期なので、form の委譲 change が先に collect() すると、切替前の
    // 人格を新しいキャラの上書き値として保存してしまう。切替完了後に未保存判定する。
    event.stopPropagation();
    queueCharacterSwitch(characterSelect.value);
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
  // --- スキル設定（⚙ のあるスキルはクリックでその場に開く）。活用ガイドとは別タブに分ける ---
  const skillSec = section("スキル設定");
  if (!data.tools.length) {
    skillSec.append(el("p", { className: "hint", textContent: "登録されているスキルはありません。" }));
  } else {
    skillSec.append(el("p", { className: "hint", textContent: "カテゴリ別の一覧です。ジャンルで絞り込めます。⚙ のあるスキルをクリックすると設定を開けます（既定は折りたたみ）。頼み方の例は「使い方」タブを参照。" }));
    skillSec.append(buildGenreChips(() => skillGenreFilter, value => { skillGenreFilter = value; }));
    skillSec.append(buildSkillCategoryList(data.tools, v));
  }
  // MCPとローカルAPIは「AIが使える道具を外部とつなぐ」設定なので、スキルと同じタブに置く。
  panels.tools.append(skillSec, mcpSec, apiSec);

  // --- スキル活用ガイド（読み取り専用の一覧）。設定とは分離して別タブに置く ---
  const skillGuideSec = section("スキル活用ガイド");
  if (!data.tools.length) {
    skillGuideSec.append(el("p", { className: "hint", textContent: "登録されているスキルはありません。" }));
  } else {
    skillGuideSec.append(el("p", { className: "hint", textContent: "各スキルの「できること」と頼み方の例の一覧です。ジャンルで絞り込めます。設定（APIキー等）は「スキル・連携」タブで行います。" }));
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
    el("p", { className: "hint", textContent: "追加機能の選択とLive2D追加案内をいつでもやり直せます。会話履歴、設定、追加済みキャラクターは削除しません。" }),
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
    { id: "mmd", label: "MMD" },
    { id: "png", label: "PNGアニメ" },
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
  const appendCharacterCard = (character) => {
    // 先頭1文字。`slice(0, 1)` は絵文字始まりの名前を半分で切って壊すため、
    // コードポイント単位で取る。
    const initial = Array.from(String(character.name || character.id || "?"))[0] || "?";
    const fallback = el("div", { className: "character-icon-fallback", textContent: initial });
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
    });
    const nameText = el("span", { textContent: character.name });
    const texts = [
      el("div", { className: "card-name" }, [
        nameText,
        el("span", { className: "config-badge ok", textContent: character.renderType }),
      ]),
      el("div", { className: "card-desc character-id", textContent: `ID: ${character.id}` }),
      el("div", { className: "card-desc", textContent: character.description || character.id }),
      selectButton,
    ];
    // 削除できるのは自分で追加したものだけ（同梱キャラは実行ファイル側にあるため消せない）。
    if (character.user) {
      const renameInput = el("input", {
        type: "text",
        value: character.name,
        maxLength: 80,
        ariaLabel: `${character.name}の表示名`,
      });
      const renameButton = el("button", { className: "secondary", type: "button", textContent: "名前を変更" });
      const renameStatus = el("div", { className: "card-desc" });
      renameButton.addEventListener("click", async () => {
        const nextName = renameInput.value.trim();
        if (!nextName || nextName === character.name) {
          renameStatus.textContent = nextName ? "表示名は変更されていません。" : "表示名を入力してください。";
          return;
        }
        setSettingsMutationBusy(true);
        renameButton.disabled = true;
        try {
          const result = await window.picoagent.settings.renameCharacter(character.id, nextName);
          character.name = result.name;
          renameInput.value = result.name;
          nameText.textContent = result.name;
          icon.alt = `${result.name}のアイコン`;
          const option = [...characterSelect.options].find(item => item.value === character.id);
          if (option) option.textContent = `${result.name} — ${character.renderType} [${character.id}]`;
          card.dataset.search = searchText(character.id, result.name, character.description, character.renderType, "追加キャラ");
          // 既定人格の照合値は捨てない。先に消すと切替時の保存で「旧名の既定文」が
          // ユーザー上書きとして固定され、名前変更に追従しないまま未保存扱いになる。
          // マニフェストのキャッシュだけ捨て、`loadCharacterPersona` が新しい既定値を入れ直す。
          characterManifestCache.delete(character.id);
          if (characterSelect.value === character.id) await queueCharacterSwitch(character.id);
          renameStatus.textContent = `${result.name}へ変更しました。内部IDは ${character.id} のままです。`;
          statusEl.textContent = renameStatus.textContent;
          applyCharacterFilters();
        } catch (error) {
          renameStatus.textContent = `名前を変更できませんでした: ${error?.message || error}`;
        } finally {
          renameButton.disabled = false;
          setSettingsMutationBusy(false);
        }
      });
      renameInput.addEventListener("keydown", event => {
        if (event.isComposing || event.keyCode === 229) return;
        if (event.key === "Enter") {
          event.preventDefault();
          renameButton.click();
        }
      });
      texts.push(el("div", { className: "character-rename-row" }, [renameInput, renameButton]), renameStatus);
      const removeStatus = el("div", { className: "card-desc" });
      const removeButton = el("button", { className: "secondary", type: "button", textContent: "削除" });
      const removeMessage = "OS のゴミ箱へ移動します。設定・会話履歴は削除しません。";
      const removeConfirmation = twoStepAction(removeButton, {
        initialLabel: "削除",
        confirmLabel: "もう一度押して削除",
        onArm: () => { removeStatus.textContent = removeMessage; },
        onCancel: () => {
          if (removeStatus.textContent === removeMessage) removeStatus.textContent = "";
        },
      });
      removeButton.addEventListener("click", async () => {
        if (!removeConfirmation.request()) return;
        setSettingsMutationBusy(true);
        removeButton.disabled = true;
        removeButton.textContent = "ゴミ箱へ移動中…";
        try {
          const result = await window.picoagent.settings.removeCharacter(character.id);
          const wasSelected = characterSelect.value === character.id;
          characterCatalog.characters = characterCatalog.characters.filter(item => item.id !== character.id);
          [...characterSelect.options].find(option => option.value === character.id)?.remove();
          card.remove();
          if (wasSelected) {
            const fallbackCharacter = result?.activeCharacter || characterCatalog.default;
            characterSelect.value = fallbackCharacter;
            if (savedSnapshot) savedSnapshot.PICOAGENT_CHARACTER = fallbackCharacter;
            data.values.PICOAGENT_CHARACTER = fallbackCharacter;
            await queueCharacterSwitch(fallbackCharacter);
          }
          // 選択中キャラの既定値と現在値を照合し終えてから破棄する。先に消すと既定人格が
          // ユーザー上書きとして残り、削除直後に「未保存」と誤表示される。
          characterManifestCache.delete(character.id);
          delete characterDefaultPersonas[character.id];
          delete characterDefaultVoices[character.id];
          applyCharacterFilters();
          statusEl.textContent = `${character.name}をOSのゴミ箱へ移動しました。`;
        } catch (error) {
          removeButton.disabled = false;
          removeButton.textContent = "削除";
          removeStatus.textContent = `削除できませんでした: ${error?.message || error}`;
        } finally {
          setSettingsMutationBusy(false);
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
  };
  for (const character of characterCatalog.characters) appendCharacterCard(character);
  refreshCharacterCards();
  const empty = el("p", { className: "hint character-browser-empty", textContent: "条件に合うキャラクターが見つかりません。" });
  charSec.append(charList, empty);
  applyCharacterFilters();

  // 追加コマンドが成功した時点でユーザー領域には検証済みパックが確定している。
  // 設定ウィンドウは閉じても同じWebViewを再表示するため、「開き直す」だけでは一覧が
  // 再構築されない。追加した1体をバックエンドの正本から読み直し、その場で一覧と選択欄へ足す。
  async function registerInstalledCharacter(id) {
    const added = (await loadUserCharacters(characterCatalog.characters))
      .find(character => character.id === id);
    if (!added) {
      // 同梱キャラと同じidのパックは一覧で同梱側だけが残るため、追加しても選べない。
      // 「追加できたのに出てこない」で終わらせず、直しかたまで伝える。
      const conflict = characterCatalog.characters.some(character => character.id === id && !character.user);
      throw new Error(conflict
        ? `同梱キャラクターと同じID（${id}）のため一覧へ出せません。パックの id を変えて追加し直してください。`
        : `追加したキャラクター ${id} を一覧から読み直せませんでした。`);
    }
    const manifest = await loadCharacterManifestForSettings(added);
    added.renderType = characterRenderType(added, manifest);
    added.renderKind = characterRenderKind(manifest);
    characterCatalog.characters.push(added);
    characterSelect.append(el("option", {
      value: added.id,
      textContent: `${added.name} — ${added.renderType} [${added.id}]`,
    }));
    appendCharacterCard(added);
    characterTypeFilter = "user";
    refreshFilterButtons();
    applyCharacterFilters();
    characterSelect.value = added.id;
    await queueCharacterSwitch(added.id);
    refreshCharacterCards();
    return added;
  }

  // --- キャラクターを自分で追加する ---
  // 同梱キャラは実行ファイルへ焼き込まれているため増減できない。追加分はユーザー領域
  // （PicoAgent/characters）へ入れる。導入は検証を通ってから確定するので、
  // 壊れたパックが一覧へ出ることはない。
  const addSec = section("キャラクターを追加");
  const addHint = el("p", {
    className: "hint",
    textContent: "取得したMMDのPMX・zip・フォルダやLive2Dを追加できます。モデルはこのPCのユーザー領域へ保存します。",
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
      // 同梱・追加済みと同じidは一覧で衝突する。ファイルを書く前に断る。
      if (characterCatalog.characters.some(character => character.id === id)) {
        throw new Error(`キャラクターID ${id} は既に使われています。別のIDにしてください。`);
      }
      const picked = await window.picoagent.settings.pickCharacterPack(folder);
      if (!picked?.path) {
        addStatus.textContent = "選択をキャンセルしました。";
        return;
      }
      addStatus.textContent = "Live2Dモデルを確認し、キャラクターパックを作成しています…";
      const result = await window.picoagent.settings.installLive2dCharacter(picked.path, id, name);
      const added = await registerInstalledCharacter(result.id);
      addStatus.textContent = `${added.name}（${added.id}）を追加して選びました。「保存」で反映します。`;
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

  const mmdIdInput = el("input", { type: "text", value: `mmd-${Date.now().toString(36)}`, maxLength: 64 });
  const mmdNameInput = el("input", { type: "text", value: "マイMMD", maxLength: 80 });
  async function installRawMmd(folder) {
    addStatus.textContent = "選択中…";
    try {
      const id = mmdIdInput.value.trim();
      const name = mmdNameInput.value.trim();
      if (!id || !name) throw new Error("キャラクターIDと表示名を入力してください。");
      if (characterCatalog.characters.some(character => character.id === id)) {
        throw new Error(`キャラクターID ${id} は既に使われています。別のIDにしてください。`);
      }
      const picked = await window.picoagent.settings.pickCharacterPack(folder);
      if (!picked?.path) { addStatus.textContent = "選択をキャンセルしました。"; return; }
      addStatus.textContent = "MMDモデルとテクスチャを確認しています…";
      const result = await window.picoagent.settings.installMmdCharacter(picked.path, id, name);
      const added = await registerInstalledCharacter(result.id);
      addStatus.textContent = `${added.name}（${added.id}）を追加して選びました。「保存」で反映します。`;
    } catch (error) {
      addStatus.textContent = `追加できませんでした: ${error?.message || error}`;
    }
  }
  const rawMmdButtons = el("div", { className: "character-add-actions" }, [
    Object.assign(el("button", { className: "primary", type: "button", textContent: "MMDのzip／PMXを追加" }), { onclick: () => installRawMmd(false) }),
    Object.assign(el("button", { className: "secondary", type: "button", textContent: "MMDフォルダを追加" }), { onclick: () => installRawMmd(true) }),
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
      const added = await registerInstalledCharacter(result.id);
      addStatus.textContent = `${added.name}（${added.id}）を追加して選びました。「保存」で反映します。`;
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
    el("div", { className: "subhead", textContent: "MMDモデルを追加" }),
    el("div", { className: "field" }, [el("label", { textContent: "キャラクターID（英数字・-・_）" }), mmdIdInput]),
    el("div", { className: "field" }, [el("label", { textContent: "表示名" }), mmdNameInput]),
    rawMmdButtons,
    el("p", { className: "hint", textContent: "PMXとテクスチャの相対配置を保ったまま追加します。複数のPMXがある場合は、表示するPMXを直接選んでください。元ファイルは加工しません。" }),
    el("div", { className: "subhead", textContent: "Live2Dモデルを追加" }),
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
  panels.character.append(charSec, characterSec);
  panels.voice.append(voiceSec);
  panels["character-register"].append(addSec);

  // --- 叩く AI コネクタごとの設定（選択中のプロバイダの設定だけ表示する） ---
  const provSec = section("AIサービスごとの設定");
  let activeProviderBlock = null;
  function renderProviderConfig(id) {
    if (activeProviderBlock) {
      rememberProviderDraft(activeProviderBlock);
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
    for (const f of provider.configSchema) block.append(field(f.key, f.label, f.type, providerDraftValues[f.key] ?? v[f.key], f.placeholder));
    activeProviderBlock = block;
    provSec.append(block);
    // CLIからローカルAIへ戻った際に作り直される入力にも、モデル同期を接続する。
    inputs.get("PICOAGENT_LOCAL_MODEL_PRESET")?.addEventListener("change", () => {
      if (providerSelect.value !== "local" || !modelInput) return;
      const preset = inputs.get("PICOAGENT_LOCAL_MODEL_PRESET").value;
      modelInput.value = preset === "auto" ? providerDefaultModel("local") : preset;
      updateDirtyState();
    });
  }
  renderProviderConfig(data.active.providerId);
  const multiAiSec = section("複数AIの使い方");
  multiAiSec.append(multiAiSettingsField(v));
  panels.providers.append(el("p", {
    className: "settings-intro",
    textContent: "会話に使うAIとモデルをまとめています。普段は「使うAI」だけ確認すれば十分です。外部ツール連携（MCP）とローカルAPIは「スキル・連携」にあります。",
  }));
  panels.providers.append(aiSec, provSec, multiAiSec, buildDownloadedModelsSection());

  // スキルごとの設定は上のスキル一覧（クリックで展開）に統合済み。
  // --- 許可（ファイル/実行）: 全スキル共通のファイル/ディレクトリ許可ルール ---
  const permSec = section("許可（ファイル/実行）");
  permSec.append(el("p", { className: "hint", textContent: "ファイル操作やコマンド実行で触ってよい場所を決めます。許可したフォルダの中だけを読み書き・操作・実行できます。C:\\Windows と Program Files は設定に関わらず常に拒否します。" }));
  permSec.append(field("PICOAGENT_ALLOWED_ROOT", "既定で許可するフォルダ", "text", v.PICOAGENT_ALLOWED_ROOT ?? "", "例: C:\\Users\\you\\work"));
  permSec.append(el("p", { className: "field-note", textContent: "ここは相対パスの基準にもなります。空ならホームフォルダを使います。" }));
  permSec.append(field("PICOAGENT_ALLOWED_PATHS", "ほかにも許可するフォルダ（改行または ; 区切り）", "textarea", v.PICOAGENT_ALLOWED_PATHS ?? "", ""));
  permSec.append(field("PICOAGENT_DENIED_PATHS", "拒否パス（改行または ; 区切り。この配下は常に拒否）", "textarea", v.PICOAGENT_DENIED_PATHS ?? "", ""));
  permSec.append(el("p", { className: "hint", textContent: "コマンド実行（run_command）と自力実行（code_runner）の有効化は「スキル・連携」タブの各スキルカードで切り替えます。既定はどちらも無効です。" }));
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
  await loadModels(providerSelect.value);
  applySettingsSearch();
  renderingSettings = false;
  updateDirtyState();
  if (requestedSettingsTab === "character-add") activateSettingsTab("character-register");
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
    const fallback = data.defaultLocalModelPreset || provider.defaultModel || "lfm25_2_6b";
    const preset = inputs.get("PICOAGENT_LOCAL_MODEL_PRESET")?.value || fallback;
    return preset === "auto" ? fallback : preset;
  }
  return provider.defaultModel || "";
}

async function loadModels(providerId, applyProviderDefault = false) {
  const generation = ++modelLoadGeneration;
  const targetInput = modelInput;
  const initialModel = targetInput?.value;
  const isCurrent = () => generation === modelLoadGeneration && targetInput === modelInput;
  modelOptions.innerHTML = "";
  statusEl.textContent = providerId === "ollama" ? "Ollama のモデル一覧を取得中…" : "";
  let models = [];
  try {
    models = await window.picoagent.settings.listModels(providerId);
    if (!isCurrent()) return;
  } catch (error) {
    if (!isCurrent()) return;
    logApp("warn", "モデル一覧を取得できませんでした", errorDetail(error));
    statusEl.textContent = `モデル一覧を取得できませんでした: ${error.message}`;
    return;
  }
  if (!isCurrent()) return;
  for (const m of models) modelOptions.append(el("option", { value: m }));
  if (!modelInput) return;
  const preferred = providerDefaultModel(providerId);
  if (modelInput.value !== initialModel) {
    // 一覧の取得中に手入力したモデル名は利用者の編集を優先する。
  } else if (applyProviderDefault) {
    modelInput.value = models.includes(preferred) ? preferred : (models[0] || preferred || modelInput.value);
  } else if (providerId !== "agent_cli" && providerId === data.active.providerId && !models.includes(modelInput.value)) {
    modelInput.value = models.includes(preferred) ? preferred : (models[0] || modelInput.value);
  }
  if (providerId === "agent_cli" && cliModelSelect) {
    if (!modelInput.value) modelInput.value = preferred;
    const choices = [...new Set([...models, modelInput.value].filter(Boolean))];
    cliModelSelect.replaceChildren(...choices.map(value => el("option", {
      value,
      textContent: ["claude", "codex", "custom"].includes(value)
        ? `${value}：アプリのデフォルト` : value.replace(":", "："),
      selected: value === modelInput.value,
    })));
  }
  statusEl.textContent = providerId === "ollama"
    ? (models.length ? `${models.length}件の Ollama モデルを読み込みました。` : "Ollama のモデルが見つかりません。")
    : "";
  updateDirtyState();
}

function collect() {
  const patch = cloneSettings(savedSnapshot || data.values || {});
  Object.assign(patch, providerDraftValues);
  for (const [key, input] of inputs) {
    patch[key] = input.type === "checkbox" ? input.checked : input.value;
  }
  for (const warning of data.unavailableSecrets || []) {
    const input = inputs.get(warning.key);
    if ((input || Object.hasOwn(providerDraftValues, warning.key)) && patch[warning.key] === "") patch[warning.key] = PRESERVE_UNREADABLE_SECRET;
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
    // 前面に出す確認なので、支援技術にもダイアログとして伝え、背後のフォームへ
    // Tab で抜けられないようにする（抜けられると、確認中に裏の設定を操作できてしまう）。
    card.setAttribute("role", "dialog");
    card.setAttribute("aria-modal", "true");
    card.setAttribute("aria-labelledby", "restartDialogTitle");
    card.append(
      el("h2", { id: "restartDialogTitle", textContent: "再起動して反映します" }),
      el("p", { textContent: "選んだ設定（使うAI・モデル・音声・キャラクターなど）は、PicoAgent を再起動して起動画面でまとめて反映します。今すぐ再起動しますか？" }),
    );
    const actions = el("div", { className: "restart-actions" });
    const later = el("button", { className: "btn", textContent: "あとで", type: "button" });
    const now = el("button", { className: "btn primary", textContent: "今すぐ再起動", type: "button" });
    const previousFocus = document.activeElement;
    const close = (value) => {
      document.removeEventListener("keydown", onKeydown, true);
      modalOpen = false;
      overlay.remove();
      if (previousFocus instanceof HTMLElement) previousFocus.focus();
      resolve(value);
    };
    function onKeydown(event) {
      if (event.key === "Escape") {
        // 取り消し側（あとで）へ倒す。設定は保存済みで、次回起動時に反映される。
        event.preventDefault();
        close(false);
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = [later, now];
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      } else if (!focusable.includes(document.activeElement)) {
        event.preventDefault();
        first.focus();
      }
    }
    later.addEventListener("click", () => close(false));
    now.addEventListener("click", () => close(true));
    actions.append(later, now);
    card.append(actions);
    overlay.append(card);
    document.body.append(overlay);
    modalOpen = true;
    document.addEventListener("keydown", onKeydown, true);
    now.focus();
  });
}

saveBtn.addEventListener("click", async () => {
  if (settingsMutationBusy || !savedSnapshot) return;
  setSettingsMutationBusy(true);
  statusEl.textContent = "保存中…";
  try {
    const current = collect();
    const request = settingsChangeRequest(current, savedSnapshot, data.valueRevisions);
    if (!Object.keys(request.patch).length) {
      statusEl.textContent = "変更はありません";
      return;
    }
    const res = await window.picoagent.settings.save(request.patch, request.baseRevisions);
    if (Array.isArray(res.conflicts) && res.conflicts.length) {
      const draft = cloneSettings(request.patch);
      const latest = await window.picoagent.settings.load();
      data = latest;
      await render();
      const latestSnapshot = cloneSettings(collect());
      data.values = { ...cloneSettings(latest.values), ...draft };
      await render();
      savedSnapshot = latestSnapshot;
      updateDirtyState();
      statusEl.textContent = `別の操作で更新された設定を読み直しました（${res.conflicts.length}件）。入力内容を確認して、もう一度保存してください`;
      return;
    }
    savedSnapshot = cloneSettings(current);
    data.values = cloneSettings(current);
    data.valueRevisions = cloneSettings(res.valueRevisions || data.valueRevisions);
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
    statusEl.textContent = `保存に失敗: ${error?.message || error}`;
  } finally {
    setSettingsMutationBusy(false);
  }
});

resetChangesBtn?.addEventListener("click", async () => {
  if (!savedSnapshot || resetChangesBtn.disabled || settingsMutationBusy) return;
  setSettingsMutationBusy(true);
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
    setSettingsMutationBusy(false);
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
  if (modalOpen) return; // 確認ダイアログ表示中は裏の保存・検索を動かさない
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
  try {
    data = await window.picoagent.settings.load();
    await render();
    savedSnapshot = cloneSettings(collect());
    updateDirtyState();
    if (data.unavailableSecrets?.length) {
      statusEl.textContent = data.unavailableSecrets
        .map(item => `${item.key}: ${item.reason}`)
        .join(" / ");
    }
  }
  catch (error) {
    renderingSettings = false;
    data = null;
    savedSnapshot = null;
    saveBtn.disabled = true;
    resetChangesBtn.disabled = true;
    logApp("error", "設定画面の読み込みに失敗しました", errorDetail(error));
    statusEl.textContent = `設定画面の読み込みに失敗: ${error?.message || error}`;
    const retry = el("button", { type: "button", textContent: "再読み込み" });
    retry.addEventListener("click", () => window.location.reload());
    form.replaceChildren(
      el("h2", { textContent: "設定を読み込めませんでした" }),
      el("p", { textContent: "設定は変更していません。再読み込みしても失敗する場合は、下のエラー内容を確認してください。" }),
      retry,
    );
  }
})().catch(error => {
  renderingSettings = false;
  logApp("error", "設定初期化に失敗しました", errorDetail(error));
  statusEl.textContent = `設定初期化に失敗: ${error?.message || error}`;
});
