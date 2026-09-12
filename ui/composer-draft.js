// composer-draft.js - 入力欄の「書きかけ」保存値を、画面から切り離して正規化する。
//
// 入力履歴（↑↓で呼び戻す過去の発話）はここで扱わない。起動時とキャラクター切替時に
// `hydrateHistory()` が会話履歴（バックエンドの永続データ）から組み直しており、
// 別の保存先を持つと同じ状態を二重に書き換えることになるため。
// 送らずに終了した書きかけだけが、どこにも残っていなかった。

(function registerComposerDraft(root) {
  "use strict";

  // 何日も前の書きかけを起動時に戻すと、いまの話題と食い違って混乱のもとになる。
  const DRAFT_TTL_MS = 24 * 60 * 60 * 1000;
  // 画面の入力欄と同じ上限を置かず、保存側は極端に長い値だけを弾く。
  const DRAFT_MAX_CHARS = 20000;

  // 保存文字列から書きかけを取り出す。壊れた値・期限切れ・型違いは空文字へ落とす。
  function readDraft(raw, now = Date.now()) {
    let parsed = null;
    try {
      parsed = JSON.parse(String(raw ?? ""));
    } catch {
      // フォールバック許可: 破損したUIキャッシュは書きかけ無しとして作り直す。
      // 会話履歴はバックエンドが別に保存しているため、ここで失われるのは未送信の下書きだけ。
      return "";
    }
    if (!parsed || typeof parsed !== "object") return "";
    if (typeof parsed.draft !== "string" || !parsed.draft) return "";
    const savedAt = Number(parsed.draftAt);
    if (!Number.isFinite(savedAt) || savedAt <= 0) return "";
    if (now - savedAt >= DRAFT_TTL_MS) return "";
    return Array.from(parsed.draft).slice(0, DRAFT_MAX_CHARS).join("");
  }

  function serializeDraft(draft, now = Date.now()) {
    const value = Array.from(String(draft ?? "")).slice(0, DRAFT_MAX_CHARS).join("");
    return JSON.stringify({ draft: value, draftAt: now });
  }

  root.PicoComposerDraft = Object.freeze({ DRAFT_TTL_MS, DRAFT_MAX_CHARS, readDraft, serializeDraft });
}(globalThis));
