// 入力欄の「打ち直させない」契約を固定する。
// 書きかけの復元と、失敗した送信のやり直しは、どちらも壊れても画面上は静かに壊れる
// （何も起きない／添付だけ落ちた同じ見た目の要求が飛ぶ）ため、静的検査で押さえる。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

await import("../../ui/composer-draft.js");

const draftStore = globalThis.PicoComposerDraft;
const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const html = await readFile(new URL("../../ui/index.html", import.meta.url), "utf8");

const NOW = Date.UTC(2026, 8, 12, 9, 0, 0);

test("保存した書きかけをそのまま戻す", () => {
  const raw = draftStore.serializeDraft("あとで聞きたいこと", NOW);
  assert.equal(draftStore.readDraft(raw, NOW + 60_000), "あとで聞きたいこと");
});

test("壊れた保存値は書きかけ無しとして扱い、例外にしない", () => {
  for (const raw of [null, "", "{", "[]", '"文字列"', "{}", '{"draft":123,"draftAt":1}']) {
    assert.equal(draftStore.readDraft(raw, NOW), "", `raw=${String(raw)}`);
  }
});

test("日時が無い・壊れている書きかけは戻さない", () => {
  assert.equal(draftStore.readDraft('{"draft":"あ"}', NOW), "");
  assert.equal(draftStore.readDraft('{"draft":"あ","draftAt":"きのう"}', NOW), "");
  assert.equal(draftStore.readDraft('{"draft":"あ","draftAt":0}', NOW), "");
});

test("古い書きかけは戻さない（いまの話題と食い違うため）", () => {
  const raw = draftStore.serializeDraft("きのうの書きかけ", NOW);
  assert.equal(draftStore.readDraft(raw, NOW + draftStore.DRAFT_TTL_MS - 1), "きのうの書きかけ");
  assert.equal(draftStore.readDraft(raw, NOW + draftStore.DRAFT_TTL_MS), "");
});

test("極端に長い書きかけは上限で切り、絵文字を割らない", () => {
  const long = "あ".repeat(draftStore.DRAFT_MAX_CHARS + 500);
  assert.equal(Array.from(draftStore.readDraft(draftStore.serializeDraft(long, NOW), NOW)).length, draftStore.DRAFT_MAX_CHARS);
  // サロゲートペアを UTF-16 単位で切ると壊れた文字が残る。
  const emoji = "🙂".repeat(draftStore.DRAFT_MAX_CHARS + 10);
  const stored = draftStore.readDraft(draftStore.serializeDraft(emoji, NOW), NOW);
  assert.ok(!stored.includes("�"));
  assert.equal(Array.from(stored).every(char => char === "🙂"), true);
});

test("画面は専用モジュールを読み込み、正規化を自前で書かない", () => {
  assert.match(html, /<script src="composer-draft\.js"><\/script>/);
  assert.match(renderer, /globalThis\.PicoComposerDraft/);
  assert.match(renderer, /composerDraftStore\.readDraft\(localStorage\.getItem\(COMPOSER_DRAFT_STORAGE_KEY\)\)/);
  assert.match(renderer, /composerDraftStore\.serializeDraft\(promptDraft\)/);
});

test("書きかけは終了時にも必ず書き切る", () => {
  // デバウンス待ちのまま閉じると、直前に打った内容だけが消える。
  const unload = renderer.slice(renderer.indexOf('window.addEventListener("beforeunload"'));
  const body = unload.slice(0, unload.indexOf("\n});"));
  assert.match(body, /promptDraft = input\.value;\s*\n\s*persistDraft\(\);/);
});

test("入力履歴を第2の保存先へ複製しない", () => {
  // 入力履歴は起動時に hydrateHistory() が会話履歴から組み直す。別の保存先を作ると
  // 同じ状態を二重に書き換えることになる。
  assert.doesNotMatch(renderer, /localStorage\.(?:get|set)Item\([^)]*[Hh]istory/);
  assert.match(renderer, /function hydrateHistory[\s\S]{0,200}promptHistory\.length = 0;/);
});

test("定義の無い入力履歴呼び出しを残さない", () => {
  // 未定義の関数・変数を参照する行は、押すたびに ReferenceError を投げてアプリログを
  // 汚すだけで何も起きない。↑↓ の呼び戻しは recallPromptHistory に一本化する。
  assert.doesNotMatch(renderer, /recallInputHistory|inputHistoryPos/);
  assert.match(renderer, /event\.key === "ArrowUp"[\s\S]{0,120}recallPromptHistory\(-1\)/);
  assert.match(renderer, /event\.key === "ArrowDown"[\s\S]{0,160}recallPromptHistory\(1\)/);
});

test("やり直しと再生成は添付ごと送り直す", () => {
  // 文面だけ戻すと、見た目は同じ質問なのに添付の無い別の要求が飛ぶ。
  assert.match(renderer, /lastUserImages = pendingImages;\s*\n\s*lastUserFiles = pendingFiles;/);
  const restore = renderer.slice(renderer.indexOf("function restoreLastAttachments"));
  assert.match(restore.slice(0, 400), /attachedImages = \[\.\.\.lastUserImages\];/);
  assert.match(restore.slice(0, 400), /attachedFiles = \[\.\.\.lastUserFiles\];/);
  const retry = renderer.slice(renderer.indexOf("function addRetryButton"), renderer.indexOf("function addRetryButton") + 900);
  assert.match(retry, /restoreLastAttachments\(\);/);
  // 入力欄を開いてから文面を入れる（逆順だと書きかけの復元に上書きされる）。
  assert.ok(retry.indexOf("openComposer()") < retry.indexOf("setInputValue(lastUserText)"));
  const regen = renderer.slice(renderer.indexOf('regenerateReplyButton?.addEventListener("click"'));
  const regenBody = regen.slice(0, regen.indexOf("\n});"));
  assert.match(regenBody, /restoreLastAttachments\(\);/);
  assert.ok(regenBody.indexOf("openComposer()") < regenBody.indexOf("setInputValue(lastUserText)"));
});

test("添付の控えは新しい会話で捨て、打ちかけのテキストは消さない", () => {
  const reset = renderer.slice(renderer.indexOf('window.picoagent.on("pet:reset"'));
  const body = reset.slice(0, reset.indexOf("\n});"));
  assert.match(body, /lastUserImages = \[\];\s*\n\s*lastUserFiles = \[\];/);
  // 入力欄の中身は利用者が打ったものなので、会話のリセットでは消さない。
  assert.doesNotMatch(body, /setInputValue\(""\)|input\.value = ""/);
  // 書きかけの保存し直しは closeComposer() が行う（2か所で書き戻さない）。
  assert.match(body, /closeComposer\(\);/);
});
