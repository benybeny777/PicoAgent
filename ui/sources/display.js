// display.js - 配信画面の中に置くディスプレイ（ダッシュボード・ゲーム・時計・画像）。

import * as bridge from "../app/bridge.js";
import { el } from "../app/util.js";

const KINDS = ["dashboard", "game", "clock", "image", "none"];

export function createDisplaySource() {
  const title = el("strong", { textContent: "PICO STUDIO" });
  const caption = el("span", { textContent: "CHAT & CHARACTER" });
  const element = el("div", { className: "stream-display" }, [title, caption], { role: "group", "aria-label": "配信画面内ディスプレイ" });
  return {
    element,
    setAppearance({ display, image }) {
      const kind = KINDS.includes(display) ? display : "dashboard";
      element.dataset.kind = kind;
      element.style.setProperty("--stream-display-image", image ? `url("${bridge.assetUrl(image)}")` : "none");
      title.textContent = kind === "clock" ? "PICO STUDIO  ON AIR" : "PICO STUDIO";
    },
  };
}
