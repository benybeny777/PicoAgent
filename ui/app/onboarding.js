// onboarding.js - 初回案内（キャラクターモデルの追加）と、音声読み上げの自動準備。
//
// 案内はキャラクターモデル追加の1画面だけ。モデルは同梱・再配布せず、公式配布元へ案内して利用者自身に
// 取得してもらう。読み上げ（Aivis）は選ばせず初回に自動で入れ、進捗は状態行（起動中は起動画面）へ出す。

import * as bridge from "./bridge.js";
import { toast } from "./toast.js";
import { errorDetail, logApp } from "./util.js";

export function createOnboarding({ dialog, status, openExternalUrl, onComplete, isEngineBusy }) {
  async function complete(openCharacterSettings) {
    dialog.querySelectorAll("button").forEach(item => { item.disabled = true; });
    try {
      onComplete(await bridge.completeOnboarding());
      dialog.hidden = true;
    } catch (error) {
      logApp("error", "初回設定を保存できませんでした", errorDetail(error));
      toast("初回設定を保存できませんでした", "error");
      dialog.querySelectorAll("button").forEach(item => { item.disabled = false; });
      return;
    }
    if (openCharacterSettings) {
      try { await bridge.openSettings("character-add"); }
      catch (error) {
        logApp("error", "キャラクター追加画面を開けませんでした", errorDetail(error));
        toast("キャラクター追加画面を開けませんでした", "error");
      }
    }
  }

  dialog.addEventListener("click", async event => {
    const target = event.target instanceof Element ? event.target : null;
    const link = target?.closest("a.onboarding-link");
    if (link) {
      event.preventDefault();
      await openExternalUrl(link.href);
      return;
    }
    const action = target?.closest("button[data-onboarding-action]")?.dataset.onboardingAction;
    if (action === "add-character") await complete(true);
    else if (action === "skip-character") await complete(false);
  });
  // 案内の中だけでフォーカスを回す。
  dialog.addEventListener("keydown", event => {
    if (event.key !== "Tab") return;
    const controls = [...dialog.querySelectorAll("button:not([disabled]),a[href]")].filter(item => item.getClientRects().length > 0);
    if (!controls.length) return;
    const first = controls[0];
    const last = controls[controls.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  });

  /** 初回の音声導入も起動画面で待ち、通常画面へ未準備の声を持ち越さない。 */
  async function installVoice() {
    const statuses = await bridge.componentStatus();
    const aivis = statuses.find(item => item.id === "aivis");
    if (!aivis || aivis.ready || !aivis.available) return;
    let release = null;
    try {
      release = await bridge.listen("component:progress", payload => {
        // ローカルモデルの取得中はそちらが状態行の主。送信を塞いでいる方の理由を消さない。
        if (isEngineBusy()) return;
        const downloaded = Number(payload?.downloaded) || 0;
        const total = Number(payload?.total) || 0;
        const size = total > 0 ? ` ${Math.round(downloaded / total * 100)}%（${(downloaded / 1e6).toFixed(0)} / ${(total / 1e6).toFixed(0)}MB）` : "";
        status.set(`音声読み上げを準備中…${size} ※初回だけ`, "voice-install");
      });
    } catch (error) {
      logApp("warn", "音声読み上げの進捗を受け取れませんでした", errorDetail(error));
    }
    try {
      await bridge.installComponents(["aivis"], true); // 導入できた時点で読み上げも有効にする
      status.flash("音声読み上げを準備しました", 3000);
    } catch (error) {
      logApp("warn", "音声読み上げを自動で準備できませんでした", errorDetail(error));
      const message = navigator.onLine
        ? "音声読み上げを準備できませんでした。設定の「追加機能・更新」から入れ直せます"
        : "オフラインのため音声読み上げの準備を見送りました";
      status.flash(message, 8000);
      return { error: message };
    } finally {
      release?.();
    }
  }

  return {
    show() {
      dialog.querySelectorAll("button").forEach(item => { item.disabled = false; });
      dialog.hidden = false;
      dialog.querySelector("button[data-onboarding-action]")?.focus();
    },
    get open() { return !dialog.hidden; },
    installVoice,
  };
}
