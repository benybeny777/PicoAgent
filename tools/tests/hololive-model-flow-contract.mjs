// ホロライブ公式モデルを利用者自身に取得してもらう導線を固定する。
// モデルは同梱・再配布しない。リンク先を公式以外へ変えたり、規約確認を外したりしない。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");
const [settings, index] = await Promise.all([read("ui/settings.js"), read("ui/index.html")]);

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
