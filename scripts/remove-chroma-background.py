#!/usr/bin/env python3
"""Remove an edge-connected green screen without resizing the source image."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter


def _green_score(rgb: np.ndarray) -> np.ndarray:
    """Return a 0..1 score for chroma-green pixels."""
    values = rgb.astype(np.float32) / 255.0
    red, green, blue = values[..., 0], values[..., 1], values[..., 2]
    dominance = green - np.maximum(red, blue)
    saturation = np.maximum(values.max(axis=2) - values.min(axis=2), 0.0)
    return np.clip((dominance - 0.08) / 0.34, 0.0, 1.0) * np.clip(
        (saturation - 0.14) / 0.52, 0.0, 1.0
    )


def remove_green_screen(source: Image.Image) -> tuple[Image.Image, dict[str, object]]:
    """Return a transparent RGBA image and a small QA report."""
    rgba = np.asarray(source.convert("RGBA")).copy()
    rgb = rgba[..., :3]
    score = _green_score(rgb)
    # This workflow uses a dedicated chroma backdrop and the approved subject
    # has no intentional green details. Remove enclosed backdrop islands too
    # (for example the gap between an arm and the torso), not only pixels that
    # happen to stay connected to the canvas edge after generation.
    matte = score
    alpha = np.rint(255.0 * (1.0 - np.clip(matte * 1.28, 0.0, 1.0))).astype(np.uint8)
    alpha[score >= 0.72] = 0

    # Decontaminate semi-transparent and just-inside fringe pixels. Chroma-key
    # compression often leaves a fully opaque green line disconnected from the
    # background, so alpha-only detection is insufficient at runtime scale.
    min_alpha = np.asarray(
        Image.fromarray(alpha, mode="L").filter(ImageFilter.MinFilter(size=9))
    )
    near_transparent = (alpha > 0) & (min_alpha < 255)
    green_dominance = rgb[..., 1].astype(np.int16) - np.maximum(
        rgb[..., 0].astype(np.int16), rgb[..., 2].astype(np.int16)
    )
    fringe = ((alpha > 0) & (alpha < 255)) | ((alpha > 0) & (green_dominance > 4))
    neutral_green = np.maximum(rgb[..., 0], rgb[..., 2]).astype(np.int16)
    corrected_green = np.minimum(rgb[..., 1].astype(np.int16), neutral_green + 2)
    rgb[..., 1] = np.where(fringe, corrected_green, rgb[..., 1]).astype(np.uint8)
    strong_edge_spill = (alpha > 0) & (green_dominance > 36)
    alpha[strong_edge_spill] = np.minimum(
        alpha[strong_edge_spill],
        np.clip(255 - (green_dominance[strong_edge_spill] - 24) * 5, 0, 255).astype(np.uint8),
    )
    # Give the silhouette a subpixel matte instead of a binary cut. Keep the
    # fully opaque interior intact so the subject is never softened.
    matte_image = Image.fromarray(alpha, mode="L").filter(ImageFilter.GaussianBlur(radius=0.7))
    softened = np.asarray(matte_image)
    alpha = np.where(alpha == 255, 255, softened).astype(np.uint8)

    # Extend edge colours into fully transparent pixels. This prevents a green
    # or black fringe when the runtime scales the straight-alpha PNG.
    colour = rgb.astype(np.float32)
    # Start from nearly opaque subject colours, then propagate them through the
    # feather and a few transparent pixels. Starting from every alpha>0 pixel
    # would preserve the yellow-green mixture already present in the fringe.
    known = alpha >= 250
    for _ in range(8):
        neighbours = np.zeros_like(colour)
        counts = np.zeros(alpha.shape, dtype=np.float32)
        for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1)):
            shifted_known = np.roll(known, (dy, dx), axis=(0, 1))
            shifted_colour = np.roll(colour, (dy, dx), axis=(0, 1))
            neighbours += shifted_colour * shifted_known[..., None]
            counts += shifted_known
        fill = (~known) & (counts > 0)
        colour[fill] = neighbours[fill] / counts[fill, None]
        known[fill] = True
    rgb[...] = np.clip(colour, 0, 255).astype(np.uint8)
    rgba[..., 3] = alpha

    corners = [int(alpha[0, 0]), int(alpha[0, -1]), int(alpha[-1, 0]), int(alpha[-1, -1])]
    report: dict[str, object] = {
        "width": source.width,
        "height": source.height,
        "resized": False,
        "transparentPixels": int(np.count_nonzero(alpha == 0)),
        "semiTransparentPixels": int(np.count_nonzero((alpha > 0) & (alpha < 255))),
        "opaquePixels": int(np.count_nonzero(alpha == 255)),
        "cornerAlpha": corners,
        "cornersTransparent": all(value == 0 for value in corners),
    }
    return Image.fromarray(rgba, mode="RGBA"), report


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--report", type=Path)
    return parser.parse_args()


def main() -> int:
    """Remove the chroma background and optionally write a JSON QA report."""
    args = parse_args()
    with Image.open(args.source) as source:
        output, report = remove_green_screen(source)
    args.destination.parent.mkdir(parents=True, exist_ok=True)
    output.save(args.destination, optimize=True)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False))
    return 0 if report["cornersTransparent"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
