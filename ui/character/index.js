// index.js - キャラクターの描画器を選び、状態（モーション・口・視線・居眠り）を渡す持ち主。
//
// 描画方式の登録表: 上から順に `supports(pack)` を満たした描画器を使う。方式を足すときは
// `ui/character/<方式>.js` に `CharacterRenderer` を実装してここへ1行足す。方式名で分岐を書かない。
// 描画器の約束（どれも同じ）:
//   load(pack) / element / framingRect(name) / defaultFraming() / sourcePixelsPerUnit /
//   resize(cssW, cssH, pixelRatio) / setFraming / setMotion / setMouth / setSpeaking / setGaze /
//   setSleeping / setActive / setReducedMotion / dispose

import { ImageRenderer } from "./image.js";
import { Live2dRenderer } from "./live2d.js";
import { loadPack, packUrl, personaText } from "./manifest.js";
import { MeshRenderer } from "./mesh.js";
import { MmdRenderer } from "./mmd.js";
import { aspectOf, resolveFraming } from "./framing.js";
import * as bridge from "../app/bridge.js";
import { createEmitter, el, errorDetail, errorMessage, logApp } from "../app/util.js";

export const RENDERERS = [MmdRenderer, Live2dRenderer, MeshRenderer, ImageRenderer];

const GPU_FAILURE_PATTERN = /webgl|shader|gpu|context ?lost|getcontext|シェーダ/i;

export function createCharacterHost({ container }) {
  const events = createEmitter();
  const reduceMotionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
  let reducedMotion = reduceMotionQuery.matches;
  let renderer = null;
  let pack = null;
  let generation = 0;
  let motion = "idle";
  let speaking = false;
  let active = true;
  let layoutId = "pet";
  let framing = "bust";
  let size = null;
  let gazeTracking = false;
  let sleeping = false;
  let gpuResetRequested = false;
  const sleepImage = el("img", { className: "character-sleep", alt: "", draggable: false, hidden: true });
  container.append(sleepImage);

  function reportGpuFailure(reason, force = false) {
    const text = errorMessage(reason);
    if (gpuResetRequested || (!force && !GPU_FAILURE_PATTERN.test(text))) return;
    gpuResetRequested = true;
    bridge.requestGpuCacheReset(text).catch(error => {
      gpuResetRequested = false;
      logApp("warn", "描画キャッシュ削除を予約できませんでした", errorDetail(error));
    });
  }
  // 描画中に GPU が描画面を失った（ドライバー更新・壊れたキャッシュ等）。次回起動で描画キャッシュを消す。
  document.addEventListener("webglcontextlost", event => {
    reportGpuFailure("WebGL context lost", true);
    logApp("warn", "GPU描画の描画面が失われました", event.target?.id || "canvas");
  }, true);

  async function createRenderer(Renderer, loadedPack) {
    const instance = new Renderer({ reducedMotion });
    container.prepend(instance.element);
    try {
      await instance.load(loadedPack);
      return instance;
    } catch (error) {
      instance.dispose();
      throw error;
    }
  }

  function applyState() {
    if (!renderer) return;
    renderer.setLayoutId?.(layoutId);
    renderer.setFraming(framing);
    renderer.setMotion(motion);
    renderer.setSpeaking(speaking);
    renderer.setActive(active && !sleeping && !document.hidden);
    if (size) renderer.resize(size.width, size.height, size.pixelRatio);
  }

  const host = {
    events,
    get pack() { return pack; },
    get kind() { return renderer?.constructor.kind || ""; },
    get renderer() { return renderer; },
    get motion() { return motion; },
    get reducedMotion() { return reducedMotion; },

    /**
     * キャラを読み込む。窓は隠したまま読み込むので、読込中の姿は見せない。
     * 表示の準備（描画器の load）が終わってから古い描画器と入れ替える。
     */
    async load(characterId, { overrides = {}, sdkOverrides = {} } = {}) {
      const loadGeneration = ++generation;
      const loaded = await loadPack(characterId, { sdkPreference: sdkOverrides[characterId] });
      if (loadGeneration !== generation) return null;
      const candidates = RENDERERS.filter(Renderer => Renderer.supports(loaded));
      let next = null;
      let lastError = null;
      for (const Renderer of candidates) {
        try {
          next = await createRenderer(Renderer, loaded);
          break;
        } catch (error) {
          lastError = error;
          reportGpuFailure(error);
          // フォールバック許可: Live2D ランタイム未取得・メッシュの WebGL 失敗でも、フレーム画像を持つ
          // キャラは2D表示で続ける。理由はログと状態行へ出す。
          logApp("error", `${Renderer.kind} の描画を初期化できませんでした（次の方式を試します）`, errorDetail(error));
          events.emit("fallback", { kind: Renderer.kind, error });
        }
        if (loadGeneration !== generation) return null;
      }
      if (loadGeneration !== generation) {
        next?.dispose();
        return null;
      }
      if (!next) throw lastError || new Error("このキャラクターを表示できる描画方式がありません");
      renderer?.dispose();
      renderer = next;
      pack = loaded;
      sleeping = false;
      sleepImage.hidden = true;
      container.dataset.renderKind = renderer.constructor.kind;
      events.emit("loaded", pack);
      applyState();
      return {
        pack,
        profile: personaText(loaded.manifest, overrides[loaded.id]),
        greeting: loaded.manifest.greeting || "",
      };
    },

    /** 構図の縦横比（幅/高さ）。読み込み前は null。 */
    framingAspect(name) {
      return renderer ? aspectOf(renderer.framingRect(name)) : null;
    },
    resolveFraming(stored, sceneDefault) {
      return resolveFraming(stored, sceneDefault, pack, renderer?.defaultFraming());
    },
    /** この構図で原画を2倍以上に引き伸ばさない高さの上限（CSS px）。 */
    maxHeight(name, pixelRatio = window.devicePixelRatio || 1) {
      if (!renderer) return Infinity;
      const rect = renderer.framingRect(name);
      return (rect.h * renderer.sourcePixelsPerUnit * 2) / Math.max(1, pixelRatio);
    },

    resize(width, height, pixelRatio) {
      size = { width, height, pixelRatio };
      renderer?.resize(width, height, pixelRatio);
    },
    setLayout(id, framingName) {
      layoutId = id;
      framing = framingName;
      renderer?.setLayoutId?.(id);
      renderer?.setFraming(framingName);
    },
    setFraming(name) {
      framing = name;
      renderer?.setFraming(name);
    },

    setMotion(name) {
      motion = String(name || "idle");
      if (sleeping && motion !== "idle") host.setSleeping(false);
      renderer?.setMotion(motion);
      container.dataset.motion = motion;
    },
    setSpeaking(value) {
      speaking = Boolean(value);
      renderer?.setSpeaking(speaking);
    },
    setMouth(level, form = 0) {
      if (speaking) renderer?.setMouth(level, form);
    },
    setGaze(x, y) {
      if (!gazeTracking || reducedMotion) return;
      renderer?.setGaze(x, y);
      scheduleGlance(); // カーソル追従中は自動よそ見を先送りする
    },
    setGazeTracking(enabled) {
      gazeTracking = enabled === true;
      renderer?.setGaze(0, 0);
      scheduleGlance();
    },
    setActive(value) {
      active = Boolean(value);
      renderer?.setActive(active && !sleeping && !document.hidden);
    },
    /** 居眠り。寝姿の画像（boot.sleeping）があるキャラだけ眠る。 */
    setSleeping(value) {
      const image = pack?.manifest?.boot?.sleeping;
      const next = Boolean(value) && Boolean(image) && host.kind !== "live2d" && host.kind !== "mmd";
      if (next === sleeping) return sleeping;
      sleeping = next;
      if (sleeping) {
        sleepImage.src = packUrl(pack, image);
        sleepImage.hidden = false;
        renderer?.element.setAttribute("hidden", "");
      } else {
        sleepImage.hidden = true;
        renderer?.element.removeAttribute("hidden");
      }
      renderer?.setActive(active && !sleeping && !document.hidden);
      return sleeping;
    },
    get sleeping() { return sleeping; },
    setPose(pose) { renderer?.setPose?.(pose); },
    capturePreview() {
      try { renderer?.capturePreview?.(); }
      catch (error) { logApp("warn", "表示前の描画確定に失敗しました", errorDetail(error)); }
    },
    reportGpuFailure,
  };

  // 追従が有効でカーソルが来ない間は、ときどき一瞬よそ見をする（カーソル追従中は先送り）。
  let glanceTimer = 0;
  function scheduleGlance() {
    clearTimeout(glanceTimer);
    if (reducedMotion || !gazeTracking) return;
    glanceTimer = setTimeout(() => {
      if (reducedMotion || !gazeTracking || !renderer) return;
      if (speaking || motion !== "idle") { scheduleGlance(); return; }
      renderer.setGaze(Math.random() < 0.5 ? -1 : 1, 0);
      glanceTimer = setTimeout(() => {
        renderer?.setGaze(0, 0);
        scheduleGlance();
      }, 350 + Math.random() * 300);
    }, 8000 + Math.random() * 6000);
  }

  reduceMotionQuery.addEventListener("change", event => {
    reducedMotion = event.matches;
    scheduleGlance();
    renderer?.setReducedMotion(reducedMotion);
    events.emit("reduced-motion", reducedMotion);
  });
  document.addEventListener("visibilitychange", () => {
    renderer?.setActive(active && !sleeping && !document.hidden);
  });
  return host;
}
