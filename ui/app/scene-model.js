// scene-model.js - シーン（形式ごとの画面構成）の計算。DOM を触らない純粋な関数だけを置く。
//
// ソースの矩形は `anchor`（9点）と、シーンの同じアンカー点からのずれ `x` / `y`、大きさ `w` / `h`
// （シーンpx）で表す。倍率は持たない。既定値は `ui/scenes/<形式ID>.json`、利用者が動かした値は
// `PICOAGENT_SCENES` の差分で、ここで両者を重ねて実際の矩形にする。
// Node のテスト（tools/tests/scene-model-contract.mjs）からも読むため、ブラウザ専用のAPIを使わない。

export const ANCHORS = Object.freeze([
  "top-left", "top", "top-right",
  "left", "center", "right",
  "bottom-left", "bottom", "bottom-right",
]);
export const FRAMINGS = Object.freeze(["full", "upper", "bust"]);
export const MIN_SIZE = 48;
/** ウィンドウ内へ最低限残す見えしろ（掴み直せる幅）。 */
export const KEEP_VISIBLE = 48;

/** アンカー名 → シーン内の基準点の割合（0〜1）。 */
export function anchorFactors(anchor) {
  const name = ANCHORS.includes(anchor) ? anchor : "top-left";
  const fx = name.endsWith("left") ? 0 : name.endsWith("right") ? 1 : 0.5;
  const fy = name.startsWith("top") ? 0 : name.startsWith("bottom") ? 1 : 0.5;
  return { fx, fy };
}

/** `{ frac, min, max }` か数値の寸法指定を、基準の長さに対する px へ解決する。 */
export function resolveLength(spec, base, fallback) {
  if (Number.isFinite(spec)) return spec;
  if (!spec || typeof spec !== "object") return fallback;
  let value = Number.isFinite(spec.px) ? spec.px : Number.isFinite(spec.frac) ? base * spec.frac : fallback;
  if (Number.isFinite(spec.min)) value = Math.max(spec.min, value);
  if (Number.isFinite(spec.max)) value = Math.min(spec.max, value);
  return value;
}

/** アンカー基準の値（anchor / x / y / w / h）を、シーン左上基準の矩形へ変える。 */
export function toRect(placement, sceneWidth, sceneHeight) {
  const { fx, fy } = anchorFactors(placement.anchor);
  const width = Math.max(1, placement.w);
  const height = Math.max(1, placement.h);
  return {
    left: sceneWidth * fx + placement.x - width * fx,
    top: sceneHeight * fy + placement.y - height * fy,
    width,
    height,
  };
}

/** シーン左上基準の矩形を、指定したアンカー基準の値へ戻す（ドラッグ後の保存に使う）。 */
export function fromRect(rect, anchor, sceneWidth, sceneHeight) {
  const { fx, fy } = anchorFactors(anchor);
  return {
    anchor: ANCHORS.includes(anchor) ? anchor : "top-left",
    x: Math.round(rect.left + rect.width * fx - sceneWidth * fx),
    y: Math.round(rect.top + rect.height * fy - sceneHeight * fy),
    w: Math.round(Math.max(MIN_SIZE, rect.width)),
    h: Math.round(Math.max(MIN_SIZE, rect.height)),
  };
}

/** 矩形の中心に近いアンカーを選ぶ。動かした先で、窓の伸縮に付いていく辺を自然に決める。 */
export function nearestAnchor(rect, sceneWidth, sceneHeight) {
  const cx = (rect.left + rect.width / 2) / Math.max(1, sceneWidth);
  const cy = (rect.top + rect.height / 2) / Math.max(1, sceneHeight);
  const col = cx < 1 / 3 ? "left" : cx > 2 / 3 ? "right" : "";
  const row = cy < 1 / 3 ? "top" : cy > 2 / 3 ? "bottom" : "";
  if (row && col) return `${row}-${col}`;
  return row || col || "center";
}

/**
 * 既定定義と保存値を重ねて、1つのソースの配置（anchor / x / y / w / h / visible）を返す。
 * `aspect` は幅を省いた定義で高さから幅を決める比（キャラの構図の縦横比）。
 */
export function resolvePlacement(definition, saved, sceneWidth, sceneHeight, aspect = null) {
  const hasSavedRect = saved && ANCHORS.includes(saved.anchor)
    && [saved.x, saved.y, saved.w, saved.h].every(Number.isFinite);
  const visible = typeof saved?.visible === "boolean" ? saved.visible : definition.visible !== false;
  if (hasSavedRect) {
    return { anchor: saved.anchor, x: saved.x, y: saved.y, w: saved.w, h: saved.h, visible, saved: true };
  }
  let h = resolveLength(definition.h, sceneHeight, sceneHeight * 0.5);
  let w = definition.w == null && aspect ? h * aspect : resolveLength(definition.w, sceneWidth, sceneWidth * 0.5);
  // 幅を構図から決めた場合も、定義の上限（シーン幅に対する割合）を越えたら高さごと縮めて縦横比を保つ。
  const maxW = Number.isFinite(definition.maxWidthFrac) ? sceneWidth * definition.maxWidthFrac : Infinity;
  if (definition.w == null && aspect && w > maxW) {
    w = maxW;
    h = w / aspect;
  }
  return {
    anchor: ANCHORS.includes(definition.anchor) ? definition.anchor : "top-left",
    x: Number.isFinite(definition.x) ? definition.x : 0,
    y: Number.isFinite(definition.y) ? definition.y : 0,
    w: Math.max(MIN_SIZE, w),
    h: Math.max(MIN_SIZE, h),
    visible,
    saved: false,
  };
}

/** 矩形を窓の内側へ完全に収める（はみ出す分は位置で、それでも足りなければ大きさで詰める）。 */
export function containRect(rect, sceneWidth, sceneHeight, margin = 0) {
  const width = Math.min(rect.width, Math.max(1, sceneWidth - margin * 2));
  const height = Math.min(rect.height, Math.max(1, sceneHeight - margin * 2));
  const left = Math.min(Math.max(margin, rect.left), sceneWidth - margin - width);
  const top = Math.min(Math.max(margin, rect.top), sceneHeight - margin - height);
  return { left, top, width, height };
}

/** 矩形を、少なくとも `keep` px が窓の内側へ残る位置まで戻す（大きさは変えない）。 */
export function keepReachable(rect, sceneWidth, sceneHeight, keep = KEEP_VISIBLE) {
  const left = Math.min(Math.max(rect.left, keep - rect.width), sceneWidth - keep);
  const top = Math.min(Math.max(rect.top, keep - rect.height), sceneHeight - keep);
  return { ...rect, left, top };
}

/** 固定寸法のシーンを窓へ等倍率で収める倍率と余白（配信形式）。 */
export function fitScene(sceneWidth, sceneHeight, viewWidth, viewHeight) {
  const scale = Math.min(viewWidth / sceneWidth, viewHeight / sceneHeight);
  return {
    scale,
    offsetX: (viewWidth - sceneWidth * scale) / 2,
    offsetY: (viewHeight - sceneHeight * scale) / 2,
  };
}

/** 保存値の深い写し（編集中の作業用の写しと保存値を分けるため）。 */
export function cloneScenes(value) {
  return value && typeof value === "object" ? JSON.parse(JSON.stringify(value)) : { version: 1 };
}

/** 保存値の中の1つの形式の置き場所を返す（無ければ作る）。 */
export function layoutEntry(scenes, layoutId) {
  scenes[layoutId] ||= {};
  return scenes[layoutId];
}

/** ソースまたはキャラの保存値を書き換える。`value` が null なら消す（既定へ戻す）。 */
export function writeSaved(scenes, layoutId, target, value) {
  const layout = layoutEntry(scenes, layoutId);
  const bucket = target.characterId ? "characters" : "sources";
  const key = target.characterId || target.source;
  layout[bucket] ||= {};
  if (value == null) delete layout[bucket][key];
  else layout[bucket][key] = value;
  if (!Object.keys(layout[bucket]).length) delete layout[bucket];
  if (!Object.keys(layout).length) delete scenes[layoutId];
  return scenes;
}
