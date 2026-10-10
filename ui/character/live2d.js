// live2d.js - Live2D（Cubism）の描画器。
//
// モデルはモデル座標（originalWidth × originalHeight）へ等倍で置き、構図の切り出し矩形をキャンバスへ
// 写す変換だけを PIXI のステージへ掛ける。モデル別の `scale` / `offsetX` / `offsetY` は、旧来の
// 285×315 の基準枠で上半身が収まるよう調整された値なので、その枠が写していたモデル座標の範囲へ
// 換算して `upper` の切り出し矩形として使う（既存の定義をそのまま読めるようにする）。

import { createTransparentApp, ensureLive2d } from "./runtime.js";
import { packUrl } from "./manifest.js";
import { backingResolution, defaultFramingFor, fitTransform } from "./framing.js";
import { el, errorDetail, logApp } from "../app/util.js";

const MODEL_TIMEOUT_MS = 8000;
/** 旧来の調整値が前提にしていた基準枠。ここは換算のためだけに使い、描画寸法には使わない。 */
const LEGACY_FRAME = Object.freeze({ width: 285, height: 315 });

function loadModel(PIXI, url) {
  const pending = PIXI.live2d.Live2DModel.from(url, { autoInteract: false, autoUpdate: false });
  return new Promise((resolve, reject) => {
    let finished = false;
    const timer = setTimeout(() => {
      finished = true;
      reject(new Error(`Live2Dモデルが${MODEL_TIMEOUT_MS}ミリ秒以内に応答しませんでした: ${url}`));
    }, MODEL_TIMEOUT_MS);
    pending.then(model => {
      if (finished) {
        try { model?.destroy?.(); }
        catch (error) { logApp("warn", "遅れて完了したLive2Dモデルを破棄できませんでした", errorDetail(error)); }
        return;
      }
      finished = true;
      clearTimeout(timer);
      resolve(model);
    }, error => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      reject(error);
    });
  });
}

/** `models`（世代付きの候補）と `model`（単独）を、同じ形の候補一覧へそろえる。 */
function candidates(config) {
  const raw = Array.isArray(config.models) && config.models.length ? config.models : [config.model];
  return raw.flatMap(candidate => {
    if (typeof candidate === "string") {
      const match = candidate.match(/(?:^|\/)sdk([45])(?:\/|$)/i);
      return candidate ? [{ path: candidate, sdk: match?.[1] || "" }] : [];
    }
    const path = String(candidate?.path || candidate?.model || "");
    return path ? [{ path, sdk: String(candidate?.sdk || "") }] : [];
  });
}

/** 旧来の基準枠で見えていたモデル座標の範囲（= 調整済みの上半身）。 */
function legacyFrameRect(frame, config, originalWidth, originalHeight) {
  const number = key => (Number.isFinite(Number(frame?.[key])) ? Number(frame[key]) : Number(config[key]) || 0);
  const fit = Math.min(LEGACY_FRAME.width / originalWidth, LEGACY_FRAME.height / originalHeight);
  const k = fit * (Number(frame?.scale) || Number(config.scale) || 1);
  const anchorX = LEGACY_FRAME.width / 2 + number("offsetX");
  const anchorY = LEGACY_FRAME.height + number("offsetY");
  const x0 = originalWidth / 2 + (0 - anchorX) / k;
  const x1 = originalWidth / 2 + (LEGACY_FRAME.width - anchorX) / k;
  const y0 = originalHeight + (0 - anchorY) / k;
  const y1 = originalHeight + (LEGACY_FRAME.height - anchorY) / k;
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

export class Live2dRenderer {
  static kind = "live2d";
  static supports(pack) {
    return Boolean(pack.live2d);
  }

  constructor({ reducedMotion = false } = {}) {
    this.canvas = el("canvas", { className: "character-canvas", id: "live2dCanvas" }, [], { "aria-hidden": "true" });
    this.element = this.canvas;
    this.reducedMotion = reducedMotion;
    this.mouth = -1;
    this.active = false;
    this.frameBudget = 0;
    this.disposed = false;
    this.ready = false;
    this.framing = "upper";
    this.layoutId = "pet";
  }

  async load(pack) {
    this.pack = pack;
    this.config = pack.live2d;
    const all = candidates(this.config);
    const sdk = pack.live2dSdk === "auto" ? (all[0]?.sdk === "5" ? "5" : "4") : pack.live2dSdk;
    // 1ページ内で Cubism Core は交換できないため、異なる世代へは無言で切り替えない。
    const matching = all.filter(candidate => (candidate.sdk === "5" ? "5" : "4") === sdk);
    if (!matching.length) throw new Error(`Cubism ${sdk} のモデルがこのキャラクターパックにありません`);
    const PIXI = await ensureLive2d(sdk);
    if (this.disposed) return;
    this.PIXI = PIXI;
    this.app = createTransparentApp(PIXI, this.canvas, 300, 300, window.devicePixelRatio || 1);
    this.budgetController = window.PicoFrameScheduler.createPixiBudget(this.app.ticker, () => {
      this.lastBlinkAt = performance.now();
      this.lastModelUpdateAt = this.lastBlinkAt;
    }, "Live2D");
    let lastError = null;
    for (const candidate of matching) {
      try {
        const model = await loadModel(PIXI, packUrl(pack, candidate.path));
        if (this.disposed) { model.destroy(); return; }
        this.model = model;
        break;
      } catch (error) {
        if (this.disposed) return;
        lastError = error;
        logApp("warn", `Live2Dモデル ${candidate.path} は現在のCubism Coreで読めません。次の互換モデルを試します`, errorDetail(error));
      }
    }
    if (!this.model) throw lastError || new Error("利用可能なLive2D互換モデルがありません");
    const model = this.model;
    const internal = model.internalModel;
    this.originalWidth = internal.originalWidth;
    this.originalHeight = internal.originalHeight;
    model.anchor.set(0, 0);
    model.scale.set(1);
    model.position.set(0, 0);
    // 表情・視線・物理・ポーズ更新の後、モデル描画の直前にリップシンク値を上書きする。
    // 口パラメータIDはモデル定義（Groups: LipSync）から自動検出する（旧モデルの命名にも追従する）。
    const lipGroup = internal.settings?.groups?.find(group => group.Name === "LipSync");
    const lipEnabled = this.config.lipSync !== false;
    const lipParam = String(this.config.lipSyncParam || lipGroup?.Ids?.[0] || "ParamMouthOpenY");
    internal.on("beforeModelUpdate", () => {
      if (!this.disposed && lipEnabled && this.mouth >= 0) internal.coreModel.setParameterValueById(lipParam, this.mouth);
    });
    // ランタイムは「モーション再生中は自動まばたきを回さない」。待機モーションをループさせ続けるため、
    // モーション適用後に自前で回してランダム間隔のまばたきを取り戻す。
    if (internal.eyeBlink) {
      this.lastBlinkAt = performance.now();
      internal.on("afterMotionUpdate", () => {
        if (this.disposed || !this.active || !this.frameBudget) return;
        const now = performance.now();
        const delta = Math.min((now - this.lastBlinkAt) / 1000, 0.5);
        this.lastBlinkAt = now;
        internal.eyeBlink.updateParameters(internal.coreModel, delta);
      });
    }
    this.motionEnabled = !this.reducedMotion || this.config.alwaysAnimate === true;
    if (!this.motionEnabled) internal.motionManager.groups.idle = "";
    else if (this.config.motions?.idle) internal.motionManager.groups.idle = this.config.motions.idle;
    // 同梱ランタイムの自動更新はTicker.sharedを使うため、appの予算内だけで進める。
    this.lastModelUpdateAt = performance.now();
    this.updateModel = () => {
      if (this.disposed || !this.ready || !this.active || !this.frameBudget) return;
      const now = performance.now();
      model.update(now - this.lastModelUpdateAt);
      this.lastModelUpdateAt = now;
    };
    this.app.ticker.add(this.updateModel, this, PIXI.UPDATE_PRIORITY.HIGH);
    this.app.stage.addChild(model);
    this.buildFramings();
    this.ready = true;
    this.budgetController.setActive(this.active);
    this.budgetController.setBudget(this.frameBudget);
    if (this.motionEnabled && this.config.motions?.idle) {
      model.motion(this.config.motions.idle, 0, PIXI.live2d?.MotionPriority?.IDLE ?? 1);
    }
  }

  buildFramings() {
    const full = { x: 0, y: 0, w: this.originalWidth, h: this.originalHeight };
    const frameFor = layoutId => {
      const frame = this.config.layoutFraming?.[layoutId] || this.config;
      const rect = legacyFrameRect(frame, this.config, this.originalWidth, this.originalHeight);
      return rect.w > 0 && rect.h > 0 ? rect : full;
    };
    this.frameFor = frameFor;
  }

  framingRect(name) {
    const full = { x: 0, y: 0, w: this.originalWidth, h: this.originalHeight };
    if (name === "full") return full;
    const upper = this.frameFor(this.layoutId);
    if (name === "bust") {
      const depth = this.config.layoutFraming?.[this.layoutId]?.bustDepth ?? this.config.bustDepth ?? 0.8;
      return { x: upper.x, y: upper.y, w: upper.w, h: upper.h * depth };
    }
    return upper;
  }

  naturalSize() { return { w: this.originalWidth, h: this.originalHeight }; }
  defaultFraming() { return defaultFramingFor(this.pack, "live2d"); }
  get sourcePixelsPerUnit() {
    // テクスチャの解像度はモデル座標の約2倍が一般的。拡大上限の目安として控えめに2を使う。
    return 2;
  }

  setLayoutId(layoutId) {
    this.layoutId = layoutId;
  }

  resize(cssWidth, cssHeight, pixelRatio) {
    if (!this.app) return;
    this.cssWidth = cssWidth;
    this.cssHeight = cssHeight;
    this.pixelRatio = backingResolution(cssWidth, cssHeight, pixelRatio);
    const renderer = this.app.renderer;
    if (Math.abs(renderer.resolution - this.pixelRatio) > 0.001) renderer.resolution = this.pixelRatio;
    renderer.resize(Math.max(1, Math.round(cssWidth)), Math.max(1, Math.round(cssHeight)));
    const fit = fitTransform(this.framingRect(this.framing), cssWidth, cssHeight);
    this.app.stage.scale.set(fit.scale);
    this.app.stage.position.set(fit.offsetX, fit.offsetY);
    // サイズ確定時は停止中でも最後の表示を一度だけ更新する。
    this.app.render();
  }

  setFraming(name) {
    this.framing = name;
    if (this.cssWidth) this.resize(this.cssWidth, this.cssHeight, this.pixelRatio);
  }

  /** 状態名（idle/thinking/talking/sad）を `live2d.motions` でグループへ写像して再生する。 */
  setMotion(name) {
    if (!this.model || !this.motionEnabled) return;
    const motions = this.config.motions || {};
    const group = motions[name] || motions.idle;
    if (!group) return;
    const priority = name === "idle"
      ? (this.PIXI.live2d?.MotionPriority?.IDLE ?? 1)
      : (this.PIXI.live2d?.MotionPriority?.FORCE ?? 3);
    this.model.motion(group, undefined, priority);
  }

  setMouth(level) {
    this.mouth = Math.max(0, Math.min(1, Number(level) || 0));
  }

  setSpeaking(speaking) {
    this.mouth = speaking ? Math.max(0, this.mouth) : -1;
  }

  /** 視線は正規化済みの内部コントローラへ弱く渡す（大きな値だと上下限へ張り付く）。 */
  setGaze(x, y) {
    const controller = this.model?.internalModel?.focusController;
    if (!controller) return;
    controller.focus(Math.max(-0.38, Math.min(0.38, x * 0.32)), Math.max(-0.2, Math.min(0.2, y * 0.18)));
  }

  setSleeping() {}

  setActive(active) {
    if (this.disposed) return;
    this.active = Boolean(active);
    this.budgetController?.setActive(this.active && this.ready);
  }

  setFrameBudget(fps) {
    if (this.disposed) return;
    this.frameBudget = fps;
    this.budgetController?.setBudget(fps);
  }

  setReducedMotion(reduced) {
    this.reducedMotion = Boolean(reduced);
    const always = this.config?.alwaysAnimate === true;
    this.motionEnabled = !this.reducedMotion || always;
    if (!this.motionEnabled) this.model?.internalModel?.motionManager?.stopAllMotions?.();
    else this.setMotion("idle");
  }

  dispose() {
    this.disposed = true;
    this.active = false;
    this.budgetController?.setActive(false);
    this.mouth = -1;
    try {
      if (this.updateModel) this.app?.ticker.remove(this.updateModel, this);
      if (this.model) {
        this.app?.stage?.removeChild(this.model);
        this.model.destroy();
      }
      this.app?.destroy(false, { children: true });
    } catch (error) {
      logApp("warn", "Live2Dの描画を解放できませんでした", errorDetail(error));
    }
    this.model = null;
    this.app = null;
    this.canvas.remove();
  }
}
