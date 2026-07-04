// fetch-engine.mjs - 推論エンジン llama.cpp（llama-server）を取得して
// src-tauri/resources/bundled/engine/ へ展開する。リポジトリには含めず各環境で取得する。
//
// 使い方: npm run setup:engine
// 既定は Windows x64 の Vulkan(GPU) ビルド。別 OS/版・CPU ビルドは環境変数で上書き可能
//   （例: CPU 固定なら LLAMACPP_ASSET="llama-.*-bin-win-cpu-x64.zip"）。
//   LLAMACPP_RELEASE  … リリースタグ（既定 latest を解決）
//   LLAMACPP_ASSET    … アセット名を直接指定（最優先。Windows は zip、macOS/Linux は tar.gz）

import { chmod, cp, mkdir, rm, readdir, rename, readFile, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir, platform, arch } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const ENGINE_DIR = join(HERE, "..", "src-tauri", "resources", "bundled", "engine");
const LEGACY_ENGINE_DIR = join(HERE, "..", "src-tauri", "resources", "engine");
const API = "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest";

async function ensureExecutable(path) {
  if (platform() !== "win32") {
    await chmod(path, 0o755);
  }
}

// プラットフォーム別のアセット名パターン（llama.cpp リリースの命名に合わせる）。
// Windows は zip、macOS/Linux は tar.gz で配布される。
function assetPattern() {
  if (process.env.LLAMACPP_ASSET) return new RegExp(process.env.LLAMACPP_ASSET.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const p = platform();
  if (p === "win32") return /llama-.*-bin-win-vulkan-x64\.zip$/; // Windows Vulkan x64（GPU 優先・NVIDIA/Intel 兼用）
  if (p === "darwin") return arch() === "arm64" ? /llama-.*-bin-macos-arm64\.tar\.gz$/ : /llama-.*-bin-macos-x64\.tar\.gz$/;
  return /llama-.*-bin-ubuntu-x64\.tar\.gz$/;
}

async function ghJson(url) {
  const res = await fetch(url, { headers: { "User-Agent": "picoagent-setup", Accept: "application/vnd.github+json" } });
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

// アーカイブを展開（Windows は zip を PowerShell Expand-Archive、他は tar.gz を tar で展開）。
function unzip(archivePath, outDir) {
  if (platform() === "win32") {
    const r = spawnSync("powershell", ["-NoProfile", "-Command", `Expand-Archive -LiteralPath '${archivePath}' -DestinationPath '${outDir}' -Force`], { stdio: "inherit" });
    if (r.status !== 0) throw new Error("Expand-Archive 失敗");
  } else {
    const r = spawnSync("tar", ["-xzf", archivePath, "-C", outDir], { stdio: "inherit" });
    if (r.status !== 0) throw new Error("tar 展開失敗");
  }
}

// 展開ツリーから llama-server 実行ファイルと同梱 DLL/dylib を engine/ 直下へ集約。
async function flatten(root, outDir) {
  const wanted = [];
  async function walk(dir) {
    for (const e of await readdir(dir, { withFileTypes: true })) {
      const full = join(dir, e.name);
      if (e.isDirectory()) await walk(full);
      else if (/^llama-server(\.exe)?$/.test(e.name) || /\.(dll|dylib|so(\.\d+)*)$/.test(e.name)) wanted.push(full);
    }
  }
  await walk(root);
  for (const f of wanted) {
    const dest = join(outDir, f.split(/[\\/]/).pop());
    if (f !== dest) await rename(f, dest).catch(async () => { await writeFile(dest, await readFile(f)); });
    if (/^llama-server$/.test(dest.split(/[\\/]/).pop())) {
      await ensureExecutable(dest);
    }
  }
  return wanted.some((f) => /llama-server(\.exe)?$/.test(f));
}

async function main() {
  await mkdir(ENGINE_DIR, { recursive: true });
  const serverName = platform() === "win32" ? "llama-server.exe" : "llama-server";
  const bundledServer = join(ENGINE_DIR, serverName);
  const legacyServer = join(LEGACY_ENGINE_DIR, serverName);
  if (existsSync(bundledServer) && process.env.LLAMACPP_FORCE !== "1") {
    await ensureExecutable(bundledServer);
    console.log("engine ready:", ENGINE_DIR);
    console.log("既に同梱 llama-server があります。再取得する場合は LLAMACPP_FORCE=1 を指定してください。");
    return;
  }
  if (existsSync(legacyServer) && process.env.LLAMACPP_FORCE !== "1") {
    await rm(ENGINE_DIR, { recursive: true, force: true });
    await mkdir(ENGINE_DIR, { recursive: true });
    for (const entry of await readdir(LEGACY_ENGINE_DIR)) {
      await cp(join(LEGACY_ENGINE_DIR, entry), join(ENGINE_DIR, entry), { recursive: true });
    }
    await writeFile(join(ENGINE_DIR, ".gitkeep"), "\n");
    await ensureExecutable(bundledServer);
    console.log("engine ready:", ENGINE_DIR);
    console.log("旧 resources/engine から resources/bundled/engine へコピーしました。");
    return;
  }

  const rel = await ghJson(API);
  const pat = assetPattern();
  const asset = (rel.assets || []).find((a) => pat.test(a.name));
  if (!asset) throw new Error(`一致するアセットが見つかりません: ${pat}. 利用可能: ${(rel.assets || []).map((a) => a.name).join(", ")}`);

  const work = join(tmpdir(), "picoagent-engine");
  await rm(work, { recursive: true, force: true });
  await mkdir(work, { recursive: true });
  const zip = join(work, asset.name);
  const bytes = await download(asset.browser_download_url, zip);
  console.log(`fetched ${asset.name} (${(bytes / 1e6).toFixed(1)} MB), tag ${rel.tag_name}`);

  const ex = join(work, "x");
  await mkdir(ex, { recursive: true });
  unzip(zip, ex);
  const ok = await flatten(ex, ENGINE_DIR);
  await rm(work, { recursive: true, force: true });
  if (!ok) throw new Error("llama-server が展開物に見つかりません");
  console.log("engine ready:", ENGINE_DIR);
  console.log("contents:", (await readdir(ENGINE_DIR)).join(", "));
}

main().catch((e) => { console.error("setup:engine 失敗:", e.message); process.exit(1); });
