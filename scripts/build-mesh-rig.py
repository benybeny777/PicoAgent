#!/usr/bin/env python3
"""既存の高精細レイヤーから、局所補間用メッシュリグ資産を生成する。"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import time

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]


def canonicalize_transparent_rgb(image: Image.Image) -> Image.Image:
    image = image.convert("RGBA")
    transparent = Image.new("RGBA", image.size, (0, 0, 0, 0))
    visible_mask = image.getchannel("A").point(lambda alpha: 255 if alpha else 0)
    return Image.composite(image, transparent, visible_mask)


def images_equal(left: Image.Image, right: Image.Image) -> bool:
    if left.size != right.size or left.mode != right.mode:
        return False
    return all(high == 0 for _, high in ImageChops.difference(left, right).getextrema())


def difference_box(left: Image.Image, right: Image.Image) -> tuple[int, int, int, int] | None:
    difference = ImageChops.difference(left, right)
    boxes = [band.getbbox() for band in difference.split()]
    visible = [box for box in boxes if box is not None]
    if not visible:
        return None
    return (
        min(box[0] for box in visible),
        min(box[1] for box in visible),
        max(box[2] for box in visible),
        max(box[3] for box in visible),
    )


def save_lossless_webp(image: Image.Image, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(".webp.part")
    image.save(temporary, format="WEBP", lossless=True, exact=True, method=6)
    with Image.open(temporary) as reopened:
        decoded = reopened.convert("RGBA")
    if not images_equal(decoded, image):
        temporary.unlink(missing_ok=True)
        raise ValueError(f"lossless WebPの復号結果が一致しません: {destination}")
    for attempt in range(5):
        try:
            temporary.replace(destination)
            break
        except PermissionError:
            if attempt == 4:
                raise
            # Windowsの画像検査が閉じた直後だけ短時間ファイルを保持する場合がある。
            time.sleep(0.2 * (attempt + 1))


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


def load_same_canvas(path: Path, size: tuple[int, int], label: str) -> Image.Image:
    with Image.open(path) as opened:
        image = canonicalize_transparent_rgb(opened)
    if image.size != size:
        raise ValueError(f"分離原画の寸法が一致しません: {label} {image.size} != {size}")
    return image


def composite_neutral(parts: list[tuple[int, Image.Image]]) -> Image.Image:
    if not parts:
        raise ValueError("separatedPartsが空です")
    canvas = Image.new("RGBA", parts[0][1].size, (0, 0, 0, 0))
    for _, part in sorted(parts, key=lambda item: item[0]):
        canvas.alpha_composite(part)
    return canvas


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
        changed_box = difference_box(source, base)
        if changed_box is None:
            raise ValueError(f"視線差分を生成できません: {name}")
        if not (
            patch_box[0] <= changed_box[0]
            and patch_box[1] <= changed_box[1]
            and patch_box[2] >= changed_box[2]
            and patch_box[3] >= changed_box[3]
        ):
            raise ValueError(f"視線差分がgaze.boxを越えています: {name} {changed_box} / {patch_box}")
        sources[name] = source
        regions[name] = {
            "source": name,
            "box": list(patch_box),
            "feather": patch_feather,
        }
    return sources, regions


def automatic_face_regions(
    base: Image.Image,
    layers_dir: Path,
    allowed_box: tuple[int, int, int, int],
) -> dict[str, dict[str, object]]:
    regions: dict[str, dict[str, object]] = {}
    for source_name in (
        "blink_half",
        "blink_closed",
        "mouth_open",
        "gaze_left",
        "gaze_right",
        "expression_happy",
        "expression_sad",
    ):
        path = layers_dir / f"{source_name}.png"
        if not path.is_file():
            continue
        source = load_same_canvas(path, base.size, source_name)
        local_box = difference_box(source.crop(allowed_box), base.crop(allowed_box))
        if local_box is None:
            continue
        changed_box = (
            local_box[0] + allowed_box[0],
            local_box[1] + allowed_box[1],
            local_box[2] + allowed_box[0],
            local_box[3] + allowed_box[1],
        )
        margin = max(8, round(min(base.size) * 0.012))
        left = max(0, changed_box[0] - margin)
        top = max(0, changed_box[1] - margin)
        right = min(base.width, changed_box[2] + margin)
        bottom = min(base.height, changed_box[3] + margin)
        regions[source_name] = {
            "source": source_name,
            "box": [left, top, right, bottom],
            "feather": max(2, margin // 3),
        }
    return regions


def build(character_id: str) -> None:
    character_dir = ROOT / "characters" / character_id
    source_config_path = character_dir / "mesh-rig" / "source" / "regions.json"
    config = json.loads(source_config_path.read_text(encoding="utf-8"))
    scale = float(config.get("runtimeScale", 0.5))
    if not 0 < scale <= 1:
        raise ValueError(f"runtimeScaleは0より大きく1以下にしてください: {scale}")
    expression_sad_mode = str(config.get("expressionSadMode", "layer"))
    if expression_sad_mode not in {"layer", "disabled"}:
        raise ValueError(f"expressionSadModeが不正です: {expression_sad_mode}")

    layers_dir = character_dir / "layers"
    base_path = layers_dir / "mouth_closed.png"
    with Image.open(base_path) as opened:
        base = canonicalize_transparent_rgb(opened)
    if base.getchannel("A").getbbox() is None:
        raise ValueError("基準レイヤーに不透明画素がありません")
    runtime_size = (round(base.width * scale), round(base.height * scale))
    base_runtime = base.resize(runtime_size, Image.Resampling.LANCZOS)

    parts_dir = character_dir / "mesh-rig" / "parts"
    separated_config = config.get("separatedParts")
    separated_drawables: dict[str, dict[str, object]] = {}
    hidden_area_sources: list[str] = []
    head_source_box: tuple[int, int, int, int] | None = None
    if separated_config is not None:
        if not isinstance(separated_config, list) or not separated_config:
            raise ValueError("separatedPartsは空でない配列にしてください")
        source_parts_dir = source_config_path.parent / "parts"
        neutral_parts: list[tuple[int, Image.Image]] = []
        seen_names: set[str] = set()
        for item in separated_config:
            if not isinstance(item, dict):
                raise ValueError("separatedPartsの各要素はオブジェクトにしてください")
            name = str(item.get("name", ""))
            if not name or name in seen_names:
                raise ValueError(f"分離原画名が空または重複しています: {name}")
            seen_names.add(name)
            relative = Path(str(item.get("source", "")))
            if relative.is_absolute() or ".." in relative.parts or relative.suffix.lower() != ".png":
                raise ValueError(f"分離原画のsourceが不正です: {relative}")
            part = load_same_canvas(source_parts_dir / relative, base.size, name)
            if part.getchannel("A").getbbox() is None:
                raise ValueError(f"分離原画に不透明画素がありません: {name}")
            part_box = part.getchannel("A").getbbox()
            assert part_box is not None
            if name in {"hair-back", "face", "hair-front"}:
                if head_source_box is None:
                    head_source_box = part_box
                else:
                    head_source_box = (
                        min(head_source_box[0], part_box[0]),
                        min(head_source_box[1], part_box[1]),
                        max(head_source_box[2], part_box[2]),
                        max(head_source_box[3], part_box[3]),
                    )
            z_index = int(item.get("z", 0))
            neutral_parts.append((z_index, part))
            if bool(item.get("hiddenAreaSource", False)):
                hidden_area_sources.append(name)
            runtime = part.resize(runtime_size, Image.Resampling.LANCZOS)
            texture_box = runtime.getchannel("A").getbbox()
            assert texture_box is not None
            save_lossless_webp(runtime.crop(texture_box), parts_dir / f"{name}.webp")
            separated_drawables[name] = {
                "texture": f"parts/{name}.webp",
                "textureBox": list(texture_box),
                "z": z_index,
                "initialAlpha": 1,
                "motion": str(item.get("motion", "fixed")),
            }
        neutral = composite_neutral(neutral_parts)
        if not images_equal(neutral, base):
            raise ValueError("分離原画の中立合成がlayers/mouth_closed.pngと一致しません")
        base_runtime = Image.new("RGBA", runtime_size, (0, 0, 0, 0))
    save_lossless_webp(base_runtime, parts_dir / "base.webp")
    generated_sources: dict[str, Image.Image] = {}
    regions = dict(config.get("regions", {}))
    if bool(config.get("autoFaceRegions", False)):
        if head_source_box is None:
            raise ValueError("autoFaceRegionsには頭部分離原画が必要です")
        for name, region in automatic_face_regions(base, layers_dir, head_source_box).items():
            regions.setdefault(name, region)
    if gaze_config := config.get("gaze"):
        if not isinstance(gaze_config, dict):
            raise ValueError("gazeはオブジェクトにしてください")
        generated_sources, gaze_regions = gaze_sources(base, gaze_config)
        for name in gaze_regions:
            regions.pop(name, None)
        regions.update(gaze_regions)

    drawables: dict[str, dict[str, object]] = dict(separated_drawables)
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
            "z": 1000,
            "initialAlpha": 0,
            "motion": "face",
        }
    expected_parts = {"base.webp", *(f"{name}.webp" for name in drawables)}
    for obsolete in parts_dir.glob("*.webp"):
        if obsolete.name not in expected_parts:
            obsolete.unlink()

    content_box = base.getchannel("A").getbbox()
    assert content_box is not None
    rig = {
        "version": 1,
        "mode": "layered-grid-v2" if separated_config is not None else "single-sheet-grid-v1",
        "sourceCanvas": [base.width, base.height],
        "canvas": list(runtime_size),
        "contentBox": list(scaled_box(list(content_box), scale)),
        "runtimeScale": scale,
        "expressionSadMode": expression_sad_mode,
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
            "source": str(config.get("sourceMethod", "既存の通常2Dレイヤー")),
            "hiddenAreaSource": bool(hidden_area_sources),
            "hiddenAreaParts": hidden_area_sources,
            "motionLimits": config.get("motionLimits", {}),
            "gazeSource": "高精細正本の瞳領域を同一座標で局所移動",
            "limit": (
                "分離原画で定義した部位の微細変形。大きな姿勢変更は対象外"
                if separated_config is not None
                else "顔内部の補間、局所視線、一枚メッシュの微細変形。大きなパーツ分離は隠れ無し原画の追加後に行う"
            ),
        },
    }
    rig_path = character_dir / "mesh-rig" / "rig.json"
    temporary_rig = rig_path.with_suffix(".json.part")
    rig_text = json.dumps(rig, ensure_ascii=False, indent=2) + "\n"
    temporary_rig.write_text(rig_text, encoding="utf-8")
    temporary_rig.replace(rig_path)

    asset_digest = hashlib.sha256(rig_text.encode("utf-8"))
    for part_path in sorted(parts_dir.glob("*.webp")):
        asset_digest.update(part_path.name.encode("utf-8"))
        asset_digest.update(part_path.read_bytes())
    mesh_asset_version = asset_digest.hexdigest()[:16]

    manifest_path = character_dir / "character.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["meshRig"] = {"rig": "mesh-rig/rig.json", "assetVersion": mesh_asset_version}
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
