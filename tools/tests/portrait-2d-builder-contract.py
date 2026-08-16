from __future__ import annotations

import argparse
import importlib.util
import json
import shutil
from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "build-portrait-2d-character.py"
WORK = ROOT / "temp" / "portrait-2d-builder-contract"


def load_builder():
    spec = importlib.util.spec_from_file_location("portrait_builder", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("生成スクリプトを読み込めません")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def args() -> argparse.Namespace:
    return argparse.Namespace(
        id="sample",
        name="サンプル",
        description="",
        greeting="",
        personality="",
        speech_style="",
        first_person="わたし",
        user_name="あなた",
        system_prompt="",
        reaction_head=[],
        reaction_cheek=[],
        reaction_hand=[],
    )


def main() -> None:
    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    builder = load_builder()
    builder.TEMP_DIR = WORK / "scratch"
    builder.INDEX_PATH = WORK / "index.json"
    builder.INDEX_PATH.write_text('{"characters": []}\n', encoding="utf-8")
    try:
        assert builder.parse_face_roi("0.1,0.2,0.3,0.4") == (0.1, 0.2, 0.3, 0.4)
        base = Image.new("RGBA", (4, 4), (0, 0, 0, 0))
        inside = base.copy()
        inside.putpixel((1, 1), (1, 2, 3, 4))
        builder.ensure_changes_inside_roi(base, inside, (0.25, 0.25, 0.5, 0.5), "inside.png")
        outside = base.copy()
        outside.putpixel((0, 0), (1, 2, 3, 4))
        try:
            builder.ensure_changes_inside_roi(base, outside, (0.25, 0.25, 0.5, 0.5), "outside.png")
        except ValueError:
            pass
        else:
            raise AssertionError("顔ROI外の変更を拒否できません")

        frames = {}
        for state, rule in builder.ANIMATION_STATES.items():
            frames[state] = [
                Image.new("RGBA", (2, 2), (index, len(state), 32, 255))
                for index in range(rule["frames"])
            ]
        generation = builder.frame_generation(frames)
        staging = WORK / "staging"
        staging.mkdir()
        paths = builder.write_action_frames(frames, staging, generation)
        manifest = builder.build_manifest(args(), "test", generation, paths)
        assert manifest["frameGeneration"] == generation
        for state, rule in builder.ANIMATION_STATES.items():
            assert len(manifest["animations"][state]["framePaths"]) == rule["frames"]
            assert all(
                path.startswith(f"frames/{generation}/")
                for path in manifest["animations"][state]["framePaths"]
            )
        (staging / "character.json").write_text(json.dumps(manifest), encoding="utf-8")
        destination = WORK / "characters" / "sample"
        destination.parent.mkdir()
        index_text = builder.build_index_text("sample", "サンプル", "")
        builder.install_staged_pack(staging, destination, index_text)
        assert (destination / "character.json").is_file()
        assert json.loads(builder.INDEX_PATH.read_text(encoding="utf-8"))["characters"][0]["id"] == "sample"

        old_marker = destination / "old.txt"
        old_marker.write_text("旧パック", encoding="utf-8")
        failed_staging = WORK / "failed-staging"
        failed_staging.mkdir()
        (failed_staging / "new.txt").write_text("新パック", encoding="utf-8")
        invalid_index = WORK / "index-directory"
        invalid_index.mkdir()
        builder.INDEX_PATH = invalid_index
        try:
            builder.install_staged_pack(failed_staging, destination, index_text)
        except OSError:
            pass
        else:
            raise AssertionError("索引更新失敗を再現できません")
        assert old_marker.read_text(encoding="utf-8") == "旧パック"
        assert not (destination / "new.txt").exists()
    finally:
        for image_name in ("base", "inside", "outside"):
            image = locals().get(image_name)
            if image is not None:
                image.close()
        for images in locals().get("frames", {}).values():
            for image in images:
                image.close()
        shutil.rmtree(WORK, ignore_errors=True)


if __name__ == "__main__":
    main()
    print("portrait-2d builder contract: OK")
