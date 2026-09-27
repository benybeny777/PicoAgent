// context-menu.js - 右クリックの統一メニュー。どこで右クリックしても同じネイティブメニューを出す。
//
// 画面は文脈（選択文字・入力欄・リンク・キャラ）の有無だけをバックエンドへ渡し、文字列そのものは渡さない。
// 文脈項目が選ばれると `pet:context-action` が返ってくるので、右クリックした時点に控えた選択・要素に対して実行する。

import * as bridge from "./bridge.js";
import { toast } from "./toast.js";
import { copyText, errorDetail, errorMessage, logApp, preview } from "./util.js";

export function createContextMenu({ onCharacterAction, openExternalUrl }) {
  let remembered = null;

  function capture(event) {
    const target = event.target instanceof Element ? event.target : null;
    const editable = target?.closest("textarea,input:not([type=checkbox]):not([type=radio]):not([type=button])") || null;
    const link = target?.closest("a[href]") || null;
    const character = Boolean(target?.closest('[data-source="character"],.edit-frame[data-target="character"]'));
    let selectionText = "";
    if (editable && typeof editable.selectionStart === "number") {
      selectionText = editable.value.slice(editable.selectionStart, editable.selectionEnd);
    } else {
      selectionText = String(window.getSelection?.() ?? "");
    }
    return {
      editable,
      link,
      selectionText,
      editableRange: editable && typeof editable.selectionStart === "number"
        ? [editable.selectionStart, editable.selectionEnd] : null,
      context: {
        selection: Boolean(selectionText.trim()),
        editable: Boolean(editable),
        link: Boolean(link),
        character,
      },
    };
  }

  document.addEventListener("contextmenu", event => {
    event.preventDefault();
    remembered = capture(event);
    bridge.menu(remembered.context, Math.round(event.clientX), Math.round(event.clientY)).catch(error => {
      // 失敗はブリッジが app.log へ残す。利用者には右クリックが無反応に見えるので理由も出す。
      toast(`メニューを開けませんでした: ${preview(errorMessage(error), 60)}`, "error");
    });
  });

  async function copy(text) {
    try {
      if (!(await copyText(text))) toast("コピーできませんでした", "error");
    } catch (error) {
      logApp("warn", "コピーできませんでした", errorDetail(error));
      toast("コピーできませんでした", "error");
    }
  }

  function replaceSelection(field, [start, end], text) {
    field.focus();
    field.setRangeText(text, start, end, "end");
    field.dispatchEvent(new Event("input", { bubbles: true }));
  }

  async function perform({ action, framing } = {}) {
    const state = remembered;
    switch (action) {
      case "copy":
        if (state?.selectionText) await copy(state.selectionText);
        break;
      case "cut":
        if (state?.editable && state.editableRange && state.selectionText) {
          await copy(state.selectionText);
          replaceSelection(state.editable, state.editableRange, "");
        }
        break;
      case "paste":
        if (!state?.editable || !state.editableRange) break;
        try {
          const text = await navigator.clipboard.readText();
          replaceSelection(state.editable, state.editableRange, text);
        } catch (error) {
          logApp("warn", "クリップボードを読めませんでした", errorDetail(error));
          toast("クリップボードを読めませんでした。Ctrl+V で貼り付けてください", "warn");
        }
        break;
      case "select-all":
        if (state?.editable) { state.editable.focus(); state.editable.select(); }
        break;
      case "open-link":
        if (state?.link) openExternalUrl(state.link.href);
        break;
      case "copy-link":
        if (state?.link) await copy(state.link.dataset.localPath || state.link.href);
        break;
      case "edit-character":
      case "reset-character":
      case "set-framing":
        onCharacterAction(action, framing);
        break;
      default:
        logApp("warn", "未知の右クリック操作です", String(action));
    }
  }

  bridge.on("pet:context-action", payload => perform(payload));
  return { perform };
}
