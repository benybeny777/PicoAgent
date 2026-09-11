"use strict";

const query = new URLSearchParams(location.search);
const characterId = query.get("characterId") || "";
const status = document.getElementById("status");
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
let controller = null;
let faceGeneration = 0;
let motionGeneration = 0;
let blinkTimer = 0;
let talkingTimer = 0;
let independentEyes = false;

function stopTalkingPreview() {
  window.clearInterval(talkingTimer);
  talkingTimer = 0;
}

function setMotionPreview(name) {
  stopTalkingPreview();
  controller?.setMotion(name);
  if (name !== "talking") {
    controller?.setMouth(0, 0);
    return;
  }
  const vowels = [[0.72, 0], [0.28, 0.9], [0.58, -0.8], [0, 0], [0.88, -0.55], [0, 0]];
  let index = 0;
  talkingTimer = window.setInterval(() => {
    controller?.setMouth(...vowels[index % vowels.length]);
    index += 1;
  }, 145);
}

async function blink() {
  if (!controller) return;
  controller.setBlink(0, 180);
  await wait(180);
  controller.setEyeOpen(0, 0);
  await wait(220);
  controller.setBlink(1, 250);
}

async function wink(side) {
  if (!controller || !independentEyes) return;
  controller.setEyeOpen(side === "L" ? 0 : 1, side === "R" ? 0 : 1);
  await wait(420);
  controller.setEyeOpen(1, 1);
}

async function faceDemo() {
  const generation = ++faceGeneration;
  stopTalkingPreview();
  const vowels = [[0, 0], [1, 0], [0, 0], [0.38, 0.95], [0, 0], [0.55, -0.95], [0, 0], [0.62, 0.70], [0, 0], [0.90, -0.75], [0, 0]];
  await blink();
  for (const [open, form] of vowels) {
    if (generation !== faceGeneration) return;
    controller?.setMouth(open, form);
    await wait(open ? 620 : 190);
  }
}

async function motionDemo() {
  const generation = ++motionGeneration;
  const sequence = [["thinking", "考え中", 2800], ["talking", "会話", 3600], ["success", "よろこび", 2800], ["concern", "困り", 2800], ["listen", "聞く", 2400], ["idle", "待機", 2200]];
  for (const [name, label, duration] of sequence) {
    if (generation !== motionGeneration) return;
    setMotionPreview(name);
    status.textContent = `動作の自動確認 — ${label}`;
    await wait(duration);
  }
  stopTalkingPreview();
  controller?.setMouth(0, 0);
}

function updateGaze() {
  const x = Number(document.getElementById("gazeX").value);
  const y = Number(document.getElementById("gazeY").value);
  document.querySelector("#gazeX + output").value = x.toFixed(2);
  document.querySelector("#gazeY + output").value = y.toFixed(2);
  controller?.setGaze(x, y);
}

async function loadPreview() {
  if (!characterId) throw new Error("キャラクターIDがありません");
  const data = await window.picoagent.settings.characterMakerPreviewData(characterId);
  document.title = `${data.displayName} - PicoAgent キャラ確認`;
  document.getElementById("title").textContent = `${data.displayName}の動作確認`;
  document.getElementById("source").src = window.picoagent.assetUrl(data.sourcePath);
  const rigUrl = window.picoagent.assetUrl(data.rigPath);
  const rootUrl = window.picoagent.assetUrl(data.rigRootPath).replace(/\/$/, "");
  const response = await fetch(rigUrl);
  if (!response.ok) throw new Error(`rig.jsonを読めません: HTTP ${response.status}`);
  const rig = await response.json();
  const rigWidth = Array.isArray(rig.canvas) ? rig.canvas[0] : rig.canvas.width;
  const rigHeight = Array.isArray(rig.canvas) ? rig.canvas[1] : rig.canvas.height;
  independentEyes = ["anime25d-v1", "lvs-anime25d-v1"].includes(rig.profile || rig.mode);
  const canvas = document.createElement("canvas");
  canvas.setAttribute("aria-label", "PicoAgent 2.5D描画");
  document.getElementById("runtimeStage").append(canvas);
  controller = await window.PicoMeshRig.create({
    canvas,
    rigUrl,
    rootUrl,
    assetVersion: data.assetVersion,
    width: 540,
    height: 760,
  });
  controller.setActive(true);
  controller.setArmClose(0);
  controller.setGaze(0, 0);
  document.querySelectorAll("[data-wink]").forEach(button => { button.disabled = !independentEyes; });
  document.getElementById("arm").disabled = !independentEyes;
  status.textContent = `${rigWidth}×${rigHeight}座標を本番と同じ描画経路で確認中です。`;
  blinkTimer = window.setInterval(() => {
    if (document.visibilityState === "visible") blink();
  }, 4200);
}

document.getElementById("blink").addEventListener("click", blink);
document.querySelectorAll("[data-wink]").forEach(button => button.addEventListener("click", () => wink(button.dataset.wink)));
document.getElementById("faceDemo").addEventListener("click", faceDemo);
document.getElementById("motionDemo").addEventListener("click", motionDemo);
document.querySelectorAll("[data-mouth]").forEach(button => button.addEventListener("click", () => {
  faceGeneration += 1;
  stopTalkingPreview();
  controller?.setMouth(...button.dataset.mouth.split(",").map(Number));
}));
document.querySelectorAll("[data-expression]").forEach(button => button.addEventListener("click", () => {
  controller?.setExpression(button.dataset.expression);
  document.querySelectorAll("[data-expression]").forEach(item => item.classList.toggle("active", item === button));
  status.textContent = `${button.textContent}を確認中です。`;
}));
document.querySelectorAll("[data-motion]").forEach(button => button.addEventListener("click", () => {
  motionGeneration += 1;
  setMotionPreview(button.dataset.motion);
  document.querySelectorAll("[data-motion]").forEach(item => item.classList.toggle("active", item === button));
  status.textContent = `${button.textContent}動作を確認中です。`;
}));
document.getElementById("gazeX").addEventListener("input", updateGaze);
document.getElementById("gazeY").addEventListener("input", updateGaze);
document.getElementById("gazeReset").addEventListener("click", () => {
  document.getElementById("gazeX").value = "0";
  document.getElementById("gazeY").value = "0";
  updateGaze();
});
document.getElementById("arm").addEventListener("input", event => {
  event.target.nextElementSibling.value = Number(event.target.value).toFixed(2);
  controller?.setArmClose(Number(event.target.value));
});
document.querySelectorAll("[data-bg]").forEach(button => button.addEventListener("click", () => {
  document.querySelectorAll(".stage").forEach(stage => { stage.className = `stage ${button.dataset.bg}`; });
  document.querySelectorAll("[data-bg]").forEach(item => item.classList.toggle("active", item === button));
}));
window.addEventListener("beforeunload", () => {
  window.clearInterval(blinkTimer);
  stopTalkingPreview();
  controller?.destroy();
});

loadPreview().catch(error => {
  status.textContent = `表示できませんでした: ${error?.message || error}`;
  console.error(error);
});
