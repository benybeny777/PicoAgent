import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

await import("../../ui/frame-scheduler.js");

const settingsHtml = await readFile(new URL("../../ui/settings.html", import.meta.url), "utf8");
const indexHtml = await readFile(new URL("../../ui/index.html", import.meta.url), "utf8");
const settings = await readFile(new URL("../../ui/settings.js", import.meta.url), "utf8");
const characterHost = await readFile(new URL("../../ui/sources/character.js", import.meta.url), "utf8");
const imageRenderer = await readFile(new URL("../../ui/character/image.js", import.meta.url), "utf8");

function fixture() {
  let nextId = 1;
  const queued = new Map();
  const cancelled = [];
  const timing = {
    requestFrame: callback => {
      const id = nextId++;
      queued.set(id, callback);
      return id;
    },
    cancelFrame: id => {
      queued.delete(id);
      cancelled.push(id);
    },
  };
  return { timing, queued, cancelled };
}

test("連続イベントを1フレーム1回へまとめる", () => {
  const { timing, queued } = fixture();
  let updates = 0;
  const scheduler = globalThis.PicoFrameScheduler.create(() => { updates += 1; }, timing);

  assert.equal(scheduler.schedule(), true);
  assert.equal(scheduler.schedule(), false);
  assert.equal(queued.size, 1);
  queued.values().next().value();
  assert.equal(updates, 1);
  assert.equal(scheduler.pending(), false);
});

test("即時更新は予約済みフレームを取り消して1回だけ実行する", () => {
  const { timing, queued, cancelled } = fixture();
  let updates = 0;
  const scheduler = globalThis.PicoFrameScheduler.create(() => { updates += 1; }, timing);

  scheduler.schedule();
  scheduler.flush();
  assert.equal(updates, 1);
  assert.equal(queued.size, 0);
  assert.equal(cancelled.length, 1);
});

test("最新イベントだけを1フレーム1回へ渡す", () => {
  const { timing, queued } = fixture();
  const applied = [];
  const scheduler = globalThis.PicoFrameScheduler.createLatest(value => applied.push(value), timing);

  assert.equal(scheduler.schedule({ x: 10, y: 20 }), true);
  assert.equal(scheduler.schedule({ x: 30, y: 40 }), false);
  assert.equal(queued.size, 1);
  queued.values().next().value();
  assert.deepEqual(applied, [{ x: 30, y: 40 }]);
  assert.equal(scheduler.pending(), false);
});

test("処理不要になったループは停止し状態変化から再開する", () => {
  const { timing, queued, cancelled } = fixture();
  let active = true;
  let ticks = 0;
  const loop = globalThis.PicoFrameScheduler.createLoop(() => {
    ticks += 1;
    return active;
  }, timing);

  assert.equal(loop.start(), true);
  assert.equal(loop.start(), false);
  let [frameId, callback] = queued.entries().next().value;
  queued.delete(frameId); // ブラウザは実行するrAFを予約一覧から先に外す
  callback(16);
  assert.equal(ticks, 1);
  assert.equal(queued.size, 1);

  active = false;
  [frameId, callback] = queued.entries().next().value;
  queued.delete(frameId);
  callback(32);
  assert.equal(ticks, 2);
  assert.equal(queued.size, 0);
  assert.equal(loop.pending(), false);

  active = true;
  assert.equal(loop.start(), true);
  assert.equal(loop.stop(), true);
  assert.equal(queued.size, 0);
  assert.equal(cancelled.length, 1);
});

test("設定画面がフレーム合流と即時更新を使い分ける", () => {
  assert.ok(
    settingsHtml.indexOf('<script src="frame-scheduler.js')
      < settingsHtml.indexOf('<script src="settings.js'),
  );
  assert.match(settings, /PicoFrameScheduler\.create\(updateDirtyStateNow\)/);
  assert.match(settings, /dirtyStateScheduler\.schedule\(\)/);
  assert.match(settings, /dirtyStateScheduler\.flush\(\)/);
});

test("メイン画面が停止可能ループと最新視線イベントを使う", () => {
  assert.ok(
    indexHtml.indexOf('<script src="frame-scheduler.js')
      < indexHtml.indexOf('<script type="module" src="app/main.js'),
  );
  assert.match(imageRenderer, /PicoFrameScheduler\.createLoop\(now => this\.step\(now\)\)/);
  assert.match(characterHost, /const gaze = window\.PicoFrameScheduler\.createLatest\(/);
  assert.match(characterHost, /gaze\.schedule\(\{ clientX: event\.clientX, clientY: event\.clientY \}\)/);
  assert.doesNotMatch(imageRenderer, /=\s*requestAnimationFrame\(/);
});

test("1枚絵を表示できたら描画ループを休ませる", () => {
  // メッシュリグを止めたキャラは preview.png の1枚だけを表示する。送り先の無い1枚絵で
  // true を返し続けると、変化の無い画面のために毎フレーム予約が続く。
  const start = imageRenderer.indexOf("  step(now) {");
  const body = imageRenderer.slice(start, imageRenderer.indexOf("\n  }\n", start));
  assert.match(body, /if \(this\.disposed \|\| !this\.active \|\| !this\.frameBudget \|\| document\.hidden \|\| !this\.animations\) return false;/);
  assert.match(body, /paths\.length > 1/);
});
