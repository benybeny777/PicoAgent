// ui-check.mjs - メイン画面を実ブラウザで描き、シーン方式の約束を検査する（作業ツール。アプリの依存ではない）。
//
// 実物の ui/ をローカル HTTP で配信し、バックエンド（window.__TAURI__）だけをモックする。検査すること:
//   1. 3形式（ペット・ミニ・配信）で起動が完了し、キャラが描かれる。
//   2. キャラが切れない: キャラ矩形の上端・左右端に描画が接していない（全身構図は下端も含む）。
//      最小・既定・大きく拡大した3つの大きさで確かめる。
//   3. 最小サイズ（200×240）でも入力欄・送信・状態表示・ウィンドウ操作が窓の内側に完全に収まる。
//   4. 右クリックはどこでも pet_menu を呼び、文脈（選択・入力欄・リンク・キャラ）だけを渡す。
//   5. 配置の保存は pet_save_scene だけを通り、倍率を持たない矩形で届く。
// 使い方:
//   PLAYWRIGHT_MODULE=$(npm root -g)/playwright/index.mjs node tools/screenshots/ui-check.mjs
// 結果の画像と JSON は temp/ui-check/ へ書く。失敗があれば終了コード1。

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const uiDir = path.join(root, "ui");
const outDir = path.join(root, "temp", "ui-check");
fs.mkdirSync(outDir, { recursive: true });
const playwrightModule = process.env.PLAYWRIGHT_MODULE;
const { chromium } = await import(playwrightModule ? pathToFileURL(playwrightModule).href : "playwright");

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".json": "application/json", ".png": "image/png", ".webp": "image/webp", ".svg": "image/svg+xml", ".wasm": "application/wasm" };
const server = http.createServer((request, response) => {
  const url = new URL(request.url, "http://localhost");
  const file = path.join(uiDir, decodeURIComponent(url.pathname === "/" ? "/index.html" : url.pathname));
  if (!file.startsWith(uiDir) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    response.writeHead(404);
    response.end("not found");
    return;
  }
  response.writeHead(200, { "Content-Type": TYPES[path.extname(file)] || "application/octet-stream", "Cache-Control": "no-store" });
  fs.createReadStream(file).pipe(response);
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const base = `http://127.0.0.1:${server.address().port}/`;

function mockScript({ layout, characterId = "mugi", scenes = {}, onboardingDone = true, history = [] }) {
  return `
window.__calls = [];
window.__listeners = {};
window.__saved = ${JSON.stringify(scenes)};
window.__TAURI__ = {
  core: { invoke: (name, args = {}) => {
    window.__calls.push({ name, args: JSON.parse(JSON.stringify(args)) });
    if (name === "pet_init") return Promise.resolve({
      up: true, providerId: "local", theme: "system", uiTheme: "glass", fontPx: "14", voiceVolume: "35",
      characterId: ${JSON.stringify(characterId)}, characterOverrides: {}, live2dSdkOverrides: {},
      layout: ${JSON.stringify(layout)}, scenes: window.__saved, history: ${JSON.stringify(history)},
      quickActions: "", napMinutes: "0", onboardingDone: ${JSON.stringify(onboardingDone)},
      relationshipMemoryEnabled: true, relationship: {}, bondLabel: "いっしょの相棒",
      gazeTrackingEnabled: false, bootProgressIntervalMs: 10000, ttsEnabled: false,
      streamBackground: "pastel", streamDisplay: "dashboard", streamCaptionColor: "#5a3b4b",
      chatterEnabled: false, chatterMinSecs: "9999", chatterMaxSecs: "9999", betaBuild: false,
    });
    if (name === "pet_save_scene") { window.__saved = args.scenes; return Promise.resolve(); }
    if (name === "engine_status") return Promise.resolve({ phase: "ready" });
    if (name === "aivis_status") return Promise.resolve({ enabled: false });
    if (name === "pet_speak") return Promise.resolve({ enabled: false });
    if (name === "pet_send") return new Promise(resolve => setTimeout(() => resolve({ reply: "了解だよ。**太字**と [リンク](https://example.com) も出せるよ。" }), 200));
    if (name === "pet_alarms") return Promise.resolve({ alarms: [] });
    if (name === "pet_daily_events") return Promise.resolve({ events: [] });
    if (name === "pet_user_characters") return Promise.resolve({ characters: [] });
    if (name === "pet_set_character_profile") return Promise.resolve({ streamBackground: "pastel", streamDisplay: "dashboard", streamCaptionColor: "#5a3b4b" });
    if (name === "pet_config_presence") return Promise.resolve({});
    if (name === "pet_schedule_runs" || name === "pet_durable_jobs") return Promise.resolve([]);
    if (name === "settings_components_status") return Promise.resolve([]);
    return Promise.resolve({});
  } },
  event: { listen: (channel, callback) => {
    const list = (window.__listeners[channel] ||= []);
    list.push(callback);
    return Promise.resolve(() => { const index = list.indexOf(callback); if (index >= 0) list.splice(index, 1); });
  } },
  window: { getCurrentWindow: () => ({
    setFullscreen: async () => {}, setSize: async () => {}, scaleFactor: async () => 1,
    outerPosition: async () => ({ x: 0, y: 0 }), setPosition: async () => {},
    startResizeDragging: async () => {}, minimize: async () => {}, toggleMaximize: async () => {}, close: async () => {},
  }) },
  dpi: {},
};
window.__emit = (channel, payload) => (window.__listeners[channel] || []).forEach(callback => callback({ payload }));
`;
}

const browser = await chromium.launch({
  executablePath: process.env.PLAYWRIGHT_CHROMIUM || undefined,
  args: ["--enable-unsafe-swiftshader", "--use-gl=swiftshader", "--ignore-gpu-blocklist"],
});
const failures = [];
const report = [];
const fail = (label, detail) => { failures.push(`${label}: ${detail}`); console.error(`NG ${label}: ${detail}`); };
const ok = label => console.log(`OK ${label}`);

async function openApp({ layout, width, height, characterId, scenes, onboardingDone, history }) {
  const context = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(String(error)));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(mockScript({ layout, characterId, scenes, onboardingDone, history }));
  await page.goto(base);
  await page.waitForFunction(() => !document.getElementById("petApp").classList.contains("is-booting"), null, { timeout: 30000 });
  await page.waitForFunction(() => {
    const canvas = document.querySelector("[data-source=character] canvas");
    return canvas && canvas.width > 1 && canvas.height > 1;
  }, null, { timeout: 30000 });
  await page.waitForTimeout(900);
  return { context, page, errors };
}

/** キャラ矩形の画素を撮り、上端・左右端・下端に描画が接しているかを返す。 */
async function edgeContact(page) {
  const box = await page.locator("[data-source=character]").boundingBox();
  // キャラの描画だけを撮る（他のソース・枠・キャラ上の小物を一時的に隠す）。
  const style = await page.addStyleTag({ content: ".source:not([data-source=character]), #editLayer, #topWindowMenu, .relationship-button, .online-dot, .beta-badge { visibility: hidden; }" });
  const png = await page.screenshot({ clip: box, omitBackground: true });
  await style.evaluate(node => node.remove());
  const probe = await browser.newPage();
  const result = await probe.evaluate(async url => {
    const image = new Image();
    await new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; image.src = url; });
    const canvas = document.createElement("canvas");
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext("2d");
    context.drawImage(image, 0, 0);
    const { data } = context.getImageData(0, 0, canvas.width, canvas.height);
    const opaque = (x, y) => data[(y * canvas.width + x) * 4 + 3] > 40;
    const row = y => { let count = 0; for (let x = 0; x < canvas.width; x++) if (opaque(x, y)) count++; return count; };
    const column = x => { let count = 0; for (let y = 0; y < canvas.height; y++) if (opaque(x, y)) count++; return count; };
    let total = 0, minX = canvas.width, minY = canvas.height, maxX = -1, maxY = -1;
    for (let y = 0; y < canvas.height; y += 2) for (let x = 0; x < canvas.width; x += 2) if (opaque(x, y)) {
      total++;
      minX = Math.min(minX, x); minY = Math.min(minY, y);
      maxX = Math.max(maxX, x); maxY = Math.max(maxY, y);
    }
    return { top: row(0), bottom: row(canvas.height - 1), left: column(0), right: column(canvas.width - 1), total, width: canvas.width, height: canvas.height, minX, minY, maxX, maxY };
  }, `data:image/png;base64,${png.toString("base64")}`);
  await probe.close();
  return { ...result, box, opaque: {
    left: box.x + result.minX / result.width * box.width,
    right: box.x + (result.maxX + 1) / result.width * box.width,
    top: box.y + result.minY / result.height * box.height,
    bottom: box.y + (result.maxY + 1) / result.height * box.height,
  } };
}

/** 要素が窓の内側に完全に収まっているか。 */
async function fullyInside(page, selector) {
  return page.evaluate(sel => {
    const element = document.querySelector(sel);
    if (!element || element.hidden || !element.getClientRects().length) return { present: false };
    const rect = element.getBoundingClientRect();
    return {
      present: true,
      inside: rect.left >= -0.5 && rect.top >= -0.5 && rect.right <= innerWidth + 0.5 && rect.bottom <= innerHeight + 0.5,
      rect: { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom },
    };
  }, selector);
}

const LAYOUTS = { pet: [900, 600], mascot: [360, 560], stream: [1280, 720] };
const characters = (process.env.UI_CHECK_CHARACTERS || "mugi,rin,photoreal-prototype").split(",");
const layoutIds = (process.env.UI_CHECK_LAYOUTS || "pet,mascot,stream").split(",");

for (const characterId of characters) {
  for (const layout of layoutIds) {
    const [width, height] = LAYOUTS[layout];
    const label = `${characterId}/${layout}`;
    const { context, page, errors } = await openApp({ layout, width, height, characterId });
    const kind = await page.evaluate(() => document.querySelector("[data-source=character] .character-view")?.dataset.renderKind);
    const framing = await page.evaluate(() => document.querySelector("[data-source=character] canvas")?.dataset.framing || "");
    const edges = await edgeContact(page);
    await page.screenshot({ path: path.join(outDir, `${characterId}-${layout}.png`), omitBackground: true });
    report.push({ label, kind, framing, edges });
    if (edges.total < 50) fail(label, `キャラが描かれていません（不透明画素 ${edges.total}）`);
    else if (edges.top > 0) fail(label, `キャラの上端が切れています（上端の接触 ${edges.top}px）`);
    else ok(`${label} 描画（${kind}）`);
    if (errors.length) fail(label, `画面エラー: ${errors.slice(0, 3).join(" / ")}`);
    if (layout === "stream") {
      // 字幕を隠した状態でも胴体の切れ目を字幕に頼らず画面下端へ置く。
      const hiddenSubtitle = await page.addStyleTag({ content: '[data-source="subtitle"] { visibility: hidden !important; }' });
      const bareCharacter = await edgeContact(page);
      await hiddenSubtitle.evaluate(node => node.remove());
      if (Math.abs(bareCharacter.box.y + bareCharacter.box.height - height) > 1 || bareCharacter.bottom === 0) {
        fail(label, `字幕なしではキャラが下端まで続きません ${JSON.stringify(bareCharacter)}`);
      } else ok(`${label} 字幕なしでもキャラが画面下端へ続く`);
      if (bareCharacter.top > 0 || bareCharacter.left > 0 || bareCharacter.right > 0) {
        fail(label, `胸上構図の上端・左右に余白がありません ${JSON.stringify(bareCharacter)}`);
      } else ok(`${label} 胸上構図の上端・左右に余白がある`);
      const faceClearance = await page.evaluate(() => {
        const character = document.querySelector('[data-source="character"]')?.getBoundingClientRect();
        const subtitle = document.querySelector('[data-source="subtitle"]')?.getBoundingClientRect();
        if (!character || !subtitle) return null;
        const horizontalOverlap = character.left < subtitle.right && subtitle.left < character.right;
        // MMD胸上は頭頂余白4%と目の深さ最大20%を含み、顎までの余裕を足して枠の上70%を守る。
        return { horizontalOverlap, faceBottom: character.top + character.height * 0.7, subtitleTop: subtitle.top };
      });
      if (!faceClearance || (faceClearance.horizontalOverlap && faceClearance.faceBottom > faceClearance.subtitleTop + 1)) {
        fail(label, `字幕が胸上の顔領域に重なります ${JSON.stringify(faceClearance)}`);
      } else ok(`${label} 字幕は胸上の顔領域に重ならない`);
      const streamUi = await page.evaluate(() => {
        const chat = document.querySelector("[data-source=chat]")?.getBoundingClientRect();
        const display = document.querySelector("[data-source=display]")?.getBoundingClientRect();
        const subtitle = document.querySelector("[data-source=subtitle]")?.getBoundingClientRect();
        const form = document.querySelector("#composer");
        const input = document.querySelector("#input")?.getBoundingClientRect();
        return { chat: chat && { bottom: chat.bottom }, display: display && { width: display.width },
          subtitle: subtitle && { width: subtitle.width }, input: input && { bottom: input.bottom },
          formVisible: Boolean(form && !form.hidden) };
      });
      if (!streamUi.formVisible || !streamUi.input || streamUi.input.bottom > streamUi.chat?.bottom) {
        fail(label, `履歴列の入力欄が見えません ${JSON.stringify(streamUi)}`);
      } else ok(`${label} 履歴列の入力欄を常時表示`);
      if (!streamUi.subtitle || streamUi.subtitle.width < 700 || !streamUi.display || streamUi.display.width > 320) {
        fail(label, `字幕帯とディスプレイの寸法が違います ${JSON.stringify(streamUi)}`);
      } else ok(`${label} 横長の字幕帯と控えめなディスプレイ`);
      const overlaps = await page.evaluate(character => {
        // 字幕帯は仕様上キャラの手前に重なる。ディスプレイとチャット列だけを除外する。
        return ["display", "chat"].filter(name => {
          const other = document.querySelector(`[data-source=${name}]`)?.getBoundingClientRect();
          return other && character.left < other.right && other.left < character.right
            && character.top < other.bottom && other.top < character.bottom;
        });
      }, edges.opaque);
      if (overlaps.length) fail(label, `キャラ矩形が他のソースと重なっています: ${overlaps.join(", ")}`);
      else ok(`${label} 配信ソースと非重複`);
      if (characterId === "mugi") {
        await page.fill("#input", "配信から送信します");
        const send = await page.evaluate(() => {
          const button = document.querySelector("#sendButton");
          const style = getComputedStyle(button);
          return { disabled: button.disabled, background: style.backgroundImage, font: parseFloat(getComputedStyle(document.querySelector('.chat[data-variant="column"]')).fontSize) };
        });
        if (send.disabled || send.background === "none" || send.font < 26) {
          fail(`${label}/chat`, `配信の入力・送信操作が見づらいか使えません ${JSON.stringify(send)}`);
        } else ok(`${label} 読める履歴文字と押せる送信ボタン`);
        await page.fill("#input", "");
        const subtitle = page.locator(".subtitle .reply");
        await subtitle.evaluate(node => { node.textContent = "字幕の文字をドラッグしても選択されないか確認します"; });
        const bounds = await subtitle.boundingBox();
        if (!bounds) fail(`${label}/subtitle`, "字幕が表示されません");
        else {
          const y = bounds.y + Math.min(bounds.height / 2, 24);
          await page.mouse.move(bounds.x + 24, y);
          await page.mouse.down();
          await page.mouse.move(bounds.x + Math.min(bounds.width - 24, 480), y, { steps: 8 });
          await page.mouse.up();
          const selected = await page.evaluate(() => String(window.getSelection?.() ?? ""));
          if (selected) fail(`${label}/subtitle`, `ドラッグ後も字幕が選択されています: ${selected}`);
          else ok(`${label} 字幕の文字はドラッグしても選択されない`);
        }
      }
    }

    // 大きく拡大しても切れない（全身構図にして、四辺どこにも接しないことを確かめる）。
    await page.evaluate(() => window.__emit("pet:context-action", { action: "set-framing", framing: "full" }));
    await page.waitForTimeout(500);
    const full = await edgeContact(page);
    if (full.top > 0 || full.left > 0 || full.right > 0) fail(`${label}/full`, `全身構図で端が切れています ${JSON.stringify(full)}`);
    else ok(`${label} 全身構図`);
    // つまみで縮小・拡大する（保存は pet_save_scene を通り、倍率ではなく矩形で届く）。
    for (const factor of [0.5, 1.8]) {
      const box = await page.locator("[data-source=character]").boundingBox();
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      await page.waitForTimeout(150);
      const handle = page.locator('.edit-frame[data-target="character"] .edit-handle[data-corner="top-left"]');
      if (!(await handle.count()) || !(await handle.isVisible())) { fail(label, "キャラに乗せてもつまみが出ません"); break; }
      const hb = await handle.boundingBox();
      const targetX = box.x + box.width - box.width * factor;
      const targetY = box.y + box.height - box.height * factor;
      await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
      await page.mouse.down();
      await page.mouse.move(targetX, targetY, { steps: 6 });
      await page.mouse.up();
      await page.waitForTimeout(600);
      const after = await edgeContact(page);
      const entry = await page.evaluate(id => window.__saved?.[document.getElementById("petApp").dataset.layout]?.characters?.[id], characterId);
      if (!entry || !Number.isFinite(entry.w) || entry.zoom != null || entry.scale != null) fail(`${label}/resize`, `保存値が矩形になっていません ${JSON.stringify(entry)}`);
      if (after.top > 0 || after.left > 0 || after.right > 0) fail(`${label}/resize×${factor}`, `拡大縮小後に端が切れています ${JSON.stringify(after)}`);
      else ok(`${label} 拡大縮小×${factor}（${Math.round(after.width)}×${Math.round(after.height)}）`);
    }
    await page.screenshot({ path: path.join(outDir, `${characterId}-${layout}-resized.png`), omitBackground: true });

    // 右クリックはどこでも同じメニュー（文脈だけを渡す）。
    await page.mouse.click(10, 60); // 何も無い所
    const targets = { character: "[data-source=character]", reply: "#reply", history: ".history-log" };
    for (const [name, selector] of Object.entries(targets)) {
      const element = page.locator(selector);
      if (!(await element.count()) || !(await element.isVisible())) continue;
      await page.evaluate(() => { window.__calls = []; });
      await element.click({ button: "right", force: true });
      const call = await page.evaluate(() => window.__calls.find(entry => entry.name === "pet_menu"));
      if (!call) fail(`${label}/contextmenu`, `${name} の右クリックで pet_menu が呼ばれません`);
      else if (name === "character" && call.args.context?.character !== true) fail(`${label}/contextmenu`, `キャラの文脈が渡りません ${JSON.stringify(call.args)}`);
      else ok(`${label} 右クリック（${name}）`);
    }
    await context.close();
  }
}

// 最小サイズ: 操作要素が窓の内側に完全に収まる。
for (const layout of Object.keys(LAYOUTS)) {
  const label = `min/${layout}`;
  const { context, page } = await openApp({ layout, width: 200, height: 240, characterId: "mugi" });
  await page.evaluate(() => document.querySelector("[data-source=chat]").click());
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(outDir, `min-${layout}.png`), omitBackground: true });
  for (const selector of ["#input", "#sendButton", "#micButton", "#topWindowMenu .window-close", "#topWindowMenu .window-caption-controls button"]) {
    const result = await fullyInside(page, selector);
    if (!result.present) fail(label, `${selector} が表示されていません`);
    else if (!result.inside) fail(label, `${selector} が窓の外へ出ています ${JSON.stringify(result.rect)}`);
  }
  ok(`${label} 最小サイズの収まり`);
  await context.close();
}

// 異常な保存値を表示時だけ収め、読み込みだけでは保存し直さない。
{
  const scenes = { version: 1, pet: { sources: { chat: {
    anchor: "top-left", x: -1000, y: -1000, w: 5000, h: 4000,
  } } } };
  const { context, page } = await openApp({ layout: "pet", width: 900, height: 600, characterId: "mugi", scenes });
  const bounds = await fullyInside(page, "[data-source=chat]");
  const state = await page.evaluate(() => ({ saved: window.__saved, calls: window.__calls.filter(call => call.name === "pet_save_scene") }));
  if (!bounds.inside) fail("saved-overflow", `会話パネルが窓外です ${JSON.stringify(bounds)}`);
  if (JSON.stringify(state.saved) !== JSON.stringify(scenes)) fail("saved-overflow", "表示補正が保存値を書き換えました");
  if (state.calls.length) fail("saved-overflow", "読み込みだけでシーンを保存しました");
  if (bounds.inside && !state.calls.length) ok("巨大な保存値は表示だけ収め、保存値を保つ");
  await context.close();
}

// 拡大を戻すと3形式とも元の会話パネル位置へ戻る。
for (const [layout, [width, height]] of Object.entries(LAYOUTS)) {
  const { context, page } = await openApp({ layout, width, height, characterId: "mugi" });
  const before = await page.locator("[data-source=chat]").boundingBox();
  const expand = page.locator('[aria-label="会話パネルを拡大"]');
  await expand.evaluate(button => button.click());
  await expand.evaluate(button => button.click());
  await page.waitForFunction(original => {
    const rect = document.querySelector('[data-source="chat"]')?.getBoundingClientRect();
    return rect && ["x", "y", "width", "height"].every(key => Math.abs(rect[key] - original[key]) <= 1);
  }, before, { timeout: 2000 }).catch(() => {});
  const after = await page.locator("[data-source=chat]").boundingBox();
  if (["x", "y", "width", "height"].some(key => Math.abs(before[key] - after[key]) > 1)) {
    fail(`expanded/${layout}`, `元の位置へ戻りません ${JSON.stringify({ before, after })}`);
  } else ok(`expanded/${layout} 配置復元`);
  await context.close();
}

// 送信と表示（ストリーミングなし）。
{
  const { context, page, errors } = await openApp({ layout: "pet", width: 900, height: 600, characterId: "mugi" });
  await page.evaluate(() => document.querySelector("[data-source=chat]").click());
  await page.fill("#input", "こんにちは");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.getElementById("reply").textContent.includes("了解だよ"), null, { timeout: 5000 }).then(
    () => ok("送信と回答の表示"),
    () => fail("send", "回答が表示されません"),
  );
  await page.screenshot({ path: path.join(outDir, "pet-conversation.png"), omitBackground: true });
  if (errors.length) fail("send", errors.join(" / "));
  await context.close();
}

fs.writeFileSync(path.join(outDir, "report.json"), JSON.stringify({ report, failures }, null, 2));
await browser.close();
server.close();
if (failures.length) {
  console.error(`\n${failures.length} 件の失敗`);
  process.exit(1);
}
console.log("\nすべて合格");
