#!/usr/bin/env python3
# build-portrait-2d-character.py - 1枚のバストアップ画像から通常2D（1枚絵）キャラクターパックを作る。
#
# こはる／セナ／ノアと同じ「preview.png + character.json だけ」の構成を出力する。
# THA3 と違い GPU も Python 常駐環境も要らず、呼吸と胸元の二次動作だけで動く。
#
# 実行例:
#   uv run --with pillow --with numpy --with rembg --with onnxruntime \
#       python scripts/build-portrait-2d-character.py \
#       --input temp/source-raw.png --id hinano --name ひなの

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
CHARACTERS_DIR = ROOT / "characters"
INDEX_PATH = CHARACTERS_DIR / "index.json"
TEMP_DIR = ROOT / "temp"

# 背景抜きモデル。人物の髪の輪郭が u2net より安定する。
REMBG_MODEL = "isnet-general-use"
# 輪郭が「不透明」と言えるアルファ下限。ハローの検査と輪郭測定で共通に使う。
SOLID_ALPHA = 40
# 頭頂部の円弧推定に使う走査高さ（画像上端からの画素数）。
CROWN_FIT_ROWS = 300
# 復元した頭頂部の上に残す透明余白。
CROWN_MARGIN = 16


def remove_background(source: Path) -> Image.Image:
    """写真の背景を抜いて RGBA を返す。"""
    from rembg import new_session, remove

    image = Image.open(source).convert("RGBA")
    session = new_session(REMBG_MODEL)
    # alpha matting は髪の毛先の半透明を残しつつ、背景色の混ざりを前景色へ置き換える
    # （これを省くと暗い画面でベージュのハローが出る）。
    return remove(
        image,
        session=session,
        alpha_matting=True,
        alpha_matting_foreground_threshold=248,
        alpha_matting_background_threshold=12,
        alpha_matting_erode_size=6,
    )


def fit_head_circle(alpha: np.ndarray) -> tuple[float, float, float]:
    """画像上端付近の輪郭へ円を最小二乗で当て、(中心x, 中心y, 半径) を返す。"""
    points: list[tuple[float, float]] = []
    for y in range(0, min(CROWN_FIT_ROWS, alpha.shape[0]), 4):
        columns = np.where(alpha[y] > SOLID_ALPHA)[0]
        if columns.size:
            points.append((float(columns.min()), float(y)))
            points.append((float(columns.max()), float(y)))
    if len(points) < 12:
        raise ValueError("頭頂部の輪郭を測れませんでした。人物が上端付近に写っているか確認してください。")
    array = np.array(points)
    x, y = array[:, 0], array[:, 1]
    design = np.c_[x, y, np.ones(len(array))]
    solution, *_ = np.linalg.lstsq(design, x**2 + y**2, rcond=None)
    center_x, center_y = solution[0] / 2, solution[1] / 2
    radius = math.sqrt(solution[2] + center_x**2 + center_y**2)
    return center_x, center_y, radius


def reconstruct_crown(image: Image.Image) -> tuple[Image.Image, int]:
    """上端で平らに切れた頭頂部を、輪郭の円弧に沿って描き足す。

    バストアップ写真は頭のてっぺんが画面外へ出ていることが多く、そのまま透過すると
    透明な画面で頭が平らに切り落とされて見える。切れ量が数十画素なら、既存の髪を
    上へ折り返した texture を円弧の形へ収めるだけで自然に埋まる。
    戻り値は (復元後の画像, 足した高さ)。切れていなければ元画像と 0 を返す。
    """
    alpha = np.asarray(image.getchannel("A"))
    if int((alpha[0] > SOLID_ALPHA).sum()) == 0:
        return image, 0

    center_x, center_y, radius = fit_head_circle(alpha)
    apex = center_y - radius
    if apex >= 0:
        return image, 0

    pad = int(math.ceil(-apex)) + CROWN_MARGIN
    width, height = image.size
    canvas = Image.new("RGBA", (width, height + pad), (0, 0, 0, 0))
    canvas.paste(image, (0, pad))

    source = np.asarray(image).astype(np.float32)
    target = np.asarray(canvas).astype(np.float32)
    seam_center_y = center_y + pad

    seam_columns = np.where(alpha[0] > SOLID_ALPHA)[0]
    seam_left, seam_right = float(seam_columns.min()), float(seam_columns.max())
    seam_mid = (seam_left + seam_right) / 2
    seam_half_width = (seam_right - seam_left) / 2
    seam_arc_half = math.sqrt(max(1.0, radius**2 - (pad - seam_center_y) ** 2))

    for y in range(pad):
        offset = y - seam_center_y
        if abs(offset) >= radius:
            continue
        # 円弧の幅比で、継ぎ目の実輪郭を頭頂へ向けて絞る（髪の左右非対称を保てる）。
        scale = math.sqrt(radius**2 - offset**2) / seam_arc_half
        half_width = seam_half_width * scale
        if half_width < 1.0:
            continue
        # 中心は継ぎ目の実中心から円の中心へ滑らかに寄せる。
        mid = center_x + (seam_mid - center_x) * scale
        left, right = int(round(mid - half_width)), int(round(mid + half_width))
        if right <= left:
            continue

        # texture は継ぎ目を軸に折り返した行から取り、目的の幅へ引き伸ばす。
        source_row = pad - y
        if source_row >= height:
            continue
        row_columns = np.where(np.asarray(image.getchannel("A"))[source_row] > SOLID_ALPHA)[0]
        if row_columns.size < 4:
            continue
        strip = source[source_row, row_columns.min() : row_columns.max() + 1, :3]
        positions = np.linspace(0, strip.shape[0] - 1, right - left)
        sampled = np.stack(
            [np.interp(positions, np.arange(strip.shape[0]), strip[:, ch]) for ch in range(3)],
            axis=1,
        )
        # 頭頂へ向かってごく軽く落とす。明るい面を足すと後光に見えるため上げない。
        sampled *= 1.0 - 0.10 * (1.0 - scale)
        target[y, left:right, :3] = sampled
        target[y, left:right, 3] = 255.0

    filled = Image.fromarray(np.clip(target, 0, 255).astype(np.uint8), "RGBA")
    # 描き足した帯と継ぎ目だけを軽くぼかし、折り返しの筋と階段状の輪郭を均す。
    band = filled.crop((0, 0, width, pad + 8))
    filled.paste(band.filter(ImageFilter.GaussianBlur(1.1)), (0, 0))
    return filled, pad


def check_no_halo(image: Image.Image) -> None:
    """四隅の透過と、半透明の縁が背景色を抱えていないことを確かめる。"""
    array = np.asarray(image).astype(np.int16)
    alpha = array[..., 3]
    corners = [alpha[0, 0], alpha[0, -1], alpha[-1, 0], alpha[-1, -1]]
    height, width = alpha.shape
    # 被写体が下端・左右へ抜けるバストアップでは下の隅が不透明になりうるので上の隅だけ必須。
    if corners[0] != 0 or corners[1] != 0:
        raise ValueError(f"上の四隅が透過していません: {corners}")
    edge = (alpha > 20) & (alpha < 235)
    if not edge.any():
        return
    edge_rgb = array[..., :3][edge].mean(axis=0)
    core_rgb = array[..., :3][alpha > 250].mean(axis=0)
    # 縁が本体よりはるかに明るいと、暗い画面でハローとして浮く。
    if float(edge_rgb.mean() - core_rgb.mean()) > 60.0:
        raise ValueError(
            f"半透明の縁が背景色を抱えています（縁 {edge_rgb.round(1)} / 本体 {core_rgb.round(1)}）。"
        )


def write_qa_sheet(image: Image.Image, path: Path) -> None:
    """市松・暗色・白・緑へ重ねた確認シートを書き出す（消し残しは実表示でしか分からない）。"""
    width, height = image.size
    grid_y, grid_x = np.mgrid[0:height, 0:width]
    checker = np.where((((grid_x // 24) + (grid_y // 24)) % 2 == 0)[..., None], 210, 160)
    backgrounds = [
        Image.fromarray(np.broadcast_to(checker, (height, width, 3)).astype(np.uint8), "RGB"),
        Image.new("RGB", image.size, (24, 24, 28)),
        Image.new("RGB", image.size, (245, 245, 245)),
        Image.new("RGB", image.size, (30, 120, 60)),
    ]
    tile_width = 420
    tile_height = int(height * tile_width / width)
    sheet = Image.new("RGB", (tile_width * len(backgrounds), tile_height))
    for index, background in enumerate(backgrounds):
        composed = background.convert("RGBA")
        composed.alpha_composite(image)
        sheet.paste(composed.convert("RGB").resize((tile_width, tile_height), Image.LANCZOS), (index * tile_width, 0))
    path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(path)


def build_manifest(args: argparse.Namespace, asset_version: str) -> dict:
    frame = {"frames": 1, "fps": 1, "framePaths": ["preview.png"]}
    return {
        "id": args.id,
        "name": args.name,
        "assetVersion": asset_version,
        "description": args.description,
        "greeting": args.greeting,
        "boot": {"sleeping": "preview.png"},
        "persona": {
            "personality": args.personality,
            "speechStyle": args.speech_style,
            "firstPerson": args.first_person,
            "userName": args.user_name,
            "systemPrompt": args.system_prompt,
        },
        "interactions": {
            "reactions": {
                "head": args.reaction_head,
                "cheek": args.reaction_cheek,
                "hand": args.reaction_hand,
            }
        },
        "secondaryMotion": {
            "type": "bust",
            "centerX": args.motion_center_x,
            "centerY": args.motion_center_y,
            "radiusX": args.motion_radius_x,
            "radiusY": args.motion_radius_y,
            "strength": args.motion_strength,
            "duration": args.motion_duration,
        },
        "animations": {
            "idle": {**frame, "motion": "breathe"},
            "thinking": {**frame, "motion": "thinking"},
            "talking": {**frame, "motion": "talking"},
            "sad": {**frame, "motion": "sad"},
        },
    }


def register_in_index(character_id: str, name: str, description: str) -> None:
    index = json.loads(INDEX_PATH.read_text(encoding="utf-8"))
    entry = {
        "description": description,
        "id": character_id,
        "manifest": f"{character_id}/character.json",
        "name": name,
    }
    entries = index.get("characters", [])
    for position, existing in enumerate(entries):
        if existing.get("id") == character_id:
            entries[position] = entry
            break
    else:
        entries.append(entry)
    index["characters"] = entries
    INDEX_PATH.write_text(json.dumps(index, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="1枚のバストアップ画像から通常2D（1枚絵）キャラクターパックを生成する。"
    )
    parser.add_argument("--input", type=Path, required=True, help="入力画像（PNG / JPEG）。")
    parser.add_argument("--id", required=True, help="キャラクターID（英数字・ハイフン・アンダースコア）。")
    parser.add_argument("--name", required=True, help="表示名。")
    parser.add_argument("--asset-version", default="", help="assetVersion。省略時は日付から作る。")
    parser.add_argument("--description", default="")
    parser.add_argument("--greeting", default="")
    parser.add_argument("--personality", default="")
    parser.add_argument("--speech-style", default="")
    parser.add_argument("--first-person", default="わたし")
    parser.add_argument("--user-name", default="あなた")
    parser.add_argument("--system-prompt", default="")
    parser.add_argument("--reaction-head", nargs="*", default=["どうしたの？"])
    parser.add_argument("--reaction-cheek", nargs="*", default=["ちょっとくすぐったいな"])
    parser.add_argument("--reaction-hand", nargs="*", default=["うん、いっしょにやろう"])
    parser.add_argument("--motion-center-x", type=float, default=50.0)
    parser.add_argument("--motion-center-y", type=float, default=34.0)
    parser.add_argument("--motion-radius-x", type=float, default=16.0)
    parser.add_argument("--motion-radius-y", type=float, default=10.0)
    parser.add_argument("--motion-strength", type=float, default=1.0)
    parser.add_argument("--motion-duration", type=float, default=3.8)
    parser.add_argument("--force", action="store_true", help="既存の preview.png を上書きする。")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if not args.id or not all(ch.isascii() and (ch.isalnum() or ch in "-_") for ch in args.id):
        raise ValueError(f"キャラクターIDに使えない文字があります: {args.id}")
    if not args.input.exists():
        raise FileNotFoundError(f"入力画像が見つかりません: {args.input}")

    destination = CHARACTERS_DIR / args.id
    preview = destination / "preview.png"
    if preview.exists() and not args.force:
        raise FileExistsError(f"既に存在します（上書きするなら --force）: {preview}")

    print(f"[build-portrait-2d-character] 背景を抜いています（{REMBG_MODEL}）…")
    cut = remove_background(args.input)
    filled, added = reconstruct_crown(cut)
    if added:
        print(f"[build-portrait-2d-character] 上端で切れた頭頂部を {added}px 復元しました。")
    check_no_halo(filled)

    destination.mkdir(parents=True, exist_ok=True)
    filled.save(preview)
    asset_version = args.asset_version or f"{__import__('datetime').date.today():%Y-%m-%d}-portrait-2d-v1"
    (destination / "character.json").write_text(
        json.dumps(build_manifest(args, asset_version), ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    register_in_index(args.id, args.name, args.description)

    qa_path = TEMP_DIR / f"qa-{args.id}.png"
    write_qa_sheet(filled, qa_path)
    print(f"[build-portrait-2d-character] {preview}（{filled.size[0]}x{filled.size[1]}）を書き出しました。")
    print(f"[build-portrait-2d-character] 透過確認シート: {qa_path}")
    print("[build-portrait-2d-character] 仕上げに `cargo xtask sync-characters` を実行してください。")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
