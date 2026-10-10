// runtime.js - 描画ランタイム（PIXI・Cubism Core・pixi-live2d-display）を必要になった時だけ読む。

import { errorMessage, loadScriptOnce } from "../app/util.js";

let pixiPromise = null;
let live2dPromise = null;
// ページへ挿入した Cubism Core の世代。script 要素を外してもグローバルは戻せないため、
// 「読み込みを試みた」時点で確定させ、以後は別世代を読ませない。
let live2dCoreSdk = "";

export async function ensurePixi() {
  if (window.PIXI?.Application && window.PIXI?.SimplePlane) return window.PIXI;
  if (!pixiPromise) {
    pixiPromise = loadScriptOnce("vendor/pixi/pixi.min.js?v=6.5.10-pinned").catch(error => {
      pixiPromise = null;
      throw new Error(`PIXIランタイムを読み込めません（cargo xtask setup pixi を実行してください）: ${errorMessage(error)}`);
    });
  }
  await pixiPromise;
  return window.PIXI;
}

/** Cubism Core → PIXI → pixi-live2d-display の順に1度だけ読む。SDK 5 は専用 Core、それ以外は互換 Core。 */
export async function ensureLive2d(sdk) {
  const requested = sdk === "5" ? "5" : "4";
  if (live2dCoreSdk && live2dCoreSdk !== requested) {
    throw new Error("Cubism Coreの世代を切り替えるにはPicoAgentの再起動が必要です");
  }
  if (window.PIXI?.live2d?.Live2DModel) return window.PIXI;
  if (!live2dPromise) {
    live2dPromise = (async () => {
      live2dCoreSdk = requested;
      await loadScriptOnce(requested === "5"
        ? "vendor/live2d/live2dcubismcore-5.3.min.js?v=5.3-pinned"
        : "vendor/live2d/live2dcubismcore-compat.min.js?v=compat-pinned");
      await ensurePixi();
      await loadScriptOnce("vendor/live2d/cubism4.min.js");
      // モデル同梱のモーション音声（サンプルボイス）は TTS 読み上げと二重再生になるため無効化する。
      if (window.PIXI?.live2d?.config) window.PIXI.live2d.config.sound = false;
    })().catch(error => {
      live2dPromise = null;
      throw new Error(`Live2D ランタイムを読み込めません（cargo xtask setup live2d を実行してください）: ${errorMessage(error)}`);
    });
  }
  await live2dPromise;
  return window.PIXI;
}

/** 透明な背景で WebGL の PIXI アプリを作る。 */
export function createTransparentApp(PIXI, canvas, width, height, resolution) {
  const options = { alpha: true, premultipliedAlpha: true, antialias: true };
  const context = canvas.getContext("webgl2", options) || canvas.getContext("webgl", options);
  if (!context) throw new Error("WebGLを初期化できません");
  const app = new PIXI.Application({
    view: canvas,
    context,
    width,
    height,
    transparent: true,
    useContextAlpha: true,
    backgroundAlpha: 0,
    antialias: true,
    resolution,
    autoDensity: true,
    autoStart: false,
  });
  canvas.style.background = "transparent";
  if (app.renderer.background) app.renderer.background.alpha = 0;
  app.renderer.backgroundAlpha = 0;
  return app;
}
