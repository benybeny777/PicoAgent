#!/usr/bin/env python3
"""PicoAgentの外部依存候補をGit追跡ファイルから棚卸しする。"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
PACKAGE_FILES = re.compile(r"(^|/)(Cargo\.toml|Cargo\.lock|package\.json|package-lock\.json|pnpm-lock\.yaml|yarn\.lock|pyproject\.toml|uv\.lock|requirements[^/]*\.txt)$")
APP_PATHS = (
    "src-tauri/resources/",
    "src-tauri/src/aivis.rs",
    "src-tauri/src/components.rs",
    "xtask/src/bundled.rs",
    "xtask/src/office.rs",
    "xtask/src/tools.rs",
    "src-tauri/src/engine.rs",
    "src-tauri/src/providers/local.rs",
    "ui/vendor/",
)
DEPENDENCY_PATTERN = re.compile(
    r"(?i)(https?://|\b(?:[A-Z][A-Z0-9_]*(?:VERSION|REVISION)|targetVersion)\b|"
    r"releases/(?:download|latest|tags)|resolve/main|sha-?256|integrity)"
)


def tracked_files() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files"], cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8"
    )
    return [line.replace("\\", "/") for line in result.stdout.splitlines() if line]


def show_packages(files: list[str]) -> None:
    print("[packages] マニフェストとロック")
    matches = [path for path in files if PACKAGE_FILES.search(path)]
    for path in matches:
        print(path)
    if not matches:
        print("(なし)")


def app_path_matches(relative: str, mode: str) -> bool:
    if mode in {"all", "apps"}:
        return True
    markers = {
        "aivis": ("aivis.rs", "component-manifest.json"),
        "whisper": ("components.rs", "component-manifest.json", "bundled.rs"),
        "engine": ("engine.rs", "bundled.rs"),
        "models": ("providers/local.rs", "engine.rs"),
        "live2d": ("ui/vendor/live2d/", "bundled.rs"),
        "tha3": ("tha3", "bundled.rs"),
        "office": ("office.rs", "component-manifest.json"),
        "tools": ("tools.rs", "ui/vendor/", "component-manifest.json"),
    }
    return any(marker in relative for marker in markers[mode])


def show_apps(files: list[str], mode: str) -> None:
    print(f"[{mode}] 固定版・取得元・検証値の候補")
    for relative in files:
        if not relative.startswith(APP_PATHS) or not app_path_matches(relative, mode):
            continue
        path = ROOT / relative
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (UnicodeDecodeError, OSError):
            continue
        for number, line in enumerate(lines, 1):
            if DEPENDENCY_PATTERN.search(line):
                compact = line.strip()
                if len(compact) > 300:
                    compact = f"{compact[:297]}..."
                print(f"{relative}:{number}:{compact}")
    bundled = ROOT / "src-tauri" / "resources" / "bundled"
    if bundled.is_dir():
        print("[apps] Git管理外を含むローカル同梱実体")
        entries: list[Path] = []
        for child in sorted(bundled.iterdir()):
            if not child.is_dir():
                continue
            entries.append(child)
            if child.name == "tools":
                entries.extend(sorted(path for path in child.iterdir() if path.is_dir()))
        for path in entries:
            if app_path_matches(path.relative_to(ROOT).as_posix(), mode):
                executable_count = sum(
                    1
                    for candidate in path.rglob("*")
                    if candidate.is_file() and candidate.suffix.lower() in {".exe", ".dylib", ".so"}
                )
                markers = sorted(
                    candidate.relative_to(ROOT).as_posix()
                    for candidate in path.rglob("*")
                    if candidate.is_file()
                    and candidate.name.lower()
                    in {"version", "version.txt", "version.ini", ".runtime-version", ".tha3-revision"}
                )
                print(f"{path.relative_to(ROOT).as_posix()} (実行ファイル {executable_count}件)")
                for marker in markers[:5]:
                    print(f"  marker: {marker}")


def main() -> int:
    mode = sys.argv[1].lower() if len(sys.argv) > 1 else "all"
    package_modes = {"all", "packages", "cargo", "web", "python"}
    app_modes = {"all", "apps", "aivis", "whisper", "engine", "models", "live2d", "tha3", "office", "tools"}
    if mode not in package_modes | app_modes:
        print(f"未対応の対象です: {mode}", file=sys.stderr)
        return 2
    files = tracked_files()
    if mode in package_modes:
        show_packages(files)
    if mode in app_modes:
        show_apps(files, mode)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
