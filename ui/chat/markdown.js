// markdown.js - 回答の Markdown を安全な DOM へ組み立てる（innerHTML を使わない）。
//
// ストリーミング中は部分テキストで繰り返し呼ばれるので、閉じていない記法は無視して描く。

import * as bridge from "../app/bridge.js";
import { toast } from "../app/toast.js";
import { copyText, el, errorDetail, logApp } from "../app/util.js";
import { parseWeatherBlock } from "./weather-card.js";

/** ローカルサーバー（ComfyUI 等）の http:// 画像 URL は安全と判定する。外部 http:// は除く。 */
function isSafeImageUrl(url) {
  return url.startsWith("https://") || /^http:\/\/(localhost|127\.0\.0\.1)(:\d+)?\//.test(url);
}

/** PicoAgent が生成物を保存する管理領域（generated-media 直下）のローカルファイルか。 */
function isGeneratedMediaPath(path) {
  return /^(?:[A-Za-z]:[\\/]|\/)[^\n]*[\\/]generated-media[\\/][^\\/\n]+$/.test(path);
}

export function openExternalUrl(url) {
  return bridge.openExternalUrl(url).catch(error => {
    logApp("error", "リンクを開けませんでした", errorDetail(error));
    toast("既定ブラウザでリンクを開けませんでした", "error");
  });
}

function openGeneratedMedia(path) {
  return bridge.openGeneratedMedia(path).catch(error => {
    logApp("error", "生成ファイルを開けませんでした", errorDetail(error));
    toast("生成ファイルを開けませんでした", "error");
  });
}

function externalLink(url, text) {
  const anchor = el("a", { href: url, textContent: text, rel: "noopener noreferrer" });
  anchor.addEventListener("click", event => {
    event.preventDefault();
    openExternalUrl(url);
  });
  return anchor;
}

function mediaLink(path, text) {
  const anchor = el("a", { href: "#", textContent: text });
  anchor.dataset.localPath = path;
  anchor.addEventListener("click", event => {
    event.preventDefault();
    openGeneratedMedia(path);
  });
  return anchor;
}

function image(src, alt, onOpen) {
  const node = el("img", { src, alt, className: "md-image", title: "クリックして開く" });
  node.addEventListener("click", onOpen);
  return node;
}

/** インライン記法（太字・斜体・コード・画像・リンク・裸URL）。 */
function appendInline(parent, raw) {
  const pattern = /\*\*([^*\n]+?)\*\*|\*([^*\n]+?)\*|`([^`\n]+)`|!\[([^\]]*)\]\((https?:\/\/[^\s)]+|(?:[A-Za-z]:[\\/]|\/)[^)\n]+)\)|\[([^\]\n]+)\]\((https?:\/\/[^\s)]+|(?:[A-Za-z]:[\\/]|\/)[^)\n]+)\)|(https?:\/\/[^\s<>,)[\]]+)/g;
  let offset = 0;
  for (const match of raw.matchAll(pattern)) {
    if (match.index > offset) parent.append(raw.slice(offset, match.index));
    if (match[1] !== undefined) parent.append(el("strong", { textContent: match[1] }));
    else if (match[2] !== undefined) parent.append(el("em", { textContent: match[2] }));
    else if (match[3] !== undefined) parent.append(el("code", { textContent: match[3] }));
    else if (match[4] !== undefined) {
      const target = match[5];
      if (isGeneratedMediaPath(target) && /\.(png|jpe?g|webp)$/i.test(target)) {
        parent.append(image(bridge.assetUrl(target), match[4] || "生成画像", () => openGeneratedMedia(target)));
      } else if (isGeneratedMediaPath(target)) {
        parent.append(mediaLink(target, match[4] || "生成ファイルを開く"));
      } else if (isSafeImageUrl(target)) {
        parent.append(image(target, match[4] || "画像", () => openExternalUrl(target)));
      } else if (/^https?:\/\//.test(target)) {
        parent.append(externalLink(target, match[4] || target));
      } else {
        parent.append(match[0]);
      }
    } else if (match[6] !== undefined) {
      if (isGeneratedMediaPath(match[7])) parent.append(mediaLink(match[7], match[6]));
      else if (/^https?:\/\//.test(match[7])) parent.append(externalLink(match[7], match[6]));
      else parent.append(match[0]);
    } else {
      parent.append(externalLink(match[0], match[0]));
    }
    offset = match.index + match[0].length;
  }
  if (offset < raw.length) parent.append(raw.slice(offset));
}

async function copyCode(button, text) {
  if (button.disabled) return;
  button.disabled = true;
  try {
    button.textContent = (await copyText(text)) ? "コピーしました" : "コピーできませんでした";
  } catch (error) {
    logApp("warn", "コードのコピーに失敗しました", errorDetail(error));
    button.textContent = "コピーできませんでした";
  } finally {
    setTimeout(() => { button.textContent = "コピー"; button.disabled = false; }, 1500);
  }
}

const APPROVAL_MARKER = "このコマンドはまだ許可されていません";

/** コマンド実行の許可待ちか（コードブロック・引用の中の一致は除く）。 */
function hasApproval(source) {
  let inFence = false;
  for (const line of source.split("\n")) {
    if (line.startsWith("```")) { inFence = !inFence; continue; }
    if (!inFence && !line.startsWith(">") && line.includes(APPROVAL_MARKER)) return true;
  }
  return false;
}

function approvalButtons(onApproval) {
  // キャンセル → 常に許可 → 今回のみ実行 の順。主ボタンは右端。
  const row = el("div", { className: "approval-actions" });
  for (const [label, className, value] of [["キャンセル", "approval-cancel", "キャンセル"], ["常に許可", "approval-persist", "永続許可"], ["今回のみ実行", "approval-ok primary", "許可"]]) {
    const node = el("button", { type: "button", className, textContent: label });
    node.addEventListener("click", () => onApproval?.(value));
    row.append(node);
  }
  return row;
}

/** Markdown をブロック単位でパースして `container` へ描く。 */
export function renderMarkdown(container, source, { onApproval } = {}) {
  container.replaceChildren();
  const wrap = el("div", { className: "md" });
  const text = String(source ?? "");
  const lines = text.split("\n");
  let index = 0;
  const isBlockStart = line => /^#{1,3}\s/.test(line) || /^[-*+]\s/.test(line) || /^\d+\.\s/.test(line)
    || /^```/.test(line) || /^>\s?/.test(line) || /^(---+|\*\*\*+|___+)$/.test(line.trim()) || /^\s*\|.+\|\s*$/.test(line);
  while (index < lines.length) {
    const line = lines[index];
    const weather = parseWeatherBlock(lines, index);
    if (weather) {
      // 直前の空行の改行は落とす（カード自身の余白と重なって間が空きすぎる）。
      if (wrap.lastChild?.tagName === "BR") wrap.lastChild.remove();
      wrap.append(weather.node);
      index = weather.next;
      continue;
    }
    const fence = line.match(/^```(\w*)/);
    if (fence) {
      index += 1;
      const code = [];
      while (index < lines.length && !lines[index].startsWith("```")) code.push(lines[index++]);
      if (index < lines.length) index += 1;
      const codeNode = el("code", { textContent: code.join("\n") });
      const copy = el("button", { type: "button", className: "copy-code", textContent: "コピー" });
      copy.addEventListener("click", () => copyCode(copy, codeNode.textContent || ""));
      const pre = el("pre", {}, [copy, codeNode]);
      if (fence[1]) pre.dataset.lang = fence[1];
      wrap.append(pre);
      continue;
    }
    const heading = line.match(/^(#{1,3})\s(.+)/);
    if (heading) {
      const node = el(`h${heading[1].length}`);
      appendInline(node, heading[2]);
      wrap.append(node);
      index += 1;
      continue;
    }
    if (/^(---+|\*\*\*+|___+)$/.test(line.trim())) { wrap.append(el("hr")); index += 1; continue; }
    if (/^>\s?/.test(line)) {
      const quote = el("blockquote");
      const quoted = [];
      while (index < lines.length && /^>\s?/.test(lines[index])) quoted.push(lines[index++].replace(/^>\s?/, ""));
      appendInline(quote, quoted.join("\n"));
      wrap.append(quote);
      continue;
    }
    if (/^[-*+]\s/.test(line) || /^\d+\.\s/.test(line)) {
      const ordered = /^\d+\.\s/.test(line);
      const list = el(ordered ? "ol" : "ul");
      const itemPattern = ordered ? /^\d+\.\s/ : /^[-*+]\s/;
      while (index < lines.length && itemPattern.test(lines[index])) {
        const item = el("li");
        appendInline(item, lines[index++].replace(ordered ? /^\d+\.\s+/ : /^[-*+]\s+/, ""));
        list.append(item);
      }
      wrap.append(list);
      continue;
    }
    if (/^\s*\|.+\|\s*$/.test(line) && index + 1 < lines.length
      && /^\s*\|?[\s:|-]+\|?\s*$/.test(lines[index + 1]) && lines[index + 1].includes("-")) {
      const cells = row => row.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map(cell => cell.trim());
      const header = cells(line);
      index += 2;
      const rows = [];
      while (index < lines.length && /^\s*\|.+\|\s*$/.test(lines[index])) rows.push(cells(lines[index++]));
      const headRow = el("tr");
      for (const cell of header) { const th = el("th"); appendInline(th, cell); headRow.append(th); }
      const body = el("tbody");
      for (const row of rows) {
        const tr = el("tr");
        for (let column = 0; column < header.length; column += 1) { const td = el("td"); appendInline(td, row[column] ?? ""); tr.append(td); }
        body.append(tr);
      }
      wrap.append(el("div", { className: "md-table-wrap" }, [el("table", {}, [el("thead", {}, [headRow]), body])]));
      continue;
    }
    if (line.trim() === "") { wrap.append(el("br")); index += 1; continue; }
    const paragraph = [];
    while (index < lines.length && lines[index].trim() !== "" && !isBlockStart(lines[index])) paragraph.push(lines[index++]);
    if (paragraph.length) {
      const node = el("p");
      paragraph.forEach((part, position) => {
        if (position) node.append(el("br"));
        appendInline(node, part);
      });
      wrap.append(node);
    } else {
      index += 1;
    }
  }
  if (hasApproval(text)) {
    // 許可待ちの本文から「コマンド/作業ディレクトリ/理由」を取り出せたら、許可ダイアログ風のカードにする。
    const command = text.match(/^コマンド:\s?(.+)$/m)?.[1];
    const cwd = text.match(/^作業ディレクトリ:\s?(.+)$/m)?.[1];
    const reason = text.match(/^理由:\s?(.+)$/m)?.[1];
    if (command) {
      const meta = el("div", { className: "approval-meta" }, [
        cwd ? el("div", { textContent: `📁 ${cwd}` }) : null,
        reason && reason !== "(理由未記載)" ? el("div", { textContent: `💬 ${reason}` }) : null,
      ]);
      wrap.replaceChildren(el("div", { className: "approval-card" }, [
        el("div", { className: "approval-card-title", textContent: "🛡️ コマンドの実行許可" }),
        el("p", { className: "approval-card-desc", textContent: "次のコマンドを実行してもいい？" }),
        el("div", { className: "approval-cmd", textContent: `$ ${command}` }),
        meta.childElementCount ? meta : null,
        approvalButtons(onApproval),
      ]));
    } else {
      wrap.append(approvalButtons(onApproval));
    }
  }
  container.append(wrap);
}

/** 字幕用に Markdown を plain text へ（URL・リンク・コード記法を途中で分けたり読み上げたりしない）。 */
export function plainText(source, sanitize) {
  return sanitize(source)
    .replace(/^\s*```[^\n]*$/gm, "")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/^\s{0,3}(?:#{1,6}\s+|>\s?|[-*+]\s+|\d+[.)]\s+)/gm, "")
    .replace(/[*_~]{1,3}/g, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
