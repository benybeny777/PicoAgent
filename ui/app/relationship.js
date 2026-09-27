// relationship.js - 「いっしょの記録」（関係性カード）の持ち主。

import * as bridge from "./bridge.js";
import { toast } from "./toast.js";
import { el, errorDetail, errorMessage, logApp, twoStepAction } from "./util.js";

// ふれあいは連打される導線なので、記録はまとめて間引く（1回ごとに関係性ファイルを読み書きしない）。
const TOUCH_RECORD_INTERVAL_MS = 5000;

export function createRelationship({ card, onResume }) {
  const label = el("strong", { id: "relationshipLabel", textContent: "出会ったばかり" });
  const stats = el("div", { className: "relationship-stats" });
  const moments = el("div", { className: "relationship-moments" });
  const clear = el("button", { type: "button", className: "chip danger", textContent: "記録をすべて消す" });
  const close = el("button", { type: "button", className: "icon-button", textContent: "×" }, [], { "aria-label": "閉じる" });
  const extra = el("div", { className: "relationship-extra" });
  card.replaceChildren(close, el("span", { className: "relationship-kicker", textContent: "いっしょの記録" }), label, stats, moments, clear, extra);
  let memoryEnabled = true;
  let opener = null;
  let lastTouchAt = 0;
  let pendingZone = "";
  let pendingTimer = null;

  const clearConfirmation = twoStepAction(clear, {
    initialLabel: "記録をすべて消す",
    confirmLabel: "もう一度押してすべて消す",
    onArm: () => toast("このキャラクターとの記録をすべて消します。もう一度押すと実行します", "info"),
  });

  function render(data) {
    if (!data) return;
    const state = data.relationship || data;
    label.textContent = data.bondLabel || state.bondLabel || "いっしょの相棒";
    stats.textContent = `会話 ${Number(state.conversations || 0)}回 ・ 完了 ${Number(state.completedTasks || 0)}件 ・ ふれあい ${Number(state.interactions || 0)}回`;
    const recent = Array.isArray(state.recentMoments) ? state.recentMoments.slice(-5).reverse() : [];
    if (!recent.length) {
      moments.replaceChildren(el("div", { className: "relationship-moment", textContent: "これからの出来事がここに残ります。" }));
    } else {
      moments.replaceChildren(...recent.map(moment => {
        const buttons = [];
        if (moment.resumePrompt) {
          const resume = el("button", { type: "button", className: "chip", textContent: "続きをやる" });
          resume.addEventListener("click", () => { hide(); onResume(moment.resumePrompt); });
          buttons.push(resume);
        }
        const pin = el("button", { type: "button", className: "chip", textContent: moment.pinned ? "固定解除" : "固定" });
        pin.addEventListener("click", () => update(() => bridge.pinRelationshipMoment(moment.id, !moment.pinned)));
        const remove = el("button", { type: "button", className: "chip", textContent: "削除" });
        const confirmation = twoStepAction(remove, {
          initialLabel: "削除",
          confirmLabel: "もう一度押す",
          onArm: () => toast("この記録だけを消します。もう一度押すと実行します", "info"),
        });
        remove.addEventListener("click", () => {
          if (!confirmation.request()) return;
          remove.textContent = "削除中…";
          update(() => bridge.deleteRelationshipMoment(moment.id));
        });
        buttons.push(pin, remove);
        return el("div", { className: "relationship-moment", dataset: { pinned: String(moment.pinned === true) } }, [
          el("div", { textContent: moment.text }),
          el("div", { className: "relationship-moment-actions" }, buttons),
        ]);
      }));
    }
    clear.hidden = !state.recentMoments?.length;
  }

  function setBusy(busy) {
    card.setAttribute("aria-busy", String(busy));
    for (const node of card.querySelectorAll("button")) node.disabled = busy;
  }

  async function update(operation) {
    setBusy(true);
    try { render(await operation()); }
    catch (error) {
      logApp("warn", "いっしょの記録を更新できませんでした", errorDetail(error));
      toast("記録を更新できませんでした", "error");
    } finally {
      setBusy(false);
    }
  }

  function hide() {
    card.hidden = true;
    opener?.setAttribute("aria-expanded", "false");
    opener?.focus({ preventScroll: true });
  }

  close.addEventListener("click", hide);
  clear.addEventListener("click", async () => {
    if (!clearConfirmation.request()) return;
    clear.textContent = "記録を削除中…";
    await update(async () => {
      const result = await bridge.clearRelationship();
      toast("いっしょの記録を消しました", "success");
      return result;
    });
    clear.textContent = "記録をすべて消す";
  });

  async function record(kind, detail = "") {
    if (!memoryEnabled) return;
    try { render(await bridge.relationshipEvent(kind, detail)); }
    catch (error) { logApp("warn", "いっしょの記録を更新できませんでした", errorDetail(error)); }
  }

  return {
    render,
    record,
    /** 間引いた分は最後の1回だけ、間隔が空いてから送る（反応そのものは毎回返している）。 */
    recordTouch(zone) {
      const now = Date.now();
      if (now - lastTouchAt >= TOUCH_RECORD_INTERVAL_MS) {
        lastTouchAt = now;
        record(`touch_${zone}`);
        return;
      }
      pendingZone = zone;
      if (pendingTimer) return;
      pendingTimer = setTimeout(() => {
        pendingTimer = null;
        lastTouchAt = Date.now();
        record(`touch_${pendingZone}`);
      }, TOUCH_RECORD_INTERVAL_MS - (now - lastTouchAt));
    },
    async toggle(button) {
      opener = button;
      if (!card.hidden) { hide(); return; }
      card.hidden = false;
      button.setAttribute("aria-expanded", "true");
      button.disabled = true;
      moments.textContent = "記録を読み込んでいます…";
      try { render(await bridge.relationshipState()); }
      catch (error) {
        logApp("warn", "いっしょの記録を読めませんでした", errorDetail(error));
        moments.textContent = `記録を読み込めませんでした: ${errorMessage(error)}`;
      } finally {
        button.disabled = false;
      }
      close.focus({ preventScroll: true });
    },
    hide,
    get open() { return !card.hidden; },
    setMemoryEnabled(enabled, button) {
      memoryEnabled = enabled !== false;
      if (button) button.title = memoryEnabled ? "いっしょの記録を見る" : "いっしょの記録は設定で停止中";
    },
    /** 見た目の違い（variant）や MMD の姿勢の選択欄を置く場所。 */
    extra,
  };
}
