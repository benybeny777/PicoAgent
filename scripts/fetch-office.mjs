// fetch-office.mjs - OfficeCLI（LibreOffice のコマンドライン）を同梱ディレクトリへ取得する。
//
// オフィスファイル（Word/Excel/PowerPoint）の PDF 変換・文書変換をローカルで行うため、
// LibreOffice を PicoAgent 管理下の src-tauri/resources/bundled/office/ へ配置する。
// npm run setup:office（npm run dev / build の前処理でも自動実行）。
//
//   Windows … 公式 MSI を administrative install（msiexec /a）で展開（管理者権限不要）
//   macOS   … 公式 DMG をマウントして LibreOffice.app をコピー
//   Linux   … 公式のポータブル配布が無いため警告のみ（apt 等で libreoffice を導入し、
//              PICOAGENT_LIBREOFFICE_PATH を設定するか PATH の soffice を使う）
//
// 失敗は既定で警告のみ（Office 変換なしで起動を続行）。必須にする場合は OFFICE_REQUIRED=1。
// 再取得は OFFICE_FORCE=1。バージョンは LIBREOFFICE_VERSION で上書き可能。

import { mkdir, rm, readdir, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { platform, arch } from "node:os";
import { spawnSync } from "node:child_process";
import { downloadTo } from "./lib/fetch-util.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const OFFICE_DIR = resolve(HERE, "..", "src-tauri", "resources", "bundled", "office");

const VERSION = process.env.LIBREOFFICE_VERSION || "24.8.4";
// downloadarchive は旧版も含めて恒久的に配布されるため、latest 追従より安定する。
const BASE = `https://downloadarchive.documentfoundation.org/libreoffice/old/${VERSION}`;

function exeCandidates() {
  if (platform() === "win32") return ["soffice.exe"];
  return ["soffice"];
}

// office ディレクトリ配下から soffice 実行ファイルを再帰探索する（展開レイアウト差異に強くする）。
async function findSoffice(dir, depth = 0) {
  if (depth > 7 || !existsSync(dir)) return null;
  let entries;
  try { entries = await readdir(dir, { withFileTypes: true }); } catch { return null; }
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isFile() && exeCandidates().includes(entry.name)) return path;
    if (entry.isDirectory()) {
      const found = await findSoffice(path, depth + 1);
      if (found) return found;
    }
  }
  return null;
}

async function fetchWindows() {
  const msi = join(OFFICE_DIR, `LibreOffice_${VERSION}.msi`);
  await downloadTo(`${BASE}/win/x86_64/LibreOffice_${VERSION}_Win_x86-64.msi`, msi);
  // administrative install はファイル展開のみで管理者権限やレジストリ変更を伴わない。
  const target = join(OFFICE_DIR, "extract");
  const result = spawnSync("msiexec", ["/a", msi, "/qn", `TARGETDIR=${target}`], { stdio: "inherit" });
  if (result.status !== 0) throw new Error(`msiexec /a 失敗 (exit ${result.status})`);
  await rm(msi, { force: true });
}

async function fetchMac() {
  const suffix = arch() === "arm64" ? "aarch64/LibreOffice_" + VERSION + "_MacOS_aarch64.dmg"
    : "x86_64/LibreOffice_" + VERSION + "_MacOS_x86-64.dmg";
  const dmg = join(OFFICE_DIR, "LibreOffice.dmg");
  await downloadTo(`${BASE}/mac/${suffix}`, dmg);
  const mount = join(OFFICE_DIR, "mnt");
  const attach = spawnSync("hdiutil", ["attach", dmg, "-nobrowse", "-mountpoint", mount], { stdio: "inherit" });
  if (attach.status !== 0) throw new Error("hdiutil attach 失敗");
  try {
    const copy = spawnSync("cp", ["-R", join(mount, "LibreOffice.app"), OFFICE_DIR], { stdio: "inherit" });
    if (copy.status !== 0) throw new Error("LibreOffice.app のコピーに失敗");
  } finally {
    spawnSync("hdiutil", ["detach", mount], { stdio: "inherit" });
  }
  await rm(dmg, { force: true });
}

async function main() {
  await mkdir(OFFICE_DIR, { recursive: true });

  if (process.env.OFFICE_FORCE !== "1") {
    const existing = await findSoffice(OFFICE_DIR);
    if (existing) {
      console.log(`office cli ready: ${existing}`);
      return;
    }
  } else {
    await rm(OFFICE_DIR, { recursive: true, force: true });
    await mkdir(OFFICE_DIR, { recursive: true });
  }

  const os = platform();
  if (os === "win32") {
    await fetchWindows();
  } else if (os === "darwin") {
    await fetchMac();
  } else {
    throw new Error(
      "Linux 向けの公式ポータブル配布が無いため同梱できません。apt 等で libreoffice を導入し、PATH の soffice か PICOAGENT_LIBREOFFICE_PATH 設定を使ってください。"
    );
  }

  const found = await findSoffice(OFFICE_DIR);
  if (!found) throw new Error("展開後に soffice が見つかりません。");
  const size = (await stat(found)).size;
  console.log(`office cli fetched: ${found} (${(size / 1e6).toFixed(1)} MB)`);
}

main().catch((error) => {
  // フォールバック許可: OfficeCLI はベストエフォート同梱（未取得でも Office 変換以外は動く）。
  if (process.env.OFFICE_REQUIRED !== "1") {
    console.warn("setup:office 警告:", error.message);
    console.warn("Office ファイル変換なしでの起動を続行します。必須にする場合は OFFICE_REQUIRED=1 を付けるか、PICOAGENT_LIBREOFFICE_PATH で既存の LibreOffice を指定してください。");
    process.exit(0);
  }
  console.error("setup:office 失敗:", error.message);
  process.exit(1);
});
