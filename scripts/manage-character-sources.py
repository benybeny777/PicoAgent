#!/usr/bin/env python3
# manage-character-sources.py - 制作正本PNGを内容ハッシュで格納・展開する。

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil


ROOT = Path(__file__).resolve().parents[1]
STORE = ROOT / "character-sources"
CHARACTER_IDS = ("mugi",)


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def compact(input_root: Path) -> None:
    input_root = input_root.resolve()
    files: dict[str, dict[str, str]] = {}
    unique: dict[str, Path] = {}
    for character_id in CHARACTER_IDS:
        character_root = input_root / character_id
        if not character_root.is_dir():
            raise FileNotFoundError(f"制作正本がありません: {character_root}")
        mappings: dict[str, str] = {}
        for path in sorted(character_root.rglob("*.png")):
            relative = path.relative_to(character_root).as_posix()
            file_hash = digest(path)
            mappings[relative] = f"blobs/{file_hash}.png"
            unique.setdefault(file_hash, path)
        if not mappings:
            raise ValueError(f"PNGがありません: {character_root}")
        files[character_id] = mappings

    staging = ROOT / "temp" / "character-sources-compact"
    if staging.exists():
        shutil.rmtree(staging)
    blobs = staging / "blobs"
    blobs.mkdir(parents=True)
    for file_hash, source in unique.items():
        shutil.copy2(source, blobs / f"{file_hash}.png")
    write_json(staging / "index.json", {"version": 1, "characters": files})
    read_index(staging)

    target_blobs = STORE / "blobs"
    if target_blobs.exists():
        shutil.rmtree(target_blobs)
    shutil.move(str(blobs), target_blobs)
    shutil.copy2(staging / "index.json", STORE / "index.json")
    for character_id in CHARACTER_IDS:
        legacy = STORE / character_id
        if legacy.exists():
            shutil.rmtree(legacy)
    print(f"[character-sources] {sum(len(value) for value in files.values())}配置を{len(unique)}画像へ共有化しました")


def read_index(root: Path = STORE) -> dict:
    index_path = root / "index.json"
    data = json.loads(index_path.read_text(encoding="utf-8"))
    if data.get("version") != 1 or not isinstance(data.get("characters"), dict):
        raise ValueError(f"制作正本indexの形式が不正です: {index_path}")
    for character_id, mappings in data["characters"].items():
        if character_id not in CHARACTER_IDS or not isinstance(mappings, dict):
            raise ValueError(f"制作正本のキャラクター定義が不正です: {character_id}")
        for relative, blob in mappings.items():
            if Path(relative).is_absolute() or ".." in Path(relative).parts:
                raise ValueError(f"危険な展開先です: {relative}")
            blob_path = root / blob
            if Path(blob).is_absolute() or ".." in Path(blob).parts or not blob_path.is_file():
                raise FileNotFoundError(f"制作正本blobがありません: {blob}")
    return data


def expand(output: Path) -> None:
    output = output.resolve()
    temp_root = (ROOT / "temp").resolve()
    if output != temp_root and temp_root not in output.parents:
        raise ValueError(f"展開先はリポジトリのtemp内にしてください: {output}")
    data = read_index()
    output.mkdir(parents=True, exist_ok=True)
    for character_id, mappings in data["characters"].items():
        for relative, blob in mappings.items():
            destination = output / character_id / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(STORE / blob, destination)
    print(f"[character-sources] {output} へ展開しました")


def main() -> None:
    parser = argparse.ArgumentParser(description="同梱2Dキャラ制作正本の共有化と一時展開")
    commands = parser.add_subparsers(dest="command", required=True)
    compact_parser = commands.add_parser("compact", help="展開済みフォルダを内容共有ストアへ戻す")
    compact_parser.add_argument("--input", type=Path, required=True)
    expand_parser = commands.add_parser("expand", help="Photoshop編集用にtempへ展開する")
    expand_parser.add_argument("--output", type=Path, default=ROOT / "temp" / "character-sources-expanded")
    args = parser.parse_args()
    if args.command == "compact":
        compact(args.input)
    else:
        expand(args.output)


if __name__ == "__main__":
    main()
