// memory-settings.js - 全キャラクター共通の長期メモリを設定画面で管理する。

(function registerMemorySettings(root) {
  "use strict";

  function sortMemories(items) {
    return [...(Array.isArray(items) ? items : [])].sort((left, right) =>
      String(right?.updated_at || "").localeCompare(String(left?.updated_at || ""))
    );
  }

  function formatTimestamp(value) {
    const raw = String(value || "");
    const date = new Date(raw);
    if (!raw || Number.isNaN(date.getTime())) return raw || "日時不明";
    return date.toLocaleString("ja-JP", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function buildMemoryManager({
    api,
    section,
    el,
    markSearchable,
    refreshSearch,
    setPageStatus,
    setBusy,
    isBusy,
    logApp,
    errorDetail,
  }) {
    const manager = section("覚えたこと");
    manager.classList.add("memory-manager");
    const list = el("div", { className: "memory-list" });
    const status = el("p", {
      className: "memory-manager-status hint",
      textContent: "長期メモリを読み込み中…",
    });
    status.setAttribute("aria-live", "polite");
    const refreshButton = el("button", {
      className: "secondary memory-refresh",
      type: "button",
      textContent: "一覧を再読み込み",
    });

    function renderItems(items) {
      list.replaceChildren();
      const sorted = sortMemories(items);
      status.textContent = sorted.length > 0
        ? `${sorted.length}件を覚えています。`
        : "現在、覚えていることはありません。";

      for (const item of sorted) {
        const category = String(item?.category || "未分類");
        const text = String(item?.text || "");
        const updatedAt = String(item?.updated_at || "");
        const hits = Math.max(0, Number(item?.hits) || 0);
        const removeButton = el("button", {
          className: "memory-delete",
          type: "button",
          textContent: "この記憶を削除",
        });
        removeButton.setAttribute("aria-label", `「${text}」を長期メモリから削除`);
        const timestamp = el("time", {
          dateTime: updatedAt,
          textContent: `覚えた日時: ${formatTimestamp(updatedAt)}`,
        });
        const itemNode = markSearchable(el("article", { className: "memory-item" }, [
          el("div", { className: "memory-item-heading" }, [
            el("span", { className: "memory-category", textContent: category }),
            el("span", { className: "memory-hits", textContent: `言及 ${hits}回` }),
          ]),
          el("p", { className: "memory-text", textContent: text }),
          el("div", { className: "memory-item-footer" }, [timestamp, removeButton]),
        ]), "記憶", "長期メモリ", category, text, updatedAt, `${hits}回`);

        removeButton.addEventListener("click", async () => {
          if (isBusy()) return;
          setBusy(true);
          removeButton.disabled = true;
          status.textContent = "長期メモリを削除中…";
          try {
            const response = await api.deleteMemory(category, text, updatedAt);
            if (!itemNode.isConnected) return;
            // 応答が一覧を含まない形（想定外）のときに空一覧で描き直すと、
            // 実際は残っている記憶を「0件」と誤って見せてしまう。読み直しへ倒す。
            if (!Array.isArray(response?.items)) {
              logApp("warn", "長期メモリの削除応答に一覧が含まれていません", JSON.stringify(response ?? null));
              setPageStatus("長期メモリを1件削除しました。次の会話から参照されません。");
              await refresh();
              return;
            }
            renderItems(response.items);
            setPageStatus("長期メモリを1件削除しました。次の会話から参照されません。");
          } catch (error) {
            logApp("error", "長期メモリの削除に失敗しました", errorDetail(error));
            status.textContent = `削除に失敗しました: ${error?.message || error}`;
          } finally {
            removeButton.disabled = false;
            setBusy(false);
            refreshSearch();
          }
        });
        list.append(itemNode);
      }
      refreshSearch();
    }

    async function refresh() {
      refreshButton.disabled = true;
      status.textContent = "長期メモリを読み込み中…";
      try {
        const response = await api.listMemories();
        if (!manager.isConnected) return;
        if (!Array.isArray(response?.items)) {
          // 一覧を受け取れていないので、表示中の内容を空へ置き換えず理由だけ出す。
          logApp("warn", "長期メモリ一覧の応答に items がありません", JSON.stringify(response ?? null));
          status.textContent = "一覧を読み込めませんでした: 応答の形式が想定と違います";
          return;
        }
        renderItems(response.items);
      } catch (error) {
        if (!manager.isConnected) return;
        logApp("warn", "長期メモリ一覧の読み込みに失敗しました", errorDetail(error));
        status.textContent = `一覧を読み込めませんでした: ${error?.message || error}`;
      } finally {
        refreshButton.disabled = false;
      }
    }

    refreshButton.addEventListener("click", refresh);
    manager.append(
      el("p", {
        className: "settings-intro",
        textContent: "会話から暗黙的に覚えた好み・自己情報・継続指示です。ここにある内容は全キャラクターで共通して使われます。",
      }),
      el("div", { className: "memory-manager-toolbar" }, [status, refreshButton]),
      list,
    );
    // 呼び出し元が manager をDOMへ追加した後に読み込みを始める。
    Promise.resolve().then(refresh);
    return manager;
  }

  root.PicoMemorySettings = Object.freeze({
    buildMemoryManager,
    formatTimestamp,
    sortMemories,
  });
})(typeof window === "undefined" ? globalThis : window);
