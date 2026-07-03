# build-live2d-psd.py - characters/<id>/layers/ の差分絵から Live2D Cubism Editor 用の
# レイヤー分け PSD を生成する。
#
# 生成物: characters/<id>/live2d-src/<id>-live2d.psd
#
# レイヤー構成（上から。名前は PSD 互換性のため ASCII）:
#   eye/    … blink_closed・blink_half・eye_open（base の目領域切り抜き）
#   mouth/  … mouth_open・mouth_closed・mouth_base（base の口領域切り抜き）
#   base（全身）
# 差分絵が全身再描画で局所切り抜きできない場合は variants/ に全身レイヤーとして収録する。
#
# 目・口の領域は base と各差分絵のピクセル差分から自動検出する（ノイズは
# 収縮→膨張で除去し、領域に余白を足して切り抜く）。差分が広すぎて領域を
# 特定できない場合はその差分絵を全身レイヤーとして収録する。
#
# 使い方:
#   python scripts/build-live2d-psd.py            # layers/ を持つ全キャラ
#   python scripts/build-live2d-psd.py mugi rin   # 指定キャラのみ
#
# 依存: pip install Pillow psd-tools

import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageFilter
from psd_tools import PSDImage
from psd_tools.api.layers import Group, PixelLayer

ROOT = Path(__file__).resolve().parent.parent
CHARACTERS = ROOT / "characters"

# 差分とみなす輝度しきい値（0-255）と、切り抜き領域へ足す余白(px)。
DIFF_THRESHOLD = 28
BBOX_PADDING = 14
# 差分領域がキャンバスのこの割合を超えたら「局所パーツとして特定できない」とみなす。
MAX_REGION_RATIO = 0.4

LAYER_FILES = ["base", "mouth_open", "mouth_closed", "blink_half", "blink_closed"]


def load_layers(layers_dir: Path) -> dict[str, Image.Image] | None:
    images = {}
    for name in LAYER_FILES:
        path = layers_dir / f"{name}.png"
        if not path.exists():
            return None
        images[name] = Image.open(path).convert("RGBA")
    size = images["base"].size
    if any(img.size != size for img in images.values()):
        print(f"  スキップ: 差分絵のサイズが一致しません ({layers_dir})")
        return None
    return images


def diff_bbox(base: Image.Image, variant: Image.Image) -> tuple[int, int, int, int] | None:
    """base と差分絵のピクセル差分から変化領域の bbox を返す（ノイズ除去付き）。"""
    diff = ImageChops.difference(base, variant).convert("L")
    mask = diff.point(lambda v: 255 if v >= DIFF_THRESHOLD else 0)
    # 孤立ノイズを除去（収縮）してから領域を戻す（膨張）。
    mask = mask.filter(ImageFilter.MinFilter(3)).filter(ImageFilter.MaxFilter(5))
    return mask.getbbox()


def union_bbox(*bboxes):
    boxes = [b for b in bboxes if b]
    if not boxes:
        return None
    return (
        min(b[0] for b in boxes),
        min(b[1] for b in boxes),
        max(b[2] for b in boxes),
        max(b[3] for b in boxes),
    )


def pad_bbox(bbox, size, padding=BBOX_PADDING):
    return (
        max(0, bbox[0] - padding),
        max(0, bbox[1] - padding),
        min(size[0], bbox[2] + padding),
        min(size[1], bbox[3] + padding),
    )


def region_is_local(bbox, size) -> bool:
    if not bbox:
        return False
    area = (bbox[2] - bbox[0]) * (bbox[3] - bbox[1])
    return area <= size[0] * size[1] * MAX_REGION_RATIO


def add_part_layers(psd, group, images, part_names, base, bbox, base_layer_name):
    """bbox で切り抜いたパーツレイヤー群をグループへ追加する。"""
    left, top = bbox[0], bbox[1]
    for name in part_names:
        crop = images[name].crop(bbox)
        layer = PixelLayer.frompil(crop, psd, name, top, left)
        layer.visible = False  # 開いた直後はベースだけが見えるようにする
        group.append(layer)
    base_crop = base.crop(bbox)
    base_layer = PixelLayer.frompil(base_crop, psd, base_layer_name, top, left)
    group.append(base_layer)


def build_psd(character_id: str) -> Path | None:
    layers_dir = CHARACTERS / character_id / "layers"
    images = load_layers(layers_dir)
    if images is None:
        return None
    base = images["base"]
    size = base.size

    psd = PSDImage.new(mode="RGBA", size=size)

    # 領域検出（口・目）。blink 差分は目以外にも微差が出ることがあるため個別に取り union する。
    mouth_bbox = union_bbox(
        diff_bbox(base, images["mouth_open"]),
        diff_bbox(base, images["mouth_closed"]),
    )
    eye_bbox = union_bbox(
        diff_bbox(base, images["blink_half"]),
        diff_bbox(base, images["blink_closed"]),
    )

    fallbacks = []

    # グループは追加順に上へ積まれる。ベース → 口 → 目 の順で作り、目を最前面にする。
    # レイヤー名は Pascal 文字列互換のため ASCII（日本語名は読み手側で文字化けし得る）。
    psd.append(PixelLayer.frompil(base, psd, "base", 0, 0))

    if region_is_local(mouth_bbox, size):
        bbox = pad_bbox(mouth_bbox, size)
        group = Group.new(psd, "mouth")
        add_part_layers(psd, group, images, ["mouth_open", "mouth_closed"], base, bbox, "mouth_base")
    else:
        fallbacks += ["mouth_open", "mouth_closed"]

    if region_is_local(eye_bbox, size):
        bbox = pad_bbox(eye_bbox, size)
        group = Group.new(psd, "eye")
        add_part_layers(psd, group, images, ["blink_closed", "blink_half"], base, bbox, "eye_open")
    else:
        fallbacks += ["blink_closed", "blink_half"]

    # 局所領域を特定できなかった差分絵は、全身レイヤーの参考としてまとめる。
    if fallbacks:
        group = Group.new(psd, "variants")
        for name in fallbacks:
            layer = PixelLayer.frompil(images[name], psd, name, 0, 0)
            layer.visible = False
            group.append(layer)

    out_dir = CHARACTERS / character_id / "live2d-src"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"{character_id}-live2d.psd"
    psd.save(out_path, encoding="utf-8")

    detected = []
    if region_is_local(mouth_bbox, size):
        detected.append(f"口={pad_bbox(mouth_bbox, size)}")
    if region_is_local(eye_bbox, size):
        detected.append(f"目={pad_bbox(eye_bbox, size)}")
    print(f"  生成: {out_path.relative_to(ROOT)} ({', '.join(detected) or '局所領域なし'})")
    return out_path


def main():
    targets = sys.argv[1:]
    if not targets:
        targets = sorted(
            d.name for d in CHARACTERS.iterdir()
            if (d / "layers" / "base.png").exists()
        )
    if not targets:
        print("layers/ を持つキャラクターが見つかりません。")
        return
    for character_id in targets:
        print(f"{character_id}:")
        build_psd(character_id)


if __name__ == "__main__":
    main()
