// edit.js - ソースの直接操作（選択枠・つまみ・移動・拡大縮小）の持ち主。
//
// 2つの入口がある。どちらも最後は stage.commit() で保存する。
//   - 右クリック「キャラの位置とサイズを調整」: キャラだけを調整するモード（本体ドラッグでキャラを動かす）。
//   - 上端メニュー「配置」・右クリック「配置を編集（OBS風）」: この形式の全ソースを動かすモード。
// ドラッグ中は stage.preview() で見た目だけを変え、保存はつまみを離した時だけ行う。

import { anchorFactors } from "./scene-model.js";
import { SOURCE_LABELS } from "../sources/index.js";
import { el } from "./util.js";

const MIN_SIZE = 48;
const CORNERS = ["top-left", "top-right", "bottom-left", "bottom-right"];

export function createEditor({ layer, toolbar, stageElement, stage, host, toast, onModeChange }) {
  let mode = ""; // "" | "character" | "all"
  let selected = "character";
  let drag = null;
  let wheelTimer = 0;
  let wheelTarget = "";
  const frames = new Map();

  function targets() {
    if (mode === "all") return stage.sourceNames().filter(name => stage.sources.get(name) && !stage.sources.get(name).element.hidden);
    if (mode === "character") return ["character"];
    return [];
  }

  function frameFor(name) {
    if (frames.has(name)) return frames.get(name);
    const label = el("span", { className: "edit-label", textContent: SOURCE_LABELS[name] || name });
    const move = el("button", { type: "button", className: "edit-move", title: "ドラッグで位置を動かす（ダブルクリックで既定へ戻す）", textContent: "✥" }, [], { "aria-label": `${SOURCE_LABELS[name] || name}を動かす` });
    const handles = CORNERS.map(corner => el("span", { className: "edit-handle", dataset: { corner } }, [], { "aria-hidden": "true" }));
    const frame = el("div", { className: "edit-frame", dataset: { target: name } }, [label, move, ...handles]);
    frame.addEventListener("pointerdown", event => startDrag(event, name));
    frame.addEventListener("wheel", event => wheelResize(event, name), { passive: false });
    move.addEventListener("dblclick", event => {
      event.preventDefault();
      reset(name);
    });
    layer.append(frame);
    frames.set(name, frame);
    return frame;
  }

  function render() {
    const wanted = new Set(targets());
    for (const [name, frame] of frames) frame.hidden = !wanted.has(name);
    const viewport = stageElement.getBoundingClientRect();
    for (const name of wanted) {
      const rect = stage.rectOf(name);
      if (!rect) continue;
      const windowRect = stage.toWindow(rect);
      const frame = frameFor(name);
      // stage 座標を固定レイヤーの viewport 座標へ変える。画面外の保存値は表示だけ切る。
      const bounds = {
        left: windowRect.left + viewport.left,
        top: windowRect.top + viewport.top,
        right: windowRect.left + windowRect.width + viewport.left,
        bottom: windowRect.top + windowRect.height + viewport.top,
      };
      const visible = {
        left: Math.max(bounds.left, viewport.left),
        top: Math.max(bounds.top, viewport.top),
        right: Math.min(bounds.right, viewport.right),
        bottom: Math.min(bounds.bottom, viewport.bottom),
      };
      frame.hidden = visible.right <= visible.left || visible.bottom <= visible.top;
      if (frame.hidden) continue;
      frame.classList.toggle("is-top-clipped", bounds.top < viewport.top);
      for (const handle of frame.querySelectorAll(".edit-handle")) {
        const corner = handle.dataset.corner;
        const x = corner.endsWith("left") ? bounds.left : bounds.right;
        const y = corner.startsWith("top") ? bounds.top : bounds.bottom;
        handle.hidden = x < viewport.left || x > viewport.right || y < viewport.top || y > viewport.bottom;
      }
      frame.classList.toggle("is-selected", name === selected);
      frame.classList.add("is-body-draggable");
      Object.assign(frame.style, {
        left: `${visible.left}px`,
        top: `${visible.top}px`,
        width: `${visible.right - visible.left}px`,
        height: `${visible.bottom - visible.top}px`,
      });
    }
    layer.hidden = wanted.size === 0;
    toolbar.hidden = !mode;
  }

  /** キャラの構図で原画を2倍以上に引き伸ばさない高さの上限（窓座標）。 */
  function maxCharacterHeight() {
    const geometry = stage.geometry();
    const pixelRatio = (window.devicePixelRatio || 1) * geometry.scale;
    return host.maxHeight(stage.currentFraming(), pixelRatio) * geometry.scale;
  }

  function startDrag(event, name) {
    if (event.button !== 0) return;
    const handle = event.target instanceof Element ? event.target.closest(".edit-handle") : null;
    if (!mode) return;
    event.preventDefault();
    event.stopPropagation();
    selected = name;
    const rect = stage.toWindow(stage.rectOf(name));
    drag = {
      name,
      corner: handle?.dataset.corner || "",
      startX: event.clientX,
      startY: event.clientY,
      rect,
      pointerId: event.pointerId,
      element: event.currentTarget,
      moved: false,
    };
    drag.element.setPointerCapture(event.pointerId);
    drag.element.addEventListener("pointermove", onDragMove);
    drag.element.addEventListener("pointerup", onDragEnd);
    drag.element.addEventListener("pointercancel", onDragEnd);
    render();
  }

  function draggedRect(current, event) {
    const dx = event.clientX - current.startX;
    const dy = event.clientY - current.startY;
    const base = current.rect;
    if (!current.corner) return { ...base, left: base.left + dx, top: base.top + dy };
    const { fx, fy } = anchorFactors(current.corner);
    // 押した角の反対側を固定する。
    const fixedX = fx === 0 ? base.left + base.width : base.left;
    const fixedY = fy === 0 ? base.top + base.height : base.top;
    let width = Math.max(MIN_SIZE, fx === 0 ? base.width - dx : base.width + dx);
    let height = Math.max(MIN_SIZE, fy === 0 ? base.height - dy : base.height + dy);
    if (current.name === "character") {
      // キャラは縦横比を保つ（構図の比率のまま拡大縮小し、どの大きさでも切れない）。
      const aspect = base.width / base.height;
      height = Math.min(Math.max(width / aspect, height), maxCharacterHeight());
      height = Math.max(MIN_SIZE, height);
      width = height * aspect;
    }
    return {
      left: fx === 0 ? fixedX - width : fixedX,
      top: fy === 0 ? fixedY - height : fixedY,
      width,
      height,
    };
  }

  function onDragMove(event) {
    if (!drag || event.pointerId !== drag.pointerId) return;
    drag.moved = true;
    stage.preview(drag.name, draggedRect(drag, event));
  }

  async function onDragEnd(event) {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const current = drag;
    drag = null;
    current.element.removeEventListener("pointermove", onDragMove);
    current.element.removeEventListener("pointerup", onDragEnd);
    current.element.removeEventListener("pointercancel", onDragEnd);
    if (current.element.hasPointerCapture(event.pointerId)) current.element.releasePointerCapture(event.pointerId);
    if (!current.moved || event.type === "pointercancel") {
      stage.cancelPreview(current.name);
      return;
    }
    await commit(current.name, draggedRect(current, event));
  }

  async function commit(name, windowRect) {
    try { await stage.commit(name, windowRect); }
    catch { /* 保存失敗は stage がトーストで知らせる */ }
    render();
  }

  function reset(name) {
    if (name === "character") stage.resetCharacter().catch(() => {});
    else stage.commit(name, null).catch(() => {});
  }

  /** 配置の編集中だけ、ホイールで中心を保って拡大縮小する。 */
  function wheelResize(event, name) {
    if (!mode) return;
    event.preventDefault();
    const rect = stage.toWindow(stage.rectOf(name));
    const factor = Math.exp(-Math.sign(event.deltaY) * 0.06);
    let height = Math.max(MIN_SIZE, rect.height * factor);
    if (name === "character") height = Math.min(height, maxCharacterHeight());
    const width = name === "character" ? height * (rect.width / rect.height) : Math.max(MIN_SIZE, rect.width * factor);
    const next = { left: rect.left + (rect.width - width) / 2, top: rect.top + rect.height - height, width, height };
    stage.preview(name, next);
    clearTimeout(wheelTimer);
    wheelTarget = name;
    wheelTimer = setTimeout(() => { wheelTimer = 0; wheelTarget = ""; commit(name, next); }, 350);
  }

  function nudge(dx, dy) {
    const rect = stage.toWindow(stage.rectOf(selected));
    commit(selected, { ...rect, left: rect.left + dx, top: rect.top + dy });
  }

  // ── ツールバー ───────────────────────────────────────────
  const title = el("strong");
  const sourceList = el("div", { className: "edit-sources" });
  const framingGroup = el("div", { className: "edit-framing" });
  const resetButton = el("button", { type: "button", className: "chip" });
  const doneButton = el("button", { type: "button", className: "chip primary", textContent: "完了" });
  toolbar.replaceChildren(title, sourceList, framingGroup, resetButton, doneButton);
  for (const [name, label] of [["full", "全身"], ["upper", "上半身"], ["bust", "胸上"]]) {
    const button = el("button", { type: "button", className: "chip", textContent: label, dataset: { framing: name } });
    button.addEventListener("click", () => stage.setFraming(name).then(render).catch(() => {}));
    framingGroup.append(button);
  }
  resetButton.addEventListener("click", () => {
    if (mode === "all") stage.resetLayout().then(render).catch(() => {});
    else reset("character");
  });
  doneButton.addEventListener("click", () => editor.stop());

  function renderToolbar() {
    title.textContent = mode === "all" ? "配置を編集中" : "キャラの位置とサイズ";
    resetButton.textContent = mode === "all" ? "この形式を既定へ戻す" : "自動配置へ戻す";
    sourceList.hidden = mode !== "all";
    framingGroup.hidden = mode === "";
    const current = stage.currentFraming();
    for (const button of framingGroup.children) button.setAttribute("aria-pressed", String(button.dataset.framing === current));
    if (mode === "all") {
      sourceList.replaceChildren(...stage.sourceNames().map(name => {
        const placement = stage.placementOf(name);
        const box = el("input", { type: "checkbox", checked: placement?.visible !== false });
        box.addEventListener("change", () => stage.setVisible(name, box.checked).then(render).catch(() => {}));
        return el("label", { className: "edit-source" }, [box, SOURCE_LABELS[name] || name]);
      }));
    }
  }

  document.addEventListener("keydown", event => {
    if (!mode || event.isComposing) return;
    if (event.key === "Escape") { event.preventDefault(); editor.stop(); return; }
    const step = event.shiftKey ? 10 : 1;
    const moves = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
    if (moves[event.key] && !(event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement)) {
      event.preventDefault();
      nudge(...moves[event.key]);
    }
  }, true);
  // 枠の外（何も無い所）をクリックしたら調整を終える。
  document.addEventListener("pointerdown", event => {
    if (mode !== "character" || drag) return;
    if (event.target instanceof Element && event.target.closest(".edit-frame,.edit-toolbar,.top-window-menu")) return;
    editor.stop();
  }, true);

  const editor = {
    get active() { return mode !== ""; },
    get mode() { return mode; },
    start(nextMode = "all") {
      if (mode) editor.stop();
      mode = nextMode === "character" ? "character" : "all";
      selected = "character";
      document.body.classList.add("is-editing");
      document.body.classList.toggle("is-editing-all", mode === "all");
      renderToolbar();
      render();
      onModeChange?.(mode);
      if (mode === "character") toast?.("つまみで大きさ、中央のつまみで位置を変えられます（Escで完了）", "info", { duration: 3200 });
    },
    stop() {
      if (!mode) return;
      if (drag) {
        const current = drag;
        drag = null;
        current.element.removeEventListener("pointermove", onDragMove);
        current.element.removeEventListener("pointerup", onDragEnd);
        current.element.removeEventListener("pointercancel", onDragEnd);
        if (current.element.hasPointerCapture(current.pointerId)) current.element.releasePointerCapture(current.pointerId);
        stage.cancelPreview(current.name);
      }
      clearTimeout(wheelTimer);
      if (wheelTarget) stage.cancelPreview(wheelTarget);
      wheelTimer = 0;
      wheelTarget = "";
      mode = "";
      document.body.classList.remove("is-editing");
      document.body.classList.remove("is-editing-all");
      render();
      onModeChange?.("");
    },
    /** 会話パネルのつまみ: アンカーの反対側の角で大きさを変える。 */
    attachGrip(name, grip) {
      grip.addEventListener("pointerdown", event => {
        if (event.button !== 0 || mode !== "all") return;
        const placement = stage.placementOf(name);
        const { fx, fy } = anchorFactors(placement?.anchor || "top-left");
        const corner = `${fy === 1 ? "top" : "bottom"}-${fx === 1 ? "left" : "right"}`;
        event.preventDefault();
        event.stopPropagation();
        drag = {
          name, corner, startX: event.clientX, startY: event.clientY,
          rect: stage.toWindow(stage.rectOf(name)), pointerId: event.pointerId, element: grip, moved: false,
        };
        grip.setPointerCapture(event.pointerId);
        grip.addEventListener("pointermove", onDragMove);
        grip.addEventListener("pointerup", onDragEnd);
        grip.addEventListener("pointercancel", onDragEnd);
      });
      stage.events.on("layout", () => {
        const placement = stage.placementOf(name);
        const { fx, fy } = anchorFactors(placement?.anchor || "top-left");
        grip.dataset.corner = `${fy === 1 ? "top" : "bottom"}-${fx === 1 ? "left" : "right"}`;
      });
    },
    wheel: wheelResize,
    render() {
      if (mode) renderToolbar();
      render();
    },
  };
  stage.events.on("layout", render);
  stage.events.on("layout-change", () => editor.render());
  return editor;
}
