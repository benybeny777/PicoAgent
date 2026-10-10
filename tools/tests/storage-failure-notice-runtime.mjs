// 同じ原因の抑制と操作ごとの復旧を通知の共通定義で検査する。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";
test("保存の原因ごとに一度だけ通知し復旧した操作だけ再通知する", () => {
  const source = readFileSync(new URL("../../ui/app/toast.js", import.meta.url), "utf8");
  const start = source.indexOf("const notifiedFailures =");
  const shown = [];
  const factory = runInNewContext(`${source.slice(start).replace(/^export /gm, "")}\ncreateBackgroundFailureReporter`, {
    logApp() {}, errorDetail: error => error.message, toast: message => shown.push(message),
  });
  const reporter = factory("保存");
  reporter.failure("書込", "容量不足", "書込失敗"); reporter.failure("書込", "容量不足", "書込失敗");
  reporter.failure("書込", "権限不足", "書込失敗"); reporter.failure("読込", "権限不足", "読込失敗");
  assert.equal(shown.length, 3);
  reporter.recover("書込");
  reporter.failure("書込", "容量不足", "書込失敗"); reporter.failure("書込", "権限不足", "書込失敗");
  reporter.failure("読込", "権限不足", "読込失敗");
  assert.equal(shown.length, 5);
});
