// README 用スクリーンショットの撮影ツール。
//
// 実物の ui/ をローカル HTTP で配信し、バックエンド（window.__TAURI__）だけを
// モックしてヘッドレス Chromium で描画する。フロントの JS・CSS・キャラクター画像は
// 本物をそのまま使うため、出てくる絵はモックアップではなく実際の描画結果。
//
// 使い方:
//   PLAYWRIGHT_MODULE=<playwright の index.mjs> node tools/screenshots/shoot.mjs [名前...]
// 詳細は同じフォルダの README.md を参照。
import http from "node:http";
import { readFile } from "node:fs/promises";
import { existsSync, mkdirSync } from "node:fs";
import { extname, isAbsolute, join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

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

// バックエンドのモック。フキダシの寸法は指定しない（tauri.conf.json の 560x430 +
// styles.css の既定レイアウトが本物の見た目になる）。
const mock = ({ theme, history, reply, characterId }) => `
window.__listeners = {};
window.__TAURI__ = {
  core: { invoke: (name) => {
    if (name === "pet_init") return Promise.resolve({
      up: true, providerId: "local", model: "gemma-3-4b-it-Q4_K_M",
      theme: ${JSON.stringify(theme)}, fontPx: "13", voiceVolume: "0",
      characterId: ${JSON.stringify(characterId)}, characterOverrides: {},
      history: ${JSON.stringify(history)},
      quickActions: "", napMinutes: "0",
      chatterEnabled: false, chatterMinSecs: "9999", chatterMaxSecs: "9999",
    });
    if (name === "engine_status") return Promise.resolve({ phase: "ready" });
    if (name === "pet_speak") return Promise.resolve({ enabled: false });
    if (name === "pet_send") return new Promise(r => setTimeout(() => r({ reply: ${JSON.stringify(reply)} }), 80));
    if (name === "pet_alarms") return Promise.resolve({ alarms: [] });
    if (name === "pet_daily_events") return Promise.resolve({ events: [] });
    return Promise.resolve({});
  } },
  event: { listen: (ch, cb) => { const a = (window.__listeners[ch] ||= []); a.push(cb);
    return Promise.resolve(() => { const i = a.indexOf(cb); if (i >= 0) a.splice(i, 1); }); } },
  window: { getCurrentWindow: () => ({ setFullscreen: async () => {}, setSize: async () => {},
    scaleFactor: async () => 1, outerPosition: async () => ({ x: 0, y: 0 }), setPosition: async () => {},
    startDragging: async () => {} }) },
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
async function trim(file) {
  const png = await readFile(file);
  const page = await trimmer.newPage();
  const out = await page.evaluate(async (url) => {
    const img = new Image();
    await new Promise((res, rej) => { img.onload = res; img.onerror = rej; img.src = url; });
    const c = document.createElement("canvas");
    c.width = img.width; c.height = img.height;
    const ctx = c.getContext("2d");
    ctx.drawImage(img, 0, 0);
    const { data } = ctx.getImageData(0, 0, c.width, c.height);
    let top = c.height, left = c.width, right = 0, bottom = 0;
    for (let y = 0; y < c.height; y++) {
      for (let x = 0; x < c.width; x++) {
        if (data[(y * c.width + x) * 4 + 3] > 8) {
          if (y < top) top = y; if (y > bottom) bottom = y;
          if (x < left) left = x; if (x > right) right = x;
        }
      }
    }
    if (right < left || bottom < top) return null;
    const pad = 8;
    top = Math.max(0, top - pad); left = Math.max(0, left - pad);
    right = Math.min(c.width - 1, right + pad); bottom = Math.min(c.height - 1, bottom + pad);
    const w = right - left + 1, h = bottom - top + 1;
    const o = document.createElement("canvas");
    o.width = w; o.height = h;
    o.getContext("2d").drawImage(c, left, top, w, h, 0, 0, w, h);
    return o.toDataURL("image/png");
  }, "data:image/png;base64," + png.toString("base64"));
  await page.close();
  if (!out) throw new Error(`不透明な領域がありません: ${file}`);
  await import("node:fs/promises").then(m => m.writeFile(file, Buffer.from(out.split(",")[1], "base64")));
}

// 既定のウィンドウサイズは tauri.conf.json の値。実物と同じ比率で撮る。
async function shoot(name, {
  width = 560, height = 430, theme = "light", history = HISTORY,
  ask = ASK, reply = REPLY, characterId = "mugi", prepare,
} = {}) {
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 2 });
  await page.addInitScript(mock({ theme, history, reply, characterId }));
  page.on("pageerror", error => console.error(`  [pageerror] ${error.message}`));
  await page.goto(`http://127.0.0.1:${PORT}/index.html`);
  await page.waitForFunction(() => !document.getElementById("petApp")?.classList.contains("is-booting"), { timeout: 20000 });

  // 起動時の挨拶ではなく、直近のやり取りが写るように1往復させる。
  await page.evaluate((text) => {
    if (typeof setInputValue === "function") setInputValue(text);
    document.getElementById("composer")?.dispatchEvent(new Event("submit", { cancelable: true }));
  }, ask);
  // 文字送りが最後まで終わってから撮る（途中で切れた文が写らないように）。
  // 本文は Markdown として描画されるので、記号（`code` のバッククォート等）と空白は無視して比べる。
  await page.waitForFunction((full) => {
    const strip = (s) => s.replace(/[\s`*_]+/g, "");
    const el = document.getElementById("reply");
    return el && strip(el.textContent) === strip(full);
  }, reply, { timeout: 20000 });
  await page.waitForTimeout(400);
  // 一時的なステータス表示（読み上げ準備中など）は消してから撮る。
  await page.evaluate(() => { const s = document.getElementById("status"); if (s) s.textContent = ""; });

  if (prepare) await prepare(page);
  await page.waitForTimeout(600);
  const file = join(OUT, name);
  await page.screenshot({ path: file, omitBackground: true });
  await trim(file);
  await page.close();
  console.log(`撮影: ${name}`);
}

// 入力欄を開くとフキダシ本文が compact 表示になるので、短い応答で撮る（途中で切れた行が写らないように）。
async function prepareComposer(page) {
  // フロントの openComposer() は応答中だと何もしないので、busy が下りるのを待ってから開く。
  await page.waitForFunction(() => typeof busy === "undefined" || busy === false, { timeout: 10000 });
  await page.evaluate(() => {
    const composer = document.getElementById("composer");
    if (typeof openComposer === "function") openComposer();
    if (composer && composer.hidden) {
      composer.hidden = false;
      document.getElementById("reply")?.classList.add("compact");
    }
  });
  await page.evaluate(() => {
    const input = document.getElementById("input");
    if (!input) return;
    input.value = "この PSD を 800px の PNG にして";
    input.dispatchEvent(new Event("input"));
    input.focus();
  });
}

// Live2D は WebGL 描画。ランタイムとモデルは cargo xtask setup live2d で取得する。
const LIVE2D_CHARACTER = "hiyori";
const LIVE2D_REQUIRED = [
  "ui/vendor/live2d/live2dcubismcore.min.js",
  "ui/vendor/live2d/pixi.min.js",
  "ui/vendor/live2d/cubism4.min.js",
  `ui/assets/characters/${LIVE2D_CHARACTER}/live2d`,
];

async function waitForLive2D(page) {
  await page.waitForFunction(() => {
    const canvas = document.getElementById("live2dCanvas");
    return canvas && canvas.style.display !== "none" && canvas.width > 0 && typeof live2dActive !== "undefined" && live2dActive;
  }, { timeout: 40000 });
  // 初回モーションが動き出してポーズが決まるまで少し待つ。
  await page.waitForTimeout(1500);
}

// ---------------------------------------------------------------- 撮る絵の一覧

const SHOTS = {
  "main-light.png": {},
  "main-dark.png": { theme: "dark" },
  "composer-light.png": {
    ask: "この PDF、2〜3ページだけ抜き出せる？",
    reply: "できたよ。`資料_p2-3.pdf` として同じフォルダに保存したよ。",
    prepare: prepareComposer,
  },
  "fullscreen-light.png": {
    width: 900, height: 600,
    prepare: async (page) => {
      // 全画面フキダシは bubble に .fullscreen が付いた状態（Tauri の setFullscreen はモック）。
      await page.evaluate(() => { if (typeof toggleBubbleFullscreen === "function") toggleBubbleFullscreen(); });
      // 履歴は最新のやり取りが見えている状態で撮る。
      await page.waitForTimeout(300);
      await page.evaluate(() => { const l = document.getElementById("historyLog"); if (l) l.scrollTop = l.scrollHeight; });
    },
  },
  "live2d-light.png": { characterId: LIVE2D_CHARACTER, requires: LIVE2D_REQUIRED, prepare: waitForLive2D },
  "live2d-dark.png": { characterId: LIVE2D_CHARACTER, theme: "dark", requires: LIVE2D_REQUIRED, prepare: waitForLive2D },
};

// ---------------------------------------------------------------- 実行

mkdirSync(OUT, { recursive: true });
const only = process.argv.slice(2);
const skipped = [];
let failed = 0;

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
