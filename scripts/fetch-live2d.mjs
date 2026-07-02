// fetch-live2d.mjs - Live2D 描画ランタイム（Cubism Core / PIXI / pixi-live2d-display）と
// 公式サンプルモデル「ひより」を取得して配置する。実体は再配布条件があるため Git には含めない。
//
// 使い方: npm run setup:live2d
//   LIVE2D_FORCE=1 … 既存配置を再取得
//
// 配置先:
//   ui/vendor/live2d/          … ランタイム JS 3点（webview が 'self' で配信）
//   characters/hiyori/live2d/  … サンプルモデル一式（sync-characters が ui/assets へ同期）
//
// ライセンス:
//   Live2D Cubism Core   … Live2D Proprietary Software License Agreement
//                          https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_jp.html
//   サンプルモデル ひより … Live2D Free Material License
//                          https://www.live2d.com/eula/live2d-free-material-license-agreement_jp.html
//   pixi.js / pixi-live2d-display … MIT License

import { mkdir, writeFile, readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const VENDOR_DIR = join(HERE, "..", "ui", "vendor", "live2d");
const MODEL_DIR = join(HERE, "..", "characters", "hiyori", "live2d");
const FORCE = process.env.LIVE2D_FORCE === "1";

// ランタイム3点。読み込み順もこの順（core → pixi → display）。
const VENDOR_FILES = [
  ["live2dcubismcore.min.js", "https://cubism.live2d.com/sdk-web/cubismcore/live2dcubismcore.min.js"],
  ["pixi.min.js", "https://cdn.jsdelivr.net/npm/pixi.js@6.5.10/dist/browser/pixi.min.js"],
  ["cubism4.min.js", "https://cdn.jsdelivr.net/npm/pixi-live2d-display@0.4.0/dist/cubism4.min.js"],
];

// Live2D 公式サンプル（CubismWebSamples）の「ひより」モデル。
const MODEL_BASE = "https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources/Hiyori/";
const MODEL_JSON = "Hiyori.model3.json";

async function download(url, dest) {
  console.log("download:", url);
  const res = await fetch(url, { headers: { "User-Agent": "picoagent-setup" }, redirect: "follow" });
  if (!res.ok) throw new Error(`download failed ${res.status}: ${url}`);
  const buf = Buffer.from(await res.arrayBuffer());
  await mkdir(dirname(dest), { recursive: true });
  await writeFile(dest, buf);
  return buf.length;
}

async function ensureFile(url, dest) {
  if (existsSync(dest) && !FORCE) return 0;
  return download(url, dest);
}

// model3.json の FileReferences から参照ファイルを列挙する（moc3・テクスチャ・物理・モーション等）。
function referencedFiles(model3) {
  const refs = model3.FileReferences || {};
  const files = [];
  for (const key of ["Moc", "Physics", "Pose", "UserData", "DisplayInfo"]) {
    if (refs[key]) files.push(refs[key]);
  }
  for (const texture of refs.Textures || []) files.push(texture);
  for (const expression of refs.Expressions || []) {
    if (expression.File) files.push(expression.File);
  }
  for (const group of Object.values(refs.Motions || {})) {
    for (const motion of group) {
      if (motion.File) files.push(motion.File);
      if (motion.Sound) files.push(motion.Sound);
    }
  }
  return files;
}

async function ensureVendor() {
  let fetched = 0;
  for (const [name, url] of VENDOR_FILES) {
    fetched += await ensureFile(url, join(VENDOR_DIR, name));
  }
  console.log(fetched > 0
    ? `live2d runtime fetched: ${VENDOR_DIR} (${(fetched / 1e6).toFixed(1)} MB)`
    : `live2d runtime ready: ${VENDOR_DIR}`);
}

async function ensureModel() {
  const manifestPath = join(MODEL_DIR, MODEL_JSON);
  if (!existsSync(manifestPath) || FORCE) {
    await download(MODEL_BASE + MODEL_JSON, manifestPath);
  }
  const model3 = JSON.parse(await readFile(manifestPath, "utf8"));
  let fetched = 0;
  for (const file of referencedFiles(model3)) {
    fetched += await ensureFile(MODEL_BASE + file, join(MODEL_DIR, file));
  }
  console.log(fetched > 0
    ? `live2d model fetched: ${MODEL_DIR} (${(fetched / 1e6).toFixed(1)} MB)`
    : `live2d model ready: ${MODEL_DIR}`);
}

async function main() {
  await ensureVendor();
  await ensureModel();
  console.log("注意: Cubism Core とサンプルモデルは Live2D 社のライセンス条件で利用してください。");
  console.log("  https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_jp.html");
  console.log("  https://www.live2d.com/eula/live2d-free-material-license-agreement_jp.html");
}

main().catch((error) => {
  console.error("setup:live2d 失敗:", error.message);
  process.exit(1);
});
