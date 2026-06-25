// stop-stale.mjs - dev/build 前に、前回の残存プロセスを停止する。
//
// 残った llama-server / アプリ本体が同梱エンジンの DLL（ggml-base.dll など）を掴んでいると、
// 次の cargo build がリソースコピーで失敗する（Windows os error 32: 別プロセスが使用中）。
// その「起動時に残っていたら落とす」をビルド前フックで自動化する。

import { spawnSync } from "node:child_process";

// dev のクレート名 picoagent.exe / 配布名 PicoAgent.exe / 同梱エンジン llama-server.exe を対象にする。
const images = ["llama-server.exe", "picoagent.exe", "PicoAgent.exe"];

if (process.platform === "win32") {
  for (const image of images) {
    // /T で子プロセスごと、/F で強制終了。未起動（コード 128）は無視する。
    const r = spawnSync("taskkill", ["/F", "/T", "/IM", image], { encoding: "utf8" });
    if (r.status === 0) console.log(`[stop-stale] 停止: ${image}`);
  }
} else {
  // macOS / Linux: pkill で名前一致を停止（未起動は無視）。
  for (const name of ["llama-server", "PicoAgent", "picoagent"]) {
    spawnSync("pkill", ["-f", name], { encoding: "utf8" });
  }
}
