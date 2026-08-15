#!/usr/bin/env python3
# build-portrait-2d-character.py - 1枚のバストアップ画像から通常2D（1枚絵）キャラクターパックを作る。
#
# こはる／セナ／ノアと同じ「preview.png + character.json だけ」の構成を出力する。
# THA3 と違い GPU も Python 常駐環境も要らず、1枚絵へ軽い状態モーションだけを適用する。
#
# 実行例:
#   uv run --with pillow --with numpy --with rembg --with onnxruntime \
#       python scripts/build-portrait-2d-character.py \
#       --input temp/source-raw.png --id my-character --name マイキャラ

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
CHARACTERS_DIR = ROOT / "characters"
INDEX_PATH = CHARACTERS_DIR / "index.json"
FORMAT_PATH = CHARACTERS_DIR / "portrait-2d-format.json"
TEMP_DIR = ROOT / "temp"

# 背景抜きモデル。人物の髪の輪郭が u2net より安定する。
REMBG_MODEL = "isnet-general-use"
# 輪郭が「不透明」と言えるアルファ下限。ハローの検査と輪郭測定で共通に使う。
SOLID_ALPHA = 40
MAX_ID_CHARS = 64
PORTRAIT_FORMAT = json.loads(FORMAT_PATH.read_text(encoding="utf-8"))
ANIMATION_STATES = PORTRAIT_FORMAT["animations"]
RIG_LAYER_NAMES = [name for name in PORTRAIT_FORMAT["layers"] if name != "base"]


def remove_background(source: Path, session: object) -> Image.Image:
    """写真の背景を抜いて RGBA を返す。"""
    from rembg import remove

    image = Image.open(source).convert("RGBA")
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


def ensure_crown_not_clipped(image: Image.Image) -> None:
    """頭頂部が入力画像の上端で切れていないことを確かめる。

    欠けた髪を既存画素の引き伸ばしで補うと、輪郭と実効解像度を偽装した素材になる。
    自動補完はせず、上端へ被写体が接していれば余白を含む元画像への差し替えを求める。
    """
    alpha = np.asarray(image.getchannel("A"))
    if int((alpha[0] > SOLID_ALPHA).sum()) > 0:
        raise ValueError(
            "被写体が画像上端へ接しているため、頭頂部が切れている可能性があります。"
            "自動で引き伸ばさず、頭上に透明余白を含む元画像へ差し替えてください。"
        )


def check_no_halo(image: Image.Image) -> None:
    """四隅の透過と、半透明の縁が背景色を抱えていないことを確かめる。"""
    array = np.asarray(image).astype(np.int16)
    alpha = array[..., 3]
    corners = [alpha[0, 0], alpha[0, -1], alpha[-1, 0], alpha[-1, -1]]
    # 被写体が下端・左右へ抜けるバストアップでは下の隅が不透明になりうるので上の隅だけ必須。
    if corners[0] != 0 or corners[1] != 0:
        raise ValueError(f"上の四隅が透過していません: {corners}")
    edge = (alpha > 20) & (alpha < 235)
    if not edge.any():
        return
    core = alpha > 250
    if not core.any():
        raise ValueError("不透明な被写体画素がなく、半透明画像だけになっています。")
    edge_rgb = array[..., :3][edge].mean(axis=0)
    core_rgb = array[..., :3][core].mean(axis=0)
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


def load_action_frames(
    frames_dir: Path | None, expected_size: tuple[int, int], background_session: object
) -> dict[str, list[Image.Image]]:
    """状態別の透過連番を読む。補間・拡大・自動変形は行わない。"""
    if frames_dir is None:
        return {}
    if not frames_dir.is_dir():
        raise FileNotFoundError(f"連番フォルダが見つかりません: {frames_dir}")

    result: dict[str, list[Image.Image]] = {}
    for state, required in ANIMATION_STATES.items():
        state_dir = frames_dir / state
        if not state_dir.exists():
            continue
        paths = sorted(state_dir.glob("*.png"))
        if not paths:
            raise ValueError(f"状態フォルダにPNGがありません: {state_dir}")
        frames: list[Image.Image] = []
        for path in paths:
            image = Image.open(path).convert("RGBA")
            if image.getchannel("A").getextrema() == (255, 255):
                image = remove_background(path, background_session)
            if image.size != expected_size:
                raise ValueError(
                    f"連番の寸法が基準画像と違います: {path}（{image.size[0]}x{image.size[1]} / "
                    f"基準 {expected_size[0]}x{expected_size[1]}）"
                )
            ensure_crown_not_clipped(image)
            check_no_halo(image)
            frames.append(image.copy())
        unique_count = len({hashlib.sha256(image.tobytes()).digest() for image in frames})
        if len(frames) < required["frames"] or unique_count < required["minUniqueFrames"]:
            raise ValueError(
                f"{state} の連番が不足しています: 全{len(frames)}枚・固有{unique_count}枚。"
                f"全{required['frames']}枚以上・固有{required['minUniqueFrames']}枚以上を用意してください。"
            )
        result[state] = frames
    return result


def load_rig_layers(
    layers_dir: Path | None, expected_size: tuple[int, int], background_session: object
) -> dict[str, Image.Image]:
    """まばたき・口・視線・感情の同一ポーズ差分を読む。"""
    if layers_dir is None:
        return {}
    if not layers_dir.is_dir():
        raise FileNotFoundError(f"表情レイヤーフォルダが見つかりません: {layers_dir}")
    layers: dict[str, Image.Image] = {}
    for name in RIG_LAYER_NAMES:
        path = layers_dir / f"{name}.png"
        if not path.is_file():
            raise FileNotFoundError(f"表情レイヤーが不足しています: {path}")
        image = Image.open(path).convert("RGBA")
        if image.getchannel("A").getextrema() == (255, 255):
            image = remove_background(path, background_session)
        if image.size != expected_size:
            raise ValueError(
                f"表情レイヤーの寸法が基準画像と違います: {path}（{image.size[0]}x{image.size[1]} / "
                f"基準 {expected_size[0]}x{expected_size[1]}）"
            )
        ensure_crown_not_clipped(image)
        check_no_halo(image)
        layers[name] = image.copy()

    for left, right in [
        ("blink_half", "blink_closed"), ("mouth_closed", "mouth_open"),
        ("gaze_left", "gaze_right"), ("expression_happy", "expression_sad"),
        ("expression_angry", "expression_shy"),
        ("expression_surprised", "expression_confused"),
    ]:
        if layers[left].tobytes() == layers[right].tobytes():
            raise ValueError(f"表情差分の内容が同じです: {left}.png / {right}.png")
    return layers


def write_rig_layers(base: Image.Image, layers: dict[str, Image.Image], destination: Path) -> None:
    if not layers:
        return
    output_dir = destination / "layers"
    output_dir.mkdir(parents=True, exist_ok=True)
    base.save(output_dir / "base.png")
    for name, image in layers.items():
        image.save(output_dir / f"{name}.png")


def write_action_frames(
    frames: dict[str, list[Image.Image]], destination: Path
) -> dict[str, list[str]]:
    """入力連番を内容ハッシュ名で保存し、manifest用の相対パスを返す。"""
    output_dir = destination / "frames" / "portrait-2d"
    output_dir.mkdir(parents=True, exist_ok=True)
    result: dict[str, list[str]] = {}
    for state, images in frames.items():
        paths: list[str] = []
        for image in images:
            digest = hashlib.sha256(image.tobytes()).hexdigest()[:20]
            relative = f"frames/portrait-2d/{digest}.png"
            target = destination / relative
            if not target.exists():
                image.save(target)
            paths.append(relative)
        result[state] = paths
    return result


def build_manifest(
    args: argparse.Namespace, asset_version: str, action_paths: dict[str, list[str]]
) -> dict:
    animations = {}
    for state, config in ANIMATION_STATES.items():
        paths = action_paths.get(state, ["preview.png"])
        animations[state] = {
            "frames": len(paths),
            "fps": config["fps"] if len(paths) > 1 else 1,
            "motion": config["motion"],
            "framePaths": paths,
        }
    return {
        "id": args.id,
        "name": args.name,
        "assetVersion": asset_version,
        "characterFormat": PORTRAIT_FORMAT["format"],
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
        "animations": animations,
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
    parser.add_argument("--id", required=True, help="キャラクターID（64文字以内の英数字・ハイフン・アンダースコア）。")
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
    parser.add_argument(
        "--frames-dir",
        type=Path,
        help="任意の透過連番ルート。idle / thinking / talking / sad の各フォルダへPNGを置く。",
    )
    parser.add_argument(
        "--layers-dir",
        type=Path,
        help="任意の表情差分ルート。目・口・左右視線と6感情のPNGを置く。",
    )
    parser.add_argument("--force", action="store_true", help="既存の preview.png を上書きする。")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if (
        not args.id
        or len(args.id) > MAX_ID_CHARS
        or not all(ch.isascii() and (ch.isalnum() or ch in "-_") for ch in args.id)
    ):
        raise ValueError(f"キャラクターIDに使えない文字があります: {args.id}")
    if not args.input.exists():
        raise FileNotFoundError(f"入力画像が見つかりません: {args.input}")

    destination = CHARACTERS_DIR / args.id
    preview = destination / "preview.png"
    if preview.exists() and not args.force:
        raise FileExistsError(f"既に存在します（上書きするなら --force）: {preview}")

    from rembg import new_session

    print(f"[build-portrait-2d-character] 背景を抜いています（{REMBG_MODEL}）…")
    background_session = new_session(REMBG_MODEL)
    filled = remove_background(args.input, background_session)
    ensure_crown_not_clipped(filled)
    check_no_halo(filled)
    action_frames = load_action_frames(args.frames_dir, filled.size, background_session)
    rig_layers = load_rig_layers(args.layers_dir, filled.size, background_session)

    destination.mkdir(parents=True, exist_ok=True)
    filled.save(preview)
    action_paths = write_action_frames(action_frames, destination)
    write_rig_layers(filled, rig_layers, destination)
    asset_version = args.asset_version or f"{__import__('datetime').date.today():%Y-%m-%d}-portrait-2d-v1"
    (destination / "character.json").write_text(
        json.dumps(build_manifest(args, asset_version, action_paths), ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    register_in_index(args.id, args.name, args.description)

    qa_path = TEMP_DIR / f"qa-{args.id}.png"
    write_qa_sheet(filled, qa_path)
    print(f"[build-portrait-2d-character] {preview}（{filled.size[0]}x{filled.size[1]}）を書き出しました。")
    if action_paths:
        summary = " / ".join(f"{state}:{len(paths)}枚" for state, paths in action_paths.items())
        print(f"[build-portrait-2d-character] 表情・アクション連番: {summary}")
    if rig_layers:
        print(f"[build-portrait-2d-character] まばたき・口・視線・感情レイヤー: {len(rig_layers) + 1}枚")
    print(f"[build-portrait-2d-character] 透過確認シート: {qa_path}")
    print("[build-portrait-2d-character] 仕上げに `cargo xtask sync-characters` を実行してください。")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
