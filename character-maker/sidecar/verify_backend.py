"""導入したPyTorchが実GPUで動くことを、モデル取得の前に確かめる。

対応表に載っているだけでは有効にしない。import、GPUの見え方、固定したアクセラレータ版、
デバイス名、最小の演算までを実際に通し、1つでも欠けたら失敗にする。失敗したバックエンドで
77GBのモデル取得を始めない。

ROCm版PyTorchも互換APIとして`torch.cuda`名前空間を使う。`.to("rocm")`のような置換はしない。
MPS（Apple Silicon）だけは`torch.cuda`ではなく`torch.backends.mps`と`mps`デバイスを使う。
"""
import argparse
import json
import sys


def check_mps(torch, detail):
    """Apple SiliconのMetal（MPS）バックエンドを確かめる。"""
    if not torch.backends.mps.is_available():
        built = torch.backends.mps.is_built()
        raise SystemExit(json.dumps(
            {"error": f"MPSを利用できません（is_available=False, is_built={built}）", **detail},
            ensure_ascii=False))
    device = torch.device("mps")
    detail["device"] = "Apple Silicon GPU (MPS)"
    # 最小テンソル演算。MPSで実際に計算できることを確かめる。
    left = torch.ones((64, 64), device=device, dtype=torch.float16)
    product = (left @ left).float().sum().item()
    if product != 64 * 64 * 64:
        raise SystemExit(json.dumps({"error": f"行列積の結果が不正です: {product}", **detail},
                                    ensure_ascii=False))
    image = torch.ones((1, 1, 8, 8), device=device, dtype=torch.float16)
    weight = torch.ones((1, 1, 3, 3), device=device, dtype=torch.float16)
    convolved = torch.nn.functional.conv2d(image, weight).float().sum().item()
    if convolved != 9 * 36:
        raise SystemExit(json.dumps({"error": f"畳み込みの結果が不正です: {convolved}", **detail},
                                    ensure_ascii=False))
    torch.mps.synchronize()
    return detail


def check(backend):
    import torch

    detail = {
        "backend": backend,
        "torch": torch.__version__,
        "cuda": torch.version.cuda,
        "hip": getattr(torch.version, "hip", None),
    }
    if backend == "mps":
        return check_mps(torch, detail)
    if not torch.cuda.is_available():
        raise SystemExit(json.dumps(
            {"error": "GPUを利用できません（torch.cuda.is_available()がFalse）", **detail},
            ensure_ascii=False))
    if backend == "rocm" and not detail["hip"]:
        raise SystemExit(json.dumps(
            {"error": "ROCm版PyTorchではありません（torch.version.hipが空）", **detail},
            ensure_ascii=False))
    if backend == "cuda" and not detail["cuda"]:
        raise SystemExit(json.dumps(
            {"error": "CUDA版PyTorchではありません（torch.version.cudaが空）", **detail},
            ensure_ascii=False))
    device = torch.device("cuda", 0)
    detail["device"] = torch.cuda.get_device_name(device)
    if not detail["device"]:
        raise SystemExit(json.dumps({"error": "GPUデバイス名を取得できません", **detail},
                                    ensure_ascii=False))
    # 最小テンソル演算。行列積と畳み込みを半精度で通し、実際に計算できることを確かめる。
    left = torch.ones((64, 64), device=device, dtype=torch.float16)
    product = (left @ left).float().sum().item()
    if product != 64 * 64 * 64:
        raise SystemExit(json.dumps({"error": f"行列積の結果が不正です: {product}", **detail},
                                    ensure_ascii=False))
    image = torch.ones((1, 1, 8, 8), device=device, dtype=torch.float16)
    weight = torch.ones((1, 1, 3, 3), device=device, dtype=torch.float16)
    convolved = torch.nn.functional.conv2d(image, weight).float().sum().item()
    if convolved != 9 * 36:
        raise SystemExit(json.dumps({"error": f"畳み込みの結果が不正です: {convolved}", **detail},
                                    ensure_ascii=False))
    torch.cuda.synchronize(device)
    return detail


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", choices=("cuda", "rocm", "mps"), required=True)
    args = parser.parse_args()
    try:
        detail = check(args.backend)
    except SystemExit:
        raise
    except Exception as error:  # noqa: BLE001 - 失敗の内容をそのまま呼び出し元へ返す
        raise SystemExit(json.dumps(
            {"error": f"{type(error).__name__}: {error}", "backend": args.backend},
            ensure_ascii=False)) from error
    print(json.dumps({"ok": True, **detail}, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    sys.exit(main())
