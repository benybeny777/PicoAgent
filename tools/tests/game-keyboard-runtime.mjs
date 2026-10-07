import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";

function keyboard() {
  let handle;
  let closed = 0;
  const source = uiFile("app/game.js");
  const start = source.indexOf('  document.addEventListener("keydown"');
  const context = vm.createContext({
    document: { addEventListener(_type, callback) { handle = callback; } },
    overlay: { hidden: false }, close() { closed += 1; },
    isImeComposing: event => event.isComposing || event.keyCode === 229,
  });
  vm.runInContext(source.slice(start, source.indexOf("  syncSoundButton();", start)), context);
  return { handle, get closed() { return closed; } };
}

test("ゲーム中の日本語変換をEscで取り消してもゲームは閉じない", () => {
  for (const composition of [{ isComposing: true }, { keyCode: 229 }]) {
    const game = keyboard();
    game.handle({ key: "Escape", ...composition, preventDefault() { assert.fail("IMEのキーを奪わない"); } });
    assert.equal(game.closed, 0);
  }
});

test("ゲームを閉じるEscは消費し背後のチャットまで閉じない", () => {
  const game = keyboard();
  const event = { key: "Escape", defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } };
  game.handle(event);
  assert.equal(game.closed, 1);
  assert.equal(event.defaultPrevented, true);
});
