#!/usr/bin/env python3
# build-character-animations.py - キャラクター表情レイヤーから連番アニメーションを生成する。

from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
CHARACTERS_DIR = ROOT / "characters"

ANIMATIONS = {
    "idle": {
        "path": "animations/idle",
        "frames": 12,
        "fps": 3.2,
        "frameDurations": [760, 760, 760, 520, 70, 90, 70, 760, 760, 760, 760, 1100],
        "motion": "live2d-idle",
    },
    "thinking": {
        "path": "animations/running",
        "frames": 12,
        "fps": 7.0,
        "frameDurations": [180, 150, 120, 170, 150, 170, 120, 120, 170, 150, 170, 150],
        "motion": "live2d-thinking",
    },
    "talking": {
        "path": "animations/review",
        "frames": 12,
        "fps": 8.0,
        "frameDurations": [120, 90, 75, 120, 120, 120, 75, 120, 90, 120, 75, 140],
        "motion": "live2d-talking",
    },
    "sad": {
        "path": "animations/failed",
        "frames": 6,
        "fps": 4.2,
        "frameDurations": [320, 110, 150, 120, 420, 560],
        "motion": "live2d-sad",
    },
}

SEQUENCES = {
    "idle": [
        ("base", 0),
        ("base", 1),
        ("base", 1),
        ("base", 0),
        ("blink_half", -1),
        ("blink_closed", -1),
        ("blink_half", 0),
        ("base", 1),
        ("base", 1),
        ("base", 0),
        ("base", -1),
        ("base", -1),
    ],
    "thinking": [
        ("gaze_left", 0),
        ("gaze_left", 1),
        ("blink_half", 1),
        ("gaze_right", 0),
        ("gaze_right", -1),
        ("gaze_left", -1),
        ("blink_closed", 0),
        ("blink_half", 1),
        ("gaze_right", 1),
        ("gaze_right", 0),
        ("gaze_left", -1),
        ("gaze_left", 0),
    ],
    "talking": [
        ("mouth_closed", 0),
        ("mouth_closed", 1),
        ("mouth_open", 1),
        ("mouth_closed", 0),
        ("mouth_closed", -1),
        ("mouth_closed", -1),
        ("mouth_open", 1),
        ("mouth_closed", 1),
        ("blink_half", 1),
        ("mouth_closed", 0),
        ("mouth_open", 0),
        ("mouth_closed", -1),
    ],
    "sad": [
        ("expression_sad", 0),
        ("expression_sad", -1),
        ("blink_closed", -1),
        ("expression_sad", 0),
        ("expression_sad", 1),
        ("expression_sad", 0),
    ],
}


def doubled_sequences() -> dict[str, list[tuple[str, int]]]:
    """Return the high-quality 84-frame sequence used with real-time springs."""
    return {
        name: [frame for frame in frames for _ in range(2)]
        for name, frames in SEQUENCES.items()
    }


def doubled_animations() -> dict[str, dict]:
    """Preserve timing while doubling expression keyframes."""
    result = copy.deepcopy(ANIMATIONS)
    for animation in result.values():
        durations = animation.get("frameDurations", [])
        animation["frameDurations"] = [
            half
            for duration in durations
            for half in (max(16, duration // 2), max(16, duration - duration // 2))
        ]
        animation["frames"] *= 2
        animation["fps"] *= 2
    return result

# THA3で追加キーポーズが揃っているキャラだけが使う42フレーム構成。
# 通常キャラは従来のSEQUENCESをそのまま使うため、既存素材の見た目は変わらない。
THA3_SEQUENCES = {
    "idle": [
        ("base", 0),
        ("idle_left", 0),
        ("idle_breathe", 0),
        ("idle_right", 0),
        ("blink_half", 0),
        ("blink_closed", 0),
        ("blink_half", 0),
        ("idle_left", 0),
        ("base", 0),
        ("idle_right", 0),
        ("idle_breathe", 0),
        ("base", 0),
    ],
    "thinking": [
        ("thinking_left", 0),
        ("thinking_right", 0),
        ("thinking_left", 0),
        ("thinking_blink", 0),
        ("thinking_right", 0),
        ("thinking_left", 0),
        ("blink_closed", 0),
        ("thinking_blink", 0),
        ("thinking_right", 0),
        ("thinking_left", 0),
        ("thinking_right", 0),
        ("thinking_left", 0),
    ],
    "talking": [
        ("talking_left_closed", 0),
        ("talking_left_open", 0),
        ("talking_right_open", 0),
        ("talking_right_closed", 0),
        ("talking_left_open", 0),
        ("talking_left_closed", 0),
        ("talking_right_open", 0),
        ("talking_right_closed", 0),
        ("blink_half", 0),
        ("talking_left_open", 0),
        ("talking_right_open", 0),
        ("talking_left_closed", 0),
    ],
    "sad": [
        ("sad", 0),
        ("sad_blink", 0),
        ("blink_closed", 0),
        ("sad_blink", 0),
        ("sad", 0),
        ("sad", 0),
    ],
}

THA3_LAYER_NAMES = {
    layer_name
    for frames in THA3_SEQUENCES.values()
    for layer_name, _sway in frames
    if layer_name not in {"base", "blink_half", "blink_closed", "mouth_closed", "mouth_open"}
}


def reject_mixed_sharp_assets(character_dir: Path, manifest: dict) -> None:
    """高精細表情パックへ旧THA3全身ポーズが混ざる状態を構造的に拒否する。"""
    if manifest.get("animationQualityMode") != "sharp-expression":
        return
    layers_dir = character_dir / "layers"
    stale = sorted(name for name in THA3_LAYER_NAMES if (layers_dir / f"{name}.png").is_file())
    if stale:
        raise ValueError(
            f"{character_dir.name} はsharp-expressionですが旧THA3全身レイヤーが残っています: "
            f"{', '.join(stale)}。新旧画像の混在を避けるため削除してください。"
        )

ANIMATION_NAMES = {
    "idle": "idle",
    "thinking": "running",
    "talking": "review",
    "sad": "failed",
}

FALLBACK_LAYERS = {
    "blink_half": "blink_closed",
    "blink_closed": "base",
    "mouth_closed": "base",
    "mouth_open": "base",
    "gaze_left": "base",
    "gaze_right": "base",
    "expression_happy": "base",
    "expression_sad": "base",
}


def character_dirs(selected: list[str]) -> list[Path]:
    if selected:
        return [CHARACTERS_DIR / name for name in selected]
    return sorted(path for path in CHARACTERS_DIR.iterdir() if path.is_dir())


def load_manifest(character_dir: Path) -> dict:
    manifest_path = character_dir / "character.json"
    if not manifest_path.exists():
        raise FileNotFoundError(f"character.json が見つかりません: {manifest_path}")
    return json.loads(manifest_path.read_text(encoding="utf-8"))


def write_manifest(character_dir: Path, manifest: dict) -> None:
    manifest_path = character_dir / "character.json"
    temporary = manifest_path.with_suffix(".json.tmp")
    temporary.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    temporary.replace(manifest_path)


def file_digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def compact_animation_frames(character_dir: Path, manifest: dict) -> None:
    """全状態を内容固定の単一世代へ封入し、新旧フレームの混在を防ぐ。"""
    animations = manifest.get("animations")
    if not isinstance(animations, dict):
        return

    frame_store = character_dir / "frames"
    sources: dict[str, list[tuple[str, Path]]] = {}
    generation_hasher = hashlib.sha256()
    for animation_name in sorted(animations):
        animation = animations[animation_name]
        if not isinstance(animation, dict):
            raise ValueError(f"animations.{animation_name} はオブジェクトで指定してください")
        numbered_path = animation.get("path")
        conventional_name = ANIMATION_NAMES.get(animation_name)
        conventional_path = f"animations/{conventional_name}" if conventional_name else None
        if not numbered_path and conventional_path and (character_dir / conventional_path).is_dir():
            # 既に共有化済みのパックをPhotoshop正本から再生成した場合は、新しい連番を優先する。
            numbered_path = conventional_path
        frame_count = int(animation.get("frames", 0))
        if frame_count <= 0:
            raise ValueError(f"animations.{animation_name}.frames は1以上にしてください")
        existing_paths = animation.get("framePaths")
        if not numbered_path and (not isinstance(existing_paths, list) or not existing_paths):
            raise ValueError(f"animations.{animation_name} に path / framePaths がありません")
        state_sources: list[tuple[str, Path]] = []
        for index in range(frame_count):
            relative = (
                f"{numbered_path}/{index:02}.png"
                if numbered_path
                else str(existing_paths[index])
            )
            source = character_dir / relative
            if not source.is_file():
                raise FileNotFoundError(f"連番フレームがありません: {source}")
            digest = file_digest(source)
            generation_hasher.update(animation_name.encode("utf-8"))
            generation_hasher.update(index.to_bytes(4, "big"))
            generation_hasher.update(digest.encode("ascii"))
            state_sources.append((digest, source))
        sources[animation_name] = state_sources

    generation = generation_hasher.hexdigest()[:16]
    staging = character_dir / f"frames-{generation}.part"
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir(parents=True)
    copied: set[str] = set()
    for animation_name, state_sources in sources.items():
        paths: list[str] = []
        for digest, source in state_sources:
            filename = f"{digest[:20]}.png"
            if filename not in copied:
                shutil.copy2(source, staging / filename)
                copied.add(filename)
            paths.append(f"frames/{generation}/{filename}")
        animations[animation_name]["framePaths"] = paths
        animation = animations[animation_name]
        animation.pop("path", None)

    generation_dir = frame_store / generation
    frame_store.mkdir(parents=True, exist_ok=True)
    if generation_dir.exists():
        shutil.rmtree(generation_dir)
    staging.replace(generation_dir)
    manifest["frameGeneration"] = generation
    # 新世代の配置とmanifestの原子的保存後だけ、旧世代と一時連番を消す。
    write_manifest(character_dir, manifest)
    animation_root = character_dir / "animations"
    if animation_root.is_dir():
        shutil.rmtree(animation_root)
    for path in frame_store.iterdir():
        if path != generation_dir:
            if path.is_dir():
                shutil.rmtree(path)
            else:
                path.unlink()


def ensure_layers(character_dir: Path) -> dict[str, Path]:
    layers_dir = character_dir / "layers"
    base = layers_dir / "base.png"
    if not base.exists():
        preview = character_dir / "preview.png"
        if not preview.exists():
            raise FileNotFoundError(f"base.png / preview.png が見つかりません: {character_dir}")
        layers_dir.mkdir(exist_ok=True)
        with Image.open(preview) as opened:
            converted = opened.convert("RGBA")
        try:
            converted.save(base)
        finally:
            converted.close()

    layer_paths = {"base": base}
    for layer, fallback in FALLBACK_LAYERS.items():
        path = layers_dir / f"{layer}.png"
        if not path.exists():
            with Image.open(layers_dir / f"{fallback}.png") as opened:
                converted = opened.convert("RGBA")
            try:
                converted.save(path)
            finally:
                converted.close()
        layer_paths[layer] = path
    return layer_paths


def warn_missing_boot(character_dir: Path, manifest: dict) -> None:
    boot_path = manifest.get("boot", {}).get("sleeping", "boot/sleeping.png")
    if not (character_dir / boot_path).exists():
        print(
            f"[build-character-animations] 警告: {character_dir.name} のローディング画像がありません: {boot_path}"
        )


def sway_image(image: Image.Image, amount: int) -> Image.Image:
    if amount == 0:
        return image.copy()
    src = image.convert("RGBA")
    width, height = src.size
    output = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    # 上半身と髪を 1〜2px だけ横にずらし、足元は固定する。
    for y in range(height):
        t = max(0.0, min(1.0, (0.76 * height - y) / (0.76 * height)))
        shift = int(round(amount * (0.45 + 1.15 * t))) if t > 0 else 0
        row = src.crop((0, y, width, y + 1))
        output.alpha_composite(row, (shift, y))
    return output


def remove_green_fringe(image: Image.Image) -> Image.Image:
    output = image.convert("RGBA")
    pixels = output.load()
    width, height = output.size
    for y in range(height):
        for x in range(width):
            red, green, blue, alpha = pixels[x, y]
            if alpha == 0:
                continue
            green_dominant = green > 60 and green > red * 1.04 and green > blue * 1.04
            vivid_green = green > 110 and green - max(red, blue) > 18
            if green_dominant or vivid_green:
                if alpha < 245:
                    pixels[x, y] = (red, green, blue, 0)
                else:
                    green = min(green, int(max(red, blue) * 1.02))
                    pixels[x, y] = (red, green, blue, alpha)
    return output


def build_frames(
    character_dir: Path,
    layer_paths: dict[str, Path],
    disable_sway: bool,
    quality_mode: str | None,
) -> None:
    layers_dir = character_dir / "layers"
    # 高精細立ち絵を正本にするパックは、512pxのTHA3全身ポーズへ切り替えると再びぼける。
    # 顔差分とCSS側の二次動作を組み合わせ、精細さを優先する。
    use_tha3_motion = quality_mode != "sharp-expression" and all(
        (layers_dir / f"{name}.png").is_file() for name in THA3_LAYER_NAMES
    )
    sequences = (
        THA3_SEQUENCES
        if use_tha3_motion
        else doubled_sequences() if quality_mode == "spring-high" else SEQUENCES
    )
    if use_tha3_motion:
        for name in THA3_LAYER_NAMES:
            layer_paths[name] = layers_dir / f"{name}.png"
        print(f"[build-character-animations] {character_dir.name} はTHA3の42フレーム構成を使います")
    rendered_frames: dict[tuple[str, int], Path] = {}
    for manifest_name, frames in sequences.items():
        animation_dir = character_dir / "animations" / ANIMATION_NAMES[manifest_name]
        animation_dir.mkdir(parents=True, exist_ok=True)
        for old in animation_dir.glob("*.png"):
            old.unlink()
        for index, (layer_name, sway) in enumerate(frames):
            destination = animation_dir / f"{index:02}.png"
            cache_key = (layer_name, 0 if disable_sway else sway)
            cached = rendered_frames.get(cache_key)
            if cached is not None:
                shutil.copy2(cached, destination)
                continue
            layer_path = layer_paths.get(layer_name, layer_paths["base"])
            with Image.open(layer_path) as opened:
                image = opened.convert("RGBA")
            try:
                if not disable_sway:
                    swayed = sway_image(image, sway)
                    image.close()
                    image = swayed
                cleaned = remove_green_fringe(image)
                try:
                    cleaned.save(destination)
                finally:
                    cleaned.close()
            finally:
                image.close()
            rendered_frames[cache_key] = destination


def clean_expression_layers(layer_paths: dict[str, Path]) -> None:
    for name in ("blink_half", "blink_closed", "mouth_open"):
        path = layer_paths.get(name)
        if path and path.exists():
            with Image.open(path) as opened:
                converted = opened.convert("RGBA")
            try:
                cleaned = remove_green_fringe(converted)
                try:
                    cleaned.save(path)
                finally:
                    cleaned.close()
            finally:
                converted.close()


def build_character(
    character_dir: Path,
    version: str | None,
    disable_sway: bool,
    compact_only: bool,
) -> None:
    manifest = load_manifest(character_dir)
    reject_mixed_sharp_assets(character_dir, manifest)
    if compact_only:
        compact_animation_frames(character_dir, manifest)
        print(f"[build-character-animations] {character_dir.name} の連番を共有化しました")
        return
    if manifest.get("live2d"):
        # Live2D キャラはモデル自身が動くため連番フレームを生成しない。
        print(f"[build-character-animations] {character_dir.name} は Live2D キャラのためスキップします")
        return
    warn_missing_boot(character_dir, manifest)
    layer_paths = ensure_layers(character_dir)
    clean_expression_layers(layer_paths)
    build_frames(character_dir, layer_paths, disable_sway, manifest.get("animationQualityMode"))
    manifest["animations"] = (
        doubled_animations()
        if manifest.get("animationQualityMode") == "spring-high"
        else copy.deepcopy(ANIMATIONS)
    )
    if version:
        manifest["assetVersion"] = f"{version}-{manifest.get('id', character_dir.name)}"
    compact_animation_frames(character_dir, manifest)
    print(f"[build-character-animations] {character_dir.name} を生成しました")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="characters/<id>/layers から連番アニメーションを生成します。"
    )
    parser.add_argument("characters", nargs="*", help="対象キャラクターID。省略時は全キャラ。")
    parser.add_argument("--character-dir", type=Path, help="GUI連携用。指定した1パックだけを生成する。")
    parser.add_argument("--version", help="assetVersion の先頭値（例: 2026-06-28-motion-v1）。")
    parser.add_argument("--disable-sway", action="store_true", help="髪・上半身の微小揺れを無効化する。")
    parser.add_argument(
        "--compact-only",
        action="store_true",
        help="既存animationsを内容共有フレームへ変換し、連番の重複配置を除く。",
    )
    args = parser.parse_args()

    targets = [args.character_dir.resolve()] if args.character_dir else character_dirs(args.characters)
    for character_dir in targets:
        build_character(character_dir, args.version, args.disable_sway, args.compact_only)


if __name__ == "__main__":
    main()
