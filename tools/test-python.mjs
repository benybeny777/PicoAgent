import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// Python探索と有限時間の起動は、通常検証の各入口で共有する。
export function runPythonTests(context, { directory, pattern, minimumVersion, unsetEnvironment = [] }) {
  const environment = { ...process.env };
  for (const name of unsetEnvironment) delete environment[name];
  const candidates = environment.LEARNING_TEST_PYTHON
    ? [[environment.LEARNING_TEST_PYTHON, []]]
    : [["python", []], ["python3", []], ["py", ["-3"]]];
  const minimum = minimumVersion.join(".");
  const found = candidates.find(([program, prefix]) => {
    const result = spawnSync(program, [...prefix, "-B", "-S", "-c",
      `import sys; assert sys.version_info >= (${minimumVersion.join(",")})`], {
      env: environment, windowsHide: true, timeout: 10000,
    });
    return result.status === 0;
  });
  assert.ok(found, `Python ${minimum}以上が必要です。PATHまたはLEARNING_TEST_PYTHONに実行ファイルを指定してください。`);
  const [program, prefix] = found;
  const result = spawnSync(program, [...prefix, "-B", "-S", "-m", "unittest", "discover", "-s", directory, "-p", pattern], {
    cwd: fileURLToPath(new URL("../", import.meta.url)), env: environment,
    encoding: "utf8", windowsHide: true, timeout: 60000,
  });
  const output = [result.stdout, result.stderr, result.error?.message].filter(Boolean).join("\n").trim();
  // unittestの件数とskipもverifyの出力に残し、成功時に検査省略を隠さない。
  if (output) context.diagnostic(output);
  assert.equal(result.status, 0, output || "Python回帰検査が正常終了しませんでした。");
}
