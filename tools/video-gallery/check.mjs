import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { mkdir } from "node:fs/promises";
import { dirname, extname, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const OUTPUT = resolve(ROOT, "temp/video-gallery-parts.png");
const MIME = {
  ".html": "text/html; charset=utf-8", ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8", ".json": "application/json; charset=utf-8",
  ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".webp": "image/webp",
};

async function loadPlaywright() {
  const specified = process.env.PLAYWRIGHT_MODULE;
  if (!specified) return import("playwright");
  return import(/^[A-Za-z]:[\\/]/.test(specified) ? pathToFileURL(specified).href : specified);
}

const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, "http://127.0.0.1").pathname);
    let target = resolve(ROOT, `.${pathname}`);
    if (target !== ROOT && !target.startsWith(`${ROOT}${sep}`)) {
      response.writeHead(403).end("forbidden");
      return;
    }
    if ((await stat(target)).isDirectory()) target = resolve(target, "index.html");
    const body = await readFile(target);
    response.writeHead(200, { "Content-Type": MIME[extname(target).toLowerCase()] || "application/octet-stream" });
    if (request.method === "HEAD") response.end();
    else response.end(body);
  } catch {
    response.writeHead(404).end("not found");
  }
});

await new Promise((resolveListen, reject) => {
  server.once("error", reject);
  server.listen(0, "127.0.0.1", resolveListen);
});
const address = server.address();
const base = `http://127.0.0.1:${address.port}/tools/video-gallery`;
let browser;
try {
  const { chromium } = await loadPlaywright();
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));

  await page.goto(`${base}/index.html`);
  const partsLink = page.locator('a[href="parts.html"]');
  if (await partsLink.count() !== 1) throw new Error("動画一覧から画像パーツ一覧へのリンクがありません");
  await partsLink.click();
  await page.waitForSelector("#loading", { state: "hidden", timeout: 30000 });
  const total = await page.locator("#partsGallery .card").count();
  if (total < 1) throw new Error("画像パーツが1件も表示されません");

  await page.selectOption("#characterFilter", "mugi");
  await page.locator('button[data-filter="layer"]').click();
  const layers = await page.locator("#partsGallery .card").count();
  if (layers < 5) throw new Error(`むぎの表情レイヤーが不足しています: ${layers}件`);
  await page.locator("#partsGallery .card button").first().click();
  if (!await page.locator("#detail").evaluate(dialog => dialog.open)) throw new Error("原寸ダイアログが開きません");
  await page.locator("#closeDetail").click();

  await page.selectOption("#characterFilter", "all");
  await page.locator('button[data-filter="all"]').click();
  await mkdir(dirname(OUTPUT), { recursive: true });
  await page.screenshot({ path: OUTPUT, fullPage: true });
  if (errors.length) throw new Error(`ページ例外: ${errors.join(" / ")}`);
  console.log(`動画・画像パーツ一覧検査完了: ${total}件 / むぎ表情${layers}件 / ${OUTPUT}`);
} finally {
  if (browser) await browser.close();
  await new Promise(resolveClose => server.close(resolveClose));
}
