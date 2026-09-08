// LocalVTuberStudio形式をPicoAgent本番描画層へ通し、目口と待機動作を実Canvasで記録する。
import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const playwrightModule = process.env.PICO_PLAYWRIGHT_MODULE;
if (!playwrightModule) throw new Error("PICO_PLAYWRIGHT_MODULE にPlaywrightのindex.mjsを指定してください");
const { chromium } = await import(pathToFileURL(playwrightModule).href);
const output = path.resolve(process.argv[2] || "temp/pico-lvs-check");
const url = process.argv[3] || "http://127.0.0.1:8776/tools/mesh-rig-preview/?character=female-a";
const captureUrl = new URL(url);
captureUrl.searchParams.set("autoBlink", "off");
await mkdir(output, { recursive: true });

const browser = await chromium.launch({ channel: "chrome", headless: true });
const page = await browser.newPage({ viewport: { width: 1500, height: 1000 }, deviceScaleFactor: 1 });
const errors = [];
page.on("pageerror", error => errors.push(`pageerror: ${error.message}`));
page.on("console", message => { if (message.type() === "error") errors.push(`console: ${message.text()}`); });

const sha256 = bytes => createHash("sha256").update(bytes).digest("hex");
const capture = async name => {
  const bytes = await page.locator("#runtime").screenshot({ animations: "allow", omitBackground: true });
  await writeFile(path.join(output, `${name}.png`), bytes);
  return { name, bytes: bytes.length, sha256: sha256(bytes) };
};

const report = { url: captureUrl.href, mouth: [], blink: [], gaze: [], expressions: [], motions: [], idle: [], errors };
try {
  await page.goto(captureUrl.href, { waitUntil: "networkidle" });
  await page.waitForFunction(() => document.querySelector("#runtime")?.dataset.meshRig === "ready");
  const status = await page.locator("#status").textContent();
  if (status?.includes("失敗")) throw new Error(status);
  report.canvas = await page.locator("#runtime").evaluate(canvas => ({
    width: canvas.width,
    height: canvas.height,
    mode: document.querySelector("#status")?.textContent,
  }));
  for (const label of ["閉口", "あ", "い", "う", "え", "お"]) {
    await page.getByRole("button", { name: label, exact: true }).click();
    await page.waitForTimeout(180);
    report.mouth.push(await capture(`mouth-${label}`));
  }
  const blinkButton = page.getByRole("button", { name: "両目まばたき", exact: true });
  // スクリーンショット処理でページ側の時刻を進めない。各位相は独立したまばたきから撮る。
  await blinkButton.click(); await page.waitForTimeout(230);
  report.blink.push(await capture("blink-closed-hold"));
  await page.waitForTimeout(500);
  report.blink.push(await capture("blink-open"));
  await blinkButton.click(); await page.waitForTimeout(90);
  report.blink.push(await capture("blink-closing"));
  await page.waitForTimeout(600);
  await blinkButton.click(); await page.waitForTimeout(430);
  report.blink.push(await capture("blink-opening"));
  for (const [name, x, y] of [["center", 0, 0], ["left", -1, 0], ["right", 1, 0], ["up", 0, -1]]) {
    await page.locator("#gazeX").evaluate((input, value) => {
      input.value = String(value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, x);
    await page.locator("#gazeY").evaluate((input, value) => {
      input.value = String(value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, y);
    await page.waitForTimeout(420);
    report.gaze.push(await capture(`gaze-${name}`));
  }
  await page.getByRole("button", { name: "視線を中央へ", exact: true }).click();
  for (const [name, label] of [["neutral", "通常"], ["happy", "笑顔"], ["concern", "困り顔"], ["thinking", "考え顔"], ["shy", "照れ"]]) {
    await page.getByRole("button", { name: label, exact: true }).click();
    await page.waitForTimeout(220);
    report.expressions.push(await capture(`expression-${name}`));
  }
  await page.getByRole("button", { name: "通常", exact: true }).click();
  for (const [name, label] of [["idle", "待機"], ["talking", "会話"], ["thinking", "考え中"], ["success", "よろこび"], ["concern", "困り"], ["listen", "聞く"], ["wake", "起きる"], ["sleep", "眠る"]]) {
    await page.getByRole("button", { name: label, exact: true }).click();
    await page.waitForTimeout(420);
    report.motions.push(await capture(`motion-${name}`));
  }
  await page.getByRole("button", { name: "閉口", exact: true }).click();
  await page.getByRole("button", { name: "通常", exact: true }).click();
  await page.getByRole("button", { name: "待機", exact: true }).click();
  for (let index = 0; index < 5; index += 1) {
    await page.waitForTimeout(260);
    report.idle.push(await capture(`idle-${index}`));
  }
  const unique = entries => new Set(entries.map(entry => entry.sha256)).size;
  report.unique = {
    mouth: unique(report.mouth),
    blink: unique(report.blink),
    gaze: unique(report.gaze),
    expressions: unique(report.expressions),
    motions: unique(report.motions),
    idle: unique(report.idle),
  };
  if (report.unique.mouth < 5) throw new Error(`母音の描画差分が不足しています: ${report.unique.mouth}`);
  if (report.unique.blink < 3) throw new Error(`まばたきの描画差分が不足しています: ${report.unique.blink}`);
  if (report.unique.gaze < 4) throw new Error(`視線の描画差分が不足しています: ${report.unique.gaze}`);
  if (/[?&]character=(?:female-a|mugi|mugi-normal)(?:&|$)/.test(captureUrl.href) && report.unique.expressions < 5) {
    throw new Error(`表情の描画差分が不足しています: ${report.unique.expressions}`);
  }
  if (/[?&]character=(?:female-a|mugi|mugi-normal)(?:&|$)/.test(captureUrl.href) && report.unique.motions < 6) {
    throw new Error(`動作の描画差分が不足しています: ${report.unique.motions}`);
  }
  if (report.unique.idle < 3) throw new Error(`待機動作の描画差分が不足しています: ${report.unique.idle}`);
  if (errors.length) throw new Error(errors.join("\n"));
} finally {
  await writeFile(path.join(output, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  await browser.close();
}
console.log(JSON.stringify(report));
