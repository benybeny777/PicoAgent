"""同じ原画の正規DINO/SAM解析から旧リグへ実測した顔座標を引き継ぐ。"""
import argparse
import hashlib
import json
import math
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def import_landmarks(source, target):
    original = source / "source/input.png"
    base = target / "layers/mouth_closed.png"
    source_hash = digest(original)
    if source_hash != digest(base):
        raise ValueError("解析原画とキャラクターの正本が一致しません")
    analysis_path = source / "analysis/analysis.json"
    selected = json.loads(analysis_path.read_text(encoding="utf-8"))["analysis"]["selected"]
    anchors = {}
    for name, key in {"face": "face", "eyeL": "left_eye", "eyeR": "right_eye", "mouth": "mouth"}.items():
        box = selected[key]["box"]
        if len(box) != 4 or not all(math.isfinite(v) for v in box) or box[2] <= box[0] or box[3] <= box[1]:
            raise ValueError(f"実測座標が不正です: {key}")
        anchors[name] = dict(zip(("x0", "y0", "x1", "y1"), box))
        anchors[name].update(cx=(box[0] + box[2])/2, cy=(box[1] + box[3])/2)
    value = {"version": 1, "source_sha256": source_hash, "analysis_sha256": digest(analysis_path), "anchors": anchors}
    destination = target / "mesh-rig/source/face-landmarks.json"
    temporary = destination.with_suffix(".json.part")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("target", type=Path)
    args = parser.parse_args()
    import_landmarks(args.source, args.target)
