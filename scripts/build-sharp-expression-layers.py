#!/usr/bin/env python3
# build-sharp-expression-layers.py - 高精細な立ち絵と顔差分から、ぼけない基本表情レイヤーを組み立てる。

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
MINIMUM_OUTPUT_SIZE = (890, 1800)


def scale_box(
    box: tuple[int, int, int, int],
    source_size: tuple[int, int],
    destination_size: tuple[int, int],
) -> tuple[int, int, int, int]:
    scale_x = destination_size[0] / source_size[0]
    scale_y = destination_size[1] / source_size[1]
    return tuple(
        round(value * (scale_x if index % 2 == 0 else scale_y))
        for index, value in enumerate(box)
    )


def normalize(
    source: Image.Image,
    output_size: tuple[int, int],
    target_box: tuple[int, int, int, int],
) -> Image.Image:
    source = source.convert("RGBA")
    source_box = source.getchannel("A").getbbox()
    if source_box is None:
        raise ValueError("立ち絵に不透明な画素がありません")
    target_size = (target_box[2] - target_box[0], target_box[3] - target_box[1])
    cropped = source.crop(source_box)
    scale = min(1.0, target_size[0] / cropped.width, target_size[1] / cropped.height)
    if scale < 1.0:
        fitted_size = (max(1, round(cropped.width * scale)), max(1, round(cropped.height * scale)))
        fitted = cropped.resize(fitted_size, Image.Resampling.LANCZOS)
    else:
        fitted = cropped
    offset = (
        target_box[0] + (target_size[0] - fitted.width) // 2,
        target_box[1] + (target_size[1] - fitted.height) // 2,
    )
    result = Image.new("RGBA", output_size, (0, 0, 0, 0))
    result.alpha_composite(fitted, offset)
    return result


def apply_face_patch(source: Image.Image, patch: Image.Image, opacity: float = 1.0) -> Image.Image:
    result = source.copy().convert("RGBA")
    patch = patch.convert("RGBA")
    if patch.size != result.size:
        raise ValueError(f"顔差分と立ち絵の寸法が違います: {patch.size} != {result.size}")
    if opacity != 1.0:
        alpha = patch.getchannel("A").point(lambda value: round(value * opacity))
        patch.putalpha(alpha)
    result.alpha_composite(patch)
    result.putalpha(source.getchannel("A"))
    return result


def save_atomic(image: Image.Image, destination: Path) -> None:
    temporary = destination.with_suffix(".png.part")
    image.save(temporary, format="PNG", optimize=True)
    temporary.replace(destination)


def build(character_id: str, prepare_photoshop: bool) -> None:
    character_dir = ROOT / "characters" / character_id
    preview_path = character_dir / "preview.png"
    layers_dir = character_dir / "layers"
    patches_dir = character_dir / "expression-patches"
    required = [
        preview_path,
        layers_dir / "base.png",
        patches_dir / "blink_half.png",
        patches_dir / "blink_closed.png",
        patches_dir / "mouth_open.png",
    ]
    missing = [str(path) for path in required if not path.is_file()]
    if missing:
        raise FileNotFoundError("必要な画像がありません: " + ", ".join(missing))

    with Image.open(layers_dir / "base.png") as old_base:
        old_base = old_base.convert("RGBA")
        output_size = old_base.size
        target_box = old_base.getchannel("A").getbbox()
    if target_box is None:
        raise ValueError("既存base.pngに不透明な画素がありません")
    if output_size[0] < MINIMUM_OUTPUT_SIZE[0] or output_size[1] < MINIMUM_OUTPUT_SIZE[1]:
        raise ValueError(f"既存base.pngが小さすぎます: {output_size}")
    if prepare_photoshop:
        # 現在の完成寸法を単純に半分へすると、再実行のたび1780→890→445と劣化する。
        # Photoshop前の正本寸法へ必ず正規化し、何度実行しても同じ結果にする。
        target_box = scale_box(target_box, output_size, MINIMUM_OUTPUT_SIZE)
        output_size = MINIMUM_OUTPUT_SIZE
    with Image.open(preview_path) as opened:
        preview = opened.convert("RGBA")
    with Image.open(patches_dir / "blink_half.png") as opened:
        blink_half_patch = opened.convert("RGBA")
    with Image.open(patches_dir / "blink_closed.png") as opened:
        blink_closed_patch = opened.convert("RGBA")
    with Image.open(patches_dir / "mouth_open.png") as opened:
        mouth_patch = opened.convert("RGBA")
    mouth_closed_path = patches_dir / "mouth_closed.png"
    if mouth_closed_path.is_file():
        with Image.open(mouth_closed_path) as opened:
            resting = apply_face_patch(preview, opened.convert("RGBA"))
    else:
        resting = preview

    variants = {
        "base": resting,
        "mouth_closed": resting,
        "blink_half": apply_face_patch(resting, blink_half_patch),
        "blink_closed": apply_face_patch(resting, blink_closed_patch),
        "mouth_open": apply_face_patch(preview, mouth_patch),
    }
    layers_dir.mkdir(parents=True, exist_ok=True)
    for name, image in variants.items():
        save_atomic(normalize(image, output_size, target_box), layers_dir / f"{name}.png")

    manifest_path = character_dir / "character.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["animationQualityMode"] = "sharp-expression"
    temporary = manifest_path.with_suffix(".json.part")
    temporary.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(manifest_path)

    if not prepare_photoshop:
        subprocess.run(
            [sys.executable, str(ROOT / "scripts" / "build-character-animations.py"), "--character-dir", str(character_dir)],
            check=True,
        )
        print(f"[sharp-expression] {character_id}: 高精細な基本表情と連番を再生成しました")
    else:
        print(f"[sharp-expression] {character_id}: Photoshop仕上げ前の基本表情を生成しました")


def main() -> None:
    parser = argparse.ArgumentParser(description="高精細な立ち絵と顔差分から基本表情を再生成します。")
    parser.add_argument("character_ids", nargs="+", help="対象キャラクターID")
    parser.add_argument(
        "--prepare-photoshop",
        action="store_true",
        help="Photoshopの2倍拡大前の寸法で書き出します。",
    )
    args = parser.parse_args()
    for character_id in args.character_ids:
        build(character_id, args.prepare_photoshop)


if __name__ == "__main__":
    main()
