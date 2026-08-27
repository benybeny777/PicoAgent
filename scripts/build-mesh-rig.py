#!/usr/bin/env python3
"""既存の高精細レイヤーから、局所補間用メッシュリグ資産を生成する。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]


def save_lossless_webp(image: Image.Image, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(".webp.part")
    image.save(temporary, format="WEBP", lossless=True, exact=True, method=6)
    with Image.open(temporary) as reopened:
        decoded = reopened.convert("RGBA")
    if decoded.size != image.size or ImageChops.difference(decoded, image).getbbox() is not None:
        temporary.unlink(missing_ok=True)
        raise ValueError(f"lossless WebPの復号結果が一致しません: {destination}")
    temporary.replace(destination)


def scaled_box(box: list[int], scale: float) -> tuple[int, int, int, int]:
    return tuple(round(value * scale) for value in box)


def feathered_region(image: Image.Image, box: tuple[int, int, int, int], feather: int) -> Image.Image:
    width, height = image.size
    left, top, right, bottom = box
    if not (0 <= left < right <= width and 0 <= top < bottom <= height):
        raise ValueError(f"局所領域が画像外です: {box} / {image.size}")
    mask = Image.new("L", image.size, 0)
    core = Image.new("L", (right - left, bottom - top), 255)
    mask.paste(core, (left, top))
    if feather > 0:
        mask = mask.filter(ImageFilter.GaussianBlur(feather))
    alpha = ImageChops.multiply(image.getchannel("A"), mask)
    result = image.copy()
    result.putalpha(alpha)
    return result


def gaze_sources(
    base: Image.Image,
    config: dict[str, object],
) -> tuple[dict[str, Image.Image], dict[str, dict[str, object]]]:
    """高精細正本の瞳だけを小さく移動し、左右視線の局所キーフォームを作る。"""
    iris_boxes = [tuple(int(value) for value in box) for box in config.get("irisBoxes", [])]
    if len(iris_boxes) != 2 or any(len(box) != 4 for box in iris_boxes):
        raise ValueError("gaze.irisBoxesには左右2個の矩形が必要です")
    for box in iris_boxes:
        left, top, right, bottom = box
        if not (0 <= left < right <= base.width and 0 <= top < bottom <= base.height):
            raise ValueError(f"瞳領域が画像外です: {box} / {base.size}")

    offset = int(config.get("offset", 0))
    if not 1 <= offset <= 24:
        raise ValueError(f"gaze.offsetは1から24画素にしてください: {offset}")
    mask_feather = max(0, int(config.get("maskFeather", 0)))
    patch_box = tuple(int(value) for value in config.get("box", []))
    if len(patch_box) != 4:
        raise ValueError("gaze.boxには局所テクスチャの矩形が必要です")
    patch_feather = max(0, int(config.get("feather", 0)))

    mask = Image.new("L", base.size, 0)
    draw = ImageDraw.Draw(mask)
    for box in iris_boxes:
        draw.ellipse(box, fill=255)
    if mask_feather:
        mask = mask.filter(ImageFilter.GaussianBlur(mask_feather))

    sources: dict[str, Image.Image] = {}
    regions: dict[str, dict[str, object]] = {}
    for name, horizontal_offset in (("gaze_left", -offset), ("gaze_right", offset)):
        shifted = ImageChops.offset(base, horizontal_offset, 0)
        source = Image.composite(shifted, base, mask)
        difference_box = ImageChops.difference(source, base).getbbox()
        if difference_box is None:
            raise ValueError(f"視線差分を生成できません: {name}")
        if not (
            patch_box[0] <= difference_box[0]
            and patch_box[1] <= difference_box[1]
            and patch_box[2] >= difference_box[2]
            and patch_box[3] >= difference_box[3]
        ):
            raise ValueError(f"視線差分がgaze.boxを越えています: {name} {difference_box} / {patch_box}")
        sources[name] = source
        regions[name] = {
            "source": name,
            "box": list(patch_box),
            "feather": patch_feather,
        }
    return sources, regions


def build(character_id: str) -> None:
    character_dir = ROOT / "characters" / character_id
    source_config_path = character_dir / "mesh-rig" / "source" / "regions.json"
    config = json.loads(source_config_path.read_text(encoding="utf-8"))
    scale = float(config.get("runtimeScale", 0.5))
    if not 0 < scale <= 1:
        raise ValueError(f"runtimeScaleは0より大きく1以下にしてください: {scale}")

    layers_dir = character_dir / "layers"
    base_path = layers_dir / "mouth_closed.png"
    with Image.open(base_path) as opened:
        base = opened.convert("RGBA")
    if base.getchannel("A").getbbox() is None:
        raise ValueError("基準レイヤーに不透明画素がありません")
    runtime_size = (round(base.width * scale), round(base.height * scale))
    base_runtime = base.resize(runtime_size, Image.Resampling.LANCZOS)

    parts_dir = character_dir / "mesh-rig" / "parts"
    save_lossless_webp(base_runtime, parts_dir / "base.webp")
    generated_sources: dict[str, Image.Image] = {}
    regions = dict(config.get("regions", {}))
    if gaze_config := config.get("gaze"):
        if not isinstance(gaze_config, dict):
            raise ValueError("gazeはオブジェクトにしてください")
        generated_sources, gaze_regions = gaze_sources(base, gaze_config)
        for name in gaze_regions:
            if name in regions:
                raise ValueError(f"局所レイヤー名が重複しています: {name}")
        regions.update(gaze_regions)

    drawables: dict[str, dict[str, object]] = {}
    for name, region in regions.items():
        source_name = str(region["source"])
        if source_name in generated_sources:
            source = generated_sources[source_name]
        else:
            with Image.open(layers_dir / f"{source_name}.png") as opened:
                source = opened.convert("RGBA")
        if source.size != base.size:
            raise ValueError(f"レイヤー寸法が一致しません: {source_name} {source.size} != {base.size}")
        runtime = source.resize(runtime_size, Image.Resampling.LANCZOS)
        box = scaled_box(list(region["box"]), scale)
        feather = max(0, round(float(region.get("feather", 0)) * scale))
        patch = feathered_region(runtime, box, feather)
        texture_box = patch.getchannel("A").getbbox()
        if texture_box is None:
            raise ValueError(f"局所レイヤーに不透明画素がありません: {name}")
        save_lossless_webp(patch.crop(texture_box), parts_dir / f"{name}.webp")
        drawables[name] = {
            "texture": f"parts/{name}.webp",
            "box": list(box),
            "textureBox": list(texture_box),
            "feather": feather,
        }
    expected_parts = {"base.webp", *(f"{name}.webp" for name in drawables)}
    for obsolete in parts_dir.glob("*.webp"):
        if obsolete.name not in expected_parts:
            obsolete.unlink()

    content_box = base.getchannel("A").getbbox()
    assert content_box is not None
    rig = {
        "version": 1,
        "mode": "single-sheet-grid-v1",
        "sourceCanvas": [base.width, base.height],
        "canvas": list(runtime_size),
        "contentBox": list(scaled_box(list(content_box), scale)),
        "runtimeScale": scale,
        "grid": list(config.get("grid", [9, 15])),
        "base": "parts/base.webp",
        "drawables": drawables,
        "parameters": {
            "ParamAngleX": [-6, 6, 0],
            "ParamAngleY": [-4, 4, 0],
            "ParamEyeLOpen": [0, 1, 1],
            "ParamEyeBallX": [-1, 1, 0],
            "ParamMouthOpenY": [0, 1, 0],
            "ParamBreath": [0, 1, 0],
        },
        "quality": {
            "source": "existing-sharp-expression-layers",
            "hiddenAreaSource": False,
            "gazeSource": "高精細正本の瞳領域を同一座標で局所移動",
            "limit": "顔内部の補間、局所視線、一枚メッシュの微細変形。大きなパーツ分離は隠れ無し原画の追加後に行う",
        },
    }
    rig_path = character_dir / "mesh-rig" / "rig.json"
    temporary_rig = rig_path.with_suffix(".json.part")
    temporary_rig.write_text(json.dumps(rig, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary_rig.replace(rig_path)

    manifest_path = character_dir / "character.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["meshRig"] = {"rig": "mesh-rig/rig.json"}
    temporary_manifest = manifest_path.with_suffix(".json.part")
    temporary_manifest.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary_manifest.replace(manifest_path)
    print(f"[mesh-rig] {character_id}: {runtime_size[0]}x{runtime_size[1]} / {len(drawables)}局所レイヤー")


def main() -> None:
    parser = argparse.ArgumentParser(description="高精細レイヤーからメッシュリグ資産を生成します")
    parser.add_argument("character_ids", nargs="+", help="対象キャラクターID")
    args = parser.parse_args()
    for character_id in args.character_ids:
        build(character_id)


if __name__ == "__main__":
    main()
