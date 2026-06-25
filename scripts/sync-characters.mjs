// sync-characters.mjs - ルートの characters/（実体）を webview 配信先 ui/assets/characters/ へ同期する。
//
// キャラは扱いやすいようプロジェクト直下の characters/ を正本（実体）とし、webview は
// frontendDist（ui/）配下しか配信できないため、dev / build 前にここへコピーして配る。
// npm run dev / npm run build の前処理として呼ばれる。
// 生成物（ui/assets/characters/）は .gitignore 対象。

import { cpSync, existsSync, rmSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const src = resolve(root, "characters");
const dest = resolve(root, "ui", "assets", "characters");

if (!existsSync(src)) {
  console.error(`[sync-characters] 正本が見つかりません: ${src}`);
  process.exit(1);
}

rmSync(dest, { recursive: true, force: true });
mkdirSync(dirname(dest), { recursive: true });
cpSync(src, dest, { recursive: true });
console.log(`[sync-characters] ${src} -> ${dest} を同期しました`);
