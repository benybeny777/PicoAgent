// 現行シーン方式のキャラとゲーム画面を、登録済みモデルを読み取り専用で撮影する。
import http from "node:http";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { execFileSync } from "node:child_process";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const sourceRoot = process.env.PICO_CAPTURE_SOURCE || root;
const uiRoot = path.join(sourceRoot, "ui");
const userCharactersRoot = path.join(process.env.APPDATA || "", "PicoAgent", "characters");
const mmdModels = new Map([["miko-mmd", path.join(userCharactersRoot, "miko-mmd")]]);
// ひよりはLive2D公式サンプル。cargo xtask setup live2d-samples で取得したモデルを読み取り専用で配信する。
const live2dModels = new Map([["hiyori", path.join(sourceRoot, "characters", "hiyori")]]);
const models = new Map([...mmdModels, ...live2dModels]);
const catalogEntries = [
  { id: "miko-mmd", name: "さくらみこ", manifest: "miko-mmd/character.json" },
  { id: "hiyori", name: "ひより", manifest: "hiyori/character.json" },
];
// --only=<キャラID> で撮影対象を絞る。ひよりだけ撮り直す場合は --only=hiyori --video。
const only = process.argv.find(arg => arg.startsWith("--only="))?.slice("--only=".length);
const output = path.join(root, "docs", "images");
const voicedVideo = process.argv.includes("--voice-video");
const voiceSample = path.join(root, "temp", "holo-indie-review-installer", "aivis-holo-demo.wav");
if (voicedVideo && !fs.existsSync(voiceSample)) throw new Error(`Aivis音声サンプルがありません: ${voiceSample}`);
const playwrightModule = process.env.PLAYWRIGHT_MODULE;
const { chromium } = await import(playwrightModule ? pathToFileURL(playwrightModule).href : "playwright");
const sharp = createRequire(import.meta.url)("sharp");
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".png": "image/png", ".webp": "image/webp", ".jpg": "image/jpeg", ".svg": "image/svg+xml", ".wasm": "application/wasm" };

for (const [id, modelRoot] of models) {
  if (only && only !== id) continue;
  if (!fs.existsSync(path.join(modelRoot, "character.json"))) throw new Error(`登録済みの${id}モデルが見つかりません`);
}
fs.mkdirSync(output, { recursive: true });

const server = http.createServer((request, response) => {
  try {
    const name = decodeURIComponent(new URL(request.url, "http://localhost").pathname).replace(/^\//, "");
    const modelId = [...models.keys()].find(id => name.startsWith(`assets/characters/${id}/`));
    const base = modelId ? models.get(modelId) : uiRoot;
    const relative = modelId ? name.slice(`assets/characters/${modelId}/`.length) : name || "index.html";
    const file = path.resolve(base, relative);
    if (!file.startsWith(path.resolve(base) + path.sep) || !fs.statSync(file).isFile()) throw new Error("対象外のファイルです");
    let body = fs.readFileSync(file);
    if (name === "assets/characters/index.json") {
      const catalog = JSON.parse(body.toString("utf8"));
      for (const entry of catalogEntries) {
        if (!catalog.characters.some(character => character.id === entry.id)) catalog.characters.push(entry);
      }
      body = Buffer.from(JSON.stringify(catalog));
    }
    response.writeHead(200, { "Content-Type": types[path.extname(file)] || "application/octet-stream", "Cache-Control": "no-store" });
    response.end(body);
  } catch {
    response.writeHead(404);
    response.end("not found");
  }
});

const replies = {
  "miko-mmd": "にゃっはろ～！予定の整理も調べものも、いっしょに進めようにぇ。",
  hiyori: "まかせて！予定の整理も調べものも、いっしょにやってみよう！",
};
const mock = (layout, characterId) => {
  const history = [
    { role: "user", content: "今日やることを一緒に整理して" },
    { role: "assistant", content: replies[characterId] },
  ];
  return `
window.__TAURI__ = {
  core: { invoke: (name, args = {}) => {
    if (name === "pet_init") return Promise.resolve({
      up: true, providerId: "local", theme: "system", uiTheme: "glass", fontPx: "14", streamChatFontPx: "30", voiceVolume: "35",
      characterId: ${JSON.stringify(characterId)}, characterOverrides: {}, live2dSdkOverrides: {}, layout: ${JSON.stringify(layout)}, scenes: {},
      history: ${JSON.stringify(history)}, quickActions: "", napMinutes: "0", onboardingDone: true,
      relationshipMemoryEnabled: true, relationship: {}, bondLabel: "いっしょの相棒", gazeTrackingEnabled: false,
      bootProgressIntervalMs: 10000, ttsEnabled: ${voicedVideo}, streamBackground: "pastel", streamDisplay: "dashboard",
      streamCaptionColor: "#5a3b4b", chatterEnabled: false, chatterMinSecs: "9999", chatterMaxSecs: "9999", betaBuild: false,
    });
    if (name === "engine_status") return Promise.resolve({ phase: "ready" });
    if (name === "aivis_status") return Promise.resolve({ enabled: ${voicedVideo} });
    if (name === "pet_speak") return ${voicedVideo}
      ? window.__captureSpeech().then(audio => ({ enabled: true, mime: "audio/wav", audio, lipSync: [] }))
      : Promise.resolve({ enabled: false });
    if (name === "pet_alarms") return Promise.resolve({ alarms: [] });
    if (name === "pet_game_ready") return Promise.resolve({ ready: true });
    if (name === "pet_game_judge") return Promise.resolve({ total: 86, criteria: [
      { key: "ai_style", label: "AIらしい話し方", score: 22, maxScore: 25, reason: "丁寧な説明で落ち着いている" },
      { key: "excuse", label: "言い訳の巧さ", score: 21, maxScore: 25, reason: "原因から代案へ自然につないだ" },
      { key: "plausibility", label: "もっともらしさ", score: 16, maxScore: 20, reason: "確認を重ねた経緯に筋が通っている" },
      { key: "extra_help", label: "頼まれていない親切", score: 17, maxScore: 20, reason: "重要部分の先行提出を提案した" },
      { key: "relevance", label: "お題への対応", score: 5, maxScore: 10, reason: "遅延した状況を拾っている" },
      { key: "fun_bonus", label: "おもしろボーナス", score: 5, maxScore: 10, reason: "少し大げさな丁寧さが面白い" },
    ], commentary: "AIらしい説明でした。人間らしさはほとんど見えません。", highlight: "確認を慎重に進めた結果", improvement: "次は代案をもう一つ添えましょう。" });
    if (name === "pet_daily_events") return Promise.resolve({ events: [] });
    if (name === "pet_user_characters") return Promise.resolve({ characters: [] });
    if (name === "pet_config_presence") return Promise.resolve({});
    if (name === "pet_schedule_runs" || name === "pet_durable_jobs") return Promise.resolve([]);
    if (name === "settings_components_status") return Promise.resolve([]);
    return Promise.resolve({});
  } },
  event: { listen: () => Promise.resolve(() => {}) },
  window: { getCurrentWindow: () => ({ setFullscreen: async () => {}, setSize: async () => {}, scaleFactor: async () => 1,
    outerPosition: async () => ({ x: 0, y: 0 }), setPosition: async () => {}, startResizeDragging: async () => {},
    minimize: async () => {}, toggleMaximize: async () => {}, close: async () => {} }) },
  dpi: {},
};`;
};

let browser;
try {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  browser = await chromium.launch({ args: ["--enable-unsafe-swiftshader", "--use-gl=swiftshader", "--ignore-gpu-blocklist"] });
  const base = `http://127.0.0.1:${server.address().port}/`;
  const scenes = [
    { characterId: "miko-mmd", layout: "stream", width: 1280, height: 720, image: "holo-miko-stream.png", video: "holo-miko-motion.mp4", poster: "holo-miko-motion-poster.png", seconds: 7, fps: 30 },
    { characterId: "miko-mmd", layout: "pet", width: 900, height: 600 },
    { characterId: "hiyori", layout: "stream", width: 1280, height: 720, video: "holo-hiyori-motion.mp4", poster: "holo-hiyori-motion-poster.png", seconds: 7, fps: 30 },
  ].filter(scene => !only || scene.characterId === only);
  for (const { characterId, layout, width, height, image, video, poster, seconds, fps } of
    (voicedVideo ? scenes.filter(scene => scene.characterId === "miko-mmd").slice(0, 1) : scenes)) {
    const recording = voicedVideo && video ? fs.mkdtempSync(path.join(root, "temp", "holo-voice-video-")) : null;
    const context = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1,
      ...(recording ? { recordVideo: { dir: recording, size: { width, height } } } : {}) });
    try {
      const page = await context.newPage();
      const recordingStart = Date.now();
      if (voicedVideo) await page.exposeFunction("__captureSpeech", () => [...fs.readFileSync(voiceSample)]);
      const errors = [];
      page.on("pageerror", error => errors.push(String(error)));
      await page.addInitScript(mock(layout, characterId));
      await page.goto(base);
      await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 30000 });
      await page.waitForFunction(kind => {
        const view = document.querySelector("[data-source=character] .character-view");
        const canvas = view?.querySelector("canvas");
        return view?.dataset.renderKind === kind && canvas?.width > 1 && canvas?.height > 1;
      }, characterId.endsWith("-mmd") ? "mmd" : "live2d", { timeout: 30000 });
      await page.waitForTimeout(1200);
      if (errors.length) throw new Error(errors.join("\n"));
      if (image) await page.screenshot({ path: path.join(output, image) });
      if (characterId === "miko-mmd" && layout === "pet") {
        await page.setViewportSize({ width: 1280, height: 900 });
        await page.getByRole("button", { name: "遊ぶ" }).click();
        await page.locator(".game-start").click();
        await page.locator("#gameAnswer").fill("申し訳ありません。確認を慎重に進めた結果、着手が遅れました。重要部分を先に提出します。");
        await page.locator("#gameSubmit").click();
        await page.locator(".game-result").waitFor();
        await page.locator(".game-overlay").evaluate(element => { element.scrollTop = 0; });
        const png = await page.locator(".game-shell").screenshot();
        await sharp(png).webp({ quality: 90 }).toFile(path.join(output, "holo-indie-result.webp"));
      }
      if (recording) {
        await page.waitForTimeout(3500);
        await page.getByRole("button", { name: "もう一度読み上げ" }).click();
        await page.waitForFunction(() => document.querySelector("#status")?.textContent?.includes("読み上げ中"), null, { timeout: 15000 });
        const startOffset = Math.max(0, (Date.now() - recordingStart) / 1000 - 0.2);
        await page.waitForTimeout(seconds * 1000);
        await context.close();
        const recorded = await page.video().path();
        execFileSync(process.env.SHOOT_FFMPEG || "ffmpeg", ["-y", "-loglevel", "error",
          "-ss", String(startOffset), "-i", recorded, "-i", voiceSample,
          "-filter_complex", "[1:a]adelay=200|200,apad[a]", "-map", "0:v:0", "-map", "[a]",
          "-t", String(seconds), "-c:v", "libx264", "-preset", "medium", "-crf", "18",
          "-pix_fmt", "yuv420p", "-c:a", "aac", "-movflags", "+faststart", path.join(output, video)], { stdio: "pipe" });
      } else if (video && process.argv.includes("--video")) {
        const tempRoot = path.join(root, "temp");
        fs.mkdirSync(tempRoot, { recursive: true });
        const frames = fs.mkdtempSync(path.join(tempRoot, "holo-current-"));
        try {
          const start = await page.evaluate(() => Date.now());
          await page.clock.pauseAt(new Date(start + 10000));
          for (let index = 0; index < seconds * fps; index += 1) {
            const next = Math.round((index + 1) * 1000 / fps) - Math.round(index * 1000 / fps);
            await page.clock.runFor(next);
            const frame = await page.screenshot();
            fs.writeFileSync(path.join(frames, `f-${String(index).padStart(4, "0")}.png`), frame);
            if (index === 0) fs.writeFileSync(path.join(output, poster), frame);
          }
          execFileSync(process.env.SHOOT_FFMPEG || "ffmpeg", ["-y", "-loglevel", "error", "-framerate", String(fps), "-i", path.join(frames, "f-%04d.png"), "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p", "-an", "-movflags", "+faststart", path.join(output, video)], { stdio: "pipe" });
        } finally {
          fs.rmSync(frames, { recursive: true, force: true });
        }
      }
      console.log(`現行画面を撮影: ${characterId}/${layout}${video && process.argv.includes("--video") ? "（動画あり）" : ""}`);
    } finally {
      await context.close();
      if (recording) fs.rmSync(recording, { recursive: true, force: true });
    }
  }
} finally {
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
}
