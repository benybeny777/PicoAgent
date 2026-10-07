"""標準の記憶検索専用環境を導入する。画像生成環境は変更しない。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import urllib.request

REVISION = "914f7f89142e33e77833254d9c9b90c3cef7303b"
FILES = {
    "1_Pooling/config.json": (90, "2730fc1de168630e474c4066987f46d091de2189"),
    "2_Normalize/config.json": (97, "8babba92405167e8718211dc51f44fb117e423e4"),
    "config.json": (4455, "dc524b2cbef566d66d8533c67dc68a6a8f5d340a"),
    "config_sentence_transformers.json": (1565, "ba7dd4d5481cbaf271d5e8d1ad4271c5a45d2fd7"),
    "modules.json": (413, "1ab20c8ab29b8338488b415e5c09af14b742fe3d"),
    "sentence_bert_config.json": (747, "b3b98f8308641875fca1e4ddbfae5f38996bb54f"),
    "tokenizer_config.json": (1599, "a9afd12fb3ca2c418fd405320ace85ad46748d03"),
    "tokenizer.json": (32170510, "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4"),
    "tokenizer.model": (4689013, "e594c8a90eb08d8bda498ff4747977dc827ae0c3c56b5c0d41a605a22d02ef03"),
    "model.safetensors": (1488915288, "197a32965d4b1105faf060417baa899e193fb73cd401f42ec9295234d5553d79"),
    "preprocessor_config.json": (511, "4adff307a29874c969eda7edb5d3f07faaef879d"),
    "processor_config.json": (1788, "63043a5a2fd2e25a2fc5336fcdaf24e515d12289"),
    "chat_template.jinja": (1016, "8df4ab8961aa08b66b470c5adbdc51374aabb63b"),
}


def verify(path, size, digest):
    if not path.is_file() or path.stat().st_size != size:
        return False
    h = hashlib.sha256() if len(digest) == 64 else hashlib.sha1()
    if len(digest) == 40:
        h.update(f"blob {size}\0".encode())
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1048576), b""):
            h.update(chunk)
    return h.hexdigest() == digest


def main():
    args = argparse.ArgumentParser()
    args.add_argument("--root", type=Path, required=True)
    args.add_argument("--uv", type=Path, required=True)
    args.add_argument("--backend", choices=["cuda", "cpu", "mps"], required=True)
    options = args.parse_args()
    root = options.root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    os.environ["UV_CACHE_DIR"] = str(root / "install-cache")
    venv = root / "venv"
    python = venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python3")
    expected = {"revision": REVISION, "backend": options.backend, "torch": "2.11.0", "transformers": "5.19.0", "sentence_transformers": "6.1.0", "torchvision": "0.26.0", "pillow": "12.3.0", "requirements_revision": "2026-10-07.1"}
    marker_path = root / "install.json"
    if marker_path.is_file():
        try:
            marker = json.loads(marker_path.read_text(encoding="utf-8"))
        except (ValueError, UnicodeError):
            marker = None
        if marker == expected and python.is_file() and all(verify(root / "model" / name, size, digest) for name, (size, digest) in FILES.items()):
            return
        marker_path.unlink()
    temporary_root = root / "install-cache" / "temp"
    temporary_root.mkdir(parents=True, exist_ok=True)
    for key in ("TMP", "TEMP", "TMPDIR"):
        os.environ[key] = str(temporary_root)
    if not python.is_file():
        subprocess.run([str(options.uv), "venv", str(venv), "--python", sys.executable, "--relocatable", "--allow-existing"], check=True)
    # 推移依存も版・配布ハッシュで固定し、GPU版とCPU版を同時に取得しない。
    lock = Path(__file__).parent / ("requirements-cuda.lock" if options.backend == "cuda" else "requirements-cpu.lock")
    command = [str(options.uv), "pip", "install", "--python", str(python), "--no-cache", "--require-hashes", "--only-binary", ":all:", "--torch-backend", "cu128" if options.backend == "cuda" else "cpu", "-r", str(lock)]
    subprocess.run(command, check=True)
    model = root / "model"
    for name, (size, digest) in FILES.items():
        target = model / name
        if verify(target, size, digest):
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        temporary = target.with_name(target.name + ".part")
        request = urllib.request.Request(f"https://huggingface.co/google/embeddinggemma-2/resolve/{REVISION}/{name}")
        with urllib.request.urlopen(request, timeout=60) as source, temporary.open("wb") as output:
            received = 0
            while chunk := source.read(1048576):
                received += len(chunk)
                if received > size:
                    raise RuntimeError(f"モデルが予定サイズを超えています: {name}")
                output.write(chunk)
        if not verify(temporary, size, digest):
            raise RuntimeError(f"モデルのサイズまたはハッシュが不一致です: {name}")
        temporary.replace(target)
    test = "import torch; from sentence_transformers import SentenceTransformer; "
    test += f"m=SentenceTransformer({str(model)!r},device={options.backend!r},local_files_only=True,config_kwargs={{'vision_config':None,'audio_config':None}},model_kwargs={{'torch_dtype':torch.bfloat16 if {options.backend!r}=='cuda' else torch.float32}}); "
    test += "v=m.encode(['記憶検索の動作確認'],prompt_name='SearchQuery',normalize_embeddings=True); assert v.shape[0]==1; assert torch.isfinite(torch.as_tensor(v)).all()"
    subprocess.run([str(python), "-c", test], check=True)
    marker = root / "install.json.part"
    marker.write_text(json.dumps(expected), encoding="utf-8")
    if (root / "install-cache").exists():
        shutil.rmtree(root / "install-cache")
    marker.replace(root / "install.json")


if __name__ == "__main__":
    main()
