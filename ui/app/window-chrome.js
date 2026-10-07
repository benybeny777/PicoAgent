// window-chrome.js - 枠なし透明ウィンドウの操作部品（上端メニュー・ウィンドウ操作・8方向リサイズ・移動）。

import * as bridge from "./bridge.js";
import { toast } from "./toast.js";
import { el, errorDetail, errorMessage, logApp, preview } from "./util.js";

function reportFailure(message, error) {
  logApp("error", message, errorDetail(error));
  toast(`${message}: ${preview(errorMessage(error), 80)}`, "error");
}

function beginWindowDrag(event) {
  if (event.button !== 0) return;
  if (event.target instanceof Element && event.target.closest("button,input,textarea,select,a,[contenteditable]")) return;
  event.preventDefault();
  bridge.startWindowDrag().catch(error => {
    logApp("warn", "ウィンドウの移動を開始できませんでした", errorDetail(error));
    toast("画面を移動できませんでした。右クリックの「今の画面位置・サイズを戻す」も試してください", "error");
  });
}

/** 押した位置に一覧を出すボタン。二度押しで重ねて開かない。 */
function menuButton(button, open) {
  button.addEventListener("click", async event => {
    event.preventDefault();
    event.stopPropagation();
    if (button.disabled) return;
    button.disabled = true;
    const rect = button.getBoundingClientRect();
    try { await open(Math.round(rect.left), Math.round(rect.bottom)); }
    catch (error) { reportFailure(`${button.textContent}の一覧を開けませんでした`, error); }
    finally { button.disabled = false; }
  });
}

// フロー表示で、上端からこの距離（px）までポインタが来たらタイトルバーを出す。
// シーン上端の近くにある会話パネルの操作を、下から近づいたときに覆わない細さにする。
const REVEAL_EDGE_PX = 6;
// タイトルバーから離れてから隠すまでの猶予。端をなぞっただけで点滅させない。
const HIDE_DELAY_MS = 450;
// 起動直後は在りかが分かるよう、しばらく出しておく。
const INTRO_MS = 2600;

export function createWindowChrome({ menu, appElement, bootScreen, stageElement, onEditLayout, onGame, listCharacters, onPinnedChange }) {
  const layoutButton = el("button", { type: "button", textContent: "表示", title: "画面レイアウトを切り替え" }, [], { "aria-haspopup": "menu" });
  const editButton = el("button", { type: "button", textContent: "配置", title: "OBS風に画面を編集" }, [], { "aria-pressed": "false" });
  const characterButton = el("button", { type: "button", textContent: "キャラ選択", title: "キャラクターを切り替え" }, [], { "aria-haspopup": "menu" });
  const gameButton = el("button", { type: "button", textContent: "遊ぶ", title: "「人間だとバレるな」で遊ぶ" });
  const settingsButton = el("button", { type: "button", textContent: "設定", title: "設定を開く" });
  const pin = el("button", { type: "button", className: "window-pin", title: "タイトルバーを常に表示", textContent: "📌" }, [], { "aria-label": "タイトルバーを常に表示", "aria-pressed": "false" });
  const minimize = el("button", { type: "button", title: "最小化", textContent: "−" }, [], { "aria-label": "最小化" });
  const maximize = el("button", { type: "button", title: "最大化", textContent: "□" }, [], { "aria-label": "最大化" });
  const close = el("button", { type: "button", className: "window-close", title: "閉じる", textContent: "✕" }, [], { "aria-label": "閉じる" });
  menu.replaceChildren(
    el("span", { className: "top-window-menu-brand", textContent: "PicoAgent" }),
    el("div", { className: "top-window-menu-items" }, [layoutButton, editButton, characterButton, gameButton, settingsButton]),
    el("span", { className: "top-window-menu-drag" }, [], { "aria-hidden": "true" }),
    el("span", { className: "window-caption-controls" }, [pin, minimize, maximize, close], { "aria-label": "ウィンドウ操作" }),
  );

  // `data-tauri-drag-region` だけに頼ると、透明 WebView ではドラッグ開始が届かないことがあるので明示的に渡す。
  menu.addEventListener("pointerdown", beginWindowDrag);
  bootScreen?.addEventListener("pointerdown", beginWindowDrag);
  stageElement.addEventListener("pointerdown", event => {
    if (event.target === stageElement || event.target?.classList?.contains("scene")) beginWindowDrag(event);
  });

  menuButton(layoutButton, (x, y) => bridge.layoutMenu(x, y));
  menuButton(characterButton, async (x, y) => {
    const characters = await listCharacters();
    if (!characters.length) throw new Error("キャラクターの一覧を取得できませんでした");
    await bridge.characterMenu(x, y, characters);
  });
  editButton.addEventListener("click", event => { event.stopPropagation(); onEditLayout(); });
  gameButton.addEventListener("click", event => { event.stopPropagation(); onGame(); });
  settingsButton.addEventListener("click", async event => {
    event.stopPropagation();
    settingsButton.disabled = true;
    try { await bridge.openSettings(); }
    catch (error) { reportFailure("設定画面を開けませんでした", error); }
    finally { settingsButton.disabled = false; }
  });
  for (const [button, action, label] of [[minimize, bridge.minimizeWindow, "最小化"], [maximize, bridge.toggleMaximizeWindow, "最大化の切り替え"], [close, bridge.closeWindow, "閉じる"]]) {
    button.addEventListener("click", event => {
      event.stopPropagation();
      action().catch(error => reportFailure(`${label}ができませんでした`, error));
    });
  }

  // decorations:false の透明ウィンドウには OS の枠が無いため、各辺と四隅を明示的なリサイズ領域にする。
  const handles = el("div", { className: "window-resize-handles" }, [], { "aria-hidden": "true" });
  for (const direction of ["North", "East", "South", "West", "NorthEast", "SouthEast", "SouthWest", "NorthWest"]) {
    const handle = el("span", { dataset: { resizeDirection: direction } });
    handle.addEventListener("pointerdown", event => {
      if (event.button !== 0) return;
      event.preventDefault();
      event.stopPropagation();
      bridge.startWindowResize(direction).catch(error => reportFailure("ウィンドウのサイズ変更を開始できませんでした", error));
    });
    handles.append(handle);
  }
  menu.after(handles);

  // ── フロー表示と「常に表示」 ──────────────────────────────
  let pinned = false;
  let editing = false;
  let hideTimer = 0;
  const shown = () => appElement.classList.contains("title-bar-shown");
  function show() {
    clearTimeout(hideTimer);
    hideTimer = 0;
    appElement.classList.add("title-bar-shown");
  }
  function hideSoon(delay = HIDE_DELAY_MS) {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      hideTimer = 0;
      // 一覧（OSのメニュー）を開いている間はボタンが押せない状態なので、その間も隠さない。
      const listOpen = layoutButton.disabled || characterButton.disabled;
      if (pinned || editing || listOpen || menu.querySelector(":focus-visible")) { if (listOpen) hideSoon(); return; }
      appElement.classList.remove("title-bar-shown");
    }, delay);
  }
  function applyPinned(next) {
    pinned = Boolean(next);
    appElement.classList.toggle("title-bar-floating", !pinned);
    pin.setAttribute("aria-pressed", String(pinned));
    pin.title = pinned ? "タイトルバーを自動で隠す" : "タイトルバーを常に表示";
    pin.setAttribute("aria-label", pin.title);
    if (pinned) show();
    else hideSoon();
  }
  document.addEventListener("pointermove", event => {
    if (pinned) return;
    const rect = menu.getBoundingClientRect();
    const withinX = event.clientX >= rect.left && event.clientX <= rect.right;
    const nearTop = event.clientY <= rect.top + REVEAL_EDGE_PX;
    if (withinX && (nearTop || (shown() && event.clientY <= rect.bottom))) show();
    else if (shown() && !hideTimer) hideSoon();
  });
  document.documentElement.addEventListener("pointerleave", () => { if (!pinned) hideSoon(); });
  // キーボードで移ってきたときだけ出す（クリック後のフォーカス残りでは出し続けない）。
  menu.addEventListener("focusin", () => { if (menu.querySelector(":focus-visible")) show(); });
  menu.addEventListener("focusout", () => { if (!pinned) hideSoon(); });
  pin.addEventListener("click", event => {
    event.stopPropagation();
    applyPinned(!pinned);
    onPinnedChange?.(pinned);
  });

  return {
    /** 保存値（PICOAGENT_TITLE_BAR_PINNED）を当てる。保存はしない。 */
    setPinned(next) { applyPinned(next); },
    /** 起動直後だけ、フロー表示でもタイトルバーの在りかを見せる。 */
    introduce() {
      if (pinned) return;
      show();
      hideSoon(INTRO_MS);
    },
    setCharacterName(name) { characterButton.textContent = name || "キャラ選択"; },
    setEditing(on) {
      editing = Boolean(on);
      editButton.setAttribute("aria-pressed", String(editing));
      // 配置の編集中は「完了」までタイトルバーを出したままにする。
      if (editing) show();
      else if (!pinned) hideSoon();
    },
  };
}
