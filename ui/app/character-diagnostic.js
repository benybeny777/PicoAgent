// キャラ読込失敗の確定した診断と、利用者による再試行だけを管理する。
import { el, errorMessage } from "./util.js";

export function createCharacterDiagnostic({ container, retry, useImage }) {
  const title = el("strong");
  const reason = el("p", { className: "character-diagnostic-reason" });
  const retryButton = el("button", { type: "button", textContent: "再試行" });
  const imageButton = el("button", { type: "button", textContent: "このキャラを2D表示にする", hidden: true });
  const panel = el("section", { className: "character-diagnostic", hidden: true }, [
    title, reason, el("div", { className: "character-diagnostic-actions" }, [retryButton, imageButton]),
  ], { role: "region", "aria-label": "キャラクターの診断", "aria-live": "polite" });
  container.append(panel);
  let characterId = "";
  let generation = 0;
  async function run(action) {
    const current = generation;
    retryButton.disabled = imageButton.disabled = true;
    panel.setAttribute("aria-busy", "true");
    try { await action(characterId); }
    catch (error) {
      if (generation === current) reason.textContent = errorMessage(error);
    } finally {
      if (generation === current) {
        retryButton.disabled = imageButton.disabled = false;
        panel.removeAttribute("aria-busy");
      }
    }
  }
  retryButton.addEventListener("click", () => run(retry));
  imageButton.addEventListener("click", () => run(useImage));
  return {
    show(id, error) {
      generation++;
      characterId = id;
      title.textContent = `${error.pack?.name || id}を表示できません`;
      reason.textContent = errorMessage(error);
      imageButton.hidden = !error.imageAvailable;
      retryButton.disabled = imageButton.disabled = false;
      panel.removeAttribute("aria-busy");
      panel.hidden = false;
    },
    clear() { generation++; panel.hidden = true; },
    dispose() { generation++; panel.remove(); },
  };
}
