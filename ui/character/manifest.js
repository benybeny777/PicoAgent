// manifest.js - キャラクターの一覧と定義（character.json）を読み、描画器へ渡す「パック」にまとめる。

import * as bridge from "../app/bridge.js";
import * as uiStore from "../app/ui-store.js";
import { fetchJson } from "../app/util.js";

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

/** 同梱と追加を突き合わせた一覧。同じ id なら同梱側を優先する（追加分が同梱キャラを乗っ取らない）。 */
export async function listCharacters() {
  const catalog = await loadCatalog();
  const bundled = (catalog?.characters || []).map(entry => ({
    id: entry.id,
    name: entry.name || entry.id,
    description: entry.description || "",
    root: `assets/characters/${String(entry.manifest).replace(/\/character\.json$/, "")}`,
  }));
  const users = await window.PicoUI.loadUserCharacters(bridge.userCharacters, bridge.assetUrl, bundled);
  return {
    entries: bundled.concat(users.entries), defaultId: catalog?.default,
    userCharactersError: users.error, userCharactersRoot: users.root,
  };
}

/** 保存された見た目の違い（variant）の選択。キャラごとに WebView の localStorage へ覚える。 */
function variantChoice(characterId) {
  const result = uiStore.read(`picoagent.variant.${characterId}`);
  if (!result.ok) throw new Error(`見た目の選択を読めません: ${result.reason}`);
  if (result.value === null) return { id: "default", saved: false };
  if (typeof result.value !== "string" || !result.value.trim()) throw new Error("保存した見た目の選択の形式が壊れています");
  return { id: result.value, saved: true };
}

export function savedVariant(characterId) {
  return variantChoice(characterId).id;
}

export function rememberVariant(characterId, variantId) {
  if (typeof variantId !== "string" || !variantId.trim()) throw new Error("保存する見た目の選択が空です");
  const result = uiStore.write(`picoagent.variant.${characterId}`, variantId);
  if (!result.ok) throw new Error(`見た目の選択を保存できません: ${result.reason}`);
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

/** 通常2Dの画像構造を、表示ホストと設定画面で同じ規則へそろえる。 */
export function imageAnimationsOf(manifest, variant = null) {
  return variant?.animations || manifest.animations
    || ((variant?.meshRig || manifest.meshRig) ? { idle: { framePaths: ["preview.png"], frames: 1, fps: 1 } } : {});
}

const hasLive2d = config => Boolean(config?.model || config?.models?.length);

/**
 * 描画器へ渡すパック。見た目の違い（variant）をここで解決し、描画器は選ばれた定義だけを見る。
 * `variant.live2d === false` なら、キャラ本体が Live2D でもフレーム表示へ戻す。
 */
export async function loadPack(characterId, { sdkPreference = "auto", onUserCharacters } = {}) {
  const { entries, userCharactersError, userCharactersRoot } = await listCharacters();
  onUserCharacters?.({ error: userCharactersError, root: userCharactersRoot });
  const entry = entries.find(item => item.id === characterId);
  if (!entry) {
    if (userCharactersError) throw new Error(`追加キャラ一覧を読めないため「${characterId}」を表示できません: ${window.PicoUI.errorMessage(userCharactersError)}`);
    throw new Error(`キャラクター「${characterId}」が一覧にありません`);
  }
  const manifest = await fetchJson(`${entry.root}/character.json?v=${MANIFEST_CACHE_KEY}`, "キャラクター定義");
  if (manifest.id == null || manifest.id === "") manifest.id = entry.id;
  else if (manifest.id !== entry.id) {
    throw new Error(`キャラクター定義のID「${manifest.id}」が選択ID「${entry.id}」と一致しません`);
  }
  const variants = Array.isArray(manifest.variants) ? manifest.variants : [];
  const choice = variantChoice(manifest.id);
  const variant = variants.find(item => item.id === choice.id) || (!choice.saved ? variants[0] || null : null);
  if (choice.saved && !variant) throw new Error("保存したスタイルがこのキャラクターの定義にありません。選択を確認してください");
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
    animations: imageAnimationsOf(manifest, variant),
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
