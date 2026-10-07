// stage.js - シーンの持ち主。どの形式で・どのソースを・どこへ置くかを決めて DOM へ当てる。
//
// 座標系は1つだけ。`pet` / `mascot` はウィンドウの論理px＝シーン座標、`stream` は固定寸法のシーンを
// ウィンドウへ等倍率で収める（ここの `transform` がソースに掛かる唯一の拡大）。各ソースへは
// `left` / `top` / `width` / `height` だけを当て、キャラの描画器にはその矩形と実効の画素密度を渡す。
// 保存値（利用者が動かした差分）は `PICOAGENT_SCENES` の写しとしてここが持ち、確定時だけ保存する。

import * as bridge from "./bridge.js";
import {
  cloneScenes, containRect, fitScene, fromRect, keepReachable, nearestAnchor,
  resolvePlacement, toRect, writeSaved,
} from "./scene-model.js";
import { clamp, createEmitter, errorDetail, errorMessage, logApp } from "./util.js";

const CHAT_MARGIN = 6;
const EXPANDED_MARGIN = 14;
const GAME_FRAMING = "bust"; // ゲーム中はどの形式でも胸上（ゲーム画面の右側へバストアップで出す）

export function createStage({ stageElement, sceneRoot, sources, character, toast }) {
  const events = createEmitter();
  let definitions = {};
  let saved = { version: 1 };
  let layoutId = "pet";
  let characterId = "";
  const overrides = { expanded: false, game: false };
  /** ドラッグ中だけの一時的な矩形（シーン座標）。確定すると保存値へ移す。 */
  const previews = new Map();
  let frame = 0;
  let lastLayout = null;
  let saveQueue = Promise.resolve();

  const definition = () => definitions[layoutId] || definitions.pet || { sources: {}, order: [] };
  const fixedSize = def => (def.size && typeof def.size === "object" ? def.size : null);

  function viewSize() {
    return { width: Math.max(1, stageElement.clientWidth), height: Math.max(1, stageElement.clientHeight) };
  }

  /** いまのシーンの寸法と、窓へ写す倍率・余白。 */
  function sceneGeometry() {
    const view = viewSize();
    const fixed = fixedSize(definition());
    if (!fixed) return { width: view.width, height: view.height, scale: 1, offsetX: 0, offsetY: 0, fixed: false };
    return { width: fixed.w, height: fixed.h, ...fitScene(fixed.w, fixed.h, view.width, view.height), fixed: true };
  }

  function savedFor(target) {
    const layout = saved[layoutId] || {};
    return target === "character"
      ? layout.characters?.[characterId]
      : layout.sources?.[target];
  }

  /** 保存された（無ければ既定の）構図。配置の保存と縦横比はこちらを使う。 */
  function savedFraming() {
    const stored = saved[layoutId]?.characters?.[characterId]?.framing;
    return character.resolveFraming(stored, definition().defaultFraming);
  }

  /** いま描く構図。ゲーム中はどの形式でも胸上にする（保存値は変えず、閉じると元へ戻る）。 */
  function currentFraming() {
    return overrides.game ? GAME_FRAMING : savedFraming();
  }

  /** 1つのソースの配置（アンカー基準）。ドラッグ中はその一時値を使う。 */
  function placementOf(target, geometry = sceneGeometry()) {
    const def = definition().sources?.[target];
    if (!def) return null;
    const aspect = target === "character" ? character.framingAspect(savedFraming()) : null;
    const base = resolvePlacement(def, savedFor(target), geometry.width, geometry.height, aspect);
    const preview = previews.get(target);
    return preview ? { ...base, ...preview } : base;
  }

  /** 画面へ当てる矩形（シーン座標）。窓内へ収める規則もここで適用する。 */
  function rectOf(target, geometry = sceneGeometry()) {
    const placement = placementOf(target, geometry);
    if (!placement) return null;
    let rect = toRect(placement, geometry.width, geometry.height);
    if (overrides.game && target === "character") rect = gameCharacterRect(geometry);
    else if (overrides.expanded && target === "chat" && !geometry.fixed) rect = expandedChatRect(geometry);
    else if (!geometry.fixed && target === "chat") rect = containRect(rect, geometry.width, geometry.height, CHAT_MARGIN);
    else if (target === "character" && !previews.has(target)) rect = keepReachable(rect, geometry.width, geometry.height);
    return { ...rect, visible: placement.visible };
  }

  /** 拡大中の会話パネル: キャラ矩形を避けた残りの領域いっぱい（保存値は変えない）。 */
  function expandedChatRect(geometry) {
    const characterRect = toRect(placementOf("character", geometry), geometry.width, geometry.height);
    const right = clamp(characterRect.left - EXPANDED_MARGIN, 180 + EXPANDED_MARGIN, geometry.width - EXPANDED_MARGIN);
    return {
      left: EXPANDED_MARGIN,
      top: EXPANDED_MARGIN,
      width: right - EXPANDED_MARGIN,
      height: geometry.height - EXPANDED_MARGIN * 2,
    };
  }

  /**
   * ゲーム中のキャラ: ゲーム画面の右側（窓座標で決めてシーン座標へ戻す）。保存値は使わない。
   * 幅と右余白は components/game.css の採点欄（.game-shell）が空ける量と同じ式にする。
   */
  function gameCharacterRect(geometry) {
    const view = viewSize();
    const aspect = character.framingAspect(GAME_FRAMING) || 0.8;
    const width = clamp(view.width * 0.32, 210, 340);
    const height = Math.min(view.height * 0.86, width / aspect);
    const windowRect = {
      left: view.width - clamp(view.width * 0.07, 36, 90) - height * aspect,
      top: view.height - height,
      width: height * aspect,
      height,
    };
    return toScene(windowRect, geometry);
  }

  /** 窓座標の矩形 → シーン座標。 */
  function toScene(rect, geometry = sceneGeometry()) {
    return {
      left: (rect.left - geometry.offsetX) / geometry.scale,
      top: (rect.top - geometry.offsetY) / geometry.scale,
      width: rect.width / geometry.scale,
      height: rect.height / geometry.scale,
    };
  }

  /** シーン座標の矩形 → 窓座標。 */
  function toWindow(rect, geometry = sceneGeometry()) {
    return {
      left: rect.left * geometry.scale + geometry.offsetX,
      top: rect.top * geometry.scale + geometry.offsetY,
      width: rect.width * geometry.scale,
      height: rect.height * geometry.scale,
    };
  }

  function applyLayout() {
    frame = 0;
    const def = definition();
    const geometry = sceneGeometry();
    const appElement = stageElement.closest(".app");
    // タイトルバーは見えているシーンの真上へ同じ幅で置く（固定シーンの余白へ伸ばさない・離さない）。
    if (appElement) {
      appElement.style.setProperty("--scene-view-left", `${geometry.offsetX}px`);
      appElement.style.setProperty("--scene-view-top", `${geometry.offsetY}px`);
      appElement.style.setProperty("--scene-view-width", geometry.fixed ? `${geometry.width * geometry.scale}px` : "100%");
    }
    sceneRoot.style.width = `${geometry.width}px`;
    sceneRoot.style.height = `${geometry.height}px`;
    sceneRoot.style.transform = geometry.fixed
      ? `translate(${geometry.offsetX}px, ${geometry.offsetY}px) scale(${geometry.scale})`
      : "";
    const order = def.order || Object.keys(def.sources || {});
    const placed = {};
    for (const [name, source] of sources.entries()) {
      const index = order.indexOf(name);
      const rect = index >= 0 ? rectOf(name, geometry) : null;
      const visible = Boolean(rect && rect.visible !== false && !(overrides.game && name !== "character"));
      source.element.hidden = !visible;
      source.setVisible?.(visible);
      if (!visible) continue;
      Object.assign(source.element.style, {
        left: `${rect.left}px`,
        top: `${rect.top}px`,
        width: `${rect.width}px`,
        height: `${rect.height}px`,
        zIndex: String(10 + index),
      });
      placed[name] = rect;
      source.update?.({
        layoutId,
        rect,
        options: def.sources[name]?.options || {},
        pixelRatio: (window.devicePixelRatio || 1) * geometry.scale,
        expanded: overrides.expanded,
      });
    }
    stageElement.classList.toggle("is-game", overrides.game);
    lastLayout = { geometry, placed };
    events.emit("layout", lastLayout);
  }

  function schedule() {
    if (frame) return;
    frame = requestAnimationFrame(applyLayout);
  }

  function persist() {
    const snapshot = cloneScenes(saved);
    const operation = saveQueue.catch(() => undefined).then(() => bridge.saveScene(snapshot));
    saveQueue = operation;
    return operation.catch(error => {
      logApp("error", "画面配置を保存できませんでした", errorDetail(error));
      toast?.(`画面配置を保存できませんでした: ${errorMessage(error)}`, "error");
      throw error;
    });
  }

  /** キャラを窓からはみ出す大きさにしたら、画面上の矩形を固定して窓を広げる（`pet` / `mascot`）。 */
  async function growWindowFor(target) {
    const geometry = sceneGeometry();
    if (geometry.fixed || target !== "character") return;
    const placement = placementOf("character", geometry);
    const rect = toRect(placement, geometry.width, geometry.height);
    const growingLayout = layoutId;
    const growingCharacter = characterId;
    const previous = savedFor("character");
    const overflowLeft = Math.max(0, -rect.left);
    const overflowTop = Math.max(0, -rect.top);
    const overflowRight = Math.max(0, rect.left + rect.width - geometry.width);
    const overflowBottom = Math.max(0, rect.top + rect.height - geometry.height);
    if (!(overflowLeft || overflowTop || overflowRight || overflowBottom)) return;
    // 固定タイトルバーが占める分も窓寸法へ含める。シーン座標と窓座標はいずれも論理px。
    const chromeWidth = Math.max(0, (window.innerWidth || geometry.width) - geometry.width);
    const chromeHeight = Math.max(0, (window.innerHeight || geometry.height) - geometry.height);
    const currentWidth = geometry.width + chromeWidth;
    const currentHeight = geometry.height + chromeHeight;
    const maxWidth = Math.max(currentWidth, window.screen?.availWidth || currentWidth);
    const maxHeight = Math.max(currentHeight, window.screen?.availHeight || currentHeight);
    const width = Math.min(maxWidth, Math.ceil(currentWidth + overflowLeft + overflowRight));
    const height = Math.min(maxHeight, Math.ceil(currentHeight + overflowTop + overflowBottom));
    const leftDelta = Math.ceil(overflowLeft);
    const topDelta = Math.ceil(overflowTop);
    try {
      if (overflowLeft || overflowTop) {
        await bridge.moveAndResizeWindow(leftDelta, topDelta, width, height);
      } else {
        await bridge.setWindowSize(width, height);
      }
      // 拡張した辺へアンカーが追従して再びはみ出さないよう、元の画面上の位置を保存し直す。
      // 他ソースの保存値は維持する。待機中に別キャラ・形式・配置へ変わった場合も上書きしない。
      if (layoutId !== growingLayout || characterId !== growingCharacter || savedFor("character") !== previous) return;
      const value = {
        ...previous,
        ...fromRect({ ...rect, left: rect.left + leftDelta, top: rect.top + topDelta },
          placement.anchor, width - chromeWidth, height - chromeHeight),
      };
      value.framing ||= savedFraming();
      writeSaved(saved, growingLayout, { characterId: growingCharacter }, value);
      applyLayout();
    } catch (error) {
      logApp("warn", "キャラの大きさに合わせてウィンドウを広げられませんでした", errorDetail(error));
    }
  }

  const stage = {
    events,
    get layoutId() { return layoutId; },
    get characterId() { return characterId; },
    get scenes() { return cloneScenes(saved); },
    get lastLayout() { return lastLayout; },
    definitionOf: id => definitions[id],
    sources,

    load(defs, scenes) {
      definitions = defs;
      saved = cloneScenes(scenes);
      schedule();
    },
    setLayout(id) {
      if (!definitions[id]) {
        logApp("warn", "未知の表示形式のため既定のペットで表示します", String(id));
        id = "pet";
      }
      if (layoutId === id) { schedule(); return; }
      events.emit("context-change");
      previews.clear();
      layoutId = id;
      overrides.expanded = false;
      events.emit("layout-change", id);
      schedule();
    },
    setCharacter(id) {
      if (characterId !== String(id || "")) events.emit("context-change");
      characterId = String(id || "");
      previews.clear();
      schedule();
    },
    currentFraming,
    placementOf: target => placementOf(target),
    rectOf: target => rectOf(target),
    toWindow: rect => toWindow(rect),
    toScene: rect => toScene(rect),
    geometry: () => sceneGeometry(),
    /** この形式に置かれているソース名（重ね順）。 */
    sourceNames: () => [...(definition().order || [])].filter(name => definition().sources?.[name]),
    layout: schedule,
    layoutNow: applyLayout,

    /** ドラッグ中の一時的な矩形（窓座標）。保存しない。 */
    preview(target, windowRect) {
      const geometry = sceneGeometry();
      const rect = toScene(windowRect, geometry);
      const anchor = placementOf(target, geometry)?.anchor || "top-left";
      previews.set(target, fromRect(rect, anchor, geometry.width, geometry.height));
      schedule();
    },
    cancelPreview(target) {
      previews.delete(target);
      schedule();
    },
    /**
     * 矩形（窓座標）を確定して保存する。アンカーは矩形の位置から選び直し、窓の伸縮に付いていく辺を決める。
     * `windowRect` が null なら、そのソースの保存値を捨てて既定へ戻す。
     */
    async commit(target, windowRect, extra = {}) {
      previews.delete(target);
      const geometry = sceneGeometry();
      const key = target === "character" ? { characterId } : { source: target };
      if (target === "character" && !characterId) return;
      if (windowRect == null) {
        const keepFraming = target === "character" && extra.keepFraming
          ? saved[layoutId]?.characters?.[characterId]?.framing : null;
        writeSaved(saved, layoutId, key, keepFraming ? { framing: keepFraming } : null);
      } else {
        const rect = toScene(windowRect, geometry);
        const anchor = nearestAnchor(rect, geometry.width, geometry.height);
        const previous = target === "character"
          ? saved[layoutId]?.characters?.[characterId]
          : saved[layoutId]?.sources?.[target];
        const value = { ...fromRect(rect, anchor, geometry.width, geometry.height) };
        if (typeof previous?.visible === "boolean") value.visible = previous.visible;
        if (target === "character") value.framing = extra.framing || previous?.framing || savedFraming();
        writeSaved(saved, layoutId, key, value);
      }
      applyLayout();
      await growWindowFor(target);
      return persist();
    },
    /** ソースの表示・非表示だけを切り替えて保存する。 */
    async setVisible(target, visible) {
      if (target === "character" && !characterId) return;
      const previous = savedFor(target) || {};
      const key = target === "character" ? { characterId } : { source: target };
      writeSaved(saved, layoutId, key, { ...previous, visible: Boolean(visible) });
      applyLayout();
      return persist();
    },
    /** 構図を切り替える。高さを保ち、幅を新しい構図の縦横比へ合わせる。 */
    async setFraming(name) {
      if (!characterId) return;
      const geometry = sceneGeometry();
      const placement = placementOf("character", geometry);
      const aspect = character.framingAspect(name);
      if (!placement || !aspect) return;
      const rect = toRect(placement, geometry.width, geometry.height);
      const width = rect.height * aspect;
      const next = { ...rect, left: rect.left + (rect.width - width) / 2, width };
      return stage.commit("character", toWindow(next, geometry), { framing: name });
    },
    /** キャラを自動配置へ戻す（この形式・このキャラの保存値を捨てる）。 */
    resetCharacter() {
      return stage.commit("character", null);
    },
    /** この形式の保存値をすべて捨てる（配置編集の「既定へ戻す」）。 */
    async resetLayout() {
      delete saved[layoutId];
      previews.clear();
      applyLayout();
      return persist();
    },
    /** バックエンドが保存値を書き換えた場合（位置とサイズのリセット）に写しを置き換える。 */
    replaceScenes(scenes) {
      events.emit("context-reset");
      saved = cloneScenes(scenes);
      previews.clear();
      schedule();
    },
    setExpanded(on) {
      overrides.expanded = Boolean(on);
      schedule();
    },
    get expanded() { return overrides.expanded; },
    setGame(on) {
      overrides.game = Boolean(on);
      applyLayout();
    },
    get playingGame() { return overrides.game; },
  };

  new ResizeObserver(schedule).observe(stageElement);
  window.matchMedia?.(`(resolution: ${window.devicePixelRatio || 1}dppx)`)?.addEventListener?.("change", schedule);
  return stage;
}
