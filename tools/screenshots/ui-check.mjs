// ui-check.mjs - メイン画面を実ブラウザで描き、シーン方式の約束を検査する（作業ツール。アプリの依存ではない）。
//
// 実物の ui/ をローカル HTTP で配信し、バックエンド（window.__TAURI__）だけをモックする。検査すること:
//   1. 3形式（ペット・ミニ・配信）で起動が完了し、キャラが描かれる。
//   2. キャラが切れない: キャラ矩形の上端・左右端に描画が接していない（全身構図は下端も含む）。
//      最小・既定・大きく拡大した3つの大きさで確かめる。
//   3. 最小サイズ（200×240）でも入力欄・送信・状態表示・ウィンドウ操作が窓の内側に完全に収まり、
//      親の overflow で切れず、前面の別要素にも覆われない。
//   4. 右クリックはどこでも pet_menu を呼び、文脈（選択・入力欄・リンク・キャラ）だけを渡す。
//   5. 配置の保存は pet_save_scene だけを通り、倍率を持たない矩形で届く。
//   6. タイトルバーは見えているシーンの真上へ同じ幅で付き、フロー表示と常に表示を切り替えられる。
//   7. 会話の文字サイズを会話パネルから変えられ、見せ方ごとの設定へ保存する。
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

function mockScript({ layout, characterId = "mugi", scenes = {}, onboardingDone = true, history = [], uiTheme = "pastel" }) {
  return `
window.__calls = [];
window.__listeners = {};
window.__saved = ${JSON.stringify(scenes)};
window.__TAURI__ = {
  core: { invoke: (name, args = {}) => {
    window.__calls.push({ name, args: JSON.parse(JSON.stringify(args)) });
    if (name === "pet_init") return Promise.resolve({
      up: true, providerId: "local", theme: "system", uiTheme: ${JSON.stringify(uiTheme)}, fontPx: "14", streamChatFontPx: "22", voiceVolume: "35",
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
    setFullscreen: async () => {}, setSize: size => window.__resizeWindow(size), scaleFactor: async () => 1,
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

async function openApp({ layout, width, height, characterId, scenes, onboardingDone, history, uiTheme }) {
  // 窓とモニターは別の寸法。キャラ拡大時の窓拡張も実ブリッジと同じく検査する。
  const context = await browser.newContext({ viewport: { width, height }, screen: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  const errors = [];
  await page.exposeFunction("__resizeWindow", size => page.setViewportSize({ width: Math.round(size.width), height: Math.round(size.height) }));
  page.on("pageerror", error => errors.push(String(error)));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(mockScript({ layout, characterId, scenes, onboardingDone, history, uiTheme }));
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
  const style = await page.addStyleTag({ content: "html, body, .app, #stage, #sceneRoot { background: transparent !important; } .source:not([data-source=character]), #editLayer, #editToolbar, #topWindowMenu, .toast, .relationship-button, .online-dot, .beta-badge { visibility: hidden; }" });
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
    let inside = rect.left >= -0.5 && rect.top >= -0.5 && rect.right <= innerWidth + 0.5 && rect.bottom <= innerHeight + 0.5;
    // 窓内にあっても、親の overflow で切れていれば操作できない。
    for (let parent = element.parentElement; parent; parent = parent.parentElement) {
      const style = getComputedStyle(parent);
      const bounds = parent.getBoundingClientRect();
      if (["hidden", "clip", "auto", "scroll"].includes(style.overflowX)) inside &&= rect.left >= bounds.left - 0.5 && rect.right <= bounds.right + 0.5;
      if (["hidden", "clip", "auto", "scroll"].includes(style.overflowY)) inside &&= rect.top >= bounds.top - 0.5 && rect.bottom <= bounds.bottom + 0.5;
    }
    // 窓内で切れていなくても、前面の別要素（タイトルバー・配置操作など）に覆われていれば押せない。
    // 中央と内側4点で最前面の要素を調べる。pointer-events:none の要素はクリックを通すので対象外になる。
    const occludedBy = [];
    if (inside) {
      for (const [fx, fy] of [[0.5, 0.5], [0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75]]) {
        const hit = document.elementFromPoint(rect.left + rect.width * fx, rect.top + rect.height * fy);
        // 窓の縁のリサイズ領域（ui/components/window.css）は枠なし窓の意図した重なりで、隅に触れても中央は押せる。
        if (hit && hit !== element && !element.contains(hit) && !hit.closest(".window-resize-handles")) {
          const name = hit.id ? `#${hit.id}` : `${hit.tagName.toLowerCase()}${hit.classList.length ? `.${[...hit.classList].join(".")}` : ""}`;
          if (!occludedBy.includes(name)) occludedBy.push(name);
        }
      }
    }
    return {
      present: true,
      inside: inside && !occludedBy.length,
      occludedBy,
      rect: { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom },
    };
  }, selector);
}

const LAYOUTS = { pet: [900, 600], mascot: [360, 560], stream: [1280, 720] };
// all は配信コピーに実在する全キャラ。見た目の不具合は1体で見つけても全員で確かめる（AGENTS）。
const characters = process.env.UI_CHECK_CHARACTERS === "all"
  ? JSON.parse(fs.readFileSync(path.join(uiDir, "assets/characters/index.json"), "utf8")).characters
    .map(entry => entry.id)
    .filter(id => fs.existsSync(path.join(uiDir, "assets/characters", id)))
  : (process.env.UI_CHECK_CHARACTERS || "mugi,rin,photoreal-prototype").split(",");
const layoutIds = (process.env.UI_CHECK_LAYOUTS || "pet,mascot,stream").split(",");

for (const characterId of characters) {
  for (const layout of layoutIds) {
    const [width, height] = LAYOUTS[layout];
    const label = `${characterId}/${layout}`;
    const history = layout === "stream" ? [
      { role: "assistant", content: "今日の目標を確認しながら、一緒に次の準備を進めましょう。" },
      { role: "user", content: "この配信画面で文字が読めるか確認したい" },
      { role: "assistant", content: "履歴と入力欄の文字を読みやすい大きさで表示します。" },
    ] : [];
    const { context, page, errors } = await openApp({ layout, width, height, characterId, history });
    const kind = await page.evaluate(() => document.querySelector("[data-source=character] .character-view")?.dataset.renderKind);
    const framing = await page.evaluate(() => document.querySelector("[data-source=character] canvas")?.dataset.framing || "");
    const edges = await edgeContact(page);
    await page.screenshot({ path: path.join(outDir, `${characterId}-${layout}.png`), omitBackground: true });
    report.push({ label, kind, framing, edges });
    const idleCharacter = await page.locator('[data-source="character"]').boundingBox();
    if (layout === "mascot") {
      const chat = await page.locator('[data-source="chat"]').boundingBox();
      if (chat.y + chat.height > idleCharacter.y + 1) fail(label, "既定のミニ表示でフキダシがキャラの頭へ重なります");
      else ok(`${label} フキダシと頭の間に余白がある`);
    }
    await page.mouse.move(idleCharacter.x + idleCharacter.width / 2, idleCharacter.y + idleCharacter.height / 2);
    const idleEdit = await page.evaluate(() => ({
      layerHidden: document.getElementById("editLayer").hidden,
      gripHidden: getComputedStyle(document.querySelector(".source-grip")).display === "none",
    }));
    if (!idleEdit.layerHidden || !idleEdit.gripHidden) fail(label, `配置を押す前に編集枠・つまみが出ています ${JSON.stringify(idleEdit)}`);
    else ok(`${label} 通常時は配置を選べない`);
    if (edges.total < 50) fail(label, `キャラが描かれていません（不透明画素 ${edges.total}）`);
    else if (edges.top > 0) fail(label, `キャラの上端が切れています（上端の接触 ${edges.top}px）`);
    else ok(`${label} 描画（${kind}）`);
    if (errors.length) fail(label, `画面エラー: ${errors.slice(0, 3).join(" / ")}`);
    if (layout === "stream") {
      const headerFit = await page.evaluate(() => {
        const scene = document.getElementById("sceneRoot").getBoundingClientRect();
        const header = document.getElementById("topWindowMenu").getBoundingClientRect();
        return { scene: { left: scene.left, right: scene.right }, header: { left: header.left, right: header.right } };
      });
      if (Math.abs(headerFit.scene.left - headerFit.header.left) > 1 || Math.abs(headerFit.scene.right - headerFit.header.right) > 1) {
        fail(label, `タイトルバーと配信画面の左右端がずれています ${JSON.stringify(headerFit)}`);
      } else ok(`${label} タイトルバーは配信画面の幅に揃う`);
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
        const display = document.querySelector('[data-source="display"]')?.getBoundingClientRect();
        const chat = document.querySelector('[data-source="chat"]')?.getBoundingClientRect();
        const displayOverlap = display ? Math.max(0, Math.min(character.right, display.right) - Math.max(character.left, display.left))
          * Math.max(0, Math.min(character.bottom, display.bottom) - Math.max(character.top, display.top))
          / (display.width * display.height) : 0;
        // チャットはキャラの右側へ重ねてよいが、中央まで隠さない。
        const chatOverlap = chat ? Math.max(0, character.right - chat.left) / (character.right - character.left) : 0;
        return { displayOverlap, chatOverlap };
      }, edges.opaque);
      if (overlaps.displayOverlap > 0.05 || overlaps.chatOverlap > 0.25) {
        fail(label, `キャラの中央やディスプレイが隠れています: ${JSON.stringify(overlaps)}`);
      } else ok(`${label} キャラの右側だけチャットと重なる`);
      if (characterId === "mugi") {
        await page.fill("#input", "配信から送信します");
        const send = await page.evaluate(() => {
          const button = document.querySelector("#sendButton");
          const style = getComputedStyle(button);
          const chat = document.querySelector('.chat[data-variant="column"]');
          return { disabled: button.disabled, background: style.backgroundImage,
            font: parseFloat(getComputedStyle(chat).fontSize),
            inputFont: parseFloat(getComputedStyle(chat.querySelector('textarea')).fontSize),
            controlFont: parseFloat(getComputedStyle(chat.querySelector('.reply-actions .chip')).fontSize),
            width: chat.getBoundingClientRect().width / document.getElementById('sceneRoot').getBoundingClientRect().width * 1920 };
        });
        if (send.disabled || send.background === "none" || send.font < 22 || send.inputFont < 22 || send.controlFont < 18 || send.width < 720) {
          fail(`${label}/chat`, `配信の入力・送信操作が見づらいか使えません ${JSON.stringify(send)}`);
        } else ok(`${label} 読める履歴文字と押せる送信ボタン`);
        await page.evaluate(() => window.__emit("pet:ui", { streamChatFontPx: "42" }));
        const largerFont = await page.evaluate(() => {
          const chat = document.querySelector('.chat[data-variant="column"]');
          return { body: parseFloat(getComputedStyle(chat).fontSize),
            input: parseFloat(getComputedStyle(chat.querySelector('textarea')).fontSize),
            controls: parseFloat(getComputedStyle(chat.querySelector('.reply-actions .chip')).fontSize) };
        });
        if (largerFont.body !== 42 || largerFont.input !== 42 || largerFont.controls !== 34) {
          fail(`${label}/chat-font`, `保存後の配信文字サイズが即反映されません ${JSON.stringify(largerFont)}`);
        } else ok(`${label} 配信文字サイズを再起動なしで変更`);
        await page.evaluate(() => window.__emit("pet:ui", { streamChatFontPx: "22" }));
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
    await page.evaluate(() => window.__emit("pet:layout-edit", "all"));
    if (await page.locator(".reply-actions").isVisible()) fail(`${label}/edit`, "配置操作と会話操作が同時に重なります");
    const editAlignment = await page.evaluate(() => {
      const stage = document.getElementById("stage").getBoundingClientRect();
      return [...document.querySelectorAll("#editLayer .edit-frame:not([hidden])")].map(frame => {
        const source = document.querySelector(`[data-source="${frame.dataset.target}"]`)?.getBoundingClientRect();
        const visible = source && {
          left: Math.max(stage.left, source.left), top: Math.max(stage.top, source.top),
          right: Math.min(stage.right, source.right), bottom: Math.min(stage.bottom, source.bottom),
        };
        const border = frame.getBoundingClientRect();
        return { name: frame.dataset.target, aligned: visible && ["left", "top", "right", "bottom"].every(key => Math.abs(visible[key] - border[key]) <= 1) };
      });
    });
    if (!editAlignment.length || editAlignment.some(item => !item.aligned)) fail(`${label}/edit`, `選択枠がソースとずれています ${JSON.stringify(editAlignment)}`);
    else ok(`${label} 配置中の選択枠が表示範囲と一致`);
    const editTools = await fullyInside(page, "#editToolbar");
    if (!editTools.inside) fail(`${label}/edit`, `配置の操作部が画面外です ${JSON.stringify(editTools)}`);
    const hiddenGrips = await page.evaluate(() => {
      const obstacles = [document.querySelector(".top-window-menu"), document.getElementById("editToolbar")]
        .map(element => element.getBoundingClientRect());
      return [...document.querySelectorAll('.edit-frame[data-target="chat"] .edit-move,.edit-frame[data-target="chat"] .edit-handle')]
        .filter(element => element.getClientRects().length).filter(element => {
          const rect = element.getBoundingClientRect();
          return obstacles.some(other => rect.left < other.right && rect.right > other.left && rect.top < other.bottom && rect.bottom > other.top);
        }).map(element => element.className);
    });
    if (hiddenGrips.length) fail(`${label}/edit`, `会話枠のつまみが上端の操作部に隠れます: ${hiddenGrips.join("、")}`);
    if (characterId === characters[0]) await page.screenshot({ path: path.join(outDir, `${layout}-editing.png`), omitBackground: true });
    await page.evaluate(() => window.__emit("pet:layout-edit", "character"));
    // 横長モデルは両端が画面外へ出るため、ホイールで枠を収めてからつまみを検査する。
    const firstHandle = page.locator('.edit-frame[data-target="character"] .edit-handle[data-corner="top-left"]');
    for (let attempt = 0; attempt < 20 && !(await firstHandle.isVisible()); attempt++) {
      const box = await page.locator("[data-source=character]").boundingBox();
      await page.mouse.move(Math.max(1, Math.min(width - 1, box.x + box.width / 2)), box.y + box.height / 2);
      await page.mouse.wheel(0, 120);
      await page.waitForTimeout(60);
    }
    await page.waitForTimeout(400);
    // つまみで縮小・拡大する（保存は pet_save_scene を通り、倍率ではなく矩形で届く）。
    for (const factor of [0.5, 1.8]) {
      const box = await page.locator("[data-source=character]").boundingBox();
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      await page.waitForTimeout(150);
      const handle = page.locator('.edit-frame[data-target="character"] .edit-handle[data-corner="top-left"]');
      if (!(await handle.count()) || !(await handle.isVisible())) { fail(label, "キャラに乗せてもつまみが出ません"); break; }
      const hb = await handle.boundingBox();
      const startX = hb.x + hb.width / 2;
      const startY = hb.y + hb.height / 2;
      const targetX = startX + box.width * (1 - factor);
      const targetY = startY + box.height * (1 - factor);
      await page.mouse.move(startX, startY);
      await page.mouse.down();
      await page.mouse.move(targetX, targetY, { steps: 6 });
      await page.mouse.up();
      await page.waitForTimeout(600);
      if (characterId === characters[0]) await page.screenshot({ path: path.join(outDir, `${layout}-resize-${factor}.png`), omitBackground: true });
      const after = await edgeContact(page);
      const entry = await page.evaluate(id => window.__saved?.[document.getElementById("petApp").dataset.layout]?.characters?.[id], characterId);
      if (!entry || !Number.isFinite(entry.w) || entry.zoom != null || entry.scale != null) fail(`${label}/resize`, `保存値が矩形になっていません ${JSON.stringify(entry)}`);
      if (after.top > 0 || after.left > 0 || after.right > 0) fail(`${label}/resize×${factor}`, `拡大縮小後に端が切れています ${JSON.stringify(after)}`);
      else ok(`${label} 拡大縮小×${factor}（${Math.round(after.width)}×${Math.round(after.height)}）`);
    }
    await page.screenshot({ path: path.join(outDir, `${characterId}-${layout}-resized.png`), omitBackground: true });
    await page.locator("#editToolbar").getByRole("button", { name: "完了" }).click();
    if (!(await page.locator("#editLayer").evaluate(node => node.hidden))) fail(`${label}/edit`, "編集を終えても選択枠が残っています");

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
    else if (!result.inside) fail(label, `${selector} が窓の外へ出ているか覆われています ${JSON.stringify(result)}`);
  }
  ok(`${label} 最小サイズの収まり`);
  await context.close();
}

// タイトルバーの幅は固定シーンでは窓幅より狭くなる。親で切れず設定と全ウィンドウ操作を押せる。
for (const layout of Object.keys(LAYOUTS)) {
  const { context, page } = await openApp({ layout, width: 900, height: 600, characterId: "mugi" });
  for (const [width, height] of [[200, 240], [360, 560], [400, 600], [480, 600], [620, 600], [900, 240]]) {
    await page.setViewportSize({ width, height });
    await page.waitForTimeout(120);
    // クラスを直接付けるだけだと、起動直後の案内（INTRO_MS）が切れた時点でアプリが隠し、
    // 隠れたボタンを検査してしまう。利用者と同じく上端へポインタを置いて表示を保つ。
    const menuBox = await page.locator("#topWindowMenu").boundingBox();
    await page.mouse.move(menuBox.x + menuBox.width / 2, 1);
    await page.waitForFunction(() => document.querySelector("#petApp").classList.contains("title-bar-shown") && getComputedStyle(document.querySelector("#topWindowMenu")).opacity === "1");
    for (const selector of ["#topWindowMenu .top-window-menu-items button:last-child", "#topWindowMenu .window-pin", "#topWindowMenu .window-caption-controls button:nth-child(2)", "#topWindowMenu .window-caption-controls button:nth-child(3)", "#topWindowMenu .window-close"]) {
      const result = await fullyInside(page, selector);
      if (!result.present || !result.inside) fail(`${layout}/${width}x${height}`, `${selector} が隠れるか切れています ${JSON.stringify(result)}`);
    }
    await page.screenshot({ path: path.join(outDir, `menu-${layout}-${width}x${height}.png`), omitBackground: true });
  }
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

// 保存矩形が画面外へ延びても、編集枠だけを見える範囲へ切って元の保存値を保つ。
{
  const scenes = { version: 1, stream: { sources: { background: {
    anchor: "top-left", x: -100, y: -100, w: 1920, h: 1080,
  } } } };
  const { context, page } = await openApp({ layout: "stream", width: 1280, height: 720, characterId: "mugi", scenes });
  await page.evaluate(() => window.__emit("pet:layout-edit", "all"));
  const clipped = await page.evaluate(() => {
    const stage = document.getElementById("stage").getBoundingClientRect();
    const frame = document.querySelector('.edit-frame[data-target="background"]');
    const rect = frame.getBoundingClientRect();
    return { inside: rect.left >= stage.left && rect.top >= stage.top && rect.right <= stage.right && rect.bottom <= stage.bottom,
      cornerHidden: frame.querySelector('[data-corner="top-left"]').hidden,
      saved: window.__saved, saves: window.__calls.filter(call => call.name === "pet_save_scene").length };
  });
  if (!clipped.inside || !clipped.cornerHidden || clipped.saves || JSON.stringify(clipped.saved) !== JSON.stringify(scenes)) {
    fail("stream/clipped-edit", `画面外の枠または保存値が不正です ${JSON.stringify(clipped)}`);
  } else ok("stream/clipped-edit 画面外の枠は表示だけ切り、保存値を保つ");
  await page.screenshot({ path: path.join(outDir, "stream-clipped-edit.png"), omitBackground: true });
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

// 保存後のイベントだけでUIモードが切り替わり、再起動せず形と色が変わることを確かめる。
for (const [layout, [width, height]] of Object.entries(LAYOUTS)) {
  const { context, page, errors } = await openApp({ layout, width, height, characterId: "mugi" });
  for (const [mode, theme] of [["pastel", "system"], ["glass", "system"], ["glass", "light"], ["pastel", "light"], ["pastel", "dark"], ["pastel", "system"]]) {
    await page.evaluate(({ uiTheme, theme }) => window.__emit("pet:ui", { theme, uiTheme }), { uiTheme: mode, theme });
    const actual = await page.locator("[data-source=chat]").evaluate(node => ({
      radius: getComputedStyle(node).borderTopLeftRadius,
      mode: document.documentElement.dataset.uiTheme,
      column: node.dataset.variant === "column",
    }));
    const radius = mode === "pastel" ? (actual.column ? "8px" : "10px") : (actual.column ? "10px" : "12px");
    if (actual.mode !== mode || actual.radius !== radius) fail(`${layout}/${mode}`, `モード即反映が不一致 ${JSON.stringify(actual)}`);
    else ok(`${layout}/${mode} UIモードを即反映`);
    await page.waitForTimeout(300); // 色の切替アニメーション完了後の確定した見た目を撮る。
    await page.screenshot({ path: path.join(outDir, `mode-${mode}-${layout}.png`), omitBackground: true });
    await page.screenshot({ path: path.join(outDir, `mode-${mode}-${theme}-${layout}.png`), omitBackground: true });
  }
  if (errors.length) fail(`mode/${layout}`, errors.join(" / "));
  await context.close();
}

// タイトルバー: 見えているシーンの真上へ同じ幅で置き、既定のフロー表示は上端へ寄せたときだけ出す。
// 📌 で常に表示へ切り替えると、シーンの上へ高さを空けて保存する。
for (const [layout, width, height] of [["stream", 1000, 760], ["stream", 1280, 720], ["pet", 900, 600], ["mascot", 360, 560]]) {
  const label = `titlebar/${layout}-${width}x${height}`;
  const { context, page, errors } = await openApp({ layout, width, height });
  const geometry = () => page.evaluate(() => {
    const box = selector => { const rect = document.querySelector(selector).getBoundingClientRect(); return { left: Math.round(rect.left), top: Math.round(rect.top), right: Math.round(rect.right), bottom: Math.round(rect.bottom) }; };
    const scene = document.getElementById("sceneRoot").getBoundingClientRect();
    return {
      menu: box(".top-window-menu"),
      scene: { left: Math.round(scene.left), top: Math.round(scene.top), right: Math.round(scene.right) },
      opacity: Number(getComputedStyle(document.querySelector(".top-window-menu")).opacity),
      stageTop: Math.round(document.getElementById("stage").getBoundingClientRect().top),
    };
  });
  await page.mouse.move(width / 2, height / 2);
  await page.waitForTimeout(3400); // 起動直後の案内表示が終わるまで待つ。
  let actual = await geometry();
  const aligned = value => Math.abs(value.menu.left - value.scene.left) <= 1 && Math.abs(value.menu.right - value.scene.right) <= 1;
  if (!aligned(actual) || Math.abs(actual.menu.top - actual.scene.top) > 1 || actual.stageTop !== 0 || actual.opacity !== 0) {
    fail(label, `フロー表示の位置・非表示が不一致 ${JSON.stringify(actual)}`);
  }
  await page.mouse.move((actual.menu.left + actual.menu.right) / 2, actual.menu.top + 2);
  await page.waitForTimeout(400);
  if ((await geometry()).opacity !== 1) fail(label, "上端へ寄せてもタイトルバーが出ません");
  const obscuredActions = await page.evaluate(() => {
    const bar = document.querySelector(".top-window-menu").getBoundingClientRect();
    return [...document.querySelectorAll(".reply-actions button")].filter(button => {
      const rect = button.getBoundingClientRect();
      return rect.left < bar.right && rect.right > bar.left && rect.top < bar.bottom && rect.bottom > bar.top;
    }).map(button => button.textContent);
  });
  if (obscuredActions.length) fail(label, `タイトルバーが会話操作を隠します: ${obscuredActions.join("、")}`);
  await page.mouse.move(width / 2, height - 20);
  await page.waitForTimeout(900);
  if ((await geometry()).opacity !== 0) fail(label, "離れてもタイトルバーが隠れません");
  await page.mouse.move((actual.menu.left + actual.menu.right) / 2, actual.menu.top + 2);
  await page.waitForTimeout(400);
  await page.locator(".window-pin").click();
  await page.mouse.move(width / 2, height - 20);
  await page.waitForTimeout(900);
  actual = await geometry();
  const saved = await page.evaluate(() => window.__calls.filter(call => call.name === "pet_save_ui_pref").map(call => call.args));
  if (!aligned(actual) || actual.opacity !== 1 || Math.abs(actual.menu.bottom - actual.scene.top) > 1) fail(label, `常に表示でシーンの真上に常設されません ${JSON.stringify(actual)}`);
  else if (JSON.stringify(saved) !== JSON.stringify([{ key: "PICOAGENT_TITLE_BAR_PINNED", value: true }])) fail(label, `常に表示の保存が不一致 ${JSON.stringify(saved)}`);
  else ok(`${label} シーンへ吸着・フロー表示・常に表示`);
  await page.screenshot({ path: path.join(outDir, `titlebar-pinned-${layout}-${width}x${height}.png`), omitBackground: true });
  if (errors.length) fail(label, errors.join(" / "));
  await context.close();
}

// 会話の文字サイズ: 「Aa」の −／＋／標準とCtrl+ホイールで即反映し、落ち着いてから見せ方ごとのキーへ保存する。
for (const [layout, width, height, key, initial, step] of [["pet", 900, 600, "PICOAGENT_BUBBLE_FONT_PX", 14, 1], ["stream", 1280, 720, "PICOAGENT_STREAM_CHAT_FONT_PX", 22, 2]]) {
  const label = `font/${layout}`;
  const { context, page, errors } = await openApp({ layout, width, height });
  await page.locator(".font-size-toggle").click();
  await page.locator(".font-size-panel").waitFor();
  const chatBox = await page.locator("[data-source=chat]").boundingBox();
  // 3回の操作は保存の待ち（600ms）より短い間隔で続けるのが前提。Playwrightの操作を
  // 1つずつ送ると負荷の高い環境では間隔が600msを超え、正しく2回保存されて検査が揺れるため、
  // 実際のボタンとホイールのハンドラへ1回の評価の中で続けて送る。
  const { afterPlus, afterWheel } = await page.evaluate(({ x, y }) => {
    const read = () => {
      const node = document.querySelector("[data-source=chat]");
      return node.style.getPropertyValue(node.dataset.variant === "column" ? "--stream-chat-font-size" : "--chat-font-size");
    };
    const larger = document.querySelector('[aria-label="文字を大きく"]');
    larger.click();
    larger.click();
    const plus = read();
    document.elementFromPoint(x, y).dispatchEvent(new WheelEvent("wheel", { deltaY: 120, ctrlKey: true, bubbles: true, cancelable: true, clientX: x, clientY: y }));
    return { afterPlus: plus, afterWheel: read() };
  }, { x: chatBox.x + chatBox.width / 2, y: chatBox.y + chatBox.height / 2 });
  await page.waitForTimeout(900);
  const saved = await page.evaluate(() => window.__calls.filter(call => call.name === "pet_save_ui_pref").map(call => call.args));
  const panelBox = await page.locator(".font-size-panel").boundingBox();
  const inside = panelBox && panelBox.x >= chatBox.x && panelBox.x + panelBox.width <= chatBox.x + chatBox.width + 1;
  const expectedSaved = [{ key, value: String(initial + step) }];
  if (afterPlus !== `${initial + step * 2}px` || afterWheel !== `${initial + step}px`) fail(label, `即反映が不一致 ${afterPlus} / ${afterWheel}`);
  else if (JSON.stringify(saved) !== JSON.stringify(expectedSaved)) fail(label, `保存が不一致 ${JSON.stringify(saved)}`);
  else if (!inside) fail(label, `文字サイズの操作が会話パネルからはみ出しています ${JSON.stringify(panelBox)}`);
  else ok(`${label} 文字サイズの即反映と保存`);
  await page.screenshot({ path: path.join(outDir, `font-${layout}.png`), omitBackground: true });
  if (errors.length) fail(label, errors.join(" / "));
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
