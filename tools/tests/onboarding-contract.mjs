// 初回セットアップの契約を、実画面を起こさずに固定する。
// 「追加機能を準備する？」の選択画面は廃止した。1件のために全画面のモーダルを挟む
// 価値がなく、選べない項目（提供元の公式インストーラが要るもの）まで並んでいたため、
// 一覧が固定高で切れる・状態確定でカードが跳ねる・状態取得前の選択が残る、といった
// 不具合が同時に出ていた。読み上げは選ばせず自動導入し、進捗は起動画面へ出す。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const base = new URL("../../", import.meta.url);
const read = name => readFile(new URL(name, base), "utf8");
const indexHtml = await read("ui/index.html");
const renderer = await read("ui/renderer.js");
const styles = await read("ui/styles.css");
const settings = await read("ui/settings.js");
const commands = await read("src-tauri/src/commands.rs");
const settingsDoc = await read("docs/SETTINGS.md");
const spec = await read("SPEC.md");

test("初回案内はLive2D追加の1画面だけで、追加機能の選択画面を持たない", () => {
  assert.match(indexHtml, /id="onboardingCharacterStep"/);
  assert.doesNotMatch(indexHtml, /onboardingComponentStep/);
  assert.doesNotMatch(indexHtml, /data-component-id=/);
  assert.doesNotMatch(indexHtml, /data-onboarding-action="(install|skip|back)-components"/);
  // 1画面になったので、既定の見出し参照もLive2D側を指す。
  assert.match(indexHtml, /id="onboarding"[^>]*aria-labelledby="onboardingCharacterTitle"/);
});

test("消した機能は案内せず終わらせず、設定への一行だけ残す", () => {
  assert.match(indexHtml, /onboarding-component-note/);
  assert.match(indexHtml, /設定 → 追加機能・更新/);
  assert.match(styles, /\.onboarding-component-note \{/);
});

test("廃止した画面のコードとCSSを残さない", () => {
  for (const dead of [
    "showOnboardingStep",
    "refreshOnboardingComponents",
    "settleOnboardingComponentBadges",
    "installSelectedOnboardingComponents",
    "setOnboardingComponentsBusy",
    "onboardingComponentStatus",
    "onboardingComponentStep",
  ]) {
    assert.doesNotMatch(renderer, new RegExp(dead), `renderer.js に ${dead} が残っています`);
  }
  for (const dead of [
    "\\.onboarding-components",
    "\\.onboarding-component-status",
    "\\.onboarding-component-actions",
    "\\.onboarding-back",
  ]) {
    assert.doesNotMatch(styles, new RegExp(dead), `styles.css に ${dead} が残っています`);
  }
});

test("音声読み上げは選ばせず初回に自動導入し、成功時に読み上げを有効にする", () => {
  assert.match(renderer, /async function autoInstallVoiceOnFirstRun\(\)/);
  // 案内の表示を待たせない（await しない）。
  assert.match(renderer, /showOnboarding\(\);\s*\n[\s\S]{0,200}?autoInstallVoiceOnFirstRun\(\)\.catch\(/);
  // 導入できた時点で読み上げを有効にする（第2引数 true）。
  assert.match(renderer, /installComponents\(\["aivis"\], true\)/);
  // 取得済み・取得不能では走らせない。
  assert.match(renderer, /if \(!aivis \|\| aivis\.ready \|\| !aivis\.available\) return;/);
});

test("取得進捗は専用画面を作らず起動画面の状態表示へ出す", () => {
  assert.match(renderer, /window\.picoagent\.listen\("component:progress"/);
  // setStatus は起動中なら起動画面の状態欄と起動ログ、通常画面ではステータス行へ流す。
  assert.match(renderer, /component:progress"[\s\S]{0,700}?setStatus\(`音声読み上げを準備中…/);
  // 送信を塞いでいるローカルモデル取得の理由を上書きしない。
  assert.match(renderer, /if \(localEngineRequired && localEnginePhase !== "ready"\) return;/);
  // 容量は十進のMBで出す（bytes もMiBも使わない）。
  assert.match(renderer, /bytes \/ 1_000_000/);
  // 失敗しても黙らせず、入れ直し先を出す。
  assert.match(renderer, /設定の「追加機能・更新」から入れ直せます/);
  assert.match(renderer, /オフラインのため音声読み上げの準備を見送りました/);
});

test("読み上げ音量の既定は実装・設定画面・文書で35に揃える", () => {
  // 画面側の初期値だけ 1（=100%）だと、設定を読む前に鳴る音が最大音量になる。
  assert.match(renderer, /^let voiceVolume = 0\.35;/m);
  assert.doesNotMatch(commands, /PICOAGENT_TTS_VOLUME", "50"/);
  assert.equal(commands.match(/PICOAGENT_TTS_VOLUME", "35"/g)?.length, 2);
  assert.match(settings, /PICOAGENT_TTS_VOLUME[^)]*初期値35[^)]*"35"[^)]*"35"/);
  assert.match(settingsDoc, /\| `PICOAGENT_TTS_VOLUME` \| `35` \|/);
});

test("仕様書が1画面・自動導入・起動画面の進捗を正本として書いている", () => {
  assert.match(spec, /初回セットアップはLive2D追加案内の1画面だけとする/);
  assert.match(spec, /音声読み上げ（Aivis）は選択肢にせず、初回起動で自動導入する/);
  assert.match(spec, /進捗は専用の画面を作らず起動画面の状態表示/);
  assert.doesNotMatch(spec, /初回セットアップは「追加機能 → Live2D追加案内」の二段階とする/);
});
