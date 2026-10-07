// 実WebViewへの検査接続は、利用者の配色・省モーション・フォーカスを変更しない。
import { mkdir } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));

export async function withNativePage(action, options = {}) {
  const port = Number(options.port ?? process.env.NATIVE_DEBUG_PORT ?? 9222);
  const target = options.target ?? process.env.NATIVE_WINDOW ?? "main";
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error("実WebViewの検査ポートが不正です。");
  if (!["main", "settings"].includes(target)) throw new Error("検査対象はmainまたはsettingsです。");
  const modulePath = process.env.PLAYWRIGHT_MODULE;
  const { chromium } = await import(modulePath ? pathToFileURL(modulePath).href : "playwright");
  // 接続後のemulateMedia解除では、接続開始から解除までに白いフレームが出る。
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { noDefaults: true });
  try {
    const pages = browser.contexts().flatMap(context => context.pages());
    const page = pages.find(page => {
      const url = new URL(page.url());
      if (target === "settings") return url.pathname === "/settings.html";
      return url.pathname === "/index.html"
        || url.hostname === "picoboot.localhost"
        || (url.hostname === "127.0.0.1" && url.port === "1430" && url.pathname === "/");
    });
    if (!page) throw new Error(`PicoAgentの${target === "settings" ? "設定" : "メイン"}画面が見つかりません。`);
    return await action(page);
  } finally {
    await browser.close();
  }
}

async function main() {
  const command = process.argv[2] || "state";
  await withNativePage(async page => {
    if (command === "screenshot") {
      const path = resolve(process.argv[3] || resolve(root, "temp/native-state.png"));
      const fromTemp = relative(resolve(root, "temp"), path);
      if (!fromTemp || fromTemp === ".." || fromTemp.startsWith(`..\\`) || fromTemp.startsWith("../") || isAbsolute(fromTemp)) {
        throw new Error("実機検査の画像は、この作業ツリーのtemp/内へ保存してください。");
      }
      await mkdir(dirname(path), { recursive: true });
      await page.screenshot({ path });
      console.log(`撮影: ${path}`);
      return;
    }
    if (command !== "state") throw new Error("指定できる操作はstateまたはscreenshotです。");
    console.log(JSON.stringify(await page.evaluate(async () => ({
      url: location.href,
      layout: document.querySelector("#petApp")?.dataset.layout,
      booting: document.querySelector("#petApp")?.classList.contains("is-booting"),
      character: document.querySelector('[data-source="character"]')?.dataset.characterId,
      renderKind: document.querySelector(".character-view")?.dataset.renderKind,
      viewport: [innerWidth, innerHeight, devicePixelRatio],
      position: await window.__TAURI__.window.getCurrentWindow().outerPosition(),
      dark: matchMedia("(prefers-color-scheme: dark)").matches,
      panelColor: getComputedStyle(document.querySelector(".chat") || document.documentElement).backgroundColor,
    })), null, 2));
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
