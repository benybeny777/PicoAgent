// ホロライブ公式モデルを利用者自身に取得してもらう導線を固定する。
// モデルは同梱・再配布しない。リンク先を公式以外へ変えたり、規約確認を外したりしない。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { installFakeDom } from "./lib/fake-dom.mjs";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");
const [settings, index, markdown] = await Promise.all([read("ui/settings.js"), read("ui/index.html"), read("ui/chat/markdown.js")]);

test("公式モデルリンクは新窓を抑止し、既定ブラウザの成功と失敗を欄内に表示する", async () => {
  const source = settings.slice(settings.indexOf("function officialModelLink("), settings.indexOf("\nlet data;"));
  const calls = [];
  let fail = false;
  const factory = new Function("el", "window", "errorMessage", `${source}; return officialModelLink;`)(
    (_tag, props) => ({ ...props, setAttribute() {}, addEventListener(_event, handler) { this.click = handler; } }),
    { picoagent: { async openExternalUrl(url) { calls.push(url); if (fail) throw new Error("ブラウザ起動失敗"); } } },
    error => error.message,
  );
  const status = { textContent: "" };
  const link = factory("公式ページ", "https://www.mmd.hololive.tv/", status);
  let prevented = 0;
  const event = { preventDefault() { prevented++; } };
  await link.click(event);
  assert.deepEqual(calls, [link.href]);
  assert.match(status.textContent, /既定ブラウザで開きました/);
  fail = true;
  await link.click(event);
  assert.match(status.textContent, /開けませんでした: ブラウザ起動失敗/);
  assert.equal(prevented, 2);
  for (const [links, statusName] of [["LIVE2D_SAMPLE_LINKS", "live2dStatus"], ["HOLOLIVE_LINKS", "hololiveStatus"]]) {
    assert.ok(settings.includes(`${links}.map(([label, href]) => officialModelLink(label, href, ${statusName}))`));
  }
});

test("初回案内は公式リンクだけを既定ブラウザへ渡し、追加ボタンの処理と分ける", async () => {
  const document = installFakeDom();
  const { createOnboarding } = await import("../../ui/app/onboarding.js");
  const dialog = document.createElement("section");
  const calls = [];
  let completed = 0;
  createOnboarding({
    dialog,
    status: { set() {}, flash() {} },
    openExternalUrl: async url => { calls.push(url); },
    onComplete: () => { completed += 1; },
    isEngineBusy: () => false,
  });
  const target = Object.assign(Object.create(Element.prototype), {
    closest: selector => (selector === "a.onboarding-link" ? { href: "https://www.mmd.hololive.tv/" } : null),
  });
  let prevented = false;
  await dialog.listeners.click[0]({ target, preventDefault() { prevented = true; } });
  assert.equal(prevented, true);
  assert.deepEqual(calls, ["https://www.mmd.hololive.tv/"]);
  assert.equal(completed, 0, "公式リンクで追加画面へ遷移してはいけない");
  // 既存の共通処理が起動失敗をトーストに表示することも維持する。
  assert.match(markdown, /export function openExternalUrl\(url\) \{[\s\S]*?toast\("既定ブラウザでリンクを開けませんでした", "error"\)/);
});

test("キャラ登録に公式MMD配布ページ・規約・ガイドラインへのリンクがある", () => {
  const links = settings.match(/const HOLOLIVE_LINKS = \[([\s\S]*?)\n  \];/)?.[1] || "";
  const urls = [...links.matchAll(/"(https:\/\/[^"]+)"/g)].map(match => match[1]);
  assert.deepEqual(urls, [
    "https://www.mmd.hololive.tv/",
    "https://www.mmd.hololive.tv/terms",
    "https://hololivepro.com/terms/",
  ]);
});

test("公式MMDの追加は規約確認と表示名を必須にし、通常のMMD追加へ渡す", () => {
  assert.match(settings, /if \(terms && !terms\.checked\) throw new Error\("ホロライブの利用規約と二次創作ガイドラインを確認してください。"\);/);
  assert.match(settings, /installRawMmd\(folder, \{ idInput: hololiveIdInput, nameInput: hololiveNameInput, terms: hololiveTermsInput, status: hololiveStatus \}\)/);
  assert.match(settings, /if \(!hololiveNameInput\.value\.trim\(\)\) \{/);
  // 公式のLive2Dは一般配布されていないので、存在しない配布ページへは案内しない。
  assert.doesNotMatch(settings, /hololive[^"\n]*live2d/i);
});

test("初回案内から公式MMD配布ページを開ける", () => {
  assert.match(index, /<a class="onboarding-link" href="https:\/\/www\.mmd\.hololive\.tv\/" target="_blank" rel="noreferrer"><b>ホロライブ公式MMDを取得<\/b>/);
  assert.match(index, /data-onboarding-action="add-character"><b>取得したモデルをPicoAgentへ追加<\/b>/);
});

test("Live2Dは公式サンプル（桃瀬ひより など）の日本語ページへ案内する", () => {
  const links = settings.match(/const LIVE2D_SAMPLE_LINKS = \[([\s\S]*?)\n  \];/)?.[1] || "";
  const urls = [...links.matchAll(/"(https:\/\/[^"]+)"/g)].map(match => match[1]);
  assert.deepEqual(urls, [
    "https://www.live2d.com/learn/sample/",
    "https://www.live2d.com/eula/live2d-sample-model-terms_jp.html",
  ]);
  // 英語版（/en/）へは案内しない。
  assert.doesNotMatch(settings + index, /live2d\.com\/en\//);
  assert.match(index, /<b>Live2D公式サンプル（桃瀬ひより など）を取得<\/b>/);
});
