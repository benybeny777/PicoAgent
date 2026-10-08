import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

// 外部モデルやPythonパッケージを取得せず、学習工程の保護境界を通常検証に含める。
test("音声学習と管理環境のPython回帰検査", () => {
  const candidates = process.env.LEARNING_TEST_PYTHON
    ? [[process.env.LEARNING_TEST_PYTHON, []]]
    : [["python", []], ["python3", []], ["py", ["-3"]]];
  const found = candidates.find(([program, prefix]) => {
    const result = spawnSync(program, [...prefix, "-S", "-c", "import sys; assert sys.version_info >= (3, 10)"], { windowsHide: true, timeout: 10000 });
    return result.status === 0;
  });
  assert.ok(found, "Python 3.10以上が必要です。PATHまたはLEARNING_TEST_PYTHONに実行ファイルを指定してください。");
  const [program, prefix] = found;
  const result = spawnSync(program, [...prefix, "-S", "-m", "unittest", "discover", "-s", "scripts/tests", "-p", "test_character_learning*.py"], {
    cwd: fileURLToPath(new URL("../../", import.meta.url)),
    encoding: "utf8", windowsHide: true, timeout: 60000,
  });
  assert.equal(result.status, 0, result.stderr || result.stdout || String(result.error));
});
