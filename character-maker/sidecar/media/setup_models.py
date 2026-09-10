"""選択された動画モデルだけを固定版・固定SHA-256で取得する。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import time
import urllib.request


MODELS = {
    "wan-2.2-5b": [
        ("diffusion_models/wan2.2_ti2v_5B_fp16.safetensors", "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/c4f60d30c55a624e35427060fdd217579a6c1d77/split_files/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors?download=true", "456f901338bd9eadbded3828b819109a9b68e8a525ca5cf8d0049a69fcfeca1e", 9_999_658_848),
        ("vae/wan2.2_vae.safetensors", "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/c4f60d30c55a624e35427060fdd217579a6c1d77/split_files/vae/wan2.2_vae.safetensors?download=true", "e40321bd36b9709991dae2530eb4ac303dd168276980d3e9bc4b6e2b75fed156", 1_409_400_960),
        ("text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors", "https://huggingface.co/Comfy-Org/Wan_2.1_ComfyUI_repackaged/resolve/617a7633e636506f850e043bc4605f290a466a8e/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors?download=true", "c3355d30191f1f066b26d93fba017ae9809dce6c627dda5f6a66eaa651204f68", 6_735_906_897),
    ],
    "ltx-0.9.8-2b-fp8": [
        ("checkpoints/ltxv-2b-0.9.8-distilled-fp8.safetensors", "https://huggingface.co/Lightricks/LTX-Video/resolve/8984fa25007f376c1a299016d0957a37a2f797bb/ltxv-2b-0.9.8-distilled-fp8.safetensors?download=true", "d6d8fa8ed3a98346787c2503ac80fb5d7cebcf80e356b79a2ba361fbadf97e15", 4_461_695_684),
        ("text_encoders/t5xxl_fp16.safetensors", "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/6af2a98e3f615bdfa612fbd85da93d1ed5f69ef5/t5xxl_fp16.safetensors?download=true", "6e480b09fae049a72d2a8c5fbccb8d3e92febeb233bbe9dfe7256958a9167635", 9_787_841_024),
    ],
}
VERSIONS = {"wan-2.2-5b": "wan-2.2-5b", "ltx-0.9.8-2b-fp8": "0.9.8"}


def digest(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def progress(path, message, downloaded, total):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps({"message": message, "downloaded": downloaded, "total": total}, ensure_ascii=False), encoding="utf-8")
    temporary.replace(path)


def safe_path(root, relative):
    target = root / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.is_symlink() or target.resolve().parent != target.parent.resolve():
        raise ValueError(f"モデル保存先にリンクを使用できません: {target}")
    return target


def download(root, cache, item, index, count, progress_path, total_all, completed):
    relative, url, expected, expected_size = item
    destination = safe_path(root, relative)
    pending = safe_path(cache, relative.replace("/", "_") + ".part")
    if destination.is_file() and destination.stat().st_size == expected_size and digest(destination) == expected:
        progress(progress_path, f"モデル {index}/{count} を確認済み: {relative}", completed + expected_size, total_all)
        return expected_size
    if destination.exists():
        raise ValueError(f"既存モデルが固定値と一致しません: {relative}")
    offset = pending.stat().st_size if pending.exists() else 0
    if offset > expected_size:
        pending.unlink()
        offset = 0
    request = urllib.request.Request(url, headers={"Range": f"bytes={offset}-"} if offset else {})
    with urllib.request.urlopen(request, timeout=120) as response:
        if offset and getattr(response, "status", 200) != 206:
            raise ValueError(f"モデル取得の再開に対応しない応答です: {relative}")
        with pending.open("ab" if offset else "wb") as output:
            downloaded = offset
            last = 0.0
            while block := response.read(1024 * 1024):
                output.write(block)
                downloaded += len(block)
                now = time.monotonic()
                if now - last >= 0.2:
                    progress(progress_path, f"モデル {index}/{count} を取得中: {relative}", completed + downloaded, total_all)
                    last = now
            output.flush()
            os.fsync(output.fileno())
    if pending.stat().st_size != expected_size or digest(pending) != expected:
        raise ValueError(f"モデルのサイズまたはSHA-256検査に失敗しました: {relative}")
    pending.replace(destination)
    progress(progress_path, f"モデル {index}/{count} を取得完了: {relative}", completed + expected_size, total_all)
    return expected_size


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", choices=MODELS, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--progress", type=Path, required=True)
    args = parser.parse_args()
    root = args.runtime.resolve()
    cache = args.cache.resolve()
    root.mkdir(parents=True, exist_ok=True)
    cache.mkdir(parents=True, exist_ok=True)
    items = MODELS[args.model]
    total = sum(item[3] for item in items)
    completed = 0
    for index, item in enumerate(items, 1):
        completed += download(root, cache, item, index, len(items), args.progress, total, completed)
    (root / "install.json").write_text(json.dumps({"version": VERSIONS[args.model], "model": args.model, "files": [item[0] for item in items]}, ensure_ascii=False, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
