// メイン画面の入力・送信・リセットの「静かに壊れる」契約。
// 入力欄（ui/chat/composer.js）は実物を最小の DOM で動かし、送信（ui/chat/send.js）と
// 録音（ui/app/mic.js）は非同期の競合を防ぐ判定の置き場所を固定する。
// 設定画面の同種の契約は ui-input-feedback-contract.mjs にある。
import assert from "node:assert/strict";
import test from "node:test";
import { installFakeDom } from "./lib/fake-dom.mjs";
import { mainScript, uiFile } from "./lib/ui-source.mjs";

const document = installFakeDom();
await import("../../ui/composer-draft.js");
await import("../../ui/quick-actions.js");
const { createComposer } = await import("../../ui/chat/composer.js");
const send = uiFile("chat/send.js");
const mic = uiFile("app/mic.js");
const panel = uiFile("chat/panel.js");
const main = uiFile("app/main.js");
const status = uiFile("app/status.js");

for (const kind of ["image", "file"]) {
  test(`同時に読む${kind}とネイティブ添付が競合しても4件の上限を守る`, async () => {
    const readers = [];
    globalThis.FileReader = class {
      readAsDataURL() { readers.push(this); }
    };
    const { composer } = makeComposer();
    const loading = Array.from({ length: 6 }, (_, index) => composer.addFile({ name: `${index}.txt`, type: kind === "image" ? "image/png" : "text/plain" }));
    assert.equal(readers.length, 6);
    assert.equal(composer.addLoaded({ kind, name: "ネイティブ", dataUrl: "native" }), "");
    for (const [index, reader] of readers.entries()) reader.onload({ target: { result: `loaded-${index}` } });
    await Promise.all(loading);
    assert.equal(kind === "image" ? composer.images.length : composer.files.length, 4);
  });
}
const notice = uiFile("app/notice.js");
const markdown = uiFile("chat/markdown.js");

function makeComposer({ value = "下書き", history = ["前の入力", "直前の入力"] } = {}) {
  const make = tag => document.createElement(tag);
  const input = make("textarea");
  const form = make("form");
  const sendButton = make("button");
  const flashes = [];
  let submitted = 0;
  const composer = createComposer({
    form, input, preview: make("div"), suggestions: make("div"), quickActions: make("div"), sendButton,
    status: { set() {}, clear() {}, flash: text => flashes.push(text), setBusy() {} },
  });
  composer.onSubmit(() => { submitted += 1; });
  composer.hydratePrompts(history.map(content => ({ role: "user", content })));
  input.value = value;
  input.selectionStart = input.selectionEnd = 0;
  const press = (key, extra = {}) => {
    let prevented = false;
    input.dispatch("keydown", { key, preventDefault() { prevented = true; }, ...extra });
    return prevented;
  };
  return { composer, input, form, sendButton, press, submitted: () => submitted, flashes };
}

test("上下キーは履歴と下書きを往復する", () => {
  const { input, press } = makeComposer();
  assert.equal(press("ArrowUp"), true);
  assert.equal(input.value, "直前の入力");
  input.selectionStart = input.selectionEnd = input.value.length;
  assert.equal(press("ArrowDown"), true);
  assert.equal(input.value, "下書き");
});

test("文中と選択中の上下キーは通常のカーソル操作を保つ", () => {
  const { input, press } = makeComposer();
  input.selectionStart = input.selectionEnd = 1;
  assert.equal(press("ArrowUp"), false);
  assert.equal(press("ArrowDown"), false);
  input.selectionStart = 0;
  assert.equal(press("ArrowUp"), false);
  assert.equal(input.value, "下書き");
});

test("IME候補操作と変換確定は履歴・送信へ流さない", () => {
  for (const extra of [{ isComposing: true }, { keyCode: 229 }]) {
    const ui = makeComposer();
    for (const key of ["ArrowUp", "ArrowDown", "Enter", "Escape"]) assert.equal(ui.press(key, extra), false);
    assert.equal(ui.input.value, "下書き");
    assert.equal(ui.submitted(), 0);
  }
});

test("修飾キー付きの上下操作は履歴で入力を置き換えない", () => {
  for (const modifier of ["shiftKey", "ctrlKey", "altKey", "metaKey"]) {
    const { input, press } = makeComposer();
    assert.equal(press("ArrowUp", { [modifier]: true }), false);
    assert.equal(input.value, "下書き");
    press("ArrowUp");
    input.selectionStart = input.selectionEnd = input.value.length;
    assert.equal(press("ArrowDown", { [modifier]: true }), false);
    assert.equal(input.value, "直前の入力");
    assert.equal(press("ArrowDown"), true);
    assert.equal(input.value, "下書き");
  }
});

test("Enterで送信し、Shift+Enterは改行のまま", () => {
  const ui = makeComposer();
  assert.equal(ui.press("Enter", { shiftKey: true }), false);
  assert.equal(ui.submitted(), 0);
  assert.equal(ui.press("Enter"), true);
  assert.equal(ui.submitted(), 1);
});

test("音声・提案・再利用で入れた文は、入力履歴の途中でも新しい下書きになる", () => {
  for (const put of [
    composer => { composer.insert("追加テキスト"); },
    composer => { composer.reuse("記録の続き"); },
    composer => { composer.setValue("確定した音声"); composer.resetNavigation(); },
  ]) {
    const { composer, input, press } = makeComposer();
    press("ArrowUp"); // 履歴の途中にいる
    put(composer);
    const draft = input.value;
    assert.notEqual(draft, "直前の入力");
    // 書きかけとしてすぐ書き切る（遅延保存を待つ間に閉じても消えない）。
    assert.equal(window.PicoComposerDraft.readDraft(localStorage.getItem("picoagent.composerDraft.v1")), draft);
    input.selectionStart = input.selectionEnd = 0;
    press("ArrowUp");
    input.selectionStart = input.selectionEnd = input.value.length;
    press("ArrowDown");
    assert.equal(input.value, draft, "履歴から戻ると新しい下書きへ戻る");
  }
});

test("処理中は入力欄を開かず、送れない理由を送信ボタンに出す", () => {
  const { composer, form, sendButton } = makeComposer();
  composer.close();
  composer.setBlocked({ busy: true, reason: "" });
  composer.open();
  assert.equal(form.hidden, true);
  composer.setBlocked({ busy: false, reason: "ローカルAIの準備完了後に送信できます" });
  assert.equal(sendButton.disabled, true);
  assert.equal(sendButton.title, "ローカルAIの準備完了後に送信できます");
  composer.setBlocked({ busy: false, reason: "" });
  assert.equal(sendButton.disabled, false, "入力があり理由も無ければ送れる");
});

test("会話リセット後に旧質問の再生成や旧回答のコピーをしない", () => {
  const reset = send.slice(send.indexOf("    reset() {"), send.indexOf("\n    },", send.indexOf("    reset() {")));
  assert.match(reset, /revision \+= 1;/);
  assert.match(reset, /last = \{ text: "", images: \[\], files: \[\] \};/);
  assert.match(reset, /voice\.cancelStream\(\);\s*\n\s*voice\.stop\(\);/);
  assert.match(reset, /chat\.lastAssistantText = "";/);
  assert.match(reset, /chat\.history\.clear\(\);/);
  assert.match(reset, /composer\.clearAfterReset\(\);/);
  assert.match(reset, /updateButtons\(\);/);
  // コピーは直前の回答（無ければ表示中の本文＝リセット後の案内）を使う。
  assert.match(main, /copyText\(chat\.lastAssistantText \|\| chat\.reply\.textContent\)/);
});

test("リセット前の進捗・ストリーム描画・通信結果は新しい会話へ戻さない", () => {
  const subscribe = send.slice(send.indexOf("async function subscribe("), send.indexOf("async function readAttachments("));
  assert.match(subscribe, /const alive = \(\) => busy && requestRevision === revision && !cancelled\(\);/);
  // どのイベントも最初に alive() を確かめる。
  for (const channel of ["pet:agent_status", "pet:agent_activity", "pet:stream_reset", "pet:stream_chunk"]) {
    const body = subscribe.slice(subscribe.indexOf(`add("${channel}"`));
    assert.match(body.slice(0, 160), /if \(!alive\(\)/, `${channel} がリセット後も画面を書き換えます`);
  }
  // 予約済みの描画フレームも、描く直前に確かめ直す。
  assert.match(subscribe, /const paint = \(\) => \{\s*\n\s*if \(!alive\(\)\) \{ stream\.frame = 0; return; \}/);
  const submit = send.slice(send.indexOf("async function submit()"), send.indexOf("  const sender = {"));
  assert.match(submit, /if \(result\?\.stale \|\| requestRevision !== revision \|\| cancelled\(\)\) \{\s*\n\s*job\.state = "cancelled";/);
  // 通信失敗も、リセット済みなら新しい会話へ書き戻さない（通常の失敗は表示する）。
  assert.match(submit, /\} catch \(error\) \{\s*\n\s*if \(requestRevision !== revision\) \{ job\.state = "cancelled"; return; \}\s*\n\s*if \(cancelled\(\)\) \{ job\.state = "cancelled"; return; \}\s*\n\s*job\.state = "failed";[\s\S]*?showFailure\(`応答に失敗しました: \$\{errorMessage\(error\)\}`\);/);
  // 履歴と終了操作は、回答を出した後でもリセット済みなら渡さない。
  assert.ok(submit.indexOf('if (requestRevision !== revision) { job.state = "cancelled"; return; }\n      chat.history.add("assistant", reply);') > 0);
});

test("添付の読み込み中・画面取得後にリセットした質問を新しい会話へ送らない", () => {
  const attachments = send.slice(send.indexOf("async function readAttachments("), send.indexOf("function showFailure("));
  assert.match(attachments, /for \(const file of files\) \{\s*\n\s*if \(requestRevision !== revision \|\| cancelled\(\)\) return null;/);
  const submit = send.slice(send.indexOf("async function submit()"), send.indexOf("  const sender = {"));
  assert.match(submit, /if \(fileContext == null\) \{ job\.state = "cancelled"; return; \}/);
  const shot = submit.indexOf("bridge.captureScreen()");
  const recheck = submit.indexOf('if (requestRevision !== revision || cancelled()) { job.state = "cancelled"; return; }', shot);
  const sendAt = submit.indexOf("bridge.sendImage(fullText, images)");
  assert.ok(shot > 0 && recheck > shot && sendAt > recheck, "画面取得の後、送信の前にリセットを確かめ直していません");
});

test("再生成は送信控えとAIの準備が揃い処理中でないときだけ有効になる", () => {
  const update = send.slice(send.indexOf("function updateButtons()"), send.indexOf("function clearCancelRelease()"));
  assert.match(update, /regenerate\.disabled = busy \|\| Boolean\(blockedReason\) \|\| !hasLastRequest\(\);/);
  // 添付だけの送信も控えとして数える。
  assert.match(send, /return Boolean\(last\.text\) \|\| last\.images\.length > 0 \|\| last\.files\.length > 0;/);
});

test("停止要求のあと応答が返らなくても画面の待機を解除できる", () => {
  // 停止を押したあと実行中の送信が返らないと、待機表示が残って入力も再送もできず、
  // 復帰する手段が無くなる。一定時間で画面側だけ中断扱いにして戻す。
  assert.match(send, /const CANCEL_RELEASE_MS = \d+;/);
  const cancel = send.slice(send.indexOf("async cancel(button) {"));
  assert.ok(cancel.indexOf("cancelTimer = setTimeout(") < cancel.indexOf("await bridge.cancel()"),
    "cancel IPC自体が返らない場合にも備え、待機解除の見張りを先に開始してください");
  assert.match(cancel, /const release = releasePending;/);
  assert.match(cancel, /release\(\{ stale: true, releasedByCancel: true \}\)/);
  assert.match(send, /const released = new Promise\(resolve => \{ releasePending = resolve; \}\);/);
  assert.match(send, /Promise\.race\(\[images\.length \? bridge\.sendImage\(fullText, images\) : bridge\.send\(fullText\), released\]\)/);
  assert.match(send, /if \(result\?\.releasedByCancel\) status\.flash\("停止しました/);
  // 見張りは送信の後始末で必ず解く（次の送信へ持ち越さない）。
  assert.match(send, /clearCancelRelease\(\);\s*\n\s*busy = false;/);
});

test("生成中の本文描画は最短間隔をあけて組み直す", () => {
  // 1回の描画で本文全体を組み直すため、トークンが毎フレーム届くローカルAIでは
  // 長い回答ほど入力とアニメが引っかかっていた。読める速さは保ったまま間引く。
  assert.match(send, /const STREAM_RENDER_MIN_MS = 80;/);
  assert.match(send, /if \(performance\.now\(\) - stream\.lastAt < STREAM_RENDER_MIN_MS\) \{\s*\n\s*stream\.frame = requestAnimationFrame\(paint\);\s*\n\s*return;/);
  // 送信直後とステップ境界では、最初のチャンクを待たせない。
  assert.match(send, /const stream = \{ buffer: "", frame: 0, lastAt: 0 \};/);
  assert.match(send, /stream\.lastAt = 0;\s*\n\s*chat\.renderStreaming\(""\);/);
});

test("前の録音の遅延結果や失敗は、再録音の入力と送信予約を変更しない", () => {
  const finish = mic.slice(mic.indexOf("async function finish()"), mic.indexOf("async function start()"));
  assert.match(finish, /const isCurrent = \(\) => current === session && request === startRequest;/);
  // 変換を待つたびに、自分の録音のままか確かめ直す。
  assert.match(finish, /const data = await blobToDataUrl\([\s\S]*?\);\s*\n\s*if \(!isCurrent\(\)\) return;/);
  assert.match(finish, /const result = await bridge\.transcribeAudio\(data\);\s*\n\s*if \(!isCurrent\(\)\) return;/);
  assert.match(finish, /\} catch \(error\) \{\s*\n\s*if \(!isCurrent\(\)\) return;/);
  assert.match(finish, /\} finally \{\s*\n\s*if \(isCurrent\(\)\) \{/);
  // 確定した文は新しい下書きにしてから送信予約する。
  assert.match(finish, /composer\.resetNavigation\(\);\s*\n\s*scheduleSubmit\(\);/);
  // 確定に失敗しても途中結果は残す。
  assert.match(finish, /確定に失敗したため途中結果を残しました/);
});

test("前の逐次文字起こしの失敗は次の録音のタイマーを止めない", () => {
  const interim = mic.slice(mic.indexOf("async function transcribeInterim(current)"), mic.indexOf("function stopCapture()"));
  assert.match(interim, /\} catch \(error\) \{\s*\n[^\n]*\n\s*if \(current !== session\) return;/);
  assert.match(interim, /\} finally \{\s*\n\s*interimBusy = false;/);
});

test("録音の再開はマイク許可を待つ前に以前の送信予約を取り消し、古い許可要求は新しい録音を消さない", () => {
  const start = mic.slice(mic.indexOf("async function start()"), mic.indexOf("function setContinuous("));
  assert.ok(start.indexOf("cancelPendingSubmit(") < start.indexOf("getUserMedia("));
  assert.match(start, /const request = \+\+startRequest;/);
  assert.match(start, /\} catch \(error\) \{\s*\n\s*if \(request !== startRequest\) return;/);
  assert.match(start, /if \(request === startRequest && !stream\) \{\s*\n\s*starting = false;\s*\n\s*micButton\.disabled = false;/);
  // 画面が隠れている間に届いた許可は使わない。
  assert.match(start, /if \(request !== startRequest \|\| document\.hidden\) \{\s*\n\s*media\.getTracks\(\)\.forEach\(track => track\.stop\(\)\);/);
});

test("コードコピーは成功・失敗を区別し、連打を受け付けず再操作できる", () => {
  const copy = markdown.slice(markdown.indexOf("async function copyCode("), markdown.indexOf("const APPROVAL_MARKER"));
  assert.match(copy, /if \(button\.disabled\) return;\s*\n\s*button\.disabled = true;/);
  assert.match(copy, /\(await copyText\(text\)\) \? "コピーしました" : "コピーできませんでした"/);
  assert.match(copy, /setTimeout\(\(\) => \{ button\.textContent = "コピー"; button\.disabled = false; \}, 1500\);/);
});

test("ダブルクリックで全画面へ切り替えず、拡大ボタンだけを使う", () => {
  // ダブルクリックは配置の移動つまみ（既定へ戻す）だけ。会話パネル・キャラには付けない。
  const offenders = mainScript().split("\n").filter(line => line.includes('addEventListener("dblclick"') && !line.includes("move.addEventListener"));
  assert.deepEqual(offenders, []);
  assert.match(panel, /expandButton = el\("button", \{ type: "button", className: "chip", textContent: "拡大"/);
});

test("準備中・失敗の理由は、一時表示を消しても消えない", () => {
  // 状態行は常設（送れない理由）と一時（読み上げ中・コピーしました）の2層。一時表示を消すと常設へ戻る。
  assert.match(status, /const text = transient \|\| sticky;/);
  assert.match(status, /clear\(by = ""\) \{\s*\n\s*if \(by && owner !== by\) return;/);
  // 状態行を直接空にする経路を作らない（status.js だけが本文を書く）。
  const outside = mainScript().split("// ==== app/status.js ====")[0] + mainScript().split("// ==== app/status.js ====")[1].split("// ==== ")[1];
  assert.doesNotMatch(outside, /statusElement\.textContent\s*=|#status"\)\.textContent\s*=/);
  // 文字数表示は持ち主付きで出し、自分の文言だけを消す。
  assert.match(uiFile("chat/composer.js"), /status\.set\(`\$\{input\.value\.length\}文字`, "count"\);\s*\n\s*else status\.clear\("count"\);/);
});

test("閉じている通知一覧を作り直さず保存もまとめる", () => {
  // エージェント実行中は進捗のたびに通知が更新される。閉じている一覧の DOM を毎回作り直さない。
  assert.match(notice, /function render\(\) \{[\s\S]{0,80}if \(panel\.hidden\) return;/);
  // 開くときは必ず組み立て直す（飛ばした更新をここで取り戻す）。
  assert.match(notice, /panel\.hidden = !open;\s*\n\s*if \(open\) \{ unread = 0; render\(\);/);
  assert.match(notice, /const persistScheduler = window\.PicoFrameScheduler\.create\(persistNow\);/);
  // 終了時はまとめ待ちを書き切る（通知の取りこぼしを残さない）。
  assert.match(notice, /window\.addEventListener\("beforeunload", \(\) => persistScheduler\.flush\(\)\);/);
});
