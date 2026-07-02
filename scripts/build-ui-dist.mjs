// build-ui-dist.mjs - ui/ を Tauri 配信用 ui-dist/ へコピーする。
// release では依存追加なしで HTML/CSS と JS の余分な空白を落とし、配布物を読みづらくする。

import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const srcRoot = resolve(root, "ui");
const destRoot = resolve(root, "ui-dist");
const release = process.argv.includes("--release");

function listFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const stat = statSync(path);
    if (stat.isDirectory()) out.push(...listFiles(path));
    else if (stat.isFile()) out.push(path);
  }
  return out;
}

function minifyHtml(text) {
  return text
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/>\s+</g, "><")
    .replace(/\s{2,}/g, " ")
    .trim();
}

function minifyCss(text) {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\s+/g, " ")
    .replace(/\s*([{}:;,>~+])\s*/g, "$1")
    .replace(/;}/g, "}")
    .trim();
}

function minifyJs(text) {
  return text
    .split(/\r?\n/)
    .map(line => line.trim())
    .filter(Boolean)
    .join("\n");
}

function minifyByExt(path, text) {
  const ext = extname(path).toLowerCase();
  if (ext === ".html") return minifyHtml(text);
  if (ext === ".css") return minifyCss(text);
  if (ext === ".js") return minifyJs(text);
  return text;
}

if (!existsSync(srcRoot)) {
  console.error(`[build-ui-dist] ui が見つかりません: ${srcRoot}`);
  process.exit(1);
}

rmSync(destRoot, { recursive: true, force: true });
mkdirSync(destRoot, { recursive: true });
cpSync(srcRoot, destRoot, { recursive: true });

if (release) {
  for (const file of listFiles(destRoot)) {
    const ext = extname(file).toLowerCase();
    if (![".html", ".css", ".js"].includes(ext)) continue;
    const src = readFileSync(file, "utf8");
    writeFileSync(file, minifyByExt(file, src), "utf8");
  }
}

console.log(`[build-ui-dist] ${relative(root, srcRoot)} -> ${relative(root, destRoot)} (${release ? "release" : "dev"})`);
