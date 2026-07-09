// fetch-tools.mjs - 精度向上用の OSS CLI ツール群を同梱ディレクトリへ取得する。
//
// OfficeCLI（fetch-office.mjs）と同じ方針で、スキルの精度を底上げする外部 CLI を
// PicoAgent 管理下の src-tauri/resources/bundled/tools/<ツール名>/ へ配置する。
// npm run setup:tools（npm run dev / build の前処理でも自動実行）。
//
//   pandoc  … 文書変換スキルの Markdown/EPUB/reST 変換を高精度化（win/mac/linux）
//   ripgrep … ローカル文書検索スキルの走査を高速化（win/mac/linux）
//   ffmpeg  … 音声文字起こしスキルの mp3/m4a 等 → 16kHz WAV 前処理（win/mac/linux）
//   qpdf    … PDF 編集スキルのページ抽出・結合・回転（win のみ配布。mac/linux は brew/apt）
//   poppler … PDF 編集スキルのテキスト抽出 pdftotext（win のみ配布。mac/linux は brew/apt）
//   imagemagick … 画像変換スキルの PSD/XCF/HEIC/TIFF/WebP 変換（win のみポータブル配布。mac/linux は brew/apt）
//
// 失敗は既定で警告のみ（各スキルは PATH のツールへフォールバックして続行）。
// 必須にする場合は TOOLS_REQUIRED=1。再取得は TOOLS_FORCE=1。
// 対象を絞る場合は PICOAGENT_SETUP_TOOLS=pandoc,ripgrep のようにカンマ区切りで指定。

import { mkdir, rm, writeFile, readdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { platform, arch } from "node:os";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOLS_DIR = resolve(HERE, "..", "src-tauri", "resources", "bundled", "tools");

const PANDOC_VERSION = process.env.PANDOC_VERSION || "3.6.3";
const RIPGREP_VERSION = process.env.RIPGREP_VERSION || "14.1.1";
const QPDF_VERSION = process.env.QPDF_VERSION || "12.2.0";
const POPPLER_WIN_VERSION = process.env.POPPLER_WIN_VERSION || "24.08.0-0";
const IMAGEMAGICK_VERSION = process.env.IMAGEMAGICK_VERSION || "7.1.1-47";

const OS = platform();
const IS_ARM = arch() === "arm64";

// ツール定義: exe は同梱判定に使う実行ファイル名（拡張子なし）、source は取得先。
// source が null のプラットフォームは公式ポータブル配布が無いため警告して PATH へ委ねる。
const TOOLS = [
  {
    name: "pandoc",
    exe: "pandoc",
    source: {
      win32: `https://github.com/jgm/pandoc/releases/download/${PANDOC_VERSION}/pandoc-${PANDOC_VERSION}-windows-x86_64.zip`,
      darwin: `https://github.com/jgm/pandoc/releases/download/${PANDOC_VERSION}/pandoc-${PANDOC_VERSION}-${IS_ARM ? "arm64" : "x86_64"}-macOS.zip`,
      linux: `https://github.com/jgm/pandoc/releases/download/${PANDOC_VERSION}/pandoc-${PANDOC_VERSION}-linux-${IS_ARM ? "arm64" : "amd64"}.tar.gz`,
    },
    fallback: "apt install pandoc / brew install pandoc で導入すると PATH から使われます。",
  },
  {
    name: "ripgrep",
    exe: "rg",
    source: {
      win32: `https://github.com/BurntSushi/ripgrep/releases/download/${RIPGREP_VERSION}/ripgrep-${RIPGREP_VERSION}-x86_64-pc-windows-msvc.zip`,
      darwin: `https://github.com/BurntSushi/ripgrep/releases/download/${RIPGREP_VERSION}/ripgrep-${RIPGREP_VERSION}-${IS_ARM ? "aarch64" : "x86_64"}-apple-darwin.tar.gz`,
      linux: `https://github.com/BurntSushi/ripgrep/releases/download/${RIPGREP_VERSION}/ripgrep-${RIPGREP_VERSION}-${IS_ARM ? "aarch64-unknown-linux-gnu" : "x86_64-unknown-linux-musl"}.tar.gz`,
    },
    fallback: "apt install ripgrep / brew install ripgrep で導入すると PATH から使われます。",
  },
  {
    name: "ffmpeg",
    exe: "ffmpeg",
    source: {
      // BtbN の latest タグは恒久 URL（自動更新）。ダウンロードは 100MB 前後と大きい。
      win32: "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
      darwin: `https://ffmpeg.martin-riedl.de/redirect/latest/macos/${IS_ARM ? "arm64" : "amd64"}/release/ffmpeg.zip`,
      linux: `https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-${IS_ARM ? "linuxarm64" : "linux64"}-gpl.tar.xz`,
    },
    fallback: "apt install ffmpeg / brew install ffmpeg で導入すると PATH から使われます。",
  },
  {
    name: "qpdf",
    exe: "qpdf",
    source: {
      win32: `https://github.com/qpdf/qpdf/releases/download/v${QPDF_VERSION}/qpdf-${QPDF_VERSION}-bin-mingw64.zip`,
      darwin: null,
      linux: null,
    },
    fallback: "apt install qpdf / brew install qpdf で導入すると PATH から使われます。",
  },
  {
    name: "poppler",
    exe: "pdftotext",
    source: {
      win32: `https://github.com/oschwartz10612/poppler-windows/releases/download/v${POPPLER_WIN_VERSION}/Release-${POPPLER_WIN_VERSION}.zip`,
      darwin: null,
      linux: null,
    },
    fallback: "apt install poppler-utils / brew install poppler で導入すると PATH から使われます。",
  },
  {
    name: "imagemagick",
    exe: "magick",
    source: {
      // archive/binaries は旧版も恒久配布されるため、latest 追従より安定する。
      win32: `https://imagemagick.org/archive/binaries/ImageMagick-${IMAGEMAGICK_VERSION}-portable-Q16-x64.zip`,
      darwin: null,
      linux: null,
    },
    fallback: "apt install imagemagick / brew install imagemagick で導入すると PATH（magick / convert）から使われます。",
  },
];

function exeName(base) {
  return OS === "win32" ? `${base}.exe` : base;
}

// ツールディレクトリ配下から実行ファイルを再帰探索する（展開レイアウト差異に強くする）。
async function findExe(dir, exe, depth = 0) {
  if (depth > 7 || !existsSync(dir)) return null;
  let entries;
  try { entries = await readdir(dir, { withFileTypes: true }); } catch { return null; }
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isFile() && entry.name === exeName(exe)) return path;
    if (entry.isDirectory()) {
      const found = await findExe(path, exe, depth + 1);
      if (found) return found;
    }
  }
  return null;
}

async function download(url, dest) {
  console.log(`download: ${url}`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`download failed ${res.status}: ${url}`);
  const buf = Buffer.from(await res.arrayBuffer());
  await writeFile(dest, buf);
  return buf.length;
}

// zip は Windows なら bsdtar（標準の tar.exe）、mac/linux なら unzip。tar 系は tar -xf。
function extract(archive, dir) {
  const isZip = archive.endsWith(".zip");
  const cmd = isZip && OS !== "win32"
    ? { bin: "unzip", args: ["-q", "-o", archive, "-d", dir] }
    : { bin: "tar", args: ["-xf", archive, "-C", dir] };
  const result = spawnSync(cmd.bin, cmd.args, { stdio: "inherit" });
  if (result.status !== 0) throw new Error(`${cmd.bin} での展開に失敗しました (exit ${result.status})`);
}

async function fetchTool(tool) {
  const dir = join(TOOLS_DIR, tool.name);
  await mkdir(dir, { recursive: true });

  if (process.env.TOOLS_FORCE === "1") {
    await rm(dir, { recursive: true, force: true });
    await mkdir(dir, { recursive: true });
  } else {
    const existing = await findExe(dir, tool.exe);
    if (existing) {
      console.log(`${tool.name} ready: ${existing}`);
      return true;
    }
  }

  const url = tool.source[OS];
  if (!url) {
    console.warn(`${tool.name}: このOS向けのポータブル配布が無いため同梱をスキップします。${tool.fallback}`);
    return true; // 配布なしは失敗扱いにしない（PATH フォールバック前提）
  }

  const archive = join(dir, url.split("/").pop().split("?")[0]);
  await download(url, archive);
  extract(archive, dir);
  await rm(archive, { force: true });

  const found = await findExe(dir, tool.exe);
  if (!found) throw new Error(`展開後に ${exeName(tool.exe)} が見つかりません。`);
  console.log(`${tool.name} fetched: ${found}`);
  return true;
}

async function main() {
  await mkdir(TOOLS_DIR, { recursive: true });
  const only = (process.env.PICOAGENT_SETUP_TOOLS || "")
    .split(",")
    .map((name) => name.trim().toLowerCase())
    .filter(Boolean);
  const targets = only.length > 0 ? TOOLS.filter((tool) => only.includes(tool.name)) : TOOLS;

  const failures = [];
  for (const tool of targets) {
    try {
      await fetchTool(tool);
    } catch (error) {
      failures.push(tool.name);
      console.warn(`setup:tools 警告 (${tool.name}): ${error.message}`);
      console.warn(`  ${tool.fallback}`);
    }
  }
  if (failures.length > 0 && process.env.TOOLS_REQUIRED === "1") {
    throw new Error(`同梱に失敗したツール: ${failures.join(", ")}`);
  }
  if (failures.length > 0) {
    console.warn(`一部ツールは同梱できませんでした（${failures.join(", ")}）。該当スキルは PATH のツールを使います。`);
  }
}

main().catch((error) => {
  // フォールバック許可: 各ツールはベストエフォート同梱（未取得でも該当スキル以外は動く）。
  if (process.env.TOOLS_REQUIRED !== "1") {
    console.warn("setup:tools 警告:", error.message);
    process.exit(0);
  }
  console.error("setup:tools 失敗:", error.message);
  process.exit(1);
});
