// status.js - 状態行（#status、role="status"）の持ち主。
//
// 状態行には2層ある。下の層は「送れない理由」など消してはいけない常設の文言（AIの準備中・失敗）、
// 上の層は一時的な文言（読み上げ中・コピーしました など）。一時的な文言を消すと常設の文言へ戻る
// （空にすると赤い空欄だけが残り、送れない理由が分からなくなるため）。
// 一時的な文言は持ち主（owner）付きで出し、持ち主が自分の文言だけを消せるようにする。

export function createStatus(element, { onBootText } = {}) {
  let sticky = "";
  let stickyKind = "";
  let transient = "";
  let owner = "";
  let flashTimer = 0;

  function render() {
    const text = transient || sticky;
    element.textContent = text;
    element.classList.toggle("engine-pending", !transient && stickyKind === "pending");
    element.classList.toggle("engine-error", stickyKind === "error" && (!transient || transient === sticky));
    onBootText?.(text);
  }

  return {
    get text() { return element.textContent; },
    get owner() { return transient ? owner : ""; },
    /** 一時的な文言を出す。`by` は持ち主（同じ持ち主の clear だけが消せる）。 */
    set(text, by = "") {
      clearTimeout(flashTimer);
      flashTimer = 0;
      transient = String(text || "");
      owner = by;
      render();
    },
    /** 一定時間だけ出して、その文言のままなら消す。 */
    flash(text, ms = 1600) {
      clearTimeout(flashTimer);
      transient = String(text || "");
      owner = "flash";
      render();
      flashTimer = setTimeout(() => {
        flashTimer = 0;
        if (owner === "flash") { transient = ""; owner = ""; render(); }
      }, ms);
    },
    /** 一時的な文言を消す。`by` を渡したときは、その持ち主の文言だけを消す。 */
    clear(by = "") {
      if (by && owner !== by) return;
      clearTimeout(flashTimer);
      flashTimer = 0;
      transient = "";
      owner = "";
      render();
    },
    /** 常設の文言（AIの準備中・失敗など）。空文字で外す。 */
    setSticky(text, kind = "pending") {
      sticky = String(text || "");
      stickyKind = sticky ? kind : "";
      render();
    },
    setBusy(on) {
      element.classList.toggle("busy", Boolean(on));
    },
  };
}
