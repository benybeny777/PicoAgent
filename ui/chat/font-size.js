// font-size.js - 会話パネルの文字サイズをパネル上で変える（「Aa」ボタン、Ctrl+ホイール、Ctrl + + / - / 0）。
//
// 見せ方ごとに別の設定を持つ。bubble（ペット・ミニ）は PICOAGENT_BUBBLE_FONT_PX、
// column（配信の履歴列）は PICOAGENT_STREAM_CHAT_FONT_PX。範囲はバックエンドの pet_save_ui_pref と同じ。
// 変更はその場で当て、操作が落ち着いてから保存する（連打のたびに設定ファイルを書かない）。

import * as bridge from "../app/bridge.js";
import { toast } from "../app/toast.js";
import { el, errorDetail, errorMessage, logApp } from "../app/util.js";

const FONT_OPTIONS = {
  bubble: { key: "PICOAGENT_BUBBLE_FONT_PX", step: 1 },
  column: { key: "PICOAGENT_STREAM_CHAT_FONT_PX", step: 2 },
};
const SAVE_DELAY_MS = 600;

/**
 * @param {{ panel: HTMLElement, variant: () => string, apply: (kind: string, px: number) => void }} options
 *   apply は見た目だけを変える（保存しない）。
 */
export function createFontSizeControl({ panel, variant, apply }) {
  const sizes = { bubble: null, column: null };
  const ranges = {};
  const timers = new Map();
  const kind = () => (variant() === "column" ? "column" : "bubble");

  const toggle = el("button", { type: "button", className: "chip font-size-toggle", textContent: "Aa", title: "会話の文字サイズ（Ctrl+ホイール）" }, [], { "aria-label": "会話の文字サイズ", "aria-expanded": "false" });
  const smaller = el("button", { type: "button", className: "chip", textContent: "−", title: "小さく（Ctrl+−）" }, [], { "aria-label": "文字を小さく" });
  const value = el("output", { className: "font-size-value" }, [], { "aria-live": "polite" });
  const larger = el("button", { type: "button", className: "chip", textContent: "＋", title: "大きく（Ctrl++）" }, [], { "aria-label": "文字を大きく" });
  const reset = el("button", { type: "button", className: "chip", textContent: "標準", title: "標準の大きさへ戻す（Ctrl+0）" }, [], { "aria-label": "文字サイズを標準へ戻す" });
  const popover = el("div", { className: "font-size-panel", hidden: true }, [
    el("span", { className: "font-size-label", textContent: "文字" }), smaller, value, larger, reset,
  ], { role: "group", "aria-label": "会話の文字サイズ" });

  function render() {
    const current = kind();
    const range = ranges[current];
    if (!range || sizes[current] === null) { toggle.disabled = true; return; }
    toggle.disabled = false;
    value.textContent = `${sizes[current]}px`;
    smaller.disabled = sizes[current] <= range.min;
    larger.disabled = sizes[current] >= range.max;
  }

  function save(current) {
    clearTimeout(timers.get(current));
    timers.set(current, setTimeout(() => {
      timers.delete(current);
      const { key } = ranges[current];
      bridge.saveUiPref(key, String(sizes[current])).catch(error => {
        logApp("error", "会話の文字サイズを保存できませんでした", errorDetail(error));
        toast(`文字サイズを保存できませんでした: ${errorMessage(error)}`, "error");
      });
    }, SAVE_DELAY_MS));
  }

  function change(next) {
    const current = kind();
    const range = ranges[current];
    if (!range || sizes[current] === null) return;
    const size = Math.min(range.max, Math.max(range.min, next));
    if (size === sizes[current]) return;
    sizes[current] = size;
    apply(current, size);
    render();
    save(current);
  }
  const step = direction => change(sizes[kind()] + direction * ranges[kind()].step);

  function setOpen(open) {
    popover.hidden = !open;
    toggle.setAttribute("aria-expanded", String(open));
    if (open) render();
  }

  toggle.addEventListener("click", event => {
    event.stopPropagation();
    setOpen(popover.hidden);
  });
  smaller.addEventListener("click", event => { event.stopPropagation(); step(-1); });
  larger.addEventListener("click", event => { event.stopPropagation(); step(1); });
  reset.addEventListener("click", event => { event.stopPropagation(); change(ranges[kind()].default); });
  popover.addEventListener("click", event => event.stopPropagation());
  document.addEventListener("pointerdown", event => {
    if (!popover.hidden && !popover.contains(event.target) && event.target !== toggle) setOpen(false);
  });
  popover.addEventListener("keydown", event => {
    if (event.key === "Escape") { setOpen(false); toggle.focus(); }
  });
  // Ctrl（⌘）+ホイールはパネルの上でだけ文字サイズにする（窓の拡大や他の要素のホイールは奪わない）。
  panel.addEventListener("wheel", event => {
    if (!(event.ctrlKey || event.metaKey) || event.deltaY === 0) return;
    event.preventDefault();
    step(event.deltaY < 0 ? 1 : -1);
  }, { passive: false });
  panel.addEventListener("keydown", event => {
    if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
    if (event.key === "+" || event.key === "=" || event.key === ";") step(1);
    else if (event.key === "-" || event.key === "_") step(-1);
    else if (event.key === "0") change(ranges[kind()].default);
    else return;
    event.preventDefault();
  });

  return {
    toggle,
    popover,
    setNumberSettings(metadata) {
      for (const [current, options] of Object.entries(FONT_OPTIONS)) {
        const schema = metadata[options.key];
        if (!schema) throw new Error(`${options.key}の整数宣言がありません`);
        ranges[current] = { ...options, min: Number(schema.range[0]), max: Number(schema.range[1]), default: Number(schema.default) };
      }
      render();
    },
    /** 共通の検証を通った設定値を当てる（保存しない）。 */
    setSize(current, px) {
      const range = ranges[current];
      if (!range || !Number.isInteger(px) || px < range.min || px > range.max) throw new Error("文字サイズは宣言範囲の整数で指定してください");
      const size = px;
      sizes[current] = size;
      render();
    },
    /** 見せ方が変わったら表示中の値を差し替える。 */
    refresh: render,
  };
}
