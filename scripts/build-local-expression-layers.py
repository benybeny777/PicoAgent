#!/usr/bin/env python3
"""Build pixel-stable facial layers from full-canvas generated references."""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter


EYE_BOXES = ((382, 326, 508, 416), (515, 320, 643, 410))
MOUTH_BOX = (457, 442, 612, 554)
IRIS_BOXES = ((424, 344, 474, 397), (546, 338, 598, 393))


def feathered_mask(size: tuple[int, int], boxes: tuple[tuple[int, int, int, int], ...]) -> Image.Image:
    """Create a softly feathered union of elliptical facial regions."""
    mask = Image.new("L", size, 0)
    draw = ImageDraw.Draw(mask)
    for box in boxes:
        draw.ellipse(box, fill=255)
    return mask.filter(ImageFilter.GaussianBlur(radius=9.0))


def composite_local(base: Image.Image, edited: Image.Image, mask: Image.Image, amount: float = 1.0) -> Image.Image:
    """Composite an edit inside a fixed mask, preserving every outside pixel."""
    if edited.size != base.size:
        raise ValueError(f"size mismatch: {edited.size} != {base.size}")
    if amount < 1.0:
        edited = Image.blend(base, edited, amount)
    return Image.composite(edited, base, mask)


def gaze_layer(base: Image.Image, horizontal_offset: int) -> Image.Image:
    """Shift only the iris regions while leaving eyelids and face unchanged."""
    iris_mask = feathered_mask(base.size, IRIS_BOXES).filter(ImageFilter.GaussianBlur(radius=3.0))
    shifted = ImageChops.offset(base, horizontal_offset, 0)
    return Image.composite(shifted, base, iris_mask)


def main() -> int:
    """Create blink and mouth layers for the approved 1024x1536 master."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path)
    parser.add_argument("blink", type=Path)
    parser.add_argument("mouth", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    with Image.open(args.base) as image:
        base = image.convert("RGBA")
    with Image.open(args.blink) as image:
        blink = image.convert("RGBA")
    with Image.open(args.mouth) as image:
        mouth = image.convert("RGBA")
    if base.size != (1024, 1536):
        raise ValueError(f"unexpected approved master size: {base.size}")

    args.output.mkdir(parents=True, exist_ok=True)
    eye_mask = feathered_mask(base.size, EYE_BOXES)
    mouth_mask = feathered_mask(base.size, (MOUTH_BOX,))
    happy = composite_local(base, mouth, mouth_mask)
    happy = composite_local(happy, blink, eye_mask, 0.32)
    layers = {
        "base.png": base,
        "mouth_closed.png": base,
        "blink_half.png": composite_local(base, blink, eye_mask, 0.46),
        "blink_closed.png": composite_local(base, blink, eye_mask),
        "mouth_open.png": composite_local(base, mouth, mouth_mask),
        "gaze_left.png": gaze_layer(base, -4),
        "gaze_right.png": gaze_layer(base, 4),
        "expression_happy.png": happy,
        "expression_sad.png": composite_local(base, blink, eye_mask, 0.58),
    }
    for name, layer in layers.items():
        layer.save(args.output / name, optimize=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
