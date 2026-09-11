"""採用済みモデルだけをPicoAgent管理領域へ取得し、SHA-256検査後に公開する。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import time
import urllib.request

SPECS = [
    ("models/rembg/isnetis.onnx", "https://huggingface.co/skytnt/anime-seg/resolve/493cb60893f47441b26ec4fb9a306bce9e342982/isnetis.onnx?download=true", "f15622d853e8260172812b657053460e20806f04b9e05147d49af7bed31a6e99"),
    ("models/sam2.1-hiera-tiny/config.json", "https://huggingface.co/facebook/sam2.1-hiera-tiny/resolve/de431c4043854a71d8101e17995dfe596bf101a5/config.json?download=true", "860aff9751b139d83a4ad7df1e5535416fded533e0ead02625edbefcb9953cce"),
    ("models/sam2.1-hiera-tiny/preprocessor_config.json", "https://huggingface.co/facebook/sam2.1-hiera-tiny/resolve/de431c4043854a71d8101e17995dfe596bf101a5/preprocessor_config.json?download=true", "6ebf229ee259368ce4a8d4f2fe893a72b053023710853e257253939e601f583d"),
    ("models/sam2.1-hiera-tiny/processor_config.json", "https://huggingface.co/facebook/sam2.1-hiera-tiny/resolve/de431c4043854a71d8101e17995dfe596bf101a5/processor_config.json?download=true", "f8a68e865cfad115c1c2763f3d93eca7b1c622da06da2a9273eb437fb2389b6d"),
    ("models/sam2.1-hiera-tiny/model.safetensors", "https://huggingface.co/facebook/sam2.1-hiera-tiny/resolve/de431c4043854a71d8101e17995dfe596bf101a5/model.safetensors?download=true", "48c14467e5cf9e51870511feb72c89688e82dd74523142c0538b663e193ac2a7"),
]

GROUNDING_BASE = "https://huggingface.co/IDEA-Research/grounding-dino-base/resolve/12bdfa3120f3e7ec7b434d90674b3396eccf88eb"
for name, digest in {
    "README.md":"a0d03193076262a585dcb1edfe4b3b72fac678055008b688feb188cceb7f977d",
    "config.json":"eda416dae6f49419ff831b1c190ec430a060b19aae688dbaf2425a075b650608",
    "model.safetensors":"5548f844c928c4b6f411fa8cbcc2bfa8dbbba437cb1d513975519f93c2a9ed21",
    "preprocessor_config.json":"8454179ba95e2ad22947835aad7b45862a601fc0055ab88bf1ee70892d3aea60",
    "special_tokens_map.json":"b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3",
    "tokenizer.json":"d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    "tokenizer_config.json":"d40ab645b68211910b9170d22433d43186a6ec8ee6fd10ba170524b25bf4fb56",
    "vocab.txt":"07eced375cec144d27c900241f3e339478dec958f92fddbc551f295c992038a3",
}.items():
    SPECS.append((f"models/grounding-dino-base/{name}", f"{GROUNDING_BASE}/{name}?download=true", digest))

QWEN = [
    ("Comfy-Org/Qwen-Image_ComfyUI", "7beb7b647f04469fbe64ba8adc2bb0d7e5e9f73f", "text_encoders/qwen_2.5_vl_7b.safetensors", "cfafd739459bc86257397259f612a9aee88e5b98e85b5c0d0d1717e898b3463a"),
    ("Comfy-Org/Qwen-Image_ComfyUI", "7beb7b647f04469fbe64ba8adc2bb0d7e5e9f73f", "vae/qwen_image_vae.safetensors", "a70580f0213e67967ee9c95f05bb400e8fb08307e017a924bf3441223e023d1f"),
    ("Comfy-Org/Qwen-Image-Edit_ComfyUI", "984166f60a9b1fcede5e9b9287b7a7aebc050010", "diffusion_models/qwen_image_edit_2511_bf16.safetensors", "ae42d927b5fac4f278b9a894554c727e619727a63622976f2d95625be4bce08c"),
]
for repo, revision, name, digest in QWEN:
    SPECS.append((f"models/qwen-eval/{name}", f"https://huggingface.co/{repo}/resolve/{revision}/split_files/{name}?download=true", digest))

def sha256(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()

def write_progress(path, *, message, downloaded, total):
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps({
        "message": message,
        "downloaded": downloaded,
        "total": total,
    }, ensure_ascii=False), encoding="utf-8")
    temporary.replace(path)

def response_total(response, offset):
    content_range = response.headers.get("Content-Range", "")
    if "/" in content_range:
        tail = content_range.rsplit("/", 1)[-1]
        if tail.isdigit():
            return int(tail)
    length = response.headers.get("Content-Length", "")
    return offset + int(length) if length.isdigit() else 0

def download(root, cache, relative, url, expected, progress, index, count):
    destination = root / relative
    pending = cache / (relative.replace("/", "_") + ".part")
    destination.parent.mkdir(parents=True, exist_ok=True)
    pending.parent.mkdir(parents=True, exist_ok=True)
    for path in (destination, pending):
        if path.is_symlink() or path.absolute() != path.resolve():
            raise ValueError(f"モデル保存先にリンクを使用できません: {path}")
    if destination.is_file() and sha256(destination) == expected:
        write_progress(progress, message=f"モデル {index}/{count} を確認済み: {relative}", downloaded=1, total=1)
        print(json.dumps({"event":"verified", "file":relative}), flush=True)
        return
    if destination.exists():
        raise ValueError(f"既存モデルのSHA-256が一致しません: {relative}")
    offset = pending.stat().st_size if pending.exists() else 0
    request = urllib.request.Request(url, headers={"Range": f"bytes={offset}-"} if offset else {})
    with urllib.request.urlopen(request, timeout=120) as response:
        status = getattr(response, "status", 200)
        if offset and status != 206:
            raise ValueError(f"モデル取得の再開に対応しない応答です: {relative}")
        total = response_total(response, offset)
        downloaded = offset
        last_progress = 0.0
        write_progress(progress, message=f"モデル {index}/{count} を取得中: {relative}", downloaded=downloaded, total=total)
        with pending.open("ab" if offset else "wb") as output:
            while block := response.read(1024 * 1024):
                output.write(block)
                downloaded += len(block)
                now = time.monotonic()
                if now - last_progress >= 0.2:
                    write_progress(progress, message=f"モデル {index}/{count} を取得中: {relative}", downloaded=downloaded, total=total)
                    last_progress = now
            output.flush(); os.fsync(output.fileno())
    if sha256(pending) != expected:
        raise ValueError(f"モデルのSHA-256検査に失敗しました: {relative}")
    pending.replace(destination)
    write_progress(progress, message=f"モデル {index}/{count} を取得完了: {relative}", downloaded=downloaded, total=total or downloaded)
    print(json.dumps({"event":"complete", "file":relative}), flush=True)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--progress", type=Path)
    args = parser.parse_args()
    root = args.runtime.resolve()
    cache = args.cache.resolve()
    root.mkdir(parents=True, exist_ok=True)
    for index, (relative, url, digest) in enumerate(SPECS, 1):
        download(root, cache, relative, url, digest, args.progress, index, len(SPECS))

if __name__ == "__main__":
    main()
