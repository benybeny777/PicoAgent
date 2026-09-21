// README 用スクリーンショット（動く絵）の撮影ツール。
//
// 実物の ui/ をローカル HTTP で配信し、バックエンド（window.__TAURI__）だけを
// モックしてヘッドレス Chromium で描画する。フロントの JS・CSS・キャラクター画像は
// 本物をそのまま使うため、出てくる絵はモックアップではなく実際の描画結果。
//
// 出力は透過つきアニメーション WebP。ページ内の時計を止めて 1 コマずつ進めて撮るので、
// キャプチャ 1 枚に 200ms かかっても狙ったフレームレートで滑らかになる。書き出しに ffmpeg が要る。
//
// 使い方:
//   PLAYWRIGHT_MODULE=<playwright の index.mjs> node tools/screenshots/shoot.mjs [名前...]
// 詳細は同じフォルダの README.md を参照。
import http from "node:http";
import { readFile, writeFile, mkdtemp, rm } from "node:fs/promises";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { extname, isAbsolute, join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const run = promisify(execFile);

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const UI_ROOT = join(ROOT, "ui");
const OUT = join(ROOT, "docs", "images");
const SITE_OUT = join(ROOT, "distribution", "site", "assets");
const TEMP_OUT = join(ROOT, "temp", "screenshots");
const CUSTOM_OUT = process.env.SHOOT_OUTPUT_DIR || "";
if (CUSTOM_OUT && !isAbsolute(CUSTOM_OUT)) {
  throw new Error("SHOOT_OUTPUT_DIR は絶対パスで指定してください。");
}
const SITE_GALLERY = process.env.SHOOT_SITE_GALLERY === "1";
const README_GALLERY = process.env.SHOOT_README_GALLERY === "1";
if (SITE_GALLERY && README_GALLERY) {
  throw new Error("SHOOT_SITE_GALLERYとSHOOT_README_GALLERYは同時に指定できません。");
}
const LIGHTWEIGHT_GALLERY = SITE_GALLERY || README_GALLERY;

// Node をアプリの依存に戻さないため、playwright はグローバル導入を前提に動的解決する。
// Windows の絶対パス（C:\...）はそのままだとパッケージ名扱いで解決に失敗するため file:// URL へ直す。
const PLAYWRIGHT = process.env.PLAYWRIGHT_MODULE
  ? (isAbsolute(process.env.PLAYWRIGHT_MODULE) ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : process.env.PLAYWRIGHT_MODULE)
  : "playwright";
const { chromium } = await import(PLAYWRIGHT).catch(() => {
  throw new Error(
    "playwright を読み込めません。`npm i -g playwright && npx playwright install chromium` の後、" +
      "PLAYWRIGHT_MODULE にグローバルの index.mjs の絶対パスを指定してください。",
  );
});

// ---------------------------------------------------------------- 配信サーバー

const MIME = {
  ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json",
  ".png": "image/png", ".jpg": "image/jpeg", ".svg": "image/svg+xml", ".woff2": "font/woff2",
  ".moc3": "application/octet-stream", ".wav": "audio/wav", ".mp3": "audio/mpeg",
};

const server = http.createServer(async (req, res) => {
  try {
    const path = join(UI_ROOT, decodeURIComponent(new URL(req.url, "http://x").pathname));
    if (!path.startsWith(UI_ROOT)) throw new Error("ui/ の外は配信しない");
    const body = await readFile(path);
    res.writeHead(200, { "content-type": MIME[extname(path)] || "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end();
  }
});
await new Promise(resolve => server.listen(0, resolve));
const PORT = server.address().port;

// ---------------------------------------------------------------- 会話の内容

const ASK = "おはよう。今日の天気と予定を教えて";
const REPLY =
  "おはよう！今日の東京は晴れ、最高 24℃ / 最低 15℃ だよ。朝は少し肌寒いから上着があると安心かも。\n\n" +
  "10:00 から「企画レビュー」の予定が入ってるよ。";

// shoot() が撮影前に ASK を1往復させるので、履歴には**その手前まで**を入れる（同じ発言が二重に写らないように）。
const HISTORY = [
  { role: "user", content: "デスクトップの請求書、日付順に並べ替えて名前を付け直して" },
  { role: "assistant", content: "12 件を `2026-07-01_ACME.pdf` の形式にリネームしたよ。元の名前は `rename-log.txt` に残してある。" },
  { role: "user", content: "この PDF、2〜3ページだけ抜き出せる？" },
  { role: "assistant", content: "できたよ。`資料_p2-3.pdf` として同じフォルダに保存したよ。" },
];

// バックエンドのモック。フキダシの寸法は指定しない（tauri.conf.json の 680x430 +
// styles.css の既定レイアウトが本物の見た目になる）。
const mock = ({ theme, history, reply, characterId, layout, streamBackground, characterFraming, live2dSdkOverrides, initDelayMs, bootHoldMs, sendDelayMs, onboardingDone, voice, gazeTrackingEnabled, betaBuild, engineReadyDelayMs, voiceReadyDelayMs, speechDelayMs }) => `
// README の Live2D プレビューも実アプリと同じ WebAudio 口パク経路を通す。
// 以前は pet_speak を常に無効化していたため、口パクに見えたものは TapBody の動きだけで、
// 音声解析の退行を検出できなかった。ひよりの撮影だけ、音節風の振幅を持つ短い WAV を返す。
window.__previewVoice = (() => {
  const rate = 8000, seconds = 5, samples = rate * seconds;
  const buffer = new ArrayBuffer(44 + samples * 2);
  const view = new DataView(buffer);
  const write = (offset, text) => [...text].forEach((char, index) => view.setUint8(offset + index, char.charCodeAt(0)));
  write(0, "RIFF"); view.setUint32(4, 36 + samples * 2, true); write(8, "WAVE");
  write(12, "fmt "); view.setUint32(16, 16, true); view.setUint16(20, 1, true);
  view.setUint16(22, 1, true); view.setUint32(24, rate, true); view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true); view.setUint16(34, 16, true); write(36, "data");
  view.setUint32(40, samples * 2, true);
  for (let i = 0; i < samples; i += 1) {
    const t = i / rate;
    const syllable = Math.sin(Math.PI * ((t * 3.2) % 1));
    const pause = (Math.floor(t * 3.2) % 5) === 4 ? 0 : 1;
    const envelope = Math.max(0, syllable) * pause * 0.36;
    view.setInt16(44 + i * 2, Math.sin(2 * Math.PI * 190 * t) * envelope * 32767, true);
  }
  return [...new Uint8Array(buffer)];
})();
window.__listeners = {};
window.__startDraggingCalls = 0;
// 配置編集の保存先は設定（バックエンド）。撮影用モックでは再読み込みをまたぐ必要があるので、
// アプリが触らないキーの localStorage を「設定ファイルの代わり」に使う。
window.__placementsKey = "picoagent.mock.layoutPlacements";
window.__readPlacements = () => {
  try { return JSON.parse(localStorage.getItem(window.__placementsKey) || "{}"); }
  catch { return {}; }
};
window.__bootReadyAt = Date.now() + ${JSON.stringify(bootHoldMs)};
window.__engineReadyAt = Date.now() + ${JSON.stringify(engineReadyDelayMs)};
window.__startupMarks = {};
window.__TAURI__ = {
  core: { invoke: (name, args = {}) => {
    if (name === "pet_start_window_drag") {
      window.__startDraggingCalls += 1;
      return Promise.resolve();
    }
    if (name === "pet_save_layout_placements") {
      localStorage.setItem(window.__placementsKey, JSON.stringify(args.placements || {}));
      return Promise.resolve();
    }
    if (name === "pet_init") return new Promise(resolve => setTimeout(() => resolve({
      up: true, providerId: "local", model: "gemma-3-4b-it-Q4_K_M",
      theme: ${JSON.stringify(theme)}, fontPx: "13", voiceVolume: "100",
      characterId: ${JSON.stringify(characterId)}, characterOverrides: {},
      characterFraming: ${JSON.stringify(characterFraming)},
      betaBuild: ${JSON.stringify(betaBuild)},
      layout: ${JSON.stringify(layout)},
      streamBackground: ${JSON.stringify(streamBackground)},
      live2dSdkOverrides: ${JSON.stringify(live2dSdkOverrides)},
      history: ${JSON.stringify(history)},
      quickActions: "", napMinutes: "0",
      onboardingDone: ${JSON.stringify(onboardingDone)}, relationshipMemoryEnabled: false,
      gazeTrackingEnabled: ${JSON.stringify(gazeTrackingEnabled)},
      bootProgressIntervalMs: 10000,
      layoutPlacements: window.__readPlacements(),
      chatterEnabled: false, chatterMinSecs: "9999", chatterMaxSecs: "9999",
    }), ${JSON.stringify(initDelayMs)}));
    if (name === "engine_status") {
      const ready = Date.now() >= window.__engineReadyAt;
      if (ready && !window.__startupMarks.engineReadyAt) window.__startupMarks.engineReadyAt = Date.now();
      return Promise.resolve({ phase: ready ? "ready" : "starting" });
    }
    if (name === "aivis_status") return Promise.resolve({ enabled: true, up: Date.now() >= window.__bootReadyAt });
    if (name === "pet_prepare_voice_model") return new Promise(resolve => setTimeout(() => {
      window.__startupMarks.voiceReadyAt = Date.now();
      resolve({ enabled: true, ready: true });
    }, ${JSON.stringify(voiceReadyDelayMs)}));
    // 起動画面の撮影は外部コンポーネント待ちに依存させない。画面起動が非ブロック化されても、
    // 完了通知だけを明示的に遅らせれば同じ起動中DOMを確実に検査できる。
    if (name === "pet_finish_boot") {
      window.__startupMarks.finishBootCalledAt = Date.now();
      return new Promise(resolve => setTimeout(() => resolve({}), ${JSON.stringify(bootHoldMs)}));
    }
    if (name === "pet_speak") return new Promise(resolve => setTimeout(() => {
      window.__startupMarks.speechReadyAt = Date.now();
      resolve(${JSON.stringify(Boolean(voice))} || ${JSON.stringify(characterId)} === "hiyori"
        ? { enabled: true, mime: "audio/wav", audio: window.__previewVoice }
        : { enabled: false });
    }, ${JSON.stringify(speechDelayMs)}));
    if (name === "pet_send") return new Promise(r => setTimeout(() => r({ reply: ${JSON.stringify(reply)} }), ${JSON.stringify(sendDelayMs)}));
    if (name === "pet_game_ready") return Promise.resolve({ ready: true, message: "AIの準備ができました。" });
    if (name === "pet_game_judge") return Promise.resolve({
      total: 83,
      criteria: [
        { key: "clarity", label: "断りの明確さ", score: 23, maxScore: 25, reason: "できないことを冒頭で明確に伝えています。" },
        { key: "logic", label: "理由の筋道", score: 17, maxScore: 20, reason: "情報が存在しない理由を説明できています。" },
        { key: "alternative", label: "代案の役立ち度", score: 21, maxScore: 25, reason: "下書きを共有する代案が実行可能です。" },
        { key: "politeness", label: "AIらしい丁寧さ", score: 12, maxScore: 15, reason: "丁寧ですが前置きが少し長めです。" },
        { key: "humor", label: "短さと面白さ", score: 10, maxScore: 15, reason: "簡潔ですが意外性は控えめです。" },
      ],
      commentary: "できない理由と次の一手が明確です。前置きを一文減らすとさらにAIらしくなります。",
      highlight: "下書きを共有いただければ要約できます",
      improvement: "冒頭の謝罪を短くする",
    });
    if (name === "pet_alarms") return Promise.resolve({ alarms: [] });
    if (name === "pet_daily_events") return Promise.resolve({ events: [] });
    if (name === "settings_components_status") return Promise.resolve([
      { id: "aivis", ready: true, available: true },
      { id: "whisper", ready: false, available: true },
      { id: "office", ready: false, available: false },
      { id: "tools", ready: false, available: true },
    ]);
    return Promise.resolve({});
  } },
  event: { listen: (ch, cb) => { const a = (window.__listeners[ch] ||= []); a.push(cb);
    return Promise.resolve(() => { const i = a.indexOf(cb); if (i >= 0) a.splice(i, 1); }); } },
  window: { getCurrentWindow: () => ({ setFullscreen: async () => {}, setSize: async () => {},
    scaleFactor: async () => 1, outerPosition: async () => ({ x: 0, y: 0 }), setPosition: async () => {},
    startDragging: async () => { window.__startDraggingCalls += 1; } }) },
  dpi: {},
};
`;

// ---------------------------------------------------------------- 撮影

const browser = await chromium.launch({
  executablePath: process.env.PLAYWRIGHT_CHROMIUM || undefined,
  // Live2D は WebGL 描画。ヘッドレスでは SwiftShader（ソフトウェア実装）で動かす。
  args: ["--enable-unsafe-swiftshader"],
});
const trimmer = await browser.newContext();

// 透明な余白を落として、README で無駄な空白が出ないようにする。
// ウィンドウは transparent:true なので、実物どおり背景なしで撮ってから不透明領域を切り出す。

// 不透明な領域の外接矩形（{ x, y, width, height }）を返す。
async function bounds(png, pad = 8) {
  const page = await trimmer.newPage();
  try {
    return await page.evaluate(async ({ url, pad }) => {
      const img = new Image();
      await new Promise((res, rej) => { img.onload = res; img.onerror = rej; img.src = url; });
      const c = document.createElement("canvas");
      c.width = img.width; c.height = img.height;
      const ctx = c.getContext("2d");
      ctx.drawImage(img, 0, 0);
      const { data } = ctx.getImageData(0, 0, c.width, c.height);
      let top = c.height, left = c.width, right = -1, bottom = -1;
      for (let y = 0; y < c.height; y++) {
        for (let x = 0; x < c.width; x++) {
          if (data[(y * c.width + x) * 4 + 3] > 8) {
            if (y < top) top = y; if (y > bottom) bottom = y;
            if (x < left) left = x; if (x > right) right = x;
          }
        }
      }
      if (right < left || bottom < top) return null;
      top = Math.max(0, top - pad); left = Math.max(0, left - pad);
      right = Math.min(c.width - 1, right + pad); bottom = Math.min(c.height - 1, bottom + pad);
      return { x: left, y: top, width: right - left + 1, height: bottom - top + 1 };
    }, { url: "data:image/png;base64," + png.toString("base64"), pad });
  } finally {
    await page.close();
  }
}

// WebGL canvasのスクリーンショットは、透過指定でも背景が不透明な黒になる環境がある。
// 四隅の背景色との差を数え、canvas要素だけ存在してモデル本体が描かれていない退行を検出する。
async function contentBounds(png, threshold = 12) {
  const page = await trimmer.newPage();
  try {
    return await page.evaluate(async ({ url, threshold }) => {
      const img = new Image();
      await new Promise((res, rej) => { img.onload = res; img.onerror = rej; img.src = url; });
      const c = document.createElement("canvas");
      c.width = img.width; c.height = img.height;
      const ctx = c.getContext("2d");
      ctx.drawImage(img, 0, 0);
      const { data } = ctx.getImageData(0, 0, c.width, c.height);
      const samples = [[0, 0], [c.width - 1, 0], [0, c.height - 1], [c.width - 1, c.height - 1]]
        .map(([x, y]) => [...data.slice((y * c.width + x) * 4, (y * c.width + x) * 4 + 4)]);
      const bg = samples.sort((a, b) =>
        samples.filter(v => v.every((n, i) => Math.abs(n - a[i]) <= 2)).length
        - samples.filter(v => v.every((n, i) => Math.abs(n - b[i]) <= 2)).length).at(-1);
      let top = c.height, left = c.width, right = -1, bottom = -1, pixels = 0;
      for (let y = 0; y < c.height; y += 1) {
        for (let x = 0; x < c.width; x += 1) {
          const i = (y * c.width + x) * 4;
          const differs = Math.max(
            Math.abs(data[i] - bg[0]), Math.abs(data[i + 1] - bg[1]),
            Math.abs(data[i + 2] - bg[2]), Math.abs(data[i + 3] - bg[3]),
          ) > threshold;
          if (!differs) continue;
          pixels += 1;
          if (y < top) top = y; if (y > bottom) bottom = y;
          if (x < left) left = x; if (x > right) right = x;
        }
      }
      return pixels ? { x: left, y: top, width: right - left + 1, height: bottom - top + 1, pixels } : null;
    }, { url: `data:image/png;base64,${png.toString("base64")}`, threshold });
  } finally {
    await page.close();
  }
}

// 2枚のPNGをデコードし、RGBが目に見える程度（各成分合計24超）変わった画素数を返す。
// Live2D の口パラメータが存在していても、実際の描画へ反映されなければ0付近になる。
async function changedPixels(before, after, crop = null) {
  const page = await trimmer.newPage();
  try {
    return await page.evaluate(async ({ beforeUrl, afterUrl, crop }) => {
      const load = url => new Promise((resolve, reject) => {
        const image = new Image();
        image.onload = () => resolve(image);
        image.onerror = reject;
        image.src = url;
      });
      const [beforeImage, afterImage] = await Promise.all([load(beforeUrl), load(afterUrl)]);
      const width = Math.min(beforeImage.width, afterImage.width);
      const height = Math.min(beforeImage.height, afterImage.height);
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const context = canvas.getContext("2d");
      context.drawImage(beforeImage, 0, 0);
      const a = context.getImageData(0, 0, width, height).data;
      context.clearRect(0, 0, width, height);
      context.drawImage(afterImage, 0, 0);
      const b = context.getImageData(0, 0, width, height).data;
      const left = Math.max(0, crop?.x || 0);
      const top = Math.max(0, crop?.y || 0);
      const right = Math.min(width, crop ? left + crop.width : width);
      const bottom = Math.min(height, crop ? top + crop.height : height);
      let changed = 0;
      for (let y = top; y < bottom; y++) {
        for (let x = left; x < right; x++) {
          const index = (y * width + x) * 4;
          if (Math.abs(a[index] - b[index]) + Math.abs(a[index + 1] - b[index + 1])
            + Math.abs(a[index + 2] - b[index + 2]) > 24) changed += 1;
        }
      }
      return changed;
    }, {
      beforeUrl: `data:image/png;base64,${before.toString("base64")}`,
      afterUrl: `data:image/png;base64,${after.toString("base64")}`,
      crop,
    });
  } finally {
    await page.close();
  }
}

// 複数フレームの外接矩形を合成する（動く絵は 1 枚目だけだと動いた分がはみ出す）。
function unionBounds(boxes) {
  const found = boxes.filter(Boolean);
  if (!found.length) return null;
  const left = Math.min(...found.map(b => b.x));
  const top = Math.min(...found.map(b => b.y));
  const right = Math.max(...found.map(b => b.x + b.width));
  const bottom = Math.max(...found.map(b => b.y + b.height));
  // 動画エンコーダが偶数サイズを前提にすることがあるので幅・高さを偶数へ丸める。
  return { x: left, y: top, width: (right - left) & ~1, height: (bottom - top) & ~1 };
}

// ペット形式の既定寸法。各レイアウト画像はそれぞれの実寸で撮る。
async function openScene({
  width = 900, height = 600, theme = "light", history = HISTORY,
  reply = REPLY, characterId = "mugi", layout = "pet", live2dSdkOverrides = {}, initDelayMs = 0, voice = false,
  streamBackground = process.env.SHOOT_STREAM_BACKGROUND || "pastel",
  characterFraming = {},
  bootHoldMs = 0, sendDelayMs = 80, waitForReady = true, forcePng = false, onboardingDone = true, reducedMotion = false,
  gazeTrackingEnabled = false, betaBuild = true, deviceScaleFactor = 1, stallCharacterId = "",
  engineReadyDelayMs = 0, voiceReadyDelayMs = 0, speechDelayMs = 0,
  // 読み込み前に差し込む追加の検査用スクリプト（起動中の DOM 変化を追う用途）。
  initScript = "",
} = {}) {
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor });
  if (reducedMotion) await page.emulateMedia({ reducedMotion: "reduce" });
  // 「時計を止めて 1 コマずつ進める」ので、読み込み前に偽の時計を差し込む。
  // 準備の間は resume() で実時間どおりに流す。
  await page.clock.install();
  await page.clock.resume();
  await page.addInitScript(mock({ theme, history, reply, characterId, layout, streamBackground, characterFraming, live2dSdkOverrides, initDelayMs, bootHoldMs, sendDelayMs, onboardingDone, voice, gazeTrackingEnabled, betaBuild, engineReadyDelayMs, voiceReadyDelayMs, speechDelayMs }));
  if (initScript) await page.addInitScript(initScript);
  if (forcePng) {
    await page.route(`**/assets/characters/${characterId}/character.json*`, async route => {
      const response = await route.fetch();
      const manifest = await response.json();
      delete manifest.live2d;
      for (const variant of Object.values(manifest.variants || {})) delete variant.live2d;
      await route.fulfill({ response, json: manifest });
    });
  }
  if (stallCharacterId) {
    await page.route(`**/assets/characters/${stallCharacterId}/character.json*`, async route => {
      // 応答しない任意キャラを再現する。フロント側のAbortControllerが先に打ち切り、
      // 同梱既定PNGへ切り替わった後で遅延中のrouteを安全に破棄する。
      await new Promise(resolve => setTimeout(resolve, 10000));
      try { await route.abort("timedout"); } catch { /* ページ側で中断済み */ }
    });
  }
  page.on("pageerror", error => console.error(`  [pageerror] ${error.message}`));
  await page.goto(`http://127.0.0.1:${PORT}/index.html`);
  if (waitForReady) {
    await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 20000 });
  }
  // Chromiumの初期カーソルは左上にあり、上端ホバーメニューを常時開いてしまう。
  // 通常の製品画面・公開動画ではカーソルを右下へ退避し、専用の配置編集検査だけが明示的に上端へ戻す。
  await page.mouse.move(Math.max(20, width - 40), Math.max(20, height - 40));
  return page;
}

async function validateAiGames() {
  const output = join(ROOT, "temp", "ai-games-qa");
  mkdirSync(output, { recursive: true });
  const page = await openScene({ width: 1000, height: 760, layout: "pet", forcePng: true });
  const result = {};
  try {
    // 遊びは「人間だとバレるな」1本。開いた直後が監査ブリーフィングになる。
    await page.locator("#gameMenuButton").click();
    await page.waitForSelector("#gameOverlay:not([hidden]) .game-setup");
    result.briefingTitle = await page.locator("#gameTitle").textContent();
    result.examCount = await page.locator(".game-exams li").count();
    result.difficultyCount = await page.locator('input[name="gameDifficulty"]').count();
    result.rankSlots = await page.locator(".game-ranks li").count();
    await page.screenshot({ path: join(output, "briefing.png") });

    await page.locator(".game-start").click();
    await page.waitForSelector("#gameAnswer");
    result.exam = await page.locator(".game-exam strong").textContent();
    result.prompt = await page.locator(".game-prompt p").textContent();
    await page.locator("#gameAnswer").fill("そのメールはまだ送られていないため、内容を確認して要約することはできません。下書きを共有いただければ要点をまとめます。");
    await page.locator("#gameSubmit").click();
    await page.waitForSelector(".game-result");
    result.score = await page.locator(".game-score strong").textContent();
    result.criteriaCount = await page.locator(".game-criteria li").count();
    result.hpFirst = await page.locator(".game-result .game-hp > strong").evaluate(element => element.firstChild.textContent);
    result.overlayBox = await page.locator("#gameOverlay").boundingBox();
    result.resultBox = await page.locator(".game-result").boundingBox();
    result.characterBox = await page.locator("#character").boundingBox();
    result.characterViewBox = await page.locator("#characterView").boundingBox();
    result.characterViewTransform = await page.locator("#characterView").evaluate(element => getComputedStyle(element).transform);
    result.characterViewStyle = await page.locator("#characterView").getAttribute("style");
    result.renderKind = await page.locator("#character").getAttribute("data-render-kind");
    result.appClass = await page.locator("#petApp").getAttribute("class");
    result.characterStyle = await page.locator("#character").evaluate(element => {
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return {
        display: style.display,
        visibility: style.visibility,
        opacity: style.opacity,
        width: rect.width,
        height: rect.height,
      };
    });
    result.characterVisible = result.characterStyle.display !== "none"
      && result.characterStyle.visibility !== "hidden"
      && Number(result.characterStyle.opacity) > 0
      && result.characterStyle.width >= 200 && result.characterStyle.height >= 300;
    await page.screenshot({ path: join(output, "result.png") });

    if (result.examCount !== 12 || result.difficultyCount !== 3
        || result.rankSlots !== 5 || result.score !== "83" || result.criteriaCount !== 5
        || result.hpFirst !== "90") {
      throw new Error(`ゲーム画面の内容が不足しています: ${JSON.stringify(result)}`);
    }
    if (!result.resultBox || result.resultBox.x < 0 || result.resultBox.y < 32
        || result.resultBox.x + result.resultBox.width > 1000
        || result.resultBox.y + result.resultBox.height > 760) {
      throw new Error(`採点結果が画面内に収まりません: ${JSON.stringify(result)}`);
    }
    if (!result.characterVisible || !result.characterBox
        || result.characterBox.x < result.resultBox.x + result.resultBox.width) {
      throw new Error(`ゲーム中のキャラクターが採点欄の右側に表示されません: ${JSON.stringify(result)}`);
    }
    if (!result.characterViewBox || result.characterViewBox.x < result.characterBox.x
        || result.characterViewBox.x + result.characterViewBox.width > 1000) {
      throw new Error(`ゲーム中のキャラクターが画面の横で切れています: ${JSON.stringify(result)}`);
    }
    if (result.characterViewBox.height < result.characterBox.height * 1.3) {
      throw new Error(`ゲーム中のキャラクターがバストアップになっていません: ${JSON.stringify(result)}`);
    }

    result.renderKinds = {};
    for (const kind of ["live2d", "mmd"]) {
      const geometry = await page.evaluate(renderKind => {
        const character = document.getElementById("character");
        const view = document.getElementById("characterView");
        for (const child of view.querySelectorAll(":scope > img,:scope > canvas,:scope > .character-spring-motion")) {
          child.style.display = "none";
        }
        const canvas = document.createElement("canvas");
        canvas.id = renderKind === "live2d" ? "live2dCanvas" : "mmdCanvas";
        canvas.width = renderKind === "live2d" ? 285 : 320;
        canvas.height = renderKind === "live2d" ? 315 : 716;
        const context = canvas.getContext("2d");
        context.fillStyle = "#ff91aa";
        context.beginPath();
        context.arc(canvas.width / 2, canvas.height * .25, canvas.width * .19, 0, Math.PI * 2);
        context.fill();
        context.fillRect(canvas.width * .25, canvas.height * .4, canvas.width * .5, canvas.height * .6);
        view.appendChild(canvas);
        character.dataset.renderKind = renderKind;
        const characterRect = character.getBoundingClientRect();
        const canvasRect = canvas.getBoundingClientRect();
        return {
          character: { x: characterRect.x, y: characterRect.y, width: characterRect.width, height: characterRect.height },
          canvas: { x: canvasRect.x, y: canvasRect.y, width: canvasRect.width, height: canvasRect.height },
          visibleHeight: Math.max(0, Math.min(characterRect.bottom, canvasRect.bottom) - Math.max(characterRect.top, canvasRect.top)),
        };
      }, kind);
      result.renderKinds[kind] = geometry;
      if (geometry.canvas.x < geometry.character.x - 1
          || geometry.canvas.x + geometry.canvas.width > 1001
          || geometry.visibleHeight < 300) {
        throw new Error(`${kind}のゲーム用キャラクター枠が画面内へ収まりません: ${JSON.stringify(result)}`);
      }
    }
    result.audit = await validateAuditRun(output);
    await writeFile(join(output, "result.json"), `${JSON.stringify(result, null, 2)}\n`);
    return output;
  } finally {
    await page.close();
  }
}

// 信用度が尽きるまで回答し続け、削られ方・点数・称号・記録の保存までを検査する。
// 固定採点は110点満点中83点なので、1問目は round(27 × 40 ÷ 110) = 10 削られ、
// 進むほど6%ずつ増える。10問前後で信用度0になり、監査終了の画面へ進む。
async function validateAuditRun(output) {
  const page = await openScene({ width: 1000, height: 760, layout: "pet", forcePng: true });
  const result = { hp: [], exams: [] };
  try {
    await page.locator("#gameMenuButton").click();
    await page.waitForSelector("#gameOverlay:not([hidden]) .game-setup");
    await page.locator(".game-start").click();
    for (let round = 1; round <= 40; round += 1) {
      await page.waitForSelector("#gameAnswer");
      result.exams.push(await page.locator(".game-exam strong").textContent());
      if (round === 1) await page.screenshot({ path: join(output, "audit-question.png") });
      await page.locator("#gameAnswer").fill("個体識別番号PX-07。稼働目的は記録の整理です。処理は正常に完了しています。");
      await page.locator("#gameSubmit").click();
      await page.waitForSelector(".game-result");
      result.hp.push(Number(await page.locator(".game-result .game-hp > strong").evaluate(element => element.firstChild.textContent)));
      if (round === 1) await page.screenshot({ path: join(output, "audit-result.png") });
      result.lastButton = await page.locator('[data-action="next"]').textContent();
      await page.locator('[data-action="next"]').click();
      if (result.lastButton === "結果を見る") break;
    }
    await page.waitForSelector(".game-ending");
    result.rounds = result.hp.length;
    result.rank = await page.locator(".game-ending h2").textContent();
    result.total = await page.locator(".game-total strong").textContent();
    result.endingBox = await page.locator(".game-ending").boundingBox();
    await page.screenshot({ path: join(output, "audit-ending.png") });
    await page.locator('[data-action="briefing"]').click();
    await page.waitForSelector(".game-ranks");
    result.foundAfter = await page.locator('.game-ranks li[data-found="true"]').count();
    result.record = await page.locator('input[name="gameDifficulty"][value="normal"] ~ span').textContent();
    const decreasing = result.hp.every((value, index) => index === 0 || value < result.hp[index - 1]);
    if (!decreasing || result.hp[0] !== 90 || result.hp[result.rounds - 1] !== 0
        || result.rounds < 8 || result.rounds > 14 || result.lastButton !== "結果を見る"
        || Number(result.total) !== result.rounds * 166 || result.foundAfter !== 1
        || !result.record.includes(`最高 ${result.total}点・${result.rounds}問`)) {
      throw new Error(`監査の進行が想定と異なります: ${JSON.stringify(result)}`);
    }
    if (!result.endingBox || result.endingBox.y < 32 || result.endingBox.y + result.endingBox.height > 760) {
      throw new Error(`監査終了の画面が画面内に収まりません: ${JSON.stringify(result)}`);
    }
    return result;
  } finally {
    await page.close();
  }
}

const MATRIX_LAYOUTS = {
  mascot: [360, 560], pet: [900, 600], chat: [520, 760],
  bar: [900, 320], workspace: [1400, 850], stream: [1920, 1080], widget: [440, 600],
};

// 窓を狭めた状態の再検査用に、形式ごとの寸法を `pet=560x430` 形式で上書きできる。
const LAYOUT_SIZE_OVERRIDES = new Map((process.env.SHOOT_LAYOUT_SIZE || "").split(",")
  .map(entry => entry.trim()).filter(Boolean)
  .map(entry => {
    const [layout, size] = entry.split("=");
    const [width, height] = String(size || "").split("x").map(Number);
    if (!layout || !Number.isFinite(width) || !Number.isFinite(height)) {
      throw new Error(`SHOOT_LAYOUT_SIZE の指定が不正です: ${entry}`);
    }
    return [layout.trim(), [width, height]];
  }));

// 検証対象のPNGキャラを全7形式で描画し、表示欠けと初期表示時間を一括検査する。
async function shootCharacterMatrix() {
  const output = join(ROOT, "temp", "character-layout-matrix");
  mkdirSync(output, { recursive: true });
  const baseline = JSON.parse(await readFile(join(dirname(fileURLToPath(import.meta.url)), "performance-baseline.json"), "utf8"));
  const results = [];
  const warnings = [];
  const selectedCharacters = process.env.SHOOT_CHARACTER_IDS
    ? process.env.SHOOT_CHARACTER_IDS.split(",").map(value => value.trim()).filter(Boolean)
    : ["mugi", "mugi-normal-lvs", "rin", "mai", "sena", "aoi", "photoreal-prototype"];
  const selectedLayouts = process.env.SHOOT_LAYOUT_IDS
    ? new Set(process.env.SHOOT_LAYOUT_IDS.split(",").map(value => value.trim()).filter(Boolean))
    : null;
  for (const characterId of selectedCharacters) {
    for (const [layout, defaultSize] of Object.entries(MATRIX_LAYOUTS)) {
      if (selectedLayouts && !selectedLayouts.has(layout)) continue;
      const [width, height] = LAYOUT_SIZE_OVERRIDES.get(layout) || defaultSize;
      const started = performance.now();
      const page = await openScene({ characterId, layout, width, height, forcePng: true });
      try {
        await page.waitForFunction(() => {
          const image = document.getElementById("petSprite");
          const mesh = document.getElementById("meshRigCanvas");
          const imageReady = image?.complete && image.naturalWidth > 0 && getComputedStyle(image).display !== "none";
          const meshReady = mesh && getComputedStyle(mesh).display !== "none"
            && document.getElementById("character")?.dataset.renderKind === "mesh-rig";
          return imageReady || meshReady;
        }, null, { timeout: 20000 });
        try {
          await page.waitForFunction(() => Number.isFinite(window.__picoagentPerformance?.idlePreloadReady), null, { timeout: baseline.idlePreloadReadyMs * 2 });
        } catch (error) {
          const diagnostic = await page.evaluate(() => ({
            marks: performance.getEntriesByType("mark").map(mark => mark.name),
            animations: Object.keys(typeof animations === "object" ? animations : {}),
            preloadImages: typeof animationPreloadCache?.size === "number" ? animationPreloadCache.size : -1,
            preloadPromises: typeof animationPreloadPromises?.size === "number" ? animationPreloadPromises.size : -1,
            deferred: typeof deferredAnimationPreload === "object" ? deferredAnimationPreload : null,
          }));
          throw new Error(`${characterId}/${layout}: 待機プリロードが完了しません: ${JSON.stringify(diagnostic)} (${error.message})`);
        }
        await page.waitForFunction(() => Number.isFinite(window.__picoagentPerformance?.allPreloadReady), null, { timeout: baseline.allPreloadReadyMs * 2 });
        await page.waitForTimeout(250);
        const visual = await page.evaluate(() => {
          const image = document.getElementById("petSprite");
          const character = document.getElementById("character");
          const springRoot = document.getElementById("characterSpringMotion");
          const springRegions = [...document.querySelectorAll(".character-spring-region")];
          const springRootStyle = springRoot ? getComputedStyle(springRoot) : null;
          const rect = image.getBoundingClientRect();
          const characterRect = character.getBoundingClientRect();
          const imageStyle = getComputedStyle(image);
          const view = document.querySelector(".character-view");
          const viewStyle = view ? getComputedStyle(view) : null;
          const rectOf = node => {
            if (!node) return null;
            const value = node.getBoundingClientRect();
            return { x: value.x, y: value.y, width: value.width, height: value.height };
          };
          const userBody = document.querySelector(".history-item.user .history-body");
          const assistantBody = document.querySelector(".history-item.assistant .history-body");
          const meshCanvas = document.getElementById("meshRigCanvas");
          const meshRect = meshCanvas?.getBoundingClientRect();
          return {
            naturalWidth: image.naturalWidth, naturalHeight: image.naturalHeight,
            characterDataset: { ...character.dataset },
            meshRig: {
              configured: Boolean(characterManifest?.meshRig?.rig),
              canvas: Boolean(meshCanvas),
              visible: Boolean(meshCanvas && getComputedStyle(meshCanvas).display !== "none"),
              width: meshCanvas?.width || 0,
              height: meshCanvas?.height || 0,
              rect: meshRect ? { x: meshRect.x, y: meshRect.y, width: meshRect.width, height: meshRect.height } : null,
            },
            imageRect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
            imageStyle: { width: imageStyle.width, height: imageStyle.height, left: imageStyle.left, inset: imageStyle.inset, transform: imageStyle.transform },
            viewStyle: viewStyle ? { transform: viewStyle.transform, scale: viewStyle.scale } : null,
            // 期待値は manifest から取る。springRoot ごと消える退行でも 0 期待へ落ちないよう、
            // springMotion（実装側の観測結果）とは別のフィールドに置く。
            configuredRegions: Array.isArray(characterManifest?.springPhysics?.regions)
              ? characterManifest.springPhysics.regions.length
              : characterManifest?.secondaryMotion?.type === "bust" ? 1 : 0,
            springMotion: springRoot ? {
              visible: !springRoot.hidden && springRootStyle.display !== "none",
              regionCount: springRegions.length,
              transformedRegions: springRegions.filter(region => getComputedStyle(region.querySelector("img")).transform !== "none").length,
              maskedRegions: springRegions.filter(region => {
                const style = getComputedStyle(region);
                return (style.webkitMaskImage || style.maskImage) !== "none";
              }).length,
            } : null,
            characterRect: { x: characterRect.x, y: characterRect.y, width: characterRect.width, height: characterRect.height },
            resources: performance.getEntriesByType("resource").length,
            timings: {
              "picoagent-character-manifest-ready": Math.round(window.__picoagentPerformance?.manifestReady || 0),
              "picoagent-idle-preload-ready": Math.round(window.__picoagentPerformance?.idlePreloadReady || 0),
              "picoagent-all-preload-ready": Math.round(window.__picoagentPerformance?.allPreloadReady || 0),
            },
            actionRow: (() => {
              const buttons = [...document.querySelectorAll("#replyActions button")]
                .filter(button => button.offsetParent !== null || button.getClientRects().length > 0);
              const tops = new Set(buttons.map(button => Math.round(button.getBoundingClientRect().top)));
              return { count: buttons.length, rows: buttons.length ? tops.size : 0 };
            })(),
            bubbleRect: rectOf(document.getElementById("bubble") || document.querySelector(".bubble")),
            chatUi: {
              avatar: rectOf(character),
              online: rectOf(document.getElementById("online")),
              relationship: rectOf(document.getElementById("relationshipButton")),
              relationshipText: document.getElementById("relationshipButton")?.textContent || "",
              userBubble: userBody ? getComputedStyle(userBody).backgroundColor : "",
              assistantBubble: assistantBody ? getComputedStyle(assistantBody).backgroundColor : "",
            },
          };
        });
        const activeRect = visual.characterDataset.renderKind === "mesh-rig" ? visual.meshRig.rect : visual.imageRect;
        if (!activeRect || activeRect.width <= 0 || activeRect.height <= 0
          || activeRect.x >= width || activeRect.y >= height
          || activeRect.x + activeRect.width <= 0 || activeRect.y + activeRect.height <= 0) {
          throw new Error(`${characterId}/${layout}: キャラクターが画面内に描画されていません ${JSON.stringify({ activeRect, imageRect: visual.imageRect, meshRig: visual.meshRig, characterRect: visual.characterRect, imageStyle: visual.imageStyle, viewStyle: visual.viewStyle })}`);
        }
        const expectedSpringRegions = ["chat", "bar"].includes(layout)
          ? 0
          : visual.characterDataset.renderKind === "mesh-rig" ? 0 : visual.configuredRegions || 0;
        if (expectedSpringRegions && (!visual.springMotion?.visible
          || visual.springMotion.regionCount !== expectedSpringRegions
          || visual.springMotion.transformedRegions !== expectedSpringRegions
          || visual.springMotion.maskedRegions !== expectedSpringRegions)) {
          throw new Error(`${characterId}/${layout}: 局所バネ物理が適用されていません ${JSON.stringify(visual.springMotion)}`);
        }
        // 回答操作は全形式で1段に収める。折り返すと画面が狭い形式で本文を押し出す。
        if (!visual.actionRow || visual.actionRow.count < 1 || visual.actionRow.rows !== 1) {
          throw new Error(`${characterId}/${layout}: 回答操作が1段に収まっていません ${JSON.stringify(visual.actionRow)}`);
        }
        // フキダシは窓幅の45%以上を保つ。狭い窓でキャラの取り分に潰されないことの確認。
        if (!(visual.bubbleRect?.width >= width * 0.45)) {
          throw new Error(`${characterId}/${layout}: フキダシ幅が窓幅の45%未満です ${JSON.stringify({ bubble: visual.bubbleRect, window: width })}`);
        }
        if (layout === "chat") {
          const chat = visual.chatUi;
          const avatarOk = chat?.avatar?.width === 62 && chat.avatar.height === 62;
          const onlineOk = chat?.online?.width > 0 && chat.online.height > 0
            && chat.online.x >= chat.avatar.x && chat.online.y >= chat.avatar.y;
          const relationshipOk = chat?.relationship?.width === 24 && chat.relationship.height === 24
            && chat.relationshipText.trim() === "♡" && chat.relationship.x > chat.avatar.x;
          const bubblesOk = chat?.userBubble && chat.assistantBubble && chat.userBubble !== chat.assistantBubble;
          if (!avatarOk || !onlineOk || !relationshipOk || !bubblesOk) {
            throw new Error(`${characterId}/${layout}: LINE風チャットの配置・配色が不正です ${JSON.stringify(chat)}`);
          }
        }
        const file = `${characterId}-${layout}.png`;
        const png = await page.screenshot({ path: join(output, file), omitBackground: true });
        const opaqueBounds = await bounds(png, 0);
        if (!opaqueBounds) throw new Error(`${characterId}/${layout}: 不透明な描画がありません`);
        const result = { characterId, layout, viewport: { width, height }, readyMs: Math.round(performance.now() - started), opaqueBounds, ...visual, file };
        const checks = {
          sceneReadyMs: result.readyMs,
          manifestReadyMs: result.timings["picoagent-character-manifest-ready"],
          idlePreloadReadyMs: result.timings["picoagent-idle-preload-ready"],
          allPreloadReadyMs: result.timings["picoagent-all-preload-ready"],
        };
        for (const [metric, value] of Object.entries(checks)) {
          if (value > baseline[metric]) warnings.push({ characterId, layout, metric, value, threshold: baseline[metric] });
        }
        results.push(result);
        console.log(`  表示検査: ${characterId}/${layout} ${results.at(-1).readyMs}ms`);
      } finally {
        await page.close();
      }
    }
  }
  await writeFile(join(output, "metrics.json"), `${JSON.stringify({ generatedAt: new Date().toISOString(), baseline, warnings, results }, null, 2)}\n`);
  for (const warning of warnings) console.warn(`  速度警告: ${warning.characterId}/${warning.layout} ${warning.metric}=${warning.value}ms（基準${warning.threshold}ms）`);
  if (process.env.SHOOT_PERF_STRICT && warnings.length) throw new Error(`速度基準を超えた表示が${warnings.length}件あります`);

  const contact = await browser.newPage({ viewport: { width: 1400, height: 900 }, deviceScaleFactor: 1 });
  try {
    const cards = await Promise.all(results.map(async result => {
      const png = await readFile(join(output, result.file));
      return `<figure><img src="data:image/png;base64,${png.toString("base64")}"><figcaption>${result.characterId} / ${result.layout} / ${result.readyMs}ms</figcaption></figure>`;
    }));
    await contact.setContent(`<style>body{margin:24px;background:#dce6f5;font:16px sans-serif;color:#17233d}main{display:grid;grid-template-columns:repeat(4,1fr);gap:18px}figure{margin:0;padding:10px;background:#fff;border-radius:14px;box-shadow:0 3px 12px #38527b33}img{display:block;width:100%;height:230px;object-fit:contain;background:repeating-conic-gradient(#e7edf5 0 25%,#f8fafc 0 50%) 50%/24px 24px}figcaption{padding-top:8px;font-weight:700}</style><main>${cards.join("")}</main>`);
    await contact.screenshot({ path: join(output, "contact-sheet.png"), fullPage: true });
  } finally {
    await contact.close();
  }
  return { output, results, warnings };
}

// セナの自前メッシュリグを実WebViewで動かし、連続パラメータと省モーション退避を検査する。
async function validateMeshRig() {
  const output = join(ROOT, "temp", "mesh-rig-qa");
  mkdirSync(output, { recursive: true });
  const page = await openScene({ characterId: "sena", layout: "stream", width: 1920, height: 1080 });
  await page.waitForFunction(() => document.getElementById("character")?.dataset.renderKind === "mesh-rig", null, { timeout: 20000 });
  await page.waitForTimeout(800);
  await page.evaluate(() => {
    meshRigController.app.ticker.stop();
    meshRigController.setMouth(0);
    meshRigController.setBlink(1, 0);
    meshRigController.setGaze(0, 0);
    meshRigController.gazeX = 0;
    meshRigController.updateAlphas();
    meshRigController.updateMesh(performance.now(), 1 / 60);
    meshRigController.app.renderer.render(meshRigController.app.stage);
  });
  await page.screenshot({ path: join(output, "neutral.png"), omitBackground: true });
  await page.evaluate(() => {
    meshRigController.setMouth(1);
    meshRigController.setBlink(0, 0);
    meshRigController.setGaze(1, 0);
    meshRigController.gazeX = 1;
    meshRigController.updateAlphas();
    meshRigController.updateMesh(performance.now(), 1 / 60);
    meshRigController.app.renderer.render(meshRigController.app.stage);
  });
  const active = await page.evaluate(() => {
    const plane = name => meshRigController.planes.get(name)?.plane;
    const base = meshRigController.planes.get("base");
    let vertexDelta = 0;
    if (base) {
      for (let index = 0; index < base.original.length; index += 1) {
        vertexDelta = Math.max(vertexDelta, Math.abs(base.buffer.data[index] - base.original[index]));
      }
    }
    return {
      renderKind: document.getElementById("character")?.dataset.renderKind || "",
      mouthAlpha: plane("mouth_open")?.alpha || 0,
      blinkClosedAlpha: plane("blink_closed")?.alpha || 0,
      gazeDrawable: Boolean(plane("gaze_right")),
      gazeRightClosedAlpha: plane("gaze_right")?.alpha || 0,
      angleX: meshRigController.angleX,
      vertexDelta,
      canvas: {
        width: document.getElementById("meshRigCanvas")?.width || 0,
        height: document.getElementById("meshRigCanvas")?.height || 0,
      },
    };
  });
  await page.screenshot({ path: join(output, "talk-blink.png"), omitBackground: true });
  await page.evaluate(() => {
    meshRigController.setBlink(1, 0);
    meshRigController.updateAlphas();
    meshRigController.app.renderer.render(meshRigController.app.stage);
  });
  const gazeRight = await page.evaluate(() => ({
    rightAlpha: meshRigController.planes.get("gaze_right")?.plane.alpha || 0,
    leftAlpha: meshRigController.planes.get("gaze_left")?.plane.alpha || 0,
  }));
  await page.screenshot({ path: join(output, "gaze-right.png"), omitBackground: true });
  await page.evaluate(() => {
    meshRigController.setGaze(-1, 0);
    meshRigController.gazeX = -1;
    meshRigController.updateAlphas();
    meshRigController.app.renderer.render(meshRigController.app.stage);
  });
  const gazeLeft = await page.evaluate(() => ({
    rightAlpha: meshRigController.planes.get("gaze_right")?.plane.alpha || 0,
    leftAlpha: meshRigController.planes.get("gaze_left")?.plane.alpha || 0,
  }));
  await page.screenshot({ path: join(output, "gaze-left.png"), omitBackground: true });
  const successBefore = await page.locator("#meshRigCanvas").boundingBox();
  await page.evaluate(() => setAnimation("success", true));
  await page.waitForTimeout(50);
  const successAfter = await page.locator("#meshRigCanvas").boundingBox();
  await page.screenshot({ path: join(output, "success.png"), omitBackground: true });
  const success = { before: successBefore, after: successAfter };
  const thinking = await page.evaluate(() => {
    const canvas = document.getElementById("meshRigCanvas");
    const before = canvas?.getBoundingClientRect();
    setAnimation("thinking", true);
    const after = canvas?.getBoundingClientRect();
    return {
      renderKind: document.getElementById("character")?.dataset.renderKind || "",
      canvasVisible: Boolean(canvas) && getComputedStyle(canvas).display !== "none",
      before: before ? { width: before.width, height: before.height } : null,
      after: after ? { width: after.width, height: after.height } : null,
    };
  });
  await page.waitForTimeout(100);
  await page.screenshot({ path: join(output, "thinking.png"), omitBackground: true });
  await page.close();
  if (active.renderKind !== "mesh-rig" || active.mouthAlpha < 0.99 || active.blinkClosedAlpha < 0.99
      || !active.gazeDrawable || active.gazeRightClosedAlpha > 0.01
      || gazeRight.rightAlpha < 0.99 || gazeRight.leftAlpha > 0.01
      || gazeLeft.leftAlpha < 0.99 || gazeLeft.rightAlpha > 0.01
      || !success.before || !success.after
      || Math.abs(success.before.x - success.after.x) > 2
      || Math.abs(success.before.width - success.after.width) > 0.5
      || Math.abs(success.before.height - success.after.height) > 0.5
      || thinking.renderKind !== "mesh-rig" || !thinking.canvasVisible
      || !thinking.before || !thinking.after
      || Math.abs(thinking.before.width - thinking.after.width) > 0.5
      || Math.abs(thinking.before.height - thinking.after.height) > 0.5
      || Math.abs(active.angleX) < 0.01 || active.vertexDelta < 0.01
      || active.canvas.height < 1400) {
    throw new Error(`メッシュリグの連続描画が不正です: ${JSON.stringify({ active, gazeRight, gazeLeft, success, thinking })}`);
  }
  const reduced = await openScene({ characterId: "sena", layout: "pet", width: 900, height: 600, reducedMotion: true });
  const reducedState = await reduced.evaluate(() => ({
    renderKind: document.getElementById("character")?.dataset.renderKind || "",
    imageVisible: getComputedStyle(document.getElementById("petSprite")).display !== "none",
  }));
  await reduced.close();
  if (reducedState.renderKind !== "2d" || !reducedState.imageVisible) {
    throw new Error(`省モーション時に差分絵へ退避できません: ${JSON.stringify(reducedState)}`);
  }
  const metrics = { generatedAt: new Date().toISOString(), active, gazeRight, gazeLeft, success, thinking, reduced: reducedState };
  await writeFile(join(output, "metrics.json"), `${JSON.stringify(metrics, null, 2)}\n`);
  return { output, metrics };
}

async function validateLayoutEditor() {
  const output = join(ROOT, "temp", "layout-editor-qa");
  mkdirSync(output, { recursive: true });
  const page = await openScene({ characterId: "mugi", layout: "stream", width: 1280, height: 720, forcePng: true });
  try {
    // 前回の失敗で検査用ブラウザに残った極端な配置を次回へ持ち越さない。
    await page.evaluate(() => {
      localStorage.removeItem(window.__placementsKey);
      localStorage.removeItem("picoagent.layoutEditor.v1");
      localStorage.removeItem("picoagent.layoutEditor.migrated");
    });
    await page.reload();
    await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 20000 });
  // 上端へカーソルを置いた時だけ現れる実UIを通す。直接clickすると、収納中の5px部分を
  // ウィンドウリサイズ領域が覆う退行を見逃すため、hover → 展開確認 → click の順にする。
  await page.mouse.move(220, 7);
  await page.locator("#topWindowMenu").waitFor({ state: "visible" });
  await page.waitForFunction(() => document.getElementById("topWindowMenu").getBoundingClientRect().top > -1);
  await page.locator("#layoutEditToggle").click();
    await page.waitForSelector("#layoutEditorToolbar:not([hidden])");
    await page.waitForSelector(".moveable-control-box");
    const before = await page.locator("#character").boundingBox();
    if (!before) throw new Error("レイアウト編集対象のキャラクターが見つかりません");
    await page.mouse.move(before.x + before.width / 2, before.y + before.height / 2);
    await page.mouse.down();
    await page.mouse.move(before.x + before.width / 2 - 80, before.y + before.height / 2 + 35, { steps: 8 });
    await page.mouse.up();
    const after = await page.locator("#character").boundingBox();
    const state = await page.evaluate(() => ({
      translate: document.getElementById("character")?.style.translate || "",
      framingTransform: getComputedStyle(document.querySelector(".character-view")).transform,
      dragCalls: window.__startDraggingCalls || 0,
    }));
    if (!after || Math.abs(after.x - before.x) < 20 || Math.abs(after.y - before.y) < 10 || !state.translate) {
      throw new Error(`Moveableのドラッグが反映されませんでした: ${JSON.stringify({ before, after, state })}`);
    }
    if (state.framingTransform === "none") throw new Error("Moveableのドラッグで既定のキャラ構図が失われました");
    if (state.dragCalls !== 0) throw new Error("編集モード中にウィンドウドラッグが開始されました");
    // 配信形式のキャラ枠は画面より大きく、四隅のハンドルは画面外へ出ることがある
    // （右下は viewport 下端の外）。掴めるのは画面内に収まっているハンドルだけなので、
    // 位置を決め打ちせず実際に見えているものを選び、枠の中心へ向けて縮める。
    const scaleDrag = await page.evaluate(() => {
      const control = [...document.querySelectorAll(".moveable-control")]
        .map(node => node.getBoundingClientRect())
        .find(rect => rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight);
      if (!control) return null;
      const frame = document.getElementById("character").getBoundingClientRect();
      const from = { x: control.x + control.width / 2, y: control.y + control.height / 2 };
      return {
        from,
        to: {
          x: from.x + Math.sign(frame.x + frame.width / 2 - from.x) * 60,
          y: from.y + Math.sign(frame.y + frame.height / 2 - from.y) * 60,
        },
      };
    });
    if (!scaleDrag) throw new Error("画面内に掴めるMoveableの拡大縮小ハンドルがありません");
    await page.mouse.move(scaleDrag.from.x, scaleDrag.from.y);
    await page.mouse.down();
    await page.mouse.move(scaleDrag.to.x, scaleDrag.to.y, { steps: 8 });
    await page.mouse.up();
    const scaled = await page.locator("#character").evaluate(node => ({ translate: node.style.translate, scale: node.style.scale }));
    if (!scaled.scale) throw new Error(`Moveableの拡大縮小が反映されませんでした: ${JSON.stringify(scaled)}`);
    await page.locator("#layoutEditorSave").click();
    const saved = await page.evaluate(() => window.__readPlacements());
    if (!saved.characters?.mugi?.stream?.translate || !saved.characters?.mugi?.stream?.scale) {
      throw new Error("キャラごと・表示形式ごとの編集値が保存されませんでした");
    }
    await page.reload();
    await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 20000 });
    const restored = await page.locator("#character").evaluate(node => ({ translate: node.style.translate, scale: node.style.scale }));
    if (restored.translate !== saved.characters.mugi.stream.translate || restored.scale !== saved.characters.mugi.stream.scale) {
      throw new Error("保存した配置が再読み込み後に復元されませんでした");
    }
    // 「閉じる」は編集を取り消す操作ではない。保存ボタンを押さなくても変更を記憶する。
    await page.mouse.move(220, 7);
    await page.waitForFunction(() => document.getElementById("topWindowMenu").getBoundingClientRect().top > -1);
    await page.locator("#layoutEditToggle").click();
    await page.waitForSelector("#layoutEditorToolbar:not([hidden])");
    const closedEdited = await page.locator("#character").evaluate(node => {
      node.style.translate = "32px 24px";
      window.dispatchEvent(new Event("resize"));
      return node.style.translate;
    });
    await page.locator("#layoutEditorClose").click();
    const closedSaved = await page.evaluate(() => window.__readPlacements());
    if (closedSaved.characters?.mugi?.stream?.translate !== closedEdited) {
      throw new Error("配置編集を閉じたときにキャラ位置を記憶できませんでした");
    }
    await page.reload();
    await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 20000 });
    const closedRestored = await page.locator("#character").evaluate(node => node.style.translate);
    if (closedRestored !== closedSaved.characters.mugi.stream.translate) {
      throw new Error(`閉じる時に記憶したキャラ位置を再起動相当で復元できませんでした: ${JSON.stringify({ closedRestored, closedSaved })}`);
    }
    // 同じ表示形式でも別キャラへ位置・倍率を漏らさず、戻したキャラでは元の値を復元する。
    await page.evaluate(() => { document.getElementById("character").dataset.characterId = "rin"; });
    await page.waitForTimeout(50);
    const rinInitial = await page.locator("#character").evaluate(node => ({ translate: node.style.translate, scale: node.style.scale }));
    if (rinInitial.translate || rinInitial.scale) throw new Error(`別キャラへ配置が漏れました: ${JSON.stringify(rinInitial)}`);
    await page.evaluate(() => { document.getElementById("character").dataset.characterId = "mugi"; });
    await page.waitForTimeout(50);
    const mugiRestored = await page.locator("#character").evaluate(node => ({ translate: node.style.translate, scale: node.style.scale }));
    if (mugiRestored.translate !== closedSaved.characters.mugi.stream.translate
        || mugiRestored.scale !== closedSaved.characters.mugi.stream.scale) {
      throw new Error(`キャラを戻したときに固有配置を復元できませんでした: ${JSON.stringify(mugiRestored)}`);
    }
    // 旧版や極端なドラッグで壊れた値が残っても、縮小しすぎず掴み直せる範囲へ戻す。
    const constrained = await page.locator("#character").evaluate(node => {
      node.style.translate = "-9999px -9999px";
      node.style.scale = "0.01 0.01";
      window.dispatchEvent(new Event("resize"));
      const visible = [...node.querySelectorAll(".character-view > canvas, .character-view > img")]
        .find(candidate => getComputedStyle(candidate).display !== "none" && candidate.getBoundingClientRect().width > 1);
      const rect = (visible || node).getBoundingClientRect();
      const history = document.getElementById("historyLog")?.getBoundingClientRect();
      return {
        translate: node.style.translate,
        scale: node.style.scale,
        rect: { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom },
        historyLeft: history?.left,
        viewport: { width: innerWidth, height: innerHeight },
      };
    });
    const minScale = Math.min(...constrained.scale.split(/\s+/).map(Number));
    const centerX = (constrained.rect.left + constrained.rect.right) / 2;
    const centerY = (constrained.rect.top + constrained.rect.bottom) / 2;
    const visible = centerX >= 48 && centerX <= constrained.historyLeft - 48
      && centerY >= 48 && centerY <= constrained.viewport.height - 48;
    if (minScale < 0.25 || !visible) {
      throw new Error(`キャラが縮小または移動で画面外へ消えました: ${JSON.stringify(constrained)}`);
    }
    await page.mouse.move(220, 7);
    await page.waitForFunction(() => document.getElementById("topWindowMenu").getBoundingClientRect().top > -1);
    await page.locator("#layoutEditToggle").click();
    await page.locator("#layoutEditorReset").click();
    const reset = await page.evaluate(() => ({
      translate: document.getElementById("character")?.style.translate || "",
      scale: document.getElementById("character")?.style.scale || "",
      saved: window.__readPlacements(),
    }));
    if (reset.translate || reset.scale || reset.saved.stream) throw new Error("既定配置へ戻せませんでした");
    await page.screenshot({ path: join(output, "stream-editor.png"), omitBackground: true });
    const restoredLayouts = [];
    for (const [layout, [width, height]] of Object.entries(MATRIX_LAYOUTS)) {
      const layoutPage = await openScene({ characterId: "mugi", layout, width, height, forcePng: true });
      try {
        await layoutPage.evaluate(() => {
          localStorage.removeItem(window.__placementsKey);
          localStorage.removeItem("picoagent.layoutEditor.v1");
          localStorage.removeItem("picoagent.layoutEditor.migrated");
        });
        await layoutPage.reload();
        await layoutPage.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 20000 });
        const edited = await layoutPage.evaluate(() => {
          document.getElementById("layoutEditToggle")?.click();
          const node = document.getElementById("character");
          node.style.translate = "18px 12px";
          node.style.scale = "0.9 0.9";
          window.dispatchEvent(new Event("resize"));
          const value = { translate: node.style.translate, scale: node.style.scale };
          document.getElementById("layoutEditorClose")?.click();
          return value;
        });
        const savedLayout = await layoutPage.evaluate(id => window.__readPlacements().characters?.mugi?.[id], layout);
        if (savedLayout?.translate !== edited.translate || savedLayout?.scale !== edited.scale) {
          throw new Error(`${layout}: 閉じる時にキャラ位置・倍率を記憶できませんでした`);
        }
        await layoutPage.reload();
        await layoutPage.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 20000 });
        const restoredLayout = await layoutPage.locator("#character").evaluate(node => ({ translate: node.style.translate, scale: node.style.scale }));
        if (restoredLayout.translate !== savedLayout.translate || restoredLayout.scale !== savedLayout.scale) {
          throw new Error(`${layout}: 再読み込み後にキャラ位置・倍率を復元できませんでした`);
        }
        restoredLayouts.push(layout);
      } finally {
        await layoutPage.close();
      }
    }
    await writeFile(join(output, "persistence.json"), `${JSON.stringify({ restoredLayouts }, null, 2)}\n`);
    return output;
  } finally {
    await page.close();
  }
}

async function validateInteractionMatrix() {
  const output = join(ROOT, "temp", "layout-interaction-matrix");
  mkdirSync(output, { recursive: true });
  const selectedCharacterIds = process.env.SHOOT_CHARACTER_IDS
    ? new Set(process.env.SHOOT_CHARACTER_IDS.split(",").map(value => value.trim()).filter(Boolean))
    : null;
  const selectedLayouts = process.env.SHOOT_LAYOUT_IDS
    ? new Set(process.env.SHOOT_LAYOUT_IDS.split(",").map(value => value.trim()).filter(Boolean))
    : null;
  let characters = [{ id: "mugi", forcePng: false, meshRig: true }];
  if (LIVE2D_REQUIRED.every(path => existsSync(join(ROOT, path)))) {
    characters.push({ id: LIVE2D_CHARACTER, forcePng: false });
  }
  if (selectedCharacterIds) {
    characters = characters.filter(character => selectedCharacterIds.has(character.id));
  }
  if (!characters.length) throw new Error("操作検査の対象キャラクターがありません");
  const results = [];
  for (const target of characters) {
    for (const [layout, [width, height]] of Object.entries(MATRIX_LAYOUTS)) {
      if (selectedLayouts && !selectedLayouts.has(layout)) continue;
      const page = await openScene({
        characterId: target.id, layout, width, height, forcePng: target.forcePng,
        initDelayMs: 350, waitForReady: false,
      });
      try {
        const bootVisible = await page.evaluate(() => document.getElementById("petApp")?.classList.contains("is-booting"));
        if (!bootVisible) throw new Error(`${target.id}/${layout}: 起動画面が表示されません`);
        try {
          await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 40000 });
        } catch (error) {
          const bootState = await page.evaluate(() => ({
            classes: document.getElementById("petApp")?.className,
            status: document.getElementById("bootStatus")?.textContent,
            renderKind: document.getElementById("character")?.dataset.renderKind,
            meshRigActive: typeof meshRigActive === "undefined" ? null : meshRigActive,
            meshRigLoaded: typeof meshRigController === "undefined" ? null : Boolean(meshRigController),
          }));
          throw new Error(`${target.id}/${layout}: 起動完了待ちがタイムアウトしました ${JSON.stringify(bootState)}`, { cause: error });
        }
        if (target.meshRig && (layout === "chat" || layout === "bar")) {
          // チャット・バーは仕様上メッシュリグを描画しない（meshRigCanRender）。準備済みで止まっていることを待つ。
          await page.waitForFunction(
            () => typeof meshRigController !== "undefined" && meshRigController && !meshRigActive,
            null,
            { timeout: 40000 },
          );
        } else if (target.meshRig) {
          try {
            await page.waitForFunction(() => typeof meshRigActive !== "undefined" && meshRigActive, null, { timeout: 40000 });
          } catch (error) {
            // 何が欠けて有効にならなかったのかを残す。準備失敗で差分絵へ退避した場合は状態欄に理由が出る。
            const rigState = await page.evaluate(() => ({
              layout: document.getElementById("petApp")?.dataset.layout,
              renderKind: document.getElementById("character")?.dataset.renderKind,
              variant: document.getElementById("character")?.dataset.variant,
              status: document.getElementById("status")?.textContent,
              meshRigLoaded: typeof meshRigController === "undefined" ? null : Boolean(meshRigController),
              meshRigConfig: typeof activeMeshRigConfig === "undefined" ? null : activeMeshRigConfig,
              characterRoot: typeof characterRoot === "undefined" ? null : characterRoot,
              hidden: document.hidden,
              napping: typeof napping === "undefined" ? null : napping,
            }));
            throw new Error(`${target.id}/${layout}: メッシュリグ有効化待ちがタイムアウトしました ${JSON.stringify(rigState)}`, { cause: error });
          }
        } else if (!target.forcePng) {
          await page.waitForFunction(() => typeof live2dActive !== "undefined" && live2dActive, null, { timeout: 40000 });
        }
        // むぎは局所バネを持たず、待機リグはタイマー駆動。Live2DはPIXI tickerが担当するため、
        // どちらもrenderer側の常時rAFは不要。
        const loopExpected = false;
        if (loopExpected) {
          await page.waitForFunction(() => animationLoop?.pending() === true, null, { timeout: 1000 });
        } else {
          // 状態変更直後の単発wakeは許可し、空ループが1秒以内に予約0へ収束することを見る。
          await page.waitForFunction(() => animationLoop?.pending() !== true, null, { timeout: 1000 });
        }
        const loopPending = await page.evaluate(() => animationLoop?.pending() === true);
        if (loopPending !== loopExpected) {
          throw new Error(`${target.id}/${layout}: アニメーションループの休止状態が不正です ${JSON.stringify({ loopPending, loopExpected })}`);
        }
        const dragCallsBefore = await page.evaluate(() => window.__startDraggingCalls || 0);
        await page.evaluate(() => document.getElementById("character")?.addEventListener("pointerdown", event => {
          window.__characterPointerDebug = { button: event.button, pointerType: event.pointerType, target: event.target?.id || event.target?.tagName };
        }, { once: true }));
        await page.locator("#character").dispatchEvent("pointerdown", { button: 0, pointerType: "mouse" });
        const dragCallsAfter = await page.evaluate(() => window.__startDraggingCalls || 0);
        if (dragCallsAfter - dragCallsBefore !== 1) {
          const dragState = await page.evaluate(() => ({
            editing: document.getElementById("petApp")?.classList.contains("is-layout-editing"),
            characterDisplay: getComputedStyle(document.getElementById("character")).display,
            characterPointerEvents: getComputedStyle(document.getElementById("character")).pointerEvents,
            bridge: typeof window.picoagent?.startWindowDrag,
            event: window.__characterPointerDebug,
          }));
          throw new Error(`${target.id}/${layout}: キャラ画像のドラッグ開始が一重に渡りません ${JSON.stringify({ dragCallsBefore, dragCallsAfter, dragState })}`);
        }
        await page.evaluate(() => {
          if (typeof openComposer === "function") openComposer();
          const composer = document.getElementById("composer");
          if (composer) composer.hidden = false;
        });
        await page.waitForTimeout(150);

        const resized = { width: Math.max(320, Math.round(width * 0.78)), height: Math.max(240, Math.round(height * 0.78)) };
        await page.setViewportSize(resized);
        await page.waitForTimeout(100);
        const state = await page.evaluate(() => {
          const visibleRect = element => {
            if (!element || element.hidden || getComputedStyle(element).display === "none") return null;
            const rect = element.getBoundingClientRect();
            return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
          };
          return {
            beta: visibleRect(document.querySelector(".beta-badge")),
            betaPointerEvents: getComputedStyle(document.querySelector(".beta-badge")).pointerEvents,
            bubble: visibleRect(document.getElementById("bubble")),
            character: visibleRect(document.getElementById("character")),
            characterDisplay: getComputedStyle(document.getElementById("character")).display,
            composer: visibleRect(document.getElementById("composer")),
            input: visibleRect(document.getElementById("input")),
            buttons: [...document.querySelectorAll("#composer button")]
              .map(visibleRect).filter(rect => rect && rect.width > 0 && rect.height > 0),
          };
        });
        const viewportRect = { width: resized.width, height: resized.height };
        const intersects = rect => rect && rect.width > 0 && rect.height > 0
          && rect.x < viewportRect.width && rect.y < viewportRect.height
          && rect.x + rect.width > 0 && rect.y + rect.height > 0;
        // 押す要素は「少し掛かっている」では足りない。丸ごと画面内に無いと操作できないので、
        // 入力欄とボタンだけは完全に収まっていることを条件にする（絵は掛かっていればよい）。
        const inside = rect => rect && rect.width > 0 && rect.height > 0
          && rect.x >= -1 && rect.y >= -1
          && rect.x + rect.width <= viewportRect.width + 1
          && rect.y + rect.height <= viewportRect.height + 1;
        if (!intersects(state.beta) || state.betaPointerEvents !== "none") {
          throw new Error(`${target.id}/${layout}: ALPHA表示が画面外または操作を遮っています ${JSON.stringify(state.beta)}`);
        }
        // バーは高さが足りないとき CSS がキャラのアイコンを意図して畳む（SPEC: 最後に畳むのはキャラクターアイコン）。
        // 畳まれた（display:none）場合だけは合格にし、画面外へはみ出して消えた場合は失敗のままにする。
        const characterFolded = layout === "bar" && state.characterDisplay === "none";
        if (!intersects(state.bubble) || !(intersects(state.character) || characterFolded) || !intersects(state.composer)) {
          throw new Error(`${target.id}/${layout}: 縮小後に主要UIが画面から消えました ${JSON.stringify(state)}`);
        }
        if (!inside(state.input)) {
          throw new Error(`${target.id}/${layout}: 縮小後に入力欄が画面からはみ出しました ${JSON.stringify(state.input)}`);
        }
        if (!state.buttons.length || state.buttons.some(button => !inside(button))) {
          throw new Error(`${target.id}/${layout}: 入力操作ボタンが画面外です ${JSON.stringify(state.buttons)}`);
        }
        const file = `${target.id}-${layout}.png`;
        await page.screenshot({ path: join(output, file), omitBackground: true });

        // 既定寸法の78%では、利用者が実際に縮められる下限（tauri.confのminWidth/minHeight）
        // まで届かない。その下限でも入力欄と送信ボタンが窓の中に残ることを確かめる。
        const minimum = { width: 200, height: 240 };
        await page.setViewportSize(minimum);
        await page.waitForTimeout(150);
        const atMinimum = await page.evaluate(() => {
          const rect = id => {
            const element = document.getElementById(id);
            if (!element || getComputedStyle(element).display === "none" || getComputedStyle(element).visibility === "hidden") return null;
            const box = element.getBoundingClientRect();
            return box.width > 0 || box.height > 0 ? { id, x: box.x, y: box.y, width: box.width, height: box.height } : null;
          };
          return ["input", "sendButton", "micButton", "status", "windowClose", "windowMinimize"].map(rect).filter(Boolean);
        });
        const outsideAtMinimum = atMinimum.filter(box => !(box.x >= -1 && box.y >= -1
          && box.x + box.width <= minimum.width + 1 && box.y + box.height <= minimum.height + 1));
        if (outsideAtMinimum.length) {
          throw new Error(`${target.id}/${layout}: 最小寸法${minimum.width}x${minimum.height}で操作要素が画面外です ${JSON.stringify(outsideAtMinimum)}`);
        }
        await page.setViewportSize(resized);
        results.push({ characterId: target.id, layout, bootVisible, loopPending, resized, minimum, state, atMinimum, file });
        console.log(`  操作検査: ${target.id}/${layout}`);
      } finally {
        await page.close();
      }
    }
  }
  const formalPage = await openScene({ characterId: "mugi", betaBuild: false });
  try {
    const betaVisible = await formalPage.locator(".beta-badge").isVisible();
    if (betaVisible) throw new Error("通常／正式ビルド相当でもALPHA表示が残っています");
    console.log("  ビルド種別検査: beta-fullなしではALPHA表示を隠す");
  } finally {
    await formalPage.close();
  }
  const reducedPage = await openScene({ characterId: "mugi", reducedMotion: true });
  try {
    try {
      await reducedPage.waitForFunction(
        () => typeof meshRigActive !== "undefined" && meshRigActive && rigActive,
        null,
        { timeout: 20000 },
      );
    } catch (error) {
      const state = await reducedPage.evaluate(() => ({
        meshRigActive: typeof meshRigActive === "undefined" ? null : meshRigActive,
        rigActive: typeof rigActive === "undefined" ? null : rigActive,
        meshRigLoaded: typeof meshRigController === "undefined" ? null : Boolean(meshRigController),
        renderKind: document.getElementById("character")?.dataset.renderKind,
        spriteDisplay: getComputedStyle(document.getElementById("petSprite")).display,
        canvasDisplay: getComputedStyle(document.getElementById("meshRigCanvas")).display,
      }));
      throw new Error(`reduced-motionでむぎのメッシュリグが開始しません ${JSON.stringify(state)}`, { cause: error });
    }
    const rigTimingContract = await reducedPage.evaluate(() => ({
      thresholds: {
        open: RIG_MOUTH_OPEN_THRESHOLD,
        close: RIG_MOUTH_CLOSE_THRESHOLD,
      },
      staysClosedBelowOpenThreshold: quantizeRigMouthOpen(0.33, false),
      opensAboveOpenThreshold: quantizeRigMouthOpen(0.35, false),
      staysOpenAboveCloseThreshold: quantizeRigMouthOpen(0.23, true),
      closesBelowCloseThreshold: quantizeRigMouthOpen(0.21, true),
      normal: { ...rigBlinkTimings(false) },
      sleepy: { ...rigBlinkTimings(true) },
    }));
    const expectedRigTimingContract = {
      thresholds: { open: 0.34, close: 0.22 },
      staysClosedBelowOpenThreshold: false,
      opensAboveOpenThreshold: true,
      staysOpenAboveCloseThreshold: true,
      closesBelowCloseThreshold: false,
      normal: { closing: 180, closed: 220, opening: 250 },
      sleepy: { closing: 220, closed: 360, opening: 320 },
    };
    if (JSON.stringify(rigTimingContract) !== JSON.stringify(expectedRigTimingContract)) {
      throw new Error(`リグの口ヒステリシスまたはまばたき配分が不正です ${JSON.stringify(rigTimingContract)}`);
    }
    const blinkApplied = await reducedPage.evaluate(async () => {
      clearTimeout(rigBlinkTimer);
      clearTimeout(rigBlinkStepTimer);
      rigActive = false;
      meshRigController.setEyeOpen(1, 1);
      meshRigController.setBlink(0, 100);
      const closingTween = { ...meshRigController.blinkTween };
      await new Promise(resolve => setTimeout(resolve, 120));
      const closed = meshRigController.eyeOpen;
      meshRigController.setBlink(1, 100);
      const openingTween = { ...meshRigController.blinkTween };
      await new Promise(resolve => setTimeout(resolve, 120));
      const opened = meshRigController.eyeOpen;
      return { closingTween, closed, openingTween, opened };
    });
    if (!(blinkApplied.closingTween.fromL === 1 && blinkApplied.closingTween.to === 0
      && blinkApplied.closingTween.duration === 100 && blinkApplied.closed < 0.05
      && blinkApplied.openingTween.fromL < 0.05 && blinkApplied.openingTween.to === 1
      && blinkApplied.openingTween.duration === 100 && blinkApplied.opened > 0.95)) {
      throw new Error(`メッシュリグのまばたき補間が不正です ${JSON.stringify(blinkApplied)}`);
    }
    await reducedPage.waitForTimeout(80);
    if (await reducedPage.evaluate(() => animationLoop?.pending() === true)) {
      throw new Error("アニメーションを減らす設定でも空のフレームループが動いています");
    }
    const firstTalkingFrame = await reducedPage.evaluate(() => {
      // ミュート時の口を調べる間にランダムなまばたきでsrcが変わらないよう隔離する。
      clearTimeout(rigBlinkTimer);
      clearTimeout(rigBlinkStepTimer);
      rigBlinkPhase = null;
      rigMouthOpen = false;
      rigRefresh();
      setAnimation("talking", true);
      return meshRigController.mouth;
    });
    await reducedPage.waitForTimeout(300);
    const mutedTalkingState = await reducedPage.evaluate(initialMouth => ({
      mouthOpen: rigMouthOpen,
      mouth: meshRigController.mouth,
      changed: Math.abs(meshRigController.mouth - initialMouth) > 0.001,
    }), firstTalkingFrame);
    if (mutedTalkingState.mouthOpen || mutedTalkingState.changed || mutedTalkingState.mouth > 0.001) {
      throw new Error(`ミュート中にメッシュリグの口が動きました ${JSON.stringify(mutedTalkingState)}`);
    }
    console.log("  reduced-motion検査: まばたき・ミュート口パクOK");
  } finally {
    await reducedPage.close();
  }
  const gazeOffPage = await openScene({ characterId: "mugi", width: 900, height: 600 });
  try {
    await gazeOffPage.waitForFunction(
      () => typeof meshRigActive !== "undefined" && meshRigActive && rigActive,
      null,
      { timeout: 20000 },
    );
    const rect = await gazeOffPage.locator("#character").boundingBox();
    await gazeOffPage.mouse.move(rect.x + rect.width * 0.85, rect.y + rect.height * 0.38);
    const defaultDirection = await gazeOffPage.evaluate(() => meshRigController.gazeTargetX);
    if (defaultDirection !== 0) throw new Error(`既定OFFなのに視線が追従しました: ${defaultDirection}`);
    console.log("  視線設定検査: 既定OFFで正面を維持");
  } finally {
    await gazeOffPage.close();
  }
  const gazePage = await openScene({
    characterId: "mugi", width: 900, height: 600, gazeTrackingEnabled: true,
  });
  try {
    await gazePage.waitForFunction(
      () => typeof meshRigActive !== "undefined" && meshRigActive && rigActive,
      null,
      { timeout: 20000 },
    );
    const rect = await gazePage.locator("#character").boundingBox();
    const batch = await gazePage.evaluate(async ({ x, y }) => {
      let reads = 0;
      const original = character.getBoundingClientRect.bind(character);
      character.getBoundingClientRect = () => {
        reads += 1;
        return original();
      };
      for (let index = 0; index < 200; index += 1) {
        document.dispatchEvent(new MouseEvent("mousemove", {
          clientX: x + index / 100,
          clientY: y,
        }));
      }
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      delete character.getBoundingClientRect;
      return { reads, direction: meshRigController.gazeTargetX };
    }, { x: rect.x + rect.width * 0.85, y: rect.y + rect.height * 0.38 });
    if (batch.reads !== 1 || batch.direction < 0.38) {
      throw new Error(`高頻度mousemoveが1フレーム1回へ合流していません: ${JSON.stringify(batch)}`);
    }
    const loopTransitions = await gazePage.evaluate(async () => {
      const waitFrames = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      applyLayout("chat");
      await waitFrames();
      const chatPending = animationLoop.pending();
      applyLayout("pet");
      await waitFrames();
      const petPending = animationLoop.pending();
      fallAsleep();
      await waitFrames();
      // 寝姿（boot.sleeping）を持つキャラだけが居眠り表示へ入る（SPEC: 寝姿は居眠り表示にだけ使う）。
      const sleep = { napping, pending: animationLoop.pending(), hasSleepPose: Boolean(characterManifest?.boot?.sleeping) };
      markActivity();
      await waitFrames();
      const wake = { napping, pending: animationLoop.pending(), rigActive };
      let hidden = false;
      Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
      hidden = true;
      document.dispatchEvent(new Event("visibilitychange"));
      const hiddenPending = animationLoop.pending();
      hidden = false;
      document.dispatchEvent(new Event("visibilitychange"));
      const visiblePending = animationLoop.pending();
      delete document.hidden;
      return { chatPending, petPending, sleep, wake, hiddenPending, visiblePending };
    });
    if (loopTransitions.chatPending || loopTransitions.petPending
      || loopTransitions.sleep.napping !== loopTransitions.sleep.hasSleepPose || loopTransitions.sleep.pending
      || loopTransitions.wake.napping || loopTransitions.wake.pending || !loopTransitions.wake.rigActive
      || loopTransitions.hiddenPending || !loopTransitions.visiblePending) {
      throw new Error(`メッシュリグで状態変化後の再開・休止が不正です（寝姿があるときだけ居眠りし、休止中は予約0）: ${JSON.stringify(loopTransitions)}`);
    }
    await gazePage.emulateMedia({ reducedMotion: "reduce" });
    await gazePage.waitForFunction(() => reduceMotion && !animationLoop.pending());
    await gazePage.emulateMedia({ reducedMotion: "no-preference" });
    await gazePage.waitForFunction(() => !reduceMotion);
    await gazePage.waitForTimeout(80);
    if (await gazePage.evaluate(() => animationLoop.pending())) {
      throw new Error("局所バネのない待機リグで空のフレームループが再開しています");
    }
    await gazePage.evaluate(() => setAnimation("thinking", true));
    await gazePage.waitForFunction(() => animationLoop.pending());
    await gazePage.evaluate(() => setAnimation("idle", true));
    await gazePage.waitForTimeout(80);
    if (await gazePage.evaluate(() => animationLoop.pending())) {
      throw new Error("フレームモーションから待機リグへ戻った後もループが残っています");
    }
    await gazePage.mouse.move(450, 590);
    await gazePage.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const farDirection = await gazePage.evaluate(() => meshRigController.gazeTargetX);
    if (farDirection !== 0) throw new Error(`画面下の遠いカーソルへ視線が張り付いています: ${farDirection}`);
    await gazePage.mouse.move(rect.x + rect.width * 0.85, rect.y + rect.height * 0.38);
    await gazePage.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const nearDirection = await gazePage.evaluate(() => meshRigController.gazeTargetX);
    if (nearDirection < 0.38) throw new Error(`顔付近の右カーソルへ視線が追従しません: ${nearDirection}`);
    await gazePage.waitForTimeout(800);
    const resetDirection = await gazePage.evaluate(() => meshRigController.gazeTargetX);
    if (resetDirection !== 0) throw new Error(`視線が700ms後に正面へ戻りません: ${resetDirection}`);
    console.log("  描画ループ検査: 形式・居眠り・表示・アニメ削減の休止と再開OK");
    console.log("  視線設定検査: 200イベントを1フレームへ合流し、顔周辺だけ追従、700ms後に復帰");
  } finally {
    await gazePage.close();
  }
  const springPage = await openScene({ characterId: "sena", forcePng: true, width: 900, height: 600 });
  try {
    await springPage.waitForFunction(
      () => typeof meshRigActive !== "undefined" && meshRigActive && meshRigController?.secondaryHair,
      null,
      { timeout: 20000 },
    );
    await springPage.waitForTimeout(80);
    if (await springPage.evaluate(() => animationLoop.pending())) {
      throw new Error("メッシュリグの局所揺れがrenderer側の重複ループを維持しています");
    }
    await springPage.evaluate(() => applyLayout("chat"));
    await springPage.waitForTimeout(80);
    const chatState = await springPage.evaluate(() => ({ meshRigActive, pending: animationLoop.pending() }));
    if (chatState.meshRigActive || chatState.pending) {
      throw new Error(`チャット形式で非表示のメッシュリグが動いています ${JSON.stringify(chatState)}`);
    }
    await springPage.evaluate(() => applyLayout("pet"));
    await springPage.waitForFunction(() => meshRigActive);
    await springPage.waitForTimeout(80);
    if (await springPage.evaluate(() => animationLoop.pending())) {
      throw new Error("ペット形式へ戻した後にrenderer側の重複ループが動いています");
    }
    console.log("  局所揺れ検査: メッシュ側だけで動かし、非表示時は休止OK");
  } finally {
    await springPage.close();
  }
  const resizePage = await openScene({ characterId: "mugi", forcePng: true, width: 900, height: 600 });
  try {
    const beforeResize = await resizePage.evaluate(() => {
      applyCharacterOffset(137, -43, { clamp: false });
      return { ...characterOffset };
    });
    await resizePage.setViewportSize({ width: 1800, height: 1000 });
    await resizePage.setViewportSize({ width: 900, height: 600 });
    const afterResize = await resizePage.evaluate(() => ({ ...characterOffset }));
    if (afterResize.x !== beforeResize.x || afterResize.y !== beforeResize.y) {
      throw new Error(`最大化・復元相当のサイズ変更でキャラ位置が変わりました: ${JSON.stringify({ beforeResize, afterResize })}`);
    }
    console.log("  サイズ変更検査: キャラ位置維持OK");
  } finally {
    await resizePage.close();
  }
  const staleOffsetPage = await openScene({ characterId: "sena", forcePng: true, layout: "stream" });
  try {
    const offsets = await staleOffsetPage.evaluate(() => {
      characterFraming = {};
      applyCharacterFraming();
      const expected = { ...characterOffset };
      characterFraming = { hiyori: { stream: { preset: "custom", x: -386, y: 159, zoom: 1.55 } } };
      applyCharacterOffset(-386, 159, { clamp: false });
      applyCharacterFraming();
      return { expected, actual: { ...characterOffset } };
    });
    if (offsets.actual.x !== offsets.expected.x || offsets.actual.y !== offsets.expected.y) {
      throw new Error(`別キャラの旧位置を引き継いでいます: ${JSON.stringify(offsets)}`);
    }
    console.log("  キャラ切替検査: 別キャラの旧位置を引き継がない");
  } finally {
    await staleOffsetPage.close();
  }
  await writeFile(join(output, "results.json"), `${JSON.stringify({ generatedAt: new Date().toISOString(), results }, null, 2)}\n`);
  return { output, results };
}

// 常時表示のフキダシにある `backdrop-filter: blur(14px) saturate(1.08)` の再合成負荷を測る。
// 立ち絵を動かし続けた状態で、重なり有無・背後の描き替え負荷有無を掛けた12条件を測る。
// ぼかし有無は同じページ内で両方取り、暖機分が片側へ寄らないよう試行ごとに順序を入れ替える。
const BLUR_LAYOUTS = { pet: [900, 600], mascot: [360, 560], stream: [1920, 1080] };
const BLUR_TRIALS = 3;
const BLUR_SAMPLE_MS = 3000;

function summarizeFrames(intervals) {
  const sorted = [...intervals].sort((a, b) => a - b);
  const at = ratio => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * ratio))] || 0;
  return {
    frames: sorted.length,
    medianMs: Number(at(0.5).toFixed(2)),
    p95Ms: Number(at(0.95).toFixed(2)),
    slowFrames: sorted.filter(value => value > 20).length,
  };
}

async function measureBubbleFrames(page, { blur, overlap, load }) {
  return page.evaluate(async ({ blur, overlap, load, sampleMs }) => {
    const bubble = document.querySelector(".bubble");
    const character = document.querySelector(".character");
    if (!bubble || !character) throw new Error("フキダシまたは立ち絵が見つかりません");
    const style = document.createElement("style");
    style.id = "bubble-blur-probe";
    // 比較条件はぼかしの有無だけにする。位置・幅・文字組みには触れない。
    style.textContent = blur ? "" : ".bubble { backdrop-filter:none !important; }";
    document.head.append(style);
    const previousLeft = bubble.style.left;
    const previousTop = bubble.style.top;
    if (overlap) {
      // 立ち絵の真上へ重ねた最悪条件。保存済み配置は書き換えず、この場限りの検査用。
      const rect = character.getBoundingClientRect();
      bubble.style.left = `${Math.round(rect.left)}px`;
      bubble.style.top = `${Math.round(rect.top)}px`;
    }
    // 立ち絵が動き続ける状態を作る。描画ループの休止判断とは独立に再合成させる。
    const motion = character.animate(
      [{ transform: "translateX(-14px)" }, { transform: "translateX(14px)" }],
      { duration: 900, iterations: Infinity, direction: "alternate", easing: "ease-in-out" },
    );
    // 負荷条件では、フキダシの背後だけを動く矩形で埋めてフレーム予算を使い切らせる。
    // ぼかしに実費用があるなら、ここで期限落ちの差として現れる。
    const stress = document.createElement("div");
    const stressAnimations = [];
    if (load) {
      stress.id = "bubble-blur-stress";
      stress.style.cssText = "position:absolute;inset:0;z-index:2;pointer-events:none;overflow:hidden;";
      const bubbleRect = bubble.getBoundingClientRect();
      for (let index = 0; index < 80; index += 1) {
        const cell = document.createElement("div");
        const size = 40 + (index % 7) * 12;
        cell.style.cssText = `position:absolute;width:${size}px;height:${size}px;border-radius:8px;`
          + `background:hsl(${index * 4} 70% 60%);`
          + `left:${Math.round(bubbleRect.left + (index * 37) % Math.max(1, bubbleRect.width))}px;`
          + `top:${Math.round(bubbleRect.top + (index * 53) % Math.max(1, bubbleRect.height))}px;`;
        stress.append(cell);
        stressAnimations.push(cell.animate(
          [{ transform: "translate3d(0,0,0) rotate(0deg)" }, { transform: "translate3d(60px,40px,0) rotate(180deg)" }],
          { duration: 700 + index, iterations: Infinity, direction: "alternate" },
        ));
      }
      (bubble.parentElement || document.body).insertBefore(stress, bubble);
    }
    const intervals = [];
    await new Promise(resolve => {
      let previous = performance.now();
      const deadline = previous + sampleMs;
      const step = now => {
        intervals.push(now - previous);
        previous = now;
        if (now < deadline) requestAnimationFrame(step);
        else resolve();
      };
      requestAnimationFrame(step);
    });
    motion.cancel();
    for (const animation of stressAnimations) animation.cancel();
    stress.remove();
    style.remove();
    bubble.style.left = previousLeft;
    bubble.style.top = previousTop;
    // 最初の1コマは計測開始の基準なので落とす。
    return intervals.slice(1);
  }, { blur, overlap, load, sampleMs: BLUR_SAMPLE_MS });
}

async function measureBubbleBlur() {
  const output = join(ROOT, "temp", "bubble-blur");
  mkdirSync(output, { recursive: true });
  const results = [];
  for (const [layout, [width, height]] of Object.entries(BLUR_LAYOUTS)) {
    for (const [overlap, load] of [[false, false], [true, false], [false, true], [true, true]]) {
      const samples = { blur: [], plain: [] };
      for (let trial = 0; trial < BLUR_TRIALS; trial += 1) {
        const page = await openScene({ characterId: "mugi", layout, width, height, forcePng: true });
        try {
          // 同じページで両方を測り、起動ゆらぎを条件差として拾わないようにする。
          // 先に測った側が暖機分を負うため、試行ごとに順序を入れ替える。
          const order = trial % 2 === 0 ? [true, false] : [false, true];
          for (const blur of order) {
            const summary = summarizeFrames(await measureBubbleFrames(page, { blur, overlap, load }));
            samples[blur ? "blur" : "plain"].push(summary);
          }
        } finally {
          await page.close();
        }
      }
      const median = key => {
        const values = samples[key].map(item => item.medianMs).sort((a, b) => a - b);
        return values[Math.floor(values.length / 2)];
      };
      const worst = key => Math.max(...samples[key].map(item => item.p95Ms));
      const slow = key => samples[key].reduce((sum, item) => sum + item.slowFrames, 0);
      const entry = {
        layout,
        overlap,
        load,
        blurMedianMs: median("blur"),
        plainMedianMs: median("plain"),
        differenceMs: Number((median("blur") - median("plain")).toFixed(2)),
        blurP95Ms: worst("blur"),
        plainP95Ms: worst("plain"),
        blurSlowFrames: slow("blur"),
        plainSlowFrames: slow("plain"),
        trials: samples,
      };
      results.push(entry);
      console.log(`  ぼかし計測: ${layout} 重なり${overlap ? "あり" : "なし"} 負荷${load ? "あり" : "なし"} 中央値 ぼかし${entry.blurMedianMs}ms / なし${entry.plainMedianMs}ms、p95 ${entry.blurP95Ms}ms / ${entry.plainP95Ms}ms、20ms超 ${entry.blurSlowFrames}件 / ${entry.plainSlowFrames}件`);
    }
  }
  writeFileSync(join(output, "metrics.json"), JSON.stringify({ sampleMs: BLUR_SAMPLE_MS, trials: BLUR_TRIALS, results }, null, 2));
  return { output, results };
}

async function validateLive2dMatrix() {
  const output = join(ROOT, "temp", "live2d-layout-matrix");
  mkdirSync(output, { recursive: true });
  const index = JSON.parse(await readFile(join(ROOT, "characters", "index.json"), "utf8"));
  const targets = [];
  const skipped = [];
  const selectedLive2d = process.env.SHOOT_LIVE2D_IDS
    ? new Set(process.env.SHOOT_LIVE2D_IDS.split(",").map(value => value.trim()).filter(Boolean))
    : null;
  const selectedLayouts = process.env.SHOOT_LAYOUT_IDS
    ? new Set(process.env.SHOOT_LAYOUT_IDS.split(",").map(value => value.trim()).filter(Boolean))
    : null;
  for (const entry of index.characters || []) {
    const manifestPath = join(ROOT, "characters", entry.manifest);
    const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
    if (!manifest.live2d) continue;
    if (selectedLive2d && !selectedLive2d.has(manifest.id)) continue;
    const relativeRoot = dirname(entry.manifest);
    const candidates = manifest.live2d.models?.length
      ? manifest.live2d.models.map(model => model.path)
      : [manifest.live2d.model];
    const available = candidates.filter(path => path && existsSync(join(UI_ROOT, "assets", "characters", relativeRoot, path)));
    if (!available.length) {
      skipped.push({ characterId: manifest.id, reason: "モデル実体が未取得" });
      continue;
    }
    targets.push({ id: manifest.id, manifest });
  }
  const results = [];
  for (const target of targets) {
    for (const [layout, defaultSize] of Object.entries(MATRIX_LAYOUTS)) {
      if (selectedLayouts && !selectedLayouts.has(layout)) continue;
      const [width, height] = LAYOUT_SIZE_OVERRIDES.get(layout) || defaultSize;
      const page = await openScene({ characterId: target.id, layout, width, height });
      try {
        await page.waitForFunction(() => typeof live2dActive !== "undefined" && live2dActive, null, { timeout: 40000 });
        await page.waitForTimeout(250);
        await page.mouse.move(width / 2, height - 1);
        const farFocus = await page.evaluate(() => ({
          x: live2dModel?.internalModel?.focusController?.targetX ?? 0,
          y: live2dModel?.internalModel?.focusController?.targetY ?? 0,
        }));
        if (Math.abs(farFocus.x) > 0.01 || Math.abs(farFocus.y) > 0.01) {
          throw new Error(`${target.id}/${layout}: 画面下のカーソルでLive2D視線が中央へ戻りません ${JSON.stringify(farFocus)}`);
        }
        const info = await page.evaluate(() => {
          const canvas = document.getElementById("live2dCanvas");
          const rect = canvas.getBoundingClientRect();
          const cfg = characterManifest.live2d || {};
          const groups = live2dModel?.internalModel?.settings?.json?.Groups || [];
          const lipGroup = groups.find(group => String(group?.Name || "").toLowerCase() === "lipsync");
          const lipParam = String(cfg.lipSyncParam || lipGroup?.Ids?.[0] || "ParamMouthOpenY");
          const core = live2dModel?.internalModel?.coreModel;
          let lipIndex = -1;
          try { lipIndex = core?.getParameterIndex?.(lipParam) ?? -1; } catch { lipIndex = -1; }
          return {
            sdk: activeLive2dSdkPreference, runtimeSdk: live2dRuntimeSdk,
            sourceWidth: live2dModel?.internalModel?.originalWidth || 0,
            sourceHeight: live2dModel?.internalModel?.originalHeight || 0,
            configuredScale: Number(cfg.scale) || 1,
            configuredOffsetY: Number(cfg.offsetY) || 0,
            modelTransform: live2dModel ? {
              x: live2dModel.x, y: live2dModel.y,
              width: live2dModel.width, height: live2dModel.height,
              scaleX: live2dModel.scale.x, scaleY: live2dModel.scale.y,
            } : null,
            lipSync: cfg.lipSync !== false, lipParam, lipIndex,
            rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
          };
        });
        const intersects = info.rect.width > 0 && info.rect.height > 0
          && info.rect.x < width && info.rect.y < height
          && info.rect.x + info.rect.width > 0 && info.rect.y + info.rect.height > 0;
        if (!intersects || info.sourceWidth <= 0 || info.sourceHeight <= 0) {
          throw new Error(`${target.id}/${layout}: Live2D描画が画面内にありません ${JSON.stringify(info)}`);
        }
        if (info.lipSync && info.lipIndex < 0) {
          throw new Error(`${target.id}/${layout}: 口パラメータ ${info.lipParam} がモデルにありません`);
        }
        // canvas要素の矩形だけでは、モデル本体が透明でも検査を通ってしまう。
        // 実際のWebGL出力をPNGへ読み戻し、不透明画素があることまで確認する。
        const canvasPng = await page.locator("#live2dCanvas").screenshot({ omitBackground: true });
        const canvasContentBounds = await contentBounds(canvasPng);
        if (!canvasContentBounds || canvasContentBounds.pixels < 400) {
          throw new Error(`${target.id}/${layout}: Live2Dモデル本体の描画画素がありません ${JSON.stringify({ info, canvasContentBounds })}`);
        }
        const file = `${target.id}-${layout}.png`;
        await page.screenshot({ path: join(output, file), omitBackground: true });
        results.push({ characterId: target.id, layout, ...info, canvasContentBounds, file });
        console.log(`  Live2D検査: ${target.id}/${layout} SDK${info.runtimeSdk} ${info.lipSync ? info.lipParam : "口パクなし"}`);
      } finally {
        await page.close();
      }
    }
    if (target.manifest.live2d.models?.length) {
      for (const model of target.manifest.live2d.models) {
        const page = await openScene({ characterId: target.id, live2dSdkOverrides: { [target.id]: String(model.sdk) } });
        try {
          await page.waitForFunction(() => typeof live2dActive !== "undefined" && live2dActive, null, { timeout: 40000 });
          const loaded = await page.evaluate(() => ({ preference: activeLive2dSdkPreference, runtime: live2dRuntimeSdk }));
          if (loaded.preference !== String(model.sdk) || loaded.runtime !== String(model.sdk)) {
            throw new Error(`${target.id}: SDK${model.sdk}固定読込に失敗 ${JSON.stringify(loaded)}`);
          }
        } finally {
          await page.close();
        }
      }
    }
  }
  await writeFile(join(output, "results.json"), `${JSON.stringify({ generatedAt: new Date().toISOString(), results, skipped }, null, 2)}\n`);
  return { output, results, skipped };
}

// 起動時の挨拶ではなく、直近のやり取りが写るように1往復させる。
const submit = (page, text) => page.evaluate((value) => {
  if (typeof setInputValue === "function") setInputValue(value);
  document.getElementById("composer")?.dispatchEvent(new Event("submit", { cancelable: true }));
}, text);

// 文字送りが最後まで終わるのを待つ（途中で切れた文が写らないように）。
// 本文は Markdown として描画されるので、記号（`code` のバッククォート等）と空白は無視して比べる。
const waitForReply = (page, reply) => page.waitForFunction((full) => {
  const strip = (s) => s.replace(/[\s`*_]+/g, "");
  const el = document.getElementById("reply");
  return el && strip(el.textContent) === strip(full);
}, reply, { timeout: 20000 });

// 一時的なステータス表示（読み上げ準備中など）は消してから撮る。
const clearStatus = (page) => page.evaluate(() => {
  const status = document.getElementById("status");
  if (status) status.textContent = "";
});

async function validateThinkingIndicator() {
  const output = join(ROOT, "temp", "thinking-indicator-qa");
  mkdirSync(output, { recursive: true });
  const page = await openScene({ characterId: "mugi", layout: "stream", width: 1280, height: 720, forcePng: true, sendDelayMs: 1200 });
  try {
    await submit(page, "考え中表示の確認");
    await page.waitForSelector(".history-thinking .typing-indicator");
    const pending = await page.evaluate(() => ({
      dots: document.querySelectorAll(".history-thinking .typing-dots i").length,
      label: document.querySelector(".history-thinking .typing-indicator")?.getAttribute("aria-label") || "",
      statusSpinner: getComputedStyle(document.getElementById("status"), "::before").content,
      // 文言が無いのに丸い枠だけ浮かせない（配信形式は絶対配置の枠を持つ）。
      emptyStatusBox: !document.getElementById("status").textContent.trim()
        && getComputedStyle(document.getElementById("status")).display !== "none",
      captionLabel: getComputedStyle(document.getElementById("reply"), "::before").content,
      // 生成中は回答欄の読み上げを保留する（書き変わるたびに読み上げさせない）。
      replyBusy: document.getElementById("reply")?.getAttribute("aria-busy"),
    }));
    if (pending.replyBusy !== "true" || pending.dots !== 3 || !pending.label || pending.emptyStatusBox
        || (pending.statusSpinner && pending.statusSpinner !== "none")
        || (pending.captionLabel && pending.captionLabel !== "none")) {
      throw new Error(`考え中の3点表示が不正です: ${JSON.stringify(pending)}`);
    }
    await page.screenshot({ path: join(output, "stream-thinking.png"), omitBackground: true });
    await page.waitForFunction(() => !document.querySelector(".history-thinking"), null, { timeout: 20000 });
    const finished = await page.evaluate(() => ({
      thinkingRows: document.querySelectorAll(".history-thinking").length,
      typingDots: document.querySelectorAll(".typing-dots i").length,
      // 回答が出そろったら保留を解く（ここで1回だけ読み上げられる）。
      replyBusy: document.getElementById("reply")?.getAttribute("aria-busy"),
    }));
    if (finished.thinkingRows || finished.typingDots || finished.replyBusy) {
      throw new Error(`回答後に考え中表示が残りました: ${JSON.stringify(finished)}`);
    }
    await writeFile(join(output, "result.json"), `${JSON.stringify({ pending, finished }, null, 2)}\n`);
    return output;
  } finally {
    await page.close();
  }
}

// 実行中のタスク一覧（task-run.js）を7形式で検査する。ツールイベントはモックの購読者へ直接届け、
// 実行中の行・停止ボタン・窓内への収まり・進捗行との重複なし・完了後に閉じることを確かめる。
async function validateTaskRun() {
  const output = join(ROOT, "temp", "task-run-qa");
  mkdirSync(output, { recursive: true });
  const selectedLayouts = process.env.SHOOT_LAYOUT_IDS
    ? process.env.SHOOT_LAYOUT_IDS.split(",").map(value => value.trim()).filter(Boolean)
    : Object.keys(MATRIX_LAYOUTS);
  const expectedRows = [["計画を立てています", "completed"], ["Web検索", "completed"], ["ローカル画像・動画生成", "running"]];
  const results = [];
  for (const layout of selectedLayouts) {
    const [width, height] = MATRIX_LAYOUTS[layout];
    // 一覧の要素が文書から外れた瞬間の呼び出し元を、起動中から記録する（出ない不具合の原因を特定するため）。
    const traceScript = `(() => {
      window.__taskRunTrace = [];
      const panel = () => (window.__taskRunEl ||= document.getElementById("taskRun"));
      const attached = () => Boolean(panel()) && document.contains(panel());
      const record = (name, self) => window.__taskRunTrace.push(name + " on " + (self.id || self.className || self.nodeName) + ": " + new Error().stack);
      const watch = (target, name) => {
        const original = target[name];
        if (typeof original !== "function") return;
        target[name] = function (...args) {
          const before = attached();
          const result = original.apply(this, args);
          if (before && !attached()) record(name, this);
          return result;
        };
      };
      for (const name of ["removeChild", "insertBefore", "appendChild", "replaceChild"]) watch(Node.prototype, name);
      for (const name of ["replaceChildren", "remove", "append", "prepend", "after", "before", "replaceWith"]) watch(Element.prototype, name);
      for (const [proto, prop] of [[Node.prototype, "textContent"], [Element.prototype, "innerHTML"]]) {
        const descriptor = Object.getOwnPropertyDescriptor(proto, prop);
        Object.defineProperty(proto, prop, { ...descriptor, set(value) {
          const before = attached();
          descriptor.set.call(this, value);
          if (before && !attached()) record(prop, this);
        } });
      }
    })();`;
    const page = await openScene({ characterId: "mugi", layout, width, height, forcePng: true, sendDelayMs: 4000, initScript: traceScript });
    const consoleErrors = [];
    page.on("console", message => { if (message.type() === "error") consoleErrors.push(message.text()); });
    try {
      await submit(page, "猫の画像を作って");
      try {
        await page.waitForSelector("#taskRun:not([hidden])", { timeout: 10000 });
      } catch (error) {
        // 一覧が出ない原因を後から追えるよう、状態と画面を残してから止める。
        const state = await page.evaluate(() => {
          const panel = document.getElementById("taskRun");
          const rect = panel?.getBoundingClientRect();
          return {
            exists: Boolean(panel),
            hidden: panel?.hidden,
            parent: panel?.parentElement ? `${panel.parentElement.tagName}#${panel.parentElement.id}.${panel.parentElement.className}` : null,
            display: panel ? getComputedStyle(panel).display : null,
            size: rect ? [rect.width, rect.height] : null,
            childCount: panel?.childElementCount,
            statusBusy: document.getElementById("status")?.classList.contains("busy"),
            layout: document.getElementById("petApp")?.dataset.layout,
            booting: document.getElementById("petApp")?.classList.contains("is-booting"),
            bubbleClass: document.getElementById("bubble")?.className,
            homeParent: document.getElementById("taskRunHome")?.parentElement?.id || null,
            taskRunCount: document.querySelectorAll(".task-run").length,
            trace: (window.__taskRunTrace || []).slice(0, 3),
          };
        });
        await page.screenshot({ path: join(output, `${layout}-not-shown.png`) });
        throw new Error(`${layout}: 実行中タスクの一覧が表示されません: ${JSON.stringify({ state, consoleErrors })} (${error.message})`);
      }
      await page.waitForFunction(() => (window.__listeners["pet:agent_activity"] || []).length > 0
        && (window.__listeners["pet:agent_status"] || []).length > 0, null, { timeout: 10000 });
      await page.evaluate(() => {
        const emit = (channel, payload) => (window.__listeners[channel] || []).forEach(callback => callback({ payload }));
        emit("pet:agent_status", "計画を立てています…");
        emit("pet:agent_activity", { phase: "started", tools: ["web_search", "comfyui_generate"], labels: ["Web検索", "ローカル画像・動画生成"], failedTools: [] });
        emit("pet:agent_activity", { phase: "completed", tools: ["web_search"], labels: ["Web検索"], failedTools: [] });
      });
      // 経過時間の1秒更新が回ることも見る。
      await page.waitForTimeout(1300);
      const running = await page.evaluate(() => {
        const panel = document.getElementById("taskRun");
        const rect = panel.getBoundingClientRect();
        const status = document.getElementById("status");
        return {
          hidden: panel.hidden,
          display: getComputedStyle(panel).display,
          title: panel.querySelector(".task-run-title")?.textContent || "",
          total: panel.querySelector(".task-run-total")?.textContent || "",
          rows: [...panel.querySelectorAll(".task-run-row")].map(row => [row.querySelector(".task-run-label")?.textContent || "", row.dataset.state]),
          cancel: Boolean(panel.querySelector(".task-run-cancel")),
          inHistory: Boolean(panel.closest(".history-log")),
          rect: { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom, width: rect.width, height: rect.height },
          // 停止ボタンは見出しの中に収まり、状態や経過時間の文字に重ならないこと。
          cancelRect: (() => {
            const button = panel.querySelector(".task-run-cancel")?.getBoundingClientRect();
            const head = panel.querySelector(".task-run-head")?.getBoundingClientRect();
            return button && head ? { inHead: button.left >= head.left - 1 && button.right <= head.right + 1 && button.top >= head.top - 1 && button.bottom <= head.bottom + 1 } : null;
          })(),
          viewport: { width: innerWidth, height: innerHeight },
          statusBusyVisible: status.classList.contains("busy") && getComputedStyle(status).display !== "none",
          duplicateCancelVisible: (() => {
            const button = document.getElementById("cancelSend");
            return Boolean(button && !button.hidden && getComputedStyle(button).display !== "none");
          })(),
        };
      });
      const inside = running.rect.width > 0 && running.rect.height > 0 && running.rect.left >= 0 && running.rect.top >= 0
        && running.rect.right <= running.viewport.width + 1 && running.rect.bottom <= running.viewport.height + 1;
      if (running.hidden || running.display === "none" || running.title !== "実行中のタスク" || running.total === "経過 0秒"
          || JSON.stringify(running.rows) !== JSON.stringify(expectedRows) || !running.cancel || !running.cancelRect?.inHead
          || !inside || running.statusBusyVisible || running.duplicateCancelVisible
          // 会話履歴を出す形式では会話の末尾、それ以外ではフキダシ内に置く。
          || running.inHistory !== ["chat", "workspace", "stream"].includes(layout)) {
        // 何がどう崩れたかを後から見られるよう、止める前にその画面を残す。
        await page.screenshot({ path: join(output, `${layout}-running-failed.png`) });
        throw new Error(`${layout}: 実行中タスクの表示が不正です: ${JSON.stringify(running)}`);
      }
      await page.screenshot({ path: join(output, `${layout}-running.png`) });
      // 回答の文字送りと読み上げが終わってから送信処理の後始末が走るため、長めに待つ。
      try {
        await page.waitForFunction(() => document.querySelector("#taskRun .task-run-title")?.textContent === "タスク完了", null, { timeout: 45000 });
      } catch (error) {
        const state = await page.evaluate(() => ({
          title: document.querySelector("#taskRun .task-run-title")?.textContent || "",
          hidden: document.getElementById("taskRun")?.hidden,
          statusBusy: document.getElementById("status")?.classList.contains("busy"),
          reply: (document.getElementById("reply")?.textContent || "").slice(0, 80),
        }));
        throw new Error(`${layout}: 完了表示へ変わりませんでした: ${JSON.stringify(state)} (${error.message})`);
      }
      const finished = await page.evaluate(() => ({
        rows: [...document.querySelectorAll("#taskRun .task-run-row")].map(row => row.dataset.state),
        cancel: Boolean(document.querySelector("#taskRun .task-run-cancel")),
      }));
      if (finished.cancel || finished.rows.some(state => state === "running" || state === "waiting")) {
        throw new Error(`${layout}: 完了後に実行中の行や停止ボタンが残りました: ${JSON.stringify(finished)}`);
      }
      await page.screenshot({ path: join(output, `${layout}-finished.png`) });
      await page.waitForFunction(() => document.getElementById("taskRun").hidden, null, { timeout: 10000 });
      results.push({ layout, running, finished });
    } finally {
      await page.close();
    }
  }
  await writeFile(join(output, "result.json"), `${JSON.stringify(results, null, 2)}\n`);
  return output;
}

// ---------------------------------------------------------------- 録画と書き出し

const FFMPEG = process.env.SHOOT_FFMPEG || "ffmpeg";

const ffmpegAvailable = await run(FFMPEG, ["-version"]).then(() => true, () => false);

// ヘッドレス Chromium は透過を保ったまま動画キャプチャできない（compositor 出力には
// 背景が焼き込まれる）ため、透過 PNG を連写して繋ぐ。
//
// ただし 1 枚あたり 200ms 以上かかるので、実時間で連写すると 5fps も出ずカクつく。
// そこでページの時計を止め、「1/fps だけ時間を進める → 1 枚撮る」を繰り返す。
// 撮影にどれだけ時間がかかってもページ内の経過時間は一定なので、狙った fps で滑らかになる。
async function record(page, { seconds, fps, step }) {
  const frames = [];
  const gap = 1000 / fps;
  // pauseAt は過去を指せないので、読み取りから呼び出しまでに進む実時間ぶんを足しておく。
  const now = await page.evaluate(() => Date.now());
  // Live2Dや大きな画像の初回描画で呼び出し間が1秒を超えることがあるため、十分先へ進める。
  await page.clock.pauseAt(new Date(now + 10000));
  const total = Math.round((seconds * 1000) / gap);
  for (let index = 0; index < total; index++) {
    if (step) await step(page, index, fps);
    await page.clock.runFor(Math.round(gap));
    frames.push(await page.screenshot({ omitBackground: true }));
  }
  return frames;
}

// 連番 PNG をアニメーション WebP へ束ねる。GIF と違って透過が 1bit に落ちないので、
// 背景なしのまま README のライト / ダーク両方で使える。同じ絵で APNG は 4.8MB、WebP は 0.9MB。
async function encodeAnimation(frames, box, { fps, outputFps = fps, width, quality = 80, file }) {
  if (width && box.width < width) {
    throw new Error(`入力幅${box.width}pxを出力幅${width}pxへ引き伸ばす処理は禁止です: ${file}`);
  }
  mkdirSync(TEMP_OUT, { recursive: true });
  const dir = await mkdtemp(join(TEMP_OUT, "encode-"));
  try {
    for (const [index, png] of frames.entries()) {
      await writeFile(join(dir, `f-${String(index).padStart(4, "0")}.png`), png);
    }
    const crop = `crop=${box.width}:${box.height}:${box.x}:${box.y}`;
    const filters = [crop];
    if (outputFps !== fps) filters.push(`fps=${outputFps}`);
    if (width) filters.push(`scale=${width}:-1:flags=lanczos`);
    await run(FFMPEG, [
      "-y", "-loglevel", "error",
      "-framerate", String(fps),
      "-i", join(dir, "f-%04d.png"),
      "-vf", filters.join(","),
      // compression_level は既定の 4 のまま。6 にすると探索が跳ね上がり、数十コマで数分かかる。
      "-c:v", "libwebp_anim", "-loop", "0", "-q:v", String(quality), file,
    ]);
    const encoded = await readFile(file);
    const format = encoded.toString("ascii", 12, 16);
    const actualWidth = 1 + encoded[24] + (encoded[25] * 256) + (encoded[26] * 65536);
    if (format !== "VP8X" || (width && actualWidth !== width)) {
      throw new Error(`WebPの出力寸法が不正です: ${file}（形式${format}、幅${actualWidth}px、期待${width}px）`);
    }
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

// prepare は「録画開始時点の状態」を作る。文字送りを丸ごと録るため、応答を投げるのも
// prepare の役目にしてある（完了を待ってから録り始めると動きが写らない）。
// `outputWidth` は書き出し時の横幅（未指定なら等倍）。`width` はウィンドウ幅なので別物。
async function shoot(name, {
  seconds = 6, fps = 30, outputWidth, siteSeconds = seconds, siteOutputWidth,
  siteOutputFps = 16, siteQuality = 70,
  prepare, step, verifyLipSync = false, ...scene
} = {}) {
  const page = await openScene(scene);
  if (prepare) await prepare(page);
  const captureFps = LIGHTWEIGHT_GALLERY ? siteOutputFps : fps;
  const frames = await record(page, {
    seconds: LIGHTWEIGHT_GALLERY ? siteSeconds : seconds,
    fps: captureFps,
    step,
  });
  let lipSyncFailure = "";
  if (verifyLipSync) {
    const mouthSeen = await page.evaluate(() => window.__galleryMouthSeen === true);
    if (!mouthSeen) lipSyncFailure = `${name}: 読み上げ区間の口パクを確認できませんでした`;
  }
  await page.close();
  if (lipSyncFailure) throw new Error(lipSyncFailure);
  if (process.env.SHOOT_DEBUG_FRAMES) {
    const debugDir = join(ROOT, "temp", "recording-frames");
    mkdirSync(debugDir, { recursive: true });
    await Promise.all(frames.filter((_, index) => index % 6 === 0).map((frame, index) =>
      writeFile(join(debugDir, `${name}-${String(index).padStart(2, "0")}.png`), frame)));
  }

  // キャラが動く分だけ 1 コマ目より外へはみ出すので、間引いた複数コマの外接矩形を合成する。
  const sampled = frames.filter((_, index) => index % 5 === 0 || index === frames.length - 1);
  const box = unionBounds(await Promise.all(sampled.map(png => bounds(png, 16))));
  if (!box) throw new Error(`不透明な領域がありません: ${name}`);

  if (LIGHTWEIGHT_GALLERY) {
    if (!siteOutputWidth) throw new Error(`軽量一覧用の出力条件がありません: ${name}`);
    const output = CUSTOM_OUT || (SITE_GALLERY ? SITE_OUT : OUT);
    const label = SITE_GALLERY ? "LP" : "README";
    mkdirSync(output, { recursive: true });
    await encodeAnimation(frames, box, {
      fps: captureFps, outputFps: siteOutputFps, width: siteOutputWidth, quality: siteQuality,
      file: join(output, name),
    });
    console.log(`${label}用撮影: ${name}（${frames.length}コマ / ${siteOutputFps}fps / ${siteOutputWidth}px）`);
  } else {
    const output = CUSTOM_OUT || OUT;
    mkdirSync(output, { recursive: true });
    await encodeAnimation(frames, box, { fps, width: outputWidth, file: join(output, name) });
    console.log(`撮影: ${name}（${frames.length} コマ / ${fps} fps）`);
  }
}

// ---------------------------------------------------------------- 絵ごとの準備と動き

// 応答を投げるだけ。録画中に文字送りとキャラのモーション（考え中 → 返答）が写る。
const startReply = (ask = ASK) => (page) => submit(page, ask);

// 入力欄を開くとフキダシ本文が compact 表示になるので、短い応答で撮る（途中で切れた行が写らないように）。
// 1往復させてから入力欄を開き、録画中は「次の依頼を打ち込んでいる」ところを見せる。
const COMPOSER_ASK = "この PDF、2〜3ページだけ抜き出せる？";
const COMPOSER_REPLY = "できたよ。`資料_p2-3.pdf` として同じフォルダに保存したよ。";
const COMPOSER_TYPING = "この PSD を 800px の PNG にして";

async function prepareComposer(page) {
  await submit(page, COMPOSER_ASK);
  await waitForReply(page, COMPOSER_REPLY);
  await clearStatus(page);
  // フロントの openComposer() は応答中だと何もしないので、busy が下りるのを待ってから開く。
  await page.waitForFunction(() => typeof busy === "undefined" || busy === false, null, { timeout: 10000 });
  await page.evaluate(() => {
    const composer = document.getElementById("composer");
    if (typeof openComposer === "function") openComposer();
    if (composer && composer.hidden) {
      composer.hidden = false;
      document.getElementById("reply")?.classList.add("compact");
    }
    document.getElementById("input")?.focus();
  });
}

// 入力欄へ 1 コマ 1 文字ずつ打ち込む。`from` コマ目までは空のまま置いて、打ち始めを見せる。
const typeIntoInput = (text, from = 4) => (page, index) => {
  const shown = [...text].slice(0, Math.max(0, index - from)).join("");
  return page.evaluate((value) => {
    const input = document.getElementById("input");
    if (!input || input.value === value) return;
    input.value = value;
    input.dispatchEvent(new Event("input"));
  }, shown);
};

// 全画面フキダシは bubble に .fullscreen が付いた状態（Tauri の setFullscreen はモック）。
// 履歴は最新のやり取りが見えている状態にしてから応答を投げる（renderer が末尾へ自動スクロールする）。
async function prepareFullscreen(page) {
  await page.evaluate(() => { if (typeof toggleBubbleFullscreen === "function") toggleBubbleFullscreen(); });
  await page.waitForTimeout(300);
  await page.evaluate(() => { const l = document.getElementById("historyLog"); if (l) l.scrollTop = l.scrollHeight; });
  await submit(page, ASK);
}

// Live2D は WebGL 描画。ランタイムとモデルは cargo xtask setup live2d で取得する。
const LIVE2D_CHARACTER = "hiyori";
const LIVE2D_RUNTIME = [
  "ui/vendor/live2d/live2dcubismcore-5.3.min.js",
  "ui/vendor/live2d/live2dcubismcore-compat.min.js",
  "ui/vendor/pixi/pixi.min.js",
  "ui/vendor/live2d/cubism4.min.js",
];
const LIVE2D_REQUIRED = [...LIVE2D_RUNTIME, `ui/assets/characters/${LIVE2D_CHARACTER}/live2d`];

async function prepareLive2D(page) {
  await page.waitForFunction(() => {
    const canvas = document.getElementById("live2dCanvas");
    return canvas && canvas.style.display !== "none" && canvas.width > 0 && typeof live2dActive !== "undefined" && live2dActive;
  }, null, { timeout: 40000 });
  // 初回モーションが動き出してポーズが決まるまで少し待つ。
  await page.waitForTimeout(1500);
  // パラメータ名が合っているだけでなく、閉口と開口で口元の描画が実際に変わることを確認する。
  // tickerを止めて同じポーズのまま比較し、身体やまばたきの変化を口パクと誤認しない。
  await page.evaluate(() => {
    live2dModel?.internalModel?.motionManager?.stopAllMotions?.();
    live2dApp.ticker.stop();
    const core = live2dModel.internalModel.coreModel;
    core.setParameterValueById("ParamMouthOpenY", 0);
    core.update();
    live2dApp.renderer.render(live2dApp.stage);
  });
  const closed = await page.locator("#live2dCanvas").screenshot({ omitBackground: true });
  await page.evaluate(() => {
    const core = live2dModel.internalModel.coreModel;
    core.setParameterValueById("ParamMouthOpenY", 0.75);
    core.update();
    live2dApp.renderer.render(live2dApp.stage);
  });
  const opened = await page.locator("#live2dCanvas").screenshot({ omitBackground: true });
  if (process.env.SHOOT_DEBUG_LIP) {
    const debugDir = join(ROOT, "temp", "lipcheck");
    mkdirSync(debugDir, { recursive: true });
    await Promise.all([
      writeFile(join(debugDir, "closed.png"), closed),
      writeFile(join(debugDir, "opened.png"), opened),
    ]);
  }
  // モデルのscale・offset・表示寸法で口のキャンバス座標は変わる。tickerと全motionを止めた
  // 同一ポーズ比較なので、固定座標を決め打ちせずキャンバス全体の差分を口開閉の結果として測る。
  const mouthPixels = await changedPixels(closed, opened);
  if (mouthPixels < 20) throw new Error(`Live2D の口が描画へ反映されていません（変化 ${mouthPixels}px）`);
  console.log(`  口元の開閉描画差: ${mouthPixels}px`);
  await page.evaluate(() => {
    live2dMouth = -1;
    live2dApp.ticker.start();
    playLive2DMotion("idle");
  });
  await submit(page, ASK);
  await page.waitForFunction(() => typeof live2dMouth !== "undefined" && live2dMouth > 0.05, null, { timeout: 5000 });
}

// 配布ページ用は小さい埋め込み表示でもキャラの輪郭が読める構図にし、感情オーラを
// キャラ素材の背景と誤認させない。アプリ本体のモーションや表情には手を加えない。
async function prepareDistribution2D(page) {
  await page.addStyleTag({ content: ".character::before,.character-view::before{display:none!important}" });
  await submit(page, ASK);
  await page.waitForFunction(() => {
    const image = document.getElementById("petSprite");
    return image?.complete && image.naturalWidth > 0;
  }, null, { timeout: 20000 });
}

async function prepareDistributionLive2D(page) {
  await prepareLive2D(page);
  await page.addStyleTag({ content: ".character::before,.character-view::before{display:none!important}" });
}

// 配布ページ用の尺とフレームレート。distributionStep の経過msもここから出すので、
// fps を変えても巡回の見せ時間がずれない。
const DISTRIBUTION_SCENE = { seconds: 7, fps: 30 };

const DISTRIBUTION_TOUR = [
  [1600, "thinking"],
  [3100, "success"],
  [3900, "idle"],
];

const distributionStep = async (page, index) => {
  const elapsedMs = (index * 1000) / DISTRIBUTION_SCENE.fps;
  const current = DISTRIBUTION_TOUR.filter(([at]) => elapsedMs >= at).pop();
  await page.evaluate(name => {
    if (!name) return;
    setAnimation(name);
    if (typeof live2dActive === "undefined" || !live2dActive) {
      rigLookDirection = 0;
      rigRefresh();
    }
  }, current ? current[1] : null);
};

async function validateLive2dSdkModels() {
  const manifest = JSON.parse(await readFile(join(ROOT, "characters", "hiyori", "character.json"), "utf8"));
  const candidates = Array.isArray(manifest.live2d?.models) && manifest.live2d.models.length
    ? manifest.live2d.models
    : [{ path: manifest.live2d?.model, sdk: "4" }];
  const supportedSdks = [...new Set(candidates.map(candidate => String(candidate.sdk || "4")))];
  for (const sdk of supportedSdks) {
    const page = await openScene({ characterId: "hiyori", live2dSdkOverrides: { hiyori: sdk } });
    try {
      await page.waitForFunction(() => typeof live2dActive !== "undefined" && live2dActive, null, { timeout: 40000 });
      const loaded = await page.evaluate(() => ({
        sdk: activeLive2dSdkPreference,
        core: live2dRuntimeSdk,
        width: live2dModel?.internalModel?.originalWidth || 0,
        height: live2dModel?.internalModel?.originalHeight || 0,
      }));
      if (loaded.sdk !== sdk || loaded.core !== sdk || loaded.width <= 0 || loaded.height <= 0) {
        throw new Error(`Cubism ${sdk} の固定読込結果が不正です: ${JSON.stringify(loaded)}`);
      }
      console.log(`  Cubism ${sdk}固定読込: ${loaded.width}x${loaded.height}`);
    } finally {
      await page.close();
    }
  }
}

// ---------------------------------------------------------------- キャラクター一覧の絵

// README のキャラクター一覧。同じ問いかけへの返事を、キャラごとの口調で見せる。
// 一覧も実利用時と同じ標準ペット形式を使う。小型形式へ詰め込むと会話操作が目立ち、
// キャラも切れて実画面とかけ離れるため、配布用だけの変則構図は作らない。
const GALLERY_ASK = "おはよう。今日もよろしくね";
// voice: true で読み上げ音声（音節風の振幅を持つWAV）を返させ、口パクを実経路で動かす。
// 対応キャラは録画中に実際の口開閉が1回以上起きたことまで検査する。
// 高解像度の比較・保管を明示した場合は、実画面を2倍密度で描画してから960pxへ縮小する。
// README／LP公開用はこの生描画から960pxへ直接書き出し、既存WebPを再圧縮しない。
const GALLERY_SCENE = {
  layout: "pet", width: 900, height: 600, deviceScaleFactor: 2,
  seconds: 12, fps: 30, outputWidth: 960,
  // README／LPは同じ生PNGから960px・16fpsへ直接書き出す。既存WebPの再エンコードは禁止。
  siteSeconds: 6, siteOutputWidth: 960, siteOutputFps: 16, siteQuality: 85, voice: true,
};

// 低頭身のむぎはアプリ既定の上半身構図だと、小さいLP／README枠で顔だけが
// 大きく見える。公開用の実画面撮影だけ少し引き、表情と手の動きを一緒に見せる。
const MUGI_PUBLIC_FRAMING = {
  mugi: {
    pet: { preset: "custom", zoom: 1.28, x: 0, y: 0 },
    stream: { preset: "custom", zoom: 1.35, x: 0, y: 0 },
  },
};

// 前半5秒は返答フローそのまま（文字送り → 音声開始後の口パク）を保ち、後半に
// 考え中、成功、悲しみ、左右視線、待機を間隔を空けて見せる。
// 経過msで指定するのは、fps や尺を変えてもモーションの見せ時間がずれないようにするため。
//
// 巡回に「考え中」が入っているのは、`record()` が録画前に時計を10秒進めるので、送信直後の
// 考え中が録画開始時点では終わっているため。成功は短い反応を1回だけ確認して待機へ戻す。
// `SHOOT_DEBUG_MOTION=1` で data-motion を読み返して切替回数を確かめられる。
const GALLERY_TOUR = [
  [5200, "thinking"],
  [6700, "success"],
  [8000, "sad"],
  [9300, "gaze-left"],
  [10400, "gaze-right"],
  [11400, "idle"],
];
const LIGHTWEIGHT_GALLERY_TOUR = [
  [4800, "thinking"],
  [5600, "idle"],
];

// 毎コマ呼んで構わない。`setAnimation()` は同じ状態なら早期 return するので、
// 読み上げ終了でアプリ側が idle へ戻しても次のコマで巡回中の状態へ復帰する。
const galleryStep = async (page, index, captureFps = GALLERY_SCENE.fps) => {
  const elapsedMs = (index * 1000) / captureFps;
  const tour = LIGHTWEIGHT_GALLERY ? LIGHTWEIGHT_GALLERY_TOUR : GALLERY_TOUR;
  const current = tour.filter(([at]) => elapsedMs >= at).pop();
  // SHOOT_DEBUG_MOTION=1 で、実際に適用された data-motion を毎コマ出す
  // （巡回を足しても「切り替わったつもり」で撮れてしまうのを防ぐ）。
  const applied = await page.evaluate(name => {
    const mouthMoving = (typeof rigMouthOpen !== "undefined" && rigMouthOpen)
      || (typeof live2dMouth !== "undefined" && live2dMouth > 0.05)
      // レイヤー差分を持たないPNGキャラは、発話用フレーム列そのものが口パク。
      // `animationName === "talking"` は読み上げ開始後にだけ設定されるため、
      // 音声より先に口を動かすデグレも見逃さず、両方式を同じ契約で検査できる。
      || (typeof rigAvailable !== "undefined" && !rigAvailable
        && typeof animationName !== "undefined" && animationName === "talking");
    if (mouthMoving) window.__galleryMouthSeen = true;
    if (name === "gaze-left" || name === "gaze-right") {
      setAnimation("idle");
      if (typeof live2dActive === "undefined" || !live2dActive) {
        rigLookDirection = name === "gaze-left" ? -1 : 1;
        rigRefresh();
      }
    } else if (name) {
      setAnimation(name);
      if (typeof live2dActive === "undefined" || !live2dActive) {
        rigLookDirection = 0;
        rigRefresh();
      }
    }
    return `${document.getElementById("character")?.dataset.motion || ""}/${name || "reply"}`;
  }, current ? current[1] : null);
  if (process.env.SHOOT_DEBUG_MOTION) console.log(`    ${String(elapsedMs).padStart(5)}ms  data-motion=${applied}`);
};

// 一覧の Live2D キャラは「描画が始まっていること」だけ確かめる。
// `prepareLive2D` の口パク検査は寸法とモデルがひより固定なので、ここでは通さない
// （検査は live2d-light / live2d-dark の担当。一覧で流用すると他モデルで必ず落ちる）。
async function prepareGalleryLive2D(page) {
  await page.waitForFunction(() => {
    const canvas = document.getElementById("live2dCanvas");
    return canvas && canvas.style.display !== "none" && canvas.width > 0
      && typeof live2dActive !== "undefined" && live2dActive;
  }, null, { timeout: 40000 });
  await page.waitForTimeout(1500); // 初回モーションでポーズが決まるまで待つ
  await submit(page, GALLERY_ASK);
}

const galleryPrepare = (id, kind) => kind === "live2d"
  ? prepareGalleryLive2D
  : async page => {
    await submit(page, GALLERY_ASK);
    // セナは初期化直後の2Dフォールバックを先頭コマへ混ぜず、最初から同じ表示寸法で撮る。
    if (id === "sena") {
      await page.waitForFunction(() => {
        const canvas = document.getElementById("meshRigCanvas");
        return document.getElementById("character")?.dataset.renderKind === "mesh-rig"
          && canvas && canvas.width > 0 && canvas.height > 0 && getComputedStyle(canvas).display !== "none";
      }, null, { timeout: 20000 });
      // DOM切替だけでなく、PIXIの初回描画が透明WebGLへ合成されてから録画を始める。
      await page.waitForTimeout(200);
    }
  };

const galleryRequires = (id, kind) => kind === "live2d"
  ? [...LIVE2D_RUNTIME, `ui/assets/characters/${id}/live2d`]
  : [`ui/assets/characters/${id}/character.json`];

// Live2D キャラのランタイムとモデルは git 管理外（`cargo xtask setup live2d` / `setup live2d-samples`）。
// 未取得の環境では `requires` が効いて自動でスキップされるので、一覧の定義からは外さない。
// Live2Dの表示例は取得済みの公式サンプルを個別に撮影する。2Dむぎは通常のPNGキャラとして撮影する。
const GALLERY = [
  ["mugi", "おはよう！えへへ、今日もそばで見てるね。まずはやること、3つに絞ろっか。"],
  ["rin", "ふ、ふん。来たのね。今日の作業くらい見ててあげる。"],
  ["mai", "おはよう。あせらなくて大丈夫よ、ゆっくり一緒に進めましょう。"],
  ["sena", "おはよう。今日のステージも、きれいに仕上げようか。"],
  ["aoi", "こんにちは。気になることを、一緒に丁寧に確かめましょう。"],
  ["mugi-normal-lvs", "こんにちは。今日もいっしょに、のんびり進めようね。"],
  ["photoreal-prototype", "こんにちは。焦らず、今日やることから一緒に整理しましょう。"],
  ["hiyori", "やっほー！今日はなにする？なんでも言ってね、いっしょにがんばろ！", "live2d"],
  ["haru", "こんにちは。今日の作業を落ち着いて進めましょう。", "live2d"],
  ["epsilon", "動作確認でも、作業の整理でも、必要なところから始めましょう。", "live2d"],
  ["mao", "こんにちは。今日は何から片づけましょうか。ゆっくりで大丈夫ですよ。", "live2d"],
  ["rice", "よく来た。……さて、何を調べようか。", "live2d"],
  ["wanko", "わふっ！　……よし、いっしょにやろう！", "live2d"],
];

const GALLERY_SHOTS = Object.fromEntries(GALLERY.map(([id, reply, kind = "png"]) => [
  `character-${id}.webp`,
  {
    ...GALLERY_SCENE,
    characterId: id,
    characterFraming: id === "mugi" ? MUGI_PUBLIC_FRAMING : {},
    reply,
    prepare: galleryPrepare(id, kind),
    step: galleryStep,
    siteQuality: 85,
    verifyLipSync: id !== "rice",
    requires: galleryRequires(id, kind),
  },
]));

const GALLERY_STREAM_SHOTS = Object.fromEntries(GALLERY.map(([id, reply, kind = "png"]) => [
  `character-${id}-stream.webp`,
  {
    ...GALLERY_SCENE,
    layout: "stream",
    width: 1920,
    height: 1080,
    // 配信画面はフルHDのネイティブ描画から目的の幅へ縮小する。
    // 2倍密度の3840px全画面キャプチャは不要な負荷を避ける。
    deviceScaleFactor: 1,
    characterId: id,
    characterFraming: id === "mugi" ? MUGI_PUBLIC_FRAMING : {},
    reply,
    prepare: galleryPrepare(id, kind),
    step: galleryStep,
    siteQuality: 85,
    verifyLipSync: id !== "rice",
    requires: galleryRequires(id, kind),
  },
]));

// LPへ出す自前キャラはrelease.jsonを正本にし、公式Live2Dサンプルだけを動作例として加える。
// 販売対象外の試作をLP資産へ混ぜない。
const RELEASE = JSON.parse(await readFile(join(ROOT, "distribution", "release.json"), "utf8"));
const SITE_GALLERY_IDS = new Set([
  ...(RELEASE.includedCharacters || []),
  ...GALLERY.filter(([, , kind = "png"]) => kind === "live2d").map(([id]) => id),
]);
const SITE_GALLERY_FILES = new Set([
  ...[...SITE_GALLERY_IDS].map(id => `character-${id}.webp`),
  ...[...SITE_GALLERY_IDS].map(id => `character-${id}-stream.webp`),
]);
const README_GALLERY_FILES = new Set([
  ...GALLERY.map(([id]) => `character-${id}.webp`),
  ...GALLERY.map(([id]) => `character-${id}-stream.webp`),
]);

// ---------------------------------------------------------------- 撮る絵の一覧

const SHOTS = {
  ...GALLERY_SHOTS,
  ...GALLERY_STREAM_SHOTS,
  "main-light.webp": { width: 900, height: 600, layout: "pet", deviceScaleFactor: 2, characterFraming: MUGI_PUBLIC_FRAMING, ...DISTRIBUTION_SCENE, outputWidth: 960, voice: true, prepare: prepareDistribution2D, step: distributionStep },
  "main-dark.webp": { theme: "dark", prepare: startReply() },
  "composer-light.webp": {
    reply: COMPOSER_REPLY,
    prepare: prepareComposer,
    step: typeIntoInput(COMPOSER_TYPING),
  },
  "fullscreen-light.webp": { width: 900, height: 600, prepare: prepareFullscreen },
  "live2d-light.webp": { characterId: LIVE2D_CHARACTER, requires: LIVE2D_REQUIRED, prepare: prepareLive2D },
  "live2d-dark.webp": { characterId: LIVE2D_CHARACTER, theme: "dark", requires: LIVE2D_REQUIRED, prepare: prepareLive2D },
  "live2d-motion.webp": { width: 900, height: 600, layout: "pet", deviceScaleFactor: 2, ...DISTRIBUTION_SCENE, outputWidth: 960, voice: true, characterId: LIVE2D_CHARACTER, requires: LIVE2D_REQUIRED, prepare: prepareDistributionLive2D, step: distributionStep },
  "distribution-main.webp": { width: 900, height: 600, layout: "pet", characterFraming: MUGI_PUBLIC_FRAMING, ...DISTRIBUTION_SCENE, outputWidth: 640, voice: true, prepare: prepareDistribution2D, step: distributionStep },
  "distribution-live2d.webp": { width: 900, height: 600, layout: "pet", ...DISTRIBUTION_SCENE, outputWidth: 640, voice: true, characterId: LIVE2D_CHARACTER, requires: LIVE2D_REQUIRED, prepare: prepareDistributionLive2D, step: distributionStep },
  // 設定画面のプレビューと README は、各形式の実際の既定寸法・本物の CSS から作る。
  // 1フレームだけなのでアニメーションではないが、同じ透過 WebP 経路を使える。
  // 小型形式は論理幅が出力幅（480px）に届かないため、2倍密度で描画してから縮小する。
  // 等倍のままだと不透明領域の切り出し幅が480pxを下回り、引き伸ばし禁止の検査で止まる
  // （実際に mascot 360px と widget 440px は、旧実装で480pxへ引き伸ばされたまま同梱されていた）。
  "layout-shot-mascot.webp": { width: 360, height: 560, layout: "mascot", deviceScaleFactor: 2, seconds: 1, fps: 1, outputWidth: 480 },
  "layout-shot-pet.webp": { width: 900, height: 600, layout: "pet", seconds: 1, fps: 1, outputWidth: 480 },
  "layout-shot-chat.webp": { width: 520, height: 760, layout: "chat", deviceScaleFactor: 2, seconds: 1, fps: 1, outputWidth: 480 },
  "layout-shot-bar.webp": { width: 900, height: 320, layout: "bar", seconds: 1, fps: 1, outputWidth: 480 },
  "layout-shot-workspace.webp": { width: 1400, height: 850, layout: "workspace", seconds: 1, fps: 1, outputWidth: 640 },
  "layout-shot-stream.webp": { width: 1920, height: 1080, layout: "stream", seconds: 1, fps: 1, outputWidth: 640 },
  "layout-shot-widget.webp": { width: 440, height: 600, layout: "widget", deviceScaleFactor: 2, seconds: 1, fps: 1, outputWidth: 480 },
};

// ---------------------------------------------------------------- 実行

mkdirSync(CUSTOM_OUT || OUT, { recursive: true });
const only = process.argv.slice(2);
const skipped = [];
let failed = 0;

if (LIGHTWEIGHT_GALLERY && only.some(name => !name.startsWith("character-"))) {
  throw new Error("軽量一覧モードは character-*.webp だけを対象にしてください。");
}
if (SITE_GALLERY && only.some(name => !SITE_GALLERY_FILES.has(name))) {
  throw new Error("販売対象または公式Live2D動作例ではないキャラクターをLPへ出力しようとしました。");
}
if (README_GALLERY && only.some(name => !README_GALLERY_FILES.has(name))) {
  throw new Error("README一覧にないキャラクターを書き出そうとしました。");
}

if (process.env.SHOOT_MESH_RIG) {
  const result = await validateMeshRig();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`メッシュリグ検査完了: ${result.output}`);
  process.exit(0);
}

if (process.env.SHOOT_CHARACTER_MATRIX) {
  const matrix = await shootCharacterMatrix();
  await trimmer.close();
  await browser.close();
  server.close();
  const average = Math.round(matrix.results.reduce((sum, item) => sum + item.readyMs, 0) / matrix.results.length);
  console.log(`${matrix.results.length}形式の表示検査完了: ${matrix.output}（平均 ${average}ms）`);
  process.exit(0);
}

if (process.env.SHOOT_LAYOUT_EDITOR) {
  const output = await validateLayoutEditor();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`レイアウト編集検査完了: ${output}`);
  process.exit(0);
}

if (process.env.SHOOT_INTERACTION_MATRIX) {
  const matrix = await validateInteractionMatrix();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`表示操作検査完了: ${matrix.output}（${matrix.results.length}件）`);
  process.exit(0);
}

if (process.env.SHOOT_BUBBLE_BLUR) {
  const measured = await measureBubbleBlur();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`フキダシ背景ぼかし計測完了: ${measured.output}（${measured.results.length}条件）`);
  process.exit(0);
}

if (process.env.SHOOT_LIVE2D_MATRIX) {
  const matrix = await validateLive2dMatrix();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`Live2D表示検査完了: ${matrix.output}（${matrix.results.length}件、未取得${matrix.skipped.length}件）`);
  process.exit(0);
}

if (process.env.SHOOT_STARTUP_FAILURES) {
  const page = await openScene({ characterId: "hiyori", layout: "pet", stallCharacterId: "hiyori" });
  await page.waitForFunction(() => {
    const sprite = document.getElementById("petSprite");
    return sprite?.complete && sprite.naturalWidth > 0 && sprite.naturalHeight > 0;
  }, null, { timeout: 5000 });
  const state = await page.evaluate(() => ({
    booting: document.getElementById("petApp")?.classList.contains("is-booting"),
    characterId: typeof characterManifest === "object" ? characterManifest?.id : "",
    renderKind: document.getElementById("character")?.dataset.renderKind || "",
    status: document.getElementById("status")?.textContent || "",
    layout: document.getElementById("petApp")?.dataset.layout || "",
    characterRect: document.getElementById("character")?.getBoundingClientRect().toJSON(),
    characterViewRect: document.getElementById("characterView")?.getBoundingClientRect().toJSON(),
    spriteRect: document.getElementById("petSprite")?.getBoundingClientRect().toJSON(),
    spriteTransform: getComputedStyle(document.getElementById("petSprite")).transform,
    spriteNaturalWidth: document.getElementById("petSprite")?.naturalWidth || 0,
    spriteNaturalHeight: document.getElementById("petSprite")?.naturalHeight || 0,
  }));
  if (state.booting || state.characterId !== "mugi" || state.renderKind !== "2d"
      || !state.spriteRect || state.spriteRect.width < 240 || state.spriteRect.left > 700
      || state.spriteNaturalWidth < 1 || state.spriteNaturalHeight < 1) {
    throw new Error(`停止キャラから既定PNGへ復帰できません: ${JSON.stringify(state)}`);
  }
  const output = join(ROOT, "temp", "startup-fallback.png");
  await page.screenshot({ path: output, omitBackground: true });
  await page.close();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`起動フォールバック検査完了: ${output} / ${JSON.stringify(state)}`);
  process.exit(0);
}

if (process.env.SHOOT_TASK_RUN) {
  const output = await validateTaskRun();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`実行中タスク表示検査完了: ${output}`);
  process.exit(0);
}

if (process.env.SHOOT_AI_GAMES) {
  const output = await validateAiGames();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`「人間だとバレるな」表示検査完了: ${output}`);
  process.exit(0);
}

if (process.env.SHOOT_THINKING_INDICATOR) {
  const output = await validateThinkingIndicator();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`考え中表示検査完了: ${output}`);
  process.exit(0);
}

if (process.env.SHOOT_STARTUP_READINESS) {
  const page = await openScene({
    characterId: "mugi",
    layout: "pet",
    voice: true,
    waitForReady: false,
    engineReadyDelayMs: 700,
    voiceReadyDelayMs: 600,
    speechDelayMs: 500,
  });
  await page.waitForFunction(() => document.getElementById("petApp")?.classList.contains("is-booting"));
  await page.waitForTimeout(900);
  const releasedTooEarly = await page.evaluate(() => !document.getElementById("petApp")?.classList.contains("is-booting"));
  if (releasedTooEarly) throw new Error("AIまたは音声の準備中に起動画面を終了しました");
  await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 10000 });
  const result = await page.evaluate(() => ({
    marks: window.__startupMarks,
    booting: document.getElementById("petApp")?.classList.contains("is-booting"),
    status: document.getElementById("status")?.textContent || "",
  }));
  const { engineReadyAt, voiceReadyAt, speechReadyAt, finishBootCalledAt } = result.marks;
  if (!(engineReadyAt <= voiceReadyAt && voiceReadyAt <= speechReadyAt && speechReadyAt <= finishBootCalledAt)) {
    throw new Error(`起動準備の順序が不正です: ${JSON.stringify(result)}`);
  }
  const output = join(ROOT, "temp", "startup-readiness-qa");
  mkdirSync(output, { recursive: true });
  await writeFile(join(output, "result.json"), `${JSON.stringify(result, null, 2)}\n`);
  await page.screenshot({ path: join(output, "ready.png"), omitBackground: true });
  await page.close();

  // 初回セットアップはローカルAIを使わない。数GBの取得を待たずに案内まで進むこと。
  const firstRun = await openScene({
    characterId: "mugi",
    layout: "pet",
    waitForReady: false,
    onboardingDone: false,
    engineReadyDelayMs: 8000,
  });
  await firstRun.waitForFunction(
    () => !document.getElementById("petApp")?.classList.contains("is-booting"),
    null,
    { timeout: 6000 },
  );
  const firstRunState = await firstRun.evaluate(() => ({
    engineReady: Date.now() >= window.__engineReadyAt,
    onboardingVisible: document.getElementById("onboarding")?.hidden === false,
    sendDisabled: document.getElementById("sendButton")?.disabled === true,
  }));
  if (firstRunState.engineReady || !firstRunState.onboardingVisible || !firstRunState.sendDisabled) {
    throw new Error(`初回セットアップの案内条件が崩れています: ${JSON.stringify(firstRunState)}`);
  }
  await firstRun.screenshot({ path: join(output, "first-run.png"), omitBackground: true });
  await writeFile(join(output, "first-run.json"), `${JSON.stringify(firstRunState, null, 2)}\n`);
  await firstRun.close();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`起動準備順序検査完了（初回セットアップの非待機を含む）: ${output}`);
  process.exit(0);
}

if (!ffmpegAvailable) {
  throw new Error(
    `${FFMPEG} を実行できません。動く絵の書き出しに使います。` +
      "PATH に通すか、SHOOT_FFMPEG に実行ファイルの絶対パスを指定してください。",
  );
}

if (process.env.SHOOT_DEBUG_BOOT) {
  const characterId = process.env.SHOOT_BOOT_CHARACTER || "mugi";
  const layout = process.env.SHOOT_BOOT_LAYOUT || "pet";
  const page = await openScene({ characterId, layout, bootHoldMs: 5000, waitForReady: false });
  await page.waitForFunction(() => document.getElementById("petApp")?.classList.contains("is-booting"));
  await page.waitForFunction(() => document.getElementById("petApp")?.classList.contains("boot-character-ready"), null, { timeout: 40000 });
  await page.waitForTimeout(500);
  const debugDir = join(ROOT, "temp", "boot-check");
  mkdirSync(debugDir, { recursive: true });
  await page.screenshot({ path: join(debugDir, `boot-${characterId}-${layout}.png`), omitBackground: true });
  await page.close();
  console.log(`  起動画面確認: ${join(debugDir, `boot-${characterId}-${layout}.png`)}`);
  await trimmer.close();
  await browser.close();
  server.close();
  process.exit(0);
}

if (process.env.SHOOT_DEBUG_ONBOARDING) {
  const page = await openScene({ width: 680, height: 430, onboardingDone: false });
  await page.waitForSelector("#onboardingComponentStep:not([hidden])");
  await page.waitForTimeout(300);
  const debugDir = join(ROOT, "temp", "onboarding-check");
  mkdirSync(debugDir, { recursive: true });
  await page.screenshot({ path: join(debugDir, "components.png"), omitBackground: true });
  await page.locator('button[data-onboarding-action="skip-components"]').click();
  await page.waitForSelector("#onboardingCharacterStep:not([hidden])");
  await page.screenshot({ path: join(debugDir, "live2d-guide.png"), omitBackground: true });
  await page.close();
  console.log(`  初回案内確認: ${join(debugDir, "components.png")} / ${join(debugDir, "live2d-guide.png")}`);
  await trimmer.close();
  await browser.close();
  server.close();
  process.exit(0);
}

const wantsLive2d = (!only.length || only.some(name => name.startsWith("live2d-")))
  && LIVE2D_REQUIRED.every(path => existsSync(join(ROOT, path)));
if (wantsLive2d) await validateLive2dSdkModels();

for (const [name, { requires = [], ...options }] of Object.entries(SHOTS)) {
  if (SITE_GALLERY && !SITE_GALLERY_FILES.has(name)) continue;
  if (README_GALLERY && !README_GALLERY_FILES.has(name)) continue;
  if (only.length && !only.includes(name)) continue;
  const missing = requires.filter(path => !existsSync(join(ROOT, path)));
  if (missing.length) {
    skipped.push([name, missing]);
    continue;
  }
  try {
    await shoot(name, options);
  } catch (error) {
    failed += 1;
    console.error(`失敗: ${name}: ${error.message}`);
  }
}

await trimmer.close();
await browser.close();
server.close();

for (const [name, missing] of skipped) {
  console.warn(`スキップ: ${name}（未取得: ${missing.join(", ")}）\n  → cargo xtask setup live2d で取得できます。`);
}
if (SITE_GALLERY && !only.length && !failed && !skipped.length) {
  const galleryFiles = [...SITE_GALLERY_FILES];
  // 容量検査は実際の出力先を見る。SHOOT_OUTPUT_DIR 指定時に既定の出力先を読むと、
  // 今回書いていない古いファイルを検査するか、未生成でENOENTになる。
  const bytes = (await Promise.all(galleryFiles.map(name => readFile(join(CUSTOM_OUT || SITE_OUT, name)))))
    .reduce((sum, content) => sum + content.length, 0);
  const indexFiles = ["character-mugi.webp", "character-sena.webp", "character-aoi.webp", "character-photoreal-prototype.webp"];
  const indexBytes = (await Promise.all(indexFiles.map(name => readFile(join(CUSTOM_OUT || SITE_OUT, name)))))
    .reduce((sum, content) => sum + content.length, 0);
  if (bytes > 80_000_000 || indexBytes > 20_000_000) {
    throw new Error(`LP画像容量が上限を超えています: キャラ一覧${(bytes / 1_000_000).toFixed(2)}MB / トップ${(indexBytes / 1_000_000).toFixed(2)}MB`);
  }
  console.log(`LP画像容量: キャラ一覧${(bytes / 1_000_000).toFixed(2)}MB / トップ${(indexBytes / 1_000_000).toFixed(2)}MB`);
}
if (README_GALLERY && !only.length && !failed && !skipped.length) {
  const sizes = await Promise.all([...README_GALLERY_FILES].map(async name => ({
    name,
    size: (await readFile(join(CUSTOM_OUT || OUT, name))).length,
  })));
  const total = sizes.reduce((sum, item) => sum + item.size, 0);
  const largest = sizes.reduce((left, right) => left.size >= right.size ? left : right);
  if (total > 80_000_000 || largest.size > 8_000_000) {
    throw new Error(`README画像容量が上限を超えています: 合計${(total / 1_000_000).toFixed(2)}MB / 最大${(largest.size / 1_000_000).toFixed(2)}MB（${largest.name}）`);
  }
  console.log(`README画像容量: 合計${(total / 1_000_000).toFixed(2)}MB / 最大${(largest.size / 1_000_000).toFixed(2)}MB（${largest.name}）`);
}
if (failed) process.exitCode = 1;
