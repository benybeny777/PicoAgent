(() => {
  "use strict";

  // 旧保存先（webviewのlocalStorage）。正本は設定の `PICOAGENT_LAYOUT_PLACEMENTS` へ移した。
  // 初回だけここから読み出して設定へ移し、以後は読み書きしない。
  const LEGACY_STORAGE_KEY = "picoagent.layoutEditor.v1";
  const LEGACY_MIGRATED_KEY = "picoagent.layoutEditor.migrated";
  const app = document.getElementById("petApp");
  const toggle = document.getElementById("layoutEditToggle");
  const toolbar = document.getElementById("layoutEditorToolbar");
  const targetSelect = document.getElementById("layoutEditorTarget");
  const undoButton = document.getElementById("layoutEditorUndo");
  const saveButton = document.getElementById("layoutEditorSave");
  const resetButton = document.getElementById("layoutEditorReset");
  const closeButton = document.getElementById("layoutEditorClose");
  if (!app || !toggle || !toolbar || !targetSelect) return;
  if (typeof window.Moveable !== "function") {
    // 黙って何も起きないボタンを残さない（配信物から vendor が落ちた場合に気づけない）。
    toggle.disabled = true;
    toggle.title = "配置編集の部品（Moveable）を読み込めませんでした";
    window.PicoUI?.logApp?.("error", "配置編集を初期化できません", "window.Moveable がありません");
    return;
  }

  const sources = {
    character: document.getElementById("character"),
    bubble: document.getElementById("bubble"),
    reply: document.getElementById("reply"),
    history: document.getElementById("historyLog"),
    display: document.getElementById("streamDisplay"),
  };
  for (const source of Object.values(sources)) source?.setAttribute("data-layout-editor-source", "true");

  // 読めない旧保存値をそのまま空で上書きすると、他の表示形式へ作った配置まで消える。
  // Rust 側の `store::quarantine_corrupt()` と同じく、退避してから空で続ける。
  const readLegacyStore = () => {
    let raw = null;
    try { raw = localStorage.getItem(LEGACY_STORAGE_KEY); }
    catch (error) { window.PicoUI?.logApp?.("warn", "旧配置設定を読めません", error); }
    if (!raw) return null;
    try {
      const value = JSON.parse(raw);
      if (value && typeof value === "object" && !Array.isArray(value)) return value;
      throw new Error("配置の保存値がオブジェクトではありません");
    } catch (error) {
      const backup = `${LEGACY_STORAGE_KEY}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
      try { localStorage.setItem(backup, raw); }
      catch (saveError) { window.PicoUI?.logApp?.("error", "壊れた配置設定を退避できませんでした", saveError); }
      window.PicoUI?.logApp?.("warn", `配置設定を読めないため空で続けます（退避先: ${backup}）`, error);
      return null;
    }
  };
  const isPlacementObject = value => Boolean(value) && typeof value === "object" && !Array.isArray(value);
  // 正本は設定側。ここは編集中の作業用の写しで、確定時にまとめて保存する。
  let store = {};
  // 設定への保存は確定時だけ（ドラッグのたびに設定ファイルを書かない）。
  const persist = () => {
    const save = window.picoagent?.saveLayoutPlacements;
    if (!save) return;
    Promise.resolve(save(store)).catch(error => {
      // 保存できなくても編集中の見た目は保つ。理由はアプリログへ残す。
      window.PicoUI?.logApp?.("error", "配置を保存できませんでした", error);
    });
  };
  let editing = false;
  let selectedKey = "character";
  let undoStack = [];
  let savedAtOpen = null;
  const MIN_SCALE = 0.25;
  const MAX_SCALE = 4;
  const KEEP_VISIBLE = 48;

  const layoutId = () => app.dataset.layout === "talk" ? "chat" : (app.dataset.layout || "pet");
  const characterId = () => sources.character?.dataset.characterId || "";
  const stateForCurrentLayout = () => {
    const layout = layoutId();
    const state = { ...(store[layout] || {}) };
    const savedCharacter = characterId() && store.characters?.[characterId()]?.[layout];
    state.character = savedCharacter || {};
    return state;
  };
  const cloneLayoutState = () => JSON.parse(JSON.stringify(stateForCurrentLayout()));
  const snapshot = () => Object.fromEntries(Object.entries(sources).map(([key, source]) => [key, {
    translate: source?.style.translate || "",
    scale: source?.style.scale || "",
  }]));
  const parseNumbers = value => String(value || "").match(/-?\d+(?:\.\d+)?/g)?.map(Number) || [];
  // CSS の既定に合わせる。`translate` は2値目を省くとY=0、`scale` は省くとXと同じ倍率になる。
  const parseTranslate = value => {
    const numbers = parseNumbers(value);
    return numbers.length ? [numbers[0], numbers[1] ?? 0] : [0, 0];
  };
  const parseScale = value => {
    const numbers = parseNumbers(value);
    return numbers.length ? [numbers[0], numbers[1] ?? numbers[0]] : [1, 1];
  };
  const safeScale = scale => scale.map(value => Math.min(MAX_SCALE, Math.max(MIN_SCALE, Number(value) || 1)));
  const visibleCharacterRect = () => {
    const candidates = sources.character?.querySelectorAll(".character-view > canvas, .character-view > img") || [];
    for (const candidate of candidates) {
      const style = getComputedStyle(candidate);
      const rect = candidate.getBoundingClientRect();
      if (style.display !== "none" && style.visibility !== "hidden" && rect.width > 1 && rect.height > 1) return rect;
    }
    return sources.character?.getBoundingClientRect();
  };
  const roundBack = (value, correction) =>
    correction > 0 ? Math.ceil(value) : correction < 0 ? Math.floor(value) : Math.round(value);
  const constrainSource = source => {
    if (!source) return;
    const hasTranslate = Boolean(source.style.translate);
    const hasScale = Boolean(source.style.scale);
    // CSSだけで決まる既定配置は変更しない。利用者または保存値が変形した対象だけを守る。
    if (!hasTranslate && !hasScale) return;
    let [x, y] = parseTranslate(source.style.translate);
    const scale = safeScale(parseScale(source.style.scale));
    if (hasScale) source.style.scale = `${scale[0]} ${scale[1]}`;
    if (hasTranslate) source.style.translate = `${x}px ${y}px`;
    const rect = source === sources.character ? visibleCharacterRect() : source.getBoundingClientRect();
    const beforeX = x;
    const beforeY = y;
    if (source === sources.character) {
      // 配信形式のキャラ枠は左ステージ全体を覆うため、枠の端だけ残しても中のモデルは消える。
      // 実際に表示中の canvas / img の中心を、チャット欄を除いた表示領域へ残す。
      const historyRect = sources.history?.getBoundingClientRect();
      const rightLimit = layoutId() === "stream" && historyRect
        ? Math.max(KEEP_VISIBLE, historyRect.left - KEEP_VISIBLE)
        : innerWidth - KEEP_VISIBLE;
      const centerX = (rect.left + rect.right) / 2;
      const centerY = (rect.top + rect.bottom) / 2;
      if (centerX < KEEP_VISIBLE) x += KEEP_VISIBLE - centerX;
      if (centerX > rightLimit) x -= centerX - rightLimit;
      if (centerY < KEEP_VISIBLE) y += KEEP_VISIBLE - centerY;
      if (centerY > innerHeight - KEEP_VISIBLE) y -= centerY - (innerHeight - KEEP_VISIBLE);
    } else {
      if (rect.right < KEEP_VISIBLE) x += KEEP_VISIBLE - rect.right;
      if (rect.left > innerWidth - KEEP_VISIBLE) x -= rect.left - (innerWidth - KEEP_VISIBLE);
      if (rect.bottom < KEEP_VISIBLE) y += KEEP_VISIBLE - rect.bottom;
      if (rect.top > innerHeight - KEEP_VISIBLE) y -= rect.top - (innerHeight - KEEP_VISIBLE);
    }
    if (hasTranslate || x !== beforeX || y !== beforeY) {
      // 補正した軸は端数を安全側（押し戻した向き）へ丸める。四捨五入だと最大0.5pxだけ
      // 戻し足りず、残すはずの余白を割ったままになる。
      source.style.translate = `${roundBack(x, x - beforeX)}px ${roundBack(y, y - beforeY)}px`;
    }
  };
  const applyState = state => {
    for (const [key, source] of Object.entries(sources)) {
      if (!source) continue;
      source.style.translate = state?.[key]?.translate || "";
      source.style.scale = state?.[key]?.scale || "";
    }
    requestAnimationFrame(() => Object.values(sources).forEach(constrainSource));
  };

  let moveable;
  const applySavedLayout = () => {
    applyState(stateForCurrentLayout());
    requestAnimationFrame(() => moveable?.updateRect());
  };
  const selectTarget = key => {
    if (!sources[key] || !moveable) return;
    selectedKey = key;
    targetSelect.value = key;
    moveable.target = sources[key];
    moveable.keepRatio = key === "character" || key === "display";
    requestAnimationFrame(() => moveable.updateRect());
  };
  const rememberUndo = () => {
    undoStack.push(snapshot());
    if (undoStack.length > 30) undoStack.shift();
    undoButton.disabled = undoStack.length === 0;
  };

  moveable = new window.Moveable(document.body, {
    target: sources.character,
    draggable: true,
    scalable: true,
    keepRatio: true,
    snappable: true,
    snapCenter: true,
    snapGap: true,
    snapThreshold: 6,
    elementGuidelines: Object.values(sources).filter(Boolean),
    origin: false,
  });
  moveable.target = null;
  moveable.on("dragStart", rememberUndo);
  moveable.on("drag", event => {
    event.target.style.translate = `${event.beforeTranslate[0]}px ${event.beforeTranslate[1]}px`;
    constrainSource(event.target);
  });
  moveable.on("dragEnd", () => moveable.updateRect());
  moveable.on("scaleStart", rememberUndo);
  moveable.on("scale", event => {
    event.target.style.translate = `${event.drag.beforeTranslate[0]}px ${event.drag.beforeTranslate[1]}px`;
    const scale = safeScale(event.scale);
    event.target.style.scale = `${scale[0]} ${scale[1]}`;
    constrainSource(event.target);
  });
  moveable.on("scaleEnd", () => moveable.updateRect());

  const enter = () => {
    if (editing) return;
    editing = true;
    savedAtOpen = cloneLayoutState();
    undoStack = [];
    undoButton.disabled = true;
    app.classList.add("is-layout-editing");
    toggle.setAttribute("aria-pressed", "true");
    toolbar.hidden = false;
    selectTarget(selectedKey);
  };
  const leave = ({ keepWorking = false } = {}) => {
    if (!editing) return;
    if (!keepWorking) applyState(savedAtOpen || {});
    editing = false;
    app.classList.remove("is-layout-editing");
    toggle.setAttribute("aria-pressed", "false");
    toolbar.hidden = true;
    moveable.target = null;
  };
  // 表示形式やキャラクターが切り替わった直後にも呼ばれるため、保存先は明示で受け取る
  // （切替後の枠へ、切替前に編集した位置・倍率を書き込まないため）。
  const save = (layout = layoutId(), id = characterId()) => {
    const state = snapshot();
    const { character: characterState, ...sharedState } = state;
    store[layout] = sharedState;
    if (id) {
      store.characters ||= {};
      store.characters[id] ||= {};
      store.characters[id][layout] = characterState;
    }
    persist();
    savedAtOpen = cloneLayoutState();
    undoStack = [];
    undoButton.disabled = true;
  };
  const saveAndLeave = (layout, id) => {
    save(layout, id);
    leave({ keepWorking: true });
  };
  const reset = () => {
    rememberUndo();
    applyState({});
    const layout = layoutId();
    delete store[layout];
    const id = characterId();
    if (id && store.characters?.[id]) {
      delete store.characters[id][layout];
      if (!Object.keys(store.characters[id]).length) delete store.characters[id];
      if (!Object.keys(store.characters).length) delete store.characters;
    }
    persist();
    savedAtOpen = {};
    moveable.updateRect();
  };

  // 画面上の「閉じる」や配置ボタンで終えた変更は記憶する。Escだけを明示的な取消にする。
  toggle.addEventListener("click", () => editing ? saveAndLeave() : enter());
  window.picoagent?.on?.("pet:layout-edit", key => {
    if (sources[key]) selectedKey = key;
    enter();
    selectTarget(selectedKey);
  });
  closeButton.addEventListener("click", () => saveAndLeave());
  saveButton.addEventListener("click", () => save());
  resetButton.addEventListener("click", reset);
  undoButton.addEventListener("click", () => {
    const state = undoStack.pop();
    if (!state) return;
    applyState(state);
    undoButton.disabled = undoStack.length === 0;
    moveable.updateRect();
  });
  targetSelect.addEventListener("change", () => selectTarget(targetSelect.value));
  app.addEventListener("pointerdown", event => {
    if (!editing || !(event.target instanceof Element)) return;
    const match = Object.entries(sources).reverse()
      .find(([, source]) => source && (event.target === source || source.contains(event.target)));
    if (match) selectTarget(match[0]);
  }, true);
  window.addEventListener("keydown", event => {
    if (!editing) return;
    if (event.key === "Escape") leave();
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") {
      event.preventDefault();
      undoButton.click();
    }
  });
  window.addEventListener("resize", () => {
    Object.values(sources).forEach(constrainSource);
    requestAnimationFrame(() => moveable?.updateRect());
  });
  // 監視コールバックは属性が変わった後に走るので、切替前の保存先を覚えておく。
  let editingLayout = layoutId();
  let editingCharacter = characterId();
  const layoutObserver = new MutationObserver(() => {
    if (editing) saveAndLeave(editingLayout, editingCharacter);
    editingLayout = layoutId();
    applySavedLayout();
  });
  layoutObserver.observe(app, { attributes: true, attributeFilter: ["data-layout"] });
  const migrateLegacyCharacterState = () => {
    const id = characterId();
    if (!id) return;
    let changed = false;
    for (const [layout, state] of Object.entries(store)) {
      if (layout === "characters" || !state?.character) continue;
      store.characters ||= {};
      store.characters[id] ||= {};
      store.characters[id][layout] ||= state.character;
      delete state.character;
      changed = true;
    }
    if (changed) persist();
  };
  const characterObserver = new MutationObserver(() => {
    // 切替途中の配置は切替前のキャラクターへ保存する（新しいキャラへ漏らさない）。
    if (editing) saveAndLeave(editingLayout, editingCharacter);
    editingCharacter = characterId();
    migrateLegacyCharacterState();
    applySavedLayout();
  });
  // キャラ要素が無い画面でも他の対象の編集は続けられるよう、監視だけを見送る。
  if (sources.character) {
    characterObserver.observe(sources.character, { attributes: true, attributeFilter: ["data-character-id"] });
  }
  // Moveable は document へ独自のリスナを張るので、離脱時に明示的に解放する。
  window.addEventListener("pagehide", () => {
    layoutObserver.disconnect();
    characterObserver.disconnect();
    moveable.destroy();
  }, { once: true });
  // 設定から配置を受け取って反映する。`renderer.js` が `pet_init` の値で1度呼ぶ。
  const hydrate = placements => {
    store = isPlacementObject(placements) ? placements : {};
    let migrated = false;
    try { migrated = localStorage.getItem(LEGACY_MIGRATED_KEY) === "1"; }
    catch (error) { window.PicoUI?.logApp?.("warn", "配置設定の移行状態を読めません", error); }
    // 設定側が空のときだけ、旧localStorageの値を1度だけ引き継ぐ。
    if (!migrated && !Object.keys(store).length) {
      const legacy = readLegacyStore();
      if (legacy && Object.keys(legacy).length) {
        store = legacy;
        migrateLegacyCharacterState();
        persist();
        window.PicoUI?.logApp?.("info", "配置編集の保存先を設定へ移しました");
      }
    }
    if (!migrated) {
      try { localStorage.setItem(LEGACY_MIGRATED_KEY, "1"); }
      catch (error) { window.PicoUI?.logApp?.("warn", "配置設定の移行状態を保存できません", error); }
    }
    migrateLegacyCharacterState();
    applySavedLayout();
  };
  window.PicoLayoutEditor = { hydrate };
  // 読み込み順に関係なく受け取れるよう、先に置かれた値があればその場で使う。
  if (window.__picoLayoutPlacements) hydrate(window.__picoLayoutPlacements);
  else applySavedLayout();
})();
