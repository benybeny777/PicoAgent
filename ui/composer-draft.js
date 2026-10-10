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

  // 不在・期限切れは正常。JSONと保存形式の破損は呼出側で回復を判断する。
  function readDraft(raw, now = Date.now()) {
    const empty = { draft: "", corrupted: false };
    if (raw == null) return empty;
    let parsed;
    try { parsed = JSON.parse(raw); }
    catch { return { draft: "", corrupted: true }; }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)
        || typeof parsed.draft !== "string" || typeof parsed.draftAt !== "number"
        || !Number.isFinite(parsed.draftAt) || parsed.draftAt <= 0) {
      return { draft: "", corrupted: true };
    }
    if (now - parsed.draftAt >= DRAFT_TTL_MS) return empty;
    return { draft: Array.from(parsed.draft).slice(0, DRAFT_MAX_CHARS).join(""), corrupted: false };
  }

  function serializeDraft(draft, now = Date.now()) {
    const value = Array.from(String(draft ?? "")).slice(0, DRAFT_MAX_CHARS).join("");
    return JSON.stringify({ draft: value, draftAt: now });
  }

  root.PicoComposerDraft = Object.freeze({ DRAFT_TTL_MS, DRAFT_MAX_CHARS, readDraft, serializeDraft });
}(globalThis));
