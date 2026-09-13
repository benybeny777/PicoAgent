// quick-actions.js - 入力欄のクイック操作の既定一覧と、表示してよいボタンの絞り込み。
//
// 連携スキル（Gmail・Slack・Googleカレンダー）に頼る既定ボタンは、設定が無いと押しても
// 設定手順の案内が返るだけで用が済まない。必要な設定キーを既定の隣に書き、値の有無だけを
// バックエンド（`pet_config_presence`）へ問い合わせて、押せば用が済むボタンだけを出す。
// 必要なキーの正本はここだけに置く（JS と Rust の2か所で判定規則を持たない）。

(function registerQuickActions(root) {
  "use strict";

  const DEFAULT_QUICK_ACTIONS = Object.freeze([
    { label: "👀 画面を見る", text: "今の画面を見て、状況を短く教えて" },
    { label: "📋 クリップボード要約", text: "クリップボードの内容を要約して" },
    { label: "🗓 今日の予定", text: "今日の予定と気をつけることを教えて" },
    {
      label: "📧 未読メール要約",
      text: "Gmailの未読メールを一覧取得して、重要そうな順に短く要約して",
      requires: ["PICOAGENT_GMAIL_ACCESS_TOKEN"],
    },
    {
      label: "💬 Slack要約",
      text: "Slackの既定チャンネルの最新10件を読んで、話題と対応が必要なことを短く要約して",
      // 履歴の読み取りは Bot token と既定チャンネルの両方が要る（Webhook URL では読めない）。
      requires: ["PICOAGENT_SLACK_BOT_TOKEN", "PICOAGENT_SLACK_CHANNEL"],
    },
    {
      label: "📅 予定確認",
      text: "Googleカレンダーの今日の予定を確認して、次に気をつけることを教えて",
      requires: ["PICOAGENT_GOOGLE_CALENDAR_ACCESS_TOKEN"],
    },
    { label: "⏩ さっきの続き", text: "さっきの続きとして、次にやることを整理して" },
    { label: "✨ できること", text: "あなたが今できることを具体的に教えて" },
  ].map(item => Object.freeze({ ...item, requires: item.requires ? Object.freeze([...item.requires]) : undefined })));

  // 設定 PICOAGENT_QUICK_ACTIONS（1行1件「ラベル|送る文章」）を読む。利用者が書いた分は
  // 必要キーを持たないので、絞り込みの対象にしない。有効な行が無ければ null（既定を使う）。
  function parseQuickActions(text) {
    const items = String(text ?? "")
      .split("\n")
      .map(line => line.trim())
      .filter(Boolean)
      .map(line => {
        const sep = line.indexOf("|");
        if (sep <= 0) return null;
        const label = line.slice(0, sep).trim();
        const body = line.slice(sep + 1).trim();
        return label && body ? { label, text: body } : null;
      })
      .filter(Boolean);
    return items.length ? items : null;
  }

  // 問い合わせに使う設定キーを重複なしで返す。
  function requiredKeys(items) {
    const keys = new Set();
    for (const item of items || []) {
      for (const key of item.requires || []) keys.add(key);
    }
    return [...keys];
  }

  // 必要キーがすべて「値あり」と確認できたボタンだけを残す。確認できていないキーは無しとして扱う。
  function visibleQuickActions(items, presence) {
    return (items || []).filter(item => !item.requires || item.requires.every(key => presence?.[key] === true));
  }

  root.PicoQuickActions = Object.freeze({ DEFAULT_QUICK_ACTIONS, parseQuickActions, requiredKeys, visibleQuickActions });
}(globalThis));
