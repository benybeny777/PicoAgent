// index.js - シーンへ置くソースの登録表。
//
// ソースを足すときは `ui/sources/<名前>.js` に `create(context)` を書き、ここへ1行足す。
// 配置（位置・大きさ・重ね順・表示/非表示）、配置編集、保存、アンカー追従は stage.js が全ソース共通で
// 行うので、ソース側は自分の中身の描画だけを持つ。返す値は `{ element, update?(info) }`。
// `update` は配置が決まるたびに `{ layoutId, rect, options, pixelRatio, expanded }` を受け取る。

import { createBackgroundSource } from "./background.js";
import { createCharacterSource } from "./character.js";
import { createDisplaySource } from "./display.js";
import { createInfoSource } from "./info.js";
import { createSubtitleSource } from "./subtitle.js";
import { createChatSource } from "../chat/panel.js";

export const SOURCE_FACTORIES = Object.freeze({
  background: createBackgroundSource,
  display: createDisplaySource,
  info: createInfoSource,
  character: createCharacterSource,
  subtitle: createSubtitleSource,
  chat: createChatSource,
});

export const SOURCE_LABELS = Object.freeze({
  background: "配信背景",
  display: "画面内ディスプレイ",
  info: "配信情報（時計）",
  character: "キャラクター",
  subtitle: "字幕",
  chat: "会話パネル",
});

/** 登録表の順にソースを作り、`#sceneRoot` へ置く。 */
export function createSources(sceneRoot, context) {
  const sources = new Map();
  for (const [name, factory] of Object.entries(SOURCE_FACTORIES)) {
    const source = factory(context);
    source.element.classList.add("source");
    source.element.dataset.source = name;
    source.element.hidden = true;
    sceneRoot.append(source.element);
    sources.set(name, source);
  }
  return sources;
}
