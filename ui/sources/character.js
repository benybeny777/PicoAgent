// character.js - キャラのソース。描画は ui/character/ の描画器が行い、ここは触れ合い（クリック・なでなで・
// 視線）と、通常ドラッグのウィンドウ移動を受け持つ。位置と大きさの調整は app/edit.js が行う。

import * as bridge from "../app/bridge.js";
import { el, errorDetail, logApp } from "../app/util.js";

const DEFAULT_REACTIONS = { head: "ふふっ", cheek: "ちょっと照れるね", hand: "どうしたの？" };

export function createCharacterSource(context) {
  const view = el("div", { className: "character-view" });
  const heart = el("button", {
    className: "relationship-button icon-button", type: "button", textContent: "♡",
    title: "いっしょの記録を見る",
  }, [], { "aria-label": "いっしょの記録を見る", "aria-controls": "relationshipCard", "aria-expanded": "false" });
  const online = el("span", { className: "online-dot" }, [], { "aria-hidden": "true" });
  const element = el("section", { className: "character", tabIndex: 0 }, [view, heart, online], {
    role: "button",
    "aria-label": "キャラクターに触れる。ドラッグで画面を移動、右クリックでメニュー",
  });
  let strokes = [];
  let petCooldownUntil = 0;

  function boop() {
    element.classList.remove("boop");
    void element.offsetWidth;
    element.classList.add("boop");
  }
  element.addEventListener("animationend", event => {
    if (event.animationName === "character-boop") element.classList.remove("boop");
  });

  // 通常ドラッグは OS 標準のウィンドウ移動。調整中（app/edit.js）はそちらが先に受け取る。
  element.addEventListener("pointerdown", event => {
    if (event.button !== 0 || event.defaultPrevented) return;
    if (event.target instanceof Element && event.target.closest("button")) return;
    context.markActivity?.();
    context.host.withDragging(() => bridge.startWindowDrag()).catch(error => {
      logApp("warn", "キャラクターからウィンドウを移動できませんでした", errorDetail(error));
      context.toast?.("画面を移動できませんでした。上部のバーからも移動できます", "error");
    });
  });

  element.addEventListener("click", event => {
    if (event.target instanceof Element && event.target.closest("button")) return;
    if (context.edit?.active) return;
    boop();
    const rect = element.getBoundingClientRect();
    const y = (event.clientY - rect.top) / Math.max(1, rect.height);
    const x = (event.clientX - rect.left) / Math.max(1, rect.width);
    const zone = y < 0.34 ? "head" : (y < 0.67 && (x < 0.3 || x > 0.7)) ? "cheek" : y > 0.7 ? "hand" : "head";
    const reactions = context.host.pack?.manifest?.interactions?.reactions?.[zone];
    const reaction = Array.isArray(reactions) && reactions.length
      ? reactions[Math.floor(Math.random() * reactions.length)]
      : DEFAULT_REACTIONS[zone];
    if (reaction) context.status?.flash(reaction, 1600);
    context.motion?.react(zone === "cheek" ? "shy" : "pleased");
    context.relationship?.recordTouch(zone);
    // 読み上げ中にキャラをクリックしたら止める。
    if (context.voice?.speaking) {
      context.voice.stop();
      context.status?.clear();
    }
  });

  element.addEventListener("keydown", event => {
    if (event.key !== "Enter" && event.key !== " ") return;
    // 枠の中の ♡ ボタンを選んで押したキーまで「触れる」にしない。枠自体に当たったときだけ扱う。
    if (event.target !== element) return;
    event.preventDefault();
    element.click();
  });

  // なでなで: キャラの上でマウスを素早く左右に往復させると照れる。
  element.addEventListener("pointermove", event => {
    if (context.host.reducedMotion || (event.pointerType === "mouse" && event.buttons)) return;
    const now = performance.now();
    strokes.push({ x: event.clientX, t: now });
    strokes = strokes.filter(stroke => now - stroke.t < 1200);
    if (now < petCooldownUntil || strokes.length < 6) return;
    let flips = 0;
    let travel = 0;
    let previous = 0;
    for (let index = 1; index < strokes.length; index += 1) {
      const dx = strokes[index].x - strokes[index - 1].x;
      travel += Math.abs(dx);
      if (dx && previous && Math.sign(dx) !== Math.sign(previous)) flips += 1;
      if (dx) previous = dx;
    }
    if (flips < 4 || travel < 150) return;
    petCooldownUntil = now + 6000;
    strokes = [];
    boop();
    for (let index = 0; index < 3; index += 1) {
      const heartMark = el("span", { className: "pet-heart", textContent: "♥" });
      heartMark.style.left = `${30 + Math.random() * 40}%`;
      heartMark.style.animationDelay = `${index * 130}ms`;
      element.append(heartMark);
      heartMark.addEventListener("animationend", () => heartMark.remove());
    }
    context.status?.flash("えへへ…", 2000);
    context.markActivity?.();
    context.motion?.react("shy");
    context.relationship?.record("petting");
  });

  heart.addEventListener("click", event => {
    event.stopPropagation();
    context.relationship?.toggle(heart);
  });

  // 視線追従（設定でONのときだけ）。顔の付近のカーソルだけへ控えめに向ける。
  const gaze = window.PicoFrameScheduler.createLatest(({ clientX, clientY }) => {
    if (element.hidden) return;
    const rect = element.getBoundingClientRect();
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height * 0.38;
    const nx = (clientX - centerX) / Math.max(160, rect.width * 0.9);
    const ny = (clientY - centerY) / Math.max(180, rect.height * 0.8);
    const nearFace = clientY >= rect.top + rect.height * 0.02 && clientY <= rect.top + rect.height * 0.58 && Math.abs(nx) <= 1.15;
    const deadZone = Math.abs(nx) < 0.28 && Math.abs(ny) < 0.28;
    const x = nearFace && !deadZone ? Math.max(-1, Math.min(1, nx)) : 0;
    const y = nearFace && !deadZone ? Math.max(-1, Math.min(1, -ny)) : 0;
    context.host.setGaze(x, y);
    clearTimeout(resetTimer);
    resetTimer = setTimeout(() => context.host.setGaze(0, 0), 700);
  });
  let resetTimer = 0;
  document.addEventListener("mousemove", event => gaze.schedule({ clientX: event.clientX, clientY: event.clientY }));

  return {
    element,
    view,
    heart,
    setVisible(visible) { context.host.setActive(visible); },
    setOnline(up) { online.dataset.up = up === null ? "unknown" : String(up === true); },
    update({ rect, pixelRatio, layoutId }) {
      context.host.setLayout(layoutId, context.stage.currentFraming());
      context.host.resize(rect.width, rect.height, pixelRatio);
    },
  };
}
