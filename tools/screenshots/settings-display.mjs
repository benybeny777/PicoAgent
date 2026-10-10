// 設定画面の「画面」タブを配布UIから描画し、現行3形式のカードを回帰確認する。
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
    PICOAGENT_MODEL: "lfm25_2_6b",
    PICOAGENT_LOCAL_MODEL_PRESET: "lfm25_2_6b",
    PICOAGENT_CHARACTER: "mugi",
    PICOAGENT_RENDERER_OVERRIDES: {},
    PICOAGENT_LAYOUT: "stream",
    PICOAGENT_THEME: "dark",
    PICOAGENT_STREAM_CHAT_FONT_PX: "30",
    PICOAGENT_CHARACTER_ZOOM: "1.55",
    PICOAGENT_STREAM_PROFILES: { mugi: { background: "simple", backgroundImage: "", display: "clock", displayImage: "", captionColor: "#a04070" } },
    PICOAGENT_ALWAYS_ON_TOP: true,
    PICOAGENT_MULTI_AI_MODE: "consensus",
    PICOAGENT_MULTI_AI_AGENTS: JSON.stringify([
      { name: "ローカル", provider: "local", model: "gemma", focus: "" },
      { name: "Codex", provider: "agent_cli", model: "codex", focus: "技術確認" },
    ]),
    PICOAGENT_MULTI_AI_JUDGE_PROVIDER: "local",
    PICOAGENT_MULTI_AI_JUDGE_MODEL: "gemma",
    PICOAGENT_MULTI_AGENT_MAX_CONCURRENCY: "3",
  },
  active: { providerId: "local", model: "lfm25_2_6b" },
  defaults: { systemPrompt: "" },
  providers: [
    { id: "local", label: "ローカル", defaultModel: "lfm25_2_6b", configSchema: [
      { key: "PICOAGENT_LOCAL_MODEL_PRESET", label: "ローカルモデル", type: "text", placeholder: "auto" },
    ] },
    { id: "ollama", label: "Ollama", defaultModel: "gemma3", configSchema: [] },
    { id: "openai", label: "OpenAI互換", defaultModel: "gpt-5", configSchema: [] },
    { id: "agent_cli", label: "手元のCLI", defaultModel: "claude", configSchema: [] },
  ],
  tools: [
    {
      name: "google_sheets",
      label: "Google スプレッドシート",
      description: "Google スプレッドシートを読み書きします。",
      configSchema: [
        { key: "PICOAGENT_GOOGLE_SHEETS_ACCESS_TOKEN", label: "Google スプレッドシートの OAuth アクセストークン", type: "password", placeholder: "ya29..." },
      ],
    },
    {
      name: "bluesky",
      label: "Bluesky",
      description: "Blueskyへ投稿します。",
      configSchema: [
        { key: "PICOAGENT_BLUESKY_IDENTIFIER", label: "Bluesky の識別子（ハンドルまたはメール）", type: "text", placeholder: "user.bsky.social" },
      ],
    },
    {
      name: "microsoft_graph",
      label: "Microsoft Graph",
      description: "Microsoft Graphで予定やメッセージを扱います。",
      configSchema: [
        { key: "PICOAGENT_MS_GRAPH_ACCESS_TOKEN", label: "Microsoft Graph の OAuth アクセストークン", type: "password", placeholder: "" },
      ],
    },
  ],
  relationships: [],
  defaultLocalModelPreset: "lfm25_2_6b",
  localHardware: { ramGb: 32, vramGb: 8.6, unifiedMemory: false },
  localModelPresets: [
    { id: "lfm25_2_6b", family: "Liquid AI", localSize: "LFM2.5 2.6B Agentic", label: "軽量LFM", recommendedVramGb: 4, recommendedRamGb: 8, downloadSize: "約1.59GB", japaneseNote: "日本語対応", note: "検証済みの既定モデル" },
    { id: "small", family: "Gemma", localSize: "Gemma 4 E2B", label: "Gemma軽量", recommendedVramGb: 0, recommendedRamGb: 8, downloadSize: "約3.1GB", japaneseNote: "日本語対応", note: "Gemma 4を継続選択可能" },
    { id: "qwen35_4b", family: "Qwen", localSize: "Qwen3.5 4B", label: "Qwen", recommendedVramGb: 4, recommendedRamGb: 12, downloadSize: "約2.7GB", japaneseNote: "日本語に強い", note: "日本語と指示追従" },
    { id: "gpt_oss_20b", family: "OpenAI", localSize: "gpt-oss-20b", label: "gpt-oss", recommendedVramGb: 16, recommendedRamGb: 32, downloadSize: "約12.1GB", japaneseNote: "日本語対応", note: "高性能PC向け" },
    { id: "phi4mini", family: "Phi", localSize: "Phi-4 mini", label: "Phi", recommendedVramGb: 4, recommendedRamGb: 12, downloadSize: "約2.5GB", japaneseNote: "日本語対応", note: "小型推論候補" },
  ],
};
const mock = `
window.__listeners = {};
window.__settingsData = JSON.parse(sessionStorage.getItem("mockSettingsData") || ${JSON.stringify(JSON.stringify(settingsData))});
window.__savedSettingsPatches = [];
window.__userCharactersRecovered = false;
if (location.search.includes("user-list-failure")) window.__settingsData.values.PICOAGENT_CHARACTER = "user-test";
window.__memoryItems = [
  { category: "好み", text: "私は結論から読むのが好き", updated_at: "2026-08-18T08:30:00Z", hits: 4 },
  { category: "継続指示", text: "今後はサイズをMBかGBで表示してほしい", updated_at: "2026-08-18T07:15:00Z", hits: 2 },
];
window.__TAURI__ = {
  core: { convertFileSrc: path => path, invoke: async (name, args = {}) => {
    if (name === "settings_load") {
      if (location.search.includes("load-failure") && !sessionStorage.getItem("settings-load-retried")) {
        sessionStorage.setItem("settings-load-retried", "1");
        throw new Error("設定読込の検証用エラー");
      }
      return window.__settingsData;
    }
    if (name === "settings_save") {
      window.__savedSettingsPatches.push(args.patch);
      Object.assign(window.__settingsData.values, args.patch);
      sessionStorage.setItem("mockSettingsData", JSON.stringify(window.__settingsData));
      return { up: true };
    }
    if (name === "settings_memory_list") return { items: window.__memoryItems };
    if (name === "settings_memory_delete") {
      window.__memoryItems = window.__memoryItems.filter(item => !(
        item.category === args.category && item.text === args.text && item.updated_at === args.updatedAt
      ));
      return { items: window.__memoryItems };
    }
    if (name === "pet_user_characters") {
      if (location.search.includes("user-list-failure") && !window.__userCharactersRecovered) throw new Error("追加キャラクター一覧の検証用エラー");
      return { characters: [{ id: "user-test", name: "追加テスト", path: "http://127.0.0.1:${port}/assets/characters/mugi", user: true }], root: "" };
    }
    if (name === "settings_character_discover_downloads") return { candidates: [
      { kind: "live2d", name: "ひより", path: "C:\\Users\\me\\Downloads\\ひより.zip" },
      { kind: "mmd", name: "モデル", path: "C:\\Users\\me\\Downloads\\モデル.pmx" },
    ] };
    if (name === "settings_character_rename") return { ok: true, id: args.id, name: args.name };
    if (name === "settings_character_remove") return { ok: true, activeCharacter: "mugi" };
    if (name === "settings_export_diagnostics") return { path: "C:\\\\Users\\\\me\\\\AppData\\\\Roaming\\\\PicoAgent\\\\log\\\\diagnostics.2026-09-02.080000.txt" };
    if (name === "settings_components_status") return [
      { id: "aivis", label: "音声読み上げ", description: "AivisSpeechでキャラクターの声を作ります。", sizeLabel: "約0.8GB＋音声モデル", ready: true, available: true, removable: true, installMode: "managed", installedVersion: "1.1.0", targetVersion: "1.2.0", updateAvailable: true, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
      { id: "whisper", label: "音声入力・文字起こし", description: "Whisperを端末内で動かします。", sizeLabel: "約0.5GB", ready: false, available: true, removable: false, installMode: "managed", updateAvailable: false, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
      { id: "character-maker", label: "キャラクター作成（テスト）", description: "原画からキャラクターを生成します。", sizeLabel: "約77GB", ready: false, available: true, removable: true, installMode: "managed", updateAvailable: false, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
      { id: "office", label: "Office変換", description: "LibreOfficeで文書変換を補助します。", sizeLabel: "外部アプリ", ready: false, available: true, removable: false, installMode: "external", updateAvailable: false, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
      { id: "tools", label: "PDF・画像・文書ツール", description: "FFmpeg等を追加します。", sizeLabel: "外部ツール", ready: false, available: true, removable: false, installMode: "external", updateAvailable: false, newerThanTarget: false, versionUnknown: false, versionError: null, sources: [] },
    ];
    if (name === "settings_components_install" && args.ids?.includes("character-maker")) {
      for (const callback of window.__listeners["component:progress"] || []) callback({ payload: { message: "Python管理ツールを取得中…", downloaded: 0, total: 0 } });
      await new Promise(resolve => setTimeout(resolve, 1200));
      for (const callback of window.__listeners["component:progress"] || []) callback({ payload: { message: "モデルを取得中…", downloaded: 25, total: 100 } });
      await new Promise(resolve => setTimeout(resolve, 1200));
      return [];
    }
    if (name === "pet_list_downloaded_models") return { totalLabel: "2GB", models: [{ name: "gemma-test.gguf", label: "Gemma", sizeLabel: "2GB", active: false }] };
    if (name === "settings_list_models") {
      if (location.search.includes("slow-models")) {
        window.__modelListPending = true;
        await new Promise(resolve => setTimeout(resolve, 1000));
        window.__modelListPending = false;
      }
      return args.id === "agent_cli"
        ? ["claude", "claude:opus", "claude:sonnet", "codex", "codex:future-model"] : [];
    }
    if (name === "settings_check_update") return { status: "available", downloadReady: true, message: "1.1.0を利用できます。" };
    if (name === "engine_status") return { phase: "ready" };
    // 学習セクションは全タブ構築時に一覧を読む。空オブジェクトではなくAPIの正規形を返す。
    if (name === "settings_character_learning_list") return { supported: true, runtimeInstalled: true, activeId: null, jobs: [] };
    if (name === "aivis_status") return { enabled: true, up: true };
    if (name === "pet_usage") return {};
    if (name === "pet_traces") return { traces: [], tools: [] };
    return {};
  } },
  event: { listen: async (name, callback) => { (window.__listeners[name] ||= []).push(callback); return () => {}; } },
  window: { getCurrentWindow: () => ({ setFullscreen: async () => {}, setSize: async () => {}, setPosition: async () => {}, startDragging: async () => {}, scaleFactor: async () => 1, outerPosition: async () => ({ x: 0, y: 0 }) }) },
  dpi: {},
};`;

const headful = process.env.PICOAGENT_UI_HEADFUL === "1";
const browser = await chromium.launch({
  executablePath: process.env.PLAYWRIGHT_CHROMIUM || undefined,
  headless: !headful,
  args: headful ? ["--window-position=0,0"] : [],
});
try {
  const page = await browser.newPage({ viewport: { width: 960, height: 760 }, deviceScaleFactor: 1 });
  await page.addInitScript(mock);
  page.on("pageerror", error => { throw error; });
  await page.goto(`http://127.0.0.1:${port}/settings.html?tab=display`);
  if (headful) {
    const windowBounds = await page.evaluate(() => ({ x: window.screenX, width: window.outerWidth }));
    const leftScreenRight = Number(process.env.PICOAGENT_LEFT_SCREEN_RIGHT || 0);
    if (leftScreenRight > 0 && (windowBounds.x < 0 || windowBounds.x + windowBounds.width > leftScreenRight)) {
      throw new Error(`可視確認ウィンドウが左画面内にありません: x=${windowBounds.x}, width=${windowBounds.width}`);
    }
  }
  await page.waitForSelector(".layout-card-grid .layout-card").catch(async error => {
    console.error("設定画面の初期表示診断:", (await page.locator("body").innerText()).slice(0,2500));
    throw error;
  });
  const displayTab = page.getByRole("tab", { name: "画面・操作", exact: true });
  await displayTab.focus();
  for (const [key, expected] of [["Home", "基本設定"], ["End", "診断・修復"], ["ArrowRight", "基本設定"], ["ArrowLeft", "診断・修復"], ["Home", "基本設定"], ["ArrowDown", "画面・操作"]]) {
    await page.keyboard.press(key);
    const tab = page.getByRole("tab", { name: expected, exact: true });
    if (await tab.getAttribute("aria-selected") !== "true" || !(await tab.evaluate(node => node === document.activeElement))) throw new Error(`${key}で設定タブを移動できません`);
    if (await page.locator('.tabs .tab[tabindex="0"]').count() !== 1) throw new Error("Tabキーの移動先が選択中のタブに絞られていません");
  }
  const cardCount = await page.locator(".layout-card").count();
  if (cardCount !== 3) throw new Error(`表示形式カードが3件ではありません: ${cardCount}`);
  await page.waitForFunction(() => [...document.querySelectorAll(".layout-card img")].every(image => image.complete && image.naturalWidth > 0));
  const selected = await page.locator(".layout-card.selected strong").textContent();
  if (selected !== "配信") throw new Error(`保存中の配信カードが選択状態ではありません: ${selected}`);
  await page.getByTitle("ミニへ変更").click();
  const selectedValue = await page.locator("select.layout-native-select").inputValue();
  if (selectedValue !== "mascot") throw new Error(`カード選択が設定値へ反映されません: ${selectedValue}`);
  await page.getByTitle("配信へ変更").click();
  if (await page.getByText("表示するモニター", { exact: true }).count()) throw new Error("不要なモニター指定欄が残っています");
  if (await page.locator(".layout-live-preview img").count() !== 1) throw new Error("選択中レイアウトの大型プレビューがありません");
  if (await page.locator('[data-setting-key="PICOAGENT_STREAM_BACKGROUND"]').count() !== 1) throw new Error("配信背景の選択欄がありません");
  const previewHeight = await page.locator(".layout-live-preview img").evaluate(node => node.getBoundingClientRect().height);
  if (previewHeight > 241) throw new Error(`選択中レイアウトのプレビューが大きすぎます: ${previewHeight}px`);
  await page.locator("main").evaluate(node => { node.scrollTop = 0; });
  await page.waitForTimeout(100);
  mkdirSync(OUT, { recursive: true });
  await page.screenshot({ path: join(OUT, "display-tab.png") });
  const streamFont = page.locator("#setting-PICOAGENT_STREAM_CHAT_FONT_PX");
  if (await streamFont.inputValue() !== "30") throw new Error("配信文字サイズの初期値が30ではありません");
  await streamFont.fill("42");
  if (await page.locator("#save").isDisabled()) throw new Error("配信文字サイズの変更を未保存として検出しません");
  await page.locator("#save").click();
  await page.waitForFunction(() => document.getElementById("status")?.textContent?.includes("保存しました"));
  const savedFont = await page.evaluate(() => window.__savedSettingsPatches.at(-1)?.PICOAGENT_STREAM_CHAT_FONT_PX);
  if (savedFont !== "42") throw new Error(`配信文字サイズを保存していません: ${savedFont}`);
  await page.reload();
  await page.waitForSelector("#setting-PICOAGENT_STREAM_CHAT_FONT_PX");
  if (await page.locator("#setting-PICOAGENT_STREAM_CHAT_FONT_PX").inputValue() !== "42") throw new Error("再読込後に配信文字サイズが戻りました");
  await page.getByRole("tab", { name: "基本設定" }).click();
  if (await page.locator(".quick-settings-card").count() !== 8) throw new Error("よく使う設定への入口が8件ではありません");
  // 基本設定は会話と記憶だけにした。別分類の項目が戻っていないことを確かめる。
  for (const key of ["PICOAGENT_PROVIDER", "PICOAGENT_NAP_MINUTES", "PICOAGENT_HOTKEY_SUMMON", "PICOAGENT_UPDATE_CHECK_ENABLED", "PICOAGENT_API_ENABLED", "PICOAGENT_MCP_SERVERS"]) {
    if (await page.locator(`#panel-basic [data-setting-key="${key}"]`).count()) throw new Error(`基本設定に別分類の ${key} が残っています`);
  }
  await page.waitForSelector(".memory-item");
  const memoryCheckbox = page.getByRole("checkbox", { name: "会話から好みや継続指示を暗黙的に覚える", exact: true });
  const memoryEnabled = await memoryCheckbox.isChecked();
  const memoryLabel = page.locator("label").filter({ hasText: "会話から好みや継続指示を暗黙的に覚える" });
  await memoryLabel.click();
  if (await memoryCheckbox.isChecked() === memoryEnabled) throw new Error("チェック項目のラベルを押しても値が切り替わりません");
  await memoryLabel.click();
  if (await page.locator(".memory-item").count() !== 2) throw new Error("長期メモリが2件表示されていません");
  const memoryText = await page.locator(".memory-manager").textContent();
  for (const expected of ["全キャラクター", "覚えた日時", "言及 4回", "この記憶を削除"]) {
    if (!memoryText?.includes(expected)) throw new Error(`長期メモリ欄に「${expected}」がありません`);
  }
  await page.screenshot({ path: join(OUT, "basic-tab.png") });
  // Playwright のクリックは自動でスクロールするため、人が下側のタブへ届くことは別に確かめる。
  const lastTabReachable = await page.evaluate(() => {
    const tabs = document.querySelector(".tabs");
    const last = document.getElementById("tab-diagnostics");
    if (!tabs || !last) return { ok: false, reason: "タブ一覧がありません" };
    tabs.scrollTop = tabs.scrollHeight;
    const listBox = tabs.getBoundingClientRect();
    const lastBox = last.getBoundingClientRect();
    const footerTop = document.querySelector("footer")?.getBoundingClientRect().top ?? window.innerHeight;
    const ok = lastBox.bottom <= listBox.bottom + 1 && lastBox.bottom <= footerTop;
    tabs.scrollTop = 0;
    return { ok, listBottom: listBox.bottom, lastBottom: lastBox.bottom, footerTop };
  });
  if (!lastTabReachable.ok) throw new Error(`960×760で最下段の診断・修復タブへ届きません: ${JSON.stringify(lastTabReachable)}`);
  await page.locator(".memory-manager").scrollIntoViewIfNeeded();
  await page.locator(".memory-manager").screenshot({ path: join(OUT, "memory-manager.png") });
  await page.locator(".memory-item").first().getByRole("button", { name: /長期メモリから削除/ }).click();
  if (await page.locator(".memory-item").count() !== 1) throw new Error("長期メモリの個別削除が一覧へ反映されません");
  if ((await page.locator(".memory-manager").textContent())?.includes("私は結論から読むのが好き")) throw new Error("削除した長期メモリが一覧に残っています");
  await page.getByRole("tab", { name: "追加機能・更新（1）", exact: true }).click();
  await page.waitForSelector(".component-card");
  if (await page.locator(".component-card").count() !== 6) throw new Error("基本を含む追加機能カードが6件ではありません");
  if ((await page.locator(".component-card").allTextContents()).some(text => text.includes("null"))) throw new Error("追加機能カードに空値が文字として表示されています");
  await page.getByRole("button", { name: "1.2.0へ更新", exact: true }).click();
  if (!(await page.locator(".component-manager-status").textContent())?.includes("音声モデル・辞書・設定は残ります")) throw new Error("更新前に維持データが案内されません");
  await page.locator(".component-card").filter({ hasText: "キャラクター作成（テスト）" }).getByRole("button", { name: "公式からインストール", exact: true }).click();
  const operationProgress = page.locator(".operation-progress.is-active");
  await operationProgress.waitFor();
  if (!(await operationProgress.textContent())?.includes("Python管理ツールを取得中")) throw new Error("取得中の工程名が進捗欄へ表示されません");
  await page.waitForFunction(() => document.querySelector(".operation-progress-elapsed")?.textContent?.includes("1秒"));
  await page.waitForFunction(() => document.querySelector(".operation-progress-track")?.getAttribute("aria-valuenow") === "25");
  if (!(await operationProgress.textContent())?.includes("モデルを取得中")) throw new Error("取得量が分かる工程へ進捗表示が更新されません");
  const progressBox = await operationProgress.boundingBox();
  if (!progressBox || progressBox.y < 0 || progressBox.y + progressBox.height > 760) throw new Error(`導入中の進捗欄が画面内にありません: ${JSON.stringify(progressBox)}`);
  const progressBackground = await operationProgress.evaluate(node => getComputedStyle(node).backgroundColor);
  if (progressBackground === "rgba(0, 0, 0, 0)" || progressBackground === "transparent") throw new Error("導入中の進捗欄の背景が透明です");
  const progressFillBackground = await operationProgress.locator(".operation-progress-fill").evaluate(node => getComputedStyle(node).backgroundImage);
  if (!progressFillBackground || progressFillBackground === "none") throw new Error("導入中の進捗バーに塗り色がありません");
  await page.screenshot({ path: join(OUT, "components-tab.png") });
  if (await page.locator('#panel-components [data-setting-key="PICOAGENT_UPDATE_CHECK_ENABLED"]').count() !== 1) throw new Error("起動時の更新確認が追加機能・更新タブにありません");
  // アプリ本体の更新は追加機能と同じタブで扱う。維持データの案内と二段階確認を保つ。
  if (!(await page.locator("#panel-components .product-update-card").textContent())?.includes("再ダウンロードしません")) throw new Error("アプリ更新で維持されるデータが案内されません");
  await page.locator("#panel-components").getByRole("button", { name: "更新を確認", exact: true }).click();
  await page.locator("#panel-components").getByRole("button", { name: "アップデート", exact: true }).click();
  if (await page.getByRole("button", { name: "もう一度押してダウンロード", exact: true }).count() !== 1) throw new Error("アプリ更新が二段階確認になっていません");
  if (!(await page.locator(".product-update-status").textContent())?.includes("モデル・追加機能は残ります")) throw new Error("アプリ更新前に維持データが案内されません");
  await page.getByRole("tab", { name: "スキル・連携", exact: true }).click();
  const skillGenres = page.locator("#panel-tools .genre-filter").first();
  await skillGenres.getByRole("button", { name: "SNS/投稿", exact: true }).click();
  if (await skillGenres.getByRole("button", { pressed: true }).textContent() !== "SNS/投稿") throw new Error("スキルの絞り込み状態が読み上げに反映されません");
  if (await page.locator(".card-item:visible").filter({ hasText: "Google スプレッドシート" }).count()) throw new Error("ジャンル外のスキルが表示されています");
  await page.locator("#settingsSearch").fill("PICOAGENT_GOOGLE_SHEETS_ACCESS_TOKEN");
  await page.waitForFunction(() => document.querySelector("#status").textContent === "検索に一致する設定はありません");
  if (await page.locator("#status").textContent() !== "検索に一致する設定はありません") throw new Error("ジャンルで隠れた設定だけの一致が検索結果として残っています");
  await page.locator("#clearSettingsSearch").click();
  await skillGenres.getByRole("button", { name: "すべて", exact: true }).click();
  if (await skillGenres.getByRole("button", { pressed: true }).textContent() !== "すべて") throw new Error("絞り込み解除後の状態が読み上げに反映されません");
  for (const key of ["PICOAGENT_MCP_SERVERS", "PICOAGENT_API_ENABLED"]) {
    if (await page.locator(`#panel-tools [data-setting-key="${key}"]`).count() !== 1) throw new Error(`${key} がスキル・連携タブにありません`);
  }
  for (const name of ["Google スプレッドシート", "Bluesky", "Microsoft Graph"]) {
    const card = page.getByRole("button", { name: `${name}の設定`, exact: true });
    await card.focus();
    await card.press("Enter");
    if (await card.getAttribute("aria-expanded") !== "true") throw new Error(`${name}をEnterで開けません`);
    await card.press("Space");
    if (await card.getAttribute("aria-expanded") !== "false") throw new Error(`${name}をSpaceで閉じられません`);
    const repeatBlocked = await card.evaluate(node => {
      const event = new KeyboardEvent("keydown", { key: " ", repeat: true, bubbles: true, cancelable: true });
      node.dispatchEvent(event);
      return event.defaultPrevented && node.getAttribute("aria-expanded") === "false";
    });
    if (!repeatBlocked) throw new Error(`${name}のSpace長押しでスクロールまたは再開閉が発生します`);
    await card.press("Enter");
  }
  const verifyLongSkillLabels = async width => {
    await page.setViewportSize({ width, height: 820 });
    const result = await page.evaluate(() => {
      const labels = [
        "Google スプレッドシートの OAuth アクセストークン",
        "Bluesky の識別子（ハンドルまたはメール）",
        "Microsoft Graph の OAuth アクセストークン",
      ];
      return labels.map(text => {
        const label = [...document.querySelectorAll(".card-config .field label")].find(node => node.textContent === text);
        const input = label?.parentElement?.querySelector("input");
        const labelBox = label?.getBoundingClientRect();
        const inputBox = input?.getBoundingClientRect();
        return {
          text,
          visible: !!labelBox && labelBox.width > 0 && labelBox.height > 0,
          inside: !!labelBox && labelBox.left >= 0 && labelBox.right <= document.documentElement.clientWidth,
          inputWidth: inputBox?.width || 0,
        };
      });
    });
    for (const item of result) {
      if (!item.visible || !item.inside || item.inputWidth < 220) {
        throw new Error(`${width}px幅でスキル設定が収まりません: ${JSON.stringify(item)}`);
      }
    }
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
    if (overflow) throw new Error(`${width}px幅でスキル設定が横にはみ出しています`);
    await page.screenshot({ path: join(OUT, `skills-${width}px.png`), fullPage: true });
    for (const [key, text] of [
      ["google-sheets", "Google スプレッドシートの OAuth アクセストークン"],
      ["bluesky", "Bluesky の識別子（ハンドルまたはメール）"],
      ["microsoft-graph", "Microsoft Graph の OAuth アクセストークン"],
    ]) {
      const setting = page.locator(".card-config .field").filter({ has: page.locator("label", { hasText: text }) });
      await setting.scrollIntoViewIfNeeded();
      await setting.screenshot({ path: join(OUT, `skill-${key}-${width}px.png`) });
    }
  };
  await verifyLongSkillLabels(600);
  await verifyLongSkillLabels(400);
  for (const label of ["Google スプレッドシートの OAuth アクセストークン", "Bluesky の識別子（ハンドルまたはメール）"]) {
    const control = page.getByLabel(label, { exact: true });
    await page.locator("label").filter({ hasText: label }).click();
    if (!(await control.evaluate(node => node === document.activeElement))) throw new Error(`${label}を押しても入力欄へ移動しません`);
  }
  await page.setViewportSize({ width: 960, height: 760 });
  // 検索で一時的に変わった開閉状態は、検索語を変更しても元の状態へ戻す。
  const blueskyCard = page.locator(".card-item").filter({ hasText: "Bluesky" });
  await blueskyCard.click();
  await page.locator("#settingsSearch").fill("Bluesky");
  await page.waitForFunction(() => [...document.querySelectorAll(".card-item")].some(card => card.textContent.includes("Bluesky") && card.classList.contains("open")));
  if (!(await blueskyCard.getAttribute("class"))?.includes("open")) throw new Error("検索に一致するスキル設定が開きません");
  await page.locator("#settingsSearch").fill("Microsoft Graph");
  await page.waitForFunction(() => [...document.querySelectorAll(".card-item")].some(card => card.textContent.includes("Microsoft Graph") && card.classList.contains("open")));
  await page.locator("#clearSettingsSearch").click();
  await page.locator("#settingsSearch").fill("存在しない設定の検証用文字列");
  await page.waitForFunction(() => document.querySelector("#status").textContent === "検索に一致する設定はありません");
  if (await page.locator("#status").textContent() !== "検索に一致する設定はありません") throw new Error("設定の検索結果なしが表示されません");
  if (await page.locator(".tab-panel:visible").count()) throw new Error("一致する設定のないタブが残っています");
  await page.locator("#clearSettingsSearch").click();
  for (const name of ["Google スプレッドシート", "Bluesky", "Microsoft Graph"]) {
    const card = page.locator(".card-item").filter({ hasText: name });
    const open = await card.evaluate(node => node.classList.contains("open") && !node.nextElementSibling.hidden);
    if (open !== (name !== "Bluesky")) throw new Error(`${name}の検索前の開閉状態が復元されません`);
    if (await card.getAttribute("aria-expanded") !== String(open)) throw new Error(`${name}の開閉状態と読み上げ情報が一致しません`);
  }
  await blueskyCard.click();
  for (const tabName of ["キャラ登録", "キャラ選択", "キャラの音声"]) {
    if (await page.getByRole("tab", { name: tabName, exact: true }).count() !== 1) throw new Error(`${tabName}タブが独立していません`);
  }
  await page.getByRole("tab", { name: "キャラ選択", exact: true }).click();
  const userCard = page.locator(".character-card").filter({ hasText: "ID: user-test" });
  await userCard.waitFor();
  const renameInput = userCard.locator(".character-rename-row input");
  await renameInput.fill("変換中の名前");
  for (const composing of [{ isComposing: true }, { keyCode: 229 }]) {
    const prevented = await renameInput.evaluate((node, composing) => {
      const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, ...composing });
      node.dispatchEvent(event);
      return event.defaultPrevented;
    }, composing);
    if (prevented) throw new Error("日本語変換確定のEnterでキャラ名変更が実行されます");
  }
  await renameInput.fill("Nicole");
  await renameInput.press("Enter");
  await page.waitForFunction(() => [...document.querySelectorAll(".character-card")]
    .some(card => card.textContent.includes("ID: user-test") && card.textContent.includes("Nicoleへ変更しました")));
  if (!(await userCard.textContent())?.includes("Nicole")) throw new Error("追加キャラの表示名がカードへ反映されません");
  if (await renameInput.getAttribute("aria-label") !== "Nicoleの表示名") throw new Error("名前変更後の入力欄が以前の名前で読み上げられます");
  const renamedOption = await page.locator('select option[value="user-test"]').textContent();
  if (!renamedOption?.includes("Nicole") || !renamedOption.includes("user-test")) {
    throw new Error(`追加キャラの表示名または内部IDが選択肢へ反映されません: ${renamedOption}`);
  }
  // 選択中キャラの名前を変えても、旧・既定人格が「利用者の上書き」として固定されないこと。
  // 固定されると名前変更に人格が追従せず、旧名の文が未保存の上書きとして残る。
  // 実機の名前変更は character.json の定型人格も書き換えるので、再取得分だけ差し替える。
  let renamedPersonaName = "";
  let holdCharacterLoad = true;
  let releaseCharacterLoad;
  const characterLoadGate = new Promise(resolveGate => { releaseCharacterLoad = resolveGate; });
  await page.route("**/assets/characters/mugi/character.json*", async route => {
    const response = await route.fetch();
    if (holdCharacterLoad) await characterLoadGate;
    const manifest = JSON.parse(await response.text());
    if (renamedPersonaName) {
      manifest.name = renamedPersonaName;
      manifest.persona = {
        ...manifest.persona,
        systemPrompt: `${renamedPersonaName}として振る舞い、利用者の依頼を丁寧に手伝う。`,
      };
    }
    await route.fulfill({ response, contentType: "application/json", body: JSON.stringify(manifest) });
  });
  await page.evaluate(() => characterManifestCache.delete("user-test"));
  try {
    await userCard.getByRole("button", { name: "このキャラを選ぶ", exact: true }).click();
    if (!(await page.locator("#save").isDisabled())) throw new Error("キャラ設定の読込中に保存できてしまいます");
    if (!(await page.locator("#form").evaluate(node => node.inert))) throw new Error("キャラ設定の読込中に別の項目を編集できてしまいます");
    if (!(await page.locator("#status").textContent())?.includes("キャラ設定を読み込み中")) throw new Error("キャラ切替の待機理由が表示されません");
  } finally {
    holdCharacterLoad = false;
    releaseCharacterLoad();
  }
  await page.waitForFunction(() => !document.getElementById("form").inert);
  if ((await page.locator("#status").textContent())?.includes("キャラ設定を読み込み中")) throw new Error("キャラ切替後も待機表示が残っています");
  // 切替は非同期。選択1件だけが未保存になった状態を基準にする。
  await page.waitForFunction(() => document.getElementById("dirtySummary")?.textContent === "未保存 1件");
  const dirtyAfterSelect = await page.locator("#dirtySummary").textContent();
  await renameInput.fill("Nicole改");
  renamedPersonaName = "Nicole改";
  await userCard.getByRole("button", { name: "名前を変更", exact: true }).click();
  await page.waitForFunction(() => [...document.querySelectorAll(".character-card")]
    .some(card => card.textContent.includes("ID: user-test") && card.textContent.includes("Nicole改へ変更しました")));
  const personaAfterRename = await page.locator('textarea[placeholder="性格・口調・一人称など"]').inputValue();
  if (!personaAfterRename.includes("Nicole改として振る舞い")) {
    throw new Error("名前変更後の人格欄が新しい既定文へ追従していません");
  }
  const dirtyAfterRename = await page.locator("#dirtySummary").textContent();
  if (dirtyAfterRename !== dirtyAfterSelect) {
    throw new Error(`選択中キャラの名前変更で未保存項目が増えました: ${dirtyAfterSelect} -> ${dirtyAfterRename}`);
  }
  await page.locator('[data-setting-key="PICOAGENT_CHARACTER"] select').selectOption("mugi");
  await page.waitForFunction(() => document.getElementById("dirtySummary")?.textContent === "保存済み");
  renamedPersonaName = "";
  await renameInput.fill("Nicole");
  await userCard.getByRole("button", { name: "名前を変更", exact: true }).click();
  await page.waitForFunction(() => [...document.querySelectorAll(".character-card")]
    .some(card => card.textContent.includes("ID: user-test") && card.textContent.includes("Nicoleへ変更しました")));
  await userCard.screenshot({ path: join(OUT, "character-rename.png") });
  await page.setViewportSize({ width: 960, height: 760 });
  await page.getByRole("tab", { name: "AIモデル", exact: true }).click();
  if (await page.locator('[data-setting-key="PICOAGENT_MODEL"]').isVisible()) throw new Error("ローカルAIで不要なAIモデル欄が表示されています");
  const providerSelect = page.locator('#panel-providers [data-setting-key="PICOAGENT_PROVIDER"] select');
  await providerSelect.selectOption("agent_cli");
  const cliModelSelect = page.getByRole("combobox", { name: "CLIのAIモデル" });
  await page.waitForFunction(() => document.querySelector('select[aria-label="CLIのAIモデル"]')?.value === "claude");
  await cliModelSelect.selectOption("codex:future-model");
  await page.getByRole("button", { name: "モデル一覧を更新", exact: true }).click();
  if (await cliModelSelect.inputValue() !== "codex:future-model") throw new Error("一覧更新で選択モデルが変わりました");
  const modelValue = await page.locator('[data-setting-key="PICOAGENT_MODEL"] input').inputValue();
  if (modelValue !== "codex:future-model") throw new Error("選択モデルが保存用入力へ反映されません");
  await page.setViewportSize({ width: 400, height: 820 });
  await cliModelSelect.locator("../..").screenshot({ path: join(OUT, "cli-model-select-narrow.png") });
  if (await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth)) throw new Error("CLIモデル欄が狭幅で横にはみ出しています");
  await page.setViewportSize({ width: 960, height: 760 });
  await providerSelect.selectOption("local");
  await page.waitForFunction(() => document.querySelector('[data-setting-key="PICOAGENT_MODEL"]').hidden);
  await page.locator("main").evaluate(node => { node.scrollTop = 0; });
  await page.screenshot({ path: join(OUT, "providers-tab.png") });
  const localRecommendation = page.locator(".local-model-recommendation");
  if (await page.locator(".local-model-card").count() !== 5) throw new Error("日本語ローカルモデル候補が5件表示されていません");
  const localText = await localRecommendation.textContent();
  for (const expected of ["RAM 32GB", "専用VRAM 8.6GB", "LFM2.5 2.6B Agentic", "Gemma 4 E2B", "Qwen3.5 4B"]) {
    if (!localText?.includes(expected)) throw new Error(`PC向け候補に「${expected}」がありません`);
  }
  const familyLabels = await page.locator('[data-setting-key="PICOAGENT_LOCAL_MODEL_PRESET"] select').first().locator("option").allTextContents();
  for (const family of ["Liquid AI", "Gemma", "Qwen", "OpenAI", "Phi"]) {
    if (!familyLabels.includes(family)) throw new Error(`${family}系列がモデル選択にありません`);
  }
  await page.locator(".local-model-card").filter({ hasText: "Gemma 4 E2B" }).getByRole("button", { name: "このモデルを使う" }).click();
  if (await page.locator('[data-setting-key="PICOAGENT_LOCAL_MODEL_PRESET"] input[type="hidden"]').inputValue() !== "small") throw new Error("カード選択がローカルモデル設定へ反映されません");
  if (await page.locator('[data-setting-key="PICOAGENT_MODEL"] input').inputValue() !== "small") throw new Error("カード選択がAIモデルへ同期されません");
  await page.waitForFunction(() => document.getElementById("dirtySummary")?.textContent?.includes("未保存"));
  if (!(await page.locator("#dirtySummary").textContent())?.includes("未保存")) throw new Error("カード選択で未保存表示になりません");
  await page.locator(".local-model-card").filter({ hasText: "LFM2.5 2.6B Agentic" }).getByRole("button", { name: "このモデルを使う" }).click();
  await page.waitForFunction(() => document.getElementById("dirtySummary")?.textContent === "保存済み");
  await localRecommendation.scrollIntoViewIfNeeded();
  await localRecommendation.screenshot({ path: join(OUT, "local-model-recommendations.png") });
  await page.setViewportSize({ width: 400, height: 820 });
  if (await localRecommendation.evaluate(node => node.scrollWidth > node.clientWidth)) throw new Error("400px幅でローカルモデル候補が横にはみ出しています");
  await localRecommendation.screenshot({ path: join(OUT, "local-model-recommendations-400px.png") });
  await page.setViewportSize({ width: 960, height: 760 });
  const multiAi = page.locator(".multi-ai-editor");
  if (await multiAi.count() !== 1) throw new Error("複数AI設定がありません");
  if (await page.locator(".multi-ai-row").count() !== 2) throw new Error("保存済みの参加AIが2件表示されていません");
  if (await page.locator('[data-setting-key="PICOAGENT_MULTI_AI_MODE"] select').inputValue() !== "consensus") throw new Error("合議モードが復元されていません");
  if (await page.locator('.multi-ai-row input[placeholder="担当の希望（空なら自動）"]').nth(1).inputValue() !== "技術確認") throw new Error("参加AIの担当希望が復元されていません");
  await multiAi.scrollIntoViewIfNeeded();
  await multiAi.screenshot({ path: join(OUT, "multi-ai.png") });
  await page.setViewportSize({ width: 400, height: 820 });
  const multiAiOverflow = await multiAi.evaluate(node => node.scrollWidth > node.clientWidth);
  if (multiAiOverflow) throw new Error("400px幅で複数AI設定が横にはみ出しています");
  await multiAi.screenshot({ path: join(OUT, "multi-ai-400px.png") });
  // カード単体が収まっていても、ページ全体が横へはみ出すと見出しや保存ボタンの左端が切れる。
  const pageOverflow = await page.evaluate(() => {
    const root = document.scrollingElement;
    const width = root.clientWidth;
    const wide = [...document.querySelectorAll("body *")]
      .map(node => ({ node, rect: node.getBoundingClientRect() }))
      .filter(({ node, rect }) => rect.width > 0 && (rect.right > width + 1 || rect.left < -1) && getComputedStyle(node).position !== "fixed")
      .slice(0, 8)
      .map(({ node, rect }) => `${node.tagName.toLowerCase()}.${[...node.classList].join(".")} ${Math.round(rect.left)}..${Math.round(rect.right)}`);
    return { scrollWidth: root.scrollWidth, clientWidth: width, scrollLeft: root.scrollLeft, wide };
  });
  if (pageOverflow.scrollWidth > pageOverflow.clientWidth + 1 || pageOverflow.scrollLeft > 0) {
    throw new Error(`400px幅で設定画面が横にはみ出しています ${JSON.stringify(pageOverflow)}`);
  }
  await page.setViewportSize({ width: 960, height: 760 });
  if (await page.getByRole("button", { name: "一覧を再読み込み", exact: true }).count() !== 1) throw new Error("モデル一覧の再読込操作が更新操作と区別できません");
  await page.locator(".model-delete").click();
  if (!(await page.locator(".model-manager-status").textContent())?.includes("再ダウンロードが必要")) throw new Error("モデル削除前に再ダウンロードが案内されません");
  await page.getByRole("tab", { name: "画面・操作", exact: true }).click();
  await page.locator('#panel-display [data-setting-key="PICOAGENT_NAP_MINUTES"] input').fill("20");
  await page.waitForFunction(() => document.getElementById("dirtySummary")?.textContent?.includes("未保存"));
  const dirtyText = await page.locator("#dirtySummary").textContent();
  if (!dirtyText?.includes("未保存")) throw new Error(`未保存件数が表示されません: ${dirtyText}`);
  if (!(await page.locator("#save").textContent())?.includes("1件")) throw new Error("保存ボタンに変更件数が表示されません");
  await page.locator("#resetChanges").click();
  await page.waitForFunction(() => document.getElementById("dirtySummary")?.textContent === "保存済み");
  await page.locator('[data-settings-query="読み上げ"]').click();
  if (await page.locator("#clearSettingsSearch").isHidden()) throw new Error("検索解除ボタンが表示されません");
  if (await page.locator('.searchable:not([hidden])').count() < 1) throw new Error("音声の検索候補が表示されません");
  await page.locator("#clearSettingsSearch").click();
  if (await page.locator("#settingsSearch").inputValue() !== "") throw new Error("検索を解除できません");
  await page.getByRole("tab", { name: "キャラ選択", exact: true }).click();
  await page.waitForSelector(".framing-editor");
  await page.waitForSelector(".stream-profile-editor");
  const streamEnabled = await page.locator('.stream-profile-editor input[type="checkbox"]').isChecked();
  if (!streamEnabled) throw new Error("むぎのキャラ別配信設定が読み込まれていません");
  if (await page.locator('.stream-profile-editor input[type="color"]').inputValue() !== "#a04070") throw new Error("キャラ別字幕色が読み込まれていません");
  const framing = page.locator(".framing-editor");
  if (!(await framing.textContent())?.includes("メイン画面のキャラにマウスを乗せ")) throw new Error("構図の調整場所が案内されません");
  if (await framing.locator('input[type="range"], .framing-preview').count()) throw new Error("使われない旧構図エディタが残っています");
  await page.screenshot({ path: join(OUT, "character-framing.png") });
  const addedCharacter = page.locator(".character-card").filter({ hasText: "ID: user-test" });
  await addedCharacter.getByRole("button", { name: "このキャラを選ぶ", exact: true }).click();
  await addedCharacter.getByRole("button", { name: "削除", exact: true }).click();
  await addedCharacter.getByRole("button", { name: "もう一度押して削除", exact: true }).click();
  if (await addedCharacter.count()) throw new Error("削除した追加キャラが一覧に残っています");
  if (await page.locator('[data-setting-key="PICOAGENT_CHARACTER"] select').inputValue() !== "mugi") throw new Error("未保存選択の削除後に保存中キャラへ戻りません");
  if (!(await page.locator('textarea[placeholder="性格・口調・一人称など"]').inputValue()).includes("年下っぽく")) throw new Error("削除後の選択と人格表示が一致しません");
  const settled = await page.waitForFunction(() => document.getElementById("dirtySummary")?.textContent === "保存済み", null, { timeout: 5000 }).then(() => true, () => false);
  if (!settled) {
    const detail = await page.evaluate(() => ({ summary: document.getElementById("dirtySummary")?.textContent, title: document.getElementById("dirtySummary")?.title }));
    throw new Error("既定人格だけで未保存扱いになっています " + JSON.stringify(detail));
  }
  await page.getByRole("tab", { name: "キャラの音声", exact: true }).click();
  const voicePanelHeadingBox = await page.locator("#panel-voice .settings-panel-head h2").boundingBox();
  if (!voicePanelHeadingBox || voicePanelHeadingBox.y < 0 || voicePanelHeadingBox.y > 260) throw new Error("音声タブの見出しが先頭から表示されません");
  const voiceHeadingBox = await page.getByRole("heading", { name: "キャラの音声設定", exact: true }).boundingBox();
  if (!voiceHeadingBox || voiceHeadingBox.y < 0 || voiceHeadingBox.y > 420) throw new Error("音声設定の本文が先頭から表示されません");
  const muteStatus = await page.locator(".tts-master-status").textContent();
  if (!muteStatus?.includes("ミュート中")) throw new Error(`読み上げのミュート状態が分かりません: ${muteStatus}`);
  if (!(await page.locator(".character-voice-context").textContent())?.includes("むぎ")) throw new Error("音声タブに選択中キャラが表示されません");
  const speakerInput = page.getByLabel("このキャラの Aivis Style ID", { exact: true });
  await speakerInput.fill("123456");
  await page.waitForFunction(() => document.getElementById("dirtySummary")?.textContent?.includes("未保存"));
  await speakerInput.fill("");
  await page.waitForFunction(() => document.getElementById("dirtySummary")?.textContent === "保存済み");
  await page.screenshot({ path: join(OUT, "character-voice-tab.png") });
  await page.getByRole("tab", { name: "キャラ登録", exact: true }).click();
  const registerPanelHeadingBox = await page.locator("#panel-character-register .settings-panel-head h2").boundingBox();
  if (!registerPanelHeadingBox || registerPanelHeadingBox.y < 0 || registerPanelHeadingBox.y > 260) throw new Error("登録タブの見出しが先頭から表示されません");
  const registerHeadingBox = await page.getByRole("heading", { name: "キャラクターを追加", exact: true }).boundingBox();
  if (!registerHeadingBox || registerHeadingBox.y < 0 || registerHeadingBox.y > 420) throw new Error("キャラ登録の本文が先頭から表示されません");
  if (await page.getByRole("heading", { name: "キャラクターを追加", exact: true }).count() !== 1) throw new Error("キャラ登録タブに追加操作がありません");
  if (await page.locator(".character-browser-list:visible").count()) throw new Error("キャラ登録タブに選択一覧が混ざっています");
  if (await page.locator(".tts-master-status:visible").count()) throw new Error("キャラ登録タブに音声設定が混ざっています");
  await page.getByRole("button", { name: "ダウンロードからモデルを探す" }).click();
  if (await page.locator(".download-discovery-results .card-item").count() !== 2) throw new Error("ダウンロード済みモデル候補が表示されません");
  await page.locator(".download-discovery-results .card-item").first().getByRole("button", { name: "このモデルを選ぶ" }).click();
  if (!await page.locator(".download-discovery-form").isVisible()) throw new Error("モデル選択後の確認欄が出ません");
  if (await page.locator(".download-discovery-form input[type=text]").first().inputValue() !== "ひより") throw new Error("候補の表示名が入力されません");
  await page.getByRole("button", { name: "選んだモデルを追加" }).click();
  if (!await page.locator(".download-discovery-status").textContent().then(text => text.includes("利用条件"))) throw new Error("利用条件の未確認時に追加を止めません");
  await page.screenshot({ path: join(OUT, "character-register-tab.png") });
  await page.getByRole("tab", { name: "診断・修復" }).click();
  // 不具合連絡用の書き出し。保存先をその場に出す（秘密値を含めないことも明示する）。
  await page.getByRole("button", { name: "診断情報を書き出す", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".diagnostics-export-status")?.textContent?.includes("diagnostics.2026-09-02.080000.txt"));
  const exportNote = await page.locator(".diagnostics-export-status").textContent();
  if (!exportNote?.includes("APIキー等は含みません")) throw new Error(`診断書き出しの案内が不足しています: ${exportNote}`);
  if (await page.locator(".product-info-card").count() !== 5) throw new Error("製品情報カードが5件ではありません");
  if (!(await page.locator(".product-info-card").first().textContent())?.includes("1.0.0")) throw new Error("製品バージョンが表示されません");
  if (await page.locator("#panel-diagnostics .product-update-card").count()) throw new Error("アプリ更新が診断・修復タブに残っています");
  await page.screenshot({ path: join(OUT, "diagnostics-tab.png") });
  await page.setViewportSize({ width: 400, height: 820 });
  const [resetBox, saveBox] = await Promise.all([
    page.locator("#resetChanges").boundingBox(),
    page.locator("#save").boundingBox(),
  ]);
  if (!resetBox || !saveBox || resetBox.width < 140 || saveBox.width < 140) throw new Error("狭幅で保存操作の可読幅が足りません");
  if (Math.abs(resetBox.y - saveBox.y) > 2) throw new Error("狭幅で保存と取り消しが不揃いです");
  const horizontalOverflow = await page.evaluate(() => ({
    width: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
    layout: Object.fromEntries(["body", "main", ".tabs", ".tab-panels"].map(selector => {
      const node = document.querySelector(selector);
      const box = node?.getBoundingClientRect();
      return [selector, node && box ? { x: box.x, width: box.width, scrollWidth: node.scrollWidth, overflowX: getComputedStyle(node).overflowX } : null];
    })),
    nodes: [...document.querySelectorAll(".tab-panel:not([hidden]) *")].filter(node => {
      const box = node.getBoundingClientRect();
      return box.width > 0 && box.right > document.documentElement.clientWidth - 16 + 1;
    }).slice(0, 12).map(node => `${node.tagName.toLowerCase()}#${node.id}.${String(node.className).split(" ").join(".")}`),
  }));
  if (horizontalOverflow.scrollWidth > horizontalOverflow.width) throw new Error(`狭幅でページ全体が横にはみ出しています: ${JSON.stringify(horizontalOverflow)}`);
  await page.evaluate(() => window.scrollTo(0, 600));
  await page.getByRole("tab", { name: "基本設定", exact: true }).click();
  const tabScroll = await page.evaluate(() => window.scrollY);
  if (tabScroll > 10) throw new Error(`狭幅のタブ切替で先頭へ戻りません: ${tabScroll}`);
  await page.screenshot({ path: join(OUT, "narrow-footer.png") });
  // キャラ一覧のアイコンを全件撮る。メッシュ化したキャラが同梱の仮画像に落ちていないかを目視する。
  await page.setViewportSize({ width: 960, height: 2600 });
  await page.getByRole("tab", { name: "キャラ選択", exact: true }).click();
  await page.waitForFunction(() => [...document.querySelectorAll(".character-card img.character-icon")].every(img => img.hidden || img.complete));
  const placeholderIcons = await page.locator(".character-card img.character-icon").evaluateAll(images => images
    .filter(img => !img.hidden && img.src.includes("_preview-placeholder"))
    .map(img => img.alt));
  const cardList = page.locator(".character-card").first().locator("xpath=..");
  await cardList.screenshot({ path: join(OUT, "character-icons.png") });
  if (placeholderIcons.length) throw new Error(`キャラ一覧のアイコンが仮画像です: ${placeholderIcons.join(", ")}`);
  await page.setViewportSize({ width: 960, height: 820 });
  // 全タブを通常幅と400px幅で撮り、横へのはみ出しがないことを確かめる。個別検査のないタブ
  // （安全・許可、使い方、キャラ作成など）も目視できるよう画像を残す。
  const tabsDir = join(OUT, "tabs");
  mkdirSync(tabsDir, { recursive: true });
  const tabNames = await page.locator(".tabs .tab").evaluateAll(nodes => nodes.map(node => node.textContent.trim()));
  const tabOverflow = [];
  for (const [label, width] of [["wide", 960], ["narrow", 400]]) {
    await page.setViewportSize({ width, height: 820 });
    for (const [index, name] of tabNames.entries()) {
      await page.locator(".tabs .tab").nth(index).click();
      await page.waitForTimeout(150);
      const wide = await page.evaluate(() => {
        const width = document.documentElement.clientWidth;
        const nodes = [...document.querySelectorAll(".tab-panel:not([hidden]) *")]
          .filter(node => {
            const rect = node.getBoundingClientRect();
            if (!(rect.width > 0 && rect.right > width + 1)) return false;
            // 構図プレビューのように、はみ出しを切り取る枠の中なら見た目には出ない。
            for (let parent = node.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
              if (["hidden", "clip", "auto", "scroll"].includes(getComputedStyle(parent).overflowX)) return parent.getBoundingClientRect().right > width + 1;
            }
            return true;
          })
          .slice(0, 3)
          .map(node => `${node.tagName.toLowerCase()}.${[...node.classList].join(".")}`);
        return { page: document.documentElement.scrollWidth > width, nodes };
      });
      if (wide.page || wide.nodes.length) tabOverflow.push(`${label}/${name}: ${wide.nodes.join(", ")}`);
      await page.evaluate(() => window.scrollTo(0, 260));
      await page.screenshot({ path: join(tabsDir, `${String(index).padStart(2, "0")}-${label}.png`) });
      await page.evaluate(() => window.scrollTo(0, 0));
    }
  }
  if (tabOverflow.length) throw new Error(`設定タブが横にはみ出しています: ${tabOverflow.join(" / ")}`);
  // UIモードは未設定時の可愛いを維持し、プレビュー・保存・再読込を同じ経路で検査する。
  const modePage = await browser.newPage({ viewport: { width: 960, height: 760 } });
  await modePage.addInitScript(mock);
  await modePage.goto(`http://127.0.0.1:${port}/settings.html?tab=display&slow-models`);
  const modeSelect = modePage.locator('[data-setting-key="PICOAGENT_UI_THEME"] select');
  await modeSelect.waitFor();
  await modePage.waitForFunction(() => window.__modelListPending === true);
  const loadingState = await modeSelect.evaluate(node => {
    node.focus();
    return {
      inert: document.getElementById("form").inert,
      saveDisabled: document.getElementById("save").disabled,
      inputFocused: document.activeElement === node,
    };
  });
  if (!loadingState.inert || !loadingState.saveDisabled || loadingState.inputFocused) {
    throw new Error(`初期値の読込中に設定を編集できます: ${JSON.stringify(loadingState)}`);
  }
  // 入力欄はキャラやモデルの読込より先に描画される。初期値の確定前に選択すると
  // 未保存変更の検査にならないため、保存操作が有効になるまで待つ。
  await modePage.waitForFunction(() => !document.getElementById("save").disabled);
  if (await modeSelect.inputValue() !== "pastel") throw new Error("未設定のUIモードが可愛いではありません");
  for (const [mode, radius] of [["glass", "12px"], ["pastel", "10px"]]) {
    await modeSelect.selectOption(mode);
    const actual = await modePage.locator(".settings-panel-head").first().evaluate(node => ({
      mode: document.documentElement.dataset.uiTheme, radius: getComputedStyle(node).borderTopLeftRadius,
    }));
    if (actual.mode !== mode || actual.radius !== radius) throw new Error(`UIモードのプレビューが不一致: ${JSON.stringify(actual)}`);
    await modePage.locator("#save").click();
    await modePage.waitForFunction(() => !document.getElementById("save").disabled);
    await modePage.reload();
    await modeSelect.waitFor();
    await modePage.waitForFunction(() => !document.getElementById("save").disabled);
    if (await modeSelect.inputValue() !== mode) throw new Error(`UIモードの保存・再読込が不一致: ${mode}`);
    // 明暗はプレビューだけ切り替えて撮る（保存しない）。アクセント面の文字はモードの対比色を使う。
    const themeSelect = modePage.locator('[data-setting-key="PICOAGENT_THEME"] select');
    for (const theme of ["light", "dark"]) {
      await themeSelect.selectOption(theme);
      const colors = await modePage.evaluate(() => {
        const probe = document.createElement("button");
        probe.className = "genre-chip active";
        document.body.append(probe);
        const color = getComputedStyle(probe).color;
        probe.remove();
        const onAccent = document.createElement("span");
        onAccent.style.color = "var(--on-accent)";
        document.body.append(onAccent);
        const expected = getComputedStyle(onAccent).color;
        onAccent.remove();
        return { color, expected };
      });
      if (colors.color !== colors.expected) throw new Error(`アクセント面の文字色がUIモードの対比色ではありません: ${mode}/${theme} ${JSON.stringify(colors)}`);
      await modePage.screenshot({ path: join(OUT, `mode-${mode}-${theme}.png`) });
    }
  }
  await modePage.close();
  // 実機で返る長い警告でも、保存操作が縦につぶれたり窓から出たりしない。
  for (const width of [960, 720, 400]) {
    await page.setViewportSize({ width, height: 760 });
    await page.evaluate(() => {
      document.getElementById("status").textContent = "保存しました。ホットキーを登録できませんでした: CommandOrControl+Alt+H（他のアプリが同じキーを使っている可能性があります）: HotKey already registered: ".repeat(4);
    });
    const buttons = await page.locator("footer button").evaluateAll(nodes => nodes.map(node => {
      const rect = node.getBoundingClientRect();
      return { width: rect.width, height: rect.height, inside: rect.left >= 0 && rect.right <= innerWidth, textFits: node.scrollWidth <= node.clientWidth };
    }));
    if (buttons.some(button => !button.inside || !button.textFits || button.width < 80)) throw new Error(`長い警告で保存操作がつぶれます: ${width} ${JSON.stringify(buttons)}`);
    if (width === 960 && buttons.some(button => button.height > 60)) throw new Error("通常幅で保存ボタンの文字が縦に並びます");
  }
  // 一覧が読めなくても保存済みIDと無関係な設定の草稿を守り、明示再取得だけで復旧する。
  const userListPage = await browser.newPage({ viewport: { width: 960, height: 760 } });
  await userListPage.addInitScript(mock);
  await userListPage.goto(`http://127.0.0.1:${port}/settings.html?tab=display&user-list-failure`);
  const userListReason = userListPage.locator(".character-list-diagnostic-container");
  await userListReason.getByText("追加キャラクター一覧の検証用エラー", { exact: true }).waitFor();
  await userListPage.waitForFunction(() => !document.getElementById("save").disabled);
  const userSelect = userListPage.locator('[data-setting-key="PICOAGENT_CHARACTER"] select');
  const rendererOf = target => target.locator(".field").filter({ has: target.locator("label", { hasText: "このキャラの描画方式" }) }).locator("select");
  if (await userSelect.inputValue() !== "user-test" || !(await userSelect.isDisabled())) throw new Error("一覧失敗で保存IDか選択禁止が失われました");
  const relatedFields = [rendererOf(userListPage), userListPage.locator('textarea[placeholder="性格・口調・一人称など"]'), userListPage.locator(".field").filter({ has: userListPage.locator("label", { hasText: "このキャラの Aivis Style ID" }) }).locator("input")];
  for (const control of relatedFields) if (!(await control.isDisabled())) throw new Error("一覧失敗中にキャラ関連設定を編集できます");
  if (!(await userListReason.isVisible())) throw new Error("一覧失敗の理由が画面に表示されません");
  const userFont = userListPage.locator("#setting-PICOAGENT_STREAM_CHAT_FONT_PX");
  await userFont.fill("31");
  await userListPage.locator("#save").click();
  await userListPage.waitForFunction(() => document.getElementById("status")?.textContent?.includes("保存しました"));
  const userPatch = await userListPage.evaluate(() => window.__savedSettingsPatches.at(-1));
  if (userPatch.PICOAGENT_STREAM_CHAT_FONT_PX !== "31" || Object.hasOwn(userPatch, "PICOAGENT_CHARACTER")) throw new Error(`他設定の保存がキャラIDを変更しました: ${JSON.stringify(userPatch)}`);
  if (!(await userListReason.textContent()).includes("追加キャラクター一覧の検証用エラー") || !(await userListReason.isVisible())) throw new Error("他設定の保存で一覧失敗の理由が消えました");
  await userFont.fill("32");
  await userListPage.evaluate(() => { window.__userCharactersRecovered = true; });
  await userListPage.getByRole("button", { name: "キャラクター一覧を再取得", exact: true }).click();
  await userListPage.waitForFunction(() => !document.getElementById("save").disabled && document.querySelector('[data-setting-key="PICOAGENT_CHARACTER"] select')?.disabled === false);
  if (await userSelect.inputValue() !== "user-test" || !(await userSelect.locator('option[value="user-test"]').textContent()).includes("追加テスト")) throw new Error("再取得後に追加キャラ一覧と元IDが復旧しません");
  if (await userListReason.isVisible() || await userFont.inputValue() !== "32") throw new Error("一覧復旧で失敗理由か配信文字サイズの草稿が不正です");
  if (!(await userListPage.locator("#dirtySummary").textContent()).includes("未保存 1件")) throw new Error("再取得後に文字サイズの未保存状態が失われました");
  await userListPage.close();

  // 明示2Dは未保存リセットで戻り、保存後の通常復帰はそのIDの指定を削除する。
  const rendererPage = await browser.newPage({ viewport: { width: 960, height: 760 } });
  await rendererPage.addInitScript(mock);
  await rendererPage.goto(`http://127.0.0.1:${port}/settings.html?tab=character`);
  await rendererPage.waitForFunction(() => !document.getElementById("save").disabled);
  const rendererSelect = rendererOf(rendererPage);
  await rendererSelect.selectOption("image");
  await rendererPage.waitForFunction(() => document.getElementById("dirtySummary")?.textContent?.includes("未保存 1件"));
  await rendererPage.locator("#resetChanges").click();
  await rendererPage.waitForFunction(() => !document.getElementById("save").disabled && document.getElementById("resetChanges").disabled);
  if (await rendererSelect.inputValue() !== "auto") throw new Error("未保存2D指定のリセットで通常描画に戻りません");
  for (const renderer of ["image", "auto"]) {
    await rendererSelect.selectOption(renderer);
    await rendererPage.locator("#save").click();
    await rendererPage.waitForFunction(() => document.getElementById("status")?.textContent?.includes("保存しました") && !document.getElementById("save").disabled);
    const overrides = await rendererPage.evaluate(() => window.__settingsData.values.PICOAGENT_RENDERER_OVERRIDES);
    if (renderer === "image" ? overrides.mugi !== "image" : Object.hasOwn(overrides, "mugi")) throw new Error(`描画方式の保存・解除が不一致: ${JSON.stringify(overrides)}`);
  }
  await rendererPage.close();
  const recoveryPage = await browser.newPage();
  await recoveryPage.addInitScript(mock);
  await recoveryPage.goto(`http://127.0.0.1:${port}/settings.html?load-failure=1`);
  await recoveryPage.getByRole("heading", { name: "設定を読み込めませんでした" }).waitFor();
  if (!(await recoveryPage.locator("#save").isDisabled())) throw new Error("設定読込失敗中に保存できてしまいます");
  if (await recoveryPage.locator("#loading").count()) throw new Error("設定読込失敗後も読み込み中表示が残っています");
  await recoveryPage.locator('[data-settings-query="画面"]').click();
  if (!(await recoveryPage.locator("#status").textContent())?.includes("設定読込の検証用エラー")) throw new Error("読込失敗の原因が表示されません");
  await recoveryPage.getByRole("button", { name: "再読み込み", exact: true }).click();
  await recoveryPage.getByRole("tab", { name: "基本設定", exact: true }).waitFor();
  await recoveryPage.waitForFunction(() => !document.getElementById("save").disabled);
  await recoveryPage.close();
  console.log(`設定画面確認: 入口8件 / キャラ作成・登録・選択・音声の4分割 / 追加機能6件 / 表示形式3件 / 複数AI2件・合議・担当希望・400px表示 / モニター指定欄なし / 未保存件数（選択中キャラの改名含む）・検索解除・製品情報・構図の調整案内・配信・ミュート・長いスキルラベル600px/400px・狭幅フッター表示OK / 選択=${selected} / ${join(OUT, "display-tab.png")}`);
} finally {
  await browser.close();
  await new Promise(resolveClose => server.close(resolveClose));
}
