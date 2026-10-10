// mesh.js - 独自2.5D（メッシュリグ）の描画器。変形・表情・口の計算は ui/mesh-rig.js の正規描画を使う。
//
// モデル座標は原画の可視範囲（contentBox）とほぼ同じ大きさで作り、キャンバスの大きさはキャラの
// ソース矩形そのものにする。構図の切り出し矩形をキャンバスへ写す変換だけを PIXI のステージへ掛ける。

import { ensurePixi } from "./runtime.js";
import { packUrl } from "./manifest.js";
import { backingResolution, defaultFramingFor, framingRectsFor } from "./framing.js";
import { el, errorDetail, fetchJson, logApp, sleep } from "../app/util.js";

const BLINK_TIMINGS = Object.freeze({
  normal: Object.freeze({ closing: 180, closed: 220, opening: 250 }),
  sleepy: Object.freeze({ closing: 220, closed: 360, opening: 320 }),
});

export class MeshRenderer {
  static kind = "mesh-rig";
  static supports(pack) {
    return Boolean(pack.meshRig?.rig);
  }

  constructor({ reducedMotion = false } = {}) {
    this.canvas = el("canvas", { className: "character-canvas", id: "meshRigCanvas" }, [], { "aria-hidden": "true" });
    this.element = this.canvas;
    this.reducedMotion = reducedMotion;
    this.active = false;
    this.frameBudget = 0;
    this.framing = "bust";
    this.speaking = false;
    this.timers = new Set();
  }

  async load(pack) {
    this.pack = pack;
    await ensurePixi();
    if (!window.PicoMeshRig?.create) throw new Error("メッシュリグ描画モジュールがありません");
    const rigPath = String(pack.meshRig.rig || "").replace(/^\/+/, "");
    const slash = rigPath.lastIndexOf("/");
    const rigRoot = slash >= 0 ? rigPath.slice(0, slash) : ".";
    // 可視範囲の縦横比でモデル座標を作る（高さ1000）。原画の画素とモデル座標がほぼ等倍になり、
    // 拡大上限の計算と構図の切り出しが素直になる。
    const rig = await fetchJson(packUrl(pack, rigPath), "rig.json", 5000);
    const canvasSize = Array.isArray(rig.canvas) ? rig.canvas : [rig.canvas?.width, rig.canvas?.height];
    const box = Array.isArray(rig.contentBox) && rig.contentBox.length === 4
      ? rig.contentBox
      : rig.layers?.neutral?.bbox || [0, 0, ...canvasSize];
    const contentWidth = Math.max(1, box[2] - box[0]);
    const contentHeight = Math.max(1, box[3] - box[1]);
    const modelHeight = 1000;
    const modelWidth = Math.round(modelHeight * contentWidth / contentHeight);
    this.controller = await window.PicoMeshRig.create({
      canvas: this.canvas,
      rigUrl: packUrl(pack, rigPath),
      rootUrl: `${pack.root}/${rigRoot}`,
      assetVersion: pack.meshRig.assetVersion || pack.assetVersion,
      mouthMode: pack.meshRig.mouthMode,
      width: modelWidth,
      height: modelHeight,
    });
    if (this.disposed) { this.controller.destroy(); this.controller = null; return; }
    this.controller.setReducedMotion(this.reducedMotion);
    this.controller.setActive(this.active);
    this.controller.setFrameBudget(this.frameBudget);
    this.content = this.controller.contentRect;
    this.framingRects = framingRectsFor(this.content, pack);
    this.sourceScale = contentHeight / this.content.h;
  }

  naturalSize() { return { w: this.content.w, h: this.content.h }; }
  framingRect(name) { return this.framingRects[name] || this.framingRects.full; }
  defaultFraming() { return defaultFramingFor(this.pack, "mesh-rig"); }
  get sourcePixelsPerUnit() { return this.sourceScale || 1; }

  resize(cssWidth, cssHeight, pixelRatio) {
    this.cssWidth = cssWidth;
    this.cssHeight = cssHeight;
    this.pixelRatio = backingResolution(cssWidth, cssHeight, pixelRatio);
    this.controller?.layout(cssWidth, cssHeight, this.pixelRatio, this.framingRect(this.framing));
  }

  setFraming(name) {
    this.framing = name;
    if (this.cssWidth) this.resize(this.cssWidth, this.cssHeight, this.pixelRatio);
  }

  setMotion(name) {
    this.canvas.dataset.motion = name;
    this.controller?.setMotion(name);
  }

  setMouth(level, form = 0) {
    this.controller?.setMouth(level, form);
  }

  setSpeaking(speaking) {
    this.speaking = Boolean(speaking);
    if (!speaking) this.controller?.setMouth(0, 0);
  }

  setGaze(x, y) {
    this.controller?.setGaze(x, y);
  }

  setSleeping() {
    // メッシュは寝姿の差し替え画像を持たない。居眠り中は描画を止め、ホストが2Dの寝姿へ切り替える。
  }

  setActive(active) {
    if (this.disposed) return;
    const next = Boolean(active);
    if (this.active === next) return;
    this.active = next;
    this.controller?.setActive(next);
    if (next && this.frameBudget) this.scheduleBlink();
  }

  setFrameBudget(fps) {
    if (this.disposed) return;
    this.frameBudget = fps;
    this.controller?.setFrameBudget(fps);
    if (this.active && fps) this.scheduleBlink();
  }

  setReducedMotion(reduced) {
    this.reducedMotion = Boolean(reduced);
    this.controller?.setReducedMotion(this.reducedMotion);
  }

  /** まばたき: 全閉まで連続補間し、閉眼保持後に開く（確認ツールと同じ動き）。 */
  scheduleBlink() {
    if (this.blinkTimer || this.disposed || !this.active || !this.frameBudget) return;
    const hour = new Date().getHours();
    const sleepy = hour >= 23 || hour < 5;
    const timings = sleepy ? BLINK_TIMINGS.sleepy : BLINK_TIMINGS.normal;
    const interval = sleepy ? 900 + Math.random() * 1600 : 1400 + Math.random() * 2800;
    const id = setTimeout(async () => {
      this.timers.delete(id);
      this.blinkTimer = 0;
      if (this.disposed || !this.active || !this.frameBudget) return;
      try {
        this.controller.setBlink(0, timings.closing);
        await sleep(timings.closing);
        if (this.disposed || !this.active || !this.frameBudget) return;
        this.controller.setEyeOpen(0, 0);
        await sleep(timings.closed);
        if (this.disposed || !this.active || !this.frameBudget) return;
        this.controller.setBlink(1, timings.opening);
        await sleep(timings.opening);
      } catch (error) {
        logApp("warn", "まばたきを更新できませんでした", errorDetail(error));
      }
      this.scheduleBlink();
    }, interval);
    this.blinkTimer = id;
    this.timers.add(id);
  }

  dispose() {
    this.disposed = true;
    for (const id of this.timers) clearTimeout(id);
    this.timers.clear();
    this.controller?.destroy();
    this.controller = null;
    this.canvas.remove();
  }
}
