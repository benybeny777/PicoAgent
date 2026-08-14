from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
from typing import Any

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = ROOT.parents[1]


def _load_validator():
    path = ROOT / "scripts" / "validate_vrm.py"
    spec = importlib.util.spec_from_file_location("validate_vrm", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load validate_vrm.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def validate_release(root: Path = REPOSITORY) -> tuple[list[str], dict[str, Any]]:
    errors: list[str] = []
    vrm_path = root / "characters" / "mugi-vrm-experimental" / "mugi.vrm"
    validator = _load_validator()
    errors.extend(validator.validate(vrm_path))
    document, _ = validator.read_glb(vrm_path)

    meshes = document.get("meshes", [])
    accessors = document.get("accessors", [])
    vertices = sum(
        accessors[primitive["attributes"]["POSITION"]]["count"]
        for mesh in meshes
        for primitive in mesh.get("primitives", [])
    )
    facial_grids = sum(
        mesh.get("extras", {}).get("grid") in ([4, 2], [5, 2]) for mesh in meshes
    )
    vrm = document["extensions"]["VRMC_vrm"]
    spring_bone = document["extensions"].get("VRMC_springBone", {})
    springs = spring_bone.get("springs", [])
    spring_joints = sum(max(0, len(spring.get("joints", [])) - 1) for spring in springs)
    metrics = {
        "bytes": vrm_path.stat().st_size,
        "meshes": len(meshes),
        "vertices": vertices,
        "facialGridMeshes": facial_grids,
        "presetExpressions": len(vrm.get("expressions", {}).get("preset", {})),
        "customExpressions": len(vrm.get("expressions", {}).get("custom", {})),
        "springChains": len(springs),
        "springJoints": spring_joints,
        "latestPreviews": 2,
    }
    expected = {
        "meshes": 17,
        "vertices": 347,
        "facialGridMeshes": 8,
        "presetExpressions": 17,
        "customExpressions": 4,
        "springChains": 3,
        "springJoints": 5,
        "latestPreviews": 2,
    }
    for name, value in expected.items():
        if metrics[name] != value:
            errors.append(f"quality metric {name}: expected {value}, got {metrics[name]}")
    if metrics["bytes"] > 10 * 1024 * 1024:
        errors.append("VRM exceeds the 10 MiB A-plan release budget")

    readme = (root / "README.md").read_text(encoding="utf-8")
    previews = {
        "character-mugi-vrm-experimental.webp": (960, 640),
        "character-mugi-vrm-experimental-stream.webp": (960, 540),
    }
    for name, expected_size in previews.items():
        path = root / "docs" / "images" / name
        if not path.is_file() or path.stat().st_size == 0:
            errors.append(f"missing runtime VRM preview: {path.relative_to(root)}")
            continue
        if f"docs/images/{name}" not in readme:
            errors.append(f"README does not link runtime VRM preview: {name}")
        with Image.open(path) as image:
            if image.format != "WEBP" or image.size != expected_size:
                errors.append(
                    f"runtime VRM preview {name}: expected WEBP {expected_size}, "
                    f"got {image.format} {image.size}"
                )
            if getattr(image, "n_frames", 1) != 134:
                errors.append(f"runtime VRM preview {name} must contain 134 frames")
    for obsolete in ("docs/vrm/mugi-vrm-preview.gif", "docs/vrm/mugi-vrm-preview.mp4"):
        if obsolete in readme:
            errors.append(f"README must not link obsolete VRM preview: {obsolete}")
    if "vrm-phase" in readme:
        errors.append("README must show only the latest VRM preview")
    if list((root / "docs" / "vrm").glob("vrm-phase*")):
        errors.append("obsolete VRM phase preview files must be removed")
    manifest = json.loads(
        (root / "characters" / "mugi-vrm-experimental" / "character.json").read_text(
            encoding="utf-8"
        )
    )
    if manifest.get("id") != "mugi-vrm-experimental" or manifest.get("vrm", {}).get("model") != "mugi.vrm":
        errors.append("experimental VRM character manifest is not wired to mugi.vrm")
    return errors, metrics


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate the complete Mugi VRM release")
    parser.parse_args()
    errors, metrics = validate_release()
    print(json.dumps({"metrics": metrics, "errors": errors}, ensure_ascii=False, indent=2))
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
