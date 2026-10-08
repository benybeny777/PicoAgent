#!/usr/bin/env python3
"""差分から、過去のデグレ台帳で読むべき節と注意する追加行を挙げる。

使い方:
  python3 regression_hints.py                 # origin/main...HEAD と未コミット変更
  python3 regression_hints.py <範囲>           # 例: origin/main...HEAD、abc123..def456
  python3 regression_hints.py --pr-base main   # origin/<base>...HEAD

判定ではなく読む順番の手掛かりを出すだけで、該当なしでも台帳の共通観点は省略しない。
"""

from __future__ import annotations

import re
import subprocess
import sys
from collections import defaultdict

SECTIONS = {
    "D1": "データ消失（全体を読んで全体を書き戻す状態）",
    "D2": "無言の失敗・意味の取り違え",
    "D3": "起動シーケンス・半端表示",
    "D4": "外部プロセス・リソース回収",
    "D5": "OS・feature・cfg別の未検査",
    "D6": "検査の形骸化",
    "D7": "人格プロンプト・内部資料の混入",
    "D8": "意図ルーティング・先回り実行の誤爆",
    "D9": "ウィンドウ位置・寸法の保存と復元",
    "D10": "非同期の競合・世代・取りこぼし",
    "D11": "設定反映・再初期化",
    "D12": "配布・ビルド種別の混入",
    "D13": "文書・仕様のずれ",
    "D14": "レビュー修正そのものによる退行",
    "D15": "日本語・文字列",
    "D16": "外部入力の上限・境界",
    "D17": "見た目: 操作部品の切れ・遮蔽・はみ出し",
    "D18": "見た目: キャラの構図・切れ・位置",
    "D19": "見た目: キャラ素材・顔パーツ・描画品質",
    "D20": "見た目: 配色・テーマ・文字",
}

# パスの正規表現 → 節
PATH_RULES = [
    (r"src-tauri/src/(history|store|memory|usage|relationship|migration|durable_jobs|startup_cleanup)\.rs", ["D1"]),
    (r"src-tauri/src/config\.rs", ["D1", "D11"]),
    (r"ui/(memory-settings|settings-schedules|composer-draft)\.js|ui/scenes/|layout-editor", ["D1"]),
    (r"src-tauri/src/(alarm|daily|scheduler)\.rs|ui/alarm-runtime\.js|ui/task-run\.js", ["D2", "D10"]),
    (r"ui/app/(toast|notice|status)\.js", ["D2"]),
    (r"src-tauri/src/(boot_screen|lib|window_surface)\.rs|ui/app/(boot|main|engine)\.js|ui/index\.html", ["D3"]),
    (r"src-tauri/src/(engine|engine_backend|aivis|components|generation_backend)\.rs", ["D3", "D4", "D10"]),
    (r"src-tauri/src/tools/process\.rs|skills/(run_command|code_runner|command_job|browser_control|whisper_transcribe|meeting_recorder|document_convert)/", ["D4"]),
    (r"src-tauri/src/tools/http\.rs|src-tauri/src/(api_server|mcp)/?|src-tauri/src/api_server\.rs", ["D4", "D16"]),
    (r"(^|/)tests?/|tools/tests/|tools/screenshots/|mod guard|guard_scan\.rs", ["D6"]),
    (r"src-tauri/src/agent(\.rs|/)|src-tauri/src/(topic_guard|message|history)\.rs|skills/[^/]+/prompts/", ["D7"]),
    (r"src-tauri/src/(layout|scene|stream_profile|hotkey)\.rs|ui/app/(stage|window-chrome|edit)\.js", ["D9", "D17", "D18"]),
    (r"ui/[^/]*\.(css|html)$|ui/chat/|ui/app/(context-menu|onboarding|notice|quick-actions)\.js|ui/components/|ui/settings", ["D17", "D20"]),
    (r"ui/themes/|ui/[^/]*\.css$", ["D20"]),
    (r"ui/character/|ui/app/motion\.js|ui/mesh-rig\.js|ui/mmd-renderer\.js|character-preview|character_framing|characters/[^/]+/character\.json", ["D18", "D19"]),
    (r"^characters/|^character-sources/|scripts/build-character", ["D19"]),
    (r"src-tauri/src/(commands|settings_memory|secret)\.rs|ui/settings[^/]*", ["D11"]),
    (r"^xtask/|^distribution/|Cargo\.(toml|lock)$|tauri\.conf\.json|\.bat$|^scripts/", ["D12"]),
    (r"\.md$", ["D13"]),
    (r"ui/chat/|ui/app/(mic|voice|lipsync)\.js|src-tauri/src/text\.rs", ["D15", "D10"]),
    (r"src-tauri/src/permissions\.rs|skills/(read_file|edit_file|pdf_tools|image_tools|trash_file|workspace_files)/", ["D16"]),
]

# 追加行の正規表現 → (節, 注意)
LINE_RULES = [
    (r"catch\s*(\(\s*\w*\s*\))?\s*\{\s*\}", "D2", "空のcatch"),
    (r"(\?\?|\|\|)\s*(\[\]|\{\})", "D1", "空の既定値。読めない値を空で保存へ流さないか"),
    (r"JSON\.parse|localStorage\.(get|set)Item", "D1", "破損時の扱いと書き戻し"),
    (r"\bcontinue\s*;|Err\(_\)\s*=>|\.ok\(\)|unwrap_or_default\(\)|unwrap_or\(", "D2", "失敗を黙って飛ばしていないか"),
    (r"remove_file|remove_dir_all|fs\.rm|unlink|rmSync", "D1", "削除対象の範囲"),
    (r"#\[cfg\(|cfg!\(|target_os", "D5", "他OSでの検査とimport"),
    (r"feature\s*=", "D5", "feature有無の両方でコンパイルするか"),
    (r"setInterval|setTimeout|addEventListener|createObjectURL|new Audio|requestAnimationFrame", "D4", "全経路の解放"),
    (r"spawn|Child|kill\(|wait\(", "D4", "孫プロセスと回収経路"),
    (r"\.chat(_stream)?\(|provider\.chat", "D7", "公開経路のガード"),
    (r"intent_(keywords|priority|invocation)", "D8", "当たる例と当たらない例"),
    (r"generation|revision|epoch|世代|AbortController|Notify", "D10", "古い結果・中断の取りこぼし"),
    (r"\.slice\(|\.substring\(|\.substr\(", "D15", "UTF-16で絵文字を割らないか"),
    (r"keydown|keyup|key\s*===\s*['\"](Enter|Escape|Arrow)", "D15", "IME変換中のキー"),
    (r"(timeout|TIMEOUT|_MS|_SECS)\b", "D3", "待ち時間の変更が案内や回収を遅らせないか"),
    (r"\bMAX_|max_bytes|limit", "D16", "上限超過の扱い"),
    (r"try\s*\{", "D3", "独立した処理を1つのtryにまとめていないか"),
]

NO_LINE_RULES = r"(^|/)tests?/|tools/tests/|tools/screenshots/|^\.agents/|^\.claude/|\.md$"


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True, encoding="utf-8").stdout


def parse_args(argv: list[str]) -> list[str]:
    if not argv:
        return []
    if argv[0] == "--pr-base" and len(argv) == 2:
        return [f"origin/{argv[1]}...HEAD"]
    return argv[:1]


def collect(range_args: list[str]) -> tuple[list[str], dict[str, list[str]]]:
    diffs = []
    if range_args:
        diffs.append(range_args)
    else:
        diffs.append(["origin/main...HEAD"])
        diffs.append(["HEAD"])  # 未コミット変更
    files: list[str] = []
    added: dict[str, list[str]] = defaultdict(list)
    for spec in diffs:
        for name in git("diff", "--name-only", *spec).splitlines():
            if name and name not in files:
                files.append(name)
        current = None
        for line in git("diff", "-U0", *spec).splitlines():
            if line.startswith("+++ "):
                current = line[6:] if line.startswith("+++ b/") else None
            elif current and line.startswith("+") and not line.startswith("+++"):
                added[current].append(line[1:])
    if not range_args:
        # 未追跡の新規ファイルは git diff に出ないため、全行を追加行として扱う
        for name in git("ls-files", "--others", "--exclude-standard").splitlines():
            if name and name not in files:
                files.append(name)
                try:
                    with open(name, encoding="utf-8") as handle:
                        added[name].extend(handle.read().splitlines())
                except (OSError, UnicodeDecodeError):
                    pass  # 画像などの非テキストはパスの節だけで扱う
    return files, added


def main() -> int:
    # Windowsの既定コンソール（cp932）では差分中の絵文字を出せず落ちるため、出せない文字は置き換える
    sys.stdout.reconfigure(errors="replace")
    range_args = parse_args(sys.argv[1:])
    files, added = collect(range_args)
    if not files:
        print("差分がありません。範囲を指定してください。")
        return 1

    by_section: dict[str, set[str]] = defaultdict(set)
    for path in files:
        for pattern, sections in PATH_RULES:
            if re.search(pattern, path):
                for section in sections:
                    by_section[section].add(path)

    notes: dict[str, list[str]] = defaultdict(list)
    for path, lines in added.items():
        # テストと文書は行の手掛かりを出すと雑音になるため、パスの節だけにする
        if re.search(NO_LINE_RULES, path):
            continue
        for pattern, section, note in LINE_RULES:
            hits = [line.strip() for line in lines if re.search(pattern, line)]
            if hits:
                by_section[section].add(path)
                notes[section].append(f"{path}: {note}（{len(hits)}行。例: {hits[0][:100]}）")

    # 修正を含む差分は常にD14、コードを変えたら文書のずれも見る
    by_section["D14"].add("（全変更: 変える行の導入理由を確かめる）")
    if any(not p.endswith(".md") for p in files):
        by_section["D13"].add("（コード変更に対する担当文書の同期）")

    print(f"変更ファイル: {len(files)}件")
    for section in sorted(by_section, key=lambda s: int(s[1:])):
        paths = sorted(by_section[section])
        print(f"\n## {section} {SECTIONS[section]}")
        for path in paths[:12]:
            print(f"  - {path}")
        if len(paths) > 12:
            print(f"  - ほか{len(paths) - 12}件")
        for note in notes.get(section, [])[:8]:
            print(f"  ! {note}")
    print("\n台帳: .agents/skills/review-picoagent-changes/references/regression-catalog.md")
    return 0


if __name__ == "__main__":
    sys.exit(main())
