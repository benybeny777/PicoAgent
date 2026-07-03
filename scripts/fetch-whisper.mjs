// fetch-whisper.mjs - whisper.cpp と ggml モデルを取得して
// src-tauri/resources/bundled/whisper/ へ配置する。実体は巨大なので Git には含めない。
//
// 使い方: npm run setup:whisper
//   WHISPER_RELEASE … whisper.cpp リリースタグ（既定 latest）
//   WHISPER_ASSET   … アセット名を直接指定（正規表現ではなく完全一致に近い文字列）
//   WHISPER_MODEL   … tiny/base/small/medium/large-v3-turbo または ggml-*.bin URL（既定 small）
//   WHISPER_FORCE=1 … 既存配置を再取得

import { chmod, mkdir, rm, readdir, rename, readFile, writeFile, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir, platform, arch } from "node:os";
import { join, dirname, basename, extname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const WHISPER_DIR = join(HERE, "..", "src-tauri", "resources", "bundled", "whisper");
const API_ROOT = "https://api.github.com/repos/ggml-org/whisper.cpp/releases";
const MODEL_ROOT = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

function exeName() {
  return platform() === "win32" ? "whisper-cli.exe" : "whisper-cli";
}

function exePath() {
  return join(WHISPER_DIR, exeName());
}

async function ensureExecutable(path) {
  if (platform() !== "win32") {
    await chmod(path, 0o755);
  }
}

function modelChoice() {
  return (process.env.WHISPER_MODEL || "small").trim();
}

function modelFileName(choice) {
  if (/^https?:\/\//.test(choice)) return choice.split("/").pop() || "ggml-model.bin";
  const normalized = choice.startsWith("ggml-") ? choice : `ggml-${choice}.bin`;
  return normalized.endsWith(".bin") ? normalized : `${normalized}.bin`;
}

function modelUrl(choice) {
  if (/^https?:\/\//.test(choice)) return choice;
  return `${MODEL_ROOT}/${modelFileName(choice)}`;
}

function modelPath() {
  return join(WHISPER_DIR, "models", modelFileName(modelChoice()));
}

function releaseUrl() {
  const tag = process.env.WHISPER_RELEASE?.trim() || "latest";
  return tag === "latest" ? `${API_ROOT}/latest` : `${API_ROOT}/tags/${encodeURIComponent(tag)}`;
}

function assetPattern() {
  if (process.env.WHISPER_ASSET) {
    return new RegExp(process.env.WHISPER_ASSET.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  }
  const p = platform();
  const a = arch();
  if (p === "win32") return a === "arm64" ? /whisper-bin-arm64\.zip$/ : /whisper-bin-x64\.zip$/;
  if (p === "darwin") return a === "arm64" ? /whisper-bin-macos-arm64\.zip$/ : /whisper-bin-macos-x64\.zip$/;
  return a === "arm64" ? /whisper-bin-ubuntu-arm64\.tar\.gz$/ : /whisper-bin-ubuntu-x64\.tar\.gz$/;
}

async function ghJson(url) {
  const res = await fetch(url, {
    headers: { "User-Agent": "picoagent-setup", Accept: "application/vnd.github+json" },
  });
  if (!res.ok) throw new Error(`GitHub API ${res.status}: ${url}`);
  return res.json();
}

async function download(url, dest) {
  console.log("download:", url);
  const res = await fetch(url, { headers: { "User-Agent": "picoagent-setup" }, redirect: "follow" });
  if (!res.ok) throw new Error(`download failed ${res.status}: ${url}`);
  const buf = Buffer.from(await res.arrayBuffer());
  await writeFile(dest, buf);
  return buf.length;
}

function psLiteral(value) {
  return `'${value.replace(/'/g, "''")}'`;
}

function extract(archivePath, outDir) {
  if (platform() === "win32") {
    const r = spawnSync("powershell", ["-NoProfile", "-Command", `Expand-Archive -LiteralPath ${psLiteral(archivePath)} -DestinationPath ${psLiteral(outDir)} -Force`], { stdio: "inherit" });
    if (r.status !== 0) throw new Error("Expand-Archive 失敗");
    return;
  }
  if (archivePath.endsWith(".tar.gz") || archivePath.endsWith(".tgz")) {
    const r = spawnSync("tar", ["-xzf", archivePath, "-C", outDir], { stdio: "inherit" });
    if (r.status !== 0) throw new Error("tar 展開に失敗しました");
    return;
  }
  if (extname(archivePath) === ".zip") {
    const r = spawnSync("unzip", ["-o", archivePath, "-d", outDir], { stdio: "inherit" });
    if (r.status !== 0) throw new Error("unzip 失敗");
    return;
  }
  throw new Error(`未対応のアーカイブ形式です: ${basename(archivePath)}`);
}

async function flatten(root, outDir) {
  const wanted = [];
  async function walk(dir) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        await walk(full);
      } else if (/^whisper-cli(\.exe)?$/.test(entry.name) || /\.(dll|dylib|so(\.\d+)*)$/.test(entry.name)) {
        wanted.push(full);
      }
    }
  }
  await walk(root);
  for (const file of wanted) {
    const dest = join(outDir, basename(file));
    if (file !== dest) {
      await rename(file, dest).catch(async () => {
        await writeFile(dest, await readFile(file));
      });
    }
    if (basename(dest) === "whisper-cli") {
      await ensureExecutable(dest);
    }
  }
  return wanted.some((file) => /^whisper-cli(\.exe)?$/.test(basename(file)));
}

async function ensureBinary() {
  if (existsSync(exePath()) && process.env.WHISPER_FORCE !== "1") {
    await ensureExecutable(exePath());
    console.log("whisper binary ready:", exePath());
    return;
  }
  await mkdir(WHISPER_DIR, { recursive: true });
  const rel = await ghJson(releaseUrl());
  const pat = assetPattern();
  const asset = (rel.assets || []).find((item) => pat.test(item.name));
  if (!asset) {
    throw new Error(`一致するアセットが見つかりません: ${pat}. 利用可能: ${(rel.assets || []).map((a) => a.name).join(", ")}`);
  }
  const work = join(tmpdir(), "picoagent-whisper");
  await rm(work, { recursive: true, force: true });
  await mkdir(work, { recursive: true });
  const zip = join(work, asset.name);
  const bytes = await download(asset.browser_download_url, zip);
  console.log(`fetched ${asset.name} (${(bytes / 1e6).toFixed(1)} MB), tag ${rel.tag_name}`);
  const extracted = join(work, "x");
  await mkdir(extracted, { recursive: true });
  extract(zip, extracted);
  const ok = await flatten(extracted, WHISPER_DIR);
  await rm(work, { recursive: true, force: true });
  if (!ok) throw new Error("whisper-cli が展開物に見つかりません");
}

async function ensureModel() {
  const dest = modelPath();
  if (existsSync(dest) && process.env.WHISPER_FORCE !== "1") {
    const size = (await stat(dest)).size;
    console.log(`whisper model ready: ${dest} (${(size / 1e6).toFixed(1)} MB)`);
    return;
  }
  await mkdir(dirname(dest), { recursive: true });
  const bytes = await download(modelUrl(modelChoice()), dest);
  console.log(`fetched ${basename(dest)} (${(bytes / 1e6).toFixed(1)} MB)`);
}

async function main() {
  await ensureBinary();
  await ensureModel();
  await writeFile(
    join(WHISPER_DIR, "whisper.json"),
    JSON.stringify(
      {
        program: exeName(),
        model: join("models", modelFileName(modelChoice())),
        args: "-m {model} -f {audio} -otxt -of {output_stem}",
        diarizeArgs: "-tdrz",
      },
      null,
      2,
    ) + "\n",
  );
  await writeFile(join(WHISPER_DIR, ".gitkeep"), "\n");
  console.log("whisper ready:", WHISPER_DIR);
  console.log("contents:", (await readdir(WHISPER_DIR)).join(", "));
}

main().catch((error) => {
  console.error("setup:whisper 失敗:", error.message);
  process.exit(1);
});
