(() => {
  "use strict";

  const menu = document.getElementById("topWindowMenu");
  if (!menu) return;

  for (const button of menu.querySelectorAll("[data-settings-tab]")) {
    button.addEventListener("click", async event => {
      event.preventDefault();
      event.stopPropagation();
      if (button.disabled) return;
      button.disabled = true;
      const tab = button.dataset.settingsTab || null;
      try {
        await window.picoagent.openSettings(tab);
      } catch (error) {
        window.PicoUI?.logApp?.("error", "設定画面を開けませんでした", error);
      } finally {
        button.disabled = false;
      }
    });
  }
})();
