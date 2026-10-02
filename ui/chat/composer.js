// composer.js - 入力欄（本文・入力履歴・書きかけ・添付・クイック操作）の持ち主。
//
// 送信の実行は send.js が行い、ここは「何を送るか」を組み立てて渡すだけ。

import * as bridge from "../app/bridge.js";
import { toast } from "../app/toast.js";
import { el, errorDetail, isImeComposing, logApp, preview } from "../app/util.js";

const DRAFT_KEY = "picoagent.composerDraft.v1";
const PROMPT_HISTORY_LIMIT = 40;
export const MAX_IMAGES = 4;
export const MAX_FILES = 4;

const FILE_ICONS = {
  xlsx: "📊", xls: "📊", xlsm: "📊", csv: "📋", tsv: "📋", docx: "📝", doc: "📝", docm: "📝",
  pptx: "📄", pptm: "📄", ppt: "📄", pdf: "📕", json: "📦", toml: "📦", yaml: "🗂", yml: "🗂",
  txt: "📃", md: "📃", log: "📃", psd: "🎨", xcf: "🎨", heic: "🖼", heif: "🖼", tif: "🖼", tiff: "🖼",
  webp: "🖼", avif: "🖼", bmp: "🖼", ico: "🖼",
};

export function createComposer({ form, input, preview: previewBox, suggestions, quickActions, sendButton, status }) {
  const draftStore = window.PicoComposerDraft;
  const quick = window.PicoQuickActions;
  const promptHistory = [];
  let historyIndex = null;
  let draft = readDraft();
  let draftTimer = 0;
  let images = [];
  let files = [];
  let blocked = { busy: false, reason: "" };
  let quickItems = null; // 利用者定義（null なら既定）
  let defaultItems = quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, {});
  let choices = null; // 質問カード（ask_user）の選択肢
  let onSubmit = () => {};
  let onOpenChange = () => {};
  let persistent = false;

  function readDraft() {
    try { return draftStore.readDraft(localStorage.getItem(DRAFT_KEY)); }
    catch (error) {
      // フォールバック許可: localStorage を読めない環境では書きかけの復元だけを諦めて会話は続ける。
      logApp("warn", "書きかけを読めませんでした", errorDetail(error));
      return "";
    }
  }
  function persistDraft() {
    if (draftTimer) { clearTimeout(draftTimer); draftTimer = 0; }
    try { localStorage.setItem(DRAFT_KEY, draftStore.serializeDraft(draft)); }
    catch (error) { logApp("warn", "書きかけを保存できませんでした", errorDetail(error)); }
  }

  function autosize() {
    input.style.height = "auto";
    input.style.height = `${Math.min(180, Math.max(44, input.scrollHeight))}px`;
  }

  function hasContent() {
    return Boolean(input.value.trim()) || images.length > 0 || files.length > 0;
  }

  function updateState() {
    sendButton.disabled = blocked.busy || Boolean(blocked.reason) || !hasContent();
    sendButton.title = blocked.reason || "Enter または Ctrl+Enter で送信 / Shift+Enterで改行 / ↑↓で入力履歴";
    for (const node of quickActions.querySelectorAll("button")) node.disabled = blocked.busy || Boolean(blocked.reason);
  }

  function setValue(value) {
    input.value = String(value ?? "");
    autosize();
    updateState();
  }

  function rememberPrompt(text) {
    const value = String(text ?? "").trim();
    if (!value || promptHistory[promptHistory.length - 1] === value) return;
    promptHistory.push(value);
    if (promptHistory.length > PROMPT_HISTORY_LIMIT) promptHistory.shift();
  }

  function resetNavigation() {
    historyIndex = null;
    draft = input.value;
    // 送信直後などの区切りでは遅延させず書き切る（送信済みの文面が書きかけとして戻らないように）。
    persistDraft();
  }

  function recall(direction) {
    if (!promptHistory.length) return false;
    if (historyIndex == null) {
      draft = input.value;
      historyIndex = promptHistory.length;
    }
    historyIndex = Math.min(promptHistory.length, Math.max(0, historyIndex + direction));
    setValue(historyIndex === promptHistory.length ? draft : promptHistory[historyIndex]);
    input.setSelectionRange(input.value.length, input.value.length);
    status.flash(historyIndex === promptHistory.length ? "編集中の入力に戻しました" : "過去の入力を呼び戻しました", 1400);
    return true;
  }

  function renderAttachments() {
    const any = images.length > 0 || files.length > 0;
    previewBox.hidden = !any;
    previewBox.replaceChildren();
    files.forEach((file, index) => {
      const remove = el("button", { type: "button", className: "file-remove", textContent: "×" }, [], { "aria-label": `${file.name} を削除` });
      remove.addEventListener("click", () => { files.splice(index, 1); renderAttachments(); updateState(); });
      const extension = String(file.name).split(".").pop().toLowerCase();
      previewBox.append(el("div", { className: "file-chip", title: file.name }, [
        el("span", { className: "file-chip-icon", textContent: FILE_ICONS[extension] || "📎" }),
        el("span", { className: "file-chip-name", textContent: preview(file.name, 20) }),
        remove,
      ]));
    });
    images.forEach((dataUrl, index) => {
      const remove = el("button", { type: "button", className: "image-remove", textContent: "×" }, [], { "aria-label": `添付画像 ${index + 1} を削除` });
      remove.addEventListener("click", () => { images.splice(index, 1); renderAttachments(); updateState(); });
      previewBox.append(el("div", { className: "image-thumb-wrap" }, [el("img", { src: dataUrl, alt: `添付画像 ${index + 1}` }), remove]));
    });
    renderSuggestions();
  }

  function renderSuggestions() {
    const extensions = new Set(files.map(file => String(file.name || "").split(".").pop().toLowerCase()));
    const list = [];
    const add = (label, text) => { if (!list.some(item => item.label === label)) list.push({ label, text }); };
    if (extensions.has("pdf")) { add("PDFを要約", "添付したPDFを要約して"); add("本文を抽出", "添付したPDFの本文を抽出して"); }
    if (["wav", "mp3", "m4a", "flac", "ogg"].some(ext => extensions.has(ext))) { add("文字起こし", "添付した音声を文字起こしして"); add("議事録にする", "添付した音声から議事録を作って"); }
    if (["psd", "xcf", "heic", "heif", "tif", "tiff", "avif"].some(ext => extensions.has(ext))) { add("PNGに変換", "添付した画像をPNGに変換して"); add("画像情報", "添付した画像のサイズとレイヤー情報を教えて"); }
    if (["docx", "xlsx", "pptx", "odt", "ods", "odp"].some(ext => extensions.has(ext))) { add("内容を要約", "添付した文書を要約して"); add("PDFに変換", "添付した文書をPDFに変換して"); }
    if (files.length + images.length > 1) add("まとめて比較", "添付したファイルを比較して、違いと要点を整理して");
    if (images.length > 0) add("画像を説明", "添付した画像の内容を詳しく説明して");
    suggestions.replaceChildren(...list.slice(0, 5).map(item => {
      const node = el("button", { type: "button", textContent: item.label });
      node.addEventListener("click", () => { setValue(item.text); resetNavigation(); input.focus(); });
      return node;
    }));
    suggestions.hidden = list.length === 0;
  }

  function renderQuickActions() {
    const items = choices || quickItems || defaultItems;
    quickActions.replaceChildren(...items.map(action => {
      const node = el("button", { type: "button", textContent: action.label, title: action.title || action.text });
      node.addEventListener("click", () => {
        if (blocked.busy) return;
        composer.open();
        setValue(action.text);
        resetNavigation();
        form.requestSubmit();
      });
      return node;
    }));
    updateState();
  }

  function readFile(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = event => resolve(event.target.result);
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(file);
    });
  }

  async function addFile(file) {
    if (!file) return;
    const isImage = file.type.startsWith("image/");
    if (isImage && images.length >= MAX_IMAGES) { toast(`画像は最大 ${MAX_IMAGES} 枚まで`, "warn"); return; }
    if (!isImage && files.length >= MAX_FILES) { toast(`ファイルは最大 ${MAX_FILES} 件まで`, "warn"); return; }
    try {
      const dataUrl = await readFile(file);
      // 同時に貼り付けたファイルやネイティブドロップも、読込完了時の残り枠で判定する。
      const reason = composer.addLoaded({ kind: isImage ? "image" : "file", name: file.name, dataUrl });
      if (reason) { toast(reason, "warn"); return; }
      toast(isImage ? "画像を添付しました" : `${preview(file.name, 18)} を添付`, "success");
    } catch (error) {
      logApp("error", "添付の読み込みに失敗しました", `${file.name}: ${errorDetail(error)}`);
      toast(isImage ? "画像を読み込めませんでした" : "ファイルを読み込めませんでした", "error");
    }
  }

  // ── イベント ───────────────────────────────────────────────
  input.addEventListener("input", () => {
    autosize();
    updateState();
    historyIndex = null;
    draft = input.value;
    if (draftTimer) clearTimeout(draftTimer);
    draftTimer = setTimeout(persistDraft, 600); // 1打鍵ごとに書かず、入力が止まってからまとめて保存する
    if (!blocked.busy) {
      if (input.value.length > 100) status.set(`${input.value.length}文字`, "count");
      else status.clear("count");
    }
  });
  input.addEventListener("keydown", event => {
    // IME 変換中のキーをアプリの操作に使わない（確定の Enter で送信、候補選択の ↑↓ で履歴、取消の Esc で閉じる）。
    if (isImeComposing(event)) return;
    if (event.key === "Escape") {
      if (composer.onEscape?.()) { event.preventDefault(); event.stopPropagation(); return; }
      event.stopPropagation();
      composer.close();
      return;
    }
    const plain = !event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey;
    if (plain && event.key === "ArrowUp" && input.selectionStart === 0 && input.selectionEnd === 0 && recall(-1)) event.preventDefault();
    if (plain && event.key === "ArrowDown" && input.selectionStart === input.value.length && input.selectionEnd === input.value.length && recall(1)) event.preventDefault();
    if (event.key === "Enter" && (!event.shiftKey || event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      form.requestSubmit();
    }
  });
  input.addEventListener("paste", event => {
    for (const item of event.clipboardData?.items || []) {
      if (!item.type.startsWith("image/")) continue;
      event.preventDefault();
      addFile(item.getAsFile());
    }
  });
  form.addEventListener("dragover", event => {
    if ([...event.dataTransfer.items].some(item => item.kind === "file")) {
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
      form.classList.add("drag-over");
    }
  });
  form.addEventListener("dragleave", () => form.classList.remove("drag-over"));
  form.addEventListener("drop", event => {
    form.classList.remove("drag-over");
    if (![...event.dataTransfer.items].some(item => item.kind === "file")) return;
    event.preventDefault();
    for (const file of event.dataTransfer.files) addFile(file);
  });
  form.addEventListener("submit", event => {
    event.preventDefault();
    onSubmit();
  });

  const composer = {
    input,
    form,
    get value() { return input.value; },
    get images() { return images; },
    get files() { return files; },
    hasContent,
    setValue,
    focus() { input.focus(); },
    onSubmit(callback) { onSubmit = callback; },
    onOpenChange(callback) { onOpenChange = callback; },
    onEscape: null,
    get isOpen() { return !form.hidden; },
    setPersistent(value) {
      const next = Boolean(value);
      if (persistent === next) return;
      persistent = next;
      if (persistent) {
        form.hidden = false;
        autosize();
        updateState();
        onOpenChange(true);
      } else {
        composer.close();
      }
    },
    open() {
      if (blocked.busy) return;
      form.hidden = false;
      // 前回の書きかけを戻す（空のときだけ入れるので、いま打っている内容は上書きしない）。
      if (!input.value && draft) setValue(draft);
      autosize();
      updateState();
      input.focus();
      onOpenChange(true);
    },
    close() {
      draft = input.value;
      persistDraft();
      if (persistent) {
        input.blur();
        return;
      }
      form.hidden = true;
      updateState();
      onOpenChange(false);
    },
    /** 送信する内容を取り出して入力欄を空にする。 */
    take() {
      const taken = { text: input.value.trim(), images: [...images], files: [...files] };
      rememberPrompt(taken.text);
      images = [];
      files = [];
      renderAttachments();
      setValue("");
      resetNavigation();
      return taken;
    },
    /** 直前の送信（文面と添付）を入力欄へ戻す（もう一度試す・再生成）。 */
    restore({ text, images: lastImages, files: lastFiles }) {
      composer.open();
      images = [...(lastImages || [])];
      files = [...(lastFiles || [])];
      renderAttachments();
      setValue(text || "");
      input.focus();
    },
    /** 入力欄へ文字を足す（音声入力・ドロップしたテキスト）。 */
    insert(text) {
      composer.open();
      setValue(input.value.trim() ? `${input.value.trimEnd()} ${text}` : text);
      resetNavigation();
      input.focus();
    },
    reuse(text) {
      composer.open();
      setValue(text);
      resetNavigation();
      input.focus();
      status.flash("入力欄へ戻しました", 1400);
    },
    resetNavigation,
    addFile,
    /** ネイティブのドロップで読み込んだ添付を足す。上限を超えた分は理由を返す。 */
    addLoaded(info) {
      if (info.kind === "image") {
        if (images.length >= MAX_IMAGES) return `画像は最大 ${MAX_IMAGES} 枚までです`;
        images.push(info.dataUrl);
      } else {
        if (files.length >= MAX_FILES) return `ファイルは最大 ${MAX_FILES} 件までです`;
        files.push({ name: info.name, dataUrl: info.dataUrl, path: info.path });
      }
      renderAttachments();
      updateState();
      return "";
    },
    hydratePrompts(list) {
      promptHistory.length = 0;
      historyIndex = null;
      for (const item of list) if (item.role === "user") rememberPrompt(item.content);
    },
    rememberPrompt,
    clearAfterReset() {
      promptHistory.length = 0;
      historyIndex = null;
      choices = null;
      renderQuickActions();
    },
    setBlocked(next) {
      blocked = { ...blocked, ...next };
      updateState();
    },
    /** 質問カードの選択肢。null / 空なら通常のクイック操作へ戻す。 */
    setChoices(list) {
      const items = Array.isArray(list)
        ? list.map(choice => ({ label: String(choice?.label || "").trim(), text: String(choice?.label || "").trim(), title: String(choice?.description || "").trim() })).filter(item => item.label)
        : [];
      const next = items.length ? items : null;
      if (next === null && choices === null) return;
      choices = next;
      renderQuickActions();
    },
    setQuickActions(text) {
      quickItems = quick.parseQuickActions(text);
      renderQuickActions();
    },
    /** 連携スキルに頼る既定ボタンは、必要な設定に値が入っているかを確かめてから出す。 */
    async refreshDefaultQuickActions() {
      try {
        const presence = await bridge.configPresence(quick.requiredKeys(quick.DEFAULT_QUICK_ACTIONS));
        defaultItems = quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, presence);
      } catch (error) {
        // フォールバック許可: 確認できないときは連携設定の要らないボタンだけを出す。
        logApp("warn", "クイック操作の設定状態を確認できませんでした", errorDetail(error));
        defaultItems = quick.visibleQuickActions(quick.DEFAULT_QUICK_ACTIONS, {});
      }
      renderQuickActions();
    },
    persistDraft() {
      draft = input.value;
      persistDraft();
    },
  };
  renderQuickActions();
  return composer;
}
