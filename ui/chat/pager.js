// pager.js - 配信字幕のページ分け。回答欄の実DOM寸法で、今の幅・文字サイズに収まる単位へ分ける。
//
// 固定文字数だけではモニターや日本語/英数字の幅で欠けるため、scrollHeight を合否に使う。
// 測るのは字幕欄そのもの（同じ字面の隠し要素では、中身に応じて伸びる字幕欄の高さを再現できない）。

import { readingDelay, sanitizeSpeechText } from "../app/voice.js";
import { plainText } from "./markdown.js";

/** 文単位に分け、長すぎる文は54文字ごとに切る。 */
export function subtitleUnits(text) {
  const units = String(text ?? "").match(/[^。．.!?！？\n]+[。．.!?！？]?\s*|\n+/g) || [String(text ?? "")];
  const out = [];
  for (const unit of units) {
    const chars = [...unit];
    for (let index = 0; index < chars.length; index += 54) out.push(chars.slice(index, index + 54).join(""));
  }
  return out.filter(Boolean);
}

export function createPager({ isActive, render, fits }) {
  let token = 0;
  let timer = null;

  function cancel() {
    token += 1;
    if (timer) clearTimeout(timer);
    timer = null;
  }

  /** 本文をページに分ける（1ページ目を描いた状態で返す）。 */
  function pages(text) {
    const source = plainText(text, sanitizeSpeechText);
    if (!isActive() || !source.trim()) return source ? [source] : [];
    const result = [];
    const units = subtitleUnits(source);
    const joined = (from, count) => units.slice(from, from + count).join("");
    // 1ページに入る単位数はどのページもほぼ同じ。直前の単位数を当たりに使い、実測を2回前後に抑える。
    let guess = 0;
    let index = 0;
    while (index < units.length) {
      const rest = units.length - index;
      let low = 0;
      let high = rest + 1;
      const probe = guess > 0 ? Math.min(guess, rest) : 1;
      if (fits(joined(index, probe))) {
        low = probe;
        let step = 1;
        while (low + step <= rest && fits(joined(index, low + step))) {
          low += step;
          step = Math.min(step * 2, 8);
        }
        high = Math.min(rest + 1, low + step);
      } else {
        high = probe;
      }
      while (low + 1 < high) {
        const middle = Math.floor((low + high) / 2);
        if (fits(joined(index, middle))) low = middle;
        else high = middle;
      }
      if (low === 0) {
        // 1単位でも収まらない極端な狭幅では、収まる最大長を文字単位で二分探索する。
        let remaining = [...units[index]];
        while (remaining.length && !fits(remaining.join(""))) {
          let charLow = 1;
          let charHigh = remaining.length;
          while (charLow < charHigh) {
            const middle = Math.ceil((charLow + charHigh) / 2);
            if (fits(remaining.slice(0, middle).join(""))) charLow = middle;
            else charHigh = middle - 1;
          }
          result.push(remaining.slice(0, charLow).join(""));
          remaining = remaining.slice(charLow);
        }
        if (remaining.length) units[index] = remaining.join("");
        else index += 1;
        guess = 0;
        continue;
      }
      result.push(joined(index, low));
      index += low;
      guess = low;
    }
    if (result.length) render(result[0]);
    return result;
  }

  function show(page) {
    if (isActive()) render(page);
  }

  /** 読み上げなしで、読める時間ごとにページを送る。 */
  function timed(text) {
    cancel();
    if (!isActive()) return;
    const list = pages(text);
    if (list.length <= 1) {
      if (list[0]) show(list[0]);
      return;
    }
    const current = token;
    let index = 0;
    const advance = () => {
      if (current !== token) return;
      show(list[index]);
      if (++index >= list.length) return;
      timer = setTimeout(advance, readingDelay(list[index - 1]));
    };
    advance();
  }

  return { paginates: isActive, pages, show, timed, cancel };
}
