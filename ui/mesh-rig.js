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

  function loadImage(url, timeoutMs = 15000) {
    return new Promise((resolve, reject) => {
      const image = new Image();
      const timer = setTimeout(() => reject(new Error("2.5D素材の読込がタイムアウトしました")), timeoutMs);
      image.onload = () => { clearTimeout(timer); resolve(image); };
      image.onerror = () => { clearTimeout(timer); reject(new Error(`2.5D素材を読み込めません: ${url}`)); };
      image.src = url;
    });
  }

  function validateLvsSecondaryMotion(sourceRig) {
    const version = sourceRig.secondary_motion_version;
    const profile = sourceRig.secondary_motion;
    if (version === undefined && profile === undefined) return null;
    const hair = profile?.hair;
    if (version !== 1 || hair?.role !== "hair"
      || !Number.isFinite(hair.frequency_hz) || hair.frequency_hz < 0.2 || hair.frequency_hz > 8
      || !Number.isFinite(hair.damping_ratio) || hair.damping_ratio < 0.2 || hair.damping_ratio > 2
      || !Number.isFinite(hair.strength) || hair.strength < 0 || hair.strength > 2) {
      throw new Error("LocalVTuberStudioの髪物理定義が不正です");
    }
    return hair;
  }

  function springStep(current, target, elapsedSeconds, frequencyHz, dampingRatio) {
    let value = current.value, velocity = current.velocity, remaining = Math.min(elapsedSeconds, 0.1);
    const omega = 2 * Math.PI * frequencyHz;
    while (remaining > 0) {
      const dt = Math.min(remaining, 0.01);
      velocity += ((target - value) * omega * omega - 2 * dampingRatio * omega * velocity) * dt;
      value += velocity * dt;
      remaining -= dt;
    }
    return { value, velocity };
  }

  function eyeAperture(layer, openness) {
    const aperture = layer?.eye_aperture;
    if (!Array.isArray(aperture) || aperture.length < 2) throw new Error("まぶたの実測境界がありません");
    const open = clamp(openness, 0, 1);
    const curve = layer.closed_curve;
    return aperture.map((point, index) => {
      if (!Array.isArray(point) || point.length !== 4 || !point.every(Number.isFinite)) throw new Error("まぶたの実測境界が不正です");
      const [x, top, bottom, closed] = point;
      const target = curve?.[index] ?? closed;
      // 解析された原画の開眼幅をそのまま上限にすると、細目の原画では待機時まで
      // 半目に見える。閉眼曲線は変えず、開眼側だけ中央から18%広げる。
      const center = (top + bottom) / 2;
      const openBoost = 1 + smoothstep(0.62, 1, open) * 0.18;
      const openTop = center + (top - center) * openBoost;
      const openBottom = center + (bottom - center) * openBoost;
      return [x, target + (openTop - target) * open, target + (openBottom - target) * open, closed];
    });
  }

  function drawBlink(ctx, images, rig, side, openness, gazeX = 0, gazeY = 0) {
    if (openness >= 1) return;
    const base = rig.layers[`${side}_eye_base`];
    const upper = rig.layers[`${side}_eyelid_upper`];
    const points = eyeAperture(base, openness);
    const draw = (name, layer) => ctx.drawImage(images.get(name), layer.texture_box[0], layer.texture_box[1]);
    // 閉眼下地PNGの周辺肌まで重ねると、半閉眼で矩形の色面になる。
    // 原画で測定した目の開口内だけへ限定し、その内側を現在の開口で描き戻す。
    const aperture = base.eye_aperture;
    ctx.save();
    ctx.beginPath();
    ctx.moveTo(aperture[0][0] - 0.5, aperture[0][1]);
    for (const [x, top] of aperture) ctx.lineTo(x, top);
    ctx.lineTo(aperture.at(-1)[0] + 0.5, aperture.at(-1)[1]);
    ctx.lineTo(aperture.at(-1)[0] + 0.5, aperture.at(-1)[2]);
    for (const [x, , bottom] of [...aperture].reverse()) ctx.lineTo(x, bottom);
    ctx.closePath();
    ctx.clip();
    draw(`${side}_eye_base`, base);
    if (openness > 0) {
      ctx.save();
      ctx.beginPath();
      ctx.moveTo(points[0][0] - 0.5, points[0][1]);
      for (const [x, y] of points) ctx.lineTo(x, y);
      ctx.lineTo(points.at(-1)[0] + 0.5, points.at(-1)[1]);
      ctx.lineTo(points.at(-1)[0] + 0.5, points.at(-1)[2]);
      for (const [x, , y] of [...points].reverse()) ctx.lineTo(x, y);
      ctx.closePath();
      ctx.clip();
      draw(`${side}_eye_backplate`, rig.layers[`${side}_eye_backplate`]);
      const iris = rig.layers[`${side}_eye_iris`];
      const irisImage = images.get(`${side}_eye_iris`);
      const apertureWidth = points.at(-1)[0] - points[0][0];
      const apertureHeight = Math.max(...points.map(point => point[2])) - Math.min(...points.map(point => point[1]));
      ctx.drawImage(
        irisImage,
        iris.texture_box[0] + clamp(gazeX, -1, 1) * apertureWidth * 0.08,
        iris.texture_box[1] + clamp(gazeY, -1, 1) * apertureHeight * 0.07,
      );
      ctx.restore();
    }
    ctx.restore();
    const image = images.get(`${side}_eyelid_upper`);
    const [left, top] = upper.texture_box;
    for (const [x, y, , closed] of points) {
      const column = Math.floor(x) - left;
      if (column >= 0 && column < image.naturalWidth) {
        ctx.drawImage(image, column, 0, 1, image.naturalHeight, left + column, top + y - closed, 1, image.naturalHeight);
      }
    }
  }

  function lipMesh(layer, opening, form, curvature = 0) {
    const box = layer?.texture_box;
    const seam = layer?.lip_seam;
    const feature = layer?.feature_box;
    if (!Array.isArray(box) || !Array.isArray(feature) || !Array.isArray(seam) || seam.length < 3) throw new Error("原画の唇の分割境界がありません");
    const [left, top, right, bottom] = box;
    const [mouthLeft, , mouthRight] = feature;
    const width = mouthRight - mouthLeft;
    const centerX = (mouthLeft + mouthRight) / 2;
    const open = clamp(opening, 0, 1);
    const shape = clamp(form, -1, 1);
    const knots = [[left, seam[0][1]], ...seam, [right, seam.at(-1)[1]]];
    const roundness = Math.max(0, -shape);
    const centerY = (feature[1] + feature[3]) / 2;
    const source = [], upper = [], lower = [];
    for (const [x, y] of knots) {
      const u = clamp((x - mouthLeft) / width, 0, 1);
      const interior = x >= mouthLeft && x <= mouthRight;
      const envelope = interior ? 1 : x < mouthLeft ? (x - left) / Math.max(1, mouthLeft - left) : (right - x) / Math.max(1, right - mouthRight);
      const bulge = interior ? Math.pow(Math.max(0, Math.sin(Math.PI * u)), 0.8 - 0.3 * roundness) : 0;
      const openBoost = 0.16 * Math.pow(Math.max(0, 1 - Math.pow(shape / 0.65, 2)), 2);
      const gap = width * (0.42 + 0.12 * roundness + openBoost) * 0.4 * open * (1 - 0.2 * shape) * bulge;
      const dx = (x - centerX) * shape * (0.30 + 0.25 * roundness) * envelope;
      const bend = (shape * 0.055 * (1 - roundness) * (bulge - 0.65)
        + curvature * 0.16 * (bulge - 0.4)) * width * envelope;
      const roundingShift = (centerY - y) * roundness * envelope;
      source.push([[x, top], [x, y], [x, bottom]]);
      upper.push([[x, top], [x + dx, y + roundingShift + bend - gap * 0.36]]);
      lower.push([[x + dx, y + roundingShift + bend + gap * 0.64], [x, bottom]]);
    }
    return { source, upper, lower, box, feature, open, form: shape };
  }

  function texturedTriangle(ctx, image, source, target, origin) {
    const [[x0, y0], [x1, y1], [x2, y2]] = source.map(([x, y]) => [x - origin[0], y - origin[1]]);
    const [[u0, v0], [u1, v1], [u2, v2]] = target;
    const det = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
    if (Math.abs(det) < 1e-8) return;
    const a = ((u1 - u0) * (y2 - y0) - (u2 - u0) * (y1 - y0)) / det;
    const c = ((u2 - u0) * (x1 - x0) - (u1 - u0) * (x2 - x0)) / det;
    const b = ((v1 - v0) * (y2 - y0) - (v2 - v0) * (y1 - y0)) / det;
    const d = ((v2 - v0) * (x1 - x0) - (v1 - v0) * (x2 - x0)) / det;
    ctx.save();
    ctx.beginPath(); ctx.moveTo(u0, v0); ctx.lineTo(u1, v1); ctx.lineTo(u2, v2); ctx.closePath(); ctx.clip();
    ctx.transform(a, b, c, d, u0 - a * x0 - c * y0, v0 - b * x0 - d * y0);
    ctx.drawImage(image, 0, 0);
    ctx.restore();
  }

  function localLipStrips(mesh) {
    const [, top, , bottom] = mesh.box;
    const width = mesh.feature[2] - mesh.feature[0];
    const smooth = value => value * value * (3 - 2 * value);
    const sourceUpper = [], targetUpper = [], sourceLower = [], targetLower = [];
    for (let index = 0; index < mesh.source.length; index += 1) {
      const [x, y] = mesh.source[index][1];
      const up = mesh.upper[index][1], down = mesh.lower[index][0];
      const fadeTop = Math.max(top, Math.min(mesh.feature[1] - width * 0.08, y - Math.max(0, y - up[1]) * 1.6 - width * 0.02));
      const fadeBottom = Math.min(bottom, Math.max(mesh.feature[3] + width * 0.12, y + width * 0.30, y + Math.max(0, down[1] - y) * 1.6 + width * 0.02));
      const su = [], tu = [], sl = [], tl = [];
      for (let row = 0; row <= 3; row += 1) {
        const ratio = row / 3;
        const upperY = fadeTop + (y - fadeTop) * ratio;
        const lowerY = y + (fadeBottom - y) * ratio;
        const upperWeight = smooth(ratio), lowerWeight = 1 - smooth(ratio);
        su.push([x, upperY]); tu.push([x + (up[0] - x) * upperWeight, upperY + (up[1] - y) * upperWeight]);
        sl.push([x, lowerY]); tl.push([x + (down[0] - x) * lowerWeight, lowerY + (down[1] - y) * lowerWeight]);
      }
      sourceUpper.push(su); targetUpper.push(tu); sourceLower.push(sl); targetLower.push(tl);
    }
    return { sourceUpper, targetUpper, sourceLower, targetLower };
  }

  function upperTeethGeometry(mesh) {
    if (mesh.open === 0) return null;
    const upper = mesh.upper.map(column => column[1]), lower = mesh.lower.map(column => column[0]);
    const left = upper[1][0], right = upper.at(-2)[0], width = right - left;
    const roundness = Math.max(0, -mesh.form);
    const sample = (points, x) => {
      for (let index = 1; index < points.length; index += 1) if (x <= points[index][0]) {
        const a = points[index - 1], b = points[index];
        const ratio = clamp((x - a[0]) / Math.max(1e-8, b[0] - a[0]), 0, 1);
        return a[1] + (b[1] - a[1]) * ratio;
      }
      return points.at(-1)[1];
    };
    const top = [], bottom = [], margin = 0.12 + 0.16 * roundness;
    for (let index = 0; index <= 16; index += 1) {
      const ratio = index / 16;
      const x = left + width * (margin + (1 - 2 * margin) * ratio);
      const y = sample(upper, x), gap = Math.max(0, sample(lower, x) - y);
      const start = y + gap * 0.025;
      const depth = Math.min(gap * 0.32, width * 0.115 * (1 - 0.75 * roundness)) * Math.sqrt(Math.max(0, Math.sin(Math.PI * ratio)));
      top.push([x, start]); bottom.push([x, start + depth]);
    }
    return { top, bottom, opacity: clamp(mesh.open / 0.16, 0, 1) * (0.94 - 0.24 * roundness) };
  }

  function drawTexturedMouth(ctx, image, layer, opening, form, color, curvature = 0) {
    const mesh = lipMesh(layer, opening, form, curvature);
    ctx.drawImage(image, mesh.box[0], mesh.box[1]);
    if (mesh.open > 0 || mesh.form !== 0) {
      const upper = mesh.upper.map(points => points[1]), lower = mesh.lower.map(points => points[0]);
      ctx.save(); ctx.beginPath(); ctx.moveTo(...upper[0]);
      for (const point of upper.slice(1)) ctx.lineTo(...point);
      for (const point of [...lower].reverse()) ctx.lineTo(...point);
      ctx.closePath();
      const rgb = color?.slice(0, 3);
      if (!rgb?.every(Number.isFinite)) throw new Error("口の原画色がありません");
      const innerTop = Math.min(...upper.map(point => point[1])), innerBottom = Math.max(...lower.map(point => point[1]));
      const cavity = ctx.createLinearGradient(0, innerTop, 0, Math.max(innerTop + 1, innerBottom));
      cavity.addColorStop(0, "rgb(26,12,18)"); cavity.addColorStop(0.55, "rgb(73,25,35)"); cavity.addColorStop(1, "rgb(82,35,45)");
      ctx.fillStyle = cavity; ctx.fill(); ctx.clip();
      const middle = lower[Math.floor(lower.length / 2)], width = layer.feature_box[2] - layer.feature_box[0];
      ctx.fillStyle = "rgb(176,88,102)"; ctx.beginPath();
      ctx.ellipse(middle[0], middle[1], width * 0.26 * (1 + mesh.form * 0.3), width * 0.09 * mesh.open, 0, 0, Math.PI * 2); ctx.fill();
      const teeth = upperTeethGeometry(mesh);
      if (teeth) {
        const teethTop = Math.min(...teeth.top.map(point => point[1]));
        const teethBottom = Math.max(...teeth.bottom.map(point => point[1]));
        const shade = ctx.createLinearGradient(0, teethTop, 0, Math.max(teethTop + 1, teethBottom));
        shade.addColorStop(0, "rgb(143,130,122)"); shade.addColorStop(0.42, "rgb(239,230,214)"); shade.addColorStop(1, "rgb(204,195,181)");
        ctx.globalAlpha *= teeth.opacity; ctx.fillStyle = shade; ctx.beginPath(); ctx.moveTo(...teeth.top[0]);
        for (const point of teeth.top.slice(1)) ctx.lineTo(...point);
        for (const point of [...teeth.bottom].reverse()) ctx.lineTo(...point);
        ctx.closePath(); ctx.fill();
      }
      ctx.restore();
      const strips = localLipStrips(mesh);
      for (const [source, target] of [[strips.sourceUpper, strips.targetUpper], [strips.sourceLower, strips.targetLower]]) {
        for (let index = 0; index < source.length - 1; index += 1) for (let row = 0; row < 3; row += 1) {
          const s0 = source[index], s1 = source[index + 1], d0 = target[index], d1 = target[index + 1];
          texturedTriangle(ctx, image, [s0[row], s1[row], s1[row + 1]], [d0[row], d1[row], d1[row + 1]], mesh.box);
          texturedTriangle(ctx, image, [s0[row], s1[row + 1], s0[row + 1]], [d0[row], d1[row + 1], d0[row + 1]], mesh.box);
        }
      }
    }
  }

  function createBlushOverlay(app, rig, originX, originY, textureFit) {
    const anchor = rig.anchors?.face;
    const legacyBox = rig.drawables?.face?.textureBox;
    const face = anchor || (Array.isArray(legacyBox) && legacyBox.length === 4 ? {
      x0: legacyBox[0], y0: legacyBox[1], x1: legacyBox[2], y1: legacyBox[3],
    } : null);
    if (!face) return null;
    const anime = ["anime25d-v1", "lvs-anime25d-v1"].includes(rig.mode);
    const width = face.x1 - face.x0;
    const height = face.y1 - face.y0;
    const cheeks = new PIXI.Graphics();
    // 照れは表情として一目で区別できる濃さにする。単色の楕円は実写で貼り物に
    // 見えるため、外側から中心へ薄い層を重ね、輪郭を弱く中心だけ濃くする。
    const color = anime ? 0xf36f98 : 0xe56f87;
    const scales = Array.from({ length: 12 }, (_, index) => 1 - index * 0.075);
    const alphas = scales.map((_, index) => (anime ? 0.065 : 0.045) * Math.sin((index + 1) / 13 * Math.PI));
    for (const [ratio, side] of [[0.24, "L"], [0.76, "R"]]) {
      const eye = rig.anchors?.[`eye${side}`];
      const mouth = rig.anchors?.mouth;
      const cheekX = eye ? (eye.x0 + eye.x1) / 2 : face.x0 + width * ratio;
      // 赤みの上端を下まぶたより下へ固定する。旧実写では視線差分の下端を
      // 目元の保護境界に使い、顔全体の比率だけで赤みを置かない。
      const gazeBox = rig.drawables?.gaze_left?.box;
      const eyeBottom = eye ? eye.y1 : gazeBox ? gazeBox[3] : face.y0 + height * 0.57;
      const eyeHeight = eye ? eye.y1 - eye.y0 : height * 0.055;
      const mouthY = mouth ? mouth.cy : face.y0 + height * 0.685;
      const safeTop = eyeBottom + Math.max(3, eyeHeight * 0.22);
      const cheekHeight = Math.max(2, mouthY - safeTop) * 0.24;
      const cheekY = safeTop + cheekHeight;
      for (let index = 0; index < scales.length; index += 1) {
        cheeks.beginFill(color, alphas[index]);
        cheeks.drawEllipse(
          originX + cheekX * textureFit,
          originY + cheekY * textureFit,
          width * (anime ? 0.14 : 0.12) * scales[index] * textureFit,
          cheekHeight * scales[index] * textureFit,
        );
        cheeks.endFill();
      }
      if (anime) {
        cheeks.lineStyle(Math.max(1, width * 0.006 * textureFit), 0xcf466b, 0.72);
        for (const offset of [-0.045, 0, 0.045]) {
          const x = originX + (cheekX + width * offset) * textureFit;
          const y = originY + cheekY * textureFit;
          cheeks.moveTo(x - width * 0.012 * textureFit, y - cheekHeight * 0.6 * textureFit);
          cheeks.lineTo(x + width * 0.012 * textureFit, y + cheekHeight * 0.6 * textureFit);
        }
        cheeks.lineStyle(0);
      }
    }
    cheeks.alpha = 0;
    app.stage.addChild(cheeks);
    return cheeks;
  }

  // 顔が一枚のパックでも眉・頬・口角を同じ座標場で動かす。
  // 重なる差分にも同じ変形を適用し、二重線や切り貼りの境界を作らない。
  function expressionDisplacement(rig, x, y, expression, includeMouth) {
    if (!expression.browY && !expression.browInnerY && !expression.mouthForm && !expression.mouthCurve) return [0, 0];
    const legacy = rig.drawables?.face?.textureBox;
    const face = rig.anchors?.face || (legacy && { x0: legacy[0], y0: legacy[1], x1: legacy[2], y1: legacy[3] });
    if (!face) return [0, 0];
    const w = face.x1 - face.x0, h = face.y1 - face.y0;
    const cx = (face.x0 + face.x1) / 2;
    const weight = (px, py, rx, ry) => {
      const q = ((x - px) / rx) ** 2 + ((y - py) / ry) ** 2;
      return q < 1 ? (1 - q) ** 2 : 0;
    };
    let dx = 0, dy = 0;
    for (const [side, direction] of [["L", -1], ["R", 1]]) {
      const eye = rig.anchors?.[`eye${side}`];
      const ex = eye ? (eye.x0 + eye.x1) / 2 : cx + direction * w * 0.20;
      const ey = eye ? (eye.y0 + eye.y1) / 2 : face.y0 + h * (direction < 0 ? 0.45 : 0.40);
      const ew = eye ? eye.x1 - eye.x0 : w * 0.26;
      const eh = eye ? eye.y1 - eye.y0 : h * 0.055;
      const inner = clamp(0.5 - direction * (x - ex) / ew, 0, 1);
      const browWeight = weight(ex, ey - eh * 0.95, ew * 0.85, eh * 1.15);
      dy += (expression.browY + expression.browInnerY * inner) * (w / 300) * browWeight;
      if (!includeMouth) {
        dy += expression.browInnerY * (inner - 0.5) * (w / 300) * 0.35
          * weight(ex, ey, ew * 0.8, eh * 1.1);
      }
      if (includeMouth) {
        // 実写は閉眼画像を薄く重ねず、元の虹彩を保って笑った下まぶたを持ち上げる。
        dy -= Math.max(0, expression.mouthForm) * h * 0.014
          * weight(ex, ey + eh * 0.45, ew * 0.8, eh * 0.9);
      }
    }
    if (includeMouth) {
      const mx = cx, my = face.y0 + h * 0.685;
      for (const direction of [-1, 1]) {
        const corner = weight(mx + direction * w * 0.14, my, w * 0.16, h * 0.09);
        dx += direction * expression.mouthForm * w * 0.020 * corner;
        dy -= expression.mouthCurve * h * (expression.mouthCurve < 0 ? 0.022 : 0.035) * corner;
      }
    }
    return [dx, dy];
  }

  class MeshRigController {
    constructor(app, canvas, rig, planes, lvsFace = null, blush = null) {
      this.app = app;
      this.canvas = canvas;
      this.rig = rig;
      this.planes = planes;
      this.lvsFace = lvsFace;
      this.blush = blush;
      this.active = false;
      this.mouth = 0;
      this.mouthForm = 0;
      this.armClose = clamp(rig.parameters?.ParamArmClose?.[2] ?? 0, 0, 1);
      this.eyeOpen = 1;
      this.eyeOpenL = 1;
      this.eyeOpenR = 1;
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
      this.secondaryHair = rig.secondaryMotion?.hair || null;
      this.motion = "idle";
      this.expression = "neutral";
      this.motionStartedAt = performance.now();
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

    setMouth(value, form = this.mouthForm) {
      this.mouth = clamp(value, 0, 1);
      this.mouthForm = clamp(form, -1, 1);
    }

    setArmClose(value) { this.armClose = clamp(value, 0, 1); }

    setEyeOpen(left, right = left) {
      this.eyeOpenL = clamp(left, 0, 1);
      this.eyeOpenR = clamp(right, 0, 1);
      // 旧形式は左右共通の閉眼差分だけを持つため、小さい側へ合わせる。
      this.eyeOpen = Math.min(this.eyeOpenL, this.eyeOpenR);
      this.blinkTween = null;
    }

    setBlink(target, duration = 0) {
      const next = clamp(target, 0, 1);
      if (duration <= 0) {
        this.setEyeOpen(next, next);
        return;
      }
      this.blinkTween = {
        fromL: this.eyeOpenL,
        fromR: this.eyeOpenR,
        to: next,
        startedAt: performance.now(),
        duration: Math.max(1, Number(duration) || 1),
      };
    }

    setGaze(x, y = 0) {
      this.gazeTargetX = clamp(x, -1, 1);
      this.gazeTargetY = clamp(y, -1, 1);
    }

    setExpression(name) {
      this.expression = String(name || "neutral").toLowerCase();
    }

    setMotion(name) {
      const next = String(name || "idle");
      if (/shy/.test(next)) this.expression = "shy";
      else if (/success|pleased|happy/.test(next)) this.expression = "happy";
      else if (/sad|concern|fail|error/.test(next)) this.expression = "concern";
      else if (/think|search|work/.test(next)) this.expression = "thinking";
      else this.expression = "neutral";
      if (this.motion === next) return;
      this.motion = next;
      this.motionStartedAt = performance.now();
    }

    expressionPose() {
      if (this.expression === "shy") {
        return { eye: 1, mouthOpen: 0.04, mouthForm: 0.5, mouthCurve: 0.65, browY: 2, browInnerY: -12, blush: 1 };
      }
      if (this.expression === "happy") {
        return { eye: 1, mouthOpen: 0.30, mouthForm: 1, mouthCurve: 1, browY: -12, browInnerY: 0, blush: 0 };
      }
      if (this.expression === "concern") {
        return { eye: 1, mouthOpen: 0, mouthForm: 0, mouthCurve: -1.6, browY: 8, browInnerY: -28, blush: 0 };
      }
      if (this.expression === "thinking") {
        return { eye: 1, mouthOpen: 0, mouthForm: -0.15, mouthCurve: -0.65, browY: 7, browInnerY: 7, blush: 0 };
      }
      return { eye: 1, mouthOpen: 0, mouthForm: 0, mouthCurve: 0, browY: 0, browInnerY: 0, blush: 0 };
    }

    anime25dMotionPose(now) {
      const elapsed = (now - this.motionStartedAt) / 1000;
      const motion = this.motion.toLowerCase();
      const pose = { bodyX: 0, bodyY: 0, bodyAngle: 0, headX: 0, headY: 0, headAngle: 0 };
      if (/talk|review/.test(motion)) {
        pose.bodyX = Math.sin(elapsed * 3.2) * 1.8;
        pose.bodyY = -Math.abs(Math.sin(elapsed * 3.8)) * 3.8;
        pose.bodyAngle = Math.sin(elapsed * 2.4) * 0.75;
        pose.headY = -Math.abs(Math.sin(elapsed * 4.6 + 0.5)) * 2.2;
        pose.headAngle = Math.sin(elapsed * 4.1 + 0.4) * 1.35;
      } else if (/think|search|work/.test(motion)) {
        pose.bodyX = Math.sin(elapsed * 1.3) * 1.2;
        pose.bodyY = Math.sin(elapsed * 1.6) * 0.8;
        pose.bodyAngle = -0.9 + Math.sin(elapsed * 1.1) * 0.45;
        pose.headX = 1.8;
        pose.headY = 1.2 + Math.sin(elapsed * 1.7) * 0.7;
        pose.headAngle = 3.2 + Math.sin(elapsed * 1.4) * 1.0;
      } else if (/success|pleased|happy|shy/.test(motion)) {
        pose.bodyX = Math.sin(elapsed * 3.1) * 2.6;
        pose.bodyY = -Math.abs(Math.sin(elapsed * 3.1)) * 8.5;
        pose.bodyAngle = Math.sin(elapsed * 2.1) * 1.6;
        pose.headY = -Math.abs(Math.sin(elapsed * 3.1 + 0.35)) * 3.2;
        pose.headAngle = -Math.sin(elapsed * 2.1) * 2.4;
      } else if (/sad|concern|fail|error/.test(motion)) {
        pose.bodyY = 4.5 + Math.sin(elapsed * 1.2) * 0.8;
        pose.bodyAngle = -0.7 + Math.sin(elapsed * 0.8) * 0.35;
        pose.headY = 4.2 + Math.sin(elapsed * 1.1) * 0.8;
        pose.headAngle = -2.8 + Math.sin(elapsed * 0.9) * 0.7;
      } else if (/listen/.test(motion)) {
        pose.bodyY = -Math.abs(Math.sin(elapsed * 2.2)) * 2.4;
        pose.headX = Math.sin(elapsed * 1.9) * 1.2;
        pose.headY = -1.8 + Math.sin(elapsed * 2.1) * 0.8;
        pose.headAngle = Math.sin(elapsed * 1.7) * 1.5;
      }
      return pose;
    }

    updateBlink(now) {
      if (!this.blinkTween) return;
      const progress = clamp((now - this.blinkTween.startedAt) / this.blinkTween.duration, 0, 1);
      const eased = progress * progress * (3 - 2 * progress);
      this.eyeOpenL = this.blinkTween.fromL
        + (this.blinkTween.to - this.blinkTween.fromL) * eased;
      this.eyeOpenR = this.blinkTween.fromR
        + (this.blinkTween.to - this.blinkTween.fromR) * eased;
      this.eyeOpen = Math.min(this.eyeOpenL, this.eyeOpenR);
      if (progress >= 1) this.blinkTween = null;
    }

    updateAlphas() {
      const expression = this.expressionPose();
      const mouthOpen = Math.max(this.mouth, expression.mouthOpen);
      if (this.lvsFace) return;
      if (["anime25d-v1", "lvs-anime25d-v1"].includes(this.rig.mode)) {
        for (const entry of this.planes.values()) {
          const fade = entry.item?.fade;
          const side = String(entry.item?.side || "").toUpperCase();
          const rawEyeOpen = side === "L" ? this.eyeOpenL : side === "R" ? this.eyeOpenR : this.eyeOpen;
          const eyeOpen = rawEyeOpen * expression.eye;
          let alpha = 1;
          if (fade === "mouthOpen") alpha = smoothstep(0.08, 0.34, mouthOpen);
          if (fade === "mouthClose") alpha = 1 - smoothstep(0.08, 0.34, mouthOpen);
          if (fade === "eyeOpen") alpha = smoothstep(0.18, 0.48, eyeOpen);
          if (fade === "eyeClose") alpha = 1 - smoothstep(0.18, 0.48, eyeOpen);
          entry.plane.alpha = alpha;
        }
        return;
      }
      const mouth = this.planes.get("mouth_open");
      if (mouth) mouth.plane.alpha = mouthOpen;
      const eyeOpen = this.eyeOpen * expression.eye;
      const gazeLeft = this.planes.get("gaze_left");
      const gazeRight = this.planes.get("gaze_right");
      // 閉眼中に開いた瞳の視線差分を重ねない。半目から開眼までだけ滑らかに戻す。
      const gazeVisibility = smoothstep(0.5, 1, eyeOpen);
      if (gazeLeft) gazeLeft.plane.alpha = Math.max(0, -this.gazeX) * gazeVisibility;
      if (gazeRight) gazeRight.plane.alpha = Math.max(0, this.gazeX) * gazeVisibility;
      const happy = this.planes.get("expression_happy");
      const sad = this.planes.get("expression_sad");
      if (happy) happy.plane.alpha = this.expression === "happy"
        && this.rig.expressionHappyMode !== "disabled" ? 1 : 0;
      if (sad) sad.plane.alpha = this.expression === "concern" && this.rig.expressionSadMode !== "disabled" ? 1 : 0;
      const half = this.planes.get("blink_half");
      const closed = this.planes.get("blink_closed");
      if (eyeOpen >= 0.5) {
        if (half) half.plane.alpha = (1 - eyeOpen) * 2;
        if (closed) closed.plane.alpha = 0;
      } else {
        if (half) half.plane.alpha = 1;
        if (closed) closed.plane.alpha = 1 - eyeOpen * 2;
      }
    }

    updateLvsAppearance() {
      const face = this.lvsFace;
      if (!face) return;
      const expression = this.expressionPose();
      const mouthOpen = Math.max(this.mouth, expression.mouthOpen);
      const mouthForm = this.mouth > 0.05 ? this.mouthForm : expression.mouthForm;
      const eyeL = this.eyeOpenL * expression.eye;
      const eyeR = this.eyeOpenR * expression.eye;
      const key = [eyeL, eyeR, mouthOpen, mouthForm, expression.mouthCurve, this.gazeX, this.gazeY]
        .map(value => value.toFixed(3)).join(",");
      if (key === face.lastKey) return;
      face.lastKey = key;
      const box = this.rig.sourceRig.layers.scene_face.texture_box;
      const { ctx, canvas, images, sourceAlpha } = face;
      ctx.save();
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.translate(-box[0], -box[1]);
      ctx.drawImage(images.get("scene_face"), box[0], box[1]);
      drawBlink(ctx, images, this.rig.sourceRig, "left", eyeL, this.gazeX, this.gazeY);
      drawBlink(ctx, images, this.rig.sourceRig, "right", eyeR, this.gazeX, this.gazeY);
      if (mouthOpen > 0 || mouthForm !== 0 || expression.mouthCurve !== 0) {
        const mouthBox = this.rig.sourceRig.layers.mouth_open.texture_box;
        ctx.drawImage(images.get("mouth_open"), mouthBox[0], mouthBox[1]);
        drawTexturedMouth(ctx, images.get("mouth_closed"), this.rig.sourceRig.layers.mouth_closed, mouthOpen, mouthForm, this.rig.sourceRig.layers.mouth_open.line_color, expression.mouthCurve);
      }
      ctx.restore();
      const frame = ctx.getImageData(0, 0, canvas.width, canvas.height);
      for (let index = 0; index < sourceAlpha.length; index += 1) frame.data[index * 4 + 3] = sourceAlpha[index];
      ctx.putImageData(frame, 0, 0);
      face.texture.baseTexture.update();
    }

    updateAnime25dMesh(now, dt) {
      const anchors = this.rig.anchors || {};
      const face = anchors.face || { cx: this.rig.canvas[0] / 2, cy: this.rig.canvas[1] * 0.28,
        x0: 0, x1: this.rig.canvas[0], y0: 0, y1: this.rig.canvas[1] * 0.5 };
      const neck = anchors.neckPivot || { cx: face.cx, cy: face.y1 };
      const mouth = anchors.mouth || { cx: face.cx, cy: face.cy + 40, x0: face.cx - 20,
        x1: face.cx + 20, y0: face.cy + 35 };
      const faceScale = Number(anchors.faceScale || 1);
      const elapsed = (now - this.startedAt) / 1000;
      const motionPose = this.anime25dMotionPose(now);
      const expression = this.expressionPose();
      const mouthOpen = Math.max(this.mouth, expression.mouthOpen);
      const mouthForm = this.mouth > 0.05 ? this.mouthForm : expression.mouthForm;
      const bodyPivot = anchors.bodyPivot || { cx: neck.cx, cy: this.rig.canvas[1] };
      const torso = anchors.torso || { x0: 0, y0: neck.cy, x1: this.rig.canvas[0], y1: this.rig.canvas[1] };
      this.gazeX += (this.gazeTargetX - this.gazeX) * Math.min(1, dt * 8);
      this.gazeY += (this.gazeTargetY - this.gazeY) * Math.min(1, dt * 8);
      const targetAngleX = clamp(
        Math.sin(elapsed * 0.83) * 0.78 + Math.sin(elapsed * 0.37 + 1.1) * 0.36
          + this.gazeX * 3.4,
        -6,
        6,
      );
      const targetAngleY = clamp(Math.sin(elapsed * 0.61 + 0.4) * 0.55 + this.gazeY * 2.2, -4, 4);
      this.angleX += (targetAngleX - this.angleX) * Math.min(1, dt * 4.5);
      this.angleY += (targetAngleY - this.angleY) * Math.min(1, dt * 4.5);
      const breath = 0.5 + 0.5 * Math.sin(elapsed * Math.PI * 2 / 3.4);
      const chestPulse = Math.sin(elapsed * Math.PI * 2 / 3.4);
      if (this.secondaryHair) {
        const next = springStep(
          { value: this.hairPosition, velocity: this.hairVelocity },
          this.angleX,
          dt,
          this.secondaryHair.frequency_hz,
          this.secondaryHair.damping_ratio,
        );
        this.hairPosition = next.value;
        this.hairVelocity = next.velocity;
      } else {
        // フォールバック許可: 物理定義を持たない既存リグは従来の髪追従を保つ。
        this.hairVelocity += (-this.hairPosition * 17 - this.hairVelocity * 5.4
          - (this.angleX - this.previousAngleX) * 7.5) * dt;
        this.hairPosition += this.hairVelocity * dt;
      }
      const mouthHalfWidth = Math.max(4, (mouth.x1 - mouth.x0) / 2);

      for (const entry of this.planes.values()) {
        const item = entry.item || {};
        const name = String(entry.name || "").replace(/[_-](l|r)$/i, "").toLowerCase();
        const side = String(item.side || "").toUpperCase();
        const isHair = item.phys === "hair";
        const isArm = /arm|handwear/.test(name);
        const eye = side === "L" ? anchors.eyeL : side === "R" ? anchors.eyeR : null;
        const rawEyeOpen = side === "L" ? this.eyeOpenL : side === "R" ? this.eyeOpenR : this.eyeOpen;
        const eyeOpen = rawEyeOpen * expression.eye;
        const data = entry.buffer.data;
        for (let index = 0; index < entry.original.length; index += 2) {
          let x = entry.original[index] + entry.offset[0];
          let y = entry.original[index + 1] + entry.offset[1];
          const originalX = x;
          const originalY = y;
          if (item.role === "face" || item.role === "residual") {
            const displacement = expressionDisplacement(this.rig, x, y, expression, false);
            x += displacement[0]; y += displacement[1];
          }

          if (item.fade === "eyeOpen" && eye) {
            if (name === "irides") {
              x += this.gazeX * 11 * faceScale;
              y += this.gazeY * 6 * faceScale;
              const closing = smoothstep(0, 0.32, 0.32 - eyeOpen);
              y = eye.closeY + (y - eye.closeY) * (1 - 0.8 * closing);
            } else {
              y = eye.closeY + (y - eye.closeY) * (0.15 + 0.85 * eyeOpen);
            }
          }
          if (item.fade === "eyeClose" && eye) y -= eyeOpen * 3;
          if (item.fade === "mouthOpen") {
            const formWidth = 1 + mouthForm * 0.36;
            x = mouth.cx + (x - mouth.cx) * formWidth;
            y = mouth.y0 + (y - mouth.y0) * (0.5 + 0.5 * mouthOpen);
            const q = Math.pow(Math.abs(x - mouth.cx) / (mouthHalfWidth + 4), 1.5);
            y -= mouthForm * 6 * faceScale * (q - 0.35);
          }
          if (item.fade === "mouthClose") {
            const formWidth = 1 + mouthForm * 0.20;
            x = mouth.cx + (x - mouth.cx) * formWidth;
            const q = Math.pow(Math.abs(x - mouth.cx) / (mouthHalfWidth + 4), 1.5);
            y -= mouthForm * 8 * faceScale * (q - 0.28);
          }
          if (name === "face" && y > mouth.cy) {
            y += mouthOpen * 6 * faceScale
              * smoothstep(0, 1, (y - mouth.cy) / Math.max(1, face.y1 - mouth.cy));
          }

          if (name === "eyebrow") {
            const innerWeight = 1 - clamp(Math.abs(x - face.cx) / Math.max(1, face.x1 - face.x0), 0, 1);
            y += (expression.browY + expression.browInnerY * innerWeight) * faceScale;
          }
          // 顔・首・襟・胴へ同じ呼吸移動を適用し、境界を開かせない。
          y -= breath * 2 * faceScale;

          // 全身の横揺れを増やさず、上胸部だけに小さな吸気変形を加える。
          // torso矩形の四辺では重みを0へ戻し、首・袖・下腹との継ぎ目を開かせない。
          if (item.role === "torso") {
            const torsoWidth = Math.max(1, torso.x1 - torso.x0);
            const torsoHeight = Math.max(1, torso.y1 - torso.y0);
            const horizontalWeight = Math.sin(Math.PI * clamp((originalX - torso.x0) / torsoWidth, 0, 1));
            const verticalPosition = clamp((originalY - torso.y0) / torsoHeight, 0, 1);
            const verticalWeight = verticalPosition < 0.58
              ? Math.sin(Math.PI * verticalPosition / 0.58)
              : 0;
            const chestWeight = horizontalWeight * verticalWeight;
            x = neck.cx + (x - neck.cx) * (1 + chestPulse * 0.004 * chestWeight);
            y -= chestPulse * 2.4 * chestWeight;
          }

          if (isHair) {
            const itemBox = item.textureBox || [item.x, item.y, item.w, item.h];
            const rootY = Math.min(...(item.strands || []).map(strand => strand.rootY), itemBox[1]);
            const tipY = Math.max(...(item.strands || []).map(strand => strand.tipY), itemBox[1] + itemBox[3]);
            const weight = Math.pow(clamp((originalY - rootY) / Math.max(1, tipY - rootY), 0, 1), 1.8);
            if (this.secondaryHair) {
              x += this.hairPosition * this.secondaryHair.strength * this.rig.canvas[0] * 0.0022 * weight;
              y += Math.abs(this.hairPosition) * this.secondaryHair.strength * this.rig.canvas[1] * 0.00035 * weight;
            } else {
              x += this.hairPosition * weight * (name === "front hair" ? 3.2 : 4.4);
              y += Math.abs(this.hairPosition) * weight * 0.28;
            }
          }

          if (isArm && this.armClose > 0) {
            const itemBox = item.textureBox || [item.x, item.y, item.w, item.h];
            const rightOfBody = itemBox[0] + itemBox[2] / 2 > neck.cx;
            const pivotX = rightOfBody ? itemBox[0] + itemBox[2] * 0.10 : itemBox[0] + itemBox[2] * 0.90;
            const pivotY = itemBox[1] + itemBox[3] * 0.08;
            const angle = this.armClose * 0.17 * (rightOfBody ? 1 : -1);
            const cosine = Math.cos(angle);
            const sine = Math.sin(angle);
            const relativeX = x - pivotX;
            const relativeY = y - pivotY;
            x = pivotX + relativeX * cosine - relativeY * sine;
            y = pivotY + relativeX * sine + relativeY * cosine;
          }
          const bodyRadians = motionPose.bodyAngle * Math.PI / 180;
          const bodyCosine = Math.cos(bodyRadians);
          const bodySine = Math.sin(bodyRadians);
          const bodyRelativeX = x - bodyPivot.cx;
          const bodyRelativeY = y - bodyPivot.cy;
          x = bodyPivot.cx + bodyRelativeX * bodyCosine - bodyRelativeY * bodySine + motionPose.bodyX;
          y = bodyPivot.cy + bodyRelativeX * bodySine + bodyRelativeY * bodyCosine + motionPose.bodyY;
          data[index] = x - entry.offset[0];
          data[index + 1] = y - entry.offset[1];
          if (!Number.isFinite(data[index]) || !Number.isFinite(data[index + 1])) {
            data[index] = originalX - entry.offset[0];
            data[index + 1] = originalY - entry.offset[1];
          }
        }
        entry.buffer.update();
      }
    }

    updateMesh(now, dt) {
      if (["anime25d-v1", "lvs-anime25d-v1"].includes(this.rig.mode)) {
        this.updateAnime25dMesh(now, dt);
        this.previousAngleX = this.angleX;
        return;
      }
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
      const expression = this.expressionPose();

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
          const motion = entry.motion || "sheet";
          const isHair = motion === "hair-back" || motion === "hair-front";
          const isHead = isHair || motion === "head" || motion === "face" || motion === "sheet";
          const isTorso = motion === "torso" || motion === "sheet";
          const armDirection = motion === "arm-left" ? -1 : motion === "arm-right" ? 1 : 0;
          const horizontal = (isHead ? head * this.angleX * (0.82 + (0.5 - nx) * 0.22) : 0)
            + (isHair ? hairEdge * this.hairPosition * (motion === "hair-front" ? 13 : 18) : 0)
            + armDirection * breath * 1.15;
          const vertical = (isHead ? head * this.angleY * 0.34 : 0)
            - (isTorso ? torso * breath * 2.4 : 0)
            - (armDirection ? torso * breath * 1.2 : 0);
          const facial = motion === "face" || motion === "sheet"
            ? expressionDisplacement(this.rig, globalX, globalY, expression, true) : [0, 0];
          data[index] = x + horizontal + facial[0];
          data[index + 1] = y + vertical + facial[1];
        }
        entry.buffer.update();
      }
    }

    updateBlush() {
      if (!this.blush) return;
      this.blush.alpha = this.expressionPose().blush;
      this.blush.position.set(this.angleX * 0.45, this.angleY * 0.20);
    }

    tick() {
      if (!this.active) return;
      const now = performance.now();
      const dt = this.lastAt ? clamp((now - this.lastAt) / 1000, 1 / 240, 1 / 30) : 1 / 60;
      this.lastAt = now;
      this.updateBlink(now);
      this.updateLvsAppearance();
      this.updateAlphas();
      this.updateMesh(now, dt);
      this.updateBlush();
    }

    destroy() {
      this.app.ticker.remove(this.tick);
      this.app.destroy(false, { children: true, texture: true, baseTexture: true });
      if (this.lvsFace) {
        this.lvsFace.images.clear();
        this.lvsFace.canvas.width = 0;
        this.lvsFace.canvas.height = 0;
        if (this.lvsFace.residualCanvas) {
          this.lvsFace.residualCanvas.width = 0;
          this.lvsFace.residualCanvas.height = 0;
        }
      }
      this.canvas.remove();
    }
  }

  function lvsBox(layer) {
    const box = layer?.texture_box;
    if (!Array.isArray(box) || box.length !== 4 || !box.every(Number.isFinite) || box[2] <= box[0] || box[3] <= box[1]) {
      throw new Error("LocalVTuberStudio素材の範囲が不正です");
    }
    return [box[0], box[1], box[2] - box[0], box[3] - box[1]];
  }

  function lvsPartPath(name) {
    if (!/^[a-z0-9_]+$/.test(name)) throw new Error(`LocalVTuberStudio素材名が不正です: ${name}`);
    return `parts/${name}.png`;
  }

  async function normalizeLvsRig(sourceRig, rootUrl, assetVersion) {
    if (sourceRig.schema_version !== 3 || sourceRig.profile !== "lvs-anime25d-v1"
      || sourceRig.eye_rig_version !== 3 || sourceRig.lip_rig_version !== 1 || sourceRig.scene_graph_version !== 1) {
      throw new Error("独立部位または目口の素材がないLocalVTuberStudioリグです");
    }
    const width = Number(sourceRig.canvas?.width), height = Number(sourceRig.canvas?.height);
    if (!(width > 0 && height > 0)) throw new Error("LocalVTuberStudioリグのキャンバス寸法が不正です");
    const graph = sourceRig.scene_graph;
    const secondaryHair = validateLvsSecondaryMotion(sourceRig);
    if (!Array.isArray(graph) || graph.length < 6 || !graph.some(item => item.role === "face")) {
      throw new Error("LocalVTuberStudioリグの独立部位構造が不正です");
    }
    const faceLayer = sourceRig.layers?.scene_face;
    const required = new Set([
      ...graph.map(item => item.layer), "scene_face", "mouth_open", "mouth_closed",
      ...["left", "right"].flatMap(side => [`${side}_eye_base`, `${side}_eye_closed`, `${side}_eyelid_upper`, `${side}_eye_backplate`, `${side}_eye_iris`]),
    ]);
    for (const name of required) {
      if (!sourceRig.layers?.[name]) throw new Error(`LocalVTuberStudioの必須素材がありません: ${name}`);
    }
    lipMesh(sourceRig.layers.mouth_closed, 0, 0);
    eyeAperture(sourceRig.layers.left_eye_base, 1);
    eyeAperture(sourceRig.layers.right_eye_base, 1);
    const images = new Map(await Promise.all([...required].map(async name => {
      const image = await loadImage(versioned(`${rootUrl}/${lvsPartPath(name)}`, assetVersion));
      const [x, y, partWidth, partHeight] = lvsBox(sourceRig.layers[name]);
      if (image.naturalWidth !== partWidth || image.naturalHeight !== partHeight) {
        throw new Error(`LocalVTuberStudio素材の寸法が一致しません: ${name} (${x},${y})`);
      }
      return [name, image];
    })));
    const faceBox = faceLayer.texture_box;
    const faceCanvas = document.createElement("canvas");
    faceCanvas.width = faceBox[2] - faceBox[0];
    faceCanvas.height = faceBox[3] - faceBox[1];
    const faceContext = faceCanvas.getContext("2d", { willReadFrequently: true });
    if (!faceContext) throw new Error("LocalVTuberStudioの顔Canvasを初期化できません");
    faceContext.drawImage(images.get("scene_face"), 0, 0);
    const sourceAlpha = new Uint8Array(faceCanvas.width * faceCanvas.height);
    const pixels = faceContext.getImageData(0, 0, faceCanvas.width, faceCanvas.height).data;
    for (let index = 0; index < sourceAlpha.length; index += 1) sourceAlpha[index] = pixels[index * 4 + 3];
    const faceTexture = PIXI.Texture.from(faceCanvas);
    await waitTexture(faceTexture);

    // 境界の隙間防止用scene_residualには、意味分離で共有した腕の輪郭画素も残る。
    // 腕を動かすと元位置の輪郭だけが残像になるため、可動腕の原位置を同じRGBAで抜く。
    const residualLayer = sourceRig.layers.scene_residual;
    const armNodes = graph.filter(node => node.role === "left_arm" || node.role === "right_arm");
    let residualCanvas = null;
    let residualTexture = null;
    if (residualLayer && armNodes.length) {
      const [residualX, residualY, residualWidth, residualHeight] = lvsBox(residualLayer);
      residualCanvas = document.createElement("canvas");
      residualCanvas.width = residualWidth;
      residualCanvas.height = residualHeight;
      const residualContext = residualCanvas.getContext("2d");
      if (!residualContext) throw new Error("LocalVTuberStudioの残余Canvasを初期化できません");
      residualContext.drawImage(images.get("scene_residual"), 0, 0);
      residualContext.globalCompositeOperation = "destination-out";
      const armClearance = Math.max(2, Math.round(Math.min(width, height) * 0.004));
      for (const node of armNodes) {
        const [armX, armY] = lvsBox(sourceRig.layers[node.layer]);
        for (const offsetX of [-armClearance, 0, armClearance]) {
          for (const offsetY of [-armClearance, 0, armClearance]) {
            residualContext.drawImage(
              images.get(node.layer),
              armX - residualX + offsetX,
              armY - residualY + offsetY,
            );
          }
        }
      }
      residualContext.globalCompositeOperation = "source-over";
      residualTexture = PIXI.Texture.from(residualCanvas);
      await waitTexture(residualTexture);
    }

    const face = sourceRig.layers.face.bbox;
    const neck = sourceRig.layers.scene_neck.bbox;
    const torso = sourceRig.layers.scene_torso.bbox;
    const eyeAnchor = side => {
      const box = sourceRig.layers[`${side}_eye_base`].feature_box || sourceRig.layers[`${side}_eye_base`].bbox;
      return { x0: box[0], y0: box[1], x1: box[2], y1: box[3], closeY: (box[1] + box[3]) / 2 };
    };
    const mouth = sourceRig.layers.mouth_closed.feature_box;
    const normalized = {
      version: 3,
      mode: "lvs-anime25d-v1",
      canvas: [width, height],
      contentBox: sourceRig.layers.neutral?.bbox || [0, 0, width, height],
      sourceRig,
      secondaryMotion: secondaryHair ? { hair: secondaryHair } : null,
      anchors: {
        face: { x0: face[0], y0: face[1], x1: face[2], y1: face[3], cx: (face[0] + face[2]) / 2, cy: (face[1] + face[3]) / 2 },
        eyeL: eyeAnchor("left"), eyeR: eyeAnchor("right"),
        mouth: { x0: mouth[0], y0: mouth[1], x1: mouth[2], y1: mouth[3], cx: (mouth[0] + mouth[2]) / 2, cy: (mouth[1] + mouth[3]) / 2 },
        neckPivot: { cx: (neck[0] + neck[2]) / 2, cy: neck[3] },
        torso: { x0: torso[0], y0: torso[1], x1: torso[2], y1: torso[3] },
        bodyPivot: { cx: (torso[0] + torso[2]) / 2, cy: height },
        faceScale: Math.max(0.25, (face[2] - face[0]) / 300),
      },
      layers: graph.map((node, index) => {
        const layer = sourceRig.layers[node.layer];
        return {
          name: node.layer,
          texture: lvsPartPath(node.layer),
          textureBox: lvsBox(layer),
          z: index,
          initialAlpha: 1,
          motion: node.role,
          phys: node.role === "hair" ? "hair" : null,
          role: node.role,
          dynamicTexture: node.role === "face" ? faceTexture
            : node.role === "residual" ? residualTexture : null,
        };
      }),
    };
    return { rig: normalized, face: {
      canvas: faceCanvas, ctx: faceContext, images, sourceAlpha, texture: faceTexture,
      residualCanvas, lastKey: "",
    } };
  }

  async function create({ canvas, rigUrl, rootUrl, assetVersion, width, height, contentBox: requestedContentBox }) {
    if (!root.PIXI?.Application || !root.PIXI?.SimplePlane) {
      throw new Error("PIXI 6のメッシュ描画機能がありません");
    }
    const response = await fetch(versioned(rigUrl, assetVersion));
    if (!response.ok) throw new Error(`rig.jsonを取得できません（HTTP ${response.status}）`);
    const incomingRig = await response.json();
    const lvs = incomingRig?.schema_version === 3 && incomingRig?.profile === "lvs-anime25d-v1"
      ? await normalizeLvsRig(incomingRig, rootUrl, assetVersion) : null;
    const rig = lvs?.rig || incomingRig;
    const legacy = rig?.version === 1 && ["single-sheet-grid-v1", "layered-grid-v2"].includes(rig?.mode);
    const anime25d = rig?.version === 2 && rig?.mode === "anime25d-v1";
    const lvsAnime25d = rig?.version === 3 && rig?.mode === "lvs-anime25d-v1";
    if (!legacy && !anime25d && !lvsAnime25d) {
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
    const contentBox = Array.isArray(requestedContentBox) && requestedContentBox.length === 4
      ? requestedContentBox
      : Array.isArray(rig.contentBox) && rig.contentBox.length === 4
        ? rig.contentBox
      : [0, 0, ...rig.canvas];
    const contentWidth = contentBox[2] - contentBox[0];
    const contentHeight = contentBox[3] - contentBox[1];
    if (contentWidth <= 0 || contentHeight <= 0) throw new Error("メッシュリグの内容範囲が不正です");
    // 透明余白や原画キャンバス比率が異なっても、見えるキャラクターの大きさを固定する。
    const textureFit = Math.min(width / contentWidth, height / contentHeight) * 0.97;
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
    const definitions = anime25d || lvsAnime25d
      ? (rig.layers || []).map((item, index) => [
        String(item.name || `part-${index}`), item.texture,
        item.textureBox || [item.x, item.y, item.w, item.h], item,
      ])
      : [
        ["base", rig.base, [0, 0, ...rig.canvas], { z: -10000, initialAlpha: 1, motion: "sheet" }],
        ...Object.entries(rig.drawables || {}).map(([name, item]) => [name, item.texture, item.textureBox || [0, 0, ...rig.canvas], item]),
      ];
    definitions.sort((left, right) => Number(left[3]?.z || 0) - Number(right[3]?.z || 0));
    const loaded = await Promise.all(definitions.map(async ([name, path, box, item]) => {
      const texture = item?.dynamicTexture || PIXI.Texture.from(versioned(`${rootUrl}/${path}`, assetVersion));
      await waitTexture(texture);
      return [name, texture, box, item];
    }));
    const planes = new Map();
    const originX = (width - contentWidth * textureFit) / 2 - contentBox[0] * textureFit;
    const originY = height - contentBox[3] * textureFit;
    for (const [name, texture, box, item] of loaded) {
      const mouthLayer = (anime25d || lvsAnime25d) && ["mouthOpen", "mouthClose"].includes(item?.fade);
      const facialLayer = item?.role === "face" || item?.role === "residual" || item?.motion === "face";
      const planeGridX = facialLayer ? 49 : mouthLayer
        ? 7
        : (anime25d || lvsAnime25d) ? clamp(Math.round(box[2] / (item?.phys ? 30 : 42)) + 1, 2, 32) : gridX;
      const planeGridY = facialLayer ? 65 : mouthLayer
        ? 3
        : (anime25d || lvsAnime25d) ? clamp(Math.round(box[3] / (item?.phys ? 30 : 42)) + 1, 2, 40) : gridY;
      const plane = new PIXI.SimplePlane(texture, planeGridX, planeGridY);
      plane.scale.set(textureFit);
      plane.position.set(originX + box[0] * textureFit, originY + box[1] * textureFit);
      plane.alpha = Number(item?.initialAlpha ?? (name === "base" ? 1 : 0));
      const buffer = plane.geometry.getBuffer("aVertexPosition");
      planes.set(name, {
        plane,
        buffer,
        original: new Float32Array(buffer.data),
        offset: [box[0], box[1]],
        motion: String(item?.motion || (name === "base" ? "sheet" : "face")),
        item,
        name,
      });
      app.stage.addChild(plane);
    }
    const blush = createBlushOverlay(app, rig, originX, originY, textureFit);
    canvas.style.background = "transparent";
    canvas.dataset.meshRig = "ready";
    return new MeshRigController(app, canvas, rig, planes, lvs?.face || null, blush);
  }

  root.PicoMeshRig = Object.freeze({ create });
})(window);
