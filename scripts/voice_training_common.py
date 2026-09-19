"""音声学習スクリプトで共有するパス検証。"""

from __future__ import annotations

from pathlib import Path, PureWindowsPath

WINDOWS_RESERVED_NAMES = {
    "CON", "PRN", "AUX", "NUL",
    *(f"COM{number}" for number in range(1, 10)),
    *(f"LPT{number}" for number in range(1, 10)),
}


def dataset_name(value: str) -> str:
    """Data直下で使える単一のWindowsフォルダー名だけを返す。"""
    if not isinstance(value, str) or not value or value != value.strip():
        raise SystemExit(f"データセット名が不正です: {value!r}")
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
        raise SystemExit(f"データセット名はData直下の安全なフォルダー名にしてください: {value}")
    return value


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
