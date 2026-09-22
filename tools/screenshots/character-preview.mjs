// キャラ作成の確認用ウィンドウ（character-preview.html）を、同梱のセナのリグで描いて撮る。
// キャラ作成の実行結果が無くても、原画と2.5D描画の並びや操作部品の崩れを確かめられる。
// 使い方: node tools/screenshots/character-preview.mjs [出力PNG] [幅] [高さ]
import { createReadStream, existsSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const root = resolve(import.meta.dirname, "../..");
const [out = join(root, "temp", "character-preview.png"), width = "1100", height = "780"] = process.argv.slice(2);
const dist = join(root, "ui-dist");
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const mime = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".png": "image/png" };
const server = createServer((request, response) => {
  const relative = normalize(decodeURIComponent(new URL(request.url, "http://x").pathname)).replace(/^[/\\]+/, "");
  const path = resolve(dist, relative);
  if (!path.startsWith(resolve(dist)) || !existsSync(path)) { response.writeHead(404).end(); return; }
  response.writeHead(200, { "Content-Type": mime[extname(path)] || "application/octet-stream" });
  createReadStream(path).pipe(response);
});
await new Promise(ok => server.listen(0, "127.0.0.1", ok));
const port = server.address().port;
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: Number(width), height: Number(height) } });
await page.addInitScript(`
  window.__TAURI__ = {
    core: { convertFileSrc: path => path, invoke: async name => {
      if (name === "settings_character_maker_preview_data") return {
        displayName: "セナ", sourcePath: "/assets/characters/sena/rig2d/parts/neutral.png",
        rigPath: "/assets/characters/sena/rig2d/rig.json", rigRootPath: "/assets/characters/sena/rig2d/", assetVersion: "",
      };
      return null;
    } },
    event: { listen: async () => () => {} },
    window: { getCurrentWindow: () => ({ label: "preview", onCloseRequested: async () => () => {} }) },
    dpi: {},
  };
`);
page.on("pageerror", error => console.error("pageerror", error.message));
await page.goto(`http://127.0.0.1:${port}/character-preview.html?characterId=sena`);
await page.waitForTimeout(4000);
console.log(await page.locator("#status").textContent());
const overflow = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
if (overflow) throw new Error("キャラ確認画面が横にはみ出しています");
await page.screenshot({ path: out, fullPage: true });
await browser.close();
server.close();
console.log(`キャラ確認画面: ${out}`);
