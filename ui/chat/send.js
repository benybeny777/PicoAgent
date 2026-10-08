// send.js - 会話1件の送信（添付の読み込み・ストリーミング表示・先行読み上げ・中断・再試行）の持ち主。

import * as bridge from "../app/bridge.js";
import { errorDetail, errorMessage, logApp } from "../app/util.js";

/** 画面を見てほしい系の発話（スクリーンショットを自動で添付する）。 */
function isScreenshotQuery(text) {
  return /画面を見て|スクリーンショット|screenshot|画面の状態|画面を確認|今の画面|デスクトップを見て|画面キャプチャ/.test(text.toLowerCase());
}

// ストリーミング表示の最短再描画間隔。12回/秒で文字が流れて見え、毎フレーム組み直すより描画量が減る。
const STREAM_RENDER_MIN_MS = 80;
// 停止要求のあと応答が返らない場合に、画面側の待機だけ解除するまでの時間。
const CANCEL_RELEASE_MS = 8000;

export function createSender({ chat, voice, motion, notices, engine, relationship, ambient, mic }) {
  const { composer, status } = chat;
  let busy = false;
  let revision = 0;
  let last = { text: "", images: [], files: [] };
  let releasePending = null;
  let cancelTimer = null;
  let activeRequest = null;
  const taskRun = window.PicoTaskRun.createController({
    container: chat.taskRun,
    onCancel: button => sender.cancel(button),
    onVisibilityChange: visible => {
      chat.element.classList.toggle("task-run-visible", visible);
      chat.history.render();
    },
    onRender: () => chat.history.scrollToEnd(),
  });

  function hasLastRequest() {
    return Boolean(last.text) || last.images.length > 0 || last.files.length > 0;
  }

  function updateButtons() {
    const blockedReason = engine.blockedReason();
    composer.setBlocked({ busy, reason: blockedReason });
    const regenerate = chat.buttons.regenerateButton;
    regenerate.disabled = busy || Boolean(blockedReason) || !hasLastRequest();
    regenerate.title = busy ? "返答が終わると再生成できます"
      : blockedReason ? "AIの準備完了後に再生成できます"
        : !hasLastRequest() ? "会話を送信すると再生成できます"
          : "同じ質問と添付で別の答えを生成";
  }

  function clearCancelRelease() {
    if (cancelTimer) clearTimeout(cancelTimer);
    cancelTimer = null;
    releasePending = null;
  }

  async function subscribe(requestRevision, jobId, job, stream, cancelled) {
    const alive = () => busy && requestRevision === revision && !cancelled();
    const unlisteners = [];
    const add = async (channel, handler) => {
      if (cancelled()) return;
      try {
        const release = await bridge.listen(channel, handler);
        if (cancelled()) release();
        else unlisteners.push(release);
      }
      catch (error) { logApp("warn", `${channel} の購読に失敗しました`, errorDetail(error)); }
    };
    // 表示文言ではなく構造化イベントで、実際に動いたツールと成否を追跡する。
    await add("pet:agent_status", progress => {
      if (!alive() || typeof progress !== "string" || !progress) return;
      const line = progress.replace(/\s*\n+\s*/g, " / ");
      status.set(line, "send");
      chat.setThinking(true, line);
      notices.upsertJob(jobId, { body: line, status: "running" });
      taskRun.status(progress);
      if (/検索|search|web|ブラウザ/i.test(progress)) motion.set("searching");
      else if (/ツールを実行中|ステップ/.test(progress)) motion.set("working");
      else if (/計画|考え/.test(progress)) motion.set("thinking");
    });
    await add("pet:agent_activity", activity => {
      if (!alive() || !activity || typeof activity !== "object") return;
      const tools = Array.isArray(activity.tools) ? activity.tools.filter(name => typeof name === "string") : [];
      for (const name of tools) job.tools.add(name);
      if (activity.phase === "started") job.didWork = tools.length > 0;
      if (activity.phase === "completed" && activity.success === false) job.toolFailed = true;
      taskRun.activity(activity);
    });
    // ステップ境界（ツール実行を挟んだ再生成）では、途中本文を捨てて最終回答だけを表示・読み上げする。
    await add("pet:stream_reset", () => {
      if (!alive() || !stream.buffer) return;
      stream.buffer = "";
      if (stream.frame) { cancelAnimationFrame(stream.frame); stream.frame = 0; }
      stream.lastAt = 0;
      chat.renderStreaming("");
      chat.setThinking(true, status.text || "考え中");
      voice.resetStream();
    });
    await add("pet:stream_chunk", chunk => {
      if (!alive()) return;
      stream.buffer += chunk;
      chat.history.setThinking(null);
      // 配信の字幕は検査済みの全文をページ表示するので、途中本文は出さず先行読み上げもしない。
      if (chat.variant === "column") return;
      voice.feedStream(stream.buffer);
      if (stream.frame) return;
      const paint = () => {
        if (!alive()) { stream.frame = 0; return; }
        if (performance.now() - stream.lastAt < STREAM_RENDER_MIN_MS) {
          stream.frame = requestAnimationFrame(paint);
          return;
        }
        stream.frame = 0;
        stream.lastAt = performance.now();
        chat.renderStreaming(stream.buffer);
        if (motion.current === "thinking") motion.set("talking");
      };
      stream.frame = requestAnimationFrame(paint);
    });
    return () => unlisteners.forEach(release => release());
  }

  async function readAttachments(files, requestRevision, released, cancelled) {
    let context = "";
    for (const file of files) {
      if (requestRevision !== revision || cancelled()) return null;
      // ネイティブのドロップ由来の添付は実パスも伝え、スキルが直接使えるようにする。
      const label = file.path ? `${file.name}（パス: ${file.path}）` : file.name;
      try {
        const content = await Promise.race([bridge.readFileContent(file.name, file.dataUrl), released]);
        if (cancelled() || content?.releasedByCancel) return null;
        context += `[添付ファイル: ${label}]\n${content}\n\n`;
      } catch (error) {
        if (cancelled()) return null;
        logApp("warn", `ファイル読み込み失敗: ${file.name}`, errorDetail(error));
        context += `[添付ファイル: ${label}] (読み込みに失敗しました)\n\n`;
      }
    }
    return context;
  }

  function showFailure(message) {
    voice.cancelStream();
    voice.stop();
    chat.showError(message, hasLastRequest() ? () => {
      if (busy) return;
      composer.restore(last);
    } : null);
    chat.history.add("assistant", message);
    motion.react("sad", 2200);
  }

  async function submit() {
    const blockedReason = engine.blockedReason();
    if (blockedReason) {
      engine.explainBlocked();
      return;
    }
    if (busy || !composer.hasContent()) return;
    const requestRevision = revision;
    const jobId = `chat-${Date.now()}-${requestRevision}`;
    const job = { state: "running", didWork: false, toolFailed: false, tools: new Set() };
    const cancellation = { cancelled: false };
    activeRequest = cancellation;
    const cancelled = () => cancellation.cancelled;
    ambient.onUserActivity();
    busy = true;
    const request = composer.take();
    last = request;
    voice.stop();
    composer.close();
    status.clear();
    status.setBusy(true);
    motion.setBusy(true);
    chat.buttons.cancelSendButton.hidden = false;
    chat.buttons.cancelSendButton.disabled = false;
    updateButtons();
    chat.history.add("user", request.text);
    chat.setThinking(true);
    notices.upsertJob(jobId, { title: request.text || "添付ファイルの処理", body: "考え中…", status: "running", prompt: request.text, at: new Date().toISOString() });
    taskRun.start();
    const stream = { buffer: "", frame: 0, lastAt: 0 };
    voice.beginStream();
    const released = new Promise(resolve => { releasePending = resolve; });
    let unsubscribe = () => {};
    try {
      const subscription = subscribe(requestRevision, jobId, job, stream, cancelled);
      const subscribed = await Promise.race([subscription, released]);
      if (typeof subscribed === "function") unsubscribe = subscribed;
      else { subscription.then(release => release()); job.state = "cancelled"; return; }
      if (requestRevision !== revision || cancelled()) { job.state = "cancelled"; return; }
      let fileContext = "";
      if (request.files.length) {
        status.set("ファイルを読み込み中…", "send");
        fileContext = await readAttachments(request.files, requestRevision, released, cancelled);
        if (fileContext == null) { job.state = "cancelled"; return; }
      }
      const fullText = fileContext ? `${fileContext}${request.text || "このファイルについて教えて"}` : request.text;
      const images = [...request.images];
      if (isScreenshotQuery(request.text)) {
        // 画面を見る依頼で撮影できなかったら、画像なしでAIへ送らない（見えていない画面を推測させない）。
        // Waylandの拒否なども成功扱いにせず、理由を出して再送できる失敗にする。
        let shot = null;
        try {
          status.set("画面を撮影中…", "send");
          shot = await Promise.race([bridge.captureScreen(), released]);
        } catch (error) {
          if (requestRevision !== revision || cancelled()) { job.state = "cancelled"; return; }
          job.state = "failed";
          logApp("warn", "スクリーンショット取得に失敗しました。", errorDetail(error));
          showFailure(`画面を撮影できませんでした: ${errorMessage(error)}`);
          return;
        }
        if (shot?.dataUrl) images.push(shot.dataUrl);
        else if (!shot?.releasedByCancel && requestRevision === revision && !cancelled()) {
          job.state = "failed";
          logApp("warn", "スクリーンショットの画像を受け取れませんでした。", JSON.stringify(shot));
          showFailure("画面を撮影できませんでした: 画像を受け取れませんでした");
          return;
        }
      }
      if (requestRevision !== revision || cancelled()) { job.state = "cancelled"; return; }
      // 停止要求のあと応答が返らない場合に備え、打ち切りの合図と競走させる。
      const result = await Promise.race([images.length ? bridge.sendImage(fullText, images) : bridge.send(fullText), released]);
      if (result?.stale || requestRevision !== revision || cancelled()) {
        job.state = "cancelled";
        chat.clearThinking();
        voice.cancelStream();
        if (result?.releasedByCancel) status.flash("停止しました（応答が返らないので画面の待機を解除したよ）", 6000);
        return;
      }
      if (String(result?.reply || "").includes("いまの処理は途中で止めたよ")) job.state = "cancelled";
      chat.setThinking(false);
      status.clear("send");
      composer.setChoices(result.error ? null : result.choices);
      if (result.error) {
        job.state = "failed";
        logApp("error", "AI応答がエラーを返しました", result.reply || "");
        showFailure(result.reply);
        status.flash("接続を確認してね", 4000);
        return;
      }
      status.setBusy(false);
      chat.buttons.cancelSendButton.hidden = true;
      const reply = String(result.reply ?? "");
      // 返信が入力のオウム返しなら読み上げない（入力は声に出さない）。
      if (reply.trim() !== request.text) {
        if (chat.variant === "column") { voice.cancelStream(); voice.speak(reply); }
        else if (!voice.finishStream(reply)) voice.speak(reply);
      } else {
        voice.cancelStream();
      }
      if (chat.variant === "column") {
        chat.lastAssistantText = reply; // 字幕のページ送りまたはページ同期の読み上げが表示を持つ
      } else {
        chat.renderReply(reply);
      }
      if (requestRevision !== revision) { job.state = "cancelled"; return; }
      chat.history.add("assistant", reply);
      if (result.quitApp) setTimeout(() => bridge.quit().catch(error => logApp("error", "アプリ終了に失敗しました", errorDetail(error))), 900);
    } catch (error) {
      if (requestRevision !== revision) { job.state = "cancelled"; return; }
      if (cancelled()) { job.state = "cancelled"; return; }
      job.state = "failed";
      logApp("error", "AI応答の取得に失敗しました", errorDetail(error));
      showFailure(`応答に失敗しました: ${errorMessage(error)}`);
      status.flash("接続を確認してね", 4000);
    } finally {
      unsubscribe();
      if (stream.frame) cancelAnimationFrame(stream.frame);
      chat.setThinking(false);
      chat.clearThinking();
      status.setBusy(false);
      status.clear("send");
      chat.buttons.cancelSendButton.hidden = true;
      clearCancelRelease();
      busy = false;
      if (activeRequest === cancellation) activeRequest = null;
      motion.setBusy(false);
      const failed = job.state === "failed";
      const cancelled = job.state === "cancelled";
      taskRun.finish(failed ? "failed" : cancelled ? "cancelled" : "completed");
      notices.upsertJob(jobId, {
        body: failed ? "処理に失敗しました。詳細は会話を確認してください。" : cancelled ? "処理を中断しました。" : "処理が完了しました。",
        status: failed ? "failed" : cancelled ? "cancelled" : "completed",
        ok: !failed,
      });
      updateButtons();
      if (!cancelled) {
        const detail = JSON.stringify({ tools: [...job.tools], resumePrompt: last.text });
        if (failed || job.toolFailed) {
          motion.react("concern", 1100);
          if (job.didWork) relationship.record("task_failure", detail);
        } else {
          motion.react("success", 1100);
          if (job.didWork) relationship.record("task_success", detail);
        }
      }
      ambient.afterConversation();
      mic.scheduleContinuous();
    }
  }

  const sender = {
    get busy() { return busy; },
    get revision() { return revision; },
    hasLastRequest,
    submit,
    updateButtons,
    /** 停止を要求する。応答が返らないまま待ち続けないよう、一定時間で画面側の待機を解除する。 */
    async cancel(button) {
      if (!busy) return;
      const cancellation = activeRequest;
      const release = releasePending;
      if (cancellation) cancellation.cancelled = true;
      voice.cancelStream();
      voice.stop();
      if (button) button.disabled = true;
      status.set("停止を要求しています…", "send");
      if (cancelTimer) clearTimeout(cancelTimer);
      cancelTimer = setTimeout(() => {
        cancelTimer = null;
        if (!busy || activeRequest !== cancellation || !release) return;
        logApp("warn", "停止要求のあと応答が返らないため、画面の待機を解除しました");
        release({ stale: true, releasedByCancel: true });
      }, CANCEL_RELEASE_MS);
      try {
        await bridge.cancel();
      } catch (error) {
        // 旧送信の停止応答が遅れても、次の送信の状態と待機解除を巻き戻さない。
        if (!busy || activeRequest !== cancellation) return;
        logApp("warn", "中断要求の送信に失敗しました", errorDetail(error));
        status.set(`停止を要求できませんでした: ${errorMessage(error)}`, "send");
        if (button) button.disabled = false;
      }
    },
    /** 再生成: 直前の文面と添付をそのまま送り直す。 */
    regenerate() {
      if (busy || !hasLastRequest()) return;
      composer.restore(last);
      chat.composer.form.requestSubmit();
    },
    /** コマンド実行の許可カードのボタン。 */
    approve(value) {
      if (busy) { status.flash("処理中です…少し待ってください", 2000); return; }
      composer.open();
      composer.setValue(value);
      chat.composer.form.requestSubmit();
    },
    /** 「新しい話」: 表示中の会話・控え・選択肢を捨てる（打ちかけの文面は残す）。 */
    reset() {
      revision += 1;
      // 旧通信が戻らなくても、リセット後の入力を待機状態へ閉じ込めない。
      if (busy) void sender.cancel();
      last = { text: "", images: [], files: [] };
      voice.cancelStream();
      voice.stop();
      chat.setThinking(false);
      chat.history.clear();
      chat.lastAssistantText = "";
      composer.clearAfterReset();
      chat.renderReply("新しい話をしよう。", { remember: false });
      composer.close();
      updateButtons();
    },
  };
  composer.onSubmit(submit);
  chat.buttons.cancelSendButton.addEventListener("click", () => { if (busy) sender.cancel(chat.buttons.cancelSendButton); });
  chat.buttons.regenerateButton.addEventListener("click", event => { event.stopPropagation(); sender.regenerate(); });
  return sender;
}
