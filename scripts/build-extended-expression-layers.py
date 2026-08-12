#!/usr/bin/env python3
"""元立ち絵と局所パッチから、視線・感情の共通レイヤーを生成する。"""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
EXTENDED_LAYERS = ("gaze_left", "gaze_right", "expression_happy", "expression_sad")
PRE_PHOTOSHOP_SIZE = (890, 1800)


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


def normalize(source: Image.Image, size: tuple[int, int], box: tuple[int, int, int, int]) -> Image.Image:
    source = source.convert("RGBA")
    source_box = source.getchannel("A").getbbox()
    if source_box is None:
        raise ValueError("元立ち絵に不透明画素がありません")
    target_size = (box[2] - box[0], box[3] - box[1])
    result = Image.new("RGBA", size, (0, 0, 0, 0))
    result.alpha_composite(source.crop(source_box).resize(target_size, Image.Resampling.LANCZOS), box[:2])
    return result


def apply_patch(source: Image.Image, patch: Image.Image) -> Image.Image:
    result = source.copy().convert("RGBA")
    patch = patch.convert("RGBA")
    if patch.size != result.size:
        raise ValueError(f"差分と元絵の寸法が違います: {patch.size} != {result.size}")
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
    required = [preview_path, layers_dir / "base.png", *(patches_dir / f"{name}.png" for name in EXTENDED_LAYERS)]
    missing = [str(path) for path in required if not path.is_file()]
    if missing:
        raise FileNotFoundError("必要な画像がありません: " + ", ".join(missing))

    with Image.open(layers_dir / "base.png") as opened:
        base = opened.convert("RGBA")
        output_size = base.size
        target_box = base.getchannel("A").getbbox()
    if target_box is None:
        raise ValueError("base.pngに不透明画素がありません")
    if prepare_photoshop:
        target_box = scale_box(target_box, output_size, PRE_PHOTOSHOP_SIZE)
        output_size = PRE_PHOTOSHOP_SIZE

    with Image.open(preview_path) as opened:
        preview = opened.convert("RGBA")
    layers_dir.mkdir(parents=True, exist_ok=True)
    for name in EXTENDED_LAYERS:
        with Image.open(patches_dir / f"{name}.png") as opened:
            composed = apply_patch(preview, opened.convert("RGBA"))
        save_atomic(normalize(composed, output_size, target_box), layers_dir / f"{name}.png")
    print(f"[extended-expression] {character_id}: 視線・感情4レイヤーを生成しました")


def main() -> None:
    parser = argparse.ArgumentParser(description="視線・感情の共通レイヤーを生成します。")
    parser.add_argument("character_ids", nargs="+")
    parser.add_argument("--prepare-photoshop", action="store_true")
    args = parser.parse_args()
    for character_id in args.character_ids:
        build(character_id, args.prepare_photoshop)


if __name__ == "__main__":
    main()
