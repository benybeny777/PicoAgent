#!/usr/bin/env python3
# build-character-animations.py - キャラクター表情レイヤーから連番アニメーションを生成する。

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import shutil
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
CHARACTERS_DIR = ROOT / "characters"

ANIMATIONS = {
    "idle": {
        "path": "animations/idle",
        "frames": 48,
        "fps": 12.0,
        "motion": "live2d-idle",
    },
    "thinking": {
        "path": "animations/running",
        "frames": 36,
        "fps": 12.0,
        "motion": "live2d-thinking",
    },
    "talking": {
        "path": "animations/review",
        "frames": 42,
        "fps": 15.0,
        "motion": "live2d-talking",
    },
    "sad": {
        "path": "animations/failed",
        "frames": 24,
        "fps": 12.0,
        "motion": "live2d-sad",
    },
}

BLINK_STAGES = [
    "blink_17", "blink_33", "blink_half", "blink_67", "blink_83", "blink_closed",
    "blink_closed", "blink_83", "blink_67", "blink_half", "blink_33", "blink_17",
]
MOUTH_STAGES = [
    "mouth_closed", "mouth_17", "mouth_33", "mouth_half", "mouth_67", "mouth_83",
    "mouth_open", "mouth_83", "mouth_67", "mouth_half", "mouth_33", "mouth_17",
]

# 旧パックの移行時だけ使う顔の局所領域。値はキャンバスに対する x, y, width, height の比率。
# 新規picoagent-portrait-2d-v1は完成済み22レイヤーを必須とし、この自動生成を通さない。
LEGACY_FACE_REGIONS = {
    "koharu": {"eyes": (0.32, 0.15, 0.36, 0.07), "mouth": (0.40, 0.20, 0.20, 0.07)},
    "mai": {"eyes": (0.20, 0.14, 0.50, 0.14), "mouth": (0.36, 0.24, 0.24, 0.08)},
    "mugi": {"eyes": (0.27, 0.20, 0.46, 0.13), "mouth": (0.42, 0.29, 0.18, 0.08)},
    "noa": {"eyes": (0.36, 0.13, 0.31, 0.09), "mouth": (0.43, 0.19, 0.16, 0.07)},
    "photoreal-prototype": {
        "eyes": (0.34, 0.18, 0.34, 0.12),
        "mouth": (0.44, 0.275, 0.15, 0.085),
    },
    "rin": {"eyes": (0.23, 0.29, 0.39, 0.045), "mouth": (0.39, 0.34, 0.09, 0.045)},
    "rin-tha3": {"eyes": (0.23, 0.29, 0.39, 0.045), "mouth": (0.38, 0.34, 0.08, 0.04)},
    "sena": {"eyes": (0.36, 0.16, 0.31, 0.10), "mouth": (0.43, 0.22, 0.16, 0.07)},
}
LEGACY_DIRECT_BLINK: set[str] = set()
LEGACY_ALIGN_TO_BASE: set[str] = set()
LEGACY_SPLIT_EYE_REGIONS = {
    "rin": [(0.23, 0.29, 0.17, 0.045), (0.45, 0.29, 0.17, 0.045)],
    "rin-tha3": [(0.23, 0.29, 0.17, 0.045), (0.45, 0.29, 0.17, 0.045)],
}
LEGACY_FACE_MIGRATION = "roi-flow-v1"


def staged_sequence(names: list[str], offset: int = 0) -> list[tuple[str, int]]:
    return [(name, ((index + offset) % 7) - 3) for index, name in enumerate(names)]


SEQUENCES = {
    "idle": (
        staged_sequence(["base"] * 6)
        + staged_sequence(BLINK_STAGES)
        + staged_sequence(["base"] * 6, 2)
        + staged_sequence(BLINK_STAGES, 3)
        + staged_sequence(["base"] * 12, 4)
    ),
    "thinking": (
        staged_sequence(["gaze_left"] * 6)
        + staged_sequence(BLINK_STAGES)
        + staged_sequence(["gaze_right"] * 6, 2)
        + staged_sequence(BLINK_STAGES, 3)
    ),
    "talking": [
        (
            ["mouth_closed", "mouth_17", "mouth_33", "mouth_half", "mouth_67", "mouth_83", "mouth_open"]
            [step if cycle % 2 == 0 else 6 - step],
            cycle - 3,
        )
        for cycle in range(6)
        for step in range(7)
    ],
    "sad": (
        staged_sequence(["expression_sad"] * 6)
        + staged_sequence(BLINK_STAGES)
        + staged_sequence(["expression_sad"] * 6, 3)
    ),
}

# 待機中の7段階瞬きは1往復約240msとし、通常姿勢はループ全体が約4秒になるよう保持する。
ANIMATIONS["idle"]["frameDurations"] = [
    (36 if layer == "blink_closed" else 20) if layer.startswith("blink_") else 147
    for layer, _sway in SEQUENCES["idle"]
]

# THA3で追加キーポーズが揃っているキャラだけが使う滑らかなフレーム構成。
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

# 専用キーポーズも通常2Dと同じ総コマ数へ揃え、同じ絵の単純複製にはしない。
# 各キーポーズへ異なる微小な横揺れを焼き込み、ランタイム物理なしで髪と上半身を動かす。
def expand_tha3_sequence(frames: list[tuple[str, int]], count: int) -> list[tuple[str, int]]:
    """元のポーズ順を保ち、同じポーズの出現ごとに異なる揺れ量を割り当てる。"""
    occurrences: dict[str, int] = {}
    expanded: list[tuple[str, int]] = []
    for index in range(count):
        layer_name = frames[index % len(frames)][0]
        occurrence = occurrences.get(layer_name, 0)
        occurrences[layer_name] = occurrence + 1
        expanded.append((layer_name, (occurrence % 9) - 4))
    return expanded


THA3_SEQUENCES = {
    name: expand_tha3_sequence(frames, ANIMATIONS[name]["frames"])
    for name, frames in THA3_SEQUENCES.items()
}

THA3_LAYER_NAMES = {
    layer_name
    for frames in THA3_SEQUENCES.values()
    for layer_name, _sway in frames
    if layer_name not in {"base", *BLINK_STAGES, *MOUTH_STAGES}
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

PORTRAIT_FORMAT_PATH = CHARACTERS_DIR / "portrait-2d-format.json"
PORTRAIT_REQUIRED_LAYERS = json.loads(PORTRAIT_FORMAT_PATH.read_text(encoding="utf-8"))["layers"]


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
            raise TypeError(f"animations.{animation_name} はオブジェクトで指定してください")
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


def normalized_box(size: tuple[int, int], region: tuple[float, float, float, float]) -> tuple[int, int, int, int]:
    width, height = size
    x, y, region_width, region_height = region
    left = max(0, round(width * x))
    top = max(0, round(height * y))
    right = min(width, round(width * (x + region_width)))
    bottom = min(height, round(height * (y + region_height)))
    if right - left < 8 or bottom - top < 8:
        raise ValueError(f"顔の局所領域が小さすぎます: {region}")
    return left, top, right, bottom


def optical_flow_patch(
    left: np.ndarray,
    right: np.ndarray,
    ratio: float,
    crossfade: bool,
) -> np.ndarray:
    """RGBA局所パッチを双方向flowで位置合わせして中間化する。"""
    if left.shape != right.shape:
        raise ValueError("局所パッチの寸法が一致しません")
    if ratio <= 0:
        return left.copy()
    if ratio >= 1:
        return right.copy()
    left_gray = cv2.cvtColor(left[:, :, :3], cv2.COLOR_RGB2GRAY)
    right_gray = cv2.cvtColor(right[:, :, :3], cv2.COLOR_RGB2GRAY)
    flow_lr = cv2.calcOpticalFlowFarneback(left_gray, right_gray, None, 0.5, 5, 31, 5, 7, 1.5, 0)
    flow_rl = cv2.calcOpticalFlowFarneback(right_gray, left_gray, None, 0.5, 5, 31, 5, 7, 1.5, 0)
    if not np.isfinite(flow_lr).all() or not np.isfinite(flow_rl).all():
        raise ValueError("局所変形のflowにNaNまたは無限値があります")
    height, width = left.shape[:2]
    grid_x, grid_y = np.meshgrid(np.arange(width, dtype=np.float32), np.arange(height, dtype=np.float32))

    flow_lengths = np.maximum(np.linalg.norm(flow_lr, axis=2), np.linalg.norm(flow_rl, axis=2))
    max_flow = float(flow_lengths.max())
    relative_limit = 0.25 * float(np.hypot(width, height))
    flow_limit = min(50.0, relative_limit)
    if max_flow > flow_limit:
        raise ValueError(
            f"局所変形のflowが上限を超えました: {max_flow:.1f}px / {flow_limit:.1f}px"
        )

    reverse_x = grid_x + flow_lr[:, :, 0]
    reverse_y = grid_y + flow_lr[:, :, 1]
    sampled_rl = cv2.remap(
        flow_rl,
        reverse_x,
        reverse_y,
        cv2.INTER_LINEAR,
        borderMode=cv2.BORDER_CONSTANT,
        borderValue=0,
    )
    valid = (
        (reverse_x >= 0)
        & (reverse_x <= width - 1)
        & (reverse_y >= 0)
        & (reverse_y <= height - 1)
    )
    consistency = np.linalg.norm(flow_lr + sampled_rl, axis=2)
    # 閉眼のようにテクスチャが消える遮蔽を含むため15%までは許容する。
    # 大振幅は別の50px／ROI対角25%上限でも停止する。
    consistency_limit = max(2.0, 0.15 * float(np.hypot(width, height)))
    if valid.any() and float(np.percentile(consistency[valid], 95)) > consistency_limit + 0.25:
        raise ValueError(
            "局所変形の双方向flowが整合しません: "
            f"p95={float(np.percentile(consistency[valid], 95)):.2f}px / "
            f"{consistency_limit:.2f}px"
        )

    def warp(image: np.ndarray, flow: np.ndarray, amount: float) -> np.ndarray:
        map_x = grid_x - flow[:, :, 0] * amount
        map_y = grid_y - flow[:, :, 1] * amount
        rgba = image.astype(np.float32) / 255.0
        alpha = rgba[:, :, 3:4]
        premultiplied = np.concatenate((rgba[:, :, :3] * alpha, alpha), axis=2)
        warped = cv2.remap(
            premultiplied,
            map_x,
            map_y,
            cv2.INTER_LINEAR,
            borderMode=cv2.BORDER_REFLECT_101,
        )
        warped_alpha = warped[:, :, 3:4]
        rgb = np.divide(
            warped[:, :, :3],
            warped_alpha,
            out=np.zeros_like(warped[:, :, :3]),
            where=warped_alpha > 1 / 255,
        )
        return np.clip(np.concatenate((rgb, warped_alpha), axis=2) * 255.0, 0, 255)

    left_amount = 1.0 - (1.0 - ratio) ** 2 if not crossfade else ratio
    from_left = warp(left, flow_lr, left_amount).astype(np.float32) / 255.0
    from_right = warp(right, flow_rl, 1.0 - ratio).astype(np.float32) / 255.0
    if not crossfade:
        # 閉眼では虹彩が消えるため、全画素の単純crossfadeは二重まぶたになる。
        # 遮蔽の前後で片側warpを選び、半透明の虹彩を残さない。
        selected = from_left if ratio < 0.5 else from_right
        return np.clip(selected * 255.0, 0, 255).astype(np.uint8)
    left_alpha = from_left[:, :, 3:4]
    right_alpha = from_right[:, :, 3:4]
    mixed_alpha = left_alpha * (1.0 - ratio) + right_alpha * ratio
    mixed_premultiplied = (
        from_left[:, :, :3] * left_alpha * (1.0 - ratio)
        + from_right[:, :, :3] * right_alpha * ratio
    )
    mixed_rgb = np.divide(
        mixed_premultiplied,
        mixed_alpha,
        out=np.zeros_like(mixed_premultiplied),
        where=mixed_alpha > 1 / 255,
    )
    return np.clip(
        np.concatenate((mixed_rgb, mixed_alpha), axis=2) * 255.0, 0, 255
    ).astype(np.uint8)


def alpha_boundary_score(patch: np.ndarray) -> tuple[int, float]:
    """半透明画素数とalpha勾配を返し、warpによる境界悪化を検出する。"""
    alpha = patch[:, :, 3].astype(np.float32)
    semi_transparent = int(((alpha > 0) & (alpha < 255)).sum())
    gradient_x = np.abs(np.diff(alpha, axis=1)).sum()
    gradient_y = np.abs(np.diff(alpha, axis=0)).sum()
    return semi_transparent, float(gradient_x + gradient_y)


def align_subject_bbox(reference: Image.Image, source: Image.Image) -> Image.Image:
    """旧終点の被写体bboxをbaseへ合わせ、ROIへ全身ずれを持ち込まない。"""
    reference_array = np.asarray(reference.convert("RGBA"), dtype=np.uint8)
    source_array = np.asarray(source.convert("RGBA"), dtype=np.uint8)

    def alpha_box(array: np.ndarray) -> tuple[int, int, int, int]:
        ys, xs = np.where(array[:, :, 3] > 40)
        if not len(xs):
            raise ValueError("旧素材に不透明な被写体がありません")
        return int(xs.min()), int(ys.min()), int(xs.max() + 1), int(ys.max() + 1)

    ref_box = alpha_box(reference_array)
    src_box = alpha_box(source_array)
    scale_x = (ref_box[2] - ref_box[0]) / (src_box[2] - src_box[0])
    scale_y = (ref_box[3] - ref_box[1]) / (src_box[3] - src_box[1])
    if abs(scale_x - scale_y) > 0.025:
        raise ValueError(f"旧素材の縦横比がbaseと一致しません: {scale_x:.4f}/{scale_y:.4f}")
    scale = (scale_x + scale_y) / 2.0
    if not 0.95 <= scale <= 1.05:
        raise ValueError(f"旧素材の位置合わせ倍率が許容外です: {scale:.4f}")
    src_center_x = (src_box[0] + src_box[2]) / 2.0
    src_center_y = (src_box[1] + src_box[3]) / 2.0
    ref_center_x = (ref_box[0] + ref_box[2]) / 2.0
    ref_center_y = (ref_box[1] + ref_box[3]) / 2.0
    matrix = np.array(
        [
            [scale, 0.0, ref_center_x - scale * src_center_x],
            [0.0, scale, ref_center_y - scale * src_center_y],
        ],
        dtype=np.float32,
    )
    rgba = source_array.astype(np.float32) / 255.0
    alpha = rgba[:, :, 3:4]
    premultiplied = np.concatenate((rgba[:, :, :3] * alpha, alpha), axis=2)
    width, height = source.size
    warped = cv2.warpAffine(
        premultiplied,
        matrix,
        (width, height),
        flags=cv2.INTER_LINEAR,
        borderMode=cv2.BORDER_CONSTANT,
        borderValue=0,
    )
    warped_alpha = warped[:, :, 3:4]
    rgb = np.divide(
        warped[:, :, :3],
        warped_alpha,
        out=np.zeros_like(warped[:, :, :3]),
        where=warped_alpha > 1 / 255,
    )
    output = np.clip(np.concatenate((rgb, warped_alpha), axis=2) * 255.0, 0, 255).astype(np.uint8)
    return Image.fromarray(output, "RGBA")


def local_morph_layers(
    base: Image.Image,
    left: Image.Image,
    right: Image.Image,
    region: tuple[float, float, float, float],
    ratios: list[float],
    crossfade: bool = True,
) -> list[Image.Image]:
    """ROI外をbaseと完全一致させ、境界だけをROI内側へフェザーして合成する。"""
    if base.size != left.size or base.size != right.size:
        raise ValueError("顔差分のキャンバス寸法が一致しません")
    box = normalized_box(base.size, region)
    base_array = np.asarray(base.convert("RGBA"), dtype=np.uint8)
    left_array = np.asarray(left.convert("RGBA"), dtype=np.uint8)[box[1] : box[3], box[0] : box[2]]
    right_array = np.asarray(right.convert("RGBA"), dtype=np.uint8)[box[1] : box[3], box[0] : box[2]]
    patch_height, patch_width = left_array.shape[:2]
    feather = max(2, min(10, patch_height // 10, patch_width // 10))
    mask = np.ones((patch_height, patch_width), dtype=np.float32)
    ramp = np.linspace(0.0, 1.0, feather, dtype=np.float32)
    mask[:feather, :] *= ramp[:, None]
    mask[-feather:, :] *= ramp[::-1, None]
    mask[:, :feather] *= ramp[None, :]
    mask[:, -feather:] *= ramp[None, ::-1]
    mask = mask[:, :, None]
    outputs: list[Image.Image] = []
    previous_progress = -1.0
    left_alpha_score = alpha_boundary_score(left_array)
    right_alpha_score = alpha_boundary_score(right_array)
    endpoint_alpha_limit = (
        max(left_alpha_score[0], right_alpha_score[0]),
        max(left_alpha_score[1], right_alpha_score[1]),
    )
    for ratio in ratios:
        patch = optical_flow_patch(left_array, right_array, ratio, crossfade)
        output = base_array.copy()
        base_patch = output[box[1] : box[3], box[0] : box[2]].astype(np.float32)
        merged = base_patch * (1.0 - mask) + patch.astype(np.float32) * mask
        output[box[1] : box[3], box[0] : box[2]] = np.clip(merged, 0, 255).astype(np.uint8)
        outside = output.copy()
        outside[box[1] : box[3], box[0] : box[2]] = base_array[box[1] : box[3], box[0] : box[2]]
        if not np.array_equal(outside, base_array):
            raise ValueError("局所変形がROI外の画素を変更しました")
        alpha_score = alpha_boundary_score(patch)
        # サブピクセルwarpは正常でも半透明画素を増やすため4倍までは許容し、
        # alpha勾配の増加と組み合わせて境界の肥大・ハローを停止する。
        semi_limit = endpoint_alpha_limit[0] * 4 + 32
        gradient_limit = endpoint_alpha_limit[1] * 1.10 + 512.0
        if alpha_score[0] > semi_limit or alpha_score[1] > gradient_limit:
            raise ValueError(
                "局所変形でalpha境界が悪化しました: "
                f"半透明={alpha_score[0]}/{semi_limit}, 勾配={alpha_score[1]:.1f}/{gradient_limit:.1f}"
            )
        visible_patch = output[box[1] : box[3], box[0] : box[2]]
        visible_left = base_array[box[1] : box[3], box[0] : box[2]].astype(np.float32) * (1.0 - mask)
        visible_left += left_array.astype(np.float32) * mask
        visible_right = base_array[box[1] : box[3], box[0] : box[2]].astype(np.float32) * (1.0 - mask)
        visible_right += right_array.astype(np.float32) * mask
        direction = visible_right - visible_left
        denominator = float(np.square(direction).sum())
        if denominator <= 0:
            raise ValueError("局所変形の始点と終点が同一です")
        progress = float(((visible_patch.astype(np.float32) - visible_left) * direction).sum()) / denominator
        if progress <= previous_progress + 1e-5:
            raise ValueError("局所変形の段階差分が単調に増加していません")
        previous_progress = progress
        outputs.append(Image.fromarray(output, "RGBA"))
    return outputs


def local_morph_multiple_regions(
    base: Image.Image,
    left: Image.Image,
    right: Image.Image,
    regions: list[tuple[float, float, float, float]],
    ratios: list[float],
) -> list[Image.Image]:
    """左右眼など独立したROIを別々にflow計算し、同じ段階へ合成する。"""
    region_outputs = [
        local_morph_layers(base, left, right, region, ratios, crossfade=False)
        for region in regions
    ]
    combined: list[Image.Image] = []
    try:
        for index in range(len(ratios)):
            output = base.convert("RGBA")
            for region, outputs in zip(regions, region_outputs, strict=True):
                box = normalized_box(base.size, region)
                output.paste(outputs[index].crop(box), box)
            combined.append(output)
    finally:
        for outputs in region_outputs:
            for output in outputs:
                output.close()
    return combined


def region_mean_difference(
    left: Image.Image,
    right: Image.Image,
    region: tuple[float, float, float, float],
) -> float:
    box = normalized_box(left.size, region)
    left_array = np.asarray(left.convert("RGBA"), dtype=np.int16)[box[1] : box[3], box[0] : box[2]]
    right_array = np.asarray(right.convert("RGBA"), dtype=np.int16)[box[1] : box[3], box[0] : box[2]]
    return float(np.abs(left_array - right_array).mean())


def synthesize_open_mouth(
    source: Image.Image,
    region: tuple[float, float, float, float],
) -> Image.Image:
    """口差分が無い旧試作だけに、局所的な開口終点を作る。"""
    box = normalized_box(source.size, region)
    output = np.asarray(source.convert("RGBA"), dtype=np.uint8).copy()
    patch = output[box[1] : box[3], box[0] : box[2]].copy()
    height, width = patch.shape[:2]
    center_x, center_y = width // 2, height // 2
    radius_x = max(6, round(width * 0.18))
    radius_y = max(4, round(height * 0.12))
    overlay = patch.copy()
    cv2.ellipse(overlay, (center_x, center_y), (radius_x, radius_y), 0, 0, 360, (75, 24, 35, 255), -1)
    cv2.ellipse(
        overlay,
        (center_x, center_y + radius_y // 2),
        (max(3, radius_x - 3), max(2, radius_y // 2)),
        0,
        0,
        180,
        (205, 92, 112, 255),
        -1,
    )
    mask = np.zeros((height, width), dtype=np.uint8)
    cv2.ellipse(mask, (center_x, center_y), (radius_x + 3, radius_y + 3), 0, 0, 360, 255, -1)
    mask = cv2.GaussianBlur(mask, (0, 0), sigmaX=max(1.0, radius_y / 3))[:, :, None] / 255.0
    patch = np.clip(patch * (1.0 - mask) + overlay * mask, 0, 255).astype(np.uint8)
    output[box[1] : box[3], box[0] : box[2]] = patch
    return Image.fromarray(output, "RGBA")


def ensure_smooth_face_layers(
    character_dir: Path,
    layer_paths: dict[str, Path],
    source_root: Path | None,
) -> None:
    """旧2～3段階素材を、顔以外を固定した目7段階・口7段階へ移行する。"""
    layers_dir = character_dir / "layers"
    regions = LEGACY_FACE_REGIONS.get(character_dir.name)
    if regions is None:
        raise ValueError(
            f"{character_dir.name} は旧素材移行用のeyes/mouth局所領域が未定義です。"
            "自動推定せず、完成済み22レイヤーを用意してください。"
        )
    source_dir = source_root / character_dir.name / "layers" if source_root else layers_dir
    required_sources = ["base", "blink_half", "blink_closed", "mouth_closed", "mouth_open"]
    missing_sources = [name for name in required_sources if not (source_dir / f"{name}.png").is_file()]
    if missing_sources:
        raise FileNotFoundError(
            f"{character_dir.name} の旧素材移行入力が不足しています: "
            + ", ".join(missing_sources)
        )
    with (
        Image.open(source_dir / "base.png") as base_opened,
        Image.open(source_dir / "blink_half.png") as half_opened,
        Image.open(source_dir / "blink_closed.png") as closed_opened,
        Image.open(source_dir / "mouth_closed.png") as mouth_closed_opened,
        Image.open(source_dir / "mouth_open.png") as mouth_open_opened,
    ):
        base = base_opened.convert("RGBA")
        blink_half = half_opened.convert("RGBA")
        blink_closed = closed_opened.convert("RGBA")
        mouth_closed = mouth_closed_opened.convert("RGBA")
        mouth_open = mouth_open_opened.convert("RGBA")
    try:
        if character_dir.name in LEGACY_ALIGN_TO_BASE:
            aligned_half = align_subject_bbox(base, blink_half)
            blink_half.close()
            blink_half = aligned_half
            aligned_closed = align_subject_bbox(base, blink_closed)
            blink_closed.close()
            blink_closed = aligned_closed
            aligned_mouth_closed = align_subject_bbox(base, mouth_closed)
            mouth_closed.close()
            mouth_closed = aligned_mouth_closed
            aligned_mouth_open = align_subject_bbox(base, mouth_open)
            mouth_open.close()
            mouth_open = aligned_mouth_open
        eye_regions = LEGACY_SPLIT_EYE_REGIONS.get(character_dir.name, [regions["eyes"]])
        if (
            region_mean_difference(blink_half, blink_closed, regions["eyes"]) < 0.5
            or character_dir.name in LEGACY_DIRECT_BLINK
        ):
            # フォールバック許可: 独立した半目が無いか、旧半目の位置が閉眼と整合しない場合だけ直接6等分する。
            print(
                f"[build-character-animations] 警告: {character_dir.name} は独立した半目がないため"
                "開眼と閉眼から7段階へ移行します"
            )
            eye_images = local_morph_multiple_regions(
                base,
                base,
                blink_closed,
                eye_regions,
                [1 / 6, 2 / 6, 3 / 6, 4 / 6, 5 / 6, 1.0],
            )
        else:
            eye_first = local_morph_multiple_regions(
                base, base, blink_half, eye_regions, [1 / 3, 2 / 3, 1.0]
            )
            eye_second = local_morph_multiple_regions(
                base, blink_half, blink_closed, eye_regions, [1 / 3, 2 / 3, 1.0]
            )
            eye_images = [*eye_first, *eye_second]
        eye_names = ["blink_17", "blink_33", "blink_half", "blink_67", "blink_83", "blink_closed"]
        if region_mean_difference(mouth_closed, mouth_open, regions["mouth"]) < 0.5:
            # フォールバック許可: 旧実験素材に開口終点が無い場合だけ、口ROI内へ終点を局所描画する。
            print(
                f"[build-character-animations] 警告: {character_dir.name} は開口差分がないため"
                "口の局所領域だけに開口終点を作ります"
            )
            mouth_open.close()
            mouth_open = synthesize_open_mouth(mouth_closed, regions["mouth"])
        mouth_images = local_morph_layers(
            base,
            mouth_closed,
            mouth_open,
            regions["mouth"],
            [0.0, 1 / 6, 2 / 6, 3 / 6, 4 / 6, 5 / 6, 1.0],
        )
        mouth_names = [
            "mouth_closed", "mouth_17", "mouth_33", "mouth_half", "mouth_67", "mouth_83", "mouth_open"
        ]
        for name, image in zip(eye_names, eye_images, strict=True):
            destination = layers_dir / f"{name}.png"
            image.save(destination)
            image.close()
            layer_paths[name] = destination
        for name, image in zip(mouth_names, mouth_images, strict=True):
            destination = layers_dir / f"{name}.png"
            image.save(destination)
            image.close()
            layer_paths[name] = destination
        for names in (eye_names, mouth_names):
            digests = {file_digest(layers_dir / f"{name}.png") for name in names}
            if len(digests) != len(names):
                raise ValueError(f"{character_dir.name} の{names[0]}系列に同一内容の段階があります")
    finally:
        base.close()
        blink_half.close()
        blink_closed.close()
        mouth_closed.close()
        mouth_open.close()


def ensure_layers(
    character_dir: Path,
    manifest: dict,
    legacy_source_root: Path | None,
    force_legacy_migration: bool,
) -> dict[str, Path]:
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

    if manifest.get("characterFormat") == "picoagent-portrait-2d-v1":
        missing = [
            name for name in PORTRAIT_REQUIRED_LAYERS
            if not (layers_dir / f"{name}.png").is_file()
        ]
        if missing:
            raise FileNotFoundError(
                "picoagent-portrait-2d-v1 の手制作22レイヤーが不足しています: "
                + ", ".join(missing)
            )
        return {name: layers_dir / f"{name}.png" for name in PORTRAIT_REQUIRED_LAYERS}

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
    smooth_names = ["base", *BLINK_STAGES[:6], *MOUTH_STAGES[:7]]
    if manifest.get("faceLayerMigration") == LEGACY_FACE_MIGRATION and not force_legacy_migration:
        missing = [name for name in smooth_names if not (layers_dir / f"{name}.png").is_file()]
        if missing:
            raise FileNotFoundError(
                f"{character_dir.name} の移行済み7段階素材が不足しています: " + ", ".join(missing)
            )
        layer_paths.update({name: layers_dir / f"{name}.png" for name in smooth_names})
        return layer_paths
    ensure_smooth_face_layers(character_dir, layer_paths, legacy_source_root)
    manifest["faceLayerMigration"] = LEGACY_FACE_MIGRATION
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
    # 上半身と髪を数pxだけ横にずらし、足元は固定する。
    for y in range(height):
        t = max(0.0, min(1.0, (0.76 * height - y) / (0.76 * height)))
        shift = round(amount * (0.45 + 1.15 * t)) if t > 0 else 0
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
    sequences = THA3_SEQUENCES if use_tha3_motion else SEQUENCES
    if use_tha3_motion:
        for name in THA3_LAYER_NAMES:
            layer_paths[name] = layers_dir / f"{name}.png"
        print(f"[build-character-animations] {character_dir.name} はTHA3の150フレーム構成を使います")
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


def build_character(
    character_dir: Path,
    version: str | None,
    disable_sway: bool,
    compact_only: bool,
    legacy_source_root: Path | None,
    force_legacy_migration: bool,
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
    layer_paths = ensure_layers(
        character_dir, manifest, legacy_source_root, force_legacy_migration
    )
    build_frames(character_dir, layer_paths, disable_sway, manifest.get("animationQualityMode"))
    manifest["animations"] = copy.deepcopy(ANIMATIONS)
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
    parser.add_argument(
        "--legacy-source-root",
        type=Path,
        help="旧素材移行のbase・目・口終点を持つ一時ルート。<root>/<id>/layers/*.pngを読む。",
    )
    parser.add_argument(
        "--force-legacy-migration",
        action="store_true",
        help="移行済みマーカーがあっても指定した旧原本から7段階を再評価する。",
    )
    args = parser.parse_args()

    targets = [args.character_dir.resolve()] if args.character_dir else character_dirs(args.characters)
    for character_dir in targets:
        build_character(
            character_dir,
            args.version,
            args.disable_sway,
            args.compact_only,
            args.legacy_source_root.resolve() if args.legacy_source_root else None,
            args.force_legacy_migration,
        )


if __name__ == "__main__":
    main()
