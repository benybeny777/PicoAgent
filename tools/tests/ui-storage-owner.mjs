// 保存領域の直接参照は共通層に限定し、別名・角括弧の参照も対象にする。
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
const root = fileURLToPath(new URL("../../", import.meta.url));
const references = source => source.split(/\r?\n/).flatMap((line, index) =>
  !/^\s*(?:\/\/|\/\*|\*)/.test(line) && /\blocalStorage\b/.test(line) ? [index + 1] : []);
test("保存領域の入口は共通層だけにする", () => {
  const files = execFileSync("git", ["ls-files", "-z", "--", "ui"], { cwd: root, encoding: "utf8" })
    .split("\0").filter(file => file.endsWith(".js") && !file.startsWith("ui/vendor/") && file !== "ui/app/ui-store.js");
  const failures = files.flatMap(file => references(readFileSync(new URL(`../../${file}`, import.meta.url), "utf8")).map(line => `${file}:${line}`));
  assert.deepEqual(failures, []);
});
test("直接操作と別名や角括弧の参照を見逃さず説明コメントは除く", () => {
  assert.deepEqual(references('const value = localStorage.getItem("x");'), [1]);
  assert.deepEqual(references('const store = window.localStorage;'), [1]);
  assert.deepEqual(references('window["localStorage"].setItem("x", "y");'), [1]);
  assert.deepEqual(references('// localStorage は共通層だけで扱う'), []);
});
