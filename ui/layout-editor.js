(() => {
  "use strict";

  const STORAGE_KEY = "picoagent.layoutEditor.v1";
  const app = document.getElementById("petApp");
  const toggle = document.getElementById("layoutEditToggle");
  const toolbar = document.getElementById("layoutEditorToolbar");
  const targetSelect = document.getElementById("layoutEditorTarget");
  const undoButton = document.getElementById("layoutEditorUndo");
  const saveButton = document.getElementById("layoutEditorSave");
  const resetButton = document.getElementById("layoutEditorReset");
  const closeButton = document.getElementById("layoutEditorClose");
  if (!app || !toggle || !toolbar || !targetSelect || typeof window.Moveable !== "function") return;

  const sources = {
    character: document.getElementById("character"),
    bubble: document.getElementById("bubble"),
    reply: document.getElementById("reply"),
    history: document.getElementById("historyLog"),
    display: document.getElementById("streamDisplay"),
  };
  for (const source of Object.values(sources)) source?.setAttribute("data-layout-editor-source", "true");

  const readStore = () => {
    try {
      const value = JSON.parse(localStorage.getItem(STORAGE_KEY) || "{}");
      return value && typeof value === "object" ? value : {};
    } catch {
      return {};
    }
  };
  let store = readStore();
  let editing = false;
  let selectedKey = "character";
  let undoStack = [];
  let savedAtOpen = null;

  const layoutId = () => app.dataset.layout === "talk" ? "chat" : (app.dataset.layout || "pet");
  const cloneLayoutState = () => JSON.parse(JSON.stringify(store[layoutId()] || {}));
  const snapshot = () => Object.fromEntries(Object.entries(sources).map(([key, source]) => [key, {
    translate: source?.style.translate || "",
    scale: source?.style.scale || "",
  }]));
  const applyState = state => {
    for (const [key, source] of Object.entries(sources)) {
      if (!source) continue;
      source.style.translate = state?.[key]?.translate || "";
      source.style.scale = state?.[key]?.scale || "";
    }
  };

  let moveable;
  const applySavedLayout = () => {
    applyState(store[layoutId()] || {});
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
  });
  moveable.on("scaleStart", rememberUndo);
  moveable.on("scale", event => {
    event.target.style.translate = `${event.drag.beforeTranslate[0]}px ${event.drag.beforeTranslate[1]}px`;
    event.target.style.scale = `${event.scale[0]} ${event.scale[1]}`;
  });

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
  const save = () => {
    store[layoutId()] = snapshot();
    localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
    savedAtOpen = cloneLayoutState();
    undoStack = [];
    undoButton.disabled = true;
  };
  const reset = () => {
    rememberUndo();
    applyState({});
    delete store[layoutId()];
    localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
    savedAtOpen = {};
    moveable.updateRect();
  };

  toggle.addEventListener("click", () => editing ? leave() : enter());
  closeButton.addEventListener("click", () => leave());
  saveButton.addEventListener("click", save);
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
  new MutationObserver(() => {
    if (editing) leave({ keepWorking: true });
    applySavedLayout();
  }).observe(app, { attributes: true, attributeFilter: ["data-layout"] });
  applySavedLayout();
})();
