// 設定画面の「画面」タブを配布UIから描画し、8形式のカードを回帰確認する。
import { createReadStream, existsSync, mkdirSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const ROOT = resolve(import.meta.dirname, "../..");
const DIST = join(ROOT, "ui-dist");
const OUT = join(ROOT, "temp", "settings-check");
const playwrightModule = process.env.PLAYWRIGHT_MODULE;
if (!playwrightModule || !existsSync(playwrightModule)) {
  throw new Error("PLAYWRIGHT_MODULE に Playwright の index.mjs を指定してください");
}
const { chromium } = await import(pathToFileURL(playwrightModule).href);
const mime = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".json": "application/json", ".webp": "image/webp", ".png": "image/png" };
const server = createServer((request, response) => {
  const relative = normalize(decodeURIComponent(new URL(request.url, "http://localhost").pathname)).replace(/^[/\\]+/, "") || "index.html";
  const path = resolve(DIST, relative);
  if (!path.startsWith(resolve(DIST)) || !existsSync(path)) {
    response.writeHead(404).end("not found");
    return;
  }
  response.writeHead(200, { "Content-Type": mime[extname(path)] || "application/octet-stream" });
  createReadStream(path).pipe(response);
});
await new Promise(resolveListen => server.listen(0, "127.0.0.1", resolveListen));
const port = server.address().port;

const settingsData = {
  values: {
    PICOAGENT_PROVIDER: "local",
    PICOAGENT_CHARACTER: "hiyori",
    PICOAGENT_LAYOUT: "stream",
    PICOAGENT_THEME: "dark",
    PICOAGENT_CHARACTER_ZOOM: "1.55",
    PICOAGENT_ALWAYS_ON_TOP: true,
  },
  active: { providerId: "local", model: "gemma" },
  defaults: { systemPrompt: "" },
  providers: [{ id: "local", label: "ローカル", defaultModel: "gemma", configSchema: [] }],
  tools: [],
  relationships: [],
  localModelPresets: [],
};
const mock = `
window.__listeners = {};
window.__TAURI__ = {
  core: { invoke: async (name) => {
    if (name === "settings_load") return ${JSON.stringify(settingsData)};
    if (name === "pet_user_characters") return { characters: [], root: "" };
    if (name === "settings_tha3_status") return { ready: false, root: "未設定", installHint: "任意", sourceReady: false, modelReady: false, runtimeReady: false };
    if (name === "pet_list_downloaded_models") return { models: [] };
    if (name === "settings_list_models") return [];
    if (name === "engine_status") return { phase: "ready" };
    if (name === "aivis_status") return { enabled: true, up: true };
    if (name === "pet_usage") return {};
    if (name === "pet_traces") return { traces: [], tools: [] };
    return {};
  } },
  event: { listen: async (name, callback) => { (window.__listeners[name] ||= []).push(callback); return () => {}; } },
  window: { getCurrentWindow: () => ({ setFullscreen: async () => {}, setSize: async () => {}, setPosition: async () => {}, startDragging: async () => {}, scaleFactor: async () => 1, outerPosition: async () => ({ x: 0, y: 0 }) }) },
  dpi: {},
};`;

const browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM || undefined });
try {
  const page = await browser.newPage({ viewport: { width: 960, height: 760 }, deviceScaleFactor: 1 });
  await page.addInitScript(mock);
  page.on("pageerror", error => { throw error; });
  await page.goto(`http://127.0.0.1:${port}/settings.html?tab=display`);
  await page.waitForSelector(".layout-card-grid .layout-card");
  const cardCount = await page.locator(".layout-card").count();
  if (cardCount !== 8) throw new Error(`表示形式カードが8件ではありません: ${cardCount}`);
  await page.waitForFunction(() => [...document.querySelectorAll(".layout-card img")].every(image => image.complete && image.naturalWidth > 0));
  const selected = await page.locator(".layout-card.selected strong").textContent();
  if (selected !== "配信") throw new Error(`保存中の配信カードが選択状態ではありません: ${selected}`);
  await page.getByTitle("チャットへ変更").click();
  const selectedValue = await page.locator("select.layout-native-select").inputValue();
  if (selectedValue !== "chat") throw new Error(`カード選択が設定値へ反映されません: ${selectedValue}`);
  await page.getByTitle("配信へ変更").click();
  await page.locator("main").evaluate(node => { node.scrollTop = 0; });
  await page.waitForTimeout(100);
  mkdirSync(OUT, { recursive: true });
  await page.screenshot({ path: join(OUT, "display-tab.png") });
  console.log(`画面タブ確認: 8件 / 選択=${selected} / ${join(OUT, "display-tab.png")}`);
} finally {
  await browser.close();
  await new Promise(resolveClose => server.close(resolveClose));
}
