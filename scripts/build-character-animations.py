#!/usr/bin/env python3
# build-character-animations.py - キャラクター表情レイヤーから連番アニメーションを生成する。

from __future__ import annotations

import argparse
import json
from pathlib import Path

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
        "fps": 9.0,
        "frameDurations": [95, 70, 90, 65, 95, 70, 90, 65, 85, 70, 90, 65],
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
        ("base", 0),
        ("base", 1),
        ("blink_half", 1),
        ("base", 0),
        ("base", -1),
        ("base", -1),
        ("blink_closed", 0),
        ("blink_half", 1),
        ("base", 1),
        ("base", 0),
        ("base", -1),
        ("base", 0),
    ],
    "talking": [
        ("mouth_closed", 0),
        ("mouth_open", 1),
        ("mouth_closed", 1),
        ("mouth_open", 0),
        ("mouth_closed", -1),
        ("mouth_open", -1),
        ("mouth_closed", 0),
        ("mouth_open", 1),
        ("blink_half", 1),
        ("mouth_open", 0),
        ("mouth_closed", -1),
        ("mouth_open", 0),
    ],
    "sad": [
        ("base", 0),
        ("blink_half", -1),
        ("blink_closed", -1),
        ("blink_half", 0),
        ("base", 1),
        ("base", 0),
    ],
}

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
    (character_dir / "character.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def ensure_layers(character_dir: Path) -> dict[str, Path]:
    layers_dir = character_dir / "layers"
    base = layers_dir / "base.png"
    if not base.exists():
        preview = character_dir / "preview.png"
        if not preview.exists():
            raise FileNotFoundError(f"base.png / preview.png が見つかりません: {character_dir}")
        layers_dir.mkdir(exist_ok=True)
        Image.open(preview).convert("RGBA").save(base)

    layer_paths = {"base": base}
    for layer, fallback in FALLBACK_LAYERS.items():
        path = layers_dir / f"{layer}.png"
        if not path.exists():
            Image.open(layers_dir / f"{fallback}.png").convert("RGBA").save(path)
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
            if green > 80 and green > red * 1.08 and green > blue * 1.08:
                if alpha < 210:
                    pixels[x, y] = (red, green, blue, 0)
                else:
                    green = min(green, int(max(red, blue) * 1.02))
                    pixels[x, y] = (red, green, blue, alpha)
    return output


def build_frames(character_dir: Path, layer_paths: dict[str, Path], disable_sway: bool) -> None:
    for manifest_name, frames in SEQUENCES.items():
        animation_dir = character_dir / "animations" / ANIMATION_NAMES[manifest_name]
        animation_dir.mkdir(parents=True, exist_ok=True)
        for old in animation_dir.glob("*.png"):
            old.unlink()
        for index, (layer_name, sway) in enumerate(frames):
            layer_path = layer_paths.get(layer_name, layer_paths["base"])
            image = Image.open(layer_path).convert("RGBA")
            if not disable_sway:
                image = sway_image(image, sway)
            image = remove_green_fringe(image)
            image.save(animation_dir / f"{index:02}.png")


def build_character(character_dir: Path, version: str | None, disable_sway: bool) -> None:
    manifest = load_manifest(character_dir)
    warn_missing_boot(character_dir, manifest)
    layer_paths = ensure_layers(character_dir)
    build_frames(character_dir, layer_paths, disable_sway)
    manifest["animations"] = ANIMATIONS
    if version:
        manifest["assetVersion"] = f"{version}-{manifest.get('id', character_dir.name)}"
    write_manifest(character_dir, manifest)
    print(f"[build-character-animations] {character_dir.name} を生成しました")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="characters/<id>/layers から連番アニメーションを生成します。"
    )
    parser.add_argument("characters", nargs="*", help="対象キャラクターID。省略時は全キャラ。")
    parser.add_argument("--version", help="assetVersion の先頭値（例: 2026-06-28-motion-v1）。")
    parser.add_argument("--disable-sway", action="store_true", help="髪・上半身の微小揺れを無効化する。")
    args = parser.parse_args()

    for character_dir in character_dirs(args.characters):
        build_character(character_dir, args.version, args.disable_sway)


if __name__ == "__main__":
    main()
