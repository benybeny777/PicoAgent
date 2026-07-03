// fetch-live2d.mjs - Live2D 描画ランタイム（Cubism Core / PIXI / pixi-live2d-display）と
// 公式サンプルモデルを取得して配置する。実体は再配布条件があるため Git には含めない。
//
// 使い方: npm run setup:live2d
//   LIVE2D_FORCE=1 … 既存配置を再取得
//
// 配置先:
//   ui/vendor/live2d/          … ランタイム JS 3点（webview が 'self' で配信）
//   characters/{hiyori,haru,epsilon}/live2d/ … サンプルモデル一式（sync-characters が ui/assets へ同期）
//
// ライセンス:
//   Live2D Cubism Core   … Live2D Proprietary Software License Agreement
//                          https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_jp.html
//   サンプルモデル … Live2D Free Material License
//                          https://www.live2d.com/eula/live2d-free-material-license-agreement_jp.html
//   pixi.js / pixi-live2d-display … MIT License

import { cp, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join, dirname, basename, extname } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir, platform } from "node:os";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const VENDOR_DIR = join(HERE, "..", "ui", "vendor", "live2d");
const CHARACTERS_DIR = join(HERE, "..", "characters");
const FORCE = process.env.LIVE2D_FORCE === "1";

// ランタイム3点。読み込み順もこの順（core → pixi → display）。
const VENDOR_FILES = [
  ["live2dcubismcore.min.js", "https://cubism.live2d.com/sdk-web/cubismcore/live2dcubismcore.min.js"],
  ["pixi.min.js", "https://cdn.jsdelivr.net/npm/pixi.js@6.5.10/dist/browser/pixi.min.js"],
  ["cubism4.min.js", "https://cdn.jsdelivr.net/npm/pixi-live2d-display@0.4.0/dist/cubism4.min.js"],
];

const SAMPLE_BIN_BASE = "https://cubism.live2d.com/sample-data/bin/";

// Live2D 公式サンプル。Hiyori は WebSamples の runtime 形式、Haru/Epsilon は公式配布 zip の runtime を使う。
const SAMPLE_MODELS = [
  {
    id: "hiyori",
    label: "ひより",
    kind: "raw",
    base: "https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources/Hiyori/",
    modelJson: "Hiyori.model3.json",
    previewUrl: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-hiyori.jpg",
  },
  {
    id: "haru",
    label: "Haru",
    kind: "zip",
    zipUrl: `${SAMPLE_BIN_BASE}haru/haru_ja.zip`,
    runtimeDir: "runtime",
    modelJson: "haru.model3.json",
    previewUrl: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-haru.jpg",
  },
  {
    id: "epsilon",
    label: "Epsilon",
    kind: "zip",
    zipUrl: `${SAMPLE_BIN_BASE}epsilon/epsilon_ja.zip`,
    runtimeDir: "epsilon_free/runtime",
    modelJson: "Epsilon_free.model3.json",
    previewUrl: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-epsillon.jpg",
  },
];

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

function psLiteral(value) {
  return `'${value.replace(/'/g, "''")}'`;
}

function extractZip(zipPath, outDir) {
  if (platform() === "win32") {
    const r = spawnSync("powershell", ["-NoProfile", "-Command", `Expand-Archive -LiteralPath ${psLiteral(zipPath)} -DestinationPath ${psLiteral(outDir)} -Force`], { stdio: "inherit" });
    if (r.status !== 0) throw new Error("Expand-Archive 失敗");
    return;
  }
  const r = spawnSync("unzip", ["-o", zipPath, "-d", outDir], { stdio: "inherit" });
  if (r.status !== 0) throw new Error("unzip 失敗");
}

async function ensurePreview(sample) {
  const ext = extname(new URL(sample.previewUrl).pathname) || ".jpg";
  const dest = join(CHARACTERS_DIR, sample.id, `preview${ext}`);
  await ensureFile(sample.previewUrl, dest);
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

async function ensureRawModel(sample) {
  const modelDir = join(CHARACTERS_DIR, sample.id, "live2d");
  const manifestPath = join(modelDir, sample.modelJson);
  if (!existsSync(manifestPath) || FORCE) {
    await download(sample.base + sample.modelJson, manifestPath);
  }
  const model3 = JSON.parse(await readFile(manifestPath, "utf8"));
  let fetched = 0;
  for (const file of referencedFiles(model3)) {
    fetched += await ensureFile(sample.base + file, join(modelDir, file));
  }
  console.log(fetched > 0
    ? `live2d model fetched: ${sample.id} (${(fetched / 1e6).toFixed(1)} MB)`
    : `live2d model ready: ${sample.id}`);
}

async function ensureZipModel(sample) {
  const modelDir = join(CHARACTERS_DIR, sample.id, "live2d");
  const manifestPath = join(modelDir, sample.modelJson);
  if (existsSync(manifestPath) && !FORCE) {
    console.log(`live2d model ready: ${sample.id}`);
    return;
  }
  const work = join(tmpdir(), `picoagent-live2d-${sample.id}`);
  await rm(work, { recursive: true, force: true });
  await mkdir(work, { recursive: true });
  const zipPath = join(work, basename(new URL(sample.zipUrl).pathname));
  const bytes = await download(sample.zipUrl, zipPath);
  const extracted = join(work, "x");
  await mkdir(extracted, { recursive: true });
  extractZip(zipPath, extracted);
  const runtime = join(extracted, sample.runtimeDir);
  if (!existsSync(join(runtime, sample.modelJson))) {
    throw new Error(`${sample.label} の runtime に ${sample.modelJson} が見つかりません`);
  }
  await rm(modelDir, { recursive: true, force: true });
  await mkdir(modelDir, { recursive: true });
  for (const entry of await readdir(runtime)) {
    await cp(join(runtime, entry), join(modelDir, entry), { recursive: true });
  }
  await rm(work, { recursive: true, force: true });
  console.log(`live2d model fetched: ${sample.id} (${(bytes / 1e6).toFixed(1)} MB)`);
}

async function ensureModels() {
  for (const sample of SAMPLE_MODELS) {
    await ensurePreview(sample);
    if (sample.kind === "raw") {
      await ensureRawModel(sample);
    } else {
      await ensureZipModel(sample);
    }
  }
}

async function main() {
  await ensureVendor();
  await ensureModels();
  console.log("注意: Cubism Core とサンプルモデルは Live2D 社のライセンス条件で利用してください。");
  console.log("  https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_jp.html");
  console.log("  https://www.live2d.com/eula/live2d-free-material-license-agreement_jp.html");
}

main().catch((error) => {
  console.error("setup:live2d 失敗:", error.message);
  process.exit(1);
});
