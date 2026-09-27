// manifest.js - キャラクターの一覧と定義（character.json）を読み、描画器へ渡す「パック」にまとめる。

import * as bridge from "../app/bridge.js";
import { errorDetail, fetchJson, logApp } from "../app/util.js";

const MANIFEST_CACHE_KEY = "2026-07-03-live2d-jp-voice-v1";

let catalogPromise = null;

/** 同梱キャラの一覧（配布物に焼き込まれ、実行中に増減しない）。最初の1回だけ読む。 */
export function loadCatalog() {
  if (!catalogPromise) {
    catalogPromise = fetchJson(`assets/characters/index.json?v=${MANIFEST_CACHE_KEY}`, "キャラクター一覧")
      .catch(error => {
        catalogPromise = null;
        throw error;
      });
  }
  return catalogPromise;
}

/** 利用者が追加したキャラ（app_data_dir 配下）を同梱一覧と同じ形へそろえる。 */
async function userEntries() {
  try {
    const result = await bridge.userCharacters();
    return (result?.characters || []).map(entry => ({
      id: entry.id,
      name: entry.name,
      description: entry.description || "",
      root: bridge.assetUrl(entry.path),
    }));
  } catch (error) {
    // フォールバック許可: 追加キャラを取れなくても同梱キャラだけで動かす（記録は残す）。
    logApp("error", "追加キャラクターの一覧を取得できませんでした", errorDetail(error));
    return [];
  }
}

/** 同梱と追加を突き合わせた一覧。同じ id なら同梱側を優先する（追加分が同梱キャラを乗っ取らない）。 */
export async function listCharacters() {
  const catalog = await loadCatalog();
  const bundled = (catalog?.characters || []).map(entry => ({
    id: entry.id,
    name: entry.name || entry.id,
    description: entry.description || "",
    root: `assets/characters/${String(entry.manifest).replace(/\/character\.json$/, "")}`,
  }));
  const known = new Set(bundled.map(entry => entry.id));
  const all = bundled.concat((await userEntries()).filter(entry => !known.has(entry.id)));
  return { entries: all, defaultId: catalog?.default };
}

/** 保存された見た目の違い（variant）の選択。キャラごとに WebView の localStorage へ覚える。 */
export function savedVariant(characterId) {
  try { return localStorage.getItem(`picoagent.variant.${characterId}`) || "default"; }
  catch { return "default"; } // フォールバック許可: 保存領域を読めない環境では既定の見た目で表示する。
}

export function rememberVariant(characterId, variantId) {
  try { localStorage.setItem(`picoagent.variant.${characterId}`, variantId); }
  catch (error) { logApp("warn", "見た目の選択を保存できませんでした", errorDetail(error)); }
}

export function personaText(manifest, override) {
  if (override?.trim()) return override.trim();
  const persona = manifest.persona || {};
  return [
    `名前: ${manifest.name || manifest.id}`,
    `性格: ${persona.personality || ""}`,
    `口調: ${persona.speechStyle || ""}`,
    `一人称: ${persona.firstPerson || ""}`,
    `ユーザーの呼び方: ${persona.userName || ""}`,
    persona.systemPrompt || "",
  ].filter(Boolean).join("\n");
}

const hasLive2d = config => Boolean(config?.model || config?.models?.length);

/**
 * 描画器へ渡すパック。見た目の違い（variant）をここで解決し、描画器は選ばれた定義だけを見る。
 * `variant.live2d === false` なら、キャラ本体が Live2D でもフレーム表示へ戻す。
 */
export async function loadPack(characterId, { sdkPreference = "auto" } = {}) {
  const { entries, defaultId } = await listCharacters();
  const entry = entries.find(item => item.id === characterId)
    || entries.find(item => item.id === defaultId)
    || entries[0];
  if (!entry) throw new Error("利用できるキャラクターがありません");
  const manifest = await fetchJson(`${entry.root}/character.json?v=${MANIFEST_CACHE_KEY}`, "キャラクター定義");
  manifest.id ||= entry.id;
  const variants = Array.isArray(manifest.variants) ? manifest.variants : [];
  const variant = variants.find(item => item.id === savedVariant(manifest.id)) || variants[0] || null;
  let root = entry.root;
  if (variant?.root) root = `${entry.root}/${String(variant.root).replace(/^\/+|\/+$/g, "")}`;
  const live2d = variant?.live2d === false ? null : (variant?.live2d || manifest.live2d || null);
  const pack = {
    id: manifest.id,
    name: manifest.name || manifest.id,
    manifest,
    root,
    variants,
    variantId: variant?.id || "default",
    assetVersion: String(manifest.assetVersion || ""),
    animations: variant?.animations || manifest.animations
      || (manifest.meshRig ? { idle: { framePaths: ["preview.png"], frames: 1, fps: 1 } } : {}),
    runtimeLayers: variant?.runtimeLayers || manifest.runtimeLayers || {},
    springPhysics: variant?.springPhysics ?? manifest.springPhysics ?? null,
    secondaryMotion: variant?.secondaryMotion ?? manifest.secondaryMotion ?? null,
    live2d: hasLive2d(live2d) ? live2d : null,
    meshRig: variant?.meshRig || manifest.meshRig || null,
    mmd: manifest.mmd?.model ? manifest.mmd : null,
    bodyProportion: manifest.bodyProportion === "compact" ? "compact" : "standard",
    sourceFraming: ["native-bust", "native-full"].includes(manifest.sourceFraming) ? manifest.sourceFraming : "default",
    live2dSdk: ["4", "5"].includes(String(sdkPreference)) ? String(sdkPreference) : "auto",
  };
  if (!pack.live2d && !pack.meshRig && !pack.mmd && !pack.animations?.idle) {
    throw new Error("MMD、Live2D、メッシュリグ、idleアニメーションのいずれも定義されていません");
  }
  return pack;
}

/** パック内の相対パスを配信 URL へ（素材の版を付けて古いキャッシュを使わせない）。 */
export function packUrl(pack, relative) {
  const url = `${pack.root}/${String(relative).replace(/^\/+/, "")}`;
  return pack.assetVersion ? `${url}?v=${encodeURIComponent(pack.assetVersion)}` : url;
}
