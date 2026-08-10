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
  product: { name: "PicoAgent", version: "1.0.0", identifier: "com.picoagent.desktop" },
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
  core: { convertFileSrc: path => path, invoke: async (name) => {
    if (name === "settings_load") return ${JSON.stringify(settingsData)};
    if (name === "pet_user_characters") return { characters: [{ id: "user-test", name: "追加テスト", path: "http://127.0.0.1:${port}/assets/characters/mugi", user: true }], root: "" };
    if (name === "settings_character_remove") return { ok: true, activeCharacter: "hiyori" };
    if (name === "settings_tha3_status") return { ready: false, root: "未設定", installHint: "任意", sourceReady: false, modelReady: false, runtimeReady: false };
    if (name === "settings_components_status") return [
      { id: "aivis", label: "音声読み上げ", description: "AivisSpeechでキャラクターの声を作ります。", sizeLabel: "約0.8GB＋音声モデル", ready: true, available: true, removable: true, installMode: "managed", installedVersion: "1.1.0", targetVersion: "1.2.0", updateAvailable: true, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
      { id: "whisper", label: "音声入力・文字起こし", description: "Whisperを端末内で動かします。", sizeLabel: "約0.5GB", ready: false, available: true, removable: false, installMode: "managed", updateAvailable: false, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
      { id: "tha3", label: "1枚絵からキャラ作成", description: "THA3で動く2Dキャラを作ります。", sizeLabel: "約5.2GB", ready: false, available: true, removable: false, installMode: "external", updateAvailable: false, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
      { id: "office", label: "Office変換", description: "LibreOfficeで文書変換を補助します。", sizeLabel: "外部アプリ", ready: false, available: true, removable: false, installMode: "external", updateAvailable: false, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
      { id: "tools", label: "PDF・画像・文書ツール", description: "FFmpeg等を追加します。", sizeLabel: "外部ツール", ready: false, available: true, removable: false, installMode: "external", updateAvailable: false, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
    ];
    if (name === "pet_list_downloaded_models") return { totalLabel: "2GB", models: [{ name: "gemma-test.gguf", label: "Gemma", sizeLabel: "2GB", active: false }] };
    if (name === "settings_list_models") return [];
    if (name === "settings_check_update") return { status: "available", downloadReady: true, message: "1.1.0を利用できます。" };
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
  if (await page.locator(".quick-settings-card").count() !== 5) throw new Error("よく使う設定への入口が5件ではありません");
  if (await page.locator('[data-setting-key="PICOAGENT_MODEL"]').isVisible()) throw new Error("ローカルAIで不要なAIモデル欄が表示されています");
  await page.screenshot({ path: join(OUT, "basic-tab.png") });
  await page.getByRole("tab", { name: "追加機能・更新（1）", exact: true }).click();
  await page.waitForSelector(".component-card");
  if (await page.locator(".component-card").count() !== 6) throw new Error("基本を含む追加機能カードが6件ではありません");
  if ((await page.locator(".component-card").allTextContents()).some(text => text.includes("null"))) throw new Error("追加機能カードに空値が文字として表示されています");
  await page.getByRole("button", { name: "1.2.0へ更新", exact: true }).click();
  if (!(await page.locator(".component-manager-status").textContent())?.includes("音声モデル・辞書・設定は残ります")) throw new Error("更新前に維持データが案内されません");
  await page.screenshot({ path: join(OUT, "components-tab.png") });
  await page.getByRole("tab", { name: "AIモデル・連携", exact: true }).click();
  if (await page.getByRole("button", { name: "一覧を再読み込み", exact: true }).count() !== 1) throw new Error("モデル一覧の再読込操作が更新操作と区別できません");
  await page.getByRole("button", { name: "削除", exact: true }).click();
  if (!(await page.locator(".model-manager-status").textContent())?.includes("再ダウンロードが必要")) throw new Error("モデル削除前に再ダウンロードが案内されません");
  await page.getByRole("tab", { name: "基本設定" }).click();
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
  const addedCharacter = page.locator(".character-card").filter({ hasText: "追加テスト" });
  await addedCharacter.getByRole("button", { name: "このキャラを選ぶ", exact: true }).click();
  await addedCharacter.getByRole("button", { name: "削除", exact: true }).click();
  await addedCharacter.getByRole("button", { name: "もう一度押して削除", exact: true }).click();
  if (await addedCharacter.count()) throw new Error("削除した追加キャラが一覧に残っています");
  if (await page.locator('[data-setting-key="PICOAGENT_CHARACTER"] select').inputValue() !== "hiyori") throw new Error("未保存選択の削除後に保存中キャラへ戻りません");
  if (!(await page.locator('textarea[placeholder="性格・口調・一人称など"]').inputValue()).includes("明るく元気")) throw new Error("削除後の選択と人格表示が一致しません");
  if (await page.locator("#dirtySummary").textContent() !== "保存済み") throw new Error("既定人格だけで未保存扱いになっています");
  await page.getByRole("tab", { name: "診断・修復" }).click();
  if (await page.locator(".product-info-card").count() !== 5) throw new Error("製品情報カードが5件ではありません");
  if (!(await page.locator(".product-info-card").first().textContent())?.includes("1.0.0")) throw new Error("製品バージョンが表示されません");
  if (!(await page.locator(".product-update-card").textContent())?.includes("再ダウンロードしません")) throw new Error("アプリ更新で維持されるデータが案内されません");
  await page.getByRole("button", { name: "更新を確認", exact: true }).click();
  await page.getByRole("button", { name: "アップデート", exact: true }).click();
  if (await page.getByRole("button", { name: "もう一度押してダウンロード", exact: true }).count() !== 1) throw new Error("アプリ更新が二段階確認になっていません");
  if (!(await page.locator(".product-update-status").textContent())?.includes("モデル・追加機能は残ります")) throw new Error("アプリ更新前に維持データが案内されません");
  await page.screenshot({ path: join(OUT, "diagnostics-tab.png") });
  await page.setViewportSize({ width: 400, height: 820 });
  const [resetBox, saveBox] = await Promise.all([
    page.locator("#resetChanges").boundingBox(),
    page.locator("#save").boundingBox(),
  ]);
  if (!resetBox || !saveBox || resetBox.width < 140 || saveBox.width < 140) throw new Error("狭幅で保存操作の可読幅が足りません");
  if (Math.abs(resetBox.y - saveBox.y) > 2) throw new Error("狭幅で保存と取り消しが不揃いです");
  const horizontalOverflow = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
  if (horizontalOverflow) throw new Error("狭幅でページ全体が横にはみ出しています");
  await page.screenshot({ path: join(OUT, "narrow-footer.png") });
  console.log(`設定画面確認: 入口5件 / 追加機能6件 / 表示形式8件 / 配置=${placementCount}件 / 未保存件数・検索解除・製品情報・構図・配信・ミュート・狭幅フッター表示OK / 選択=${selected} / ${join(OUT, "display-tab.png")}`);
} finally {
  await browser.close();
  await new Promise(resolveClose => server.close(resolveClose));
}
