// info.js - 配信情報（LIVE 表示と時計）。表示している間だけ時計を進める。

import { el } from "../app/util.js";

export function createInfoSource() {
  const time = el("strong");
  const date = el("span");
  const element = el("div", { className: "stream-info" }, [el("span", { className: "stream-live", textContent: "● LIVE" }), time, date], { role: "group", "aria-label": "配信情報" });
  let timer = null;
  const render = () => {
    const now = new Date();
    const weekday = "日月火水木金土"[now.getDay()];
    time.textContent = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
    date.textContent = `${now.getFullYear()}.${String(now.getMonth() + 1).padStart(2, "0")}.${String(now.getDate()).padStart(2, "0")} ${weekday}`;
  };
  const sync = () => {
    const visible = !element.hidden;
    // 分単位の表示なので15秒ごとで足りる（常駐アプリなので秒間更新はしない）。
    if (visible && !timer) { render(); timer = setInterval(render, 15000); }
    else if (!visible && timer) { clearInterval(timer); timer = null; }
  };
  new MutationObserver(sync).observe(element, { attributes: true, attributeFilter: ["hidden"] });
  window.addEventListener("beforeunload", () => { if (timer) clearInterval(timer); });
  return { element, update: sync };
}
