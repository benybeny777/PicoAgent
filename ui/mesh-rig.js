// 既存の高精細レイヤーを同じ格子メッシュへ重ね、顔の局所差分と微細な変形を連続化する。
(function initMeshRig(root) {
  "use strict";

  const clamp = (value, min, max) => Math.min(max, Math.max(min, Number(value) || 0));
  const smoothstep = (edge0, edge1, value) => {
    const t = clamp((value - edge0) / Math.max(0.0001, edge1 - edge0), 0, 1);
    return t * t * (3 - 2 * t);
  };

  function versioned(url, version) {
    if (!version) return url;
    return `${url}${url.includes("?") ? "&" : "?"}v=${encodeURIComponent(version)}`;
  }

  function waitTexture(texture, timeoutMs = 15000) {
    if (texture.baseTexture.valid) return Promise.resolve(texture);
    return new Promise((resolve, reject) => {
      let finished = false;
      const finish = (error) => {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        texture.baseTexture.off("loaded", loaded);
        texture.baseTexture.off("error", failed);
        if (error) reject(error); else resolve(texture);
      };
      const loaded = () => finish();
      const failed = () => finish(new Error("メッシュリグのテクスチャを読み込めません"));
      const timer = setTimeout(() => finish(new Error("メッシュリグのテクスチャ読込がタイムアウトしました")), timeoutMs);
      texture.baseTexture.once("loaded", loaded);
      texture.baseTexture.once("error", failed);
    });
  }

  class MeshRigController {
    constructor(app, canvas, rig, planes) {
      this.app = app;
      this.canvas = canvas;
      this.rig = rig;
      this.planes = planes;
      this.active = false;
      this.mouth = 0;
      this.eyeOpen = 1;
      this.blinkTween = null;
      this.gazeTargetX = 0;
      this.gazeTargetY = 0;
      this.gazeX = 0;
      this.gazeY = 0;
      this.angleX = 0;
      this.angleY = 0;
      this.previousAngleX = 0;
      this.hairPosition = 0;
      this.hairVelocity = 0;
      this.motion = "idle";
      this.startedAt = performance.now();
      this.lastAt = 0;
      this.tick = this.tick.bind(this);
      this.app.ticker.add(this.tick);
      this.app.ticker.stop();
    }

    setActive(active) {
      const next = Boolean(active);
      // 同じメッシュ内の状態遷移でtickerを再起動すると、WebGLの次回描画まで
      // キャンバスが空になる瞬間がある。表示状態が変わるときだけ開始・停止する。
      if (this.active === next) return;
      this.active = next;
      this.canvas.style.display = this.active ? "" : "none";
      if (this.active) {
        this.lastAt = 0;
        this.app.ticker.start();
        this.tick();
      } else {
        this.app.ticker.stop();
      }
    }

    setMouth(value) { this.mouth = clamp(value, 0, 1); }

    setBlink(target, duration = 0) {
      const next = clamp(target, 0, 1);
      if (duration <= 0) {
        this.eyeOpen = next;
        this.blinkTween = null;
        return;
      }
      this.blinkTween = {
        from: this.eyeOpen,
        to: next,
        startedAt: performance.now(),
        duration: Math.max(1, Number(duration) || 1),
      };
    }

    setGaze(x, y = 0) {
      this.gazeTargetX = clamp(x, -1, 1);
      this.gazeTargetY = clamp(y, -1, 1);
    }

    setMotion(name) { this.motion = String(name || "idle"); }

    updateBlink(now) {
      if (!this.blinkTween) return;
      const progress = clamp((now - this.blinkTween.startedAt) / this.blinkTween.duration, 0, 1);
      const eased = progress * progress * (3 - 2 * progress);
      this.eyeOpen = this.blinkTween.from + (this.blinkTween.to - this.blinkTween.from) * eased;
      if (progress >= 1) this.blinkTween = null;
    }

    updateAlphas() {
      const mouth = this.planes.get("mouth_open");
      if (mouth) mouth.plane.alpha = this.mouth;
      const gazeLeft = this.planes.get("gaze_left");
      const gazeRight = this.planes.get("gaze_right");
      // 閉眼中に開いた瞳の視線差分を重ねない。半目から開眼までだけ滑らかに戻す。
      const gazeVisibility = smoothstep(0.5, 1, this.eyeOpen);
      if (gazeLeft) gazeLeft.plane.alpha = Math.max(0, -this.gazeX) * gazeVisibility;
      if (gazeRight) gazeRight.plane.alpha = Math.max(0, this.gazeX) * gazeVisibility;
      const half = this.planes.get("blink_half");
      const closed = this.planes.get("blink_closed");
      if (this.eyeOpen >= 0.5) {
        if (half) half.plane.alpha = (1 - this.eyeOpen) * 2;
        if (closed) closed.plane.alpha = 0;
      } else {
        if (half) half.plane.alpha = 1;
        if (closed) closed.plane.alpha = 1 - this.eyeOpen * 2;
      }
    }

    updateMesh(now, dt) {
      const elapsed = (now - this.startedAt) / 1000;
      const idleX = Math.sin(elapsed * 0.83) * 1.25 + Math.sin(elapsed * 0.37 + 1.1) * 0.65;
      const idleY = Math.sin(elapsed * 0.61 + 0.4) * 0.7;
      this.gazeX += (this.gazeTargetX - this.gazeX) * Math.min(1, dt * 8);
      this.gazeY += (this.gazeTargetY - this.gazeY) * Math.min(1, dt * 8);
      const targetAngleX = clamp(idleX + this.gazeX * 4.2, -6, 6);
      const targetAngleY = clamp(idleY + this.gazeY * 2.5, -4, 4);
      this.angleX += (targetAngleX - this.angleX) * Math.min(1, dt * 4.5);
      this.angleY += (targetAngleY - this.angleY) * Math.min(1, dt * 4.5);
      const angularVelocity = (this.angleX - this.previousAngleX) / Math.max(dt, 1 / 240);
      this.previousAngleX = this.angleX;
      const acceleration = (-angularVelocity * 0.018 - this.hairPosition) * 16 - this.hairVelocity * 6;
      this.hairVelocity += acceleration * dt;
      this.hairPosition += this.hairVelocity * dt;
      const breath = Math.sin(elapsed * Math.PI * 2 / 3.8);

      for (const entry of this.planes.values()) {
        const data = entry.buffer.data;
        for (let index = 0; index < entry.original.length; index += 2) {
          const x = entry.original[index];
          const y = entry.original[index + 1];
          const globalX = x + entry.offset[0];
          const globalY = y + entry.offset[1];
          const nx = globalX / this.rig.canvas[0];
          const ny = globalY / this.rig.canvas[1];
          const head = 1 - smoothstep(0.43, 0.64, ny);
          const torso = smoothstep(0.34, 0.55, ny) * (1 - smoothstep(0.76, 0.93, ny));
          const hairEdge = head * smoothstep(0.18, 0.46, Math.abs(nx - 0.5));
          const horizontal = head * this.angleX * (0.82 + (0.5 - nx) * 0.22)
            + hairEdge * this.hairPosition * 18;
          const vertical = head * this.angleY * 0.34 - torso * breath * 2.4;
          data[index] = x + horizontal;
          data[index + 1] = y + vertical;
        }
        entry.buffer.update();
      }
    }

    tick() {
      if (!this.active) return;
      const now = performance.now();
      const dt = this.lastAt ? clamp((now - this.lastAt) / 1000, 1 / 240, 1 / 30) : 1 / 60;
      this.lastAt = now;
      this.updateBlink(now);
      this.updateAlphas();
      this.updateMesh(now, dt);
    }

    destroy() {
      this.app.ticker.remove(this.tick);
      this.app.destroy(false, { children: true, texture: true, baseTexture: true });
      this.canvas.remove();
    }
  }

  async function create({ canvas, rigUrl, rootUrl, assetVersion, width, height }) {
    if (!root.PIXI?.Application || !root.PIXI?.SimplePlane) {
      throw new Error("PIXI 6のメッシュ描画機能がありません");
    }
    const response = await fetch(versioned(rigUrl, assetVersion));
    if (!response.ok) throw new Error(`rig.jsonを取得できません（HTTP ${response.status}）`);
    const rig = await response.json();
    if (rig?.version !== 1 || rig?.mode !== "single-sheet-grid-v1") {
      throw new Error("未対応のメッシュリグ形式です");
    }
    if (!Array.isArray(rig.canvas) || rig.canvas.some(value => !Number.isFinite(value) || value <= 0)) {
      throw new Error("メッシュリグのキャンバス寸法が不正です");
    }
    const gridX = clamp(rig.grid?.[0], 2, 32);
    const gridY = clamp(rig.grid?.[1], 2, 40);
    const transparentContextOptions = { alpha: true, premultipliedAlpha: true, antialias: true };
    const context = canvas.getContext("webgl2", transparentContextOptions)
      || canvas.getContext("webgl", transparentContextOptions);
    if (!context) throw new Error("WebGLを初期化できません");
    // CSS上は既存キャラ枠の285x315へ合わせるが、描画バッファは制作素材を縮めない。
    // pet/stream形式でCSS拡大されても、890x1800の元画素を先に失わないための倍率。
    const textureFit = Math.min(width / rig.canvas[0], height / rig.canvas[1]);
    // 4.5倍（1283x1418）で1080p配信の横方向と高DPIのペット表示を覆う。原画相当の5.7倍を
    // 常時再描画すると1080p撮影の初期表示を遅らせるため、画面に出ない余剰だけを落とす。
    const renderResolution = Math.max(root.devicePixelRatio || 1, Math.min(4.5, 1 / textureFit));
    const app = new PIXI.Application({
      view: canvas,
      context,
      width,
      height,
      transparent: true,
      useContextAlpha: true,
      backgroundAlpha: 0,
      antialias: true,
      resolution: renderResolution,
      autoDensity: true,
    });
    const definitions = [
      ["base", rig.base, [0, 0, ...rig.canvas]],
      ...Object.entries(rig.drawables || {}).map(([name, item]) => [name, item.texture, item.textureBox || [0, 0, ...rig.canvas]]),
    ];
    const loaded = await Promise.all(definitions.map(async ([name, path, box]) => {
      const texture = PIXI.Texture.from(versioned(`${rootUrl}/${path}`, assetVersion));
      await waitTexture(texture);
      return [name, texture, box];
    }));
    const planes = new Map();
    const originX = (width - rig.canvas[0] * textureFit) / 2;
    const originY = height - rig.canvas[1] * textureFit;
    for (const [name, texture, box] of loaded) {
      const plane = new PIXI.SimplePlane(texture, gridX, gridY);
      plane.scale.set(textureFit);
      plane.position.set(originX + box[0] * textureFit, originY + box[1] * textureFit);
      if (name !== "base") plane.alpha = 0;
      const buffer = plane.geometry.getBuffer("aVertexPosition");
      planes.set(name, { plane, buffer, original: new Float32Array(buffer.data), offset: [box[0], box[1]] });
      app.stage.addChild(plane);
    }
    canvas.style.background = "transparent";
    canvas.dataset.meshRig = "ready";
    return new MeshRigController(app, canvas, rig, planes);
  }

  root.PicoMeshRig = Object.freeze({ create });
})(window);
