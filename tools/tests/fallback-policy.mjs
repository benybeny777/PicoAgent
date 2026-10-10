import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../../", import.meta.url));
const policy = JSON.parse(readFileSync(new URL("../contracts/fallback-policy.json", import.meta.url), "utf8"));
const hasBasis = line => {
  const start = line.indexOf(policy.basisOpen);
  if (start < 0) return false;
  const tail = line.slice(start + policy.basisOpen.length);
  const end = tail.indexOf(policy.basisClose);
  return end >= 0 && policy.authorityPrefixes.some(prefix => {
    const authority = tail.slice(0, end);
    return authority.startsWith(prefix) && (prefix !== policy.numberedAuthorityPrefix || /^[0-9]/.test(authority.slice(prefix.length)));
  });
};
const invalid = text => text.split(/\r?\n/).flatMap((line, index) =>
  line.includes("//") && line.slice(line.indexOf("//") + 2).includes(policy.marker) && !hasBasis(line.slice(line.indexOf("//") + 2)) ? [index + 1] : []);

test("画面の許可コメントは共通方針で根拠を必須にする", () => {
  const files = execFileSync("git", ["ls-files", "-z", "--", "ui"], { cwd: root, encoding: "utf8" })
    .split("\0").filter(file => file.endsWith(".js") && !file.startsWith("ui/vendor/"));
  const errors = files.flatMap(file => invalid(readFileSync(path.join(root, file), "utf8")).map(line => `${file}:${line}`));
  assert.deepEqual(errors, []);
});

test("根拠なしの許可を拒否しPRや仕様の根拠を受け入れる", () => {
  assert.deepEqual(invalid("// フォールバック許可: 便利なので続ける"), [1]);
  assert.deepEqual(invalid("return []; // フォールバック許可: 続行（根拠: PR #不明）"), [1]);
  assert.deepEqual(invalid("// フォールバック許可: 利用者が採用した候補（根拠: PR #467）"), []);
  assert.deepEqual(invalid("// フォールバック許可: 退避後に新規開始（根拠: AGENTS.md 状態が壊れていたときの空で継続）"), []);
});
