#!/usr/bin/env python3
# build-tha3-layers.py - 1枚絵からTHA3のキーポーズとPicoAgent用の動く自作キャラクターを生成する。

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CHARACTERS_DIR = ROOT / "characters"
INDEX_PATH = CHARACTERS_DIR / "index.json"
THA3_DIR = ROOT / "tools" / "tha3"
TEMP_DIR = ROOT / "temp"
MODEL_FILES = (
    "editor.pt",
    "eyebrow_decomposer.pt",
    "eyebrow_morphing_combiner.pt",
    "face_morpher.pt",
    "two_algo_face_body_rotator.pt",
)

# 上流の pose_parameters.py にある名前だけで指定する。配列位置を固定値で持たない。
# 必須5枚に加え、ひよりのような自然な待機・思考・会話・落胆へ近づけるため、
# 頭・首・身体・呼吸を組み合わせたキーポーズを生成する。
POSES: dict[str, dict[str, float]] = {
    "base": {},
    "blink_half": {"eye_relaxed_left": 0.55, "eye_relaxed_right": 0.55},
    "blink_closed": {"eye_wink_left": 1.0, "eye_wink_right": 1.0},
    "mouth_open": {"mouth_aaa": 0.85},
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
        choices=("standard_float", "standard_half", "separable_float", "separable_half"),
        default="standard_float",
        help="THA3モデル。既定は画質優先のstandard_float。",
    )
    parser.add_argument("--force", action="store_true", help="既存のTHA3生成レイヤーを上書きする。")
    parser.add_argument("--layers-only", action="store_true", help="レイヤーだけ生成し、登録・連番生成・同期は行わない。")
    parser.add_argument("--no-sync", action="store_true", help="生成後の cargo xtask sync-characters を省略する。")
    parser.add_argument(
        "--remove",
        action="store_true",
        help="THA3で作成した指定キャラをOSのゴミ箱へ移し、一覧から外す。",
    )
    args = parser.parse_args()
    if not args.character_id:
        parser.error("キャラクターIDが必要です。")
    return args


def validate_character_id(character_id: str) -> None:
    if len(character_id) > 64 or not character_id or any(
        not (char.isascii() and (char.isalnum() or char in "-_")) for char in character_id
    ):
        raise ValueError("キャラクターIDは64文字以内の英数字・ハイフン・アンダースコアにしてください。")


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
        from PIL import Image
    except ImportError as error:
        raise RuntimeError("CUDA対応PyTorch・NumPy・Pillowが必要です。上流environment.ymlの環境で実行してください。") from error
    if not torch.cuda.is_available():
        raise RuntimeError("THA3の生成にはCUDA対応の高性能NVIDIA GPUが必要です。")

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

        device = torch.device("cuda")
        poser = load_poser(model, device)
        dtype = poser.get_dtype()
        source = extract_pytorch_image_from_PIL_image(image.copy()).to(device).to(dtype)
        parameters = get_pose_parameters()
        with torch.inference_mode():
            for name in names:
                values = POSES[name]
                pose = torch.zeros(poser.get_num_parameters(), device=device, dtype=dtype)
                for parameter_name, value in values.items():
                    pose[parameters.get_parameter_index(parameter_name)] = value
                output = poser.pose(source, pose)[0].detach().cpu()
                rgba = rgba_to_numpy_image(output)
                with Image.fromarray(
                    numpy.uint8(numpy.rint(rgba * 255.0)), mode="RGBA"
                ) as rendered:
                    rendered.save(temp_paths[name])
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
                if "torch" in locals() and torch.cuda.is_available():
                    torch.cuda.empty_cache()


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


def ensure_target_allowed(character_id: str, layers_only: bool) -> None:
    character_dir = CHARACTERS_DIR / character_id
    expected = CHARACTERS_DIR.resolve() / character_id
    if character_dir.exists() and character_dir.resolve() != expected:
        raise ValueError("キャラクターフォルダのリンク実体がcharacters外を指しているため拒否しました。")
    manifest_path = character_dir / "character.json"
    if manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest.get("generator", {}).get("type") != "tha3" and not layers_only:
            raise ValueError(
                "既存の組み込み・手動管理キャラは自作キャラとして上書き登録できません。"
                "レイヤーだけ試す場合は --layers-only を指定してください。"
            )
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
    validate_character_id(args.character_id)
    if args.remove:
        remove_character(args.character_id)
        if not args.no_sync:
            run_checked(["cargo", "xtask", "sync-characters"], "キャラクター同期")
        return

    ensure_target_allowed(args.character_id, args.layers_only)
    character_dir = CHARACTERS_DIR / args.character_id
    input_path = (args.input or (character_dir / "source.png")).resolve()
    image = load_input(input_path)
    layers_dir = character_dir / "layers"
    wanted = [name for name in POSES if args.force or not (layers_dir / f"{name}.png").is_file()]
    mouth_closed_missing = args.force or not (layers_dir / "mouth_closed.png").is_file()

    TEMP_DIR.mkdir(parents=True, exist_ok=True)
    pose_temp_dir = TEMP_DIR / f"tha3-{args.character_id}-{os.getpid()}"
    if pose_temp_dir.exists():
        raise RuntimeError(f"同じ生成処理の一時フォルダが既にあります: {pose_temp_dir}")
    pose_temp_dir.mkdir(parents=True, exist_ok=False)
    try:
        temp_paths = {name: pose_temp_dir / f"{name}.png" for name in wanted}
        if wanted:
            render_poses(image, args.model, wanted, temp_paths)
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
        manifest = custom_manifest(args.character_id, args.name or args.character_id, args.model)
        write_json_atomic(manifest_path, manifest)
        print(f"[build-tha3-layers] 自作キャラ定義を作成しました: {manifest_path.relative_to(ROOT)}")

    if generated or not (character_dir / "animations" / "idle" / "00.png").exists():
        run_checked(
            [sys.executable, str(ROOT / "scripts" / "build-character-animations.py"), args.character_id],
            "連番アニメーション生成",
        )
    register_character(args.character_id, manifest)
    if not args.no_sync:
        run_checked(["cargo", "xtask", "sync-characters"], "キャラクター同期")
    print(
        f"[build-tha3-layers] 完了: {manifest['name']}。"
        f"人格と挨拶は characters/{args.character_id}/character.json で調整できます。"
    )


def main() -> None:
    try:
        build(parse_args())
    except (FileNotFoundError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"エラー: {error}", file=sys.stderr)
        raise SystemExit(1) from error


if __name__ == "__main__":
    main()
