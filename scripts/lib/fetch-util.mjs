// scripts/lib/fetch-util.mjs - 同梱物の取得で共通に使うダウンロード処理。
//
// 各 fetch-*.mjs が同じ download() を4本コピーして持っており、どれも
// (1) 応答全体をメモリへ読み込んでから (2) 最終パスへ直接書いていた。
//
// (1) は whisper の large モデル（約1.6GB）や Aivis の音声モデルで無駄が大きく、
//     メモリの少ない環境では失敗する。
// (2) は書き込み中に中断（Ctrl-C・電源断・ディスク不足）すると**途中まで書けた
//     ファイルが残る**。各スクリプトは「ファイルがあれば取得済み」と判断するため、
//     壊れたモデルや実行ファイルが永久に「取得済み」として扱われ、実行時に
//     原因の分かりにくいエラーになる。
//
// ここでは応答をストリームで一時ファイルへ書き、完了してから rename で確定する。
// 中断された場合は一時ファイルだけが残り、次回は最初から取り直せる。

import { createWriteStream } from "node:fs";
import { rename, rm, mkdir, stat } from "node:fs/promises";
import { dirname } from "node:path";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

/**
 * URL の内容を dest へ取得する。完了するまで dest は作られない。
 * @returns {Promise<number>} 書き込んだバイト数
 */
export async function downloadTo(url, dest, options = {}) {
  console.log("download:", url);
  const res = await fetch(url, {
    headers: { "User-Agent": "picoagent-setup", ...(options.headers || {}) },
    redirect: "follow",
  });
  if (!res.ok) throw new Error(`download failed ${res.status}: ${url}`);
  if (!res.body) throw new Error(`download failed (本文が空): ${url}`);

  await mkdir(dirname(dest), { recursive: true });
  const temp = `${dest}.part`;
  try {
    // ストリームで書くので、巨大なモデルでも全体をメモリへ載せない。
    await pipeline(Readable.fromWeb(res.body), createWriteStream(temp));
    // 完全に書けたものだけを最終パスへ出す（rename は同一ファイルシステムで原子的）。
    await rename(temp, dest);
  } catch (error) {
    await rm(temp, { force: true });
    throw error;
  }
  return (await stat(dest)).size;
}
