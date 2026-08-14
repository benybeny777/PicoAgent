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
import { existsSync, mkdirSync } from "node:fs";
import { execFile } from "node:child_process";
import { tmpdir } from "node:os";
import { promisify } from "node:util";
import { extname, isAbsolute, join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const run = promisify(execFile);

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const UI_ROOT = join(ROOT, "ui");
const OUT = join(ROOT, "docs", "images");

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
const mock = ({ theme, history, reply, characterId, layout, live2dSdkOverrides, initDelayMs, bootHoldMs, onboardingDone, voice, gazeTrackingEnabled, betaBuild }) => `
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
window.__bootReadyAt = Date.now() + ${JSON.stringify(bootHoldMs)};
window.__TAURI__ = {
  core: { invoke: (name) => {
    if (name === "pet_init") return new Promise(resolve => setTimeout(() => resolve({
      up: true, providerId: "local", model: "gemma-3-4b-it-Q4_K_M",
      theme: ${JSON.stringify(theme)}, fontPx: "13", voiceVolume: "100",
      characterId: ${JSON.stringify(characterId)}, characterOverrides: {},
      betaBuild: ${JSON.stringify(betaBuild)},
      layout: ${JSON.stringify(layout)},
      live2dSdkOverrides: ${JSON.stringify(live2dSdkOverrides)},
      history: ${JSON.stringify(history)},
      quickActions: "", napMinutes: "0",
      onboardingDone: ${JSON.stringify(onboardingDone)}, relationshipMemoryEnabled: false,
      gazeTrackingEnabled: ${JSON.stringify(gazeTrackingEnabled)},
      chatterEnabled: false, chatterMinSecs: "9999", chatterMaxSecs: "9999",
    }), ${JSON.stringify(initDelayMs)}));
    if (name === "engine_status") return Promise.resolve({ phase: "ready" });
    if (name === "aivis_status") return Promise.resolve({ enabled: true, up: Date.now() >= window.__bootReadyAt });
    if (name === "pet_speak") return Promise.resolve(
      ${JSON.stringify(Boolean(voice))} || ${JSON.stringify(characterId)} === "hiyori"
        ? { enabled: true, mime: "audio/wav", audio: window.__previewVoice }
        : { enabled: false },
    );
    if (name === "pet_send") return new Promise(r => setTimeout(() => r({ reply: ${JSON.stringify(reply)} }), 80));
    if (name === "pet_alarms") return Promise.resolve({ alarms: [] });
    if (name === "pet_daily_events") return Promise.resolve({ events: [] });
    if (name === "settings_components_status") return Promise.resolve([
      { id: "aivis", ready: true, available: true },
      { id: "whisper", ready: false, available: true },
      { id: "tha3", ready: false, available: true },
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
  bootHoldMs = 0, waitForReady = true, forcePng = false, onboardingDone = true, reducedMotion = false,
  gazeTrackingEnabled = false, betaBuild = true, deviceScaleFactor = 1,
} = {}) {
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor });
  if (reducedMotion) await page.emulateMedia({ reducedMotion: "reduce" });
  // 「時計を止めて 1 コマずつ進める」ので、読み込み前に偽の時計を差し込む。
  // 準備の間は resume() で実時間どおりに流す。
  await page.clock.install();
  await page.clock.resume();
  await page.addInitScript(mock({ theme, history, reply, characterId, layout, live2dSdkOverrides, initDelayMs, bootHoldMs, onboardingDone, voice, gazeTrackingEnabled, betaBuild }));
  if (forcePng) {
    await page.route(`**/assets/characters/${characterId}/character.json*`, async route => {
      const response = await route.fetch();
      const manifest = await response.json();
      delete manifest.live2d;
      for (const variant of Object.values(manifest.variants || {})) delete variant.live2d;
      await route.fulfill({ response, json: manifest });
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

const MATRIX_LAYOUTS = {
  mascot: [360, 560], pet: [900, 600], chat: [520, 760],
  bar: [900, 320], workspace: [1400, 850], stream: [1920, 1080], widget: [440, 600],
};

// 検証対象のPNGキャラを全7形式で描画し、表示欠けと初期表示時間を一括検査する。
async function shootCharacterMatrix() {
  const output = join(ROOT, "temp", "character-layout-matrix");
  mkdirSync(output, { recursive: true });
  const baseline = JSON.parse(await readFile(join(dirname(fileURLToPath(import.meta.url)), "performance-baseline.json"), "utf8"));
  const results = [];
  const warnings = [];
  const selectedCharacters = process.env.SHOOT_CHARACTER_IDS
    ? process.env.SHOOT_CHARACTER_IDS.split(",").map(value => value.trim()).filter(Boolean)
    : ["mugi", "rin", "mai", "rin-tha3", "koharu", "sena", "noa", "photoreal-prototype"];
  const selectedLayouts = process.env.SHOOT_LAYOUT_IDS
    ? new Set(process.env.SHOOT_LAYOUT_IDS.split(",").map(value => value.trim()).filter(Boolean))
    : null;
  for (const characterId of selectedCharacters) {
    for (const [layout, [width, height]] of Object.entries(MATRIX_LAYOUTS)) {
      if (selectedLayouts && !selectedLayouts.has(layout)) continue;
      const started = performance.now();
      const page = await openScene({ characterId, layout, width, height, forcePng: true });
      try {
        await page.waitForFunction(() => {
          const image = document.getElementById("petSprite");
          return image?.complete && image.naturalWidth > 0 && getComputedStyle(image).display !== "none";
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
          return {
            naturalWidth: image.naturalWidth, naturalHeight: image.naturalHeight,
            characterDataset: { ...character.dataset },
            imageRect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
            imageStyle: { width: imageStyle.width, height: imageStyle.height, left: imageStyle.left, inset: imageStyle.inset, transform: imageStyle.transform },
            viewStyle: viewStyle ? { transform: viewStyle.transform, scale: viewStyle.scale } : null,
            springMotion: springRoot ? {
              visible: !springRoot.hidden && springRootStyle.display !== "none",
              configuredRegions: Array.isArray(characterManifest?.springPhysics?.regions)
                ? characterManifest.springPhysics.regions.length
                : characterManifest?.secondaryMotion?.type === "bust" ? 1 : 0,
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
          };
        });
        if (visual.imageRect.width <= 0 || visual.imageRect.height <= 0
          || visual.imageRect.x >= width || visual.imageRect.y >= height
          || visual.imageRect.x + visual.imageRect.width <= 0 || visual.imageRect.y + visual.imageRect.height <= 0) {
          throw new Error(`${characterId}/${layout}: キャラクターが画面内に描画されていません ${JSON.stringify({ imageRect: visual.imageRect, characterRect: visual.characterRect, imageStyle: visual.imageStyle, viewStyle: visual.viewStyle })}`);
        }
        const expectedSpringRegions = ["chat", "bar"].includes(layout)
          ? 0
          : visual.springMotion?.configuredRegions || 0;
        if (expectedSpringRegions && (!visual.springMotion?.visible
          || visual.springMotion.regionCount !== expectedSpringRegions
          || visual.springMotion.transformedRegions !== expectedSpringRegions
          || visual.springMotion.maskedRegions !== expectedSpringRegions)) {
          throw new Error(`${characterId}/${layout}: 局所バネ物理が適用されていません ${JSON.stringify(visual.springMotion)}`);
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

async function validateLayoutEditor() {
  const output = join(ROOT, "temp", "layout-editor-qa");
  mkdirSync(output, { recursive: true });
  const page = await openScene({ characterId: "mugi", layout: "stream", width: 1280, height: 720, forcePng: true });
  try {
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
    const scaleHandle = page.locator(".moveable-control.moveable-ne");
    const scaleBox = await scaleHandle.boundingBox();
    if (!scaleBox) throw new Error("Moveableの拡大縮小ハンドルが見つかりません");
    await page.mouse.move(scaleBox.x + scaleBox.width / 2, scaleBox.y + scaleBox.height / 2);
    await page.mouse.down();
    await page.mouse.move(scaleBox.x + scaleBox.width / 2 - 60, scaleBox.y + scaleBox.height / 2 + 60, { steps: 8 });
    await page.mouse.up();
    const scaled = await page.locator("#character").evaluate(node => ({ translate: node.style.translate, scale: node.style.scale }));
    if (!scaled.scale) throw new Error(`Moveableの拡大縮小が反映されませんでした: ${JSON.stringify(scaled)}`);
    await page.locator("#layoutEditorSave").click();
    const saved = await page.evaluate(() => JSON.parse(localStorage.getItem("picoagent.layoutEditor.v1") || "{}"));
    if (!saved.stream?.character?.translate || !saved.stream?.character?.scale) throw new Error("表示形式ごとの編集値が保存されませんでした");
    await page.reload();
    await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 20000 });
    const restored = await page.locator("#character").evaluate(node => ({ translate: node.style.translate, scale: node.style.scale }));
    if (restored.translate !== saved.stream.character.translate || restored.scale !== saved.stream.character.scale) {
      throw new Error("保存した配置が再読み込み後に復元されませんでした");
    }
    await page.mouse.move(220, 7);
    await page.waitForFunction(() => document.getElementById("topWindowMenu").getBoundingClientRect().top > -1);
    await page.locator("#layoutEditToggle").click();
    await page.locator("#layoutEditorReset").click();
    const reset = await page.evaluate(() => ({
      translate: document.getElementById("character")?.style.translate || "",
      scale: document.getElementById("character")?.style.scale || "",
      saved: JSON.parse(localStorage.getItem("picoagent.layoutEditor.v1") || "{}"),
    }));
    if (reset.translate || reset.scale || reset.saved.stream) throw new Error("既定配置へ戻せませんでした");
    await page.screenshot({ path: join(output, "stream-editor.png"), omitBackground: true });
    return output;
  } finally {
    await page.close();
  }
}

async function validateInteractionMatrix() {
  const output = join(ROOT, "temp", "layout-interaction-matrix");
  mkdirSync(output, { recursive: true });
  const characters = [{ id: "mugi", forcePng: true }];
  if (LIVE2D_REQUIRED.every(path => existsSync(join(ROOT, path)))) {
    characters.push({ id: LIVE2D_CHARACTER, forcePng: false });
  }
  const results = [];
  for (const target of characters) {
    for (const [layout, [width, height]] of Object.entries(MATRIX_LAYOUTS)) {
      const page = await openScene({
        characterId: target.id, layout, width, height, forcePng: target.forcePng,
        initDelayMs: 350, waitForReady: false,
      });
      try {
        const bootVisible = await page.evaluate(() => document.getElementById("petApp")?.classList.contains("is-booting"));
        if (!bootVisible) throw new Error(`${target.id}/${layout}: 起動画面が表示されません`);
        await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), null, { timeout: 40000 });
        if (!target.forcePng) {
          await page.waitForFunction(() => typeof live2dActive !== "undefined" && live2dActive, null, { timeout: 40000 });
        }
        await page.locator("#character").dispatchEvent("pointerdown", { button: 0, pointerType: "mouse" });
        if (await page.evaluate(() => window.__startDraggingCalls) < 1) {
          throw new Error(`${target.id}/${layout}: キャラ画像のドラッグがウィンドウ移動へ渡りません`);
        }
        if (!await page.locator("#petSprite").evaluate((node) => node.hasAttribute("data-tauri-drag-region"))) {
          throw new Error(`${target.id}/${layout}: キャラ画像にTauriドラッグ領域がありません`);
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
        if (!intersects(state.beta) || state.betaPointerEvents !== "none") {
          throw new Error(`${target.id}/${layout}: BETA表示が画面外または操作を遮っています ${JSON.stringify(state.beta)}`);
        }
        if (!intersects(state.bubble) || !intersects(state.character) || !intersects(state.composer) || !intersects(state.input)) {
          throw new Error(`${target.id}/${layout}: 縮小後に主要UIが画面から消えました ${JSON.stringify(state)}`);
        }
        if (!state.buttons.length || state.buttons.some(button => !intersects(button))) {
          throw new Error(`${target.id}/${layout}: 入力操作ボタンが画面外です ${JSON.stringify(state.buttons)}`);
        }
        const file = `${target.id}-${layout}.png`;
        await page.screenshot({ path: join(output, file), omitBackground: true });
        results.push({ characterId: target.id, layout, bootVisible, resized, state, file });
        console.log(`  操作検査: ${target.id}/${layout}`);
      } finally {
        await page.close();
      }
    }
  }
  const formalPage = await openScene({ characterId: "mugi", forcePng: true, betaBuild: false });
  try {
    const betaVisible = await formalPage.locator(".beta-badge").isVisible();
    if (betaVisible) throw new Error("通常／正式ビルド相当でもBETA表示が残っています");
    console.log("  ビルド種別検査: beta-fullなしではBETA表示を隠す");
  } finally {
    await formalPage.close();
  }
  const reducedPage = await openScene({ characterId: "mugi", forcePng: true, reducedMotion: true });
  try {
    await reducedPage.waitForFunction(() => typeof rigAvailable !== "undefined" && rigAvailable && rigActive, null, { timeout: 20000 });
    const blinkApplied = await reducedPage.evaluate(() => {
      rigBlinkPhase = "blink_closed";
      rigRefresh();
      return petSprite.src.includes("blink_closed");
    });
    if (!blinkApplied) throw new Error("アニメーションを減らす設定でまばたき差分が反映されません");
    const firstTalkingFrame = await reducedPage.evaluate(() => {
      rigBlinkPhase = null;
      setAnimation("talking", true);
      return petSprite.src;
    });
    await reducedPage.waitForFunction(
      (initialSrc) => petSprite.src !== initialSrc,
      firstTalkingFrame,
      { timeout: 2000 },
    );
    console.log("  reduced-motion検査: まばたき・ミュート口パクOK");
  } finally {
    await reducedPage.close();
  }
  const gazeOffPage = await openScene({ characterId: "mugi", forcePng: true, width: 900, height: 600 });
  try {
    await gazeOffPage.waitForFunction(() => typeof rigAvailable !== "undefined" && rigAvailable && rigActive, null, { timeout: 20000 });
    const rect = await gazeOffPage.locator("#character").boundingBox();
    await gazeOffPage.mouse.move(rect.x + rect.width * 0.85, rect.y + rect.height * 0.38);
    const defaultDirection = await gazeOffPage.evaluate(() => rigLookDirection);
    if (defaultDirection !== 0) throw new Error(`既定OFFなのに視線が追従しました: ${defaultDirection}`);
    console.log("  視線設定検査: 既定OFFで正面を維持");
  } finally {
    await gazeOffPage.close();
  }
  const gazePage = await openScene({
    characterId: "mugi", forcePng: true, width: 900, height: 600, gazeTrackingEnabled: true,
  });
  try {
    await gazePage.waitForFunction(() => typeof rigAvailable !== "undefined" && rigAvailable && rigActive, null, { timeout: 20000 });
    const rect = await gazePage.locator("#character").boundingBox();
    await gazePage.mouse.move(450, 590);
    const farDirection = await gazePage.evaluate(() => rigLookDirection);
    if (farDirection !== 0) throw new Error(`画面下の遠いカーソルへ視線が張り付いています: ${farDirection}`);
    await gazePage.mouse.move(rect.x + rect.width * 0.85, rect.y + rect.height * 0.38);
    const nearDirection = await gazePage.evaluate(() => rigLookDirection);
    if (nearDirection !== 1) throw new Error(`顔付近の右カーソルへ視線が追従しません: ${nearDirection}`);
    await gazePage.waitForTimeout(800);
    const resetDirection = await gazePage.evaluate(() => rigLookDirection);
    if (resetDirection !== 0) throw new Error(`視線が700ms後に正面へ戻りません: ${resetDirection}`);
    console.log("  視線設定検査: ONでは顔周辺だけ追従し、画面下では正面、700ms後に復帰");
  } finally {
    await gazePage.close();
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

async function validateLive2dMatrix() {
  const output = join(ROOT, "temp", "live2d-layout-matrix");
  mkdirSync(output, { recursive: true });
  const index = JSON.parse(await readFile(join(ROOT, "characters", "index.json"), "utf8"));
  const targets = [];
  const skipped = [];
  const selectedLive2d = process.env.SHOOT_LIVE2D_IDS
    ? new Set(process.env.SHOOT_LIVE2D_IDS.split(",").map(value => value.trim()).filter(Boolean))
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
    for (const [layout, [width, height]] of Object.entries(MATRIX_LAYOUTS)) {
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
        const file = `${target.id}-${layout}.png`;
        await page.screenshot({ path: join(output, file), omitBackground: true });
        results.push({ characterId: target.id, layout, ...info, file });
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
    if (step) await step(page, index);
    await page.clock.runFor(Math.round(gap));
    frames.push(await page.screenshot({ omitBackground: true }));
  }
  return frames;
}

// 連番 PNG をアニメーション WebP へ束ねる。GIF と違って透過が 1bit に落ちないので、
// 背景なしのまま README のライト / ダーク両方で使える。同じ絵で APNG は 4.8MB、WebP は 0.9MB。
async function encodeAnimation(frames, box, { fps, width, file }) {
  if (width && box.width < width) {
    throw new Error(`入力幅${box.width}pxを出力幅${width}pxへ引き伸ばす処理は禁止です: ${file}`);
  }
  const dir = await mkdtemp(join(tmpdir(), "picoagent-shot-"));
  try {
    for (const [index, png] of frames.entries()) {
      await writeFile(join(dir, `f-${String(index).padStart(4, "0")}.png`), png);
    }
    const crop = `crop=${box.width}:${box.height}:${box.x}:${box.y}`;
    await run(FFMPEG, [
      "-y", "-loglevel", "error",
      "-framerate", String(fps),
      "-i", join(dir, "f-%04d.png"),
      "-vf", width ? `${crop},scale=${width}:-1:flags=lanczos` : crop,
      // compression_level は既定の 4 のまま。6 にすると探索が跳ね上がり、数十コマで数分かかる。
      "-c:v", "libwebp_anim", "-loop", "0", "-q:v", "80", file,
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
  seconds = 6, fps = 30, outputWidth, prepare, step, verifyLipSync = false, ...scene
} = {}) {
  const page = await openScene(scene);
  if (prepare) await prepare(page);
  const frames = await record(page, { seconds, fps, step });
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

  await encodeAnimation(frames, box, { fps, width: outputWidth, file: join(OUT, name) });
  console.log(`撮影: ${name}（${frames.length} コマ / ${fps} fps）`);
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
  "ui/vendor/live2d/pixi.min.js",
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

const DISTRIBUTION_TOUR = [
  [1600, "thinking"],
  [3100, "success"],
  [3900, "idle"],
];

const distributionStep = async (page, index) => {
  const elapsedMs = (index * 1000) / 24;
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
// 公開用は実画面を2倍密度で描画してから960pxへ縮小する。480px撮影を960pxへ
// 引き伸ばすのではなく、PNG／Live2Dとも高密度な実レンダリングを入力にする。
const GALLERY_SCENE = {
  layout: "pet", width: 900, height: 600, deviceScaleFactor: 2,
  seconds: 12, fps: 30, outputWidth: 960, voice: true,
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

// 毎コマ呼んで構わない。`setAnimation()` は同じ状態なら早期 return するので、
// 読み上げ終了でアプリ側が idle へ戻しても次のコマで巡回中の状態へ復帰する。
const galleryStep = async (page, index) => {
  const elapsedMs = (index * 1000) / GALLERY_SCENE.fps;
  const current = GALLERY_TOUR.filter(([at]) => elapsedMs >= at).pop();
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

const galleryPrepare = kind => kind === "live2d"
  ? prepareGalleryLive2D
  : startReply(GALLERY_ASK);

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
  ["koharu", "おはよう。今日のこと、気軽に話してね。"],
  ["sena", "おはよう。今日のステージも、きれいに仕上げようか。"],
  ["noa", "べ、別に待ってたわけじゃないけど……用件は聞いてあげる。"],
  ["rin-tha3", "ふ、ふん。来たのね。今日の作業くらい見ててあげる。"],
  ["photoreal-prototype", "こんにちは。焦らず、今日やることから一緒に整理しましょう。"],
  ["hiyori", "やっほー！今日はなにする？なんでも言ってね、いっしょにがんばろ！", "live2d"],
  ["haru", "こんにちは。今日の作業を落ち着いて進めましょう。", "live2d"],
  ["epsilon", "動作確認でも、作業の整理でも、必要なところから始めましょう。", "live2d"],
  ["mao", "こんにちは。今日は何から片づけましょうか。ゆっくりで大丈夫ですよ。", "live2d"],
  ["ren", "準備はできています。どこから手をつけますか。", "live2d"],
  ["rice", "よく来た。……さて、何を調べようか。", "live2d"],
  ["wanko", "わふっ！　……よし、いっしょにやろう！", "live2d"],
];

const GALLERY_SHOTS = Object.fromEntries(GALLERY.map(([id, reply, kind = "png"]) => [
  `character-${id}.webp`,
  {
    ...GALLERY_SCENE,
    characterId: id,
    reply,
    prepare: galleryPrepare(kind),
    step: galleryStep,
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
    // 配信画面はフルHDのネイティブ描画から960pxへ縮小すれば
    // 引き伸ばしにならない。2倍密度の3840px全画面キャプチャは不要な負荷を避ける。
    deviceScaleFactor: 1,
    characterId: id,
    reply,
    prepare: galleryPrepare(kind),
    step: galleryStep,
    verifyLipSync: id !== "rice",
    requires: galleryRequires(id, kind),
  },
]));

// ---------------------------------------------------------------- 撮る絵の一覧

const SHOTS = {
  ...GALLERY_SHOTS,
  ...GALLERY_STREAM_SHOTS,
  "main-light.webp": { prepare: startReply() },
  "main-dark.webp": { theme: "dark", prepare: startReply() },
  "composer-light.webp": {
    reply: COMPOSER_REPLY,
    prepare: prepareComposer,
    step: typeIntoInput(COMPOSER_TYPING),
  },
  "fullscreen-light.webp": { width: 900, height: 600, prepare: prepareFullscreen },
  "live2d-light.webp": { characterId: LIVE2D_CHARACTER, requires: LIVE2D_REQUIRED, prepare: prepareLive2D },
  "live2d-dark.webp": { characterId: LIVE2D_CHARACTER, theme: "dark", requires: LIVE2D_REQUIRED, prepare: prepareLive2D },
  "distribution-main.webp": { width: 900, height: 600, layout: "pet", seconds: 7, fps: 30, outputWidth: 640, voice: true, prepare: prepareDistribution2D, step: distributionStep },
  "distribution-live2d.webp": { width: 900, height: 600, layout: "pet", seconds: 7, fps: 30, outputWidth: 640, voice: true, characterId: LIVE2D_CHARACTER, requires: LIVE2D_REQUIRED, prepare: prepareDistributionLive2D, step: distributionStep },
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

mkdirSync(OUT, { recursive: true });
const only = process.argv.slice(2);
const skipped = [];
let failed = 0;

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

if (process.env.SHOOT_LIVE2D_MATRIX) {
  const matrix = await validateLive2dMatrix();
  await trimmer.close();
  await browser.close();
  server.close();
  console.log(`Live2D表示検査完了: ${matrix.output}（${matrix.results.length}件、未取得${matrix.skipped.length}件）`);
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
}

if (process.env.SHOOT_DEBUG_ONBOARDING) {
  const page = await openScene({ width: 680, height: 430, onboardingDone: false });
  await page.locator('button[data-mode="both"]').click();
  await page.waitForSelector("#onboardingComponentStep:not([hidden])");
  await page.waitForTimeout(300);
  const debugDir = join(ROOT, "temp", "onboarding-check");
  mkdirSync(debugDir, { recursive: true });
  await page.screenshot({ path: join(debugDir, "components.png"), omitBackground: true });
  await page.close();
  console.log(`  初回追加機能確認: ${join(debugDir, "components.png")}`);
}

const wantsLive2d = (!only.length || only.some(name => name.startsWith("live2d-")))
  && LIVE2D_REQUIRED.every(path => existsSync(join(ROOT, path)));
if (wantsLive2d) await validateLive2dSdkModels();

for (const [name, { requires = [], ...options }] of Object.entries(SHOTS)) {
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
if (failed) process.exitCode = 1;
