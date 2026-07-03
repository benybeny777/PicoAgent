# build-live2d-psd.py - characters/<id>/layers/ の差分絵から Live2D Cubism Editor 用の
# レイヤー分け PSD を生成する。
#
# 生成物: characters/<id>/live2d-src/<id>-live2d.psd
#
# レイヤー構成は Live2D 公式サンプル「ひより」（Hiyori.cdi3.json）のパーツ構成に合わせ、
# パーツごとのグループへ base の該当領域を切り抜いて収録する（上から前髪→…→後ろ髪の描画順）。
# 各切り抜きは元位置に整列しているため、全レイヤーを合成すると base と同じ絵になる。
# Cubism Editor ではこのグループがそのままパーツになり、各レイヤーを整形して使う。
#
#   PartHairFront（前髪）/ PartHairSide（横髪）/ PartBrow（まゆ毛）/ PartEye（目）/
#   PartEyeBall（目玉）/ PartCheek（頬）/ PartNose（鼻）/ PartMouth（口）/ PartFace（顔）/
#   PartEar（耳）/ PartNeck（首）/ PartArmA（腕A）/ PartArmB（腕B）/ PartBody（体）/
#   PartHairBack（後ろ髪）
#   variants/ … mouth_open・mouth_closed・blink_half・blink_closed（口・目の作画参考。非表示）
#   base_full … 全身の原本（非表示の参考レイヤー）
#
# 領域はキャラの内容領域（透過を除いた bbox）に対する比率で定義した目安。
# Cubism Editor 側でパーツの余分な部分を削り、隠れる部分を描き足して使う。
#
# 使い方:
#   python scripts/build-live2d-psd.py            # layers/ を持つ全キャラ
#   python scripts/build-live2d-psd.py mugi rin   # 指定キャラのみ
#
# 依存: pip install Pillow psd-tools

import sys
from pathlib import Path

from PIL import Image
from psd_tools import PSDImage
from psd_tools.api.layers import Group, PixelLayer

ROOT = Path(__file__).resolve().parent.parent
CHARACTERS = ROOT / "characters"

LAYER_FILES = ["base", "mouth_open", "mouth_closed", "blink_half", "blink_closed"]

# ひより（Hiyori.cdi3.json）の作画パーツ一覧に対応するスキャフォールド定義。
# (パーツID, 日本語名, 内容領域に対する比率 (left, top, right, bottom))
# 並び順は「下（奥）→ 上（手前）」。PSD には後ろ髪が最下層、前髪が最上層で入る。
HIYORI_PARTS = [
    ("PartHairBack",  "後ろ髪", (0.00, 0.00, 1.00, 0.75)),
    ("PartBody",      "体",     (0.05, 0.35, 0.95, 1.00)),
    ("PartArmB",      "腕B",    (0.50, 0.28, 1.00, 0.72)),
    ("PartArmA",      "腕A",    (0.00, 0.28, 0.50, 0.72)),
    ("PartNeck",      "首",     (0.35, 0.34, 0.65, 0.50)),
    ("PartEar",       "耳",     (0.10, 0.15, 0.90, 0.34)),
    ("PartFace",      "顔",     (0.18, 0.04, 0.82, 0.50)),
    ("PartMouth",     "口",     (0.30, 0.24, 0.70, 0.42)),
    ("PartNose",      "鼻",     (0.40, 0.22, 0.60, 0.34)),
    ("PartCheek",     "頬",     (0.22, 0.20, 0.78, 0.38)),
    ("PartEyeBall",   "目玉",   (0.22, 0.12, 0.78, 0.34)),
    ("PartEye",       "目",     (0.20, 0.10, 0.80, 0.36)),
    ("PartBrow",      "まゆ毛", (0.22, 0.06, 0.78, 0.22)),
    ("PartHairSide",  "横髪",   (0.00, 0.05, 1.00, 0.55)),
    ("PartHairFront", "前髪",   (0.02, 0.00, 0.98, 0.32)),
]


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


def part_bbox(content_bbox, ratio):
    """内容領域 bbox とパーツ比率から絶対座標の切り抜き bbox を返す。"""
    cx0, cy0, cx1, cy1 = content_bbox
    width = cx1 - cx0
    height = cy1 - cy0
    return (
        cx0 + round(width * ratio[0]),
        cy0 + round(height * ratio[1]),
        cx0 + round(width * ratio[2]),
        cy0 + round(height * ratio[3]),
    )


def build_psd(character_id: str) -> Path | None:
    layers_dir = CHARACTERS / character_id / "layers"
    images = load_layers(layers_dir)
    if images is None:
        return None
    base = images["base"]
    content_bbox = base.getbbox() or (0, 0, base.size[0], base.size[1])

    psd = PSDImage.new(mode="RGBA", size=base.size)

    # 全身の原本（非表示の参考レイヤー。最下層）。
    # レイヤー名は PSD の Pascal 文字列互換のため ASCII（日本語名は読み手側で文字化けし得る）。
    base_full = PixelLayer.frompil(base, psd, "base_full", 0, 0)
    base_full.visible = False
    psd.append(base_full)

    # ひよりのパーツ構成でグループを作る（追加順に上へ積まれる: 後ろ髪 → … → 前髪）。
    # 各グループの中身は base の該当領域の切り抜き（元位置に整列済み）。
    for part_id, _label, ratio in HIYORI_PARTS:
        bbox = part_bbox(content_bbox, ratio)
        group = Group.new(psd, part_id)
        crop = base.crop(bbox)
        group.append(PixelLayer.frompil(crop, psd, f"{part_id}_src", bbox[1], bbox[0]))

    # 口・目の開閉差分（作画参考）。最前面に非表示で置く。
    variants = Group.new(psd, "variants")
    for name in ["mouth_open", "mouth_closed", "blink_half", "blink_closed"]:
        layer = PixelLayer.frompil(images[name], psd, name, 0, 0)
        layer.visible = False
        variants.append(layer)

    out_dir = CHARACTERS / character_id / "live2d-src"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"{character_id}-live2d.psd"
    psd.save(out_path, encoding="utf-8")

    # 埋め込み合成プレビューを base で書き直す。psd-tools 1.17 は新規 PSD の save 時、
    # 「既存プレビュー（空＝黒）」を composite() がそのまま返す循環でプレビューが黒く
    # 焼き込まれるため、save 後に image_data を差し替えて書き直す。
    record = psd._record
    record.image_data.set_data(
        [base.getchannel(i).tobytes() for i in range(4)], record.header
    )
    with open(out_path, "wb") as fp:
        record.write(fp, encoding="utf-8")
    print(f"  生成: {out_path.relative_to(ROOT)} (パーツ {len(HIYORI_PARTS)} + variants)")
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
