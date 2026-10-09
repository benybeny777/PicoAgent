import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";

const source = readFileSync(new URL("../../xtask/src/assets.rs", import.meta.url), "utf8").replaceAll("\r\n", "\n");
function stopScript(name) {
  const section = source.split(`#[cfg(not(unix))]\nfn ${name}(`)[1];
  assert.ok(section, "Windowsの停止処理がありません");
  const literal = section.match(/"(\$(?:needle|pattern) = [^\n]+)",/)[1];
  // Rustの通常文字列とformat!を復号し、製品が実際にPowerShellへ渡すコードを検査する。
  let script = JSON.parse(`"${literal}"`).replaceAll("{{", "{").replaceAll("}}", "}");
  for (const key of ["SCREENSHOT_TOOL_NEEDLE", "BROWSER_PROFILE_PATTERN"]) {
    const value = source.match(new RegExp(`const ${key}: &str = "([^"]+)";`))[1];
    script = script.replaceAll(`{${key}}`, value);
  }
  const names = source.match(/const BROWSER_PROCESS_NAMES:.*?= \[([^\]]+)\]/s)[1];
  script = script.replaceAll("{names}", [...names.matchAll(/"([^"]+)"/g)].map(match => `Name='${match[1]}'`).join(" OR "));
  return script;
}

function selectedProcesses(name, fixtures) {
  const json = JSON.stringify(fixtures).replaceAll("'", "''");
  // CIMとtaskkillだけを置換する。利用者のプロセスを列挙・停止せず、実際の絞り込みを実行する。
  const script = `$fixtures = '${json}' | ConvertFrom-Json
$selected = [System.Collections.Generic.List[int]]::new()
function Get-CimInstance { param($ClassName, $Filter) $fixtures | Where-Object { $Filter.Contains("Name='" + $_.Name + "'") } }
function taskkill { $selected.Add([int]$args[1]) }
${stopScript(name)}
ConvertTo-Json -Compress -InputObject @($selected.ToArray())`;
  const result = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8", timeout: 15000 });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout.trim().split(/\r?\n/).at(-1));
}

test("Windowsのブラウザ停止は通常・開発・検証版の実パスを拾い無関係なプロセスを除外する", { skip: process.platform !== "win32" }, () => {
  const paths = [
    ["chrome.exe", String.raw`chrome --user-data-dir=C:\Users\me\PicoAgent\browser-profile`],
    ["msedge.exe", String.raw`msedge --user-data-dir="C:\Users\my name\PicoAgent-Dev\browser-profile"`],
    ["chromium.exe", String.raw`chromium --user-data-dir=C:\Users\me\PicoAgent\instances\com.picoagent.ui-check\browser-profile`],
    ["chrome.exe", "chrome --user-data-dir=C:/Users/me/PicoAgent-Dev/instances/check/browser-profile"],
    ["chrome.exe", String.raw`chrome --user-data-dir=C:\Users\me\Google\Chrome\User Data`],
    ["node.exe", String.raw`node C:\Users\me\PicoAgent\browser-profile\server.js`],
    ["powershell.exe", String.raw`powershell C:\Users\me\PicoAgent\browser-profile`],
    ["chrome.exe", null],
  ];
  assert.deepEqual(selectedProcesses("stop_managed_browser", paths.map(([Name, CommandLine], index) => ({ Name, CommandLine, ProcessId: index + 1 }))), [1, 2, 3, 4]);
});

test("Windowsの撮影Node停止は円記号とスラッシュの実パスを拾いMCPやAdobeを除外する", { skip: process.platform !== "win32" }, () => {
  const paths = [
    ["node.exe", String.raw`node C:\repo\tools\screenshots\shoot.mjs`],
    ["node.exe", "node tools/screenshots/panel-overflow.mjs"],
    ["node.exe", String.raw`node C:\repo\mcp\server.mjs`],
    ["node.exe", String.raw`node C:\Adobe\js\main.js`],
    ["powershell.exe", "powershell tools/screenshots/shoot.mjs"],
    ["node.exe", null],
  ];
  assert.deepEqual(selectedProcesses("stop_managed_screenshot_node", paths.map(([Name, CommandLine], index) => ({ Name, CommandLine, ProcessId: index + 1 }))), [1, 2]);
});
