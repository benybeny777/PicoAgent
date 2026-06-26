// fetch-aivis.mjs - AivisSpeech Engine を取得して
// src-tauri/resources/bundled/aivis/ へ展開する。実体は巨大なので Git には含めない。
//
// 使い方: npm run setup:aivis
//   AIVIS_RELEASE … リリースタグ（既定 1.2.0。latest を指定すると最新版）
//   AIVIS_ASSET   … アセット名を直接指定（正規表現ではなく完全一致に近い文字列）

import { mkdir, rm, readdir, cp, writeFile, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir, platform, arch } from "node:os";
import { join, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const AIVIS_DIR = join(HERE, "..", "src-tauri", "resources", "bundled", "aivis");
const API_ROOT = "https://api.github.com/repos/Aivis-Project/AivisSpeech-Engine/releases";
const DEFAULT_RELEASE = "1.2.0";

function bundledRunPath() {
  return join(AIVIS_DIR, platform() === "win32" ? "run.exe" : "run");
}

function releaseUrl() {
  const tag = process.env.AIVIS_RELEASE?.trim() || DEFAULT_RELEASE;
  return tag === "latest" ? `${API_ROOT}/latest` : `${API_ROOT}/tags/${encodeURIComponent(tag)}`;
}

function assetPattern() {
  if (process.env.AIVIS_ASSET) {
    return new RegExp(process.env.AIVIS_ASSET.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  }
  const p = platform();
  const a = arch();
  if (p === "win32") return /AivisSpeech-Engine-Windows-x64-.*\.7z\.001$/;
  if (p === "darwin") {
    return a === "arm64"
      ? /AivisSpeech-Engine-macOS-arm64-.*\.7z\.001$/
      : /AivisSpeech-Engine-macOS-x64-.*\.7z\.001$/;
  }
  return a === "arm64"
    ? /AivisSpeech-Engine-Linux-arm64-.*\.7z\.001$/
    : /AivisSpeech-Engine-Linux-x64-.*\.7z\.001$/;
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

function commandExists(cmd) {
  const check = platform() === "win32" ? spawnSync("where.exe", [cmd]) : spawnSync("sh", ["-lc", `command -v ${cmd}`]);
  return check.status === 0;
}

function extract(archivePath, outDir) {
  const sevenZip = commandExists("7z") ? "7z" : commandExists("7za") ? "7za" : null;
  if (sevenZip) {
    const r = spawnSync(sevenZip, ["x", archivePath, `-o${outDir}`, "-y"], { stdio: "inherit" });
    if (r.status !== 0) throw new Error(`${sevenZip} 展開に失敗しました`);
    return;
  }

  const r = spawnSync("tar", ["-xf", archivePath, "-C", outDir], { stdio: "inherit" });
  if (r.status !== 0) {
    throw new Error("展開に失敗しました。7-Zip をインストールするか、tar が 7z 展開に対応する環境で実行してください。");
  }
}

async function findRun(dir) {
  const exe = platform() === "win32" ? "run.exe" : "run";
  async function walk(current) {
    for (const entry of await readdir(current, { withFileTypes: true })) {
      const full = join(current, entry.name);
      if (entry.isDirectory()) {
        const found = await walk(full);
        if (found) return found;
      } else if (entry.name === exe) {
        return full;
      }
    }
    return null;
  }
  return walk(dir);
}

async function copyEngineTree(runPath, destDir) {
  const root = dirname(runPath);
  await rm(destDir, { recursive: true, force: true });
  await mkdir(destDir, { recursive: true });
  for (const entry of await readdir(root)) {
    await cp(join(root, entry), join(destDir, entry), { recursive: true });
  }
  await writeFile(join(destDir, ".gitkeep"), "\n");
  const destRun = join(destDir, basename(runPath));
  if (!existsSync(destRun) || !(await stat(destRun)).isFile()) {
    throw new Error("run 実行ファイルの配置に失敗しました");
  }
}

async function main() {
  if (existsSync(bundledRunPath()) && process.env.AIVIS_FORCE !== "1") {
    console.log("aivis ready:", AIVIS_DIR);
    console.log("既に同梱 AivisSpeech Engine があります。再取得する場合は AIVIS_FORCE=1 を指定してください。");
    return;
  }

  const rel = await ghJson(releaseUrl());
  const pat = assetPattern();
  const asset = (rel.assets || []).find((a) => pat.test(a.name));
  if (!asset) {
    throw new Error(`一致するアセットが見つかりません: ${pat}. 利用可能: ${(rel.assets || []).map((a) => a.name).join(", ")}`);
  }

  const work = join(tmpdir(), "picoagent-aivis");
  await rm(work, { recursive: true, force: true });
  await mkdir(work, { recursive: true });
  const archive = join(work, asset.name);
  const bytes = await download(asset.browser_download_url, archive);
  console.log(`fetched ${asset.name} (${(bytes / 1e6).toFixed(1)} MB), tag ${rel.tag_name}`);

  const extracted = join(work, "x");
  await mkdir(extracted, { recursive: true });
  extract(archive, extracted);
  const runPath = await findRun(extracted);
  if (!runPath) throw new Error("AivisSpeech Engine の run 実行ファイルが見つかりません");
  await copyEngineTree(runPath, AIVIS_DIR);
  await rm(work, { recursive: true, force: true });
  console.log("aivis ready:", AIVIS_DIR);
  console.log("contents:", (await readdir(AIVIS_DIR)).slice(0, 20).join(", "));
}

main().catch((e) => {
  console.error("setup:aivis 失敗:", e.message);
  process.exit(1);
});
