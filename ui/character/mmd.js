// mmd.js - MMD（PMX）の描画器。3D の描画とカメラ構図は ui/mmd-renderer.js が担う。
//
// キャンバスの大きさはキャラのソース矩形そのもので、mmd-renderer.js はキャンバスの寸法を見て
// カメラを合わせる（縦長・横長どちらでも構図の範囲が収まる）。

import { el, loadScriptOnce } from "../app/util.js";
import { defaultFramingFor } from "./framing.js";

export class MmdRenderer {
  static kind = "mmd";
  static supports(pack) {
    return Boolean(pack.mmd?.model);
  }

  constructor({ reducedMotion = false } = {}) {
    this.canvas = el("canvas", { className: "character-canvas", id: "mmdCanvas" }, [], { "aria-hidden": "true" });
    this.element = this.canvas;
    this.reducedMotion = reducedMotion;
    this.framing = "bust";
    this.abort = new AbortController();
  }

  async load(pack) {
    this.pack = pack;
    await loadScriptOnce("mmd-renderer.js");
    // MMD はキャンバスの大きさでカメラを決めるため、読み込み前に仮の大きさを与える。
    this.canvas.style.width = "300px";
    this.canvas.style.height = "400px";
    this.controller = await window.PicoMMD.create(this.canvas, {
      modelUrl: `${pack.root}/${pack.mmd.model}`,
      config: { ...pack.mmd, bodyProportion: pack.bodyProportion },
      interactive: false,
      signal: this.abort.signal,
    });
    this.controller.setMotionEnabled(!this.reducedMotion && !document.hidden);
    this.controller.setMouthLevel(0);
    this.pose = pack.mmd.defaultPose === "original" ? "original" : "natural";
    this.controller.setPose(this.pose);
  }

  /** 3D は構図ごとの縦横比をカメラが吸収する。既定の比率（幅/高さ）だけを返す。 */
  framingRect(name) {
    const aspect = window.PicoMMD.framingAspect(name);
    return { x: 0, y: 0, w: aspect, h: 1 };
  }
  naturalSize() { return { w: window.PicoMMD.framingAspect("full"), h: 1 }; }
  defaultFraming() { return defaultFramingFor(this.pack, "mmd"); }
  get sourcePixelsPerUnit() { return Infinity; }

  resize(cssWidth, cssHeight, pixelRatio) {
    this.canvas.style.width = `${Math.max(1, cssWidth)}px`;
    this.canvas.style.height = `${Math.max(1, cssHeight)}px`;
    this.controller?.setPixelRatio(pixelRatio);
  }

  setFraming(name) {
    this.framing = name;
    this.controller?.setFraming(name);
  }

  setMotion(name) { this.controller?.setExpression(name); }
  setMouth(level) { this.controller?.setMouthLevel(level); }
  setSpeaking(speaking) { if (!speaking) this.controller?.setMouthLevel(0); }
  setGaze() {}
  setSleeping() {}
  setPose(pose) {
    this.pose = pose === "original" ? "original" : "natural";
    this.controller?.setPose(this.pose);
  }
  setActive(active) {
    this.controller?.setMotionEnabled(Boolean(active) && !this.reducedMotion);
    if (!active) this.controller?.setMouthLevel(0);
  }
  setReducedMotion(reduced) {
    this.reducedMotion = Boolean(reduced);
    this.controller?.setMotionEnabled(!this.reducedMotion && !document.hidden);
  }
  /** 表示直前に1回描き切る（未描画の canvas のまま窓を出さない）。 */
  capturePreview() { this.controller?.capturePreview(); }

  dispose() {
    this.abort.abort();
    this.controller?.setMouthLevel(0);
    this.controller?.dispose();
    this.controller = null;
    this.canvas.remove();
  }
}
