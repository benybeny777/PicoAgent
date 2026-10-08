"""Windows/NVIDIA用の学習環境を専用stageへ導入する。既存環境は交換しない。

失敗stageは診断用に残し、再試行は新しいstageで行う。成功後は同じ導入先の
所有記録がある失敗stageだけを回収する。取得は明示実行時のみ。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import time
import uuid
import zipfile
from pathlib import Path, PurePosixPath
from urllib.request import Request, urlopen

OFFICIAL_RELEASE = "2.7.0"
OFFICIAL_COMMIT = "d8148f3090ee5038ca7b4e4b327116c64467f952"
FORK = "sync-dev-org/Style-Bert-VITS2"
COMMIT = "72941d6b98013f2ab9d59bfdaea9ed5b53b7a846"
TREE = "4fb47281016d5c7c699c9f14b99f6366d5fd9ef0"
FORK_REASON = "公式2.7.0のtorch<2.4はBlackwell非対応。保守forkを固定し、CUDA12.8/Torch2.10と単GPU学習・Silero互換修正を再現する。"
MODEL_SPECS = (
    ("ku-nlp/deberta-v2-large-japanese-char-wwm", "547b0e8b044fba3f9b84d0ab9f990440bd130c8b", "bert/deberta-v2-large-japanese-char-wwm", ("pytorch_model.bin", "config.json", "tokenizer*", "special_tokens_map.json", "spm.model", "vocab.txt")),
    ("litagin/Style-Bert-VITS2-2.0-base-JP-Extra", "a731761009f3c96d104487be6ad332bf1bb5a3a5", "pretrained_jp_extra", ("G_0.safetensors", "D_0.safetensors", "WD_0.safetensors")),
    ("pyannote/wespeaker-voxceleb-resnet34-LM", "837717ddb9ff5507820346191109dc79c958d614", None, ("config.yaml", "pytorch_model.bin")),
    ("Systran/faster-whisper-large-v3", "edaa852ec7e145841d8ffdb056a99866b5f0a478", None, ("config.json", "model.bin", "tokenizer.json", "vocabulary.json", "preprocessor_config.json")),
)
SILERO_COMMIT = "0467799ff2f3df0f866bfa3867e9d97c6e5cce50"
SILERO_TREE = "87601798fc80deb0d25162372c5e43d7869322e6"

# 実績環境の学習・取得・QA依存closureを固定。ORTは共有module衝突を避けCUDA版だけにする。
FROZEN = """absl-py==2.5.0
accelerate==1.14.0
aivmlib==1.2.2
alembic==1.18.5
annotated-doc==0.0.4
annotated-types==0.7.0
anyio==4.14.1
asteroid-filterbanks==0.4.0
audioread==3.1.0
av==18.0.0
certifi==2026.6.17
cffi==2.1.0
charset-normalizer==3.4.7
click==8.4.2
cmudict==1.1.3
cn2an==0.5.24
colorama==0.4.6
colorlog==6.10.1
contourpy==1.3.3
ctranslate2==4.8.1
cycler==0.12.1
decorator==5.3.1
distance==0.1.3
docopt==0.6.2
e2k==0.6.2
einops==0.8.2
faster-whisper==1.2.1
filelock==3.29.6
flatbuffers==25.12.19
fonttools==4.63.0
fsspec==2026.6.0
g2p-en==2.1.0
googleapis-common-protos==1.75.0
gputil==1.4.0
greenlet==3.5.3
grpcio==1.82.0
h11==0.16.0
hf-xet==1.5.1
httpcore==1.0.9
httpx==0.28.1
huggingface-hub==1.22.0
idna==3.18
importlib-metadata==9.0.0
importlib-resources==7.1.0
inflect==7.5.0
jaconv==0.5.0
jieba==0.42.1
jinja2==3.1.6
joblib==1.5.3
julius==0.2.8
kiwisolver==1.5.0
lazy-loader==0.5
librosa==0.11.0
lightning==2.6.5
lightning-utilities==0.15.3
llvmlite==0.48.0
loguru==0.7.3
mako==1.3.12
markdown==3.10.2
markdown-it-py==4.2.0
markupsafe==3.0.3
matplotlib==3.11.0
mdurl==0.1.2
ml-dtypes==0.5.4
more-itertools==11.1.0
mpmath==1.3.0
msgpack==1.2.1
narwhals==2.23.0
networkx==3.6.1
nltk==3.8.1
num2words==0.5.14
numba==0.66.0
numpy==2.4.6
onnx==1.22.0
onnxconverter-common==1.16.0
onnxruntime-gpu==1.24.4
onnxsim-prebuilt==0.4.39.post2
opentelemetry-api==1.43.0
opentelemetry-exporter-otlp==1.43.0
opentelemetry-exporter-otlp-proto-common==1.43.0
opentelemetry-exporter-otlp-proto-grpc==1.43.0
opentelemetry-exporter-otlp-proto-http==1.43.0
opentelemetry-proto==1.43.0
opentelemetry-sdk==1.43.0
opentelemetry-semantic-conventions==0.64b0
optuna==4.9.0
packaging==26.2
pandas==2.3.3
pillow==11.3.0
platformdirs==4.10.0
pooch==1.9.0
primepy==1.3
proces==0.1.7
protobuf==7.35.1
psutil==7.2.2
pyannote-audio==4.0.7
pyannote-core==6.0.1
pyannote-database==6.1.1
pyannote-metrics==4.1
pyannote-pipeline==4.0.0
pyannoteai-sdk==0.4.0
pycparser==3.0
pydantic==2.12.3
pydantic-core==2.41.4
pygments==2.20.0
pyloudnorm==0.2.0
pynndescent==0.6.0
pyopenjtalk-plus==0.4.1.post8
pyparsing==3.3.2
pypinyin==0.55.0
python-dateutil==2.9.0.post0
pytorch-lightning==2.6.5
pytorch-metric-learning==2.9.0
pytz==2026.2
pyworld-prebuilt==0.3.5.post2
pyyaml==6.0.3
regex==2026.6.28
requests==2.32.5
rich==15.0.0
safetensors==0.8.0
scikit-learn==1.9.0
scipy==1.18.0
sentencepiece==0.2.1
setuptools==81.0.0
shellingham==1.5.4
six==1.17.0
sortedcontainers==2.4.0
soundfile==0.14.0
soxr==1.1.0
sqlalchemy==2.0.51
sudachidict-core==20260428
sudachipy==0.6.11
sympy==1.14.0
tensorboard==2.21.0
tensorboard-data-server==0.7.2
threadpoolctl==3.6.0
tokenizers==0.22.2
torch==2.10.0+cu128
torch-audiomentations==0.12.0
torch-pitch-shift==1.2.5
torchaudio==2.10.0+cu128
torchcodec==0.14.0
torchmetrics==1.9.0
tqdm==4.68.4
transformers==5.13.0
typeguard==4.5.2
typer==0.26.8
typing-extensions==4.16.0
typing-inspection==0.4.2
tzdata==2026.2
umap-learn==0.5.12
urllib3==2.6.3
werkzeug==3.1.8
wheel==0.46.3
win32-setctime==1.2.0
yt-dlp==2026.8.19
zipp==4.1.0
"""
PATCHES = {
    'slice.py': """diff --git a/slice.py b/slice.py
index b231ce8..d9691d0 100644
--- a/slice.py
+++ b/slice.py
@@ -5,13 +5,25 @@ from queue import Queue
 from threading import Thread
 from typing import Any, Optional
 
-import soundfile as sf
-import torch
-from tqdm import tqdm
+import soundfile as sf
+import torch
+import torchaudio
+from tqdm import tqdm
 
 from config import get_path_config
 from style_bert_vits2.logging import logger
-from style_bert_vits2.utils.stdout_wrapper import SAFE_STDOUT
+from style_bert_vits2.utils.stdout_wrapper import SAFE_STDOUT
+
+
+# 旧Silero VADが参照するAPIはtorchaudio 2.10で削除された。
+# 現行のtorchaudio.load経路へ進ませるため、互換用に空のbackend一覧を返す。
+if not hasattr(torchaudio, "list_audio_backends"):
+    def _load_wav_with_soundfile(path: str):
+        audio, sample_rate = sf.read(path, dtype="float32", always_2d=True)
+        return torch.from_numpy(audio.T), sample_rate
+
+    torchaudio.list_audio_backends = lambda: []
+    torchaudio.load = _load_wav_with_soundfile
 
 
 def is_audio_file(file: Path) -> bool:
""",
    'train_ms_jp_extra.py': """diff --git a/train_ms_jp_extra.py b/train_ms_jp_extra.py
index db78730..5004977 100644
--- a/train_ms_jp_extra.py
+++ b/train_ms_jp_extra.py
@@ -128,13 +128,20 @@ def run():
 
     local_rank = int(os.environ["LOCAL_RANK"])
     training_device = prepare_training_device(local_rank)
-    dist.init_process_group(
-        backend=training_device.backend,
-        init_method="env://",
-        timeout=datetime.timedelta(seconds=300),
-    )  # Use torchrun instead of mp.spawn
-    rank = dist.get_rank()
-    n_gpus = dist.get_world_size()
+    world_size = int(os.environ["WORLD_SIZE"])
+    if world_size > 1:
+        dist.init_process_group(
+            backend=training_device.backend,
+            init_method="env://",
+            timeout=datetime.timedelta(seconds=300),
+        )  # Use torchrun instead of mp.spawn
+        rank = dist.get_rank()
+        n_gpus = dist.get_world_size()
+    else:
+        # Windows の PyTorch 2.10 では Gloo の単一プロセス初期化が失敗する。
+        # 1 GPU 学習では通信不要なので、分散初期化自体を省略する。
+        rank = 0
+        n_gpus = 1
 
     hps = HyperParameters.load_from_json(args.config)
     # This is needed because we have to pass values to `train_and_evaluate()
@@ -404,28 +411,29 @@ def run():
         )
     else:
         optim_wd = None
-    net_g = DDP(
-        net_g,
-        device_ids=training_device.ddp_device_ids,
-        # bucket_cap_mb=512
-    )
-    net_d = DDP(
-        net_d,
-        device_ids=training_device.ddp_device_ids,
-        # bucket_cap_mb=512
-    )
-    if net_dur_disc is not None:
-        net_dur_disc = DDP(
-            net_dur_disc,
+    if world_size > 1:
+        net_g = DDP(
+            net_g,
             device_ids=training_device.ddp_device_ids,
-            # bucket_cap_mb=512,
+            # bucket_cap_mb=512
         )
-    if net_wd is not None:
-        net_wd = DDP(
-            net_wd,
+        net_d = DDP(
+            net_d,
             device_ids=training_device.ddp_device_ids,
-            #  bucket_cap_mb=512
+            # bucket_cap_mb=512
         )
+        if net_dur_disc is not None:
+            net_dur_disc = DDP(
+                net_dur_disc,
+                device_ids=training_device.ddp_device_ids,
+                # bucket_cap_mb=512,
+            )
+        if net_wd is not None:
+            net_wd = DDP(
+                net_wd,
+                device_ids=training_device.ddp_device_ids,
+                #  bucket_cap_mb=512
+            )
 
     if utils.is_resuming(model_dir):
         if net_dur_disc is not None:
@@ -738,12 +746,13 @@ def train_and_evaluate(
         bert,
         style_vec,
     ) in enumerate(train_loader):
-        if net_g.module.use_noise_scaled_mas:
+        generator_module = getattr(net_g, "module", net_g)
+        if generator_module.use_noise_scaled_mas:
             current_mas_noise_scale = (
-                net_g.module.mas_noise_scale_initial
-                - net_g.module.noise_scale_delta * global_step
+                generator_module.mas_noise_scale_initial
+                - generator_module.noise_scale_delta * global_step
             )
-            net_g.module.current_mas_noise_scale = max(current_mas_noise_scale, 0.0)
+            generator_module.current_mas_noise_scale = max(current_mas_noise_scale, 0.0)
         x, x_lengths = move_to_device(x, device), move_to_device(x_lengths, device)
         spec, spec_lengths = move_to_device(spec, device), move_to_device(
             spec_lengths, device
@@ -1091,7 +1100,8 @@ def evaluate(hps, generator, eval_loader, writer_eval, device: torch.device):
             language = move_to_device(language, device)
             style_vec = move_to_device(style_vec, device)
             for use_sdp in [True, False]:
-                y_hat, attn, mask, *_ = generator.module.infer(
+                generator_module = getattr(generator, "module", generator)
+                y_hat, attn, mask, *_ = generator_module.infer(
                     x,
                     x_lengths,
                     speakers,
""",
}


def atomic_json(path: Path, value: dict):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex + ".incoming")
    with temporary.open("x", encoding="utf-8") as handle:
        json.dump(value, handle, ensure_ascii=False, indent=2)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)


def api(url: str):
    with urlopen(Request(url, headers={"User-Agent": "PicoAgent-character-learning"}), timeout=30) as response:
        return json.load(response)


def git_blob(data: bytes) -> str:
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def file_digest(path: Path, lfs: bool) -> str:
    digest = hashlib.sha256() if lfs else hashlib.sha1()
    if not lfs:
        digest.update(b"blob " + str(path.stat().st_size).encode() + b"\0")
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def unpack_verified(archive: Path, destination: Path, expected: dict[str, str]):
    seen = set()
    with zipfile.ZipFile(archive) as zipped:
        for entry in zipped.infolist():
            components = PurePosixPath(entry.filename).parts
            if ".." in components or "\\" in entry.filename or entry.filename.startswith("/"):
                raise ValueError("取得書庫に範囲外のパスがあります")
            if entry.is_dir():
                continue
            if len(components) < 2 or (entry.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("取得書庫に不正なファイルがあります")
            name = "/".join(components[1:])
            data = zipped.read(entry)
            if name in seen or expected.get(name) != git_blob(data):
                raise ValueError(f"固定Git blobと内容が一致しません: {name}")
            target = destination.joinpath(*PurePosixPath(name).parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            seen.add(name)
    if seen != set(expected):
        raise ValueError("固定Git treeのファイルが欠損しています")


def fetch_source(repository: str, commit: str, tree: str, destination: Path, archive: Path):
    manifest = api(f"https://api.github.com/repos/{repository}/git/trees/{tree}?recursive=1")
    if manifest.get("sha") != tree or manifest.get("truncated"):
        raise ValueError("固定Git treeを取得できません")
    expected = {row["path"]: row["sha"] for row in manifest["tree"] if row["type"] == "blob"}
    with urlopen(f"https://codeload.github.com/{repository}/zip/{commit}", timeout=120) as response, archive.open("xb") as handle:
        shutil.copyfileobj(response, handle)
    unpack_verified(archive, destination, expected)
    archive.unlink()


def apply_patch(text: str, patch: str) -> str:
    """固定diffだけを適用し、元の文脈不一致では停止する。"""
    source = text.splitlines(keepends=True)
    output, offset = [], 0
    lines = patch.splitlines(keepends=True)
    index = 0
    while index < len(lines):
        match = re.match(r"@@ -(\d+)(?:,\d+)? \+\d+(?:,\d+)? @@", lines[index])
        if not match:
            index += 1
            continue
        start = int(match.group(1)) - 1
        output.extend(source[offset:start])
        offset = start
        index += 1
        while index < len(lines) and not lines[index].startswith("@@"):
            row = lines[index]
            if row.startswith((" ", "-")):
                if offset >= len(source) or source[offset].rstrip("\r\n") != row[1:].rstrip("\r\n"):
                    raise ValueError("互換patchの元コードが固定版と一致しません")
                if row.startswith(" "):
                    output.append(source[offset])
                offset += 1
            elif row.startswith("+"):
                output.append(row[1:])
            index += 1
    output.extend(source[offset:])
    return "".join(output)


def select_cuda_gpu(output: str) -> dict[str, str]:
    # PyTorch 2.10 cu128の公式wheel対象とCUDA 12.8 GAのWindows driverを取得前に判定する。
    supported = {(7, 0), (7, 5), (8, 0), (8, 6), (8, 7), (8, 9), (9, 0), (10, 0), (10, 3), (12, 0), (12, 1)}
    selected = None
    for line in output.splitlines():
        fields = [part.strip() for part in line.split(",")]
        if len(fields) != 3 or not re.fullmatch(r"\d+", fields[0]):
            raise ValueError("nvidia-smiのGPU能力応答が不正です。取得していません")
        if not re.fullmatch(r"\d+\.\d+", fields[1]) or not re.fullmatch(r"\d+\.\d+(?:\.\d+)?", fields[2]):
            raise ValueError("nvidia-smiのGPU能力応答が不正です。取得していません")
        capability = tuple(map(int, fields[1].split(".")))
        driver = tuple(map(int, fields[2].split(".")))
        if selected is None and capability in supported and driver >= (570, 65):
            selected = {"CUDA_VISIBLE_DEVICES": fields[0], "CUDA_DEVICE_ORDER": "PCI_BUS_ID"}
    if selected is not None:
        return selected
    raise ValueError("PyTorch 2.10 cu128対応GPUとWindows NVIDIA driver 570.65以上が必要です。取得していません")


def detect_cuda_gpu() -> dict[str, str]:
    executable = shutil.which("nvidia-smi")
    if not executable:
        raise ValueError("nvidia-smiがなくGPU能力を確認できません。取得していません")
    try:
        result = subprocess.run([executable, "--query-gpu=index,compute_cap,driver_version", "--format=csv,noheader,nounits"],
                                capture_output=True, text=True, check=True, timeout=15,
                                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    except (OSError, subprocess.SubprocessError) as error:
        raise ValueError("NVIDIA GPU能力の確認に失敗しました。取得していません") from error
    return select_cuda_gpu(result.stdout)


def environment(root: Path, gpu: dict[str, str] | None = None) -> dict[str, str]:
    return {"HF_HOME": str(root / "cache/huggingface"), "HF_HUB_CACHE": str(root / "cache/huggingface/hub"),
            "TORCH_HOME": str(root / "cache/torch"), "PYTHONPATH": str(root),
            "HF_HUB_OFFLINE": "1", "TRANSFORMERS_OFFLINE": "1", "HF_DATASETS_OFFLINE": "1"} | (gpu or {})


def run(command: list[str], cwd: Path, env: dict, log: Path):
    with log.open("ab") as handle:
        handle.write(("実行: " + json.dumps(command, ensure_ascii=False) + "\n").encode())
        handle.flush()
        subprocess.run(command, cwd=cwd, env=env, stdout=handle, stderr=subprocess.STDOUT, check=True)


def acquire_models(root: Path, python: Path, env: dict, log: Path):
    # APIから固定revisionのGit/LFSハッシュを得て、snapshotの全選択ファイルを照合する。
    for repository, revision, local, patterns in MODEL_SPECS:
        metadata = api(f"https://huggingface.co/api/models/{repository}/revision/{revision}?blobs=true")
        if metadata["sha"] != revision:
            raise ValueError("音声学習資産のrevisionが一致しません")
        code = "from huggingface_hub import snapshot_download; import sys; snapshot_download(repo_id=sys.argv[1],revision=sys.argv[2],allow_patterns=sys.argv[3:])"
        online = dict(env, HF_HUB_OFFLINE="0", TRANSFORMERS_OFFLINE="0")
        run([str(python), "-c", code, repository, revision, *patterns], root, online, log)
        snapshot = root / "cache/huggingface/hub" / ("models--" + repository.replace("/", "--")) / "snapshots" / revision
        files = list(snapshot.rglob("*"))
        count = 0
        for file in files:
            if not file.is_file():
                continue
            name = file.relative_to(snapshot).as_posix()
            entry = next((row for row in metadata["siblings"] if row["rfilename"] == name), None)
            if entry is None:
                raise ValueError(f"未定義の学習資産があります: {name}")
            expected = entry.get("lfs", {}).get("sha256")
            valid = file_digest(file, bool(expected)) == (expected or entry.get("blobId"))
            if not valid:
                raise ValueError(f"音声学習資産のハッシュが一致しません: {repository}/{name}")
            if local:
                destination = root / local / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(file, destination)
            count += 1
        if count == 0:
            raise ValueError(f"必要な学習資産が取得できません: {repository}")
        # 既存workerのモデル名参照を、固定snapshotへオフラインで解決する。
        reference = snapshot.parent.parent / "refs/main"
        reference.parent.mkdir(parents=True, exist_ok=True)
        reference.write_text(revision, encoding="utf-8")


PROBE = r'''
import json,sys,importlib
from pathlib import Path
import torch,torchaudio
assert sys.version_info[:2]==(3,12), 'Python3.12が必要'
assert torch.__version__=='2.10.0+cu128' and torchaudio.__version__=='2.10.0+cu128'
assert torch.cuda.is_available(), 'CUDAを利用できません'
# Blackwellで実際にCUDAカーネルを実行できることまで確認する。
assert (torch.ones(16,device='cuda')*2).sum().item()==32
for module in ('aivmlib','onnx','onnxsim','faster_whisper','pyannote.audio','soundfile','yt_dlp','transformers','style_bert_vits2'):
    importlib.import_module(module)
import pyopenjtalk
assert pyopenjtalk.extract_fullcontext('こんにちは'), '日本語辞書を利用できません'
root=Path.cwd()
assert all((root/'pretrained_jp_extra'/name).is_file() for name in ('G_0.safetensors','D_0.safetensors','WD_0.safetensors'))
from transformers import AutoTokenizer,AutoModel
bert=str(root/'bert/deberta-v2-large-japanese-char-wwm')
AutoTokenizer.from_pretrained(bert,local_files_only=True)
AutoModel.from_pretrained(bert,local_files_only=True)
from faster_whisper import WhisperModel
WhisperModel('large-v3',device='cpu',compute_type='int8',local_files_only=True)
from pyannote.audio import Model
Model.from_pretrained('pyannote/wespeaker-voxceleb-resnet34-LM')
torch.hub.load(str(root/'cache/torch/hub/litagin02_silero-vad_master'),'silero_vad',source='local',trust_repo=True,onnx=False)
print(json.dumps({'cuda':True,'device':torch.cuda.get_device_name(0)}))
'''


def populate(stage: Path, shared: Path, uv: Path, ffmpeg: Path, gpu: dict[str, str]):
    fetch_source(FORK, COMMIT, TREE, stage, stage / "source.zip")
    for name, patch in PATCHES.items():
        path = stage / name
        path.write_text(apply_patch(path.read_text(encoding="utf-8"), patch), encoding="utf-8", newline="\n")
    env = os.environ | environment(stage, gpu)
    log = stage / "setup.log"
    run([str(uv), "venv", str(stage / ".venv"), "--python", str(shared), "--relocatable"], stage, env, log)
    python = stage / ".venv/Scripts/python.exe"
    frozen = stage / "requirements-runtime.txt"
    frozen.write_text(FROZEN, encoding="utf-8")
    run([str(uv), "pip", "install", "--python", str(python), "setuptools==81.0.0", "wheel==0.46.3"], stage, env, log)
    run([str(uv), "pip", "install", "--python", str(python), "--no-build-isolation", "--no-deps", "--extra-index-url", "https://download.pytorch.org/whl/cu128", "--index-strategy", "unsafe-best-match", "-r", str(frozen)], stage, env, log)
    site = stage / ".venv/Lib/site-packages"
    (site / "picoagent-sbv2.pth").write_text("../../../\n", encoding="utf-8")
    acquire_models(stage, python, env, log)
    fetch_source("litagin02/silero-vad", SILERO_COMMIT, SILERO_TREE, stage / "cache/torch/hub/litagin02_silero-vad_master", stage / "silero.zip")
    shutil.copyfile(stage / "configs/default_paths.yml", stage / "configs/paths.yml")
    shutil.copyfile(stage / "default_config.yml", stage / "config.yml")
    run([str(ffmpeg), "-version"], stage, env, log)
    run([str(python), "-m", "yt_dlp", "--version"], stage, env, log)
    run([str(python), "-c", PROBE], stage, env, log)


def cleanup_failed_stages(root: Path):
    for candidate in root.parent.glob(".installing-" + root.name + "-*"):
        if candidate.is_symlink() or candidate.resolve().parent != root.parent:
            continue
        owner, failed = candidate / "stage-owner.json", candidate / "failure.json"
        if not owner.is_file() or not failed.is_file():
            continue
        try:
            data = json.loads(owner.read_text(encoding="utf-8"))
        except (ValueError, OSError):
            continue
        if data.get("destination") == str(root):
            shutil.rmtree(candidate)


def verify_existing(root: Path, ffmpeg: Path, status: Path, gpu: dict[str, str]):
    record = json.loads((root / "install.json").read_text(encoding="utf-8"))
    expected_patches = {name: hashlib.sha256(patch.encode()).hexdigest() for name, patch in PATCHES.items()}
    if (record.get("state") != "installed"
            or record.get("source", {}).get("repository") != FORK
            or record.get("source", {}).get("commit") != COMMIT
            or record.get("source", {}).get("tree") != TREE
            or record.get("requirementsSha256") != hashlib.sha256(FROZEN.encode()).hexdigest()
            or record.get("patchSha256") != expected_patches
            or (root / "requirements-runtime.txt").read_text(encoding="utf-8") != FROZEN
            or json.loads((root / "runtime-env.json").read_text(encoding="utf-8")) != record
            or {key: value for key, value in record.get("environment", {}).items()
                if key not in ("CUDA_VISIBLE_DEVICES", "CUDA_DEVICE_ORDER")} != environment(root)):
        raise ValueError("既存学習環境の固定版記録が一致しません。環境は変更しません")
    # 確認ログはジョブ側へ置き、既存rootのファイルを上書きしない。
    status.parent.mkdir(parents=True, exist_ok=True)
    env, log = os.environ | environment(root, gpu), status.parent / "runtime-check.log"
    run([str(ffmpeg), "-version"], root, env, log)
    run([str(root / ".venv/Scripts/aivmlib.exe"), "--help"], root, env, log)
    run([str(root / ".venv/Scripts/python.exe"), "-m", "yt_dlp", "--version"], root, env, log)
    run([str(root / ".venv/Scripts/python.exe"), "-c", PROBE], root, env, log)
    atomic_json(status, {"state": "completed", "phase": "setup", "message": "既存音声学習環境の検証が完了しました", "runtimeRoot": str(root)})


def recover_unrecorded_root(root: Path):
    try:
        owner = json.loads((root / "stage-owner.json").read_text(encoding="utf-8"))
    except (ValueError, OSError) as error:
        raise ValueError("未確定環境の所有記録を確認できません。環境は変更しません") from error
    if (owner.get("destination") != str(root) or type(owner.get("pid")) is not int
            or owner["pid"] <= 0 or root.is_symlink()
            or (hasattr(root, "is_junction") and root.is_junction())):
        raise ValueError("未確定環境の所有記録が一致しません。環境は変更しません")
    # 取得資産とは別の学習・利用者データがある場合は退避も再導入もしない。
    for name in ("Data", "model_assets", "characters", "jobs", "outputs"):
        path = root / name
        if path.exists() and (not path.is_dir() or path.is_symlink()
                or (hasattr(path, "is_junction") and path.is_junction())
                or any(item.is_symlink() or (hasattr(item, "is_junction") and item.is_junction())
                       or (item.is_file() and item.name != ".gitkeep") for item in path.rglob("*"))):
            raise ValueError("未確定環境に利用者データがあります。環境は変更しません")
    stage = root.with_name(".installing-" + root.name + "-" + uuid.uuid4().hex)
    if stage.exists() or stage.parent.resolve() != root.parent:
        raise ValueError("診断stageの境界を確認できません")
    root.rename(stage)
    atomic_json(stage / "failure.json", {"state": "failed", "message": "導入記録の確定前に中断", "retry": "所有確認済みの未確定rootを診断stageへ退避し、新規stageで再試行"})


def install(root: Path, shared: Path, ffmpeg: Path, uv: Path, status: Path, confirmed: bool):
    if root.absolute() != root.resolve():
        raise ValueError("導入先に別領域へのリンクを指定できません")
    root, shared, ffmpeg, uv, status = [path.resolve() for path in (root, shared, ffmpeg, uv, status)]
    if platform.system() != "Windows" or platform.machine().lower() not in ("amd64", "x86_64") or not confirmed:
        raise ValueError("Windows x64とNVIDIA GPUの事前確認が必要です。何も取得していません")
    gpu = detect_cuda_gpu()
    if root == root.parent or root in shared.parents or shared in root.parents or status == root or root in status.parents:
        raise ValueError("既存環境・共有Python・導入先内statusは変更できません")
    for file in (shared, ffmpeg, uv):
        if not file.is_file() or root in file.parents:
            raise ValueError(f"管理実体の実行ファイルがありません: {file}")
    subprocess.run([str(shared), "-c", "import sys; assert sys.version_info[:2]==(3,12)"], env=os.environ | gpu, check=True)
    if root.exists():
        if (root / "install.json").exists():
            try:
                verify_existing(root, ffmpeg, status, gpu)
            except BaseException as error:
                atomic_json(status, {"state": "failed", "phase": "setup", "message": str(error)})
                raise
            return
        recover_unrecorded_root(root)
    root.parent.mkdir(parents=True, exist_ok=True)
    stage = root.with_name(".installing-" + root.name + "-" + uuid.uuid4().hex)
    stage.mkdir()
    atomic_json(stage / "stage-owner.json", {"destination": str(root), "pid": os.getpid()})
    atomic_json(status, {"state": "running", "phase": "setup", "message": "専用音声学習環境を導入中"})
    try:
        populate(stage, shared, uv, ffmpeg, gpu)
        # relocatablevenvでも実CLIとcacheの移動後解決を確認してからinstalledを記録する。
        stage.rename(root)
        final_env = os.environ | environment(root, gpu)
        try:
            run([str(root / ".venv/Scripts/aivmlib.exe"), "--help"], root, final_env, root / "setup.log")
            run([str(root / ".venv/Scripts/python.exe"), "-c", PROBE], root, final_env, root / "setup.log")
        except BaseException:
            root.rename(stage)
            raise
        record = {"schemaVersion": 1, "state": "installed", "source": {"repository": FORK, "commit": COMMIT, "tree": TREE, "officialRelease": OFFICIAL_RELEASE, "officialCommit": OFFICIAL_COMMIT, "reason": FORK_REASON},
                  "patchSha256": {name: hashlib.sha256(patch.encode()).hexdigest() for name, patch in PATCHES.items()},
                  "requirementsSha256": hashlib.sha256(FROZEN.encode()).hexdigest(), "environment": environment(root, gpu), "ffmpeg": str(ffmpeg), "sharedPython": str(shared), "capabilities": {"windows": True, "nvidia": True, "cuda": True, "jpExtra": True, "transcribe": True, "export": True, "qa": True}, "models": [{"repository": row[0], "revision": row[1]} for row in MODEL_SPECS]}
        atomic_json(root / "runtime-env.json", record)
        atomic_json(root / "install.json", record)
        cleanup_failed_stages(root)
        atomic_json(status, {"state": "completed", "phase": "setup", "message": "音声学習環境の導入と検証が完了しました", "runtimeRoot": str(root)})
    except BaseException as error:
        if stage.exists():
            atomic_json(stage / "failure.json", {"state": "failed", "message": str(error), "retry": "診断stageは残し、新規stageで再試行。利用者の既存資産は変更しない"})
        atomic_json(status, {"state": "failed", "phase": "setup", "message": str(error), "diagnosticStage": str(stage)})
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--shared-python", type=Path, required=True)
    parser.add_argument("--status", type=Path, required=True)
    parser.add_argument("--ffmpeg-path", type=Path, required=True)
    parser.add_argument("--uv", "--uv-path", dest="uv_path", type=Path)
    parser.add_argument("--nvidia-confirmed", action="store_true")
    args = parser.parse_args()
    try:
        uv = args.uv_path
        if uv is None:
            candidates = list((args.root.parent / "embedding-install-tools/uv").rglob("uv.exe"))
            if len(candidates) != 1:
                raise ValueError("管理uvの実体を--uv-pathで指定してください")
            uv = candidates[0]
        install(args.root, args.shared_python, args.ffmpeg_path, uv, args.status, args.nvidia_confirmed)
        return 0
    except Exception as error:
        detail = str(error)[:2000] or type(error).__name__
        try:
            # 不正なstatus指定で既存環境や共有Pythonへ書き込まない。
            status, root, shared = args.status.resolve(), args.root.resolve(), args.shared_python.resolve()
            if status == root or root in status.parents or status == shared or shared.parent in status.parents:
                raise ValueError("診断statusは既存環境と共有Pythonの外へ指定してください")
            atomic_json(status, {"state": "failed", "phase": "setup", "error": detail})
        except Exception as status_error:
            print(f"失敗状態を保存できません: {status_error}", file=sys.stderr)
        print(detail, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
