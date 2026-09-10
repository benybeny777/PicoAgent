(() => {
  "use strict";

  const menu = document.getElementById("topWindowMenu");
  if (!menu) return;

  const minimize = document.getElementById("windowMinimize");
  const maximize = document.getElementById("windowMaximize");
  const close = document.getElementById("windowClose");

  minimize?.addEventListener("click", event => {
    event.stopPropagation();
    window.picoagent.minimizeWindow().catch(error => {
      window.PicoUI?.logApp?.("error", "最小化できませんでした", error);
    });
  });
  maximize?.addEventListener("click", event => {
    event.stopPropagation();
    window.picoagent.toggleMaximizeWindow().catch(error => {
      window.PicoUI?.logApp?.("error", "最大化を切り替えられませんでした", error);
    });
  });
  close?.addEventListener("click", event => {
    event.stopPropagation();
    window.picoagent.closeWindow().catch(error => {
      window.PicoUI?.logApp?.("error", "ウィンドウを閉じられませんでした", error);
    });
  });

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
