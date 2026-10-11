import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../../", import.meta.url));
const bom = Buffer.from([0xef, 0xbb, 0xbf]);

function validateEncoding(bytes, name) {
  new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  if (bytes.some((byte) => byte >= 0x80)) {
    assert.ok(bytes.subarray(0, 3).equals(bom), `${name}: 日本語入りのPS1はUTF-8 BOM付きで保存してください`);
  }
}

test("文字コード検査はBOM欠損と不正なUTF-8を拒否する", () => {
  const japanese = Buffer.from("Write-Host '完了'\n");
  assert.throws(() => validateEncoding(japanese, "確認.ps1"), /BOM/);
  assert.doesNotThrow(() => validateEncoding(Buffer.concat([bom, japanese]), "確認.ps1"));
  assert.doesNotThrow(() => validateEncoding(Buffer.from("exit 0\n"), "確認.ps1"));
  assert.throws(() => validateEncoding(Buffer.from([0xff]), "確認.ps1"));
});

test("追跡中の日本語入りPowerShellスクリプトはUTF-8 BOMを持つ", () => {
  const paths = execFileSync("git", ["ls-files", "-z", "--", "*.ps1"], { cwd: root })
    .toString("utf8").split("\0").filter(Boolean);
  assert.ok(paths.length > 0, "追跡中のPS1が必要です");
  for (const path of paths) validateEncoding(readFileSync(join(root, path)), path);
});

test("Windows PowerShell 5.1で呼ぶ診断と待機の入口は構文解析できる", {
  skip: process.platform !== "win32" ? "Windows専用の互換性確認" : false,
}, () => {
  const source = "$errors = $null; $tokens = $null; foreach ($path in @('scripts/show-powershell7-required.ps1', 'scripts/wait-before-close.ps1')) { [System.Management.Automation.Language.Parser]::ParseFile((Join-Path (Get-Location) $path), [ref]$tokens, [ref]$errors) | Out-Null; if ($errors.Count) { throw ($errors | Out-String) } }";
  execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", source], { cwd: root });
});

test("関係者版の初期設定はBOM付きの配布用コピーを組み込む", () => {
  const source = readFileSync(new URL("../../scripts/build-partner-installer.ps1", import.meta.url), "utf8");
  assert.match(source, /WriteAllText\(\$packagedSeedScript,\s*\$seedText,\s*\[System\.Text\.UTF8Encoding\]::new\(\$true\)\)/);
  assert.match(source, /File `"\$packagedSeedScript`"/);
  assert.doesNotMatch(source, /File `"\$seedScript`"/);
});

test("配布用コピーは日本語を保ち元の素材を上書きしない", {
  skip: process.platform !== "win32" ? "Windows専用の互換性確認" : false,
}, () => {
  const source = readFileSync(new URL("../../scripts/build-partner-installer.ps1", import.meta.url), "utf8");
  const begin = source.indexOf("$packagedSeedScript =");
  const end = source.indexOf("$hookPath =", begin);
  assert.ok(begin >= 0 && end > begin, "配布用コピーの処理が必要です");
  const parent = join(root, "temp");
  mkdirSync(parent, { recursive: true });
  const directory = mkdtempSync(join(parent, "powershell-encoding-"));
  const input = join(directory, "input.ps1");
  const output = join(directory, "seed-voices.ps1");
  const text = "Write-Host '完了'\n";
  writeFileSync(input, text);
  const command = "$ErrorActionPreference = 'Stop'; $seedScript = $env:PICO_TEST_SEED; $outputRoot = $env:PICO_TEST_OUTPUT; " + source.slice(begin, end);
  const run = (seed) => execFileSync("pwsh", ["-NoProfile", "-NonInteractive", "-Command", command], {
    cwd: root,
    env: { ...process.env, PICO_TEST_SEED: seed, PICO_TEST_OUTPUT: directory },
    stdio: "pipe",
  });
  try {
    run(input);
    const bytes = readFileSync(output);
    validateEncoding(bytes, output);
    assert.equal(bytes.subarray(3).toString("utf8"), text);
    assert.equal(readFileSync(input, "utf8"), text);
    assert.throws(() => run(output), "素材と出力の同一パスを拒否する");
    assert.deepEqual(readFileSync(output), bytes, "拒否後も素材を変更しない");
    execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-File", output], { cwd: root, stdio: "pipe" });
    writeFileSync(input, Buffer.from([0xff]));
    assert.throws(() => run(input), "不正なUTF-8の素材を拒否する");
    assert.deepEqual(readFileSync(output), bytes, "不正な素材で配布用コピーを上書きしない");
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
