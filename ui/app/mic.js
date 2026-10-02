// mic.js - 音声入力（マイク → WAV → ローカル Whisper 文字起こし）と連続会話の持ち主。

import * as bridge from "./bridge.js";
import { toast } from "./toast.js";
import { blobToDataUrl, errorDetail, logApp, preview } from "./util.js";

// 録音の上限（16bit モノラル WAV 換算）。バックエンドの上限30MBに当たってから弾くと録音が無駄になるため、
// 録音側で先に打ち切ってそこまでを確定させる（16kHz なら約14分）。
const MAX_RECORD_BYTES = 27_000_000;
const CONTINUOUS_MIN_MS = 450;
const CONTINUOUS_MAX_MS = 5000;

/** Float32 の PCM を 16bit モノラル WAV に変換する（whisper-cli は wav 入力のため）。 */
function encodeWav(chunks, sampleRate) {
  const length = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const buffer = new ArrayBuffer(44 + length * 2);
  const view = new DataView(buffer);
  const text = (offset, value) => { for (let index = 0; index < value.length; index += 1) view.setUint8(offset + index, value.charCodeAt(index)); };
  text(0, "RIFF"); view.setUint32(4, 36 + length * 2, true); text(8, "WAVE");
  text(12, "fmt "); view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true); view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  text(36, "data"); view.setUint32(40, length * 2, true);
  let offset = 44;
  for (const chunk of chunks) {
    for (let index = 0; index < chunk.length; index += 1) {
      const value = Math.max(-1, Math.min(1, chunk[index]));
      view.setInt16(offset, value < 0 ? value * 0x8000 : value * 0x7fff, true);
      offset += 2;
    }
  }
  return buffer;
}

export function createMic({ chat, voice, motion, isBusy, canListen, onActivity, onListening }) {
  const { composer, status, buttons } = chat;
  const micButton = buttons.micButton;
  const loopButton = buttons.voiceLoopButton;
  let stream = null;
  let starting = false;
  let startRequest = 0;
  let context = null;
  let node = null;
  let chunks = [];
  let sampleRate = 16000;
  let samples = 0;
  let autoStopping = false;
  let speechDetected = false;
  let lastVoiceAt = 0;
  let startedAt = 0;
  let noiseFloor = 0;
  let noiseFrames = 0;
  // 連続会話はマイクの意図しない再開を避けるため、起動ごとにOFFから始める。
  let continuous = false;
  let continuousTimer = null;
  let submitTimer = null;
  // 逐次文字起こし（録音中に数秒ごとの区間を裏で起こして入力欄へ出し、停止時に全体で確定する）。
  let session = 0;
  let interimTimer = null;
  let interimBusy = false;
  let interimIndex = 0;
  let interimTexts = [];
  let interimBase = "";

  function setListening(on) {
    micButton.classList.toggle("recording", on);
    onListening?.(on);
    if (on) motion.react("listening", 60 * 60 * 1000);
    else if (!isBusy() && !voice.speaking) motion.rest();
  }

  async function transcribeInterim(current) {
    if (interimBusy || current !== session || !stream) return;
    const from = interimIndex;
    const to = chunks.length;
    const seconds = chunks.slice(from, to).reduce((sum, chunk) => sum + chunk.length, 0) / sampleRate;
    if (seconds < 2) return;
    interimBusy = true;
    interimIndex = to;
    try {
      const data = await blobToDataUrl(new Blob([encodeWav(chunks.slice(from, to), sampleRate)], { type: "audio/wav" }));
      if (current !== session) return;
      const result = await bridge.transcribeAudio(data);
      if (current !== session) return;
      const text = String(result?.text ?? "").trim();
      if (text) {
        interimTexts.push(text);
        const joined = interimTexts.join(" ");
        composer.setValue(interimBase ? `${interimBase} ${joined}` : joined);
        status.set("🎙 録音中…（認識中の文はあとで確定します）", "mic");
      }
    } catch (error) {
      // Whisper 未設定などで失敗したら、この録音では逐次変換を止める（停止時の一括変換に任せる）。
      if (current !== session) return;
      logApp("warn", "途中の文字起こしに失敗しました", errorDetail(error));
      if (interimTimer) { clearInterval(interimTimer); interimTimer = null; }
    } finally {
      interimBusy = false;
    }
  }

  function stopCapture() {
    startRequest += 1;
    starting = false;
    micButton.disabled = false;
    session += 1;
    if (interimTimer) { clearInterval(interimTimer); interimTimer = null; }
    node?.disconnect();
    node = null;
    stream?.getTracks().forEach(track => track.stop());
    stream = null;
    const closing = context;
    context = null;
    closing?.close().catch(error => logApp("warn", "マイク用の音声処理を解放できませんでした", errorDetail(error)));
    setListening(false);
  }

  function cancelPendingSubmit(message = "音声の送信を取り消しました") {
    if (!submitTimer) return false;
    clearTimeout(submitTimer);
    submitTimer = null;
    status.clear("mic");
    toast(message, "warn");
    composer.focus();
    return true;
  }

  function scheduleSubmit() {
    if (!continuous || !composer.value.trim() || isBusy()) return;
    if (submitTimer) clearTimeout(submitTimer);
    status.set("1.5秒後に送信（Escで取消）", "mic");
    submitTimer = setTimeout(() => {
      submitTimer = null;
      status.clear("mic");
      if (continuous && composer.value.trim() && !isBusy()) composer.form.requestSubmit();
    }, 1500);
  }

  async function finish() {
    if (!stream) return;
    const recorded = chunks;
    chunks = [];
    samples = 0;
    const rate = sampleRate;
    const base = interimBase;
    const hadInterim = interimTexts.length > 0;
    stopCapture();
    const current = session;
    const request = startRequest;
    const isCurrent = () => current === session && request === startRequest;
    let hasTranscript = hadInterim;
    if (recorded.reduce((sum, chunk) => sum + chunk.length, 0) / rate < 0.4) {
      status.clear("mic");
      toast("録音が短すぎました", "warn");
      return;
    }
    status.set(hadInterim ? "文字起こしを確定中…" : "文字起こし中…", "mic");
    status.setBusy(true);
    try {
      const data = await blobToDataUrl(new Blob([encodeWav(recorded, rate)], { type: "audio/wav" }));
      if (!isCurrent()) return;
      const result = await bridge.transcribeAudio(data);
      if (!isCurrent()) return;
      const text = String(result?.text ?? "").trim();
      if (text) {
        hasTranscript = true;
        composer.setValue(base ? `${base} ${text}` : text);
        composer.focus();
        toast("文字起こししました", "success");
      }
    } catch (error) {
      if (!isCurrent()) return;
      logApp("error", "音声入力に失敗しました", errorDetail(error));
      if (!hadInterim) toast(preview(error?.message || error || "文字起こしに失敗しました", 60), "error");
      else toast("確定に失敗したため途中結果を残しました", "warn");
    } finally {
      if (isCurrent()) {
        status.setBusy(isBusy());
        status.clear("mic");
        if (hasTranscript) {
          composer.resetNavigation();
          scheduleSubmit();
        }
      }
    }
  }

  async function start() {
    if (stream || starting) return;
    cancelPendingSubmit("録音を再開するため音声送信を取り消しました");
    const request = ++startRequest;
    starting = true;
    micButton.disabled = true;
    voice.stop(); // 読み上げ中でも話し始められるよう、マイク操作を割り込みとして扱う。
    try {
      const media = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
      if (request !== startRequest || document.hidden) {
        media.getTracks().forEach(track => track.stop());
        return;
      }
      stream = media;
    } catch (error) {
      if (request !== startRequest) return;
      logApp("warn", "マイクを利用できません", errorDetail(error));
      toast("マイクを利用できません（権限を確認してね）", "error");
      stream = null;
      return;
    } finally {
      if (request === startRequest && !stream) {
        starting = false;
        micButton.disabled = false;
      }
    }
    try {
      context = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 16000 });
      sampleRate = context.sampleRate; // 16kHz 非対応環境では実サンプルレートで記録する
      const source = context.createMediaStreamSource(stream);
      // ScriptProcessor は非推奨だが WebView 互換性が高い（AudioWorklet はファイル分離が必要）。
      node = context.createScriptProcessor(4096, 1, 1);
      chunks = [];
      samples = 0;
      autoStopping = false;
      speechDetected = false;
      startedAt = performance.now();
      lastVoiceAt = startedAt;
      noiseFloor = 0;
      noiseFrames = 0;
      const maxSamples = Math.floor((MAX_RECORD_BYTES - 44) / 2);
      node.onaudioprocess = event => {
        if (autoStopping) return;
        const raw = event.inputBuffer.getChannelData(0);
        const room = maxSamples - samples;
        const frame = new Float32Array(raw.length <= room ? raw : raw.subarray(0, room));
        if (frame.length > 0) {
          chunks.push(frame);
          samples += frame.length;
          let energy = 0;
          for (let index = 0; index < frame.length; index += 1) energy += frame[index] * frame[index];
          const rms = Math.sqrt(energy / frame.length);
          const now = performance.now();
          if (now - startedAt < 700) {
            noiseFrames += 1;
            noiseFloor += (rms - noiseFloor) / noiseFrames;
            return;
          }
          if (rms > Math.max(0.012, noiseFloor * 2.8)) {
            speechDetected = true;
            lastVoiceAt = now;
          } else if (continuous && speechDetected && now - lastVoiceAt > 1300 && samples / sampleRate > 1.2) {
            autoStopping = true;
            finish();
            return;
          } else if (continuous && !speechDetected && now - startedAt > 15000) {
            autoStopping = true;
            setContinuous(false);
            stopCapture();
            toast("無音が続いたため連続会話を停止しました", "warn");
            return;
          }
        }
        if (samples >= maxSamples) {
          autoStopping = true;
          toast("録音が上限に達したので自動で確定しました", "warn");
          finish();
        }
      };
      source.connect(node);
      node.connect(context.destination);
      setListening(true);
      status.set("🎙 録音中…（マイクをもう一度クリックで停止）", "mic");
      const current = ++session;
      interimBase = composer.value.trim();
      interimTexts = [];
      interimIndex = 0;
      interimTimer = setInterval(() => transcribeInterim(current), 3000);
      onActivity?.();
    } catch (error) {
      logApp("error", "録音を開始できません", errorDetail(error));
      toast("録音を開始できませんでした", "error");
      stopCapture();
    } finally {
      if (request === startRequest) {
        starting = false;
        micButton.disabled = false;
      }
    }
  }

  function setContinuous(on) {
    continuous = Boolean(on);
    loopButton.setAttribute("aria-pressed", String(continuous));
    loopButton.title = continuous ? "連続会話を止める" : "返答後も自動で聞き続ける";
    if (!continuous && continuousTimer) { clearTimeout(continuousTimer); continuousTimer = null; }
  }

  /** 待てないときは間隔を伸ばして待ち直す（長い応答のあいだ短い間隔で回り続けない）。 */
  function scheduleContinuous(delay = CONTINUOUS_MIN_MS) {
    if (continuousTimer) { clearTimeout(continuousTimer); continuousTimer = null; }
    if (!continuous) return;
    continuousTimer = setTimeout(async () => {
      continuousTimer = null;
      if (!continuous) return;
      if (isBusy() || voice.speaking || stream || !canListen()) {
        scheduleContinuous(Math.min(delay * 2, CONTINUOUS_MAX_MS));
        return;
      }
      composer.open();
      await start();
    }, delay);
  }

  loopButton.addEventListener("click", event => {
    event.stopPropagation();
    setContinuous(!continuous);
    if (!continuous) {
      if (stream) stopCapture();
      toast("連続会話を終了しました", "success");
      return;
    }
    toast("この起動中だけ、話し終わると自動送信します", "success");
    scheduleContinuous();
  });

  // 長押し中だけ録音 / クリックで開始・停止。
  let holdTimer = null;
  let holding = false;
  let suppressClick = false;
  micButton.addEventListener("pointerdown", event => {
    if (event.button !== 0 || stream || starting) return;
    holdTimer = setTimeout(async () => {
      holdTimer = null;
      holding = true;
      suppressClick = true;
      await start();
      if (stream) status.set("🎙 押している間だけ録音中…", "mic");
    }, 320);
  });
  micButton.addEventListener("pointerup", async () => {
    if (holdTimer) { clearTimeout(holdTimer); holdTimer = null; }
    if (!holding) return;
    holding = false;
    if (stream) await finish();
    else if (starting) stopCapture(); // 許可待ちの長押しも離した時点で取り消す。
  });
  micButton.addEventListener("pointercancel", () => {
    if (holdTimer) { clearTimeout(holdTimer); holdTimer = null; }
    if (holding) { holding = false; stopCapture(); }
  });
  micButton.addEventListener("click", async event => {
    event.stopPropagation();
    if (suppressClick) { suppressClick = false; return; }
    if (stream) { await finish(); return; }
    await start();
  });

  document.addEventListener("visibilitychange", () => {
    if (!document.hidden || !continuous) return;
    setContinuous(false);
    if (stream) stopCapture();
    cancelPendingSubmit("画面が非表示になったため音声送信を取り消しました");
  });
  window.addEventListener("beforeunload", () => {
    if (continuousTimer) clearTimeout(continuousTimer);
    if (submitTimer) clearTimeout(submitTimer);
    if (holdTimer) clearTimeout(holdTimer);
    if (stream) stopCapture();
  });

  return {
    get active() { return Boolean(stream); },
    scheduleContinuous,
    cancelPendingSubmit,
  };
}
