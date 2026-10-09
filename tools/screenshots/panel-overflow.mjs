// 共通の会話CSSを狭いパネルと長いMarkdownで描き、親からのはみ出しを検査する。
import assert from "node:assert/strict";
import fs from "node:fs";
import { pathToFileURL } from "node:url";
import { createRequire } from "node:module";
import { execSync } from "node:child_process";
import path from "node:path";
let modulePath = process.env.PLAYWRIGHT_MODULE;
if (!modulePath) {
  const require = createRequire(import.meta.url);
  try { modulePath = require.resolve("playwright"); }
  catch {
    // 開発用のグローバル導入を再利用する。無ければ明示エラーで検証を止める。
    modulePath = path.join(execSync("npm root -g", { encoding: "utf8" }).trim(), "playwright", "index.mjs");
    if (!fs.existsSync(modulePath)) throw new Error("Chromium検査には開発用Playwrightが必要です。tools/screenshots/README.mdの導入手順を確認してください。");
  }
}
// require.resolve は CommonJS の index.js を返し、動的importでは default にしか載らない。
const playwright = await import(pathToFileURL(modulePath).href);
const { chromium } = playwright.chromium ? playwright : playwright.default;
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage();
  const css = ["ui/components/base.css", "ui/themes/pastel/tokens.css", "ui/themes/glass/tokens.css", "ui/components/chat.css"].map(p => fs.readFileSync(p, "utf8")).join("\n");
  const failures = [];
  for (const mode of ["pastel", "glass"]) for (const theme of ["light", "dark"]) for (const width of [200, 320]) {
    await page.setContent(`<html data-ui-theme="${mode}" data-theme="${theme}"><style>${css}\n${process.env.PANEL_OVERFLOW_REGRESSION || ""}</style><section class="chat" style="width:${width}px;height:240px"><div class="reply-home"><div class="reply"><div class="md"><pre><code>${"long_code_".repeat(100)}</code></pre><div class="md-table-wrap"><table><tr>${"<td>very_long_unbroken_cell_content</td>".repeat(10)}</tr></table></div>${"<p>長い会話本文も最後までスクロールして読めます。</p>".repeat(12)}</div></div></div><div class="composer"><textarea></textarea></div></section></html>`);
    const result = await page.evaluate(() => {
      const panel = document.querySelector(".chat");
      const rect = panel.getBoundingClientRect(), style = getComputedStyle(panel);
      const left = rect.left + parseFloat(style.paddingLeft) + 1;
      const right = rect.right - parseFloat(style.paddingRight) - 1;
      const overflow = [...panel.querySelectorAll(".reply-home,.reply,.md,.md-table-wrap,pre,.composer,textarea")].filter(el => {
        const r = el.getBoundingClientRect();
        return r.left < left - 1 || r.right > right + 1 || (el.tagName === "TEXTAREA" && (r.top < rect.top + 1 || r.bottom > rect.bottom - 1));
      }).map(el => el.className || el.tagName);
      const home = panel.querySelector(".reply-home");
      if (home.scrollWidth > home.clientWidth + 1) overflow.push("reply-home内部の横はみ出し");
      for (const el of panel.querySelectorAll("pre,.md-table-wrap")) {
        if (el.scrollWidth <= el.clientWidth + 1) continue;
        if (!["auto", "scroll"].includes(getComputedStyle(el).overflowX)) overflow.push(`${el.tagName}の横スクロール不可`);
        el.scrollLeft = el.scrollWidth;
        if (Math.abs(el.scrollLeft - (el.scrollWidth - el.clientWidth)) > 1) overflow.push(`${el.tagName}の末尾へ到達不可`);
      }
      const reply = panel.querySelector(".reply");
      if (reply.scrollHeight > reply.clientHeight + 1) {
        if (!["auto", "scroll"].includes(getComputedStyle(reply).overflowY)) overflow.push("本文の縦スクロール不可");
        reply.scrollTop = reply.scrollHeight;
        if (Math.abs(reply.scrollTop - (reply.scrollHeight - reply.clientHeight)) > 1) overflow.push("本文の末尾へ到達不可");
      }
      return { radius: parseFloat(style.borderTopLeftRadius), overflow };
    });
    if (mode === "pastel" && result.radius > 12) failures.push(`${mode}/${theme}/${width}: 角丸${result.radius}px`);
    if (result.overflow.length) failures.push(`${mode}/${theme}/${width}: ${result.overflow.join(",")}が親からはみ出し`);
  }
  assert.deepEqual(failures, [], failures.join("\n"));
  console.log("可愛い・かっこいい×明暗×200/320px: 角丸と長いコード・表の包含検査成功");
} finally {
  await browser.close();
}
