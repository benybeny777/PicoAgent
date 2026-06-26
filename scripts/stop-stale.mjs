// stop-stale.mjs - dev/build 前に、前回の残存プロセスを停止する。
//
// 残った llama-server / アプリ本体が同梱エンジンの DLL（ggml-base.dll など）を掴んでいると、
// 次の cargo build がリソースコピーで失敗する（Windows os error 32: 別プロセスが使用中）。
// さらに、残った picoagent.exe / その WebView2 子プロセスが WebView のユーザーデータフォルダ
// （EBWebView）を掴んだままだと、直後に cargo run が起動する新しいインスタンスの WebView 生成が
// 一時的に失敗し、`build()` が Err になって exit code 1 で落ちる。
//
// taskkill / pkill は終了を「要求」するだけで完了を待たない。kill しただけだと直後の cargo run が
// 終了途中の古いプロセスとレースするため、ここでは「何か kill したら、対象が実際に消えるまで待つ」。

import { spawnSync } from "node:child_process";

// dev のクレート名 picoagent.exe / 配布名 PicoAgent.exe / 同梱エンジン llama-server.exe を対象にする。
// Aivis は実行ファイル名が run.exe で汎用的なので、下の専用処理でパス一致だけを止める。
const winImages = ["llama-server.exe", "picoagent.exe", "PicoAgent.exe"];
const unixNames = ["llama-server", "PicoAgent", "picoagent"];
const isWin = process.platform === "win32";
const targets = isWin ? winImages : unixNames;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 対象がまだ走っているか（win: tasklist、unix: pgrep）。
function isRunning(target) {
  if (isWin) {
    const r = spawnSync("tasklist", ["/FI", `IMAGENAME eq ${target}`, "/NH"], { encoding: "utf8" });
    return typeof r.stdout === "string" && r.stdout.toLowerCase().includes(target.toLowerCase());
  }
  return spawnSync("pgrep", ["-f", target], { encoding: "utf8" }).status === 0;
}

// 強制終了を要求する（未起動は無視）。1 つでも実際に落とせたら true。
function killImage(target) {
  if (isWin) {
    // /T で子プロセス（WebView2 / llama-server）ごと、/F で強制終了。未起動はコード 128。
    return spawnSync("taskkill", ["/F", "/T", "/IM", target], { encoding: "utf8" }).status === 0;
  }
  return spawnSync("pkill", ["-9", "-f", target], { encoding: "utf8" }).status === 0;
}

let killedAny = false;
for (const target of targets) {
  if (killImage(target)) {
    killedAny = true;
    console.log(`[stop-stale] 停止: ${target}`);
  }
}

if (isWin) {
  const script = [
    "$procs = Get-CimInstance Win32_Process |",
    "Where-Object { $_.Name -eq 'run.exe' -and ($_.ExecutablePath -like '*resources\\\\bundled\\\\aivis\\\\run.exe' -or $_.ExecutablePath -like '*resources\\\\aivis\\\\run.exe') };",
    "foreach ($p in $procs) { Stop-Process -Id $p.ProcessId -Force; Write-Output $p.ProcessId }",
  ].join(" ");
  const r = spawnSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8" });
  if (r.status === 0 && String(r.stdout || "").trim()) {
    killedAny = true;
    console.log("[stop-stale] 停止: resources/bundled/aivis/run.exe");
  }
}

// 何も残っていなければ待つ必要はない（通常起動にオーバーヘッドを足さない）。
if (killedAny) {
  const deadline = Date.now() + 6000;
  let remaining = targets.filter(isRunning);
  while (remaining.length > 0 && Date.now() < deadline) {
    await sleep(150);
    remaining = targets.filter(isRunning);
  }
  if (remaining.length > 0) {
    console.warn(`[stop-stale] 警告: まだ終了していません: ${remaining.join(", ")}`);
  } else {
    // プロセス消滅後も WebView2 のロック解放・ポート解放にわずかな遅延があるため落ち着かせる。
    await sleep(400);
  }
}
