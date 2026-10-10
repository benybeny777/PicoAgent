// image.js - 通常2D（連番フレーム・差分レイヤー）の描画器。
//
// 原画をキャンバスへ直接縮小描画する。キャンバスの大きさはキャラのソース矩形そのもので、
// 構図の切り出し範囲を縦横比を保って収める（CSS の拡大を使わない）。差分レイヤー（runtimeLayers）が
// 揃っていれば、待機・発話中は差分を切り替えてまばたき・口パク・よそ見をする（PNGTuber 方式）。

import { packUrl } from "./manifest.js";
import { backingResolution, defaultFramingFor, fitTransform, framingRectsFor } from "./framing.js";
import { clamp, el } from "../app/util.js";

const RESOURCE_TIMEOUT_MS = 5000;
const MOUTH_OPEN_THRESHOLD = 0.34;
const MOUTH_CLOSE_THRESHOLD = 0.22;
// LocalVTuberStudioの合格画面と同じ650ms。閉眼を220ms保持し、低fpsでも必ず見せる。
const BLINK_TIMINGS = Object.freeze({
  normal: Object.freeze({ closing: 180, closed: 220, opening: 250 }),
  sleepy: Object.freeze({ closing: 220, closed: 360, opening: 320 }),
});
const MOTION_FALLBACKS = {
  listening: "idle", searching: "thinking", working: "thinking", success: "talking", pleased: "talking",
  shy: "idle", concern: "sad", wake: "idle", sleep: "idle",
};

function framePaths(animation) {
  if (Array.isArray(animation?.framePaths) && animation.framePaths.length) return animation.framePaths;
  const count = Number.isInteger(animation?.frames) && animation.frames > 0 ? animation.frames : 0;
  if (!animation?.path || count === 0) return [];
  return Array.from({ length: count }, (_unused, index) => `${animation.path}/${String(index).padStart(2, "0")}.png`);
}

/** 画像を1枚読む。壊れていても全体の準備完了を塞がないよう、失敗も時間切れも「読めなかった」で返す。 */
function loadImage(url, signal) {
  return new Promise(resolve => {
    const image = new Image();
    let finished = false;
    const finish = ok => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      image.onload = null;
      image.onerror = null;
      signal?.removeEventListener("abort", abort);
      if (!ok) image.removeAttribute("src");
      resolve(ok && image.naturalWidth > 0 ? image : null);
    };
    const abort = () => finish(false);
    const timer = setTimeout(() => finish(false), RESOURCE_TIMEOUT_MS);
    if (signal?.aborted) { finish(false); return; }
    signal?.addEventListener("abort", abort, { once: true });
    image.onload = () => finish(true);
    image.onerror = () => finish(false);
    image.decoding = "async";
    image.src = url;
    if (image.complete) queueMicrotask(() => finish(image.naturalWidth > 0));
  });
}

/** 透明余白を除いた可視範囲。縮小した写しのアルファから求める（全画素を読むと大判で重いため）。 */
function measureContent(image) {
  const width = image.naturalWidth;
  const height = image.naturalHeight;
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) throw new Error("キャラ画像の寸法が不正です");
  const sampleHeight = Math.min(512, height);
  const factor = sampleHeight / height;
  const sampleWidth = Math.max(1, Math.round(width * factor));
  const canvas = el("canvas", { width: sampleWidth, height: sampleHeight });
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("キャラ画像の可視範囲を測る canvas を準備できません");
  let data;
  try {
    context.drawImage(image, 0, 0, sampleWidth, sampleHeight);
    data = context.getImageData(0, 0, sampleWidth, sampleHeight).data;
  }
  finally { canvas.width = 0; canvas.height = 0; }
  let minX = sampleWidth, minY = sampleHeight, maxX = -1, maxY = -1;
  for (let y = 0; y < sampleHeight; y += 1) {
    for (let x = 0; x < sampleWidth; x += 1) {
      if (data[(y * sampleWidth + x) * 4 + 3] > 12) {
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
    }
  }
  if (maxX < 0) return { x: 0, y: 0, w: width, h: height };
  const scale = 1 / factor;
  const x = Math.max(0, (minX - 1) * scale);
  const y = Math.max(0, (minY - 1) * scale);
  // 頭頂の上に3%の安全余白（メッシュと同じ）。
  const top = Math.max(0, y - (maxY - minY) * scale * 0.03);
  return { x, y: top, w: Math.min(width, (maxX + 2) * scale) - x, h: Math.min(height, (maxY + 2) * scale) - top };
}

export class ImageRenderer {
  static kind = "2d";
  static supports(pack) {
    return Boolean(pack.animations?.idle);
  }

  constructor({ reducedMotion = false } = {}) {
    this.canvas = el("canvas", { className: "character-canvas" }, [], { "aria-hidden": "true" });
    this.context = this.canvas.getContext("2d");
    this.element = this.canvas;
    this.images = new Map();
    this.animationName = "idle";
    this.motion = "idle";
    this.frame = 0;
    this.lastFrameAt = 0;
    this.reducedMotion = reducedMotion;
    this.active = true;
    this.mouthLevel = 0;
    this.mouthOpen = false;
    this.blinkPhase = null;
    this.lookDirection = 0;
    this.speaking = false;
    this.sleepingImage = null;
    this.cssWidth = 1;
    this.cssHeight = 1;
    this.pixelRatio = 1;
    this.framing = "bust";
    this.timers = new Set();
    this.abort = new AbortController();
    this.springs = [];
    this.bodyMotion = null;
    this.loop = window.PicoFrameScheduler.createLoop(now => this.step(now));
  }

  async load(pack) {
    this.pack = pack;
    this.animations = pack.animations;
    const idle = framePaths(this.animations.idle);
    const first = await loadImage(packUrl(pack, idle[0] || "preview.png"), this.abort.signal);
    if (this.disposed) throw new Error("キャラ画像の読み込みを中断しました");
    if (!first) throw new Error("待機画像を読み込めません");
    this.images.set(packUrl(pack, idle[0] || "preview.png"), first);
    this.modelWidth = first.naturalWidth;
    this.modelHeight = first.naturalHeight;
    this.content = measureContent(first);
    this.framingRects = framingRectsFor(this.content, pack);
    await Promise.all(idle.map(path => this.preload(path)));
    // 残りのモーションは表示を塞がず裏で温める。
    this.later(() => {
      for (const animation of Object.values(this.animations)) framePaths(animation).forEach(path => this.preload(path));
    }, 120);
    const layerNames = ["base", "mouth_open", "mouth_closed", "blink_half", "blink_closed"];
    const layers = await Promise.all(layerNames.map(name => this.preload(this.layerPath(name))));
    this.layersAvailable = layers.every(Boolean);
    if (this.layersAvailable) {
      const extra = await Promise.all(["gaze_left", "gaze_right", "expression_happy"].map(name => this.preload(this.layerPath(name))));
      this.gazeAvailable = Boolean(extra[0] && extra[1]);
      this.happyAvailable = Boolean(extra[2]);
    }
    this.configureSprings(pack.springPhysics, pack.secondaryMotion);
    this.setMotion("idle");
  }

  layerPath(name) {
    const layers = this.pack.runtimeLayers || {};
    return typeof layers[name] === "string" ? layers[name] : `layers/${name}.png`;
  }

  async preload(relative) {
    if (this.disposed) return null;
    const url = packUrl(this.pack, relative);
    if (this.images.has(url)) return this.images.get(url);
    const pending = loadImage(url, this.abort.signal);
    this.images.set(url, pending);
    const image = await pending;
    if (this.disposed) return null;
    this.images.set(url, image);
    return image;
  }

  imageFor(relative) {
    const image = this.images.get(packUrl(this.pack, relative));
    return image instanceof HTMLImageElement ? image : null;
  }

  naturalSize() { return { w: this.modelWidth, h: this.modelHeight }; }
  framingRect(name) { return this.framingRects[name] || this.framingRects.full; }
  defaultFraming() { return defaultFramingFor(this.pack, "2d"); }
  /** 原画1画素あたりのモデル座標（拡大上限の計算用）。 */
  get sourcePixelsPerUnit() { return 1; }

  resize(cssWidth, cssHeight, pixelRatio) {
    this.cssWidth = Math.max(1, cssWidth);
    this.cssHeight = Math.max(1, cssHeight);
    this.pixelRatio = backingResolution(this.cssWidth, this.cssHeight, pixelRatio);
    const width = Math.max(1, Math.round(this.cssWidth * this.pixelRatio));
    const height = Math.max(1, Math.round(this.cssHeight * this.pixelRatio));
    if (this.canvas.width !== width) this.canvas.width = width;
    if (this.canvas.height !== height) this.canvas.height = height;
    this.canvas.style.width = `${this.cssWidth}px`;
    this.canvas.style.height = `${this.cssHeight}px`;
    this.draw();
  }

  setFraming(name) {
    this.framing = name;
    this.draw();
  }

  currentImage() {
    if (this.sleepingImage) return this.sleepingImage;
    if (this.layersAvailable && (this.animationName === "idle" || this.animationName === "talking")) {
      const gazing = this.gazeAvailable && !this.speaking && this.animationName === "idle" && this.lookDirection !== 0;
      const gaze = gazing ? (this.lookDirection < 0 ? "gaze_left" : "gaze_right") : "mouth_closed";
      const blink = gazing ? null : this.blinkPhase;
      const happy = this.happyAvailable && ["success", "pleased", "shy"].includes(this.motion) ? "expression_happy" : null;
      const name = blink || (this.mouthOpen ? "mouth_open" : (happy || gaze));
      return this.imageFor(this.layerPath(name)) || this.imageFor(this.layerPath("base"));
    }
    const animation = this.animations[this.animationName] || this.animations.idle;
    const paths = framePaths(animation);
    return this.imageFor(paths[this.frame % Math.max(1, paths.length)] || "preview.png")
      || this.imageFor(framePaths(this.animations.idle)[0] || "preview.png");
  }

  draw(now = performance.now()) {
    const image = this.currentImage();
    const context = this.context;
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, this.canvas.width, this.canvas.height);
    if (!image || !this.framingRects) return;
    const frame = this.framingRect(this.framing);
    const fit = fitTransform(frame, this.cssWidth, this.cssHeight);
    const bodyY = this.bodyOffset(now);
    context.setTransform(
      fit.scale * this.pixelRatio, 0, 0, fit.scale * this.pixelRatio,
      fit.offsetX * this.pixelRatio, (fit.offsetY + bodyY) * this.pixelRatio,
    );
    context.imageSmoothingEnabled = true;
    context.imageSmoothingQuality = "high";
    context.drawImage(image, 0, 0, this.modelWidth, this.modelHeight);
    this.drawSprings(image, now);
  }

  // ── 局所の二次動作（揺れる部位） ───────────────────────────────
  configureSprings(config, legacy) {
    this.springs = [];
    this.bodyMotion = null;
    const legacyRegions = legacy?.type === "bust" ? [{
      id: "bust", centerX: legacy.centerX, centerY: legacy.centerY,
      radiusX: legacy.radiusX, radiusY: legacy.radiusY, drive: Number(legacy.strength || 1) * 0.55,
    }] : [];
    const regions = Array.isArray(config?.regions) ? config.regions : legacyRegions;
    const number = (value, min, max, fallback) => {
      const numeric = Number(value);
      return Number.isFinite(numeric) ? clamp(numeric, min, max) : fallback;
    };
    if (config?.bodyMotion && typeof config.bodyMotion === "object") {
      this.bodyMotion = {
        amplitudeY: number(config.bodyMotion.amplitudeY, 0, 4, 1.2),
        frequency: number(config.bodyMotion.frequency, 0.08, 0.8, 0.18),
        phase: number(config.bodyMotion.phase, -6.3, 6.3, 0),
      };
    }
    regions.slice(0, 12).forEach((region, index) => {
      const cx = number(region.centerX, 0, 100, 50) / 100 * this.modelWidth;
      const cy = number(region.centerY, 0, 100, 34) / 100 * this.modelHeight;
      const rx = number(region.radiusX, 3, 48, 16) / 100 * this.modelWidth;
      const ry = number(region.radiusY, 3, 42, 10) / 100 * this.modelHeight;
      this.springs.push({
        cx, cy, rx, ry, position: 0, velocity: 0,
        stiffness: number(region.stiffness, 4, 80, 22),
        damping: number(region.damping, 1, 24, 8),
        drive: number(region.drive, 0.05, 2.4, 0.45),
        frequency: number(region.frequency, 0.08, 1.2, 0.24),
        phase: number(region.phase, -6.3, 6.3, index * 0.7),
        axisX: number(region.axisX, -8, 8, 0),
        axisY: number(region.axisY, -8, 8, -1),
        rotation: number(region.rotation, -6, 6, 0),
      });
    });
    this.springLastAt = 0;
  }

  bodyOffset(now) {
    if (!this.bodyMotion || this.reducedMotion || !this.active) return 0;
    return Math.sin((now / 1000) * Math.PI * 2 * this.bodyMotion.frequency + this.bodyMotion.phase) * this.bodyMotion.amplitudeY;
  }

  /** 楕円で切り出した部位を、ばねの変位ぶん少しずらして重ねる（変位は数px以内）。 */
  drawSprings(image, now) {
    if (!this.springs.length || this.reducedMotion || !this.active) return;
    const dt = this.springLastAt ? clamp((now - this.springLastAt) / 1000, 1 / 240, 1 / 30) : 1 / 60;
    this.springLastAt = now;
    const context = this.context;
    for (const spring of this.springs) {
      const target = Math.sin((now / 1000) * Math.PI * 2 * spring.frequency + spring.phase) * spring.drive;
      spring.velocity += ((target - spring.position) * spring.stiffness - spring.velocity * spring.damping) * dt;
      spring.position += spring.velocity * dt;
      const p = spring.position;
      if (Math.abs(p) < 0.01) continue;
      context.save();
      context.beginPath();
      context.ellipse(spring.cx, spring.cy, spring.rx, spring.ry, 0, 0, Math.PI * 2);
      context.clip();
      context.translate(spring.cx + p * spring.axisX, spring.cy + p * spring.axisY);
      context.rotate((p * spring.rotation * Math.PI) / 180);
      context.translate(-spring.cx, -spring.cy);
      context.drawImage(image, 0, 0, this.modelWidth, this.modelHeight);
      context.restore();
    }
  }

  // ── 状態 ─────────────────────────────────────────────────────
  setMotion(name) {
    this.motion = name;
    const source = this.animations[name] ? name
      : MOTION_FALLBACKS[name] && this.animations[MOTION_FALLBACKS[name]] ? MOTION_FALLBACKS[name] : "idle";
    if (source !== this.animationName) {
      this.animationName = source;
      this.frame = 0;
      this.lastFrameAt = performance.now();
      framePaths(this.animations[source]).forEach(path => this.preload(path));
    }
    this.canvas.dataset.motion = name;
    this.scheduleBlink();
    this.wake();
  }

  setMouth(level) {
    this.mouthLevel = clamp(Number(level) || 0, 0, 1);
    // 差分絵は開閉2枚しか無い。開く側を高く閉じる側を低くして、境界付近での細かな往復を止める。
    const open = this.mouthLevel > (this.mouthOpen ? MOUTH_CLOSE_THRESHOLD : MOUTH_OPEN_THRESHOLD);
    if (open !== this.mouthOpen) {
      this.mouthOpen = open;
      this.draw();
    }
  }

  setSpeaking(speaking) {
    this.speaking = Boolean(speaking);
    if (!this.speaking) this.setMouth(0);
  }

  setGaze(x) {
    const direction = x < -0.38 ? -1 : x > 0.38 ? 1 : 0;
    if (direction === this.lookDirection) return;
    this.lookDirection = direction;
    this.draw();
  }

  setSleeping(relative) {
    this.sleepingImage = null;
    if (relative) {
      this.preload(relative).then(image => {
        if (!image) return;
        this.sleepingImage = image;
        this.draw();
      });
    }
    this.draw();
  }

  setActive(active) {
    this.active = Boolean(active);
    if (this.active) this.wake();
    else this.loop.stop();
  }

  setReducedMotion(reduced) {
    this.reducedMotion = Boolean(reduced);
    this.wake();
  }

  /** まばたき: half → closed → half の段階表示（閉じ側を詰め、開き側を伸ばした非対称の配分）。 */
  scheduleBlink() {
    if (this.blinkTimer || !this.layersAvailable) return;
    const hour = new Date().getHours();
    const sleepy = hour >= 23 || hour < 5;
    const timings = sleepy ? BLINK_TIMINGS.sleepy : BLINK_TIMINGS.normal;
    const interval = sleepy ? 900 + Math.random() * 1600 : 1400 + Math.random() * 2800;
    this.blinkTimer = this.later(async () => {
      const step = (phase, ms) => new Promise(resolve => {
        this.blinkPhase = phase;
        this.draw();
        this.later(resolve, ms);
      });
      await step("blink_half", timings.closing);
      await step("blink_closed", timings.closed);
      await step("blink_half", timings.opening);
      this.blinkPhase = null;
      this.draw();
      this.blinkTimer = 0;
      if (!this.disposed) this.scheduleBlink();
    }, interval);
  }

  later(callback, ms) {
    if (this.disposed) return 0;
    const id = setTimeout(() => {
      this.timers.delete(id);
      if (!this.disposed) callback();
    }, ms);
    this.timers.add(id);
    return id;
  }

  wake() {
    if (!this.disposed) this.loop.start();
  }

  step(now) {
    if (this.disposed || !this.active || document.hidden) return false;
    const animation = this.animations[this.animationName] || this.animations.idle;
    const paths = framePaths(animation);
    const layered = this.layersAvailable && (this.animationName === "idle" || this.animationName === "talking");
    const springing = this.springs.length > 0 || Boolean(this.bodyMotion);
    // 「アニメーションを減らす」でも、発話中の口パクは状態を伝える機能なので残す。
    if (this.reducedMotion && this.animationName !== "talking") {
      this.frame = 0;
      this.draw(now);
      return false;
    }
    if (!layered && !this.sleepingImage && paths.length > 1) {
      const duration = Number(animation.frameDurations?.[this.frame]) > 0
        ? Number(animation.frameDurations[this.frame])
        : 1000 / (Number(animation.fps) || 6);
      if (!this.lastFrameAt) this.lastFrameAt = now;
      if (now - this.lastFrameAt >= duration) {
        this.frame = (this.frame + 1) % paths.length;
        this.lastFrameAt = now;
      }
      this.draw(now);
      return true;
    }
    if (springing && !this.sleepingImage) {
      // 揺れは30fpsで足りる。大判の原画を毎フレーム描き直さない。
      if (now - (this.lastSpringDrawAt || 0) >= 1000 / 30) {
        this.lastSpringDrawAt = now;
        this.draw(now);
      }
      return true;
    }
    this.draw(now);
    return false;
  }

  dispose() {
    this.disposed = true;
    this.abort.abort();
    this.loop.stop();
    for (const id of this.timers) clearTimeout(id);
    this.timers.clear();
    this.images.clear();
    this.canvas.width = 0;
    this.canvas.height = 0;
    this.canvas.remove();
  }
}
