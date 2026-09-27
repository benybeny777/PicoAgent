// framing.js - 構図（描画範囲の切り出し）の決め方。描画器の種類に依らない共通の規則。
//
// 構図は倍率ではなく、モデル座標上の矩形。描画器はこの矩形をキャンバスへ縦横比を保って収めるので、
// どの構図・どの大きさでもキャンバスの外へ描くことが無い（キャラが切れない）。

export const FRAMING_NAMES = Object.freeze(["full", "upper", "bust"]);

/** 可視範囲の上側 `heightFrac` を、幅 `heightFrac × 高さ × widthPerHeight`（可視範囲の幅まで）で切り出す。 */
function topSlice(content, heightFrac, widthPerHeight) {
  const h = content.h * heightFrac;
  const w = Math.min(content.w, h * widthPerHeight);
  return { x: content.x + (content.w - w) / 2, y: content.y, w, h };
}

/**
 * 可視範囲（透明余白を除いた矩形）から3つの構図を作る。
 * 低頭身（compact）は頭の比率が大きいので、同じ名前でも深めに切り出す。
 * 原画がもともと胸上（native-bust）なら、全身＝上半身＝原画全体とする。
 */
export function framingRectsFor(visible, { proportion = "standard", sourceFraming = "default" } = {}) {
  // 可視範囲はアホ毛や指先ちょうどで切った矩形なので、上と左右へ少し余白を足してから切り出す。
  // 下端は足元（または原画の下端）なので足さない。これで髪先や手先が枠に接して切れて見えない。
  const content = padded(visible);
  if (sourceFraming === "native-bust") {
    return { full: content, upper: content, bust: topSlice(content, 0.78, 1.1) };
  }
  const compact = proportion === "compact";
  return {
    full: content,
    upper: topSlice(content, compact ? 0.82 : 0.62, compact ? 1.05 : 0.92),
    bust: topSlice(content, compact ? 0.62 : 0.4, compact ? 1.15 : 1.08),
  };
}

const PAD_TOP = 0.04;
const PAD_SIDE = 0.03;

function padded(rect) {
  const top = rect.h * PAD_TOP;
  const side = rect.h * PAD_SIDE;
  return { x: rect.x - side, y: rect.y - top, w: rect.w + side * 2, h: rect.h + top };
}

export function aspectOf(rect) {
  return rect && rect.h > 0 ? rect.w / rect.h : null;
}

/**
 * 保存値が無いときの構図。シーン定義に `defaultFraming` があればそれ（配信は胸上）、無ければ描画器の既定。
 * 原画がもともと胸上なら、どの形式でも原画全体（full）を使う（胸上をさらに切り出さない）。
 */
export function resolveFraming(stored, sceneDefault, pack, rendererDefault) {
  if (FRAMING_NAMES.includes(stored)) return stored;
  if (FRAMING_NAMES.includes(sceneDefault)) return pack?.sourceFraming === "native-bust" ? "full" : sceneDefault;
  return FRAMING_NAMES.includes(rendererDefault) ? rendererDefault : "upper";
}

/** 描画器の既定の構図（シーン定義に既定が無い形式で使う）。 */
export function defaultFramingFor(pack, kind) {
  if (pack?.sourceFraming === "native-bust") return "full";
  if (kind === "live2d") return "upper";
  if (kind === "mmd") return "bust";
  if (pack?.bodyProportion === "compact") return "upper";
  return "bust";
}

/** 切り出し矩形をキャンバス（cssW × cssH）へ収める写像。下端中央基準。 */
export function fitTransform(frame, cssWidth, cssHeight) {
  const scale = Math.min(cssWidth / frame.w, cssHeight / frame.h);
  return {
    scale,
    offsetX: (cssWidth - frame.w * scale) / 2 - frame.x * scale,
    offsetY: cssHeight - (frame.y + frame.h) * scale,
  };
}

/** 描画バッファの1辺の上限（GPU の最大テクスチャ寸法より十分小さく）。 */
export const MAX_BACKING_EDGE = 4096;

/** 描画バッファの画素密度。1辺が上限を越えるなら密度の側を下げる。 */
export function backingResolution(cssWidth, cssHeight, pixelRatio) {
  const ratio = Math.max(0.25, Number(pixelRatio) || 1);
  const edge = Math.max(cssWidth, cssHeight) * ratio;
  return edge > MAX_BACKING_EDGE ? ratio * (MAX_BACKING_EDGE / edge) : ratio;
}
