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
    PICOAGENT_STREAM_PROFILES: { hiyori: { background: "simple", backgroundImage: "", display: "clock", displayImage: "", captionColor: "#a04070" } },
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
  const placementCount = await page.locator(".placement-actions button").count();
  if (placementCount !== 4) throw new Error(`画面配置ボタンが4件ではありません: ${placementCount}`);
  if (await page.locator(".layout-live-preview img").count() !== 1) throw new Error("選択中レイアウトの大型プレビューがありません");
  if (await page.locator('[data-setting-key="PICOAGENT_STREAM_BACKGROUND"]').count() !== 1) throw new Error("配信背景の選択欄がありません");
  const previewHeight = await page.locator(".layout-live-preview img").evaluate(node => node.getBoundingClientRect().height);
  if (previewHeight > 241) throw new Error(`選択中レイアウトのプレビューが大きすぎます: ${previewHeight}px`);
  await page.locator("main").evaluate(node => { node.scrollTop = 0; });
  await page.waitForTimeout(100);
  mkdirSync(OUT, { recursive: true });
  await page.screenshot({ path: join(OUT, "display-tab.png") });
  await page.getByRole("tab", { name: "基本設定" }).click();
  if (await page.locator(".quick-settings-card").count() !== 4) throw new Error("よく使う設定への入口が4件ではありません");
  if (await page.locator('[data-setting-key="PICOAGENT_MODEL"]').isVisible()) throw new Error("ローカルAIで不要なAIモデル欄が表示されています");
  await page.screenshot({ path: join(OUT, "basic-tab.png") });
  await page.locator('[data-setting-key="PICOAGENT_NAP_MINUTES"] input').fill("20");
  const dirtyText = await page.locator("#dirtySummary").textContent();
  if (!dirtyText?.includes("未保存")) throw new Error(`未保存件数が表示されません: ${dirtyText}`);
  if (!(await page.locator("#save").textContent())?.includes("1件")) throw new Error("保存ボタンに変更件数が表示されません");
  await page.locator("#resetChanges").click();
  await page.locator('[data-settings-query="読み上げ"]').click();
  if (await page.locator("#clearSettingsSearch").isHidden()) throw new Error("検索解除ボタンが表示されません");
  if (await page.locator('.searchable:not([hidden])').count() < 1) throw new Error("音声の検索候補が表示されません");
  await page.locator("#clearSettingsSearch").click();
  if (await page.locator("#settingsSearch").inputValue() !== "") throw new Error("検索を解除できません");
  await page.getByRole("tab", { name: "キャラ・音声" }).click();
  await page.waitForSelector(".framing-editor");
  await page.waitForSelector(".stream-profile-editor");
  const streamEnabled = await page.locator('.stream-profile-editor input[type="checkbox"]').isChecked();
  if (!streamEnabled) throw new Error("ひよりのキャラ別配信設定が読み込まれていません");
  if (await page.locator('.stream-profile-editor input[type="color"]').inputValue() !== "#a04070") throw new Error("キャラ別字幕色が読み込まれていません");
  const muteStatus = await page.locator(".tts-master-status").textContent();
  if (!muteStatus?.includes("ミュート中")) throw new Error(`読み上げのミュート状態が分かりません: ${muteStatus}`);
  console.log(`設定画面確認: 入口4件 / 表示形式8件 / 配置=${placementCount}件 / 未保存件数・検索解除・構図・配信・ミュート表示OK / 選択=${selected} / ${join(OUT, "display-tab.png")}`);
} finally {
  await browser.close();
  await new Promise(resolveClose => server.close(resolveClose));
}
