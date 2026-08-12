#!/usr/bin/env python3
# build-tha3-layers.py - 1枚絵からTHA3のキーポーズとPicoAgent用の動く自作キャラクターを生成する。

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CHARACTERS_DIR = ROOT / "characters"
INDEX_PATH = CHARACTERS_DIR / "index.json"
THA3_DIR = ROOT / "src-tauri" / "resources" / "bundled" / "tha3"
TEMP_DIR = ROOT / "temp"
OUTPUT_SIZE = (445, 900)
OUTPUT_MARGIN = (24, 38)
# 基準ポーズの被写体範囲を全レイヤーへ共通適用するが、頭・身体を振るポーズは基準の輪郭より
# わずかに外へ出る。切り取り箱をこの比率だけ広げて、髪先や肩が欠けないようにする。
OUTPUT_BOX_PADDING = 0.05
# 口の比較・補正に使う窓（左上・右下の画像比）。入力要件の頭部領域
# （x=192..320 / y=64..192）内で、目や首を拾わない口元だけを見る。
MOUTH_WINDOW = (0.40, 0.25, 0.60, 0.36)
MODEL_FILES = (
    "editor.pt",
    "eyebrow_decomposer.pt",
    "eyebrow_morphing_combiner.pt",
    "face_morpher.pt",
    "two_algo_face_body_rotator.pt",
)
MOUTH_CANDIDATES = ("mouth_aaa", "mouth_iii", "mouth_uuu", "mouth_eee", "mouth_ooo", "mouth_delta")

# 上流の pose_parameters.py にある名前だけで指定する。配列位置を固定値で持たない。
# 必須5枚に加え、ひよりのような自然な待機・思考・会話・落胆へ近づけるため、
# 頭・首・身体・呼吸を組み合わせたキーポーズを生成する。
POSES: dict[str, dict[str, float]] = {
    "base": {},
    "blink_half": {"eye_relaxed_left": 0.55, "eye_relaxed_right": 0.55},
    "blink_closed": {"eye_wink_left": 1.0, "eye_wink_right": 1.0},
    # mouth_openは入力絵との相性差が大きいため、描画時に全口形候補から差が最大のものを自動選択する。
    "mouth_open": {},
    "idle_left": {"head_y": -0.08, "neck_z": -0.025, "body_y": -0.03, "breathing": 0.30},
    "idle_right": {"head_y": 0.08, "neck_z": 0.025, "body_y": 0.03, "breathing": 0.65},
    "idle_breathe": {"head_x": -0.025, "breathing": 0.90},
    "thinking_left": {
        "head_x": -0.07,
        "head_y": -0.13,
        "neck_z": -0.04,
        "body_y": -0.04,
        "eye_unimpressed_left": 0.15,
        "eye_unimpressed_right": 0.15,
    },
    "thinking_right": {
        "head_x": -0.05,
        "head_y": 0.11,
        "neck_z": 0.035,
        "body_y": 0.035,
        "eye_unimpressed_left": 0.10,
        "eye_unimpressed_right": 0.10,
    },
    "thinking_blink": {
        "head_x": -0.06,
        "head_y": -0.04,
        "eye_relaxed_left": 0.65,
        "eye_relaxed_right": 0.65,
    },
    "talking_left_closed": {"head_y": -0.055, "neck_z": -0.02, "body_y": -0.02, "breathing": 0.40},
    "talking_left_open": {
        "head_y": -0.055,
        "neck_z": -0.02,
        "body_y": -0.02,
        "breathing": 0.55,
        "mouth_aaa": 0.78,
    },
    "talking_right_closed": {"head_y": 0.055, "neck_z": 0.02, "body_y": 0.02, "breathing": 0.55},
    "talking_right_open": {
        "head_y": 0.055,
        "neck_z": 0.02,
        "body_y": 0.02,
        "breathing": 0.70,
        "mouth_aaa": 0.88,
    },
    "sad": {
        "eyebrow_troubled_left": 0.70,
        "eyebrow_troubled_right": 0.70,
        "eye_relaxed_left": 0.25,
        "eye_relaxed_right": 0.25,
        "mouth_lowered_corner_left": 0.55,
        "mouth_lowered_corner_right": 0.55,
        "head_x": 0.08,
    },
    "sad_blink": {
        "eyebrow_troubled_left": 0.70,
        "eyebrow_troubled_right": 0.70,
        "eye_relaxed_left": 0.75,
        "eye_relaxed_right": 0.75,
        "mouth_lowered_corner_left": 0.55,
        "mouth_lowered_corner_right": 0.55,
        "head_x": 0.09,
    },
}
REQUIRED_OUTPUTS = tuple(POSES) + ("mouth_closed",)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "透過512x512 PNGからTHA3キーポーズ17枚と連番42枚を生成し、"
            "PicoAgentの自作キャラクターとして登録します。"
        )
    )
    parser.add_argument("character_id", nargs="?", help="英数字・ハイフン・アンダースコアのキャラクターID。")
    parser.add_argument("--input", type=Path, help="入力PNG。省略時は characters/<id>/source.png。")
    parser.add_argument("--name", help="キャラクター名。新規作成時はIDを使用。表示には「（自作）」が付きます。")
    parser.add_argument(
        "--model",
        choices=("auto", "standard_float", "standard_half", "separable_float", "separable_half"),
        default="auto",
        help="THA3モデル。autoはWindowsのVRAM量で3モデルから選び、Apple Silicon Macではstandard_float。",
    )
    parser.add_argument("--force", action="store_true", help="既存のTHA3生成レイヤーを上書きする。")
    parser.add_argument("--layers-only", action="store_true", help="レイヤーだけ生成し、登録・連番生成・同期は行わない。")
    parser.add_argument("--no-sync", action="store_true", help="生成後の cargo xtask sync-characters を省略する。")
    parser.add_argument("--pack-dir", type=Path, help="GUI連携用。登録せず、このフォルダへ完成パックを生成する。")
    parser.add_argument("--tha3-dir", type=Path, help="THA3本体とdata/modelsを置いたフォルダ。")
    parser.add_argument("--temp-dir", type=Path, help="生成途中の一時フォルダ親。省略時はリポジトリ直下temp。")
    parser.add_argument("--animation-script", type=Path, help="連番生成スクリプトの明示パス。")
    parser.add_argument(
        "--remove",
        action="store_true",
        help="THA3で作成した指定キャラをOSのゴミ箱へ移し、一覧から外す。",
    )
    args = parser.parse_args()
    if not args.character_id:
        parser.error("キャラクターIDが必要です。")
    if args.remove and args.pack_dir:
        parser.error("--remove と --pack-dir は同時に指定できません。")
    return args


def validate_character_id(character_id: str) -> None:
    if len(character_id) > 64 or not character_id or any(
        not (char.isascii() and (char.isalnum() or char in "-_")) for char in character_id
    ):
        raise ValueError("キャラクターIDは64文字以内の英数字・ハイフン・アンダースコアにしてください。")


def ensure_layer_sizes(layers_dir: Path, wanted: list[str]) -> None:
    """作り直さない既存レイヤーが現在の出力寸法と揃っているかを確かめる。

    出力寸法を変えたあとに `--force` なしで一部だけ作り直すと、レイヤーごとに寸法が
    混ざったまま連番アニメが組まれて表示が破綻する。黙って進めず、作り直しを促す。
    """
    try:
        from PIL import Image
    except ImportError as error:
        raise RuntimeError("Pillowが必要です。THA3用Python環境で実行してください。") from error

    mismatched = []
    for name in REQUIRED_OUTPUTS:
        if name in wanted:
            continue
        path = layers_dir / f"{name}.png"
        if not path.is_file():
            continue
        with Image.open(path) as existing:
            if existing.size != OUTPUT_SIZE:
                mismatched.append(f"{name}.png({existing.width}x{existing.height})")
    if mismatched:
        raise RuntimeError(
            f"既存レイヤーの寸法が現在の出力{OUTPUT_SIZE[0]}x{OUTPUT_SIZE[1]}と異なります: "
            f"{', '.join(mismatched)}。寸法が混ざると連番アニメが破綻するため、"
            "--force を付けて全レイヤーを作り直してください。"
        )


def load_input(path: Path):
    try:
        from PIL import Image
    except ImportError as error:
        raise RuntimeError("Pillowが必要です。THA3用Python環境で実行してください。") from error

    if not path.is_file():
        raise FileNotFoundError(f"入力画像が見つかりません: {path}")
    with Image.open(path) as opened:
        image_format = opened.format
        image = opened.copy()
    if image_format != "PNG":
        raise ValueError(f"入力はPNGにしてください: {path}")
    if image.size != (512, 512):
        raise ValueError(f"入力画像は512x512が必要です（現在: {image.width}x{image.height}）。")
    if image.mode != "RGBA":
        raise ValueError(f"入力画像にはアルファチャンネルが必要です（現在のモード: {image.mode}）。")
    alpha = image.getchannel("A")
    minimum, maximum = alpha.getextrema()
    if minimum != 0 or maximum == 0:
        raise ValueError("背景を完全透過（alpha=0）にし、キャラクター本体は不透明にしてください。")
    corners = ((0, 0), (511, 0), (0, 511), (511, 511))
    if any(alpha.getpixel(point) != 0 for point in corners):
        raise ValueError("四隅に不透明画素があります。背景を完全透過し、被写体をキャンバス内へ収めてください。")
    bbox = alpha.getbbox()
    if bbox is None or bbox[0] <= 0 or bbox[1] <= 0 or bbox[2] >= 512 or bbox[3] >= 512:
        raise ValueError("被写体がキャンバス端で切れています。全身を512x512内へ余白付きで配置してください。")
    head_box = alpha.crop((192, 64, 320, 192))
    if head_box.getbbox() is None:
        raise ValueError("上半分中央の128x128領域に頭部が見つかりません。THA3の入力配置へ合わせてください。")
    print(
        "[build-tha3-layers] 入力検査OK。正面向きの1人、直立、手が頭から離れていることは目視でも確認してください。"
    )
    return image


def model_dir(model: str) -> Path:
    return THA3_DIR / "data" / "models" / model


def platform_model(model: str) -> str:
    if model != "auto":
        return model
    if sys.platform == "win32":
        try:
            import torch
        except ImportError as error:
            raise RuntimeError("同梱THA3ランタイムが不足しています。`cargo xtask setup tha3` を実行してください。") from error
        if not torch.cuda.is_available():
            raise RuntimeError("WindowsでのTHA3生成にはCUDA対応NVIDIA GPUが必要です。")
        vram_gib = torch.cuda.get_device_properties(0).total_memory / (1024**3)
        if vram_gib < 4.0:
            raise RuntimeError(f"THA3生成には4GB以上のVRAMが必要です（検出: {vram_gib:.1f}GB）。")
        if vram_gib >= 10.0:
            selected = "standard_float"
        elif vram_gib >= 6.0:
            selected = "standard_half"
        else:
            selected = "separable_half"
        print(f"[build-tha3-layers] VRAM {vram_gib:.1f}GBに合わせて{selected}モデルを選択しました。")
        return selected
    if sys.platform == "darwin" and platform.machine().lower() in ("arm64", "aarch64"):
        return "standard_float"
    if sys.platform == "darwin":
        raise RuntimeError("Intel MacでのTHA3生成は対象外です。Apple Silicon Macを使用してください。")
    raise RuntimeError("THA3生成の対応OSはWindows x64とApple Silicon Macです。")


def padded_box(box, width: int, height: int):
    """被写体範囲を少しだけ広げる（頭・身体を振るポーズが基準の輪郭から出るため）。"""
    x1, y1, x2, y2 = box
    pad_x = int(round((x2 - x1) * OUTPUT_BOX_PADDING))
    pad_y = int(round((y2 - y1) * OUTPUT_BOX_PADDING))
    return (max(0, x1 - pad_x), max(0, y1 - pad_y), min(width, x2 + pad_x), min(height, y2 + pad_y))


def mouth_window(width: int, height: int):
    """口の比較・補正に使う窓を画素座標で返す。"""
    left, top, right, bottom = MOUTH_WINDOW
    return (int(width * left), int(height * top), int(width * right), int(height * bottom))


def sharpen_upscaled(image, image_filter):
    """拡大でにじんだ輪郭を軽く戻す。

    THA3 の出力は 512x512 で、入力要件（頭部が上半分中央の128x128付近）に従うと全身キャラの
    被写体は 150x330 程度にしかならない。これを出力枠へ 2 倍以上に拡大するため、そのままでは
    手描き素材より眠い絵になる。アルファは対象外にする（縁に輪郭が出るため）。
    """
    sharpened = image.convert("RGB").filter(image_filter.UnsharpMask(radius=1.2, percent=55, threshold=3))
    sharpened.putalpha(image.getchannel("A"))
    return sharpened


def canvas_placement(box):
    """基準ポーズの箱を出力キャンバスへ収める倍率後サイズと貼り付け位置を返す。

    縦横比を保ったまま（非等倍で伸ばすと原画と体型が変わる）、立ち位置がぶれないよう
    下端を揃え、横は中央へ置く。全レイヤーで同じ箱を渡すので位置合わせは一致する。
    """
    margin_x, margin_y = OUTPUT_MARGIN
    box_width = max(1, box[2] - box[0])
    box_height = max(1, box[3] - box[1])
    scale = min((OUTPUT_SIZE[0] - margin_x * 2) / box_width, (OUTPUT_SIZE[1] - margin_y * 2) / box_height)
    size = (max(1, int(round(box_width * scale))), max(1, int(round(box_height * scale))))
    return size, ((OUTPUT_SIZE[0] - size[0]) // 2, OUTPUT_SIZE[1] - margin_y - size[1])


def ensure_tha3(model: str) -> None:
    if not (THA3_DIR / "tha3" / "poser" / "modes" / "load_poser.py").is_file():
        raise RuntimeError("THA3本体がありません。先に `cargo xtask setup tha3` を実行してください。")
    missing = [name for name in MODEL_FILES if not (model_dir(model) / name).is_file()]
    if missing:
        raise RuntimeError(
            f"THA3の{model}モデルが不足しています: {', '.join(missing)}。"
            "`cargo xtask setup tha3` の警告に従って学習済みモデルを配置してください。"
        )


def render_poses(image, model: str, names: list[str], temp_paths: dict[str, Path]) -> None:
    ensure_tha3(model)
    try:
        import numpy
        import torch
        from PIL import Image, ImageDraw, ImageFilter
    except ImportError as error:
        raise RuntimeError("同梱THA3ランタイム（PyTorch・NumPy・Pillow）が不足しています。`cargo xtask setup tha3` を実行してください。") from error
    if sys.platform == "win32":
        if not torch.cuda.is_available():
            raise RuntimeError("WindowsでのTHA3生成にはCUDA対応NVIDIA GPUが必要です。")
        device = torch.device("cuda")
    elif sys.platform == "darwin":
        if platform.machine().lower() not in ("arm64", "aarch64"):
            raise RuntimeError("Intel MacでのTHA3生成は対象外です。Apple Silicon Macを使用してください。")
        if not torch.backends.mps.is_available():
            raise RuntimeError("Apple Silicon MacでMPSを利用できません。macOSとPyTorchランタイムを確認してください。")
        device = torch.device("mps")
    else:
        raise RuntimeError("THA3生成の対応OSはWindows x64とApple Silicon Macです。")

    sys.path.insert(0, str(THA3_DIR))
    previous_cwd = Path.cwd()
    poser = None
    source = None
    pose = None
    output = None
    try:
        os.chdir(THA3_DIR)
        from tha3.poser.modes.load_poser import load_poser
        from tha3.poser.modes.pose_parameters import get_pose_parameters
        from tha3.util import extract_pytorch_image_from_PIL_image, rgba_to_numpy_image

        poser = load_poser(model, device)
        dtype = poser.get_dtype()
        source = extract_pytorch_image_from_PIL_image(image.copy()).to(device).to(dtype)
        parameters = get_pose_parameters()
        base_rgba = None
        base_box = None

        def render(values: dict[str, float]):
            nonlocal pose, output
            pose = torch.zeros(poser.get_num_parameters(), device=device, dtype=dtype)
            for parameter_name, value in values.items():
                pose[parameters.get_parameter_index(parameter_name)] = value
            output = poser.pose(source, pose)[0].detach().cpu()
            return rgba_to_numpy_image(output)

        def alpha_box(rgba):
            points = numpy.argwhere(rgba[:, :, 3] > 0.02)
            if not len(points):
                return (0, 0, rgba.shape[1], rgba.shape[0])
            return (
                int(points[:, 1].min()),
                int(points[:, 0].min()),
                int(points[:, 1].max()) + 1,
                int(points[:, 0].max()) + 1,
            )

        def ensure_base():
            # 位置合わせの基準は常に base ポーズ。`--force` なしの部分再生成では names に "base" が
            # 入らないため、必要になった時点で基準ポーズだけ描いて切り取り箱を確定する。
            nonlocal base_rgba, base_box
            if base_rgba is None:
                base_rgba = render(POSES["base"])
                base_box = padded_box(alpha_box(base_rgba), base_rgba.shape[1], base_rgba.shape[0])
            return base_rgba

        def enhance_small_mouth(rendered, difference, window):
            # THA3が小さなデフォルメ口をほぼ動かせない場合も、差分位置を手掛かりに自動補正する。
            x1, y1, x2, y2 = window
            intensity = difference[:, :, :3].sum(axis=2)
            strong_pixels = int((intensity > 0.10).sum())
            if strong_pixels >= 80:
                return rendered, False
            # 既定位置は比較窓（頭部がある上半分中央）の中心。窓の中に少しでも差分があれば
            # その重心へ寄せる。窓の外は見ないので、髪や目の弱い差分を拾って顔の別の場所へ
            # 口を描くことはない。
            center_x = (x1 + x2) // 2
            center_y = (y1 + y2) // 2
            total = float(intensity.sum())
            if total > 1e-3:
                rows, columns = numpy.indices(intensity.shape)
                center_x = x1 + int(round(float((intensity * columns).sum()) / total))
                center_y = y1 + int(round(float((intensity * rows).sum()) / total))
            # デフォルメ絵で肌や髪を誤って拾わないよう、口内と舌は小さな共通色に限定する。
            dark = (84, 38, 52)
            tongue = (226, 120, 138)
            scale = 4
            patch = Image.new("RGBA", (8 * scale, 8 * scale), (0, 0, 0, 0))
            draw = ImageDraw.Draw(patch)
            draw.ellipse((1 * scale, 1 * scale, 7 * scale, 7 * scale), fill=(*dark, 255))
            draw.ellipse((2.5 * scale, 4.5 * scale, 5.5 * scale, 6.5 * scale), fill=(*tongue, 255))
            patch = patch.resize((8, 8), Image.Resampling.LANCZOS)
            rendered.alpha_composite(patch, (center_x - 4, center_y - 4))
            return rendered, True

        with torch.inference_mode():
            for name in names:
                base = ensure_base()
                # 入力要件どおり頭部がある上半分中央だけを比較し、身体揺れ等を選択基準へ混ぜない。
                window = mouth_window(base.shape[1], base.shape[0])
                x1, y1, x2, y2 = window
                if name == "mouth_open":
                    best = None
                    for parameter_name in MOUTH_CANDIDATES:
                        candidate = render({parameter_name: 1.0})
                        score = float(numpy.abs(candidate[y1:y2, x1:x2, :3] - base[y1:y2, x1:x2, :3]).sum())
                        if best is None or score > best[0]:
                            best = (score, parameter_name, candidate)
                    assert best is not None
                    _, selected_mouth, rgba = best
                    print(f"[build-tha3-layers] 口パク候補から{selected_mouth}を自動選択しました。")
                elif name == "base":
                    rgba = base
                else:
                    rgba = render(POSES[name])
                with Image.fromarray(numpy.uint8(numpy.rint(rgba * 255.0))) as rendered:
                    if name == "mouth_open":
                        difference = numpy.abs(rgba[y1:y2, x1:x2] - base[y1:y2, x1:x2])
                        rendered, enhanced = enhance_small_mouth(rendered, difference, window)
                        if enhanced:
                            print("[build-tha3-layers] 小さい口の差分を自動検出し、見える口パクへ補正しました。")
                    assert base_box is not None
                    size, position = canvas_placement(base_box)
                    with Image.new("RGBA", OUTPUT_SIZE, (0, 0, 0, 0)) as normalized:
                        with rendered.crop(base_box).resize(size, Image.Resampling.LANCZOS) as fitted:
                            normalized.alpha_composite(sharpen_upscaled(fitted, ImageFilter), position)
                        normalized.save(temp_paths[name])
                print(f"[build-tha3-layers] 生成: {name}.png")
    finally:
        try:
            if poser is not None:
                poser.free()
        finally:
            source = None
            pose = None
            output = None
            try:
                os.chdir(previous_cwd)
            finally:
                if str(THA3_DIR) in sys.path:
                    sys.path.remove(str(THA3_DIR))
                if "torch" in locals():
                    if torch.cuda.is_available():
                        torch.cuda.empty_cache()
                    if hasattr(torch, "mps") and torch.backends.mps.is_available():
                        torch.mps.empty_cache()


def write_json_atomic(path: Path, value: dict) -> None:
    TEMP_DIR.mkdir(parents=True, exist_ok=True)
    temporary = TEMP_DIR / f"tha3-{path.parent.name}-{path.name}-{os.getpid()}.part"
    try:
        temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def copy_file_atomic(source: Path, destination: Path, label: str) -> None:
    TEMP_DIR.mkdir(parents=True, exist_ok=True)
    temporary = TEMP_DIR / f"tha3-{label}-{os.getpid()}.part"
    try:
        shutil.copy2(source, temporary)
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)


def custom_manifest(character_id: str, name: str, model: str) -> dict:
    display_name = name if name.endswith("（自作）") else f"{name}（自作）"
    return {
        "id": character_id,
        "name": display_name,
        "assetVersion": f"tha3-generated-v1-{character_id}",
        "description": "1枚絵からTHA3で生成した自作キャラクター。人格・挨拶はcharacter.jsonで調整できます。",
        "greeting": f"こんにちは、{name}です。今日も一緒に作業しましょう。",
        "boot": {"sleeping": "preview.png"},
        "persona": {
            "personality": "落ち着いて協力的な作業相棒。分からないことはごまかさず、確認しながら進める。",
            "speechStyle": "自然で簡潔な日本語。結論と次の行動を分かりやすく伝える。",
            "firstPerson": "わたし",
            "userName": "あなた",
            "systemPrompt": f"{name}として振る舞う。ユーザーの作業を安全かつ現実的に前へ進める。",
        },
        "interactions": {"reactions": {"head": ["どうしました？"], "cheek": ["くすぐったいです"], "hand": ["一緒に進めましょう"]}},
        "variants": [{"id": "default", "name": "いつもの姿"}],
        "generator": {
            "type": "tha3",
            "label": "自作（THA3）",
            "model": model,
            "source": "source.png",
            "keyPoseCount": len(REQUIRED_OUTPUTS),
        },
    }


def register_character(character_id: str, manifest: dict) -> None:
    index = json.loads(INDEX_PATH.read_text(encoding="utf-8"))
    characters = index.get("characters")
    if not isinstance(characters, list):
        raise ValueError(f"characters/index.json の characters が配列ではありません: {INDEX_PATH}")
    if any(entry.get("id") == character_id for entry in characters if isinstance(entry, dict)):
        return
    characters.append(
        {
            "id": character_id,
            "name": manifest["name"],
            "manifest": f"{character_id}/character.json",
            "description": "自作（THA3）: 1枚絵から生成したキャラクター。",
        }
    )
    write_json_atomic(INDEX_PATH, index)
    print(f"[build-tha3-layers] characters/index.json に {character_id} を登録しました")


def ensure_target_allowed(character_id: str, layers_only: bool, character_dir: Path, managed_repo: bool) -> None:
    expected = (
        CHARACTERS_DIR.resolve() / character_id
        if managed_repo
        else character_dir.parent.resolve() / character_dir.name
    )
    if character_dir.exists() and character_dir.resolve() != expected:
        raise ValueError("キャラクターフォルダのリンク実体が生成先外を指しているため拒否しました。")
    manifest_path = character_dir / "character.json"
    if manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest.get("generator", {}).get("type") != "tha3" and not layers_only:
            raise ValueError(
                "既存の組み込み・手動管理キャラは自作キャラとして上書き登録できません。"
                "レイヤーだけ試す場合は --layers-only を指定してください。"
            )
        return
    if not managed_repo:
        return
    index = json.loads(INDEX_PATH.read_text(encoding="utf-8"))
    if any(
        isinstance(entry, dict) and entry.get("id") == character_id
        for entry in index.get("characters", [])
    ):
        raise ValueError(f"characters/index.json に同じIDが既にあります: {character_id}")


def run_checked(command: list[str], label: str) -> None:
    result = subprocess.run(command, cwd=ROOT, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"{label}に失敗しました（終了コード {result.returncode}）。")


def move_to_trash(path: Path) -> None:
    environment = os.environ.copy()
    environment["THA3_REMOVE_TARGET"] = str(path)
    if sys.platform == "win32":
        command = [
            "powershell",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            (
                "Add-Type -AssemblyName Microsoft.VisualBasic; "
                "[Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory("
                "$env:THA3_REMOVE_TARGET, "
                "[Microsoft.VisualBasic.FileIO.UIOption]::OnlyErrorDialogs, "
                "[Microsoft.VisualBasic.FileIO.RecycleOption]::SendToRecycleBin)"
            ),
        ]
    elif sys.platform == "darwin":
        command = [
            "osascript",
            "-e",
            'set targetPath to system attribute "THA3_REMOVE_TARGET"',
            "-e",
            'tell application "Finder" to delete POSIX file targetPath',
        ]
    else:
        raise RuntimeError("キャラ削除はWindowsとmacOSだけに対応しています。")
    result = subprocess.run(command, cwd=ROOT, env=environment, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"OSのゴミ箱へ移動できませんでした（終了コード {result.returncode}）。")


def remove_character(character_id: str) -> None:
    character_dir = CHARACTERS_DIR / character_id
    expected = CHARACTERS_DIR.resolve() / character_id
    if character_dir.exists() and character_dir.resolve() != expected:
        raise ValueError("キャラクターフォルダのリンク実体がcharacters外を指しているため拒否しました。")
    manifest_path = character_dir / "character.json"
    if not manifest_path.is_file():
        raise FileNotFoundError(f"キャラクターが見つかりません: {character_id}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("generator", {}).get("type") != "tha3":
        raise ValueError("組み込み・手動管理キャラはこのコマンドでは削除できません。THA3自作キャラだけが対象です。")
    index = json.loads(INDEX_PATH.read_text(encoding="utf-8"))
    entries = index.get("characters", [])
    if not isinstance(entries, list):
        raise ValueError(f"characters/index.json の characters が配列ではありません: {INDEX_PATH}")
    move_to_trash(character_dir)
    index["characters"] = [
        entry for entry in entries if not isinstance(entry, dict) or entry.get("id") != character_id
    ]
    if index.get("default") == character_id:
        index["default"] = "mugi"
    write_json_atomic(INDEX_PATH, index)
    print(f"[build-tha3-layers] {character_id} をOSのゴミ箱へ移し、一覧から外しました")


def build(args: argparse.Namespace) -> None:
    global TEMP_DIR, THA3_DIR
    validate_character_id(args.character_id)
    if args.tha3_dir:
        THA3_DIR = args.tha3_dir.resolve()
    if args.temp_dir:
        TEMP_DIR = args.temp_dir.resolve()
    if args.remove:
        remove_character(args.character_id)
        if not args.no_sync:
            run_checked(["cargo", "xtask", "sync-characters"], "キャラクター同期")
        return

    selected_model = platform_model(args.model)

    managed_repo = args.pack_dir is None
    character_dir = (
        CHARACTERS_DIR / args.character_id if managed_repo else args.pack_dir.resolve()
    )
    ensure_target_allowed(args.character_id, args.layers_only, character_dir, managed_repo)
    input_path = (args.input or (character_dir / "source.png")).resolve()
    image = load_input(input_path)
    layers_dir = character_dir / "layers"
    wanted = [name for name in POSES if args.force or not (layers_dir / f"{name}.png").is_file()]
    mouth_closed_missing = args.force or not (layers_dir / "mouth_closed.png").is_file()
    ensure_layer_sizes(layers_dir, wanted if not mouth_closed_missing else [*wanted, "mouth_closed"])

    TEMP_DIR.mkdir(parents=True, exist_ok=True)
    pose_temp_dir = TEMP_DIR / f"tha3-{args.character_id}-{os.getpid()}"
    if pose_temp_dir.exists():
        raise RuntimeError(f"同じ生成処理の一時フォルダが既にあります: {pose_temp_dir}")
    pose_temp_dir.mkdir(parents=True, exist_ok=False)
    try:
        temp_paths = {name: pose_temp_dir / f"{name}.png" for name in wanted}
        if wanted:
            render_poses(image, selected_model, wanted, temp_paths)
        layers_dir.mkdir(parents=True, exist_ok=True)
        for name, temporary in temp_paths.items():
            temporary.replace(layers_dir / f"{name}.png")
        if mouth_closed_missing:
            source = layers_dir / "base.png"
            if not source.is_file():
                raise RuntimeError("mouth_closed用のbase.pngを生成できませんでした。")
            copy_file_atomic(source, layers_dir / "mouth_closed.png", f"{args.character_id}-mouth-closed")
            print("[build-tha3-layers] 生成: mouth_closed.png（baseの複製）")
    finally:
        try:
            if pose_temp_dir.exists():
                shutil.rmtree(pose_temp_dir)
        finally:
            image.close()

    generated = bool(wanted or mouth_closed_missing)
    if not generated:
        print("[build-tha3-layers] 既存レイヤーを保護するため全てスキップしました（上書きは --force）。")
    if args.layers_only:
        return

    source_dest = character_dir / "source.png"
    if input_path != source_dest.resolve() and not source_dest.exists():
        copy_file_atomic(input_path, source_dest, f"{args.character_id}-source")
    preview = character_dir / "preview.png"
    if not preview.exists():
        copy_file_atomic(layers_dir / "base.png", preview, f"{args.character_id}-preview")
    manifest_path = character_dir / "character.json"
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    else:
        manifest = custom_manifest(args.character_id, args.name or args.character_id, selected_model)
        write_json_atomic(manifest_path, manifest)
        print(f"[build-tha3-layers] 自作キャラ定義を作成しました: {manifest_path}")

    if generated or not (character_dir / "animations" / "idle" / "00.png").exists():
        animation_script = (
            args.animation_script.resolve()
            if args.animation_script
            else ROOT / "scripts" / "build-character-animations.py"
        )
        run_checked(
            [sys.executable, str(animation_script), "--character-dir", str(character_dir)],
            "連番アニメーション生成",
        )
    if managed_repo:
        register_character(args.character_id, manifest)
    if managed_repo and not args.no_sync:
        run_checked(["cargo", "xtask", "sync-characters"], "キャラクター同期")
    location = (
        f"characters/{args.character_id}/character.json"
        if managed_repo
        else str(character_dir / "character.json")
    )
    print(f"[build-tha3-layers] 完了: {manifest['name']}。人格と挨拶は {location} で調整できます。")


def main() -> None:
    try:
        build(parse_args())
    except (FileNotFoundError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"エラー: {error}", file=sys.stderr)
        raise SystemExit(1) from error


if __name__ == "__main__":
    main()
