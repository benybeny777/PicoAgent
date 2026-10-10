// util.js - 画面モジュールが共有する小さな道具。状態は持たない。

const ui = window.PicoUI;

export const { errorDetail, errorMessage, logApp, preview, twoStepAction } = ui;

export function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

export function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/** 要素を作る。`props` は DOM プロパティ（`className` / `textContent` / `hidden` など）、`attrs` は属性。 */
export function el(tag, props = {}, children = [], attrs = {}) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined) continue;
    if (key === "dataset") Object.assign(node.dataset, value);
    else if (key === "style") Object.assign(node.style, value);
    else node[key] = value;
  }
  for (const [key, value] of Object.entries(attrs)) {
    if (value !== undefined && value !== null) node.setAttribute(key, String(value));
  }
  for (const child of [].concat(children)) {
    if (child == null || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

/** クリックの伝播を止めるボタンを作る（パネル内の操作が背後のクリック処理へ抜けないように）。 */
export function button(label, onClick, props = {}) {
  const node = el("button", { type: "button", textContent: label, ...props });
  node.addEventListener("click", event => {
    event.preventDefault();
    event.stopPropagation();
    onClick(event, node);
  });
  return node;
}

/** 日本語入力（IME）の変換中か。変換中のキーはアプリの操作へ流さない。 */
export function isImeComposing(event) {
  return event.isComposing === true || event.keyCode === 229;
}

export async function copyText(text) {
  const source = String(text ?? "");
  if (!source.trim()) return false;
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(source);
    return true;
  }
  const area = el("textarea", { value: source, style: { position: "fixed", left: "-9999px" } });
  document.body.append(area);
  area.select();
  let ok = false;
  try { ok = document.execCommand("copy"); }
  finally { area.remove(); }
  return ok;
}

export function blobToDataUrl(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

/** 上限まで待ち、間に合わなければ `timedOut` を返す（遅れて届いた結果は使わない）。 */
export async function waitWithLimit(promise, timeoutMs) {
  const TIMEOUT = Symbol("timeout");
  let timer = null;
  const limit = new Promise(resolve => { timer = setTimeout(() => resolve(TIMEOUT), timeoutMs); });
  try {
    const result = await Promise.race([promise, limit]);
    return result === TIMEOUT ? { timedOut: true, value: null } : { timedOut: false, value: result };
  } finally {
    if (timer) clearTimeout(timer);
  }
}

/** 指定時間内に応答しない fetch を打ち切って JSON を返す。 */
export async function fetchJson(url, label, timeoutMs = 4000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { cache: "no-store", signal: controller.signal });
    if (!response.ok) throw new Error(`${label}: HTTP ${response.status}`);
    return await response.json();
  } catch (error) {
    if (error?.name === "AbortError") throw new Error(`${label}が${timeoutMs}ミリ秒以内に応答しませんでした`);
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

/** 同じスクリプトを1度だけ読む。読み込み途中で失敗したら要素を外して再試行できるようにする。 */
const scriptLoads = new Map();
export function loadScriptOnce(src, timeoutMs = 8000) {
  if (scriptLoads.has(src)) return scriptLoads.get(src);
  const promise = new Promise((resolve, reject) => {
    const script = el("script", { src });
    let finished = false;
    const finish = error => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      script.onload = null;
      script.onerror = null;
      if (error) {
        script.remove();
        scriptLoads.delete(src);
        reject(error);
      } else resolve();
    };
    const timer = setTimeout(() => finish(new Error(`スクリプトが${timeoutMs}ミリ秒以内に応答しませんでした: ${src}`)), timeoutMs);
    script.onload = () => finish(null);
    script.onerror = () => finish(new Error(`スクリプトを読み込めませんでした: ${src}`));
    document.head.append(script);
  });
  scriptLoads.set(src, promise);
  return promise;
}

/** 小さな購読器。状態の持ち主が変化を知らせるときに使う。 */
export function createEmitter() {
  const listeners = new Map();
  return {
    on(name, callback) {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name).add(callback);
      return () => listeners.get(name)?.delete(callback);
    },
    emit(name, value) {
      for (const callback of [...(listeners.get(name) || [])]) {
        try { callback(value); }
        catch (error) { logApp("error", `画面イベント ${name} の処理に失敗しました`, errorDetail(error)); }
      }
    },
  };
}

// 起動・更新payloadの整数はバックエンドの宣言と同じ範囲で確認し、代替値を作らない。
export function readIntegerSetting(payload, key, property) {
  const invalid = payload.invalidNumbers?.find(item => item.key === key);
  if (invalid) return { value: null, error: invalid.message };
  let value = payload[property];
  if (typeof value === "string" && /^-?\d+$/.test(value)) {
    const exact = BigInt(value);
    if (exact > BigInt(Number.MAX_SAFE_INTEGER) || exact < BigInt(Number.MIN_SAFE_INTEGER)) value = exact;
  }
  const schema = payload.numberSettings?.[key];
  if (!schema || (typeof value !== "bigint" && !Number.isSafeInteger(value))) return { value: null, error: `${key}の整数設定を確認できません` };
  const quantity = BigInt(value);
  if (quantity < BigInt(schema.range[0]) || quantity > BigInt(schema.range[1])) return { value: null, error: `${key}は${schema.range.join("〜")}${schema.unit || ""}の範囲で設定してください` };
  return { value, error: null };
}
