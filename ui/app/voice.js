// voice.js - 読み上げ（音声合成の再生・先行読み上げ・停止）の持ち主。
//
// ゲームなど通常会話以外の画面も、音量・停止・字幕・口パクを含む同じ経路を使う
// （`pet_speak` を各画面が直接扱うと、音声の重複や Object URL の解放漏れが起きるため公開口を1つに絞る）。
// ここへ来る本文は、どれも Rust 側の公開検査（`agent::guard_public_answer`）を通ったものだけ。

import * as bridge from "./bridge.js";
import { errorDetail, errorMessage, logApp } from "./util.js";

/** 読み上げ用に本文を整える（参考リンク・URL・記号だけの行を落とす）。 */
export function sanitizeSpeechText(text) {
  let out = String(text ?? "").replace(/\r\n/g, "\n");
  out = out.split(/\n\s*参考[:：]/)[0];
  out = out.replace(/!?\[([^\]]*)\]\((?:https?:\/\/|mailto:)[^)]+\)/g, "$1");
  out = out.replace(/https?:\/\/[^\s、。）」』】]+/g, "");
  return out
    .split("\n")
    .map(line => line.trim())
    .filter(line => line && line.replace(/^[-*・\s:：]+$/, ""))
    .filter(line => !/^フィード[:：]/.test(line))
    .filter(line => !/^-\s+\d{4}-\d{2}-\d{2}/.test(line))
    .join("\n")
    .trim();
}

/** 文末記号・改行の直後で区切る。短い断片は結合し、長すぎる文はそのまま1チャンクにする。 */
export function splitSpeechChunks(text, maxLength = 90) {
  const normalized = sanitizeSpeechText(text);
  if (!normalized) return [];
  const chunks = [];
  let buffer = "";
  for (const part of normalized.split(/(?<=[。．.!?！？\n])/)) {
    if (buffer && (buffer + part).length > maxLength) { chunks.push(buffer.trim()); buffer = part; }
    else buffer += part;
  }
  if (buffer.trim()) chunks.push(buffer.trim());
  return chunks.filter(Boolean);
}

/** reasoning モデルの `<think>` を読み上げから外す（閉じタグが無ければ以降すべて推論扱い）。 */
export function stripThinkForSpeech(text) {
  let rest = String(text ?? "");
  let out = "";
  for (;;) {
    const start = rest.indexOf("<think>");
    if (start === -1) return out + rest;
    out += rest.slice(0, start);
    const after = rest.slice(start + "<think>".length);
    const end = after.indexOf("</think>");
    if (end === -1) return out;
    rest = after.slice(end + "</think>".length);
  }
}

/** 字幕を読める時間（合成に失敗したページ・読み上げ無効時の送り間隔）。 */
export function readingDelay(text) {
  return Math.max(1800, Math.min(6000, 900 + [...String(text)].length * 55));
}

export function createVoice({ motion, lipSync, pager, status, onCancelVisible, onFinished }) {
  let volume = 0.35; // PICOAGENT_TTS_VOLUME の既定(35)。設定適用前に鳴る音を100%にしない
  let enabled = null; // 読み上げが有効か（pet_speak の結果から覚える）。false のとき合成を要求しない
  let current = null;
  let session = 0;
  let failureNotified = false;
  let streamSpeech = null;

  // 画面の0〜100は人が感じる音量として扱う。振幅へ直線変換すると1%でも大きく聞こえるため二乗する。
  const gain = () => volume * volume;

  function shouldLipSync(audio) {
    return Boolean(audio && audio === current && enabled !== false && volume > 0.001 && !audio.paused && !audio.ended);
  }

  function notifyFailure(error) {
    logApp("warn", "音声の事前合成に失敗しました", errorDetail(error));
    if (failureNotified) return;
    failureNotified = true;
    const detail = String(error?.message || error || "").trim();
    status.set(detail
      ? `読み上げできません: ${detail}`
      : "読み上げできません。設定の「追加機能」から音声読み上げを導入してください");
  }

  /** 合成だけ行い、再生はしない。失敗・無効・空は null。 */
  async function prepare(text) {
    const speech = String(text ?? "").trim();
    if (!speech) return null;
    try {
      const result = await bridge.speak(speech);
      if (result?.enabled === false) enabled = false;
      else if (result?.enabled) { enabled = true; failureNotified = false; }
      if (!result?.enabled || result.skipped || !result.audio?.length) return null;
      return result;
    } catch (error) {
      notifyFailure(error);
      return null;
    }
  }

  function makeAudio(result) {
    const url = URL.createObjectURL(new Blob([new Uint8Array(result.audio)], { type: result.mime || "audio/wav" }));
    const audio = new Audio(url);
    audio.picoLipSync = result.lipSync || [];
    audio.volume = gain();
    return { audio, url };
  }

  function onPlaying(audio, isCurrent) {
    audio.addEventListener("playing", () => {
      if (!isCurrent()) return;
      status.set("読み上げ中…", "voice");
      if (!shouldLipSync(audio)) { motion.setSpeaking(false); return; }
      motion.setSpeaking(true);
      lipSync.attach(audio, () => shouldLipSync(audio));
    }, { once: true });
  }

  function endSpeaking() {
    lipSync.detach();
    motion.setSpeaking(false);
  }

  /** 1つの音声を再生して、終わる（または止められる・失敗する）まで待つ。 */
  function playAndWait(result, voiceSession) {
    return new Promise(resolve => {
      if (voiceSession !== session) { resolve(); return; }
      const { audio, url } = makeAudio(result);
      current = audio;
      let settled = false;
      const done = () => {
        if (settled) return;
        settled = true;
        URL.revokeObjectURL(url);
        if (current === audio) current = null;
        resolve();
      };
      audio.addEventListener("ended", done, { once: true });
      audio.addEventListener("pause", done, { once: true });
      audio.addEventListener("error", () => {
        logApp("error", "音声データの読み込みに失敗しました", audio.error?.message || "");
        done();
      }, { once: true });
      onPlaying(audio, () => voiceSession === session && current === audio);
      audio.play().catch(error => {
        logApp("error", "音声再生に失敗しました", errorDetail(error));
        done();
      });
      status.set("音声再生準備中…", "voice");
    });
  }

  function finish() {
    onCancelVisible(false);
    endSpeaking();
    status.clear("voice");
    onFinished?.();
  }

  /** 進行中の読み上げ（合成待ち・連続再生・字幕送り）をすべて止める。 */
  function stop() {
    session += 1;
    pager.cancel();
    const playing = current;
    current = null;
    playing?.pause();
    streamSpeech = null;
    onCancelVisible(false);
    endSpeaking();
    status.clear("voice");
  }

  /** 事前合成済みの音声を再生する（起動時の挨拶・目覚まし・自発雑談）。 */
  async function playPrepared(result) {
    stop();
    const voiceSession = session;
    onCancelVisible(true);
    await playAndWait(result, voiceSession);
    if (voiceSession === session) finish();
  }

  /** 本文を読み上げる。配信形式では字幕のページと同期して読む。 */
  async function speak(text) {
    // 読み上げが無効・音量0なら合成しない（準備中表示や中止ボタンを出さない）。字幕の送りだけ行う。
    if (enabled === false || volume <= 0.001) {
      pager.timed(text);
      return;
    }
    const chunks = pager.paginates() ? pager.pages(text) : splitSpeechChunks(text);
    stop();
    const voiceSession = session;
    if (!chunks.length) return;
    onCancelVisible(true);
    const startedAt = Date.now();
    status.set("読み上げ準備中…", "voice");
    const prepareTimer = setInterval(() => {
      if (status.owner === "voice" && status.text.startsWith("読み上げ準備中")) {
        status.set(`読み上げ準備中… ${Math.floor((Date.now() - startedAt) / 1000)}秒`, "voice");
      }
    }, 1000);
    try {
      let next = prepare(chunks[0]);
      for (let index = 0; index < chunks.length; index += 1) {
        const result = await next;
        if (voiceSession !== session) return;
        next = index + 1 < chunks.length ? prepare(chunks[index + 1]) : null;
        pager.show(chunks[index]);
        if (!result) {
          // 合成失敗でも字幕を飛ばさず、読める時間だけ表示して次へ進む。
          await new Promise(resolve => setTimeout(resolve, readingDelay(chunks[index])));
          continue;
        }
        clearInterval(prepareTimer);
        await playAndWait(result, voiceSession);
        if (voiceSession !== session) return;
      }
      finish();
    } catch (error) {
      if (voiceSession === session) stop();
      logApp("error", "読み上げに失敗しました", errorDetail(error));
      status.flash(`読み上げに失敗: ${errorMessage(error)}`, 12000);
    } finally {
      clearInterval(prepareTimer);
    }
  }

  // ── 生成中の先行読み上げ: 確定した文から順に合成・再生する ──────────────
  function beginStream() {
    streamSpeech = { queue: [], consumed: "", spoken: false, done: false, disabled: enabled === false || volume <= 0.001, running: false };
  }

  function resetStream() {
    const disabled = streamSpeech?.disabled ?? (enabled === false);
    if (streamSpeech?.running) stop();
    beginStream();
    streamSpeech.disabled = disabled;
  }

  function cancelStream() {
    if (streamSpeech?.running) stop();
    streamSpeech = null;
  }

  function feedStream(buffer) {
    const state = streamSpeech;
    if (!state || state.disabled || state.done) return;
    const cleaned = stripThinkForSpeech(buffer);
    if (cleaned.length <= state.consumed.length) return;
    const settled = cleaned.slice(state.consumed.length).match(/^[\s\S]*[。．.!?！？\n]/)?.[0];
    if (!settled || settled.trim().length < 8) return; // 極端に短い断片は次の文とまとめる
    state.consumed += settled;
    state.queue.push(...splitSpeechChunks(settled));
    runStream();
  }

  async function runStream() {
    const state = streamSpeech;
    if (!state || state.running || state.disabled || !state.queue.length) return;
    state.running = true;
    const playing = current;
    current = null;
    playing?.pause();
    const voiceSession = ++session;
    onCancelVisible(true);
    try {
      let pending = state.queue.shift();
      let synth = prepare(pending);
      while (pending !== undefined) {
        const result = await synth;
        if (voiceSession !== session || streamSpeech !== state) return;
        if (!result && !state.spoken) {
          // 最初から合成できない（読み上げ無効など）: 先行読み上げを諦め、完了時の全文読み上げに任せる。
          state.disabled = true;
          state.queue.length = 0;
          onCancelVisible(false);
          return;
        }
        pending = state.queue.shift();
        synth = pending !== undefined ? prepare(pending) : null;
        if (result) {
          state.spoken = true;
          await playAndWait(result, voiceSession);
          if (voiceSession !== session || streamSpeech !== state) return;
        }
        while (pending === undefined && !state.done) {
          await new Promise(resolve => setTimeout(resolve, 120));
          if (voiceSession !== session || streamSpeech !== state) return;
          pending = state.queue.shift();
          if (pending !== undefined) synth = prepare(pending);
        }
      }
      finish();
    } finally {
      state.running = false;
    }
  }

  /** 最終本文で締める。先行読み上げが無効・本文不一致なら false（呼び出し側が全文を読み上げる）。 */
  function finishStream(finalText) {
    const state = streamSpeech;
    if (!state || state.disabled) { streamSpeech = null; return false; }
    const text = String(finalText ?? "");
    if (!text.startsWith(state.consumed)) { cancelStream(); return false; }
    state.queue.push(...splitSpeechChunks(text.slice(state.consumed.length)));
    state.done = true;
    runStream();
    return true;
  }

  return {
    prepare,
    playPrepared,
    speak,
    stop,
    beginStream,
    resetStream,
    cancelStream,
    feedStream,
    finishStream,
    gain,
    get volume() { return volume; },
    setVolume(value) {
      const numeric = parseFloat(value);
      if (!Number.isFinite(numeric)) return;
      volume = Math.min(1, Math.max(0, numeric / 100));
      if (current) current.volume = gain();
      if (volume <= 0.001) endSpeaking();
      else if (shouldLipSync(current)) {
        motion.setSpeaking(true);
        lipSync.attach(current, () => shouldLipSync(current));
      }
    },
    get enabled() { return enabled; },
    setEnabled(value) {
      enabled = value !== false;
      if (!enabled) stop();
    },
    get speaking() { return Boolean(current); },
  };
}
