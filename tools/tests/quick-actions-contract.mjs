// クイック操作の「押せば用が済むボタンだけ出す」契約を固定する。
// 連携スキルに頼る既定ボタンが未設定のまま並ぶと、押しても設定案内しか返らない。
// 逆に判定を誤ると、設定済みの人のボタンが黙って消える。どちらも画面上は静かに壊れる。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

await import("../../ui/quick-actions.js");

const quick = globalThis.PicoQuickActions;
const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");
const [composer, main, html, shim, lib, commands] = await Promise.all([
  read("ui/chat/composer.js"),
  read("ui/app/main.js"),
  read("ui/index.html"),
  read("ui/picoagent.js"),
  read("src-tauri/src/lib.rs"),
  read("src-tauri/src/commands.rs"),
]);
const labels = items => items.map(item => item.label);

test("未設定と確認したキーの連携ボタンを出さない", () => {
  assert.deepEqual(labels(quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, {})), [
    "👀 画面を見る",
    "📋 クリップボード要約",
    "🗓 今日の予定",
    "⏩ さっきの続き",
    "✨ できること",
  ]);
  assert.deepEqual(labels(quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, undefined)), labels(quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, {})));
});

test("Slack要約は Bot token と既定チャンネルの両方がそろったときだけ出す", () => {
  const slack = "💬 Slack要約";
  const tokenOnly = quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, { PICOAGENT_SLACK_BOT_TOKEN: true });
  assert.ok(!labels(tokenOnly).includes(slack));
  const webhookOnly = quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, { PICOAGENT_SLACK_WEBHOOK_URL: true, PICOAGENT_SLACK_CHANNEL: true });
  assert.ok(!labels(webhookOnly).includes(slack));
  const both = quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, { PICOAGENT_SLACK_BOT_TOKEN: true, PICOAGENT_SLACK_CHANNEL: true });
  assert.ok(labels(both).includes(slack));
});

test("すべて設定済みなら既定の8件を元の順で出す", () => {
  const presence = Object.fromEntries(quick.requiredKeys(quick.DEFAULT_QUICK_ACTIONS).map(key => [key, true]));
  assert.deepEqual(labels(quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, presence)), labels(quick.DEFAULT_QUICK_ACTIONS));
  assert.equal(quick.DEFAULT_QUICK_ACTIONS.length, 8);
});

test("値が true と確認できたキーだけを設定済みとして扱う", () => {
  const presence = { PICOAGENT_GMAIL_ACCESS_TOKEN: "true" };
  assert.ok(!labels(quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, presence)).includes("📧 未読メール要約"));
});

test("問い合わせるキーは重複なしで、バックエンドの受け付け条件（接頭辞と32件以内）に収まる", () => {
  const keys = quick.requiredKeys(quick.DEFAULT_QUICK_ACTIONS);
  assert.equal(new Set(keys).size, keys.length);
  assert.ok(keys.length > 0 && keys.length <= 32);
  for (const key of keys) assert.match(key, /^PICOAGENT_[A-Z0-9_]+$/);
  assert.match(commands, /const CONFIG_PRESENCE_MAX_KEYS: usize = 32;/);
});

test("利用者が書いたクイック操作は絞り込まない", () => {
  const items = quick.parseQuickActions("📧 メール|Gmailの未読を見て\n不正な行\n|本文だけ\n");
  assert.deepEqual(items, [{ label: "📧 メール", text: "Gmailの未読を見て" }]);
  assert.deepEqual(quick.visibleQuickActions(items, {}), items);
  assert.equal(quick.parseQuickActions(""), null);
});

test("画面は quick-actions.js の既定と絞り込みを使い、起動時に設定状態を問い合わせる", () => {
  assert.doesNotMatch(composer, /const QUICK_ACTIONS\s*=/);
  assert.doesNotMatch(composer, /function parseQuickActions\(/);
  assert.match(composer, /const quick = window\.PicoQuickActions;/);
  assert.match(composer, /quickItems = quick\.parseQuickActions\(text\);/);
  assert.match(composer, /quickItems \|\| defaultItems/);
  assert.match(main, /composer\.setQuickActions\(info\.quickActions\);\s*composer\.refreshDefaultQuickActions\(\);/);
  const quickScript = html.indexOf('<script src="quick-actions.js');
  assert.ok(quickScript > 0 && quickScript < html.indexOf('<script type="module" src="app/main.js'), "quick-actions.js はメイン画面のモジュールより先に読み込む");
  assert.match(shim, /configPresence: \(keys = \[\]\) => call\("pet_config_presence", \{ keys \}\)/);
  assert.match(lib, /commands::pet_config_presence,/);
});

test("設定の保存直後にも、自分のクイック操作と既定ボタンの出し分けを反映する", () => {
  const handler = main.slice(main.indexOf('bridge.on("pet:ui"'));
  const body = handler.slice(0, handler.indexOf("\n});"));
  assert.match(body, /if \(ui\.quickActions != null\) composer\.setQuickActions\(ui\.quickActions\);/);
  assert.match(body, /composer\.refreshDefaultQuickActions\(\);/);
  assert.match(commands, /ui\["quickActions"\] = json!\(state\.config\.get\("PICOAGENT_QUICK_ACTIONS", ""\)\);/);
});

test("狭幅で省略するのは通常一覧だけで診断と質問選択肢を残す", async () => {
  const css = await read("ui/components/chat.css");
  assert.doesNotMatch(css, /@container \(max-width: 260px\)\s*\{\s*\.quick-actions\s*\{/);
  assert.match(css, /\.quick-actions\[data-state="ready"\] \{ display: none;/);
  for (const state of ["loading", "error", "choices"]) assert.ok(css.includes(`.quick-actions[data-state="${state}"]`));
  assert.match(css, /\.draft-recovery\s*\{[^}]*overflow-y: auto;/);
  assert.match(css, /\.chat:has\(\.draft-recovery:not\(\[hidden\]\)\)[^{]*\{[^}]*overflow-y: auto;/);
});
