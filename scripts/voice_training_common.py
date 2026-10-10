"""音声学習スクリプトで共有するパス検証と原子的配置。"""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path, PureWindowsPath

WINDOWS_RESERVED_NAMES = {
    "CON", "PRN", "AUX", "NUL",
    *(f"COM{number}" for number in range(1, 10)),
    *(f"LPT{number}" for number in range(1, 10)),
}


def windows_name(value: str, label: str) -> str:
    """Windowsで安全に使える単一のファイル名要素だけを返す。"""
    if not isinstance(value, str) or not value or value != value.strip():
        raise SystemExit(f"{label}が不正です: {value!r}")
    path = PureWindowsPath(value.replace("/", "\\"))
    invalid = '<>:"/\\|?*'
    if (
        path.is_absolute()
        or path.drive
        or path.root
        or len(path.parts) != 1
        or value in (".", "..")
        or value.endswith((".", " "))
        or any(character in invalid or ord(character) < 32 for character in value)
        or value.split(".", 1)[0].upper() in WINDOWS_RESERVED_NAMES
    ):
        raise SystemExit(f"{label}は区切りや親参照を含まない安全な名前にしてください: {value}")
    return value


def dataset_name(value: str) -> str:
    """Data直下で使える単一のWindowsフォルダー名だけを返す。"""
    return windows_name(value, "データセット名")


def output_prefix(value: str) -> str:
    """切り出しWAVとレポートへ使える単一の接頭辞だけを返す。"""
    return windows_name(value, "出力接頭辞")


def relative_wav_parts(file_name: str) -> tuple[str, ...]:
    """WindowsのWAV相対パスを検証し、管理ルートへ安全に結合できる部品を返す。"""
    if not isinstance(file_name, str) or not file_name.strip():
        raise SystemExit("WAVの相対パスが空です")
    path = PureWindowsPath(file_name.replace("/", "\\"))
    if path.is_absolute() or path.drive or path.root or any(part in ("", ".", "..") for part in path.parts):
        raise SystemExit(f"WAVの相対パスが管理フォルダー外を指しています: {file_name}")
    if path.suffix.casefold() != ".wav":
        raise SystemExit(f"学習断片はWAVだけを指定してください: {file_name}")
    return tuple(path.parts)


def wav_path(root: Path, file_name: str) -> Path:
    """検証済み相対パスをroot配下の実体へ解決する。"""
    resolved_root = root.resolve()
    candidate = resolved_root.joinpath(*relative_wav_parts(file_name)).resolve()
    try:
        candidate.relative_to(resolved_root)
    except ValueError as error:
        raise SystemExit(f"WAVの相対パスが管理フォルダー外を指しています: {file_name}") from error
    return candidate


def unique_wav_names(file_names: list[str], label: str) -> None:
    """Windows上で同じ実体になるWAV名の重複を拒否する。"""
    seen: dict[str, str] = {}
    for file_name in file_names:
        parts = relative_wav_parts(file_name)
        key = "\\".join(part.casefold() for part in parts)
        if key in seen:
            raise SystemExit(f"候補一覧に同名WAVが重複しています: {label} {seen[key]} / {file_name}")
        seen[key] = file_name


# 同じOS操作だけを再試行する。恒久的な権限不足・長い保持は元の例外を返す。
ATOMIC_MOVE_DELAYS = (0.025, 0.05, 0.1, 0.2, 0.4)


def atomic_move(source: Path, destination: Path, *, replace: bool = True, cleanup_file: bool = False):
    """Windows共有違反だけ有限待機し、失敗stageは保持する。"""
    operation = os.replace if replace else os.rename
    try:
        for attempt in range(len(ATOMIC_MOVE_DELAYS) + 1):
            try:
                operation(source, destination)
                return
            except OSError as error:
                if sys.platform != "win32" or getattr(error, "winerror", None) not in (5, 32, 33) or attempt == len(ATOMIC_MOVE_DELAYS):
                    raise
                time.sleep(ATOMIC_MOVE_DELAYS[attempt])
    except BaseException as error:
        if cleanup_file:
            try:
                source.unlink(missing_ok=True)
            except OSError as cleanup:
                raise RuntimeError(f"原子的配置失敗後の一時ファイルを回収できません: {cleanup}") from error
        raise
