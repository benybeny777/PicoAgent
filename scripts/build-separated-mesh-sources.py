#!/usr/bin/env python3
"""通常2Dの同一座標原画から、微細変形用の分離原画を再構成する。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import cv2
import numpy as np
from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SETTINGS = {
    "mugi": (0.40, 0.36, 0.70, 0.16),
    "rin": (0.40, 0.38, 0.69, 0.17),
    "mai": (0.36, 0.34, 0.66, 0.18),
    "koharu": (0.25, 0.24, 0.59, 0.18),
    "sena": (0.29, 0.27, 0.59, 0.17),
    "noa": (0.30, 0.27, 0.59, 0.18),
    "rin-tha3": (0.40, 0.37, 0.70, 0.17),
    "photoreal-prototype": (0.37, 0.34, 1.00, 0.20),
}
COUPLED_HAIR_IDS = {"mugi", "photoreal-prototype"}


def write_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".json.part")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def normalized_masks(alpha: np.ndarray, settings: tuple[float, float, float, float]) -> dict[str, np.ndarray]:
    visible = alpha > 0
    ys, xs = np.nonzero(visible)
    if not len(xs):
        raise ValueError("基準画像に不透明画素がありません")
    left, right = int(xs.min()), int(xs.max()) + 1
    top, bottom = int(ys.min()), int(ys.max()) + 1
    width, height = right - left, bottom - top
    head_fraction, arm_top_fraction, arm_bottom_fraction, inner_gap_fraction = settings
    yy, xx = np.indices(alpha.shape)
    head_cut = top + round(height * head_fraction)
    arm_top = top + round(height * arm_top_fraction)
    arm_bottom = top + round(height * arm_bottom_fraction)
    center = (left + right) / 2
    gap = width * inner_gap_fraction
    head_progress = np.clip((yy - top) / max(1, head_cut - top), 0, 1)
    head_half_width = width * (0.43 - head_progress * 0.23)
    head = visible & (yy < head_cut) & (np.abs(xx - center) < head_half_width)
    left_arm = visible & ~head & (yy >= arm_top) & (yy < arm_bottom) & (xx < center - gap)
    right_arm = visible & ~head & (yy >= arm_top) & (yy < arm_bottom) & (xx > center + gap)
    body = visible & ~head & ~left_arm & ~right_arm
    return {"body": body, "arm-left": left_arm, "arm-right": right_arm, "head": head}


def split_head_hair(original: np.ndarray, head: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    ys, xs = np.nonzero(head)
    top, bottom = int(ys.min()), int(ys.max()) + 1
    left, right = int(xs.min()), int(xs.max()) + 1
    height, width = bottom - top, right - left
    pixels = original[head, :3].astype(np.float32)
    if len(pixels) > 50000:
        indices = np.linspace(0, len(pixels) - 1, 50000, dtype=np.int32)
        sample = pixels[indices]
    else:
        sample = pixels
    cv2.setRNGSeed(20260828)
    _, _, centers = cv2.kmeans(
        sample,
        6,
        None,
        (cv2.TERM_CRITERIA_EPS + cv2.TERM_CRITERIA_MAX_ITER, 40, 0.25),
        5,
        cv2.KMEANS_PP_CENTERS,
    )
    head_pixels = original[head, :3].astype(np.float32)
    distances = ((head_pixels[:, None, :] - centers[None, :, :]) ** 2).sum(axis=2)
    labels = np.full(head.shape, -1, dtype=np.int16)
    labels[head] = distances.argmin(axis=1)
    top_band = head & (np.indices(head.shape)[0] < top + max(4, round(height * 0.28)))
    counts = np.bincount(labels[top_band], minlength=len(centers))
    selected: set[int] = set()
    target = max(1, round(counts.sum() * 0.94))
    accumulated = 0
    for cluster in np.argsort(counts)[::-1]:
        selected.add(int(cluster))
        accumulated += int(counts[cluster])
        if accumulated >= target:
            break
    hair = head & np.isin(labels, list(selected))
    yy, xx = np.indices(head.shape)
    center = (left + right) / 2
    central_face = (np.abs(xx - center) < width * 0.27) & (yy >= top + height * 0.52)
    hair &= ~central_face
    front_zone = (np.abs(xx - center) < width * 0.29) & (yy < top + height * 0.58)
    hair_front = hair & front_zone
    hair_back = hair & ~hair_front
    face = head & ~hair
    if not all(np.any(mask) for mask in (hair_back, face, hair_front)):
        raise ValueError("頭部を後ろ髪・顔・前髪へ分離できません")
    return hair_back, face, hair_front


def add_joint_underlay(base: np.ndarray, original: np.ndarray, overlay: np.ndarray, extension: int) -> None:
    opaque_overlay = overlay & (original[:, :, 3] == 255)
    retained = base[:, :, 3] == 255
    kernel = np.ones((extension * 2 + 1, extension * 2 + 1), np.uint8)
    joint = (cv2.dilate(retained.astype(np.uint8), kernel) > 0) & opaque_overlay
    if not np.any(joint):
        return
    inpaint_mask = (joint.astype(np.uint8) * 255)
    filled = cv2.inpaint(original[:, :, :3], inpaint_mask, extension, cv2.INPAINT_TELEA)
    base[joint, :3] = filled[joint]
    base[joint, 3] = 255


def save_part(original: np.ndarray, mask: np.ndarray, path: Path) -> None:
    part = np.zeros_like(original)
    part[mask] = original[mask]
    if not np.any(part[:, :, 3]):
        raise ValueError(f"分離対象に画素がありません: {path.name}")
    path.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(part, "RGBA").save(path)


def verify_neutral(original: np.ndarray, ordered_paths: list[Path]) -> None:
    canvas = Image.new("RGBA", (original.shape[1], original.shape[0]), (0, 0, 0, 0))
    for path in ordered_paths:
        with Image.open(path) as opened:
            canvas.alpha_composite(opened.convert("RGBA"))
    if not np.array_equal(np.asarray(canvas), original):
        raise ValueError("分離原画の中立合成が基準画像と一致しません")


def build(character_id: str) -> None:
    if character_id not in SETTINGS:
        raise ValueError(f"分離設定がありません: {character_id}")
    character_dir = ROOT / "characters" / character_id
    base_path = character_dir / "layers" / "mouth_closed.png"
    original = np.array(Image.open(base_path).convert("RGBA"))
    original[original[:, :, 3] == 0, :3] = 0
    masks = normalized_masks(original[:, :, 3], SETTINGS[character_id])
    head_mask = masks.pop("head")
    hair_back, face, hair_front = split_head_hair(original, head_mask)
    masks.update({"hair-back": hair_back, "face": face, "hair-front": hair_front})
    source_dir = character_dir / "mesh-rig" / "source"
    parts_dir = source_dir / "parts"
    parts_dir.mkdir(parents=True, exist_ok=True)
    body = np.zeros_like(original)
    body[masks["body"]] = original[masks["body"]]
    for name in ("arm-left", "arm-right"):
        add_joint_underlay(body, original, masks[name], max(6, round(min(original.shape[:2]) * 0.008)))
    add_joint_underlay(body, original, head_mask, max(6, round(min(original.shape[:2]) * 0.008)))
    body_path = parts_dir / "body.png"
    Image.fromarray(body, "RGBA").save(body_path)
    ordered = [body_path]
    definitions = [{"name": "body", "source": "body.png", "z": 0, "motion": "torso", "hiddenAreaSource": True}]
    ordered_names = ("hair-back", "arm-left", "arm-right", "face", "hair-front")
    z_indexes = (5, 10, 11, 20, 30)
    source_arrays: dict[str, np.ndarray] = {}
    for name in ordered_names:
        source = np.zeros_like(original)
        source[masks[name]] = original[masks[name]]
        source_arrays[name] = source
    add_joint_underlay(source_arrays["hair-back"], original, masks["face"], max(5, round(min(original.shape[:2]) * 0.006)))
    add_joint_underlay(source_arrays["face"], original, masks["hair-front"], max(5, round(min(original.shape[:2]) * 0.006)))
    for z_index, name in zip(z_indexes, ordered_names, strict=True):
        path = parts_dir / f"{name}.png"
        if name in ("hair-back", "face"):
            Image.fromarray(source_arrays[name], "RGBA").save(path)
        else:
            save_part(original, masks[name], path)
        ordered.append(path)
        if character_id == "mai" and name == "face":
            motion = "fixed"
        elif character_id in COUPLED_HAIR_IDS and name in ("hair-back", "hair-front"):
            motion = "head"
        else:
            motion = name
        definitions.append({"name": name, "source": path.name, "z": z_index, "motion": motion, "hiddenAreaSource": True})
    expected_sources = {"body.png", *(f"{name}.png" for name in ordered_names)}
    for obsolete in parts_dir.glob("*.png"):
        if obsolete.name not in expected_sources:
            obsolete.unlink()
    verify_neutral(original, ordered)
    scale = 0.5 if max(original.shape[:2]) > 2400 else 1.0
    config = {
        "runtimeScale": scale,
        "grid": [9, 15],
        "autoFaceRegions": True,
        "separatedParts": definitions,
        "sourceMethod": "同一座標原画の可視画素分離と関節境界の局所再構成",
        "motionLimits": {
            "range": "微細変形のみ",
            "hairCoupledToHead": character_id in COUPLED_HAIR_IDS,
            "faceTranslationFixed": character_id == "mai",
        },
    }
    if character_id == "sena":
        config["gaze"] = {
            "irisBoxes": [[770, 630, 850, 735], [920, 610, 1010, 720]],
            "offset": 16,
            "maskFeather": 8,
            "box": [700, 560, 1050, 780],
            "feather": 24,
        }
    write_json(source_dir / "regions.json", config)
    print(f"[separated-source] {character_id}: {original.shape[1]}x{original.shape[0]} / 6パーツ")


def main() -> None:
    parser = argparse.ArgumentParser(description="通常2Dキャラの分離原画を同一座標で生成します")
    parser.add_argument("character_ids", nargs="+", choices=sorted(SETTINGS))
    args = parser.parse_args()
    for character_id in args.character_ids:
        build(character_id)


if __name__ == "__main__":
    main()
