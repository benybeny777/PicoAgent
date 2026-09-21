"""任意導入するNVFP4モデルの固定資産と取得前条件。"""
import os
from pathlib import Path
import platform
import sys

LICENSE_REVISION = "2026-09-21"
KREA = "krea-2-turbo-nvfp4"
LTX = "ltx-2.5-nvfp4"


def asset(repository, revision, path, sha256, size):
    return (path, f"https://huggingface.co/{repository}/resolve/{revision}/{path}", sha256, size)


MODELS = {
    KREA: [
        asset("Comfy-Org/Krea-2", "e5ea8b4dd7f38f348b138eb0fe29f92c0e367e96", "diffusion_models/krea2_turbo_nvfp4.safetensors", "61527003b2d537055494d01bc8efe51d6e86e64192ba23e3721a5647231fe394", 7673668448),
        asset("Comfy-Org/Krea-2", "e5ea8b4dd7f38f348b138eb0fe29f92c0e367e96", "text_encoders/qwen3vl_4b_fp8_scaled.safetensors", "54bd5144df0bbc25dd6ccadfcb826b521445a1b06ae5a42570bdd2974ca87094", 5242467968),
        asset("Comfy-Org/Krea-2", "e5ea8b4dd7f38f348b138eb0fe29f92c0e367e96", "vae/qwen_image_vae.safetensors", "a70580f0213e67967ee9c95f05bb400e8fb08307e017a924bf3441223e023d1f", 253806246),
        ("licenses/Krea-2-LICENSE.pdf", "https://huggingface.co/Comfy-Org/Krea-2/resolve/e5ea8b4dd7f38f348b138eb0fe29f92c0e367e96/LICENSE.pdf", "b82a2805162bde714a4eb27b9063c4fc3345d08a30be055134a6160e5430ba74", 137711),
    ],
    LTX: [
        asset("Lightricks/LTX-2.5", "5e6e71018ee1756ed329b697a7b4aedc934dfce9", "diffusion_models/ltx-2.5-22b-distilled-transformer-nvfp4.safetensors", "4b94231e734c1950f8f6826cb8bd8715d94be5b3e04f8256ee060c5bc3886c30", 18721548408),
        asset("Lightricks/LTX-2.5", "5e6e71018ee1756ed329b697a7b4aedc934dfce9", "text_encoders/gemma4-12b-with-proj-ltx-2.5-comfy-int8-convrot.safetensors", "6ce688a0aa98a5fa36a9f1e6c3f42152a498cc2b53ee8c15674c64244f91487f", 15372969374),
        asset("Lightricks/LTX-2.5", "5e6e71018ee1756ed329b697a7b4aedc934dfce9", "vae/ltx-2.5-audio-vae-bf16.safetensors", "c52733d37f6a7fb7949c3dc0fb468c6cb2169e4d836983a73babb9f0d54837a5", 364866540),
        asset("Lightricks/LTX-2.5", "5e6e71018ee1756ed329b697a7b4aedc934dfce9", "vae/ltx-2.5-video-vae-conv-bf16.safetensors", "685b06ee3d9b2039647698fc4ea33175112462fc374e2777312c907897dfce8d", 1452269922),
        ("licenses/LTX-LICENSE.txt", "https://raw.githubusercontent.com/Lightricks/LTX-2/a95ab856bf29407b6b066ede0abe1846050db56c/LICENSE-2_x", "be75acae5c99b0fb16ed6cfbf8f731e5121a729bef112d20337699407e796451", 30399),
    ],
}


def check_hardware():
    """未確認のGPUへCUDA資産を取得しない。8GB級は評価条件で動作保証ではない。"""
    if sys.platform != "win32" or platform.machine().lower() not in ("amd64", "x86_64"):
        raise RuntimeError("このNVFP4固定ランタイムはWindows x64専用です")
    import psutil
    import torch
    if not torch.cuda.is_available() or torch.version.hip:
        raise RuntimeError("NVFP4版はCUDA対応NVIDIA Blackwell以降のGPUが必要です")
    device = torch.cuda.current_device()
    if torch.cuda.get_device_capability(device)[0] < 12:
        raise RuntimeError("NVFP4版は演算能力12.0以上のNVIDIA GPUが必要です")
    if torch.cuda.get_device_properties(device).total_memory < 8_000_000_000:
        raise RuntimeError("NVFP4評価構成には8GB以上のVRAMが必要です")
    if psutil.virtual_memory().total < 32_000_000_000:
        raise RuntimeError("NVFP4評価構成には32GB以上のシステムメモリが必要です")


def hf_token():
    """利用者が標準HF CLIへ保存した認証を再利用し、値は表示しない。"""
    token = os.environ.get("HF_TOKEN", "").strip()
    if token:
        return token
    home = Path(os.environ.get("HF_HOME", Path.home() / ".cache/huggingface"))
    token_file = Path(os.environ.get("HF_TOKEN_PATH", home / "token"))
    return token_file.read_text(encoding="utf-8").strip() if token_file.is_file() else ""
