// lipsync.js - 読み上げ音声の振幅から口の開きを求める。結果はキャラのホストへ渡すだけで、描画は持たない。
//
// 振幅の瞬間値をそのまま口へ入れると、開閉が毎フレーム往復して口だけが痙攣して見える。
// 無音域を除いて正規化し、開く側は素早く・閉じる側は少し遅い包絡線で滑らかにする。

import { errorDetail, logApp } from "./util.js";

function mouthFormAt(currentTime, timeline) {
  if (!Array.isArray(timeline) || timeline.length === 0) return 0;
  const time = Number(currentTime) || 0;
  const segment = timeline.find(item => time >= Number(item.start) && time < Number(item.end));
  return Math.max(-1, Math.min(1, Number(segment?.form) || 0));
}

export function createLipSync({ onMouth }) {
  let audioContext = null;
  let source = null;
  let analyser = null;
  let frame = 0;
  let timer = 0;
  let level = 0;
  let form = 0;
  let attached = null;

  function detach() {
    if (timer) { clearInterval(timer); timer = 0; }
    if (frame) { cancelAnimationFrame(frame); frame = 0; }
    // 解放処理なので、切断済みで例外になっても残りの参照を捨てる。
    try { source?.disconnect(); analyser?.disconnect(); }
    catch (error) { logApp("warn", "リップシンクの音声解析を切断できませんでした", errorDetail(error)); }
    source = null;
    analyser = null;
    attached = null;
    level = 0;
    form = 0;
    onMouth(0, 0);
  }

  /** 再生中の音声へ接続する。`isActive()` が偽になった時点で自動的に外す。 */
  function attach(audio, isActive) {
    // 同じ音声へ2回つながない（音声要素ごとの解析の口は1つしか作れない）。
    if (attached === audio && (analyser || timer)) return;
    detach();
    attached = audio;
    let useTimer = true;
    try {
      audioContext ||= new (window.AudioContext || window.webkitAudioContext)();
      if (audioContext.state === "suspended") {
        audioContext.resume().catch(error => logApp("warn", "リップシンク用の音声解析を再開できませんでした", errorDetail(error)));
      }
      if (audioContext.state === "running") {
        source = audioContext.createMediaElementSource(audio);
        analyser = audioContext.createAnalyser();
        analyser.fftSize = 512;
        source.connect(analyser);
        analyser.connect(audioContext.destination);
        useTimer = false;
      }
    } catch (error) {
      logApp("warn", "リップシンクの音声解析を初期化できませんでした（タイマー式で継続）", errorDetail(error));
    }
    if (useTimer) {
      let open = false;
      timer = setInterval(() => {
        if (!isActive()) { onMouth(0, 0); return; }
        open = !open;
        // 最大開口と完全閉口を往復させない（デフォルメ口では「口だけ跳ねる」見え方になる）。
        onMouth(open ? 0.58 : 0.08, mouthFormAt(audio.currentTime, audio.picoLipSync));
      }, 150);
      return;
    }
    const data = new Uint8Array(analyser.fftSize);
    let peak = 0.04;
    const loop = () => {
      if (!analyser) return;
      if (!isActive()) { detach(); return; }
      analyser.getByteTimeDomainData(data);
      let sum = 0;
      for (let index = 0; index < data.length; index += 1) {
        const value = (data[index] - 128) / 128;
        sum += value * value;
      }
      // 解析点は音量調整（audio.volume）より後ろ。要素の音量で割り戻し、聞こえる大きさに依存させない。
      const gain = Math.max(0.05, audio.volume || 1);
      const rms = Math.sqrt(sum / data.length) / gain;
      peak = Math.max(peak * 0.995, rms, 0.02);
      const floor = peak * 0.22;
      const span = Math.max(peak * 0.72, 0.001);
      const target = Math.max(0, Math.min(1, (rms - floor) / span));
      const desired = Math.min(0.78, target * 0.78);
      level += (desired - level) * (desired > level ? 0.34 : 0.16);
      if (level < 0.015) level = 0;
      const formTarget = mouthFormAt(audio.currentTime, audio.picoLipSync);
      form += (formTarget - form) * Math.min(1, 0.42 + target * 0.25);
      onMouth(level, form);
      frame = requestAnimationFrame(loop);
    };
    frame = requestAnimationFrame(loop);
  }

  function dispose() {
    detach();
    const closing = audioContext;
    audioContext = null;
    closing?.close().catch(error => logApp("warn", "終了時に音声解析を解放できませんでした", errorDetail(error)));
  }

  return { attach, detach, dispose };
}
