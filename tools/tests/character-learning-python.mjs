import test from "node:test";
import { runPythonTests } from "../test-python.mjs";

// 外部モデルやPythonパッケージを取得せず、学習工程の保護境界を通常検証に含める。
test("音声学習と管理環境のPython回帰検査", (context) => {
  runPythonTests(context, {
    directory: "scripts/tests", pattern: "test_character_learning*.py", minimumVersion: [3, 10],
  });
});
