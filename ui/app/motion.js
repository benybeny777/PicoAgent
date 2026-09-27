// motion.js - キャラのモーション状態（待機・考え中・話し中・反応）の持ち主。
//
// 画面の各所は「何が起きたか」だけを伝え、待機へ戻すタイミングはここがまとめて決める。

export function createMotion(host) {
  let current = "idle";
  let speaking = false;
  let busy = false;
  let reactionTimer = 0;

  function apply(name) {
    current = name;
    host.setMotion(name);
  }

  /** 会話・読み上げの状態から、いま戻るべき基本のモーション。 */
  function base() {
    if (speaking) return "talking";
    if (busy) return "thinking";
    return "idle";
  }

  return {
    get current() { return current; },
    /** 状態を直接指定する（考え中の種類など）。 */
    set(name) {
      clearTimeout(reactionTimer);
      reactionTimer = 0;
      apply(name);
    },
    /** 一時的な反応。`ms` 後に基本のモーションへ戻す。 */
    react(name, ms = 850) {
      clearTimeout(reactionTimer);
      apply(name);
      reactionTimer = setTimeout(() => {
        reactionTimer = 0;
        if (current === name) apply(base());
      }, ms);
    },
    setBusy(value) {
      busy = Boolean(value);
      if (!reactionTimer) apply(base());
    },
    setSpeaking(value) {
      speaking = Boolean(value);
      host.setSpeaking(speaking);
      if (!reactionTimer || speaking) {
        clearTimeout(reactionTimer);
        reactionTimer = 0;
        apply(base());
      }
    },
    get speaking() { return speaking; },
    rest() {
      clearTimeout(reactionTimer);
      reactionTimer = 0;
      apply(base());
    },
  };
}
