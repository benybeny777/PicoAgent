// settings-schedules.js - 目覚ましとスケジュールの設定フォーム。
(function exposeSettingsSchedules(global) {
  "use strict";

  const WEEKDAYS = [
    ["mon", "月"], ["tue", "火"], ["wed", "水"], ["thu", "木"],
    ["fri", "金"], ["sat", "土"], ["sun", "日"],
  ];

  // 壊れた保存値を空一覧へ置き換えず、利用者が編集するまで原文を温存する。
  function parseStoredList(currentJson) {
    const raw = currentJson || "[]";
    try {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) return { items: parsed, broken: false, raw };
    } catch { /* 下の broken 扱いへ */ }
    return { items: [], broken: true, raw };
  }

  function create({ el, setAlarmEditorState, setScheduleEditorState }) {
    function brokenListNotice(label) {
      return el("div", {
        className: "hint",
        textContent: `⚠ 保存済みの${label}を読み取れませんでした。上書きで失わないよう、ここで編集して保存するまで元の内容はそのまま残します。`,
      });
    }

    function buildAlarmEditor(currentJson) {
      const stored = parseStoredList(currentJson);
      let touched = false;
      const wrap = el("div", { className: "alarm-editor" });
      if (stored.broken) wrap.append(brokenListNotice("目覚まし"));
      const listEl = el("div", { className: "alarm-list" });
      const rows = [];
      wrap.append(listEl);

      function addRow(alarm = {}) {
        const time = el("input", { type: "time", value: alarm.time || "07:00", className: "alarm-time" });
        const label = el("input", { type: "text", placeholder: "ラベル（任意）", value: alarm.label || "", className: "alarm-label" });
        const snooze = el("input", { type: "number", min: "1", max: "60", step: "1", value: String(Math.min(60, Math.max(1, Number(alarm.snooze_minutes || 5)))), className: "alarm-snooze" });
        const repeat = el("input", { type: "checkbox", checked: alarm.repeat !== false });
        const enabled = el("input", { type: "checkbox", checked: alarm.enabled !== false });
        const dayBoxes = WEEKDAYS.map(([code, jp]) => {
          const checkbox = el("input", { type: "checkbox", checked: Array.isArray(alarm.days) && alarm.days.includes(code) });
          return { code, checkbox, label: el("label", { className: "alarm-day" }, [checkbox, document.createTextNode(jp)]) };
        });
        const remove = el("button", { type: "button", className: "alarm-del", textContent: "削除" });
        const row = el("div", { className: "alarm-row" }, [
          el("div", { className: "alarm-row-top" }, [time, el("span", { className: "alarm-days" }, dayBoxes.map(day => day.label))]),
          el("div", { className: "alarm-row-bottom" }, [
            el("label", { className: "alarm-opt" }, [repeat, document.createTextNode("繰り返し")]),
            el("label", { className: "alarm-opt" }, [enabled, document.createTextNode("有効")]),
            el("label", { className: "alarm-opt" }, [snooze, document.createTextNode("分後に再通知")]),
            label,
            remove,
          ]),
        ]);
        const entry = {
          id: alarm.id || `al${Date.now()}${Math.random().toString(36).slice(2, 6)}`,
          time, label, repeat, enabled, snooze, dayBoxes, row,
        };
        remove.addEventListener("click", () => {
          listEl.removeChild(row);
          const index = rows.indexOf(entry);
          if (index >= 0) rows.splice(index, 1);
        });
        rows.push(entry);
        listEl.append(row);
      }

      for (const alarm of stored.items) addRow(alarm);
      const add = el("button", { type: "button", className: "alarm-add", textContent: "＋ 目覚ましを追加" });
      add.addEventListener("click", () => { touched = true; addRow(); });
      wrap.append(add, el("div", { className: "hint", textContent: "曜日を選ばないと毎日鳴ります。「繰り返し」OFFは一度だけ鳴って自動で無効になります。アプリ起動中のみ鳴ります。" }));

      setAlarmEditorState({
        getJson() {
          if (stored.broken && !touched) return stored.raw;
          return JSON.stringify(rows.map(row => ({
            id: row.id,
            time: row.time.value || "07:00",
            days: row.dayBoxes.filter(day => day.checkbox.checked).map(day => day.code),
            repeat: row.repeat.checked,
            label: row.label.value.trim(),
            enabled: row.enabled.checked,
            snooze_minutes: Math.min(60, Math.max(1, Number(row.snooze.value || 5))),
          })));
        },
      });
      return wrap;
    }

    function buildScheduleEditor(currentJson) {
      const stored = parseStoredList(currentJson);
      let touched = false;
      const wrap = el("div", { className: "schedule-editor" });
      if (stored.broken) wrap.append(brokenListNotice("スケジュール"));
      const listEl = el("div", { className: "schedule-list" });
      const rows = [];
      wrap.append(listEl);

      function addRow(task = {}) {
        const kind = el("select", { className: "schedule-kind" }, [
          el("option", { value: "tool", textContent: "スキル", selected: (task.kind || "tool") === "tool" }),
          el("option", { value: "command", textContent: "コマンド", selected: task.kind === "command" }),
        ]);
        const time = el("input", { type: "time", value: task.time || "09:00", className: "schedule-time" });
        const cron = el("input", { type: "text", value: task.cron || "", placeholder: "cron 任意: 0 9 * * 1", className: "schedule-cron" });
        const label = el("input", { type: "text", value: task.label || "", placeholder: "ラベル", className: "schedule-label" });
        const toolName = el("input", { type: "text", value: task.tool_name || "weather", placeholder: "スキル名 例: weather", className: "schedule-tool" });
        const toolArgs = el("textarea", { value: JSON.stringify(task.tool_args || {}, null, 2), placeholder: "{ }", className: "schedule-json" });
        const program = el("input", { type: "text", value: task.program || "git", placeholder: "program", className: "schedule-program" });
        const originalArgs = Array.isArray(task.args) ? [...task.args] : [];
        const args = el("input", { type: "text", value: originalArgs.join(" "), placeholder: "引数（空白区切り）", className: "schedule-args" });
        const argsText = args.value;
        const cwd = el("input", { type: "text", value: task.cwd || "", placeholder: "作業ディレクトリ（任意）", className: "schedule-cwd" });
        const timeout = el("input", { type: "number", min: "1", max: "120", step: "1", value: String(task.timeout_secs || ""), placeholder: "秒", className: "schedule-timeout" });
        const repeat = el("input", { type: "checkbox", checked: task.repeat !== false });
        const enabled = el("input", { type: "checkbox", checked: task.enabled !== false });
        const dayBoxes = WEEKDAYS.map(([code, jp]) => {
          const checkbox = el("input", { type: "checkbox", checked: Array.isArray(task.days) && task.days.includes(code) });
          return { code, checkbox, label: el("label", { className: "alarm-day" }, [checkbox, document.createTextNode(jp)]) };
        });
        const toolFields = el("div", { className: "schedule-tool-fields" }, [toolName, toolArgs]);
        const commandFields = el("div", { className: "schedule-command-fields" }, [program, args, cwd, timeout]);
        const target = el("div", { className: "schedule-target" }, [toolFields, commandFields]);
        const remove = el("button", { type: "button", className: "alarm-del", textContent: "削除" });
        const row = el("div", { className: "schedule-row" }, [
          el("div", { className: "schedule-row-top" }, [kind, time, cron, label]),
          el("div", { className: "alarm-days" }, dayBoxes.map(day => day.label)),
          target,
          el("div", { className: "schedule-row-bottom" }, [
            el("label", { className: "alarm-opt" }, [repeat, document.createTextNode("繰り返し")]),
            el("label", { className: "alarm-opt" }, [enabled, document.createTextNode("有効")]),
            remove,
          ]),
        ]);
        const entry = { original: { ...task }, originalArgs, argsText, id: task.id || `st${Date.now()}${Math.random().toString(36).slice(2, 6)}`, row, kind, time, cron, label, toolName, toolArgs, program, args, cwd, timeout, repeat, enabled, dayBoxes, toolFields, commandFields };
        function updateKind() {
          const isTool = kind.value === "tool";
          toolFields.hidden = !isTool;
          commandFields.hidden = isTool;
        }
        kind.addEventListener("change", updateKind);
        remove.addEventListener("click", () => {
          listEl.removeChild(row);
          const index = rows.indexOf(entry);
          if (index >= 0) rows.splice(index, 1);
        });
        updateKind();
        rows.push(entry);
        listEl.append(row);
      }

      for (const task of stored.items) addRow(task);
      const add = el("button", { type: "button", className: "alarm-add", textContent: "＋ スケジュールを追加" });
      add.addEventListener("click", () => { touched = true; addRow({ kind: "tool", tool_name: "weather", tool_args: {} }); });
      wrap.append(add, el("div", { className: "hint", textContent: "cron を空にすると時刻と曜日で実行します。スキル実行はコマンド許可なしで使えます。コマンド実行は許可ルートと allowlist の制限を受けます。" }));

      setScheduleEditorState({
        getJson() {
          if (stored.broken && !touched) return stored.raw;
          return JSON.stringify(rows.map(row => {
            let parsedArgs = {};
            try { parsedArgs = JSON.parse(row.toolArgs.value || "{}"); }
            catch {
              throw new Error(`スケジュール「${row.label.value.trim() || row.toolName.value.trim() || row.program.value.trim() || row.id}」の tool_args が JSON として読めません。`);
            }
            const { last_run_minute: _lastRun, last_checked_minute: _lastChecked, ...definition } = row.original;
            return {
              // 発火判定状態はschedulerの別状態が所有する。日付等の予約定義は維持する。
              ...definition,
              id: row.id,
              kind: row.kind.value,
              time: row.cron.value.trim() ? "" : (row.time.value || "09:00"),
              cron: row.cron.value.trim(),
              days: row.dayBoxes.filter(day => day.checkbox.checked).map(day => day.code),
              repeat: row.repeat.checked,
              label: row.label.value.trim(),
              enabled: row.enabled.checked,
              program: row.program.value.trim(),
              // 未編集の引数は配列の境界を維持する（空白入りのパスや空文字を壊さない）。
              args: row.args.value === row.argsText ? [...row.originalArgs] : row.args.value.trim().split(/\s+/).filter(Boolean),
              cwd: row.cwd.value.trim(),
              timeout_secs: row.timeout.value ? Math.min(120, Math.max(1, Number(row.timeout.value))) : null,
              tool_name: row.toolName.value.trim(),
              tool_args: parsedArgs,
            };
          }));
        },
      });
      return wrap;
    }

    return { buildAlarmEditor, buildScheduleEditor };
  }

  global.PicoSettingsSchedules = Object.freeze({ create, parseStoredList });
})(globalThis);
