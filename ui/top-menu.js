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

  // 「表示」は設定画面を開かず、押した位置に画面レイアウトの一覧を出して1手で切り替える。
  // 一覧の最後の「画面レイアウト・位置…」から従来の設定画面へ進める。
  const layoutButton = document.getElementById("layoutMenuButton");
  layoutButton?.addEventListener("click", async event => {
    event.preventDefault();
    event.stopPropagation();
    if (layoutButton.disabled) return;
    layoutButton.disabled = true;
    // ウィンドウ左上を原点とする論理座標。メイン画面はOS枠を持たないので、WebViewの座標と一致する。
    const rect = layoutButton.getBoundingClientRect();
    try {
      await window.picoagent.layoutMenu(Math.round(rect.left), Math.round(rect.bottom));
    } catch (error) {
      window.PicoUI?.logApp?.("error", "画面レイアウトの一覧を開けませんでした", error);
    } finally {
      layoutButton.disabled = false;
    }
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
