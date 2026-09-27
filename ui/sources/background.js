// background.js - 配信の背景。キャラ画像へ合成せず、独立した背景として置く。

import * as bridge from "../app/bridge.js";
import { el } from "../app/util.js";

const BACKGROUNDS = ["pastel", "gaming", "simple", "sakura-room", "dog-hobby-room", "custom"];

export function createBackgroundSource() {
  const element = el("div", { className: "stream-background" }, [], { "aria-hidden": "true" });
  return {
    element,
    setAppearance({ background, image }) {
      element.dataset.background = BACKGROUNDS.includes(background) ? background : "pastel";
      element.style.setProperty("--stream-custom-background", image ? `url("${bridge.assetUrl(image)}")` : "none");
    },
  };
}
