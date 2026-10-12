import test from "node:test";
import { runPythonTests } from "../test-python.mjs";

test("LoRAの標準ライブラリだけで動くPython回帰検査", (context) => {
  runPythonTests(context, {
    directory: "tools/lora", pattern: "test_*.py", minimumVersion: [3, 12],
    // 実トークナイザの3検査は別途の明示実行とし、通常verifyへ引き継がない。
    unsetEnvironment: ["LORA_TEST_MODEL"],
  });
});
