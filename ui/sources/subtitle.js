// subtitle.js - 配信の字幕枠。回答欄（#reply）そのものは会話パネルが持ち、配信形式ではここへ移して表示する。
// 字幕は映像の一部なので選択させない。ドラッグで入り込んだ選択も解除する（選択色が配信に映らないように）。

import { el } from "../app/util.js";

export function createSubtitleSource() {
  const element = el("div", { className: "subtitle" });
  element.addEventListener("selectstart", event => event.preventDefault());
  document.addEventListener("selectionchange", () => {
    if (element.hidden || !element.firstChild) return;
    const selection = window.getSelection?.();
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return;
    for (let index = 0; index < selection.rangeCount; index++) {
      if (selection.getRangeAt(index).intersectsNode(element)) {
        selection.removeAllRanges();
        return;
      }
    }
  });
  return { element };
}
